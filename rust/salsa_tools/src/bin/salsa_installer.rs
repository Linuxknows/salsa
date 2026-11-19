// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::env;
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use salsa_tools::debug_core::SystemAuditScanner;
use salsa_tools::host::{
    detect_pkg_db_path, ensure_dir_0700, find_cargo, find_host_lib, getuid, home_dir, install_root,
    read_text_lossy, sha256_file, state_dir, which, write_atomic_secure,
};
use salsa_tools::json::JsonValue;
use salsa_tools::zlib_png::{decode_png_rgba, encode_png_rgba, resize_rgba};
use salsa_wire::legal::{
    handle_common_cli_flag, COPY_TEXT, LESSER_COPY_TEXT, THIRD_PARTY_TEXT,
};
use salsa_wire::unix_sys::reset_sigpipe_default;

const ICON_SIZES: [u32; 5] = [512, 256, 128, 64, 48];

const PACMAN_HOOK_PATH: &str = "/etc/pacman.d/hooks/99-salsa-autosync.hook";
const LEGACY_PACMAN_HOOK_PATHS: [&str; 3] = [
    "/etc/pacman.d/hooks/95-salsa-autosync.hook",
    "/etc/pacman.d/hooks/99-lwasapi-autosync.hook",
    "/etc/pacman.d/hooks/lwasapi-autosync.hook",
];
const AUTOSYNC_MARKER_DIR: &str = "/var/lib/salsa-autosync";
const AUTOSYNC_MARKER_PATH: &str = "/var/lib/salsa-autosync/libpulse-updated";
const PACMAN_HOOK_CONTENT: &str = "[Trigger]\n\
Operation = Upgrade\n\
Operation = Install\n\
Operation = Remove\n\
Type = Package\n\
Target = libpulse\n\
Target = lib32-libpulse\n\
Target = alsa-lib\n\
Target = lib32-alsa-lib\n\
Target = jack2\n\
Target = pipewire\n\
Target = pipewire-alsa\n\
Target = pipewire-pulse\n\
Target = pipewire-jack\n\
Target = fdk-aac\n\
Target = libfdk-aac\n\
Target = libldac\n\
Target = libfreeaptx\n\
Target = bluez\n\
Target = dbus\n\
Target = qt6-base\n\
\n\
[Action]\n\
Description = Marking Salsa WASAPI runtime modules for ABI auto-sync...\n\
When = PostTransaction\n\
Exec = /bin/sh -c '/usr/bin/mkdir -p /var/lib/salsa-autosync && /usr/bin/date +%s%N > /var/lib/salsa-autosync/libpulse-updated'\n";

fn sh_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn systemd_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '%' => out.push_str("%%"),
            '\n' | '\r' | '\0' => {}
            c => out.push(c),
        }
    }
    out
}

fn systemd_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn desktop_exec_quote(s: &str) -> String {
    let needs_quote = s.chars().any(|c| {
        c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '`' | '>' | '<' | '~' | '|' | '&' | ';' | '(' | ')' | '*' | '?' | '#' | '=')
    });
    let escaped_pct = s.replace('%', "%%");
    if !needs_quote {
        return escaped_pct;
    }
    let mut out = String::with_capacity(escaped_pct.len() + 2);
    out.push('"');
    for ch in escaped_pct.chars() {
        match ch {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Install,
    Check,
    Uninstall,
    InstallPacmanHook,
    RemovePacmanHook,
}

fn config_home() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| v.starts_with('/'))
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config"))
}

fn data_home() -> PathBuf {
    env::var("XDG_DATA_HOME")
        .ok()
        .filter(|v| v.starts_with('/'))
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/share"))
}

fn bin_home() -> PathBuf {
    home_dir().join(".local/bin")
}

fn force_symlink(target: &Path, link_path: &Path) -> Result<(), String> {
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let tmp = link_path.with_extension("tmp_sym");
    let _ = fs::remove_file(&tmp);
    symlink(target, &tmp).map_err(|e| format!("symlink {} -> {}: {e}", tmp.display(), target.display()))?;
    fs::rename(&tmp, link_path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), link_path.display()))
}

fn run_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_quiet(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ensure_host_deps(check_only: bool) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let required_tools = ["systemctl", "make", "gcc", "g++", "pkg-config", "readelf", "ldconfig"];
    let mut missing = Vec::new();
    for tool in required_tools {
        if which(tool).is_none() {
            missing.push(tool.to_string());
        }
    }
    if find_cargo().is_none() {
        missing.push("cargo".to_string());
    }
    for lib in [
        "libz.so.1",
        "libasound.so.2",
        "libpulse.so.0",
        "libQt6Widgets.so.6",
        "libQt6DBus.so.6",
    ] {
        if find_host_lib(lib).is_none() {
            missing.push(lib.to_string());
        }
    }

    if !missing.is_empty() {
        if check_only {
            return Err(format!("Missing host dependencies: {}", missing.join(", ")));
        }
        notes.push(format!("Detected missing host deps: {}", missing.join(", ")));
        if which("pacman").is_some() {
            println!("Attempting dependency check via pacman (requires sudo)...");
            let mut args = vec!["pacman", "-S", "--needed", "--noconfirm"];
            args.extend(["base-devel", "pkgconf", "alsa-lib", "libpulse", "zlib", "qt6-base"]);
            if missing.iter().any(|m| m == "cargo") {
                args.push("rust");
            }
            let _ = Command::new("sudo").args(&args).status();
        }
    } else {
        notes.push("Host toolchain, Qt6 & core audio libraries OK".to_string());
    }

    Ok(notes)
}

