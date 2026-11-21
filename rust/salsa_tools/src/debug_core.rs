// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::{c_char, c_int, c_long, c_void};
use core::mem::{self, offset_of};
use core::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::collections::VecDeque;
use std::env;
use std::ffi::CString;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use salsa_wire::c_abi::{
    LwasapiMidiEvent, LwasapiPwPreviewShm, WaveFormatEx, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, LWASAPI_FMT_FLOAT32, LWASAPI_PW_PREVIEW_MAGIC,
    LWASAPI_PW_PREVIEW_MAX_BYTES, LWASAPI_PW_PREVIEW_MAX_H, LWASAPI_PW_PREVIEW_MAX_W,
    LWASAPI_PW_PREVIEW_VERSION,
};
use salsa_wire::unix_sys::{
    audiodg_sock_path, current_hostname, debug_in_sock_path, mono_ns,
    resolve_sock_path_common, MappedMem, O_CLOEXEC, O_NOFOLLOW,
};

use crate::host::{
    ensure_dir_0700, getuid, host_arch, host_cpu_name, host_os_pretty_name, install_root,
    peercred_for_unix_socket, private_runtime_dir, read_text_lossy, sha256_file, state_dir,
    uname_srm, which, xdg_runtime_dir,
};
use crate::json::JsonValue;
use crate::zlib_png::ZipWriter;

pub const SUBSYSTEMS: &[&str] = &[
    "ALL", "ENGINE", "WASAPI", "PA", "BLUEZ", "MIDI", "SHIM", "ALSA", "PW_VIDEO", "JOURNAL",
    "AUTOSYNC", "DBUS",
];
pub const LOG_LEVELS: &[&str] = &["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
pub const MAX_EVENT_RING: usize = 8192;

pub fn level_rank(lvl: &str) -> usize {
    let upper = lvl.trim().to_ascii_uppercase();
    match upper.as_str() {
        "TRACE" => 0,
        "DEBUG" => 1,
        "INFO" => 2,
        "WARN" | "WARNING" => 3,
        "ERROR" | "ERR" | "FATAL" => 4,
        _ => 2,
    }
}

pub fn resolve_debug_sock_path() -> String {
    resolve_sock_path_common("LWASAPI_DEBUG_SOCK", "debug.sock", "/tmp/lwasapi-debug.sock")
}

pub fn resolve_audiodg_sock_path() -> String {
    audiodg_sock_path()
}

pub fn resolve_debug_in_sock_path() -> String {
    debug_in_sock_path()
}

pub fn cstr_to_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[repr(C)]
struct LibcTm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

const RTLD_LAZY: c_int = 1;
const RTLD_LOCAL: c_int = 0;

extern "C" {
    fn localtime_r(timep: *const c_long, result: *mut LibcTm) -> *mut LibcTm;
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

pub fn format_ts_ms(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return "--:--:--.---".to_string();
    }
    let secs = (ts_ms / 1000) as c_long;
    let ms = ts_ms.rem_euclid(1000);
    let mut tm: LibcTm = unsafe { mem::zeroed() };
    let res = unsafe { localtime_r(&secs, &mut tm) };
    if res.is_null() {
        return format!("{:02}:{:02}:{:02}.{ms:03}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    }
    format!(
        "{:02}:{:02}:{:02}.{ms:03}",
        tm.tm_hour, tm.tm_min, tm.tm_sec
    )
}

fn format_now_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as c_long)
        .unwrap_or(0);
    let mut tm: LibcTm = unsafe { mem::zeroed() };
    let res = unsafe { localtime_r(&secs, &mut tm) };
    if res.is_null() {
        return format!("{secs}");
    }
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[derive(Debug, Clone)]
pub struct DebugClientStats {
    pub sock_path: String,
    pub connected: bool,
    pub last_error: String,
    pub total_events_rx: u64,
    pub dropped_events: u64,
    pub buffered_events: usize,
    pub has_snapshot: bool,
}

struct DebugClientState {
    sock_path: String,
    connected: bool,
    last_error: String,
    hello: Option<JsonValue>,
    latest_snapshot: Option<JsonValue>,
    snapshot_seq: u64,
    events: VecDeque<JsonValue>,
    total_events_rx: u64,
    dropped_events: u64,
    pongs: VecDeque<JsonValue>,
    cmd_replies: VecDeque<JsonValue>,
    writer: Option<UnixStream>,
}

struct DebugClientInner {
    state: Mutex<DebugClientState>,
    cv: Condvar,
    stop: AtomicBool,
}

pub struct SalsaDebugClient {
    inner: Arc<DebugClientInner>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl SalsaDebugClient {
    pub fn new(sock_path: Option<String>) -> Self {
        let path = sock_path.unwrap_or_else(resolve_debug_sock_path);
        let state = DebugClientState {
            sock_path: path,
            connected: false,
            last_error: String::new(),
            hello: None,
            latest_snapshot: None,
            snapshot_seq: 0,
            events: VecDeque::with_capacity(1024),
            total_events_rx: 0,
            dropped_events: 0,
            pongs: VecDeque::with_capacity(16),
            cmd_replies: VecDeque::with_capacity(16),
            writer: None,
        };
        Self {
            inner: Arc::new(DebugClientInner {
                state: Mutex::new(state),
                cv: Condvar::new(),
                stop: AtomicBool::new(false),
            }),
            worker: Mutex::new(None),
        }
    }

    pub fn start(&self) {
        let mut guard = self.worker.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_some() {
            return;
        }
        self.inner.stop.store(false, Ordering::Release);
        let inner = Arc::clone(&self.inner);
        let handle = thread::Builder::new()
            .name("salsa-dbg-rx".into())
            .spawn(move || Self::io_loop(inner))
            .ok();
        *guard = handle;
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Release);
        {
            let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(w) = st.writer.take() {
                let _ = w.shutdown(std::net::Shutdown::Both);
            }
            self.inner.cv.notify_all();
        }
        let handle = self
            .worker
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(h) = handle {
            let _ = h.join();
        }
    }

    fn io_loop(inner: Arc<DebugClientInner>) {
        while !inner.stop.load(Ordering::Acquire) {
            let path = {
                let st = inner.state.lock().unwrap_or_else(|p| p.into_inner());
                st.sock_path.clone()
            };
            match UnixStream::connect(&path) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                    let writer = stream.try_clone().ok();
                    {
                        let mut st = inner.state.lock().unwrap_or_else(|p| p.into_inner());
                        st.connected = true;
                        st.last_error.clear();
                        st.writer = writer;
                        inner.cv.notify_all();
                    }
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    while !inner.stop.load(Ordering::Acquire) {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) => break,
                            Ok(_) => {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                if let Ok(val) = JsonValue::parse(trimmed) {
                                    Self::handle_msg(&inner, val);
                                }
                            }
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    || e.kind() == std::io::ErrorKind::TimedOut =>
                            {
                                continue;
                            }
                            Err(e) => {
                                let mut st =
                                    inner.state.lock().unwrap_or_else(|p| p.into_inner());
                                st.last_error = e.to_string();
                                break;
                            }
                        }
                    }
                    let mut st = inner.state.lock().unwrap_or_else(|p| p.into_inner());
                    st.connected = false;
                    st.writer = None;
                    inner.cv.notify_all();
                }
                Err(e) => {
                    {
                        let mut st = inner.state.lock().unwrap_or_else(|p| p.into_inner());
                        st.connected = false;
                        st.writer = None;
                        st.last_error = format!("connect {path}: {e}");
                    }
                    thread::sleep(Duration::from_millis(350));
                }
            }
        }
    }

    fn handle_msg(inner: &Arc<DebugClientInner>, val: JsonValue) {
        let msg_type = val.get("type").and_then(JsonValue::as_str).unwrap_or("");
        let mut st = inner.state.lock().unwrap_or_else(|p| p.into_inner());
        match msg_type {
            "hello" => {
                st.hello = Some(val);
            }
            "telemetry" => {
                if let Some(d) = val.get("dropped_events").and_then(JsonValue::as_u64) {
                    st.dropped_events = d;
                }
                st.latest_snapshot = Some(val);
                st.snapshot_seq += 1;
                inner.cv.notify_all();
            }
            "event" => {
                st.total_events_rx += 1;
                if st.events.len() >= MAX_EVENT_RING {
                    st.events.pop_front();
                }
                st.events.push_back(val);
                inner.cv.notify_all();
            }
            "pong" => {
                if st.pongs.len() >= 32 {
                    st.pongs.pop_front();
                }
                st.pongs.push_back(val);
                inner.cv.notify_all();
            }
            "cmd_reply" => {
                if st.cmd_replies.len() >= 32 {
                    st.cmd_replies.pop_front();
                }
                st.cmd_replies.push_back(val);
                inner.cv.notify_all();
            }
            _ => {}
        }
    }

    pub fn wait_connected(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        while !st.connected {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, res) = self
                .inner
                .cv
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|p| p.into_inner());
            st = guard;
            if res.timed_out() && !st.connected {
                return false;
            }
        }
        true
    }

    pub fn send_command(&self, cmd: &str) -> Result<(), String> {
        let mut writer = {
            let st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
            st.writer
                .as_ref()
                .and_then(|s| s.try_clone().ok())
                .ok_or_else(|| "not connected to debug socket".to_string())?
        };
        let mut line = cmd.trim_end_matches(['\r', '\n']).to_string();
        line.push('\n');
        writer
            .write_all(line.as_bytes())
            .map_err(|e| format!("send '{cmd}': {e}"))
    }

    pub fn request_snapshot(&self) -> Result<(), String> {
        self.send_command("SNAPSHOT")
    }

    pub fn wait_snapshot(&self, timeout: Duration) -> Option<JsonValue> {
        let deadline = Instant::now() + timeout;
        let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(ref snap) = st.latest_snapshot {
            return Some(snap.clone());
        }
        let seq_start = st.snapshot_seq;
        while st.snapshot_seq == seq_start && st.latest_snapshot.is_none() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (guard, _) = self
                .inner
                .cv
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|p| p.into_inner());
            st = guard;
        }
        st.latest_snapshot.clone()
    }

    pub fn ping(&self, tag: &str, timeout: Duration) -> Option<JsonValue> {
        {
            let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
            st.pongs.clear();
        }
        let cmd = if tag.is_empty() {
            "PING".to_string()
        } else {
            format!("PING {tag}")
        };
        self.send_command(&cmd).ok()?;
        let deadline = Instant::now() + timeout;
        let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            while let Some(pong) = st.pongs.pop_front() {
                if tag.is_empty() || pong.get("tag").and_then(JsonValue::as_str) == Some(tag) {
                    return Some(pong);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self
                .inner
                .cv
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|p| p.into_inner());
            st = guard;
        }
    }

    pub fn reset_stats(&self) -> Option<JsonValue> {
        let prev_seq = {
            let st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
            st.snapshot_seq
        };
        self.send_command("RESET_STATS").ok()?;
        let deadline = Instant::now() + Duration::from_millis(1200);
        let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        while st.snapshot_seq == prev_seq {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (guard, _) = self
                .inner
                .cv
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|p| p.into_inner());
            st = guard;
        }
        st.latest_snapshot.clone()
    }

    pub fn reset_pa_clients(&self) -> Option<JsonValue> {
        self.send_cmd_and_wait_reply("RESET_PA_CLIENTS")
    }

    pub fn reset_net_clients(&self) -> Option<JsonValue> {
        self.send_cmd_and_wait_reply("RESET_NET_CLIENTS")
    }

    fn send_cmd_and_wait_reply(&self, cmd: &str) -> Option<JsonValue> {
        {
            let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
            st.cmd_replies.clear();
        }
        self.send_command(cmd).ok()?;
        let deadline = Instant::now() + Duration::from_millis(1200);
        let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        while st.cmd_replies.is_empty() {
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self
                .inner
                .cv
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|p| p.into_inner());
            st = guard;
        }
        st.cmd_replies.pop_front()
    }

    pub fn drain_events(&self, max_items: usize) -> Vec<JsonValue> {
        let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        let n = st.events.len().min(max_items);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(ev) = st.events.pop_front() {
                out.push(ev);
            }
        }
        out
    }

    pub fn recent_events(&self, max_items: usize) -> Vec<JsonValue> {
        let st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        let skip = st.events.len().saturating_sub(max_items);
        st.events.iter().skip(skip).cloned().collect()
    }

    pub fn latest_snapshot(&self) -> Option<JsonValue> {
        let st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        st.latest_snapshot.clone()
    }

    pub fn stats(&self) -> DebugClientStats {
        let st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        DebugClientStats {
            sock_path: st.sock_path.clone(),
            connected: st.connected,
            last_error: st.last_error.clone(),
            total_events_rx: st.total_events_rx,
            dropped_events: st.dropped_events,
            buffered_events: st.events.len(),
            has_snapshot: st.latest_snapshot.is_some(),
        }
    }
}

