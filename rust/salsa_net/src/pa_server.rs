// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]

use core::mem;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, LasapiDeviceList, LasapiEngineStatus, LASAPI_DBG_INFO,
    LASAPI_DBG_PA, LASAPI_DBG_TRACE, LASAPI_DBG_WARN, LASAPI_MAX_DEVICES, LASAPI_MAX_SESSIONS,
};
use crate::json::JsonValue;

pub trait PaEngineHost: Send + Sync {
    fn fill_status(&self, out: &mut LasapiEngineStatus);
    fn enumerate_devices(&self, out: &mut LasapiDeviceList);
    fn switch_devices(&self, pb_id: Option<&str>, cap_id: Option<&str>) -> i32;
    fn set_master_volume(&self, is_capture: bool, vol_pct: i32, mute_flag: i32) -> i32;
    fn set_mic_gain_ppm(&self, gain_ppm: u32, vol_pct: i32, mute_flag: i32) -> i32;
}

#[cfg(test)]
#[derive(Default)]
struct NullPaEngineHost;

#[cfg(test)]
impl PaEngineHost for NullPaEngineHost {
    fn fill_status(&self, _out: &mut LasapiEngineStatus) {}
    fn enumerate_devices(&self, _out: &mut LasapiDeviceList) {}
    fn switch_devices(&self, _pb_id: Option<&str>, _cap_id: Option<&str>) -> i32 {
        0
    }
    fn set_master_volume(&self, _is_capture: bool, _vol_pct: i32, _mute_flag: i32) -> i32 {
        0
    }
    fn set_mic_gain_ppm(&self, _gain_ppm: u32, _vol_pct: i32, _mute_flag: i32) -> i32 {
        0
    }
}
use crate::failsafe;
use crate::pa_wire::{
    self, cvol_to_milli, decode_sample, encode_sample, format_sample_bits, format_sample_size,
    frame_packet, parse_frame_hdr, pct_to_pa_vol, std_channel_map, PaCVolume, PaChannelMap,
    PaFormatInfo, PaPropBag, PaSampleSpec, TagBuilder, TagReader, FMT_FLOAT32LE, PA_ENCODING_ANY,
    PA_ENCODING_PCM, PA_SAMPLE_MAX, PA_TAG_S64, PA_TAG_U32, PA_TAG_U64, PA_TAG_U8, PA_TAG_USEC,
    PA_TAG_VOLUME, PA_VOLUME_NORM,
};
use crate::unix_sys::{
    self, bind_unix_listener_0600, current_hostname, current_pid, current_uid, current_username,
    dbg_emit_vals, do_poll, enable_realtime, ensure_owned_dir_0700, eventfd_drain, eventfd_signal,
    is_safe_owner_dir, mono_ns, nb_recv, nb_send, new_eventfd, realtime_sec_usec, socket_peercred,
    user_home_dir, PollFd, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT,
};

const PA_PROTO_VERSION: u32 = 35;
const PA_INVALID_INDEX: u32 = 0xFFFF_FFFF;
const PA_PSTREAM_CTRL_CHANNEL: u32 = 0xFFFF_FFFF;
const PA_NATIVE_COOKIE_LEN: usize = 256;
const PA_SERVER_COOKIE_ID: u32 = 0x5341_4C53;

const PA_MAX_CLIENTS: usize = 32;
const PA_MAX_TCP_CLIENTS: usize = 12;
const PA_MAX_UNAUTH_CLIENTS: usize = 6;
const PA_UNAUTH_TIMEOUT_MS: u64 = 2_000;
const PA_ACCEPT_BATCH_CAP: usize = 16;
const PA_MAX_STREAMS: usize = 48;
const PA_MAX_OUT_QUEUE: usize = 2048;
const PA_STREAM_RING_FRAMES: usize = 262_144;
const PA_STREAM_RING_MASK: usize = PA_STREAM_RING_FRAMES - 1;
const PA_DEFAULT_MAXLENGTH: u32 = 8 * 1024 * 1024;
const PA_PSTREAM_DESC_BYTES: usize = 20;
const PA_MAX_DOI_PERIOD: usize = 2048;
const PA_MAX_STABLE_IDS: usize = 128;
const PA_MAX_APP_VOLUMES: usize = 256;

const PA_FAC_SINK: u32 = 0;
const PA_FAC_SOURCE: u32 = 1;
const PA_FAC_CARD: u32 = 2;

const SUB_EVENT_SINK: u32 = 0x0000;
const SUB_EVENT_SOURCE: u32 = 0x0001;
const SUB_EVENT_SINK_INPUT: u32 = 0x0002;
const SUB_EVENT_SOURCE_OUTPUT: u32 = 0x0003;
const SUB_EVENT_CLIENT: u32 = 0x0005;
const SUB_EVENT_SERVER: u32 = 0x0007;
const SUB_EVENT_CARD: u32 = 0x0009;
const SUB_EVENT_NEW: u32 = 0x0000;
const SUB_EVENT_CHANGE: u32 = 0x0010;
const SUB_EVENT_REMOVE: u32 = 0x0020;
const SUB_MASK_ALL: u32 = 0x02FF;

const CMD_ERROR: u32 = 0;
const CMD_CREATE_PLAYBACK_STREAM: u32 = 3;
const CMD_DELETE_PLAYBACK_STREAM: u32 = 4;
const CMD_CREATE_RECORD_STREAM: u32 = 5;
const CMD_DELETE_RECORD_STREAM: u32 = 6;
const CMD_EXIT: u32 = 7;
const CMD_AUTH: u32 = 8;
const CMD_SET_CLIENT_NAME: u32 = 9;
const CMD_LOOKUP_SINK: u32 = 10;
const CMD_LOOKUP_SOURCE: u32 = 11;
const CMD_DRAIN_PLAYBACK_STREAM: u32 = 12;
const CMD_STAT: u32 = 13;
const CMD_GET_PLAYBACK_LATENCY: u32 = 14;
const CMD_GET_SERVER_INFO: u32 = 20;
const CMD_GET_SINK_INFO: u32 = 21;
const CMD_GET_SINK_INFO_LIST: u32 = 22;
const CMD_GET_SOURCE_INFO: u32 = 23;
const CMD_GET_SOURCE_INFO_LIST: u32 = 24;
const CMD_GET_MODULE_INFO: u32 = 25;
const CMD_GET_MODULE_INFO_LIST: u32 = 26;
const CMD_GET_CLIENT_INFO: u32 = 27;
const CMD_GET_CLIENT_INFO_LIST: u32 = 28;
const CMD_GET_SINK_INPUT_INFO: u32 = 29;
const CMD_GET_SINK_INPUT_INFO_LIST: u32 = 30;
const CMD_GET_SOURCE_OUTPUT_INFO: u32 = 31;
const CMD_GET_SOURCE_OUTPUT_INFO_LIST: u32 = 32;
const CMD_GET_SAMPLE_INFO_LIST: u32 = 34;
const CMD_SUBSCRIBE: u32 = 35;
const CMD_SET_SINK_VOLUME: u32 = 36;
const CMD_SET_SINK_INPUT_VOLUME: u32 = 37;
const CMD_SET_SOURCE_VOLUME: u32 = 38;
const CMD_SET_SINK_MUTE: u32 = 39;
const CMD_SET_SOURCE_MUTE: u32 = 40;
const CMD_CORK_PLAYBACK_STREAM: u32 = 41;
const CMD_FLUSH_PLAYBACK_STREAM: u32 = 42;
const CMD_TRIGGER_PLAYBACK_STREAM: u32 = 43;
const CMD_SET_DEFAULT_SINK: u32 = 44;
const CMD_SET_DEFAULT_SOURCE: u32 = 45;
const CMD_SET_PLAYBACK_STREAM_NAME: u32 = 46;
const CMD_SET_RECORD_STREAM_NAME: u32 = 47;
const CMD_KILL_SINK_INPUT: u32 = 49;
const CMD_KILL_SOURCE_OUTPUT: u32 = 50;
const CMD_GET_RECORD_LATENCY: u32 = 57;
const CMD_CORK_RECORD_STREAM: u32 = 58;
const CMD_FLUSH_RECORD_STREAM: u32 = 59;
const CMD_PREBUF_PLAYBACK_STREAM: u32 = 60;
const CMD_REQUEST: u32 = 61;
const CMD_SUBSCRIBE_EVENT: u32 = 66;
const CMD_MOVE_SINK_INPUT: u32 = 67;
const CMD_MOVE_SOURCE_OUTPUT: u32 = 68;
const CMD_SET_SINK_INPUT_MUTE: u32 = 69;
const CMD_SUSPEND_SINK: u32 = 70;
const CMD_SUSPEND_SOURCE: u32 = 71;
const CMD_SET_PLAYBACK_STREAM_BUFFER_ATTR: u32 = 72;
const CMD_SET_RECORD_STREAM_BUFFER_ATTR: u32 = 73;
const CMD_UPDATE_PLAYBACK_STREAM_SAMPLE_RATE: u32 = 74;
const CMD_UPDATE_RECORD_STREAM_SAMPLE_RATE: u32 = 75;
const CMD_UPDATE_RECORD_STREAM_PROPLIST: u32 = 80;
const CMD_UPDATE_PLAYBACK_STREAM_PROPLIST: u32 = 81;
const CMD_UPDATE_CLIENT_PROPLIST: u32 = 82;
const CMD_REMOVE_RECORD_STREAM_PROPLIST: u32 = 83;
const CMD_REMOVE_PLAYBACK_STREAM_PROPLIST: u32 = 84;
const CMD_REMOVE_CLIENT_PROPLIST: u32 = 85;
const CMD_STARTED: u32 = 86;
const CMD_EXTENSION: u32 = 87;
const CMD_GET_CARD_INFO: u32 = 88;
const CMD_GET_CARD_INFO_LIST: u32 = 89;
const CMD_SET_CARD_PROFILE: u32 = 90;
const CMD_SET_SINK_PORT: u32 = 96;
const CMD_SET_SOURCE_PORT: u32 = 97;
const CMD_SET_SOURCE_OUTPUT_VOLUME: u32 = 98;
const CMD_SET_SOURCE_OUTPUT_MUTE: u32 = 99;
const CMD_SET_PORT_LATENCY_OFFSET: u32 = 100;

const ERR_ACCESS: u32 = 1;
const ERR_COMMAND: u32 = 2;
const ERR_INVALID: u32 = 3;
const ERR_NOENTITY: u32 = 5;
const ERR_PROTOCOL: u32 = 7;
const ERR_NOTSUPPORTED: u32 = 19;
const ERR_NOEXTENSION: u32 = 21;

struct RtCursor {
    resamp_phase: f64,
    hist_l: [f32; 2],
    hist_r: [f32; 2],
    have_hist: bool,
    cap_hist_l: f32,
    cap_hist_r: f32,
    have_cap_hist: bool,
    peak_accum_l: f32,
    peak_accum_r: f32,
    last_mix_l: f32,
    last_mix_r: f32,
    cap_smooth_gain: f32,
    seen_flush_seq: u64,
}

impl Default for RtCursor {
    fn default() -> Self {
        Self {
            resamp_phase: 0.0,
            hist_l: [0.0; 2],
            hist_r: [0.0; 2],
            have_hist: false,
            cap_hist_l: 0.0,
            cap_hist_r: 0.0,
            have_cap_hist: false,
            peak_accum_l: 0.0,
            peak_accum_r: 0.0,
            last_mix_l: 0.0,
            last_mix_r: 0.0,
            cap_smooth_gain: 1.0,
            seen_flush_seq: 0,
        }
    }
}

impl RtCursor {
    fn reset_state(&mut self) {
        self.resamp_phase = 0.0;
        self.hist_l = [0.0; 2];
        self.hist_r = [0.0; 2];
        self.have_hist = false;
        self.cap_hist_l = 0.0;
        self.cap_hist_r = 0.0;
        self.have_cap_hist = false;
        self.peak_accum_l = 0.0;
        self.peak_accum_r = 0.0;
        self.last_mix_l = 0.0;
        self.last_mix_r = 0.0;
        self.cap_smooth_gain = 1.0;
    }
}

struct PaRtStream {
    active: AtomicU32,
    in_use: AtomicBool,
    index: AtomicU32,
    is_playback: AtomicBool,
    is_monitor: AtomicBool,
    peak_detect: AtomicBool,
    direct_on_input: AtomicU32,
    fmt: AtomicU8,
    frame_size: AtomicU32,
    tlength: AtomicU32,
    prebuf: AtomicU32,
    fragsize: AtomicU32,
    corked: AtomicU32,
    in_prebuf: AtomicU32,
    underflow_resync: AtomicBool,
    draining: AtomicU32,
    muted: AtomicU32,
    volume_milli: AtomicU32,
    sample_rate_atomic: AtomicU32,
    wr_frame: AtomicU64,
    rd_frame: AtomicU64,
    write_index: AtomicU64,
    read_index: AtomicU64,
    underrun_baseline_bytes: AtomicU64,
    flush_seq: AtomicU64,
    mix_peak_l: AtomicU32,
    mix_peak_r: AtomicU32,
    period_frames: AtomicU32,
    ring_l: Box<[AtomicU32]>,
    ring_r: Box<[AtomicU32]>,
    period_l: Vec<AtomicU32>,
    period_r: Vec<AtomicU32>,
    cursor: Mutex<RtCursor>,
}

const _: () = {
    assert!(core::mem::size_of::<AtomicU32>() == core::mem::size_of::<u32>());
    assert!(core::mem::align_of::<AtomicU32>() == core::mem::align_of::<u32>());
};

fn alloc_zeroed_atomic_ring(len: usize) -> Box<[AtomicU32]> {
    let raw = Box::into_raw(vec![0u32; len].into_boxed_slice()) as *mut [AtomicU32];
    unsafe { Box::from_raw(raw) }
}

impl PaRtStream {
    fn new() -> Self {
        let ring_l = alloc_zeroed_atomic_ring(PA_STREAM_RING_FRAMES);
        let ring_r = alloc_zeroed_atomic_ring(PA_STREAM_RING_FRAMES);
        let mut period_l = Vec::with_capacity(PA_MAX_DOI_PERIOD);
        let mut period_r = Vec::with_capacity(PA_MAX_DOI_PERIOD);
        for _ in 0..PA_MAX_DOI_PERIOD {
            period_l.push(AtomicU32::new(0));
            period_r.push(AtomicU32::new(0));
        }
        Self {
            active: AtomicU32::new(0),
            in_use: AtomicBool::new(false),
            index: AtomicU32::new(0),
            is_playback: AtomicBool::new(false),
            is_monitor: AtomicBool::new(false),
            peak_detect: AtomicBool::new(false),
            direct_on_input: AtomicU32::new(PA_INVALID_INDEX),
            fmt: AtomicU8::new(0),
            frame_size: AtomicU32::new(0),
            tlength: AtomicU32::new(0),
            prebuf: AtomicU32::new(0),
            fragsize: AtomicU32::new(0),
            corked: AtomicU32::new(0),
            in_prebuf: AtomicU32::new(0),
            underflow_resync: AtomicBool::new(false),
            draining: AtomicU32::new(0),
            muted: AtomicU32::new(0),
            volume_milli: AtomicU32::new(1000),
            sample_rate_atomic: AtomicU32::new(48000),
            wr_frame: AtomicU64::new(0),
            rd_frame: AtomicU64::new(0),
            write_index: AtomicU64::new(0),
            read_index: AtomicU64::new(0),
            underrun_baseline_bytes: AtomicU64::new(0),
            flush_seq: AtomicU64::new(0),
            mix_peak_l: AtomicU32::new(0),
            mix_peak_r: AtomicU32::new(0),
            period_frames: AtomicU32::new(0),
            ring_l,
            ring_r,
            period_l,
            period_r,
            cursor: Mutex::new(RtCursor::default()),
        }
    }

    #[inline]
    fn ring_get(&self, idx: usize) -> (f32, f32) {
        let i = idx & PA_STREAM_RING_MASK;
        (
            f32::from_bits(self.ring_l[i].load(Ordering::Relaxed)),
            f32::from_bits(self.ring_r[i].load(Ordering::Relaxed)),
        )
    }

    #[inline]
    fn ring_set(&self, idx: usize, l: f32, r: f32) {
        let i = idx & PA_STREAM_RING_MASK;
        self.ring_l[i].store(l.to_bits(), Ordering::Relaxed);
        self.ring_r[i].store(r.to_bits(), Ordering::Relaxed);
    }
}

struct PaLasapiRtSlot {
    active: AtomicBool,
    session_id: AtomicU32,
    pa_stream_idx: AtomicU32,
    volume_milli: AtomicU32,
    muted: AtomicBool,
    doi_watched: AtomicBool,
    pa_ctl_seq: AtomicU64,
    pa_target_vol_milli: AtomicU32,
    pa_target_muted: AtomicBool,
    mix_peak_l: AtomicU32,
    mix_peak_r: AtomicU32,
    period_frames: AtomicU32,
    period_l: Vec<AtomicU32>,
    period_r: Vec<AtomicU32>,
}

impl PaLasapiRtSlot {
    fn new() -> Self {
        let mut period_l = Vec::with_capacity(PA_MAX_DOI_PERIOD);
        let mut period_r = Vec::with_capacity(PA_MAX_DOI_PERIOD);
        for _ in 0..PA_MAX_DOI_PERIOD {
            period_l.push(AtomicU32::new(0));
            period_r.push(AtomicU32::new(0));
        }
        Self {
            active: AtomicBool::new(false),
            session_id: AtomicU32::new(0),
            pa_stream_idx: AtomicU32::new(PA_INVALID_INDEX),
            volume_milli: AtomicU32::new(1000),
            muted: AtomicBool::new(false),
            doi_watched: AtomicBool::new(false),
            pa_ctl_seq: AtomicU64::new(0),
            pa_target_vol_milli: AtomicU32::new(1000),
            pa_target_muted: AtomicBool::new(false),
            mix_peak_l: AtomicU32::new(0),
            mix_peak_r: AtomicU32::new(0),
            period_frames: AtomicU32::new(0),
            period_l,
            period_r,
        }
    }
}

struct PaRtShared {
    streams: Vec<PaRtStream>,
    lasapi_slots: Vec<PaLasapiRtSlot>,
    has_active_doi: AtomicBool,
    lasapi_doi_mask: AtomicU32,
    mon_vol_milli: AtomicU32,
    mon_muted: AtomicI32,
    dom_stream_rate: AtomicU32,
    dom_stream_bits: AtomicU32,
    wake_fd: AtomicI32,
}

impl PaRtShared {
    fn new(wake_fd: RawFd) -> Self {
        let mut streams = Vec::with_capacity(PA_MAX_STREAMS);
        for _ in 0..PA_MAX_STREAMS {
            streams.push(PaRtStream::new());
        }
        let mut lasapi_slots = Vec::with_capacity(LASAPI_MAX_SESSIONS);
        for _ in 0..LASAPI_MAX_SESSIONS {
            lasapi_slots.push(PaLasapiRtSlot::new());
        }
        Self {
            streams,
            lasapi_slots,
            has_active_doi: AtomicBool::new(false),
            lasapi_doi_mask: AtomicU32::new(0),
            mon_vol_milli: AtomicU32::new(1000),
            mon_muted: AtomicI32::new(0),
            dom_stream_rate: AtomicU32::new(0),
            dom_stream_bits: AtomicU32::new(0),
            wake_fd: AtomicI32::new(wake_fd),
        }
    }

    #[inline]
    fn sync_active_doi(&self) {
        let mut any_doi = false;
        for s in &self.streams {
            if s.in_use.load(Ordering::Relaxed)
                && !s.is_playback.load(Ordering::Relaxed)
                && s.direct_on_input.load(Ordering::Relaxed) != PA_INVALID_INDEX
            {
                any_doi = true;
                break;
            }
        }
        self.has_active_doi.store(any_doi, Ordering::Release);

        let mut wmask = 0u32;
        for (slot_idx, ws) in self.lasapi_slots.iter().enumerate() {
            let w_idx = ws.pa_stream_idx.load(Ordering::Acquire);
            let watched = any_doi
                && ws.active.load(Ordering::Acquire)
                && w_idx != PA_INVALID_INDEX
                && self.streams.iter().any(|s| {
                    s.in_use.load(Ordering::Relaxed)
                        && !s.is_playback.load(Ordering::Relaxed)
                        && s.direct_on_input.load(Ordering::Relaxed) == w_idx
                });
            ws.doi_watched.store(watched, Ordering::Release);
            if !watched {
                ws.period_frames.store(0, Ordering::Relaxed);
            } else if slot_idx < 32 {
                wmask |= 1u32 << slot_idx;
            }
        }
        self.lasapi_doi_mask.store(wmask, Ordering::Release);
    }

    #[inline]
    fn signal_wake(&self) {
        let fd = self.wake_fd.load(Ordering::Relaxed);
        if fd >= 0 {
            eventfd_signal(fd);
        }
    }
}

struct OutPacket {
    data: Vec<u8>,
    sent: usize,
}

#[inline]
fn mono_ms() -> u64 {
    mono_ns() / 1_000_000
}

