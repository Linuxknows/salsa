// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::mem;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use crate::c_abi::{c_str_to_string, LasapiDbgDgram, LasapiDbgRtStats};
use crate::failsafe;
use crate::json::JsonValue;

pub trait DebugEngineHost: Send + Sync {
    fn build_snapshot_json(&self, rt: &LasapiDbgRtStats) -> JsonValue;
    fn handle_debug_cmd(&self, cmd: &str) -> Result<String, (i32, String)>;
}
use crate::unix_sys::{
    bind_unix_dgram_passcred_0600, bind_unix_listener_0600, current_pid, do_poll, mono_ns,
    nb_recv, nb_send, posix_owner_id, realtime_ms, recv_dgram_with_ucred, resolve_sock_path_common,
    socket_peercred, PollFd, RecvDgramCred, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT,
};

pub const LASAPI_DEBUG_SOCK_DEFAULT: &str = "/tmp/lasapi-debug.sock";
pub const LASAPI_DEBUG_IN_SOCK_DEFAULT: &str = "/tmp/lasapi-debug-in.sock";
pub const LASAPI_DEBUG_DGRAM_MAGIC: u32 = 0x4C44_4247;

pub const DBG_RING_CAP: usize = 2048;
pub const DBG_HISTORY_CAP: usize = 512;
pub const DBG_MAX_CLIENTS: usize = 16;
pub const DBG_CLIENT_MAX_OUT: usize = 1024 * 1024;
const DBG_TELEM_INTERVAL_MS: i64 = 200;

const DBG_LVL_INFO: u8 = 1;

#[derive(Clone, Copy)]
struct DbgEvent {
    id: u64,
    ts_mono_ns: u64,
    ts_real_ms: i64,
    pid: u32,
    sid: u32,
    v1: i32,
    v2: i32,
    hr: i32,
    sub: u8,
    lvl: u8,
    op: [u8; 32],
    target: [u8; 64],
    detail: [u8; 192],
}

impl DbgEvent {
    const fn zeroed() -> Self {
        Self {
            id: 0,
            ts_mono_ns: 0,
            ts_real_ms: 0,
            pid: 0,
            sid: 0,
            v1: 0,
            v2: 0,
            hr: 0,
            sub: 0,
            lvl: 0,
            op: [0; 32],
            target: [0; 64],
            detail: [0; 192],
        }
    }
}

struct DbgSlot {
    turn: AtomicU64,
    ev: std::cell::UnsafeCell<DbgEvent>,
}

// Safety: Access to slot.ev is strictly governed by Vyukov bounded MPMC turn protocol
// using Acquire/Release ordering; only one producer writes a slot when turn == pos
// and only the single debug server consumer reads it when turn == tail + 1.
#[allow(unsafe_code)]
unsafe impl Sync for DbgSlot {}

impl DbgSlot {
    const fn new(idx: u64) -> Self {
        Self {
            turn: AtomicU64::new(idx),
            ev: std::cell::UnsafeCell::new(DbgEvent::zeroed()),
        }
    }
}

fn ring_slots() -> &'static [DbgSlot; DBG_RING_CAP] {
    static SLOTS: OnceLock<Box<[DbgSlot; DBG_RING_CAP]>> = OnceLock::new();
    SLOTS.get_or_init(|| {
        let mut v = Vec::with_capacity(DBG_RING_CAP);
        for i in 0..DBG_RING_CAP {
            v.push(DbgSlot::new(i as u64));
        }
        v.try_into().unwrap_or_else(|_| unreachable!())
    })
}

static G_RING_HEAD: AtomicU64 = AtomicU64::new(0);
static G_NEXT_EV_ID: AtomicU64 = AtomicU64::new(1);
static G_DROPPED_EVENTS: AtomicU64 = AtomicU64::new(0);

static G_RT_CYCLE_COUNT: AtomicU64 = AtomicU64::new(0);
static G_RT_LAST_CYCLE_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_AVG_CYCLE_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_MAX_CYCLE_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_MIN_CYCLE_NS: AtomicU64 = AtomicU64::new(u64::MAX);
static G_RT_LAST_JITTER_NS: AtomicI64 = AtomicI64::new(0);
static G_RT_MAX_JITTER_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_EQ_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_PA_MIX_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_HW_WRITE_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_BUDGET_NS: AtomicU64 = AtomicU64::new(0);
static G_RT_HW_XRUNS: AtomicU64 = AtomicU64::new(0);

static G_DBG_RUNNING: AtomicBool = AtomicBool::new(false);
static G_TEST_BYPASS_RUNNING: AtomicBool = AtomicBool::new(false);

#[inline]
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn is_dbg_running() -> bool {
    G_DBG_RUNNING.load(Ordering::Relaxed) || G_TEST_BYPASS_RUNNING.load(Ordering::Relaxed)
}

struct ServerRuntime {
    sock_path: String,
    in_sock_path: String,
    join: Option<JoinHandle<()>>,
}

static G_RUNTIME: OnceLock<Mutex<ServerRuntime>> = OnceLock::new();

fn runtime_lock() -> &'static Mutex<ServerRuntime> {
    G_RUNTIME.get_or_init(|| {
        Mutex::new(ServerRuntime {
            sock_path: LASAPI_DEBUG_SOCK_DEFAULT.to_string(),
            in_sock_path: LASAPI_DEBUG_IN_SOCK_DEFAULT.to_string(),
            join: None,
        })
    })
}

