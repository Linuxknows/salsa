// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_void};
use core::mem;
use core::ptr;
use core::slice;
use core::sync::atomic::{AtomicI32, Ordering};
use std::ffi::CStr;
use std::fs;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::sync::OnceLock;

use salsa_wire::c_abi::{
    copy_str_to_c, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_ALREADY_INITIALIZED,
    AUDCLNT_E_BUFFER_TOO_LARGE, AUDCLNT_E_NOT_INITIALIZED, E_FAIL, E_INVALIDARG, E_OUTOFMEMORY,
    LASAPI_CFG_FLAG_CAPTURE_RATE, LASAPI_CMD_BT_CONNECTION, LASAPI_CMD_CREATE_SESSION,
    LASAPI_CMD_GET_STATUS, LASAPI_CMD_INJECT_MIDI, LASAPI_CMD_LIST_DEVICES,
    LASAPI_CMD_RELOAD_CONFIG, LASAPI_CMD_SAVE_CONFIG, LASAPI_CMD_SET_BT_CODEC,
    LASAPI_CMD_SET_DEVICES, LASAPI_CMD_SET_ENGINE_CONFIG, LASAPI_CMD_SET_EQ,
    LASAPI_CMD_SET_HW_ANALOG, LASAPI_CMD_SET_MIC_APO, LASAPI_CMD_SET_PARAM_EQ,
    LASAPI_CMD_SET_SESSION_CONTROL, LASAPI_DBG_INFO, LASAPI_DBG_SHIM, LASAPI_DBG_TRACE,
    LASAPI_DBG_WARN, LASAPI_DBG_LASAPI, LASAPI_DEBUG_DGRAM_MAGIC, LASAPI_FMT_FLOAT32,
    LASAPI_FMT_S16_LE, LASAPI_FMT_S32_LE, LASAPI_MAX_CHANNELS, LASAPI_MIDI_RING_EVENTS,
    LASAPI_MIDI_RING_MASK, LASAPI_RING_FRAMES, LASAPI_RING_MAGIC, LASAPI_RING_MASK,
    LasapiBtCodecCfg, LasapiDbgDgram, LasapiDeviceList, LasapiEngineStatus, LasapiEqConfig,
    LasapiHwAnalogCfg, LasapiIpcReq, LasapiIpcResp, LasapiMicApoCfg, LasapiMidiEvent,
    LasapiParamEqExt, LasapiParamEqPayload, LasapiShmRing, WaveFormatEx, S_OK,
};
use salsa_wire::unix_sys::{
    audiodg_sock_path, close_raw_fd, connect_unix_stream_blocking, current_pid, debug_in_sock_path,
    do_poll, eventfd_drain, last_errno, memfd_size_valid, recv_blocking_exact_with_2fds,
    recv_waitall, send_all_nosignal, sendto_unix_dgram_nonblock, socket_raw, MappedMem, PollFd,
    AF_UNIX, POLLIN, SOCK_CLOEXEC, SOCK_DGRAM, SOCK_NONBLOCK,
};

pub type HRESULT = i32;
pub type AUDCLNT_SHAREMODE = u32;
pub type lasapi_sample_format_t = u32;

pub struct IAudioClient {
    client_name: [u8; 64],
    client_pid: u32,
    session_id: u32,
    initialized: bool,
    sock_fd: Option<OwnedFd>,
    shm_fd: Option<OwnedFd>,
    event_fd: Option<OwnedFd>,
    ring: Option<MappedMem>,
    channels: usize,
    buffer_frames: u32,
    locked_frames: u32,
    staging_buf: Vec<f32>,
}

static DBG_DGRAM_FD: AtomicI32 = AtomicI32::new(-1);
static DBG_IN_SOCK_CACHE: OnceLock<String> = OnceLock::new();
static PROC_COMM_CACHE: OnceLock<String> = OnceLock::new();

#[allow(clippy::too_many_arguments)]
fn dbg_emit_dgram(
    sub: u8,
    lvl: u8,
    sid: u32,
    v1: i32,
    v2: i32,
    hr: i32,
    op: &str,
    target: &str,
    detail: &str,
) {
    let mut fd = DBG_DGRAM_FD.load(Ordering::Acquire);
    if fd < 0 {
        let nfd = socket_raw(AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
        if nfd < 0 {
            return;
        }
        match DBG_DGRAM_FD.compare_exchange(-1, nfd, Ordering::Release, Ordering::Acquire) {
            Ok(_) => fd = nfd,
            Err(existing) => {
                close_raw_fd(nfd);
                fd = existing;
            }
        }
        if fd < 0 {
            return;
        }
    }

    let mut dg: LasapiDbgDgram = unsafe { mem::zeroed() };
    dg.magic = LASAPI_DEBUG_DGRAM_MAGIC;
    dg.pid = current_pid();
    dg.session_id = sid;
    dg.v1 = v1;
    dg.v2 = v2;
    dg.hr = hr;
    dg.sub = sub;
    dg.lvl = lvl;
    copy_str_to_c(&mut dg.op, op);
    copy_str_to_c(&mut dg.target, target);
    copy_str_to_c(&mut dg.detail, detail);

    let path = DBG_IN_SOCK_CACHE.get_or_init(debug_in_sock_path);
    let bytes = unsafe {
        slice::from_raw_parts(
            (&dg as *const LasapiDbgDgram).cast::<u8>(),
            mem::size_of::<LasapiDbgDgram>(),
        )
    };
    let _ = sendto_unix_dgram_nonblock(fd, path, bytes);
}

fn get_process_comm() -> String {
    PROC_COMM_CACHE
        .get_or_init(|| {
            if let Ok(raw) = fs::read_to_string("/proc/self/comm") {
                let trimmed = raw.trim_end_matches(['\r', '\n']);
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
            "app".to_string()
        })
        .clone()
}

#[inline]
unsafe fn cstr_to_str<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(p) }.to_str().unwrap_or("")
    }
}

