// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void};
use core::mem;
use core::ptr;
use std::ffi::CStr;
use std::fs;
use std::path::Path;

use salsa_wire::c_abi::{
    AUDCLNT_E_ALREADY_INITIALIZED, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_CAPTURE,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, LASAPI_DBG_ERR, LASAPI_DBG_INFO, LASAPI_DBG_SHIM,
    LASAPI_DBG_WARN, LASAPI_FMT_FLOAT32, LASAPI_FMT_S16_LE, LASAPI_FMT_S32_LE, WaveFormatEx,
    S_OK,
};
use salsa_wire::unix_sys::{
    audiodg_sock_path, O_NONBLOCK, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT,
};

const EIO: c_int = 5;
const ENOMEM: c_int = 12;
const EINVAL: c_int = 22;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;

const SND_PCM_STREAM_CAPTURE: c_int = 1;
const SND_PCM_ACCESS_MMAP_INTERLEAVED: c_uint = 0;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_uint = 3;
const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_FORMAT_S32_LE: c_int = 10;
const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;

const SND_PCM_IOPLUG_HW_ACCESS: c_int = 0;
const SND_PCM_IOPLUG_HW_FORMAT: c_int = 1;
const SND_PCM_IOPLUG_HW_CHANNELS: c_int = 2;
const SND_PCM_IOPLUG_HW_RATE: c_int = 3;
const SND_PCM_IOPLUG_HW_PERIOD_BYTES: c_int = 4;
const SND_PCM_IOPLUG_HW_BUFFER_BYTES: c_int = 5;
const SND_PCM_IOPLUG_HW_PERIODS: c_int = 6;

const SND_PCM_IOPLUG_VERSION: c_uint = 0x01_00_02;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

#[repr(C)]
pub struct SndPcmChannelArea {
    addr: *mut c_void,
    first: c_uint,
    step: c_uint,
}

#[repr(C)]
pub struct SndPcmIoplug {
    version: c_uint,
    name: *const c_char,
    flags: c_uint,
    poll_fd: c_int,
    poll_events: c_uint,
    mmap_rw: c_uint,
    callback: *const SndPcmIoplugCallback,
    private_data: *mut c_void,
    pcm: *mut c_void,
    stream: c_int,
    state: c_int,
    appl_ptr: c_ulong,
    hw_ptr: c_ulong,
    nonblock: c_int,
    access: c_int,
    format: c_int,
    channels: c_uint,
    rate: c_uint,
    period_size: c_ulong,
    buffer_size: c_ulong,
}

#[repr(C)]
pub struct SndPcmIoplugCallback {
    start: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    stop: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    pointer: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_long>,
    transfer: Option<
        unsafe extern "C" fn(
            *mut SndPcmIoplug,
            *const SndPcmChannelArea,
            c_ulong,
            c_ulong,
        ) -> c_long,
    >,
    close: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    hw_params: Option<unsafe extern "C" fn(*mut SndPcmIoplug, *mut c_void) -> c_int>,
    hw_free: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    sw_params: Option<unsafe extern "C" fn(*mut SndPcmIoplug, *mut c_void) -> c_int>,
    prepare: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    drain: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    pause: Option<unsafe extern "C" fn(*mut SndPcmIoplug, c_int) -> c_int>,
    resume: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    poll_descriptors_count: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> c_int>,
    poll_descriptors: Option<unsafe extern "C" fn(*mut SndPcmIoplug, *mut PollFd, c_uint) -> c_int>,
    poll_revents:
        Option<unsafe extern "C" fn(*mut SndPcmIoplug, *mut PollFd, c_uint, *mut c_ushort) -> c_int>,
    dump: Option<unsafe extern "C" fn(*mut SndPcmIoplug, *mut c_void)>,
    delay: Option<unsafe extern "C" fn(*mut SndPcmIoplug, *mut c_long) -> c_int>,
    query_chmaps: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> *mut *mut c_void>,
    get_chmap: Option<unsafe extern "C" fn(*mut SndPcmIoplug) -> *mut c_void>,
    set_chmap: Option<unsafe extern "C" fn(*mut SndPcmIoplug, *const c_void) -> c_int>,
}

unsafe impl Sync for SndPcmIoplugCallback {}