impl Drop for SalsaDebugClient {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn snapshot_once(sock_path: Option<&str>, timeout: Duration) -> Result<JsonValue, String> {
    let path = sock_path
        .map(str::to_string)
        .unwrap_or_else(resolve_debug_sock_path);
    let mut stream =
        UnixStream::connect(&path).map_err(|e| format!("connect {path}: {e}"))?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.write_all(b"SNAPSHOT\n");
    let deadline = Instant::now() + timeout;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if let Ok(val) = JsonValue::parse(line.trim()) {
                    if val.get("type").and_then(JsonValue::as_str) == Some("telemetry") {
                        return Ok(val);
                    }
                }
            }
            Err(e) => return Err(format!("read {path}: {e}")),
        }
    }
    Err(format!("timed out waiting for SNAPSHOT telemetry from {path}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtUsbAdapterPower {
    pub hci: String,
    pub usb_id: String,
    pub power_control: String,
    pub suspended_ms: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtUsbPowerReport {
    pub btusb_autosuspend: Option<bool>,
    pub adapters: Vec<BtUsbAdapterPower>,
    pub autosuspend_risk: bool,
    pub summary: String,
}

pub struct SystemAuditScanner;

impl SystemAuditScanner {
    pub fn check_build_freshness(base_dir: &Path) -> JsonValue {
        let mut stale = Vec::new();
        let net_deps = [
            "rust/Cargo.toml",
            "rust/salsa_net/Cargo.toml",
            "rust/salsa_net/src",
        ];
        let client_deps = [
            "rust/Cargo.toml",
            "rust/salsa_net/Cargo.toml",
            "rust/salsa_net/src",
            "rust/lwasapi_client/Cargo.toml",
            "rust/lwasapi_client/src",
        ];
        let tools_deps = [
            "rust/Cargo.toml",
            "rust/salsa_net/Cargo.toml",
            "rust/salsa_net/src",
            "rust/lwasapi_client/Cargo.toml",
            "rust/lwasapi_client/src",
            "rust/salsa_tools/Cargo.toml",
            "rust/salsa_tools/src",
        ];
        let ui_shared_deps = [
            "Makefile",
            "qt6/salsa_bridge.h",
            "qt6/salsa_common.hpp",
            "qt6/salsa_common.cpp",
            "rust/Cargo.toml",
            "rust/Cargo.lock",
            "rust/salsa_net/Cargo.toml",
            "rust/salsa_net/src",
            "rust/lwasapi_client/Cargo.toml",
            "rust/lwasapi_client/src",
            "rust/salsa_tools/Cargo.toml",
            "rust/salsa_tools/src",
            "rust/salsa_ui/Cargo.toml",
            "rust/salsa_ui/src",
        ];
        let mut tray_deps = ui_shared_deps.to_vec();
        tray_deps.push("qt6/lwasapi_tray.cpp");
        let mut debug_deps = ui_shared_deps.to_vec();
        debug_deps.extend([
            "qt6/salsa_widgets.hpp",
            "qt6/salsa_widgets.cpp",
            "qt6/salsa_debugger_widget.cpp",
            "qt6/salsa_debug.cpp",
        ]);
        let mut studio_deps = ui_shared_deps.to_vec();
        studio_deps.extend([
            "qt6/salsa_widgets.hpp",
            "qt6/salsa_widgets.cpp",
            "qt6/salsa_debugger_widget.cpp",
            "qt6/salsa_studio.cpp",
        ]);

        let is_dev_repo = base_dir.join("rust/Cargo.toml").is_file();
        let mut lwasapi_deps = client_deps.to_vec();
        lwasapi_deps.push("src/lwasapi.version");

        let mut checks: Vec<(&str, Vec<&str>)> = vec![
            ("build/lwasapi-audiodg", net_deps.to_vec()),
            ("build/lwasapi-ctl", net_deps.to_vec()),
            ("build/liblwasapi.so", lwasapi_deps),
            ("build/lwasapi-test", client_deps.to_vec()),
            ("build/lwasapi-bench", client_deps.to_vec()),
            ("build/lwasapi-run", tools_deps.to_vec()),
            ("build/lwasapi-autosync", tools_deps.to_vec()),
            ("build/salsa-installer", tools_deps.to_vec()),
            ("build/salsa-debug-cli", tools_deps.to_vec()),
            ("build/lwasapi-tray", tray_deps),
            ("build/salsa-debug", debug_deps),
            ("build/salsa-studio", studio_deps),
        ];

        for (rel, deps) in [
            ("build/salsa-gen-pa-proxy", net_deps.to_vec()),
            ("build/salsa-dist", tools_deps.to_vec()),
            ("build/salsa-regression", tools_deps.to_vec()),
        ] {
            if is_dev_repo || base_dir.join(rel).exists() {
                checks.push((rel, deps));
            }
        }

        let mut alsa_plugin_deps = client_deps.to_vec();
        alsa_plugin_deps.extend([
            "rust/alsa_plugin_lwasapi/Cargo.toml",
            "rust/alsa_plugin_lwasapi/src",
            "src/alsa_plugin.version",
        ]);
        checks.push(("build/libasound_module_pcm_lwasapi.so", alsa_plugin_deps));

        checks.push((
            "build/libsalsa_pa_reconnect.so",
            vec![
                "rust/Cargo.toml",
                "rust/salsa_pa_reconnect/Cargo.toml",
                "rust/salsa_pa_reconnect/src",
                "src/pa_reconnect.version",
            ],
        ));
        checks.push((
            "build/pa_reconnect/libpulse.so.0",
            vec![
                "rust/Cargo.toml",
                "rust/salsa_pa_reconnect_proxy/Cargo.toml",
                "rust/salsa_pa_reconnect_proxy/src",
                "src/pulse.version",
            ],
        ));

        let mut shim_alsa_deps = client_deps.to_vec();
        shim_alsa_deps.extend([
            "rust/shim_alsa/Cargo.toml",
            "rust/shim_alsa/src",
            "src/alsa.version",
        ]);
        checks.push(("build/shims/libasound.so.2", shim_alsa_deps));

        let mut shim_pulse_deps = client_deps.to_vec();
        shim_pulse_deps.extend([
            "rust/shim_pulse/Cargo.toml",
            "rust/shim_pulse/src",
            "src/pulse.version",
        ]);
        checks.push(("build/shims/libpulse.so.0", shim_pulse_deps.clone()));
        checks.push(("build/shims/libpulse-simple.so.0", shim_pulse_deps.clone()));
        checks.push(("build/shims/libpulse-mainloop-glib.so.0", shim_pulse_deps));

        let mut shim_jack_deps = client_deps.to_vec();
        shim_jack_deps.extend([
            "rust/shim_jack/Cargo.toml",
            "rust/shim_jack/src",
            "src/jack.version",
        ]);
        checks.push(("build/shims/libjack.so.0", shim_jack_deps));

        let checked_count = checks.len();
        for (art_rel, deps) in checks {
            let art_path = base_dir.join(art_rel);
            let Ok(art_meta) = fs::metadata(&art_path) else {
                stale.push(JsonValue::obj(vec![
                    ("artifact", JsonValue::Str(art_rel.to_string())),
                    ("reason", JsonValue::Str("missing".to_string())),
                ]));
                continue;
            };
            let Ok(art_mtime) = art_meta.modified() else {
                continue;
            };
            let mut newest: Option<(String, SystemTime)> = None;
            for dep in deps {
                let dep_path = base_dir.join(dep);
                collect_newest_mtime(&dep_path, &mut newest);
            }
            if let Some((src_rel, src_mtime)) = newest {
                if src_mtime > art_mtime {
                    stale.push(JsonValue::obj(vec![
                        ("artifact", JsonValue::Str(art_rel.to_string())),
                        ("reason", JsonValue::Str(format!("older than {src_rel}"))),
                    ]));
                }
            }
        }

        let is_stale = !stale.is_empty();
        let summary = if is_stale {
            format!("{} of {checked_count} artifacts stale or missing", stale.len())
        } else {
            format!("all {checked_count} build artifacts up to date")
        };
        JsonValue::obj(vec![
            ("stale", JsonValue::Bool(is_stale)),
            ("checked_count", JsonValue::Int(checked_count as i64)),
            ("stale_artifacts", JsonValue::Array(stale)),
            ("summary", JsonValue::Str(summary)),
        ])
    }

    pub fn inspect_bt_usb_power() -> BtUsbPowerReport {
        Self::inspect_bt_usb_power_in(Path::new("/sys"))
    }

    /// Checks `btusb` module autosuspend and per-HCI USB `power/control` sysfs attributes.
    pub fn inspect_bt_usb_power_in(sys_root: &Path) -> BtUsbPowerReport {
        let btusb_param = read_text_lossy(
            &sys_root.join("module/btusb/parameters/enable_autosuspend"),
            128,
        );
        let btusb_autosuspend = match btusb_param.trim() {
            "Y" | "y" | "1" => Some(true),
            "N" | "n" | "0" => Some(false),
            _ => None,
        };

        let mut adapters = Vec::new();
        let bt_class = sys_root.join("class/bluetooth");
        if let Ok(rd) = fs::read_dir(&bt_class) {
            for ent in rd.flatten() {
                let hci = ent.file_name().to_string_lossy().into_owned();
                if !hci.starts_with("hci") || hci.contains(':') {
                    continue;
                }
                let dev_link = ent.path().join("device");
                let resolved = fs::canonicalize(&dev_link).unwrap_or(dev_link);
                let usb_root = resolved
                    .parent()
                    .filter(|p| p.join("power/control").is_file() || p.join("idVendor").is_file())
                    .map(Path::to_path_buf)
                    .unwrap_or(resolved);
                let ctrl = read_text_lossy(&usb_root.join("power/control"), 128)
                    .trim()
                    .to_string();
                if ctrl.is_empty() {
                    continue;
                }
                let vid = read_text_lossy(&usb_root.join("idVendor"), 128)
                    .trim()
                    .to_string();
                let pid = read_text_lossy(&usb_root.join("idProduct"), 128)
                    .trim()
                    .to_string();
                let usb_id = if !vid.is_empty() && !pid.is_empty() {
                    format!("{vid}:{pid}")
                } else {
                    String::new()
                };
                let suspended_ms =
                    read_text_lossy(&usb_root.join("power/runtime_suspended_time"), 128)
                        .trim()
                        .to_string();
                adapters.push(BtUsbAdapterPower {
                    hci,
                    usb_id,
                    power_control: ctrl,
                    suspended_ms,
                });
            }
        }
        adapters.sort_by(|a, b| a.hci.cmp(&b.hci));

        let auto_ctrl = adapters.iter().any(|a| a.power_control == "auto");
        let autosuspend_risk = btusb_autosuspend == Some(true) || auto_ctrl;
        let mod_str = match btusb_autosuspend {
            Some(true) => "btusb.enable_autosuspend=Y",
            Some(false) => "btusb.enable_autosuspend=N",
            None => "btusb.enable_autosuspend=n/a",
        };
        let devs = if adapters.is_empty() {
            "none".to_string()
        } else {
            adapters
                .iter()
                .map(|a| {
                    if a.usb_id.is_empty() {
                        format!("{}(power={})", a.hci, a.power_control)
                    } else {
                        format!(
                            "{}[{}](power={},susp_ms={})",
                            a.hci,
                            a.usb_id,
                            a.power_control,
                            if a.suspended_ms.is_empty() { "0" } else { &a.suspended_ms }
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let summary = format!("{mod_str}; {devs}");
        BtUsbPowerReport {
            btusb_autosuspend,
            adapters,
            autosuspend_risk,
            summary,
        }
    }

    pub fn collect_full_audit() -> JsonValue {
        let base_dir = install_root();
        let xdg_rt = xdg_runtime_dir();
        let priv_rt = private_runtime_dir();

        let host_obj = JsonValue::obj(vec![
            ("os", JsonValue::Str(host_os_pretty_name())),
            ("kernel", JsonValue::Str(uname_srm())),
            ("arch", JsonValue::Str(host_arch())),
            ("cpu", JsonValue::Str(host_cpu_name())),
            ("hostname", JsonValue::Str(current_hostname())),
            ("uid", JsonValue::Int(i64::from(getuid()))),
            ("base_dir", JsonValue::Str(base_dir.to_string_lossy().into_owned())),
            ("xdg_runtime_dir", JsonValue::Str(xdg_rt.to_string_lossy().into_owned())),
            ("private_runtime_dir", JsonValue::Str(priv_rt.to_string_lossy().into_owned())),
        ]);

        let sock_specs = [
            ("audiodg", PathBuf::from(resolve_audiodg_sock_path())),
            ("debug", PathBuf::from(resolve_debug_sock_path())),
            ("debug_in", PathBuf::from(resolve_debug_in_sock_path())),
            ("pulse_native", xdg_rt.join("pulse").join("native")),
            ("pipewire_0", xdg_rt.join("pipewire-0")),
            ("pipewire_0_manager", xdg_rt.join("pipewire-0-manager")),
        ];
        let mut sockets_arr = Vec::with_capacity(sock_specs.len());
        let mut pw_is_salsa = false;
        for (name, path) in &sock_specs {
            let info = peercred_for_unix_socket(path);
            if *name == "pipewire_0" && info.peer_comm == "lwasapi-audiodg" {
                pw_is_salsa = true;
            }
            sockets_arr.push(JsonValue::obj(vec![
                ("name", JsonValue::Str((*name).to_string())),
                ("path", JsonValue::Str(info.path)),
                ("exists", JsonValue::Bool(info.exists)),
                ("is_socket", JsonValue::Bool(info.is_socket)),
                (
                    "mode_oct",
                    info.mode_oct.map(JsonValue::Str).unwrap_or(JsonValue::Null),
                ),
                ("connectable", JsonValue::Bool(info.connectable)),
                (
                    "peer_pid",
                    info.peer_pid.map(|p| JsonValue::Int(i64::from(p))).unwrap_or(JsonValue::Null),
                ),
                (
                    "peer_uid",
                    info.peer_uid.map(|u| JsonValue::Int(i64::from(u))).unwrap_or(JsonValue::Null),
                ),
                (
                    "peer_gid",
                    info.peer_gid.map(|g| JsonValue::Int(i64::from(g))).unwrap_or(JsonValue::Null),
                ),
                ("peer_comm", JsonValue::Str(info.peer_comm)),
                ("error", JsonValue::Str(info.error)),
            ]));
        }

        let unit_names = [
            "lwasapi-audiodg.service",
            "lwasapi-tray.service",
            "lwasapi-module-watcher.path",
            "lwasapi-module-watcher.service",
            "lwasapi-module-watcher.timer",
            "pipewire.service",
            "pipewire.socket",
            "pipewire-pulse.service",
            "pipewire-pulse.socket",
            "wireplumber.service",
            "pulseaudio.service",
            "pulseaudio.socket",
        ];
        let mut units_arr = Vec::with_capacity(unit_names.len());
        for unit in unit_names {
            units_arr.push(query_systemd_unit(unit));
        }

        let procs_arr = scan_audio_processes();
        let unix_socks = scan_ss_unix_sockets();
        let alsa_obj = scan_alsa_proc();
        let configs_obj = scan_configs(&base_dir);
        let freshness_obj = Self::check_build_freshness(&base_dir);

        let pw_note = if pw_is_salsa {
            "pipewire-0 is actively served by Salsa Video-Only ScreenCast broker (lwasapi-audiodg)"
        } else if xdg_rt.join("pipewire-0").exists() {
            "pipewire-0 is served by an external PipeWire instance"
        } else {
            "pipewire-0 is not active"
        };

        JsonValue::obj(vec![
            ("host", host_obj),
            ("sockets", JsonValue::Array(sockets_arr)),
            ("systemd_units", JsonValue::Array(units_arr)),
            ("processes", JsonValue::Array(procs_arr)),
            ("unix_sockets", JsonValue::Array(unix_socks)),
            ("alsa", alsa_obj),
            ("configs", configs_obj),
            ("build_freshness", freshness_obj),
            ("pipewire_note", JsonValue::Str(pw_note.to_string())),
        ])
    }
}

fn collect_newest_mtime(path: &Path, newest: &mut Option<(String, SystemTime)>) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_dir() {
        let Ok(rd) = fs::read_dir(path) else {
            return;
        };
        for ent in rd.flatten() {
            let name = ent.file_name();
            let s = name.to_string_lossy();
            if s == "__pycache__" || s == "target" || s == ".git" {
                continue;
            }
            collect_newest_mtime(&ent.path(), newest);
        }
    } else if meta.is_file() {
        if let Ok(mt) = meta.modified() {
            let replace = newest.as_ref().map(|(_, cur)| mt > *cur).unwrap_or(true);
            if replace {
                *newest = Some((path.to_string_lossy().into_owned(), mt));
            }
        }
    }
}

fn query_systemd_unit(unit: &str) -> JsonValue {
    let mut active = "unknown".to_string();
    let mut sub_state = "unknown".to_string();
    let mut enabled = "unknown".to_string();
    let mut main_pid: i64 = 0;

    if let Ok(out) = Command::new("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "--property=ActiveState,SubState,UnitFileState,MainPID",
        ])
        .output()
    {
        let txt = String::from_utf8_lossy(&out.stdout);
        for line in txt.lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k {
                    "ActiveState" => active = v.trim().to_string(),
                    "SubState" => sub_state = v.trim().to_string(),
                    "UnitFileState" => enabled = v.trim().to_string(),
                    "MainPID" => main_pid = v.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
    }

    JsonValue::obj(vec![
        ("unit", JsonValue::Str(unit.to_string())),
        ("active", JsonValue::Str(active)),
        ("sub_state", JsonValue::Str(sub_state)),
        ("enabled", JsonValue::Str(enabled)),
        ("main_pid", JsonValue::Int(main_pid)),
    ])
}

fn scan_audio_processes() -> Vec<JsonValue> {
    scan_process_routing_audit()
}

pub fn scan_process_routing_audit() -> Vec<JsonValue> {
    let needles = [
        "lwasapi",
        "salsa",
        "pipewire",
        "wireplumber",
        "pulseaudio",
        "pulse",
        "alsa",
        "jack",
        "xdg-desktop-portal",
    ];
    let lib_markers = [
        "liblwasapi",
        "shims/lib",
        "pa_reconnect",
        "libasound",
        "libpulse",
        "libjack",
        "libpipewire",
    ];
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return out;
    };
    for ent in rd.flatten() {
        let fname = ent.file_name();
        let pid_str = fname.to_string_lossy();
        let Ok(pid) = pid_str.parse::<i64>() else {
            continue;
        };
        let p = ent.path();
        let comm = read_text_lossy(&p.join("comm"), 128).trim().to_string();
        if comm.is_empty() {
            continue;
        }
        let cmdline_raw = fs::read(p.join("cmdline")).unwrap_or_default();
        let cmdline = cmdline_raw
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect::<Vec<_>>()
            .join(" ");

        let mut sockets_shm = Vec::new();
        let mut alsa_fds = Vec::new();
        let mut saw_salsa_fd = false;
        if let Ok(fd_rd) = fs::read_dir(p.join("fd")) {
            for f_ent in fd_rd.flatten().take(256) {
                if let Ok(tgt) = fs::read_link(f_ent.path()) {
                    let s = tgt.to_string_lossy();
                    if s.contains("/dev/snd/") {
                        alsa_fds.push(s.into_owned());
                    } else if s.contains("memfd:") || s.contains("lwasapi") || s.contains("salsa") {
                        if s.contains("lwasapi") || s.contains("salsa") {
                            saw_salsa_fd = true;
                        }
                        if sockets_shm.len() < 16 {
                            sockets_shm.push(s.into_owned());
                        }
                    } else if s.contains("socket:[") && sockets_shm.len() < 12 {
                        sockets_shm.push(s.into_owned());
                    }
                }
            }
        }

        let mut mapped_libs = Vec::new();
        let maps_txt = read_text_lossy(&p.join("maps"), 512 * 1024);
        for line in maps_txt.lines() {
            if let Some(path_part) = line.split_whitespace().last() {
                if path_part.starts_with('/')
                    && lib_markers.iter().any(|m| path_part.contains(m))
                    && !mapped_libs.iter().any(|x| x == path_part)
                {
                    mapped_libs.push(path_part.to_string());
                    if mapped_libs.len() >= 16 {
                        break;
                    }
                }
            }
        }

        let lc_comm = comm.to_ascii_lowercase();
        let lc_cmd = cmdline.to_ascii_lowercase();
        let relevant = !alsa_fds.is_empty()
            || !mapped_libs.is_empty()
            || needles
                .iter()
                .any(|n| lc_comm.contains(n) || lc_cmd.contains(n));
        if !relevant {
            continue;
        }

        let is_daemon = lc_comm.contains("lwasapi-audiodg") || lc_cmd.contains("lwasapi-audiodg");
        let has_salsa_map = mapped_libs
            .iter()
            .any(|l| l.contains("liblwasapi") || l.contains("shims/") || l.contains("pa_reconnect"));
        let has_salsa_name = lc_comm.contains("lwasapi") || lc_comm.contains("salsa") || saw_salsa_fd;
        let classification = if is_daemon {
            "Salsa Daemon"
        } else if has_salsa_map || has_salsa_name {
            "Salsa Routed"
        } else if !alsa_fds.is_empty() {
            "Direct ALSA Leak"
        } else {
            "Audio Client"
        };

        let detail = format!(
            "PID {pid} ({comm})\nClassification: {classification}\nCmdline: {cmdline}\nALSA /dev/snd FDs: {}\nSockets/SHMs: {}\nMapped Audio Libraries:\n  {}",
            if alsa_fds.is_empty() { "(none)".to_string() } else { alsa_fds.join(", ") },
            if sockets_shm.is_empty() { "(none)".to_string() } else { sockets_shm.join(", ") },
            if mapped_libs.is_empty() { "(none)".to_string() } else { mapped_libs.join("\n  ") },
        );

        out.push(JsonValue::obj(vec![
            ("pid", JsonValue::Int(pid)),
            ("comm", JsonValue::Str(comm)),
            ("classification", JsonValue::Str(classification.to_string())),
            ("sockets_shm", JsonValue::Str(sockets_shm.join(", "))),
            ("alsa_fds", JsonValue::Str(alsa_fds.join(", "))),
            ("mapped_libs", JsonValue::Str(mapped_libs.join(", "))),
            ("cmdline", JsonValue::Str(cmdline)),
            ("detail", JsonValue::Str(detail)),
        ]));
    }
    out
}

fn strip_ansi_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for ch in chars.by_ref() {
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn scan_ss_unix_sockets() -> Vec<JsonValue> {
    let mut out = Vec::new();
    if which("ss").is_none() {
        return out;
    }
    let Ok(res) = Command::new("ss").args(["-xanH"]).output() else {
        return out;
    };
    let txt = String::from_utf8_lossy(&res.stdout);
    for line in txt.lines() {
        let lc = line.to_ascii_lowercase();
        if lc.contains("lwasapi")
            || lc.contains("salsa")
            || lc.contains("pulse")
            || lc.contains("pipewire")
        {
            out.push(JsonValue::Str(line.trim().to_string()));
        }
    }
    out
}

fn scan_alsa_proc() -> JsonValue {
    let cards = read_text_lossy(Path::new("/proc/asound/cards"), 16384);
    let pcm = read_text_lossy(Path::new("/proc/asound/pcm"), 16384);
    let mut active_streams = Vec::new();
    if let Ok(rd) = fs::read_dir("/proc/asound") {
        for card_ent in rd.flatten() {
            let cname = card_ent.file_name();
            let cstr = cname.to_string_lossy();
            if !cstr.starts_with("card") || !card_ent.path().is_dir() {
                continue;
            }
            let Ok(pcms) = fs::read_dir(card_ent.path()) else {
                continue;
            };
            for pcm_ent in pcms.flatten() {
                let pname = pcm_ent.file_name();
                let pstr = pname.to_string_lossy();
                if !pstr.starts_with("pcm") || !pcm_ent.path().is_dir() {
                    continue;
                }
                let Ok(subs) = fs::read_dir(pcm_ent.path()) else {
                    continue;
                };
                for sub_ent in subs.flatten() {
                    let sname = sub_ent.file_name();
                    if !sname.to_string_lossy().starts_with("sub") {
                        continue;
                    }
                    let sp = sub_ent.path();
                    let status = read_text_lossy(&sp.join("status"), 2048);
                    let trimmed = status.trim();
                    if trimmed.is_empty() || trimmed == "closed" {
                        continue;
                    }
                    let hw_params = read_text_lossy(&sp.join("hw_params"), 2048);
                    active_streams.push(JsonValue::obj(vec![
                        (
                            "stream",
                            JsonValue::Str(format!("{cstr}/{pstr}/{}", sname.to_string_lossy())),
                        ),
                        ("status", JsonValue::Str(trimmed.to_string())),
                        ("hw_params", JsonValue::Str(hw_params.trim().to_string())),
                    ]));
                }
            }
        }
    }
    JsonValue::obj(vec![
        ("cards", JsonValue::Str(cards)),
        ("pcm", JsonValue::Str(pcm)),
        ("active_streams", JsonValue::Array(active_streams)),
    ])
}

fn scan_configs(base_dir: &Path) -> JsonValue {
    let home = crate::host::home_dir();
    let st_dir = state_dir();
    let cfg_home = env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| s.starts_with('/'))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));

    let repo_plugin = base_dir.join("build/libasound_module_pcm_lwasapi.so");
    let sys_plugin = PathBuf::from("/usr/lib/alsa-lib/libasound_module_pcm_lwasapi.so");
    let repo_sha = sha256_file(&repo_plugin);
    let sys_sha = sha256_file(&sys_plugin);
    let plugin_matches = repo_sha.is_some() && repo_sha == sys_sha;

    let mut state_files = Vec::new();
    for fname in [
        "gui_settings.json",
        "pipewire_zero_mode",
        "module_versions.state",
        "autosync.log",
        "plasmaparc_osd.state",
        "flatpak_overrides.list",
    ] {
        let p = st_dir.join(fname);
        state_files.push(JsonValue::obj(vec![
            ("name", JsonValue::Str(fname.to_string())),
            ("path", JsonValue::Str(p.to_string_lossy().into_owned())),
            ("exists", JsonValue::Bool(p.exists())),
        ]));
    }

    JsonValue::obj(vec![
        (
            "asoundrc_exists",
            JsonValue::Bool(home.join(".asoundrc").exists()),
        ),
        (
            "user_asound_conf_exists",
            JsonValue::Bool(cfg_home.join("asound.conf").exists()),
        ),
        (
            "etc_asound_conf_exists",
            JsonValue::Bool(Path::new("/etc/asound.conf").exists()),
        ),
        (
            "plugin_repo_sha256",
            repo_sha.map(JsonValue::Str).unwrap_or(JsonValue::Null),
        ),
        (
            "plugin_sys_sha256",
            sys_sha.map(JsonValue::Str).unwrap_or(JsonValue::Null),
        ),
        ("plugin_matches_system", JsonValue::Bool(plugin_matches)),
        ("state_files", JsonValue::Array(state_files)),
    ])
}

pub struct ExternalLogManager;

impl ExternalLogManager {
    pub fn tail_journal(lines: usize) -> Vec<String> {
        let n = lines.clamp(1, 2000).to_string();
        let Ok(out) = Command::new("journalctl")
            .args([
                "--user",
                "-u",
                "lwasapi-audiodg.service",
                "-u",
                "lwasapi-tray.service",
                "-u",
                "lwasapi-module-watcher.service",
                "-n",
                &n,
                "--no-pager",
            ])
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    }

    pub fn tail_autosync_log(lines: usize) -> Vec<String> {
        let path = state_dir().join("autosync.log");
        let txt = read_text_lossy(&path, 512 * 1024);
        let all: Vec<&str> = txt.lines().collect();
        let skip = all.len().saturating_sub(lines);
        all.into_iter().skip(skip).map(str::to_string).collect()
    }

    pub fn capture_dbus_sample(timeout_ms: u64) -> Vec<String> {
        if which("timeout").is_none() || which("dbus-monitor").is_none() {
            return Vec::new();
        }
        let secs = ((timeout_ms.max(200) as f64) / 1000.0).min(10.0);
        let Ok(out) = Command::new("timeout")
            .args([
                &format!("{secs:.2}s"),
                "dbus-monitor",
                "--session",
                "type='signal'",
            ])
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .take(80)
            .map(str::to_string)
            .collect()
    }
}

pub struct DiagnosticProbes;

impl DiagnosticProbes {
    pub fn run_all_probes(base_dir: &Path) -> JsonValue {
        let ctl_path = base_dir.join("build/lwasapi-ctl");
        let ctl_ok = if ctl_path.is_file() {
            Command::new(&ctl_path)
                .arg("status")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        } else {
            false
        };

        let dbg_sock = resolve_debug_sock_path();
        let t0 = Instant::now();
        let ping_rtt_ms = probe_debug_ping(&dbg_sock).map(|_| t0.elapsed().as_secs_f64() * 1000.0);
        let snap_res = snapshot_once(Some(&dbg_sock), Duration::from_millis(900));

        let alsa_plugin = base_dir.join("build/libasound_module_pcm_lwasapi.so");
        let alsa_plugin_ok = dlopen_symbol_check(&alsa_plugin, "_snd_pcm_lwasapi_open");

        let liblwasapi = base_dir.join("build/liblwasapi.so");
        let liblwasapi_ok = dlopen_symbol_check(&liblwasapi, "LWASAPI_GetEngineStatus");

        let shim_alsa_ok = base_dir.join("build/shims/libasound.so.2").is_file();
        let shim_pulse_ok = base_dir.join("build/shims/libpulse.so.0").is_file();
        let shim_jack_ok = base_dir.join("build/shims/libjack.so.0").is_file();

        let xdg_rt = xdg_runtime_dir();
        let pa_sock = peercred_for_unix_socket(&xdg_rt.join("pulse/native"));
        let pw_sock = peercred_for_unix_socket(&xdg_rt.join("pipewire-0"));

        JsonValue::obj(vec![
            ("ctl_status_ok", JsonValue::Bool(ctl_ok)),
            ("debug_snapshot_ok", JsonValue::Bool(snap_res.is_ok())),
            (
                "debug_ping_rtt_ms",
                ping_rtt_ms.map(JsonValue::Float).unwrap_or(JsonValue::Null),
            ),
            ("alsa_plugin_dlopen_ok", JsonValue::Bool(alsa_plugin_ok)),
            ("liblwasapi_dlopen_ok", JsonValue::Bool(liblwasapi_ok)),
            ("shim_alsa_present", JsonValue::Bool(shim_alsa_ok)),
            ("shim_pulse_present", JsonValue::Bool(shim_pulse_ok)),
            ("shim_jack_present", JsonValue::Bool(shim_jack_ok)),
            ("pa_socket_connectable", JsonValue::Bool(pa_sock.connectable)),
            ("pa_socket_comm", JsonValue::Str(pa_sock.peer_comm)),
            ("pw_socket_connectable", JsonValue::Bool(pw_sock.connectable)),
            ("pw_socket_comm", JsonValue::Str(pw_sock.peer_comm)),
        ])
    }

    pub fn play_test_tone(freq_hz: f64, duration_ms: u32, amp: f32) -> Result<String, String> {
        let mut client: *mut lwasapi::IAudioClient = core::ptr::null_mut();
        let hr = unsafe {
            lwasapi::LWASAPI_ActivateAudioClient(c"salsa-debug-tone".as_ptr(), &mut client)
        };
        if hr < 0 || client.is_null() {
            return Err(format!("LWASAPI_ActivateAudioClient failed (0x{hr:08x})"));
        }

        let res = (|| {
            let mut wfx = WaveFormatEx::default();
            let hr_fmt = unsafe { lwasapi::IAudioClient_GetMixFormat(client, &mut wfx) };
            if hr_fmt < 0 {
                return Err(format!("GetMixFormat failed (0x{hr_fmt:08x})"));
            }
            let rate = wfx.n_samples_per_sec.max(44100);
            let ch = wfx.n_channels.clamp(1, 2) as usize;
            let period_frames = (rate / 100).max(64);
            let buf_frames = period_frames * 4;

            let hr_init = unsafe {
                lwasapi::IAudioClient_Initialize(
                    client,
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                    buf_frames,
                    period_frames,
                    &wfx,
                )
            };
            if hr_init < 0 {
                return Err(format!("Initialize failed (0x{hr_init:08x})"));
            }
            let _ = unsafe { lwasapi::ISimpleAudioVolume_SetMasterVolume(client, 1.0) };
            let hr_start = unsafe { lwasapi::IAudioClient_Start(client) };
            if hr_start < 0 {
                return Err(format!("Start failed (0x{hr_start:08x})"));
            }

            let total_frames = ((u64::from(rate) * u64::from(duration_ms.max(50))) / 1000) as usize;
            let fade_frames = (rate as usize / 100).max(16);
            let mut rendered = 0usize;
            let mut chunk = vec![0.0f32; period_frames as usize * ch];
            let safe_amp = amp.clamp(0.0, 0.9);
            while rendered < total_frames {
                let n = (total_frames - rendered).min(period_frames as usize);
                for i in 0..n {
                    let g = rendered + i;
                    let env = if g < fade_frames {
                        g as f32 / fade_frames as f32
                    } else if total_frames - g < fade_frames {
                        (total_frames - g) as f32 / fade_frames as f32
                    } else {
                        1.0
                    };
                    let phase = 2.0 * core::f64::consts::PI * freq_hz * (g as f64) / f64::from(rate);
                    let s = (phase.sin() as f32) * safe_amp * env;
                    for c in 0..ch {
                        chunk[i * ch + c] = s;
                    }
                }
                let hr_w = unsafe {
                    lwasapi::IAudioRenderClient_WriteFrames(
                        client,
                        chunk.as_ptr().cast::<c_void>(),
                        n as u32,
                        LWASAPI_FMT_FLOAT32,
                    )
                };
                if hr_w < 0 {
                    let _ = unsafe { lwasapi::IAudioClient_Stop(client) };
                    return Err(format!("WriteFrames failed (0x{hr_w:08x})"));
                }
                rendered += n;
                thread::sleep(Duration::from_millis((u64::from(n as u32) * 1000 / u64::from(rate)).max(2)));
            }
            thread::sleep(Duration::from_millis(60));
            let _ = unsafe { lwasapi::IAudioClient_Stop(client) };
            Ok(format!(
                "Rendered {rendered} frames ({:.1} Hz sine @ {rate} Hz, {ch}ch)",
                freq_hz
            ))
        })();

        unsafe {
            lwasapi::IAudioClient_Release(client);
        }
        res
    }

    pub fn send_test_midi_note(note: u8, velocity: u8, duration_ms: u32) -> Result<String, String> {
        let on = LwasapiMidiEvent {
            frame_offset: 0,
            size: 3,
            port: 0,
            data: [0x90, note & 0x7F, velocity.clamp(1, 127), 0],
            timestamp_ns: mono_ns(),
        };
        let hr_on = unsafe { lwasapi::IAudioClient_WriteMidiEvent(core::ptr::null_mut(), &on) };
        if hr_on < 0 {
            return Err(format!("WriteMidiEvent NoteOn failed (0x{hr_on:08x})"));
        }
        thread::sleep(Duration::from_millis(u64::from(duration_ms.clamp(20, 2000))));
        let off = LwasapiMidiEvent {
            frame_offset: 0,
            size: 3,
            port: 0,
            data: [0x80, note & 0x7F, 0, 0],
            timestamp_ns: mono_ns(),
        };
        let _ = unsafe { lwasapi::IAudioClient_WriteMidiEvent(core::ptr::null_mut(), &off) };
        Ok(format!(
            "Injected MIDI NoteOn/Off note={} vel={} ({} ms)",
            note & 0x7F,
            velocity.clamp(1, 127),
            duration_ms
        ))
    }

    pub fn build_detailed_text_report(
        base_dir: &Path,
        snapshot: Option<&JsonValue>,
        audit: Option<&JsonValue>,
        probes: Option<&JsonValue>,
        events: &[JsonValue],
    ) -> String {
        let mut out = String::with_capacity(32 * 1024);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let _ = writeln!(out, "================================================================================");
        let _ = writeln!(out, "SALSA (LWASAPI) DETAILED TEXT DIAGNOSTIC REPORT");
        let _ = writeln!(out, "Generated: {} (epoch_ms={now_ms})", format_ts_ms(now_ms));
        let _ = writeln!(out, "================================================================================\n");

        fn jstr<'a>(obj: &'a JsonValue, key: &str) -> &'a str {
            obj.get(key).and_then(JsonValue::as_str).unwrap_or("")
        }
        let s_str = jstr;
        let s_i64 = |obj: &JsonValue, key: &str| -> i64 {
            obj.get(key).and_then(JsonValue::as_i64).unwrap_or(0)
        };
        let s_f64 = |obj: &JsonValue, key: &str| -> f64 {
            obj.get(key).and_then(JsonValue::as_f64).unwrap_or(0.0)
        };
        let s_bool = |obj: &JsonValue, key: &str| -> bool {
            obj.get(key).and_then(JsonValue::as_bool).unwrap_or(false)
        };

        // [1] HOST, BUILD & ENVIRONMENT
        let _ = writeln!(out, "[1] HOST, ENVIRONMENT & BUILD FRESHNESS");
        if let Some(a) = audit {
            if let Some(h) = a.get("host") {
                let _ = writeln!(
                    out,
                    "  Host      : {} | OS={} | Kernel={} | Arch={}",
                    s_str(h, "hostname"),
                    s_str(h, "os"),
                    s_str(h, "kernel"),
                    s_str(h, "arch")
                );
                let _ = writeln!(out, "  CPU       : {}", s_str(h, "cpu"));
                let _ = writeln!(
                    out,
                    "  UID/Dirs  : uid={} root={} xdg_rt={} priv_rt={}",
                    s_i64(h, "uid"),
                    s_str(h, "base_dir"),
                    s_str(h, "xdg_runtime_dir"),
                    s_str(h, "private_runtime_dir")
                );
            }
            if let Some(bf) = a.get("build_freshness") {
                let _ = writeln!(
                    out,
                    "  Build     : stale={} checked={} summary={}",
                    s_bool(bf, "stale"),
                    s_i64(bf, "checked_count"),
                    s_str(bf, "summary")
                );
                if let Some(arr) = bf.get("stale_artifacts").and_then(JsonValue::as_array) {
                    for st in arr {
                        let _ = writeln!(out, "    * stale: {}", st.to_json_compact());
                    }
                }
            }
            let _ = writeln!(out, "  PW Note   : {}", s_str(a, "pipewire_note"));
        } else {
            let _ = writeln!(out, "  Root      : {}", base_dir.display());
        }
        let mem_info = read_text_lossy(Path::new("/proc/meminfo"), 4096);
        for line in mem_info.lines().take(5) {
            let _ = writeln!(out, "  MemInfo   : {}", line.trim());
        }
        if let Ok(rd) = fs::read_dir("/sys/class/drm") {
            let mut render_cards: Vec<String> = Vec::new();
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                if name.starts_with("renderD") {
                    let ue = read_text_lossy(&ent.path().join("device/uevent"), 2048);
                    let driver = ue.lines().find(|l| l.starts_with("DRIVER=")).unwrap_or("");
                    let pci_id = ue.lines().find(|l| l.starts_with("PCI_ID=")).unwrap_or("");
                    let slot = ue.lines().find(|l| l.starts_with("PCI_SLOT_NAME=")).unwrap_or("");
                    render_cards.push(format!("/dev/dri/{name} [{driver} {pci_id} {slot}]"));
                }
            }
            render_cards.sort();
            for rc in render_cards {
                let _ = writeln!(out, "  GPU Node  : {rc}");
            }
        }
        out.push('\n');

        // [2] DAEMON ENGINE, RT TIMING & FAILSAFE
        let _ = writeln!(out, "[2] ENGINE, DSP, RT TIMING & FAILSAFE");
        if let Some(snap) = snapshot {
            let ts = s_i64(snap, "ts_ms");
            let _ = writeln!(
                out,
                "  Snapshot  : ts={} ({ts}) mono_ns={} dropped_events={}",
                format_ts_ms(ts),
                s_i64(snap, "ts_mono_ns"),
                s_i64(snap, "dropped_events")
            );
            if let Some(eng) = snap.get("engine") {
                let _ = writeln!(
                    out,
                    "  Playback  : {} (id={}) | {} Hz / {}ch / {}-bit | period={} buf={}",
                    s_str(eng, "hw_device"),
                    s_str(eng, "active_playback_id"),
                    s_i64(eng, "sample_rate"),
                    s_i64(eng, "channels"),
                    s_i64(eng, "format_bits"),
                    s_i64(eng, "period_frames"),
                    s_i64(eng, "buffer_frames")
                );
                let _ = writeln!(
                    out,
                    "  CaptureHW : {} (id={}) | MIDI={}",
                    s_str(eng, "hw_capture_device"),
                    s_str(eng, "active_capture_id"),
                    s_str(eng, "midi_device")
                );
                let _ = writeln!(
                    out,
                    "  Levels    : out_vol={}% muted={} peak_L={:.4} peak_R={:.4} | mic_vol={}% muted={}",
                    s_i64(eng, "desktop_volume_pct"),
                    s_i64(eng, "desktop_muted"),
                    s_f64(eng, "output_peak_l"),
                    s_f64(eng, "output_peak_r"),
                    s_i64(eng, "desktop_mic_volume_pct"),
                    s_i64(eng, "desktop_mic_muted")
                );
                let _ = writeln!(
                    out,
                    "  DSP/EQ    : preset='{}' enabled={} dyn_mode={} preamp={:.2}dB thresh={:.2}dB ratio={:.2} limiter={}",
                    s_str(eng, "eq_preset"),
                    s_i64(eng, "eq_enabled"),
                    s_i64(eng, "eq_dynamic_mode"),
                    s_f64(eng, "eq_preamp_db"),
                    s_f64(eng, "eq_dyn_threshold_db"),
                    s_f64(eng, "eq_dyn_ratio"),
                    s_i64(eng, "eq_limiter_enabled")
                );
                let _ = writeln!(
                    out,
                    "  Scheduling: rt_prio={} total_cycles={} hw_xruns={} excl_owner={} midi_routed={} pa_up={} pa_clients={}",
                    s_i64(eng, "realtime_priority"),
                    s_i64(eng, "total_cycles"),
                    s_i64(eng, "hw_xruns"),
                    s_i64(eng, "exclusive_owner_id"),
                    s_i64(eng, "midi_events_routed"),
                    s_i64(eng, "pa_server_running"),
                    s_i64(eng, "pa_clients")
                );
            }
            if let Some(rt) = snap.get("rt") {
                let _ = writeln!(
                    out,
                    "  RT Timing : cycles={} cpu_load={:.2}% budget={:.1}us | last={:.1}us avg={:.1}us min={:.1}us max={:.1}us",
                    s_i64(rt, "cycle_count"),
                    s_f64(rt, "cpu_load_pct"),
                    s_f64(rt, "budget_us"),
                    s_f64(rt, "last_cycle_us"),
                    s_f64(rt, "avg_cycle_us"),
                    s_f64(rt, "min_cycle_us"),
                    s_f64(rt, "max_cycle_us")
                );
                let _ = writeln!(
                    out,
                    "  RT Stages : eq={:.1}us pa_mix={:.1}us hw_write={:.1}us | jitter_last={:.1}us jitter_max={:.1}us",
                    s_f64(rt, "eq_us"),
                    s_f64(rt, "pa_mix_us"),
                    s_f64(rt, "hw_write_us"),
                    s_f64(rt, "last_jitter_us"),
                    s_f64(rt, "max_jitter_us")
                );
            }
            if let Some(wire) = snap.get("wire") {
                let _ = writeln!(
                    out,
                    "  Wire/Safe : backend={} is_rust={} armed={} trips={} pa_q={} pw_q={} bt_fb={} fd_blk={} resets={} last='{}'",
                    s_str(wire, "backend"),
                    s_i64(wire, "is_rust"),
                    s_i64(wire, "armed"),
                    s_i64(wire, "trips_total"),
                    s_i64(wire, "pa_quarantined"),
                    s_i64(wire, "pw_quarantined"),
                    s_i64(wire, "bt_fallbacks"),
                    s_i64(wire, "fd_leaks_blocked"),
                    s_i64(wire, "manual_resets"),
                    s_str(wire, "last_reason")
                );
            }
        } else {
            let _ = writeln!(out, "  (No live snapshot available — audiodg offline or unreachable)");
        }
        let ctl_path = base_dir.join("build/lwasapi-ctl");
        if ctl_path.is_file() {
            if let Ok(ctl_out) = Command::new(&ctl_path).arg("status").output() {
                let txt = String::from_utf8_lossy(&ctl_out.stdout);
                if !txt.trim().is_empty() {
                    let _ = writeln!(out, "  [lwasapi-ctl status]");
                    for line in txt.lines() {
                        let _ = writeln!(out, "    {line}");
                    }
                }
            }
        }
        out.push('\n');

        // [3] BLUEZ A2DP / HFP, CAPTURE DUPLEX & MIDI HUB
        let _ = writeln!(out, "[3] BLUEZ A2DP/HFP, DUPLEX CAPTURE & MIDI HUB");
        if let Some(snap) = snapshot {
            if let Some(bt) = snap.get("bluez") {
                let _ = writeln!(
                    out,
                    "  BlueZ     : active={} init={} endpoints={} direct_sock={} has_tr={} tx_paused={}",
                    s_i64(bt, "active"),
                    s_i64(bt, "initialized"),
                    s_i64(bt, "endpoints_registered"),
                    s_i64(bt, "direct_socket"),
                    s_i64(bt, "has_transport"),
                    s_i64(bt, "tx_paused")
                );
                let _ = writeln!(
                    out,
                    "  BT Stream : alias='{}' codec={} rate={}Hz ch={} delay={}ms avrcp_vol={} mtu_r={} mtu_w={}",
                    s_str(bt, "alias"),
                    s_str(bt, "codec"),
                    s_i64(bt, "sample_rate"),
                    s_i64(bt, "channels"),
                    s_i64(bt, "delay_ms"),
                    s_i64(bt, "avrcp_vol"),
                    s_i64(bt, "mtu_r"),
                    s_i64(bt, "mtu_w")
                );
                let _ = writeln!(
                    out,
                    "  BT TXRing : fill={}/{} pkts={} underruns={} errors={} dev={} tr={} ep={}",
                    s_i64(bt, "tx_ring_fill"),
                    s_i64(bt, "tx_ring_cap"),
                    s_i64(bt, "tx_packets"),
                    s_i64(bt, "tx_underruns"),
                    s_i64(bt, "tx_errors"),
                    s_str(bt, "device_path"),
                    s_str(bt, "transport_path"),
                    s_str(bt, "endpoint_path")
                );
                let usb_pwr = SystemAuditScanner::inspect_bt_usb_power();
                let _ = writeln!(
                    out,
                    "  BT USB Pwr: risk={} ({})",
                    u8::from(usb_pwr.autosuspend_risk),
                    usb_pwr.summary
                );
            }
            if let Some(cap) = snap.get("capture") {
                let _ = writeln!(
                    out,
                    "  Capture   : active={} alias='{}' id='{}' pcm='{}' usb={} duplex_locked={}",
                    s_i64(cap, "active"),
                    s_str(cap, "alias"),
                    s_str(cap, "device_id"),
                    s_str(cap, "pcm_path"),
                    s_i64(cap, "is_usb"),
                    s_i64(cap, "duplex_locked")
                );
                let _ = writeln!(
                    out,
                    "  CapClock  : hw_rate={} pref_rate={} measured_rate={} ch={} fmt={} ring_fill={} kicks={}",
                    s_i64(cap, "hw_rate"),
                    s_i64(cap, "pref_rate"),
                    s_i64(cap, "measured_rate"),
                    s_i64(cap, "hw_channels"),
                    s_i64(cap, "hw_format"),
                    s_i64(cap, "ring_fill"),
                    s_i64(cap, "kick_count")
                );
            }
            if let Some(midi) = snap.get("midi") {
                let _ = writeln!(
                    out,
                    "  MIDI Hub  : init={} rawmidi={} seq={}:{} routed={} ctrl='{}' dev='{}'",
                    s_i64(midi, "initialized"),
                    s_i64(midi, "rawmidi_open"),
                    s_i64(midi, "seq_client"),
                    s_i64(midi, "seq_in_port"),
                    s_i64(midi, "events_routed"),
                    s_str(midi, "controller"),
                    s_str(midi, "dev_path")
                );
            }
        }
        out.push('\n');

        // [4] ZERO-PIPEWIRE SCREENCAST BROKER, SHM PREVIEW & DISPLAY OUTPUTS
        let _ = writeln!(out, "[4] PIPEWIRE SCREENCAST BROKER, SHM & DISPLAY REFRESH");
        if let Some(snap) = snapshot {
            if let Some(pw) = snap.get("pw") {
                let act_fps = s_i64(pw, "actual_fps_x100") as f64 / 100.0;
                let _ = writeln!(
                    out,
                    "  PW Broker : running={} zero_pw={} preview_req={} casting={} fps={:.2} relayed={} dropped={}",
                    s_i64(pw, "broker_running"),
                    s_i64(pw, "zero_pw_mode"),
                    s_i64(pw, "preview_requested"),
                    s_i64(pw, "active_casting"),
                    act_fps,
                    s_i64(pw, "frames_relayed_total"),
                    s_i64(pw, "frames_dropped")
                );
                let _ = writeln!(
                    out,
                    "  PW Targets: dmabuf_streams={} memfd_streams={} audio_rejects={} prev={}x{}@{}fps cap_lim={}x{}@{}fps",
                    s_i64(pw, "dmabuf_streams"),
                    s_i64(pw, "memfd_streams"),
                    s_i64(pw, "audio_rejects"),
                    s_i64(pw, "preview_target_w"),
                    s_i64(pw, "preview_target_h"),
                    s_i64(pw, "preview_target_fps"),
                    s_i64(pw, "cast_limit_w"),
                    s_i64(pw, "cast_limit_h"),
                    s_i64(pw, "cast_limit_fps")
                );
                let _ = writeln!(
                    out,
                    "  PW Paths  : sock={} mgr={} shm={}",
                    s_str(pw, "socket_path"),
                    s_str(pw, "manager_socket_path"),
                    s_str(pw, "preview_shm_name")
                );
                let shm_path = s_str(pw, "preview_shm_name");
                if !shm_path.is_empty() {
                    if let Ok(rd) = ScreenCastShmReader::open(shm_path) {
                        if let Some(fr) = rd.read_frame() {
                            let _ = writeln!(
                                out,
                                "  PW SHMHdr : active={} dmabuf={} mappable={} size={}x{} seq={} fps={:.2} cons={} prod='{}' fmt='{}'",
                                fr.active_stream,
                                fr.is_dmabuf,
                                fr.dmabuf_mappable,
                                fr.width,
                                fr.height,
                                fr.seq,
                                fr.actual_fps_x100 as f64 / 100.0,
                                fr.cons_count,
                                fr.prod_name,
                                fr.format_name
                            );
                        }
                    }
                }
                if let Some(clients) = pw.get("clients").and_then(JsonValue::as_array) {
                    let _ = writeln!(out, "  PW Clients ({}):", clients.len());
                    for c in clients {
                        let _ = writeln!(
                            out,
                            "    - id={} pid={} portal={} nodes={} app='{}' portal_id='{}' bin='{}'",
                            s_i64(c, "id"),
                            s_i64(c, "pid"),
                            s_i64(c, "is_portal"),
                            s_i64(c, "num_nodes"),
                            s_str(c, "app_name"),
                            s_str(c, "portal_app_id"),
                            s_str(c, "binary")
                        );
                    }
                }
                if let Some(nodes) = pw.get("nodes").and_then(JsonValue::as_array) {
                    let _ = writeln!(out, "  PW Nodes ({}):", nodes.len());
                    for n in nodes {
                        let _ = writeln!(
                            out,
                            "    - id={} cid={} pid={} dir={} state={} {}x{}@{}/{} {} {} mod={} bufs={} frames={} name='{}' class='{}'",
                            s_i64(n, "id"),
                            s_i64(n, "client_id"),
                            s_i64(n, "client_pid"),
                            s_i64(n, "direction"),
                            s_i64(n, "state"),
                            s_i64(n, "width"),
                            s_i64(n, "height"),
                            s_i64(n, "fps_num"),
                            s_i64(n, "fps_denom"),
                            s_str(n, "format_name"),
                            s_str(n, "mem_type"),
                            s_str(n, "drm_modifier"),
                            s_i64(n, "n_buffers"),
                            s_i64(n, "frames"),
                            s_str(n, "name"),
                            s_str(n, "media_class")
                        );
                    }
                }
                if let Some(links) = pw.get("links").and_then(JsonValue::as_array) {
                    let _ = writeln!(out, "  PW Links ({}):", links.len());
                    for l in links {
                        let _ = writeln!(
                            out,
                            "    - id={} {} -> {} state={} dmabuf={} relayed={} [{}]",
                            s_i64(l, "id"),
                            s_i64(l, "out_node_id"),
                            s_i64(l, "in_node_id"),
                            s_i64(l, "state"),
                            s_i64(l, "is_dmabuf"),
                            s_i64(l, "frames_relayed"),
                            s_str(l, "format_summary")
                        );
                    }
                }
            }
        }
        if let Some(kdoc) = which("kscreen-doctor") {
            if let Ok(ko) = Command::new(kdoc).arg("-o").output() {
                let raw = String::from_utf8_lossy(&ko.stdout);
                let txt = strip_ansi_escapes(&raw);
                if !txt.trim().is_empty() {
                    let _ = writeln!(out, "  [Display Outputs (kscreen-doctor -o)]");
                    for line in txt.lines().filter(|l| {
                        let t = l.trim();
                        t.starts_with("Output:")
                            || t.starts_with("Modes:")
                            || t.starts_with("Geometry:")
                            || t.starts_with("Scale:")
                            || t.starts_with("Vrr:")
                            || t.starts_with("HDR:")
                    }) {
                        let _ = writeln!(out, "    {}", line.trim_end());
                    }
                }
            }
        }
        out.push('\n');

        // [5] ACTIVE AUDIO CLIENTS & STREAMS
        let _ = writeln!(out, "[5] ACTIVE WASAPI SESSIONS, PULSEAUDIO CLIENTS & STREAMS");
        if let Some(snap) = snapshot {
            if let Some(ws) = snap.get("wasapi_sessions").and_then(JsonValue::as_array) {
                let _ = writeln!(out, "  WASAPI Sessions ({}):", ws.len());
                for s in ws {
                    let _ = writeln!(
                        out,
                        "    - sid={} pid={} name='{}' mode={} cap={} {}Hz/{}ch active={} muted={} vol={}% buf={} per={} buffered={} xruns={}",
                        s_i64(s, "session_id"),
                        s_i64(s, "pid"),
                        s_str(s, "name"),
                        s_i64(s, "share_mode"),
                        s_i64(s, "is_capture"),
                        s_i64(s, "rate"),
                        s_i64(s, "channels"),
                        s_i64(s, "active"),
                        s_i64(s, "muted"),
                        s_i64(s, "volume_milli") / 10,
                        s_i64(s, "buffer_frames"),
                        s_i64(s, "period_frames"),
                        s_i64(s, "buffered_frames"),
                        s_i64(s, "xruns")
                    );
                }
            }
            if let Some(pc) = snap.get("pa_clients").and_then(JsonValue::as_array) {
                let _ = writeln!(out, "  PulseAudio Clients ({}):", pc.len());
                for c in pc {
                    let _ = writeln!(
                        out,
                        "    - idx={} pid={} name='{}' bin='{}' ver={} authed={} subs=0x{:x} outs={}",
                        s_i64(c, "index"),
                        s_i64(c, "pid"),
                        s_str(c, "name"),
                        s_str(c, "binary"),
                        s_i64(c, "version"),
                        s_i64(c, "authed"),
                        s_i64(c, "sub_mask"),
                        s_i64(c, "out_count")
                    );
                }
            }
            if let Some(ps) = snap.get("pa_streams").and_then(JsonValue::as_array) {
                let _ = writeln!(out, "  PulseAudio Streams ({}):", ps.len());
                for st in ps {
                    let _ = writeln!(
                        out,
                        "    - idx={} cid={} pid={} name='{}' dev='{}' pb={} mon={} peak={} {}Hz/{}ch fmt={} act={} cork={} mut={} vol={}% buf={}/{} peak={:.3}/{:.3} wr={} rd={}",
                        s_i64(st, "index"),
                        s_i64(st, "client_index"),
                        s_i64(st, "pid"),
                        s_str(st, "name"),
                        s_str(st, "device_id"),
                        s_i64(st, "is_playback"),
                        s_i64(st, "is_monitor"),
                        s_i64(st, "peak_detect"),
                        s_i64(st, "rate"),
                        s_i64(st, "channels"),
                        s_i64(st, "format"),
                        s_i64(st, "active"),
                        s_i64(st, "corked"),
                        s_i64(st, "muted"),
                        s_i64(st, "volume_milli") / 10,
                        s_i64(st, "buffered_frames"),
                        s_i64(st, "ring_cap_frames"),
                        s_f64(st, "mix_peak_l"),
                        s_f64(st, "mix_peak_r"),
                        s_i64(st, "write_index"),
                        s_i64(st, "read_index")
                    );
                }
            }
        }
        out.push('\n');

        // [6] HOST AUDIT: SOCKETS, SYSTEMD UNITS, AUDIO PROCESSES & ALSA PROC
        let _ = writeln!(out, "[6] HOST AUDIT: SOCKETS, SYSTEMD UNITS, PROCESSES & ALSA KERNEL");
        if let Some(a) = audit {
            if let Some(socks) = a.get("sockets").and_then(JsonValue::as_array) {
                let _ = writeln!(out, "  IPC Sockets ({}):", socks.len());
                for sk in socks {
                    let _ = writeln!(
                        out,
                        "    - {:<18} conn={:<5} mode={} pid={} comm='{}' path={} err={}",
                        s_str(sk, "name"),
                        s_bool(sk, "connectable"),
                        s_str(sk, "mode_oct"),
                        sk.get("peer_pid").and_then(JsonValue::as_i64).unwrap_or(0),
                        s_str(sk, "peer_comm"),
                        s_str(sk, "path"),
                        s_str(sk, "error")
                    );
                }
            }
            if let Some(units) = a.get("systemd_units").and_then(JsonValue::as_array) {
                let _ = writeln!(out, "  Systemd User Units:");
                for u in units {
                    let _ = writeln!(
                        out,
                        "    - {:<32} active={:<10} ({}) enabled={:<10} pid={}",
                        s_str(u, "unit"),
                        s_str(u, "active"),
                        s_str(u, "sub_state"),
                        s_str(u, "enabled"),
                        s_i64(u, "main_pid")
                    );
                }
            }
            if let Some(uss) = a.get("unix_sockets").and_then(JsonValue::as_array) {
                let _ = writeln!(out, "  Listening Unix Sockets (ss -xnH, {}):", uss.len());
                for s in uss {
                    if let Some(line) = s.as_str() {
                        let _ = writeln!(out, "    {line}");
                    }
                }
            }
            if let Some(procs) = a.get("processes").and_then(JsonValue::as_array) {
                let _ = writeln!(out, "  Audited Audio/Portal Processes ({}):", procs.len());
                for p in procs {
                    let flat_cmd = s_str(p, "cmdline").replace(['\n', '\r'], " ");
                    let _ = writeln!(
                        out,
                        "    * pid={} comm='{}' class='{}' cmd='{}' alsa_fds=[{}] shm=[{}] libs=[{}]",
                        s_i64(p, "pid"),
                        s_str(p, "comm"),
                        s_str(p, "classification"),
                        flat_cmd,
                        s_str(p, "alsa_fds"),
                        s_str(p, "sockets_shm"),
                        s_str(p, "mapped_libs")
                    );
                }
            }
            if let Some(alsa) = a.get("alsa") {
                let cards = s_str(alsa, "cards");
                if !cards.trim().is_empty() {
                    let _ = writeln!(out, "  ALSA /proc/asound/cards:");
                    for l in cards.lines() {
                        let _ = writeln!(out, "    {l}");
                    }
                }
                let pcm = s_str(alsa, "pcm");
                if !pcm.trim().is_empty() {
                    let _ = writeln!(out, "  ALSA /proc/asound/pcm:");
                    for l in pcm.lines() {
                        let _ = writeln!(out, "    {l}");
                    }
                }
                if let Some(ast) = alsa.get("active_streams").and_then(JsonValue::as_array) {
                    if !ast.is_empty() {
                        let _ = writeln!(out, "  Active Kernel /proc/asound Substreams ({}):", ast.len());
                        for s in ast {
                            let _ = writeln!(
                                out,
                                "    * {} -> {} | {}",
                                s_str(s, "stream"),
                                s_str(s, "status").replace('\n', " "),
                                s_str(s, "hw_params").replace('\n', " ")
                            );
                        }
                    }
                }
            }
            if let Some(cfg) = a.get("configs") {
                let _ = writeln!(
                    out,
                    "  Config/Shims: plugin_matches_sys={} repo_sha={} sys_sha={}",
                    s_bool(cfg, "plugin_matches_system"),
                    s_str(cfg, "plugin_repo_sha256"),
                    s_str(cfg, "plugin_sys_sha256")
                );
            }
        }
        out.push('\n');

        // [7] DIAGNOSTIC PROBES & CONFIGURATION FILES
        let _ = writeln!(out, "[7] DIAGNOSTIC PROBES & RUNTIME CONFIGS");
        if let Some(p) = probes {
            if let Some(obj) = p.as_object() {
                for (k, v) in obj {
                    let _ = writeln!(out, "  {k:<26}: {}", v.to_json_compact());
                }
            }
        }
        let st_dir = state_dir();
        for fname in [
            "gui_settings.json",
            "pipewire_zero_mode",
            "module_versions.state",
            "flatpak_overrides.list",
        ] {
            let p = st_dir.join(fname);
            if p.is_file() {
                let txt = read_text_lossy(&p, 16384);
                let _ = writeln!(out, "  [Config: {}]", p.display());
                for line in txt.lines() {
                    let _ = writeln!(out, "    {line}");
                }
            }
        }
        out.push('\n');

        // [8] RECENT EVENTS & SYSTEM LOG TAILS
        let _ = writeln!(out, "[8] STRUCTURED EVENTS & SERVICE LOGS");
        if !events.is_empty() {
            let _ = writeln!(out, "  Structured Debug Events ({}):", events.len());
            for ev in events {
                let _ = writeln!(
                    out,
                    "    [{}] [{}/{}] pid={} {} {}: {}",
                    format_ts_ms(s_i64(ev, "ts_ms")),
                    s_str(ev, "sub"),
                    s_str(ev, "lvl"),
                    s_i64(ev, "pid"),
                    s_str(ev, "op"),
                    s_str(ev, "target"),
                    s_str(ev, "detail")
                );
            }
        }
        let j_lines = ExternalLogManager::tail_journal(150);
        if !j_lines.is_empty() {
            let _ = writeln!(out, "  Journal Tail (last {} lines):", j_lines.len());
            for line in &j_lines {
                let _ = writeln!(out, "    {line}");
            }
        }
        let a_lines = ExternalLogManager::tail_autosync_log(120);
        if !a_lines.is_empty() {
            let _ = writeln!(out, "  Autosync Log Tail (last {} lines):", a_lines.len());
            for line in &a_lines {
                let _ = writeln!(out, "    {line}");
            }
        }
        out
    }

    pub fn export_text_file(out_path: Option<&Path>, report_text: &str) -> Result<PathBuf, String> {
        let dest = match out_path {
            None => {
                let dir = private_runtime_dir().join("exports");
                ensure_dir_0700(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
                dir.join(format!("salsa-debug-{}.txt", format_now_stamp()))
            }
            Some(p) => {
                let looks_like_dir = p.is_dir()
                    || p.to_string_lossy().ends_with('/')
                    || p.extension().is_none();
                if looks_like_dir {
                    fs::create_dir_all(p).map_err(|e| format!("mkdir {}: {e}", p.display()))?;
                    p.join(format!("salsa-debug-{}.txt", format_now_stamp()))
                } else {
                    if let Some(parent) = p.parent() {
                        if !parent.as_os_str().is_empty() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
                        }
                    }
                    p.to_path_buf()
                }
            }
        };
        if let Ok(meta) = fs::symlink_metadata(&dest) {
            if meta.file_type().is_symlink() {
                return Err(format!("refusing to overwrite symlink {}", dest.display()));
            }
        }
        fs::write(&dest, report_text.as_bytes())
            .map_err(|e| format!("write {}: {e}", dest.display()))?;
        let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o600));
        Ok(dest)
    }

    pub fn export_bundle(
        out_dir: Option<&Path>,
        snapshot: Option<&JsonValue>,
        audit: Option<&JsonValue>,
        probes: Option<&JsonValue>,
        events: &[JsonValue],
    ) -> Result<PathBuf, String> {
        let (target_dir, is_custom) = match out_dir {
            Some(d) => (d.to_path_buf(), true),
            None => (private_runtime_dir().join("exports"), false),
        };
        if is_custom {
            fs::create_dir_all(&target_dir)
                .map_err(|e| format!("mkdir {}: {e}", target_dir.display()))?;
        } else {
            ensure_dir_0700(&target_dir)
                .map_err(|e| format!("mkdir {}: {e}", target_dir.display()))?;
        }
        let zip_path = target_dir.join(format!("salsa-debug-{}.zip", format_now_stamp()));

        let mut zw = ZipWriter::new();
        let report_txt = Self::build_detailed_text_report(&install_root(), snapshot, audit, probes, events);
        zw.add_file("report.txt", report_txt.as_bytes(), 0o644)?;
        if let Some(s) = snapshot {
            let txt = s.to_json_pretty(2);
            zw.add_file("snapshot.json", txt.as_bytes(), 0o644)?;
        }
        if let Some(a) = audit {
            let txt = a.to_json_pretty(2);
            zw.add_file("audit.json", txt.as_bytes(), 0o644)?;
        }
        if let Some(p) = probes {
            let txt = p.to_json_pretty(2);
            zw.add_file("probes.json", txt.as_bytes(), 0o644)?;
        }
        if !events.is_empty() {
            let mut nd = String::new();
            for ev in events {
                nd.push_str(&ev.to_json_compact());
                nd.push('\n');
            }
            zw.add_file("events.ndjson", nd.as_bytes(), 0o644)?;
        }
        let j_lines = ExternalLogManager::tail_journal(300).join("\n");
        if !j_lines.is_empty() {
            zw.add_file("journal.log", j_lines.as_bytes(), 0o644)?;
        }
        let a_lines = ExternalLogManager::tail_autosync_log(200).join("\n");
        if !a_lines.is_empty() {
            zw.add_file("autosync.log", a_lines.as_bytes(), 0o644)?;
        }

        zw.write_to_path_mode(&zip_path, 0o600)?;
        Ok(zip_path)
    }
}

