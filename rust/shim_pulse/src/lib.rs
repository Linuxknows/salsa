// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicI32, Ordering};
use std::ffi::CString;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use salsa_host_dl::pa_abi::{
    pa_buffer_attr, pa_channel_map, pa_cvolume, pa_mainloop_api, pa_sample_spec, pa_server_info,
    pa_sink_info, pa_sink_input_info, pa_source_info, pa_timing_info, PA_CHANNELS_MAX,
    PA_CHANNEL_POSITION_FRONT_LEFT, PA_CHANNEL_POSITION_FRONT_RIGHT, PA_CONTEXT_FAILED,
    PA_CONTEXT_READY, PA_CONTEXT_TERMINATED, PA_CONTEXT_UNCONNECTED, PA_INVALID_INDEX,
    PA_OPERATION_CANCELLED, PA_OPERATION_DONE, PA_SAMPLE_FLOAT32LE, PA_SAMPLE_S16BE,
    PA_SAMPLE_S16LE, PA_SAMPLE_S32BE, PA_SAMPLE_S32LE, PA_SINK_HARDWARE, PA_SINK_LATENCY,
    PA_SINK_RUNNING, PA_SOURCE_HARDWARE, PA_SOURCE_LATENCY, PA_SOURCE_RUNNING, PA_STREAM_FAILED,
    PA_STREAM_READY, PA_STREAM_RECORD, PA_STREAM_START_CORKED, PA_STREAM_TERMINATED,
    PA_STREAM_UNCONNECTED, PA_VOLUME_MUTED, PA_VOLUME_NORM,
};
use salsa_wire::c_abi::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_CAPTURE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, LASAPI_DBG_ERR, LASAPI_DBG_INFO, LASAPI_DBG_SHIM,
    LASAPI_FMT_FLOAT32, LASAPI_FMT_S16_LE, LASAPI_FMT_S32_LE, LASAPI_RING_FRAMES,
    WaveFormatEx, S_OK,
};
use salsa_wire::client_seam::ClientHandle;

salsa_wire::define_lasapi_ffi_backend!();

const MAX_PA_STREAMS: usize = 16;
const PTHREAD_MUTEX_RECURSIVE: c_int = 1;
const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

pub type PaContextNotifyCb = Option<unsafe extern "C" fn(*mut pa_context, *mut c_void)>;
pub type PaContextSuccessCb = Option<unsafe extern "C" fn(*mut pa_context, c_int, *mut c_void)>;
pub type PaContextSubscribeCb =
    Option<unsafe extern "C" fn(*mut pa_context, c_uint, u32, *mut c_void)>;
pub type PaServerInfoCb =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_server_info, *mut c_void)>;
pub type PaSinkInfoCb =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_sink_info, c_int, *mut c_void)>;
pub type PaSourceInfoCb =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_source_info, c_int, *mut c_void)>;
pub type PaSinkInputInfoCb =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_sink_input_info, c_int, *mut c_void)>;
pub type PaStreamNotifyCb = Option<unsafe extern "C" fn(*mut pa_stream, *mut c_void)>;
pub type PaStreamRequestCb = Option<unsafe extern "C" fn(*mut pa_stream, usize, *mut c_void)>;
pub type PaStreamSuccessCb = Option<unsafe extern "C" fn(*mut pa_stream, c_int, *mut c_void)>;
pub type PaOperationNotifyCb = Option<unsafe extern "C" fn(*mut pa_operation, *mut c_void)>;
pub type PaFreeCb = Option<unsafe extern "C" fn(*mut c_void)>;

#[repr(C)]
pub struct pa_threaded_mainloop {
    mutex: [u64; 8],
    cond: [u64; 8],
    thread: c_ulong,
    running: AtomicI32,
    in_thread: AtomicI32,
    api: pa_mainloop_api,
    ctx: *mut pa_context,
}

#[repr(C)]
pub struct pa_operation {
    ref_cnt: AtomicI32,
    state: c_int,
    cb: PaOperationNotifyCb,
    userdata: *mut c_void,
}

#[repr(C)]
pub struct pa_proplist {
    name: [u8; 64],
}

#[repr(C)]
pub struct pa_context {
    ref_cnt: AtomicI32,
    mainloop: *mut pa_threaded_mainloop,
    state: c_int,
    state_cb: PaContextNotifyCb,
    state_userdata: *mut c_void,
    sub_cb: PaContextSubscribeCb,
    sub_userdata: *mut c_void,
    streams: [*mut pa_stream; MAX_PA_STREAMS],
}

#[repr(C)]
pub struct pa_stream {
    ref_cnt: AtomicI32,
    ctx: *mut pa_context,
    name: [u8; 64],
    ss: pa_sample_spec,
    cmap: pa_channel_map,
    battr: pa_buffer_attr,
    state: c_int,
    corked: c_int,
    is_record: bool,
    index: u32,
    lasapi: ClientHandle,
    lfmt: u32,
    frame_bytes: u32,
    write_scratch: *mut c_void,
    write_scratch_cap: usize,
    read_scratch: *mut c_void,
    read_scratch_cap: usize,
    read_scratch_len: usize,
    bytes_written: u64,
    timing: pa_timing_info,
    state_cb: PaStreamNotifyCb,
    state_userdata: *mut c_void,
    write_cb: PaStreamRequestCb,
    write_userdata: *mut c_void,
    read_cb: PaStreamRequestCb,
    read_userdata: *mut c_void,
}

#[repr(C)]
pub struct pa_simple {
    client: ClientHandle,
    rate: u32,
    channels: u32,
    fmt: u32,
    frame_bytes: u32,
    is_record: bool,
}

extern "C" {
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn clock_gettime(clk_id: c_int, tp: *mut Timespec) -> c_int;

    fn pthread_mutexattr_init(attr: *mut c_void) -> c_int;
    fn pthread_mutexattr_destroy(attr: *mut c_void) -> c_int;
    fn pthread_mutexattr_settype(attr: *mut c_void, kind: c_int) -> c_int;
    fn pthread_mutex_init(mutex: *mut c_void, attr: *const c_void) -> c_int;
    fn pthread_mutex_destroy(mutex: *mut c_void) -> c_int;
    fn pthread_mutex_lock(mutex: *mut c_void) -> c_int;
    fn pthread_mutex_unlock(mutex: *mut c_void) -> c_int;
    fn pthread_cond_init(cond: *mut c_void, attr: *const c_void) -> c_int;
    fn pthread_cond_destroy(cond: *mut c_void) -> c_int;
    fn pthread_cond_broadcast(cond: *mut c_void) -> c_int;
    fn pthread_cond_timedwait(
        cond: *mut c_void,
        mutex: *mut c_void,
        abstime: *const Timespec,
    ) -> c_int;
    fn pthread_create(
        thread: *mut c_ulong,
        attr: *const c_void,
        start_routine: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        arg: *mut c_void,
    ) -> c_int;
    fn pthread_join(thread: c_ulong, retval: *mut *mut c_void) -> c_int;
}

fn copy_cstr_buf(dst: &mut [u8], src: *const c_char, fallback: &str) {
    dst.fill(0);
    if dst.is_empty() {
        return;
    }
    let s = if src.is_null() {
        fallback.as_bytes()
    } else {
        unsafe { CStr::from_ptr(src) }.to_bytes()
    };
    let n = s.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&s[..n]);
}

fn map_pa_fmt(pa_fmt: c_int, bytes_per_sample: &mut u32) -> u32 {
    if pa_fmt == PA_SAMPLE_S16LE || pa_fmt == PA_SAMPLE_S16BE {
        *bytes_per_sample = 2;
        return LASAPI_FMT_S16_LE;
    }
    if pa_fmt == PA_SAMPLE_S32LE || pa_fmt == PA_SAMPLE_S32BE {
        *bytes_per_sample = 4;
        return LASAPI_FMT_S32_LE;
    }
    *bytes_per_sample = 4;
    LASAPI_FMT_FLOAT32
}

unsafe fn alloc_done_op() -> *mut pa_operation {
    let op = unsafe { calloc(1, mem::size_of::<pa_operation>()) }.cast::<pa_operation>();
    if !op.is_null() {
        unsafe {
            (*op).ref_cnt.store(1, Ordering::Release);
            (*op).state = PA_OPERATION_DONE;
        }
    }
    op
}

fn current_user_cstr() -> *const c_char {
    static USER_C: OnceLock<CString> = OnceLock::new();
    USER_C
        .get_or_init(|| {
            let name = std::env::var("USER")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| std::env::var("LOGNAME").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "user".to_string());
            CString::new(name).unwrap_or_else(|_| CString::new("user").unwrap())
        })
        .as_ptr()
}