#[repr(C)]
struct LasapiAlsaIoplug {
    io: SndPcmIoplug,
    client: *mut c_void,
    hw_frames: u64,
    fmt: u32,
    event_fd: c_int,
    initialized: bool,
    cur_rate: u32,
    cur_ch: u16,
    cur_fmt: u32,
    cur_buf: u32,
    cur_per: u32,
}

extern "C" {
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;

    fn LASAPI_DebugTraceClient(
        sub: u8,
        lvl: u8,
        sid: u32,
        op: *const c_char,
        target: *const c_char,
        detail: *const c_char,
    );
    fn LASAPI_ActivateAudioClient(
        client_name: *const c_char,
        pp_audio_client: *mut *mut c_void,
    ) -> i32;
    fn IAudioClient_Initialize(
        client: *mut c_void,
        share_mode: u32,
        stream_flags: u32,
        buffer_frame_count: u32,
        periodicity_frames: u32,
        p_format: *const WaveFormatEx,
    ) -> i32;
    fn IAudioClient_SetEventHandle(client: *mut c_void, p_event_fd_out: *mut c_int) -> i32;
    fn IAudioClient_GetCurrentPadding(client: *mut c_void, p_num_padding_frames: *mut u32) -> i32;
    fn IAudioClient_Start(client: *mut c_void) -> i32;
    fn IAudioClient_Stop(client: *mut c_void) -> i32;
    fn IAudioClient_Reset(client: *mut c_void) -> i32;
    fn IAudioClient_WaitForPeriod(client: *mut c_void, timeout_ms: c_int) -> i32;
    fn IAudioClient_Release(client: *mut c_void) -> i32;
    fn IAudioRenderClient_WriteFrames(
        client: *mut c_void,
        src: *const c_void,
        frames: u32,
        src_fmt: u32,
    ) -> i32;
    fn IAudioCaptureClient_GetNextPacketSize(
        client: *mut c_void,
        p_num_frames_in_next_packet: *mut u32,
    ) -> i32;
    fn IAudioCaptureClient_ReadFrames(
        client: *mut c_void,
        dst: *mut c_void,
        frames: u32,
        dst_fmt: u32,
    ) -> i32;

    fn snd_pcm_open(
        pcmp: *mut *mut c_void,
        name: *const c_char,
        stream: c_int,
        mode: c_int,
    ) -> c_int;
    fn snd_pcm_ioplug_create(
        io: *mut SndPcmIoplug,
        name: *const c_char,
        stream: c_int,
        mode: c_int,
    ) -> c_int;
    fn snd_pcm_ioplug_reinit_status(ioplug: *mut SndPcmIoplug) -> c_int;
    fn snd_pcm_ioplug_set_param_list(
        io: *mut SndPcmIoplug,
        param_type: c_int,
        num_list: c_uint,
        list: *const c_uint,
    ) -> c_int;
    fn snd_pcm_ioplug_set_param_minmax(
        io: *mut SndPcmIoplug,
        param_type: c_int,
        min: c_uint,
        max: c_uint,
    ) -> c_int;
}

fn is_system_audio_daemon() -> bool {
    let Ok(raw) = fs::read_to_string("/proc/self/comm") else {
        return false;
    };
    let comm = raw.trim_end_matches(['\r', '\n']);
    matches!(
        comm,
        "pipewire"
            | "wireplumber"
            | "pipewire-pulse"
            | "pulseaudio"
            | "lasapi-audiodg"
            | "plasmashell"
    )
}

unsafe extern "C" fn lasapi_io_start(io: *mut SndPcmIoplug) -> c_int {
    if io.is_null() {
        return -EIO;
    }
    let p = unsafe { (*io).private_data.cast::<LasapiAlsaIoplug>() };
    if p.is_null() || unsafe { (*p).client.is_null() } {
        return -EIO;
    }
    unsafe { IAudioClient_Start((*p).client) };
    0
}

unsafe extern "C" fn lasapi_io_stop(io: *mut SndPcmIoplug) -> c_int {
    if io.is_null() {
        return -EIO;
    }
    let p = unsafe { (*io).private_data.cast::<LasapiAlsaIoplug>() };
    if p.is_null() || unsafe { (*p).client.is_null() } {
        return -EIO;
    }
    unsafe { IAudioClient_Stop((*p).client) };
    0
}