fn probe_debug_ping(sock_path: &str) -> Option<JsonValue> {
    let mut stream = UnixStream::connect(sock_path).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(800)));
    stream.write_all(b"PING probe\n").ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let deadline = Instant::now() + Duration::from_millis(900);
    while Instant::now() < deadline {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        if let Ok(val) = JsonValue::parse(line.trim()) {
            if val.get("type").and_then(JsonValue::as_str) == Some("pong") {
                return Some(val);
            }
        }
    }
    None
}

fn dlopen_symbol_check(path: &Path, symbol: &str) -> bool {
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let Ok(c_sym) = CString::new(symbol) else {
        return false;
    };
    unsafe {
        let _ = dlerror();
        let h = dlopen(c_path.as_ptr(), RTLD_LAZY | RTLD_LOCAL);
        if h.is_null() {
            return false;
        }
        let sym = dlsym(h, c_sym.as_ptr());
        let ok = !sym.is_null();
        let _ = dlclose(h);
        ok
    }
}

#[derive(Debug, Clone)]
pub struct PreviewFrameData {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub seq: u64,
    pub pts_ns: u64,
    pub prod_node_id: u32,
    pub cons_count: u32,
    pub active_stream: bool,
    pub is_dmabuf: bool,
    pub dmabuf_mappable: bool,
    pub actual_fps_x100: u32,
    pub target_max_w: u32,
    pub target_max_h: u32,
    pub target_fps: u32,
    pub cast_fps_cap: u32,
    pub prod_name: String,
    pub format_name: String,
    pub rgba: Vec<u8>,
}

