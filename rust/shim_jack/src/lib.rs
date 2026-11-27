// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_ulong, c_void, CStr};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicI32, Ordering};

use salsa_wire::c_abi::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, LASAPI_DBG_ERR,
    LASAPI_DBG_INFO, LASAPI_DBG_SHIM, LASAPI_FMT_FLOAT32, LASAPI_MAX_CHANNELS,
    LasapiMidiEvent, WaveFormatEx, S_OK,
};
use salsa_wire::client_seam::{AudioSessionBackend, ClientHandle};

salsa_wire::define_lasapi_ffi_backend!();

const JACK_MAX_PORT_MIDI_EVENTS: usize = 128;
const JACK_MIDI_MAGIC: u32 = 0x4A4D_4944;
const CLOCK_MONOTONIC: c_int = 1;

type JackNframes = u32;
type JackProcessCallback = Option<unsafe extern "C" fn(JackNframes, *mut c_void) -> c_int>;

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
pub struct jack_midi_event_t {
    time: JackNframes,
    size: usize,
    buffer: *mut u8,
}

#[repr(C)]
struct jack_midi_buf_t {
    magic: u32,
    lasapi_client: ClientHandle,
    count: u32,
    events: [LasapiMidiEvent; JACK_MAX_PORT_MIDI_EVENTS],
}

#[repr(C)]
pub struct jack_port_t {
    name: [u8; 64],
    full_name: [u8; 128],
    is_midi: bool,
    is_input: bool,
    flags: c_ulong,
    audio_buffer: *mut f32,
    midi_buffer: jack_midi_buf_t,
}

#[repr(C)]
pub struct jack_client_t {
    name: [u8; 64],
    lasapi_client: ClientHandle,
    sample_rate: u32,
    period_frames: u32,
    process_cb: JackProcessCallback,
    process_arg: *mut c_void,
    ports: [jack_port_t; LASAPI_MAX_CHANNELS * 2],
    num_ports: AtomicI32,
    active: AtomicI32,
    transport_rolling: AtomicI32,
    frame_counter: u64,
    rt_thread: c_ulong,
}

extern "C" {
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn clock_gettime(clk_id: c_int, tp: *mut Timespec) -> c_int;
    fn pthread_create(
        thread: *mut c_ulong,
        attr: *const c_void,
        start_routine: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        arg: *mut c_void,
    ) -> c_int;
    fn pthread_join(thread: c_ulong, retval: *mut *mut c_void) -> c_int;
}

fn copy_cstr_buf(dst: &mut [u8], src: &str) {
    dst.fill(0);
    if dst.is_empty() {
        return;
    }
    let b = src.as_bytes();
    let n = b.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&b[..n]);
}

fn cbuf_to_str(buf: &[u8]) -> &str {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    core::str::from_utf8(&buf[..end]).unwrap_or("")
}