unsafe extern "C" fn mainloop_worker(arg: *mut c_void) -> *mut c_void {
    let m = unsafe { &mut *arg.cast::<pa_threaded_mainloop>() };
    while m.running.load(Ordering::Acquire) != 0 {
        thread::sleep(Duration::from_millis(4));

        unsafe { pthread_mutex_lock(m.mutex.as_mut_ptr().cast()) };
        m.in_thread.store(1, Ordering::Release);

        let ctx = m.ctx;
        if !ctx.is_null() && unsafe { (*ctx).state == PA_CONTEXT_READY } {
            unsafe { pa_context_ref(ctx) };
            for i in 0..MAX_PA_STREAMS {
                let s = unsafe { (*ctx).streams[i] };
                if s.is_null() {
                    continue;
                }
                unsafe { pa_stream_ref(s) };
                let (is_rec, r_cb, r_ud, w_cb, w_ud, lasapi, fbytes) = {
                    let s_ref = unsafe { &*s };
                    if s_ref.state != PA_STREAM_READY
                        || s_ref.corked != 0
                        || s_ref.lasapi.is_null()
                    {
                        (false, None, ptr::null_mut(), None, ptr::null_mut(), ClientHandle::NULL, 0)
                    } else {
                        (
                            s_ref.is_record,
                            s_ref.read_cb,
                            s_ref.read_userdata,
                            s_ref.write_cb,
                            s_ref.write_userdata,
                            s_ref.lasapi,
                            s_ref.frame_bytes as usize,
                        )
                    }
                };
                if !lasapi.is_null() {
                    if is_rec {
                        if let Some(cb) = r_cb {
                            let pkt_frames =
                                with_lasapi_backend(|b| b.get_next_packet_size(lasapi))
                                    .unwrap_or(0);
                            if pkt_frames >= 256 {
                                let readable_bytes = (pkt_frames as usize) * fbytes;
                                unsafe { cb(s, readable_bytes, r_ud) };
                            }
                        }
                    } else if let Some(cb) = w_cb {
                        let (buf_frames, pad_frames) = with_lasapi_backend(|b| {
                            (
                                b.get_buffer_size(lasapi).unwrap_or(2048),
                                b.get_current_padding(lasapi).unwrap_or(0),
                            )
                        });
                        let avail_frames = buf_frames.saturating_sub(pad_frames);
                        if avail_frames >= 256 {
                            let writable_bytes = (avail_frames as usize) * fbytes;
                            unsafe { cb(s, writable_bytes, w_ud) };
                        }
                    }
                }
                unsafe { pa_stream_unref(s) };
            }
            unsafe { pa_context_unref(ctx) };
        }

        m.in_thread.store(0, Ordering::Release);
        unsafe {
            pthread_cond_broadcast(m.cond.as_mut_ptr().cast());
            pthread_mutex_unlock(m.mutex.as_mut_ptr().cast());
        }
    }
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_new() -> *mut pa_threaded_mainloop {
    let m = unsafe { calloc(1, mem::size_of::<pa_threaded_mainloop>()) }
        .cast::<pa_threaded_mainloop>();
    if m.is_null() {
        return ptr::null_mut();
    }
    let m_ref = unsafe { &mut *m };
    let mut attr = [0u64; 4];
    unsafe {
        pthread_mutexattr_init(attr.as_mut_ptr().cast());
        pthread_mutexattr_settype(attr.as_mut_ptr().cast(), PTHREAD_MUTEX_RECURSIVE);
        pthread_mutex_init(m_ref.mutex.as_mut_ptr().cast(), attr.as_ptr().cast());
        pthread_mutexattr_destroy(attr.as_mut_ptr().cast());
        pthread_cond_init(m_ref.cond.as_mut_ptr().cast(), ptr::null());
    }
    m_ref.api.userdata = m.cast();
    m
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_free(m: *mut pa_threaded_mainloop) {
    if m.is_null() {
        return;
    }
    let m_ref = unsafe { &mut *m };
    if m_ref.running.swap(0, Ordering::AcqRel) != 0 {
        unsafe { pthread_join(m_ref.thread, ptr::null_mut()) };
    }
    unsafe {
        pthread_mutex_destroy(m_ref.mutex.as_mut_ptr().cast());
        pthread_cond_destroy(m_ref.cond.as_mut_ptr().cast());
        free(m.cast());
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_start(m: *mut pa_threaded_mainloop) -> c_int {
    if m.is_null() {
        return -1;
    }
    let m_ref = unsafe { &mut *m };
    if m_ref
        .running
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let rc = unsafe {
            pthread_create(&mut m_ref.thread, ptr::null(), mainloop_worker, m.cast())
        };
        if rc != 0 {
            m_ref.running.store(0, Ordering::Release);
            return -1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_stop(m: *mut pa_threaded_mainloop) {
    if m.is_null() {
        return;
    }
    let m_ref = unsafe { &mut *m };
    if m_ref.running.swap(0, Ordering::AcqRel) != 0 {
        unsafe { pthread_join(m_ref.thread, ptr::null_mut()) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_lock(m: *mut pa_threaded_mainloop) {
    if !m.is_null() {
        unsafe { pthread_mutex_lock((*m).mutex.as_mut_ptr().cast()) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_unlock(m: *mut pa_threaded_mainloop) {
    if !m.is_null() {
        unsafe { pthread_mutex_unlock((*m).mutex.as_mut_ptr().cast()) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_wait(m: *mut pa_threaded_mainloop) {
    if m.is_null() {
        return;
    }
    let mut ts = Timespec::default();
    unsafe { clock_gettime(CLOCK_REALTIME, &mut ts) };
    ts.tv_nsec += 10_000_000;
    if ts.tv_nsec >= 1_000_000_000 {
        ts.tv_nsec -= 1_000_000_000;
        ts.tv_sec += 1;
    }
    unsafe {
        pthread_cond_timedwait(
            (*m).cond.as_mut_ptr().cast(),
            (*m).mutex.as_mut_ptr().cast(),
            &ts,
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_signal(
    m: *mut pa_threaded_mainloop,
    _wait_for_accept: c_int,
) {
    if !m.is_null() {
        unsafe { pthread_cond_broadcast((*m).cond.as_mut_ptr().cast()) };
    }
}

#[no_mangle]
pub extern "C" fn pa_threaded_mainloop_accept(_m: *mut pa_threaded_mainloop) {}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_in_thread(m: *mut pa_threaded_mainloop) -> c_int {
    if m.is_null() {
        0
    } else {
        unsafe { (*m).in_thread.load(Ordering::Acquire) }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_get_api(
    m: *mut pa_threaded_mainloop,
) -> *mut pa_mainloop_api {
    if m.is_null() {
        ptr::null_mut()
    } else {
        unsafe { &raw mut (*m).api }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_mainloop_api_once(
    a: *mut pa_mainloop_api,
    callback: Option<unsafe extern "C" fn(*mut pa_mainloop_api, *mut c_void)>,
    userdata: *mut c_void,
) {
    if !a.is_null() {
        if let Some(cb) = callback {
            unsafe { cb(a, userdata) };
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_new(
    mainloop: *mut pa_mainloop_api,
    _name: *const c_char,
) -> *mut pa_context {
    let c = unsafe { calloc(1, mem::size_of::<pa_context>()) }.cast::<pa_context>();
    if c.is_null() {
        return ptr::null_mut();
    }
    let c_ref = unsafe { &mut *c };
    c_ref.ref_cnt.store(1, Ordering::Release);
    c_ref.state = PA_CONTEXT_UNCONNECTED;
    if !mainloop.is_null() && unsafe { !(*mainloop).userdata.is_null() } {
        let ml = unsafe { (*mainloop).userdata.cast::<pa_threaded_mainloop>() };
        c_ref.mainloop = ml;
        unsafe { (*ml).ctx = c };
    }
    c
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_new_with_proplist(
    mainloop: *mut pa_mainloop_api,
    name: *const c_char,
    _proplist: *const pa_proplist,
) -> *mut pa_context {
    unsafe { pa_context_new(mainloop, name) }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_ref(c: *mut pa_context) -> *mut pa_context {
    if !c.is_null() {
        unsafe { (*c).ref_cnt.fetch_add(1, Ordering::AcqRel) };
    }
    c
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_unref(c: *mut pa_context) {
    if c.is_null() {
        return;
    }
    if unsafe { (*c).ref_cnt.fetch_sub(1, Ordering::AcqRel) } <= 1 {
        let ml = unsafe { (*c).mainloop };
        if !ml.is_null() && unsafe { (*ml).ctx == c } {
            unsafe { (*ml).ctx = ptr::null_mut() };
        }
        unsafe { free(c.cast()) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_set_state_callback(
    c: *mut pa_context,
    cb: PaContextNotifyCb,
    userdata: *mut c_void,
) {
    if c.is_null() {
        return;
    }
    unsafe {
        (*c).state_cb = cb;
        (*c).state_userdata = userdata;
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_connect(
    c: *mut pa_context,
    server: *const c_char,
    _flags: c_uint,
    _api: *const c_void,
) -> c_int {
    if c.is_null() {
        return -1;
    }
    let srv = if server.is_null() {
        std::borrow::Cow::Borrowed("default")
    } else {
        unsafe { CStr::from_ptr(server) }.to_string_lossy()
    };
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_PA_CTX",
            &srv,
            "libpulse shim pa_context_connect READY",
        );
    });
    let (scb, sud, ml) = {
        let c_ref = unsafe { &mut *c };
        c_ref.state = PA_CONTEXT_READY;
        (c_ref.state_cb, c_ref.state_userdata, c_ref.mainloop)
    };
    unsafe { pa_context_ref(c) };
    if let Some(cb) = scb {
        unsafe { cb(c, sud) };
    }
    if !ml.is_null() {
        unsafe { pthread_cond_broadcast((*ml).cond.as_mut_ptr().cast()) };
    }
    unsafe { pa_context_unref(c) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_disconnect(c: *mut pa_context) {
    if c.is_null() {
        return;
    }
    let c_ref = unsafe { &mut *c };
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_PA_DISC",
            "libpulse-shim",
            "pa_context_disconnect",
        );
    });
    c_ref.state = PA_CONTEXT_TERMINATED;
    if let Some(cb) = c_ref.state_cb {
        unsafe { cb(c, c_ref.state_userdata) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_state(c: *const pa_context) -> c_int {
    if c.is_null() {
        PA_CONTEXT_FAILED
    } else {
        unsafe { (*c).state }
    }
}

#[no_mangle]
pub extern "C" fn pa_context_errno(_c: *const pa_context) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_drain(
    c: *mut pa_context,
    cb: PaContextNotifyCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        unsafe { f(c, userdata) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub extern "C" fn pa_context_rttime_new(
    _c: *const pa_context,
    _usec: u64,
    _cb: *mut c_void,
    _userdata: *mut c_void,
) -> *mut c_void {
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_set_subscribe_callback(
    c: *mut pa_context,
    cb: PaContextSubscribeCb,
    userdata: *mut c_void,
) {
    if c.is_null() {
        return;
    }
    unsafe {
        (*c).sub_cb = cb;
        (*c).sub_userdata = userdata;
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_subscribe(
    c: *mut pa_context,
    _m: c_uint,
    cb: PaContextSuccessCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        unsafe { f(c, 1, userdata) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_server_info(
    c: *mut pa_context,
    cb: PaServerInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        let mut info: pa_server_info = unsafe { mem::zeroed() };
        info.user_name = current_user_cstr();
        info.host_name = c"CachyOS-LASAPI".as_ptr();
        info.server_version = c"16.1.0".as_ptr();
        info.server_name = c"LASAPI-Universal-AudioEngine".as_ptr();
        info.sample_spec.format = PA_SAMPLE_FLOAT32LE;
        info.sample_spec.rate = 48000;
        info.sample_spec.channels = 2;
        info.default_sink_name = c"lasapi_default_sink".as_ptr();
        info.default_source_name = c"lasapi_default_source".as_ptr();
        unsafe {
            pa_channel_map_init_stereo(&mut info.channel_map);
            f(c, &info, userdata);
        }
    }
    if !c.is_null() && unsafe { !(*c).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*c).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

unsafe fn fill_default_sink_info(si: &mut pa_sink_info) {
    *si = unsafe { mem::zeroed() };
    si.name = c"lasapi_default_sink".as_ptr();
    si.index = 0;
    si.description = c"LASAPI Universal Audio Sink".as_ptr();
    si.sample_spec.format = PA_SAMPLE_FLOAT32LE;
    si.sample_spec.rate = 48000;
    si.sample_spec.channels = 2;
    unsafe {
        pa_channel_map_init_stereo(&mut si.channel_map);
        pa_cvolume_set(&mut si.volume, 2, PA_VOLUME_NORM);
    }
    si.n_volume_steps = 65536;
    si.card = 0;
    si.monitor_source = 1;
    si.monitor_source_name = c"lasapi_default_sink.monitor".as_ptr();
    si.state = PA_SINK_RUNNING;
    si.flags = PA_SINK_HARDWARE | PA_SINK_LATENCY;
}

unsafe fn fill_source_info(src: &mut pa_source_info, is_monitor: bool) {
    *src = unsafe { mem::zeroed() };
    if is_monitor {
        src.name = c"lasapi_default_sink.monitor".as_ptr();
        src.index = 1;
        src.description =
            c"Monitor of LASAPI Universal Audio Sink (Desktop Loopback)".as_ptr();
        src.monitor_of_sink = 0;
        src.monitor_of_sink_name = c"lasapi_default_sink".as_ptr();
    } else {
        src.name = c"lasapi_default_source".as_ptr();
        src.index = 0;
        src.description = c"LASAPI Universal Microphone Input (Direct Kernel ALSA)".as_ptr();
        src.monitor_of_sink = PA_INVALID_INDEX;
        src.monitor_of_sink_name = ptr::null();
    }
    src.sample_spec.format = PA_SAMPLE_FLOAT32LE;
    src.sample_spec.rate = 48000;
    src.sample_spec.channels = 2;
    unsafe {
        pa_channel_map_init_stereo(&mut src.channel_map);
        pa_cvolume_set(&mut src.volume, 2, PA_VOLUME_NORM);
    }
    src.n_volume_steps = 65536;
    src.card = 0;
    src.state = PA_SOURCE_RUNNING;
    src.flags = PA_SOURCE_HARDWARE | PA_SOURCE_LATENCY;
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_sink_info_by_name(
    c: *mut pa_context,
    _name: *const c_char,
    cb: PaSinkInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        let mut si: pa_sink_info = unsafe { mem::zeroed() };
        unsafe {
            fill_default_sink_info(&mut si);
            f(c, &si, 0, userdata);
            f(c, ptr::null(), 1, userdata);
        }
    }
    if !c.is_null() && unsafe { !(*c).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*c).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_sink_info_by_index(
    c: *mut pa_context,
    _idx: u32,
    cb: PaSinkInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    unsafe { pa_context_get_sink_info_by_name(c, ptr::null(), cb, userdata) }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_sink_info_list(
    c: *mut pa_context,
    cb: PaSinkInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    unsafe { pa_context_get_sink_info_by_name(c, ptr::null(), cb, userdata) }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_source_info_by_name(
    c: *mut pa_context,
    name: *const c_char,
    cb: PaSourceInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        let mut src: pa_source_info = unsafe { mem::zeroed() };
        let is_mon = if name.is_null() {
            false
        } else {
            let s = unsafe { CStr::from_ptr(name) }.to_string_lossy();
            s.contains("monitor") || s.contains("loopback")
        };
        unsafe {
            fill_source_info(&mut src, is_mon);
            f(c, &src, 0, userdata);
            f(c, ptr::null(), 1, userdata);
        }
    }
    if !c.is_null() && unsafe { !(*c).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*c).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_source_info_by_index(
    c: *mut pa_context,
    idx: u32,
    cb: PaSourceInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        let mut src: pa_source_info = unsafe { mem::zeroed() };
        unsafe {
            fill_source_info(&mut src, idx == 1);
            f(c, &src, 0, userdata);
            f(c, ptr::null(), 1, userdata);
        }
    }
    if !c.is_null() && unsafe { !(*c).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*c).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_source_info_list(
    c: *mut pa_context,
    cb: PaSourceInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        let mut mic_src: pa_source_info = unsafe { mem::zeroed() };
        let mut mon_src: pa_source_info = unsafe { mem::zeroed() };
        unsafe {
            fill_source_info(&mut mic_src, false);
            fill_source_info(&mut mon_src, true);
            f(c, &mic_src, 0, userdata);
            f(c, &mon_src, 0, userdata);
            f(c, ptr::null(), 1, userdata);
        }
    }
    if !c.is_null() && unsafe { !(*c).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*c).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_sink_input_info(
    c: *mut pa_context,
    idx: u32,
    cb: PaSinkInputInfoCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        let mut ii: pa_sink_input_info = unsafe { mem::zeroed() };
        ii.index = idx;
        ii.name = c"Firefox-Stream".as_ptr();
        ii.sample_spec.format = PA_SAMPLE_FLOAT32LE;
        ii.sample_spec.rate = 48000;
        ii.sample_spec.channels = 2;
        ii.has_volume = 1;
        ii.volume_writable = 1;
        unsafe {
            pa_channel_map_init_stereo(&mut ii.channel_map);
            pa_cvolume_set(&mut ii.volume, 2, PA_VOLUME_NORM);
            f(c, &ii, 0, userdata);
            f(c, ptr::null(), 1, userdata);
        }
    }
    if !c.is_null() && unsafe { !(*c).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*c).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_set_sink_input_volume(
    c: *mut pa_context,
    idx: u32,
    volume: *const pa_cvolume,
    cb: PaContextSuccessCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if !c.is_null() && !volume.is_null() && unsafe { (*volume).channels > 0 } {
        let linear = (unsafe { (*volume).values[0] } as f32) / (PA_VOLUME_NORM as f32);
        for i in 0..MAX_PA_STREAMS {
            let s = unsafe { (*c).streams[i] };
            if !s.is_null() && unsafe { !(*s).lasapi.is_null() && (*s).index == idx } {
                let lasapi = unsafe { (*s).lasapi };
                with_lasapi_backend(|b| b.set_master_volume(lasapi, linear));
            }
        }
    }
    if let Some(f) = cb {
        unsafe { f(c, 1, userdata) };
    }
    if !c.is_null() && unsafe { !(*c).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*c).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_new(
    c: *mut pa_context,
    name: *const c_char,
    ss: *const pa_sample_spec,
    map: *const pa_channel_map,
) -> *mut pa_stream {
    if c.is_null() || ss.is_null() {
        return ptr::null_mut();
    }
    let s = unsafe { calloc(1, mem::size_of::<pa_stream>()) }.cast::<pa_stream>();
    if s.is_null() {
        return ptr::null_mut();
    }
    let mut ss_val = unsafe { *ss };
    if ss_val.channels == 0 {
        ss_val.channels = 2;
    } else if usize::from(ss_val.channels) > PA_CHANNELS_MAX {
        ss_val.channels = PA_CHANNELS_MAX as u8;
    }
    let write_scratch_cap = 65536usize;
    let write_scratch = unsafe { calloc(1, write_scratch_cap) };
    let read_scratch_cap = 65536usize;
    let read_scratch = unsafe { calloc(1, read_scratch_cap) };
    if write_scratch.is_null() || read_scratch.is_null() {
        unsafe {
            free(write_scratch);
            free(read_scratch);
            free(s.cast());
        }
        return ptr::null_mut();
    }
    let s_ref = unsafe { &mut *s };
    s_ref.ref_cnt.store(1, Ordering::Release);
    s_ref.ctx = unsafe { pa_context_ref(c) };
    copy_cstr_buf(&mut s_ref.name, name, "AudioStream");
    s_ref.ss = ss_val;
    if !map.is_null() {
        s_ref.cmap = unsafe { *map };
    } else {
        unsafe { pa_channel_map_init_auto(&mut s_ref.cmap, c_uint::from(ss_val.channels), 0) };
    }
    let mut bps = 4u32;
    s_ref.lfmt = map_pa_fmt(ss_val.format, &mut bps);
    let ch = u32::from(ss_val.channels);
    s_ref.frame_bytes = ch * bps;
    s_ref.state = PA_STREAM_UNCONNECTED;
    s_ref.write_scratch_cap = write_scratch_cap;
    s_ref.write_scratch = write_scratch;
    s_ref.read_scratch_cap = read_scratch_cap;
    s_ref.read_scratch = read_scratch;

    for i in 0..MAX_PA_STREAMS {
        if unsafe { (*c).streams[i].is_null() } {
            unsafe { (*c).streams[i] = s };
            s_ref.index = (i + 1) as u32;
            break;
        }
    }
    s
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_new_with_proplist(
    c: *mut pa_context,
    name: *const c_char,
    ss: *const pa_sample_spec,
    map: *const pa_channel_map,
    _p: *mut pa_proplist,
) -> *mut pa_stream {
    unsafe { pa_stream_new(c, name, ss, map) }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_ref(s: *mut pa_stream) -> *mut pa_stream {
    if !s.is_null() {
        unsafe { (*s).ref_cnt.fetch_add(1, Ordering::AcqRel) };
    }
    s
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_unref(s: *mut pa_stream) {
    if s.is_null() {
        return;
    }
    if unsafe { (*s).ref_cnt.fetch_sub(1, Ordering::AcqRel) } <= 1 {
        let s_ref = unsafe { &mut *s };
        let parent_ctx = s_ref.ctx;
        s_ref.ctx = ptr::null_mut();
        if !parent_ctx.is_null() {
            for i in 0..MAX_PA_STREAMS {
                if unsafe { (*parent_ctx).streams[i] == s } {
                    unsafe { (*parent_ctx).streams[i] = ptr::null_mut() };
                }
            }
        }
        if !s_ref.lasapi.is_null() {
            let old = s_ref.lasapi;
            with_lasapi_backend(|b| b.release(old));
            s_ref.lasapi = ClientHandle::NULL;
        }
        unsafe {
            free(s_ref.write_scratch);
            free(s_ref.read_scratch);
            free(s.cast());
            if !parent_ctx.is_null() {
                pa_context_unref(parent_ctx);
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_state(s: *const pa_stream) -> c_int {
    if s.is_null() {
        PA_STREAM_FAILED
    } else {
        unsafe { (*s).state }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_context(s: *const pa_stream) -> *mut pa_context {
    if s.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*s).ctx }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_index(s: *const pa_stream) -> u32 {
    if s.is_null() { 0 } else { unsafe { (*s).index } }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_device_name(s: *const pa_stream) -> *const c_char {
    if !s.is_null() && unsafe { (*s).is_record } {
        c"lasapi_default_source".as_ptr()
    } else {
        c"lasapi_default_sink".as_ptr()
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_state_callback(
    s: *mut pa_stream,
    cb: PaStreamNotifyCb,
    userdata: *mut c_void,
) {
    if !s.is_null() {
        unsafe {
            (*s).state_cb = cb;
            (*s).state_userdata = userdata;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_write_callback(
    s: *mut pa_stream,
    cb: PaStreamRequestCb,
    userdata: *mut c_void,
) {
    if !s.is_null() {
        unsafe {
            (*s).write_cb = cb;
            (*s).write_userdata = userdata;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_read_callback(
    s: *mut pa_stream,
    cb: PaStreamRequestCb,
    userdata: *mut c_void,
) {
    if !s.is_null() {
        unsafe {
            (*s).read_cb = cb;
            (*s).read_userdata = userdata;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_connect_playback(
    s: *mut pa_stream,
    dev: *const c_char,
    attr: *const pa_buffer_attr,
    flags: c_uint,
    volume: *const pa_cvolume,
    _sync_stream: *mut pa_stream,
) -> c_int {
    if s.is_null() {
        return -1;
    }
    let s_ref = unsafe { &mut *s };
    s_ref.is_record = false;
    if !s_ref.lasapi.is_null() {
        let old = s_ref.lasapi;
        with_lasapi_backend(|b| {
            b.stop(old);
            b.release(old);
        });
        s_ref.lasapi = ClientHandle::NULL;
    }

    let sname = unsafe { CStr::from_ptr(s_ref.name.as_ptr().cast()) }
        .to_string_lossy()
        .into_owned();
    match with_lasapi_backend(|b| b.activate_client(Some(&sname))) {
        Ok(h) => s_ref.lasapi = h,
        Err(_) => {
            with_lasapi_backend(|b| {
                b.debug_trace(
                    LASAPI_DBG_SHIM,
                    LASAPI_DBG_ERR,
                    0,
                    "SHIM_PA_ERR",
                    &sname,
                    "ActivateAudioClient failed (playback)",
                );
            });
            s_ref.state = PA_STREAM_FAILED;
            return -1;
        }
    }

    let ch = if s_ref.ss.channels != 0 {
        u16::from(s_ref.ss.channels)
    } else {
        2
    };
    let rate = if s_ref.ss.rate != 0 { s_ref.ss.rate } else { 48000 };
    let wfx = WaveFormatEx {
        w_format_tag: s_ref.lfmt as u16,
        n_channels: ch,
        n_samples_per_sec: rate,
        w_bits_per_sample: if s_ref.lfmt == LASAPI_FMT_S16_LE { 16 } else { 32 },
        n_block_align: s_ref.frame_bytes as u16,
        n_avg_bytes_per_sec: rate * s_ref.frame_bytes,
        cb_size: 0,
    };

    let init_rc = with_lasapi_backend(|b| {
        b.initialize(
            s_ref.lasapi,
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            4096,
            256,
            &wfx,
        )
    });
    if init_rc != S_OK {
        let old = s_ref.lasapi;
        with_lasapi_backend(|b| {
            b.debug_trace(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_ERR,
                0,
                "SHIM_PA_ERR",
                &sname,
                "IAudioClient_Initialize failed (playback)",
            );
            b.release(old);
        });
        s_ref.lasapi = ClientHandle::NULL;
        s_ref.state = PA_STREAM_FAILED;
        return -1;
    }

    if !volume.is_null() && unsafe { (*volume).channels > 0 } {
        let linear = (unsafe { (*volume).values[0] } as f32) / (PA_VOLUME_NORM as f32);
        with_lasapi_backend(|b| b.set_master_volume(s_ref.lasapi, linear));
    }

    s_ref.battr.maxlength = 4096 * s_ref.frame_bytes;
    s_ref.battr.tlength = if !attr.is_null() && unsafe { (*attr).tlength != u32::MAX } {
        unsafe { (*attr).tlength }
    } else {
        1024 * s_ref.frame_bytes
    };
    s_ref.battr.prebuf = 0;
    s_ref.battr.minreq = 256 * s_ref.frame_bytes;
    s_ref.battr.fragsize = 256 * s_ref.frame_bytes;

    s_ref.corked = c_int::from((flags & PA_STREAM_START_CORKED) != 0);
    if s_ref.corked == 0 {
        with_lasapi_backend(|b| b.start(s_ref.lasapi));
    }

    let dev_s = if dev.is_null() {
        "default".to_string()
    } else {
        unsafe { CStr::from_ptr(dev) }.to_string_lossy().into_owned()
    };
    let detail = format!(
        "rate={} ch={} tlen={} corked={} dev={dev_s}",
        wfx.n_samples_per_sec, wfx.n_channels, s_ref.battr.tlength, s_ref.corked
    );
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_PA_PLAY",
            &sname,
            &detail,
        );
    });

    let (scb, sud, pctx) = {
        s_ref.state = PA_STREAM_READY;
        (s_ref.state_cb, s_ref.state_userdata, s_ref.ctx)
    };
    unsafe { pa_stream_ref(s) };
    if let Some(cb) = scb {
        unsafe { cb(s, sud) };
    }
    if !pctx.is_null() && unsafe { !(*pctx).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*pctx).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { pa_stream_unref(s) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_connect_record(
    s: *mut pa_stream,
    dev: *const c_char,
    attr: *const pa_buffer_attr,
    flags: c_uint,
) -> c_int {
    if s.is_null() {
        return -1;
    }
    let s_ref = unsafe { &mut *s };
    s_ref.is_record = true;
    if !s_ref.lasapi.is_null() {
        let old = s_ref.lasapi;
        with_lasapi_backend(|b| {
            b.stop(old);
            b.release(old);
        });
        s_ref.lasapi = ClientHandle::NULL;
    }

    let sname = unsafe { CStr::from_ptr(s_ref.name.as_ptr().cast()) }
        .to_string_lossy()
        .into_owned();
    match with_lasapi_backend(|b| b.activate_client(Some(&sname))) {
        Ok(h) => s_ref.lasapi = h,
        Err(_) => {
            with_lasapi_backend(|b| {
                b.debug_trace(
                    LASAPI_DBG_SHIM,
                    LASAPI_DBG_ERR,
                    0,
                    "SHIM_PA_ERR",
                    &sname,
                    "ActivateAudioClient failed (record)",
                );
            });
            s_ref.state = PA_STREAM_FAILED;
            return -1;
        }
    }

    let ch = if s_ref.ss.channels != 0 {
        u16::from(s_ref.ss.channels)
    } else {
        2
    };
    let rate = if s_ref.ss.rate != 0 { s_ref.ss.rate } else { 48000 };
    let wfx = WaveFormatEx {
        w_format_tag: s_ref.lfmt as u16,
        n_channels: ch,
        n_samples_per_sec: rate,
        w_bits_per_sample: if s_ref.lfmt == LASAPI_FMT_S16_LE { 16 } else { 32 },
        n_block_align: s_ref.frame_bytes as u16,
        n_avg_bytes_per_sec: rate * s_ref.frame_bytes,
        cb_size: 0,
    };

    let dev_s = if dev.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(dev) }.to_string_lossy().into_owned()
    };
    let mut lflags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if dev_s.contains("monitor") || dev_s.contains("loopback") {
        lflags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    } else {
        lflags |= AUDCLNT_STREAMFLAGS_CAPTURE;
    }

    let init_rc = with_lasapi_backend(|b| {
        b.initialize(
            s_ref.lasapi,
            AUDCLNT_SHAREMODE_SHARED,
            lflags,
            4096,
            256,
            &wfx,
        )
    });
    if init_rc != S_OK {
        let old = s_ref.lasapi;
        with_lasapi_backend(|b| {
            b.debug_trace(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_ERR,
                0,
                "SHIM_PA_ERR",
                &sname,
                "IAudioClient_Initialize failed (record)",
            );
            b.release(old);
        });
        s_ref.lasapi = ClientHandle::NULL;
        s_ref.state = PA_STREAM_FAILED;
        return -1;
    }

    s_ref.battr.maxlength = 4096 * s_ref.frame_bytes;
    s_ref.battr.fragsize = if !attr.is_null() && unsafe { (*attr).fragsize != u32::MAX } {
        unsafe { (*attr).fragsize }
    } else {
        256 * s_ref.frame_bytes
    };

    s_ref.corked = c_int::from((flags & PA_STREAM_START_CORKED) != 0);
    if s_ref.corked == 0 {
        with_lasapi_backend(|b| b.start(s_ref.lasapi));
    }

    let dev_disp = if dev_s.is_empty() { "default" } else { &dev_s };
    let detail = format!(
        "rate={} ch={} flags=0x{lflags:x} dev={dev_disp}",
        wfx.n_samples_per_sec, wfx.n_channels
    );
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_PA_REC",
            &sname,
            &detail,
        );
    });

    let (scb, sud, pctx) = {
        s_ref.state = PA_STREAM_READY;
        (s_ref.state_cb, s_ref.state_userdata, s_ref.ctx)
    };
    unsafe { pa_stream_ref(s) };
    if let Some(cb) = scb {
        unsafe { cb(s, sud) };
    }
    if !pctx.is_null() && unsafe { !(*pctx).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*pctx).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { pa_stream_unref(s) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_disconnect(s: *mut pa_stream) -> c_int {
    if s.is_null() {
        return -1;
    }
    let s_ref = unsafe { &mut *s };
    let sname = unsafe { CStr::from_ptr(s_ref.name.as_ptr().cast()) }
        .to_string_lossy()
        .into_owned();
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_PA_CLOSE",
            &sname,
            "pa_stream_disconnect",
        );
        if !s_ref.lasapi.is_null() {
            b.stop(s_ref.lasapi);
            b.release(s_ref.lasapi);
        }
    });
    s_ref.lasapi = ClientHandle::NULL;
    s_ref.state = PA_STREAM_TERMINATED;
    if let Some(cb) = s_ref.state_cb {
        unsafe { cb(s, s_ref.state_userdata) };
    }
    if !s_ref.ctx.is_null() && unsafe { !(*s_ref.ctx).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*s_ref.ctx).mainloop).cond.as_mut_ptr().cast()) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_writable_size(s: *const pa_stream) -> usize {
    if s.is_null() {
        return 0;
    }
    let s_ref = unsafe { &*s };
    if s_ref.lasapi.is_null() || s_ref.is_record {
        return 0;
    }
    let (buf, pad) = with_lasapi_backend(|b| {
        (
            b.get_buffer_size(s_ref.lasapi).unwrap_or(2048),
            b.get_current_padding(s_ref.lasapi).unwrap_or(0),
        )
    });
    (buf.saturating_sub(pad) as usize) * (s_ref.frame_bytes as usize)
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_readable_size(s: *const pa_stream) -> usize {
    if s.is_null() {
        return 0;
    }
    let s_ref = unsafe { &*s };
    if s_ref.lasapi.is_null() || !s_ref.is_record {
        return 0;
    }
    let pkt = with_lasapi_backend(|b| b.get_next_packet_size(s_ref.lasapi)).unwrap_or(0);
    (pkt as usize) * (s_ref.frame_bytes as usize)
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_peek(
    s: *mut pa_stream,
    data: *mut *const c_void,
    nbytes: *mut usize,
) -> c_int {
    if s.is_null() || data.is_null() || nbytes.is_null() {
        return -1;
    }
    let s_ref = unsafe { &mut *s };
    if s_ref.lasapi.is_null() {
        return -1;
    }
    if s_ref.read_scratch_len == 0 {
        let pkt = with_lasapi_backend(|b| b.get_next_packet_size(s_ref.lasapi)).unwrap_or(0);
        if pkt == 0 {
            unsafe {
                *data = ptr::null();
                *nbytes = 0;
            }
            return 0;
        }
        let max_frames = (s_ref.read_scratch_cap / (s_ref.frame_bytes as usize)) as u32;
        let to_read = pkt.min(max_frames);
        let read_bytes = (to_read as usize) * (s_ref.frame_bytes as usize);
        let dst = unsafe {
            core::slice::from_raw_parts_mut(s_ref.read_scratch.cast::<u8>(), read_bytes)
        };
        with_lasapi_backend(|b| b.read_frames(s_ref.lasapi, dst, to_read, s_ref.lfmt));
        s_ref.read_scratch_len = read_bytes;
    }
    unsafe {
        *data = s_ref.read_scratch;
        *nbytes = s_ref.read_scratch_len;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_drop(s: *mut pa_stream) -> c_int {
    if s.is_null() {
        return -1;
    }
    unsafe { (*s).read_scratch_len = 0 };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_begin_write(
    s: *mut pa_stream,
    data: *mut *mut c_void,
    nbytes: *mut usize,
) -> c_int {
    if s.is_null() || data.is_null() || nbytes.is_null() {
        return -1;
    }
    let s_ref = unsafe { &mut *s };
    let n_in = unsafe { *nbytes };
    let req = if n_in == usize::MAX || n_in == 0 {
        1024 * (s_ref.frame_bytes as usize)
    } else {
        n_in
    };
    if req > s_ref.write_scratch_cap {
        let nb = unsafe { realloc(s_ref.write_scratch, req) };
        if nb.is_null() {
            return -1;
        }
        s_ref.write_scratch = nb;
        s_ref.write_scratch_cap = req;
    }
    unsafe {
        *data = s_ref.write_scratch;
        *nbytes = req;
    }
    0
}

#[no_mangle]
pub extern "C" fn pa_stream_cancel_write(_s: *mut pa_stream) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_write(
    s: *mut pa_stream,
    data: *const c_void,
    nbytes: usize,
    free_cb: PaFreeCb,
    _offset: i64,
    _seek: c_int,
) -> c_int {
    if s.is_null() || data.is_null() {
        return -1;
    }
    let s_ref = unsafe { &mut *s };
    if s_ref.lasapi.is_null() || s_ref.frame_bytes == 0 {
        return -1;
    }
    let frames = (nbytes / (s_ref.frame_bytes as usize)) as u32;
    let frame_bytes = (frames as usize) * (s_ref.frame_bytes as usize);
    let src = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), frame_bytes) };
    with_lasapi_backend(|b| b.write_frames(s_ref.lasapi, src, frames, s_ref.lfmt));
    s_ref.bytes_written += nbytes as u64;
    if let Some(f) = free_cb {
        if !ptr::eq(data, s_ref.write_scratch) {
            unsafe { f(data.cast_mut()) };
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_cork(
    s: *mut pa_stream,
    b: c_int,
    cb: PaStreamSuccessCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if !s.is_null() {
        let s_ref = unsafe { &mut *s };
        s_ref.corked = c_int::from(b != 0);
        if !s_ref.lasapi.is_null() {
            if s_ref.corked != 0 {
                with_lasapi_backend(|b| b.stop(s_ref.lasapi));
            } else {
                with_lasapi_backend(|b| b.start(s_ref.lasapi));
            }
        }
    }
    if let Some(f) = cb {
        unsafe { f(s, 1, userdata) };
    }
    if !s.is_null() && unsafe { !(*s).ctx.is_null() && !(*(*s).ctx).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*(*s).ctx).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_is_corked(s: *const pa_stream) -> c_int {
    if s.is_null() { 0 } else { unsafe { (*s).corked } }
}

#[no_mangle]
pub extern "C" fn pa_stream_is_suspended(_s: *const pa_stream) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_buffer_attr(s: *mut pa_stream) -> *const pa_buffer_attr {
    if s.is_null() {
        ptr::null()
    } else {
        unsafe { &raw const (*s).battr }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_sample_spec(s: *mut pa_stream) -> *const pa_sample_spec {
    if s.is_null() {
        ptr::null()
    } else {
        unsafe { &raw const (*s).ss }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_channel_map(s: *mut pa_stream) -> *const pa_channel_map {
    if s.is_null() {
        ptr::null()
    } else {
        unsafe { &raw const (*s).cmap }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_name(
    s: *mut pa_stream,
    name: *const c_char,
    cb: PaStreamSuccessCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if !s.is_null() && !name.is_null() {
        unsafe { copy_cstr_buf(&mut (*s).name, name, "AudioStream") };
    }
    if let Some(f) = cb {
        unsafe { f(s, 1, userdata) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_update_timing_info(
    s: *mut pa_stream,
    cb: PaStreamSuccessCb,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if let Some(f) = cb {
        unsafe { f(s, 1, userdata) };
    }
    if !s.is_null() && unsafe { !(*s).ctx.is_null() && !(*(*s).ctx).mainloop.is_null() } {
        unsafe { pthread_cond_broadcast((*(*(*s).ctx).mainloop).cond.as_mut_ptr().cast()) };
    }
    unsafe { alloc_done_op() }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_time(s: *mut pa_stream, r_usec: *mut u64) -> c_int {
    if s.is_null() || r_usec.is_null() {
        return -1;
    }
    let s_ref = unsafe { &*s };
    if s_ref.frame_bytes == 0 || s_ref.ss.rate == 0 {
        return -1;
    }
    let pad = if !s_ref.lasapi.is_null() {
        with_lasapi_backend(|b| b.get_current_padding(s_ref.lasapi)).unwrap_or(0)
    } else {
        0
    };
    let total_frames = s_ref.bytes_written / u64::from(s_ref.frame_bytes);
    let played_frames = total_frames.saturating_sub(u64::from(pad));
    unsafe { *r_usec = (played_frames * 1_000_000) / u64::from(s_ref.ss.rate) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_latency(
    s: *mut pa_stream,
    r_usec: *mut u64,
    negative: *mut c_int,
) -> c_int {
    if !negative.is_null() {
        unsafe { *negative = 0 };
    }
    if s.is_null() || r_usec.is_null() {
        return -1;
    }
    let s_ref = unsafe { &*s };
    if s_ref.ss.rate == 0 {
        return -1;
    }
    let pad = if !s_ref.lasapi.is_null() {
        with_lasapi_backend(|b| b.get_current_padding(s_ref.lasapi)).unwrap_or(0)
    } else {
        0
    };
    unsafe { *r_usec = (u64::from(pad) * 1_000_000) / u64::from(s_ref.ss.rate) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_ref(o: *mut pa_operation) -> *mut pa_operation {
    if !o.is_null() {
        unsafe { (*o).ref_cnt.fetch_add(1, Ordering::AcqRel) };
    }
    o
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_unref(o: *mut pa_operation) {
    if o.is_null() {
        return;
    }
    if unsafe { (*o).ref_cnt.fetch_sub(1, Ordering::AcqRel) } <= 1 {
        unsafe { free(o.cast()) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_cancel(o: *mut pa_operation) {
    if !o.is_null() {
        unsafe { (*o).state = PA_OPERATION_CANCELLED };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_get_state(o: *const pa_operation) -> c_int {
    if o.is_null() {
        PA_OPERATION_DONE
    } else {
        unsafe { (*o).state }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_set_state_callback(
    o: *mut pa_operation,
    cb: PaOperationNotifyCb,
    userdata: *mut c_void,
) {
    if !o.is_null() {
        unsafe {
            (*o).cb = cb;
            (*o).userdata = userdata;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_proplist_new() -> *mut pa_proplist {
    unsafe { calloc(1, mem::size_of::<pa_proplist>()) }.cast()
}

#[no_mangle]
pub unsafe extern "C" fn pa_proplist_free(p: *mut pa_proplist) {
    unsafe { free(p.cast()) };
}

#[no_mangle]
pub unsafe extern "C" fn pa_proplist_sets(
    p: *mut pa_proplist,
    _key: *const c_char,
    value: *const c_char,
) -> c_int {
    if !p.is_null() && !value.is_null() {
        unsafe { copy_cstr_buf(&mut (*p).name, value, "") };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_proplist_gets(
    p: *const pa_proplist,
    _key: *const c_char,
) -> *const c_char {
    if p.is_null() {
        ptr::null()
    } else {
        unsafe { (*p).name.as_ptr().cast() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_channel_map_init(m: *mut pa_channel_map) -> *mut pa_channel_map {
    if !m.is_null() {
        unsafe { ptr::write_bytes(m, 0, 1) };
    }
    m
}

#[no_mangle]
pub unsafe extern "C" fn pa_channel_map_init_stereo(
    m: *mut pa_channel_map,
) -> *mut pa_channel_map {
    if m.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        (*m).channels = 2;
        (*m).map[0] = PA_CHANNEL_POSITION_FRONT_LEFT;
        (*m).map[1] = PA_CHANNEL_POSITION_FRONT_RIGHT;
    }
    m
}

#[no_mangle]
pub unsafe extern "C" fn pa_channel_map_init_auto(
    m: *mut pa_channel_map,
    channels: c_uint,
    _def: c_int,
) -> *mut pa_channel_map {
    if m.is_null() {
        return ptr::null_mut();
    }
    let ch = if channels != 0 {
        channels.min(PA_CHANNELS_MAX as c_uint) as u8
    } else {
        2
    };
    unsafe {
        (*m).channels = ch;
        for i in 0..usize::from(ch) {
            (*m).map[i] = if i == 0 {
                PA_CHANNEL_POSITION_FRONT_LEFT
            } else {
                PA_CHANNEL_POSITION_FRONT_RIGHT
            };
        }
    }
    m
}

#[no_mangle]
pub extern "C" fn pa_channel_map_can_balance(_map: *const pa_channel_map) -> c_int {
    1
}

#[no_mangle]
pub unsafe extern "C" fn pa_cvolume_set(
    a: *mut pa_cvolume,
    channels: c_uint,
    v: u32,
) -> *mut pa_cvolume {
    if a.is_null() {
        return ptr::null_mut();
    }
    let ch = channels.min(PA_CHANNELS_MAX as c_uint) as u8;
    unsafe {
        (*a).channels = ch;
        for i in 0..usize::from(ch) {
            (*a).values[i] = v;
        }
    }
    a
}

#[no_mangle]
pub extern "C" fn pa_cvolume_set_balance(
    v: *mut pa_cvolume,
    _map: *const pa_channel_map,
    _new_balance: f32,
) -> *mut pa_cvolume {
    v
}

#[no_mangle]
pub extern "C" fn pa_sw_volume_from_linear(v: f64) -> u32 {
    if v <= 0.0 {
        PA_VOLUME_MUTED
    } else {
        (v * f64::from(PA_VOLUME_NORM)) as u32
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_frame_size(spec: *const pa_sample_spec) -> usize {
    if spec.is_null() {
        return 8;
    }
    let mut bps = 4u32;
    let s = unsafe { &*spec };
    let _ = map_pa_fmt(s.format, &mut bps);
    let ch = usize::from(s.channels).min(PA_CHANNELS_MAX);
    ch * (bps as usize)
}

#[no_mangle]
pub unsafe extern "C" fn pa_sample_size(spec: *const pa_sample_spec) -> usize {
    if spec.is_null() {
        return 4;
    }
    let mut bps = 4u32;
    let _ = map_pa_fmt(unsafe { (*spec).format }, &mut bps);
    bps as usize
}

#[no_mangle]
pub unsafe extern "C" fn pa_usec_to_bytes(t: u64, spec: *const pa_sample_spec) -> usize {
    if spec.is_null() {
        return 0;
    }
    let rate = u128::from(unsafe { (*spec).rate });
    let frame_sz = u128::from(unsafe { pa_frame_size(spec) } as u64);
    ((u128::from(t).saturating_mul(rate) / 1_000_000).saturating_mul(frame_sz))
        .min(usize::MAX as u128) as usize
}

#[no_mangle]
pub extern "C" fn pa_rtclock_now() -> u64 {
    let mut ts = Timespec::default();
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64)
        .saturating_mul(1_000_000)
        .saturating_add((ts.tv_nsec as u64) / 1000)
}

#[no_mangle]
pub extern "C" fn pa_get_library_version() -> *const c_char {
    c"16.1.0".as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn pa_xrealloc(ptr_in: *mut c_void, size: usize) -> *mut c_void {
    unsafe { realloc(ptr_in, if size != 0 { size } else { 1 }) }
}

#[no_mangle]
pub unsafe extern "C" fn pa_simple_new(
    _server: *const c_char,
    name: *const c_char,
    dir: c_int,
    dev: *const c_char,
    stream_name: *const c_char,
    ss: *const pa_sample_spec,
    _map: *const pa_channel_map,
    attr: *const pa_buffer_attr,
    error: *mut c_int,
) -> *mut pa_simple {
    if ss.is_null() {
        if !error.is_null() {
            unsafe { *error = -1 };
        }
        return ptr::null_mut();
    }
    let s = unsafe { calloc(1, mem::size_of::<pa_simple>()) }.cast::<pa_simple>();
    if s.is_null() {
        if !error.is_null() {
            unsafe { *error = -2 };
        }
        return ptr::null_mut();
    }
    let s_ref = unsafe { &mut *s };
    let ss_val = unsafe { *ss };
    let mut bps = 4u32;
    s_ref.fmt = map_pa_fmt(ss_val.format, &mut bps);
    s_ref.rate = if ss_val.rate != 0 { ss_val.rate } else { 48000 };
    s_ref.channels = if ss_val.channels != 0 {
        u32::from(ss_val.channels).min(PA_CHANNELS_MAX as u32)
    } else {
        2
    };
    s_ref.frame_bytes = s_ref.channels * bps;
    s_ref.is_record = dir == PA_STREAM_RECORD;

    let label_str = if !stream_name.is_null() && unsafe { *stream_name != 0 } {
        unsafe { CStr::from_ptr(stream_name) }.to_string_lossy().into_owned()
    } else if !name.is_null() {
        unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
    } else {
        "stream".to_string()
    };
    match with_lasapi_backend(|b| b.activate_client(Some(&label_str))) {
        Ok(h) => s_ref.client = h,
        Err(_) => {
            unsafe { free(s.cast()) };
            if !error.is_null() {
                unsafe { *error = -3 };
            }
            return ptr::null_mut();
        }
    }

    let wfx = WaveFormatEx {
        w_format_tag: s_ref.fmt as u16,
        n_channels: s_ref.channels as u16,
        n_samples_per_sec: s_ref.rate,
        w_bits_per_sample: (bps * 8) as u16,
        n_block_align: s_ref.frame_bytes as u16,
        n_avg_bytes_per_sec: s_ref.rate * s_ref.frame_bytes,
        cb_size: 0,
    };

    let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if s_ref.is_record {
        let dev_s = if dev.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(dev) }.to_string_lossy().into_owned()
        };
        if dev_s.contains("monitor") || dev_s.contains("loopback") {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        } else {
            flags |= AUDCLNT_STREAMFLAGS_CAPTURE;
        }
    }

    let mut buf_frames = 2048u32;
    let mut period_frames = 256u32;
    if !attr.is_null() && s_ref.frame_bytes > 0 {
        let a = unsafe { &*attr };
        if a.maxlength != u32::MAX && a.maxlength >= s_ref.frame_bytes {
            buf_frames = a.maxlength / s_ref.frame_bytes;
        } else if a.tlength != u32::MAX && a.tlength >= s_ref.frame_bytes {
            buf_frames = a.tlength / s_ref.frame_bytes;
        }
        buf_frames = buf_frames.clamp(512, LASAPI_RING_FRAMES as u32);

        if !s_ref.is_record && a.minreq != u32::MAX && a.minreq >= s_ref.frame_bytes {
            period_frames = a.minreq / s_ref.frame_bytes;
        } else if s_ref.is_record && a.fragsize != u32::MAX && a.fragsize >= s_ref.frame_bytes {
            period_frames = a.fragsize / s_ref.frame_bytes;
        }
        period_frames = period_frames.clamp(64, buf_frames / 2);
    }

    let init_rc = with_lasapi_backend(|b| {
        b.initialize(
            s_ref.client,
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            buf_frames,
            period_frames,
            &wfx,
        )
    });
    if init_rc != S_OK {
        with_lasapi_backend(|b| b.release(s_ref.client));
        unsafe {
            free(s.cast());
        }
        if !error.is_null() {
            unsafe { *error = -4 };
        }
        return ptr::null_mut();
    }

    with_lasapi_backend(|b| b.start(s_ref.client));
    if !error.is_null() {
        unsafe { *error = 0 };
    }
    s
}

#[no_mangle]
pub unsafe extern "C" fn pa_simple_write(
    s: *mut pa_simple,
    data: *const c_void,
    bytes: usize,
    error: *mut c_int,
) -> c_int {
    if s.is_null() || data.is_null() || unsafe { (*s).frame_bytes == 0 } {
        if !error.is_null() {
            unsafe { *error = -1 };
        }
        return -1;
    }
    let s_ref = unsafe { &*s };
    let frames = (bytes / (s_ref.frame_bytes as usize)) as u32;
    let frame_bytes = (frames as usize) * (s_ref.frame_bytes as usize);
    let src = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), frame_bytes) };
    if with_lasapi_backend(|b| b.write_frames(s_ref.client, src, frames, s_ref.fmt)) != S_OK {
        if !error.is_null() {
            unsafe { *error = -5 };
        }
        return -1;
    }
    if !error.is_null() {
        unsafe { *error = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_simple_read(
    s: *mut pa_simple,
    data: *mut c_void,
    bytes: usize,
    error: *mut c_int,
) -> c_int {
    if s.is_null() || data.is_null() || unsafe { (*s).frame_bytes == 0 } {
        if !error.is_null() {
            unsafe { *error = -1 };
        }
        return -1;
    }
    let s_ref = unsafe { &*s };
    let frames = (bytes / (s_ref.frame_bytes as usize)) as u32;
    let frame_bytes = (frames as usize) * (s_ref.frame_bytes as usize);
    let dst = unsafe { core::slice::from_raw_parts_mut(data.cast::<u8>(), frame_bytes) };
    let hr = with_lasapi_backend(|b| {
        let pkt = b.get_next_packet_size(s_ref.client).unwrap_or(0);
        if pkt < frames {
            b.wait_for_period(s_ref.client, 15);
        }
        b.read_frames(s_ref.client, dst, frames, s_ref.fmt)
    });
    if hr != S_OK {
        if !error.is_null() {
            unsafe { *error = -5 };
        }
        return -1;
    }
    if !error.is_null() {
        unsafe { *error = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_simple_drain(s: *mut pa_simple, error: *mut c_int) -> c_int {
    if s.is_null() {
        return -1;
    }
    let s_ref = unsafe { &*s };
    if !s_ref.is_record {
        for _ in 0..50 {
            let pad = with_lasapi_backend(|b| b.get_current_padding(s_ref.client)).unwrap_or(0);
            if pad == 0 {
                break;
            }
            with_lasapi_backend(|b| b.wait_for_period(s_ref.client, 10));
        }
    }
    if !error.is_null() {
        unsafe { *error = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_simple_flush(s: *mut pa_simple, error: *mut c_int) -> c_int {
    if s.is_null() {
        return -1;
    }
    let client = unsafe { (*s).client };
    with_lasapi_backend(|b| b.reset(client));
    if !error.is_null() {
        unsafe { *error = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_simple_get_latency(s: *mut pa_simple, error: *mut c_int) -> u64 {
    if !error.is_null() {
        unsafe { *error = 0 };
    }
    if s.is_null() {
        return 0;
    }
    let client = unsafe { (*s).client };
    let pad = with_lasapi_backend(|b| b.get_current_padding(client)).unwrap_or(0);
    (u64::from(pad) * 1_000_000) / u64::from(unsafe { (*s).rate })
}

#[no_mangle]
pub unsafe extern "C" fn pa_simple_free(s: *mut pa_simple) {
    if s.is_null() {
        return;
    }
    let client = unsafe { (*s).client };
    if !client.is_null() {
        with_lasapi_backend(|b| b.release(client));
    }
    unsafe {
        free(s.cast());
    }
}

#[no_mangle]
pub extern "C" fn pa_strerror(_error: c_int) -> *const c_char {
    c"LASAPI PulseAudio Shim Status".as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn pa_glib_mainloop_new(_glib_context: *mut c_void) -> *mut c_void {
    let m = unsafe { pa_threaded_mainloop_new() };
    if !m.is_null() {
        unsafe { pa_threaded_mainloop_start(m) };
    }
    m.cast()
}

#[no_mangle]
pub unsafe extern "C" fn pa_glib_mainloop_free(g: *mut c_void) {
    if !g.is_null() {
        unsafe { pa_threaded_mainloop_free(g.cast()) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_glib_mainloop_get_api(g: *mut c_void) -> *mut pa_mainloop_api {
    if g.is_null() {
        ptr::null_mut()
    } else {
        unsafe { pa_threaded_mainloop_get_api(g.cast()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use salsa_wire::client_seam::InMemoryLasapiBackend;
    use std::sync::Arc;

    #[test]
    fn pa_simple_and_stream_lifecycle_against_in_memory_backend() {
        let mem = Arc::new(InMemoryLasapiBackend::new(48_000, 2, 256));
        with_test_lasapi_backend(Arc::clone(&mem) as _, || unsafe {
            let ss = pa_sample_spec {
                format: PA_SAMPLE_FLOAT32LE,
                rate: 48_000,
                channels: 2,
            };
            let mut err = -1;
            let pb = pa_simple_new(
                ptr::null(),
                c"SimpleApp".as_ptr(),
                1,
                ptr::null(),
                c"music".as_ptr(),
                &raw const ss,
                ptr::null(),
                ptr::null(),
                &raw mut err,
            );
            assert!(!pb.is_null());
            assert_eq!(err, 0);
            let h = (*pb).client;

            let pcm = [0.25f32; 256 * 2];
            assert_eq!(
                pa_simple_write(pb, pcm.as_ptr().cast(), core::mem::size_of_val(&pcm), &raw mut err),
                0
            );
            assert_eq!(pa_simple_get_latency(pb, &raw mut err), 256 * 1_000_000 / 48_000);
            assert_eq!(pa_simple_drain(pb, &raw mut err), 0);
            assert_eq!(pa_simple_get_latency(pb, &raw mut err), 0);
            pa_simple_free(pb);
            let snap = mem.snapshot_client(h).unwrap();
            assert_eq!(snap.written_frames, 256);
            assert_eq!(snap.release_count, 1);

            let rec = pa_simple_new(
                ptr::null(),
                c"RecApp".as_ptr(),
                PA_STREAM_RECORD,
                ptr::null(),
                c"mic".as_ptr(),
                &raw const ss,
                ptr::null(),
                ptr::null(),
                &raw mut err,
            );
            assert!(!rec.is_null());
            let rh = (*rec).client;
            mem.push_capture_bytes(rh, &[0x6Du8; 64 * 8]);
            let mut rx = vec![0u8; 64 * 8];
            assert_eq!(pa_simple_read(rec, rx.as_mut_ptr().cast(), rx.len(), &raw mut err), 0);
            assert!(rx.iter().all(|&b| b == 0x6D));
            assert_eq!(pa_simple_flush(rec, &raw mut err), 0);
            pa_simple_free(rec);

            let ml = pa_threaded_mainloop_new();
            let ctx = pa_context_new(pa_threaded_mainloop_get_api(ml), c"AsyncApp".as_ptr());
            let mut cmap = pa_channel_map::default();
            pa_channel_map_init_stereo(&raw mut cmap);
            let s = pa_stream_new(ctx, c"voice".as_ptr(), &raw const ss, &raw const cmap);
            assert!(!s.is_null());
            let mut vol = pa_cvolume::default();
            pa_cvolume_set(&raw mut vol, 2, PA_VOLUME_NORM / 2);
            assert_eq!(
                pa_stream_connect_playback(s, ptr::null(), ptr::null(), 0, &raw const vol, ptr::null_mut()),
                0
            );
            let sh = (*s).lasapi;
            assert!(!sh.is_null());
            assert_eq!(pa_stream_write(s, pcm.as_ptr().cast(), core::mem::size_of_val(&pcm), None, 0, 0), 0);
            let mut lat = 0u64;
            let mut neg = -1;
            assert_eq!(pa_stream_get_latency(s, &raw mut lat, &raw mut neg), 0);
            assert_eq!(neg, 0);
            assert!(lat > 0);
            let op = pa_stream_cork(s, 1, None, ptr::null_mut());
            pa_operation_unref(op);
            assert!(!mem.snapshot_client(sh).unwrap().started);
            assert_eq!(pa_stream_disconnect(s), 0);
            pa_stream_unref(s);
            pa_context_unref(ctx);
            pa_threaded_mainloop_free(ml);
        });
    }
}