pub struct ScreenCastShmReader {
    _fd: OwnedFd,
    map: MappedMem,
}

impl ScreenCastShmReader {
    pub fn open(shm_name: &str) -> Result<Self, String> {
        let map_size = mem::size_of::<LwasapiPwPreviewShm>();
        let trimmed = shm_name.trim_start_matches('/');
        if trimmed.is_empty() {
            return Err("empty preview shm name".to_string());
        }
        let path = if shm_name.starts_with('/') && Path::new(shm_name).exists() {
            PathBuf::from(shm_name)
        } else {
            PathBuf::from(format!("/dev/shm/{trimmed}"))
        };
        let f: File = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_CLOEXEC | O_NOFOLLOW)
            .open(&path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        let meta = f
            .metadata()
            .map_err(|e| format!("stat {}: {e}", path.display()))?;
        if !meta.file_type().is_file() || (meta.len() as usize) < map_size {
            return Err(format!(
                "invalid preview shm {}: size={}",
                path.display(),
                meta.len()
            ));
        }
        let fd: OwnedFd = f.into();
        let map = MappedMem::map_fd(fd.as_raw_fd(), map_size, true, 0)
            .ok_or_else(|| format!("mmap {} failed", path.display()))?;
        let shm = unsafe { &*(map.as_ptr() as *const LwasapiPwPreviewShm) };
        let magic = unsafe { core::ptr::read_volatile(&shm.magic) };
        let version = unsafe { core::ptr::read_volatile(&shm.version) };
        if magic != LWASAPI_PW_PREVIEW_MAGIC || version != LWASAPI_PW_PREVIEW_VERSION {
            return Err(format!(
                "invalid preview shm header in {} (magic=0x{magic:08x}, version={version})",
                path.display()
            ));
        }
        Ok(Self { _fd: fd, map })
    }

