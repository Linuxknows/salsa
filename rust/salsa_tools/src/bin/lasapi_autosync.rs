// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::c_int;
use std::collections::BTreeSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use salsa_tools::debug_core::SystemAuditScanner;
use salsa_tools::host::{
    ensure_dir_0700, find_host_lib, find_host_lib32, flock_fd, funlock_fd, install_root,
    natural_version_cmp, read_text_lossy, sha256_file, state_dir, which, write_atomic_secure,
};
use salsa_tools::json::JsonValue;
use salsa_wire::unix_sys::reset_sigpipe_default;

const LIBS: [&str; 6] = [
    "libasound.so.2",
    "libpulse.so.0",
    "libpulse-simple.so.0",
    "libpulse-mainloop-glib.so.0",
    "libjack.so.0",
    "libjackserver.so.0",
];

const EXTRA_HOST_LIBS_64: [&str; 4] = [
    "libfdk-aac.so.2",
    "libldacBT_enc.so.2",
    "libfreeaptx.so.0",
    "libdbus-1.so.3",
];

#[derive(Default)]
struct Opts {
    print_fingerprint: bool,
    force: bool,
    auto_mode: bool,
    quiet: bool,
    check_only: bool,
}

fn log_line(st_dir: &Path, quiet: bool, msg: &str) {
    if !quiet {
        println!("[lasapi-autosync] {msg}");
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let log_path = st_dir.join("autosync.log");
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)
    {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

fn stat_token(label: &str, path_opt: Option<&str>) -> String {
    let Some(p) = path_opt else {
        return format!("{label}=missing");
    };
    let path = Path::new(p);
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Ok(meta) = fs::metadata(&real) else {
        return format!("{label}={p}:missing");
    };
    let sha = sha256_file(&real).unwrap_or_else(|| "nohash".to_string());
    format!(
        "{label}={}:{}:{}.{}:{}",
        real.display(),
        meta.size(),
        meta.mtime(),
        meta.mtime_nsec(),
        &sha[..16.min(sha.len())]
    )
}

fn compute_fingerprint() -> String {
    let mut lines = Vec::with_capacity(20);
    for lib in LIBS {
        let p64 = find_host_lib(lib);
        lines.push(stat_token(&format!("64:{lib}"), p64.as_deref()));
    }
    for lib in LIBS {
        let p32 = find_host_lib32(lib);
        lines.push(stat_token(&format!("32:{lib}"), p32.as_deref()));
    }
    for lib in EXTRA_HOST_LIBS_64 {
        let p64 = find_host_lib(lib);
        lines.push(stat_token(&format!("64:{lib}"), p64.as_deref()));
    }
    let marker = "/var/lib/salsa-autosync/libpulse-updated";
    if let Ok(meta) = fs::metadata(marker) {
        lines.push(format!(
            "hook={marker}:{}:{}.{}",
            meta.size(),
            meta.mtime(),
            meta.mtime_nsec()
        ));
    } else {
        lines.push(format!("hook={marker}:none"));
    }
    lines.join("\n")
}

fn pipewire_zero_enabled(st_dir: &Path) -> bool {
    matches!(
        read_text_lossy(&st_dir.join("pipewire_zero_mode"), 64).trim(),
        "enabled" | "1"
    )
}

fn pipewire_zero_drifted(st_dir: &Path) -> bool {
    if !pipewire_zero_enabled(st_dir) {
        return false;
    }
    for unit in [
        "pipewire.service",
        "pipewire-pulse.service",
        "wireplumber.service",
        "pulseaudio.service",
    ] {
        let active = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", unit])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if active {
            return true;
        }
    }
    st_dir
        .parent()
        .is_some_and(|cfg| cfg.join("systemd/user/lasapi-pipewire.socket").is_file())
        && systemd_is_active("lasapi-audiodg.service")
        && !systemd_is_active("lasapi-pipewire.socket")
}

fn pkg_lock_busy() -> bool {
    if Path::new("/var/lib/pacman/db.lck").exists() || Path::new("/var/run/yum.pid").exists() {
        return true;
    }
    let dpkg_lock = Path::new("/var/lib/dpkg/lock-frontend");
    if dpkg_lock.exists() && which("fuser").is_some() {
        if let Ok(st) = Command::new("fuser").arg(dpkg_lock).status() {
            if st.success() {
                return true;
            }
        }
    }
    false
}

fn wait_for_pkg_locks(st_dir: &Path, quiet: bool) -> bool {
    let start = Instant::now();
    let max_wait = Duration::from_secs(240);
    let mut logged = false;
    while pkg_lock_busy() {
        if !logged {
            log_line(
                st_dir,
                quiet,
                "Waiting up to 240s for system package manager lock...",
            );
            logged = true;
        }
        if start.elapsed() >= max_wait {
            log_line(
                st_dir,
                quiet,
                "Timed out waiting for package manager lock; skipping rebuild this cycle.",
            );
            return false;
        }
        thread::sleep(Duration::from_secs(2));
    }
    true
}

fn extract_symbol_versions(lib_path: &str, prefix: &str) -> Vec<String> {
    if which("readelf").is_none() {
        return Vec::new();
    }
    let Ok(out) = Command::new("readelf").args(["-V", "-W", lib_path]).output() else {
        return Vec::new();
    };
    let mut set = BTreeSet::new();
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        for tok in line.split_whitespace() {
            let cleaned = tok.trim_matches(|c: char| matches!(c, '(' | ')' | ':' | ';' | ','));
            if cleaned.starts_with(prefix) && !cleaned.contains('@') {
                set.insert(cleaned.to_string());
            }
        }
    }
    let mut list: Vec<String> = set.into_iter().collect();
    list.sort_by(|a, b| natural_version_cmp(a, b));
    list
}

