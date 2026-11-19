// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int};
use core::fmt::Write as _;
use core::mem;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, LwasapiDbgCallbacks, LwasapiDbgDgram, LwasapiDbgRtStats,
    LwasapiDbgSnapshot, LWASAPI_DEBUG_MAX_PA_CLIENTS, LWASAPI_DEBUG_MAX_PA_STREAMS,
    LWASAPI_DEBUG_MAX_PW_CLIENTS, LWASAPI_DEBUG_MAX_PW_LINKS, LWASAPI_DEBUG_MAX_PW_NODES,
    LWASAPI_MAX_SESSIONS,
};
use crate::failsafe;
use crate::unix_sys::{
    bind_unix_dgram_passcred_0600, bind_unix_listener_0600, current_pid, current_uid, do_poll,
    mono_ns, nb_recv, nb_send, realtime_ms, recv_dgram_with_ucred, resolve_sock_path_common,
    socket_peercred, PollFd, RecvDgramCred, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT,
};

pub const LWASAPI_DEBUG_SOCK_DEFAULT: &str = "/tmp/lwasapi-debug.sock";
pub const LWASAPI_DEBUG_IN_SOCK_DEFAULT: &str = "/tmp/lwasapi-debug-in.sock";
pub const LWASAPI_DEBUG_DGRAM_MAGIC: u32 = 0x4C44_4247;

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

struct ServerRuntime {
    cbs: LwasapiDbgCallbacks,
    sock_path: String,
    in_sock_path: String,
    join: Option<JoinHandle<()>>,
}

static G_RUNTIME: OnceLock<Mutex<ServerRuntime>> = OnceLock::new();

fn runtime_lock() -> &'static Mutex<ServerRuntime> {
    G_RUNTIME.get_or_init(|| {
        Mutex::new(ServerRuntime {
            cbs: LwasapiDbgCallbacks::default(),
            sock_path: LWASAPI_DEBUG_SOCK_DEFAULT.to_string(),
            in_sock_path: LWASAPI_DEBUG_IN_SOCK_DEFAULT.to_string(),
            join: None,
        })
    })
}