    pub fn send_preview_ping(
        &self,
        enabled: bool,
        max_w: u32,
        max_h: u32,
        target_fps: u32,
        cast_fps_cap: u32,
    ) {
        let base = self.map.as_ptr();
        if base.is_null() {
            return;
        }
        unsafe {
            let ping_ptr = base.add(offset_of!(LwasapiPwPreviewShm, last_gui_ping_ns)) as *const AtomicU64;
            let req_ptr = base.add(offset_of!(LwasapiPwPreviewShm, preview_requested)) as *const AtomicU32;
            let tw_ptr = base.add(offset_of!(LwasapiPwPreviewShm, target_max_w)) as *const AtomicU32;
            let th_ptr = base.add(offset_of!(LwasapiPwPreviewShm, target_max_h)) as *const AtomicU32;
            let tf_ptr = base.add(offset_of!(LwasapiPwPreviewShm, target_fps)) as *const AtomicU32;
            let cap_ptr = base.add(offset_of!(LwasapiPwPreviewShm, cast_fps_cap)) as *const AtomicU32;
            (*tw_ptr).store(max_w.min(LWASAPI_PW_PREVIEW_MAX_W), Ordering::Relaxed);
            (*th_ptr).store(max_h.min(LWASAPI_PW_PREVIEW_MAX_H), Ordering::Relaxed);
            (*tf_ptr).store(target_fps, Ordering::Relaxed);
            (*cap_ptr).store(cast_fps_cap, Ordering::Relaxed);
            (*req_ptr).store(u32::from(enabled), Ordering::Release);
            (*ping_ptr).store(mono_ns(), Ordering::Release);
        }
    }

