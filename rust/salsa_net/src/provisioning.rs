// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

//! Canonical host provisioning, Zero-PipeWire cutover, path/unit constants,
//! and hardened config rendering shared by CLI, installer, autosync, doctor,
//! and Qt Studio.

#![deny(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::unix_sys::{
    posix_owner_id, resolve_sock_path_common, user_home_dir, write_atomic_secure_file, ERRNO_ELOOP,
    O_CLOEXEC, O_NOFOLLOW,
};

pub const MANAGED_USER_UNITS: [&str; 6] = [
    "lasapi-pipewire.socket",
    "lasapi-audiodg.service",
    "lasapi-tray.service",
    "lasapi-module-watcher.path",
    "lasapi-module-watcher.service",
    "lasapi-module-watcher.timer",
];

pub const MASKED_PIPEWIRE_UNITS: [&str; 7] = [
    "pipewire-pulse.socket",
    "pipewire-pulse.service",
    "wireplumber.service",
    "pipewire.socket",
    "pipewire.service",
    "pulseaudio.socket",
    "pulseaudio.service",
];

pub const RESTORE_PIPEWIRE_UNITS: [&str; 7] = [
    "pipewire.socket",
    "pipewire.service",
    "wireplumber.service",
    "pipewire-pulse.socket",
    "pipewire-pulse.service",
    "pulseaudio.socket",
    "pulseaudio.service",
];

pub const DRIFT_WATCHED_PIPEWIRE_UNITS: [&str; 4] = [
    "pipewire.service",
    "pipewire-pulse.service",
    "wireplumber.service",
    "pulseaudio.service",
];

pub const PORTAL_USER_UNITS: [&str; 7] = [
    "xdg-desktop-portal.service",
    "plasma-xdg-desktop-portal-kde.service",
    "xdg-desktop-portal-kde.service",
    "xdg-desktop-portal-wlr.service",
    "xdg-desktop-portal-hyprland.service",
    "xdg-desktop-portal-gnome.service",
    "xdg-desktop-portal-gtk.service",
];

pub const PACMAN_HOOK_PATH: &str = "/etc/pacman.d/hooks/99-salsa-autosync.hook";
pub const LEGACY_PACMAN_HOOK_PATHS: [&str; 3] = [
    "/etc/pacman.d/hooks/95-salsa-autosync.hook",
    "/etc/pacman.d/hooks/99-lasapi-autosync.hook",
    "/etc/pacman.d/hooks/lasapi-autosync.hook",
];

pub fn all_pacman_hook_paths() -> [&'static str; 4] {
    [
        PACMAN_HOOK_PATH,
        LEGACY_PACMAN_HOOK_PATHS[0],
        LEGACY_PACMAN_HOOK_PATHS[1],
        LEGACY_PACMAN_HOOK_PATHS[2],
    ]
}

pub const CANONICAL_ALSOFT_DRIVERS: &str = "pulse,alsa";
pub const CANONICAL_PROTON_USE_PIPEWIRE: &str = "0";
pub const CANONICAL_SDL_AUDIODRIVER: &str = "pulse";
pub const UNIFIED_SETTINGS_FILENAME: &str = "settings.json";
pub const LEGACY_SETTINGS_FILENAME: &str = "salsa_settings.json";
pub const DIRECT_ENV_REL_PATH: &str = "environment.d/99-salsa-direct.conf";
pub const VIDEO_ONLY_CONF_REL_PATH: &str = "pipewire/pipewire.conf.d/99-lasapi-video-only.conf";

const READ_LIMIT: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SalsaHostPaths {
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub data_home: PathBuf,
    pub bin_home: PathBuf,
    pub runtime_dir: PathBuf,
    pub install_root: PathBuf,
}

impl SalsaHostPaths {
    pub fn resolve(
        home: PathBuf,
        xdg_config_home: Option<&str>,
        xdg_data_home: Option<&str>,
        xdg_runtime_dir: Option<&str>,
        owner_id: u32,
        install_root: PathBuf,
    ) -> Self {
        let config_home = xdg_config_home
            .map(str::trim)
            .filter(|s| s.starts_with('/'))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let data_home = xdg_data_home
            .map(str::trim)
            .filter(|s| s.starts_with('/'))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        let bin_home = home.join(".local/bin");
        let runtime_dir = xdg_runtime_dir
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{owner_id}")));
        Self {
            home,
            config_home,
            data_home,
            bin_home,
            runtime_dir,
            install_root,
        }
    }

    pub fn from_env(install_root: PathBuf) -> Self {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(user_home_dir()));
        let cfg = std::env::var("XDG_CONFIG_HOME").ok();
        let data = std::env::var("XDG_DATA_HOME").ok();
        let rt = std::env::var("XDG_RUNTIME_DIR").ok();
        Self::resolve(
            home,
            cfg.as_deref(),
            data.as_deref(),
            rt.as_deref(),
            posix_owner_id(),
            install_root,
        )
    }

    pub fn config_lasapi_dir(&self) -> PathBuf {
        self.config_home.join("lasapi")
    }

    pub fn pipewire_zero_flag(&self) -> PathBuf {
        self.config_lasapi_dir().join("pipewire_zero_mode")
    }

    pub fn masked_units_list(&self) -> PathBuf {
        self.config_lasapi_dir().join("masked_units.list")
    }

    pub fn pw_backup_dir(&self) -> PathBuf {
        self.config_lasapi_dir().join("pw_backup")
    }

    pub fn asoundrc(&self) -> PathBuf {
        self.home.join(".asoundrc")
    }

    pub fn asoundrc_backup(&self) -> PathBuf {
        self.home.join(".asoundrc.lasapi.bak")
    }

    pub fn alsoft_conf(&self) -> PathBuf {
        self.config_home.join("OpenAL/alsoft.conf")
    }

    pub fn alsoft_conf_backup(&self) -> PathBuf {
        self.config_home.join("OpenAL/alsoft.conf.lasapi.bak")
    }

    pub fn direct_env_conf(&self) -> PathBuf {
        self.config_home.join(DIRECT_ENV_REL_PATH)
    }

    pub fn video_only_pw_conf(&self) -> PathBuf {
        self.config_home.join(VIDEO_ONLY_CONF_REL_PATH)
    }

    pub fn systemd_user_dir(&self) -> PathBuf {
        self.config_home.join("systemd/user")
    }

    pub fn applications_dir(&self) -> PathBuf {
        self.data_home.join("applications")
    }

    pub fn autostart_dir(&self) -> PathBuf {
        self.config_home.join("autostart")
    }

    pub fn steam_wrapper_bin(&self) -> PathBuf {
        self.bin_home.join("salsa-steam-launch")
    }

    pub fn steam_app_desktop(&self) -> PathBuf {
        self.applications_dir().join("steam.desktop")
    }

    pub fn steam_app_desktop_backup(&self) -> PathBuf {
        self.config_lasapi_dir().join("steam_app.desktop.bak")
    }

    pub fn steam_autostart_desktop(&self) -> PathBuf {
        self.autostart_dir().join("steam.desktop")
    }

    pub fn steam_autostart_desktop_backup(&self) -> PathBuf {
        self.config_lasapi_dir().join("steam_autostart.desktop.bak")
    }

    pub fn build_dir(&self) -> PathBuf {
        self.install_root.join("build")
    }

    pub fn shims_dir(&self) -> PathBuf {
        self.build_dir().join("shims")
    }

    pub fn pa_reconnect_dir(&self) -> PathBuf {
        self.build_dir().join("pa_reconnect")
    }

    pub fn pa_reconnect32_dir(&self) -> PathBuf {
        self.build_dir().join("pa_reconnect32")
    }

    pub fn alsa_plugin_so(&self) -> PathBuf {
        self.build_dir().join("libasound_module_pcm_lasapi.so")
    }

    pub fn pa_reconnect_so(&self) -> PathBuf {
        self.build_dir().join("libsalsa_pa_reconnect.so")
    }

    pub fn pipewire_socket_path(&self) -> PathBuf {
        self.runtime_dir.join("pipewire-0")
    }
}