unsafe fn run_one_jack_cycle(
    jc: &mut jack_client_t,
    interleaved: &mut [f32],
    cap_interleaved: &mut [f32],
    backend: &dyn AudioSessionBackend,
) {
    let nframes = jc.period_frames;
    let pad = backend.get_current_padding(jc.lasapi_client).unwrap_or(0);
    if pad > nframes * 2 {
        backend.wait_for_period(jc.lasapi_client, 15);
        return;
    }

    let frame_bytes = (nframes as usize) * 2 * mem::size_of::<f32>();
    let cap_bytes = unsafe {
        core::slice::from_raw_parts_mut(cap_interleaved.as_mut_ptr().cast::<u8>(), frame_bytes)
    };
    backend.read_frames(jc.lasapi_client, cap_bytes, nframes, LASAPI_FMT_FLOAT32);

    let mut incoming_midi = [LasapiMidiEvent::default(); JACK_MAX_PORT_MIDI_EVENTS];
    let midi_count = backend.read_midi_events(jc.lasapi_client, &mut incoming_midi);

    let nports = jc.num_ports.load(Ordering::Acquire) as usize;
    let mut in_ch = 0usize;
    for port in jc.ports.iter_mut().take(nports) {
        if port.is_midi {
            if port.is_input {
                let cnt = (midi_count as usize).min(JACK_MAX_PORT_MIDI_EVENTS);
                port.midi_buffer.count = cnt as u32;
                port.midi_buffer.events[..cnt].copy_from_slice(&incoming_midi[..cnt]);
            } else {
                port.midi_buffer.count = 0;
            }
        } else if !port.audio_buffer.is_null() {
            if port.is_input {
                let ch = in_ch & 1;
                for f in 0..nframes as usize {
                    unsafe {
                        *port.audio_buffer.add(f) = cap_interleaved[f * 2 + ch];
                    }
                }
                in_ch += 1;
            } else {
                unsafe { ptr::write_bytes(port.audio_buffer, 0, nframes as usize) };
            }
        }
    }

    if let Some(cb) = jc.process_cb {
        unsafe { cb(nframes, jc.process_arg) };
    }

    let mut left: *const f32 = ptr::null();
    let mut right: *const f32 = ptr::null();
    for port in jc.ports.iter().take(nports) {
        if !port.is_midi && !port.is_input && !port.audio_buffer.is_null() {
            if left.is_null() {
                left = port.audio_buffer;
            } else if right.is_null() {
                right = port.audio_buffer;
                break;
            }
        }
    }
    if right.is_null() {
        right = left;
    }

    for f in 0..nframes as usize {
        unsafe {
            interleaved[f * 2] = if left.is_null() { 0.0 } else { *left.add(f) };
            interleaved[f * 2 + 1] = if right.is_null() { 0.0 } else { *right.add(f) };
        }
    }

    let out_bytes = unsafe {
        core::slice::from_raw_parts(interleaved.as_ptr().cast::<u8>(), frame_bytes)
    };
    backend.write_frames(jc.lasapi_client, out_bytes, nframes, LASAPI_FMT_FLOAT32);
    jc.frame_counter += u64::from(nframes);
}

unsafe extern "C" fn jack_wavert_callback_thread(arg: *mut c_void) -> *mut c_void {
    let jc = unsafe { &mut *arg.cast::<jack_client_t>() };
    let nframes = jc.period_frames.max(1) as usize;
    let mut interleaved = vec![0.0f32; nframes * 2];
    let mut cap_interleaved = vec![0.0f32; nframes * 2];

    while jc.active.load(Ordering::Acquire) != 0 {
        with_lasapi_backend(|b| unsafe {
            run_one_jack_cycle(jc, &mut interleaved, &mut cap_interleaved, b);
        });
    }

    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn jack_client_open(
    client_name: *const c_char,
    _options: c_int,
    status: *mut c_int,
) -> *mut jack_client_t {
    let jc = unsafe { calloc(1, mem::size_of::<jack_client_t>()) }.cast::<jack_client_t>();
    if jc.is_null() {
        return ptr::null_mut();
    }
    let j = unsafe { &mut *jc };
    let cname = if client_name.is_null() {
        "daw"
    } else {
        unsafe { CStr::from_ptr(client_name) }.to_str().unwrap_or("daw")
    };
    copy_cstr_buf(&mut j.name, &format!("JACK({cname})"));

    let jname = cbuf_to_str(&j.name).to_string();
    match with_lasapi_backend(|b| b.activate_client(Some(&jname))) {
        Ok(h) => j.lasapi_client = h,
        Err(_) => {
            with_lasapi_backend(|b| {
                b.debug_trace(
                    LASAPI_DBG_SHIM,
                    LASAPI_DBG_ERR,
                    0,
                    "SHIM_JACK_ERR",
                    &jname,
                    "LASAPI_ActivateAudioClient failed",
                );
            });
            unsafe { free(jc.cast()) };
            return ptr::null_mut();
        }
    }

    let wfx = with_lasapi_backend(|b| b.get_mix_format(j.lasapi_client))
        .unwrap_or_else(|_| WaveFormatEx::default());
    let init_rc = with_lasapi_backend(|b| {
        b.initialize(
            j.lasapi_client,
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            1024,
            256,
            &wfx,
        )
    });
    if init_rc != S_OK {
        with_lasapi_backend(|b| {
            b.debug_trace(
                LASAPI_DBG_SHIM,
                LASAPI_DBG_ERR,
                0,
                "SHIM_JACK_ERR",
                &jname,
                "IAudioClient_Initialize failed",
            );
            b.release(j.lasapi_client);
        });
        unsafe { free(jc.cast()) };
        return ptr::null_mut();
    }

    let (def_per, _) =
        with_lasapi_backend(|b| b.get_device_period(j.lasapi_client)).unwrap_or((256, 256));
    j.sample_rate = wfx.n_samples_per_sec;
    j.period_frames = def_per;
    j.transport_rolling.store(1, Ordering::Release);

    let detail = format!("rate={} period={}", j.sample_rate, j.period_frames);
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_JACK_OPEN",
            &jname,
            &detail,
        );
    });
    if !status.is_null() {
        unsafe { *status = 0 };
    }
    jc
}

