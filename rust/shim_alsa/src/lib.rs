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
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void, CStr};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use salsa_wire::c_abi::{
    AUDCLNT_E_ALREADY_INITIALIZED, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_CAPTURE,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, LASAPI_DBG_ERR, LASAPI_DBG_INFO, LASAPI_DBG_SHIM,
    LASAPI_FMT_FLOAT32, LASAPI_FMT_S16_LE, LASAPI_FMT_S32_LE, WaveFormatEx, S_OK,
};
use salsa_wire::unix_sys::POLLIN;

const POLLOUT: c_short = 4;
const RTLD_LAZY: c_int = 1;
const RTLD_LOCAL: c_int = 0;

const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_FORMAT_S32_LE: c_int = 10;
const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;

const SND_PCM_STATE_PREPARED: c_int = 2;
const SND_PCM_STATE_RUNNING: c_int = 3;
const SND_PCM_STATE_PAUSED: c_int = 6;

#[repr(C)]
pub struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

#[repr(C)]
pub struct snd_pcm_t {
    client: *mut c_void,
    event_fd: c_int,
    sample_rate: u32,
    channels: u32,
    fmt: u32,
    buffer_frames: u32,
    period_frames: u32,
    started: bool,
    paused: bool,
    is_capture: bool,
    init_rate: u32,
    init_channels: u32,
    init_fmt: u32,
    init_buffer_frames: u32,
    init_period_frames: u32,
}

#[repr(C)]
pub struct snd_pcm_hw_params_t {
    rate: u32,
    channels: u32,
    alsa_format: c_int,
    buffer_frames: u32,
    period_frames: u32,
}

#[repr(C)]
pub struct snd_pcm_sw_params_t {
    start_threshold: u32,
    avail_min: u32,
}

extern "C" {
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn strerror(errnum: c_int) -> *const c_char;
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;

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
    fn IAudioClient_GetBufferSize(client: *mut c_void, p_num_buffer_frames: *mut u32) -> i32;
    fn IAudioClient_GetDevicePeriod(
        client: *mut c_void,
        p_default_period_frames: *mut u32,
        p_minimum_period_frames: *mut u32,
    ) -> i32;
    fn IAudioClient_GetCurrentPadding(client: *mut c_void, p_num_padding_frames: *mut u32) -> i32;
    fn IAudioClient_SetEventHandle(client: *mut c_void, p_event_fd_out: *mut c_int) -> i32;
    fn IAudioClient_Start(client: *mut c_void) -> i32;
    fn IAudioClient_Stop(client: *mut c_void) -> i32;
    fn IAudioClient_Reset(client: *mut c_void) -> i32;
    fn IAudioClient_Release(client: *mut c_void) -> i32;
    fn IAudioClient_WaitForPeriod(client: *mut c_void, timeout_ms: c_int) -> i32;
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
}

static REAL_ASOUND_DL: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

fn get_real_asound() -> *mut c_void {
    let cur = REAL_ASOUND_DL.load(Ordering::Acquire);
    if !cur.is_null() {
        return cur;
    }
    const CANDS: [&CStr; 12] = [
        c"/usr/lib/libasound.so.2.0.0",
        c"/usr/lib/libasound.so.2",
        c"/usr/lib64/libasound.so.2.0.0",
        c"/usr/lib64/libasound.so.2",
        c"/usr/lib/x86_64-linux-gnu/libasound.so.2.0.0",
        c"/usr/lib/x86_64-linux-gnu/libasound.so.2",
        c"/usr/lib/aarch64-linux-gnu/libasound.so.2.0.0",
        c"/usr/lib/aarch64-linux-gnu/libasound.so.2",
        c"/lib/x86_64-linux-gnu/libasound.so.2",
        c"/lib/aarch64-linux-gnu/libasound.so.2",
        c"/lib64/libasound.so.2",
        c"/lib/libasound.so.2",
    ];
    for cand in CANDS {
        let h = unsafe { dlopen(cand.as_ptr(), RTLD_LAZY | RTLD_LOCAL) };
        if !h.is_null() {
            let _ = REAL_ASOUND_DL.compare_exchange(
                ptr::null_mut(),
                h,
                Ordering::Release,
                Ordering::Acquire,
            );
            return REAL_ASOUND_DL.load(Ordering::Acquire);
        }
    }
    ptr::null_mut()
}

fn resolve_sym(slot: &AtomicPtr<c_void>, name: &CStr) -> *mut c_void {
    let cur = slot.load(Ordering::Acquire);
    if !cur.is_null() {
        return cur;
    }
    let dl = get_real_asound();
    if dl.is_null() {
        return ptr::null_mut();
    }
    let sym = unsafe { dlsym(dl, name.as_ptr()) };
    if !sym.is_null() {
        slot.store(sym, Ordering::Release);
    }
    sym
}

macro_rules! resolve_asound {
    ($ret:ty, $name:ident, ($($arg:ident : $aty:ty),* $(,)?), $fallback:expr) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($arg: $aty),*) -> $ret {
            static FN_PTR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
            let sym = resolve_sym(&FN_PTR, c_ident!($name));
            if !sym.is_null() {
                let f: unsafe extern "C" fn($($aty),*) -> $ret = unsafe { mem::transmute(sym) };
                return unsafe { f($($arg),*) };
            }
            $fallback
        }
    };
}

