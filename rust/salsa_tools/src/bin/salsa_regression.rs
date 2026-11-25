// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::mem;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use salsa_tools::debug_core::{
    cstr_to_string, snapshot_once, DryRunDesktopCapturer, SalsaDebugClient, SystemAuditScanner,
};
use salsa_tools::host::{
    ensure_dir_0700, find_cargo, find_host_lib, getpid, getuid, install_root,
    peercred_for_unix_socket, private_runtime_dir, state_dir, which, write_atomic_secure,
    xdg_runtime_dir,
};
use salsa_tools::ipc::{
    get_engine_status, gui_settings_path, load_gui_settings, load_screencast_prefs,
    normalize_vol_step_pct, save_gui_settings, set_bt_codec_config, set_capture_sample_rate,
    set_engine_config, set_hw_analog, set_mic_apo, set_parametric_eq, set_session_control,
    LasapiEngineStatus, LASAPI_BT_PREF_LDAC, LASAPI_MAX_SESSIONS,
};
use salsa_tools::json::JsonValue;
use salsa_tools::zlib_png::{decode_png_rgba, encode_png_rgba, resize_rgba, RgbaImage};
use salsa_wire::hw_analog::{lasapi_hw_analog_default_cfg, lasapi_hw_analog_score_card};
use salsa_wire::unix_sys::reset_sigpipe_default;

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Result<Self, String> {
        let p = private_runtime_dir().join(format!("{prefix}_{}", getpid()));
        let _ = fs::remove_dir_all(&p);
        ensure_dir_0700(&p).map_err(|e| format!("mkdir {}: {e}", p.display()))?;
        Ok(Self { path: p })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        self.child
            .take()
            .expect("child present")
            .wait_with_output()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(ref mut c) = self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, val: &Path) -> Self {
        let prev = env::var(key).ok();
        env::set_var(key, val);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(ref v) = self.prev {
            env::set_var(self.key, v);
        } else {
            env::remove_var(self.key);
        }
    }
}

struct AppVolBackupGuard {
    path: PathBuf,
    backup: Option<Vec<u8>>,
}

impl Drop for AppVolBackupGuard {
    fn drop(&mut self) {
        if let Some(ref bytes) = self.backup {
            let _ = write_atomic_secure(&self.path, bytes, 0o600);
            if is_audiodg_active() {
                let _ = Command::new("systemctl")
                    .args(["--user", "restart", "lasapi-audiodg.service"])
                    .status();
            }
        } else if self.path.is_file() {
            if let Ok(txt) = fs::read_to_string(&self.path) {
                if txt.contains("SalsaReg") || txt.contains("salsa_regtest_bin") {
                    let kept: Vec<&str> = txt
                        .lines()
                        .filter(|l| !l.contains("SalsaReg") && !l.contains("salsa_regtest_bin"))
                        .collect();
                    if kept.is_empty() {
                        let _ = fs::remove_file(&self.path);
                    } else {
                        let body = format!("{}\n", kept.join("\n"));
                        let _ = write_atomic_secure(&self.path, body.as_bytes(), 0o600);
                    }
                    if is_audiodg_active() {
                        let _ = Command::new("systemctl")
                            .args(["--user", "restart", "lasapi-audiodg.service"])
                            .status();
                    }
                }
            }
        }
    }
}

struct StudioStateRestoreGuard {
    orig: LasapiEngineStatus,
    orig_cap_pref: u32,
}

impl Drop for StudioStateRestoreGuard {
    fn drop(&mut self) {
        let _ = set_hw_analog(&self.orig.hw_analog_cfg);
        let _ = set_parametric_eq(&self.orig.eq_cfg, &self.orig.param_eq);
        let _ = set_mic_apo(&self.orig.mic_apo);
        let _ = set_bt_codec_config(&self.orig.bt_codec_cfg);
        let _ = set_engine_config(
            self.orig.hw_sample_rate,
            self.orig.hw_period_frames,
            self.orig.hw_format_bits,
        );
        let _ = set_capture_sample_rate(self.orig_cap_pref);
    }
}

fn is_audiodg_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "lasapi-audiodg.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn dynamic_pa_symbols(so_path: &Path) -> Result<BTreeSet<String>, String> {
    let out = Command::new("nm")
        .args(["-D", "--defined-only", &so_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("nm {}: {e}", so_path.display()))?;
    if !out.status.success() {
        return Err(format!("nm failed on {}", so_path.display()));
    }
    let mut set = BTreeSet::new();
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(sym_full) = line.split_whitespace().last() {
            let base = sym_full.split('@').next().unwrap_or(sym_full);
            if base.starts_with("pa_") {
                set.insert(base.to_string());
            }
        }
    }
    Ok(set)
}