fn build_version_script(versions: &[String], globs: &[&str]) -> String {
    if versions.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(&versions[0]);
    out.push_str(" {\n    global:\n");
    for g in globs {
        out.push_str("        ");
        out.push_str(g);
        out.push_str(";\n");
    }
    out.push_str("    local:\n        *;\n};\n");
    for pair in versions.windows(2) {
        out.push('\n');
        out.push_str(&pair[1]);
        out.push_str(" {\n} ");
        out.push_str(&pair[0]);
        out.push_str(";\n");
    }
    out
}

fn sync_version_scripts(base_dir: &Path, st_dir: &Path, quiet: bool) -> bool {
    let mut changed = false;
    if let Some(alsa_lib) = find_host_lib("libasound.so.2") {
        let mut vers = extract_symbol_versions(&alsa_lib, "ALSA_");
        if !vers.iter().any(|v| v == "ALSA_0.9") {
            vers.insert(0, "ALSA_0.9".to_string());
        }
        let body = build_version_script(&vers, &["snd_*", "_snd_*"]);
        let dst = base_dir.join("src/alsa.version");
        let cur = read_text_lossy(&dst, 64 * 1024);
        if !body.is_empty() && body != cur {
            if let Err(e) = write_atomic_secure(&dst, body.as_bytes(), 0o644) {
                log_line(st_dir, quiet, &format!("failed updating alsa.version: {e}"));
            } else {
                log_line(
                    st_dir,
                    quiet,
                    &format!("Updated src/alsa.version ({} version nodes)", vers.len()),
                );
                changed = true;
            }
        }
    }
    if let Some(pulse_lib) = find_host_lib("libpulse.so.0") {
        let mut vers = extract_symbol_versions(&pulse_lib, "PULSE_");
        if !vers.iter().any(|v| v == "PULSE_0") {
            vers.insert(0, "PULSE_0".to_string());
        }
        let body = build_version_script(&vers, &["pa_*"]);
        let dst = base_dir.join("src/pulse.version");
        let cur = read_text_lossy(&dst, 64 * 1024);
        if !body.is_empty() && body != cur {
            if let Err(e) = write_atomic_secure(&dst, body.as_bytes(), 0o644) {
                log_line(st_dir, quiet, &format!("failed updating pulse.version: {e}"));
            } else {
                log_line(
                    st_dir,
                    quiet,
                    &format!("Updated src/pulse.version ({} version nodes)", vers.len()),
                );
                changed = true;
            }
        }
    }
    changed
}

fn systemd_is_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn print_help() {
    println!(
        "Usage: lasapi-autosync [OPTIONS]\n\
         \nOptions:\n  --print-fingerprint   Print current host library fingerprint and exit\n  \
         --force               Force version sync + rebuild even if fingerprint unchanged\n  \
         --auto                Background watcher mode\n  \
         --quiet               Reduce stdout output\n  \
         --check               Exit 0 if up-to-date, 1 if sync/rebuild needed\n  \
         -V, --version         Show Salsa version & LGPL-3.0 legal notice\n  \
         --license             Show full LGPL-3.0, GPL-3.0, and third-party terms\n  \
         -h, --help            Show this help"
    );
}