unsafe extern "C" fn lasapi_io_pointer(io: *mut SndPcmIoplug) -> c_long {
    if io.is_null() {
        return 0;
    }
    let io_ref = unsafe { &*io };
    let p = io_ref.private_data.cast::<LasapiAlsaIoplug>();
    if p.is_null() || unsafe { (*p).client.is_null() } || io_ref.buffer_size == 0 {
        return 0;
    }
    let p_ref = unsafe { &*p };
    if io_ref.stream == SND_PCM_STREAM_CAPTURE {
        let mut pkt = 0u32;
        unsafe { IAudioCaptureClient_GetNextPacketSize(p_ref.client, &mut pkt) };
        let capped = u64::from(pkt).min(io_ref.buffer_size);
        let captured = p_ref.hw_frames.wrapping_add(capped);
        return (captured % io_ref.buffer_size) as c_long;
    }
    let mut pad = 0u32;
    unsafe { IAudioClient_GetCurrentPadding(p_ref.client, &mut pad) };
    let played = p_ref.hw_frames.saturating_sub(u64::from(pad));
    (played % io_ref.buffer_size) as c_long
}

unsafe extern "C" fn lasapi_io_transfer(
    io: *mut SndPcmIoplug,
    areas: *const SndPcmChannelArea,
    offset: c_ulong,
    size: c_ulong,
) -> c_long {
    if io.is_null() || areas.is_null() {
        return -c_long::from(EIO);
    }
    let io_ref = unsafe { &*io };
    let p = io_ref.private_data.cast::<LasapiAlsaIoplug>();
    if p.is_null() || unsafe { (*p).client.is_null() } {
        return -c_long::from(EIO);
    }
    let p_ref = unsafe { &mut *p };
    let area0 = unsafe { &*areas };
    let byte_off = ((area0.first as usize) + (area0.step as usize) * (offset as usize)) / 8;

    if io_ref.stream == SND_PCM_STREAM_CAPTURE {
        let dst = unsafe { area0.addr.cast::<u8>().add(byte_off) }.cast::<c_void>();
        if unsafe { IAudioCaptureClient_ReadFrames(p_ref.client, dst, size as u32, p_ref.fmt) }
            != S_OK
        {
            return -c_long::from(EIO);
        }
        p_ref.hw_frames = p_ref.hw_frames.wrapping_add(size);
        return size as c_long;
    }

    let src = unsafe { area0.addr.cast::<u8>().add(byte_off) }.cast::<c_void>();
    if unsafe { IAudioRenderClient_WriteFrames(p_ref.client, src, size as u32, p_ref.fmt) } != S_OK
    {
        return -c_long::from(EIO);
    }
    p_ref.hw_frames = p_ref.hw_frames.wrapping_add(size);
    size as c_long
}