pub fn reject_control_chars(label: &str, raw: &str) -> Result<(), String> {
    if raw.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{label} contains invalid control character or newline"));
    }
    Ok(())
}

/// Validates and escapes a string literal for `alsa.conf` / `~/.asoundrc` double quotes.
pub fn quote_alsa_conf_string(raw: &str) -> Result<String, String> {
    reject_control_chars("ALSA config value", raw)?;
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// Validates and quotes a `KEY=VALUE` value for `systemd` `environment.d/*.conf`.
pub fn quote_systemd_env_value(raw: &str) -> Result<String, String> {
    reject_control_chars("environment.d value", raw)?;
    let needs_quotes = raw.is_empty()
        || raw.chars().any(|c| {
            c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '#' | ';' | '`')
        });
    if !needs_quotes {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

pub fn sh_single_quote(raw: &str) -> Result<String, String> {
    reject_control_chars("shell argument", raw)?;
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('\'');
    for ch in raw.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    Ok(out)
}

fn is_salsa_pa_dir(seg: &str) -> bool {
    seg.ends_with("/build/pa_reconnect") || seg.ends_with("/build/pa_reconnect32")
}

pub fn build_filtered_ld_path(prepend_dir: Option<&str>, cur_ld: Option<&str>) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(p) = prepend_dir.filter(|s| !s.is_empty()) {
        parts.push(p);
    }
    if let Some(cur) = cur_ld.filter(|s| !s.is_empty()) {
        for seg in cur.split(':') {
            if seg.is_empty() || is_salsa_pa_dir(seg) {
                continue;
            }
            if prepend_dir == Some(seg) {
                continue;
            }
            parts.push(seg);
        }
    }
    let mut out = String::new();
    for seg in parts {
        let sep = usize::from(!out.is_empty());
        if out.len() + sep + seg.len() > 2047 {
            break;
        }
        if sep != 0 {
            out.push(':');
        }
        out.push_str(seg);
    }
    out
}

pub fn render_asoundrc(plugin_so: &Path) -> Result<String, String> {
    let plugin_str = plugin_so.to_string_lossy();
    let quoted = quote_alsa_conf_string(&plugin_str)?;
    Ok(format!(
        "# LASAPI Full-Duplex Automatic ALSA Application Recognition Config\n\
         # Routes both Playback and Capture streams through LASAPI.\n\
         # Revert anytime with: lasapi-ctl disable-auto\n\
         pcm_type.lasapi {{\n    \
             lib {quoted}\n\
         }}\n\n\
         pcm.lasapi_raw {{\n    \
             type lasapi\n\
         }}\n\n\
         pcm.lasapi {{\n    \
             type plug\n    \
             slave.pcm \"lasapi_raw\"\n\
         }}\n\n\
         pcm.!default {{\n    \
             type asym\n    \
             playback.pcm \"lasapi\"\n    \
             capture.pcm \"lasapi\"\n\
         }}\n"
    ))
}

pub fn render_alsoft_conf() -> String {
    format!(
        "# LASAPI OpenAL Soft Automatic Routing\n\
         [general]\n\
         drivers = {CANONICAL_ALSOFT_DRIVERS}\n\
         channels = stereo\n\
         sample-type = float32\n\
         frequency = 48000\n\
         period_size = 256\n\
         periods = 4\n"
    )
}

pub fn render_direct_env_conf(ld_val: &str) -> Result<String, String> {
    let quoted_ld = quote_systemd_env_value(ld_val)?;
    Ok(format!(
        "# Salsa Zero-PipeWire Direct Steam/Proton/Desktop Audio Environment\n\
         # Revert anytime with: lasapi-ctl disable-auto\n\
         PROTON_USE_PIPEWIRE={CANONICAL_PROTON_USE_PIPEWIRE}\n\
         SDL_AUDIODRIVER={CANONICAL_SDL_AUDIODRIVER}\n\
         ALSOFT_DRIVERS={CANONICAL_ALSOFT_DRIVERS}\n\
         LD_LIBRARY_PATH={quoted_ld}\n"
    ))
}

pub fn build_shim_launch_env(paths: &SalsaHostPaths, cur_ld: Option<&str>) -> Vec<(&'static str, String)> {
    let mut ld_path = format!("{}:{}", paths.shims_dir().display(), paths.build_dir().display());
    if let Some(prev) = cur_ld.filter(|s| !s.is_empty()) {
        ld_path.push(':');
        ld_path.push_str(prev);
    }
    vec![
        ("PROTON_USE_PIPEWIRE", CANONICAL_PROTON_USE_PIPEWIRE.to_string()),
        ("SDL_AUDIODRIVER", CANONICAL_SDL_AUDIODRIVER.to_string()),
        ("ALSOFT_DRIVERS", CANONICAL_ALSOFT_DRIVERS.to_string()),
        ("LD_LIBRARY_PATH", ld_path),
    ]
}

pub fn transform_steam_desktop(contents: &str, wrap_cmd: &str, enable: bool) -> String {
    let wrap_prefix = format!("Exec={wrap_cmd}");
    let mut out = String::with_capacity(contents.len() + 64);
    for chunk in contents.split_inclusive('\n') {
        if enable {
            if let Some(rest) = chunk.strip_prefix("Exec=/usr/bin/steam") {
                if rest.is_empty()
                    || rest.starts_with(' ')
                    || rest.starts_with('\t')
                    || rest.starts_with('\r')
                    || rest.starts_with('\n')
                {
                    out.push_str(&wrap_prefix);
                    out.push_str(rest);
                    continue;
                }
            }
            if let Some(rest) = chunk.strip_prefix("Exec=steam") {
                if rest.is_empty()
                    || rest.starts_with(' ')
                    || rest.starts_with('\t')
                    || rest.starts_with('\r')
                    || rest.starts_with('\n')
                {
                    out.push_str(&wrap_prefix);
                    out.push_str(rest);
                    continue;
                }
            }
        } else if let Some(rest) = chunk.strip_prefix(&wrap_prefix) {
            if rest.is_empty()
                || rest.starts_with(' ')
                || rest.starts_with('\t')
                || rest.starts_with('\r')
                || rest.starts_with('\n')
            {
                out.push_str("Exec=/usr/bin/steam");
                out.push_str(rest);
                continue;
            }
        }
        out.push_str(chunk);
    }
    out
}

pub trait HostEnvOps {
    fn path_exists(&self, path: &Path) -> bool;
    fn path_is_file(&self, path: &Path) -> bool;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn read_to_string_nofollow(&self, path: &Path, max_bytes: usize) -> io::Result<Option<String>>;
    fn write_atomic_nofollow(&self, path: &Path, data: &[u8], mode: u32) -> io::Result<()>;
    fn rename_file(&self, src: &Path, dst: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn get_env(&self, key: &str) -> Option<String>;
    fn run_command_quiet(&self, bin: &str, args: &[&str]) -> bool;
    fn command_output_stdout(&self, bin: &str, args: &[&str]) -> Option<String>;
    fn sleep_ms(&self, ms: u64);
    fn trigger_broker_sync_mode(&self) -> bool;
}

pub struct RealHostEnvOps;

pub fn write_atomic_secure_nofollow(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(io::Error::from_raw_os_error(ERRNO_ELOOP));
        }
        Ok(_) | Err(_) => {}
    }
    write_atomic_secure_file(path, data, mode)
}