    pub fn read_frame(&self) -> Option<PreviewFrameData> {
        self.read_frame_since(None)
    }

    pub fn read_frame_since(&self, prev_seq: Option<u64>) -> Option<PreviewFrameData> {
        let base = self.map.as_ptr();
        if base.is_null() {
            return None;
        }
        let shm = unsafe { &*(base as *const LwasapiPwPreviewShm) };
        let magic = unsafe { core::ptr::read_volatile(&shm.magic) };
        let version = unsafe { core::ptr::read_volatile(&shm.version) };
        if magic != LWASAPI_PW_PREVIEW_MAGIC || version != LWASAPI_PW_PREVIEW_VERSION {
            return None;
        }
        let seqlock_ptr =
            unsafe { base.add(offset_of!(LwasapiPwPreviewShm, seqlock)) as *const AtomicU64 };
        let mut deadline: Option<Instant> = None;
        for attempt in 0..64 {
            let s1 = unsafe { (*seqlock_ptr).load(Ordering::Acquire) };
            if (s1 & 1) == 0 {
                let (
                    w,
                    h,
                    src_w,
                    src_h,
                    stride,
                    seq,
                    pts_ns,
                    prod_node_id,
                    cons_count,
                    active_stream,
                    is_dmabuf,
                    dmabuf_mappable,
                    actual_fps_x100,
                    target_max_w,
                    target_max_h,
                    target_fps,
                    cast_fps_cap,
                    prod_raw,
                    fmt_raw,
                ) = unsafe {
                    (
                        core::ptr::read_volatile(&shm.dst_width),
                        core::ptr::read_volatile(&shm.dst_height),
                        core::ptr::read_volatile(&shm.src_width),
                        core::ptr::read_volatile(&shm.src_height),
                        core::ptr::read_volatile(&shm.dst_stride),
                        core::ptr::read_volatile(&shm.frame_seq),
                        core::ptr::read_volatile(&shm.pts_ns),
                        core::ptr::read_volatile(&shm.prod_node_id),
                        core::ptr::read_volatile(&shm.cons_count),
                        core::ptr::read_volatile(&shm.active_stream) != 0,
                        core::ptr::read_volatile(&shm.is_dmabuf) != 0,
                        core::ptr::read_volatile(&shm.dmabuf_mappable) != 0,
                        core::ptr::read_volatile(&shm.actual_fps_x100),
                        core::ptr::read_volatile(&shm.target_max_w),
                        core::ptr::read_volatile(&shm.target_max_h),
                        core::ptr::read_volatile(&shm.target_fps),
                        core::ptr::read_volatile(&shm.cast_fps_cap),
                        core::ptr::read_volatile(&shm.prod_name),
                        core::ptr::read_volatile(&shm.format_name),
                    )
                };
                let unchanged = active_stream && prev_seq.is_some_and(|p| p != 0 && seq == p);
                let row_bytes = (w as usize).saturating_mul(4);
                let byte_len = (stride as usize).saturating_mul(h as usize);
                let rgba = if !unchanged
                    && w > 0
                    && h > 0
                    && w <= LWASAPI_PW_PREVIEW_MAX_W
                    && h <= LWASAPI_PW_PREVIEW_MAX_H
                    && (stride as usize) >= row_bytes
                    && byte_len <= LWASAPI_PW_PREVIEW_MAX_BYTES
                    && byte_len <= shm.pixels.len()
                {
                    let total_out = row_bytes.saturating_mul(h as usize);
                    let mut out = vec![0u8; total_out];
                    let src = &shm.pixels[..byte_len];
                    let st = stride as usize;
                    let wu = w as usize;
                    for (y, drow) in out.chunks_exact_mut(row_bytes).enumerate() {
                        let srow = &src[y * st..y * st + row_bytes];
                        let (schunks, _) = srow.as_chunks::<4>();
                        let (dchunks, _) = drow.as_chunks_mut::<4>();
                        for (&[b, g, r, _], dp) in schunks.iter().take(wu).zip(dchunks.iter_mut()) {
                            // PipeWire preview pixels arrive in BGRx/BGRA byte order;
                            // force opaque alpha so BGRx frames render visibly and export cleanly.
                            *dp = [r, g, b, 255];
                        }
                    }
                    out
                } else {
                    Vec::new()
                };
                let out_stride = if rgba.is_empty() { 0 } else { w.saturating_mul(4) };
                let disp_w = if w > 0 { w } else { src_w };
                let disp_h = if h > 0 { h } else { src_h };
                let prod_name = cstr_to_string(&prod_raw);
                let format_name = cstr_to_string(&fmt_raw);

                fence(Ordering::Acquire);
                let s2 = unsafe { (*seqlock_ptr).load(Ordering::Acquire) };
                if s1 == s2 {
                    return Some(PreviewFrameData {
                        width: disp_w,
                        height: disp_h,
                        stride: out_stride,
                        seq,
                        pts_ns,
                        prod_node_id,
                        cons_count,
                        active_stream,
                        is_dmabuf,
                        dmabuf_mappable,
                        actual_fps_x100,
                        target_max_w,
                        target_max_h,
                        target_fps,
                        cast_fps_cap,
                        prod_name,
                        format_name,
                        rgba,
                    });
                }
            }
            if attempt < 4 {
                std::hint::spin_loop();
            } else {
                let d = *deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(8));
                if Instant::now() >= d {
                    return None;
                }
                if attempt == 4 {
                    thread::yield_now();
                } else {
                    thread::sleep(Duration::from_micros(250));
                }
            }
        }
        None
    }
}