fn sub_name(sub: u8) -> &'static str {
    match sub {
        0 => "ENGINE",
        1 => "WASAPI",
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

#[inline]
fn finite_d(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

pub fn escape_json_str(bytes: &[u8], out: &mut String) {
    out.push('"');
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\x08' => out.push_str("\\b"),
            b'\x0c' => out.push_str("\\f"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0..=0x1F => {
                let _ = write!(out, "\\u{c:04x}");
            }
            0x20..=0x7F => out.push(c as char),
            _ => {
                // Decode valid UTF-8 sequence if possible so multi-byte strings stay intact,
                // otherwise pass raw byte escaped/lossy consistently with C octet passthrough.
                match core::str::from_utf8(&bytes[i..]) {
                    Ok(valid) => {
                        let ch = valid.chars().next().unwrap();
                        out.push(ch);
                        i += ch.len_utf8();
                        continue;
                    }
                    Err(e) => {
                        let valid_len = e.valid_up_to();
                        if valid_len > 0 {
                            if let Ok(seg) = core::str::from_utf8(&bytes[i..i + valid_len]) {
                                if let Some(ch) = seg.chars().next() {
                                    out.push(ch);
                                    i += ch.len_utf8();
                                    continue;
                                }
                            }
                        }
                        let _ = write!(out, "\\u{c:04x}");
                    }
                }
            }
        }
        i += 1;
    }
    out.push('"');
}

fn append_c_json_str(out: &mut String, src: &[u8]) {
    let end = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    escape_json_str(&src[..end], out);
}

pub fn trim_cmd_line(raw: &[u8]) -> &[u8] {
    let end0 = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let mut end = end0;
    while end > 0 && matches!(raw[end - 1], b'\r' | b'\n' | b' ' | b'\t') {
        end -= 1;
    }
    &raw[..end]
}

pub fn parse_dgram_bytes(raw: &[u8]) -> Option<LwasapiDbgDgram> {
    if raw.len() < mem::size_of::<LwasapiDbgDgram>() {
        return None;
    }
    let magic = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    if magic != LWASAPI_DEBUG_DGRAM_MAGIC {
        return None;
    }
    let mut dg: LwasapiDbgDgram = unsafe { mem::zeroed() };
    let dst = unsafe {
        core::slice::from_raw_parts_mut(
            (&mut dg as *mut LwasapiDbgDgram).cast::<u8>(),
            mem::size_of::<LwasapiDbgDgram>(),
        )
    };
    dst.copy_from_slice(&raw[..mem::size_of::<LwasapiDbgDgram>()]);
    dg.op[dg.op.len() - 1] = 0;
    dg.target[dg.target.len() - 1] = 0;
    dg.detail[dg.detail.len() - 1] = 0;
    Some(dg)
}

fn peer_uid_allowed(peer_uid: u32) -> bool {
    if peer_uid == current_uid() || peer_uid == 0 {
        return true;
    }
    std::env::var("LWASAPI_DEBUG_ALLOW_ALL")
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

pub fn get_rt_stats() -> LwasapiDbgRtStats {
    let count = G_RT_CYCLE_COUNT.load(Ordering::Relaxed);
    let min_v = G_RT_MIN_CYCLE_NS.load(Ordering::Relaxed);
    LwasapiDbgRtStats {
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

fn serialize_event_json(ev: &DbgEvent, out: &mut String) {
    out.clear();
    let _ = write!(
        out,
        "{{\"type\":\"event\",\"id\":{},\"ts_mono_ns\":{},\"ts_ms\":{},\"sub\":",
        ev.id, ev.ts_mono_ns, ev.ts_real_ms
    );
    escape_json_str(sub_name(ev.sub).as_bytes(), out);
    out.push_str(",\"lvl\":");
    escape_json_str(lvl_name(ev.lvl).as_bytes(), out);
    let _ = write!(
        out,
        ",\"pid\":{},\"sid\":{},\"v1\":{},\"v2\":{},\"hr\":{},\"op\":",
        ev.pid, ev.sid, ev.v1, ev.v2, ev.hr
    );
    append_c_json_str(out, &ev.op);
    out.push_str(",\"target\":");
    append_c_json_str(out, &ev.target);
    out.push_str(",\"detail\":");
    append_c_json_str(out, &ev.detail);
    out.push('}');
}

fn serialize_snapshot_json(snap: &LwasapiDbgSnapshot, out: &mut String) {
    out.clear();
    let st = &snap.engine_st;
    let rt = &snap.rt;
    let cpu_pct = if rt.budget_ns > 0 {
        finite_d((rt.last_cycle_ns as f64) * 100.0 / (rt.budget_ns as f64))
    } else {
        0.0
    };

    let _ = write!(
        out,
        "{{\"type\":\"telemetry\",\"ts_ms\":{},\"ts_mono_ns\":{},\"dropped_events\":{},",
        realtime_ms(),
        mono_ns(),
        G_DROPPED_EVENTS.load(Ordering::Relaxed)
    );

    out.push_str("\"engine\":{\"hw_device\":");
    append_c_json_str(out, &st.hw_device);
    out.push_str(",\"hw_capture_device\":");
    append_c_json_str(out, &st.hw_capture_device);
    out.push_str(",\"midi_device\":");
    append_c_json_str(out, &st.midi_device);
    out.push_str(",\"active_playback_id\":");
    if st.active_playback_id[0] != 0 {
        append_c_json_str(out, &st.active_playback_id);
    } else {
        append_c_json_str(out, &snap.target_playback_id);
    }
    out.push_str(",\"active_capture_id\":");
    if st.active_capture_id[0] != 0 {
        append_c_json_str(out, &st.active_capture_id);
    } else {
        append_c_json_str(out, &snap.target_capture_id);
    }
    let _ = write!(
        out,
        ",\"sample_rate\":{},\"channels\":{},\"period_frames\":{},\"buffer_frames\":{},\
        \"format_bits\":{},\"desktop_volume_pct\":{},\"desktop_muted\":{},\
        \"desktop_mic_volume_pct\":{},\"desktop_mic_muted\":{},\
        \"realtime_priority\":{},\"exclusive_owner_id\":{},\
        \"total_cycles\":{},\"hw_xruns\":{},\"midi_events_routed\":{},\
        \"eq_enabled\":{},\"eq_dynamic_mode\":{},\"eq_preamp_db\":{:.2},\
        \"eq_dyn_threshold_db\":{:.2},\"eq_dyn_ratio\":{:.2},\"eq_limiter_enabled\":{},\
        \"output_peak_l\":{:.4},\"output_peak_r\":{:.4},\
        \"pa_server_running\":{},\"pa_clients\":{},\"eq_preset\":",
        st.hw_sample_rate,
        st.hw_channels,
        st.hw_period_frames,
        st.hw_buffer_frames,
        st.hw_format_bits,
        st.desktop_volume_pct,
        st.desktop_muted,
        st.desktop_mic_volume_pct,
        st.desktop_mic_muted,
        st.realtime_priority,
        st.exclusive_owner_id,
        st.total_cycles,
        st.hw_xruns,
        st.midi_events_routed,
        st.eq_cfg.enabled,
        st.eq_cfg.dynamic_mode,
        finite_d(f64::from(st.eq_cfg.preamp_db)),
        finite_d(f64::from(st.eq_cfg.dyn_threshold_db)),
        finite_d(f64::from(st.eq_cfg.dyn_ratio)),
        st.eq_cfg.limiter_enabled,
        finite_d(f64::from(st.output_peak_l)),
        finite_d(f64::from(st.output_peak_r)),
        st.pa_server_running,
        st.pa_clients
    );
    append_c_json_str(out, &st.eq_cfg.preset_name);
    out.push_str("},");

    let _ = write!(
        out,
        "\"rt\":{{\"cycle_count\":{},\"last_cycle_us\":{:.1},\"avg_cycle_us\":{:.1},\
        \"max_cycle_us\":{:.1},\"min_cycle_us\":{:.1},\"last_jitter_us\":{:.1},\
        \"max_jitter_us\":{:.1},\"eq_us\":{:.1},\"pa_mix_us\":{:.1},\"hw_write_us\":{:.1},\
        \"budget_us\":{:.1},\"cpu_load_pct\":{:.2}}},",
        rt.cycle_count,
        finite_d(rt.last_cycle_ns as f64 / 1000.0),
        finite_d(rt.avg_cycle_ns as f64 / 1000.0),
        finite_d(rt.max_cycle_ns as f64 / 1000.0),
        finite_d(rt.min_cycle_ns as f64 / 1000.0),
        finite_d(rt.last_jitter_ns as f64 / 1000.0),
        finite_d(rt.max_jitter_ns as f64 / 1000.0),
        finite_d(rt.eq_ns as f64 / 1000.0),
        finite_d(rt.pa_mix_ns as f64 / 1000.0),
        finite_d(rt.hw_write_ns as f64 / 1000.0),
        finite_d(rt.budget_ns as f64 / 1000.0),
        cpu_pct
    );

    let _ = write!(
        out,
        "\"bluez\":{{\"active\":{},\"initialized\":{},\"endpoints_registered\":{},\
        \"direct_socket\":{},\"has_transport\":{},\"tx_paused\":{},\
        \"sample_rate\":{},\"channels\":{},\"delay_ms\":{},\
        \"mtu_r\":{},\"mtu_w\":{},\"avrcp_vol\":{},\
        \"tx_ring_fill\":{},\"tx_ring_cap\":{},\
        \"tx_packets\":{},\"tx_underruns\":{},\"tx_errors\":{},\"alias\":",
        st.bt_active,
        snap.bt_initialized,
        snap.bt_endpoints_registered,
        snap.bt_direct_socket,
        snap.bt_has_transport,
        snap.bt_tx_paused,
        snap.bt_sample_rate,
        snap.bt_channels,
        st.bt_delay_ms,
        snap.bt_mtu_r,
        snap.bt_mtu_w,
        snap.bt_avrcp_vol,
        snap.bt_tx_ring_fill,
        snap.bt_tx_ring_cap,
        snap.bt_tx_packets,
        snap.bt_tx_underruns,
        snap.bt_tx_errors
    );
    append_c_json_str(out, &st.bt_alias);
    out.push_str(",\"codec\":");
    append_c_json_str(out, &st.bt_codec);
    out.push_str(",\"device_path\":");
    append_c_json_str(out, &snap.bt_device_path);
    out.push_str(",\"transport_path\":");
    append_c_json_str(out, &snap.bt_transport_path);
    out.push_str(",\"endpoint_path\":");
    append_c_json_str(out, &snap.bt_endpoint_path);
    out.push_str("},");

    let _ = write!(
        out,
        "\"capture\":{{\"active\":{},\"hw_rate\":{},\"hw_channels\":{},\
        \"hw_format\":{},\"ring_fill\":{},\"pref_rate\":{},\
        \"measured_rate\":{},\"kick_count\":{},\"is_usb\":{},\"duplex_locked\":{},\"device_id\":",
        snap.cap_active,
        snap.cap_hw_rate,
        snap.cap_hw_channels,
        snap.cap_hw_format,
        snap.cap_ring_fill,
        snap.cap_pref_rate,
        snap.cap_measured_rate,
        snap.cap_kick_count,
        snap.cap_is_usb,
        snap.cap_duplex_locked
    );
    append_c_json_str(out, &snap.cap_device_id);
    out.push_str(",\"pcm_path\":");
    append_c_json_str(out, &snap.cap_pcm_path);
    out.push_str(",\"alias\":");
    append_c_json_str(out, &snap.cap_alias);
    out.push_str("},");

    let _ = write!(
        out,
        "\"midi\":{{\"initialized\":{},\"rawmidi_open\":{},\"seq_client\":{},\
        \"seq_in_port\":{},\"events_routed\":{},\"controller\":",
        snap.midi_initialized,
        snap.midi_rawmidi_open,
        snap.midi_seq_client,
        snap.midi_seq_in_port,
        snap.midi_events_routed
    );
    append_c_json_str(out, &snap.midi_controller);
    out.push_str(",\"dev_path\":");
    append_c_json_str(out, &snap.midi_dev_path);
    out.push_str("},");

    out.push_str("\"wasapi_sessions\":[");
    let nw = (snap.num_wasapi as usize).min(LWASAPI_MAX_SESSIONS);
    for (i, ws) in snap.wasapi[..nw].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"session_id\":{},\"pid\":{},\"name\":",
            ws.session_id, ws.client_pid
        );
        append_c_json_str(out, &ws.client_name);
        let _ = write!(
            out,
            ",\"share_mode\":{},\"stream_flags\":{},\"is_capture\":{},\
            \"rate\":{},\"channels\":{},\"active\":{},\"muted\":{},\
            \"volume_milli\":{},\"buffer_frames\":{},\"period_frames\":{},\
            \"buffered_frames\":{},\"xruns\":{}}}",
            ws.share_mode,
            ws.stream_flags,
            ws.is_capture,
            ws.sample_rate,
            ws.channels,
            ws.active,
            ws.muted,
            ws.volume_milli,
            ws.buffer_frames,
            ws.period_frames,
            ws.buffered_frames,
            ws.xruns
        );
    }
    out.push_str("],");

    out.push_str("\"pa_clients\":[");
    let nc = (snap.pa.num_clients as usize).min(LWASAPI_DEBUG_MAX_PA_CLIENTS);
    for (i, pc) in snap.pa.clients[..nc].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"index\":{},\"pid\":{},\"name\":", pc.index, pc.pid);
        append_c_json_str(out, &pc.name);
        out.push_str(",\"binary\":");
        append_c_json_str(out, &pc.binary);
        let _ = write!(
            out,
            ",\"version\":{},\"authed\":{},\"sub_mask\":{},\"out_count\":{}}}",
            pc.version, pc.authed, pc.sub_mask, pc.out_count
        );
    }
    out.push_str("],");

    out.push_str("\"pa_streams\":[");
    let ns = (snap.pa.num_streams as usize).min(LWASAPI_DEBUG_MAX_PA_STREAMS);
    for (i, ps) in snap.pa.streams[..ns].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"index\":{},\"client_index\":{},\"pid\":{},\"name\":",
            ps.index, ps.client_index, ps.pid
        );
        append_c_json_str(out, &ps.name);
        out.push_str(",\"device_id\":");
        append_c_json_str(out, &ps.device_id);
        let _ = write!(
            out,
            ",\"is_playback\":{},\"is_monitor\":{},\"peak_detect\":{},\
            \"direct_on_input\":{},\"target_dev_idx\":{},\"rate\":{},\
            \"channels\":{},\"format\":{},\"active\":{},\"corked\":{},\
            \"in_prebuf\":{},\"draining\":{},\"muted\":{},\"volume_milli\":{},\
            \"frame_size\":{},\"buffered_frames\":{},\"ring_cap_frames\":{},\
            \"tlength_frames\":{},\"prebuf_frames\":{},\"minreq_frames\":{},\
            \"fragsize_frames\":{},\"mix_peak_l\":{:.4},\"mix_peak_r\":{:.4},\
            \"write_index\":{},\"read_index\":{}}}",
            ps.is_playback,
            ps.is_monitor,
            ps.peak_detect,
            ps.direct_on_input,
            ps.target_dev_idx,
            ps.rate,
            ps.channels,
            ps.format,
            ps.active,
            ps.corked,
            ps.in_prebuf,
            ps.draining,
            ps.muted,
            ps.volume_milli,
            ps.frame_size,
            ps.buffered_frames,
            ps.ring_cap_frames,
            ps.tlength_frames,
            ps.prebuf_frames,
            ps.minreq_frames,
            ps.fragsize_frames,
            finite_d(f64::from(ps.mix_peak_l)),
            finite_d(f64::from(ps.mix_peak_r)),
            ps.write_index,
            ps.read_index
        );
    }
    out.push(']');

    let pw = &snap.pw;
    let _ = write!(
        out,
        ",\"pw\":{{\"broker_running\":{},\"zero_pw_mode\":{},\
        \"preview_requested\":{},\"active_casting\":{},\
        \"frames_relayed_total\":{},\"frames_dropped\":{},\
        \"dmabuf_streams\":{},\"memfd_streams\":{},\"audio_rejects\":{},\
        \"preview_target_w\":{},\"preview_target_h\":{},\"preview_target_fps\":{},\
        \"cast_limit_w\":{},\"cast_limit_h\":{},\"cast_limit_fps\":{},\"actual_fps_x100\":{},\
        \"socket_path\":",
        pw.broker_running,
        pw.zero_pw_mode,
        pw.preview_requested,
        pw.active_casting,
        pw.frames_relayed_total,
        pw.frames_dropped,
        pw.dmabuf_streams,
        pw.memfd_streams,
        pw.audio_rejects,
        pw.preview_target_w,
        pw.preview_target_h,
        pw.preview_target_fps,
        pw.cast_limit_w,
        pw.cast_limit_h,
        pw.cast_limit_fps,
        pw.actual_fps_x100
    );
    append_c_json_str(out, &pw.socket_path);
    out.push_str(",\"manager_socket_path\":");
    append_c_json_str(out, &pw.manager_socket_path);
    out.push_str(",\"preview_shm_name\":");
    append_c_json_str(out, &pw.preview_shm_name);

    out.push_str(",\"clients\":[");
    let pwc = (pw.num_clients as usize).min(LWASAPI_DEBUG_MAX_PW_CLIENTS);
    for (i, pc) in pw.clients[..pwc].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"id\":{},\"pid\":{},\"is_portal\":{},\"num_nodes\":{},\"app_name\":",
            pc.id, pc.pid, pc.is_portal, pc.num_nodes
        );
        append_c_json_str(out, &pc.app_name);
        out.push_str(",\"binary\":");
        append_c_json_str(out, &pc.binary);
        out.push_str(",\"portal_app_id\":");
        append_c_json_str(out, &pc.portal_app_id);
        out.push('}');
    }

    out.push_str("],\"nodes\":[");
    let pwn = (pw.num_nodes as usize).min(LWASAPI_DEBUG_MAX_PW_NODES);
    for (i, pn) in pw.nodes[..pwn].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"id\":{},\"client_id\":{},\"client_pid\":{},\
            \"target_node_id\":{},\"linked_node_id\":{},\
            \"direction\":{},\"state\":{},\"is_driver\":{},\"alloc_buffers\":{},\
            \"width\":{},\"height\":{},\"fps_num\":{},\"fps_denom\":{},\
            \"n_buffers\":{},\"drm_modifier\":\"0x{:x}\",\"frames\":{},\"name\":",
            pn.id,
            pn.client_id,
            pn.client_pid,
            pn.target_node_id,
            pn.linked_node_id,
            pn.direction,
            pn.state,
            pn.is_driver,
            pn.alloc_buffers,
            pn.width,
            pn.height,
            pn.fps_num,
            pn.fps_denom,
            pn.n_buffers,
            pn.drm_modifier,
            pn.frames
        );
        append_c_json_str(out, &pn.name);
        out.push_str(",\"media_class\":");
        append_c_json_str(out, &pn.media_class);
        out.push_str(",\"format_name\":");
        append_c_json_str(out, &pn.format_name);
        out.push_str(",\"mem_type\":");
        append_c_json_str(out, &pn.mem_type);
        out.push('}');
    }

    out.push_str("],\"links\":[");
    let pwl = (pw.num_links as usize).min(LWASAPI_DEBUG_MAX_PW_LINKS);
    for (i, pl) in pw.links[..pwl].iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"id\":{},\"out_node_id\":{},\"in_node_id\":{},\
            \"state\":{},\"is_dmabuf\":{},\"frames_relayed\":{},\"format_summary\":",
            pl.id, pl.out_node_id, pl.in_node_id, pl.state, pl.is_dmabuf, pl.frames_relayed
        );
        append_c_json_str(out, &pl.format_summary);
        out.push('}');
    }

    let fs = failsafe::get_stats();
    let _ = write!(
        out,
        "]}},\"wire\":{{\"backend\":\"{}\",\"is_rust\":{},\"armed\":{},\
        \"trips_total\":{},\"pa_quarantined\":{},\"pw_quarantined\":{},\
        \"bt_fallbacks\":{},\"fd_leaks_blocked\":{},\"manual_resets\":{},\"last_reason\":",
        if fs.is_rust { "rust" } else { "c_fallback" },
        if fs.is_rust { 1 } else { 0 },
        if fs.armed { 1 } else { 0 },
        fs.trips_total,
        fs.pa_quarantined,
        fs.pw_quarantined,
        fs.bt_fallbacks,
        fs.fd_leaks_blocked,
        fs.manual_resets
    );
    append_c_json_str(out, &fs.last_reason);
    out.push_str("}}");
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
    cbs: LwasapiDbgCallbacks,
    sock_path: String,
    listener: Option<UnixListener>,
    dgram: Option<UnixDatagram>,
    clients: Vec<DbgClient>,
    imp_hist: VecDeque<HistEntry>,
    trace_hist: VecDeque<HistEntry>,
    ring_tail: u64,
    json_scratch: String,
    snap_box: Box<LwasapiDbgSnapshot>,
}