fn write_user_units(root: &Path) -> Result<(), String> {
    let unit_dir = config_home().join("systemd/user");
    fs::create_dir_all(&unit_dir).map_err(|e| format!("mkdir {}: {e}", unit_dir.display()))?;

    let root_wd = systemd_path(&root.to_string_lossy());
    let audiodg_exec = systemd_quote(&root.join("build/lwasapi-audiodg").to_string_lossy());
    let tray_exec = systemd_quote(&root.join("build/lwasapi-tray").to_string_lossy());
    let autosync_exec = systemd_quote(&root.join("build/lwasapi-autosync").to_string_lossy());

    let audiodg_unit = format!(
        "[Unit]\n\
         Description=Salsa — Linux WASAPI (lwasapi-audiodg) WaveRT 192kHz, Native PA v35, Direct BlueZ, Video Broker & Dynamic EQ Engine\n\
         After=bluetooth.target sound.target\n\
         Wants=bluetooth.target sound.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={root_wd}\n\
         ExecStart={audiodg_exec}\n\
         UMask=0077\n\
         Restart=always\n\
         RestartSec=1\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    );
    write_atomic_secure(&unit_dir.join("lwasapi-audiodg.service"), audiodg_unit.as_bytes(), 0o644)
        .map_err(|e| format!("write audiodg unit: {e}"))?;

    let tray_unit = format!(
        "[Unit]\n\
         Description=Salsa — Linux WASAPI Pure Native Audio System Tray Switcher\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target lwasapi-audiodg.service\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={root_wd}\n\
         ExecStart={tray_exec}\n\
         KillMode=process\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n"
    );
    write_atomic_secure(&unit_dir.join("lwasapi-tray.service"), tray_unit.as_bytes(), 0o644)
        .map_err(|e| format!("write tray unit: {e}"))?;

    let watcher_svc = format!(
        "[Unit]\n\
         Description=Salsa — Linux WASAPI Automatic Audio Module ABI & Config Sync Service\n\
         After=network.target\n\
         StartLimitIntervalSec=0\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         TimeoutStartSec=300\n\
         WorkingDirectory={root_wd}\n\
         ExecStart={autosync_exec} --auto\n"
    );
    write_atomic_secure(
        &unit_dir.join("lwasapi-module-watcher.service"),
        watcher_svc.as_bytes(),
        0o644,
    )
    .map_err(|e| format!("write watcher service: {e}"))?;

    let pkg_db = detect_pkg_db_path();
    let mut path_lines = format!(
        "[Unit]\n\
         Description=Salsa — Linux WASAPI Automatic Audio Module Update Watcher (Path Trigger)\n\
         StartLimitIntervalSec=0\n\
         \n\
         [Path]\n\
         PathChanged={pkg_db}\n\
         PathModified=/var/lib/salsa-autosync/libpulse-updated\n"
    );
    for lib in [
        "libasound.so.2",
        "libpulse.so.0",
        "libjack.so.0",
        "libfdk-aac.so.2",
        "libldacBT_enc.so.2",
        "libfreeaptx.so.0",
        "libdbus-1.so.3",
    ] {
        if let Some(p) = find_host_lib(lib) {
            path_lines.push_str(&format!("PathChanged={p}\n"));
        }
    }
    path_lines.push_str(
        "TriggerLimitIntervalSec=0\n\
         Unit=lwasapi-module-watcher.service\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
    );
    write_atomic_secure(
        &unit_dir.join("lwasapi-module-watcher.path"),
        path_lines.as_bytes(),
        0o644,
    )
    .map_err(|e| format!("write watcher path unit: {e}"))?;

    let watcher_timer = "[Unit]\n\
         Description=Salsa — Periodic ABI & Config Sync Timer\n\
         \n\
         [Timer]\n\
         OnBootSec=1min\n\
         OnUnitActiveSec=6h\n\
         Unit=lwasapi-module-watcher.service\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n";
    write_atomic_secure(
        &unit_dir.join("lwasapi-module-watcher.timer"),
        watcher_timer.as_bytes(),
        0o644,
    )
    .map_err(|e| format!("write watcher timer: {e}"))?;

    Ok(())
}

fn install_bin_links(root: &Path) -> Result<(), String> {
    let bdir = bin_home();
    fs::create_dir_all(&bdir).map_err(|e| format!("mkdir {}: {e}", bdir.display()))?;

    let links: [(&str, PathBuf); 8] = [
        ("lwasapi-audiodg", root.join("build/lwasapi-audiodg")),
        ("lwasapi-ctl", root.join("build/lwasapi-ctl")),
        ("lwasapi-bench", root.join("build/lwasapi-bench")),
        ("lwasapi-run", root.join("build/lwasapi-run")),
        ("salsa-debug-cli", root.join("build/salsa-debug-cli")),
        ("salsa", root.join("build/salsa-studio")),
        ("salsa-debug", root.join("build/salsa-debug")),
        ("lwasapi-tray", root.join("build/lwasapi-tray")),
    ];
    for (name, target) in links {
        force_symlink(&target, &bdir.join(name))?;
    }

    let studio_q = sh_single_quote(&root.join("build/salsa-studio").to_string_lossy());
    let salsa_graph = format!(
        "#!/usr/bin/env bash\nexec {studio_q} --graph \"$@\"\n"
    );
    write_atomic_secure(&bdir.join("salsa-graph"), salsa_graph.as_bytes(), 0o755)
        .map_err(|e| format!("write salsa-graph: {e}"))?;

    Ok(())
}

fn install_icons_and_desktop(root: &Path) -> Result<(), String> {
    let icon_src = root.join("assets/salsa-lwasapi.png");
    if icon_src.is_file() {
        let png_bytes =
            fs::read(&icon_src).map_err(|e| format!("read {}: {e}", icon_src.display()))?;
        let rgba = decode_png_rgba(&png_bytes)?;
        let hicolor = data_home().join("icons/hicolor");
        for sz in ICON_SIZES {
            let dir = hicolor.join(format!("{sz}x{sz}/apps"));
            fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
            let resized = if rgba.width == sz && rgba.height == sz {
                rgba.clone()
            } else {
                resize_rgba(&rgba, sz, sz)
            };
            let encoded = encode_png_rgba(&resized)?;
            write_atomic_secure(&dir.join("salsa-lwasapi.png"), &encoded, 0o644)
                .map_err(|e| format!("write icon {sz}x{sz}: {e}"))?;
        }
        if which("gtk-update-icon-cache").is_some() {
            let _ = run_quiet(
                "gtk-update-icon-cache",
                &["-f", "-t", &hicolor.to_string_lossy()],
            );
        }
    }

    let apps_dir = data_home().join("applications");
    fs::create_dir_all(&apps_dir).map_err(|e| format!("mkdir {}: {e}", apps_dir.display()))?;

    let studio_exec = desktop_exec_quote(&root.join("build/salsa-studio").to_string_lossy());
    let debug_exec = desktop_exec_quote(&root.join("build/salsa-debug").to_string_lossy());
    let tray_exec = desktop_exec_quote(&root.join("build/lwasapi-tray").to_string_lossy());

    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Salsa Audio Studio\n\
         Comment=Linux WASAPI Engine Control, Equalizer & Audio Matrix\n\
         Exec={studio_exec}\n\
         Icon=salsa-lwasapi\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;Mixer;\n\
         StartupNotify=true\n"
    );
    write_atomic_secure(&apps_dir.join("salsa-lwasapi.desktop"), desktop.as_bytes(), 0o644)
        .map_err(|e| format!("write desktop entry: {e}"))?;

    let debug_desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Salsa — Live Interaction Debugger\n\
         GenericName=SALSA Telemetry, Routing Audit & Protocol Debugger\n\
         Comment=Live event stream, /proc audio routing audit, RT SCHED_FIFO jitter, BlueZ/PA v35/WASAPI SHM inspection & interactive probes\n\
         Exec={debug_exec}\n\
         Icon=salsa-lwasapi\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;Development;System;\n\
         Keywords=WASAPI;Salsa;Debugger;Telemetry;Trace;BlueZ;PulseAudio;MIDI;ALSA;\n\
         StartupNotify=true\n"
    );
    write_atomic_secure(
        &apps_dir.join("salsa-debug.desktop"),
        debug_desktop.as_bytes(),
        0o644,
    )
    .map_err(|e| format!("write debug desktop entry: {e}"))?;

    let tray_desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Salsa — Audio System Tray\n\
         GenericName=Salsa 192kHz Audio Control\n\
         Comment=Control Salsa LWASAPI (192kHz WaveRT, Dynamic EQ, Sample Rate, and Status) from the system tray\n\
         Exec={tray_exec}\n\
         Icon=salsa-lwasapi\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;Settings;\n\
         StartupNotify=false\n"
    );
    write_atomic_secure(
        &apps_dir.join("lwasapi-tray.desktop"),
        tray_desktop.as_bytes(),
        0o644,
    )
    .map_err(|e| format!("write tray desktop entry: {e}"))?;

    if which("update-desktop-database").is_some() {
        let _ = run_quiet("update-desktop-database", &[&apps_dir.to_string_lossy()]);
    }
    Ok(())
}