unsafe extern "C" fn lasapi_io_hw_params(io: *mut SndPcmIoplug, _params: *mut c_void) -> c_int {
    if io.is_null() {
        return -EIO;
    }
    let io_ref = unsafe { &*io };
    let p = io_ref.private_data.cast::<LasapiAlsaIoplug>();
    if p.is_null() {
        return -EIO;
    }
    let p_ref = unsafe { &mut *p };

    p_ref.fmt = if io_ref.format == SND_PCM_FORMAT_S16_LE {
        LASAPI_FMT_S16_LE
    } else if io_ref.format == SND_PCM_FORMAT_S32_LE {
        LASAPI_FMT_S32_LE
    } else {
        LASAPI_FMT_FLOAT32
    };

    let ch = if io_ref.channels != 0 {
        io_ref.channels as u16
    } else {
        2
    };
    let rate = if io_ref.rate != 0 { io_ref.rate } else { 48000 };
    let bits: u16 = if p_ref.fmt == LASAPI_FMT_S16_LE { 16 } else { 32 };
    let block_align = ch * (bits / 8);
    let wfx = WaveFormatEx {
        w_format_tag: p_ref.fmt as u16,
        n_channels: ch,
        n_samples_per_sec: rate,
        w_bits_per_sample: bits,
        n_block_align: block_align,
        n_avg_bytes_per_sec: rate * u32::from(block_align),
        cb_size: 0,
    };

    let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if io_ref.stream == SND_PCM_STREAM_CAPTURE {
        flags |= AUDCLNT_STREAMFLAGS_CAPTURE;
    }

    let buf_frames = if io_ref.buffer_size != 0 {
        io_ref.buffer_size as u32
    } else {
        2048
    };
    let per_frames = if io_ref.period_size != 0 {
        io_ref.period_size as u32
    } else {
        256
    };

    let params_changed = p_ref.initialized
        && (p_ref.cur_rate != rate
            || p_ref.cur_ch != ch
            || p_ref.cur_fmt != p_ref.fmt
            || p_ref.cur_buf != buf_frames
            || p_ref.cur_per != per_frames);
    if params_changed && !p_ref.client.is_null() {
        unsafe {
            IAudioClient_Stop(p_ref.client);
            IAudioClient_Release(p_ref.client);
        }
        p_ref.client = ptr::null_mut();
        p_ref.event_fd = -1;
        p_ref.io.poll_fd = -1;
        p_ref.initialized = false;
    }
    if p_ref.client.is_null()
        && unsafe { LASAPI_ActivateAudioClient(ptr::null(), &mut p_ref.client) } != S_OK
    {
        return -EIO;
    }

    let hr = unsafe {
        IAudioClient_Initialize(
            p_ref.client,
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            buf_frames,
            per_frames,
            &wfx,
        )
    };
    if hr != S_OK && hr != AUDCLNT_E_ALREADY_INITIALIZED {
        unsafe {
            LASAPI_DebugTraceClient(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_ERR,
                0,
                c"ALSA_PLG_ERR".as_ptr(),
                c"lasapi-ioplug".as_ptr(),
                c"IAudioClient_Initialize failed in hw_params".as_ptr(),
            );
        }
        return -EIO;
    }

    p_ref.initialized = true;
    p_ref.cur_rate = rate;
    p_ref.cur_ch = ch;
    p_ref.cur_fmt = p_ref.fmt;
    p_ref.cur_buf = buf_frames;
    p_ref.cur_per = per_frames;

    let dir_s = if io_ref.stream == SND_PCM_STREAM_CAPTURE {
        "CAPTURE"
    } else {
        "PLAYBACK"
    };
    let detail = format!(
        "{dir_s} rate={rate} ch={ch} buf={} per={}\0",
        io_ref.buffer_size, io_ref.period_size
    );
    unsafe {
        LASAPI_DebugTraceClient(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            c"ALSA_PLG_HW".as_ptr(),
            p_ref.io.name,
            detail.as_ptr().cast::<c_char>(),
        );
        IAudioClient_SetEventHandle(p_ref.client, &mut p_ref.event_fd);
    }
    if p_ref.event_fd >= 0 {
        let fl = unsafe { fcntl(p_ref.event_fd, F_GETFL, 0) };
        if fl >= 0 && (fl & O_NONBLOCK) == 0 {
            let _ = unsafe { fcntl(p_ref.event_fd, F_SETFL, fl | O_NONBLOCK) };
        }
    }
    p_ref.io.poll_fd = p_ref.event_fd;
    p_ref.io.poll_events = POLLIN as c_uint;
    unsafe { snd_pcm_ioplug_reinit_status(&mut p_ref.io) };
    0
}

unsafe extern "C" fn lasapi_io_prepare(io: *mut SndPcmIoplug) -> c_int {
    if io.is_null() {
        return -EIO;
    }
    let p = unsafe { (*io).private_data.cast::<LasapiAlsaIoplug>() };
    if p.is_null() || unsafe { (*p).client.is_null() } {
        return -EIO;
    }
    let p_ref = unsafe { &mut *p };
    p_ref.hw_frames = 0;
    unsafe {
        IAudioClient_Stop(p_ref.client);
        IAudioClient_Reset(p_ref.client);
    }
    if p_ref.event_fd >= 0 {
        let mut val: u64 = 0;
        let _ = unsafe { read(p_ref.event_fd, (&mut val as *mut u64).cast::<c_void>(), 8) };
    }
    0
}

unsafe extern "C" fn lasapi_io_drain(io: *mut SndPcmIoplug) -> c_int {
    if io.is_null() {
        return 0;
    }
    let io_ref = unsafe { &*io };
    let p = io_ref.private_data.cast::<LasapiAlsaIoplug>();
    if p.is_null() || unsafe { (*p).client.is_null() } {
        return 0;
    }
    let p_ref = unsafe { &*p };
    if io_ref.stream == SND_PCM_STREAM_CAPTURE {
        unsafe { IAudioClient_Stop(p_ref.client) };
        return 0;
    }
    let mut prev_pad = u32::MAX;
    let mut stall = 0i32;
    for _ in 0..200 {
        let mut pad = 0u32;
        unsafe { IAudioClient_GetCurrentPadding(p_ref.client, &mut pad) };
        if pad == 0 {
            break;
        }
        if pad == prev_pad {
            stall += 1;
            if stall >= 3 {
                break;
            }
        } else {
            stall = 0;
            prev_pad = pad;
        }
        unsafe { IAudioClient_WaitForPeriod(p_ref.client, 10) };
    }
    unsafe { IAudioClient_Stop(p_ref.client) };
    0
}