fn sub_name(sub: u8) -> &'static str {
    match sub {
        0 => "ENGINE",
        1 => "LASAPI",
        2 => "PA",
        3 => "BLUEZ",
        4 => "MIDI",
        5 => "SHIM",
        6 => "ALSA",
        7 => "PW_VIDEO",
        _ => "UNKNOWN",
    }
}

fn lvl_name(lvl: u8) -> &'static str {
    match lvl {
        0 => "TRACE",
        1 => "INFO",
        2 => "WARN",
        3 => "ERR",
        _ => "INFO",
    }
}

pub fn trim_cmd_line(raw: &[u8]) -> &[u8] {
    let end0 = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let mut end = end0;
    while end > 0 && matches!(raw[end - 1], b'\r' | b'\n' | b' ' | b'\t') {
        end -= 1;
    }
    &raw[..end]
}

pub fn parse_dgram_bytes(raw: &[u8]) -> Option<LasapiDbgDgram> {
    if raw.len() < mem::size_of::<LasapiDbgDgram>() {
        return None;
    }
    let magic = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    if magic != LASAPI_DEBUG_DGRAM_MAGIC {
        return None;
    }
    let mut op = [0u8; 28];
    let mut target = [0u8; 48];
    let mut detail = [0u8; 140];
    op.copy_from_slice(&raw[28..56]);
    target.copy_from_slice(&raw[56..104]);
    detail.copy_from_slice(&raw[104..244]);
    op[27] = 0;
    target[47] = 0;
    detail[139] = 0;
    Some(LasapiDbgDgram {
        magic,
        pid: u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
        session_id: u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
        v1: i32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]),
        v2: i32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]),
        hr: i32::from_le_bytes([raw[20], raw[21], raw[22], raw[23]]),
        sub: raw[24],
        lvl: raw[25],
        reserved: u16::from_le_bytes([raw[26], raw[27]]),
        op,
        target,
        detail,
    })
}