impl ServerLoopState {
    fn new(
        cbs: LwasapiDbgCallbacks,
        sock_path: String,
        listener: Option<UnixListener>,
        dgram: Option<UnixDatagram>,
    ) -> Self {
        let zero_snap = unsafe { mem::zeroed::<LwasapiDbgSnapshot>() };
        Self {
            cbs,
            sock_path,
            listener,
            dgram,
            clients: Vec::with_capacity(DBG_MAX_CLIENTS),
            imp_hist: VecDeque::with_capacity(DBG_HISTORY_CAP),
            trace_hist: VecDeque::with_capacity(DBG_HISTORY_CAP),
            ring_tail: 0,
            json_scratch: String::with_capacity(16384),
            snap_box: Box::new(zero_snap),
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

    fn build_snapshot_line(&mut self) -> Option<String> {
        let fill = self.cbs.fill_snapshot?;
        unsafe {
            core::ptr::write_bytes(self.snap_box.as_mut(), 0, 1);
            self.snap_box.rt = get_rt_stats();
            fill(self.cbs.user_ctx, self.snap_box.as_mut());
        }
        let mut s = String::with_capacity(16384);
        serialize_snapshot_json(&self.snap_box, &mut s);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
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
            let mut s = String::with_capacity(128);
            let _ = write!(
                s,
                "{{\"type\":\"pong\",\"ts_ms\":{},\"ts_mono_ns\":{},\"tag\":",
                realtime_ms(),
                mono_ns()
            );
            escape_json_str(tag, &mut s);
            s.push('}');
            if let Some(c) = self.clients.get_mut(idx) {
                c.queue_line(&s);
            }
        } else if let Some(handler) = self.cbs.handle_cmd {
            let mut cmd_z = Vec::with_capacity(cmd.len() + 1);
            cmd_z.extend_from_slice(cmd);
            cmd_z.push(0);
            let mut reply = [0u8; 512];
            let rc = unsafe {
                handler(
                    self.cbs.user_ctx,
                    cmd_z.as_ptr().cast::<c_char>(),
                    reply.as_mut_ptr().cast::<c_char>(),
                    reply.len(),
                )
            };
            let mut s = String::with_capacity(256);
            let _ = write!(
                s,
                "{{\"type\":\"cmd_reply\",\"ok\":{},\"rc\":{},\"cmd\":",
                if rc == 0 { "true" } else { "false" },
                rc
            );
            escape_json_str(cmd, &mut s);
            s.push_str(",\"msg\":");
            append_c_json_str(&mut s, &reply);
            s.push('}');
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
        let mut hello = String::with_capacity(256);
        let _ = write!(
            hello,
            "{{\"type\":\"hello\",\"version\":1,\"pid\":{},\"ts_ms\":{},\"sock\":",
            current_pid(),
            realtime_ms()
        );
        escape_json_str(self.sock_path.as_bytes(), &mut hello);
        let _ = write!(
            hello,
            ",\"dropped_events\":{},\"history_count\":{}}}",
            G_DROPPED_EVENTS.load(Ordering::Relaxed),
            self.imp_hist.len() + self.trace_hist.len()
        );
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

                    serialize_event_json(&ev, &mut self.json_scratch);
                    if !self.json_scratch.is_empty() {
                        let line = self.json_scratch.clone();
                        self.history_push(ev.id, ev.lvl, line.clone());
                        self.broadcast_line(&line);
                    }
                }
                RecvDgramCred::WouldBlock | RecvDgramCred::Err => break,
            }
        }
    }

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

            serialize_event_json(&ev, &mut self.json_scratch);
            if !self.json_scratch.is_empty() {
                let line = self.json_scratch.clone();
                self.history_push(ev.id, ev.lvl, line.clone());
                self.broadcast_line(&line);
            }
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