unsafe extern "C" fn lasapi_io_poll_revents(
    io: *mut SndPcmIoplug,
    pfd: *mut PollFd,
    nfds: c_uint,
    revents: *mut c_ushort,
) -> c_int {
    if revents.is_null() {
        return -EINVAL;
    }
    unsafe { *revents = 0 };
    if io.is_null() || pfd.is_null() || nfds == 0 {
        return 0;
    }
    let pfd0 = unsafe { &*pfd };
    if (pfd0.revents & POLLIN) != 0 {
        let mut val: u64 = 0;
        let _ = unsafe { read(pfd0.fd, (&mut val as *mut u64).cast::<c_void>(), 8) };
        unsafe {
            *revents = if (*io).stream == SND_PCM_STREAM_CAPTURE {
                POLLIN as c_ushort
            } else {
                POLLOUT as c_ushort
            };
        }
    }
    unsafe {
        *revents |= (pfd0.revents & (POLLERR | POLLHUP | POLLNVAL)) as c_ushort;
    }
    0
}

unsafe extern "C" fn lasapi_io_close(io: *mut SndPcmIoplug) -> c_int {
    if io.is_null() {
        return 0;
    }
    let p = unsafe { (*io).private_data.cast::<LasapiAlsaIoplug>() };
    if !p.is_null() {
        let io_name = unsafe {
            if (*p).io.name.is_null() {
                c"lasapi".as_ptr()
            } else {
                (*p).io.name
            }
        };
        unsafe {
            LASAPI_DebugTraceClient(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_INFO,
                0,
                c"ALSA_PLG_CLOSE".as_ptr(),
                io_name,
                c"lasapi_io_close".as_ptr(),
            );
            if !(*p).client.is_null() {
                IAudioClient_Release((*p).client);
            }
            free(p.cast::<c_void>());
        }
    }
    0
}

unsafe extern "C" fn lasapi_io_hw_free(io: *mut SndPcmIoplug) -> c_int {
    if io.is_null() {
        return 0;
    }
    let p = unsafe { (*io).private_data.cast::<LasapiAlsaIoplug>() };
    if p.is_null() {
        return 0;
    }
    let p_ref = unsafe { &mut *p };
    if !p_ref.client.is_null() {
        unsafe {
            IAudioClient_Stop(p_ref.client);
            IAudioClient_Release(p_ref.client);
        }
        p_ref.client = ptr::null_mut();
    }
    p_ref.event_fd = -1;
    p_ref.io.poll_fd = -1;
    p_ref.io.poll_events = 0;
    p_ref.initialized = false;
    0
}

static LASAPI_IO_CALLBACKS: SndPcmIoplugCallback = SndPcmIoplugCallback {
    start: Some(lasapi_io_start),
    stop: Some(lasapi_io_stop),
    pointer: Some(lasapi_io_pointer),
    transfer: Some(lasapi_io_transfer),
    close: Some(lasapi_io_close),
    hw_params: Some(lasapi_io_hw_params),
    hw_free: Some(lasapi_io_hw_free),
    sw_params: None,
    prepare: Some(lasapi_io_prepare),
    drain: Some(lasapi_io_drain),
    pause: None,
    resume: None,
    poll_descriptors_count: None,
    poll_descriptors: None,
    poll_revents: Some(lasapi_io_poll_revents),
    dump: None,
    delay: None,
    query_chmaps: None,
    get_chmap: None,
    set_chmap: None,
};