const LICENSE_FILES: [(&str, &str); 3] = [
    ("COPYING", COPY_TEXT),
    ("COPYING.LESSER", LESSER_COPY_TEXT),
    ("THIRD_PARTY_LICENSES.md", THIRD_PARTY_TEXT),
];

fn licenses_install_dir() -> PathBuf {
    data_home().join("licenses/salsa")
}

fn install_license_files(root: &Path) -> Result<(), String> {
    let lic_dir = licenses_install_dir();
    fs::create_dir_all(&lic_dir).map_err(|e| format!("mkdir {}: {e}", lic_dir.display()))?;
    for (name, fallback) in LICENSE_FILES {
        let src = root.join(name);
        let bytes = fs::read(&src).unwrap_or_else(|_| fallback.as_bytes().to_vec());
        write_atomic_secure(&lic_dir.join(name), &bytes, 0o644)
            .map_err(|e| format!("write {}: {e}", lic_dir.join(name).display()))?;
    }
    Ok(())
}

fn is_symlink_path(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn configure_flatpak_and_kde(st_dir: &Path) {
    const FLATPAK_ALLOWLIST: &[&str] = &[
        "com.valvesoftware.Steam",
        "com.usebottles.bottles",
        "com.github.Matoking.protontricks",
        "com.discordapp.Discord",
        "dev.vencord.Vesktop",
        "org.mozilla.firefox",
        "com.obsproject.Studio",
        "net.lutris.Lutris",
        "com.heroicgameslauncher.hgl",
    ];
    const OSD_KEYS: &[&str] = &[
        "VolumeOsd",
        "MuteOsd",
        "MicrophoneSensitivityOsd",
        "DefaultOutputDeviceOsd",
    ];

    if which("flatpak").is_some() {
        let ovr_path = st_dir.join("flatpak_overrides.list");
        if !is_symlink_path(&ovr_path) {
            let mut recorded: Vec<String> = read_text_lossy(&ovr_path, 64 * 1024)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            let installed_txt = Command::new("flatpak")
                .args(["list", "--app", "--columns=application"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let installed: Vec<&str> = installed_txt.lines().map(str::trim).collect();
            for app_id in FLATPAK_ALLOWLIST {
                if !installed.contains(app_id) {
                    continue;
                }
                let show_txt = Command::new("flatpak")
                    .args(["override", "--user", "--show", app_id])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default();
                let mut cur_sockets = "";
                let mut cur_fs = "";
                for line in show_txt.lines() {
                    if let Some(v) = line.strip_prefix("sockets=") {
                        cur_sockets = v;
                    } else if let Some(v) = line.strip_prefix("filesystems=") {
                        cur_fs = v;
                    }
                }
                if !format!(";{cur_sockets};").contains(";pulseaudio;")
                    && run_quiet("flatpak", &["override", "--user", "--socket=pulseaudio", app_id])
                {
                    let entry = format!("socket:{app_id}");
                    if !recorded.contains(&entry) {
                        recorded.push(entry);
                    }
                }
                if !format!(";{cur_fs};").contains(";xdg-run/pipewire-0")
                    && run_quiet(
                        "flatpak",
                        &["override", "--user", "--filesystem=xdg-run/pipewire-0:ro", app_id],
                    )
                {
                    let entry = format!("fs:{app_id}");
                    if !recorded.contains(&entry) {
                        recorded.push(entry);
                    }
                }
            }
            let body = if recorded.is_empty() {
                String::new()
            } else {
                format!("{}\n", recorded.join("\n"))
            };
            let _ = write_atomic_secure(&ovr_path, body.as_bytes(), 0o600);
        }
    }

    let cfg_home = config_home();
    let plasmaparc = cfg_home.join("plasmaparc");
    let kde_session = env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.contains("KDE"))
        .unwrap_or(false)
        || plasmaparc.exists()
        || run_quiet("pgrep", &["-x", "plasmashell"]);

    if which("kwriteconfig6").is_some() && kde_session {
        let osd_state = st_dir.join("plasmaparc_osd.state");
        if !is_symlink_path(&plasmaparc) && !is_symlink_path(&osd_state) {
            if !osd_state.exists() {
                let mut lines = Vec::with_capacity(OSD_KEYS.len());
                for osd_k in OSD_KEYS {
                    let prev_v = if which("kreadconfig6").is_some() {
                        Command::new("kreadconfig6")
                            .args([
                                "--file",
                                &plasmaparc.to_string_lossy(),
                                "--group",
                                "General",
                                "--key",
                                osd_k,
                                "--default",
                                "__UNSET__",
                            ])
                            .output()
                            .map(|o| {
                                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                                if s.is_empty() { "__UNSET__".to_string() } else { s }
                            })
                            .unwrap_or_else(|_| "__UNSET__".to_string())
                    } else {
                        "__UNSET__".to_string()
                    };
                    lines.push(format!("{osd_k}={prev_v}"));
                }
                let _ = write_atomic_secure(
                    &osd_state,
                    format!("{}\n", lines.join("\n")).as_bytes(),
                    0o600,
                );
            }
            for osd_k in OSD_KEYS {
                let _ = run_quiet(
                    "kwriteconfig6",
                    &[
                        "--file",
                        &plasmaparc.to_string_lossy(),
                        "--group",
                        "General",
                        "--key",
                        osd_k,
                        "false",
                    ],
                );
            }
            if which("qdbus6").is_some() {
                let _ = run_quiet(
                    "qdbus6",
                    &["org.kde.kded6", "/kded", "unloadModule", "audioshortcutsservice"],
                );
                let _ = run_quiet(
                    "qdbus6",
                    &["org.kde.kded6", "/kded", "loadModule", "audioshortcutsservice"],
                );
            }
        }

        let ksm_rc = cfg_home.join("ksmserverrc");
        if !is_symlink_path(&ksm_rc) {
            let cur_excl = if which("kreadconfig6").is_some() {
                Command::new("kreadconfig6")
                    .args([
                        "--file",
                        &ksm_rc.to_string_lossy(),
                        "--group",
                        "General",
                        "--key",
                        "excludeApps",
                        "--default",
                        "",
                    ])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let mut items: Vec<String> = cur_excl
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            for ex in ["lwasapi-tray", "lwasapi-tray.py", "lwasapi-tray.desktop"] {
                if !items.iter().any(|x| x == ex) {
                    items.push(ex.to_string());
                }
            }
            let new_excl = items.join(",");
            if new_excl != cur_excl {
                let _ = run_quiet(
                    "kwriteconfig6",
                    &[
                        "--file",
                        &ksm_rc.to_string_lossy(),
                        "--group",
                        "General",
                        "--key",
                        "excludeApps",
                        &new_excl,
                    ],
                );
            }
        }
    }
}

fn do_install(root: &Path) -> ExitCode {
    if getuid() == 0 {
        eprintln!("Do not run salsa-installer --install as root; run as your normal user.");
        return ExitCode::from(1);
    }

    println!("==> Checking host dependencies...");
    match ensure_host_deps(false) {
        Ok(notes) => {
            for n in notes {
                println!("  [OK] {n}");
            }
        }
        Err(e) => {
            eprintln!("  [ERR] {e}");
            return ExitCode::from(1);
        }
    }

    println!("==> Ensuring runtime build is up to date...");
    let freshness = SystemAuditScanner::check_build_freshness(root);
    if freshness
        .get("stale")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true)
    {
        let jobs_flag = format!(
            "-j{}",
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        );
        if !Command::new("make")
            .args(["-C", &root.to_string_lossy(), &jobs_flag, "all"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            eprintln!("Build failed during install.");
            return ExitCode::from(1);
        }
    } else {
        println!("  [OK] Build artifacts are already up to date.");
    }

    let st_dir = state_dir();
    if let Err(e) = ensure_dir_0700(&st_dir) {
        eprintln!("Failed creating {}: {e}", st_dir.display());
        return ExitCode::from(1);
    }

    println!("==> Writing systemd user units & binaries...");
    if let Err(e) = write_user_units(root) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    if let Err(e) = install_bin_links(root) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    if let Err(e) = install_icons_and_desktop(root) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    if let Err(e) = install_license_files(root) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    configure_flatpak_and_kde(&st_dir);

    println!("==> Enabling systemd user services...");
    if !run_quiet("systemctl", &["--user", "daemon-reload"]) {
        eprintln!("  [WARN] systemctl --user daemon-reload returned non-zero");
    }
    let ctl = root.join("build/lwasapi-ctl");
    if ctl.is_file() && !run_quiet(&ctl.to_string_lossy(), &["enable-auto"]) {
        eprintln!("  [WARN] lwasapi-ctl enable-auto returned non-zero");
    }
    let autosync = root.join("build/lwasapi-autosync");
    if autosync.is_file() && !run_quiet(&autosync.to_string_lossy(), &["--quiet"]) {
        eprintln!("  [WARN] lwasapi-autosync --quiet returned non-zero");
    }

    for unit in [
        "lwasapi-audiodg.service",
        "lwasapi-module-watcher.path",
        "lwasapi-module-watcher.timer",
    ] {
        if !run_ok("systemctl", &["--user", "enable", "--now", unit]) {
            eprintln!("  [WARN] systemctl --user enable --now {unit} failed");
        }
    }
    if !run_quiet("systemctl", &["--user", "restart", "lwasapi-audiodg.service"]) {
        eprintln!("  [WARN] systemctl --user restart lwasapi-audiodg.service returned non-zero");
    }
    if !run_quiet("systemctl", &["--user", "enable", "--now", "lwasapi-tray.service"]) {
        eprintln!("  [WARN] systemctl --user enable --now lwasapi-tray.service returned non-zero");
    }

    println!("[OK] Salsa installation complete. Run `./install-salsa.sh --check` to audit.");
    ExitCode::SUCCESS
}

fn do_check(root: &Path) -> ExitCode {
    println!("==> Salsa Installation & Host Parity Check ({})", root.display());
    let mut failures = 0u32;

    if let Err(e) = ensure_host_deps(true) {
        println!("  [FAIL] {e}");
        failures += 1;
    } else {
        println!("  [OK] Host toolchain, Qt6 & core audio libraries available");
    }

    let freshness = SystemAuditScanner::check_build_freshness(root);
    let stale = freshness
        .get("stale")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let summary = freshness
        .get("summary")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    if stale {
        println!("  [FAIL] Build freshness: {summary}");
        if let Some(arr) = freshness.get("stale_artifacts").and_then(JsonValue::as_array) {
            for item in arr {
                let a = item.get("artifact").and_then(JsonValue::as_str).unwrap_or("?");
                let r = item.get("reason").and_then(JsonValue::as_str).unwrap_or("?");
                println!("         - {a}: {r}");
            }
        }
        failures += 1;
    } else {
        println!("  [OK] Build freshness: {summary}");
    }

    let bdir = bin_home();
    for name in [
        "lwasapi-ctl",
        "lwasapi-bench",
        "lwasapi-run",
        "salsa-debug-cli",
        "salsa",
        "salsa-debug",
        "lwasapi-tray",
        "salsa-graph",
    ] {
        let p = bdir.join(name);
        if p.exists() {
            println!("  [OK] ~/.local/bin/{name} -> {}", fs::read_link(&p).unwrap_or(p.clone()).display());
        } else {
            println!("  [FAIL] ~/.local/bin/{name} missing");
            failures += 1;
        }
    }

    let unit_dir = config_home().join("systemd/user");
    for unit in [
        "lwasapi-audiodg.service",
        "lwasapi-tray.service",
        "lwasapi-module-watcher.path",
        "lwasapi-module-watcher.service",
        "lwasapi-module-watcher.timer",
    ] {
        let up = unit_dir.join(unit);
        let active = run_quiet("systemctl", &["--user", "is-active", unit]);
        let enabled = run_quiet("systemctl", &["--user", "is-enabled", unit]);
        println!(
            "  [{}] unit {unit}: file={}, enabled={enabled}, active={active}",
            if up.is_file() { "OK" } else { "FAIL" },
            up.is_file()
        );
        if !up.is_file() {
            failures += 1;
        }
    }

    let repo_plugin = root.join("build/libasound_module_pcm_lwasapi.so");
    let sys_plugin = Path::new("/usr/lib/alsa-lib/libasound_module_pcm_lwasapi.so");
    let sha_repo = sha256_file(&repo_plugin);
    let sha_sys = sha256_file(sys_plugin);
    if sha_repo.is_some() && sha_repo == sha_sys {
        println!("  [OK] System ALSA lwasapi plugin matches build artifact");
    } else {
        println!(
            "  [INFO] System ALSA plugin (/usr/lib/alsa-lib/libasound_module_pcm_lwasapi.so) differs or not installed (optional root install)"
        );
    }

    let apps = data_home().join("applications");
    let desktop_ok = [
        "salsa-lwasapi.desktop",
        "salsa-debug.desktop",
        "lwasapi-tray.desktop",
    ]
    .iter()
    .all(|f| apps.join(f).is_file());
    let icon_ok = ICON_SIZES.iter().all(|sz| {
        data_home()
            .join(format!("icons/hicolor/{sz}x{sz}/apps/salsa-lwasapi.png"))
            .is_file()
    });
    println!(
        "  [{}] Desktop icons ({:?}) & .desktop entries (all_present={desktop_ok})",
        if icon_ok && desktop_ok { "OK" } else { "WARN" },
        ICON_SIZES
    );

    let lic_dir = licenses_install_dir();
    let missing_lics: Vec<&str> = LICENSE_FILES
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !lic_dir.join(n).is_file())
        .collect();
    if missing_lics.is_empty() {
        println!(
            "  [OK] LGPL-3.0 license bundle installed ({})",
            lic_dir.display()
        );
    } else {
        println!(
            "  [FAIL] Missing LGPL-3.0 license file(s) in {}: {:?}",
            lic_dir.display(),
            missing_lics
        );
        failures += 1;
    }

    if Path::new("/etc/pacman.d").is_dir() {
        let hook = Path::new(PACMAN_HOOK_PATH).is_file();
        let up_to_date = hook
            && fs::read_to_string(PACMAN_HOOK_PATH).is_ok_and(|cur| cur == PACMAN_HOOK_CONTENT);
        println!(
            "  [{}] Pacman autosync hook ({PACMAN_HOOK_PATH}): {}",
            if up_to_date { "OK" } else { "INFO" },
            if up_to_date {
                "installed & up-to-date"
            } else if hook {
                "outdated trigger (re-run --install-pacman-hook to refresh)"
            } else {
                "not installed (run --install-pacman-hook)"
            }
        );
    }

    let bt_pwr = SystemAuditScanner::inspect_bt_usb_power();
    if bt_pwr.autosuspend_risk {
        println!(
            "  [WARN] Bluetooth USB autosuspend enabled ({}) — can cause A2DP dropouts; set power/control=on or btusb.enable_autosuspend=0",
            bt_pwr.summary
        );
    } else if !bt_pwr.adapters.is_empty() {
        println!("  [OK] Bluetooth USB power control: {}", bt_pwr.summary);
    }

    if failures == 0 {
        println!("==> Check PASSED (0 failures)");
        ExitCode::SUCCESS
    } else {
        println!("==> Check FAILED ({failures} failures)");
        ExitCode::from(1)
    }
}

fn do_uninstall(root: &Path) -> ExitCode {
    println!("==> Stopping & disabling Salsa user units...");
    for unit in [
        "lwasapi-module-watcher.timer",
        "lwasapi-module-watcher.path",
        "lwasapi-module-watcher.service",
        "lwasapi-tray.service",
        "lwasapi-audiodg.service",
    ] {
        let _ = run_quiet("systemctl", &["--user", "disable", "--now", unit]);
        let _ = fs::remove_file(config_home().join("systemd/user").join(unit));
    }
    let _ = run_quiet("systemctl", &["--user", "daemon-reload"]);

    let bdir = bin_home();
    for name in [
        "lwasapi-audiodg",
        "lwasapi-ctl",
        "lwasapi-bench",
        "lwasapi-run",
        "salsa-debug-cli",
        "salsa",
        "salsa-debug",
        "lwasapi-tray",
        "salsa-graph",
    ] {
        let _ = fs::remove_file(bdir.join(name));
    }

    for df in [
        "salsa-lwasapi.desktop",
        "salsa-debug.desktop",
        "lwasapi-tray.desktop",
    ] {
        let _ = fs::remove_file(data_home().join("applications").join(df));
    }
    for sz in ICON_SIZES {
        let _ = fs::remove_file(
            data_home().join(format!("icons/hicolor/{sz}x{sz}/apps/salsa-lwasapi.png")),
        );
    }
    let lic_dir = licenses_install_dir();
    for (name, _) in LICENSE_FILES {
        let _ = fs::remove_file(lic_dir.join(name));
    }
    let _ = fs::remove_dir(&lic_dir);

    let st_dir = state_dir();
    let cfg_home = config_home();
    let plasmaparc = cfg_home.join("plasmaparc");
    let osd_state = st_dir.join("plasmaparc_osd.state");
    if osd_state.is_file()
        && !is_symlink_path(&osd_state)
        && !is_symlink_path(&plasmaparc)
        && which("kwriteconfig6").is_some()
    {
        let content = read_text_lossy(&osd_state, 16 * 1024);
        for line in content.lines() {
            let Some((osd_k, osd_v)) = line.trim().split_once('=') else {
                continue;
            };
            if osd_k.is_empty() {
                continue;
            }
            if osd_v == "__UNSET__" {
                let _ = run_quiet(
                    "kwriteconfig6",
                    &[
                        "--file",
                        &plasmaparc.to_string_lossy(),
                        "--group",
                        "General",
                        "--key",
                        osd_k,
                        "--delete",
                    ],
                );
            } else {
                let _ = run_quiet(
                    "kwriteconfig6",
                    &[
                        "--file",
                        &plasmaparc.to_string_lossy(),
                        "--group",
                        "General",
                        "--key",
                        osd_k,
                        osd_v,
                    ],
                );
            }
        }
        let _ = fs::remove_file(&osd_state);
        if which("qdbus6").is_some() {
            let _ = run_quiet(
                "qdbus6",
                &["org.kde.kded6", "/kded", "unloadModule", "audioshortcutsservice"],
            );
            let _ = run_quiet(
                "qdbus6",
                &["org.kde.kded6", "/kded", "loadModule", "audioshortcutsservice"],
            );
        }
    }

    let fp_file = st_dir.join("flatpak_overrides.list");
    if fp_file.is_file() && !is_symlink_path(&fp_file) && which("flatpak").is_some() {
        let txt = read_text_lossy(&fp_file, 64 * 1024);
        for line in txt.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(app_id) = line.strip_prefix("socket:") {
                let _ = run_quiet(
                    "flatpak",
                    &["override", "--user", "--nosocket=pulseaudio", app_id],
                );
            } else if let Some(app_id) = line.strip_prefix("fs:") {
                let _ = run_quiet(
                    "flatpak",
                    &["override", "--user", "--nofilesystem=xdg-run/pipewire-0:ro", app_id],
                );
            }
        }
        let _ = fs::remove_file(&fp_file);
    }

    let ctl = root.join("build/lwasapi-ctl");
    if ctl.is_file() {
        let _ = run_quiet(&ctl.to_string_lossy(), &["disable-auto"]);
    }

    println!("[OK] Salsa user installation removed.");
    ExitCode::SUCCESS
}

fn stdin_is_tty() -> bool {
    extern "C" {
        fn isatty(fd: core::ffi::c_int) -> core::ffi::c_int;
    }
    unsafe { isatty(0) != 0 }
}

fn run_privileged_self(flag: &str) -> ExitCode {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("salsa-installer"));
    let has_gui = env::var_os("DISPLAY").is_some() || env::var_os("WAYLAND_DISPLAY").is_some();
    if (!stdin_is_tty() || has_gui && which("sudo").is_none()) && which("pkexec").is_some() {
        if let Ok(st) = Command::new("pkexec").arg(&exe).arg(flag).status() {
            if st.success() {
                return ExitCode::SUCCESS;
            }
        }
    }
    let st = Command::new("sudo").arg(&exe).arg(flag).status();
    if st.map(|s| s.success()).unwrap_or(false) {
        ExitCode::SUCCESS
    } else if which("pkexec").is_some() {
        let st2 = Command::new("pkexec").arg(&exe).arg(flag).status();
        if st2.map(|s| s.success()).unwrap_or(false) {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        }
    } else {
        ExitCode::from(1)
    }
}