fn readelf_dynamic_text(so_path: &Path) -> Result<String, String> {
    let out = Command::new("readelf")
        .args(["-d", "-W", &so_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("readelf -d {}: {e}", so_path.display()))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn test_c_unit_suite(base_dir: &Path) -> Result<String, String> {
    if is_audiodg_active() {
        let ctl_bin = base_dir.join("build/lasapi-ctl");
        for _ in 0..30 {
            if Command::new(&ctl_bin)
                .arg("status")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    let test_bin = base_dir.join("build/lasapi-test");
    if !test_bin.is_file() {
        return Err(format!("Missing {}", test_bin.display()));
    }
    let out = Command::new(&test_bin)
        .output()
        .map_err(|e| format!("exec lasapi-test: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "lasapi-test failed (rc={:?}):\n{}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok("build/lasapi-test passed".to_string())
}

fn test_tools_and_settings_security(base_dir: &Path) -> Result<String, String> {
    for retired in [
        "lasapi-tray.py",
        "salsa-debug.py",
        "salsa_debug_core.py",
        "salsa_debug_ui.py",
        "salsa_ipc.py",
        "salsa-gui.py",
        "salsa_theme_editor.py",
    ] {
        let rp = base_dir.join(retired);
        if rp.exists() {
            return Err(format!("Retired Python file still present: {}", rp.display()));
        }
    }

    for self_test_bin in [
        "build/salsa-gen-pa-proxy",
        "build/lasapi-tray",
        "build/salsa-debug",
        "build/salsa-studio",
    ] {
        let bin_path = base_dir.join(self_test_bin);
        if !bin_path.is_file() {
            return Err(format!("Missing {}", bin_path.display()));
        }
        let st_out = Command::new(&bin_path)
            .arg("--self-test")
            .output()
            .map_err(|e| format!("{self_test_bin} --self-test: {e}"))?;
        if !st_out.status.success() {
            return Err(format!(
                "{self_test_bin} --self-test failed: {}",
                String::from_utf8_lossy(&st_out.stderr)
            ));
        }
    }

    let tmp = TempDirGuard::new("salsa_reg_sec")?;
    let cfg_dir = tmp.path.join("cfg");
    {
        let _guard = EnvVarGuard::set("XDG_CONFIG_HOME", &cfg_dir);
        let payload = JsonValue::obj(vec![
            ("eq_per_device", JsonValue::Bool(true)),
            (
                "eq_device_profiles",
                JsonValue::obj(vec![(
                    "dev0",
                    JsonValue::obj(vec![("enabled", JsonValue::Bool(true))]),
                )]),
            ),
        ]);
        save_gui_settings(&payload)?;
        let sp = gui_settings_path();
        let parent = sp.parent().ok_or("missing gui_settings parent")?;
        let dmode = fs::metadata(parent)
            .map_err(|e| format!("stat dir: {e}"))?
            .permissions()
            .mode()
            & 0o777;
        let fmode = fs::metadata(&sp)
            .map_err(|e| format!("stat file: {e}"))?
            .permissions()
            .mode()
            & 0o777;
        if dmode != 0o700 || fmode != 0o600 {
            return Err(format!(
                "expected 0700 dir / 0600 file, got 0o{dmode:o} / 0o{fmode:o}"
            ));
        }
        let loaded = load_gui_settings();
        if loaded.get("eq_per_device").and_then(JsonValue::as_bool) != Some(true) {
            return Err("load_gui_settings eq_per_device mismatch".to_string());
        }
        fs::remove_file(&sp).map_err(|e| format!("unlink settings: {e}"))?;
        symlink("/etc/passwd", &sp).map_err(|e| format!("symlink test: {e}"))?;
        let sym_loaded = load_gui_settings();
        if !sym_loaded
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(false)
        {
            return Err("symlinked salsa_settings.json was not rejected cleanly".to_string());
        }
    }

    if normalize_vol_step_pct("5") != 5
        || normalize_vol_step_pct("10") != 10
        || normalize_vol_step_pct("99") != 5
    {
        return Err("normalize_vol_step_pct mismatch".to_string());
    }
    let prefs = load_screencast_prefs();
    if prefs.res_key.is_empty() {
        return Err("load_screencast_prefs returned empty res_key".to_string());
    }

    let run_out = Command::new(base_dir.join("build/lasapi-run"))
        .args([
            "sh",
            "-c",
            "printf '%s|%s|%s' \"$SDL_AUDIODRIVER\" \"$PROTON_USE_PIPEWIRE\" \"$LD_LIBRARY_PATH\"",
        ])
        .output()
        .map_err(|e| format!("lasapi-run: {e}"))?;
    let run_str = String::from_utf8_lossy(&run_out.stdout);
    if !run_str.starts_with("pulse|0|") || !run_str.contains("build/shims:") {
        return Err(format!("lasapi-run env unexpected: {run_str}"));
    }

    let fp_out = Command::new(base_dir.join("build/lasapi-autosync"))
        .arg("--print-fingerprint")
        .output()
        .map_err(|e| format!("lasapi-autosync --print-fingerprint: {e}"))?;
    if !fp_out.status.success() || fp_out.stdout.is_empty() {
        return Err("lasapi-autosync --print-fingerprint failed".to_string());
    }

    let dist_zip = tmp.path.join("dist_test.zip");
    let dist_out = Command::new(base_dir.join("build/salsa-dist"))
        .args(["--out", &dist_zip.to_string_lossy()])
        .output()
        .map_err(|e| format!("salsa-dist: {e}"))?;
    if !dist_out.status.success() {
        return Err("salsa-dist failed".to_string());
    }
    if which("unzip").is_some()
        && !Command::new("unzip")
            .args(["-t", &dist_zip.to_string_lossy()])
            .stdout(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    {
        return Err("salsa-dist archive failed unzip -t".to_string());
    }

    for subcmd in ["--audit", "--probe"] {
        let cli_out = Command::new(base_dir.join("build/salsa-debug-cli"))
            .args([subcmd, "--json"])
            .output()
            .map_err(|e| format!("salsa-debug-cli {subcmd}: {e}"))?;
        if !cli_out.status.success() {
            return Err(format!("salsa-debug-cli {subcmd} --json failed"));
        }
        JsonValue::parse(&String::from_utf8_lossy(&cli_out.stdout))
            .map_err(|e| format!("salsa-debug-cli {subcmd} invalid JSON: {e}"))?;
    }

    let exp_dir = tmp.path.join("exports");
    let exp_out = Command::new(base_dir.join("build/salsa-debug-cli"))
        .args(["--export", &exp_dir.to_string_lossy()])
        .output()
        .map_err(|e| format!("salsa-debug-cli --export: {e}"))?;
    if !exp_out.status.success() {
        return Err("salsa-debug-cli --export failed".to_string());
    }
    let exp_zip = fs::read_dir(&exp_dir)
        .map_err(|e| format!("read export dir: {e}"))?
        .flatten()
        .find(|e| e.path().extension().and_then(|s| s.to_str()) == Some("zip"))
        .map(|e| e.path())
        .ok_or("salsa-debug-cli --export created no zip")?;
    if which("unzip").is_some()
        && !Command::new("unzip")
            .args(["-t", &exp_zip.to_string_lossy()])
            .stdout(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    {
        return Err("export zip failed unzip -t".to_string());
    }

    let txt_report = tmp.path.join("debug_report.txt");
    let txt_out = Command::new(base_dir.join("build/salsa-debug-cli"))
        .args(["--export-text", &txt_report.to_string_lossy()])
        .output()
        .map_err(|e| format!("salsa-debug-cli --export-text: {e}"))?;
    if !txt_out.status.success() {
        return Err("salsa-debug-cli --export-text failed".to_string());
    }
    let txt_body = fs::read_to_string(&txt_report)
        .map_err(|e| format!("read exported text report: {e}"))?;
    for needle in [
        "[1] HOST, ENVIRONMENT & BUILD FRESHNESS",
        "[2] ENGINE, DSP, RT TIMING & FAILSAFE",
        "[4] PIPEWIRE SCREENCAST BROKER, SHM & DISPLAY REFRESH",
        "[6] HOST AUDIT: SOCKETS, SYSTEMD UNITS, PROCESSES & ALSA KERNEL",
    ] {
        if !txt_body.contains(needle) {
            return Err(format!("exported text report missing section: {needle}"));
        }
    }

    let mut cap = DryRunDesktopCapturer::new(64, 48);
    let img = RgbaImage {
        width: 64,
        height: 48,
        data: cap.next_frame(),
    };
    let small = resize_rgba(&img, 32, 24);
    let encoded = encode_png_rgba(&small)?;
    let decoded = decode_png_rgba(&encoded)?;
    if decoded.width != 32 || decoded.height != 24 || decoded.data.len() != 32 * 24 * 4 {
        return Err("zlib_png resize/encode/decode mismatch".to_string());
    }

    verify_lgpl_compliance(base_dir)?;

    Ok(
        "0700/0600 settings + symlink guard + tools + zlib_png + LGPL-3.0 compliance + Studio/Tray/Debugger UI --self-test OK"
            .to_string(),
    )
}

fn check_spdx_dir_rs(dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            check_spdx_dir_rs(&p)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            let txt = fs::read_to_string(&p)
                .map_err(|e| format!("read {}: {e}", p.display()))?;
            if !txt
                .chars()
                .take(400)
                .collect::<String>()
                .contains("SPDX-License-Identifier: LGPL-3.0-only")
            {
                return Err(format!("missing LGPL-3.0 SPDX header in {}", p.display()));
            }
        }
    }
    Ok(())
}

fn verify_lgpl_compliance(base_dir: &Path) -> Result<(), String> {
    let copying = fs::read_to_string(base_dir.join("COPYING"))
        .map_err(|e| format!("missing COPYING (GPL-3.0): {e}"))?;
    if !copying.contains("GNU GENERAL PUBLIC LICENSE") || !copying.contains("Version 3, 29 June 2007") {
        return Err("COPYING does not contain verbatim GPL-3.0 text".to_string());
    }
    let lesser = fs::read_to_string(base_dir.join("COPYING.LESSER"))
        .map_err(|e| format!("missing COPYING.LESSER (LGPL-3.0): {e}"))?;
    if !lesser.contains("GNU LESSER GENERAL PUBLIC LICENSE") || !lesser.contains("Version 3, 29 June 2007") {
        return Err("COPYING.LESSER does not contain verbatim LGPL-3.0 text".to_string());
    }
    let tp = fs::read_to_string(base_dir.join("THIRD_PARTY_LICENSES.md"))
        .map_err(|e| format!("missing THIRD_PARTY_LICENSES.md: {e}"))?;
    if !tp.contains("libfdk-aac.so.2") {
        return Err("THIRD_PARTY_LICENSES.md missing optional dlopen libfdk-aac disclosure".to_string());
    }

    for f in ["Makefile", "install-salsa.sh"] {
        let txt = fs::read_to_string(base_dir.join(f))
            .map_err(|e| format!("read {f}: {e}"))?;
        if !txt
            .chars()
            .take(400)
            .collect::<String>()
            .contains("SPDX-License-Identifier: LGPL-3.0-only")
        {
            return Err(format!("missing LGPL-3.0 SPDX header in {f}"));
        }
    }

    for entry in fs::read_dir(base_dir.join("qt6")).map_err(|e| format!("read_dir qt6: {e}"))?.flatten() {
        let p = entry.path();
        if matches!(p.extension().and_then(|s| s.to_str()), Some("cpp" | "hpp" | "h")) {
            let txt = fs::read_to_string(&p)
                .map_err(|e| format!("read {}: {e}", p.display()))?;
            if !txt
                .chars()
                .take(400)
                .collect::<String>()
                .contains("SPDX-License-Identifier: LGPL-3.0-only")
            {
                return Err(format!("missing LGPL-3.0 SPDX header in {}", p.display()));
            }
        }
    }

    for entry in fs::read_dir(base_dir.join("rust")).map_err(|e| format!("read_dir rust: {e}"))?.flatten() {
        let pkg_dir = entry.path();
        let manifest = pkg_dir.join("Cargo.toml");
        if manifest.is_file() {
            let mtxt = fs::read_to_string(&manifest)
                .map_err(|e| format!("read {}: {e}", manifest.display()))?;
            if !mtxt.contains("license = \"LGPL-3.0-only\"") {
                return Err(format!("Cargo.toml missing LGPL-3.0-only license: {}", manifest.display()));
            }
            let src = pkg_dir.join("src");
            if src.is_dir() {
                check_spdx_dir_rs(&src)?;
            }
        }
    }

    for bin_name in ["build/lasapi-ctl", "build/salsa-installer", "build/salsa-debug-cli", "build/lasapi-audiodg"] {
        let out = Command::new(base_dir.join(bin_name))
            .arg("--version")
            .output()
            .map_err(|e| format!("{bin_name} --version: {e}"))?;
        if !out.status.success() {
            return Err(format!("{bin_name} --version failed"));
        }
        let s = String::from_utf8_lossy(&out.stdout);
        if !s.contains("LGPL-3.0-only") || !s.contains("WITHOUT ANY WARRANTY") {
            return Err(format!("{bin_name} --version missing LGPL-3.0 notice: {s}"));
        }
    }
    Ok(())
}

fn test_elf_hardening_and_proxy_parity(base_dir: &Path) -> Result<String, String> {
    let audiodg = base_dir.join("build/lasapi-audiodg");
    if !audiodg.is_file() {
        return Err("missing build/lasapi-audiodg".to_string());
    }
    let ad_dyn = readelf_dynamic_text(&audiodg)?;
    if !ad_dyn.contains("BIND_NOW") && !ad_dyn.contains(" NOW") {
        return Err("build/lasapi-audiodg missing BIND_NOW hardening flag".to_string());
    }

    let so_checks = [
        "build/liblasapi.so",
        "build/libasound_module_pcm_lasapi.so",
        "build/shims/libasound.so.2",
        "build/shims/libpulse.so.0",
        "build/shims/libjack.so.0",
        "build/libsalsa_pa_reconnect.so",
        "build/pa_reconnect/libpulse.so.0",
    ];
    for rel in so_checks {
        let p = base_dir.join(rel);
        if !p.is_file() {
            return Err(format!("missing ELF artifact: {rel}"));
        }
        let dyn_txt = readelf_dynamic_text(&p)?;
        if !dyn_txt.contains("BIND_NOW") && !dyn_txt.contains(" NOW") {
            return Err(format!("{rel} missing BIND_NOW hardening flag"));
        }
        if rel == "build/pa_reconnect/libpulse.so.0" && !dyn_txt.contains("NODELETE") {
            return Err(format!("{rel} missing NODELETE hardening flag"));
        }
        let syms = Command::new("nm")
            .args(["-D", "--defined-only", &p.to_string_lossy()])
            .output()
            .map_err(|e| format!("nm {rel}: {e}"))?;
        let sym_str = String::from_utf8_lossy(&syms.stdout);
        if sym_str.contains("rust_eh_personality") || sym_str.contains("_RNv") {
            return Err(format!("{rel} leaks internal Rust symbols"));
        }
    }

    let proxy_so = base_dir.join("build/pa_reconnect/libpulse.so.0");
    let shim_so = base_dir.join("build/shims/libpulse.so.0");
    let shim_syms = dynamic_pa_symbols(&shim_so)?;
    if shim_syms.len() < 80 {
        return Err(format!(
            "build/shims/libpulse.so.0 exports only {} pa_* symbols (expected >= 80)",
            shim_syms.len()
        ));
    }

    let mut parity_count = 0usize;
    if let Some(sys_pulse) = find_host_lib("libpulse.so.0") {
        let host_syms = dynamic_pa_symbols(Path::new(&sys_pulse))?;
        let proxy_syms = dynamic_pa_symbols(&proxy_so)?;
        let missing: Vec<&String> = host_syms.difference(&proxy_syms).collect();
        if !missing.is_empty() {
            return Err(format!(
                "build/pa_reconnect/libpulse.so.0 missing {} host symbols (e.g. {:?})",
                missing.len(),
                &missing[..missing.len().min(5)]
            ));
        }
        parity_count = host_syms.len();
    }

    if which("gcc").is_some() && Path::new("/usr/include/pulse/pulseaudio.h").exists() {
        let tmp = TempDirGuard::new("salsa_reg_pa_c")?;
        let c_src = tmp.path.join("probe.c");
        let bin = tmp.path.join("probe");
        fs::write(
            &c_src,
            b"#include <pulse/pulseaudio.h>\n#include <assert.h>\nint main(void) {\n  pa_channel_map cm;\n  pa_channel_map_init_stereo(&cm);\n  assert(cm.channels == 2);\n  assert(pa_get_library_version() != NULL);\n  pa_proplist *pl = pa_proplist_new();\n  assert(pl != NULL);\n  pa_proplist_free(pl);\n  assert(pa_strerror(0) != NULL);\n  return 0;\n}\n",
        )
        .map_err(|e| format!("write probe.c: {e}"))?;
        let st = Command::new("gcc")
            .arg(&c_src)
            .arg("-o")
            .arg(&bin)
            .arg(format!("-L{}", base_dir.join("build/shims").display()))
            .arg(format!("-Wl,-rpath,{}", base_dir.join("build/shims").display()))
            .arg("-lpulse")
            .status()
            .map_err(|e| format!("gcc probe: {e}"))?;
        if !st.success() {
            return Err("gcc link against build/shims/libpulse.so.0 failed".to_string());
        }
        let run_st = Command::new(&bin)
            .env(
                "LD_LIBRARY_PATH",
                format!(
                    "{}:{}",
                    base_dir.join("build/shims").display(),
                    base_dir.join("build").display()
                ),
            )
            .status()
            .map_err(|e| format!("run c probe: {e}"))?;
        if !run_st.success() {
            return Err("C Pulse probe runtime assertion failed".to_string());
        }
    }

    Ok(format!(
        "BIND_NOW/NODELETE verified, proxy 100% ({parity_count} pa_* symbols), shim {} pa_* symbols, C ABI OK",
        shim_syms.len()
    ))
}

fn test_rev62_hw_analog_scoring_and_defaults(_base_dir: &Path) -> Result<String, String> {
    let mut cfg = unsafe { mem::zeroed() };
    unsafe { lasapi_hw_analog_default_cfg(&mut cfg) };
    if cfg.hw_dac_unity_vol != 1
        || cfg.hp_amp_tier != 0
        || cfg.output_route != 0
        || cfg.unlock_0db_rails != 0
        || cfg.analog_boost_db.abs() > 1e-5
        || cfg.mic_hw_boost_db.abs() > 1e-5
    {
        return Err("lasapi_hw_analog_default_cfg unexpected defaults".to_string());
    }

    let sc_strix = unsafe { lasapi_hw_analog_score_card(99, c"STRIX RAID DLX USB Audio".as_ptr()) };
    let sc_soar = unsafe { lasapi_hw_analog_score_card(99, c"ASUS STRIX SOAR Audio".as_ptr()) };
    let sc_scar = unsafe { lasapi_hw_analog_score_card(99, c"Scarlett 2i2 4th Gen".as_ptr()) };
    let sc_hdmi = unsafe { lasapi_hw_analog_score_card(99, c"HDA ATI HDMI".as_ptr()) };
    if sc_strix <= sc_hdmi + 100 || sc_soar <= sc_hdmi + 100 || sc_scar <= sc_hdmi + 100 {
        return Err(format!(
            "unexpected scores strix={sc_strix} soar={sc_soar} scarlett={sc_scar} hdmi={sc_hdmi}"
        ));
    }
    Ok(format!(
        "defaults OK; scores strix={sc_strix} soar={sc_soar} scarlett={sc_scar} > hdmi={sc_hdmi}"
    ))
}

fn test_rust_wire_core_workspace(base_dir: &Path) -> Result<String, String> {
    let Some(cargo) = find_cargo() else {
        return Err("cargo not found in PATH or ~/.cargo/bin".to_string());
    };
    let manifest = base_dir.join("rust/Cargo.toml");
    let path_env = match (cargo.parent(), env::var_os("PATH")) {
        (Some(dir), Some(cur)) => format!("{}:{}", dir.display(), cur.to_string_lossy()),
        (Some(dir), None) => dir.display().to_string(),
        (None, Some(cur)) => cur.to_string_lossy().into_owned(),
        (None, None) => String::new(),
    };
    let r_clippy = Command::new(&cargo)
        .env("PATH", &path_env)
        .args([
            "clippy",
            "--manifest-path",
            &manifest.to_string_lossy(),
            "--workspace",
            "--offline",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .output()
        .map_err(|e| format!("cargo clippy: {e}"))?;
    if !r_clippy.status.success() {
        return Err(format!(
            "cargo clippy failed:\n{}\n{}",
            String::from_utf8_lossy(&r_clippy.stdout),
            String::from_utf8_lossy(&r_clippy.stderr)
        ));
    }
    let r_test = Command::new(&cargo)
        .env("PATH", &path_env)
        .args([
            "test",
            "--manifest-path",
            &manifest.to_string_lossy(),
            "--workspace",
            "--offline",
        ])
        .output()
        .map_err(|e| format!("cargo test: {e}"))?;
    if !r_test.status.success() {
        return Err(format!(
            "cargo test failed:\n{}\n{}",
            String::from_utf8_lossy(&r_test.stdout),
            String::from_utf8_lossy(&r_test.stderr)
        ));
    }
    Ok("workspace cargo clippy -D warnings + cargo test --offline OK".to_string())
}

fn test_live_daemon_security_and_pw_broker(base_dir: &Path) -> Result<String, String> {
    let rt = xdg_runtime_dir().join("lasapi");
    let rmode = fs::metadata(&rt)
        .map_err(|e| format!("stat {}: {e}", rt.display()))?
        .permissions()
        .mode()
        & 0o777;
    if rmode != 0o700 {
        return Err(format!("{} mode expected 0700, got 0o{rmode:o}", rt.display()));
    }

    let uid = getuid();
    for sock_name in ["audiodg.sock", "debug.sock", "debug-in.sock"] {
        let p = rt.join(sock_name);
        let meta = fs::metadata(&p).map_err(|e| format!("stat {}: {e}", p.display()))?;
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(format!("{} mode expected 0600, got 0o{mode:o}", p.display()));
        }
        if sock_name != "debug-in.sock" {
            let info = peercred_for_unix_socket(&p);
            if !info.connectable || info.peer_uid != Some(uid) {
                return Err(format!(
                    "{sock_name} peercred unexpected: connectable={} uid={:?}",
                    info.connectable, info.peer_uid
                ));
            }
        }
    }

    let ctl_out = Command::new(base_dir.join("build/lasapi-ctl"))
        .arg("status")
        .output()
        .map_err(|e| format!("lasapi-ctl status: {e}"))?;
    if !ctl_out.status.success() {
        return Err("lasapi-ctl status failed".to_string());
    }

    let snap = snapshot_once(None, Duration::from_millis(1200))?;
    if snap
        .get("pw")
        .and_then(|p| p.get("broker_running"))
        .and_then(JsonValue::as_i64)
        != Some(1)
    {
        return Err("live pw.broker_running != 1".to_string());
    }

    let freshness = SystemAuditScanner::check_build_freshness(base_dir);
    if freshness
        .get("stale")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true)
    {
        let sum = freshness
            .get("summary")
            .and_then(JsonValue::as_str)
            .unwrap_or("stale");
        return Err(format!("live build freshness stale: {sum}"));
    }

    let show_out = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "ActiveState,MainPID",
            "--value",
            "lasapi-tray.service",
        ])
        .output()
        .map_err(|e| format!("systemctl show lasapi-tray.service: {e}"))?;
    if !show_out.status.success() {
        return Err("systemctl show lasapi-tray.service failed".to_string());
    }
    let show_txt = String::from_utf8_lossy(&show_out.stdout);
    let mut show_lines = show_txt.lines().map(str::trim);
    let active_state = show_lines.next().unwrap_or("");
    let main_pid: u32 = show_lines
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if active_state != "active" || main_pid == 0 {
        return Err(format!(
            "lasapi-tray.service not active (ActiveState={active_state}, MainPID={main_pid})"
        ));
    }
    let cmd_bytes = fs::read(format!("/proc/{main_pid}/cmdline"))
        .map_err(|e| format!("read /proc/{main_pid}/cmdline: {e}"))?;
    let cmdline = cmd_bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    if !cmdline.contains("lasapi-tray") || cmdline.contains("python") {
        return Err(format!(
            "lasapi-tray.service MainPID={main_pid} unexpected cmdline: {cmdline}"
        ));
    }

    let sni_dest = format!("org.kde.StatusNotifierItem-{main_pid}-1");
    if which("gdbus").is_some() {
        let sni_out = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                &sni_dest,
                "--object-path",
                "/StatusNotifierItem",
                "--method",
                "org.freedesktop.DBus.Properties.GetAll",
                "org.kde.StatusNotifierItem",
            ])
            .output()
            .map_err(|e| format!("gdbus SNI inspect: {e}"))?;
        if !sni_out.status.success() {
            return Err(format!(
                "SNI object {sni_dest} GetAll failed: {}",
                String::from_utf8_lossy(&sni_out.stderr)
            ));
        }
        let props = String::from_utf8_lossy(&sni_out.stdout);
        if !props.contains("'IconName': <'")
            || props.contains("'IconName': <''>")
            || !props.contains("'IconThemePath': <'")
            || props.contains("'IconThemePath': <''>")
        {
            return Err(format!("SNI {sni_dest} missing IconName/IconThemePath: {props}"));
        }
        let theme_path = props
            .split("'IconThemePath': <'")
            .nth(1)
            .and_then(|s| s.split('\'').next())
            .unwrap_or("");
        if !theme_path.ends_with("/tray-icons") || theme_path.ends_with("/hicolor") {
            return Err(format!(
                "SNI {sni_dest} unexpected IconThemePath '{theme_path}' (must end with /tray-icons)"
            ));
        }
    } else if which("busctl").is_some() {
        for prop in ["IconName", "IconThemePath"] {
            let p_out = Command::new("busctl")
                .args([
                    "--user",
                    "get-property",
                    &sni_dest,
                    "/StatusNotifierItem",
                    "org.kde.StatusNotifierItem",
                    prop,
                ])
                .output()
                .map_err(|e| format!("busctl SNI {prop}: {e}"))?;
            if !p_out.status.success() {
                return Err(format!("busctl SNI {prop} failed on {sni_dest}"));
            }
            let val = String::from_utf8_lossy(&p_out.stdout);
            let trimmed = val.trim();
            if !trimmed.starts_with("s \"") || trimmed == "s \"\"" {
                return Err(format!("SNI {sni_dest} empty {prop}: {val}"));
            }
            if prop == "IconThemePath"
                && (!trimmed.ends_with("/tray-icons\"") || trimmed.ends_with("/hicolor\""))
            {
                return Err(format!(
                    "SNI {sni_dest} unexpected IconThemePath {trimmed} (must end with /tray-icons)"
                ));
            }
        }
    } else {
        return Err("Neither gdbus nor busctl available to verify SNI registration".to_string());
    }

    let ctl_bin = base_dir.join("build/lasapi-ctl");
    let pw_out = Command::new(&ctl_bin)
        .arg("pw-self-test")
        .output()
        .map_err(|e| format!("exec {} pw-self-test: {e}", ctl_bin.display()))?;
    if !pw_out.status.success() {
        return Err(format!(
            "lasapi-ctl pw-self-test failed: {}",
            String::from_utf8_lossy(&pw_out.stderr)
        ));
    }

    Ok("0700/0600 runtime & socket peercred + PW broker + pw-self-test + pure-Rust tray SNI + build freshness OK".to_string())
}

fn find_regtest_session() -> Option<(salsa_tools::ipc::LasapiSessionInfo, LasapiEngineStatus)> {
    let st = get_engine_status()?;
    let n = (st.num_sessions as usize).min(LASAPI_MAX_SESSIONS);
    for i in 0..n {
        let s = st.sessions[i];
        let nm = cstr_to_string(&s.client_name);
        if nm.contains("SalsaRegressionTest") || nm.contains("SalsaRegTestStream") {
            return Some((s, st));
        }
    }
    None
}

fn test_live_session_control_reconnect_and_app_volumes(base_dir: &Path) -> Result<String, String> {
    if which("gcc").is_none() || !Path::new("/usr/include/pulse/pulseaudio.h").exists() {
        return Err("gcc / pulse headers missing for PA reconnect regression".to_string());
    }
    let st_dir = state_dir();
    let app_vol_path = st_dir.join("app_volumes.conf");
    let backup = fs::read(&app_vol_path).ok();
    let _vol_guard = AppVolBackupGuard {
        path: app_vol_path.clone(),
        backup,
    };

    let tmp = TempDirGuard::new("salsa_reg_reconnect")?;
    let stop_flag = tmp.path.join("stop");
    let c_src = tmp.path.join("client.c");
    let exe = tmp.path.join("client");

    let client_c = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pulse/pulseaudio.h>

static volatile int g_ready = 0;

static void ctx_cb(pa_context *c, void *u) {
    (void)u;
    if (pa_context_get_state(c) == PA_CONTEXT_READY) g_ready = 1;
}

static void write_cb(pa_stream *s, size_t nbytes, void *u) {
    (void)u;
    static char zeros[4096];
    while (nbytes > 0) {
        size_t chunk = nbytes > sizeof(zeros) ? sizeof(zeros) : nbytes;
        if (pa_stream_write(s, zeros, chunk, NULL, 0, PA_SEEK_RELATIVE) < 0) break;
        nbytes -= chunk;
    }
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    pa_threaded_mainloop *ml = pa_threaded_mainloop_new();
    pa_mainloop_api *api = pa_threaded_mainloop_get_api(ml);
    pa_proplist *props = pa_proplist_new();
    pa_proplist_sets(props, PA_PROP_APPLICATION_NAME, "SalsaRegressionTest");
    pa_proplist_sets(props, PA_PROP_APPLICATION_PROCESS_BINARY, "salsa_regtest_bin");

    pa_context *ctx = pa_context_new_with_proplist(api, "SalsaRegressionTest", props);
    pa_proplist_free(props);
    pa_context_set_state_callback(ctx, ctx_cb, NULL);
    pa_threaded_mainloop_start(ml);

    pa_threaded_mainloop_lock(ml);
    if (pa_context_connect(ctx, NULL, PA_CONTEXT_NOFLAGS, NULL) < 0) return 2;
    for (int i = 0; i < 100 && !g_ready; i++) {
        pa_threaded_mainloop_unlock(ml);
        usleep(50000);
        pa_threaded_mainloop_lock(ml);
    }
    if (!g_ready) { pa_threaded_mainloop_unlock(ml); return 3; }

    pa_sample_spec ss = { .format = PA_SAMPLE_S16LE, .rate = 48000, .channels = 2 };
    pa_stream *st = pa_stream_new(ctx, "SalsaRegTestStream", &ss, NULL);
    pa_stream_set_write_callback(st, write_cb, NULL);
    if (pa_stream_connect_playback(st, NULL, NULL, PA_STREAM_AUTO_TIMING_UPDATE, NULL, NULL) < 0) {
        pa_threaded_mainloop_unlock(ml);
        return 4;
    }
    pa_threaded_mainloop_unlock(ml);

    const char *stop_path = (argc > 1) ? argv[1] : "";
    for (int i = 0; i < 100; i++) {
        usleep(100000);
        pa_usec_t usec = 0;
        pa_threaded_mainloop_lock(ml);
        int rc = pa_stream_get_time(st, &usec);
        pa_stream_state_t sst = pa_stream_get_state(st);
        pa_threaded_mainloop_unlock(ml);
        printf("TICK state=%d rc=%d time=%llu\n", (int)sst, rc, (unsigned long long)usec);
        if (stop_path[0] && access(stop_path, F_OK) == 0) break;
    }

    pa_threaded_mainloop_lock(ml);
    pa_stream_disconnect(st);
    pa_stream_unref(st);
    pa_context_disconnect(ctx);
    pa_context_unref(ctx);
    pa_threaded_mainloop_unlock(ml);
    pa_threaded_mainloop_stop(ml);
    pa_threaded_mainloop_free(ml);
    return 0;
}
"#;
    fs::write(&c_src, client_c.as_bytes()).map_err(|e| format!("write client.c: {e}"))?;
    let cc = Command::new("gcc")
        .args(["-O2", &c_src.to_string_lossy(), "-lpulse", "-o", &exe.to_string_lossy()])
        .output()
        .map_err(|e| format!("gcc reconnect probe: {e}"))?;
    if !cc.status.success() {
        return Err(format!(
            "Failed compiling PA reconnect probe: {}",
            String::from_utf8_lossy(&cc.stderr)
        ));
    }

    let ld_path = format!(
        "{}:{}",
        base_dir.join("build/pa_reconnect").display(),
        base_dir.join("build").display()
    );
    let child = Command::new(&exe)
        .arg(&stop_flag)
        .env("LD_LIBRARY_PATH", &ld_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn PA reconnect client: {e}"))?;
    let mut guard = ChildGuard::new(child);

    let mut found = None;
    for _ in 0..40 {
        if let Some(pair) = find_regtest_session() {
            found = Some(pair);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let (sess, st) = found.ok_or("Test PA stream never appeared in live engine status")?;
    let sid = sess.session_id;
    let active_dev = cstr_to_string(&st.active_playback_id).trim().to_string();

    if set_session_control(sid, 73, 1, None) != 0 {
        return Err("set_session_control(73, mute=1) failed".to_string());
    }
    let (s2, _) = find_regtest_session().ok_or("session vanished after mute")?;
    if s2.volume_milli != 730 || s2.muted != 1 {
        return Err(format!(
            "expected vol=730 muted=1, got vol={} muted={}",
            s2.volume_milli, s2.muted
        ));
    }

    if !active_dev.is_empty() && active_dev != "auto" {
        if set_session_control(sid, 88, 0, Some(&active_dev)) != 0 {
            return Err("set_session_control(88, 0, active_dev) failed".to_string());
        }
        let (s3, _) = find_regtest_session().ok_or("session vanished after pin")?;
        if (s3.routing_flags & 1) != 1 || cstr_to_string(&s3.target_device) != active_dev {
            return Err("device pin routing_flags mismatch".to_string());
        }
        if set_session_control(sid, -1, -1, Some("auto")) != 0 {
            return Err("set_session_control(auto) failed".to_string());
        }
        let (s4, _) = find_regtest_session().ok_or("session vanished after unpin")?;
        if (s4.routing_flags & 1) != 0 {
            return Err("auto reset failed to clear routing_flags".to_string());
        }
    } else if set_session_control(sid, 88, 0, None) != 0 {
        return Err("set_session_control(88, 0) failed".to_string());
    }

    let av_mode = fs::metadata(&app_vol_path)
        .map_err(|e| format!("stat {}: {e}", app_vol_path.display()))?
        .permissions()
        .mode()
        & 0o777;
    if av_mode != 0o600 {
        return Err(format!("expected 0600 mode on app_volumes.conf, got 0o{av_mode:o}"));
    }

    let rr = Command::new("systemctl")
        .args(["--user", "restart", "lasapi-audiodg.service"])
        .output()
        .map_err(|e| format!("systemctl restart: {e}"))?;
    if !rr.status.success() {
        return Err(format!(
            "Failed restarting lasapi-audiodg.service: {}",
            String::from_utf8_lossy(&rr.stderr)
        ));
    }

    let mut reconnected = None;
    for _ in 0..50 {
        if let Some((sr, _)) = find_regtest_session() {
            reconnected = Some(sr);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let rec = reconnected.ok_or("PA client did not auto-reconnect after audiodg restart")?;
    if rec.volume_milli != 880 || rec.muted != 0 {
        return Err(format!(
            "Persisted app_volumes failed across restart: vol={} muted={}",
            rec.volume_milli, rec.muted
        ));
    }

    thread::sleep(Duration::from_millis(600));
    let _ = fs::write(&stop_flag, b"stop");
    let mut times = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if let Some(ref mut ch) = guard.child {
            if let Ok(Some(_)) = ch.try_wait() {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    if let Ok(o) = guard.wait_with_output() {
        let txt = String::from_utf8_lossy(&o.stdout);
        for line in txt.lines() {
            if let Some(rest) = line.split("time=").nth(1) {
                if let Ok(us) = rest.trim().parse::<u64>() {
                    times.push(us);
                }
            }
        }
    }
    if times.len() < 5 {
        return Err(format!("Insufficient PA timing samples ({})", times.len()));
    }
    for w in times.windows(2) {
        if w[1] < w[0] {
            return Err(format!("Non-monotonic pa_stream_get_time: {} -> {}", w[0], w[1]));
        }
    }
    if times[times.len() - 1] <= times[0] {
        return Err("Stream time did not advance across restart".to_string());
    }
    Ok(format!(
        "PA reconnect + app_volumes persistence + monotonic stream time ({} -> {} us) OK",
        times[0],
        times[times.len() - 1]
    ))
}

fn test_live_rev61_studio_apis(base_dir: &Path) -> Result<String, String> {
    let mut orig = None;
    for _ in 0..40 {
        if let Some(st) = get_engine_status() {
            orig = Some(st);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let mut orig = orig.ok_or("Failed fetching initial live engine status")?;

    let mut orig_cap_pref = if (44100..=192000).contains(&orig.hw_analog_info.reserved)
        && orig.hw_analog_info.reserved != orig.hw_sample_rate
    {
        orig.hw_analog_info.reserved
    } else {
        0
    };

    if let Ok(snap0) = snapshot_once(None, Duration::from_millis(1200)) {
        if let Some(bz) = snap0.get("bluez") {
            let has_ep = bz
                .get("endpoints_registered")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0)
                != 0;
            let has_dev = !bz
                .get("device_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .is_empty();
            if (has_ep || has_dev) && orig.bt_active == 0 {
                for _ in 0..35 {
                    thread::sleep(Duration::from_millis(100));
                    if let Some(cur) = get_engine_status() {
                        orig = cur;
                        if orig.bt_active != 0 {
                            break;
                        }
                    }
                }
            }
        }
        if let Ok(snap1) = snapshot_once(None, Duration::from_millis(1200)) {
            if let Some(pr) = snap1
                .get("capture")
                .and_then(|c| c.get("pref_rate"))
                .and_then(JsonValue::as_u64)
            {
                orig_cap_pref = pr as u32;
            }
        }
    }

    let _restore_guard = StudioStateRestoreGuard {
        orig,
        orig_cap_pref,
    };

    if set_engine_config(48000, orig.hw_period_frames, 16) != 0 {
        return Err("set_engine_config(48000, period, 16) failed".to_string());
    }
    let st = get_engine_status().ok_or("get_engine_status failed after clock set")?;
    if st.hw_sample_rate < 44100 || !matches!(st.hw_format_bits, 16 | 24 | 32) {
        return Err(format!(
            "Invalid clock state rate={} bits={}",
            st.hw_sample_rate, st.hw_format_bits
        ));
    }

    let mut eq = orig.eq_cfg;
    let mut ext = orig.param_eq;
    ext.band_freq_hz[0] = 105.0;
    ext.band_type[0] = 1;
    eq.band_q[0] = 0.71;
    eq.band_gains_db[0] = 2.5;
    eq.enabled = 1;
    ext.crossfeed_enabled = 1;
    ext.crossfeed_level = 0.35;
    if set_parametric_eq(&eq, &ext) != 0 {
        return Err("set_parametric_eq failed".to_string());
    }
    let st = get_engine_status().ok_or("get_engine_status failed after param_eq")?;
    if st.param_eq.band_type[0] != 1
        || (st.param_eq.band_freq_hz[0] - 105.0).abs() >= 0.1
        || (st.eq_cfg.band_q[0] - 0.71).abs() >= 0.01
        || st.param_eq.crossfeed_enabled != 1
        || (st.param_eq.crossfeed_level - 0.35).abs() >= 0.02
    {
        return Err("parametric EQ / crossfeed verification mismatch".to_string());
    }

    let mut mic = orig.mic_apo;
    mic.enabled = 1;
    mic.rnnoise_enabled = 1;
    mic.sidetone_enabled = 1;
    mic.sidetone_pct = 25;
    mic.hpf_hz = 90.0;
    if set_mic_apo(&mic) != 0 {
        return Err("set_mic_apo failed".to_string());
    }
    let st = get_engine_status().ok_or("get_engine_status failed after mic_apo")?;
    if st.mic_apo.enabled != 1
        || st.mic_apo.rnnoise_enabled != 1
        || st.mic_apo.sidetone_enabled != 1
        || st.mic_apo.sidetone_pct != 25
    {
        return Err("mic APO verification mismatch".to_string());
    }

    let mut bt = orig.bt_codec_cfg;
    bt.wireless_abr_enabled = 1;
    if set_bt_codec_config(&bt) != 0 {
        return Err("set_bt_codec_config failed".to_string());
    }
    let st = get_engine_status().ok_or("get_engine_status failed after bt_codec")?;
    if st.bt_codec_cfg.wireless_abr_enabled != 1 {
        return Err("wireless_abr_enabled mismatch".to_string());
    }
    let sup_mask = orig.bt_codec_cfg.supported_codecs_mask;
    if orig.bt_active != 0 && sup_mask != 0 && (sup_mask & 1) == 0 {
        let mut bt_rej = orig.bt_codec_cfg;
        bt_rej.preferred_codec = LASAPI_BT_PREF_LDAC;
        if set_bt_codec_config(&bt_rej) != 0 {
            return Err("set_bt_codec_config preflight check failed".to_string());
        }
        let st2 = get_engine_status().ok_or("get_engine_status failed after bt preflight")?;
        if st2.bt_active == 0 || st2.bt_codec_cfg.preferred_codec != orig.bt_codec_cfg.preferred_codec
        {
            return Err("unsupported BT codec preflight must not drop A2DP link".to_string());
        }
    }

    for fname in ["salsa_settings.json", "salsa_state.bin", "app_volumes.conf"] {
        let p = state_dir().join(fname);
        if p.exists() {
            let lm = fs::symlink_metadata(&p).map_err(|e| format!("lstat {fname}: {e}"))?;
            if lm.file_type().is_symlink() {
                return Err(format!("{} must not be a symlink", p.display()));
            }
            let m = lm.permissions().mode() & 0o777;
            if m != 0o600 {
                return Err(format!("{} mode expected 0600, got 0o{m:o}", p.display()));
            }
        }
    }

    let mut hw_cfg = orig.hw_analog_cfg;
    hw_cfg.analog_boost_db = 4.0;
    hw_cfg.mic_hw_boost_db = 8.0;
    if set_hw_analog(&hw_cfg) != 0 {
        return Err("set_hw_analog failed".to_string());
    }
    let st = get_engine_status().ok_or("get_engine_status failed after hw_analog")?;
    if (st.hw_analog_cfg.analog_boost_db - 4.0).abs() >= 0.05
        || (st.hw_analog_cfg.mic_hw_boost_db - 8.0).abs() >= 0.05
    {
        return Err("hw_analog gain verification mismatch".to_string());
    }

    let _ = set_engine_config(192000, orig.hw_period_frames, orig.hw_format_bits);
    if set_capture_sample_rate(48000) != 0 {
        return Err("set_capture_sample_rate(48000) failed".to_string());
    }
    thread::sleep(Duration::from_millis(250));
    let st = get_engine_status().ok_or("get_engine_status failed after capture rate")?;
    if st.hw_analog_info.reserved != 48000 {
        return Err(format!(
            "unexpected capture rate in hw_analog_info.reserved: {}",
            st.hw_analog_info.reserved
        ));
    }

    let ctl_bin = base_dir.join("build/lasapi-ctl");
    let st_m0 = get_engine_status().ok_or("status before midi-send failed")?;
    if !Command::new(&ctl_bin)
        .args(["midi-send", "60", "0"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Err("lasapi-ctl midi-send 60 0 failed".to_string());
    }
    thread::sleep(Duration::from_millis(80));
    let st_m1 = get_engine_status().ok_or("status after vel=0 failed")?;
    if st_m1.midi_events_routed != st_m0.midi_events_routed {
        return Err("vel=0 Note-Off incremented key-press counter".to_string());
    }
    if !Command::new(&ctl_bin)
        .args(["midi-send", "60", "100"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Err("lasapi-ctl midi-send 60 100 failed".to_string());
    }
    let mut midi_ok = false;
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(50));
        if let Some(sm2) = get_engine_status() {
            if sm2.midi_events_routed == st_m0.midi_events_routed + 1 {
                midi_ok = true;
                break;
            }
        }
    }
    if !midi_ok {
        return Err("vel=100 MIDI Note-On did not increment counter by +1".to_string());
    }

    Ok("Studio APIs (clock, param_eq/crossfeed, mic_apo, bt_codec, hw_analog, split cap clock, MIDI counter) OK".to_string())
}

fn test_live_rust_wire_failsafe(base_dir: &Path) -> Result<String, String> {
    let cli = SalsaDebugClient::new(None);
    cli.start();
    if !cli.wait_connected(Duration::from_secs(3)) {
        cli.stop();
        return Err("SalsaDebugClient failed to connect to live debug.sock".to_string());
    }

    let res = (|| {
        let telem = cli
            .wait_snapshot(Duration::from_secs(3))
            .ok_or("Did not receive live telemetry snapshot")?;
        let wire0 = telem.get("wire").ok_or("snapshot missing wire object")?;
        let armed = wire0.get("armed").and_then(JsonValue::as_i64).unwrap_or(0);
        let is_rust = wire0.get("is_rust").and_then(JsonValue::as_i64).unwrap_or(0);
        let backend = wire0
            .get("backend")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        if armed != 1 || is_rust != 1 || backend != "rust" {
            return Err(format!("unexpected wire state: {}", wire0.to_json_compact()));
        }

        let pong = cli
            .ping("phase2-live", Duration::from_secs(2))
            .ok_or("PING timed out")?;
        if pong.get("tag").and_then(JsonValue::as_str) != Some("phase2-live") {
            return Err("PING tag echo mismatch".to_string());
        }

        let trips_before = wire0
            .get("trips_total")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let resets_before = wire0
            .get("manual_resets")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);

        let rep_pa = cli
            .reset_pa_clients()
            .ok_or("RESET_PA_CLIENTS timed out")?;
        if rep_pa.get("rc").and_then(JsonValue::as_i64) != Some(0)
            || !rep_pa
                .get("msg")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .contains("ScreenCast & audio engine preserved")
        {
            return Err(format!("RESET_PA_CLIENTS unexpected: {}", rep_pa.to_json_compact()));
        }

        let rep_net = cli
            .reset_net_clients()
            .ok_or("RESET_NET_CLIENTS timed out")?;
        if rep_net.get("rc").and_then(JsonValue::as_i64) != Some(0)
            || !rep_net
                .get("msg")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .contains("audio engine uninterrupted")
        {
            return Err(format!("RESET_NET_CLIENTS unexpected: {}", rep_net.to_json_compact()));
        }

        let _ = cli.request_snapshot();
        let mut verified = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(s1) = cli.latest_snapshot() {
                if let Some(w1) = s1.get("wire") {
                    let mr = w1.get("manual_resets").and_then(JsonValue::as_u64).unwrap_or(0);
                    let tt = w1.get("trips_total").and_then(JsonValue::as_u64).unwrap_or(0);
                    if mr >= resets_before + 2 && tt >= trips_before + 2 {
                        verified = true;
                        break;
                    }
                }
            }
            thread::sleep(Duration::from_millis(80));
        }
        if !verified {
            return Err("manual_resets/trips_total did not increment after RESET_*".to_string());
        }

        for dbg_bin in ["build/salsa-debug-cli", "build/salsa-debug"] {
            let snap_cli = Command::new(base_dir.join(dbg_bin))
                .args(["--snapshot", "--json"])
                .output()
                .map_err(|e| format!("{dbg_bin} --snapshot --json: {e}"))?;
            if !snap_cli.status.success() {
                return Err(format!("{dbg_bin} --snapshot --json failed"));
            }
            let parsed = JsonValue::parse(&String::from_utf8_lossy(&snap_cli.stdout))
                .map_err(|e| format!("{dbg_bin} JSON invalid: {e}"))?;
            if parsed.get("type").and_then(JsonValue::as_str) != Some("telemetry") {
                return Err(format!("{dbg_bin} telemetry JSON type mismatch"));
            }
        }

        Ok("Live Rust wire backend=rust, armed=1, PING, RESET_PA_CLIENTS, RESET_NET_CLIENTS, salsa-debug-cli & salsa-debug OK".to_string())
    })();

    cli.stop();
    res
}

type SuiteFn = fn(&Path) -> Result<String, String>;

fn main() -> ExitCode {
    reset_sigpipe_default();
    let mut require_live = false;
    let mut skip_cargo = false;
    let mut filter: Option<String> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--require-live" => require_live = true,
            "--skip-cargo" => skip_cargo = true,
            "--filter" => filter = args.next(),
            "-h" | "--help" => {
                println!(
                    "Usage: salsa-regression [--require-live] [--skip-cargo] [--filter SUBSTR]\n\
                     Runs the Salsa pure-Rust integration and regression suite."
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown option: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let base_dir = install_root();
    let offline_suites: [(&str, SuiteFn); 5] = [
        ("test_c_unit_suite", test_c_unit_suite),
        ("test_tools_and_settings_security", test_tools_and_settings_security),
        ("test_elf_hardening_and_proxy_parity", test_elf_hardening_and_proxy_parity),
        ("test_rev62_hw_analog_scoring_and_defaults", test_rev62_hw_analog_scoring_and_defaults),
        ("test_rust_wire_core_workspace", test_rust_wire_core_workspace),
    ];

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    for (name, func) in offline_suites {
        if let Some(ref f) = filter {
            if !name.contains(f.as_str()) {
                continue;
            }
        }
        if skip_cargo && name == "test_rust_wire_core_workspace" {
            skipped += 1;
            println!("[SKIP] {name} (--skip-cargo)");
            continue;
        }
        let t0 = Instant::now();
        match func(&base_dir) {
            Ok(msg) => {
                passed += 1;
                println!("[PASS] {name} ({:.2}s): {msg}", t0.elapsed().as_secs_f64());
            }
            Err(err) => {
                failed += 1;
                eprintln!("[FAIL] {name} ({:.2}s): {err}", t0.elapsed().as_secs_f64());
            }
        }
    }

    let live_suites: [(&str, SuiteFn); 4] = [
        ("test_live_daemon_security_and_pw_broker", test_live_daemon_security_and_pw_broker),
        ("test_live_session_control_reconnect_and_app_volumes", test_live_session_control_reconnect_and_app_volumes),
        ("test_live_rev61_studio_apis", test_live_rev61_studio_apis),
        ("test_live_rust_wire_failsafe", test_live_rust_wire_failsafe),
    ];

    let live_up = is_audiodg_active();
    if !live_up {
        if require_live {
            failed += 1;
            eprintln!("[FAIL] live daemon check: lasapi-audiodg.service is not active (--require-live)");
        } else {
            skipped += live_suites.len();
            println!(
                "[SKIP] live daemon suites ({} suites skipped; lasapi-audiodg.service not active)",
                live_suites.len()
            );
        }
    } else {
        for (name, func) in live_suites {
            if let Some(ref f) = filter {
                if !name.contains(f.as_str()) {
                    continue;
                }
            }
            let t0 = Instant::now();
            match func(&base_dir) {
                Ok(msg) => {
                    passed += 1;
                    println!("[PASS] {name} ({:.2}s): {msg}", t0.elapsed().as_secs_f64());
                }
                Err(err) => {
                    failed += 1;
                    eprintln!("[FAIL] {name} ({:.2}s): {err}", t0.elapsed().as_secs_f64());
                }
            }
        }
    }

    println!(
        "=== Salsa Regression Summary: {passed} passed, {skipped} skipped, {failed} failed ==="
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