#[inline]
fn ct_eq_256(a: &[u8], b: &[u8; PA_NATIVE_COOKIE_LEN]) -> bool {
    if a.len() != PA_NATIVE_COOKIE_LEN {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..PA_NATIVE_COOKIE_LEN {
        diff |= a[i] ^ b[i];
    }
    core::hint::black_box(diff) == 0
}

fn rand_tmp_suffix() -> String {
    let mut b = [0u8; 8];
    let ok = fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    let v = if ok {
        u64::from_le_bytes(b)
    } else {
        mono_ns() ^ ((current_pid() as u64) << 32)
    };
    format!("{}.{v:016x}.tmp", current_pid())
}

struct PaClient {
    in_use: bool,
    _slot: usize,
    index: u32,
    fd: Option<OwnedFd>,
    is_tcp: bool,
    accepted_ms: u64,
    auth_failures: u8,
    version: u32,
    authed: bool,
    sub_mask: u32,
    props: PaPropBag,
    name: String,
    pid: u32,
    uid: u32,
    uid_valid: bool,
    next_channel: u32,
    rx_hdr: [u8; PA_PSTREAM_DESC_BYTES],
    rx_hdr_got: usize,
    rx_body: Vec<u8>,
    rx_body_len: usize,
    rx_body_got: usize,
    rx_channel: u32,
    last_cmd: u32,
    out_q: VecDeque<OutPacket>,
}

impl PaClient {
    fn new(slot: usize) -> Self {
        Self {
            in_use: false,
            _slot: slot,
            index: 0,
            fd: None,
            is_tcp: false,
            accepted_ms: 0,
            auth_failures: 0,
            version: 0,
            authed: false,
            sub_mask: 0,
            props: PaPropBag::default(),
            name: String::new(),
            pid: 0,
            uid: 0,
            uid_valid: false,
            next_channel: 0,
            rx_hdr: [0; PA_PSTREAM_DESC_BYTES],
            rx_hdr_got: 0,
            rx_body: Vec::with_capacity(4096),
            rx_body_len: 0,
            rx_body_got: 0,
            rx_channel: 0,
            last_cmd: 0,
            out_q: VecDeque::with_capacity(32),
        }
    }

    fn reset_slot(&mut self, index: u32, fd: OwnedFd, uid_opt: Option<(u32, u32)>, is_tcp: bool) {
        self.in_use = true;
        self.index = index;
        self.fd = Some(fd);
        self.is_tcp = is_tcp;
        self.accepted_ms = mono_ms();
        self.auth_failures = 0;
        self.version = 0;
        self.authed = false;
        self.sub_mask = 0;
        self.props = PaPropBag::default();
        self.name.clear();
        self.pid = 0;
        self.uid = 0;
        self.uid_valid = false;
        if let Some((u, p)) = uid_opt {
            self.uid_valid = true;
            self.uid = u;
            self.pid = p;
        }
        self.next_channel = 0;
        self.rx_hdr = [0; PA_PSTREAM_DESC_BYTES];
        self.rx_hdr_got = 0;
        self.rx_body.clear();
        self.rx_body_len = 0;
        self.rx_body_got = 0;
        self.rx_channel = 0;
        self.last_cmd = 0;
        self.out_q.clear();
    }

    fn raw_fd(&self) -> RawFd {
        self.fd.as_ref().map(|f| f.as_raw_fd()).unwrap_or(-1)
    }

    fn queue_packet(&mut self, channel: u32, payload: &[u8]) -> bool {
        if !self.in_use || self.fd.is_none() || self.out_q.len() >= PA_MAX_OUT_QUEUE {
            return false;
        }
        let pkt = frame_packet(channel, payload);
        self.out_q.push_back(OutPacket { data: pkt, sent: 0 });
        true
    }

    fn queue_tb(&mut self, tb: TagBuilder) {
        let bytes = tb.into_vec();
        let _ = self.queue_packet(PA_PSTREAM_CTRL_CHANNEL, &bytes);
    }

    fn reply_simple_ack(&mut self, tag: u32) {
        let tb = TagBuilder::begin_reply(tag, 32);
        self.queue_tb(tb);
    }

    fn reply_err(&mut self, tag: u32, pa_err: u32) {
        let cname = pa_cmd_name(self.last_cmd);
        let benign_race = pa_err == ERR_NOEXTENSION
            || (pa_err == ERR_NOENTITY
                && matches!(
                    self.last_cmd,
                    CMD_GET_SINK_INPUT_INFO
                        | CMD_GET_SOURCE_OUTPUT_INFO
                        | CMD_GET_CLIENT_INFO
                        | CMD_GET_MODULE_INFO
                        | CMD_DELETE_PLAYBACK_STREAM
                        | CMD_DELETE_RECORD_STREAM
                        | CMD_CORK_PLAYBACK_STREAM
                        | CMD_CORK_RECORD_STREAM
                        | CMD_FLUSH_PLAYBACK_STREAM
                        | CMD_FLUSH_RECORD_STREAM
                        | CMD_GET_PLAYBACK_LATENCY
                        | CMD_GET_RECORD_LATENCY
                ));
        let lvl = if benign_race {
            LASAPI_DBG_TRACE
        } else {
            LASAPI_DBG_WARN
        };
        let tgt = if self.name.is_empty() {
            "pa-client"
        } else {
            &self.name
        };
        dbg_emit_vals(
            LASAPI_DBG_PA,
            lvl,
            self.pid,
            self.index,
            pa_err as i32,
            tag as i32,
            -(pa_err as i32),
            "PA_ERR",
            tgt,
            &format!("PA reply error={pa_err} on {cname} (tag={tag})"),
        );
        let mut tb = TagBuilder::new(32);
        tb.put_u32(PA_TAG_U32, CMD_ERROR);
        tb.put_u32(PA_TAG_U32, tag);
        tb.put_u32(PA_TAG_U32, pa_err);
        self.queue_tb(tb);
    }
}

#[derive(Clone, Default)]
struct PaStreamMeta {
    in_use: bool,
    index: u32,
    channel: u32,
    client_slot: usize,
    client_index: u32,
    client_pid: u32,
    client_name: String,
    is_playback: bool,
    is_monitor: bool,
    peak_detect: bool,
    direct_on_input: u32,
    pinned_route: bool,
    target_dev_idx: u32,
    device_id: String,
    name: String,
    props: PaPropBag,
    ss: PaSampleSpec,
    map: PaChannelMap,
    vol: PaCVolume,
    frame_size: u32,
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
    drain_tag: u32,
    started_sent: bool,
    requested_bytes_total: u64,
}

#[derive(Clone, Default)]
struct StableId {
    used: bool,
    facility: u32,
    idx: u32,
    key: String,
}

#[derive(Clone)]
struct AppVolEntry {
    key: String,
    volume_milli: u32,
    muted: u8,
}

#[derive(Clone, Default)]
struct LasapiPaProxyMeta {
    active: bool,
    session_id: u32,
    pa_stream_idx: u32,
    pa_client_idx: u32,
    client_pid: u32,
    client_name: String,
    stream_name: String,
    target_dev_idx: u32,
    ss: PaSampleSpec,
    map: PaChannelMap,
    vol: PaCVolume,
    client_props: PaPropBag,
    stream_props: PaPropBag,
}

struct PaState {
    host: Arc<dyn PaEngineHost>,
    _sock_path: String,
    _pid_path: String,
    cookie: [u8; PA_NATIVE_COOKIE_LEN],
    cookie_valid: bool,
    clients: Vec<PaClient>,
    streams: Vec<PaStreamMeta>,
    lasapi_proxies: Vec<LasapiPaProxyMeta>,
    next_client_idx: u32,
    next_stream_idx: u32,
    stable_ids: Vec<StableId>,
    next_fac_idx: [u32; 3],
    cached_status: LasapiEngineStatus,
    cached_devices: LasapiDeviceList,
    devices_cache_valid: bool,
    sink_pa_vol_valid: bool,
    sink_pa_vol: u32,
    hw_src_pa_vol_valid: bool,
    hw_src_pa_vol: u32,
    mon_pa_vol: u32,
    app_vols: Vec<AppVolEntry>,
    app_vols_path: String,
    scratch_rx: Vec<u8>,
    scratch_tx_pcm: Vec<u8>,
}

impl PaState {
    fn new(host: Arc<dyn PaEngineHost>, sock_path: String, pid_path: String) -> Self {
        let mut clients = Vec::with_capacity(PA_MAX_CLIENTS);
        for s in 0..PA_MAX_CLIENTS {
            clients.push(PaClient::new(s));
        }
        let streams = vec![PaStreamMeta::default(); PA_MAX_STREAMS];
        let lasapi_proxies = vec![LasapiPaProxyMeta::default(); LASAPI_MAX_SESSIONS];
        let stable_ids = vec![StableId::default(); PA_MAX_STABLE_IDS];
        let mut st = Self {
            host,
            _sock_path: sock_path,
            _pid_path: pid_path,
            cookie: [0; PA_NATIVE_COOKIE_LEN],
            cookie_valid: false,
            clients,
            streams,
            lasapi_proxies,
            next_client_idx: 1,
            next_stream_idx: 1,
            stable_ids,
            next_fac_idx: [0; 3],
            cached_status: unsafe { mem::zeroed() },
            cached_devices: unsafe { mem::zeroed() },
            devices_cache_valid: false,
            sink_pa_vol_valid: false,
            sink_pa_vol: PA_VOLUME_NORM,
            hw_src_pa_vol_valid: false,
            hw_src_pa_vol: PA_VOLUME_NORM,
            mon_pa_vol: PA_VOLUME_NORM,
            app_vols: Vec::with_capacity(64),
            app_vols_path: init_app_vols_path(),
            scratch_rx: Vec::with_capacity(4096),
            scratch_tx_pcm: Vec::with_capacity(16384),
        };
        ensure_pulse_cookie(&mut st.cookie, &mut st.cookie_valid);
        st.load_app_volumes_from_disk();
        st
    }

    fn accept_client_fd(&mut self, owned: OwnedFd, uid_opt: Option<(u32, u32)>, is_tcp: bool) {
        let unauth_cnt = self
            .clients
            .iter()
            .filter(|c| c.in_use && !c.authed)
            .count();
        if unauth_cnt >= PA_MAX_UNAUTH_CLIENTS {
            return;
        }
        if is_tcp {
            let tcp_cnt = self
                .clients
                .iter()
                .filter(|c| c.in_use && c.is_tcp)
                .count();
            if tcp_cnt >= PA_MAX_TCP_CLIENTS {
                return;
            }
        }
        if let Some(slot) = self.clients.iter().position(|c| !c.in_use) {
            let idx = self.next_client_idx;
            self.next_client_idx += 1;
            self.clients[slot].reset_slot(idx, owned, uid_opt, is_tcp);
        }
    }

    fn stable_pa_idx(&mut self, fac: u32, key: &str) -> u32 {
        if key.is_empty() || fac >= 3 {
            return PA_INVALID_INDEX;
        }
        for s in &self.stable_ids {
            if s.used && s.facility == fac && s.key == key {
                return s.idx;
            }
        }
        for s in &mut self.stable_ids {
            if !s.used {
                s.used = true;
                s.facility = fac;
                s.idx = self.next_fac_idx[fac as usize];
                self.next_fac_idx[fac as usize] += 1;
                s.key = key.chars().take(95).collect();
                return s.idx;
            }
        }
        PA_INVALID_INDEX
    }

    fn refresh_status_only(&mut self) {
        self.host.fill_status(&mut self.cached_status);
    }

    fn refresh_device_cache_force(&mut self) {
        self.refresh_status_only();
        self.host.enumerate_devices(&mut self.cached_devices);
        self.devices_cache_valid = true;
    }

    fn refresh_device_cache(&mut self) {
        self.refresh_status_only();
        if !self.devices_cache_valid {
            self.host.enumerate_devices(&mut self.cached_devices);
            self.devices_cache_valid = true;
        }
    }

    fn broadcast_sub_event(&mut self, facility: u32, ev_type: u32, idx: u32) {
        let ev = facility | ev_type;
        let bit = 1u32 << (facility & 0x0F);
        for c in &mut self.clients {
            if !c.in_use || !c.authed || (c.sub_mask & bit) == 0 {
                continue;
            }
            let mut tb = TagBuilder::new(48);
            tb.put_u32(PA_TAG_U32, CMD_SUBSCRIBE_EVENT);
            tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
            tb.put_u32(PA_TAG_U32, ev);
            tb.put_u32(PA_TAG_U32, idx);
            c.queue_tb(tb);
        }
    }

    fn cached_dev_count(&self) -> usize {
        (self.cached_devices.num_devices as usize).min(LASAPI_MAX_DEVICES)
    }

    fn get_default_sink_pa_name(&self) -> String {
        let devs = &self.cached_devices.devices[..self.cached_dev_count()];
        let act = c_str_to_string(&self.cached_status.active_playback_id);
        if !act.is_empty()
            && act != "virtual"
            && devs
                .iter()
                .any(|d| d.is_capture == 0 && c_str_to_string(&d.id) == act)
        {
            return make_pa_sink_name(&act);
        }
        if let Some(d) = devs.iter().find(|d| d.is_capture == 0 && d.is_active != 0) {
            return make_pa_sink_name(&c_str_to_string(&d.id));
        }
        if let Some(d) = devs.iter().find(|d| d.is_capture == 0) {
            return make_pa_sink_name(&c_str_to_string(&d.id));
        }
        "salsa_out.virtual".to_string()
    }

    fn get_default_source_pa_name(&self) -> String {
        let devs = &self.cached_devices.devices[..self.cached_dev_count()];
        let act = c_str_to_string(&self.cached_status.active_capture_id);
        if !act.is_empty()
            && act != "virtual"
            && devs
                .iter()
                .any(|d| d.is_capture != 0 && c_str_to_string(&d.id) == act)
        {
            return make_pa_source_name(&act);
        }
        if let Some(d) = devs.iter().find(|d| d.is_capture != 0 && d.is_active != 0) {
            return make_pa_source_name(&c_str_to_string(&d.id));
        }
        if let Some(d) = devs.iter().find(|d| d.is_capture != 0) {
            return make_pa_source_name(&c_str_to_string(&d.id));
        }
        let sink_n = self.get_default_sink_pa_name();
        format!("{sink_n}.monitor")
    }

    fn find_card_idx_for_id(&mut self, dev_id: &str) -> u32 {
        if dev_id.is_empty() || dev_id == "virtual" {
            return PA_INVALID_INDEX;
        }
        let n = self.cached_dev_count();
        for i in 0..n {
            let id = c_str_to_string(&self.cached_devices.devices[i].id);
            if id == dev_id {
                return self.stable_pa_idx(PA_FAC_CARD, dev_id);
            }
        }
        PA_INVALID_INDEX
    }

    fn find_monitor_src_idx_for_sink(&mut self, sink_idx: u32) -> u32 {
        let mut pb_cnt = 0u32;
        let n = self.cached_dev_count();
        for i in 0..n {
            if self.cached_devices.devices[i].is_capture != 0 {
                continue;
            }
            pb_cnt += 1;
            let did = c_str_to_string(&self.cached_devices.devices[i].id);
            if self.stable_pa_idx(PA_FAC_SINK, &did) == sink_idx {
                let mk = make_mon_key(&did);
                return self.stable_pa_idx(PA_FAC_SOURCE, &mk);
            }
        }
        if pb_cnt == 0 {
            return self.stable_pa_idx(PA_FAC_SOURCE, "mon:virtual");
        }
        PA_INVALID_INDEX
    }

    fn resolve_pa_target_name(&self, is_source: bool, req_nm: Option<&str>) -> String {
        let has_hw_pb = self.cached_devices.devices[..self.cached_dev_count()]
            .iter()
            .any(|d| d.is_capture == 0);
        match req_nm {
            None => {
                if is_source {
                    self.get_default_source_pa_name()
                } else {
                    self.get_default_sink_pa_name()
                }
            }
            Some("@DEFAULT_SINK@") => self.get_default_sink_pa_name(),
            Some("@DEFAULT_SOURCE@") if is_source => self.get_default_source_pa_name(),
            Some("@DEFAULT_MONITOR@") if is_source => {
                format!("{}.monitor", self.get_default_sink_pa_name())
            }
            Some("salsa_out.virtual") if !is_source && has_hw_pb => {
                self.get_default_sink_pa_name()
            }
            Some("salsa_out.virtual.monitor") if is_source && has_hw_pb => {
                format!("{}.monitor", self.get_default_sink_pa_name())
            }
            Some(s) => s.to_string(),
        }
    }

    fn resolve_sink_idx_and_name(&mut self, req_idx: u32, req_nm: Option<&str>) -> Option<(u32, String)> {
        let tgt = if req_idx == PA_INVALID_INDEX {
            self.resolve_pa_target_name(false, req_nm)
        } else {
            String::new()
        };
        let mut pb_cnt = 0u32;
        let n = self.cached_dev_count();
        for i in 0..n {
            if self.cached_devices.devices[i].is_capture != 0 {
                continue;
            }
            pb_cnt += 1;
            let did = c_str_to_string(&self.cached_devices.devices[i].id);
            let sidx = self.stable_pa_idx(PA_FAC_SINK, &did);
            let pa_n = make_pa_sink_name(&did);
            if (req_idx != PA_INVALID_INDEX && req_idx == sidx)
                || (req_idx == PA_INVALID_INDEX && tgt == pa_n)
            {
                return Some((sidx, pa_n));
            }
        }
        if pb_cnt == 0 {
            let vidx = self.stable_pa_idx(PA_FAC_SINK, "virtual");
            if req_idx == vidx
                || (req_idx == PA_INVALID_INDEX && (req_nm.is_none() || tgt == "salsa_out.virtual"))
            {
                return Some((vidx, "salsa_out.virtual".to_string()));
            }
        }
        None
    }

    fn resolve_source_idx_and_name(
        &mut self,
        req_idx: u32,
        req_nm: Option<&str>,
    ) -> Option<(u32, String, bool)> {
        let tgt = if req_idx == PA_INVALID_INDEX {
            self.resolve_pa_target_name(true, req_nm)
        } else {
            String::new()
        };
        let n = self.cached_dev_count();
        for i in 0..n {
            if self.cached_devices.devices[i].is_capture == 0 {
                continue;
            }
            let did = c_str_to_string(&self.cached_devices.devices[i].id);
            let src_idx = self.stable_pa_idx(PA_FAC_SOURCE, &did);
            let pa_n = make_pa_source_name(&did);
            if (req_idx != PA_INVALID_INDEX && req_idx == src_idx)
                || (req_idx == PA_INVALID_INDEX && tgt == pa_n)
            {
                return Some((src_idx, pa_n, false));
            }
        }
        let mut pb_cnt = 0u32;
        for i in 0..n {
            if self.cached_devices.devices[i].is_capture != 0 {
                continue;
            }
            pb_cnt += 1;
            let did = c_str_to_string(&self.cached_devices.devices[i].id);
            let mk = make_mon_key(&did);
            let src_idx = self.stable_pa_idx(PA_FAC_SOURCE, &mk);
            let mon_n = format!("{}.monitor", make_pa_sink_name(&did));
            if (req_idx != PA_INVALID_INDEX && req_idx == src_idx)
                || (req_idx == PA_INVALID_INDEX && tgt == mon_n)
            {
                return Some((src_idx, mon_n, true));
            }
        }
        if pb_cnt == 0 {
            let vmon = self.stable_pa_idx(PA_FAC_SOURCE, "mon:virtual");
            if req_idx == vmon
                || (req_idx == PA_INVALID_INDEX
                    && (req_nm.is_none() || tgt == "salsa_out.virtual.monitor"))
            {
                return Some((vmon, "salsa_out.virtual.monitor".to_string(), true));
            }
        }
        None
    }

    fn find_active_sink_idx(&mut self) -> u32 {
        self.resolve_sink_idx_and_name(PA_INVALID_INDEX, None)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn find_active_source_idx(&mut self) -> u32 {
        self.resolve_source_idx_and_name(PA_INVALID_INDEX, None)
            .map(|(i, _, _)| i)
            .unwrap_or(0)
    }

    fn source_idx_to_salsa_id(&mut self, target_src_idx: u32) -> Option<String> {
        let n = self.cached_dev_count();
        for i in 0..n {
            if self.cached_devices.devices[i].is_capture == 0 {
                continue;
            }
            let did = c_str_to_string(&self.cached_devices.devices[i].id);
            if self.stable_pa_idx(PA_FAC_SOURCE, &did) == target_src_idx {
                return Some(did);
            }
        }
        None
    }

    fn sink_idx_to_salsa_id(&mut self, target_sink_idx: u32) -> Option<String> {
        let mut pb_cnt = 0u32;
        let n = self.cached_dev_count();
        for i in 0..n {
            if self.cached_devices.devices[i].is_capture != 0 {
                continue;
            }
            pb_cnt += 1;
            let did = c_str_to_string(&self.cached_devices.devices[i].id);
            if self.stable_pa_idx(PA_FAC_SINK, &did) == target_sink_idx {
                return Some(did);
            }
        }
        if pb_cnt == 0 && target_sink_idx == self.stable_pa_idx(PA_FAC_SINK, "virtual") {
            return Some("virtual".to_string());
        }
        None
    }

    fn pa_name_to_salsa_id(&self, pa_name: &str, is_capture: bool) -> Option<String> {
        if pa_name == "salsa_out.virtual" && !is_capture {
            return Some("virtual".to_string());
        }
        for d in &self.cached_devices.devices[..self.cached_dev_count()] {
            if (d.is_capture != 0) != is_capture {
                continue;
            }
            let did = c_str_to_string(&d.id);
            let cand = if is_capture {
                make_pa_source_name(&did)
            } else {
                make_pa_sink_name(&did)
            };
            if cand == pa_name || did == pa_name {
                return Some(did);
            }
        }
        None
    }

    fn stream_update_device_id(&mut self, si: usize) {
        let is_pb = self.streams[si].is_playback;
        let is_mon = self.streams[si].is_monitor;
        let tgt_idx = self.streams[si].target_dev_idx;
        let new_id = if is_pb {
            self.sink_idx_to_salsa_id(tgt_idx).unwrap_or_else(|| {
                let act = c_str_to_string(&self.cached_status.active_playback_id);
                if act.is_empty() {
                    "virtual".to_string()
                } else {
                    act
                }
            })
        } else if is_mon {
            "monitor".to_string()
        } else {
            self.source_idx_to_salsa_id(tgt_idx).unwrap_or_else(|| {
                let act = c_str_to_string(&self.cached_status.active_capture_id);
                if act.is_empty() {
                    "virtual".to_string()
                } else {
                    act
                }
            })
        };
        self.streams[si].device_id = new_id;
    }

    fn sync_active_capture_if_explicit(
        &mut self,
        actual_src_idx: u32,
        is_mon: bool,
        peak_detect: bool,
        explicit_req: bool,
    ) {
        if is_mon || peak_detect || !explicit_req {
            return;
        }
        let Some(salsa_id) = self.source_idx_to_salsa_id(actual_src_idx) else { return };
        if !salsa_id.is_empty() && salsa_id != c_str_to_string(&self.cached_status.active_capture_id) {
            self.host.switch_devices(None, Some(&salsa_id));
            self.refresh_device_cache();
        }
    }

    fn load_app_volumes_from_disk(&mut self) {
        if self.app_vols_path.is_empty() {
            return;
        }
        let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(unix_sys::O_NOFOLLOW | unix_sys::O_CLOEXEC)
            .open(&self.app_vols_path)
        else {
            return;
        };
        let Ok(meta) = file.metadata() else { return };
        if !meta.is_file() || meta.len() == 0 || meta.len() > 262_144 {
            return;
        }
        self.app_vols.clear();
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            if self.app_vols.len() >= PA_MAX_APP_VOLUMES {
                break;
            }
            let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((k, v)) = trimmed.split_once('=') else {
                continue;
            };
            let (is_pb, raw_k) = if let Some(rest) = k.strip_prefix("play:") {
                (true, rest)
            } else if let Some(rest) = k.strip_prefix("rec:") {
                (false, rest)
            } else {
                continue;
            };
            let Some(clean_k) = sanitize_app_vol_key(is_pb, raw_k) else {
                continue;
            };
            let (milli_s, muted_s) = match v.split_once(',') {
                Some((a, b)) => (a, Some(b)),
                None => (v, None),
            };
            let Ok(milli) = milli_s.parse::<u32>() else {
                continue;
            };
            if milli > 1500 {
                continue;
            }
            let muted = muted_s
                .and_then(|s| s.trim().parse::<u32>().ok())
                .map(|m| u8::from(m != 0))
                .unwrap_or(0);
            self.app_vols.push(AppVolEntry {
                key: clean_k,
                volume_milli: milli,
                muted,
            });
        }
    }

    fn upsert_app_vol(&mut self, key: String, milli: u32, muted: u8) {
        let milli = milli.min(1500);
        if let Some(entry) = self.app_vols.iter_mut().find(|e| e.key == key) {
            entry.volume_milli = milli;
            entry.muted = u8::from(muted != 0);
            return;
        }
        if self.app_vols.len() >= PA_MAX_APP_VOLUMES {
            self.app_vols.remove(0);
        }
        self.app_vols.push(AppVolEntry {
            key,
            volume_milli: milli,
            muted: u8::from(muted != 0),
        });
    }

    fn app_vols_snapshot(&self) -> (String, Vec<AppVolEntry>) {
        (self.app_vols_path.clone(), self.app_vols.clone())
    }

    fn build_stream_app_vol_key(&self, si: usize) -> Option<String> {
        let sm = &self.streams[si];
        if sm.peak_detect {
            return None;
        }
        let client_props = self
            .clients
            .get(sm.client_slot)
            .filter(|c| c.in_use)
            .map(|c| &c.props);
        let raw = sm
            .props
            .get_str("application.process.binary")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                client_props
                    .and_then(|p| p.get_str("application.process.binary"))
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                sm.props
                    .get_str("application.name")
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                client_props
                    .and_then(|p| p.get_str("application.name"))
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                self.clients
                    .get(sm.client_slot)
                    .filter(|c| !c.name.is_empty())
                    .map(|c| c.name.clone())
            })
            .unwrap_or_else(|| sm.name.clone());
        sanitize_app_vol_key(sm.is_playback, &raw)
    }

    fn record_app_volume(&mut self, si: usize, rt: &PaRtShared) {
        if let Some(key) = self.build_stream_app_vol_key(si) {
            let milli = rt.streams[si].volume_milli.load(Ordering::Relaxed);
            let muted = u8::from(rt.streams[si].muted.load(Ordering::Relaxed) != 0);
            self.upsert_app_vol(key, milli, muted);
        }
    }

    fn apply_saved_app_volume(&mut self, si: usize, vol_explicit: bool, rt: &PaRtShared) {
        let Some(key) = self.build_stream_app_vol_key(si) else { return };
        let Some(entry) = self.app_vols.iter().find(|e| e.key == key).cloned() else {
            return;
        };
        let cur_milli = rt.streams[si].volume_milli.load(Ordering::Relaxed);
        if !vol_explicit || cur_milli == 1000 {
            let milli = entry.volume_milli.min(1500);
            let pv = pct_to_pa_vol(milli / 10);
            let ch = (self.streams[si].vol.channels as usize).min(32);
            for v in &mut self.streams[si].vol.values[..ch] {
                *v = pv;
            }
            rt.streams[si].volume_milli.store(milli, Ordering::Relaxed);
            rt.streams[si]
                .muted
                .store(u32::from(entry.muted != 0), Ordering::Relaxed);
        }
    }

    fn free_stream(&mut self, si: usize, rt: &PaRtShared) {
        if si >= PA_MAX_STREAMS || !self.streams[si].in_use {
            return;
        }
        let is_pb = self.streams[si].is_playback;
        let idx = self.streams[si].index;
        rt.streams[si].active.store(0, Ordering::Release);
        rt.streams[si].in_use.store(false, Ordering::Release);
        rt.streams[si]
            .direct_on_input
            .store(PA_INVALID_INDEX, Ordering::Relaxed);
        self.streams[si].in_use = false;
        rt.sync_active_doi();
        self.broadcast_sub_event(
            if is_pb {
                SUB_EVENT_SINK_INPUT
            } else {
                SUB_EVENT_SOURCE_OUTPUT
            },
            SUB_EVENT_REMOVE,
            idx,
        );
    }

    fn close_client(&mut self, ci: usize, rt: &PaRtShared) {
        if ci >= PA_MAX_CLIENTS || !self.clients[ci].in_use {
            return;
        }
        if self.clients[ci].authed || !self.clients[ci].name.is_empty() {
            let pid = self.clients[ci].pid;
            let idx = self.clients[ci].index;
            let ver = self.clients[ci].version;
            let out_c = self.clients[ci].out_q.len();
            let tgt = if self.clients[ci].name.is_empty() {
                "pa-client"
            } else {
                &self.clients[ci].name
            };
            dbg_emit_vals(
                LASAPI_DBG_PA,
                LASAPI_DBG_INFO,
                pid,
                idx,
                ver as i32,
                out_c as i32,
                0,
                "PA_CLIENT_CLOSE",
                tgt,
                &format!("closing PA client #{idx} (pid={pid})"),
            );
        }
        for si in 0..PA_MAX_STREAMS {
            if self.streams[si].in_use && self.streams[si].client_slot == ci {
                self.free_stream(si, rt);
            }
        }
        let c = &mut self.clients[ci];
        c.out_q.clear();
        c.rx_body.clear();
        c.fd = None;
        let cidx = c.index;
        let was_authed = c.authed;
        c.in_use = false;
        if was_authed {
            self.broadcast_sub_event(SUB_EVENT_CLIENT, SUB_EVENT_REMOVE, cidx);
        }
    }

    fn find_client_stream(&self, ci: usize, channel: u32) -> Option<usize> {
        for (si, s) in self.streams.iter().enumerate() {
            if s.in_use && s.client_slot == ci && s.channel == channel {
                return Some(si);
            }
        }
        None
    }

    fn find_stream_by_index(&self, idx: u32) -> Option<usize> {
        for (si, s) in self.streams.iter().enumerate() {
            if s.in_use && s.index == idx {
                return Some(si);
            }
        }
        None
    }

    fn find_lasapi_proxy_by_stream_idx(&self, idx: u32) -> Option<usize> {
        for (slot_idx, w) in self.lasapi_proxies.iter().enumerate() {
            if w.active && w.pa_stream_idx == idx {
                return Some(slot_idx);
            }
        }
        None
    }

    fn sync_lasapi_proxy_vol_from_rt(
        w: &mut LasapiPaProxyMeta,
        ws: &PaLasapiRtSlot,
    ) -> (bool, bool) {
        let milli = ws.volume_milli.load(Ordering::Relaxed);
        let muted = ws.muted.load(Ordering::Relaxed);
        let ch = w.ss.channels.max(1);
        w.vol = milli_to_cvol(milli, ch);
        (muted, false)
    }

    fn register_lasapi_proxy(
        &mut self,
        slot_idx: usize,
        session_id: u32,
        pid: u32,
        raw_name: &str,
        rate: u32,
        channels: u32,
        rt: &PaRtShared,
    ) {
        if slot_idx >= LASAPI_MAX_SESSIONS || slot_idx >= self.lasapi_proxies.len() {
            return;
        }
        if self.lasapi_proxies[slot_idx].active {
            if self.lasapi_proxies[slot_idx].session_id == session_id {
                return;
            }
            self.unregister_lasapi_proxy(slot_idx, self.lasapi_proxies[slot_idx].session_id, rt);
        }
        self.refresh_device_cache();
        let active_sink = self.find_active_sink_idx();
        let pa_client_idx = self.next_client_idx;
        self.next_client_idx = self.next_client_idx.wrapping_add(1).max(1);
        let pa_stream_idx = self.next_stream_idx;
        self.next_stream_idx = self.next_stream_idx.wrapping_add(1).max(1);

        let clean_name = if raw_name.trim().is_empty() {
            "AudioSession"
        } else {
            raw_name.trim()
        };
        let app_bin = clean_name
            .split(':')
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(clean_name);

        let mut cprops = PaPropBag::default();
        cprops.set_kv("application.name", app_bin);
        cprops.set_kv("application.process.binary", app_bin);
        cprops.set_kv("salsa.lasapi.session_id", &session_id.to_string());
        if pid > 0 {
            cprops.set_kv("application.process.id", &pid.to_string());
        }

        let mut sprops = cprops.clone();
        sprops.set_kv("media.name", clean_name);

        let ch_u8 = (channels as u8).clamp(1, 8);
        let eff_rate = rate.clamp(8_000, 768_000);
        let ss = PaSampleSpec {
            format: FMT_FLOAT32LE,
            channels: ch_u8,
            rate: eff_rate,
        };
        let map = std_channel_map(ch_u8);
        let vol = milli_to_cvol(1000, ch_u8);

        self.lasapi_proxies[slot_idx] = LasapiPaProxyMeta {
            active: true,
            session_id,
            pa_stream_idx,
            pa_client_idx,
            client_pid: pid,
            client_name: app_bin.to_string(),
            stream_name: clean_name.to_string(),
            target_dev_idx: active_sink,
            ss,
            map,
            vol,
            client_props: cprops,
            stream_props: sprops,
        };

        let ws = &rt.lasapi_slots[slot_idx];
        ws.session_id.store(session_id, Ordering::Relaxed);
        ws.pa_stream_idx.store(pa_stream_idx, Ordering::Release);
        ws.volume_milli.store(1000, Ordering::Relaxed);
        ws.muted.store(false, Ordering::Relaxed);
        ws.doi_watched.store(false, Ordering::Release);
        ws.pa_ctl_seq.store(0, Ordering::Release);
        ws.pa_target_vol_milli.store(1000, Ordering::Relaxed);
        ws.pa_target_muted.store(false, Ordering::Relaxed);
        ws.mix_peak_l.store(0, Ordering::Relaxed);
        ws.mix_peak_r.store(0, Ordering::Relaxed);
        ws.period_frames.store(0, Ordering::Relaxed);
        ws.active.store(true, Ordering::Release);
        rt.sync_active_doi();

        self.broadcast_sub_event(SUB_EVENT_CLIENT, SUB_EVENT_NEW, pa_client_idx);
        self.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_NEW, pa_stream_idx);
        dbg_emit_vals(
            LASAPI_DBG_PA,
            LASAPI_DBG_INFO,
            pid,
            10000 + pa_stream_idx,
            eff_rate as i32,
            i32::from(ch_u8),
            session_id as i32,
            "PA_LASAPI_ADD",
            clean_name,
            &format!("pa_si=#{pa_stream_idx} pa_cid=#{pa_client_idx} lasapi_sid=#{session_id}"),
        );
    }

    fn unregister_lasapi_proxy(&mut self, slot_idx: usize, session_id: u32, rt: &PaRtShared) {
        if slot_idx >= LASAPI_MAX_SESSIONS || slot_idx >= self.lasapi_proxies.len() {
            return;
        }
        let w = &self.lasapi_proxies[slot_idx];
        if !w.active || (session_id != 0 && w.session_id != session_id) {
            return;
        }
        let pa_stream_idx = w.pa_stream_idx;
        let pa_client_idx = w.pa_client_idx;
        let pid = w.client_pid;
        let nm = w.stream_name.clone();

        let ws = &rt.lasapi_slots[slot_idx];
        ws.active.store(false, Ordering::Release);
        ws.doi_watched.store(false, Ordering::Release);
        ws.period_frames.store(0, Ordering::Relaxed);
        ws.pa_stream_idx.store(PA_INVALID_INDEX, Ordering::Release);
        ws.session_id.store(0, Ordering::Relaxed);
        self.lasapi_proxies[slot_idx] = LasapiPaProxyMeta::default();
        rt.sync_active_doi();

        self.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_REMOVE, pa_stream_idx);
        self.broadcast_sub_event(SUB_EVENT_CLIENT, SUB_EVENT_REMOVE, pa_client_idx);
        dbg_emit_vals(
            LASAPI_DBG_PA,
            LASAPI_DBG_INFO,
            pid,
            10000 + pa_stream_idx,
            0,
            0,
            session_id as i32,
            "PA_LASAPI_DEL",
            &nm,
            &format!("pa_si=#{pa_stream_idx} pa_cid=#{pa_client_idx} lasapi_sid=#{session_id}"),
        );
    }

    fn alloc_stream(&mut self, ci: usize, rt: &PaRtShared) -> Option<usize> {
        let c_idx = self.clients[ci].index;
        let c_pid = self.clients[ci].pid;
        let c_name = self.clients[ci].name.clone();
        let ch = self.clients[ci].next_channel;
        self.clients[ci].next_channel += 1;

        for si in 0..PA_MAX_STREAMS {
            if !self.streams[si].in_use {
                let s_idx = self.next_stream_idx;
                self.next_stream_idx += 1;
                let sm = PaStreamMeta {
                    in_use: true,
                    index: s_idx,
                    channel: ch,
                    client_slot: ci,
                    client_index: c_idx,
                    client_pid: c_pid,
                    client_name: c_name,
                    direct_on_input: PA_INVALID_INDEX,
                    ..PaStreamMeta::default()
                };
                self.streams[si] = sm;

                let rs = &rt.streams[si];
                rs.active.store(0, Ordering::Release);
                rs.in_use.store(true, Ordering::Release);
                rs.index.store(s_idx, Ordering::Relaxed);
                rs.wr_frame.store(0, Ordering::Relaxed);
                rs.rd_frame.store(0, Ordering::Relaxed);
                rs.write_index.store(0, Ordering::Relaxed);
                rs.read_index.store(0, Ordering::Relaxed);
                rs.underrun_baseline_bytes.store(0, Ordering::Relaxed);
                rs.corked.store(0, Ordering::Relaxed);
                rs.in_prebuf.store(0, Ordering::Relaxed);
                rs.underflow_resync.store(false, Ordering::Relaxed);
                rs.draining.store(0, Ordering::Relaxed);
                rs.muted.store(0, Ordering::Relaxed);
                rs.volume_milli.store(1000, Ordering::Relaxed);
                rs.mix_peak_l.store(0, Ordering::Relaxed);
                rs.mix_peak_r.store(0, Ordering::Relaxed);
                rs.period_frames.store(0, Ordering::Relaxed);
                rs.flush_seq.fetch_add(1, Ordering::Release);
                return Some(si);
            }
        }
        None
    }

    fn send_stream_request(&mut self, si: usize, req_bytes: u32) {
        if !self.streams[si].in_use || req_bytes == 0 {
            return;
        }
        let ci = self.streams[si].client_slot;
        let ch = self.streams[si].channel;
        let mut tb = TagBuilder::new(48);
        tb.put_u32(PA_TAG_U32, CMD_REQUEST);
        tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
        tb.put_u32(PA_TAG_U32, ch);
        tb.put_u32(PA_TAG_U32, req_bytes);
        self.clients[ci].queue_tb(tb);
        self.streams[si].requested_bytes_total =
            self.streams[si].requested_bytes_total.saturating_add(req_bytes as u64);
    }

    fn send_stream_started(&mut self, si: usize) {
        if !self.streams[si].in_use || self.streams[si].started_sent {
            return;
        }
        self.streams[si].started_sent = true;
        let ci = self.streams[si].client_slot;
        let ch = self.streams[si].channel;
        let mut tb = TagBuilder::new(32);
        tb.put_u32(PA_TAG_U32, CMD_STARTED);
        tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
        tb.put_u32(PA_TAG_U32, ch);
        self.clients[ci].queue_tb(tb);
    }

    fn handle_data_memblock(&mut self, ci: usize, channel: u32, pcm: &[u8], rt: &PaRtShared) {
        let Some(si) = self.find_client_stream(ci, channel) else {
            return;
        };
        let sm = &self.streams[si];
        if !sm.is_playback || sm.frame_size == 0 {
            return;
        }
        let fsz = sm.frame_size as usize;
        let mut frames = pcm.len() / fsz;
        if frames == 0 {
            return;
        }
        let ssz = format_sample_size(sm.ss.format) as usize;
        let ch = sm.ss.channels;
        let fmt = sm.ss.format;
        let prebuf = sm.prebuf;
        let cmap = sm.map;

        let rs = &rt.streams[si];
        let mut wr = rs.wr_frame.load(Ordering::Relaxed);
        let rd = rs.rd_frame.load(Ordering::Acquire);
        if wr < rd {
            wr = rd;
        }
        let free_f = (PA_STREAM_RING_FRAMES as u64).saturating_sub(wr - rd) as usize;
        if frames > free_f {
            frames = free_f;
        }

        if ch <= 2 {
            for i in 0..frames {
                let fp = &pcm[i * fsz..];
                let l = decode_sample(fp, fmt);
                let r = if ch >= 2 && fp.len() >= ssz * 2 {
                    decode_sample(&fp[ssz..], fmt)
                } else {
                    l
                };
                rs.ring_set(wr as usize + i, l, r);
            }
        } else {
            let ch_n = (ch as usize).min(32);
            let mut w_l = [0.0f32; 32];
            let mut w_r = [0.0f32; 32];
            let mut sum_l = 0.0f32;
            let mut sum_r = 0.0f32;
            for c in 0..ch_n {
                let pos = if c < cmap.channels as usize {
                    cmap.map[c]
                } else {
                    12
                };
                let (gl, gr) = match pos {
                    0 | 3 | 4 | 44 | 47 | 50 => (0.707_106_77, 0.707_106_77), // mono / centers
                    7 => (0.5, 0.5),                                          // LFE
                    1 => (1.0, 0.0),                                          // front-left
                    2 => (0.0, 1.0),                                          // front-right
                    5 | 10 | 45 | 48 => (0.707_106_77, 0.0),                  // rear/side/top left
                    6 | 11 | 46 | 49 => (0.0, 0.707_106_77),                  // rear/side/top right
                    8 => (0.85, 0.35),                                        // front-left-of-center
                    9 => (0.35, 0.85),                                        // front-right-of-center
                    _ => {
                        if c % 2 == 0 {
                            (0.5, 0.0)
                        } else {
                            (0.0, 0.5)
                        }
                    }
                };
                w_l[c] = gl;
                w_r[c] = gr;
                sum_l += gl;
                sum_r += gr;
            }
            let max_sum = sum_l.max(sum_r);
            let norm = if max_sum > 1.5 {
                1.0 / max_sum.sqrt()
            } else {
                1.0
            };
            for c in 0..ch_n {
                w_l[c] *= norm;
                w_r[c] *= norm;
            }
            for i in 0..frames {
                let fp = &pcm[i * fsz..];
                let mut l = 0.0f32;
                let mut r = 0.0f32;
                for c in 0..ch_n {
                    let off = c * ssz;
                    if off + ssz <= fp.len() {
                        let s = decode_sample(&fp[off..], fmt);
                        l += s * w_l[c];
                        r += s * w_r[c];
                    }
                }
                rs.ring_set(wr as usize + i, l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0));
            }
        }

        let new_wr = wr + frames as u64;
        rs.wr_frame.store(new_wr, Ordering::Release);
        rs.write_index
            .fetch_add((frames as u64) * (fsz as u64), Ordering::Relaxed);

        if rs.in_prebuf.load(Ordering::Acquire) != 0
            && (new_wr.saturating_sub(rd)) * (fsz as u64) >= (prebuf as u64)
        {
            rs.in_prebuf.store(0, Ordering::Release);
            self.send_stream_started(si);
        } else if rs.in_prebuf.load(Ordering::Acquire) == 0
            && rs.corked.load(Ordering::Relaxed) == 0
            && !self.streams[si].started_sent
        {
            self.send_stream_started(si);
        }
    }

    fn service_stream_events_and_capture(&mut self, rt: &PaRtShared) {
        for si in 0..PA_MAX_STREAMS {
            if !self.streams[si].in_use {
                continue;
            }
            let ci = self.streams[si].client_slot;
            if ci >= PA_MAX_CLIENTS || !self.clients[ci].in_use {
                continue;
            }
            let rs = &rt.streams[si];
            if self.streams[si].is_playback {
                if rs.underflow_resync.swap(false, Ordering::AcqRel) {
                    self.streams[si].started_sent = false;
                }
                let wr = rs.wr_frame.load(Ordering::Acquire);
                let rd = rs.rd_frame.load(Ordering::Acquire);
                if rs.draining.load(Ordering::Relaxed) != 0 && rd >= wr {
                    rs.draining.store(0, Ordering::Relaxed);
                    let dtag = self.streams[si].drain_tag;
                    self.clients[ci].reply_simple_ack(dtag);
                }
                let fsz = self.streams[si].frame_size as u64;
                let queued_bytes = wr.saturating_sub(rd) * fsz;
                let wi = rs.write_index.load(Ordering::Relaxed);
                let ack_pending = self.streams[si].requested_bytes_total.saturating_sub(wi);
                let tl = self.streams[si].tlength as u64;
                let prebuf = self.streams[si].prebuf as u64;
                let in_prebuf_now = rs.in_prebuf.load(Ordering::Acquire) != 0;
                if queued_bytes + ack_pending < tl && rs.draining.load(Ordering::Relaxed) == 0 {
                    let need = (tl - (queued_bytes + ack_pending)) as u32;
                    if need >= self.streams[si].minreq
                        || (in_prebuf_now && queued_bytes + ack_pending < prebuf.min(tl))
                    {
                        self.send_stream_request(si, need);
                    }
                }
            } else {
                let wr = rs.wr_frame.load(Ordering::Acquire);
                let mut rd = rs.rd_frame.load(Ordering::Relaxed);
                if rs.corked.load(Ordering::Relaxed) != 0 {
                    rs.rd_frame.store(wr, Ordering::Relaxed);
                    continue;
                }
                let sm = &self.streams[si];
                let fsz = sm.frame_size as usize;
                if fsz == 0 {
                    continue;
                }
                let frag = sm.fragsize as u64;
                let ss_fmt = sm.ss.format;
                let ss_ch = sm.ss.channels;
                let ssz = format_sample_size(ss_fmt) as usize;
                let ch_id = sm.channel;
                let voice_fold = is_voice_comms_stream(sm);

                if wr >= rd && wr.saturating_sub(rd) * (fsz as u64) >= frag
                    && self.scratch_tx_pcm.len() < 16384
                {
                    self.scratch_tx_pcm.resize(16384, 0);
                }

                while wr >= rd && wr.saturating_sub(rd) * (fsz as u64) >= frag {
                    let max_b = (self.scratch_tx_pcm.len() / fsz) * fsz;
                    let send_bytes = (sm.fragsize as usize).min(max_b);
                    let send_frames = send_bytes / fsz;
                    if send_frames == 0 {
                        break;
                    }
                    for f in 0..send_frames {
                        let (mut l, mut r) = rs.ring_get(rd as usize + f);
                        if voice_fold {
                            let m = if (l - r).abs() < 1e-6 { l } else { (l + r).clamp(-1.0, 1.0) };
                            l = m;
                            r = m;
                        }
                        let dp = &mut self.scratch_tx_pcm[f * fsz..];
                        if ss_ch == 1 {
                            encode_sample(
                                dp,
                                ss_fmt,
                                if voice_fold { l } else { 0.5 * (l + r) },
                            );
                        } else {
                            encode_sample(dp, ss_fmt, l);
                            if dp.len() >= ssz {
                                encode_sample(&mut dp[ssz..], ss_fmt, r);
                            }
                            for ch_i in 2..(ss_ch as usize) {
                                let off = ch_i * ssz;
                                if dp.len() >= off + ssz {
                                    encode_sample(&mut dp[off..], ss_fmt, 0.0);
                                }
                            }
                        }
                    }
                    if !self.clients[ci].queue_packet(ch_id, &self.scratch_tx_pcm[..send_frames * fsz]) {
                        break;
                    }
                    rd += send_frames as u64;
                    let bytes = (send_frames as u64) * (fsz as u64);
                    rs.write_index.fetch_add(bytes, Ordering::Relaxed);
                    rs.read_index.fetch_add(bytes, Ordering::Relaxed);
                }
                rs.rd_frame.store(rd, Ordering::Release);
            }
        }
    }

    fn handle_control_packet(
        &mut self,
        ci: usize,
        buf: &[u8],
        rt: &Arc<PaRtShared>,
        vol_lock: &Mutex<()>,
    ) {
        let Some(mut tr) = TagReader::new(buf, 0) else {
            return;
        };
        let (Some(cmd), Some(tag)) = (tr.read_u32(), tr.read_u32()) else {
            failsafe::record_trip(failsafe::FS_PA_BAD_PKT, b"PA truncated control header quarantined");
            return;
        };
        self.clients[ci].last_cmd = cmd;

        if !pa_is_lifecycle_cmd(cmd) {
            let pid = self.clients[ci].pid;
            let idx = self.clients[ci].index;
            let tgt = if self.clients[ci].name.is_empty() {
                "pa-client"
            } else {
                &self.clients[ci].name
            };
            dbg_emit_vals(
                LASAPI_DBG_PA,
                LASAPI_DBG_TRACE,
                pid,
                idx,
                cmd as i32,
                tag as i32,
                buf.len() as i32,
                pa_cmd_name(cmd),
                tgt,
                &format!("len={}", buf.len()),
            );
        }

        if cmd == CMD_AUTH {
            let Some(cver) = tr.read_u32() else {
                failsafe::record_trip(failsafe::FS_PA_BAD_PKT, b"PA malformed AUTH frame");
                self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                return;
            };
            let Some((ck_off, ck_len)) = tr.read_arbitrary_slice() else {
                failsafe::record_trip(failsafe::FS_PA_BAD_PKT, b"PA malformed AUTH cookie");
                self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                return;
            };
            let uid_ok = self.clients[ci].uid_valid
                && (self.clients[ci].uid == current_uid() || self.clients[ci].uid == 0);
            let cookie_ok = self.cookie_valid
                && ck_len == PA_NATIVE_COOKIE_LEN
                && ct_eq_256(&buf[ck_off..ck_off + PA_NATIVE_COOKIE_LEN], &self.cookie);
            if !uid_ok && !cookie_ok {
                self.clients[ci].auth_failures = self.clients[ci].auth_failures.saturating_add(1);
                self.clients[ci].reply_err(tag, ERR_ACCESS);
                if self.clients[ci].auth_failures >= 2 {
                    self.close_client(ci, rt);
                }
                return;
            }
            self.clients[ci].version = cver & 0xFFFF;
            self.clients[ci].authed = true;
            dbg_emit_vals(
                LASAPI_DBG_PA,
                LASAPI_DBG_INFO,
                self.clients[ci].pid,
                self.clients[ci].index,
                self.clients[ci].version as i32,
                tag as i32,
                0,
                "PA_AUTH",
                "pulse/native",
                &format!(
                    "client#{} authed proto_ver={}",
                    self.clients[ci].index, self.clients[ci].version
                ),
            );
            let mut tb = TagBuilder::begin_reply(tag, 32);
            tb.put_u32(PA_TAG_U32, PA_PROTO_VERSION);
            self.clients[ci].queue_tb(tb);
            return;
        }

        if !self.clients[ci].authed {
            self.clients[ci].reply_err(tag, ERR_ACCESS);
            return;
        }

        match cmd {
            CMD_EXIT => {
                self.close_client(ci, rt);
            }
            CMD_SET_CLIENT_NAME => {
                if self.clients[ci].version < 13 {
                    if let Some(Some((off, slen))) = tr.read_str_slice() {
                        self.clients[ci].name =
                            String::from_utf8_lossy(&buf[off..off + slen]).into_owned();
                    }
                } else {
                    let mut pb = self.clients[ci].props.clone();
                    let _ = tr.read_proplist(Some(&mut pb));
                    self.clients[ci].props = pb;
                    if let Some(an) = self.clients[ci].props.get_str("application.name") {
                        self.clients[ci].name = an;
                    }
                    if let Some(ps) = self.clients[ci].props.get_str("application.process.id") {
                        if let Ok(p) = ps.trim().parse::<u32>() {
                            self.clients[ci].pid = p;
                        }
                    }
                }
                let bin = self.clients[ci]
                    .props
                    .get_str("application.process.binary")
                    .unwrap_or_else(|| "unknown".to_string());
                let tgt = if self.clients[ci].name.is_empty() {
                    "pa-client"
                } else {
                    &self.clients[ci].name
                };
                dbg_emit_vals(
                    LASAPI_DBG_PA,
                    LASAPI_DBG_INFO,
                    self.clients[ci].pid,
                    self.clients[ci].index,
                    self.clients[ci].version as i32,
                    0,
                    0,
                    "PA_CLIENT",
                    tgt,
                    &format!("pid={} bin={bin}", self.clients[ci].pid),
                );
                let mut tb = TagBuilder::begin_reply(tag, 32);
                if self.clients[ci].version >= 13 {
                    tb.put_u32(PA_TAG_U32, self.clients[ci].index);
                }
                self.clients[ci].queue_tb(tb);
                let cidx = self.clients[ci].index;
                self.broadcast_sub_event(SUB_EVENT_CLIENT, SUB_EVENT_NEW, cidx);
            }
            CMD_GET_SERVER_INFO => {
                self.refresh_device_cache();
                let uname = current_username();
                let hname = current_hostname();
                let def_sink = self.get_default_sink_pa_name();
                let def_src = self.get_default_source_pa_name();
                let rate = if self.cached_status.hw_sample_rate != 0 {
                    self.cached_status.hw_sample_rate
                } else {
                    48000
                };
                let ss = PaSampleSpec {
                    format: pa_wire::FMT_FLOAT32LE,
                    channels: 2,
                    rate,
                };
                let mp = std_channel_map(2);
                let mut tb = TagBuilder::begin_reply(tag, 512);
                tb.put_str(Some("PulseAudio (on Salsa LASAPI 1.0)"));
                tb.put_str(Some("15.0.0"));
                tb.put_str(Some(&uname));
                tb.put_str(Some(&hname));
                tb.put_ss(&ss);
                tb.put_str(Some(&def_sink));
                tb.put_str(Some(&def_src));
                tb.put_u32(PA_TAG_U32, PA_SERVER_COOKIE_ID);
                if self.clients[ci].version >= 15 {
                    tb.put_map(&mp);
                }
                self.clients[ci].queue_tb(tb);
            }
            CMD_STAT => {
                let mut tb = TagBuilder::begin_reply(tag, 64);
                for _ in 0..5 {
                    tb.put_u32(PA_TAG_U32, 0);
                }
                self.clients[ci].queue_tb(tb);
            }
            CMD_LOOKUP_SINK | CMD_LOOKUP_SOURCE => {
                let Some(nm_opt) = tr.read_str_slice() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let nm = nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                self.refresh_device_cache();
                let found = if cmd == CMD_LOOKUP_SINK {
                    self.resolve_sink_idx_and_name(PA_INVALID_INDEX, nm.as_deref())
                        .map(|(i, _)| i)
                } else {
                    self.resolve_source_idx_and_name(PA_INVALID_INDEX, nm.as_deref())
                        .map(|(i, _, _)| i)
                };
                match found {
                    Some(idx) if idx != PA_INVALID_INDEX => {
                        let mut tb = TagBuilder::begin_reply(tag, 32);
                        tb.put_u32(PA_TAG_U32, idx);
                        self.clients[ci].queue_tb(tb);
                    }
                    _ => self.clients[ci].reply_err(tag, ERR_NOENTITY),
                }
            }
            CMD_GET_SINK_INFO | CMD_GET_SINK_INFO_LIST => {
                self.refresh_device_cache();
                let mut req_idx = PA_INVALID_INDEX;
                let mut resolved_nm = String::new();
                if cmd == CMD_GET_SINK_INFO {
                    let (Some(ri), Some(nm_opt)) = (tr.read_u32(), tr.read_str_slice()) else {
                        self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                        return;
                    };
                    req_idx = ri;
                    if let Some((o, l)) = nm_opt {
                        if l > 0 {
                            let s = String::from_utf8_lossy(&buf[o..o + l]);
                            resolved_nm = self.resolve_pa_target_name(false, Some(&s));
                        }
                    }
                }
                let ver = self.clients[ci].version;
                let mut tb = TagBuilder::begin_reply(tag, 4096);
                let mut pb_cnt = 0u32;
                let mut added = 0u32;
                let ndev = self.cached_dev_count();
                for i in 0..ndev {
                    if self.cached_devices.devices[i].is_capture != 0 {
                        continue;
                    }
                    pb_cnt += 1;
                    let did = c_str_to_string(&self.cached_devices.devices[i].id);
                    let dname = c_str_to_string(&self.cached_devices.devices[i].name);
                    let is_bt = self.cached_devices.devices[i].is_bluetooth != 0;
                    let sidx = self.stable_pa_idx(PA_FAC_SINK, &did);
                    let mon_idx = self.stable_pa_idx(PA_FAC_SOURCE, &make_mon_key(&did));
                    let pa_n = make_pa_sink_name(&did);
                    let do_match = cmd == CMD_GET_SINK_INFO_LIST
                        || (req_idx != PA_INVALID_INDEX && req_idx == sidx)
                        || (!resolved_nm.is_empty() && resolved_nm == pa_n);
                    if do_match {
                        let card_idx = self.find_card_idx_for_id(&did);
                        let pavol = if self.sink_pa_vol_valid {
                            self.sink_pa_vol
                        } else {
                            PA_INVALID_INDEX
                        };
                        put_sink_entry(
                            &mut tb,
                            ver,
                            sidx,
                            mon_idx,
                            card_idx,
                            &pa_n,
                            &dname,
                            &did,
                            is_bt,
                            self.cached_status.hw_sample_rate,
                            self.cached_status.desktop_volume_pct,
                            pavol,
                            self.cached_status.desktop_muted != 0,
                        );
                        added += 1;
                    }
                }
                if pb_cnt == 0 {
                    let vsidx = self.stable_pa_idx(PA_FAC_SINK, "virtual");
                    let vmon = self.stable_pa_idx(PA_FAC_SOURCE, "mon:virtual");
                    if cmd == CMD_GET_SINK_INFO_LIST
                        || req_idx == vsidx
                        || resolved_nm == "salsa_out.virtual"
                    {
                        let pavol = if self.sink_pa_vol_valid {
                            self.sink_pa_vol
                        } else {
                            PA_INVALID_INDEX
                        };
                        put_sink_entry(
                            &mut tb,
                            ver,
                            vsidx,
                            vmon,
                            PA_INVALID_INDEX,
                            "salsa_out.virtual",
                            "Salsa Master Virtual Sink",
                            "virtual",
                            false,
                            self.cached_status.hw_sample_rate,
                            self.cached_status.desktop_volume_pct,
                            pavol,
                            self.cached_status.desktop_muted != 0,
                        );
                        added += 1;
                    }
                }
                if cmd == CMD_GET_SINK_INFO && added == 0 {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                } else {
                    self.clients[ci].queue_tb(tb);
                }
            }
            CMD_GET_SOURCE_INFO | CMD_GET_SOURCE_INFO_LIST => {
                self.refresh_device_cache();
                let mut req_idx = PA_INVALID_INDEX;
                let mut resolved_nm = String::new();
                if cmd == CMD_GET_SOURCE_INFO {
                    let (Some(ri), Some(nm_opt)) = (tr.read_u32(), tr.read_str_slice()) else {
                        self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                        return;
                    };
                    req_idx = ri;
                    if let Some((o, l)) = nm_opt {
                        if l > 0 {
                            let s = String::from_utf8_lossy(&buf[o..o + l]);
                            resolved_nm = self.resolve_pa_target_name(true, Some(&s));
                        }
                    }
                }
                let ver = self.clients[ci].version;
                let mut tb = TagBuilder::begin_reply(tag, 4096);
                let mut added = 0u32;
                let ndev = self.cached_dev_count();

                for i in 0..ndev {
                    if self.cached_devices.devices[i].is_capture == 0 {
                        continue;
                    }
                    let did = c_str_to_string(&self.cached_devices.devices[i].id);
                    let dname = c_str_to_string(&self.cached_devices.devices[i].name);
                    let src_idx = self.stable_pa_idx(PA_FAC_SOURCE, &did);
                    let pa_n = make_pa_source_name(&did);
                    let do_match = cmd == CMD_GET_SOURCE_INFO_LIST
                        || (req_idx != PA_INVALID_INDEX && req_idx == src_idx)
                        || (!resolved_nm.is_empty() && resolved_nm == pa_n);
                    if do_match {
                        let card_idx = self.find_card_idx_for_id(&did);
                        let cap_rate = self.cached_status.hw_analog_info.reserved;
                        let src_rate = if (8000..=384_000).contains(&cap_rate) {
                            cap_rate
                        } else {
                            self.cached_status.hw_sample_rate
                        };
                        let pavol = if self.hw_src_pa_vol_valid {
                            self.hw_src_pa_vol
                        } else {
                            PA_INVALID_INDEX
                        };
                        put_source_entry(
                            &mut tb,
                            ver,
                            src_idx,
                            card_idx,
                            &pa_n,
                            &dname,
                            Some(&did),
                            PA_INVALID_INDEX,
                            None,
                            src_rate,
                            self.cached_status.desktop_mic_volume_pct,
                            pavol,
                            self.cached_status.desktop_mic_muted != 0,
                        );
                        added += 1;
                    }
                }

                let mon_pct = rt.mon_vol_milli.load(Ordering::Relaxed) / 10;
                let mon_muted = rt.mon_muted.load(Ordering::Relaxed) != 0;
                let mut pb_cnt = 0u32;
                for i in 0..ndev {
                    if self.cached_devices.devices[i].is_capture != 0 {
                        continue;
                    }
                    pb_cnt += 1;
                    let did = c_str_to_string(&self.cached_devices.devices[i].id);
                    let dname = c_str_to_string(&self.cached_devices.devices[i].name);
                    let src_idx = self.stable_pa_idx(PA_FAC_SOURCE, &make_mon_key(&did));
                    let sidx = self.stable_pa_idx(PA_FAC_SINK, &did);
                    let sink_n = make_pa_sink_name(&did);
                    let mon_n = format!("{sink_n}.monitor");
                    let mon_desc = format!("Monitor of {dname}");
                    let do_match = cmd == CMD_GET_SOURCE_INFO_LIST
                        || (req_idx != PA_INVALID_INDEX && req_idx == src_idx)
                        || (!resolved_nm.is_empty() && resolved_nm == mon_n);
                    if do_match {
                        put_source_entry(
                            &mut tb,
                            ver,
                            src_idx,
                            PA_INVALID_INDEX,
                            &mon_n,
                            &mon_desc,
                            Some(&did),
                            sidx,
                            Some(&sink_n),
                            self.cached_status.hw_sample_rate,
                            mon_pct,
                            self.mon_pa_vol,
                            mon_muted,
                        );
                        added += 1;
                    }
                }
                if pb_cnt == 0 {
                    let vsidx = self.stable_pa_idx(PA_FAC_SINK, "virtual");
                    let vmon = self.stable_pa_idx(PA_FAC_SOURCE, "mon:virtual");
                    if cmd == CMD_GET_SOURCE_INFO_LIST
                        || req_idx == vmon
                        || resolved_nm == "salsa_out.virtual.monitor"
                    {
                        put_source_entry(
                            &mut tb,
                            ver,
                            vmon,
                            PA_INVALID_INDEX,
                            "salsa_out.virtual.monitor",
                            "Monitor of Salsa Master Virtual Sink",
                            Some("virtual"),
                            vsidx,
                            Some("salsa_out.virtual"),
                            self.cached_status.hw_sample_rate,
                            mon_pct,
                            self.mon_pa_vol,
                            mon_muted,
                        );
                        added += 1;
                    }
                }
                if cmd == CMD_GET_SOURCE_INFO && added == 0 {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                } else {
                    self.clients[ci].queue_tb(tb);
                }
            }
            CMD_GET_CARD_INFO | CMD_GET_CARD_INFO_LIST => {
                self.refresh_device_cache();
                let mut req_idx = PA_INVALID_INDEX;
                let mut req_nm: Option<String> = None;
                if cmd == CMD_GET_CARD_INFO {
                    let (Some(ri), Some(nm_opt)) = (tr.read_u32(), tr.read_str_slice()) else {
                        self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                        return;
                    };
                    req_idx = ri;
                    req_nm = nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                }
                let ver = self.clients[ci].version;
                let mut tb = TagBuilder::begin_reply(tag, 4096);
                let mut seen: Vec<String> = Vec::new();
                let mut added = 0u32;
                let ndev = self.cached_dev_count();
                for i in 0..ndev {
                    let did = c_str_to_string(&self.cached_devices.devices[i].id);
                    if seen.contains(&did) {
                        continue;
                    }
                    seen.push(did.clone());
                    let card_idx = self.stable_pa_idx(PA_FAC_CARD, &did);
                    let is_bt = self.cached_devices.devices[i].is_bluetooth != 0;
                    let suffix = if is_bt && did.len() >= 6 { &did[6..] } else { &did };
                    let do_match = cmd == CMD_GET_CARD_INFO_LIST
                        || (req_idx != PA_INVALID_INDEX && req_idx == card_idx)
                        || req_nm
                            .as_deref()
                            .map(|nm| nm.contains(suffix))
                            .unwrap_or(false);
                    if do_match {
                        let d_copy = self.cached_devices.devices[i];
                        put_card_entry(&mut tb, ver, card_idx, &self.cached_devices, &d_copy);
                        added += 1;
                    }
                }
                if cmd == CMD_GET_CARD_INFO && added == 0 {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                } else {
                    self.clients[ci].queue_tb(tb);
                }
            }
            CMD_GET_MODULE_INFO | CMD_GET_MODULE_INFO_LIST => {
                if cmd == CMD_GET_MODULE_INFO && tr.read_u32() != Some(0) {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                }
                let ver = self.clients[ci].version;
                let mut tb = TagBuilder::begin_reply(tag, 256);
                tb.put_u32(PA_TAG_U32, 0);
                tb.put_str(Some("module-salsa-lasapi"));
                tb.put_str(Some("backend=kernel_alsa+native_bluez"));
                tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
                if ver < 15 {
                    tb.put_bool(false);
                } else {
                    let mut pb = PaPropBag::default();
                    pb.set_kv("module.description", "Salsa LASAPI Direct Audio & BlueZ Server");
                    tb.put_proplist(Some(&pb));
                }
                self.clients[ci].queue_tb(tb);
            }
            CMD_GET_CLIENT_INFO | CMD_GET_CLIENT_INFO_LIST => {
                let mut req_idx = PA_INVALID_INDEX;
                if cmd == CMD_GET_CLIENT_INFO {
                    let Some(ri) = tr.read_u32() else {
                        self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                        return;
                    };
                    req_idx = ri;
                }
                let ver = self.clients[ci].version;
                let mut tb = TagBuilder::begin_reply(tag, 1024);
                let mut added = 0u32;
                for cl in &self.clients {
                    if !cl.in_use || !cl.authed {
                        continue;
                    }
                    if cmd == CMD_GET_CLIENT_INFO && cl.index != req_idx {
                        continue;
                    }
                    let nm = if cl.name.is_empty() {
                        "PulseAudio Client"
                    } else {
                        &cl.name
                    };
                    put_client_fields(&mut tb, ver, cl.index, nm, &cl.props);
                    added += 1;
                }
                for w in &self.lasapi_proxies {
                    if !w.active {
                        continue;
                    }
                    if cmd == CMD_GET_CLIENT_INFO && w.pa_client_idx != req_idx {
                        continue;
                    }
                    put_client_fields(&mut tb, ver, w.pa_client_idx, &w.client_name, &w.client_props);
                    added += 1;
                }
                if cmd == CMD_GET_CLIENT_INFO && added == 0 {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                } else {
                    self.clients[ci].queue_tb(tb);
                }
            }
            CMD_GET_SINK_INPUT_INFO | CMD_GET_SINK_INPUT_INFO_LIST => {
                let mut req_idx = PA_INVALID_INDEX;
                if cmd == CMD_GET_SINK_INPUT_INFO {
                    let Some(ri) = tr.read_u32() else {
                        self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                        return;
                    };
                    req_idx = ri;
                }
                let ver = self.clients[ci].version;
                let mut tb = TagBuilder::begin_reply(tag, 2048);
                let mut added = 0u32;
                for si in 0..PA_MAX_STREAMS {
                    let s = &self.streams[si];
                    if !s.in_use || !s.is_playback {
                        continue;
                    }
                    if cmd == CMD_GET_SINK_INPUT_INFO && s.index != req_idx {
                        continue;
                    }
                    put_sink_input_entry(
                        &mut tb,
                        ver,
                        s,
                        rt.streams[si].muted.load(Ordering::Relaxed) != 0,
                        rt.streams[si].corked.load(Ordering::Relaxed) != 0,
                    );
                    added += 1;
                }
                let active_sink = self.find_active_sink_idx();
                for (slot_idx, w) in self.lasapi_proxies.iter_mut().enumerate() {
                    if !w.active {
                        continue;
                    }
                    if cmd == CMD_GET_SINK_INPUT_INFO && w.pa_stream_idx != req_idx {
                        continue;
                    }
                    w.target_dev_idx = active_sink;
                    let (muted, corked) =
                        Self::sync_lasapi_proxy_vol_from_rt(w, &rt.lasapi_slots[slot_idx]);
                    let nm = if w.stream_name.is_empty() {
                        w.client_name.as_str()
                    } else {
                        w.stream_name.as_str()
                    };
                    put_sink_input_fields(
                        &mut tb,
                        ver,
                        w.pa_stream_idx,
                        nm,
                        w.pa_client_idx,
                        w.target_dev_idx,
                        &w.ss,
                        &w.map,
                        &w.vol,
                        &w.stream_props,
                        muted,
                        corked,
                    );
                    added += 1;
                }
                if cmd == CMD_GET_SINK_INPUT_INFO && added == 0 {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                } else {
                    self.clients[ci].queue_tb(tb);
                }
            }
            CMD_GET_SOURCE_OUTPUT_INFO | CMD_GET_SOURCE_OUTPUT_INFO_LIST => {
                let mut req_idx = PA_INVALID_INDEX;
                if cmd == CMD_GET_SOURCE_OUTPUT_INFO {
                    let Some(ri) = tr.read_u32() else {
                        self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                        return;
                    };
                    req_idx = ri;
                }
                let ver = self.clients[ci].version;
                let mut tb = TagBuilder::begin_reply(tag, 2048);
                let mut added = 0u32;
                for si in 0..PA_MAX_STREAMS {
                    let s = &self.streams[si];
                    if !s.in_use || s.is_playback {
                        continue;
                    }
                    if cmd == CMD_GET_SOURCE_OUTPUT_INFO && s.index != req_idx {
                        continue;
                    }
                    put_source_output_entry(
                        &mut tb,
                        ver,
                        s,
                        rt.streams[si].muted.load(Ordering::Relaxed) != 0,
                        rt.streams[si].corked.load(Ordering::Relaxed) != 0,
                    );
                    added += 1;
                }
                if cmd == CMD_GET_SOURCE_OUTPUT_INFO && added == 0 {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                } else {
                    self.clients[ci].queue_tb(tb);
                }
            }
            CMD_GET_SAMPLE_INFO_LIST => {
                let tb = TagBuilder::begin_reply(tag, 32);
                self.clients[ci].queue_tb(tb);
            }
            CMD_SUBSCRIBE => {
                let Some(mask) = tr.read_u32() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                self.clients[ci].sub_mask = mask & SUB_MASK_ALL;
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_SET_SINK_VOLUME | CMD_SET_SOURCE_VOLUME => {
                let (Some(idx), Some(nm_opt), Some(cv)) =
                    (tr.read_u32(), tr.read_str_slice(), tr.read_cvol())
                else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let nm = nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                let ch = cv.channels.min(32) as usize;
                let sum: u64 = cv.values[..ch].iter().map(|&v| v as u64).sum();
                let mut avg_pv = if ch > 0 {
                    (sum / ch as u64) as u32
                } else {
                    PA_VOLUME_NORM
                };
                avg_pv = avg_pv.min(PA_VOLUME_NORM * 2);
                let norm = (avg_pv as f64) / (PA_VOLUME_NORM as f64);
                let lin = norm * norm * norm;
                let mut gain_ppm = (lin * 1_000_000.0 + 0.5) as u32;
                if avg_pv > 0 && gain_ppm == 0 {
                    gain_ppm = 1;
                }
                gain_ppm = gain_ppm.min(2_000_000);
                let mut pct = (lin * 100.0 + 0.5) as i32;
                if avg_pv > 0 && pct == 0 {
                    pct = 1;
                }
                pct = pct.min(150);

                if cmd == CMD_SET_SOURCE_VOLUME {
                    let resolved = self.resolve_source_idx_and_name(idx, nm.as_deref());
                    let (tgt_idx, is_mon) = match resolved {
                        Some((ti, _, mon)) => (ti, mon),
                        None => {
                            if idx != PA_INVALID_INDEX
                                || nm.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                            {
                                self.clients[ci].reply_err(tag, ERR_NOENTITY);
                                return;
                            }
                            let ti = self.find_active_source_idx();
                            let asink = self.find_active_sink_idx();
                            let mon = self.find_monitor_src_idx_for_sink(asink) == ti;
                            (ti, mon)
                        }
                    };
                    if is_mon {
                        let mut milli = (lin * 1000.0 + 0.5) as u32;
                        if avg_pv > 0 && milli == 0 {
                            milli = 1;
                        }
                        self.mon_pa_vol = avg_pv;
                        rt.mon_vol_milli.store(milli.min(2000), Ordering::Relaxed);
                    } else {
                        self.hw_src_pa_vol = avg_pv;
                        self.hw_src_pa_vol_valid = true;
                        self.host.set_mic_gain_ppm(gain_ppm, pct, -1);
                        self.refresh_status_only();
                    }
                    self.clients[ci].reply_simple_ack(tag);
                    self.broadcast_sub_event(SUB_EVENT_SOURCE, SUB_EVENT_CHANGE, tgt_idx);
                } else {
                    let tgt_idx = self
                        .resolve_sink_idx_and_name(idx, nm.as_deref())
                        .map(|(ti, _)| ti)
                        .unwrap_or_else(|| self.find_active_sink_idx());
                    let cname = if self.clients[ci].name.is_empty() {
                        "pa-client"
                    } else {
                        &self.clients[ci].name
                    };
                    dbg_emit_vals(
                        LASAPI_DBG_PA,
                        LASAPI_DBG_INFO,
                        self.clients[ci].pid,
                        tgt_idx,
                        pct,
                        avg_pv as i32,
                        0,
                        "PA_SET_SINK_VOL",
                        cname,
                        &format!("sink=#{tgt_idx} pct={pct}% avg_pv={avg_pv} ch={ch}"),
                    );
                    self.sink_pa_vol = avg_pv;
                    self.sink_pa_vol_valid = true;
                    self.host.set_master_volume(false, pct, -1);
                    self.refresh_status_only();
                    self.broadcast_sub_event(SUB_EVENT_SINK, SUB_EVENT_CHANGE, tgt_idx);
                    self.clients[ci].reply_simple_ack(tag);
                }
            }
            CMD_SET_SINK_MUTE | CMD_SET_SOURCE_MUTE => {
                let (Some(idx), Some(nm_opt), Some(mute)) =
                    (tr.read_u32(), tr.read_str_slice(), tr.read_bool())
                else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let nm = nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                if cmd == CMD_SET_SOURCE_MUTE {
                    let resolved = self.resolve_source_idx_and_name(idx, nm.as_deref());
                    let (tgt_idx, is_mon) = match resolved {
                        Some((ti, _, mon)) => (ti, mon),
                        None => {
                            if idx != PA_INVALID_INDEX
                                || nm.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                            {
                                self.clients[ci].reply_err(tag, ERR_NOENTITY);
                                return;
                            }
                            let ti = self.find_active_source_idx();
                            let asink = self.find_active_sink_idx();
                            let mon = self.find_monitor_src_idx_for_sink(asink) == ti;
                            (ti, mon)
                        }
                    };
                    if is_mon {
                        rt.mon_muted.store(i32::from(mute), Ordering::Relaxed);
                    } else {
                        self.host.set_master_volume(true, -1, i32::from(mute));
                        self.refresh_status_only();
                    }
                    self.clients[ci].reply_simple_ack(tag);
                    self.broadcast_sub_event(SUB_EVENT_SOURCE, SUB_EVENT_CHANGE, tgt_idx);
                } else {
                    let tgt_idx = self
                        .resolve_sink_idx_and_name(idx, nm.as_deref())
                        .map(|(ti, _)| ti)
                        .unwrap_or_else(|| self.find_active_sink_idx());
                    self.host.set_master_volume(false, -1, i32::from(mute));
                    self.refresh_status_only();
                    self.broadcast_sub_event(SUB_EVENT_SINK, SUB_EVENT_CHANGE, tgt_idx);
                    self.clients[ci].reply_simple_ack(tag);
                }
            }
            CMD_SET_SINK_INPUT_VOLUME | CMD_SET_SOURCE_OUTPUT_VOLUME => {
                let (Some(idx), Some(cv)) = (tr.read_u32(), tr.read_cvol()) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                if let Some(si) = self.find_stream_by_index(idx) {
                    self.streams[si].vol = cv;
                    rt.streams[si]
                        .volume_milli
                        .store(cvol_to_milli(&cv), Ordering::Relaxed);
                    let is_pb = self.streams[si].is_playback;
                    self.record_app_volume(si, rt);
                    let (p, snap) = self.app_vols_snapshot();
                    write_app_volumes_disk(vol_lock, &p, &snap);
                    self.clients[ci].reply_simple_ack(tag);
                    self.broadcast_sub_event(
                        if is_pb {
                            SUB_EVENT_SINK_INPUT
                        } else {
                            SUB_EVENT_SOURCE_OUTPUT
                        },
                        SUB_EVENT_CHANGE,
                        idx,
                    );
                } else if cmd == CMD_SET_SINK_INPUT_VOLUME {
                    let Some(wi) = self.find_lasapi_proxy_by_stream_idx(idx) else {
                        self.clients[ci].reply_err(tag, ERR_NOENTITY);
                        return;
                    };
                    let milli = cvol_to_milli(&cv).min(1500);
                    self.lasapi_proxies[wi].vol = cv;
                    let ws = &rt.lasapi_slots[wi];
                    ws.volume_milli.store(milli, Ordering::Relaxed);
                    ws.pa_target_vol_milli.store(milli, Ordering::Relaxed);
                    ws.pa_target_muted
                        .store(ws.muted.load(Ordering::Relaxed), Ordering::Relaxed);
                    ws.pa_ctl_seq.fetch_add(1, Ordering::Release);
                    self.clients[ci].reply_simple_ack(tag);
                    self.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_CHANGE, idx);
                } else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                }
            }
            CMD_SET_SINK_INPUT_MUTE | CMD_SET_SOURCE_OUTPUT_MUTE => {
                let (Some(idx), Some(mute)) = (tr.read_u32(), tr.read_bool()) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                if let Some(si) = self.find_stream_by_index(idx) {
                    rt.streams[si]
                        .muted
                        .store(u32::from(mute), Ordering::Relaxed);
                    let is_pb = self.streams[si].is_playback;
                    self.record_app_volume(si, rt);
                    let (p, snap) = self.app_vols_snapshot();
                    write_app_volumes_disk(vol_lock, &p, &snap);
                    self.clients[ci].reply_simple_ack(tag);
                    self.broadcast_sub_event(
                        if is_pb {
                            SUB_EVENT_SINK_INPUT
                        } else {
                            SUB_EVENT_SOURCE_OUTPUT
                        },
                        SUB_EVENT_CHANGE,
                        idx,
                    );
                } else if cmd == CMD_SET_SINK_INPUT_MUTE {
                    let Some(wi) = self.find_lasapi_proxy_by_stream_idx(idx) else {
                        self.clients[ci].reply_err(tag, ERR_NOENTITY);
                        return;
                    };
                    let ws = &rt.lasapi_slots[wi];
                    ws.muted.store(mute, Ordering::Relaxed);
                    ws.pa_target_vol_milli
                        .store(ws.volume_milli.load(Ordering::Relaxed), Ordering::Relaxed);
                    ws.pa_target_muted.store(mute, Ordering::Relaxed);
                    ws.pa_ctl_seq.fetch_add(1, Ordering::Release);
                    self.clients[ci].reply_simple_ack(tag);
                    self.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_CHANGE, idx);
                } else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                }
            }
            CMD_SET_DEFAULT_SINK | CMD_SET_DEFAULT_SOURCE => {
                let Some(nm_opt) = tr.read_str_slice() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let nm = nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                self.refresh_status_only();
                let is_cap = cmd == CMD_SET_DEFAULT_SOURCE;
                if let Some(ref name) = nm {
                    if name != "@NONE@" {
                        let mut dev_id = self.pa_name_to_salsa_id(name, is_cap);
                        if dev_id.is_none() {
                            self.refresh_device_cache_force();
                            dev_id = self.pa_name_to_salsa_id(name, is_cap);
                        }
                        let Some(did) = dev_id else {
                            if is_cap && name.contains(".monitor") {
                                self.clients[ci].reply_simple_ack(tag);
                                return;
                            }
                            self.clients[ci].reply_err(tag, ERR_NOENTITY);
                            return;
                        };
                        self.host.switch_devices(
                            if is_cap { None } else { Some(&did) },
                            if is_cap { Some(&did) } else { None },
                        );
                        self.refresh_status_only();
                    }
                }
                self.clients[ci].reply_simple_ack(tag);
                self.broadcast_sub_event(SUB_EVENT_SERVER, SUB_EVENT_CHANGE, PA_INVALID_INDEX);
                let act_idx = if is_cap {
                    self.find_active_source_idx()
                } else {
                    self.find_active_sink_idx()
                };
                self.broadcast_sub_event(
                    if is_cap {
                        SUB_EVENT_SOURCE
                    } else {
                        SUB_EVENT_SINK
                    },
                    SUB_EVENT_CHANGE,
                    act_idx,
                );
            }
            CMD_SET_CARD_PROFILE
            | CMD_SET_SINK_PORT
            | CMD_SET_SOURCE_PORT
            | CMD_SUSPEND_SINK
            | CMD_SUSPEND_SOURCE
            | CMD_SET_PORT_LATENCY_OFFSET => {
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_CREATE_PLAYBACK_STREAM => {
                let ver = self.clients[ci].version;
                let mut sname: Option<String> = None;
                if ver < 13 {
                    if let Some(Some((o, l))) = tr.read_str_slice() {
                        sname = Some(String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                    }
                }
                let (
                    Some(mut ss),
                    Some(mut mp),
                    Some(sink_idx),
                    Some(sink_nm_opt),
                    Some(maxl),
                    Some(corked),
                    Some(tl),
                    Some(preb),
                    Some(minr),
                    Some(_syncid),
                    Some(cv),
                ) = (
                    tr.read_ss(),
                    tr.read_map(),
                    tr.read_u32(),
                    tr.read_str_slice(),
                    tr.read_u32(),
                    tr.read_bool(),
                    tr.read_u32(),
                    tr.read_u32(),
                    tr.read_u32(),
                    tr.read_u32(),
                    tr.read_cvol(),
                )
                else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let sink_nm =
                    sink_nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                let mut pb = PaPropBag::default();
                let mut muted = false;
                let mut adjust_latency = false;
                let mut volume_set = true;

                if ver >= 12 {
                    for _ in 0..7 {
                        let _ = tr.read_bool();
                    }
                }
                if ver >= 13 {
                    if let Some(m) = tr.read_bool() {
                        muted = m;
                    }
                    if let Some(al) = tr.read_bool() {
                        adjust_latency = al;
                    }
                    let _ = tr.read_proplist(Some(&mut pb));
                }
                if ver >= 14 {
                    if let Some(vs) = tr.read_bool() {
                        volume_set = vs;
                    }
                    let _ = tr.read_bool();
                }
                if ver >= 15 {
                    let muted_set = tr.read_bool().unwrap_or(false);
                    if !muted_set {
                        muted = false;
                    }
                    let _ = tr.read_bool();
                    let _ = tr.read_bool();
                }
                if ver >= 17 {
                    let _ = tr.read_bool();
                }
                if ver >= 18 {
                    let _ = tr.read_bool();
                }
                self.refresh_device_cache();
                let hw_rate = self.cached_status.hw_sample_rate;
                let mut req_formats: Vec<PaFormatInfo> = Vec::new();
                if ver >= 21 {
                    if let Some(nf) = tr.read_u8() {
                        for _ in 0..nf {
                            if let Some(fi) = tr.read_format_info(hw_rate) {
                                req_formats.push(fi);
                            }
                        }
                    }
                }
                if let Err(err_code) =
                    resolve_stream_format_spec(&mut ss, &mut mp, &req_formats, hw_rate)
                {
                    self.clients[ci].reply_err(tag, err_code);
                    return;
                }

                let (actual_sink_idx, actual_sink_nm) = self
                    .resolve_sink_idx_and_name(sink_idx, sink_nm.as_deref())
                    .unwrap_or_else(|| {
                        (self.find_active_sink_idx(), self.get_default_sink_pa_name())
                    });

                let Some(si) = self.alloc_stream(ci, rt) else {
                    self.clients[ci].reply_err(tag, ERR_INVALID);
                    return;
                };
                let frame_size = (format_sample_size(ss.format) * ss.channels as u32).max(1);
                let mut eff_tl = tl;
                if adjust_latency && eff_tl != PA_INVALID_INDEX && eff_tl > 0 {
                    let sink_bytes =
                        ((5000u64 * ss.rate as u64) / 1_000_000u64) * frame_size as u64;
                    let mut floor_bytes = ((ss.rate as u64 * 30) / 1000) * frame_size as u64;
                    if minr != PA_INVALID_INDEX && minr > 0 {
                        floor_bytes = floor_bytes.max((minr as u64).saturating_mul(3));
                    }
                    if (eff_tl as u64) > sink_bytes + floor_bytes {
                        eff_tl = (eff_tl as u64 - sink_bytes) as u32;
                    }
                }

                let c_pid = self.clients[ci].pid;
                let c_name = self.clients[ci].name.clone();
                let c_bin = self.clients[ci].props.get_str("application.process.binary");
                let c_app = self.clients[ci].props.get_str("application.name");

                let sm = &mut self.streams[si];
                sm.is_playback = true;
                sm.target_dev_idx = actual_sink_idx;
                sm.ss = ss;
                sm.map = mp;
                sm.props = pb;
                if c_pid > 0 && sm.props.get_str("application.process.id").is_none() {
                    sm.props.set_kv("application.process.id", &c_pid.to_string());
                }
                if sm.props.get_str("application.name").is_none() {
                    if let Some(ref an) = c_app {
                        sm.props.set_kv("application.name", an);
                    } else if !c_name.is_empty() {
                        sm.props.set_kv("application.name", &c_name);
                    }
                }
                if sm.props.get_str("application.process.binary").is_none() {
                    if let Some(ref bn) = c_bin {
                        sm.props.set_kv("application.process.binary", bn);
                    }
                }
                sm.name = sm
                    .props
                    .get_str("media.name")
                    .or(sname)
                    .unwrap_or(c_name);
                sm.frame_size = frame_size;
                enforce_playback_attr(sm, maxl, eff_tl, minr, preb);

                if volume_set && cv.channels > 0 {
                    let norm_cv = normalize_stream_cvol(&cv, ss.channels);
                    self.streams[si].vol = norm_cv;
                    rt.streams[si]
                        .volume_milli
                        .store(cvol_to_milli(&norm_cv), Ordering::Relaxed);
                } else {
                    self.streams[si].vol.channels = ss.channels;
                    for v in &mut self.streams[si].vol.values[..ss.channels as usize] {
                        *v = PA_VOLUME_NORM;
                    }
                    rt.streams[si].volume_milli.store(1000, Ordering::Relaxed);
                }
                let preb_flag = u32::from(self.streams[si].prebuf > 0);
                let rs = &rt.streams[si];
                rs.is_playback.store(true, Ordering::Relaxed);
                rs.is_monitor.store(false, Ordering::Relaxed);
                rs.peak_detect.store(false, Ordering::Relaxed);
                rs.direct_on_input
                    .store(PA_INVALID_INDEX, Ordering::Relaxed);
                rs.fmt.store(ss.format, Ordering::Relaxed);
                rs.frame_size.store(frame_size, Ordering::Relaxed);
                rs.tlength
                    .store(self.streams[si].tlength, Ordering::Relaxed);
                rs.prebuf.store(self.streams[si].prebuf, Ordering::Relaxed);
                rs.in_prebuf.store(preb_flag, Ordering::Release);
                rs.underrun_baseline_bytes.store(0, Ordering::Relaxed);
                rs.muted.store(u32::from(muted), Ordering::Relaxed);
                rs.corked.store(u32::from(corked), Ordering::Relaxed);
                rs.sample_rate_atomic.store(ss.rate, Ordering::Relaxed);

                self.stream_update_device_id(si);
                self.apply_saved_app_volume(si, volume_set, rt);
                rt.streams[si].active.store(1, Ordering::Release);

                let sm = &mut self.streams[si];
                sm.requested_bytes_total = sm.tlength as u64;
                let mut tb = TagBuilder::begin_reply(tag, 384);
                tb.put_u32(PA_TAG_U32, sm.channel);
                tb.put_u32(PA_TAG_U32, sm.index);
                tb.put_u32(PA_TAG_U32, sm.tlength);
                if ver >= 9 {
                    tb.put_u32(PA_TAG_U32, sm.maxlength);
                    tb.put_u32(PA_TAG_U32, sm.tlength);
                    tb.put_u32(PA_TAG_U32, sm.prebuf);
                    tb.put_u32(PA_TAG_U32, sm.minreq);
                }
                if ver >= 12 {
                    tb.put_ss(&sm.ss);
                    tb.put_map(&sm.map);
                    tb.put_u32(PA_TAG_U32, actual_sink_idx);
                    tb.put_str(Some(&actual_sink_nm));
                    tb.put_bool(false);
                }
                if ver >= 13 {
                    tb.put_u64_raw(PA_TAG_USEC, 5000);
                }
                if ver >= 21 {
                    tb.put_pcm_format_spec(&sm.ss, &sm.map);
                }
                let sidx = sm.index;
                let srate = sm.ss.rate;
                let stl = sm.tlength;
                let s_ch = sm.ss.channels;
                let s_fmt = sm.ss.format;
                let s_prebuf = sm.prebuf;
                let s_minreq = sm.minreq;
                let send_initial_started = !corked && s_prebuf == 0;
                let s_name = if sm.name.is_empty() {
                    self.clients[ci].name.clone()
                } else {
                    sm.name.clone()
                };
                self.clients[ci].queue_tb(tb);
                if send_initial_started {
                    self.send_stream_started(si);
                }

                dbg_emit_vals(
                    LASAPI_DBG_PA,
                    LASAPI_DBG_INFO,
                    self.clients[ci].pid,
                    10000 + sidx,
                    srate as i32,
                    stl as i32,
                    i32::from(corked),
                    "PA_NEW_PLAY",
                    &s_name,
                    &format!(
                        "sink={actual_sink_nm} ch={s_ch} fmt={s_fmt} prebuf={s_prebuf} minreq={s_minreq}"
                    ),
                );
                self.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_NEW, sidx);
            }
            CMD_CREATE_RECORD_STREAM => {
                let ver = self.clients[ci].version;
                let mut sname: Option<String> = None;
                if ver < 13 {
                    if let Some(Some((o, l))) = tr.read_str_slice() {
                        sname = Some(String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                    }
                }
                let (
                    Some(mut ss),
                    Some(mut mp),
                    Some(src_idx),
                    Some(src_nm_opt),
                    Some(maxl),
                    Some(corked),
                    Some(frags),
                ) = (
                    tr.read_ss(),
                    tr.read_map(),
                    tr.read_u32(),
                    tr.read_str_slice(),
                    tr.read_u32(),
                    tr.read_bool(),
                    tr.read_u32(),
                )
                else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let src_nm =
                    src_nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                let mut pb = PaPropBag::default();
                let mut peak_detect = false;
                let mut doi = PA_INVALID_INDEX;
                let mut cv = PaCVolume::default();
                let mut muted = false;
                let mut volume_set = false;

                if ver >= 12 {
                    for _ in 0..7 {
                        let _ = tr.read_bool();
                    }
                }
                if ver >= 13 {
                    if let Some(pd) = tr.read_bool() {
                        peak_detect = pd;
                    }
                    let _ = tr.read_bool();
                    let _ = tr.read_proplist(Some(&mut pb));
                    doi = tr.read_u32().unwrap_or(PA_INVALID_INDEX);
                }
                if ver >= 14 {
                    let _ = tr.read_bool();
                }
                if ver >= 15 {
                    let _ = tr.read_bool();
                    let _ = tr.read_bool();
                }
                self.refresh_device_cache();
                let hw_rate = self.cached_status.hw_sample_rate;
                let mut req_formats: Vec<PaFormatInfo> = Vec::new();
                if ver >= 22 {
                    if let Some(nf) = tr.read_u8() {
                        for _ in 0..nf {
                            if let Some(fi) = tr.read_format_info(hw_rate) {
                                req_formats.push(fi);
                            }
                        }
                    }
                    if let Some(cvol_in) = tr.read_cvol() {
                        cv = cvol_in;
                        muted = tr.read_bool().unwrap_or(false);
                        volume_set = tr.read_bool().unwrap_or(false);
                        let muted_set = tr.read_bool().unwrap_or(false);
                        let _ = tr.read_bool();
                        let _ = tr.read_bool();
                        if !muted_set {
                            muted = false;
                        }
                    }
                }
                if let Err(err_code) =
                    resolve_stream_format_spec(&mut ss, &mut mp, &req_formats, hw_rate)
                {
                    self.clients[ci].reply_err(tag, err_code);
                    return;
                }

                let (actual_src_idx, actual_src_nm, is_mon) = if doi != PA_INVALID_INDEX {
                    let tgt_dev = if let Some(target_si) = self.find_stream_by_index(doi) {
                        if !self.streams[target_si].is_playback {
                            self.clients[ci].reply_err(tag, ERR_NOENTITY);
                            return;
                        }
                        self.streams[target_si].target_dev_idx
                    } else if let Some(target_wi) = self.find_lasapi_proxy_by_stream_idx(doi) {
                        let d = self.lasapi_proxies[target_wi].target_dev_idx;
                        if d != PA_INVALID_INDEX {
                            d
                        } else {
                            self.find_active_sink_idx()
                        }
                    } else {
                        self.clients[ci].reply_err(tag, ERR_NOENTITY);
                        return;
                    };
                    let mut mon_idx = self.find_monitor_src_idx_for_sink(tgt_dev);
                    if mon_idx == PA_INVALID_INDEX {
                        mon_idx = self.stable_pa_idx(PA_FAC_SOURCE, "mon:virtual");
                    }
                    self.resolve_source_idx_and_name(mon_idx, None)
                        .map(|(idx, nm, _)| (idx, nm, true))
                        .unwrap_or_else(|| {
                            (
                                mon_idx,
                                format!("{}.monitor", self.get_default_sink_pa_name()),
                                true,
                            )
                        })
                } else {
                    match self.resolve_source_idx_and_name(src_idx, src_nm.as_deref()) {
                        Some(res) => res,
                        None => {
                            if src_idx == PA_INVALID_INDEX
                                && src_nm.as_deref().map(|s| s.is_empty()).unwrap_or(true)
                            {
                                let aidx = self.find_active_source_idx();
                                let anm = self.get_default_source_pa_name();
                                let im = anm.contains(".monitor");
                                (aidx, anm, im)
                            } else if src_nm.as_deref().is_some_and(|s| s.ends_with(".monitor")) {
                                let mon_nm = format!("{}.monitor", self.get_default_sink_pa_name());
                                self.resolve_source_idx_and_name(PA_INVALID_INDEX, Some(&mon_nm))
                                    .unwrap_or((self.find_active_source_idx(), mon_nm, true))
                            } else if src_nm.as_deref().is_some_and(|s| s.starts_with("salsa_in.")) {
                                let anm = self.get_default_source_pa_name();
                                let im = anm.contains(".monitor");
                                self.resolve_source_idx_and_name(PA_INVALID_INDEX, Some(&anm))
                                    .unwrap_or((self.find_active_source_idx(), anm, im))
                            } else {
                                self.clients[ci].reply_err(tag, ERR_NOENTITY);
                                return;
                            }
                        }
                    }
                };

                let explicit_req = doi == PA_INVALID_INDEX
                    && (src_idx != PA_INVALID_INDEX
                        || src_nm
                            .as_deref()
                            .map(|s| !s.is_empty() && !s.starts_with("@DEFAULT_"))
                            .unwrap_or(false));
                self.sync_active_capture_if_explicit(actual_src_idx, is_mon, peak_detect, explicit_req);

                let Some(si) = self.alloc_stream(ci, rt) else {
                    self.clients[ci].reply_err(tag, ERR_INVALID);
                    return;
                };
                let frame_size = (format_sample_size(ss.format) * ss.channels as u32).max(1);
                let sm = &mut self.streams[si];
                sm.is_playback = false;
                sm.is_monitor = is_mon;
                sm.peak_detect = peak_detect;
                sm.direct_on_input = doi;
                sm.target_dev_idx = actual_src_idx;
                sm.ss = ss;
                sm.map = mp;
                sm.props = pb;
                sm.name = sm
                    .props
                    .get_str("media.name")
                    .or(sname)
                    .unwrap_or_else(|| self.clients[ci].name.clone());
                sm.frame_size = frame_size;
                let _ = maxl;
                sm.maxlength = (PA_STREAM_RING_FRAMES as u32)
                    .saturating_mul(frame_size)
                    .min(PA_DEFAULT_MAXLENGTH);
                enforce_record_attr(sm, frags);

                if volume_set && cv.channels > 0 {
                    let norm_cv = normalize_stream_cvol(&cv, ss.channels);
                    self.streams[si].vol = norm_cv;
                    rt.streams[si]
                        .volume_milli
                        .store(cvol_to_milli(&norm_cv), Ordering::Relaxed);
                } else {
                    self.streams[si].vol.channels = ss.channels;
                    for v in &mut self.streams[si].vol.values[..ss.channels as usize] {
                        *v = PA_VOLUME_NORM;
                    }
                    rt.streams[si].volume_milli.store(1000, Ordering::Relaxed);
                }
                let rs = &rt.streams[si];
                rs.is_playback.store(false, Ordering::Relaxed);
                rs.is_monitor.store(is_mon, Ordering::Relaxed);
                rs.peak_detect.store(peak_detect, Ordering::Relaxed);
                rs.direct_on_input.store(doi, Ordering::Relaxed);
                rt.sync_active_doi();
                rs.fmt.store(ss.format, Ordering::Relaxed);
                rs.frame_size.store(frame_size, Ordering::Relaxed);
                rs.fragsize
                    .store(self.streams[si].fragsize, Ordering::Relaxed);
                rs.muted.store(u32::from(muted), Ordering::Relaxed);
                rs.corked.store(u32::from(corked), Ordering::Relaxed);
                rs.sample_rate_atomic.store(ss.rate, Ordering::Relaxed);

                self.stream_update_device_id(si);
                self.apply_saved_app_volume(si, volume_set, rt);
                rt.streams[si].active.store(1, Ordering::Release);

                let sm = &self.streams[si];
                let mut tb = TagBuilder::begin_reply(tag, 384);
                tb.put_u32(PA_TAG_U32, sm.channel);
                tb.put_u32(PA_TAG_U32, sm.index);
                if ver >= 9 {
                    tb.put_u32(PA_TAG_U32, sm.maxlength);
                    tb.put_u32(PA_TAG_U32, sm.fragsize);
                }
                if ver >= 12 {
                    tb.put_ss(&sm.ss);
                    tb.put_map(&sm.map);
                    tb.put_u32(PA_TAG_U32, actual_src_idx);
                    tb.put_str(Some(&actual_src_nm));
                    tb.put_bool(false);
                }
                if ver >= 13 {
                    tb.put_u64_raw(PA_TAG_USEC, 5000);
                }
                if ver >= 22 {
                    tb.put_pcm_format_spec(&sm.ss, &sm.map);
                }
                let sidx = sm.index;
                let s_ch = sm.ss.channels;
                let s_rate = sm.ss.rate;
                let s_frag = sm.fragsize;
                let s_name = if sm.name.is_empty() {
                    self.clients[ci].name.clone()
                } else {
                    sm.name.clone()
                };
                self.clients[ci].queue_tb(tb);

                dbg_emit_vals(
                    LASAPI_DBG_PA,
                    LASAPI_DBG_INFO,
                    self.clients[ci].pid,
                    10000 + sidx,
                    s_rate as i32,
                    s_frag as i32,
                    i32::from(is_mon),
                    "PA_NEW_REC",
                    &s_name,
                    &format!(
                        "src={actual_src_nm} mon={is_mon} peak={peak_detect} doi={doi} ch={s_ch} cork={corked}"
                    ),
                );
                self.broadcast_sub_event(SUB_EVENT_SOURCE_OUTPUT, SUB_EVENT_NEW, sidx);
            }
            CMD_DELETE_PLAYBACK_STREAM | CMD_DELETE_RECORD_STREAM => {
                let Some(ch) = tr.read_u32() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                let sidx = self.streams[si].index;
                let is_pb = self.streams[si].is_playback;
                let did = self.streams[si].device_id.clone();
                let nm = if self.streams[si].name.is_empty() {
                    self.clients[ci].name.clone()
                } else {
                    self.streams[si].name.clone()
                };
                dbg_emit_vals(
                    LASAPI_DBG_PA,
                    LASAPI_DBG_INFO,
                    self.clients[ci].pid,
                    10000 + sidx,
                    i32::from(is_pb),
                    ch as i32,
                    0,
                    "PA_DEL_STREAM",
                    &nm,
                    &format!("{} dev={did}", if is_pb { "playback" } else { "record" }),
                );
                self.free_stream(si, rt);
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_CORK_PLAYBACK_STREAM | CMD_CORK_RECORD_STREAM => {
                let (Some(ch), Some(cork)) = (tr.read_u32(), tr.read_bool()) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                let is_pb = self.streams[si].is_playback;
                if is_pb {
                    if cork {
                        self.streams[si].started_sent = false;
                    } else {
                        let rs = &rt.streams[si];
                        let wr = rs.wr_frame.load(Ordering::Acquire);
                        let rd = rs.rd_frame.load(Ordering::Acquire);
                        let fsz = self.streams[si].frame_size as u64;
                        let queued_bytes = wr.saturating_sub(rd) * fsz;
                        let preb = self.streams[si].prebuf as u64;
                        if preb > 0 && queued_bytes < preb {
                            rs.in_prebuf.store(1, Ordering::Release);
                            self.streams[si].started_sent = false;
                        }
                    }
                }
                rt.streams[si]
                    .corked
                    .store(u32::from(cork), Ordering::Relaxed);
                let sidx = self.streams[si].index;
                let nm = if self.streams[si].name.is_empty() {
                    self.clients[ci].name.clone()
                } else {
                    self.streams[si].name.clone()
                };
                dbg_emit_vals(
                    LASAPI_DBG_PA,
                    LASAPI_DBG_INFO,
                    self.clients[ci].pid,
                    10000 + sidx,
                    i32::from(cork),
                    i32::from(is_pb),
                    0,
                    "PA_CORK",
                    &nm,
                    &format!(
                        "corked={} ({})",
                        i32::from(cork),
                        if is_pb { "playback" } else { "record" }
                    ),
                );
                self.clients[ci].reply_simple_ack(tag);
                if is_pb && !cork && rt.streams[si].draining.load(Ordering::Relaxed) == 0 {
                    if rt.streams[si].in_prebuf.load(Ordering::Acquire) == 0 {
                        self.send_stream_started(si);
                    }
                    let rs = &rt.streams[si];
                    let wr = rs.wr_frame.load(Ordering::Acquire);
                    let rd = rs.rd_frame.load(Ordering::Acquire);
                    let fsz = self.streams[si].frame_size as u64;
                    let queued_bytes = wr.saturating_sub(rd) * fsz;
                    let wi = rs.write_index.load(Ordering::Relaxed);
                    let ack_pending = self.streams[si].requested_bytes_total.saturating_sub(wi);
                    let tl = self.streams[si].tlength as u64;
                    if queued_bytes + ack_pending < tl {
                        let need = (tl - (queued_bytes + ack_pending)) as u32;
                        if need >= self.streams[si].minreq
                            || (rs.in_prebuf.load(Ordering::Acquire) != 0
                                && queued_bytes + ack_pending < (self.streams[si].prebuf as u64))
                        {
                            self.send_stream_request(si, need);
                        }
                    }
                }
                self.broadcast_sub_event(
                    if is_pb {
                        SUB_EVENT_SINK_INPUT
                    } else {
                        SUB_EVENT_SOURCE_OUTPUT
                    },
                    SUB_EVENT_CHANGE,
                    sidx,
                );
            }
            CMD_FLUSH_PLAYBACK_STREAM | CMD_FLUSH_RECORD_STREAM => {
                let Some(ch) = tr.read_u32() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                let rs = &rt.streams[si];
                let wr = rs.wr_frame.load(Ordering::Relaxed);
                let wi = rs.write_index.load(Ordering::Relaxed);
                rs.rd_frame.store(wr, Ordering::Relaxed);
                rs.read_index.store(wi, Ordering::Relaxed);
                rs.flush_seq.fetch_add(1, Ordering::Release);
                if self.streams[si].is_playback {
                    rs.in_prebuf
                        .store(u32::from(self.streams[si].prebuf > 0), Ordering::Release);
                    rs.underflow_resync.store(false, Ordering::Release);
                    self.streams[si].started_sent = false;
                    rs.underrun_baseline_bytes.store(wi, Ordering::Relaxed);
                }
                self.clients[ci].reply_simple_ack(tag);
                if self.streams[si].is_playback {
                    self.streams[si].requested_bytes_total = wi;
                    let tl = self.streams[si].tlength;
                    self.send_stream_request(si, tl);
                }
            }
            CMD_TRIGGER_PLAYBACK_STREAM | CMD_PREBUF_PLAYBACK_STREAM => {
                let Some(ch) = tr.read_u32() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                rt.streams[si].in_prebuf.store(0, Ordering::Release);
                self.clients[ci].reply_simple_ack(tag);
                self.send_stream_started(si);
            }
            CMD_DRAIN_PLAYBACK_STREAM => {
                let Some(ch) = tr.read_u32() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                let rs = &rt.streams[si];
                rs.in_prebuf.store(0, Ordering::Release);
                let wr = rs.wr_frame.load(Ordering::Relaxed);
                let rd = rs.rd_frame.load(Ordering::Relaxed);
                if rd >= wr {
                    self.clients[ci].reply_simple_ack(tag);
                } else {
                    self.streams[si].drain_tag = tag;
                    rs.draining.store(1, Ordering::Relaxed);
                }
            }
            CMD_GET_PLAYBACK_LATENCY | CMD_GET_RECORD_LATENCY => {
                let (Some(ch), Some((ts_s, ts_u))) = (tr.read_u32(), tr.read_timeval()) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                let (now_s, now_u) = realtime_sec_usec();
                let rs = &rt.streams[si];
                let wi = rs.write_index.load(Ordering::Acquire);
                let ri = rs.read_index.load(Ordering::Acquire).min(wi);
                let mut tb = TagBuilder::begin_reply(tag, 128);
                if cmd == CMD_GET_PLAYBACK_LATENCY {
                    tb.put_u64_raw(PA_TAG_USEC, 5000);
                    tb.put_u64_raw(PA_TAG_USEC, 0);
                    let playing = rs.corked.load(Ordering::Relaxed) == 0
                        && rs.in_prebuf.load(Ordering::Acquire) == 0;
                    tb.put_bool(playing);
                    tb.put_timeval(ts_s, ts_u);
                    tb.put_timeval(now_s, now_u);
                    tb.put_u64_raw(PA_TAG_S64, wi);
                    tb.put_u64_raw(PA_TAG_S64, ri);
                    if self.clients[ci].version >= 13 {
                        let base = rs.underrun_baseline_bytes.load(Ordering::Relaxed);
                        let playing_bytes = ri.saturating_sub(base);
                        tb.put_u64_raw(PA_TAG_U64, 0);
                        tb.put_u64_raw(PA_TAG_U64, playing_bytes);
                    }
                } else {
                    tb.put_u64_raw(PA_TAG_USEC, 0);
                    tb.put_u64_raw(PA_TAG_USEC, 2000);
                    tb.put_bool(rs.corked.load(Ordering::Relaxed) == 0);
                    tb.put_timeval(ts_s, ts_u);
                    tb.put_timeval(now_s, now_u);
                    tb.put_u64_raw(PA_TAG_S64, wi);
                    tb.put_u64_raw(PA_TAG_S64, ri);
                }
                self.clients[ci].queue_tb(tb);
            }
            CMD_SET_PLAYBACK_STREAM_BUFFER_ATTR => {
                let (Some(ch), Some(maxl), Some(tl), Some(preb), Some(minr)) = (
                    tr.read_u32(),
                    tr.read_u32(),
                    tr.read_u32(),
                    tr.read_u32(),
                    tr.read_u32(),
                ) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                let sm = &mut self.streams[si];
                let nm = if maxl == PA_INVALID_INDEX { sm.maxlength } else { maxl };
                let nt = if tl == PA_INVALID_INDEX { sm.tlength } else { tl };
                let nmn = if minr == PA_INVALID_INDEX { sm.minreq } else { minr };
                let npr = if preb == PA_INVALID_INDEX { sm.prebuf } else { preb };
                enforce_playback_attr(sm, nm, nt, nmn, npr);
                rt.streams[si].tlength.store(sm.tlength, Ordering::Relaxed);
                rt.streams[si].prebuf.store(sm.prebuf, Ordering::Relaxed);
                let mut tb = TagBuilder::begin_reply(tag, 64);
                tb.put_u32(PA_TAG_U32, sm.maxlength);
                tb.put_u32(PA_TAG_U32, sm.tlength);
                tb.put_u32(PA_TAG_U32, sm.prebuf);
                tb.put_u32(PA_TAG_U32, sm.minreq);
                if self.clients[ci].version >= 13 {
                    tb.put_u64_raw(PA_TAG_USEC, 5000);
                }
                self.clients[ci].queue_tb(tb);
            }
            CMD_SET_RECORD_STREAM_BUFFER_ATTR => {
                let (Some(ch), Some(_maxl), Some(frags)) =
                    (tr.read_u32(), tr.read_u32(), tr.read_u32())
                else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                let sm = &mut self.streams[si];
                let nf = if frags == PA_INVALID_INDEX { sm.fragsize } else { frags };
                enforce_record_attr(sm, nf);
                rt.streams[si].fragsize.store(sm.fragsize, Ordering::Relaxed);
                let mut tb = TagBuilder::begin_reply(tag, 48);
                tb.put_u32(PA_TAG_U32, sm.maxlength);
                tb.put_u32(PA_TAG_U32, sm.fragsize);
                if self.clients[ci].version >= 13 {
                    tb.put_u64_raw(PA_TAG_USEC, 2000);
                }
                self.clients[ci].queue_tb(tb);
            }
            CMD_UPDATE_PLAYBACK_STREAM_SAMPLE_RATE | CMD_UPDATE_RECORD_STREAM_SAMPLE_RATE => {
                let (Some(ch), Some(rate)) = (tr.read_u32(), tr.read_u32()) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let Some(si) = self.find_client_stream(ci, ch) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                if (8000..=384_000).contains(&rate) {
                    self.streams[si].ss.rate = rate;
                    rt.streams[si]
                        .sample_rate_atomic
                        .store(rate, Ordering::Relaxed);
                }
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_SET_PLAYBACK_STREAM_NAME | CMD_SET_RECORD_STREAM_NAME => {
                let (Some(ch), Some(nm_opt)) = (tr.read_u32(), tr.read_str_slice()) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                if let Some(si) = self.find_client_stream(ci, ch) {
                    if let Some((o, l)) = nm_opt {
                        self.streams[si].name =
                            String::from_utf8_lossy(&buf[o..o + l]).into_owned();
                    }
                }
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_UPDATE_PLAYBACK_STREAM_PROPLIST | CMD_UPDATE_RECORD_STREAM_PROPLIST => {
                let (Some(ch), Some(_mode)) = (tr.read_u32(), tr.read_u32()) else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                if let Some(si) = self.find_client_stream(ci, ch) {
                    let mut pb = self.streams[si].props.clone();
                    let _ = tr.read_proplist(Some(&mut pb));
                    self.streams[si].props = pb;
                    if let Some(mn) = self.streams[si].props.get_str("media.name") {
                        self.streams[si].name = mn;
                    }
                }
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_UPDATE_CLIENT_PROPLIST => {
                if tr.read_u32().is_some() {
                    let mut pb = self.clients[ci].props.clone();
                    let _ = tr.read_proplist(Some(&mut pb));
                    self.clients[ci].props = pb;
                }
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_REMOVE_PLAYBACK_STREAM_PROPLIST
            | CMD_REMOVE_RECORD_STREAM_PROPLIST
            | CMD_REMOVE_CLIENT_PROPLIST => {
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_MOVE_SINK_INPUT | CMD_MOVE_SOURCE_OUTPUT => {
                let (Some(sidx), Some(dev_idx), Some(nm_opt)) =
                    (tr.read_u32(), tr.read_u32(), tr.read_str_slice())
                else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                let dev_nm =
                    nm_opt.map(|(o, l)| String::from_utf8_lossy(&buf[o..o + l]).into_owned());
                if cmd == CMD_MOVE_SINK_INPUT {
                    if let Some(wi) = self.find_lasapi_proxy_by_stream_idx(sidx) {
                        self.refresh_device_cache();
                        let Some((resolved_idx, _)) =
                            self.resolve_sink_idx_and_name(dev_idx, dev_nm.as_deref())
                        else {
                            self.clients[ci].reply_err(tag, ERR_NOENTITY);
                            return;
                        };
                        self.lasapi_proxies[wi].target_dev_idx = resolved_idx;
                        self.clients[ci].reply_simple_ack(tag);
                        self.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_CHANGE, sidx);
                        return;
                    }
                }
                let Some(si) = self.find_stream_by_index(sidx) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                self.refresh_device_cache();
                if cmd == CMD_MOVE_SINK_INPUT {
                    let Some((resolved_idx, _)) =
                        self.resolve_sink_idx_and_name(dev_idx, dev_nm.as_deref())
                    else {
                        self.clients[ci].reply_err(tag, ERR_NOENTITY);
                        return;
                    };
                    self.streams[si].target_dev_idx = resolved_idx;
                    self.streams[si].pinned_route = dev_idx != PA_INVALID_INDEX
                        || dev_nm
                            .as_deref()
                            .map(|s| !s.is_empty() && !s.starts_with("@DEFAULT_"))
                            .unwrap_or(false);
                } else {
                    let Some((resolved_idx, _, is_mon)) =
                        self.resolve_source_idx_and_name(dev_idx, dev_nm.as_deref())
                    else {
                        self.clients[ci].reply_err(tag, ERR_NOENTITY);
                        return;
                    };
                    let explicit_req = dev_idx != PA_INVALID_INDEX
                        || dev_nm
                            .as_deref()
                            .map(|s| !s.is_empty() && !s.starts_with("@DEFAULT_"))
                            .unwrap_or(false);
                    let pk_det = self.streams[si].peak_detect;
                    self.sync_active_capture_if_explicit(resolved_idx, is_mon, pk_det, explicit_req);
                    self.streams[si].target_dev_idx = resolved_idx;
                    self.streams[si].is_monitor = is_mon;
                    self.streams[si].pinned_route = explicit_req;
                    rt.streams[si].is_monitor.store(is_mon, Ordering::Relaxed);
                }
                self.stream_update_device_id(si);
                let is_pb = self.streams[si].is_playback;
                let stream_idx = self.streams[si].index;
                self.clients[ci].reply_simple_ack(tag);
                self.broadcast_sub_event(
                    if is_pb {
                        SUB_EVENT_SINK_INPUT
                    } else {
                        SUB_EVENT_SOURCE_OUTPUT
                    },
                    SUB_EVENT_CHANGE,
                    stream_idx,
                );
            }
            CMD_KILL_SINK_INPUT | CMD_KILL_SOURCE_OUTPUT => {
                let Some(idx) = tr.read_u32() else {
                    self.clients[ci].reply_err(tag, ERR_PROTOCOL);
                    return;
                };
                if cmd == CMD_KILL_SINK_INPUT && self.find_lasapi_proxy_by_stream_idx(idx).is_some() {
                    self.clients[ci].reply_err(tag, ERR_ACCESS);
                    return;
                }
                let Some(si) = self.find_stream_by_index(idx) else {
                    self.clients[ci].reply_err(tag, ERR_NOENTITY);
                    return;
                };
                self.free_stream(si, rt);
                self.clients[ci].reply_simple_ack(tag);
            }
            CMD_EXTENSION => {
                self.clients[ci].reply_err(tag, ERR_NOEXTENSION);
            }
            _ => {
                self.clients[ci].reply_err(tag, ERR_COMMAND);
            }
        }
    }

    fn process_client_io(
        &mut self,
        ci: usize,
        revents: i16,
        rt: &Arc<PaRtShared>,
        vol_lock: &Mutex<()>,
    ) {
        if (revents & (POLLERR | POLLHUP | POLLNVAL)) != 0 {
            self.close_client(ci, rt);
            return;
        }
        if (revents & POLLOUT) != 0 {
            let fd = self.clients[ci].raw_fd();
            while let Some(front) = self.clients[ci].out_q.front_mut() {
                match nb_send(fd, &front.data[front.sent..]) {
                    Ok(w) => {
                        front.sent += w;
                        if front.sent >= front.data.len() {
                            self.clients[ci].out_q.pop_front();
                        } else {
                            break;
                        }
                    }
                    Err(e) => {
                        let raw = e.raw_os_error().unwrap_or(0);
                        if raw == 11 || raw == 4 {
                            break;
                        }
                        self.close_client(ci, rt);
                        return;
                    }
                }
            }
        }
        if (revents & POLLIN) != 0 {
            for _ in 0..64 {
                let fd = self.clients[ci].raw_fd();
                if fd < 0 {
                    return;
                }
                if self.clients[ci].rx_hdr_got < PA_PSTREAM_DESC_BYTES {
                    let got = self.clients[ci].rx_hdr_got;
                    let mut tmp = [0u8; PA_PSTREAM_DESC_BYTES];
                    match nb_recv(fd, &mut tmp[..PA_PSTREAM_DESC_BYTES - got]) {
                        Ok(0) => {
                            self.close_client(ci, rt);
                            return;
                        }
                        Ok(n) => {
                            self.clients[ci].rx_hdr[got..got + n].copy_from_slice(&tmp[..n]);
                            self.clients[ci].rx_hdr_got += n;
                            if self.clients[ci].rx_hdr_got < PA_PSTREAM_DESC_BYTES {
                                break;
                            }
                            let Some(hdr) = parse_frame_hdr(&self.clients[ci].rx_hdr) else {
                                failsafe::record_trip(
                                    failsafe::FS_PA_BAD_PKT,
                                    b"PA malformed/oversized frame quarantined",
                                );
                                self.close_client(ci, rt);
                                return;
                            };
                            if !self.clients[ci].authed && hdr.body_len > 64 * 1024 {
                                failsafe::record_trip(
                                    failsafe::FS_PA_BAD_PKT,
                                    b"PA unauthenticated client oversized frame quarantined",
                                );
                                self.close_client(ci, rt);
                                return;
                            }
                            self.clients[ci].rx_body_len = hdr.body_len as usize;
                            self.clients[ci].rx_channel = hdr.channel;
                            self.clients[ci].rx_body_got = 0;
                            let blen = self.clients[ci].rx_body_len;
                            self.clients[ci].rx_body.resize(blen, 0);
                        }
                        Err(e) => {
                            let raw = e.raw_os_error().unwrap_or(0);
                            if raw == 11 {
                                break;
                            }
                            if raw == 4 {
                                continue;
                            }
                            self.close_client(ci, rt);
                            return;
                        }
                    }
                }
                let blen = self.clients[ci].rx_body_len;
                let mut need_more_poll = false;
                while self.clients[ci].rx_body_got < blen {
                    let bgot = self.clients[ci].rx_body_got;
                    match nb_recv(fd, &mut self.clients[ci].rx_body[bgot..blen]) {
                        Ok(0) => {
                            self.close_client(ci, rt);
                            return;
                        }
                        Ok(n) => {
                            self.clients[ci].rx_body_got += n;
                        }
                        Err(e) => {
                            let raw = e.raw_os_error().unwrap_or(0);
                            if raw == 11 {
                                need_more_poll = true;
                                break;
                            }
                            if raw == 4 {
                                continue;
                            }
                            self.close_client(ci, rt);
                            return;
                        }
                    }
                }
                if need_more_poll || self.clients[ci].rx_body_got < blen {
                    break;
                }
                let channel = self.clients[ci].rx_channel;
                core::mem::swap(&mut self.scratch_rx, &mut self.clients[ci].rx_body);
                self.clients[ci].rx_hdr_got = 0;
                self.clients[ci].rx_body_len = 0;
                self.clients[ci].rx_body_got = 0;
                let body = core::mem::take(&mut self.scratch_rx);
                if channel == PA_PSTREAM_CTRL_CHANNEL {
                    self.handle_control_packet(ci, &body[..blen], rt, vol_lock);
                } else if !self.clients[ci].authed {
                    self.scratch_rx = body;
                    self.close_client(ci, rt);
                    return;
                } else {
                    self.handle_data_memblock(ci, channel, &body[..blen], rt);
                }
                self.scratch_rx = body;
                if self.scratch_rx.capacity() > 262_144 {
                    self.scratch_rx.shrink_to(65_536);
                }
                core::mem::swap(&mut self.scratch_rx, &mut self.clients[ci].rx_body);
            }
        }
    }
}

pub struct PaServerHandle {
    state: Arc<Mutex<PaState>>,
    rt: Arc<PaRtShared>,
    vol_file_lock: Arc<Mutex<()>>,
    running: Arc<AtomicBool>,
    wake_efd: OwnedFd,
    sock_path: String,
    pid_path: String,
    tcp_port: Option<u16>,
    tcp_port_path: Option<String>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

fn collect_pa_facility_ids(dl: &LasapiDeviceList, facility: u32) -> Vec<String> {
    let ndev = (dl.num_devices as usize).min(LASAPI_MAX_DEVICES);
    let mut out = Vec::new();
    match facility {
        SUB_EVENT_SINK => {
            for d in &dl.devices[..ndev] {
                if d.is_capture == 0 {
                    out.push(c_str_to_string(&d.id));
                }
            }
            if out.is_empty() {
                out.push("virtual".to_string());
            }
        }
        SUB_EVENT_SOURCE => {
            for d in &dl.devices[..ndev] {
                if d.is_capture != 0 {
                    out.push(c_str_to_string(&d.id));
                }
            }
            let mut pb_n = 0;
            for d in &dl.devices[..ndev] {
                if d.is_capture == 0 {
                    out.push(make_mon_key(&c_str_to_string(&d.id)));
                    pb_n += 1;
                }
            }
            if pb_n == 0 {
                out.push("mon:virtual".to_string());
            }
        }
        SUB_EVENT_CARD => {
            for d in &dl.devices[..ndev] {
                let id = c_str_to_string(&d.id);
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        _ => {}
    }
    out
}

fn emit_facility_diff(
    st: &mut PaState,
    facility: u32,
    old_dl: &LasapiDeviceList,
    new_dl: &LasapiDeviceList,
    active_idx: u32,
) {
    let old_ids = collect_pa_facility_ids(old_dl, facility);
    let new_ids = collect_pa_facility_ids(new_dl, facility);
    let fac = match facility {
        SUB_EVENT_SINK => PA_FAC_SINK,
        SUB_EVENT_SOURCE => PA_FAC_SOURCE,
        _ => PA_FAC_CARD,
    };
    let mut emitted_active = false;
    for oi in &old_ids {
        if !new_ids.contains(oi) {
            let pa_idx = st.stable_pa_idx(fac, oi);
            st.broadcast_sub_event(facility, SUB_EVENT_REMOVE, pa_idx);
        }
    }
    for ni in &new_ids {
        let pa_idx = st.stable_pa_idx(fac, ni);
        if !old_ids.contains(ni) {
            st.broadcast_sub_event(facility, SUB_EVENT_NEW, pa_idx);
            if pa_idx == active_idx {
                emitted_active = true;
            }
        }
    }
    if active_idx != PA_INVALID_INDEX && !emitted_active {
        st.broadcast_sub_event(facility, SUB_EVENT_CHANGE, active_idx);
    }
}

fn server_thread_main(
    listener: UnixListener,
    tcp_listener: Option<TcpListener>,
    wake_fd: RawFd,
    running: Arc<AtomicBool>,
    state: Arc<Mutex<PaState>>,
    rt: Arc<PaRtShared>,
    vol_lock: Arc<Mutex<()>>,
) {
    enable_realtime(60, -15);
    let listen_fd = listener.as_raw_fd();
    let tcp_listen_fd = tcp_listener.as_ref().map(|l| l.as_raw_fd());
    let mut pfds = Vec::with_capacity(PA_MAX_CLIENTS + 3);
    let mut cmap: Vec<usize> = Vec::with_capacity(PA_MAX_CLIENTS + 3);

    while running.load(Ordering::Relaxed) {
        pfds.clear();
        cmap.clear();
        pfds.push(PollFd {
            fd: listen_fd,
            events: POLLIN,
            revents: 0,
        });
        cmap.push(usize::MAX);
        pfds.push(PollFd {
            fd: wake_fd,
            events: POLLIN,
            revents: 0,
        });
        cmap.push(usize::MAX);
        let tcp_pfd_idx = if let Some(tfd) = tcp_listen_fd {
            let idx = pfds.len();
            pfds.push(PollFd {
                fd: tfd,
                events: POLLIN,
                revents: 0,
            });
            cmap.push(usize::MAX);
            Some(idx)
        } else {
            None
        };
        let client_pfd_start = pfds.len();
        let mut has_unauth = false;

        let any_stream = {
            let now_ms = mono_ms();
            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
            st.service_stream_events_and_capture(&rt);
            for ci in 0..PA_MAX_CLIENTS {
                if !st.clients[ci].in_use {
                    continue;
                }
                if !st.clients[ci].authed
                    && now_ms.saturating_sub(st.clients[ci].accepted_ms) >= PA_UNAUTH_TIMEOUT_MS
                {
                    st.close_client(ci, &rt);
                    continue;
                }
                if !st.clients[ci].authed {
                    has_unauth = true;
                }
                let fd = st.clients[ci].raw_fd();
                if fd < 0 {
                    continue;
                }
                pfds.push(PollFd {
                    fd,
                    events: POLLIN | if !st.clients[ci].out_q.is_empty() { POLLOUT } else { 0 },
                    revents: 0,
                });
                cmap.push(ci);
            }
            st.streams.iter().any(|s| s.in_use)
        };

        let poll_timeout = if has_unauth || any_stream { 200 } else { 500 };
        let pr = do_poll(&mut pfds, poll_timeout);
        if pr < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(4) {
                continue;
            }
            break;
        }
        if (pfds[1].revents & POLLIN) != 0 {
            eventfd_drain(wake_fd);
        }

        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
        if (pfds[0].revents & POLLIN) != 0 {
            for _ in 0..PA_ACCEPT_BATCH_CAP {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(true);
                        let owned: OwnedFd = stream.into();
                        let cred = socket_peercred(owned.as_raw_fd());
                        if let Some(uc) = cred {
                            if uc.uid != current_uid() && uc.uid != 0 {
                                continue;
                            }
                        }
                        let uid_pair = cred.map(|uc| {
                            (uc.uid, if uc.pid > 0 { uc.pid as u32 } else { 0 })
                        });
                        st.accept_client_fd(owned, uid_pair, false);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
        if let (Some(ref tl), Some(tidx)) = (&tcp_listener, tcp_pfd_idx) {
            if (pfds[tidx].revents & POLLIN) != 0 {
                for _ in 0..PA_ACCEPT_BATCH_CAP {
                    match tl.accept() {
                        Ok((stream, addr)) if addr.ip().is_loopback() => {
                            let _ = stream.set_nodelay(true);
                            let _ = stream.set_nonblocking(true);
                            let owned: OwnedFd = stream.into();
                            st.accept_client_fd(owned, None, true);
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
            }
        }

        for i in client_pfd_start..pfds.len() {
            let rev = pfds[i].revents;
            let ci = cmap[i];
            if rev != 0 && ci < PA_MAX_CLIENTS && st.clients[ci].in_use {
                st.process_client_io(ci, rev, &rt, &vol_lock);
            }
        }
    }
}

fn make_pa_sink_name(salsa_id: &str) -> String {
    if salsa_id.is_empty() || salsa_id == "virtual" {
        return "salsa_out.virtual".to_string();
    }
    let clean: String = salsa_id
        .chars()
        .take(63)
        .map(|c| if c == ':' || c == ',' { '_' } else { c })
        .collect();
    format!("salsa_out.{clean}")
}

fn make_pa_source_name(salsa_id: &str) -> String {
    if salsa_id.is_empty() {
        return "salsa_in.none".to_string();
    }
    let clean: String = salsa_id
        .chars()
        .take(63)
        .map(|c| if c == ':' || c == ',' { '_' } else { c })
        .collect();
    format!("salsa_in.{clean}")
}

fn make_mon_key(dev_id: &str) -> String {
    let base = if dev_id.is_empty() { "virtual" } else { dev_id };
    let trunc: String = base.chars().take(58).collect();
    format!("mon:{trunc}")
}

fn sanitize_app_vol_key(is_playback: bool, raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let base = match raw.rsplit_once('/') {
        Some((_, b)) if !b.is_empty() => b,
        _ => raw,
    };
    let mut out = String::with_capacity(64);
    out.push_str(if is_playback { "play:" } else { "rec:" });
    let mut added = 0usize;
    for b in base.bytes() {
        if out.len() >= 127 {
            break;
        }
        let lc = b.to_ascii_lowercase();
        if lc.is_ascii_lowercase()
            || lc.is_ascii_digit()
            || lc == b'.'
            || lc == b'_'
            || lc == b'-'
        {
            out.push(lc as char);
            added += 1;
        } else if lc == b' ' {
            out.push('_');
            added += 1;
        }
    }
    if added > 0 {
        Some(out)
    } else {
        None
    }
}

fn init_app_vols_path() -> String {
    let home = user_home_dir();
    if !home.starts_with('/') {
        return String::new();
    }
    let uid = current_uid();
    let cfg_dir = format!("{home}/.config");
    let _ = ensure_owned_dir_0700(&cfg_dir, uid);
    let app_dir = format!("{cfg_dir}/lasapi");
    let _ = ensure_owned_dir_0700(&app_dir, uid);
    format!("{app_dir}/app_volumes.conf")
}

fn write_app_volumes_disk(file_lock: &Mutex<()>, path: &str, entries: &[AppVolEntry]) {
    if path.is_empty() {
        return;
    }
    let tmp = format!("{path}.{}", rand_tmp_suffix());
    let _guard = file_lock.lock().unwrap_or_else(|p| p.into_inner());
    let Ok(mut f) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(unix_sys::O_NOFOLLOW | unix_sys::O_CLOEXEC)
        .open(&tmp)
    else {
        return;
    };
    let mut ok = writeln!(f, "# Salsa Persistent Per-App Volumes (key=volume_milli,muted)").is_ok();
    if ok {
        for e in entries {
            if writeln!(f, "{}={},{}", e.key, e.volume_milli, e.muted).is_err() {
                ok = false;
                break;
            }
        }
    }
    if ok {
        ok = f.flush().is_ok() && f.sync_all().is_ok();
    }
    drop(f);
    if !ok || fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn ensure_pulse_cookie(out_cookie: &mut [u8; PA_NATIVE_COOKIE_LEN], out_valid: &mut bool) {
    *out_valid = false;
    let uid = current_uid();
    let home = user_home_dir();
    if home == "/tmp" || !is_safe_owner_dir(Path::new(&home), uid, false) {
        return;
    }
    let dir = format!("{home}/.config/pulse");
    if !ensure_owned_dir_0700(&dir, uid) {
        return;
    }
    let cpath = format!("{dir}/cookie");
    if let Ok(f) = OpenOptions::new()
        .read(true)
        .custom_flags(unix_sys::O_NOFOLLOW | unix_sys::O_NONBLOCK | unix_sys::O_CLOEXEC)
        .open(&cpath)
    {
        if let Ok(meta) = f.metadata() {
            if meta.is_file()
                && meta.uid() == uid
                && (meta.mode() & 0o077) == 0
                && meta.len() == PA_NATIVE_COOKIE_LEN as u64
            {
                let mut f = f;
                let mut tmp = [0u8; PA_NATIVE_COOKIE_LEN];
                if f.read_exact(&mut tmp).is_ok() {
                    *out_cookie = tmp;
                    *out_valid = true;
                }
                return;
            }
        }
    }
    let mut cookie = [0u8; PA_NATIVE_COOKIE_LEN];
    if let Ok(mut ur) = fs::File::open("/dev/urandom") {
        if ur.read_exact(&mut cookie).is_ok() {
            let tmp_cpath = format!("{cpath}.{}", rand_tmp_suffix());
            let mut wrote = false;
            if let Ok(mut f) = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(unix_sys::O_NOFOLLOW | unix_sys::O_CLOEXEC)
                .open(&tmp_cpath)
            {
                if f.write_all(&cookie).is_ok()
                    && f.flush().is_ok()
                    && fs::rename(&tmp_cpath, &cpath).is_ok()
                {
                    *out_cookie = cookie;
                    *out_valid = true;
                    wrote = true;
                }
            }
            if !wrote {
                let _ = fs::remove_file(&tmp_cpath);
            }
        }
    }
}

fn resolve_stream_format_spec(
    ss: &mut PaSampleSpec,
    mp: &mut PaChannelMap,
    formats: &[PaFormatInfo],
    hw_rate: u32,
) -> Result<(), u32> {
    if !formats.is_empty() {
        let Some(fi) = formats
            .iter()
            .find(|f| f.encoding == PA_ENCODING_PCM || f.encoding == PA_ENCODING_ANY)
        else {
            return Err(ERR_NOTSUPPORTED);
        };
        if ss.format >= PA_SAMPLE_MAX {
            if let Some(fmt) = fi.sample_format {
                ss.format = fmt;
            }
        }
        if ss.rate == 0 || ss.rate > 768_000 {
            if let Some(rate) = fi.rate {
                ss.rate = rate;
            }
        }
        if ss.channels == 0 || ss.channels > 32 {
            if let Some(ch) = fi.channels {
                ss.channels = ch;
            } else if let Some(cm) = fi.channel_map {
                ss.channels = cm.channels;
            }
        }
        if let Some(cm) = fi.channel_map {
            if mp.channels == 0 || (ss.channels > 0 && cm.channels == ss.channels) {
                *mp = cm;
            }
        }
    }
    if ss.format >= PA_SAMPLE_MAX {
        ss.format = FMT_FLOAT32LE;
    }
    let fallback_rate = if (8000..=768_000).contains(&hw_rate) {
        hw_rate
    } else {
        48000
    };
    if ss.rate == 0 || ss.rate > 768_000 {
        ss.rate = fallback_rate;
    }
    if ss.channels == 0 || ss.channels > 32 {
        ss.channels = if mp.channels > 0 && mp.channels <= 32 {
            mp.channels
        } else {
            2
        };
    }
    if mp.channels != ss.channels {
        *mp = std_channel_map(ss.channels);
    }
    Ok(())
}

fn normalize_stream_cvol(cv: &PaCVolume, channels: u8) -> PaCVolume {
    let ch = if channels == 0 { 2 } else { channels.min(32) };
    if cv.channels == ch {
        return *cv;
    }
    let src_n = cv.channels.min(32) as usize;
    let avg = if src_n > 0 {
        let sum: u64 = cv.values[..src_n].iter().map(|&v| v as u64).sum();
        (sum / src_n as u64) as u32
    } else {
        PA_VOLUME_NORM
    };
    let mut out = PaCVolume {
        channels: ch,
        values: [0; 32],
    };
    for v in &mut out.values[..ch as usize] {
        *v = avg;
    }
    out
}

fn enforce_playback_attr(s: &mut PaStreamMeta, maxl: u32, tl: u32, minr: u32, preb: u32) {
    if s.frame_size == 0 {
        return;
    }
    let fsz = s.frame_size;
    let rate = if s.ss.rate != 0 { s.ss.rate } else { 48000 };
    let max_ring = (PA_STREAM_RING_FRAMES as u32)
        .saturating_mul(fsz)
        .min(PA_DEFAULT_MAXLENGTH);
    let mut bytes_10ms = if rate >= 100 { (rate / 100) * fsz } else { fsz };
    if rate >= 1000 && bytes_10ms < fsz * 256 {
        bytes_10ms = fsz * 256;
    }
    bytes_10ms = ((bytes_10ms / fsz) * fsz).max(fsz);
    let mut bytes_5ms = ((bytes_10ms / 2) / fsz) * fsz;
    if bytes_5ms < fsz * 128 && rate >= 24000 {
        bytes_5ms = fsz * 128;
    }
    bytes_5ms = bytes_5ms.max(fsz);
    let bytes_20ms = bytes_10ms * 2;
    let bytes_15ms = ((bytes_10ms + bytes_5ms) / fsz) * fsz;
    let bytes_30ms = ((bytes_10ms * 3) / fsz) * fsz;

    s.maxlength = if maxl == PA_INVALID_INDEX || maxl > max_ring {
        max_ring
    } else {
        maxl
    };
    if s.maxlength < bytes_20ms * 2 && max_ring >= bytes_20ms * 2 {
        s.maxlength = bytes_20ms * 2;
    }
    s.maxlength = ((s.maxlength.min(max_ring) / fsz) * fsz).max(fsz);

    s.tlength = if tl == PA_INVALID_INDEX || tl == 0 {
        bytes_20ms * 4
    } else {
        tl
    };
    let min_tl = if preb == 0 {
        bytes_20ms
    } else {
        bytes_30ms.min(s.maxlength)
    };
    s.tlength = ((s.tlength.clamp(min_tl, s.maxlength) / fsz) * fsz).max(fsz);

    let min_floor = if s.tlength <= bytes_20ms * 2 {
        bytes_5ms
    } else {
        bytes_10ms
    };
    let def_minreq = (((s.tlength / 4) / fsz) * fsz).max(min_floor);

    s.minreq = if minr == PA_INVALID_INDEX || minr == 0 {
        def_minreq
    } else {
        minr
    };
    if s.minreq < bytes_5ms {
        s.minreq = bytes_5ms;
    }
    if s.minreq > s.tlength / 2 && s.tlength >= fsz * 2 {
        s.minreq = s.tlength / 2;
    }
    s.minreq = ((s.minreq / fsz) * fsz).max(fsz);
    if s.tlength < s.minreq * 2 && s.maxlength >= s.minreq * 2 {
        s.tlength = s.minreq * 2;
    } else if s.tlength < s.minreq {
        s.tlength = s.minreq;
    }

    let max_preb = if s.tlength > s.minreq {
        ((s.tlength - s.minreq) / fsz) * fsz
    } else {
        s.tlength
    };
    let def_prebuf = ((s.tlength / 2).max(bytes_15ms).min(max_preb) / fsz) * fsz;
    s.prebuf = if preb == PA_INVALID_INDEX {
        def_prebuf.max(s.minreq.min(max_preb))
    } else if preb == 0 {
        0
    } else {
        let min_safe_preb = bytes_10ms.min(max_preb);
        ((preb.max(min_safe_preb) / fsz) * fsz).min(s.tlength)
    };
    s.prebuf = ((s.prebuf / fsz) * fsz).min(s.tlength);
}

fn enforce_record_attr(s: &mut PaStreamMeta, frags: u32) {
    if s.frame_size == 0 {
        return;
    }
    let fsz = s.frame_size;
    let rate = if s.ss.rate != 0 { s.ss.rate } else { 48000 };
    let mut bytes_10ms = if rate >= 100 { (rate / 100) * fsz } else { fsz };
    let mut bytes_20ms = bytes_10ms * 2;
    if rate >= 1000 {
        bytes_10ms = bytes_10ms.max(fsz * 256);
        bytes_20ms = bytes_20ms.max(fsz * 512);
    } else {
        bytes_20ms = bytes_20ms.max(fsz);
    }
    s.fragsize = if frags == PA_INVALID_INDEX || frags == 0 {
        bytes_20ms
    } else {
        frags
    };
    if !s.peak_detect && rate >= 1000 && s.fragsize < bytes_10ms {
        s.fragsize = bytes_10ms;
    }
    s.fragsize = ((s.fragsize.max(fsz) / fsz) * fsz).max(fsz);
    if s.maxlength >= fsz * 2 && s.fragsize > s.maxlength / 2 {
        s.fragsize = ((s.maxlength / 2) / fsz) * fsz;
    }
}

fn is_voice_comms_stream(s: &PaStreamMeta) -> bool {
    if s.is_playback || s.is_monitor || s.peak_detect || s.direct_on_input != PA_INVALID_INDEX {
        return false;
    }
    if let Some(role) = s.props.get_str("media.role") {
        if role.eq_ignore_ascii_case("phone") || role.eq_ignore_ascii_case("communication") {
            return true;
        }
    }
    if s.name.eq_ignore_ascii_case("recStream") {
        return true;
    }
    let lc = s.client_name.to_ascii_lowercase();
    ["discord", "webrtc", "zoom", "teamspeak", "mumble", "skype", "slack"]
        .iter()
        .any(|k| lc.contains(k))
}

fn pa_cmd_name(cmd: u32) -> &'static str {
    match cmd {
        CMD_CREATE_PLAYBACK_STREAM => "PA_CMD_CREATE_PB",
        CMD_DELETE_PLAYBACK_STREAM => "PA_CMD_DEL_PB",
        CMD_CREATE_RECORD_STREAM => "PA_CMD_CREATE_REC",
        CMD_DELETE_RECORD_STREAM => "PA_CMD_DEL_REC",
        CMD_EXIT => "PA_CMD_EXIT",
        CMD_AUTH => "PA_CMD_AUTH",
        CMD_SET_CLIENT_NAME => "PA_CMD_SET_CNAME",
        CMD_LOOKUP_SINK => "PA_CMD_LOOKUP_SINK",
        CMD_LOOKUP_SOURCE => "PA_CMD_LOOKUP_SRC",
        CMD_DRAIN_PLAYBACK_STREAM => "PA_CMD_DRAIN_PB",
        CMD_STAT => "PA_CMD_STAT",
        CMD_GET_PLAYBACK_LATENCY => "PA_CMD_GET_PB_LAT",
        CMD_GET_SERVER_INFO => "PA_CMD_SERVER_INFO",
        CMD_GET_SINK_INFO => "PA_CMD_SINK_INFO",
        CMD_GET_SINK_INFO_LIST => "PA_CMD_SINK_LIST",
        CMD_GET_SOURCE_INFO => "PA_CMD_SRC_INFO",
        CMD_GET_SOURCE_INFO_LIST => "PA_CMD_SRC_LIST",
        CMD_GET_MODULE_INFO => "PA_CMD_MOD_INFO",
        CMD_GET_MODULE_INFO_LIST => "PA_CMD_MOD_LIST",
        CMD_GET_CLIENT_INFO => "PA_CMD_CLI_INFO",
        CMD_GET_CLIENT_INFO_LIST => "PA_CMD_CLI_LIST",
        CMD_GET_SINK_INPUT_INFO => "PA_CMD_SI_INFO",
        CMD_GET_SINK_INPUT_INFO_LIST => "PA_CMD_SI_LIST",
        CMD_GET_SOURCE_OUTPUT_INFO => "PA_CMD_SO_INFO",
        CMD_GET_SOURCE_OUTPUT_INFO_LIST => "PA_CMD_SO_LIST",
        CMD_GET_SAMPLE_INFO_LIST => "PA_CMD_SAMPLE_LIST",
        CMD_SUBSCRIBE => "PA_CMD_SUBSCRIBE",
        CMD_SET_SINK_VOLUME => "PA_CMD_SET_SINK_VOL",
        CMD_SET_SINK_INPUT_VOLUME => "PA_CMD_SET_SI_VOL",
        CMD_SET_SOURCE_VOLUME => "PA_CMD_SET_SRC_VOL",
        CMD_SET_SINK_MUTE => "PA_CMD_SET_SINK_MUTE",
        CMD_SET_SOURCE_MUTE => "PA_CMD_SET_SRC_MUTE",
        CMD_CORK_PLAYBACK_STREAM => "PA_CMD_CORK_PB",
        CMD_FLUSH_PLAYBACK_STREAM => "PA_CMD_FLUSH_PB",
        CMD_TRIGGER_PLAYBACK_STREAM => "PA_CMD_TRIG_PB",
        CMD_SET_DEFAULT_SINK => "PA_CMD_SET_DEF_SINK",
        CMD_SET_DEFAULT_SOURCE => "PA_CMD_SET_DEF_SRC",
        CMD_SET_PLAYBACK_STREAM_NAME => "PA_CMD_SET_PB_NAME",
        CMD_SET_RECORD_STREAM_NAME => "PA_CMD_SET_REC_NAME",
        CMD_KILL_SINK_INPUT => "PA_CMD_KILL_SI",
        CMD_KILL_SOURCE_OUTPUT => "PA_CMD_KILL_SO",
        CMD_GET_RECORD_LATENCY => "PA_CMD_GET_REC_LAT",
        CMD_CORK_RECORD_STREAM => "PA_CMD_CORK_REC",
        CMD_FLUSH_RECORD_STREAM => "PA_CMD_FLUSH_REC",
        CMD_PREBUF_PLAYBACK_STREAM => "PA_CMD_PREBUF_PB",
        CMD_MOVE_SINK_INPUT => "PA_CMD_MOVE_SI",
        CMD_MOVE_SOURCE_OUTPUT => "PA_CMD_MOVE_SO",
        CMD_SET_SINK_INPUT_MUTE => "PA_CMD_SET_SI_MUTE",
        CMD_SET_PLAYBACK_STREAM_BUFFER_ATTR => "PA_CMD_SET_PB_ATTR",
        CMD_SET_RECORD_STREAM_BUFFER_ATTR => "PA_CMD_SET_REC_ATTR",
        CMD_GET_CARD_INFO => "PA_CMD_CARD_INFO",
        CMD_GET_CARD_INFO_LIST => "PA_CMD_CARD_LIST",
        CMD_SET_CARD_PROFILE => "PA_CMD_SET_PROFILE",
        CMD_SET_SOURCE_OUTPUT_VOLUME => "PA_CMD_SET_SO_VOL",
        CMD_SET_SOURCE_OUTPUT_MUTE => "PA_CMD_SET_SO_MUTE",
        _ => "PA_CMD_OTHER",
    }
}

fn pa_is_lifecycle_cmd(cmd: u32) -> bool {
    matches!(
        cmd,
        CMD_AUTH
            | CMD_SET_CLIENT_NAME
            | CMD_EXIT
            | CMD_CREATE_PLAYBACK_STREAM
            | CMD_CREATE_RECORD_STREAM
            | CMD_DELETE_PLAYBACK_STREAM
            | CMD_DELETE_RECORD_STREAM
            | CMD_CORK_PLAYBACK_STREAM
            | CMD_CORK_RECORD_STREAM
    )
}

fn put_sink_entry(
    tb: &mut TagBuilder,
    ver: u32,
    sink_idx: u32,
    mon_src_idx: u32,
    card_idx: u32,
    pa_name: &str,
    desc: &str,
    salsa_id: &str,
    is_bt: bool,
    rate: u32,
    vol_pct: u32,
    pa_vol_override: u32,
    muted: bool,
) {
    let ss = PaSampleSpec {
        format: pa_wire::FMT_FLOAT32LE,
        channels: 2,
        rate: if rate != 0 { rate } else { 48000 },
    };
    let mp = std_channel_map(2);
    let pv = if pa_vol_override != PA_INVALID_INDEX {
        pa_vol_override
    } else {
        pct_to_pa_vol(vol_pct)
    };
    let mut cv = PaCVolume {
        channels: 2,
        values: [0; 32],
    };
    cv.values[0] = pv;
    cv.values[1] = pv;
    let mon_name = format!("{pa_name}.monitor");
    let flags = 0x0002 | 0x0004 | 0x0020 | 0x0080 | if is_bt { 0x0001 } else { 0 };

    tb.put_u32(PA_TAG_U32, sink_idx);
    tb.put_str(Some(pa_name));
    tb.put_str(Some(desc));
    tb.put_ss(&ss);
    tb.put_map(&mp);
    tb.put_u32(PA_TAG_U32, 0);
    tb.put_cvol(&cv);
    tb.put_bool(muted);
    tb.put_u32(PA_TAG_U32, mon_src_idx);
    tb.put_str(Some(&mon_name));
    tb.put_u64_raw(PA_TAG_USEC, if is_bt { 20000 } else { 2000 });
    tb.put_str(Some("Salsa-LASAPI"));
    tb.put_u32(PA_TAG_U32, flags);
    if ver >= 13 {
        let mut pb = PaPropBag::default();
        pb.set_kv("device.description", desc);
        pb.set_kv("device.class", "sound");
        pb.set_kv("device.api", if is_bt { "bluez" } else { "alsa" });
        pb.set_kv("salsa.device.id", salsa_id);
        pb.set_kv("node.driver", "lasapi-audiodg");
        tb.put_proplist(Some(&pb));
        tb.put_u64_raw(PA_TAG_USEC, 2000);
    }
    if ver >= 15 {
        tb.put_u32(PA_TAG_VOLUME, PA_VOLUME_NORM);
        tb.put_u32(PA_TAG_U32, 0);
        tb.put_u32(PA_TAG_U32, 65536);
        tb.put_u32(PA_TAG_U32, card_idx);
    }
    if ver >= 16 {
        tb.put_u32(PA_TAG_U32, 1);
        let pname = if is_bt { "headset-output" } else { "analog-output" };
        tb.put_str(Some(pname));
        tb.put_str(Some(desc));
        tb.put_u32(PA_TAG_U32, 9900);
        if ver >= 24 {
            tb.put_u32(PA_TAG_U32, 2);
        }
        if ver >= 34 {
            tb.put_str(None);
            tb.put_u32(PA_TAG_U32, if is_bt { 6 } else { 2 });
        }
        tb.put_str(Some(pname));
    }
    if ver >= 21 {
        tb.put_u8(PA_TAG_U8, 1);
        tb.put_pcm_format();
    }
}

fn put_source_entry(
    tb: &mut TagBuilder,
    ver: u32,
    src_idx: u32,
    card_idx: u32,
    pa_name: &str,
    desc: &str,
    salsa_id: Option<&str>,
    mon_of_sink_idx: u32,
    mon_of_sink_name: Option<&str>,
    rate: u32,
    vol_pct: u32,
    pa_vol_override: u32,
    muted: bool,
) {
    let ss = PaSampleSpec {
        format: pa_wire::FMT_FLOAT32LE,
        channels: 2,
        rate: if rate != 0 { rate } else { 48000 },
    };
    let mp = std_channel_map(2);
    let pv = if pa_vol_override != PA_INVALID_INDEX {
        pa_vol_override
    } else {
        pct_to_pa_vol(vol_pct)
    };
    let mut cv = PaCVolume {
        channels: 2,
        values: [0; 32],
    };
    cv.values[0] = pv;
    cv.values[1] = pv;
    let is_mon = mon_of_sink_idx != PA_INVALID_INDEX;
    let flags = 0x0002 | 0x0020 | 0x0040 | if !is_mon { 0x0004 } else { 0 };

    tb.put_u32(PA_TAG_U32, src_idx);
    tb.put_str(Some(pa_name));
    tb.put_str(Some(desc));
    tb.put_ss(&ss);
    tb.put_map(&mp);
    tb.put_u32(PA_TAG_U32, 0);
    tb.put_cvol(&cv);
    tb.put_bool(muted);
    tb.put_u32(PA_TAG_U32, mon_of_sink_idx);
    tb.put_str(mon_of_sink_name);
    tb.put_u64_raw(PA_TAG_USEC, 2000);
    tb.put_str(Some("Salsa-LASAPI"));
    tb.put_u32(PA_TAG_U32, flags);
    if ver >= 13 {
        let mut pb = PaPropBag::default();
        pb.set_kv("device.description", desc);
        pb.set_kv("device.class", if is_mon { "monitor" } else { "sound" });
        pb.set_kv("device.api", if is_mon { "salsa-monitor" } else { "alsa" });
        if let Some(sid) = salsa_id {
            pb.set_kv("salsa.device.id", sid);
        }
        tb.put_proplist(Some(&pb));
        tb.put_u64_raw(PA_TAG_USEC, 2000);
    }
    if ver >= 15 {
        tb.put_u32(PA_TAG_VOLUME, PA_VOLUME_NORM);
        tb.put_u32(PA_TAG_U32, 0);
        tb.put_u32(PA_TAG_U32, 65536);
        tb.put_u32(PA_TAG_U32, if is_mon { PA_INVALID_INDEX } else { card_idx });
    }
    if ver >= 16 {
        if !is_mon {
            tb.put_u32(PA_TAG_U32, 1);
            tb.put_str(Some("analog-input"));
            tb.put_str(Some(desc));
            tb.put_u32(PA_TAG_U32, 9900);
            if ver >= 24 {
                tb.put_u32(PA_TAG_U32, 2);
            }
            if ver >= 34 {
                tb.put_str(None);
                tb.put_u32(PA_TAG_U32, 4);
            }
            tb.put_str(Some("analog-input"));
        } else {
            tb.put_u32(PA_TAG_U32, 0);
            tb.put_str(None);
        }
    }
    if ver >= 22 {
        tb.put_u8(PA_TAG_U8, 1);
        tb.put_pcm_format();
    }
}

fn put_card_entry(
    tb: &mut TagBuilder,
    ver: u32,
    card_idx: u32,
    dl: &LasapiDeviceList,
    d: &crate::c_abi::LasapiDeviceEntry,
) {
    let did = c_str_to_string(&d.id);
    let dname = c_str_to_string(&d.name);
    let is_bt = d.is_bluetooth != 0;
    let raw_cname = if is_bt && did.len() >= 6 {
        format!("bluez_card.{}", &did[6..])
    } else {
        format!("alsa_card.{did}")
    };
    let cname: String = raw_cname
        .chars()
        .take(95)
        .map(|c| if c == ':' || c == ',' { '_' } else { c })
        .collect();

    let mut has_pb = false;
    let mut has_cap = false;
    let ndev = (dl.num_devices as usize).min(LASAPI_MAX_DEVICES);
    for dev in &dl.devices[..ndev] {
        if c_str_to_string(&dev.id) == did {
            if dev.is_capture != 0 {
                has_cap = true;
            } else {
                has_pb = true;
            }
        }
    }
    if !has_pb && !has_cap {
        has_pb = d.is_capture == 0;
        has_cap = d.is_capture != 0;
    }

    tb.put_u32(PA_TAG_U32, card_idx);
    tb.put_str(Some(&cname));
    tb.put_u32(PA_TAG_U32, 0);
    tb.put_str(Some(if is_bt {
        "module-bluez5-device.c"
    } else {
        "module-alsa-card.c"
    }));
    let prof_name = if is_bt {
        "a2dp-sink"
    } else if has_pb && has_cap {
        "output+input"
    } else if has_cap {
        "input"
    } else {
        "output"
    };
    let prof_desc = if is_bt {
        "Native BlueZ A2DP High Fidelity (SBC/AAC)"
    } else if has_pb && has_cap {
        "Direct Kernel ALSA Duplex"
    } else {
        "Direct Kernel ALSA"
    };
    tb.put_u32(PA_TAG_U32, 1);
    tb.put_str(Some(prof_name));
    tb.put_str(Some(prof_desc));
    tb.put_u32(PA_TAG_U32, u32::from(has_pb));
    tb.put_u32(PA_TAG_U32, u32::from(has_cap));
    tb.put_u32(PA_TAG_U32, 100);
    if ver >= 29 {
        tb.put_u32(PA_TAG_U32, 1);
    }
    tb.put_str(Some(prof_name));
    let mut pb = PaPropBag::default();
    pb.set_kv("device.description", &dname);
    pb.set_kv("device.api", if is_bt { "bluez" } else { "alsa" });
    pb.set_kv("salsa.device.id", &did);
    tb.put_proplist(Some(&pb));

    if ver >= 26 {
        let n_ports = u32::from(has_pb) + u32::from(has_cap);
        tb.put_u32(PA_TAG_U32, n_ports);
        if has_pb {
            tb.put_str(Some(if is_bt {
                "headset-output"
            } else {
                "analog-output"
            }));
            tb.put_str(Some(&dname));
            tb.put_u32(PA_TAG_U32, 9900);
            tb.put_u32(PA_TAG_U32, 2);
            tb.put_u8(PA_TAG_U8, 1);
            tb.put_proplist(Some(&pb));
            tb.put_u32(PA_TAG_U32, 1);
            tb.put_str(Some(prof_name));
            if ver >= 27 {
                tb.put_u64_raw(PA_TAG_S64, 0);
            }
            if ver >= 34 {
                tb.put_str(None);
                tb.put_u32(PA_TAG_U32, if is_bt { 6 } else { 2 });
            }
        }
        if has_cap {
            tb.put_str(Some("analog-input"));
            tb.put_str(Some(&dname));
            tb.put_u32(PA_TAG_U32, 9900);
            tb.put_u32(PA_TAG_U32, 2);
            tb.put_u8(PA_TAG_U8, 2);
            tb.put_proplist(Some(&pb));
            tb.put_u32(PA_TAG_U32, 1);
            tb.put_str(Some(prof_name));
            if ver >= 27 {
                tb.put_u64_raw(PA_TAG_S64, 0);
            }
            if ver >= 34 {
                tb.put_str(None);
                tb.put_u32(PA_TAG_U32, 4);
            }
        }
    }
}

fn milli_to_cvol(milli: u32, channels: u8) -> PaCVolume {
    let ch = channels.clamp(1, 32);
    let pv = pct_to_pa_vol((milli.min(1500) + 5) / 10);
    let mut vol = PaCVolume {
        channels: ch,
        values: [0; 32],
    };
    for v in &mut vol.values[..ch as usize] {
        *v = pv;
    }
    vol
}

fn put_client_fields(tb: &mut TagBuilder, ver: u32, idx: u32, name: &str, props: &PaPropBag) {
    tb.put_u32(PA_TAG_U32, idx);
    tb.put_str(Some(name));
    tb.put_u32(PA_TAG_U32, 0);
    tb.put_str(Some("Salsa-LASAPI"));
    if ver >= 13 {
        tb.put_proplist(Some(props));
    }
}

fn put_sink_input_fields(
    tb: &mut TagBuilder,
    ver: u32,
    idx: u32,
    nm: &str,
    client_index: u32,
    sink_idx: u32,
    ss: &PaSampleSpec,
    map: &PaChannelMap,
    vol: &PaCVolume,
    props: &PaPropBag,
    muted: bool,
    corked: bool,
) {
    tb.put_u32(PA_TAG_U32, idx);
    tb.put_str(Some(nm));
    tb.put_u32(PA_TAG_U32, 0);
    tb.put_u32(PA_TAG_U32, client_index);
    tb.put_u32(PA_TAG_U32, sink_idx);
    tb.put_ss(ss);
    tb.put_map(map);
    tb.put_cvol(vol);
    tb.put_u64_raw(PA_TAG_USEC, 2000);
    tb.put_u64_raw(PA_TAG_USEC, 2000);
    tb.put_str(Some("linear"));
    tb.put_str(Some("Salsa-LASAPI"));
    if ver >= 11 {
        tb.put_bool(muted);
    }
    if ver >= 13 {
        tb.put_proplist(Some(props));
    }
    if ver >= 19 {
        tb.put_bool(corked);
    }
    if ver >= 20 {
        tb.put_bool(true);
        tb.put_bool(true);
    }
    if ver >= 21 {
        tb.put_pcm_format_spec(ss, map);
    }
}

fn put_sink_input_entry(
    tb: &mut TagBuilder,
    ver: u32,
    s: &PaStreamMeta,
    muted: bool,
    corked: bool,
) {
    let nm = if !s.name.is_empty() {
        s.name.as_str()
    } else if !s.client_name.is_empty() {
        s.client_name.as_str()
    } else {
        "Salsa Stream"
    };
    put_sink_input_fields(
        tb,
        ver,
        s.index,
        nm,
        s.client_index,
        s.target_dev_idx,
        &s.ss,
        &s.map,
        &s.vol,
        &s.props,
        muted,
        corked,
    );
}

fn put_source_output_entry(
    tb: &mut TagBuilder,
    ver: u32,
    s: &PaStreamMeta,
    muted: bool,
    corked: bool,
) {
    let nm = if !s.name.is_empty() {
        s.name.as_str()
    } else if !s.client_name.is_empty() {
        s.client_name.as_str()
    } else {
        "Salsa Capture"
    };
    tb.put_u32(PA_TAG_U32, s.index);
    tb.put_str(Some(nm));
    tb.put_u32(PA_TAG_U32, 0);
    tb.put_u32(PA_TAG_U32, s.client_index);
    tb.put_u32(PA_TAG_U32, s.target_dev_idx);
    tb.put_ss(&s.ss);
    tb.put_map(&s.map);
    tb.put_u64_raw(PA_TAG_USEC, 2000);
    tb.put_u64_raw(PA_TAG_USEC, 2000);
    tb.put_str(Some("linear"));
    tb.put_str(Some("Salsa-LASAPI"));
    if ver >= 13 {
        tb.put_proplist(Some(&s.props));
    }
    if ver >= 19 {
        tb.put_bool(corked);
    }
    if ver >= 22 {
        tb.put_cvol(&s.vol);
        tb.put_bool(muted);
        tb.put_bool(true);
        tb.put_bool(true);
        tb.put_pcm_format_spec(&s.ss, &s.map);
    }
}

#[inline]
fn pa_catmull_rom(ym1: f32, y0: f32, y1: f32, y2: f32, frac: f64) -> f32 {
    let t = frac as f32;
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * y0)
        + (-ym1 + y1) * t
        + (2.0 * ym1 - 5.0 * y0 + 4.0 * y1 - y2) * t2
        + (-ym1 + 3.0 * y0 - 3.0 * y1 + y2) * t3)
}

fn stream_save_hist(rs: &PaRtStream, cur: &mut RtCursor, consumed: u64, new_rd: u64) {
    if consumed == 0 {
        return;
    }
    let m1 = new_rd.saturating_sub(1) as usize;
    let (m1l, m1r) = rs.ring_get(m1);
    if consumed >= 2 {
        let m2 = new_rd.saturating_sub(2) as usize;
        let (m2l, m2r) = rs.ring_get(m2);
        cur.hist_l[0] = m2l;
        cur.hist_r[0] = m2r;
    } else if cur.have_hist {
        cur.hist_l[0] = cur.hist_l[1];
        cur.hist_r[0] = cur.hist_r[1];
    } else {
        cur.hist_l[0] = m1l;
        cur.hist_r[0] = m1r;
    }
    cur.hist_l[1] = m1l;
    cur.hist_r[1] = m1r;
    cur.have_hist = true;
}

fn emit_fade_out(
    rs: &PaRtStream,
    cur: &mut RtCursor,
    stereo_out: &mut [f32],
    offset: usize,
    rem: usize,
    engine_rate: u32,
    doi_cap: usize,
) {
    if rem == 0 {
        cur.last_mix_l = 0.0;
        cur.last_mix_r = 0.0;
        return;
    }
    let target = (if engine_rate >= 4000 {
        engine_rate / 2000
    } else {
        24
    })
    .max(16) as usize;
    let fade_n = rem.min(target);
    let l0 = cur.last_mix_l;
    let r0 = cur.last_mix_r;
    if l0 == 0.0 && r0 == 0.0 {
        let zero_end = (offset + rem).min(doi_cap);
        for k in offset..zero_end {
            rs.period_l[k].store(0, Ordering::Relaxed);
            rs.period_r[k].store(0, Ordering::Relaxed);
        }
        return;
    }
    for j in 0..rem {
        let mut fl = 0.0f32;
        let mut fr = 0.0f32;
        if j < fade_n {
            let g = ((fade_n - 1 - j) as f32) / (fade_n as f32);
            fl = l0 * g;
            fr = r0 * g;
            let oidx = (offset + j) * 2;
            if oidx + 1 < stereo_out.len() {
                stereo_out[oidx] += fl;
                stereo_out[oidx + 1] += fr;
            }
        }
        if offset + j < doi_cap {
            rs.period_l[offset + j].store(fl.to_bits(), Ordering::Relaxed);
            rs.period_r[offset + j].store(fr.to_bits(), Ordering::Relaxed);
        }
    }
    let cur_pf = rs.period_frames.load(Ordering::Relaxed) as usize;
    if offset + fade_n > cur_pf {
        rs.period_frames
            .store((offset + fade_n).min(doi_cap) as u32, Ordering::Relaxed);
    }
    cur.last_mix_l = 0.0;
    cur.last_mix_r = 0.0;
}

// ----------------------------------------------------------------------------
// C ABI Exports matching salsa_net::c_abi
// ----------------------------------------------------------------------------

fn bind_loopback_tcp_and_write_port(
    cookie_valid: bool,
) -> (Option<TcpListener>, Option<u16>, Option<String>) {
    if !cookie_valid {
        return (None, None, None);
    }
    let Ok(tcp) = TcpListener::bind(("127.0.0.1", 0)) else {
        return (None, None, None);
    };
    if tcp.set_nonblocking(true).is_err() {
        return (None, None, None);
    }
    let Ok(addr) = tcp.local_addr() else {
        return (None, None, None);
    };
    let port = addr.port();
    if port == 0 {
        return (None, None, None);
    }
    let uid = current_uid();
    let home = user_home_dir();
    if home == "/tmp" || !is_safe_owner_dir(Path::new(&home), uid, false) {
        return (None, None, None);
    }
    let cfg_dir = format!("{home}/.config/pulse");
    if !ensure_owned_dir_0700(&cfg_dir, uid) {
        return (None, None, None);
    }
    let port_path = format!("{cfg_dir}/salsa-tcp-port");
    let tmp_path = format!("{port_path}.{}", rand_tmp_suffix());
    let mut wrote = false;
    if let Ok(mut f) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(unix_sys::O_NOFOLLOW | unix_sys::O_CLOEXEC)
        .open(&tmp_path)
    {
        if writeln!(f, "{port}").is_ok()
            && f.flush().is_ok()
            && f.sync_all().is_ok()
            && fs::rename(&tmp_path, &port_path).is_ok()
        {
            wrote = true;
        }
    }
    if !wrote {
        let _ = fs::remove_file(&tmp_path);
        return (None, None, None);
    }
    (Some(tcp), Some(port), Some(port_path))
}

pub fn start_pa_server(host: Arc<dyn PaEngineHost>) -> Option<PaServerHandle> {
    let uid = current_uid();
    let xdg = match std::env::var("XDG_RUNTIME_DIR") {
        Ok(v) if !v.is_empty() && is_safe_owner_dir(Path::new(&v), uid, true) => v,
        _ => format!("/run/user/{uid}"),
    };
    let pulse_dir = format!("{xdg}/pulse");
    if !ensure_owned_dir_0700(&pulse_dir, uid) {
        return None;
    }
    let sock_path = format!("{pulse_dir}/native");
    let pid_path = format!("{pulse_dir}/pid");

    let listener = bind_unix_listener_0600(&sock_path)?;
    if let Ok(mut pf) = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(unix_sys::O_NOFOLLOW | unix_sys::O_CLOEXEC)
        .open(&pid_path)
    {
        let _ = writeln!(pf, "{}", current_pid());
    }
    let Some(wake_efd) = new_eventfd() else {
        let _ = fs::remove_file(&sock_path);
        let _ = fs::remove_file(&pid_path);
        return None;
    };
    let wake_raw = wake_efd.as_raw_fd();
    let rt = Arc::new(PaRtShared::new(wake_raw));
    let mut state_val = PaState::new(host, sock_path.clone(), pid_path.clone());
    let (tcp_listener, tcp_port, tcp_port_path) =
        bind_loopback_tcp_and_write_port(state_val.cookie_valid);
    state_val.refresh_device_cache_force();
    {
        let dl = state_val.cached_devices;
        for id in collect_pa_facility_ids(&dl, SUB_EVENT_CARD) {
            state_val.stable_pa_idx(PA_FAC_CARD, &id);
        }
        for id in collect_pa_facility_ids(&dl, SUB_EVENT_SINK) {
            state_val.stable_pa_idx(PA_FAC_SINK, &id);
        }
        for id in collect_pa_facility_ids(&dl, SUB_EVENT_SOURCE) {
            state_val.stable_pa_idx(PA_FAC_SOURCE, &id);
        }
    }
    let state = Arc::new(Mutex::new(state_val));
    let vol_lock = Arc::new(Mutex::new(()));
    let running = Arc::new(AtomicBool::new(true));

    let t_run = running.clone();
    let t_st = state.clone();
    let t_rt = rt.clone();
    let t_vl = vol_lock.clone();
    let Ok(thread) = thread::Builder::new()
        .name("salsa-pa-srv".to_string())
        .spawn(move || server_thread_main(listener, tcp_listener, wake_raw, t_run, t_st, t_rt, t_vl))
    else {
        let _ = fs::remove_file(&sock_path);
        let _ = fs::remove_file(&pid_path);
        if let Some(ref p) = tcp_port_path {
            let _ = fs::remove_file(p);
        }
        return None;
    };

    if let Some(port) = tcp_port {
        println!(
            "[Salsa-PA] Native PulseAudio v35 Rust server bound at {sock_path} (+ loopback tcp4:127.0.0.1:{port})"
        );
    } else {
        println!("[Salsa-PA] Native PulseAudio v35 Rust server bound at {sock_path}");
    }
    Some(PaServerHandle {
        state,
        rt,
        vol_file_lock: vol_lock,
        running,
        wake_efd,
        sock_path,
        pid_path,
        tcp_port,
        tcp_port_path,
        thread: Mutex::new(Some(thread)),
    })
}

impl PaServerHandle {
    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
        eventfd_signal(self.wake_efd.as_raw_fd());
        if let Some(th) = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = th.join();
        }
        {
            let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
            for ci in 0..PA_MAX_CLIENTS {
                if st.clients[ci].in_use {
                    st.close_client(ci, &self.rt);
                }
            }
        }
        let _ = fs::remove_file(&self.sock_path);
        let _ = fs::remove_file(&self.pid_path);
        if let (Some(port), Some(ref ppath)) = (self.tcp_port, &self.tcp_port_path) {
            if let Ok(f) = OpenOptions::new()
                .read(true)
                .custom_flags(unix_sys::O_NOFOLLOW | unix_sys::O_NONBLOCK | unix_sys::O_CLOEXEC)
                .open(ppath)
            {
                if let Ok(meta) = f.metadata() {
                    if meta.is_file() && meta.len() <= 32 {
                        let mut buf = [0u8; 32];
                        if let Ok(n) = (&f).read(&mut buf) {
                            if core::str::from_utf8(&buf[..n])
                                .ok()
                                .and_then(|s| s.trim().parse::<u16>().ok())
                                == Some(port)
                            {
                                let _ = fs::remove_file(ppath);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn notify_change(&self) {
        if !self.running.load(Ordering::Relaxed) {
            return;
        }
        let mut new_st: LasapiEngineStatus = unsafe { mem::zeroed() };
        let mut new_dl: LasapiDeviceList = unsafe { mem::zeroed() };
        let host = {
            let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(&st.host)
        };
        host.fill_status(&mut new_st);
        host.enumerate_devices(&mut new_dl);

        let h = self;
        {
            let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        let old_dl = st.cached_devices;
        st.cached_status = new_st;
        st.cached_devices = new_dl;
        st.devices_cache_valid = true;

        let sink_idx = st.find_active_sink_idx();
        let src_idx = st.find_active_source_idx();
        let act_pb = c_str_to_string(&new_st.active_playback_id);
        let card_idx = st.find_card_idx_for_id(&act_pb);
        emit_facility_diff(&mut st, SUB_EVENT_CARD, &old_dl, &new_dl, card_idx);
        emit_facility_diff(&mut st, SUB_EVENT_SINK, &old_dl, &new_dl, sink_idx);
        emit_facility_diff(&mut st, SUB_EVENT_SOURCE, &old_dl, &new_dl, src_idx);
        let active_mon_idx = st.find_monitor_src_idx_for_sink(sink_idx);

        for si in 0..PA_MAX_STREAMS {
            if !st.streams[si].in_use {
                continue;
            }
            let s_idx = st.streams[si].index;
            if st.streams[si].is_playback {
                let tgt = st.streams[si].target_dev_idx;
                let cur_valid = st.sink_idx_to_salsa_id(tgt).is_some();
                if st.streams[si].pinned_route && cur_valid {
                    st.stream_update_device_id(si);
                    continue;
                }
                if !cur_valid {
                    st.streams[si].pinned_route = false;
                }
                if tgt != sink_idx {
                    st.streams[si].target_dev_idx = sink_idx;
                    st.stream_update_device_id(si);
                    st.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_CHANGE, s_idx);
                } else {
                    st.stream_update_device_id(si);
                }
            } else if st.streams[si].direct_on_input != PA_INVALID_INDEX
                && active_mon_idx != PA_INVALID_INDEX
                && st.streams[si].target_dev_idx != active_mon_idx
            {
                st.streams[si].target_dev_idx = active_mon_idx;
                st.stream_update_device_id(si);
                st.broadcast_sub_event(SUB_EVENT_SOURCE_OUTPUT, SUB_EVENT_CHANGE, s_idx);
            } else {
                st.stream_update_device_id(si);
            }
        }
        let mut changed_lasapi: [u32; LASAPI_MAX_SESSIONS] = [0; LASAPI_MAX_SESSIONS];
        let mut cw_cnt = 0usize;
        for w in &mut st.lasapi_proxies {
            if w.active && w.target_dev_idx != sink_idx {
                w.target_dev_idx = sink_idx;
                if cw_cnt < changed_lasapi.len() {
                    changed_lasapi[cw_cnt] = w.pa_stream_idx;
                    cw_cnt += 1;
                }
            }
        }
        for &w_idx in &changed_lasapi[..cw_cnt] {
            st.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_CHANGE, w_idx);
        }
            st.broadcast_sub_event(SUB_EVENT_SERVER, SUB_EVENT_CHANGE, PA_INVALID_INDEX);
        }
        h.rt.signal_wake();
    }

    pub fn notify_volume(&self, is_capture: bool) {
        let h = self;
        if !h.running.load(Ordering::Relaxed) {
            return;
        }
        let mut new_st: LasapiEngineStatus = unsafe { mem::zeroed() };
        let host = {
            let st = h.state.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(&st.host)
        };
        host.fill_status(&mut new_st);
    {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        st.cached_status = new_st;
        if is_capture {
            st.hw_src_pa_vol_valid = false;
        } else {
            st.sink_pa_vol_valid = false;
        }
        let idx = if is_capture {
            st.find_active_source_idx()
        } else {
            st.find_active_sink_idx()
        };
        st.broadcast_sub_event(
            if is_capture {
                SUB_EVENT_SOURCE
            } else {
                SUB_EVENT_SINK
            },
            SUB_EVENT_CHANGE,
            idx,
        );
    }
    h.rt.signal_wake();
}

    pub fn mix_playback(&self, stereo_out: &mut [f32], frames: u32, engine_rate: u32) {
        let h = self;
        if frames == 0 || engine_rate == 0 || stereo_out.len() < (frames as usize) * 2 {
            return;
        }
    let mut wake_needed = false;
    let doi_cap = if h.rt.has_active_doi.load(Ordering::Relaxed) {
        (frames as usize).min(PA_MAX_DOI_PERIOD)
    } else {
        0
    };
    let mut best_rate = 0u32;
    let mut best_bits = 0u32;

    for rs in &h.rt.streams {
        if rs.active.load(Ordering::Acquire) == 0 || !rs.is_playback.load(Ordering::Relaxed) {
            continue;
        }
        let Ok(mut cur) = rs.cursor.try_lock() else {
            continue;
        };
        let fseq = rs.flush_seq.load(Ordering::Acquire);
        if fseq != cur.seen_flush_seq {
            cur.reset_state();
            cur.seen_flush_seq = fseq;
        }

        rs.mix_peak_l.store(0, Ordering::Relaxed);
        rs.mix_peak_r.store(0, Ordering::Relaxed);
        rs.period_frames.store(0, Ordering::Relaxed);

        if rs.in_prebuf.load(Ordering::Acquire) != 0 || rs.corked.load(Ordering::Relaxed) != 0 {
            if cur.last_mix_l != 0.0 || cur.last_mix_r != 0.0 {
                for j in 0..doi_cap {
                    rs.period_l[j].store(0, Ordering::Relaxed);
                    rs.period_r[j].store(0, Ordering::Relaxed);
                }
                emit_fade_out(rs, &mut cur, stereo_out, 0, frames as usize, engine_rate, doi_cap);
            }
            continue;
        }

        let wr = rs.wr_frame.load(Ordering::Acquire);
        let mut rd = rs.rd_frame.load(Ordering::Relaxed);
        let avail = wr.saturating_sub(rd);
        if avail == 0 {
            if cur.last_mix_l != 0.0 || cur.last_mix_r != 0.0 {
                for j in 0..doi_cap {
                    rs.period_l[j].store(0, Ordering::Relaxed);
                    rs.period_r[j].store(0, Ordering::Relaxed);
                }
                emit_fade_out(rs, &mut cur, stereo_out, 0, frames as usize, engine_rate, doi_cap);
            }
            if rs.draining.load(Ordering::Relaxed) == 0 {
                rs.underrun_baseline_bytes
                    .store(rs.read_index.load(Ordering::Relaxed), Ordering::Relaxed);
                if rs.prebuf.load(Ordering::Relaxed) > 0 {
                    rs.in_prebuf.store(1, Ordering::Release);
                    rs.underflow_resync.store(true, Ordering::Release);
                }
                wake_needed = true;
            }
            continue;
        }

        let muted = rs.muted.load(Ordering::Relaxed) != 0;
        let vol = if muted {
            0.0
        } else {
            (rs.volume_milli.load(Ordering::Relaxed) as f32) / 1000.0
        };
        let mut srate = rs.sample_rate_atomic.load(Ordering::Relaxed);
        if srate == 0 {
            srate = engine_rate;
        }
        let sbits = format_sample_bits(rs.fmt.load(Ordering::Relaxed));
        if srate > best_rate || (srate == best_rate && sbits > best_bits) {
            best_rate = srate;
            best_bits = sbits;
        }

        let fsz = rs.frame_size.load(Ordering::Relaxed) as u64;
        let mut pk_l = 0.0f32;
        let mut pk_r = 0.0f32;
        let mut last_sl = cur.last_mix_l;
        let mut last_sr = cur.last_mix_r;

        if srate == engine_rate {
            let n = (avail.min(frames as u64)) as usize;
            for f in 0..n {
                let (rl, rr) = rs.ring_get(rd as usize + f);
                let sl = rl * vol;
                let sr = rr * vol;
                last_sl = sl;
                last_sr = sr;
                stereo_out[f * 2] += sl;
                stereo_out[f * 2 + 1] += sr;
                if f < doi_cap {
                    rs.period_l[f].store(sl.to_bits(), Ordering::Relaxed);
                    rs.period_r[f].store(sr.to_bits(), Ordering::Relaxed);
                }
                pk_l = pk_l.max(sl.abs());
                pk_r = pk_r.max(sr.abs());
            }
            cur.last_mix_l = last_sl;
            cur.last_mix_r = last_sr;
            if n < frames as usize {
                emit_fade_out(
                    rs,
                    &mut cur,
                    stereo_out,
                    n,
                    (frames as usize) - n,
                    engine_rate,
                    doi_cap,
                );
                if rs.draining.load(Ordering::Relaxed) == 0 && rs.prebuf.load(Ordering::Relaxed) > 0 {
                    rs.in_prebuf.store(1, Ordering::Release);
                    rs.underflow_resync.store(true, Ordering::Release);
                    wake_needed = true;
                }
            }
            rd += n as u64;
            stream_save_hist(rs, &mut cur, n as u64, rd);
            rs.read_index
                .fetch_add((n as u64) * fsz, Ordering::Relaxed);
        } else {
            let step = (srate as f64) / (engine_rate as f64);
            let mut pos = cur.resamp_phase;
            let mut f = 0usize;
            while f < frames as usize && (pos.floor() as u64) < avail {
                let p0 = rd + (pos.floor() as u64);
                let frac = pos - pos.floor();
                let (y0l, y0r) = rs.ring_get(p0 as usize);
                let (l, r) = if p0 + 2 < wr && (p0 > rd || cur.have_hist) {
                    let (lm1, rm1) = if p0 > rd {
                        rs.ring_get((p0 - 1) as usize)
                    } else {
                        (cur.hist_l[1], cur.hist_r[1])
                    };
                    let (y1l, y1r) = rs.ring_get((p0 + 1) as usize);
                    let (y2l, y2r) = rs.ring_get((p0 + 2) as usize);
                    (
                        pa_catmull_rom(lm1, y0l, y1l, y2l, frac),
                        pa_catmull_rom(rm1, y0r, y1r, y2r, frac),
                    )
                } else {
                    let p1 = if p0 + 1 < wr { p0 + 1 } else { p0 };
                    let (y1l, y1r) = rs.ring_get(p1 as usize);
                    (
                        ((y0l as f64) * (1.0 - frac) + (y1l as f64) * frac) as f32,
                        ((y0r as f64) * (1.0 - frac) + (y1r as f64) * frac) as f32,
                    )
                };
                let sl = l * vol;
                let sr = r * vol;
                last_sl = sl;
                last_sr = sr;
                stereo_out[f * 2] += sl;
                stereo_out[f * 2 + 1] += sr;
                if f < doi_cap {
                    rs.period_l[f].store(sl.to_bits(), Ordering::Relaxed);
                    rs.period_r[f].store(sr.to_bits(), Ordering::Relaxed);
                }
                pk_l = pk_l.max(sl.abs());
                pk_r = pk_r.max(sr.abs());
                pos += step;
                f += 1;
            }
            cur.last_mix_l = last_sl;
            cur.last_mix_r = last_sr;
            if f < frames as usize {
                emit_fade_out(
                    rs,
                    &mut cur,
                    stereo_out,
                    f,
                    (frames as usize) - f,
                    engine_rate,
                    doi_cap,
                );
                if rs.draining.load(Ordering::Relaxed) == 0 && rs.prebuf.load(Ordering::Relaxed) > 0 {
                    rs.in_prebuf.store(1, Ordering::Release);
                    rs.underflow_resync.store(true, Ordering::Release);
                    wake_needed = true;
                }
            }
            let mut adv = pos.floor() as u64;
            if rd + adv > wr {
                adv = wr.saturating_sub(rd);
            }
            rd += adv;
            stream_save_hist(rs, &mut cur, adv, rd);
            cur.resamp_phase = pos - (adv as f64);
            rs.read_index.fetch_add(adv * fsz, Ordering::Relaxed);
        }

        rs.period_frames.store(doi_cap as u32, Ordering::Relaxed);
        rs.mix_peak_l.store(pk_l.to_bits(), Ordering::Relaxed);
        rs.mix_peak_r.store(pk_r.to_bits(), Ordering::Relaxed);
        let fseq_after = rs.flush_seq.load(Ordering::Acquire);
        if fseq_after != fseq {
            cur.reset_state();
            cur.seen_flush_seq = fseq_after;
            let wr_now = rs.wr_frame.load(Ordering::Acquire);
            rs.rd_frame.store(wr_now, Ordering::Release);
            rd = wr_now;
        } else {
            rs.rd_frame.store(rd, Ordering::Release);
        }

        let tl = rs.tlength.load(Ordering::Relaxed) as u64;
        if wr.saturating_sub(rd) * fsz < tl {
            wake_needed = true;
        }
    }

    h.rt.dom_stream_rate.store(best_rate, Ordering::Relaxed);
    h.rt.dom_stream_bits.store(best_bits, Ordering::Relaxed);
    if wake_needed {
        h.rt.signal_wake();
    }
}

    pub fn push_capture(
        &self,
        mic_slice: Option<&[f32]>,
        mic_rate: u32,
        mon_slice: Option<&[f32]>,
        mon_rate: u32,
    ) {
        let h = self;
        let mic_frames = mic_slice.map_or(0, |s| (s.len() / 2) as u32);
        let mon_frames = mon_slice.map_or(0, |s| (s.len() / 2) as u32);
        let mic_slice = mic_slice.filter(|s| s.len() >= 2);
        let mon_slice = mon_slice.filter(|s| s.len() >= 2);
    let mut wake_needed = false;

    for rs in &h.rt.streams {
        if rs.active.load(Ordering::Acquire) == 0
            || rs.is_playback.load(Ordering::Relaxed)
            || rs.corked.load(Ordering::Relaxed) != 0
        {
            continue;
        }
        let Ok(mut cur) = rs.cursor.try_lock() else {
            continue;
        };
        let fseq = rs.flush_seq.load(Ordering::Acquire);
        if fseq != cur.seen_flush_seq {
            cur.reset_state();
            cur.seen_flush_seq = fseq;
        }

        let doi = rs.direct_on_input.load(Ordering::Relaxed);
        let has_doi = doi != PA_INVALID_INDEX;
        let is_mon = rs.is_monitor.load(Ordering::Relaxed);
        let peak_detect = rs.peak_detect.load(Ordering::Relaxed);
        let use_mon_src = is_mon || has_doi;
        let frames = if use_mon_src { mon_frames } else { mic_frames };
        let engine_rate = if use_mon_src { mon_rate } else { mic_rate };
        if frames == 0 || engine_rate == 0 {
            continue;
        }

        let mut doi_l = 0.0f32;
        let mut doi_r = 0.0f32;
        enum DoiSource<'a> {
            Pa(&'a PaRtStream),
            Lasapi(&'a PaLasapiRtSlot),
        }
        let mut doi_stream_ref: Option<DoiSource<'_>> = None;
        let mut src_slice = if is_mon { mon_slice } else { mic_slice };

        if has_doi {
            for cand in &h.rt.streams {
                if cand.active.load(Ordering::Acquire) != 0
                    && cand.is_playback.load(Ordering::Relaxed)
                    && cand.index.load(Ordering::Relaxed) == doi
                    && cand.corked.load(Ordering::Relaxed) == 0
                {
                    doi_l = f32::from_bits(cand.mix_peak_l.load(Ordering::Relaxed));
                    doi_r = f32::from_bits(cand.mix_peak_r.load(Ordering::Relaxed));
                    if !peak_detect && cand.period_frames.load(Ordering::Relaxed) >= frames {
                        doi_stream_ref = Some(DoiSource::Pa(cand));
                    }
                    break;
                }
            }
            if doi_stream_ref.is_none() {
                for ws in &h.rt.lasapi_slots {
                    if ws.active.load(Ordering::Acquire)
                        && ws.pa_stream_idx.load(Ordering::Acquire) == doi
                    {
                        doi_l = f32::from_bits(ws.mix_peak_l.load(Ordering::Relaxed));
                        doi_r = f32::from_bits(ws.mix_peak_r.load(Ordering::Relaxed));
                        if !peak_detect && ws.period_frames.load(Ordering::Acquire) >= frames {
                            doi_stream_ref = Some(DoiSource::Lasapi(ws));
                        }
                        break;
                    }
                }
            }
            if !peak_detect {
                src_slice = None;
            }
        } else if !peak_detect && src_slice.is_none() {
            continue;
        }

        let get_src = |idx: usize| -> Option<(f32, f32)> {
            if let Some(ref ds) = doi_stream_ref {
                let (pl, pr) = match ds {
                    DoiSource::Pa(s) => (&s.period_l, &s.period_r),
                    DoiSource::Lasapi(w) => (&w.period_l, &w.period_r),
                };
                if idx < pl.len() {
                    return Some((
                        f32::from_bits(pl[idx].load(Ordering::Relaxed)),
                        f32::from_bits(pr[idx].load(Ordering::Relaxed)),
                    ));
                }
            }
            if let Some(sl) = src_slice {
                if idx * 2 + 1 < sl.len() {
                    return Some((sl[idx * 2], sl[idx * 2 + 1]));
                }
            }
            None
        };
        let have_src = doi_stream_ref.is_some() || src_slice.is_some();

        let s_muted = rs.muted.load(Ordering::Relaxed) != 0;
        let s_gain = if s_muted {
            0.0
        } else {
            ((rs.volume_milli.load(Ordering::Relaxed) as f32) / 1000.0).min(2.0)
        };
        let mut mon_gain = 1.0f32;
        if is_mon && !has_doi {
            let mm = h.rt.mon_muted.load(Ordering::Relaxed) != 0;
            let mv = h.rt.mon_vol_milli.load(Ordering::Relaxed);
            mon_gain = if mm { 0.0 } else { ((mv as f32) / 1000.0).min(2.0) };
        }
        let target_gain = (s_gain * mon_gain).min(2.0);
        let mut cur_gain = cur.cap_smooth_gain;
        let gain_step = (target_gain - cur_gain) / (frames as f32);

        let mut wr = rs.wr_frame.load(Ordering::Relaxed);
        let rd = rs.rd_frame.load(Ordering::Acquire);
        let mut srate = rs.sample_rate_atomic.load(Ordering::Relaxed);
        if srate == 0 {
            srate = engine_rate;
        }

        if peak_detect {
            let step = ((engine_rate as f64) / (srate as f64)).max(1.0);
            for f in 0..(frames as usize) {
                cur_gain += gain_step;
                let (sl, sr) = if has_doi {
                    (doi_l, doi_r)
                } else if let Some((l, r)) = get_src(f) {
                    (l.abs(), r.abs())
                } else {
                    (0.0, 0.0)
                };
                let al = (sl * cur_gain).min(1.0);
                let ar = (sr * cur_gain).min(1.0);
                if al > cur.peak_accum_l {
                    cur.peak_accum_l = al;
                }
                if ar > cur.peak_accum_r {
                    cur.peak_accum_r = ar;
                }
                cur.resamp_phase += 1.0;
                if cur.resamp_phase >= step {
                    if wr >= rd && (wr - rd) < PA_STREAM_RING_FRAMES as u64 {
                        rs.ring_set(wr as usize, cur.peak_accum_l, cur.peak_accum_r);
                        wr += 1;
                    }
                    cur.peak_accum_l = 0.0;
                    cur.peak_accum_r = 0.0;
                    cur.resamp_phase -= step;
                }
            }
        } else if srate == engine_rate {
            for f in 0..(frames as usize) {
                cur_gain += gain_step;
                if wr < rd || (wr - rd) >= PA_STREAM_RING_FRAMES as u64 {
                    break;
                }
                let (l, r) = get_src(f).unwrap_or((0.0, 0.0));
                rs.ring_set(
                    wr as usize,
                    (l * cur_gain).clamp(-1.0, 1.0),
                    (r * cur_gain).clamp(-1.0, 1.0),
                );
                wr += 1;
            }
            if have_src && frames > 0 {
                if let Some((hl, hr)) = get_src((frames - 1) as usize) {
                    cur.cap_hist_l = hl;
                    cur.cap_hist_r = hr;
                    cur.have_cap_hist = true;
                }
            }
        } else if frames > 0 {
            let step = (engine_rate as f64) / (srate as f64);
            let mut pos = cur.resamp_phase;
            let approx_out = ((frames as f64) / step).max(1.0);
            let out_step = (target_gain - cur_gain) / (approx_out as f32);
            let limit = (frames - 1) as f64;
            while pos < limit {
                if (out_step >= 0.0 && cur_gain < target_gain)
                    || (out_step < 0.0 && cur_gain > target_gain)
                {
                    cur_gain += out_step;
                }
                if wr >= rd && (wr - rd) < PA_STREAM_RING_FRAMES as u64 {
                    let (sl, sr) = if !have_src {
                        (0.0, 0.0)
                    } else if pos < 0.0 {
                        let frac = (pos + 1.0).clamp(0.0, 1.0);
                        let (s0l, s0r) = get_src(0).unwrap_or((0.0, 0.0));
                        let (l0, r0) = if cur.have_cap_hist {
                            (cur.cap_hist_l, cur.cap_hist_r)
                        } else {
                            (s0l, s0r)
                        };
                        (
                            (((l0 as f64) * (1.0 - frac) + (s0l as f64) * frac) as f32 * cur_gain)
                                .clamp(-1.0, 1.0),
                            (((r0 as f64) * (1.0 - frac) + (s0r as f64) * frac) as f32 * cur_gain)
                                .clamp(-1.0, 1.0),
                        )
                    } else {
                        let i0 = pos as usize;
                        let i1 = if i0 + 1 < frames as usize { i0 + 1 } else { i0 };
                        let frac = pos - (i0 as f64);
                        let (a0l, a0r) = get_src(i0).unwrap_or((0.0, 0.0));
                        let (a1l, a1r) = get_src(i1).unwrap_or((0.0, 0.0));
                        (
                            (((a0l as f64) * (1.0 - frac) + (a1l as f64) * frac) as f32
                                * cur_gain)
                                .clamp(-1.0, 1.0),
                            (((a0r as f64) * (1.0 - frac) + (a1r as f64) * frac) as f32
                                * cur_gain)
                                .clamp(-1.0, 1.0),
                        )
                    };
                    rs.ring_set(wr as usize, sl, sr);
                    wr += 1;
                }
                pos += step;
            }
            if let Some((hl, hr)) = get_src((frames - 1) as usize) {
                cur.cap_hist_l = hl;
                cur.cap_hist_r = hr;
                cur.have_cap_hist = true;
            }
            cur.resamp_phase = (pos - (frames as f64)).max(-1.0);
        }

        cur.cap_smooth_gain = target_gain;
        rs.wr_frame.store(wr, Ordering::Release);
        let fsz = rs.frame_size.load(Ordering::Relaxed) as u64;
        let frag = rs.fragsize.load(Ordering::Relaxed) as u64;
        if wr >= rd && wr.saturating_sub(rd) * fsz >= frag {
            wake_needed = true;
        }
    }

    if wake_needed {
        h.rt.signal_wake();
    }
}

    pub fn fill_sessions(&self, out: &mut LasapiEngineStatus) {
        let h = self;
        let st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        let nc = st
            .clients
            .iter()
            .filter(|c| c.in_use && c.authed)
            .count() as u32;
        out.pa_clients = nc;
    for si in 0..PA_MAX_STREAMS {
        if out.num_sessions as usize >= LASAPI_MAX_SESSIONS {
            break;
        }
        let s = &st.streams[si];
        if !s.in_use {
            continue;
        }
        let rs = &h.rt.streams[si];
        let dst = &mut out.sessions[out.num_sessions as usize];
        out.num_sessions += 1;
        *dst = unsafe { mem::zeroed() };
        dst.session_id = 10000 + s.index;
        dst.client_pid = s.client_pid;
        let nm = if !s.name.is_empty() {
            s.name.as_str()
        } else if !s.client_name.is_empty() {
            s.client_name.as_str()
        } else {
            "PA Client"
        };
        copy_str_to_c(&mut dst.client_name, nm);
        dst.share_mode = 2;
        dst.is_capture = u32::from(!s.is_playback);
        dst.sample_rate = s.ss.rate;
        dst.channels = u32::from(s.ss.channels);
        dst.active = u32::from(
            rs.active.load(Ordering::Relaxed) != 0 && rs.corked.load(Ordering::Relaxed) == 0,
        );
        dst.volume_milli = rs.volume_milli.load(Ordering::Relaxed);
        dst.muted = u32::from(rs.muted.load(Ordering::Relaxed) != 0);
        dst.routing_flags = u32::from(s.pinned_route);
        copy_str_to_c(&mut dst.target_device, &s.device_id);
        let wr = rs.wr_frame.load(Ordering::Relaxed);
        let rd = rs.rd_frame.load(Ordering::Relaxed);
        dst.buffered_frames = wr.saturating_sub(rd);
    }
}

    pub fn control_stream(
        &self,
        session_id: u32,
        mut vol_pct: i32,
        mute_mode: i32,
        target_device: Option<&str>,
    ) -> i32 {
        if session_id < 10000 {
            return -1;
        }
        let h = self;
        let target_idx = session_id - 10000;
        if vol_pct > 150 {
            vol_pct = 150;
        }
        let tgt_dev = target_device.filter(|s| !s.is_empty()).map(|s| s.to_string());

    let mut save_info: Option<(String, Vec<AppVolEntry>)> = None;
    {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some(si) = st.find_stream_by_index(target_idx) else {
            return -1;
        };

        if let Some(ref td) = tgt_dev {
            st.refresh_device_cache();
            let is_pb = st.streams[si].is_playback;
            if td == "auto" {
                st.streams[si].pinned_route = false;
                st.streams[si].target_dev_idx = if is_pb {
                    st.find_active_sink_idx()
                } else {
                    st.find_active_source_idx()
                };
            } else {
                let mut matched = false;
                let mut dir_cnt = 0u32;
                let ndev = st.cached_dev_count();
                for i in 0..ndev {
                    if (st.cached_devices.devices[i].is_capture == 0) != is_pb {
                        continue;
                    }
                    dir_cnt += 1;
                    let did = c_str_to_string(&st.cached_devices.devices[i].id);
                    if &did == td {
                        st.streams[si].target_dev_idx = st.stable_pa_idx(
                            if is_pb { PA_FAC_SINK } else { PA_FAC_SOURCE },
                            &did,
                        );
                        st.streams[si].pinned_route = true;
                        matched = true;
                        break;
                    }
                }
                if !matched && is_pb && dir_cnt == 0 && td == "virtual" {
                    st.streams[si].target_dev_idx = st.stable_pa_idx(PA_FAC_SINK, "virtual");
                    st.streams[si].pinned_route = true;
                    matched = true;
                }
                if !matched {
                    return -1;
                }
            }
            st.stream_update_device_id(si);
        }

        let mut changed = false;
        let rs = &h.rt.streams[si];
        if vol_pct >= 0 {
            let pv = pct_to_pa_vol(vol_pct as u32);
            let ch = (st.streams[si].vol.channels as usize).min(32);
            for v in &mut st.streams[si].vol.values[..ch] {
                *v = pv;
            }
            rs.volume_milli
                .store((vol_pct as u32) * 10, Ordering::Relaxed);
            changed = true;
        }
        if (0..=2).contains(&mute_mode) {
            let new_m = if mute_mode == 2 {
                u32::from(rs.muted.load(Ordering::Relaxed) == 0)
            } else {
                u32::from(mute_mode != 0)
            };
            rs.muted.store(new_m, Ordering::Relaxed);
            changed = true;
        }
        if changed {
            st.record_app_volume(si, &h.rt);
            let (p, snap) = st.app_vols_snapshot();
            if !p.is_empty() && !snap.is_empty() {
                save_info = Some((p, snap));
            }
        }
        let is_pb = st.streams[si].is_playback;
        let sidx = st.streams[si].index;
        st.broadcast_sub_event(
            if is_pb {
                SUB_EVENT_SINK_INPUT
            } else {
                SUB_EVENT_SOURCE_OUTPUT
            },
            SUB_EVENT_CHANGE,
            sidx,
        );
    }
    if let Some((p, snap)) = save_info {
        write_app_volumes_disk(&h.vol_file_lock, &p, &snap);
    }
    h.rt.signal_wake();
    0
}

    pub fn set_stream_volume(&self, session_id: u32, vol_pct: u32) -> i32 {
        self.control_stream(session_id, vol_pct as i32, -1, None)
    }

    pub fn debug_snapshot_json(&self) -> (JsonValue, JsonValue) {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut clients_json = Vec::new();
        for c in &st.clients {
            if !c.in_use {
                continue;
            }
            let bin = c.props.get_str("application.process.binary").unwrap_or_default();
            clients_json.push(JsonValue::obj(vec![
                ("index", JsonValue::Int(i64::from(c.index))),
                ("pid", JsonValue::Int(i64::from(c.pid))),
                ("name", JsonValue::Str(c.name.clone())),
                ("binary", JsonValue::Str(bin)),
                ("version", JsonValue::Int(i64::from(c.version))),
                ("authed", JsonValue::Int(i64::from(c.authed))),
                ("sub_mask", JsonValue::Int(i64::from(c.sub_mask))),
                ("out_count", JsonValue::Int(i64::try_from(c.out_q.len()).unwrap_or(i64::MAX))),
            ]));
        }

        let mut streams_json = Vec::new();
        for si in 0..PA_MAX_STREAMS {
            let s = &st.streams[si];
            if !s.in_use {
                continue;
            }
            let rs = &self.rt.streams[si];
            let fsz = if s.frame_size != 0 { s.frame_size } else { 4 };
            let wr = rs.wr_frame.load(Ordering::Relaxed);
            let rd = rs.rd_frame.load(Ordering::Relaxed);
            let peak_l = f32::from_bits(rs.mix_peak_l.load(Ordering::Relaxed));
            let peak_r = f32::from_bits(rs.mix_peak_r.load(Ordering::Relaxed));
            streams_json.push(JsonValue::obj(vec![
                ("index", JsonValue::Int(i64::from(s.index))),
                ("client_index", JsonValue::Int(i64::from(s.client_index))),
                ("pid", JsonValue::Int(i64::from(s.client_pid))),
                ("name", JsonValue::Str(s.name.clone())),
                ("device_id", JsonValue::Str(s.device_id.clone())),
                ("is_playback", JsonValue::Int(i64::from(s.is_playback))),
                ("is_monitor", JsonValue::Int(i64::from(s.is_monitor))),
                ("peak_detect", JsonValue::Int(i64::from(s.peak_detect))),
                ("direct_on_input", JsonValue::Int(i64::from(s.direct_on_input))),
                ("target_dev_idx", JsonValue::Int(i64::from(s.target_dev_idx))),
                ("rate", JsonValue::Int(i64::from(s.ss.rate))),
                ("channels", JsonValue::Int(i64::from(s.ss.channels))),
                ("format", JsonValue::Int(i64::from(s.ss.format))),
                ("active", JsonValue::Int(i64::from(rs.active.load(Ordering::Relaxed) as u8))),
                ("corked", JsonValue::Int(i64::from(rs.corked.load(Ordering::Relaxed) as u8))),
                (
                    "in_prebuf",
                    JsonValue::Int(i64::from(rs.in_prebuf.load(Ordering::Relaxed) != 0)),
                ),
                (
                    "draining",
                    JsonValue::Int(i64::from(rs.draining.load(Ordering::Relaxed) as u8)),
                ),
                ("muted", JsonValue::Int(i64::from(rs.muted.load(Ordering::Relaxed) as u8))),
                (
                    "volume_milli",
                    JsonValue::Int(i64::from(rs.volume_milli.load(Ordering::Relaxed))),
                ),
                ("frame_size", JsonValue::Int(i64::from(fsz))),
                ("buffered_frames", JsonValue::from_u64(wr.saturating_sub(rd))),
                (
                    "ring_cap_frames",
                    JsonValue::Int(i64::try_from(PA_STREAM_RING_FRAMES).unwrap_or(i64::MAX)),
                ),
                ("tlength_frames", JsonValue::Int(i64::from(s.tlength / fsz))),
                ("prebuf_frames", JsonValue::Int(i64::from(s.prebuf / fsz))),
                ("minreq_frames", JsonValue::Int(i64::from(s.minreq / fsz))),
                ("fragsize_frames", JsonValue::Int(i64::from(s.fragsize / fsz))),
                ("mix_peak_l", JsonValue::rounded_float(f64::from(peak_l), 10_000.0)),
                ("mix_peak_r", JsonValue::rounded_float(f64::from(peak_r), 10_000.0)),
                ("write_index", JsonValue::from_u64(rs.write_index.load(Ordering::Relaxed))),
                ("read_index", JsonValue::from_u64(rs.read_index.load(Ordering::Relaxed))),
            ]));
        }
        (JsonValue::Array(clients_json), JsonValue::Array(streams_json))
    }

    pub fn reset_clients(&self) -> i32 {
        let h = self;
    let mut flushed = 0i32;
    {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        for ci in 0..PA_MAX_CLIENTS {
            if st.clients[ci].in_use {
                st.close_client(ci, &h.rt);
                flushed += 1;
            }
        }
    }
    h.rt.signal_wake();
    flushed
}

    pub fn register_lasapi_session(
        &self,
        slot_idx: u32,
        session_id: u32,
        pid: u32,
        name: &str,
        rate: u32,
        channels: u32,
    ) {
        if (slot_idx as usize) >= LASAPI_MAX_SESSIONS {
            return;
        }
        let h = self;
        let nm = if name.is_empty() { "AudioSession" } else { name };
    {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        st.register_lasapi_proxy(
            slot_idx as usize,
            session_id,
            pid,
            nm,
            rate,
            channels,
            &h.rt,
        );
    }
    h.rt.signal_wake();
}

    pub fn unregister_lasapi_session(&self, slot_idx: u32, session_id: u32) {
        if (slot_idx as usize) >= LASAPI_MAX_SESSIONS {
            return;
        }
        let h = self;
    {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        st.unregister_lasapi_proxy(slot_idx as usize, session_id, &h.rt);
    }
    h.rt.signal_wake();
}

    pub fn update_lasapi_session_vol(
        &self,
        slot_idx: u32,
        session_id: u32,
        volume_milli: u32,
        muted: bool,
    ) {
        if (slot_idx as usize) >= LASAPI_MAX_SESSIONS {
            return;
        }
        let h = self;
    let si = slot_idx as usize;
    let clamped = volume_milli.min(1500);
    let pa_idx = {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        let w = &mut st.lasapi_proxies[si];
        if !w.active || w.session_id != session_id {
            return;
        }
        let ws = &h.rt.lasapi_slots[si];
        ws.volume_milli.store(clamped, Ordering::Relaxed);
        ws.muted.store(muted, Ordering::Relaxed);
        w.vol = milli_to_cvol(clamped, w.ss.channels.max(1));
        let idx = w.pa_stream_idx;
        st.broadcast_sub_event(SUB_EVENT_SINK_INPUT, SUB_EVENT_CHANGE, idx);
        idx
    };
    let _ = pa_idx;
    h.rt.signal_wake();
}

    pub fn lasapi_doi_mask(&self) -> u32 {
        self.rt.lasapi_doi_mask.load(Ordering::Acquire)
    }

    pub fn push_lasapi_period(
        &self,
        slot_idx: u32,
        src: &[f32],
        frames: u32,
        channels: u32,
    ) {
        if frames == 0 || channels == 0 {
            return;
        }
        let si = slot_idx as usize;
        let h = self;
        if si >= h.rt.lasapi_slots.len() {
            return;
        }
        let ws = &h.rt.lasapi_slots[si];
        if !ws.active.load(Ordering::Acquire) {
            return;
        }
        let ch = channels as usize;
        let need = (frames as usize).saturating_mul(ch);
        if src.len() < need {
            return;
        }
        let n = (frames as usize).min(PA_MAX_DOI_PERIOD);
    let mut pk_l = 0.0f32;
    let mut pk_r = 0.0f32;
    for f in 0..n {
        let base = f * ch;
        let l = src[base];
        let r = if ch > 1 { src[base + 1] } else { l };
        pk_l = pk_l.max(l.abs());
        pk_r = pk_r.max(r.abs());
        ws.period_l[f].store(l.to_bits(), Ordering::Relaxed);
        ws.period_r[f].store(r.to_bits(), Ordering::Relaxed);
    }
    ws.mix_peak_l.store(pk_l.to_bits(), Ordering::Relaxed);
    ws.mix_peak_r.store(pk_r.to_bits(), Ordering::Relaxed);
    ws.period_frames.store(n as u32, Ordering::Release);
}

    pub fn poll_lasapi_ctl(&self, slot_idx: u32, last_seq: &mut u64) -> Option<(u32, u32)> {
        let si = slot_idx as usize;
        let ws = self.rt.lasapi_slots.get(si)?;
        let seq = ws.pa_ctl_seq.load(Ordering::Acquire);
        if seq == *last_seq {
            return None;
        }
        *last_seq = seq;
        Some((
            ws.pa_target_vol_milli.load(Ordering::Relaxed),
            u32::from(ws.pa_target_muted.load(Ordering::Relaxed)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn null_host() -> Arc<dyn PaEngineHost> {
        Arc::new(NullPaEngineHost)
    }

    #[test]
    fn default_names_and_virtual_aliases_prefer_active_hw_device() {
        let mut st = PaState::new(null_host(), String::new(), String::new());
        st.devices_cache_valid = true;
        st.cached_devices.num_devices = 2;

        // Playback device 0: Bluetooth headphones
        copy_str_to_c(&mut st.cached_devices.devices[0].id, "bt_AA_BB");
        copy_str_to_c(&mut st.cached_devices.devices[0].name, "Bose QC Ultra 2 HP");
        st.cached_devices.devices[0].is_capture = 0;
        st.cached_devices.devices[0].is_bluetooth = 1;
        st.cached_devices.devices[0].is_active = 1;

        // Capture device 1: USB Scarlett mic
        copy_str_to_c(&mut st.cached_devices.devices[1].id, "hw:2,0");
        copy_str_to_c(&mut st.cached_devices.devices[1].name, "Scarlett 2i2");
        st.cached_devices.devices[1].is_capture = 1;
        st.cached_devices.devices[1].is_active = 1;

        // Case 1: status reports "virtual" even though hardware playback device exists.
        copy_str_to_c(&mut st.cached_status.active_playback_id, "virtual");
        assert_eq!(st.get_default_sink_pa_name(), "salsa_out.bt_AA_BB");

        // Case 2: status reports stale "hw:6,0" missing from cached_devices.
        copy_str_to_c(&mut st.cached_status.active_playback_id, "hw:6,0");
        copy_str_to_c(&mut st.cached_status.active_capture_id, "hw:9,0");
        assert_eq!(st.get_default_sink_pa_name(), "salsa_out.bt_AA_BB");
        assert_eq!(st.get_default_source_pa_name(), "salsa_in.hw_2_0");

        // Virtual aliases resolve cleanly to the active HW sink and its monitor.
        let resolved_sink = st.resolve_sink_idx_and_name(PA_INVALID_INDEX, Some("salsa_out.virtual"));
        assert_eq!(
            resolved_sink.as_ref().map(|(_, nm)| nm.as_str()),
            Some("salsa_out.bt_AA_BB")
        );
        let resolved_mon =
            st.resolve_source_idx_and_name(PA_INVALID_INDEX, Some("salsa_out.virtual.monitor"));
        assert_eq!(
            resolved_mon.as_ref().map(|(_, nm, mon)| (nm.as_str(), *mon)),
            Some(("salsa_out.bt_AA_BB.monitor", true))
        );
    }

    #[test]
    fn tcp_auth_requires_matching_cookie_without_peercred() {
        use std::os::unix::net::UnixStream;

        let mut st = PaState::new(null_host(), String::new(), String::new());
        st.cookie = [0xA5u8; PA_NATIVE_COOKIE_LEN];
        st.cookie_valid = true;

        let Ok((sock_a, _sock_b)) = UnixStream::pair() else {
            panic!("UnixStream::pair failed");
        };
        let owned: OwnedFd = sock_a.into();
        st.accept_client_fd(owned, None, true);
        assert!(st.clients[0].in_use);
        assert!(!st.clients[0].uid_valid);
        assert!(!st.clients[0].authed);

        let rt = Arc::new(PaRtShared::new(-1));
        let vol_lock = Mutex::new(());

        let mut bad_tb = TagBuilder::new(320);
        bad_tb.put_u32(PA_TAG_U32, CMD_AUTH);
        bad_tb.put_u32(PA_TAG_U32, 1);
        bad_tb.put_u32(PA_TAG_U32, PA_PROTO_VERSION);
        bad_tb.put_arbitrary(&[0x5Au8; PA_NATIVE_COOKIE_LEN]);
        st.handle_control_packet(0, &bad_tb.into_vec(), &rt, &vol_lock);
        assert!(!st.clients[0].authed);

        let mut good_tb = TagBuilder::new(320);
        good_tb.put_u32(PA_TAG_U32, CMD_AUTH);
        good_tb.put_u32(PA_TAG_U32, 2);
        good_tb.put_u32(PA_TAG_U32, PA_PROTO_VERSION);
        good_tb.put_arbitrary(&st.cookie.clone());
        st.handle_control_packet(0, &good_tb.into_vec(), &rt, &vol_lock);
        assert!(st.clients[0].authed);
    }

    #[test]
    fn playback_attr_and_underrun_rearm_prebuf_for_smooth_video_start() {
        let mut sm = PaStreamMeta {
            frame_size: 8,
            ..PaStreamMeta::default()
        };
        sm.ss.rate = 48_000;
        sm.ss.channels = 2;

        // Low-latency browser video defaults (preb=INVALID) get >= 15ms prebuf and >= 30ms tlength
        enforce_playback_attr(&mut sm, PA_INVALID_INDEX, 1920 * 4, PA_INVALID_INDEX, PA_INVALID_INDEX);
        assert!(sm.tlength >= 1440 * 8, "tlength={}", sm.tlength);
        assert!(sm.prebuf >= 720 * 8, "prebuf={}", sm.prebuf);
        assert!(sm.prebuf <= sm.tlength.saturating_sub(sm.minreq));

        // Explicit prebuf=0 remains zero
        let mut sm_zero = PaStreamMeta {
            frame_size: 8,
            ..PaStreamMeta::default()
        };
        sm_zero.ss.rate = 48_000;
        sm_zero.ss.channels = 2;
        enforce_playback_attr(&mut sm_zero, PA_INVALID_INDEX, 960 * 8, PA_INVALID_INDEX, 0);
        assert_eq!(sm_zero.prebuf, 0);

        // Partial/empty underrun re-arms in_prebuf cleanly and fades smoothly instead of stuttering
        let h = PaServerHandle {
            state: Arc::new(Mutex::new(PaState::new(
                null_host(),
                String::new(),
                String::new(),
            ))),
            rt: Arc::new(PaRtShared::new(-1)),
            vol_file_lock: Arc::new(Mutex::new(())),
            running: Arc::new(AtomicBool::new(false)),
            wake_efd: crate::unix_sys::new_eventfd().expect("eventfd"),
            sock_path: String::new(),
            pid_path: String::new(),
            tcp_port: None,
            tcp_port_path: None,
            thread: Mutex::new(None),
        };
        let rt_ref = Arc::clone(&h.rt);
        let rs = &rt_ref.streams[0];
        rs.active.store(1, Ordering::Release);
        rs.is_playback.store(true, Ordering::Release);
        rs.frame_size.store(8, Ordering::Relaxed);
        rs.sample_rate_atomic.store(48_000, Ordering::Relaxed);
        rs.tlength.store(sm.tlength, Ordering::Relaxed);
        rs.prebuf.store(sm.prebuf, Ordering::Relaxed);
        rs.in_prebuf.store(0, Ordering::Release);
        rs.wr_frame.store(64, Ordering::Release);
        rs.rd_frame.store(0, Ordering::Release);
        for i in 0..64 {
            rs.ring_set(i, 0.25, 0.25);
        }
        let mut out = vec![0.0f32; 256];
        h.mix_playback(&mut out, 128, 48_000);
        assert_eq!(rs.rd_frame.load(Ordering::Acquire), 64);
        assert_eq!(rs.in_prebuf.load(Ordering::Acquire), 1);
        assert!(rs.underflow_resync.load(Ordering::Acquire));
    }

    #[test]
    fn lasapi_session_exposes_pa_sink_input_and_feeds_doi_record_stream() {
        use std::os::unix::net::UnixStream;

        let h = PaServerHandle {
            state: Arc::new(Mutex::new(PaState::new(
                null_host(),
                String::new(),
                String::new(),
            ))),
            rt: Arc::new(PaRtShared::new(-1)),
            vol_file_lock: Arc::new(Mutex::new(())),
            running: Arc::new(AtomicBool::new(false)),
            wake_efd: crate::unix_sys::new_eventfd().expect("eventfd"),
            sock_path: String::new(),
            pid_path: String::new(),
            tcp_port: None,
            tcp_port_path: None,
            thread: Mutex::new(None),
        };

        // 1. Register a LASAPI playback session (e.g., FL64.exe:Salsa-WaveRT-ASIO @ 192kHz).
        h.register_lasapi_session(2, 77, 451327, "FL64.exe:Salsa-WaveRT-ASIO", 192_000, 2);

        // Authenticate a mock Discord capture PA client.
        let Ok((sock_a, _sock_b)) = UnixStream::pair() else {
            panic!("UnixStream::pair failed");
        };
        let owned: OwnedFd = sock_a.into();
        let (pa_stream_idx, _pa_client_idx) = {
            let mut st = h.state.lock().unwrap();
            st.cookie = [0x11u8; PA_NATIVE_COOKIE_LEN];
            st.cookie_valid = true;
            st.accept_client_fd(owned, Some((1000, 340351)), false);

            let mut auth_tb = TagBuilder::new(320);
            auth_tb.put_u32(PA_TAG_U32, CMD_AUTH);
            auth_tb.put_u32(PA_TAG_U32, 1);
            auth_tb.put_u32(PA_TAG_U32, PA_PROTO_VERSION);
            auth_tb.put_arbitrary(&st.cookie.clone());
            st.handle_control_packet(0, &auth_tb.into_vec(), &h.rt, &h.vol_file_lock);
            assert!(st.clients[0].authed);

            // Verify CMD_GET_SINK_INPUT_INFO_LIST includes the LASAPI session with its PID.
            let mut si_tb = TagBuilder::new(32);
            si_tb.put_u32(PA_TAG_U32, CMD_GET_SINK_INPUT_INFO_LIST);
            si_tb.put_u32(PA_TAG_U32, 2);
            st.clients[0].out_q.clear();
            st.handle_control_packet(0, &si_tb.into_vec(), &h.rt, &h.vol_file_lock);
            let reply = st.clients[0].out_q.pop_front().expect("sink input list reply");
            let w = &st.lasapi_proxies[2];
            assert!(w.active);
            assert_eq!(w.client_pid, 451327);
            assert_eq!(w.stream_props.get_str("application.process.id").as_deref(), Some("451327"));
            assert_eq!(w.client_props.get_str("application.process.binary").as_deref(), Some("FL64.exe"));
            assert!(!reply.data.is_empty());

            // Verify fill_sessions does NOT duplicate the LASAPI proxy in Salsa's PA sessions list.
            let mut eng_st: LasapiEngineStatus = unsafe { mem::zeroed() };
            drop(st);
            h.fill_sessions(&mut eng_st);
            assert_eq!(eng_st.num_sessions, 0);

            let st = h.state.lock().unwrap();
            (st.lasapi_proxies[2].pa_stream_idx, st.lasapi_proxies[2].pa_client_idx)
        };

        // 2. Attach a direct_on_input (pa_stream_set_monitor_stream) record stream to the LASAPI proxy.
        {
            let mut st = h.state.lock().unwrap();
            let mut rec_tb = TagBuilder::new(256);
            rec_tb.put_u32(PA_TAG_U32, CMD_CREATE_RECORD_STREAM);
            rec_tb.put_u32(PA_TAG_U32, 3);
            rec_tb.put_ss(&PaSampleSpec {
                format: FMT_FLOAT32LE,
                channels: 2,
                rate: 48_000,
            });
            rec_tb.put_map(&std_channel_map(2));
            rec_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
            rec_tb.put_str(None);
            rec_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX); // maxlength
            rec_tb.put_bool(false); // corked
            rec_tb.put_u32(PA_TAG_U32, 128 * 8); // fragsize
            for _ in 0..7 {
                rec_tb.put_bool(false);
            }
            rec_tb.put_bool(false); // peak_detect
            rec_tb.put_bool(false); // adjust_latency
            rec_tb.put_proplist(None);
            rec_tb.put_u32(PA_TAG_U32, pa_stream_idx); // direct_on_input!
            st.handle_control_packet(0, &rec_tb.into_vec(), &h.rt, &h.vol_file_lock);
            assert!(st.streams[0].in_use);
            assert_eq!(st.streams[0].direct_on_input, pa_stream_idx);
        }

        // Verify LASAPI doi mask includes bit 2.
        let mask = h.lasapi_doi_mask();
        assert_eq!(mask & (1u32 << 2), 1u32 << 2);

        // 3. Push one 128-frame LASAPI period and pump capture; verify DOI record stream receives exact samples.
        let mut period_pcm = vec![0.0f32; 128 * 2];
        for f in 0..128 {
            period_pcm[f * 2] = 0.25;
            period_pcm[f * 2 + 1] = -0.5;
        }
        h.push_lasapi_period(2, &period_pcm, 128, 2);
        h.push_capture(None, 48_000, Some(&period_pcm), 48_000);
        let wr = h.rt.streams[0].wr_frame.load(Ordering::Acquire);
        assert_eq!(wr, 128);
        let (sl, sr) = h.rt.streams[0].ring_get(10);
        assert!((sl - 0.25).abs() < 1e-5);
        assert!((sr + 0.5).abs() < 1e-5);

        // 4. PA SET_SINK_INPUT_MUTE + SET_SINK_INPUT_VOLUME forward lock-free control to LASAPI poller.
        {
            let mut st = h.state.lock().unwrap();
            let mut mute_tb = TagBuilder::new(64);
            mute_tb.put_u32(PA_TAG_U32, CMD_SET_SINK_INPUT_MUTE);
            mute_tb.put_u32(PA_TAG_U32, 4);
            mute_tb.put_u32(PA_TAG_U32, pa_stream_idx);
            mute_tb.put_bool(true);
            st.handle_control_packet(0, &mute_tb.into_vec(), &h.rt, &h.vol_file_lock);
        }
        let mut seq = 0u64;
        let (_vm, mu) = h.poll_lasapi_ctl(2, &mut seq).expect("ctl");
        assert_eq!(mu, 1);

        // 5. Unregister LASAPI session clears mask and proxy state cleanly.
        h.unregister_lasapi_session(2, 77);
        assert_eq!(h.lasapi_doi_mask() & (1u32 << 2), 0);
        assert!(!h.state.lock().unwrap().lasapi_proxies[2].active);
    }

    #[test]
    fn playback_stream_inherits_pid_and_emit_fade_out_zeros_doi_tail() {
        use std::os::unix::net::UnixStream;

        let mut st = PaState::new(null_host(), String::new(), String::new());
        st.cookie = [0x22u8; PA_NATIVE_COOKIE_LEN];
        st.cookie_valid = true;
        let Ok((sock_a, _sock_b)) = UnixStream::pair() else {
            panic!("UnixStream::pair failed");
        };
        let owned: OwnedFd = sock_a.into();
        st.accept_client_fd(owned, Some((1000, 99123)), false);
        let rt = Arc::new(PaRtShared::new(-1));
        let vol_lock = Mutex::new(());

        let mut auth_tb = TagBuilder::new(320);
        auth_tb.put_u32(PA_TAG_U32, CMD_AUTH);
        auth_tb.put_u32(PA_TAG_U32, 1);
        auth_tb.put_u32(PA_TAG_U32, PA_PROTO_VERSION);
        auth_tb.put_arbitrary(&st.cookie.clone());
        st.handle_control_packet(0, &auth_tb.into_vec(), &rt, &vol_lock);

        let mut pb_tb = TagBuilder::new(256);
        pb_tb.put_u32(PA_TAG_U32, CMD_CREATE_PLAYBACK_STREAM);
        pb_tb.put_u32(PA_TAG_U32, 2);
        pb_tb.put_ss(&PaSampleSpec {
            format: FMT_FLOAT32LE,
            channels: 2,
            rate: 48_000,
        });
        pb_tb.put_map(&std_channel_map(2));
        pb_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
        pb_tb.put_str(None);
        pb_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX); // maxl
        pb_tb.put_bool(false); // corked
        pb_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX); // tlength
        pb_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX); // prebuf
        pb_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX); // minreq
        pb_tb.put_u32(PA_TAG_U32, 0); // syncid
        pb_tb.put_cvol(&milli_to_cvol(1000, 2));
        for _ in 0..7 {
            pb_tb.put_bool(false);
        }
        pb_tb.put_bool(false);
        pb_tb.put_bool(false);
        pb_tb.put_proplist(None);
        st.handle_control_packet(0, &pb_tb.into_vec(), &rt, &vol_lock);
        assert!(st.streams[0].in_use);
        assert_eq!(st.streams[0].props.get_str("application.process.id").as_deref(), Some("99123"));

        // Seed stale DOI tail in period_l/r and verify emit_fade_out from zero zeros it cleanly.
        let rs = &rt.streams[0];
        for i in 10..64 {
            rs.period_l[i].store(0.75f32.to_bits(), Ordering::Relaxed);
            rs.period_r[i].store((-0.75f32).to_bits(), Ordering::Relaxed);
        }
        let mut cur = RtCursor::default();
        let mut out = vec![0.0f32; 128 * 2];
        emit_fade_out(rs, &mut cur, &mut out, 10, 54, 48_000, 128);
        assert_eq!(f32::from_bits(rs.period_l[30].load(Ordering::Relaxed)), 0.0);
        assert_eq!(f32::from_bits(rs.period_r[30].load(Ordering::Relaxed)), 0.0);
    }

    #[test]
    fn adjust_latency_preserves_3x_minreq_and_cork_sends_started() {
        use std::os::unix::net::UnixStream;

        let mut st = PaState::new(null_host(), String::new(), String::new());
        st.cookie = [0x33u8; PA_NATIVE_COOKIE_LEN];
        st.cookie_valid = true;
        let Ok((sock_a, _sock_b)) = UnixStream::pair() else {
            panic!("UnixStream::pair failed");
        };
        let owned: OwnedFd = sock_a.into();
        st.accept_client_fd(owned, Some((1000, 99124)), false);
        let rt = Arc::new(PaRtShared::new(-1));
        let vol_lock = Mutex::new(());

        let mut auth_tb = TagBuilder::new(320);
        auth_tb.put_u32(PA_TAG_U32, CMD_AUTH);
        auth_tb.put_u32(PA_TAG_U32, 1);
        auth_tb.put_u32(PA_TAG_U32, PA_PROTO_VERSION);
        auth_tb.put_arbitrary(&st.cookie.clone());
        st.handle_control_packet(0, &auth_tb.into_vec(), &rt, &vol_lock);
        assert!(st.clients[0].authed);

        // 1. Mimic winepulse.drv: 30ms tlength (11520B), 10ms minreq (3840B), prebuf=0, corked=true, adjust_latency=true.
        st.clients[0].out_q.clear();
        let mut pb_tb = TagBuilder::new(256);
        pb_tb.put_u32(PA_TAG_U32, CMD_CREATE_PLAYBACK_STREAM);
        pb_tb.put_u32(PA_TAG_U32, 2);
        pb_tb.put_ss(&PaSampleSpec {
            format: FMT_FLOAT32LE,
            channels: 2,
            rate: 48_000,
        });
        pb_tb.put_map(&std_channel_map(2));
        pb_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
        pb_tb.put_str(None);
        pb_tb.put_u32(PA_TAG_U32, 11_520); // maxl
        pb_tb.put_bool(true); // corked
        pb_tb.put_u32(PA_TAG_U32, 11_520); // tlength
        pb_tb.put_u32(PA_TAG_U32, 0); // prebuf
        pb_tb.put_u32(PA_TAG_U32, 3_840); // minreq
        pb_tb.put_u32(PA_TAG_U32, 0); // syncid
        pb_tb.put_cvol(&milli_to_cvol(1000, 2));
        for _ in 0..7 {
            pb_tb.put_bool(false);
        }
        pb_tb.put_bool(false); // muted
        pb_tb.put_bool(true); // adjust_latency
        pb_tb.put_proplist(None);
        st.handle_control_packet(0, &pb_tb.into_vec(), &rt, &vol_lock);

        assert!(st.streams[0].in_use);
        assert_eq!(st.streams[0].tlength, 11_520);
        assert!(!st.streams[0].started_sent);
        let ch = st.streams[0].channel;

        let has_cmd_started = |q: &VecDeque<OutPacket>| {
            q.iter().any(|p| {
                p.data.len() > PA_PSTREAM_DESC_BYTES
                    && TagReader::new(&p.data[PA_PSTREAM_DESC_BYTES..], 0)
                        .and_then(|mut tr| tr.read_u32())
                        == Some(CMD_STARTED)
            })
        };
        assert!(!has_cmd_started(&st.clients[0].out_q));

        // 2. Uncork stream -> must emit CMD_STARTED immediately when prebuf == 0.
        st.clients[0].out_q.clear();
        let mut uncork_tb = TagBuilder::new(64);
        uncork_tb.put_u32(PA_TAG_U32, CMD_CORK_PLAYBACK_STREAM);
        uncork_tb.put_u32(PA_TAG_U32, 3);
        uncork_tb.put_u32(PA_TAG_U32, ch);
        uncork_tb.put_bool(false);
        st.handle_control_packet(0, &uncork_tb.into_vec(), &rt, &vol_lock);
        assert!(st.streams[0].started_sent);
        assert!(has_cmd_started(&st.clients[0].out_q));

        // 3. Re-cork stream -> resets started_sent so future uncork emits CMD_STARTED again.
        let mut cork_tb = TagBuilder::new(64);
        cork_tb.put_u32(PA_TAG_U32, CMD_CORK_PLAYBACK_STREAM);
        cork_tb.put_u32(PA_TAG_U32, 4);
        cork_tb.put_u32(PA_TAG_U32, ch);
        cork_tb.put_bool(true);
        st.handle_control_packet(0, &cork_tb.into_vec(), &rt, &vol_lock);
        assert!(!st.streams[0].started_sent);

        // 4. Stream created already uncorked with prebuf == 0 immediately emits CMD_STARTED.
        st.clients[0].out_q.clear();
        let mut pb2_tb = TagBuilder::new(256);
        pb2_tb.put_u32(PA_TAG_U32, CMD_CREATE_PLAYBACK_STREAM);
        pb2_tb.put_u32(PA_TAG_U32, 5);
        pb2_tb.put_ss(&PaSampleSpec {
            format: FMT_FLOAT32LE,
            channels: 2,
            rate: 48_000,
        });
        pb2_tb.put_map(&std_channel_map(2));
        pb2_tb.put_u32(PA_TAG_U32, PA_INVALID_INDEX);
        pb2_tb.put_str(None);
        pb2_tb.put_u32(PA_TAG_U32, 11_520);
        pb2_tb.put_bool(false); // corked = false
        pb2_tb.put_u32(PA_TAG_U32, 11_520);
        pb2_tb.put_u32(PA_TAG_U32, 0); // prebuf = 0
        pb2_tb.put_u32(PA_TAG_U32, 3_840);
        pb2_tb.put_u32(PA_TAG_U32, 0);
        pb2_tb.put_cvol(&milli_to_cvol(1000, 2));
        for _ in 0..7 {
            pb2_tb.put_bool(false);
        }
        pb2_tb.put_bool(false);
        pb2_tb.put_bool(true);
        pb2_tb.put_proplist(None);
        st.handle_control_packet(0, &pb2_tb.into_vec(), &rt, &vol_lock);
        assert!(st.streams[1].in_use);
        assert!(st.streams[1].started_sent);
        assert!(has_cmd_started(&st.clients[0].out_q));
    }
}
