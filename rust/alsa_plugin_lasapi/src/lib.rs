// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void};
use core::mem;
#[cfg(test)]
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
use salsa_wire::client_seam::{bytes_per_sample_for_fmt, ClientHandle};
use salsa_wire::unix_sys::{
    audiodg_sock_path, O_NONBLOCK, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT,
};

salsa_wire::define_lasapi_ffi_backend!();

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
    client: ClientHandle,
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

    #[cfg_attr(test, allow(dead_code))]
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
    #[cfg_attr(test, allow(dead_code))]
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
    let client = unsafe { (*p).client };
    with_lasapi_backend(|b| b.start(client));
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
    let client = unsafe { (*p).client };
    with_lasapi_backend(|b| b.stop(client));
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
        let pkt = with_lasapi_backend(|b| b.get_next_packet_size(p_ref.client)).unwrap_or(0);
        let capped = u64::from(pkt).min(io_ref.buffer_size);
        let captured = p_ref.hw_frames.wrapping_add(capped);
        return (captured % io_ref.buffer_size) as c_long;
    }
    let pad = with_lasapi_backend(|b| b.get_current_padding(p_ref.client)).unwrap_or(0);
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

    let ch = if io_ref.channels != 0 {
        io_ref.channels as usize
    } else {
        usize::from(p_ref.cur_ch.max(1))
    };
    let nbytes = (size as usize) * ch * bytes_per_sample_for_fmt(p_ref.fmt);
    if io_ref.stream == SND_PCM_STREAM_CAPTURE {
        let dst = unsafe {
            core::slice::from_raw_parts_mut(area0.addr.cast::<u8>().add(byte_off), nbytes)
        };
        if with_lasapi_backend(|b| b.read_frames(p_ref.client, dst, size as u32, p_ref.fmt))
            != S_OK
        {
            return -c_long::from(EIO);
        }
        p_ref.hw_frames = p_ref.hw_frames.wrapping_add(size);
        return size as c_long;
    }

    let src = unsafe {
        core::slice::from_raw_parts(area0.addr.cast::<u8>().add(byte_off), nbytes)
    };
    if with_lasapi_backend(|b| b.write_frames(p_ref.client, src, size as u32, p_ref.fmt)) != S_OK
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
        let old = p_ref.client;
        with_lasapi_backend(|b| {
            b.stop(old);
            b.release(old);
        });
        p_ref.client = ClientHandle::NULL;
        p_ref.event_fd = -1;
        p_ref.io.poll_fd = -1;
        p_ref.initialized = false;
    }
    if p_ref.client.is_null() {
        match with_lasapi_backend(|b| b.activate_client(None)) {
            Ok(h) => p_ref.client = h,
            Err(_) => return -EIO,
        }
    }

    let hr = with_lasapi_backend(|b| {
        b.initialize(
            p_ref.client,
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            buf_frames,
            per_frames,
            &wfx,
        )
    });
    if hr != S_OK && hr != AUDCLNT_E_ALREADY_INITIALIZED {
        with_lasapi_backend(|b| {
            b.debug_trace(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_ERR,
                0,
                "ALSA_PLG_ERR",
                "lasapi-ioplug",
                "IAudioClient_Initialize failed in hw_params",
            );
        });
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
        "{dir_s} rate={rate} ch={ch} buf={} per={}",
        io_ref.buffer_size, io_ref.period_size
    );
    let io_name = if p_ref.io.name.is_null() {
        "lasapi".to_string()
    } else {
        unsafe { CStr::from_ptr(p_ref.io.name) }
            .to_string_lossy()
            .into_owned()
    };
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "ALSA_PLG_HW",
            &io_name,
            &detail,
        );
        if let Ok(fd) = b.set_event_handle(p_ref.client) {
            p_ref.event_fd = fd;
        }
    });
    if p_ref.event_fd >= 0 {
        let fl = unsafe { fcntl(p_ref.event_fd, F_GETFL, 0) };
        if fl >= 0 && (fl & O_NONBLOCK) == 0 {
            let _ = unsafe { fcntl(p_ref.event_fd, F_SETFL, fl | O_NONBLOCK) };
        }
    }
    p_ref.io.poll_fd = p_ref.event_fd;
    p_ref.io.poll_events = POLLIN as c_uint;
    #[cfg(not(test))]
    unsafe {
        snd_pcm_ioplug_reinit_status(&mut p_ref.io);
    }
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
    with_lasapi_backend(|b| {
        b.stop(p_ref.client);
        b.reset(p_ref.client);
    });
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
        with_lasapi_backend(|b| b.stop(p_ref.client));
        return 0;
    }
    let mut prev_pad = u32::MAX;
    let mut stall = 0i32;
    for _ in 0..200 {
        let pad = with_lasapi_backend(|b| b.get_current_padding(p_ref.client)).unwrap_or(0);
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
        with_lasapi_backend(|b| b.wait_for_period(p_ref.client, 10));
    }
    with_lasapi_backend(|b| b.stop(p_ref.client));
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
                "lasapi".to_string()
            } else {
                CStr::from_ptr((*p).io.name).to_string_lossy().into_owned()
            }
        };
        let client = unsafe { (*p).client };
        with_lasapi_backend(|b| {
            b.debug_trace(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_INFO,
                0,
                "ALSA_PLG_CLOSE",
                &io_name,
                "lasapi_io_close",
            );
            if !client.is_null() {
                b.release(client);
            }
        });
        unsafe {
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
        let client = p_ref.client;
        with_lasapi_backend(|b| {
            b.stop(client);
            b.release(client);
        });
        p_ref.client = ClientHandle::NULL;
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
    let target_name_str = unsafe { CStr::from_ptr(target_name) }
        .to_string_lossy()
        .into_owned();
    if is_system_audio_daemon() || !Path::new(&sock_path).exists() {
        with_lasapi_backend(|b| {
            b.debug_trace(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_WARN,
                0,
                "ALSA_PLG_FALLBACK",
                &target_name_str,
                "Falling back to ALSA sysdefault (daemon or no audiodg socket)",
            );
        });
        return unsafe { snd_pcm_open(pcmp, c"sysdefault".as_ptr(), stream, mode) };
    }

    let p = unsafe { calloc(1, mem::size_of::<LasapiAlsaIoplug>()) }.cast::<LasapiAlsaIoplug>();
    if p.is_null() {
        return -ENOMEM;
    }
    let p_ref = unsafe { &mut *p };
    match with_lasapi_backend(|b| b.activate_client(None)) {
        Ok(h) => p_ref.client = h,
        Err(_) => {
            with_lasapi_backend(|b| {
                b.debug_trace(
                    LASAPI_DBG_SHIM,
                    LASAPI_DBG_WARN,
                    0,
                    "ALSA_PLG_FALLBACK",
                    &target_name_str,
                    "ActivateAudioClient failed, falling back to sysdefault",
                );
            });
            unsafe {
                free(p.cast::<c_void>());
                return snd_pcm_open(pcmp, c"sysdefault".as_ptr(), stream, mode);
            }
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
        "lasapi".to_string()
    } else {
        unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
    };
    let io_name_s = unsafe { CStr::from_ptr(p_ref.io.name) }
        .to_string_lossy()
        .into_owned();
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "ALSA_PLG_OPEN",
            &open_tgt,
            &io_name_s,
        );
    });

    let err = unsafe { snd_pcm_ioplug_create(&mut p_ref.io, name, stream, mode) };
    if err < 0 {
        let client = p_ref.client;
        with_lasapi_backend(|b| b.release(client));
        unsafe {
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

#[cfg(test)]
mod tests {
    use super::*;
    use salsa_wire::client_seam::InMemoryLasapiBackend;
    use std::sync::Arc;

    #[test]
    fn ioplug_playback_and_capture_lifecycle_against_in_memory_backend() {
        let mem = Arc::new(InMemoryLasapiBackend::new(48_000, 2, 256));
        with_test_lasapi_backend(Arc::clone(&mem) as _, || unsafe {
            let p = calloc(1, mem::size_of::<LasapiAlsaIoplug>()).cast::<LasapiAlsaIoplug>();
            assert!(!p.is_null());
            let p_ref = &mut *p;
            p_ref.event_fd = -1;
            p_ref.io.name = c"TestPlayback".as_ptr();
            p_ref.io.stream = 0;
            p_ref.io.channels = 2;
            p_ref.io.rate = 48_000;
            p_ref.io.format = SND_PCM_FORMAT_FLOAT_LE;
            p_ref.io.buffer_size = 1024;
            p_ref.io.period_size = 256;
            p_ref.io.private_data = p.cast();

            assert_eq!(lasapi_io_hw_params(&raw mut p_ref.io, ptr::null_mut()), 0);
            let h = p_ref.client;
            assert!(!h.is_null());
            assert_eq!(lasapi_io_prepare(&raw mut p_ref.io), 0);
            assert_eq!(lasapi_io_start(&raw mut p_ref.io), 0);

            let mut samples = [0.25f32; 256 * 2];
            let area = SndPcmChannelArea {
                addr: samples.as_mut_ptr().cast(),
                first: 0,
                step: 64,
            };
            assert_eq!(lasapi_io_transfer(&raw mut p_ref.io, &raw const area, 0, 256), 256);
            assert_eq!(lasapi_io_pointer(&raw mut p_ref.io), 0);
            assert_eq!(lasapi_io_drain(&raw mut p_ref.io), 0);
            let snap = mem.snapshot_client(h).unwrap();
            assert_eq!(snap.written_frames, 256);
            assert!(!snap.started);

            assert_eq!(lasapi_io_hw_free(&raw mut p_ref.io), 0);
            assert!(p_ref.client.is_null());

            // Capture path
            p_ref.io.stream = SND_PCM_STREAM_CAPTURE;
            assert_eq!(lasapi_io_hw_params(&raw mut p_ref.io, ptr::null_mut()), 0);
            let ch = p_ref.client;
            assert_ne!(ch, h);
            mem.push_capture_bytes(ch, &[0x7Au8; 128 * 8]);
            let mut rx = vec![0u8; 128 * 8];
            let cap_area = SndPcmChannelArea {
                addr: rx.as_mut_ptr().cast(),
                first: 0,
                step: 64,
            };
            assert_eq!(lasapi_io_transfer(&raw mut p_ref.io, &raw const cap_area, 0, 128), 128);
            assert!(rx.iter().all(|&b| b == 0x7A));
            assert_eq!(lasapi_io_close(&raw mut p_ref.io), 0);
        });
    }
}