#[no_mangle]
pub unsafe extern "C" fn jack_client_close(jc: *mut jack_client_t) -> c_int {
    if jc.is_null() {
        return -1;
    }
    let j = unsafe { &mut *jc };
    let jname = cbuf_to_str(&j.name).to_string();
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_JACK_CLOSE",
            &jname,
            "jack_client_close",
        );
    });
    if j.active.swap(0, Ordering::AcqRel) != 0 {
        unsafe { pthread_join(j.rt_thread, ptr::null_mut()) };
    }
    for port in j.ports.iter().take(j.num_ports.load(Ordering::Acquire) as usize) {
        unsafe { free(port.audio_buffer.cast()) };
    }
    if !j.lasapi_client.is_null() {
        with_lasapi_backend(|b| b.release(j.lasapi_client));
    }
    unsafe { free(jc.cast()) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_client_name(jc: *mut jack_client_t) -> *mut c_char {
    if jc.is_null() {
        c"LASAPI-JACK".as_ptr().cast_mut()
    } else {
        unsafe { (*jc).name.as_mut_ptr().cast() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_process_callback(
    jc: *mut jack_client_t,
    cb: JackProcessCallback,
    arg: *mut c_void,
) -> c_int {
    if jc.is_null() {
        return -1;
    }
    unsafe {
        (*jc).process_cb = cb;
        (*jc).process_arg = arg;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_sample_rate_callback(
    jc: *mut jack_client_t,
    cb: Option<unsafe extern "C" fn(JackNframes, *mut c_void) -> c_int>,
    arg: *mut c_void,
) -> c_int {
    if !jc.is_null() {
        if let Some(f) = cb {
            unsafe { f((*jc).sample_rate, arg) };
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_set_buffer_size_callback(
    jc: *mut jack_client_t,
    cb: Option<unsafe extern "C" fn(JackNframes, *mut c_void) -> c_int>,
    arg: *mut c_void,
) -> c_int {
    if !jc.is_null() {
        if let Some(f) = cb {
            unsafe { f((*jc).period_frames, arg) };
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn jack_set_xrun_callback(
    _jc: *mut jack_client_t,
    _cb: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    _arg: *mut c_void,
) -> c_int {
    0
}

#[no_mangle]
pub extern "C" fn jack_on_shutdown(
    _jc: *mut jack_client_t,
    _cb: Option<unsafe extern "C" fn(*mut c_void)>,
    _arg: *mut c_void,
) {
}

#[no_mangle]
pub extern "C" fn jack_on_info_shutdown(
    _jc: *mut jack_client_t,
    _cb: Option<unsafe extern "C" fn(c_int, *const c_char, *mut c_void)>,
    _arg: *mut c_void,
) {
}

#[no_mangle]
pub extern "C" fn jack_set_error_function(_func: Option<unsafe extern "C" fn(*const c_char)>) {}

#[no_mangle]
pub extern "C" fn jack_set_info_function(_func: Option<unsafe extern "C" fn(*const c_char)>) {}

#[no_mangle]
pub unsafe extern "C" fn jack_port_register(
    jc: *mut jack_client_t,
    port_name: *const c_char,
    port_type: *const c_char,
    flags: c_ulong,
    _buffer_size: c_ulong,
) -> *mut jack_port_t {
    if jc.is_null() {
        return ptr::null_mut();
    }
    let j = unsafe { &mut *jc };
    let idx = j.num_ports.load(Ordering::Acquire) as usize;
    if idx >= (LASAPI_MAX_CHANNELS * 2) {
        return ptr::null_mut();
    }
    let cname = cbuf_to_str(&j.name).to_string();
    let per_frames = j.period_frames as usize;
    let client_ptr = j.lasapi_client;
    let p_ptr = {
        let p = &mut j.ports[idx];
        unsafe { ptr::write_bytes(p, 0, 1) };

        let pname = if port_name.is_null() {
            "port"
        } else {
            unsafe { CStr::from_ptr(port_name) }.to_str().unwrap_or("port")
        };
        copy_cstr_buf(&mut p.name, pname);
        copy_cstr_buf(&mut p.full_name, &format!("{cname}:{pname}"));
        p.flags = flags;
        p.is_input = (flags & 0x1) != 0 && (flags & 0x2) == 0;

        let pt = if port_type.is_null() {
            ""
        } else {
            unsafe { CStr::from_ptr(port_type) }.to_str().unwrap_or("")
        };
        if pt.contains("midi") || pt.contains("MIDI") {
            p.is_midi = true;
            p.midi_buffer.magic = JACK_MIDI_MAGIC;
            p.midi_buffer.lasapi_client = client_ptr;
        } else {
            p.is_midi = false;
            p.audio_buffer = unsafe { calloc(per_frames, mem::size_of::<f32>()) }.cast::<f32>();
        }
        &raw mut *p
    };
    j.num_ports.store((idx + 1) as c_int, Ordering::Release);
    p_ptr
}

#[no_mangle]
pub extern "C" fn jack_port_unregister(_jc: *mut jack_client_t, _p: *mut jack_port_t) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_get_buffer(
    p: *mut jack_port_t,
    _nframes: JackNframes,
) -> *mut c_void {
    if p.is_null() {
        return ptr::null_mut();
    }
    let p_ref = unsafe { &mut *p };
    if p_ref.is_midi {
        (&raw mut p_ref.midi_buffer).cast()
    } else {
        p_ref.audio_buffer.cast()
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_name(p: *const jack_port_t) -> *const c_char {
    if p.is_null() {
        c"system:playback_1".as_ptr()
    } else {
        unsafe { (*p).full_name.as_ptr().cast() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_short_name(p: *const jack_port_t) -> *const c_char {
    if p.is_null() {
        c"playback_1".as_ptr()
    } else {
        unsafe { (*p).name.as_ptr().cast() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_flags(p: *const jack_port_t) -> c_int {
    if p.is_null() {
        2
    } else {
        unsafe { (*p).flags as c_int }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_port_by_name(
    jc: *mut jack_client_t,
    port_name: *const c_char,
) -> *mut jack_port_t {
    if jc.is_null() || port_name.is_null() {
        return ptr::null_mut();
    }
    let j = unsafe { &mut *jc };
    let target = unsafe { CStr::from_ptr(port_name) }.to_str().unwrap_or("");
    let nports = j.num_ports.load(Ordering::Acquire) as usize;
    for port in j.ports.iter_mut().take(nports) {
        if cbuf_to_str(&port.full_name) == target || cbuf_to_str(&port.name) == target {
            return &raw mut *port;
        }
    }
    if nports > 0 {
        &raw mut j.ports[0]
    } else {
        ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_ports(
    _jc: *mut jack_client_t,
    _port_name_pattern: *const c_char,
    _type_name_pattern: *const c_char,
    _flags: c_ulong,
) -> *mut *const c_char {
    let list = unsafe { calloc(7, mem::size_of::<*const c_char>()) }.cast::<*const c_char>();
    if list.is_null() {
        return ptr::null_mut();
    }
    const NAMES: [&CStr; 6] = [
        c"system:capture_1",
        c"system:capture_2",
        c"system:playback_1",
        c"system:playback_2",
        c"system:midi_capture_1",
        c"system:midi_playback_1",
    ];
    for (i, n) in NAMES.iter().enumerate() {
        unsafe { *list.add(i) = n.as_ptr() };
    }
    unsafe { *list.add(6) = ptr::null() };
    list
}

#[no_mangle]
pub unsafe extern "C" fn jack_free(ptr_in: *mut c_void) {
    unsafe { free(ptr_in) };
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_sample_rate(jc: *mut jack_client_t) -> JackNframes {
    if jc.is_null() {
        48000
    } else {
        unsafe { (*jc).sample_rate }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_get_buffer_size(jc: *mut jack_client_t) -> JackNframes {
    if jc.is_null() {
        256
    } else {
        unsafe { (*jc).period_frames }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_frame_time(jc: *const jack_client_t) -> JackNframes {
    if jc.is_null() {
        0
    } else {
        unsafe { (*jc).frame_counter as JackNframes }
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_last_frame_time(jc: *const jack_client_t) -> JackNframes {
    if jc.is_null() {
        0
    } else {
        unsafe { (*jc).frame_counter as JackNframes }
    }
}

#[no_mangle]
pub extern "C" fn jack_get_time() -> u64 {
    let mut ts = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64)
        .saturating_mul(1_000_000)
        .saturating_add((ts.tv_nsec as u64) / 1000)
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_start(jc: *mut jack_client_t) {
    if !jc.is_null() {
        unsafe { (*jc).transport_rolling.store(1, Ordering::Release) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_stop(jc: *mut jack_client_t) {
    if !jc.is_null() {
        unsafe { (*jc).transport_rolling.store(0, Ordering::Release) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_locate(
    jc: *mut jack_client_t,
    frame: JackNframes,
) -> c_int {
    if !jc.is_null() {
        unsafe { (*jc).frame_counter = u64::from(frame) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_transport_query(
    jc: *const jack_client_t,
    pos: *mut c_void,
) -> c_int {
    if !pos.is_null() {
        unsafe { ptr::write_bytes(pos.cast::<u8>(), 0, 136) };
    }
    if !jc.is_null() && unsafe { (*jc).transport_rolling.load(Ordering::Acquire) != 0 } {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn jack_activate(jc: *mut jack_client_t) -> c_int {
    if jc.is_null() {
        return -1;
    }
    let j = unsafe { &mut *jc };
    if j.active
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return -1;
    }
    let jname = cbuf_to_str(&j.name).to_string();
    with_lasapi_backend(|b| b.start(j.lasapi_client));
    let rc = unsafe {
        pthread_create(
            &mut j.rt_thread,
            ptr::null(),
            jack_wavert_callback_thread,
            jc.cast(),
        )
    };
    if rc != 0 {
        j.active.store(0, Ordering::Release);
        with_lasapi_backend(|b| b.stop(j.lasapi_client));
        return -1;
    }
    with_lasapi_backend(|b| {
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_JACK_ACT",
            &jname,
            "RT callback thread started",
        );
    });
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_deactivate(jc: *mut jack_client_t) -> c_int {
    if jc.is_null() {
        return -1;
    }
    let j = unsafe { &mut *jc };
    let jname = cbuf_to_str(&j.name).to_string();
    if j.active.swap(0, Ordering::AcqRel) != 0 {
        unsafe { pthread_join(j.rt_thread, ptr::null_mut()) };
    }
    with_lasapi_backend(|b| {
        b.stop(j.lasapi_client);
        b.debug_trace(
            LASAPI_DBG_SHIM,
            LASAPI_DBG_INFO,
            0,
            "SHIM_JACK_DEACT",
            &jname,
            "RT callback thread stopped",
        );
    });
    0
}

#[no_mangle]
pub extern "C" fn jack_connect(
    _jc: *mut jack_client_t,
    _src: *const c_char,
    _dst: *const c_char,
) -> c_int {
    0
}

#[no_mangle]
pub extern "C" fn jack_disconnect(
    _jc: *mut jack_client_t,
    _src: *const c_char,
    _dst: *const c_char,
) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_get_event_count(port_buffer: *mut c_void) -> u32 {
    if port_buffer.is_null() {
        return 0;
    }
    let mb = unsafe { &*port_buffer.cast::<jack_midi_buf_t>() };
    if mb.magic != JACK_MIDI_MAGIC {
        return 0;
    }
    mb.count
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_event_get(
    event: *mut jack_midi_event_t,
    port_buffer: *mut c_void,
    event_index: u32,
) -> c_int {
    if event.is_null() || port_buffer.is_null() {
        return -1;
    }
    let mb = unsafe { &mut *port_buffer.cast::<jack_midi_buf_t>() };
    if mb.magic != JACK_MIDI_MAGIC || event_index >= mb.count {
        return -1;
    }
    let src = &mut mb.events[event_index as usize];
    unsafe {
        (*event).time = src.frame_offset;
        (*event).size = usize::from(src.size);
        (*event).buffer = src.data.as_mut_ptr();
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_clear_buffer(port_buffer: *mut c_void) {
    if port_buffer.is_null() {
        return;
    }
    let mb = unsafe { &mut *port_buffer.cast::<jack_midi_buf_t>() };
    if mb.magic == JACK_MIDI_MAGIC {
        mb.count = 0;
    }
}

#[no_mangle]
pub extern "C" fn jack_midi_max_event_size(_port_buffer: *mut c_void) -> usize {
    3
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_event_reserve(
    port_buffer: *mut c_void,
    time: JackNframes,
    data_size: usize,
) -> *mut u8 {
    if port_buffer.is_null() {
        return ptr::null_mut();
    }
    let mb = unsafe { &mut *port_buffer.cast::<jack_midi_buf_t>() };
    if mb.magic != JACK_MIDI_MAGIC
        || (mb.count as usize) >= JACK_MAX_PORT_MIDI_EVENTS
        || data_size > 3
    {
        return ptr::null_mut();
    }
    let idx = mb.count as usize;
    mb.count += 1;
    let ev = &mut mb.events[idx];
    *ev = LasapiMidiEvent::default();
    ev.frame_offset = time;
    ev.size = data_size as u8;
    ev.data.as_mut_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn jack_midi_event_write(
    port_buffer: *mut c_void,
    time: JackNframes,
    data: *const u8,
    data_size: usize,
) -> c_int {
    if port_buffer.is_null() || data.is_null() || data_size == 0 || data_size > 3 {
        return -1;
    }
    let mb_ptr = port_buffer.cast::<jack_midi_buf_t>();
    if unsafe { ptr::addr_of!((*mb_ptr).magic).read() } != JACK_MIDI_MAGIC {
        return -1;
    }
    let dst = unsafe { jack_midi_event_reserve(port_buffer, time, data_size) };
    if dst.is_null() {
        return -1;
    }
    unsafe { ptr::copy_nonoverlapping(data, dst, data_size) };
    let mb = unsafe { &*mb_ptr };
    if !mb.lasapi_client.is_null() && mb.count > 0 {
        let ev = &mb.events[(mb.count - 1) as usize];
        with_lasapi_backend(|b| b.write_midi_event(mb.lasapi_client, ev));
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use salsa_wire::client_seam::InMemoryLasapiBackend;
    use std::sync::Arc;

    struct JackCtx {
        out_l: *mut jack_port_t,
        out_r: *mut jack_port_t,
        in_l: *mut jack_port_t,
        midi_in: *mut jack_port_t,
        midi_out: *mut jack_port_t,
        got_midi: u32,
        in_sample_l: f32,
    }

    unsafe extern "C" fn test_process_cb(nframes: JackNframes, arg: *mut c_void) -> c_int {
        let ctx = &mut *arg.cast::<JackCtx>();
        let il = jack_port_get_buffer(ctx.in_l, nframes).cast::<f32>();
        let ol = jack_port_get_buffer(ctx.out_l, nframes).cast::<f32>();
        let or_ = jack_port_get_buffer(ctx.out_r, nframes).cast::<f32>();
        ctx.in_sample_l = *il;
        for f in 0..nframes as usize {
            *ol.add(f) = *il.add(f) * 0.5;
            *or_.add(f) = -0.25;
        }
        let mi = jack_port_get_buffer(ctx.midi_in, nframes);
        ctx.got_midi = jack_midi_get_event_count(mi);
        let mo = jack_port_get_buffer(ctx.midi_out, nframes);
        let note_on = [0x90u8, 64, 127];
        jack_midi_event_write(mo, 4, note_on.as_ptr(), 3);
        0
    }

    #[test]
    fn jack_open_ports_cycle_and_close_against_in_memory_backend() {
        let mem = Arc::new(InMemoryLasapiBackend::new(48_000, 2, 256));
        with_test_lasapi_backend(Arc::clone(&mem) as _, || unsafe {
            let mut status = -1;
            let jc = jack_client_open(c"ArdourTest".as_ptr(), 0, &raw mut status);
            assert!(!jc.is_null());
            assert_eq!(status, 0);
            let jref = &mut *jc;
            assert_eq!(jref.sample_rate, 48_000);
            assert_eq!(jref.period_frames, 256);
            let h = jref.lasapi_client;

            let in_l = jack_port_register(jc, c"cap_1".as_ptr(), c"32 bit float mono audio".as_ptr(), 0x1, 0);
            let out_l = jack_port_register(jc, c"out_1".as_ptr(), c"32 bit float mono audio".as_ptr(), 0x2, 0);
            let out_r = jack_port_register(jc, c"out_2".as_ptr(), c"32 bit float mono audio".as_ptr(), 0x2, 0);
            let midi_in = jack_port_register(jc, c"midi_in".as_ptr(), c"8 bit raw midi".as_ptr(), 0x1, 0);
            let midi_out = jack_port_register(jc, c"midi_out".as_ptr(), c"8 bit raw midi".as_ptr(), 0x2, 0);

            let mut ctx = JackCtx {
                out_l,
                out_r,
                in_l,
                midi_in,
                midi_out,
                got_midi: 0,
                in_sample_l: 0.0,
            };
            jack_set_process_callback(jc, Some(test_process_cb), (&raw mut ctx).cast());

            let mut cap_samples = vec![0u8; 256 * 8];
            for f in 0..256 {
                let l_bytes = 0.8f32.to_le_bytes();
                cap_samples[f * 8..f * 8 + 4].copy_from_slice(&l_bytes);
            }
            mem.push_capture_bytes(h, &cap_samples);
            mem.push_midi_in(
                h,
                LasapiMidiEvent {
                    frame_offset: 1,
                    size: 3,
                    data: [0x90, 60, 99, 0],
                    ..LasapiMidiEvent::default()
                },
            );

            let mut inter = vec![0.0f32; 256 * 2];
            let mut cap_inter = vec![0.0f32; 256 * 2];
            run_one_jack_cycle(jref, &mut inter, &mut cap_inter, mem.as_ref());

            assert!((ctx.in_sample_l - 0.8).abs() < 1e-6);
            assert_eq!(ctx.got_midi, 1);
            assert!((inter[0] - 0.4).abs() < 1e-6);
            assert!((inter[1] - (-0.25)).abs() < 1e-6);
            assert_eq!(jref.frame_counter, 256);

            let snap = mem.snapshot_client(h).unwrap();
            assert_eq!(snap.written_frames, 256);
            assert_eq!(snap.midi_out.len(), 1);
            assert_eq!(&snap.midi_out[0].data[..3], &[0x90, 64, 127]);

            assert_eq!(jack_client_close(jc), 0);
            assert_eq!(mem.snapshot_client(h).unwrap().release_count, 1);
        });
    }
}