#[inline]
fn cbuf_to_str(buf: &[u8]) -> &str {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    core::str::from_utf8(&buf[..end]).unwrap_or("")
}

#[inline]
unsafe fn as_bytes<T>(val: &T) -> &[u8] {
    unsafe { slice::from_raw_parts((val as *const T).cast::<u8>(), mem::size_of::<T>()) }
}

#[inline]
unsafe fn as_bytes_mut<T>(val: &mut T) -> &mut [u8] {
    unsafe { slice::from_raw_parts_mut((val as *mut T).cast::<u8>(), mem::size_of::<T>()) }
}

#[inline]
unsafe fn ring_ptr(client: &IAudioClient) -> *mut LasapiShmRing {
    client
        .ring
        .as_ref()
        .map_or(ptr::null_mut(), |m| m.as_ptr().cast::<LasapiShmRing>().cast_mut())
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_DebugTraceClient(
    sub: u8,
    lvl: u8,
    sid: u32,
    op: *const c_char,
    target: *const c_char,
    detail: *const c_char,
) {
    dbg_emit_dgram(
        sub,
        lvl,
        sid,
        0,
        0,
        0,
        unsafe { cstr_to_str(op) },
        unsafe { cstr_to_str(target) },
        unsafe { cstr_to_str(detail) },
    );
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_ActivateAudioClient(
    client_name: *const c_char,
    ppAudioClient: *mut *mut IAudioClient,
) -> HRESULT {
    if ppAudioClient.is_null() {
        return E_INVALIDARG;
    }

    let proc_comm = get_process_comm();
    let cname_str = unsafe { cstr_to_str(client_name) };
    let final_name = if cname_str.is_empty()
        || cname_str == "default"
        || cname_str == "LASAPI-Client"
        || cname_str == proc_comm
    {
        proc_comm
    } else if cname_str.starts_with(&proc_comm) {
        cname_str.to_string()
    } else {
        format!("{proc_comm}:{cname_str}")
    };

    let mut client = Box::new(IAudioClient {
        client_name: [0u8; 64],
        client_pid: current_pid(),
        session_id: 0,
        initialized: false,
        sock_fd: None,
        shm_fd: None,
        event_fd: None,
        ring: None,
        channels: 0,
        buffer_frames: 0,
        locked_frames: 0,
        staging_buf: Vec::new(),
    });
    copy_str_to_c(&mut client.client_name, &final_name);
    unsafe { *ppAudioClient = Box::into_raw(client) };
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_GetMixFormat(
    client: *mut IAudioClient,
    pFormat: *mut WaveFormatEx,
) -> HRESULT {
    if client.is_null() || pFormat.is_null() {
        return E_INVALIDARG;
    }
    let fmt = WaveFormatEx {
        w_format_tag: LASAPI_FMT_FLOAT32 as u16,
        n_channels: 2,
        n_samples_per_sec: 48000,
        w_bits_per_sample: 32,
        n_block_align: 2 * mem::size_of::<f32>() as u16,
        n_avg_bytes_per_sec: 48000 * 2 * mem::size_of::<f32>() as u32,
        cb_size: 0,
    };
    unsafe { *pFormat = fmt };
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_Initialize(
    client: *mut IAudioClient,
    ShareMode: AUDCLNT_SHAREMODE,
    StreamFlags: u32,
    BufferFrameCount: u32,
    PeriodicityFrames: u32,
    pFormat: *const WaveFormatEx,
) -> HRESULT {
    if client.is_null() || pFormat.is_null() {
        return E_INVALIDARG;
    }
    let c = unsafe { &mut *client };
    if c.initialized {
        return AUDCLNT_E_ALREADY_INITIALIZED;
    }
    let fmt = unsafe { &*pFormat };
    let cname = cbuf_to_str(&c.client_name).to_string();

    let sock_path = audiodg_sock_path();
    let Some(sock) = connect_unix_stream_blocking(&sock_path) else {
        let err = last_errno();
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            0,
            fmt.n_samples_per_sec as i32,
            i32::from(fmt.n_channels),
            E_FAIL,
            "LASAPI_INIT_FAIL",
            &cname,
            &format!("connect({sock_path}) failed: errno={err}"),
        );
        return E_FAIL;
    };

    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_CREATE_SESSION;
    req.client_name = c.client_name;
    req.client_pid = c.client_pid;
    req.share_mode = ShareMode;
    req.stream_flags = StreamFlags;
    req.sample_rate = if fmt.n_samples_per_sec != 0 {
        fmt.n_samples_per_sec
    } else {
        48000
    };
    req.channels = if fmt.n_channels != 0 {
        u32::from(fmt.n_channels)
    } else {
        2
    };
    req.format = if fmt.w_format_tag != 0 {
        u32::from(fmt.w_format_tag)
    } else {
        LASAPI_FMT_FLOAT32
    };
    req.requested_buffer_frames = if BufferFrameCount != 0 {
        BufferFrameCount
    } else {
        2048
    };
    req.requested_period_frames = if PeriodicityFrames != 0 {
        PeriodicityFrames
    } else {
        256
    };

    if send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) }).is_err() {
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            0,
            req.sample_rate as i32,
            req.channels as i32,
            E_FAIL,
            "LASAPI_INIT_FAIL",
            &cname,
            "IPC send CREATE_SESSION failed",
        );
        return E_FAIL;
    }

    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    let Some((opt_shm, opt_ev)) =
        recv_blocking_exact_with_2fds(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) })
    else {
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            0,
            req.sample_rate as i32,
            req.channels as i32,
            E_FAIL,
            "LASAPI_INIT_FAIL",
            &cname,
            "recv_response_with_fds failed",
        );
        return E_FAIL;
    };

    let (Some(shm_fd), Some(ev_fd)) = (opt_shm, opt_ev) else {
        let fail_hr = if resp.hr != S_OK { resp.hr } else { E_FAIL };
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            resp.session_id,
            req.sample_rate as i32,
            req.channels as i32,
            fail_hr,
            "LASAPI_INIT_FAIL",
            &cname,
            &format!("audiodg rejected hr=0x{:08x}", resp.hr as u32),
        );
        return fail_hr;
    };

    if resp.hr != S_OK {
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            resp.session_id,
            req.sample_rate as i32,
            req.channels as i32,
            resp.hr,
            "LASAPI_INIT_FAIL",
            &cname,
            &format!("audiodg rejected hr=0x{:08x}", resp.hr as u32),
        );
        return resp.hr;
    }

    if !memfd_size_valid(
        shm_fd.as_raw_fd(),
        0,
        mem::size_of::<LasapiShmRing>() as u64,
    ) {
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            resp.session_id,
            0,
            0,
            E_FAIL,
            "LASAPI_INIT_FAIL",
            &cname,
            "shm ring size validation failed",
        );
        return E_FAIL;
    }

    let Some(ring) = MappedMem::map_fd(shm_fd.as_raw_fd(), mem::size_of::<LasapiShmRing>(), true, 0)
    else {
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            resp.session_id,
            0,
            0,
            E_OUTOFMEMORY,
            "LASAPI_INIT_FAIL",
            &cname,
            "mmap shm ring failed",
        );
        return E_OUTOFMEMORY;
    };

    let ring_hdr = ring.as_ptr().cast::<LasapiShmRing>();
    let (ring_magic, ring_ch, ring_buf_frames) = unsafe {
        (
            ptr::addr_of!((*ring_hdr).magic).read(),
            ptr::addr_of!((*ring_hdr).channels).read(),
            ptr::addr_of!((*ring_hdr).buffer_frames).read(),
        )
    };
    if ring_magic != LASAPI_RING_MAGIC
        || ring_ch == 0
        || ring_ch as usize > LASAPI_MAX_CHANNELS
        || ring_buf_frames == 0
        || ring_buf_frames as usize > LASAPI_RING_FRAMES
    {
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            resp.session_id,
            ring_ch as i32,
            ring_buf_frames as i32,
            E_FAIL,
            "LASAPI_INIT_FAIL",
            &cname,
            "shm ring header validation failed",
        );
        return E_FAIL;
    }

    let sb_frames = if resp.actual_buffer_frames != 0 {
        resp.actual_buffer_frames.min(ring_buf_frames)
    } else {
        ring_buf_frames
    };
    let sb_ch = if resp.actual_channels != 0 {
        resp.actual_channels.min(ring_ch)
    } else {
        ring_ch
    };
    if sb_frames == 0 || sb_ch == 0 {
        return E_FAIL;
    }

    c.staging_buf = vec![0.0f32; (sb_frames as usize) * (sb_ch as usize)];
    c.sock_fd = Some(sock);
    c.shm_fd = Some(shm_fd);
    c.event_fd = Some(ev_fd);
    c.ring = Some(ring);
    c.channels = sb_ch as usize;
    c.buffer_frames = sb_frames;
    c.session_id = resp.session_id;
    c.initialized = true;

    dbg_emit_dgram(
        LASAPI_DBG_LASAPI,
        LASAPI_DBG_INFO,
        resp.session_id,
        resp.actual_sample_rate as i32,
        resp.actual_period_frames as i32,
        S_OK,
        "LASAPI_INIT",
        &cname,
        &format!(
            "rate={} ch={} buf={} per={} flags=0x{:x} mode={}",
            resp.actual_sample_rate,
            resp.actual_channels,
            resp.actual_buffer_frames,
            resp.actual_period_frames,
            StreamFlags,
            ShareMode
        ),
    );
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_GetBufferSize(
    client: *mut IAudioClient,
    pNumBufferFrames: *mut u32,
) -> HRESULT {
    if client.is_null() || pNumBufferFrames.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized || unsafe { ring_ptr(c) }.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    unsafe { *pNumBufferFrames = c.buffer_frames };
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_GetDevicePeriod(
    client: *mut IAudioClient,
    pDefaultPeriodFrames: *mut u32,
    pMinimumPeriodFrames: *mut u32,
) -> HRESULT {
    if client.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r = unsafe { &*r_ptr };
    if !pDefaultPeriodFrames.is_null() {
        unsafe { *pDefaultPeriodFrames = r.period_frames };
    }
    if !pMinimumPeriodFrames.is_null() {
        unsafe { *pMinimumPeriodFrames = r.period_frames };
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_GetCurrentPadding(
    client: *mut IAudioClient,
    pNumPaddingFrames: *mut u32,
) -> HRESULT {
    if client.is_null() || pNumPaddingFrames.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let (w, rd) = unsafe {
        (
            (*ptr::addr_of!((*r_ptr).write_frame)).load(Ordering::Relaxed),
            (*ptr::addr_of!((*r_ptr).read_frame)).load(Ordering::Acquire),
        )
    };
    unsafe { *pNumPaddingFrames = w.saturating_sub(rd) as u32 };
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_SetEventHandle(
    client: *mut IAudioClient,
    pEventFdOut: *mut c_int,
) -> HRESULT {
    if client.is_null() || pEventFdOut.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    unsafe {
        *pEventFdOut = c.event_fd.as_ref().map_or(-1, AsRawFd::as_raw_fd);
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_Start(client: *mut IAudioClient) -> HRESULT {
    if client.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r = unsafe { &*r_ptr };
    r.active.store(1, Ordering::Release);
    dbg_emit_dgram(
        LASAPI_DBG_LASAPI,
        LASAPI_DBG_INFO,
        c.session_id,
        0,
        0,
        S_OK,
        "LASAPI_START",
        cbuf_to_str(&c.client_name),
        "stream active=1",
    );
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_Stop(client: *mut IAudioClient) -> HRESULT {
    if client.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r = unsafe { &*r_ptr };
    r.active.store(0, Ordering::Release);
    dbg_emit_dgram(
        LASAPI_DBG_LASAPI,
        LASAPI_DBG_INFO,
        c.session_id,
        0,
        0,
        S_OK,
        "LASAPI_STOP",
        cbuf_to_str(&c.client_name),
        "stream active=0",
    );
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_Reset(client: *mut IAudioClient) -> HRESULT {
    if client.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r = unsafe { &*r_ptr };
    if r.is_capture != 0 {
        let cw = r.cap_write_frame.load(Ordering::Acquire);
        r.cap_read_frame.store(cw, Ordering::Release);
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            LASAPI_DBG_TRACE,
            c.session_id,
            0,
            0,
            S_OK,
            "LASAPI_RESET",
            cbuf_to_str(&c.client_name),
            &format!("cap_read_frame=cap_write_frame={cw}"),
        );
    } else {
        let rd = r.read_frame.load(Ordering::Acquire);
        r.write_frame.store(rd, Ordering::Release);
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            LASAPI_DBG_TRACE,
            c.session_id,
            0,
            0,
            S_OK,
            "LASAPI_RESET",
            cbuf_to_str(&c.client_name),
            &format!("write_frame=read_frame={rd}"),
        );
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioRenderClient_GetBuffer(
    client: *mut IAudioClient,
    NumFramesRequested: u32,
    ppData: *mut *mut f32,
) -> HRESULT {
    if client.is_null() || ppData.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let mut padding = 0u32;
    let pad_hr = unsafe { IAudioClient_GetCurrentPadding(client, &mut padding) };
    if pad_hr != S_OK {
        return pad_hr;
    }
    let c = unsafe { &mut *client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() || c.channels == 0 || c.buffer_frames == 0 {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    if padding.saturating_add(NumFramesRequested) > c.buffer_frames {
        return AUDCLNT_E_BUFFER_TOO_LARGE;
    }
    let Some(need) = (NumFramesRequested as usize).checked_mul(c.channels) else {
        return AUDCLNT_E_BUFFER_TOO_LARGE;
    };
    if need > c.staging_buf.len() {
        return AUDCLNT_E_BUFFER_TOO_LARGE;
    }

    c.locked_frames = NumFramesRequested;
    unsafe { *ppData = c.staging_buf.as_mut_ptr() };
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioRenderClient_ReleaseBuffer(
    client: *mut IAudioClient,
    NumFramesWritten: u32,
    dwFlags: u32,
) -> HRESULT {
    if client.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &mut *client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    if NumFramesWritten > c.locked_frames {
        return E_INVALIDARG;
    }

    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() || c.channels == 0 {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let ch = c.channels;
    let total_frames = NumFramesWritten as usize;
    let Some(need) = total_frames.checked_mul(ch) else {
        return E_INVALIDARG;
    };
    if need > c.staging_buf.len() {
        return E_INVALIDARG;
    }
    let w = unsafe { (*ptr::addr_of!((*r_ptr).write_frame)).load(Ordering::Relaxed) };
    let pcm_base = unsafe { ptr::addr_of_mut!((*r_ptr).pcm_data.0) }.cast::<f32>();

    if total_frames > 0 {
        let start = (w as usize) & LASAPI_RING_MASK;
        let first = total_frames.min(LASAPI_RING_FRAMES - start);
        let second = total_frames - first;
        if (dwFlags & AUDCLNT_BUFFERFLAGS_SILENT) != 0 {
            unsafe {
                ptr::write_bytes(
                    pcm_base.add(start * ch),
                    0,
                    first * ch * mem::size_of::<f32>(),
                );
                if second > 0 {
                    ptr::write_bytes(pcm_base, 0, second * ch * mem::size_of::<f32>());
                }
            }
        } else {
            unsafe {
                ptr::copy_nonoverlapping(
                    c.staging_buf.as_ptr(),
                    pcm_base.add(start * ch),
                    first * ch,
                );
                if second > 0 {
                    ptr::copy_nonoverlapping(
                        c.staging_buf.as_ptr().add(first * ch),
                        pcm_base,
                        second * ch,
                    );
                }
            }
        }
    }

    unsafe {
        (*ptr::addr_of!((*r_ptr).write_frame))
            .store(w + u64::from(NumFramesWritten), Ordering::Release);
    }
    c.locked_frames = 0;
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_WaitForPeriod(
    client: *mut IAudioClient,
    timeout_ms: c_int,
) -> HRESULT {
    if client.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let ev_fd = c.event_fd.as_ref().map_or(-1, AsRawFd::as_raw_fd);
    let mut pfd = [PollFd {
        fd: ev_fd,
        events: POLLIN,
        revents: 0,
    }];
    let r = do_poll(&mut pfd, timeout_ms);
    if r > 0 && (pfd[0].revents & POLLIN) != 0 {
        eventfd_drain(ev_fd);
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioRenderClient_WriteFrames(
    client: *mut IAudioClient,
    src: *const c_void,
    frames: u32,
    src_fmt: lasapi_sample_format_t,
) -> HRESULT {
    if client.is_null() || src.is_null() {
        return E_INVALIDARG;
    }
    if src_fmt != LASAPI_FMT_FLOAT32
        && src_fmt != LASAPI_FMT_S16_LE
        && src_fmt != LASAPI_FMT_S32_LE
    {
        return E_INVALIDARG;
    }
    let (ch, buf_frames, r_ptr) = {
        let c = unsafe { &*client };
        if !c.initialized || c.channels == 0 || c.buffer_frames == 0 {
            return E_INVALIDARG;
        }
        let rp = unsafe { ring_ptr(c) };
        if rp.is_null() {
            return AUDCLNT_E_NOT_INITIALIZED;
        }
        (c.channels as u32, c.buffer_frames, rp)
    };

    let mut remaining = frames;
    let mut byte_ptr = src.cast::<u8>();
    let mut stall_waits = 0u32;
    let mut prev_pad = u32::MAX;

    while remaining > 0 {
        let mut padding = 0u32;
        if unsafe { IAudioClient_GetCurrentPadding(client, &mut padding) } != S_OK {
            return E_FAIL;
        }
        let space = buf_frames.saturating_sub(padding);
        if space == 0 {
            let active = unsafe { (*ptr::addr_of!((*r_ptr).active)).load(Ordering::Acquire) };
            if padding >= prev_pad || active == 0 {
                stall_waits += 1;
            } else {
                stall_waits = 0;
            }
            prev_pad = padding;
            if (active == 0 && stall_waits >= 25) || stall_waits >= 250 {
                return E_FAIL;
            }
            let _ = unsafe { IAudioClient_WaitForPeriod(client, 20) };
            continue;
        }
        stall_waits = 0;
        prev_pad = u32::MAX;

        let chunk = remaining.min(space);
        let mut dst: *mut f32 = ptr::null_mut();
        if unsafe { IAudioRenderClient_GetBuffer(client, chunk, &mut dst) } != S_OK || dst.is_null() {
            break;
        }

        let samples = (chunk as usize) * (ch as usize);
        let dst_slice = unsafe { slice::from_raw_parts_mut(dst, samples) };
        if src_fmt == LASAPI_FMT_FLOAT32 {
            unsafe {
                ptr::copy_nonoverlapping(byte_ptr.cast::<f32>(), dst, samples);
                byte_ptr = byte_ptr.add(samples * mem::size_of::<f32>());
            }
        } else if src_fmt == LASAPI_FMT_S16_LE {
            let s16 = byte_ptr.cast::<i16>();
            for (i, out) in dst_slice.iter_mut().enumerate() {
                let v = unsafe { ptr::read_unaligned(s16.add(i)) };
                *out = f32::from(v) / 32768.0;
            }
            byte_ptr = unsafe { byte_ptr.add(samples * mem::size_of::<i16>()) };
        } else if src_fmt == LASAPI_FMT_S32_LE {
            let s32 = byte_ptr.cast::<i32>();
            for (i, out) in dst_slice.iter_mut().enumerate() {
                let v = unsafe { ptr::read_unaligned(s32.add(i)) };
                *out = (f64::from(v) / 2_147_483_648.0) as f32;
            }
            byte_ptr = unsafe { byte_ptr.add(samples * mem::size_of::<i32>()) };
        }

        let _ = unsafe { IAudioRenderClient_ReleaseBuffer(client, chunk, 0) };
        remaining -= chunk;
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioCaptureClient_GetNextPacketSize(
    client: *mut IAudioClient,
    pNumFramesInNextPacket: *mut u32,
) -> HRESULT {
    if client.is_null() || pNumFramesInNextPacket.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r = unsafe { &*r_ptr };
    let cw = r.cap_write_frame.load(Ordering::Acquire);
    let cr = r.cap_read_frame.load(Ordering::Relaxed);
    unsafe { *pNumFramesInNextPacket = cw.saturating_sub(cr) as u32 };
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioCaptureClient_ReadFrames(
    client: *mut IAudioClient,
    dst: *mut c_void,
    frames: u32,
    dst_fmt: lasapi_sample_format_t,
) -> HRESULT {
    if client.is_null() || dst.is_null() {
        return E_INVALIDARG;
    }
    if dst_fmt != LASAPI_FMT_FLOAT32
        && dst_fmt != LASAPI_FMT_S16_LE
        && dst_fmt != LASAPI_FMT_S32_LE
    {
        return E_INVALIDARG;
    }
    let c = unsafe { &*client };
    if !c.initialized || c.channels == 0 {
        return E_INVALIDARG;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }

    let r = unsafe { &*r_ptr };
    let ch = c.channels.min(LASAPI_MAX_CHANNELS);
    let bps = if dst_fmt == LASAPI_FMT_S16_LE { 2usize } else { 4usize };
    if (frames as usize)
        .checked_mul(ch)
        .and_then(|s| s.checked_mul(bps))
        .is_none()
    {
        return E_INVALIDARG;
    }
    let out_ptr = dst.cast::<u8>();

    let mut cw = r.cap_write_frame.load(Ordering::Acquire);
    let mut cr = r.cap_read_frame.load(Ordering::Relaxed);
    if cw <= cr {
        let _ = unsafe { IAudioClient_WaitForPeriod(client, 10) };
        cw = r.cap_write_frame.load(Ordering::Acquire);
        cr = r.cap_read_frame.load(Ordering::Relaxed);
    }

    let mut avail = cw.saturating_sub(cr);
    if avail > LASAPI_RING_FRAMES as u64 {
        cr = cw - (LASAPI_RING_FRAMES as u64 / 2);
        avail = cw - cr;
    }

    let to_copy = avail.min(u64::from(frames)) as u32;
    let vol = if r.muted.load(Ordering::Relaxed) != 0 {
        0.0f32
    } else {
        (r.volume_milli.load(Ordering::Relaxed) as f32) / 1000.0
    };

    for f in 0..to_copy as usize {
        let idx = (((cr + f as u64) as usize) & LASAPI_RING_MASK) * 2;
        for ch_i in 0..ch {
            let raw_s = r.cap_pcm_data.0[idx + (ch_i & 1)] * vol;
            let sample = if raw_s.is_finite() {
                raw_s.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let out_idx = f * ch + ch_i;
            if dst_fmt == LASAPI_FMT_FLOAT32 {
                unsafe { ptr::write_unaligned(out_ptr.cast::<f32>().add(out_idx), sample) };
            } else if dst_fmt == LASAPI_FMT_S16_LE {
                unsafe {
                    ptr::write_unaligned(
                        out_ptr.cast::<i16>().add(out_idx),
                        (sample * 32767.0) as i16,
                    );
                }
            } else {
                unsafe {
                    ptr::write_unaligned(
                        out_ptr.cast::<i32>().add(out_idx),
                        (sample * 2_147_483_520.0) as i32,
                    );
                }
            }
        }
    }

    if to_copy < frames {
        let zero_off = (to_copy as usize) * ch * bps;
        let zero_len = ((frames - to_copy) as usize) * ch * bps;
        unsafe { ptr::write_bytes(out_ptr.add(zero_off), 0, zero_len) };
    }

    if to_copy > 0 {
        r.cap_read_frame
            .store(cr + u64::from(to_copy), Ordering::Release);
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn ISimpleAudioVolume_SetMasterVolume(
    client: *mut IAudioClient,
    fLevel: f32,
) -> HRESULT {
    if client.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let lvl = fLevel.clamp(0.0, 2.0);
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r = unsafe { &*r_ptr };
    r.volume_milli
        .store((lvl * 1000.0) as u32, Ordering::Relaxed);
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn ISimpleAudioVolume_GetMasterVolume(
    client: *mut IAudioClient,
    pfLevel: *mut f32,
) -> HRESULT {
    if client.is_null() || pfLevel.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return AUDCLNT_E_NOT_INITIALIZED;
    }
    let r = unsafe { &*r_ptr };
    unsafe { *pfLevel = (r.volume_milli.load(Ordering::Relaxed) as f32) / 1000.0 };
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_ReadMidiEvents(
    client: *mut IAudioClient,
    events_out: *mut LasapiMidiEvent,
    max_events: u32,
) -> u32 {
    if client.is_null() || events_out.is_null() || max_events == 0 {
        return 0;
    }
    let c = unsafe { &*client };
    if !c.initialized {
        return 0;
    }
    let r_ptr = unsafe { ring_ptr(c) };
    if r_ptr.is_null() {
        return 0;
    }
    let r = unsafe { &*r_ptr };
    let w = r.midi_write_idx.load(Ordering::Acquire);
    let mut rd = r.midi_read_idx.load(Ordering::Relaxed);
    if rd > w {
        rd = w;
    } else if w - rd > LASAPI_MIDI_RING_EVENTS as u64 {
        rd = w - LASAPI_MIDI_RING_EVENTS as u64;
    }

    let mut count = 0u32;
    while rd < w && count < max_events {
        let ev = r.midi_ring[(rd as usize) & LASAPI_MIDI_RING_MASK];
        unsafe { *events_out.add(count as usize) = ev };
        count += 1;
        rd += 1;
    }
    if count > 0 || rd != r.midi_read_idx.load(Ordering::Relaxed) {
        r.midi_read_idx.store(rd, Ordering::Release);
    }
    count
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_WriteMidiEvent(
    client: *mut IAudioClient,
    ev: *const LasapiMidiEvent,
) -> HRESULT {
    if ev.is_null() {
        return E_INVALIDARG;
    }
    let ev_val = unsafe { *ev };
    if ev_val.size == 0 || ev_val.size > 3 {
        return E_INVALIDARG;
    }
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_INJECT_MIDI;
    req.midi_ev = ev_val;
    if !client.is_null() {
        let c = unsafe { &*client };
        if c.client_name[0] != 0 {
            req.client_name = c.client_name;
        }
    }
    if send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) }).is_err() {
        return E_FAIL;
    }
    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    if !recv_waitall(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) }) {
        return E_FAIL;
    }
    resp.hr
}

#[no_mangle]
pub unsafe extern "C" fn IAudioClient_Release(client: *mut IAudioClient) -> HRESULT {
    if client.is_null() {
        return E_INVALIDARG;
    }
    let c = unsafe { Box::from_raw(client) };
    if c.initialized {
        dbg_emit_dgram(
            LASAPI_DBG_LASAPI,
            LASAPI_DBG_INFO,
            c.session_id,
            0,
            0,
            S_OK,
            "LASAPI_RELEASE",
            cbuf_to_str(&c.client_name),
            "closing session shm/eventfd",
        );
        let r_ptr = unsafe { ring_ptr(&c) };
        if !r_ptr.is_null() {
            let r = unsafe { &*r_ptr };
            r.active.store(0, Ordering::SeqCst);
        }
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_GetEngineStatus(out_status: *mut LasapiEngineStatus) -> HRESULT {
    if out_status.is_null() {
        return E_INVALIDARG;
    }
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_GET_STATUS;
    if send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) }).is_err() {
        return E_FAIL;
    }
    let out_bytes = unsafe {
        slice::from_raw_parts_mut(out_status.cast::<u8>(), mem::size_of::<LasapiEngineStatus>())
    };
    if recv_waitall(sock.as_raw_fd(), out_bytes) {
        S_OK
    } else {
        E_FAIL
    }
}

fn send_engine_config_ipc(
    sample_rate: u32,
    period_frames: u32,
    format_bits: u32,
    stream_flags: u32,
) -> HRESULT {
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        dbg_emit_dgram(
            LASAPI_DBG_SHIM,
            salsa_wire::c_abi::LASAPI_DBG_ERR,
            0,
            sample_rate as i32,
            period_frames as i32,
            E_FAIL,
            "CTL_SET_CFG_FAIL",
            "audiodg",
            "connect failed",
        );
        return E_FAIL;
    };

    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_SET_ENGINE_CONFIG;
    req.sample_rate = sample_rate;
    req.requested_period_frames = period_frames;
    req.format = format_bits;
    req.stream_flags = stream_flags;
    let _ = send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) });

    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    let hr = if recv_waitall(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) }) {
        resp.hr
    } else {
        E_FAIL
    };
    let pname = get_process_comm();
    dbg_emit_dgram(
        LASAPI_DBG_SHIM,
        if hr == S_OK {
            LASAPI_DBG_INFO
        } else {
            LASAPI_DBG_WARN
        },
        0,
        sample_rate as i32,
        period_frames as i32,
        hr,
        if stream_flags == LASAPI_CFG_FLAG_CAPTURE_RATE {
            "CTL_SET_CAP_CFG"
        } else {
            "CTL_SET_CFG"
        },
        if pname.is_empty() { "ctl" } else { &pname },
        &format!(
            "rate={sample_rate} period={period_frames} bits={format_bits} flags=0x{stream_flags:x} hr=0x{:08x}",
            hr as u32
        ),
    );
    hr
}

#[no_mangle]
pub extern "C" fn LASAPI_SetEngineConfig(
    sample_rate: u32,
    period_frames: u32,
    format_bits: u32,
) -> HRESULT {
    send_engine_config_ipc(sample_rate, period_frames, format_bits, 0)
}

#[no_mangle]
pub extern "C" fn LASAPI_SetCaptureSampleRate(sample_rate: u32) -> HRESULT {
    send_engine_config_ipc(sample_rate, 0, 0, LASAPI_CFG_FLAG_CAPTURE_RATE)
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SetDynamicEQ(eq_cfg: *const LasapiEqConfig) -> HRESULT {
    if eq_cfg.is_null() {
        return E_INVALIDARG;
    }
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let eq = unsafe { *eq_cfg };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_SET_EQ;
    req.eq_cfg = eq;
    let _ = send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) });

    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    let hr = if recv_waitall(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) }) {
        resp.hr
    } else {
        E_FAIL
    };
    let pname = get_process_comm();
    let preset = cbuf_to_str(&eq.preset_name);
    dbg_emit_dgram(
        LASAPI_DBG_SHIM,
        LASAPI_DBG_INFO,
        0,
        eq.enabled as i32,
        eq.dynamic_mode as i32,
        hr,
        "CTL_SET_EQ",
        if pname.is_empty() { "ctl" } else { &pname },
        &format!(
            "en={} dyn={} preamp={:.1}dB thr={:.1}dB ratio={:.1} lim={} {preset}",
            eq.enabled,
            eq.dynamic_mode,
            eq.preamp_db,
            eq.dyn_threshold_db,
            eq.dyn_ratio,
            eq.limiter_enabled
        ),
    );
    hr
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_ListDevices(out_list: *mut LasapiDeviceList) -> HRESULT {
    if out_list.is_null() {
        return E_INVALIDARG;
    }
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_LIST_DEVICES;
    if send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) }).is_err() {
        return E_FAIL;
    }
    let out_bytes = unsafe {
        slice::from_raw_parts_mut(out_list.cast::<u8>(), mem::size_of::<LasapiDeviceList>())
    };
    if recv_waitall(sock.as_raw_fd(), out_bytes) {
        S_OK
    } else {
        E_FAIL
    }
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SetDevices(
    playback_id: *const c_char,
    capture_id: *const c_char,
) -> HRESULT {
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let pb = unsafe { cstr_to_str(playback_id) };
    let cap = unsafe { cstr_to_str(capture_id) };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_SET_DEVICES;
    if !pb.is_empty() {
        copy_str_to_c(&mut req.target_device, pb);
    }
    if !cap.is_empty() {
        copy_str_to_c(&mut req.target_capture_device, cap);
    }
    let _ = send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) });

    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    let hr = if recv_waitall(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) }) {
        resp.hr
    } else {
        E_FAIL
    };
    dbg_emit_dgram(
        LASAPI_DBG_SHIM,
        if hr == S_OK {
            LASAPI_DBG_INFO
        } else {
            salsa_wire::c_abi::LASAPI_DBG_ERR
        },
        0,
        0,
        0,
        hr,
        "CTL_SET_DEVICES",
        if pb.is_empty() { "(keep)" } else { pb },
        &format!(
            "cap={} hr=0x{:08x}",
            if cap.is_empty() { "(keep)" } else { cap },
            hr as u32
        ),
    );
    hr
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SetSessionControl(
    session_id: u32,
    vol_pct: i32,
    mute_mode: i32,
    target_device: *const c_char,
) -> HRESULT {
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_SET_SESSION_CONTROL;
    req.target_session_id = session_id;
    if vol_pct >= 0 {
        let clamped = vol_pct.min(150) as u32;
        req.volume_milli = clamped * 10;
    } else {
        req.volume_milli = 0xFFFF_FFFF;
    }
    if (0..=2).contains(&mute_mode) {
        req.share_mode = (mute_mode + 1) as u32;
    } else {
        req.share_mode = 0;
    }
    let dev = unsafe { cstr_to_str(target_device) };
    if !dev.is_empty() {
        copy_str_to_c(&mut req.target_device, dev);
    }
    let _ = send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) });

    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    if recv_waitall(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) }) {
        resp.hr
    } else {
        E_FAIL
    }
}