fn main() -> ExitCode {
    reset_sigpipe_default();
    let mut opts = Opts::default();
    for arg in env::args().skip(1) {
        if matches!(arg.as_str(), "--version" | "-V" | "--license" | "--legal") {
            if let Some(rc) = salsa_wire::legal::handle_common_cli_flag("lasapi-autosync", &arg) {
                return ExitCode::from(rc as u8);
            }
        }
        match arg.as_str() {
            "--print-fingerprint" => opts.print_fingerprint = true,
            "--force" => opts.force = true,
            "--auto" => opts.auto_mode = true,
            "--quiet" | "-q" => opts.quiet = true,
            "--check" => opts.check_only = true,
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown option: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let fp = compute_fingerprint();
    if opts.print_fingerprint {
        println!("{fp}");
        return ExitCode::SUCCESS;
    }

    let base_dir = install_root();
    let st_dir = state_dir();
    if let Err(e) = ensure_dir_0700(&st_dir) {
        eprintln!("Failed to prepare {}: {e}", st_dir.display());
        return ExitCode::from(1);
    }

    let state_path = st_dir.join("module_versions.state");
    let prev_fp = read_text_lossy(&state_path, 128 * 1024);
    let fp_changed = prev_fp.trim() != fp.trim();
    let drifted = pipewire_zero_drifted(&st_dir);
    let freshness = SystemAuditScanner::check_build_freshness(&base_dir);
    let build_stale = freshness
        .get("stale")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    if opts.check_only {
        if fp_changed || drifted || build_stale {
            log_line(
                &st_dir,
                opts.quiet,
                &format!(
                    "Sync needed (fp_changed={fp_changed}, zero_pw_drift={drifted}, build_stale={build_stale})"
                ),
            );
            return ExitCode::from(1);
        }
        log_line(&st_dir, opts.quiet, "Salsa host modules and build are in sync.");
        return ExitCode::SUCCESS;
    }

    let lock_path = st_dir.join("autosync.lock");
    let lock_file = match OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Cannot open {}: {e}", lock_path.display());
            return ExitCode::from(1);
        }
    };
    let lock_fd = lock_file.as_raw_fd();
    if let Err(e) = flock_fd(lock_fd, !opts.force) {
        if !opts.force && e.raw_os_error() == Some(libc_ewouldblock()) {
            log_line(
                &st_dir,
                opts.quiet || opts.auto_mode,
                "Another lasapi-autosync run is already active; exiting cleanly.",
            );
            return ExitCode::SUCCESS;
        }
        eprintln!("Failed locking {}: {e}", lock_path.display());
        return ExitCode::from(1);
    }

    let code = run_sync_locked(
        &base_dir,
        &st_dir,
        &state_path,
        &fp,
        fp_changed,
        drifted,
        build_stale,
        &opts,
    );
    funlock_fd(lock_fd);
    ExitCode::from(code)
}

const fn libc_ewouldblock() -> c_int {
    11
}

#[allow(clippy::too_many_arguments)]
fn run_sync_locked(
    base_dir: &Path,
    st_dir: &Path,
    state_path: &Path,
    fp: &str,
    fp_changed: bool,
    drifted: bool,
    build_stale: bool,
    opts: &Opts,
) -> u8 {
    if !opts.force && !fp_changed && !drifted && !build_stale {
        if !opts.auto_mode {
            log_line(
                st_dir,
                opts.quiet,
                "No host module changes or stale artifacts detected.",
            );
        }
        return 0;
    }

    if drifted {
        log_line(
            st_dir,
            opts.quiet,
            "pipewire_zero_mode drift detected; re-applying lasapi-ctl enable-auto",
        );
        let ctl = base_dir.join("build/lasapi-ctl");
        if ctl.is_file() {
            let _ = Command::new(&ctl).arg("enable-auto").status();
        }
    }

    if !opts.force && !fp_changed && !build_stale {
        return 0;
    }

    if !wait_for_pkg_locks(st_dir, opts.quiet) {
        return 1;
    }

    let ver_changed = sync_version_scripts(base_dir, st_dir, opts.quiet);
    let make_target_flag = if opts.force || fp_changed || ver_changed {
        Some("-B")
    } else {
        None
    };

    log_line(
        st_dir,
        opts.quiet,
        &format!(
            "Rebuilding Salsa artifacts in {} (force={}, fp_changed={}, build_stale={})...",
            base_dir.display(),
            opts.force,
            fp_changed,
            build_stale
        ),
    );
    let jobs = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16);
    let mut cmd = Command::new("make");
    cmd.arg("-C")
        .arg(base_dir)
        .arg(format!("-j{jobs}"));
    if let Some(f) = make_target_flag {
        cmd.arg(f);
    }
    cmd.arg("all");
    match cmd.status() {
        Ok(st) if st.success() => {}
        Ok(st) => {
            log_line(
                st_dir,
                opts.quiet,
                &format!("make -C {} failed with status {:?}", base_dir.display(), st.code()),
            );
            return 1;
        }
        Err(e) => {
            log_line(st_dir, opts.quiet, &format!("failed running make: {e}"));
            return 1;
        }
    }

    let mut state_body = fp.to_string();
    state_body.push('\n');
    if let Err(e) = write_atomic_secure(state_path, state_body.as_bytes(), 0o600) {
        log_line(
            st_dir,
            opts.quiet,
            &format!("failed saving {}: {e}", state_path.display()),
        );
    }

    if !drifted && pipewire_zero_enabled(st_dir) && pipewire_zero_drifted(st_dir) {
        let ctl = base_dir.join("build/lasapi-ctl");
        if ctl.is_file() {
            let _ = Command::new(&ctl).arg("enable-auto").status();
        }
    }

    if systemd_is_active("lasapi-audiodg.service") {
        log_line(
            st_dir,
            opts.quiet,
            "Restarting active lasapi-audiodg.service...",
        );
        let _ = Command::new("systemctl")
            .args(["--user", "restart", "lasapi-audiodg.service"])
            .status();
    }
    if systemd_is_active("lasapi-tray.service") {
        log_line(
            st_dir,
            opts.quiet,
            "Restarting active lasapi-tray.service...",
        );
        let _ = Command::new("systemctl")
            .args(["--user", "restart", "lasapi-tray.service"])
            .status();
    }

    log_line(st_dir, opts.quiet, "Auto-sync complete.");
    0
}