#[no_mangle]
pub unsafe extern "C" fn _snd_pcm_lasapi_open(
    pcmp: *mut *mut c_void,
    name: *const c_char,
    _root: *mut c_void,
    _conf: *mut c_void,
    stream: c_int,
    mode: c_int,
) -> c_int {
    let sock_path = audiodg_sock_path();
    let target_name = if name.is_null() {
        c"default".as_ptr()
    } else {
        name
    };
    if is_system_audio_daemon() || !Path::new(&sock_path).exists() {
        unsafe {
            LASAPI_DebugTraceClient(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_WARN,
                0,
                c"ALSA_PLG_FALLBACK".as_ptr(),
                target_name,
                c"Falling back to ALSA sysdefault (daemon or no audiodg socket)".as_ptr(),
            );
            return snd_pcm_open(pcmp, c"sysdefault".as_ptr(), stream, mode);
        }
    }

    let p = unsafe { calloc(1, mem::size_of::<LasapiAlsaIoplug>()) }.cast::<LasapiAlsaIoplug>();
    if p.is_null() {
        return -ENOMEM;
    }
    let p_ref = unsafe { &mut *p };
    if unsafe { LASAPI_ActivateAudioClient(ptr::null(), &mut p_ref.client) } != S_OK {
        unsafe {
            LASAPI_DebugTraceClient(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_WARN,
                0,
                c"ALSA_PLG_FALLBACK".as_ptr(),
                target_name,
                c"ActivateAudioClient failed, falling back to sysdefault".as_ptr(),
            );
            free(p.cast::<c_void>());
            return snd_pcm_open(pcmp, c"sysdefault".as_ptr(), stream, mode);
        }
    }

    p_ref.event_fd = -1;
    p_ref.io.version = SND_PCM_IOPLUG_VERSION;
    p_ref.io.name = if stream == SND_PCM_STREAM_CAPTURE {
        c"LASAPI Direct WaveRT Capture Engine".as_ptr()
    } else {
        c"LASAPI Direct WaveRT Render Engine".as_ptr()
    };
    p_ref.io.poll_fd = -1;
    p_ref.io.poll_events = 0;
    p_ref.io.callback = &LASAPI_IO_CALLBACKS;
    p_ref.io.private_data = p.cast::<c_void>();

    let open_tgt = if name.is_null() {
        c"lasapi".as_ptr()
    } else {
        let _ = unsafe { CStr::from_ptr(name) };
        name
    };
    unsafe {
        LASAPI_DebugTraceClient(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            c"ALSA_PLG_OPEN".as_ptr(),
            open_tgt,
            p_ref.io.name,
        );
    }

    let err = unsafe { snd_pcm_ioplug_create(&mut p_ref.io, name, stream, mode) };
    if err < 0 {
        unsafe {
            IAudioClient_Release(p_ref.client);
            free(p.cast::<c_void>());
        }
        return err;
    }

    const ACCESSES: [c_uint; 2] = [
        SND_PCM_ACCESS_RW_INTERLEAVED,
        SND_PCM_ACCESS_MMAP_INTERLEAVED,
    ];
    const FORMATS: [c_uint; 3] = [
        SND_PCM_FORMAT_FLOAT_LE as c_uint,
        SND_PCM_FORMAT_S32_LE as c_uint,
        SND_PCM_FORMAT_S16_LE as c_uint,
    ];

    unsafe {
        snd_pcm_ioplug_set_param_list(
            &mut p_ref.io,
            SND_PCM_IOPLUG_HW_ACCESS,
            2,
            ACCESSES.as_ptr(),
        );
        snd_pcm_ioplug_set_param_list(&mut p_ref.io, SND_PCM_IOPLUG_HW_FORMAT, 3, FORMATS.as_ptr());
        snd_pcm_ioplug_set_param_minmax(&mut p_ref.io, SND_PCM_IOPLUG_HW_CHANNELS, 1, 8);
        snd_pcm_ioplug_set_param_minmax(&mut p_ref.io, SND_PCM_IOPLUG_HW_RATE, 8000, 192000);
        snd_pcm_ioplug_set_param_minmax(&mut p_ref.io, SND_PCM_IOPLUG_HW_PERIOD_BYTES, 256, 16384);
        snd_pcm_ioplug_set_param_minmax(&mut p_ref.io, SND_PCM_IOPLUG_HW_BUFFER_BYTES, 1024, 65536);
        snd_pcm_ioplug_set_param_minmax(&mut p_ref.io, SND_PCM_IOPLUG_HW_PERIODS, 2, 64);
        if !pcmp.is_null() {
            *pcmp = p_ref.io.pcm;
        }
    }
    0
}

#[used]
#[no_mangle]
pub static mut __snd_pcm_lasapi_open_dlsym_pcm_001: u8 = 0;