impl HostEnvOps for RealHostEnvOps {
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn path_is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn read_to_string_nofollow(&self, path: &Path, max_bytes: usize) -> io::Result<Option<String>> {
        let f = match OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW | O_CLOEXEC)
            .open(path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut buf = Vec::new();
        f.take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut buf)?;
        if buf.len() > max_bytes {
            buf.truncate(max_bytes);
        }
        Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
    }

    fn write_atomic_nofollow(&self, path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
        write_atomic_secure_nofollow(path, data, mode)
    }

    fn rename_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        fs::rename(src, dst)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn get_env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn run_command_quiet(&self, bin: &str, args: &[&str]) -> bool {
        Command::new(bin)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn command_output_stdout(&self, bin: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(bin)
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn sleep_ms(&self, ms: u64) {
        thread::sleep(Duration::from_millis(ms));
    }

    fn trigger_broker_sync_mode(&self) -> bool {
        let dsock = resolve_sock_path_common("LASAPI_DEBUG_SOCK", "debug.sock", "/tmp/lasapi-debug.sock");
        let Ok(mut stream) = std::os::unix::net::UnixStream::connect(dsock) else {
            return false;
        };
        let to = Some(Duration::from_millis(500));
        let _ = stream.set_read_timeout(to);
        let _ = stream.set_write_timeout(to);
        if stream.write_all(b"PW_SYNC_MODE\n").is_err() {
            return false;
        }
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut rbuf = [0u8; 512];
        stream.read(&mut rbuf).is_ok_and(|n| n > 0)
    }
}

#[derive(Default)]
pub struct FakeHostEnvOps {
    inner: Mutex<FakeHostEnvState>,
}

#[derive(Default)]
struct FakeHostEnvState {
    files: BTreeMap<PathBuf, Vec<u8>>,
    dirs: Vec<PathBuf>,
    env: BTreeMap<String, String>,
    unit_files: Vec<String>,
    active_units: Vec<String>,
    cmd_log: Vec<(String, Vec<String>)>,
    broker_synced: bool,
    command_stdout_queue: VecDeque<Option<String>>,
}

impl FakeHostEnvOps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed_file(&self, path: impl Into<PathBuf>, data: impl AsRef<[u8]>) {
        self.inner
            .lock()
            .unwrap()
            .files
            .insert(path.into(), data.as_ref().to_vec());
    }

    pub fn read_file_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().files.get(path).cloned()
    }

    pub fn read_file_text(&self, path: &Path) -> Option<String> {
        self.read_file_bytes(path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    pub fn set_env(&self, k: &str, v: &str) {
        self.inner
            .lock()
            .unwrap()
            .env
            .insert(k.to_string(), v.to_string());
    }

    pub fn add_unit_file(&self, unit: &str) {
        self.inner.lock().unwrap().unit_files.push(unit.to_string());
    }

    pub fn set_unit_active(&self, unit: &str, active: bool) {
        let mut st = self.inner.lock().unwrap();
        st.active_units.retain(|u| u != unit);
        if active {
            st.active_units.push(unit.to_string());
        }
    }

    pub fn commands(&self) -> Vec<(String, Vec<String>)> {
        self.inner.lock().unwrap().cmd_log.clone()
    }
}

impl HostEnvOps for FakeHostEnvOps {
    fn path_exists(&self, path: &Path) -> bool {
        let st = self.inner.lock().unwrap();
        st.files.contains_key(path) || st.dirs.iter().any(|d| d == path)
    }

    fn path_is_file(&self, path: &Path) -> bool {
        self.inner.lock().unwrap().files.contains_key(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut st = self.inner.lock().unwrap();
        if !st.dirs.iter().any(|d| d == path) {
            st.dirs.push(path.to_path_buf());
        }
        Ok(())
    }

    fn read_to_string_nofollow(&self, path: &Path, max_bytes: usize) -> io::Result<Option<String>> {
        let st = self.inner.lock().unwrap();
        Ok(st.files.get(path).map(|b| {
            let lim = &b[..b.len().min(max_bytes)];
            String::from_utf8_lossy(lim).into_owned()
        }))
    }

    fn write_atomic_nofollow(&self, path: &Path, data: &[u8], _mode: u32) -> io::Result<()> {
        self.inner
            .lock()
            .unwrap()
            .files
            .insert(path.to_path_buf(), data.to_vec());
        Ok(())
    }

    fn rename_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let mut st = self.inner.lock().unwrap();
        let Some(v) = st.files.remove(src) else {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        };
        st.files.insert(dst.to_path_buf(), v);
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.inner.lock().unwrap().files.remove(path);
        Ok(())
    }

    fn get_env(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap().env.get(key).cloned()
    }

    fn run_command_quiet(&self, bin: &str, args: &[&str]) -> bool {
        let mut st = self.inner.lock().unwrap();
        st.cmd_log.push((
            bin.to_string(),
            args.iter().map(|s| (*s).to_string()).collect(),
        ));
        if bin == "systemctl" && args.len() >= 4 && args[0] == "--user" && args[1] == "is-active" {
            let unit = args[args.len() - 1];
            return st.active_units.iter().any(|u| u == unit);
        }
        true
    }

    fn command_output_stdout(&self, bin: &str, args: &[&str]) -> Option<String> {
        let mut st = self.inner.lock().unwrap();
        st.cmd_log.push((
            bin.to_string(),
            args.iter().map(|s| (*s).to_string()).collect(),
        ));
        if let Some(q) = st.command_stdout_queue.pop_front() {
            return q;
        }
        if bin == "systemctl"
            && args.len() >= 5
            && args[0] == "--user"
            && args[1] == "list-unit-files"
        {
            let unit = args[4];
            if st.unit_files.iter().any(|u| u == unit) {
                return Some(format!("{unit} enabled preset\n"));
            }
            return Some(String::new());
        }
        None
    }

    fn sleep_ms(&self, _ms: u64) {}

    fn trigger_broker_sync_mode(&self) -> bool {
        let mut st = self.inner.lock().unwrap();
        st.broker_synced = true;
        true
    }
}

fn unit_exists_with(ops: &dyn HostEnvOps, unit: &str) -> bool {
    let Some(text) = ops.command_output_stdout(
        "systemctl",
        &["--user", "list-unit-files", "--full", "--no-legend", unit],
    ) else {
        return false;
    };
    if let Some(first_line) = text.lines().next() {
        if let Some(rest) = first_line.strip_prefix(unit) {
            return rest.starts_with(' ') || rest.starts_with('\t');
        }
    }
    false
}

fn record_masked_unit_with(ops: &dyn HostEnvOps, list_path: &Path, unit: &str) {
    let mut existing = ops
        .read_to_string_nofollow(list_path, READ_LIMIT)
        .ok()
        .flatten()
        .unwrap_or_default();
    if existing
        .lines()
        .any(|l| l.trim_end_matches(&['\r', '\n', ' ', '\t'][..]) == unit)
    {
        return;
    }
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(unit);
    existing.push('\n');
    let _ = ops.write_atomic_nofollow(list_path, existing.as_bytes(), 0o600);
}

fn copy_file_nofollow_with(ops: &dyn HostEnvOps, src: &Path, dst: &Path, mode: u32) -> bool {
    let Ok(Some(contents)) = ops.read_to_string_nofollow(src, READ_LIMIT) else {
        return false;
    };
    ops.write_atomic_nofollow(dst, contents.as_bytes(), mode)
        .is_ok()
}

fn rewrite_steam_desktop_exec_with(
    ops: &dyn HostEnvOps,
    src_path: &Path,
    dst_path: &Path,
    wrap_cmd: &str,
    enable: bool,
    mode: u32,
) -> bool {
    let Ok(Some(contents)) = ops.read_to_string_nofollow(src_path, READ_LIMIT) else {
        return false;
    };
    let rewritten = transform_steam_desktop(&contents, wrap_cmd, enable);
    ops.write_atomic_nofollow(dst_path, rewritten.as_bytes(), mode)
        .is_ok()
}

fn install_steam_wrapper_with(ops: &dyn HostEnvOps, paths: &SalsaHostPaths) {
    let _ = ops.create_dir_all(&paths.bin_home);
    let _ = ops.create_dir_all(&paths.applications_dir());
    let _ = ops.create_dir_all(&paths.config_lasapi_dir());

    let recon_so_path = paths.pa_reconnect_so().to_string_lossy().into_owned();
    let Ok(recon_quoted) = sh_single_quote(&recon_so_path) else {
        return;
    };

    let wrap_path = paths.steam_wrapper_bin();
    let wrap_str = wrap_path.to_string_lossy().into_owned();
    let script = format!(
        "#!/bin/sh\n\
         # Salsa PulseAudio Reconnect wrapper for Steam (propagates inside pressure-vessel)\n\
         # Revert anytime with: lasapi-ctl disable-auto\n\
         RECON_SO={recon_quoted}\n\
         if [ -r \"$RECON_SO\" ]; then\n    \
             case \":${{LD_PRELOAD:-}}:\" in\n        \
                 *\":$RECON_SO:\"*) ;;\n        \
                 *) export LD_PRELOAD=\"${{RECON_SO}}${{LD_PRELOAD:+:$LD_PRELOAD}}\" ;;\n    \
             esac\n\
         fi\n\
         if [ -x /usr/bin/steam ]; then\n    \
             exec /usr/bin/steam \"$@\"\n\
         fi\n\
         exec steam \"$@\"\n"
    );
    if ops
        .write_atomic_nofollow(&wrap_path, script.as_bytes(), 0o700)
        .is_err()
    {
        return;
    }

    let app_desk = paths.steam_app_desktop();
    let app_bak = paths.steam_app_desktop_backup();
    let sys_desk = Path::new("/usr/share/applications/steam.desktop");
    if ops.path_exists(&app_desk)
        && !ops
            .read_to_string_nofollow(&app_desk, READ_LIMIT)
            .ok()
            .flatten()
            .is_some_and(|s| s.contains("salsa-steam-launch"))
        && !ops.path_exists(&app_bak)
    {
        copy_file_nofollow_with(ops, &app_desk, &app_bak, 0o600);
    }
    let app_src = if ops.path_exists(&app_bak) {
        app_bak.as_path()
    } else {
        sys_desk
    };
    if ops.path_exists(app_src)
        && rewrite_steam_desktop_exec_with(ops, app_src, &app_desk, &wrap_str, true, 0o644)
    {
        let apps_dir = paths.applications_dir();
        let _ = ops.run_command_quiet("update-desktop-database", &[&apps_dir.to_string_lossy()]);
    }

    let auto_desk = paths.steam_autostart_desktop();
    let auto_bak = paths.steam_autostart_desktop_backup();
    if ops.path_exists(&auto_desk) {
        if !ops
            .read_to_string_nofollow(&auto_desk, READ_LIMIT)
            .ok()
            .flatten()
            .is_some_and(|s| s.contains("salsa-steam-launch"))
            && !ops.path_exists(&auto_bak)
        {
            copy_file_nofollow_with(ops, &auto_desk, &auto_bak, 0o600);
        }
        let auto_src = if ops.path_exists(&auto_bak) {
            auto_bak.as_path()
        } else {
            auto_desk.as_path()
        };
        rewrite_steam_desktop_exec_with(ops, auto_src, &auto_desk, &wrap_str, true, 0o644);
    }
}

fn remove_steam_wrapper_with(ops: &dyn HostEnvOps, paths: &SalsaHostPaths) {
    let wrap_path = paths.steam_wrapper_bin();
    let wrap_str = wrap_path.to_string_lossy().into_owned();
    let _ = ops.remove_file(&wrap_path);

    let apps_dir = paths.applications_dir();
    let app_desk = paths.steam_app_desktop();
    let app_bak = paths.steam_app_desktop_backup();
    if ops.path_exists(&app_bak) {
        if copy_file_nofollow_with(ops, &app_bak, &app_desk, 0o644) {
            let _ = ops.remove_file(&app_bak);
        }
        let _ = ops.run_command_quiet("update-desktop-database", &[&apps_dir.to_string_lossy()]);
    } else if ops
        .read_to_string_nofollow(&app_desk, READ_LIMIT)
        .ok()
        .flatten()
        .is_some_and(|s| s.contains("salsa-steam-launch"))
    {
        let _ = ops.remove_file(&app_desk);
        let _ = ops.run_command_quiet("update-desktop-database", &[&apps_dir.to_string_lossy()]);
    }

    let auto_desk = paths.steam_autostart_desktop();
    let auto_bak = paths.steam_autostart_desktop_backup();
    if ops.path_exists(&auto_bak) {
        if copy_file_nofollow_with(ops, &auto_bak, &auto_desk, 0o644) {
            let _ = ops.remove_file(&auto_bak);
        }
    } else if ops
        .read_to_string_nofollow(&auto_desk, READ_LIMIT)
        .ok()
        .flatten()
        .is_some_and(|s| s.contains("salsa-steam-launch"))
    {
        rewrite_steam_desktop_exec_with(ops, &auto_desk, &auto_desk, &wrap_str, false, 0o644);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CutoverSummary {
    pub asoundrc: PathBuf,
    pub alsoft_conf: PathBuf,
    pub zero_flag: PathBuf,
    pub direct_env_file: PathBuf,
    pub steam_wrapper: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreSummary {
    pub restored_asoundrc_from_backup: bool,
    pub asoundrc: PathBuf,
}

pub fn apply_zero_pipewire_cutover(
    ops: &dyn HostEnvOps,
    paths: &SalsaHostPaths,
) -> Result<CutoverSummary, String> {
    let asoundrc = paths.asoundrc();
    let backup = paths.asoundrc_backup();
    if ops.path_exists(&asoundrc) && !ops.path_exists(&backup) {
        let _ = ops.rename_file(&asoundrc, &backup);
    }

    let asoundrc_body = render_asoundrc(&paths.alsa_plugin_so())?;
    ops.write_atomic_nofollow(&asoundrc, asoundrc_body.as_bytes(), 0o644)
        .map_err(|e| format!("write {}: {e}", asoundrc.display()))?;

    let oal_conf = paths.alsoft_conf();
    let oal_bak = paths.alsoft_conf_backup();
    if let Some(p) = oal_conf.parent() {
        let _ = ops.create_dir_all(p);
    }
    if ops.path_exists(&oal_conf) && !ops.path_exists(&oal_bak) {
        let _ = ops.rename_file(&oal_conf, &oal_bak);
    }
    let _ = ops.write_atomic_nofollow(&oal_conf, render_alsoft_conf().as_bytes(), 0o644);

    let wp_dir = paths.config_home.join("wireplumber/wireplumber.conf.d");
    let wp_lua_main = paths.config_home.join("wireplumber/main.lua.d");
    let wp_lua_bt = paths.config_home.join("wireplumber/bluetooth.lua.d");
    let pw_conf_dir = paths.config_home.join("pipewire/pipewire.conf.d");
    let bak_dir = paths.pw_backup_dir();
    let _ = ops.create_dir_all(&bak_dir);

    for legacy in [
        pw_conf_dir.join("99-lasapi-bridge.conf"),
        wp_dir.join("99-lasapi-video-only.conf"),
        wp_lua_main.join("99-lasapi-no-audio.lua"),
        wp_lua_bt.join("99-lasapi-no-bluez.lua"),
    ] {
        let _ = ops.remove_file(&legacy);
    }

    for (src, dst) in [
        (
            pw_conf_dir.join("10-bose-persistent-bridge.conf"),
            bak_dir.join("10-bose-persistent-bridge.conf"),
        ),
        (
            wp_dir.join("51-bluetooth-autoswitch.conf"),
            bak_dir.join("51-bluetooth-autoswitch.conf"),
        ),
        (
            wp_dir.join("52-bose-keepalive.conf"),
            bak_dir.join("52-bose-keepalive.conf"),
        ),
    ] {
        if ops.path_exists(&src) {
            let _ = ops.rename_file(&src, &dst);
        }
    }

    let zero_flag = paths.pipewire_zero_flag();
    let _ = ops.write_atomic_nofollow(&zero_flag, b"enabled\n", 0o644);

    let pa_dir = if ops.path_exists(&paths.pa_reconnect32_dir().join("libpulse.so.0")) {
        format!(
            "{}:{}",
            paths.pa_reconnect_dir().display(),
            paths.pa_reconnect32_dir().display()
        )
    } else {
        paths.pa_reconnect_dir().display().to_string()
    };
    let cur_ld = ops.get_env("LD_LIBRARY_PATH");
    let ld_val = build_filtered_ld_path(Some(&pa_dir), cur_ld.as_deref());
    let ld_env = format!("LD_LIBRARY_PATH={ld_val}");

    let env_file = paths.direct_env_conf();
    if let Some(p) = env_file.parent() {
        let _ = ops.create_dir_all(p);
    }
    let env_body = render_direct_env_conf(&ld_val)?;
    let _ = ops.write_atomic_nofollow(&env_file, env_body.as_bytes(), 0o644);

    let proton_kv = format!("PROTON_USE_PIPEWIRE={CANONICAL_PROTON_USE_PIPEWIRE}");
    let sdl_kv = format!("SDL_AUDIODRIVER={CANONICAL_SDL_AUDIODRIVER}");
    let alsoft_kv = format!("ALSOFT_DRIVERS={CANONICAL_ALSOFT_DRIVERS}");

    let _ = ops.run_command_quiet(
        "systemctl",
        &["--user", "set-environment", &proton_kv, &sdl_kv, &alsoft_kv, &ld_env],
    );
    let _ = ops.run_command_quiet(
        "dbus-update-activation-environment",
        &["--systemd", &proton_kv, &sdl_kv, &alsoft_kv, &ld_env],
    );

    install_steam_wrapper_with(ops, paths);

    let units_list = paths.masked_units_list();
    for unit in MASKED_PIPEWIRE_UNITS {
        if unit_exists_with(ops, unit) {
            record_masked_unit_with(ops, &units_list, unit);
            let _ = ops.run_command_quiet("systemctl", &["--user", "stop", unit]);
            let _ = ops.run_command_quiet("systemctl", &["--user", "mask", unit]);
        }
    }

    let has_pw_socket_unit = unit_exists_with(ops, "lasapi-pipewire.socket");
    let pw_socket_was_active = has_pw_socket_unit
        && ops.run_command_quiet("systemctl", &["--user", "is-active", "--quiet", "lasapi-pipewire.socket"]);
    let audiodg_was_active =
        ops.run_command_quiet("systemctl", &["--user", "is-active", "--quiet", "lasapi-audiodg.service"]);

    if has_pw_socket_unit && !pw_socket_was_active {
        if audiodg_was_active {
            let _ = ops.run_command_quiet("systemctl", &["--user", "stop", "lasapi-audiodg.service"]);
        }
        let _ = ops.run_command_quiet(
            "systemctl",
            &["--user", "enable", "--now", "lasapi-pipewire.socket"],
        );
        if audiodg_was_active {
            let _ = ops.run_command_quiet("systemctl", &["--user", "start", "lasapi-audiodg.service"]);
        }
    }

    if audiodg_was_active {
        if !(has_pw_socket_unit && !pw_socket_was_active) && !ops.trigger_broker_sync_mode() {
            let _ = ops.run_command_quiet(
                "systemctl",
                &["--user", "try-restart", "lasapi-audiodg.service"],
            );
        }
        let pw_sock = paths.pipewire_socket_path();
        for _ in 0..40 {
            if ops.path_exists(&pw_sock) {
                break;
            }
            ops.sleep_ms(50);
        }
        let mut portal_args = vec!["--user", "try-restart"];
        portal_args.extend(PORTAL_USER_UNITS);
        let _ = ops.run_command_quiet("systemctl", &portal_args);
    }

    Ok(CutoverSummary {
        asoundrc,
        alsoft_conf: oal_conf,
        zero_flag,
        direct_env_file: env_file,
        steam_wrapper: paths.steam_wrapper_bin(),
    })
}

pub fn restore_stock_pipewire(
    ops: &dyn HostEnvOps,
    paths: &SalsaHostPaths,
) -> Result<RestoreSummary, String> {
    let asoundrc = paths.asoundrc();
    let backup = paths.asoundrc_backup();
    let _ = ops.remove_file(&asoundrc);
    let restored_asoundrc_from_backup = if ops.path_exists(&backup) {
        let _ = ops.rename_file(&backup, &asoundrc);
        true
    } else {
        false
    };

    let oal_conf = paths.alsoft_conf();
    let oal_bak = paths.alsoft_conf_backup();
    let _ = ops.remove_file(&oal_conf);
    if ops.path_exists(&oal_bak) {
        let _ = ops.rename_file(&oal_bak, &oal_conf);
    }

    let wp_dir = paths.config_home.join("wireplumber/wireplumber.conf.d");
    let wp_lua_main = paths.config_home.join("wireplumber/main.lua.d");
    let wp_lua_bt = paths.config_home.join("wireplumber/bluetooth.lua.d");
    let pw_dir = paths.config_home.join("pipewire/pipewire.conf.d");
    let bak_dir = paths.pw_backup_dir();

    for legacy in [
        pw_dir.join("99-lasapi-bridge.conf"),
        wp_dir.join("99-lasapi-video-only.conf"),
        wp_lua_main.join("99-lasapi-no-audio.lua"),
        wp_lua_bt.join("99-lasapi-no-bluez.lua"),
        paths.pipewire_zero_flag(),
        paths.direct_env_conf(),
    ] {
        let _ = ops.remove_file(&legacy);
    }

    remove_steam_wrapper_with(ops, paths);

    let cur_ld = ops.get_env("LD_LIBRARY_PATH");
    let ld_val = build_filtered_ld_path(None, cur_ld.as_deref());
    if ld_val.is_empty() {
        let _ = ops.run_command_quiet(
            "systemctl",
            &[
                "--user",
                "unset-environment",
                "PROTON_USE_PIPEWIRE",
                "SDL_AUDIODRIVER",
                "ALSOFT_DRIVERS",
                "LD_LIBRARY_PATH",
            ],
        );
    } else {
        let ld_env = format!("LD_LIBRARY_PATH={ld_val}");
        let _ = ops.run_command_quiet(
            "systemctl",
            &[
                "--user",
                "unset-environment",
                "PROTON_USE_PIPEWIRE",
                "SDL_AUDIODRIVER",
                "ALSOFT_DRIVERS",
            ],
        );
        let _ = ops.run_command_quiet("systemctl", &["--user", "set-environment", &ld_env]);
        let _ = ops.run_command_quiet("dbus-update-activation-environment", &["--systemd", &ld_env]);
    }

    for (src, dst, parent_dir) in [
        (
            bak_dir.join("10-bose-persistent-bridge.conf"),
            pw_dir.join("10-bose-persistent-bridge.conf"),
            &pw_dir,
        ),
        (
            bak_dir.join("51-bluetooth-autoswitch.conf"),
            wp_dir.join("51-bluetooth-autoswitch.conf"),
            &wp_dir,
        ),
        (
            bak_dir.join("52-bose-keepalive.conf"),
            wp_dir.join("52-bose-keepalive.conf"),
            &wp_dir,
        ),
    ] {
        if ops.path_exists(&src) {
            let _ = ops.create_dir_all(parent_dir);
            let _ = ops.rename_file(&src, &dst);
        }
    }

    let _ = ops.trigger_broker_sync_mode();
    let _ = ops.run_command_quiet("systemctl", &["--user", "stop", "lasapi-audiodg.service"]);
    let _ = ops.run_command_quiet(
        "systemctl",
        &["--user", "disable", "--now", "lasapi-pipewire.socket"],
    );

    let units_list = paths.masked_units_list();
    if let Ok(Some(text)) = ops.read_to_string_nofollow(&units_list, READ_LIMIT) {
        for line in text.lines() {
            let unit = line.trim_end_matches(&['\r', '\n', ' ', '\t'][..]);
            if !unit.is_empty() {
                let _ = ops.run_command_quiet("systemctl", &["--user", "unmask", unit]);
                let _ = ops.run_command_quiet("systemctl", &["--user", "enable", "--now", unit]);
            }
        }
        let _ = ops.remove_file(&units_list);
    }

    for unit in RESTORE_PIPEWIRE_UNITS {
        if unit_exists_with(ops, unit) {
            let _ = ops.run_command_quiet("systemctl", &["--user", "unmask", unit]);
            let _ = ops.run_command_quiet("systemctl", &["--user", "enable", "--now", unit]);
        }
    }

    let _ = ops.run_command_quiet(
        "systemctl",
        &[
            "--user",
            "reset-failed",
            "pipewire.service",
            "pipewire.socket",
            "wireplumber.service",
            "pipewire-pulse.service",
            "pipewire-pulse.socket",
        ],
    );
    if unit_exists_with(ops, "pipewire.socket") {
        let _ = ops.run_command_quiet("systemctl", &["--user", "start", "pipewire.socket"]);
    }
    if unit_exists_with(ops, "pipewire.service") {
        let _ = ops.run_command_quiet("systemctl", &["--user", "restart", "pipewire.service"]);
    }
    if unit_exists_with(ops, "wireplumber.service")
        && !ops.run_command_quiet("systemctl", &["--user", "is-active", "--quiet", "wireplumber.service"])
    {
        let _ = ops.run_command_quiet("systemctl", &["--user", "start", "wireplumber.service"]);
    }

    let mut portal_args = vec!["--user", "try-restart"];
    portal_args.extend(PORTAL_USER_UNITS);
    let _ = ops.run_command_quiet("systemctl", &portal_args);

    Ok(RestoreSummary {
        restored_asoundrc_from_backup,
        asoundrc,
    })
}

pub fn pipewire_zero_enabled(ops: &dyn HostEnvOps, paths: &SalsaHostPaths) -> bool {
    ops.read_to_string_nofollow(&paths.pipewire_zero_flag(), 64)
        .ok()
        .flatten()
        .is_some_and(|s| matches!(s.trim(), "enabled" | "1"))
}

pub fn is_direct_cutover_active(ops: &dyn HostEnvOps, paths: &SalsaHostPaths) -> bool {
    pipewire_zero_enabled(ops, paths) && ops.path_is_file(&paths.direct_env_conf())
}

pub fn pipewire_zero_drifted(ops: &dyn HostEnvOps, paths: &SalsaHostPaths) -> bool {
    if !pipewire_zero_enabled(ops, paths) {
        return false;
    }
    for unit in DRIFT_WATCHED_PIPEWIRE_UNITS {
        if ops.run_command_quiet("systemctl", &["--user", "is-active", "--quiet", unit]) {
            return true;
        }
    }
    ops.path_is_file(&paths.systemd_user_dir().join("lasapi-pipewire.socket"))
        && ops.run_command_quiet("systemctl", &["--user", "is-active", "--quiet", "lasapi-audiodg.service"])
        && !ops.run_command_quiet("systemctl", &["--user", "is-active", "--quiet", "lasapi-pipewire.socket"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths(base: &Path) -> SalsaHostPaths {
        SalsaHostPaths::resolve(
            base.join("home"),
            Some(&base.join("home/.config").to_string_lossy()),
            None,
            Some(&base.join("run").to_string_lossy()),
            1000,
            base.join("install root"),
        )
    }

    #[test]
    fn hostile_paths_are_rejected_or_safely_quoted() {
        assert!(quote_alsa_conf_string("/tmp/ok \"path\"\\x").is_ok());
        assert_eq!(
            quote_alsa_conf_string("/opt/a \"b\"\\c").unwrap(),
            "\"/opt/a \\\"b\\\"\\\\c\""
        );
        assert!(quote_alsa_conf_string("/bad\ninjection").is_err());
        assert!(quote_alsa_conf_string("/bad\0nul").is_err());

        assert!(quote_systemd_env_value("/usr/lib:/opt/salsa/build").is_ok());
        assert_eq!(
            quote_systemd_env_value("/opt/my \"salsa\"/$dir").unwrap(),
            "\"/opt/my \\\"salsa\\\"/\\$dir\""
        );
        assert!(quote_systemd_env_value("FOO\nBAR=baz").is_err());

        let with_quotes = Path::new("/home/alice/my \"audio\"/build/libasound.so");
        let rendered = render_asoundrc(with_quotes).unwrap();
        assert!(rendered.contains("lib \"/home/alice/my \\\"audio\\\"/build/libasound.so\""));

        assert!(render_direct_env_conf("/a/b\nLD_PRELOAD=/evil.so").is_err());
    }

    #[test]
    fn write_atomic_nofollow_rejects_symlinks_with_eloop() {
        let dir = std::env::temp_dir().join(format!(
            "salsa_prov_sym_{}_{}",
            std::process::id(),
            posix_owner_id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let target = dir.join("real.txt");
        fs::write(&target, b"orig").unwrap();
        let link = dir.join("sym.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = write_atomic_secure_nofollow(&link, b"evil", 0o600).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(ERRNO_ELOOP));
        assert_eq!(fs::read(&target).unwrap(), b"orig");

        let clean = dir.join("clean.txt");
        write_atomic_secure_nofollow(&clean, b"hello", 0o640).unwrap();
        assert_eq!(fs::read(&clean).unwrap(), b"hello");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cutover_and_restore_roundtrip_on_fake_host() {
        let base = PathBuf::from("/fake_host");
        let paths = test_paths(&base);
        let ops = FakeHostEnvOps::new();
        ops.seed_file(paths.asoundrc(), b"# user custom asoundrc\n");
        ops.seed_file(paths.alsoft_conf(), b"[general]\ndrivers = pipewire\n");
        ops.seed_file(
            paths.config_home.join("pipewire/pipewire.conf.d/10-bose-persistent-bridge.conf"),
            b"# old bridge\n",
        );
        ops.seed_file("/usr/share/applications/steam.desktop", "[Desktop Entry]\nExec=/usr/bin/steam %U\n");
        ops.set_env("LD_LIBRARY_PATH", "/opt/stale/build/pa_reconnect:/usr/local/lib");
        for u in MASKED_PIPEWIRE_UNITS {
            ops.add_unit_file(u);
        }
        ops.add_unit_file("lasapi-pipewire.socket");
        ops.set_unit_active("lasapi-audiodg.service", true);
        ops.seed_file(paths.pipewire_socket_path(), b"");

        let summary = apply_zero_pipewire_cutover(&ops, &paths).unwrap();
        assert_eq!(summary.direct_env_file, paths.direct_env_conf());
        assert!(is_direct_cutover_active(&ops, &paths));
        assert_eq!(
            ops.read_file_text(&paths.asoundrc_backup()).as_deref(),
            Some("# user custom asoundrc\n")
        );
        let env_txt = ops.read_file_text(&paths.direct_env_conf()).unwrap();
        assert!(env_txt.contains("ALSOFT_DRIVERS=pulse,alsa\n"));
        assert!(!env_txt.contains("/opt/stale/build/pa_reconnect"));
        assert!(!pipewire_zero_drifted(&ops, &paths));

        // Simulate PipeWire waking up (drift) and verify detector catches it
        ops.set_unit_active("pipewire.service", true);
        assert!(pipewire_zero_drifted(&ops, &paths));
        ops.set_unit_active("pipewire.service", false);

        let rsum = restore_stock_pipewire(&ops, &paths).unwrap();
        assert!(rsum.restored_asoundrc_from_backup);
        assert_eq!(
            ops.read_file_text(&paths.asoundrc()).as_deref(),
            Some("# user custom asoundrc\n")
        );
        assert!(!ops.path_exists(&paths.pipewire_zero_flag()));
        assert!(!ops.path_exists(&paths.direct_env_conf()));
        assert!(!ops.path_exists(&paths.steam_wrapper_bin()));
    }

    #[test]
    fn host_paths_and_shim_env_match_canonical_policy() {
        let base = PathBuf::from("/tmp/salsa_test_home");
        let paths = SalsaHostPaths::resolve(
            base.clone(),
            Some("/tmp/salsa_cfg"),
            Some("/tmp/salsa_data"),
            Some("/run/user/1234"),
            1234,
            PathBuf::from("/opt/salsa"),
        );
        assert_eq!(
            paths.direct_env_conf(),
            PathBuf::from("/tmp/salsa_cfg/environment.d/99-salsa-direct.conf")
        );
        assert_eq!(
            paths.video_only_pw_conf(),
            PathBuf::from("/tmp/salsa_cfg/pipewire/pipewire.conf.d/99-lasapi-video-only.conf")
        );

        let envs = build_shim_launch_env(&paths, Some("/custom/lib"));
        assert_eq!(envs[0], ("PROTON_USE_PIPEWIRE", "0".to_string()));
        assert_eq!(envs[1], ("SDL_AUDIODRIVER", "pulse".to_string()));
        assert_eq!(envs[2], ("ALSOFT_DRIVERS", "pulse,alsa".to_string()));
        assert_eq!(
            envs[3],
            (
                "LD_LIBRARY_PATH",
                "/opt/salsa/build/shims:/opt/salsa/build:/custom/lib".to_string()
            )
        );
    }

    #[test]
    fn filtered_ld_path_and_steam_rewrite_work() {
        let cur = "/usr/lib:/opt/lasapi/build/pa_reconnect:/foo:/opt/lasapi/build/pa_reconnect32:/bar";
        let res = build_filtered_ld_path(
            Some("/new/build/pa_reconnect:/new/build/pa_reconnect32"),
            Some(cur),
        );
        assert_eq!(
            res,
            "/new/build/pa_reconnect:/new/build/pa_reconnect32:/usr/lib:/foo:/bar"
        );
        assert_eq!(build_filtered_ld_path(None, Some(cur)), "/usr/lib:/foo:/bar");

        let orig = "[Desktop Entry]\nName=Steam\nExec=/usr/bin/steam %U\n";
        let wrap = "/home/user/.local/bin/salsa-steam-launch";
        let wrapped = transform_steam_desktop(orig, wrap, true);
        assert!(wrapped.contains(&format!("Exec={wrap} %U\n")));
        let restored = transform_steam_desktop(&wrapped, wrap, false);
        assert_eq!(restored, orig);
    }
}