fn do_install_pacman_hook() -> ExitCode {
    if getuid() == 0 {
        if !Path::new("/etc/pacman.d").is_dir() {
            eprintln!("/etc/pacman.d does not exist; skipping Pacman hook installation.");
            return ExitCode::from(1);
        }
        let hook = Path::new(PACMAN_HOOK_PATH);
        if let Some(p) = hook.parent() {
            if let Err(e) = fs::create_dir_all(p) {
                eprintln!("Failed creating {}: {e}", p.display());
                return ExitCode::from(1);
            }
        }
        if let Err(e) = write_atomic_secure(hook, PACMAN_HOOK_CONTENT.as_bytes(), 0o644) {
            eprintln!("Failed writing {PACMAN_HOOK_PATH}: {e}");
            return ExitCode::from(1);
        }
        for legacy in LEGACY_PACMAN_HOOK_PATHS {
            let _ = fs::remove_file(legacy);
        }
        let marker_dir = Path::new(AUTOSYNC_MARKER_DIR);
        let _ = fs::create_dir_all(marker_dir);
        let _ = fs::set_permissions(marker_dir, Permissions::from_mode(0o755));
        let _ = write_atomic_secure(Path::new(AUTOSYNC_MARKER_PATH), b"0\n", 0o644);
        println!("[OK] Installed {PACMAN_HOOK_PATH}");
        return ExitCode::SUCCESS;
    }
    run_privileged_self("--install-pacman-hook")
}