macro_rules! resolve_asound_void {
    ($name:ident, ($($arg:ident : $aty:ty),* $(,)?)) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($arg: $aty),*) {
            static FN_PTR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
            let sym = resolve_sym(&FN_PTR, c_ident!($name));
            if !sym.is_null() {
                let f: unsafe extern "C" fn($($aty),*) = unsafe { mem::transmute(sym) };
                unsafe { f($($arg),*) };
            }
        }
    };
}

macro_rules! c_ident {
    ($id:ident) => {
        match CStr::from_bytes_with_nul(concat!(stringify!($id), "\0").as_bytes()) {
            Ok(s) => s,
            Err(_) => unreachable!(),
        }
    };
}

#[used]
#[no_mangle]
pub static mut snd_config: usize = 1;

#[no_mangle]
pub extern "C" fn snd_config_update_free_global() -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_config_copy(dst: *mut *mut c_void, _src: *const c_void) -> c_int {
    if !dst.is_null() {
        unsafe { *dst = ptr::without_provenance_mut(1) };
    }
    0
}

#[no_mangle]
pub extern "C" fn snd_config_delete(_top: *mut c_void) -> c_int {
    0
}

#[no_mangle]
pub extern "C" fn snd_config_search(
    _config: *const c_void,
    _key: *const c_char,
    _result: *mut *mut c_void,
) -> c_int {
    -2
}

#[no_mangle]
pub extern "C" fn snd_config_search_definition(
    _config: *const c_void,
    _base: *const c_char,
    _key: *const c_char,
    _result: *mut *mut c_void,
) -> c_int {
    -2
}