fn peer_uid_allowed(peer_uid: u32) -> bool {
    if peer_uid == posix_owner_id() || peer_uid == 0 {
        return true;
    }
    std::env::var("LASAPI_DEBUG_ALLOW_ALL")
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn copy_bytes_nul(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    if dst.is_empty() {
        return;
    }
    let end = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    let n = end.min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
}

#[allow(unsafe_code)]
fn ring_push_event(src_ev: &DbgEvent) {
    let slots = ring_slots();
    let mut pos = G_RING_HEAD.load(Ordering::Relaxed);
    loop {
        let slot = &slots[(pos as usize) & (DBG_RING_CAP - 1)];
        let turn = slot.turn.load(Ordering::Acquire);
        let diff = (turn as i64).wrapping_sub(pos as i64);
        if diff == 0 {
            if G_RING_HEAD
                .compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                let id = G_NEXT_EV_ID.fetch_add(1, Ordering::Relaxed);
                unsafe {
                    let p = slot.ev.get();
                    *p = *src_ev;
                    (*p).id = id;
                }
                slot.turn.store(pos + 1, Ordering::Release);
                return;
            }
        } else if diff < 0 {
            G_DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
            return;
        } else {
            pos = G_RING_HEAD.load(Ordering::Relaxed);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn emit_raw_bytes(
    sub: u8,
    lvl: u8,
    pid: u32,
    sid: u32,
    v1: i32,
    v2: i32,
    hr: i32,
    op: &[u8],
    target: &[u8],
    detail: &[u8],
) {
    if !G_DBG_RUNNING.load(Ordering::Relaxed) && !G_TEST_BYPASS_RUNNING.load(Ordering::Relaxed) {
        return;
    }
    let mut ev = DbgEvent::zeroed();
    ev.ts_mono_ns = mono_ns();
    ev.ts_real_ms = realtime_ms();
    ev.pid = pid;
    ev.sid = sid;
    ev.v1 = v1;
    ev.v2 = v2;
    ev.hr = hr;
    ev.sub = sub;
    ev.lvl = lvl;
    copy_bytes_nul(&mut ev.op, op);
    copy_bytes_nul(&mut ev.target, target);
    copy_bytes_nul(&mut ev.detail, detail);
    ring_push_event(&ev);
}

#[allow(clippy::too_many_arguments)]
pub fn emit_str(
    sub: u8,
    lvl: u8,
    pid: u32,
    sid: u32,
    v1: i32,
    v2: i32,
    hr: i32,
    op: &str,
    target: &str,
    detail: &str,
) {
    emit_raw_bytes(
        sub,
        lvl,
        pid,
        sid,
        v1,
        v2,
        hr,
        op.as_bytes(),
        target.as_bytes(),
        detail.as_bytes(),
    );
}

pub fn record_rt_period(
    cycle_ns: u64,
    wake_jitter_ns: i64,
    budget_ns: u64,
    eq_ns: u64,
    pa_mix_ns: u64,
    hw_write_ns: u64,
) {
    let c = G_RT_CYCLE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    G_RT_LAST_CYCLE_NS.store(cycle_ns, Ordering::Relaxed);
    G_RT_LAST_JITTER_NS.store(wake_jitter_ns, Ordering::Relaxed);
    G_RT_BUDGET_NS.store(budget_ns, Ordering::Relaxed);
    G_RT_EQ_NS.store(eq_ns, Ordering::Relaxed);
    G_RT_PA_MIX_NS.store(pa_mix_ns, Ordering::Relaxed);
    G_RT_HW_WRITE_NS.store(hw_write_ns, Ordering::Relaxed);

    let prev_avg = G_RT_AVG_CYCLE_NS.load(Ordering::Relaxed);
    let new_avg = if c == 1 || prev_avg == 0 {
        cycle_ns
    } else {
        (prev_avg.saturating_mul(31).saturating_add(cycle_ns)) >> 5
    };
    G_RT_AVG_CYCLE_NS.store(new_avg, Ordering::Relaxed);

    if cycle_ns > G_RT_MAX_CYCLE_NS.load(Ordering::Relaxed) {
        G_RT_MAX_CYCLE_NS.store(cycle_ns, Ordering::Relaxed);
    }
    if cycle_ns < G_RT_MIN_CYCLE_NS.load(Ordering::Relaxed) {
        G_RT_MIN_CYCLE_NS.store(cycle_ns, Ordering::Relaxed);
    }
    let abs_jit = wake_jitter_ns.unsigned_abs();
    if abs_jit > G_RT_MAX_JITTER_NS.load(Ordering::Relaxed) {
        G_RT_MAX_JITTER_NS.store(abs_jit, Ordering::Relaxed);
    }
}

pub fn get_rt_stats() -> LasapiDbgRtStats {
    let count = G_RT_CYCLE_COUNT.load(Ordering::Relaxed);
    let min_v = G_RT_MIN_CYCLE_NS.load(Ordering::Relaxed);
    LasapiDbgRtStats {
        cycle_count: count,
        last_cycle_ns: G_RT_LAST_CYCLE_NS.load(Ordering::Relaxed),
        avg_cycle_ns: G_RT_AVG_CYCLE_NS.load(Ordering::Relaxed),
        max_cycle_ns: G_RT_MAX_CYCLE_NS.load(Ordering::Relaxed),
        min_cycle_ns: if count > 0 && min_v != u64::MAX {
            min_v
        } else {
            0
        },
        last_jitter_ns: G_RT_LAST_JITTER_NS.load(Ordering::Relaxed),
        max_jitter_ns: G_RT_MAX_JITTER_NS.load(Ordering::Relaxed),
        eq_ns: G_RT_EQ_NS.load(Ordering::Relaxed),
        pa_mix_ns: G_RT_PA_MIX_NS.load(Ordering::Relaxed),
        hw_write_ns: G_RT_HW_WRITE_NS.load(Ordering::Relaxed),
        budget_ns: G_RT_BUDGET_NS.load(Ordering::Relaxed),
        hw_xruns: G_RT_HW_XRUNS.load(Ordering::Relaxed),
    }
}

pub fn reset_rt_stats() {
    G_RT_CYCLE_COUNT.store(0, Ordering::Relaxed);
    G_RT_LAST_CYCLE_NS.store(0, Ordering::Relaxed);
    G_RT_AVG_CYCLE_NS.store(0, Ordering::Relaxed);
    G_RT_MAX_CYCLE_NS.store(0, Ordering::Relaxed);
    G_RT_MIN_CYCLE_NS.store(u64::MAX, Ordering::Relaxed);
    G_RT_LAST_JITTER_NS.store(0, Ordering::Relaxed);
    G_RT_MAX_JITTER_NS.store(0, Ordering::Relaxed);
    G_RT_EQ_NS.store(0, Ordering::Relaxed);
    G_RT_PA_MIX_NS.store(0, Ordering::Relaxed);
    G_RT_HW_WRITE_NS.store(0, Ordering::Relaxed);
    G_RT_HW_XRUNS.store(0, Ordering::Relaxed);
}

pub fn rt_stats_to_json(rt: &LasapiDbgRtStats) -> JsonValue {
    let cpu_pct = if rt.budget_ns > 0 {
        (rt.last_cycle_ns as f64) * 100.0 / (rt.budget_ns as f64)
    } else {
        0.0
    };
    JsonValue::obj(vec![
        ("cycle_count", JsonValue::from_u64(rt.cycle_count)),
        ("last_cycle_us", JsonValue::rounded_float(rt.last_cycle_ns as f64 / 1000.0, 10.0)),
        ("avg_cycle_us", JsonValue::rounded_float(rt.avg_cycle_ns as f64 / 1000.0, 10.0)),
        ("max_cycle_us", JsonValue::rounded_float(rt.max_cycle_ns as f64 / 1000.0, 10.0)),
        ("min_cycle_us", JsonValue::rounded_float(rt.min_cycle_ns as f64 / 1000.0, 10.0)),
        ("last_jitter_us", JsonValue::rounded_float(rt.last_jitter_ns as f64 / 1000.0, 10.0)),
        ("max_jitter_us", JsonValue::rounded_float(rt.max_jitter_ns as f64 / 1000.0, 10.0)),
        ("eq_us", JsonValue::rounded_float(rt.eq_ns as f64 / 1000.0, 10.0)),
        ("pa_mix_us", JsonValue::rounded_float(rt.pa_mix_ns as f64 / 1000.0, 10.0)),
        ("hw_write_us", JsonValue::rounded_float(rt.hw_write_ns as f64 / 1000.0, 10.0)),
        ("budget_us", JsonValue::rounded_float(rt.budget_ns as f64 / 1000.0, 10.0)),
        ("cpu_load_pct", JsonValue::rounded_float(cpu_pct, 100.0)),
    ])
}

pub fn wire_stats_to_json() -> JsonValue {
    let fs = failsafe::get_stats();
    JsonValue::obj(vec![
        (
            "backend",
            JsonValue::Str(if fs.is_rust { "rust" } else { "c_fallback" }.into()),
        ),
        ("is_rust", JsonValue::Int(i64::from(fs.is_rust))),
        ("armed", JsonValue::Int(i64::from(fs.armed))),
        ("trips_total", JsonValue::from_u64(fs.trips_total)),
        ("pa_quarantined", JsonValue::from_u64(fs.pa_quarantined)),
        ("pw_quarantined", JsonValue::from_u64(fs.pw_quarantined)),
        ("bt_fallbacks", JsonValue::from_u64(fs.bt_fallbacks)),
        ("fd_leaks_blocked", JsonValue::from_u64(fs.fd_leaks_blocked)),
        ("manual_resets", JsonValue::from_u64(fs.manual_resets)),
        ("last_reason", JsonValue::Str(c_str_to_string(&fs.last_reason))),
    ])
}

fn serialize_event_json(ev: &DbgEvent) -> String {
    JsonValue::obj(vec![
        ("type", JsonValue::Str("event".into())),
        ("id", JsonValue::from_u64(ev.id)),
        ("ts_mono_ns", JsonValue::from_u64(ev.ts_mono_ns)),
        ("ts_ms", JsonValue::Int(ev.ts_real_ms)),
        ("sub", JsonValue::Str(sub_name(ev.sub).into())),
        ("lvl", JsonValue::Str(lvl_name(ev.lvl).into())),
        ("pid", JsonValue::Int(i64::from(ev.pid))),
        ("sid", JsonValue::Int(i64::from(ev.sid))),
        ("v1", JsonValue::Int(i64::from(ev.v1))),
        ("v2", JsonValue::Int(i64::from(ev.v2))),
        ("hr", JsonValue::Int(i64::from(ev.hr))),
        ("op", JsonValue::Str(c_str_to_string(&ev.op))),
        ("target", JsonValue::Str(c_str_to_string(&ev.target))),
        ("detail", JsonValue::Str(c_str_to_string(&ev.detail))),
    ])
    .to_json_compact()
}

struct HistEntry {
    id: u64,
    line: String,
}

struct DbgClient {
    stream: UnixStream,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    closed: bool,
}

impl DbgClient {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            inbuf: Vec::with_capacity(512),
            outbuf: Vec::with_capacity(16384),
            closed: false,
        }
    }

    fn queue_line(&mut self, line: &str) {
        if self.closed {
            return;
        }
        let need = line.len().saturating_add(1);
        if self.outbuf.len().saturating_add(need) > DBG_CLIENT_MAX_OUT {
            self.closed = true;
            return;
        }
        self.outbuf.extend_from_slice(line.as_bytes());
        self.outbuf.push(b'\n');
    }

    fn flush(&mut self) {
        while !self.closed && !self.outbuf.is_empty() {
            match nb_send(self.stream.as_raw_fd(), &self.outbuf) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => {
                    self.outbuf.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
    }
}

struct ServerLoopState {
    host: Arc<dyn DebugEngineHost>,
    sock_path: String,
    listener: Option<UnixListener>,
    dgram: Option<UnixDatagram>,
    clients: Vec<DbgClient>,
    imp_hist: VecDeque<HistEntry>,
    trace_hist: VecDeque<HistEntry>,
    ring_tail: u64,
}

impl ServerLoopState {
    fn new(
        host: Arc<dyn DebugEngineHost>,
        sock_path: String,
        listener: Option<UnixListener>,
        dgram: Option<UnixDatagram>,
    ) -> Self {
        Self {
            host,
            sock_path,
            listener,
            dgram,
            clients: Vec::with_capacity(DBG_MAX_CLIENTS),
            imp_hist: VecDeque::with_capacity(DBG_HISTORY_CAP),
            trace_hist: VecDeque::with_capacity(DBG_HISTORY_CAP),
            ring_tail: 0,
        }
    }

    fn history_push(&mut self, id: u64, lvl: u8, line: String) {
        let q = if lvl >= DBG_LVL_INFO {
            &mut self.imp_hist
        } else {
            &mut self.trace_hist
        };
        if q.len() >= DBG_HISTORY_CAP {
            q.pop_front();
        }
        q.push_back(HistEntry { id, line });
    }

    fn broadcast_line(&mut self, line: &str) {
        for c in &mut self.clients {
            c.queue_line(line);
        }
    }

    fn build_snapshot_line(&self) -> Option<String> {
        let rt = get_rt_stats();
        let host_val = self.host.build_snapshot_json(&rt);
        let mut entries = vec![
            ("type".to_string(), JsonValue::Str("telemetry".into())),
            ("ts_ms".to_string(), JsonValue::Int(realtime_ms())),
            ("ts_mono_ns".to_string(), JsonValue::from_u64(mono_ns())),
            (
                "dropped_events".to_string(),
                JsonValue::from_u64(G_DROPPED_EVENTS.load(Ordering::Relaxed)),
            ),
        ];
        if let JsonValue::Object(host_entries) = host_val {
            for (k, v) in host_entries {
                entries.push((k, v));
            }
        }
        if !entries.iter().any(|(k, _)| k == "rt") {
            entries.push(("rt".to_string(), rt_stats_to_json(&rt)));
        }
        if !entries.iter().any(|(k, _)| k == "wire") {
            entries.push(("wire".to_string(), wire_stats_to_json()));
        }
        Some(JsonValue::Object(entries).to_json_compact())
    }

    fn send_snapshot(&mut self, target_idx: Option<usize>) {
        let Some(line) = self.build_snapshot_line() else {
            return;
        };
        if let Some(idx) = target_idx {
            if let Some(c) = self.clients.get_mut(idx) {
                c.queue_line(&line);
            }
        } else {
            self.broadcast_line(&line);
        }
    }

    fn handle_client_command(&mut self, idx: usize, raw_cmd: &[u8]) {
        let cmd = trim_cmd_line(raw_cmd);
        if cmd.is_empty() {
            return;
        }
        if cmd == b"SNAPSHOT" {
            self.send_snapshot(Some(idx));
        } else if cmd == b"RESET_STATS" || cmd == b"RESET_RT_STATS" {
            reset_rt_stats();
            emit_str(
                0,
                DBG_LVL_INFO,
                current_pid(),
                0,
                0,
                0,
                0,
                "RESET_STATS",
                "rt_telemetry",
                "RT min/max/jitter statistics reset via debugger",
            );
            self.send_snapshot(Some(idx));
        } else if cmd == b"PING" || cmd.starts_with(b"PING ") {
            let tag = if cmd.len() > 5 { &cmd[5..] } else { b"" };
            let s = JsonValue::obj(vec![
                ("type", JsonValue::Str("pong".into())),
                ("ts_ms", JsonValue::Int(realtime_ms())),
                ("ts_mono_ns", JsonValue::from_u64(mono_ns())),
                ("tag", JsonValue::Str(String::from_utf8_lossy(tag).into_owned())),
            ])
            .to_json_compact();
            if let Some(c) = self.clients.get_mut(idx) {
                c.queue_line(&s);
            }
        } else {
            let cmd_str = String::from_utf8_lossy(cmd).into_owned();
            let (ok, rc, msg) = match self.host.handle_debug_cmd(&cmd_str) {
                Ok(m) => (true, 0, m),
                Err((r, m)) => (false, r, m),
            };
            let s = JsonValue::obj(vec![
                ("type", JsonValue::Str("cmd_reply".into())),
                ("ok", JsonValue::Bool(ok)),
                ("rc", JsonValue::Int(i64::from(rc))),
                ("cmd", JsonValue::Str(cmd_str)),
                ("msg", JsonValue::Str(msg)),
            ])
            .to_json_compact();
            if let Some(c) = self.clients.get_mut(idx) {
                c.queue_line(&s);
            }
        }
    }

    fn accept_new_client(&mut self) {
        let Some(ref lst) = self.listener else {
            return;
        };
        let Ok((stream, _)) = lst.accept() else {
            return;
        };
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let Some(uc) = socket_peercred(stream.as_raw_fd()) else {
            return;
        };
        if !peer_uid_allowed(uc.uid) || self.clients.len() >= DBG_MAX_CLIENTS {
            return;
        }

        let mut c = DbgClient::new(stream);
        let hello = JsonValue::obj(vec![
            ("type", JsonValue::Str("hello".into())),
            ("version", JsonValue::Int(1)),
            ("pid", JsonValue::Int(i64::from(current_pid()))),
            ("ts_ms", JsonValue::Int(realtime_ms())),
            ("sock", JsonValue::Str(self.sock_path.clone())),
            (
                "dropped_events",
                JsonValue::from_u64(G_DROPPED_EVENTS.load(Ordering::Relaxed)),
            ),
            (
                "history_count",
                JsonValue::Int(
                    i64::try_from(self.imp_hist.len() + self.trace_hist.len()).unwrap_or(i64::MAX),
                ),
            ),
        ])
        .to_json_compact();
        c.queue_line(&hello);

        let mut ii = 0usize;
        let mut ti = 0usize;
        while ii < self.imp_hist.len() || ti < self.trace_hist.len() {
            let pick_imp = if ii < self.imp_hist.len() && ti < self.trace_hist.len() {
                self.imp_hist[ii].id <= self.trace_hist[ti].id
            } else {
                ii < self.imp_hist.len()
            };
            if pick_imp {
                c.queue_line(&self.imp_hist[ii].line);
                ii += 1;
            } else {
                c.queue_line(&self.trace_hist[ti].line);
                ti += 1;
            }
        }

        self.clients.push(c);
        let idx = self.clients.len() - 1;
        self.send_snapshot(Some(idx));
        if let Some(cl) = self.clients.get_mut(idx) {
            cl.flush();
        }
    }

    fn drain_dgram_socket(&mut self) {
        let Some(dg_fd) = self.dgram.as_ref().map(AsRawFd::as_raw_fd) else {
            return;
        };
        let mut raw = [0u8; 512];
        for _ in 0..64 {
            match recv_dgram_with_ucred(dg_fd, &mut raw) {
                RecvDgramCred::Packet { len, ucred } => {
                    let Some(uc) = ucred else {
                        continue;
                    };
                    if !peer_uid_allowed(uc.uid) {
                        continue;
                    }
                    let Some(dg) = parse_dgram_bytes(&raw[..len]) else {
                        continue;
                    };
                    let mut ev = DbgEvent::zeroed();
                    ev.id = G_NEXT_EV_ID.fetch_add(1, Ordering::Relaxed);
                    ev.ts_mono_ns = mono_ns();
                    ev.ts_real_ms = realtime_ms();
                    ev.pid = dg.pid;
                    ev.sid = dg.session_id;
                    ev.v1 = dg.v1;
                    ev.v2 = dg.v2;
                    ev.hr = dg.hr;
                    ev.sub = dg.sub;
                    ev.lvl = dg.lvl;
                    copy_bytes_nul(&mut ev.op, &dg.op);
                    copy_bytes_nul(&mut ev.target, &dg.target);
                    copy_bytes_nul(&mut ev.detail, &dg.detail);

                    let line = serialize_event_json(&ev);
                    self.history_push(ev.id, ev.lvl, line.clone());
                    self.broadcast_line(&line);
                }
                RecvDgramCred::WouldBlock | RecvDgramCred::Err => break,
            }
        }
    }

    #[allow(unsafe_code)]
    fn drain_rt_ring(&mut self) {
        let slots = ring_slots();
        for _ in 0..512 {
            let slot = &slots[(self.ring_tail as usize) & (DBG_RING_CAP - 1)];
            let turn = slot.turn.load(Ordering::Acquire);
            if turn != self.ring_tail + 1 {
                break;
            }
            let ev = unsafe { *slot.ev.get() };
            slot.turn
                .store(self.ring_tail + DBG_RING_CAP as u64, Ordering::Release);
            self.ring_tail += 1;

            let line = serialize_event_json(&ev);
            self.history_push(ev.id, ev.lvl, line.clone());
            self.broadcast_line(&line);
        }
    }

    fn service_client_read(&mut self, idx: usize) {
        let mut rx = [0u8; 512];
        loop {
            let Some(c) = self.clients.get(idx) else {
                return;
            };
            if c.closed {
                return;
            }
            let fd = c.stream.as_raw_fd();
            match nb_recv(fd, &mut rx) {
                Ok(0) => {
                    self.clients[idx].closed = true;
                    return;
                }
                Ok(n) => {
                    self.clients[idx].inbuf.extend_from_slice(&rx[..n]);
                    while let Some(pos) = self.clients[idx].inbuf.iter().position(|&b| b == b'\n') {
                        let cmd_bytes: Vec<u8> = self.clients[idx].inbuf.drain(..=pos).collect();
                        let line = &cmd_bytes[..cmd_bytes.len() - 1];
                        self.handle_client_command(idx, line);
                        if self.clients[idx].closed {
                            return;
                        }
                    }
                    if self.clients[idx].inbuf.len() >= 1024 {
                        self.clients[idx].inbuf.clear();
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.clients[idx].closed = true;
                    return;
                }
            }
        }
    }

    fn run_loop(&mut self) {
        let mut last_telem_ms: i64 = 0;
        let mut pfds = Vec::with_capacity(2 + DBG_MAX_CLIENTS);
        let mut tags = Vec::with_capacity(2 + DBG_MAX_CLIENTS);

        while G_DBG_RUNNING.load(Ordering::Relaxed) {
            pfds.clear();
            tags.clear();

            if let Some(ref lst) = self.listener {
                pfds.push(PollFd {
                    fd: lst.as_raw_fd(),
                    events: POLLIN,
                    revents: 0,
                });
                tags.push(-1i32);
            }
            if let Some(ref dg) = self.dgram {
                pfds.push(PollFd {
                    fd: dg.as_raw_fd(),
                    events: POLLIN,
                    revents: 0,
                });
                tags.push(-2i32);
            }
            for (ci, c) in self.clients.iter().enumerate() {
                if !c.closed {
                    pfds.push(PollFd {
                        fd: c.stream.as_raw_fd(),
                        events: POLLIN | if c.outbuf.is_empty() { 0 } else { POLLOUT },
                        revents: 0,
                    });
                    tags.push(ci as i32);
                }
            }

            let _ = do_poll(&mut pfds, 25);

            for i in 0..pfds.len() {
                let rev = pfds[i].revents;
                if rev == 0 {
                    continue;
                }
                let tag = tags[i];
                if tag == -1 && (rev & POLLIN) != 0 {
                    self.accept_new_client();
                } else if tag == -2 && (rev & POLLIN) != 0 {
                    self.drain_dgram_socket();
                } else if tag >= 0 {
                    let ci = tag as usize;
                    if ci >= self.clients.len() {
                        continue;
                    }
                    if (rev & POLLIN) != 0 {
                        self.service_client_read(ci);
                    }
                    if ci < self.clients.len() && !self.clients[ci].closed && (rev & (POLLERR | POLLHUP | POLLNVAL)) != 0 {
                        self.clients[ci].closed = true;
                        continue;
                    }
                    if ci < self.clients.len() && !self.clients[ci].closed && (rev & POLLOUT) != 0 {
                        self.clients[ci].flush();
                    }
                }
            }

            self.clients.retain(|c| !c.closed);
            self.drain_rt_ring();
            self.drain_dgram_socket();

            let now_ms = realtime_ms();
            if !self.clients.is_empty() && now_ms.saturating_sub(last_telem_ms) >= DBG_TELEM_INTERVAL_MS {
                last_telem_ms = now_ms;
                self.send_snapshot(None);
            }

            for c in &mut self.clients {
                if !c.outbuf.is_empty() {
                    c.flush();
                }
            }
            self.clients.retain(|c| !c.closed);
        }
    }
}

pub fn debug_start(host: Arc<dyn DebugEngineHost>) -> i32 {
    if G_DBG_RUNNING.load(Ordering::Relaxed) {
        return 0;
    }

    let mut rt = runtime_lock().lock().unwrap_or_else(|p| p.into_inner());
    if G_DBG_RUNNING.load(Ordering::Relaxed) {
        return 0;
    }

    G_RING_HEAD.store(0, Ordering::Relaxed);
    G_NEXT_EV_ID.store(1, Ordering::Relaxed);
    G_DROPPED_EVENTS.store(0, Ordering::Relaxed);
    let slots = ring_slots();
    for (i, slot) in slots.iter().enumerate() {
        slot.turn.store(i as u64, Ordering::Relaxed);
    }
    reset_rt_stats();

    let sock_path = resolve_sock_path_common("LASAPI_DEBUG_SOCK", "debug.sock", LASAPI_DEBUG_SOCK_DEFAULT);
    let in_sock_path =
        resolve_sock_path_common("LASAPI_DEBUG_IN_SOCK", "debug-in.sock", LASAPI_DEBUG_IN_SOCK_DEFAULT);
    if sock_path != LASAPI_DEBUG_SOCK_DEFAULT {
        let _ = fs::remove_file(LASAPI_DEBUG_SOCK_DEFAULT);
    }
    if in_sock_path != LASAPI_DEBUG_IN_SOCK_DEFAULT {
        let _ = fs::remove_file(LASAPI_DEBUG_IN_SOCK_DEFAULT);
    }

    let listener = bind_unix_listener_0600(&sock_path);
    let dgram = bind_unix_dgram_passcred_0600(&in_sock_path);

    rt.sock_path = sock_path.clone();
    rt.in_sock_path = in_sock_path.clone();

    G_DBG_RUNNING.store(true, Ordering::Release);
    let mut loop_state = ServerLoopState::new(host, sock_path.clone(), listener, dgram);
    let builder = thread::Builder::new().name("salsa-debug-srv".to_string());
    match builder.spawn(move || {
        loop_state.run_loop();
    }) {
        Ok(jh) => {
            rt.join = Some(jh);
        }
        Err(_) => {
            G_DBG_RUNNING.store(false, Ordering::Release);
            let _ = fs::remove_file(&sock_path);
            let _ = fs::remove_file(&in_sock_path);
            return -1;
        }
    }
    drop(rt);

    emit_str(
        0,
        DBG_LVL_INFO,
        current_pid(),
        0,
        0,
        0,
        0,
        "DEBUG_START",
        &sock_path,
        "SALSA live telemetry & interaction stream initialized",
    );
    0
}

pub fn debug_stop() {
    if !G_DBG_RUNNING.swap(false, Ordering::AcqRel) {
        return;
    }
    let mut rt = runtime_lock().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(jh) = rt.join.take() {
        let _ = jh.join();
    }
    let _ = fs::remove_file(&rt.sock_path);
    let _ = fs::remove_file(&rt.in_sock_path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct TestDebugHost;

    impl DebugEngineHost for TestDebugHost {
        fn build_snapshot_json(&self, rt: &LasapiDbgRtStats) -> JsonValue {
            JsonValue::obj(vec![
                (
                    "engine",
                    JsonValue::obj(vec![
                        ("hw_device", JsonValue::Str("Rust Test Sink".into())),
                        ("sample_rate", JsonValue::Int(192_000)),
                        ("channels", JsonValue::Int(2)),
                    ]),
                ),
                ("rt", rt_stats_to_json(rt)),
                ("bluez", JsonValue::obj(vec![])),
                ("capture", JsonValue::obj(vec![])),
                ("midi", JsonValue::obj(vec![])),
                ("lasapi_sessions", JsonValue::Array(Vec::new())),
                ("pa_clients", JsonValue::Array(Vec::new())),
                ("pa_streams", JsonValue::Array(Vec::new())),
                (
                    "pw",
                    JsonValue::obj(vec![("broker_running", JsonValue::Int(1))]),
                ),
                ("wire", wire_stats_to_json()),
            ])
        }

        fn handle_debug_cmd(&self, cmd: &str) -> Result<String, (i32, String)> {
            if cmd == "PW_SYNC_MODE" {
                Ok("sync_mode rc=0".into())
            } else {
                Err((-1, "unknown".into()))
            }
        }
    }

    #[test]
    fn dgram_and_json_escape_validation() {
        assert!(parse_dgram_bytes(&[0u8; 20]).is_none());
        let mut raw = [0u8; mem::size_of::<LasapiDbgDgram>()];
        raw[0..4].copy_from_slice(&LASAPI_DEBUG_DGRAM_MAGIC.to_le_bytes());
        raw[24] = 2;
        raw[25] = 1;
        raw[28..32].copy_from_slice(b"PING");
        let parsed = parse_dgram_bytes(&raw).unwrap();
        assert_eq!(parsed.magic, LASAPI_DEBUG_DGRAM_MAGIC);
        assert_eq!(parsed.sub, 2);

        let escaped = JsonValue::Str("hi\"\\\n\x07ok".into()).to_json_compact();
        assert_eq!(escaped, "\"hi\\\"\\\\\\n\\u0007ok\"");
        assert_eq!(trim_cmd_line(b" SNAPSHOT \r\n\t "), b" SNAPSHOT");
    }

    #[test]
    fn rt_stats_and_server_stream_roundtrip() {
        let _g = TEST_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        reset_rt_stats();
        record_rt_period(2000, -150, 666_000, 300, 100, 50);
        record_rt_period(4000, 350, 666_000, 300, 100, 50);
        let st = get_rt_stats();
        assert_eq!(st.cycle_count, 2);
        assert_eq!(st.min_cycle_ns, 2000);
        assert_eq!(st.max_cycle_ns, 4000);
        assert_eq!(st.max_jitter_ns, 350);

        let tmp = format!("/tmp/lasapi-dbg-rust-test-{}", std::process::id());
        let _ = fs::create_dir_all(&tmp);
        let sock = format!("{tmp}/dbg.sock");
        let in_sock = format!("{tmp}/dbg-in.sock");
        std::env::set_var("LASAPI_DEBUG_SOCK", &sock);
        std::env::set_var("LASAPI_DEBUG_IN_SOCK", &in_sock);

        assert_eq!(debug_start(Arc::new(TestDebugHost)), 0);

        emit_str(2, 1, 42, 7, 1, 2, 0, "PA_OPEN", "Discord", "test stream");

        let mut client = UnixStream::connect(&sock).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();

        // Expect hello
        reader.read_line(&mut line).unwrap();
        let hello_json = JsonValue::parse(line.trim()).expect("hello json");
        assert_eq!(hello_json.get("type").and_then(JsonValue::as_str), Some("hello"));

        // Read until telemetry snapshot and PA_OPEN event both appear, verifying full schema via JsonValue::parse
        let mut saw_snap = false;
        let mut saw_event = false;
        for _ in 0..8 {
            line.clear();
            reader.read_line(&mut line).unwrap();
            let parsed = JsonValue::parse(line.trim()).expect("valid debug line json");
            match parsed.get("type").and_then(JsonValue::as_str) {
                Some("event") => {
                    if parsed.get("op").and_then(JsonValue::as_str) == Some("PA_OPEN") {
                        assert_eq!(parsed.get("target").and_then(JsonValue::as_str), Some("Discord"));
                        saw_event = true;
                    }
                }
                Some("telemetry") => {
                    for key in [
                        "type",
                        "ts_ms",
                        "ts_mono_ns",
                        "dropped_events",
                        "engine",
                        "rt",
                        "bluez",
                        "capture",
                        "midi",
                        "lasapi_sessions",
                        "pa_clients",
                        "pa_streams",
                        "pw",
                        "wire",
                    ] {
                        assert!(parsed.get(key).is_some(), "missing telemetry key {key}");
                    }
                    assert_eq!(
                        parsed.get("engine").and_then(|e| e.get("hw_device")).and_then(JsonValue::as_str),
                        Some("Rust Test Sink")
                    );
                    assert_eq!(
                        parsed.get("wire").and_then(|w| w.get("backend")).and_then(JsonValue::as_str),
                        Some("rust")
                    );
                    saw_snap = true;
                }
                _ => {}
            }
            if saw_snap && saw_event {
                break;
            }
        }
        assert!(saw_snap, "expected telemetry snapshot");
        assert!(saw_event, "expected parsed event");

        client.write_all(b"PING hello\n").unwrap();
        client.write_all(b"PW_SYNC_MODE\n").unwrap();
        client.write_all(b"RESET_STATS\n").unwrap();
        client.flush().unwrap();

        let mut saw_pong = false;
        let mut saw_reply = false;
        let mut saw_reset = false;
        for _ in 0..16 {
            line.clear();
            if reader.read_line(&mut line).is_err() {
                break;
            }
            let parsed = JsonValue::parse(line.trim()).expect("reply line json");
            match parsed.get("type").and_then(JsonValue::as_str) {
                Some("pong") if parsed.get("tag").and_then(JsonValue::as_str) == Some("hello") => {
                    saw_pong = true;
                }
                Some("cmd_reply")
                    if parsed.get("msg").and_then(JsonValue::as_str) == Some("sync_mode rc=0") =>
                {
                    saw_reply = true;
                }
                Some("telemetry")
                    if parsed.get("rt").and_then(|r| r.get("cycle_count")).and_then(JsonValue::as_i64)
                        == Some(0) =>
                {
                    saw_reset = true;
                }
                _ => {}
            }
            if saw_pong && saw_reply && saw_reset {
                break;
            }
        }
        assert!(saw_pong && saw_reply && saw_reset);

        debug_stop();
        std::env::remove_var("LASAPI_DEBUG_SOCK");
        std::env::remove_var("LASAPI_DEBUG_IN_SOCK");
        let _ = fs::remove_dir_all(&tmp);
    }
}