pub struct DryRunDesktopCapturer {
    pub width: u32,
    pub height: u32,
    frame_idx: u32,
    base_rgba: Vec<u8>,
}

impl DryRunDesktopCapturer {
    pub fn new(width: u32, height: u32) -> Self {
        let cw = width.clamp(16, LWASAPI_PW_PREVIEW_MAX_W);
        let ch = height.clamp(16, LWASAPI_PW_PREVIEW_MAX_H);
        let w = cw as usize;
        let h = ch as usize;
        let mut base_rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            let g_val = ((y * 255) / h) as u8;
            let row = &mut base_rgba[y * w * 4..(y + 1) * w * 4];
            let (chunks, _) = row.as_chunks_mut::<4>();
            for (x, px) in chunks.iter_mut().enumerate() {
                px[0] = ((x * 255) / w) as u8;
                px[1] = g_val;
                px[2] = 140;
                px[3] = 255;
            }
        }
        Self {
            width: cw,
            height: ch,
            frame_idx: 0,
            base_rgba,
        }
    }

    pub fn next_frame(&mut self) -> Vec<u8> {
        fn fill_bar(slice: &mut [u8], g_val: u8) {
            let (chunks, _) = slice.as_chunks_mut::<4>();
            for px in chunks {
                *px = [240, g_val, 64, 255];
            }
        }

        let w = self.width as usize;
        let h = self.height as usize;
        let mut rgba = self.base_rgba.clone();
        let bar_w = (w / 8).max(1);
        let phase = ((self.frame_idx.wrapping_mul(7)) as usize) % w.max(1);
        let bar_start = (w - phase) % w;
        for y in 0..h {
            let g_val = ((y * 255) / h) as u8;
            let row = &mut rgba[y * w * 4..(y + 1) * w * 4];
            let first_len = bar_w.min(w - bar_start);
            fill_bar(&mut row[bar_start * 4..(bar_start + first_len) * 4], g_val);
            if first_len < bar_w {
                let rem = bar_w - first_len;
                fill_bar(&mut row[..rem * 4], g_val);
            }
        }
        self.frame_idx = self.frame_idx.wrapping_add(1);
        rgba
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pw_preview_shm_pixel_offset_matches_c() {
        assert_eq!(offset_of!(LwasapiPwPreviewShm, pixels), 520);
        assert_eq!(
            mem::size_of::<LwasapiPwPreviewShm>(),
            520 + LWASAPI_PW_PREVIEW_MAX_BYTES
        );
    }

    #[test]
    fn helpers_and_dry_run_capturer_work() {
        assert_eq!(level_rank("trace"), 0);
        assert_eq!(level_rank("WARN"), 3);
        assert_eq!(level_rank("error"), 4);
        assert_eq!(format_ts_ms(0), "--:--:--.---");
        let s = format_ts_ms(1_700_000_000_123);
        assert!(s.ends_with(".123"));

        let mut cap = DryRunDesktopCapturer::new(64, 32);
        let f0 = cap.next_frame();
        let f1 = cap.next_frame();
        assert_eq!(f0.len(), 64 * 32 * 4);
        assert_ne!(f0, f1);

        let freshness = SystemAuditScanner::check_build_freshness(&install_root());
        assert!(freshness.get("checked_count").and_then(JsonValue::as_i64).unwrap_or(0) >= 18);

        let fake_sys =
            env::temp_dir().join(format!("salsa_bt_usb_pwr_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&fake_sys);
        let usb_dev = fake_sys.join("devices/usb8/8-12");
        let usb_intf = usb_dev.join("8-12:1.0");
        let bt_class = fake_sys.join("class/bluetooth/hci0");
        fs::create_dir_all(fake_sys.join("module/btusb/parameters")).unwrap();
        fs::create_dir_all(usb_dev.join("power")).unwrap();
        fs::create_dir_all(&usb_intf).unwrap();
        fs::create_dir_all(&bt_class).unwrap();
        fs::write(
            fake_sys.join("module/btusb/parameters/enable_autosuspend"),
            "Y\n",
        )
        .unwrap();
        fs::write(usb_dev.join("idVendor"), "13d3\n").unwrap();
        fs::write(usb_dev.join("idProduct"), "3616\n").unwrap();
        fs::write(usb_dev.join("power/control"), "auto\n").unwrap();
        fs::write(usb_dev.join("power/runtime_suspended_time"), "1234\n").unwrap();
        std::os::unix::fs::symlink(&usb_intf, bt_class.join("device")).unwrap();
        let rep = SystemAuditScanner::inspect_bt_usb_power_in(&fake_sys);
        assert!(rep.autosuspend_risk);
        assert_eq!(rep.adapters.len(), 1);
        assert_eq!(rep.adapters[0].usb_id, "13d3:3616");
        assert_eq!(rep.adapters[0].power_control, "auto");
        let _ = fs::remove_dir_all(&fake_sys);
    }

    #[test]
    fn screencast_shm_reader_waits_for_seqlock_and_skips_unchanged_seq() {
        let shm_path = env::temp_dir().join(format!("salsa_sc_shm_test_{}", std::process::id()));
        let map_size = mem::size_of::<LwasapiPwPreviewShm>();
        {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&shm_path)
                .unwrap();
            f.set_len(map_size as u64).unwrap();
        }
        let shm_str = shm_path.to_string_lossy().into_owned();
        let fd = OpenOptions::new().read(true).write(true).open(&shm_path).unwrap();
        let mut map = MappedMem::map_fd(fd.as_raw_fd(), map_size, true, 0).unwrap();
        let base = map.as_mut_slice().as_mut_ptr();
        unsafe {
            let shm = &mut *(base as *mut LwasapiPwPreviewShm);
            shm.magic = LWASAPI_PW_PREVIEW_MAGIC;
            shm.version = LWASAPI_PW_PREVIEW_VERSION;
            shm.active_stream = 1;
            shm.dst_width = 4;
            shm.dst_height = 2;
            shm.dst_stride = 16;
            shm.frame_seq = 1;
            shm.seqlock = 2;
            for px in shm.pixels[..32].as_chunks_mut::<4>().0 {
                px.copy_from_slice(&[0x11, 0x55, 0xaa, 0x00]);
            }
        }

        let reader = ScreenCastShmReader::open(&shm_str).unwrap();
        let f1 = reader.read_frame_since(None).expect("initial frame");
        assert!(f1.active_stream);
        assert_eq!(f1.seq, 1);
        assert_eq!(f1.rgba.len(), 32);
        assert_eq!(&f1.rgba[..4], &[0xaa, 0x55, 0x11, 255]);

        let unchanged = reader.read_frame_since(Some(1)).expect("unchanged seq");
        assert!(unchanged.active_stream);
        assert_eq!(unchanged.seq, 1);
        assert!(unchanged.rgba.is_empty());

        // Hold writer seqlock odd for 2ms, simulating a slow 1440p SHM copy.
        unsafe {
            let seq_ptr = base.add(offset_of!(LwasapiPwPreviewShm, seqlock)) as *const AtomicU64;
            (*seq_ptr).store(3, Ordering::Release);
        }
        let shm_addr = base as usize;
        let writer = thread::spawn(move || {
            let base_p = shm_addr as *mut u8;
            let seq_ptr =
                unsafe { base_p.add(offset_of!(LwasapiPwPreviewShm, seqlock)) as *const AtomicU64 };
            let shm = unsafe { &mut *(base_p as *mut LwasapiPwPreviewShm) };
            thread::sleep(Duration::from_millis(2));
            shm.frame_seq = 2;
            for px in shm.pixels[..32].as_chunks_mut::<4>().0 {
                px.copy_from_slice(&[0x22, 0x66, 0xbb, 0x00]);
            }
            unsafe { (*seq_ptr).store(4, Ordering::Release) };
        });

        let f2 = reader
            .read_frame_since(Some(1))
            .expect("reader must wait past 2ms odd seqlock");
        writer.join().unwrap();
        assert_eq!(f2.seq, 2);
        assert_eq!(f2.rgba.len(), 32);
        assert_eq!(&f2.rgba[..4], &[0xbb, 0x66, 0x22, 255]);
        let _ = fs::remove_file(&shm_path);
    }
}