#[no_mangle]
pub unsafe extern "C" fn snd_config_get_id(
    _config: *const c_void,
    value: *mut *const c_char,
) -> c_int {
    if !value.is_null() {
        unsafe { *value = c"default".as_ptr() };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_config_get_string(
    _config: *const c_void,
    value: *mut *const c_char,
) -> c_int {
    if !value.is_null() {
        unsafe { *value = c"default".as_ptr() };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_config_imake_integer(
    config: *mut *mut c_void,
    _key: *const c_char,
    _value: c_long,
) -> c_int {
    if !config.is_null() {
        unsafe { *config = ptr::without_provenance_mut(1) };
    }
    0
}

#[no_mangle]
pub extern "C" fn snd_config_add(_parent: *mut c_void, _child: *mut c_void) -> c_int {
    0
}

#[no_mangle]
pub extern "C" fn snd_lib_error_set_handler(_handler: *mut c_void) -> c_int {
    0
}

resolve_asound!(c_int, snd_midi_event_new, (bufsize: usize, rdev: *mut *mut c_void), -19);
resolve_asound_void!(snd_midi_event_free, (dev: *mut c_void));
resolve_asound_void!(snd_midi_event_no_status, (dev: *mut c_void, on: c_int));
resolve_asound!(c_int, snd_midi_event_resize_buffer, (dev: *mut c_void, bufsize: usize), -19);
resolve_asound!(c_long, snd_midi_event_encode, (dev: *mut c_void, buf: *const u8, count: c_long, ev: *mut c_void), -19);
resolve_asound!(c_long, snd_midi_event_decode, (dev: *mut c_void, buf: *mut u8, count: c_long, ev: *const c_void), -19);

resolve_asound!(c_int, snd_seq_open, (handle: *mut *mut c_void, name: *const c_char, streams: c_int, mode: c_int), -19);
resolve_asound!(c_int, snd_seq_close, (handle: *mut c_void), 0);
resolve_asound!(c_int, snd_seq_client_id, (handle: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_set_client_name, (handle: *mut c_void, name: *const c_char), -19);
resolve_asound!(c_int, snd_seq_alloc_named_queue, (seq: *mut c_void, name: *const c_char), -19);
resolve_asound!(c_int, snd_seq_free_queue, (handle: *mut c_void, q: c_int), 0);
resolve_asound!(c_int, snd_seq_control_queue, (seq: *mut c_void, q: c_int, kind: c_int, value: c_int, ev: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_create_port, (handle: *mut c_void, info: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_create_simple_port, (seq: *mut c_void, name: *const c_char, caps: c_uint, kind: c_uint), -19);
resolve_asound!(c_int, snd_seq_delete_port, (handle: *mut c_void, port: c_int), 0);
resolve_asound!(c_int, snd_seq_drain_output, (handle: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_event_input, (handle: *mut c_void, ev: *mut *mut c_void), -11);
resolve_asound!(c_int, snd_seq_event_input_pending, (seq: *mut c_void, fetch_sequencer: c_int), 0);
resolve_asound!(c_int, snd_seq_event_output, (handle: *mut c_void, ev: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_event_output_direct, (handle: *mut c_void, ev: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_get_any_client_info, (handle: *mut c_void, client: c_int, info: *mut c_void), -2);
resolve_asound!(c_int, snd_seq_get_any_port_info, (handle: *mut c_void, client: c_int, port: c_int, info: *mut c_void), -2);
resolve_asound!(c_int, snd_seq_poll_descriptors_count, (handle: *mut c_void, events: c_short), 0);
resolve_asound!(c_int, snd_seq_poll_descriptors, (handle: *mut c_void, pfds: *mut PollFd, space: c_uint, events: c_short), 0);
resolve_asound!(c_int, snd_seq_query_next_client, (handle: *mut c_void, info: *mut c_void), -2);
resolve_asound!(c_int, snd_seq_query_next_port, (handle: *mut c_void, info: *mut c_void), -2);
resolve_asound!(c_int, snd_seq_set_queue_tempo, (handle: *mut c_void, q: c_int, tempo: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_subscribe_port, (handle: *mut c_void, sub: *mut c_void), -19);
resolve_asound!(c_int, snd_seq_unsubscribe_port, (handle: *mut c_void, sub: *mut c_void), 0);
resolve_asound!(c_int, snd_seq_connect_from, (seq: *mut c_void, my_port: c_int, src_client: c_int, src_port: c_int), -19);
resolve_asound!(c_int, snd_seq_connect_to, (seq: *mut c_void, my_port: c_int, dest_client: c_int, dest_port: c_int), -19);

resolve_asound!(usize, snd_seq_client_info_sizeof, (), 256);
resolve_asound!(c_int, snd_seq_client_info_malloc, (ptr: *mut *mut c_void), -12);
resolve_asound_void!(snd_seq_client_info_free, (ptr: *mut c_void));
resolve_asound!(c_int, snd_seq_client_info_get_client, (info: *const c_void), -19);
resolve_asound!(*const c_char, snd_seq_client_info_get_name, (info: *mut c_void), c"LASAPI-Seq".as_ptr());
resolve_asound_void!(snd_seq_client_info_set_client, (info: *mut c_void, client: c_int));

resolve_asound!(usize, snd_seq_port_info_sizeof, (), 256);
resolve_asound!(c_int, snd_seq_port_info_malloc, (ptr: *mut *mut c_void), -12);
resolve_asound_void!(snd_seq_port_info_free, (ptr: *mut c_void));
resolve_asound!(c_uint, snd_seq_port_info_get_capability, (info: *const c_void), 0);
resolve_asound!(c_int, snd_seq_port_info_get_client, (info: *const c_void), -19);
resolve_asound!(*const c_char, snd_seq_port_info_get_name, (info: *const c_void), c"Port".as_ptr());
resolve_asound!(c_int, snd_seq_port_info_get_port, (info: *const c_void), -19);
resolve_asound!(c_uint, snd_seq_port_info_get_type, (info: *const c_void), 0);
resolve_asound_void!(snd_seq_port_info_set_capability, (info: *mut c_void, capability: c_uint));
resolve_asound_void!(snd_seq_port_info_set_client, (info: *mut c_void, client: c_int));
resolve_asound_void!(snd_seq_port_info_set_midi_channels, (info: *mut c_void, channels: c_int));
resolve_asound_void!(snd_seq_port_info_set_name, (info: *mut c_void, name: *const c_char));
resolve_asound_void!(snd_seq_port_info_set_port, (info: *mut c_void, port: c_int));
resolve_asound_void!(snd_seq_port_info_set_timestamping, (info: *mut c_void, enable: c_int));
resolve_asound_void!(snd_seq_port_info_set_timestamp_queue, (info: *mut c_void, queue: c_int));
resolve_asound_void!(snd_seq_port_info_set_timestamp_real, (info: *mut c_void, enable: c_int));
resolve_asound_void!(snd_seq_port_info_set_type, (info: *mut c_void, kind: c_uint));

resolve_asound!(usize, snd_seq_port_subscribe_sizeof, (), 256);
resolve_asound!(c_int, snd_seq_port_subscribe_malloc, (ptr: *mut *mut c_void), -12);
resolve_asound_void!(snd_seq_port_subscribe_free, (ptr: *mut c_void));
resolve_asound!(*const c_void, snd_seq_port_subscribe_get_dest, (info: *const c_void), info);
resolve_asound!(*const c_void, snd_seq_port_subscribe_get_sender, (info: *const c_void), info);
resolve_asound_void!(snd_seq_port_subscribe_set_dest, (info: *mut c_void, addr: *const c_void));
resolve_asound_void!(snd_seq_port_subscribe_set_sender, (info: *mut c_void, addr: *const c_void));
resolve_asound_void!(snd_seq_port_subscribe_set_time_real, (info: *mut c_void, val: c_int));
resolve_asound_void!(snd_seq_port_subscribe_set_time_update, (info: *mut c_void, val: c_int));

resolve_asound!(usize, snd_seq_queue_tempo_sizeof, (), 256);
resolve_asound!(c_int, snd_seq_queue_tempo_malloc, (ptr: *mut *mut c_void), -12);
resolve_asound_void!(snd_seq_queue_tempo_free, (ptr: *mut c_void));
resolve_asound_void!(snd_seq_queue_tempo_set_ppq, (info: *mut c_void, ppq: c_int));
resolve_asound_void!(snd_seq_queue_tempo_set_tempo, (info: *mut c_void, tempo: c_uint));

resolve_asound!(c_int, snd_rawmidi_open, (in_rmidi: *mut *mut c_void, out_rmidi: *mut *mut c_void, name: *const c_char, mode: c_int), -19);
resolve_asound!(c_int, snd_rawmidi_close, (rmidi: *mut c_void), 0);
resolve_asound!(isize, snd_rawmidi_read, (rmidi: *mut c_void, buffer: *mut c_void, size: usize), -11);
resolve_asound!(isize, snd_rawmidi_write, (rmidi: *mut c_void, buffer: *const c_void, size: usize), -19);
resolve_asound!(c_int, snd_rawmidi_drain, (rmidi: *mut c_void), -19);
resolve_asound!(c_int, snd_rawmidi_drop, (rmidi: *mut c_void), -19);
resolve_asound!(c_int, snd_rawmidi_nonblock, (rmidi: *mut c_void, nonblock: c_int), -19);
resolve_asound!(c_int, snd_rawmidi_poll_descriptors_count, (rmidi: *mut c_void), 0);
resolve_asound!(c_int, snd_rawmidi_poll_descriptors, (rmidi: *mut c_void, pfds: *mut PollFd, space: c_uint), 0);
resolve_asound!(c_int, snd_rawmidi_poll_descriptors_revents, (rmidi: *mut c_void, pfds: *mut PollFd, nfds: c_uint, revent: *mut c_ushort), -19);

resolve_asound!(c_int, snd_device_name_hint, (card: c_int, iface: *const c_char, hints: *mut *mut *mut c_void), -2);
resolve_asound!(*mut c_char, snd_device_name_get_hint, (hint: *const c_void, id: *const c_char), ptr::null_mut());
resolve_asound!(c_int, snd_device_name_free_hint, (hints: *mut *mut c_void), 0);

#[no_mangle]
pub unsafe extern "C" fn snd_card_next(rcard: *mut c_int) -> c_int {
    static FN_PTR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    let sym = resolve_sym(&FN_PTR, c"snd_card_next");
    if !sym.is_null() {
        let f: unsafe extern "C" fn(*mut c_int) -> c_int = unsafe { mem::transmute(sym) };
        return unsafe { f(rcard) };
    }
    if !rcard.is_null() {
        unsafe { *rcard = -1 };
    }
    -19
}
resolve_asound!(c_int, snd_card_get_index, (string: *const c_char), -19);
resolve_asound!(c_int, snd_card_get_name, (card: c_int, name: *mut *mut c_char), -19);
resolve_asound!(c_int, snd_card_get_longname, (card: c_int, name: *mut *mut c_char), -19);

resolve_asound!(c_int, snd_ctl_open, (ctl: *mut *mut c_void, name: *const c_char, mode: c_int), -19);
resolve_asound!(c_int, snd_ctl_close, (ctl: *mut c_void), 0);
resolve_asound!(c_int, snd_ctl_card_info, (ctl: *mut c_void, info: *mut c_void), -19);
resolve_asound!(usize, snd_ctl_card_info_sizeof, (), 512);
resolve_asound!(c_int, snd_ctl_card_info_malloc, (ptr: *mut *mut c_void), -12);
resolve_asound_void!(snd_ctl_card_info_free, (obj: *mut c_void));
resolve_asound!(*const c_char, snd_ctl_card_info_get_name, (obj: *const c_void), c"LASAPI".as_ptr());
resolve_asound!(*const c_char, snd_ctl_card_info_get_id, (obj: *const c_void), c"lasapi".as_ptr());
resolve_asound!(c_int, snd_ctl_pcm_next_device, (ctl: *mut c_void, device: *mut c_int), -19);
resolve_asound!(c_int, snd_ctl_pcm_info, (ctl: *mut c_void, info: *mut c_void), -19);

resolve_asound!(c_int, snd_mixer_open, (mixer: *mut *mut c_void, mode: c_int), -19);
resolve_asound!(c_int, snd_mixer_close, (mixer: *mut c_void), 0);
resolve_asound!(c_int, snd_mixer_attach, (mixer: *mut c_void, name: *const c_char), -19);
resolve_asound!(c_int, snd_mixer_load, (mixer: *mut c_void), -19);
resolve_asound!(c_int, snd_mixer_selem_register, (mixer: *mut c_void, options: *mut c_void, classp: *mut *mut c_void), -19);
resolve_asound!(*mut c_void, snd_mixer_first_elem, (mixer: *mut c_void), ptr::null_mut());
resolve_asound!(*mut c_void, snd_mixer_elem_next, (elem: *mut c_void), ptr::null_mut());
resolve_asound!(*mut c_void, snd_mixer_find_selem, (mixer: *mut c_void, id: *const c_void), ptr::null_mut());
resolve_asound!(*const c_char, snd_mixer_selem_get_name, (elem: *mut c_void), c"Master".as_ptr());
resolve_asound!(c_int, snd_mixer_selem_has_playback_volume, (elem: *mut c_void), 0);
resolve_asound!(c_int, snd_mixer_selem_has_capture_volume, (elem: *mut c_void), 0);
resolve_asound!(c_int, snd_mixer_selem_get_playback_volume_range, (elem: *mut c_void, min: *mut c_long, max: *mut c_long), -19);
resolve_asound!(c_int, snd_mixer_selem_get_playback_volume, (elem: *mut c_void, channel: c_int, value: *mut c_long), -19);
resolve_asound!(c_int, snd_mixer_selem_set_playback_volume_all, (elem: *mut c_void, value: c_long), -19);

fn map_alsa_fmt(alsa_fmt: c_int) -> u32 {
    if alsa_fmt == SND_PCM_FORMAT_S16_LE {
        LASAPI_FMT_S16_LE
    } else if alsa_fmt == SND_PCM_FORMAT_S32_LE {
        LASAPI_FMT_S32_LE
    } else {
        LASAPI_FMT_FLOAT32
    }
}

unsafe fn ensure_lasapi_initialized(pcm: *mut snd_pcm_t) -> c_int {
    if pcm.is_null() {
        return -22;
    }
    let (
        need_reinit,
        is_capture,
        sample_rate,
        channels,
        fmt,
        buffer_frames,
        period_frames,
    ) = {
        let p = unsafe { &mut *pcm };
        let changed = p.started
            && (p.init_rate != p.sample_rate
                || p.init_channels != p.channels
                || p.init_fmt != p.fmt
                || p.init_buffer_frames != p.buffer_frames
                || p.init_period_frames != p.period_frames);
        if changed && !p.client.is_null() {
            unsafe {
                IAudioClient_Stop(p.client);
                IAudioClient_Release(p.client);
            }
            p.client = ptr::null_mut();
            p.event_fd = -1;
            p.started = false;
        }
        (
            !p.started,
            p.is_capture,
            p.sample_rate,
            p.channels,
            p.fmt,
            p.buffer_frames,
            p.period_frames,
        )
    };
    if !need_reinit {
        return 0;
    }
    let client_ptr = unsafe { (*pcm).client };
    if client_ptr.is_null() {
        let cname = if is_capture {
            c"ALSA-Cap(default)".as_ptr()
        } else {
            c"ALSA-Play(default)".as_ptr()
        };
        if unsafe { LASAPI_ActivateAudioClient(cname, ptr::addr_of_mut!((*pcm).client)) } != S_OK
        {
            return -19;
        }
    }
    let p = unsafe { &mut *pcm };
    let bits: u16 = if fmt == LASAPI_FMT_S16_LE { 16 } else { 32 };
    let block_align = (channels as u16) * (bits / 8);
    let wfx = WaveFormatEx {
        w_format_tag: fmt as u16,
        n_channels: channels as u16,
        n_samples_per_sec: sample_rate,
        w_bits_per_sample: bits,
        n_block_align: block_align,
        n_avg_bytes_per_sec: sample_rate * u32::from(block_align),
        cb_size: 0,
    };
    let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if is_capture {
        flags |= AUDCLNT_STREAMFLAGS_CAPTURE;
    }
    let hr = unsafe {
        IAudioClient_Initialize(
            p.client,
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            buffer_frames,
            period_frames,
            &wfx,
        )
    };
    if hr != S_OK && hr != AUDCLNT_E_ALREADY_INITIALIZED {
        return -5;
    }
    unsafe {
        IAudioClient_SetEventHandle(p.client, &mut p.event_fd);
        IAudioClient_GetBufferSize(p.client, &mut p.buffer_frames);
        IAudioClient_GetDevicePeriod(p.client, &mut p.period_frames, ptr::null_mut());
        IAudioClient_Start(p.client);
    }
    p.init_rate = sample_rate;
    p.init_channels = channels;
    p.init_fmt = fmt;
    p.init_buffer_frames = buffer_frames;
    p.init_period_frames = period_frames;
    p.started = true;
    p.paused = false;
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_open(
    pcmp: *mut *mut snd_pcm_t,
    name: *const c_char,
    stream: c_int,
    _mode: c_int,
) -> c_int {
    if pcmp.is_null() {
        return -22;
    }
    let pcm = unsafe { calloc(1, mem::size_of::<snd_pcm_t>()) }.cast::<snd_pcm_t>();
    if pcm.is_null() {
        return -12;
    }
    let p = unsafe { &mut *pcm };
    p.is_capture = stream == 1;
    let name_str = if name.is_null() {
        "default"
    } else {
        unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("default")
    };
    let cname = format!(
        "ALSA-{}({name_str})\0",
        if p.is_capture { "Cap" } else { "Play" }
    );
    if unsafe { LASAPI_ActivateAudioClient(cname.as_ptr().cast(), &mut p.client) } != S_OK {
        unsafe {
            LASAPI_DebugTraceClient(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_ERR,
                0,
                c"SHIM_ALSA_ERR".as_ptr(),
                cname.as_ptr().cast(),
                c"LASAPI_ActivateAudioClient failed".as_ptr(),
            );
            free(pcm.cast());
        }
        return -19;
    }

    unsafe {
        LASAPI_DebugTraceClient(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            c"SHIM_ALSA_OPEN".as_ptr(),
            cname.as_ptr().cast(),
            if p.is_capture {
                c"stream=CAPTURE".as_ptr()
            } else {
                c"stream=PLAYBACK".as_ptr()
            },
        );
    }

    p.event_fd = -1;
    p.sample_rate = 48000;
    p.channels = 2;
    p.fmt = LASAPI_FMT_FLOAT32;
    p.buffer_frames = 2048;
    p.period_frames = 256;
    unsafe { *pcmp = pcm };
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_open_lconf(
    pcmp: *mut *mut snd_pcm_t,
    name: *const c_char,
    stream: c_int,
    mode: c_int,
    _lconf: *mut c_void,
) -> c_int {
    unsafe { snd_pcm_open(pcmp, name, stream, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_close(pcm: *mut snd_pcm_t) -> c_int {
    if pcm.is_null() {
        return -22;
    }
    let p = unsafe { &mut *pcm };
    unsafe {
        LASAPI_DebugTraceClient(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            c"SHIM_ALSA_CLOSE".as_ptr(),
            if p.is_capture {
                c"ALSA-Cap".as_ptr()
            } else {
                c"ALSA-Play".as_ptr()
            },
            c"snd_pcm_close".as_ptr(),
        );
        if !p.client.is_null() {
            IAudioClient_Release(p.client);
        }
        free(pcm.cast());
    }
    0
}

#[no_mangle]
pub extern "C" fn snd_pcm_nonblock(_pcm: *mut snd_pcm_t, _nonblock: c_int) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_set_params(
    pcm: *mut snd_pcm_t,
    format: c_int,
    _access: c_int,
    channels: c_uint,
    rate: c_uint,
    _soft_resample: c_int,
    latency: c_uint,
) -> c_int {
    if pcm.is_null() {
        return -22;
    }
    {
        let p = unsafe { &mut *pcm };
        p.fmt = map_alsa_fmt(format);
        p.channels = if channels != 0 { channels } else { 2 };
        p.sample_rate = if rate != 0 { rate } else { 48000 };
        p.buffer_frames = if latency > 0 {
            ((u64::from(p.sample_rate) * u64::from(latency)) / 1_000_000) as u32
        } else {
            2048
        };
        if p.buffer_frames < 512 {
            p.buffer_frames = 1024;
        }
        p.period_frames = 256;
    }
    unsafe { ensure_lasapi_initialized(pcm) }
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_get_params(
    pcm: *mut snd_pcm_t,
    buffer_size: *mut c_ulong,
    period_size: *mut c_ulong,
) -> c_int {
    if pcm.is_null() {
        return -22;
    }
    let p = unsafe { &*pcm };
    if !buffer_size.is_null() {
        unsafe { *buffer_size = c_ulong::from(p.buffer_frames) };
    }
    if !period_size.is_null() {
        unsafe { *period_size = c_ulong::from(p.period_frames) };
    }
    0
}

#[no_mangle]
pub extern "C" fn snd_pcm_hw_params_sizeof() -> usize {
    mem::size_of::<snd_pcm_hw_params_t>()
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_malloc(ptr_out: *mut *mut snd_pcm_hw_params_t) -> c_int {
    if ptr_out.is_null() {
        return -22;
    }
    let p = unsafe { calloc(1, mem::size_of::<snd_pcm_hw_params_t>()) }
        .cast::<snd_pcm_hw_params_t>();
    unsafe { *ptr_out = p };
    if p.is_null() { -12 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_free(obj: *mut snd_pcm_hw_params_t) {
    unsafe { free(obj.cast()) };
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_any(
    _pcm: *mut snd_pcm_t,
    params: *mut snd_pcm_hw_params_t,
) -> c_int {
    if params.is_null() {
        return -22;
    }
    let p = unsafe { &mut *params };
    p.rate = 48000;
    p.channels = 2;
    p.alsa_format = SND_PCM_FORMAT_FLOAT_LE;
    p.buffer_frames = 2048;
    p.period_frames = 256;
    0
}

#[no_mangle]
pub extern "C" fn snd_pcm_hw_params_set_access(
    _pcm: *mut snd_pcm_t,
    _params: *mut snd_pcm_hw_params_t,
    _access: c_int,
) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_set_format(
    _pcm: *mut snd_pcm_t,
    params: *mut snd_pcm_hw_params_t,
    val: c_int,
) -> c_int {
    if !params.is_null() {
        unsafe { (*params).alsa_format = val };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_set_channels(
    _pcm: *mut snd_pcm_t,
    params: *mut snd_pcm_hw_params_t,
    val: c_uint,
) -> c_int {
    if !params.is_null() {
        unsafe { (*params).channels = val };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_get_channels(
    params: *const snd_pcm_hw_params_t,
    val: *mut c_uint,
) -> c_int {
    if !val.is_null() {
        unsafe {
            *val = if !params.is_null() && (*params).channels != 0 {
                (*params).channels
            } else {
                2
            };
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_get_channels_max(
    _params: *const snd_pcm_hw_params_t,
    val: *mut c_uint,
) -> c_int {
    if !val.is_null() {
        unsafe { *val = 2 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_get_rate(
    params: *const snd_pcm_hw_params_t,
    val: *mut c_uint,
    dir: *mut c_int,
) -> c_int {
    if !val.is_null() {
        unsafe {
            *val = if !params.is_null() && (*params).rate != 0 {
                (*params).rate
            } else {
                48000
            };
        }
    }
    if !dir.is_null() {
        unsafe { *dir = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_get_period_size(
    params: *const snd_pcm_hw_params_t,
    val: *mut c_ulong,
    dir: *mut c_int,
) -> c_int {
    if !val.is_null() {
        unsafe {
            *val = c_ulong::from(if !params.is_null() && (*params).period_frames != 0 {
                (*params).period_frames
            } else {
                256
            });
        }
    }
    if !dir.is_null() {
        unsafe { *dir = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_get_buffer_size(
    params: *const snd_pcm_hw_params_t,
    val: *mut c_ulong,
) -> c_int {
    if !val.is_null() {
        unsafe {
            *val = c_ulong::from(if !params.is_null() && (*params).buffer_frames != 0 {
                (*params).buffer_frames
            } else {
                2048
            });
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_set_rate_near(
    _pcm: *mut snd_pcm_t,
    params: *mut snd_pcm_hw_params_t,
    val: *mut c_uint,
    _dir: *mut c_int,
) -> c_int {
    if !params.is_null() && !val.is_null() {
        unsafe { (*params).rate = *val };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_set_period_size_near(
    _pcm: *mut snd_pcm_t,
    params: *mut snd_pcm_hw_params_t,
    val: *mut c_ulong,
    _dir: *mut c_int,
) -> c_int {
    if !params.is_null() && !val.is_null() {
        unsafe { (*params).period_frames = *val as u32 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params_set_buffer_size_near(
    _pcm: *mut snd_pcm_t,
    params: *mut snd_pcm_hw_params_t,
    val: *mut c_ulong,
) -> c_int {
    if !params.is_null() && !val.is_null() {
        unsafe { (*params).buffer_frames = *val as u32 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_hw_params(
    pcm: *mut snd_pcm_t,
    params: *mut snd_pcm_hw_params_t,
) -> c_int {
    if pcm.is_null() || params.is_null() {
        return -22;
    }
    let (is_cap, sr, ch, bf, pf) = {
        let p = unsafe { &mut *pcm };
        let pr = unsafe { &*params };
        p.sample_rate = if pr.rate != 0 { pr.rate } else { 48000 };
        p.channels = if pr.channels != 0 { pr.channels } else { 2 };
        p.fmt = map_alsa_fmt(pr.alsa_format);
        p.buffer_frames = if pr.buffer_frames != 0 {
            pr.buffer_frames
        } else {
            2048
        };
        p.period_frames = if pr.period_frames != 0 {
            pr.period_frames
        } else {
            256
        };
        (p.is_capture, p.sample_rate, p.channels, p.buffer_frames, p.period_frames)
    };
    let detail = format!(
        "{} rate={sr} ch={ch} buf={bf} per={pf}\0",
        if is_cap { "CAPTURE" } else { "PLAYBACK" },
    );
    unsafe {
        LASAPI_DebugTraceClient(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            c"SHIM_ALSA_HW".as_ptr(),
            if is_cap {
                c"ALSA-Cap".as_ptr()
            } else {
                c"ALSA-Play".as_ptr()
            },
            detail.as_ptr().cast(),
        );
        ensure_lasapi_initialized(pcm)
    }
}

#[no_mangle]
pub extern "C" fn snd_pcm_sw_params_sizeof() -> usize {
    mem::size_of::<snd_pcm_sw_params_t>()
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_sw_params_malloc(ptr_out: *mut *mut snd_pcm_sw_params_t) -> c_int {
    if ptr_out.is_null() {
        return -22;
    }
    let p = unsafe { calloc(1, mem::size_of::<snd_pcm_sw_params_t>()) }
        .cast::<snd_pcm_sw_params_t>();
    unsafe { *ptr_out = p };
    if p.is_null() { -12 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_sw_params_free(obj: *mut snd_pcm_sw_params_t) {
    unsafe { free(obj.cast()) };
}

#[no_mangle]
pub extern "C" fn snd_pcm_sw_params_current(
    _pcm: *mut snd_pcm_t,
    _params: *mut snd_pcm_sw_params_t,
) -> c_int {
    0
}

#[no_mangle]
pub extern "C" fn snd_pcm_sw_params(
    _pcm: *mut snd_pcm_t,
    _params: *mut snd_pcm_sw_params_t,
) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_prepare(pcm: *mut snd_pcm_t) -> c_int {
    if pcm.is_null() {
        return -22;
    }
    if unsafe { !(*pcm).started } {
        return unsafe { ensure_lasapi_initialized(pcm) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_start(pcm: *mut snd_pcm_t) -> c_int {
    if pcm.is_null() {
        return -22;
    }
    if unsafe { !(*pcm).started } && unsafe { ensure_lasapi_initialized(pcm) } < 0 {
        return -5;
    }
    let p = unsafe { &mut *pcm };
    unsafe { IAudioClient_Start(p.client) };
    p.paused = false;
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_pause(pcm: *mut snd_pcm_t, enable: c_int) -> c_int {
    if pcm.is_null() {
        return -22;
    }
    let p = unsafe { &mut *pcm };
    if !p.started {
        return -22;
    }
    if enable != 0 {
        unsafe { IAudioClient_Stop(p.client) };
        p.paused = true;
    } else {
        unsafe { IAudioClient_Start(p.client) };
        p.paused = false;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_state(pcm: *mut snd_pcm_t) -> c_int {
    if pcm.is_null() || unsafe { !(*pcm).started } {
        return SND_PCM_STATE_PREPARED;
    }
    if unsafe { (*pcm).paused } {
        return SND_PCM_STATE_PAUSED;
    }
    SND_PCM_STATE_RUNNING
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_delay(pcm: *mut snd_pcm_t, delayp: *mut c_long) -> c_int {
    if pcm.is_null() || unsafe { !(*pcm).started } {
        return -22;
    }
    let mut pad = 0u32;
    unsafe { IAudioClient_GetCurrentPadding((*pcm).client, &mut pad) };
    if !delayp.is_null() {
        unsafe { *delayp = c_long::from(pad as i32) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_frames_to_bytes(pcm: *mut snd_pcm_t, frames: c_long) -> isize {
    if pcm.is_null() {
        return 0;
    }
    let p = unsafe { &*pcm };
    let bps: isize = if p.fmt == LASAPI_FMT_S16_LE { 2 } else { 4 };
    (frames as isize) * (p.channels as isize) * bps
}

#[no_mangle]
pub extern "C" fn snd_pcm_poll_descriptors_count(_pcm: *mut snd_pcm_t) -> c_int {
    1
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_poll_descriptors(
    pcm: *mut snd_pcm_t,
    pfds: *mut PollFd,
    space: c_uint,
) -> c_int {
    if pcm.is_null() || pfds.is_null() || space < 1 {
        return -22;
    }
    if unsafe { !(*pcm).started } {
        let _ = unsafe { ensure_lasapi_initialized(pcm) };
    }
    let p = unsafe { &*pcm };
    unsafe {
        (*pfds).fd = p.event_fd;
        (*pfds).events = POLLIN;
        (*pfds).revents = 0;
    }
    1
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_poll_descriptors_revents(
    pcm: *mut snd_pcm_t,
    pfds: *mut PollFd,
    nfds: c_uint,
    revents: *mut c_ushort,
) -> c_int {
    if pcm.is_null() || pfds.is_null() || nfds < 1 || revents.is_null() {
        return -22;
    }
    let p = unsafe { &*pcm };
    if unsafe { ((*pfds).revents & POLLIN) != 0 } {
        let mut val = 0u64;
        let _ = unsafe { read(p.event_fd, (&mut val as *mut u64).cast(), 8) };
    }
    let avail = unsafe { snd_pcm_avail_update(pcm) };
    unsafe {
        *revents = if avail >= c_long::from(p.period_frames as i32) {
            if p.is_capture {
                POLLIN as c_ushort
            } else {
                POLLOUT as c_ushort
            }
        } else {
            0
        };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_writei(
    pcm: *mut snd_pcm_t,
    buffer: *const c_void,
    size: c_ulong,
) -> c_long {
    if pcm.is_null() || buffer.is_null() {
        return -22;
    }
    if unsafe { !(*pcm).started } && unsafe { ensure_lasapi_initialized(pcm) } < 0 {
        return -5;
    }
    let p = unsafe { &*pcm };
    if unsafe { IAudioRenderClient_WriteFrames(p.client, buffer, size as u32, p.fmt) } != S_OK {
        return -5;
    }
    size as c_long
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_readi(
    pcm: *mut snd_pcm_t,
    buffer: *mut c_void,
    size: c_ulong,
) -> c_long {
    if pcm.is_null() || buffer.is_null() {
        return -22;
    }
    if unsafe { !(*pcm).started } && unsafe { ensure_lasapi_initialized(pcm) } < 0 {
        return -5;
    }
    let p = unsafe { &*pcm };
    let mut pkt = 0u32;
    unsafe { IAudioCaptureClient_GetNextPacketSize(p.client, &mut pkt) };
    if c_ulong::from(pkt) < size {
        unsafe { IAudioClient_WaitForPeriod(p.client, 15) };
    }
    if unsafe { IAudioCaptureClient_ReadFrames(p.client, buffer, size as u32, p.fmt) } != S_OK {
        return -5;
    }
    size as c_long
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_avail_update(pcm: *mut snd_pcm_t) -> c_long {
    if pcm.is_null() || unsafe { !(*pcm).started } {
        return 0;
    }
    let p = unsafe { &*pcm };
    if p.is_capture {
        let mut pkt = 0u32;
        unsafe { IAudioCaptureClient_GetNextPacketSize(p.client, &mut pkt) };
        return pkt as c_long;
    }
    let mut buf = 0u32;
    let mut pad = 0u32;
    unsafe {
        IAudioClient_GetBufferSize(p.client, &mut buf);
        IAudioClient_GetCurrentPadding(p.client, &mut pad);
    }
    buf.saturating_sub(pad) as c_long
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_avail(pcm: *mut snd_pcm_t) -> c_long {
    unsafe { snd_pcm_avail_update(pcm) }
}

#[no_mangle]
pub extern "C" fn snd_pcm_forward(_pcm: *mut snd_pcm_t, frames: c_ulong) -> c_long {
    frames as c_long
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_wait(pcm: *mut snd_pcm_t, timeout: c_int) -> c_int {
    if pcm.is_null() || unsafe { !(*pcm).started } {
        return -22;
    }
    unsafe { IAudioClient_WaitForPeriod((*pcm).client, timeout) };
    1
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_drain(pcm: *mut snd_pcm_t) -> c_int {
    if pcm.is_null() || unsafe { !(*pcm).started } {
        return 0;
    }
    let p = unsafe { &*pcm };
    if p.is_capture {
        unsafe { IAudioClient_Stop(p.client) };
        return 0;
    }
    for _ in 0..50 {
        let mut pad = 0u32;
        unsafe { IAudioClient_GetCurrentPadding(p.client, &mut pad) };
        if pad == 0 {
            break;
        }
        unsafe { IAudioClient_WaitForPeriod(p.client, 10) };
    }
    unsafe { IAudioClient_Stop(p.client) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_drop(pcm: *mut snd_pcm_t) -> c_int {
    if pcm.is_null() || unsafe { !(*pcm).started } {
        return 0;
    }
    let p = unsafe { &*pcm };
    unsafe {
        IAudioClient_Stop(p.client);
        IAudioClient_Reset(p.client);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snd_pcm_recover(
    pcm: *mut snd_pcm_t,
    _err: c_int,
    _silent: c_int,
) -> c_int {
    unsafe { snd_pcm_prepare(pcm) }
}

#[no_mangle]
pub unsafe extern "C" fn snd_strerror(errnum: c_int) -> *const c_char {
    unsafe { strerror(if errnum < 0 { -errnum } else { errnum }) }
}