fn send_ext_ipc(cmd: u32, payload: &[u8], target_dev: &str, stream_flags: u32) -> HRESULT {
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = cmd;
    req.requested_buffer_frames = payload.len() as u32;
    req.stream_flags = stream_flags;
    if !target_dev.is_empty() {
        copy_str_to_c(&mut req.target_device, target_dev);
    }
    if send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) }).is_err() {
        return E_FAIL;
    }
    if !payload.is_empty() && send_all_nosignal(sock.as_raw_fd(), payload).is_err() {
        return E_FAIL;
    }
    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    if recv_waitall(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) }) {
        resp.hr
    } else {
        E_FAIL
    }
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SetParametricEQ(
    eq_cfg: *const LasapiEqConfig,
    param_ext: *const LasapiParamEqExt,
) -> HRESULT {
    if eq_cfg.is_null() || param_ext.is_null() {
        return E_INVALIDARG;
    }
    let pay = LasapiParamEqPayload {
        eq: unsafe { *eq_cfg },
        ext: unsafe { *param_ext },
    };
    send_ext_ipc(LASAPI_CMD_SET_PARAM_EQ, unsafe { as_bytes(&pay) }, "", 0)
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SetMicAPO(mic_cfg: *const LasapiMicApoCfg) -> HRESULT {
    if mic_cfg.is_null() {
        return E_INVALIDARG;
    }
    send_ext_ipc(
        LASAPI_CMD_SET_MIC_APO,
        unsafe { as_bytes(&*mic_cfg) },
        "",
        0,
    )
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SetBtCodecConfig(bt_cfg: *const LasapiBtCodecCfg) -> HRESULT {
    if bt_cfg.is_null() {
        return E_INVALIDARG;
    }
    send_ext_ipc(
        LASAPI_CMD_SET_BT_CODEC,
        unsafe { as_bytes(&*bt_cfg) },
        "",
        0,
    )
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SetHwAnalog(hw_cfg: *const LasapiHwAnalogCfg) -> HRESULT {
    if hw_cfg.is_null() {
        return E_INVALIDARG;
    }
    send_ext_ipc(
        LASAPI_CMD_SET_HW_ANALOG,
        unsafe { as_bytes(&*hw_cfg) },
        "",
        0,
    )
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_BtConnectionCtl(
    action: u32,
    mac_or_empty: *const c_char,
    out_flags: *mut u32,
) -> HRESULT {
    let Some(sock) = connect_unix_stream_blocking(&audiodg_sock_path()) else {
        return E_FAIL;
    };
    let mut req: LasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LASAPI_CMD_BT_CONNECTION;
    req.requested_period_frames = action;
    let mac = unsafe { cstr_to_str(mac_or_empty) };
    if !mac.is_empty() {
        copy_str_to_c(&mut req.target_device, mac);
    }
    if send_all_nosignal(sock.as_raw_fd(), unsafe { as_bytes(&req) }).is_err() {
        return E_FAIL;
    }
    let mut resp: LasapiIpcResp = unsafe { mem::zeroed() };
    if recv_waitall(sock.as_raw_fd(), unsafe { as_bytes_mut(&mut resp) }) {
        if !out_flags.is_null() {
            unsafe { *out_flags = resp.session_id };
        }
        resp.hr
    } else {
        E_FAIL
    }
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_SaveConfig() -> HRESULT {
    send_ext_ipc(LASAPI_CMD_SAVE_CONFIG, &[], "", 0)
}

#[no_mangle]
pub unsafe extern "C" fn LASAPI_ReloadConfig() -> HRESULT {
    send_ext_ipc(LASAPI_CMD_RELOAD_CONFIG, &[], "", 0)
}