pub fn debug_start(cbs: LwasapiDbgCallbacks) -> i32 {
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

    let sock_path = resolve_sock_path_common("LWASAPI_DEBUG_SOCK", "debug.sock", LWASAPI_DEBUG_SOCK_DEFAULT);
    let in_sock_path =
        resolve_sock_path_common("LWASAPI_DEBUG_IN_SOCK", "debug-in.sock", LWASAPI_DEBUG_IN_SOCK_DEFAULT);
    if sock_path != LWASAPI_DEBUG_SOCK_DEFAULT {
        let _ = fs::remove_file(LWASAPI_DEBUG_SOCK_DEFAULT);
    }
    if in_sock_path != LWASAPI_DEBUG_IN_SOCK_DEFAULT {
        let _ = fs::remove_file(LWASAPI_DEBUG_IN_SOCK_DEFAULT);
    }

    let listener = bind_unix_listener_0600(&sock_path);
    let dgram = bind_unix_dgram_passcred_0600(&in_sock_path);

    rt.cbs = cbs;
    rt.sock_path = sock_path.clone();
    rt.in_sock_path = in_sock_path.clone();

    G_DBG_RUNNING.store(true, Ordering::Release);
    let mut loop_state = ServerLoopState::new(cbs, sock_path.clone(), listener, dgram);
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

#[no_mangle]
pub unsafe extern "C" fn salsa_dbg_emit_raw(
    sub: u8,
    lvl: u8,
    pid: u32,
    sid: u32,
    v1: i32,
    v2: i32,
    hr: i32,
    op: *const c_char,
    target: *const c_char,
    detail: *const c_char,
) {
    let op_b = if op.is_null() {
        b"".as_slice()
    } else {
        unsafe { core::ffi::CStr::from_ptr(op) }.to_bytes()
    };
    let tgt_b = if target.is_null() {
        b"".as_slice()
    } else {
        unsafe { core::ffi::CStr::from_ptr(target) }.to_bytes()
    };
    let det_b = if detail.is_null() {
        b"".as_slice()
    } else {
        unsafe { core::ffi::CStr::from_ptr(detail) }.to_bytes()
    };
    emit_raw_bytes(sub, lvl, pid, sid, v1, v2, hr, op_b, tgt_b, det_b);
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dbg_parse_dgram(
    buf: *const u8,
    len: usize,
    out: *mut LwasapiDbgDgram,
) -> bool {
    if buf.is_null() || out.is_null() {
        return false;
    }
    let raw = unsafe { core::slice::from_raw_parts(buf, len) };
    let Some(parsed) = parse_dgram_bytes(raw) else {
        return false;
    };
    unsafe {
        *out = parsed;
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dbg_escape_json_str(
    src: *const c_char,
    out: *mut c_char,
    out_cap: usize,
) -> usize {
    if out.is_null() || out_cap == 0 {
        return 0;
    }
    let bytes = if src.is_null() {
        b"".as_slice()
    } else {
        unsafe { core::ffi::CStr::from_ptr(src) }.to_bytes()
    };
    let mut s = String::with_capacity(bytes.len() + 8);
    escape_json_str(bytes, &mut s);
    let dst = unsafe { core::slice::from_raw_parts_mut(out.cast::<u8>(), out_cap) };
    copy_str_to_c(dst, &s);
    c_str_to_string(dst).len()
}

#[no_mangle]
pub extern "C" fn lwasapi_debug_start(cbs: LwasapiDbgCallbacks) -> c_int {
    debug_start(cbs)
}

#[no_mangle]
pub extern "C" fn lwasapi_debug_stop() {
    debug_stop();
}

#[no_mangle]
pub extern "C" fn lwasapi_debug_record_rt_period(
    cycle_ns: u64,
    wake_jitter_ns: i64,
    budget_ns: u64,
    eq_ns: u64,
    pa_mix_ns: u64,
    hw_write_ns: u64,
) {
    record_rt_period(cycle_ns, wake_jitter_ns, budget_ns, eq_ns, pa_mix_ns, hw_write_ns);
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_debug_get_rt_stats(out_rt: *mut LwasapiDbgRtStats) {
    if out_rt.is_null() {
        return;
    }
    unsafe {
        *out_rt = get_rt_stats();
    }
}

#[no_mangle]
pub extern "C" fn lwasapi_debug_reset_rt_stats() {
    reset_rt_stats();
}

// Unused import suppression cleanly checked
const _: Option<OwnedFd> = None;
const _: Option<AtomicU32> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::c_void;
    use std::io::{BufRead, BufReader, Write};

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    unsafe extern "C" fn test_fill_snapshot(_ctx: *mut c_void, out: *mut LwasapiDbgSnapshot) {
        let s = unsafe { &mut *out };
        copy_str_to_c(&mut s.engine_st.hw_device, "Rust Test Sink");
        s.engine_st.hw_sample_rate = 192_000;
        s.engine_st.hw_channels = 2;
        s.pw.broker_running = 1;
        s.num_wasapi = 0;
    }

    unsafe extern "C" fn test_handle_cmd(
        _ctx: *mut c_void,
        cmd: *const c_char,
        reply: *mut c_char,
        reply_len: usize,
    ) -> c_int {
        let c = unsafe { core::ffi::CStr::from_ptr(cmd) }.to_string_lossy();
        let out = unsafe { core::slice::from_raw_parts_mut(reply.cast::<u8>(), reply_len) };
        if c == "PW_SYNC_MODE" {
            copy_str_to_c(out, "sync_mode rc=0");
            0
        } else {
            copy_str_to_c(out, "unknown");
            -1
        }
    }

    #[test]
    fn dgram_and_json_escape_validation() {
        assert!(parse_dgram_bytes(&[0u8; 20]).is_none());
        let mut raw = [0u8; mem::size_of::<LwasapiDbgDgram>()];
        raw[0..4].copy_from_slice(&LWASAPI_DEBUG_DGRAM_MAGIC.to_le_bytes());
        raw[24] = 2;
        raw[25] = 1;
        raw[28..32].copy_from_slice(b"PING");
        let parsed = parse_dgram_bytes(&raw).unwrap();
        assert_eq!(parsed.magic, LWASAPI_DEBUG_DGRAM_MAGIC);
        assert_eq!(parsed.sub, 2);

        let mut escaped = String::new();
        escape_json_str(b"hi\"\\\n\x07ok", &mut escaped);
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

        let tmp = format!("/tmp/lwasapi-dbg-rust-test-{}", std::process::id());
        let _ = fs::create_dir_all(&tmp);
        let sock = format!("{tmp}/dbg.sock");
        let in_sock = format!("{tmp}/dbg-in.sock");
        std::env::set_var("LWASAPI_DEBUG_SOCK", &sock);
        std::env::set_var("LWASAPI_DEBUG_IN_SOCK", &in_sock);

        let cbs = LwasapiDbgCallbacks {
            user_ctx: core::ptr::null_mut(),
            fill_snapshot: Some(test_fill_snapshot),
            handle_cmd: Some(test_handle_cmd),
        };
        assert_eq!(debug_start(cbs), 0);

        emit_str(2, 1, 42, 7, 1, 2, 0, "PA_OPEN", "Discord", "test stream");

        let mut client = UnixStream::connect(&sock).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();

        // Expect hello
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("\"type\":\"hello\""));

        // Read until telemetry snapshot appears (skipping replay DEBUG_START/PA_OPEN events)
        let mut saw_snap = false;
        for _ in 0..8 {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line.contains("\"type\":\"telemetry\"") {
                assert!(line.contains("\"hw_device\":\"Rust Test Sink\""));
                assert!(line.contains("\"backend\":\"rust\""));
                saw_snap = true;
                break;
            }
        }
        assert!(saw_snap);

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
            if line.contains("\"type\":\"pong\"") && line.contains("\"tag\":\"hello\"") {
                saw_pong = true;
            } else if line.contains("\"type\":\"cmd_reply\"") && line.contains("sync_mode rc=0") {
                saw_reply = true;
            } else if line.contains("\"cycle_count\":0") {
                saw_reset = true;
            }
            if saw_pong && saw_reply && saw_reset {
                break;
            }
        }
        assert!(saw_pong && saw_reply && saw_reset);

        debug_stop();
        std::env::remove_var("LWASAPI_DEBUG_SOCK");
        std::env::remove_var("LWASAPI_DEBUG_IN_SOCK");
        let _ = fs::remove_dir_all(&tmp);
    }
}