fn do_remove_pacman_hook() -> ExitCode {
    if getuid() == 0 {
        let _ = fs::remove_file(PACMAN_HOOK_PATH);
        for legacy in LEGACY_PACMAN_HOOK_PATHS {
            let _ = fs::remove_file(legacy);
        }
        let _ = fs::remove_file(AUTOSYNC_MARKER_PATH);
        let _ = fs::remove_dir(AUTOSYNC_MARKER_DIR);
        println!("[OK] Removed {PACMAN_HOOK_PATH}");
        return ExitCode::SUCCESS;
    }
    run_privileged_self("--remove-pacman-hook")
}

fn print_help() {
    println!(
        "Usage: salsa-installer [OPTION]\n\n\
         Options:\n  \
         --install             Install Salsa user services, icons, links, and integration (default)\n  \
         --check               Verify host toolchain, build freshness, units, and symlinks\n  \
         --uninstall           Remove Salsa user units, symlinks, and desktop integration\n  \
         --install-pacman-hook Install {PACMAN_HOOK_PATH} (uses pkexec/sudo)\n  \
         --remove-pacman-hook  Remove {PACMAN_HOOK_PATH} (uses pkexec/sudo)\n  \
         -V, --version         Show Salsa version & LGPL-3.0 legal notice\n  \
         --license             Show full LGPL-3.0, GPL-3.0, and third-party terms\n  \
         -h, --help            Show this help"
    );
}

fn main() -> ExitCode {
    reset_sigpipe_default();
    let mut action = Action::Install;
    for arg in env::args().skip(1) {
        if let Some(rc) = handle_common_cli_flag("salsa-installer", &arg) {
            return ExitCode::from(rc as u8);
        }
        match arg.as_str() {
            "--install" => action = Action::Install,
            "--check" => action = Action::Check,
            "--uninstall" => action = Action::Uninstall,
            "--install-pacman-hook" => action = Action::InstallPacmanHook,
            "--remove-pacman-hook" => action = Action::RemovePacmanHook,
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown option: {other}");
                print_help();
                return ExitCode::from(2);
            }
        }
    }

    let root = install_root();
    match action {
        Action::Install => do_install(&root),
        Action::Check => do_check(&root),
        Action::Uninstall => do_uninstall(&root),
        Action::InstallPacmanHook => do_install_pacman_hook(),
        Action::RemovePacmanHook => do_remove_pacman_hook(),
    }
}
