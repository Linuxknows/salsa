// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::{c_char, c_int, c_void};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::c_abi::{
    LasapiMidiEvent, WaveFormatEx, AUDCLNT_E_ALREADY_INITIALIZED, AUDCLNT_E_NOT_INITIALIZED,
    E_INVALIDARG, LASAPI_FMT_FLOAT32, LASAPI_FMT_S16_LE, LASAPI_FMT_S32_LE, S_OK,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ClientHandle(pub usize);

impl ClientHandle {
    pub const NULL: Self = Self(0);

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

pub fn bytes_per_sample_for_fmt(fmt: u32) -> usize {
    match fmt {
        LASAPI_FMT_S16_LE => 2,
        LASAPI_FMT_S32_LE | LASAPI_FMT_FLOAT32 => 4,
        _ => 4,
    }
}

pub trait AudioSessionBackend: Send + Sync {
    fn debug_trace(&self, sub: u8, lvl: u8, sid: u32, op: &str, target: &str, detail: &str);
    fn activate_client(&self, client_name: Option<&str>) -> Result<ClientHandle, i32>;
    fn get_mix_format(&self, client: ClientHandle) -> Result<WaveFormatEx, i32>;
    fn initialize(
        &self,
        client: ClientHandle,
        share_mode: u32,
        stream_flags: u32,
        buffer_frames: u32,
        period_frames: u32,
        format: &WaveFormatEx,
    ) -> i32;
    fn get_buffer_size(&self, client: ClientHandle) -> Result<u32, i32>;
    fn get_device_period(&self, client: ClientHandle) -> Result<(u32, u32), i32>;
    fn get_current_padding(&self, client: ClientHandle) -> Result<u32, i32>;
    fn set_event_handle(&self, client: ClientHandle) -> Result<i32, i32>;
    fn start(&self, client: ClientHandle) -> i32;
    fn stop(&self, client: ClientHandle) -> i32;
    fn reset(&self, client: ClientHandle) -> i32;
    fn wait_for_period(&self, client: ClientHandle, timeout_ms: i32) -> i32;
    fn write_frames(&self, client: ClientHandle, src_bytes: &[u8], frames: u32, src_fmt: u32) -> i32;
    fn get_next_packet_size(&self, client: ClientHandle) -> Result<u32, i32>;
    fn read_frames(
        &self,
        client: ClientHandle,
        dst_bytes: &mut [u8],
        frames: u32,
        dst_fmt: u32,
    ) -> i32;
    fn set_master_volume(&self, client: ClientHandle, level: f32) -> i32;
    fn read_midi_events(&self, client: ClientHandle, out: &mut [LasapiMidiEvent]) -> u32;
    fn write_midi_event(&self, client: ClientHandle, ev: &LasapiMidiEvent) -> i32;
    fn release(&self, client: ClientHandle) -> i32;
}

#[derive(Clone, Copy)]
pub struct LasapiFfiVTable {
    pub debug_trace: unsafe extern "C" fn(u8, u8, u32, *const c_char, *const c_char, *const c_char),
    pub activate_audio_client: unsafe extern "C" fn(*const c_char, *mut *mut c_void) -> i32,
    pub get_mix_format: unsafe extern "C" fn(*mut c_void, *mut WaveFormatEx) -> i32,
    pub initialize:
        unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32, *const WaveFormatEx) -> i32,
    pub get_buffer_size: unsafe extern "C" fn(*mut c_void, *mut u32) -> i32,
    pub get_device_period: unsafe extern "C" fn(*mut c_void, *mut u32, *mut u32) -> i32,
    pub get_current_padding: unsafe extern "C" fn(*mut c_void, *mut u32) -> i32,
    pub set_event_handle: unsafe extern "C" fn(*mut c_void, *mut c_int) -> i32,
    pub start: unsafe extern "C" fn(*mut c_void) -> i32,
    pub stop: unsafe extern "C" fn(*mut c_void) -> i32,
    pub reset: unsafe extern "C" fn(*mut c_void) -> i32,
    pub wait_for_period: unsafe extern "C" fn(*mut c_void, c_int) -> i32,
    pub write_frames: unsafe extern "C" fn(*mut c_void, *const c_void, u32, u32) -> i32,
    pub get_next_packet_size: unsafe extern "C" fn(*mut c_void, *mut u32) -> i32,
    pub read_frames: unsafe extern "C" fn(*mut c_void, *mut c_void, u32, u32) -> i32,
    pub set_master_volume: unsafe extern "C" fn(*mut c_void, f32) -> i32,
    pub read_midi_events: unsafe extern "C" fn(*mut c_void, *mut LasapiMidiEvent, u32) -> u32,
    pub write_midi_event: unsafe extern "C" fn(*mut c_void, *const LasapiMidiEvent) -> i32,
    pub release: unsafe extern "C" fn(*mut c_void) -> i32,
}

pub struct FfiLasapiBackend {
    vtable: LasapiFfiVTable,
}

impl FfiLasapiBackend {
    pub const fn from_vtable(vtable: LasapiFfiVTable) -> Self {
        Self { vtable }
    }
}

fn copy_stack_cstr<const N: usize>(s: &str) -> [c_char; N] {
    let mut buf = [0 as c_char; N];
    if N <= 1 {
        return buf;
    }
    let bytes = s.as_bytes();
    let mut len = 0usize;
    for &b in bytes.iter().take(N - 1) {
        if b == 0 {
            break;
        }
        buf[len] = b as c_char;
        len += 1;
    }
    buf[len] = 0;
    buf
}

#[allow(unsafe_code)]
impl AudioSessionBackend for FfiLasapiBackend {
    fn debug_trace(&self, sub: u8, lvl: u8, sid: u32, op: &str, target: &str, detail: &str) {
        let op_b: [c_char; 64] = copy_stack_cstr(op);
        let tgt_b: [c_char; 128] = copy_stack_cstr(target);
        let det_b: [c_char; 256] = copy_stack_cstr(detail);
        unsafe {
            (self.vtable.debug_trace)(
                sub,
                lvl,
                sid,
                op_b.as_ptr(),
                tgt_b.as_ptr(),
                det_b.as_ptr(),
            );
        }
    }

    fn activate_client(&self, client_name: Option<&str>) -> Result<ClientHandle, i32> {
        let mut raw: *mut c_void = core::ptr::null_mut();
        let hr = match client_name {
            Some(name) => {
                let cname: [c_char; 128] = copy_stack_cstr(name);
                unsafe { (self.vtable.activate_audio_client)(cname.as_ptr(), &raw mut raw) }
            }
            None => unsafe {
                (self.vtable.activate_audio_client)(core::ptr::null(), &raw mut raw)
            },
        };
        if hr == S_OK && !raw.is_null() {
            Ok(ClientHandle(raw as usize))
        } else {
            Err(hr)
        }
    }

    fn get_mix_format(&self, client: ClientHandle) -> Result<WaveFormatEx, i32> {
        let mut wfx = WaveFormatEx::default();
        let hr = unsafe { (self.vtable.get_mix_format)(client.0 as *mut c_void, &raw mut wfx) };
        if hr == S_OK {
            Ok(wfx)
        } else {
            Err(hr)
        }
    }

    fn initialize(
        &self,
        client: ClientHandle,
        share_mode: u32,
        stream_flags: u32,
        buffer_frames: u32,
        period_frames: u32,
        format: &WaveFormatEx,
    ) -> i32 {
        unsafe {
            (self.vtable.initialize)(
                client.0 as *mut c_void,
                share_mode,
                stream_flags,
                buffer_frames,
                period_frames,
                format,
            )
        }
    }

    fn get_buffer_size(&self, client: ClientHandle) -> Result<u32, i32> {
        let mut frames = 0u32;
        let hr = unsafe { (self.vtable.get_buffer_size)(client.0 as *mut c_void, &raw mut frames) };
        if hr == S_OK {
            Ok(frames)
        } else {
            Err(hr)
        }
    }

    fn get_device_period(&self, client: ClientHandle) -> Result<(u32, u32), i32> {
        let mut def_p = 0u32;
        let mut min_p = 0u32;
        let hr = unsafe {
            (self.vtable.get_device_period)(client.0 as *mut c_void, &raw mut def_p, &raw mut min_p)
        };
        if hr == S_OK {
            Ok((def_p, min_p))
        } else {
            Err(hr)
        }
    }

    fn get_current_padding(&self, client: ClientHandle) -> Result<u32, i32> {
        let mut pad = 0u32;
        let hr = unsafe {
            (self.vtable.get_current_padding)(client.0 as *mut c_void, &raw mut pad)
        };
        if hr == S_OK {
            Ok(pad)
        } else {
            Err(hr)
        }
    }

    fn set_event_handle(&self, client: ClientHandle) -> Result<i32, i32> {
        let mut fd: c_int = -1;
        let hr = unsafe { (self.vtable.set_event_handle)(client.0 as *mut c_void, &raw mut fd) };
        if hr == S_OK {
            Ok(fd)
        } else {
            Err(hr)
        }
    }

    fn start(&self, client: ClientHandle) -> i32 {
        unsafe { (self.vtable.start)(client.0 as *mut c_void) }
    }

    fn stop(&self, client: ClientHandle) -> i32 {
        unsafe { (self.vtable.stop)(client.0 as *mut c_void) }
    }

    fn reset(&self, client: ClientHandle) -> i32 {
        unsafe { (self.vtable.reset)(client.0 as *mut c_void) }
    }

    fn wait_for_period(&self, client: ClientHandle, timeout_ms: i32) -> i32 {
        unsafe { (self.vtable.wait_for_period)(client.0 as *mut c_void, timeout_ms) }
    }

    fn write_frames(&self, client: ClientHandle, src_bytes: &[u8], frames: u32, src_fmt: u32) -> i32 {
        unsafe {
            (self.vtable.write_frames)(
                client.0 as *mut c_void,
                src_bytes.as_ptr().cast::<c_void>(),
                frames,
                src_fmt,
            )
        }
    }

    fn get_next_packet_size(&self, client: ClientHandle) -> Result<u32, i32> {
        let mut pkt = 0u32;
        let hr = unsafe {
            (self.vtable.get_next_packet_size)(client.0 as *mut c_void, &raw mut pkt)
        };
        if hr == S_OK {
            Ok(pkt)
        } else {
            Err(hr)
        }
    }

    fn read_frames(
        &self,
        client: ClientHandle,
        dst_bytes: &mut [u8],
        frames: u32,
        dst_fmt: u32,
    ) -> i32 {
        unsafe {
            (self.vtable.read_frames)(
                client.0 as *mut c_void,
                dst_bytes.as_mut_ptr().cast::<c_void>(),
                frames,
                dst_fmt,
            )
        }
    }

    fn set_master_volume(&self, client: ClientHandle, level: f32) -> i32 {
        unsafe { (self.vtable.set_master_volume)(client.0 as *mut c_void, level) }
    }

    fn read_midi_events(&self, client: ClientHandle, out: &mut [LasapiMidiEvent]) -> u32 {
        unsafe {
            (self.vtable.read_midi_events)(
                client.0 as *mut c_void,
                out.as_mut_ptr(),
                out.len() as u32,
            )
        }
    }

    fn write_midi_event(&self, client: ClientHandle, ev: &LasapiMidiEvent) -> i32 {
        unsafe { (self.vtable.write_midi_event)(client.0 as *mut c_void, ev) }
    }

    fn release(&self, client: ClientHandle) -> i32 {
        unsafe { (self.vtable.release)(client.0 as *mut c_void) }
    }
}

#[derive(Debug, Clone)]
pub struct InMemoryTrace {
    pub sub: u8,
    pub lvl: u8,
    pub sid: u32,
    pub op: String,
    pub target: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct InMemoryClientState {
    pub name: String,
    pub initialized: bool,
    pub share_mode: u32,
    pub stream_flags: u32,
    pub format: WaveFormatEx,
    pub buffer_frames: u32,
    pub period_frames: u32,
    pub padding_frames: u32,
    pub started: bool,
    pub master_volume: f32,
    pub written_frames: u64,
    pub written_bytes: Vec<u8>,
    pub capture_queue: VecDeque<u8>,
    pub midi_in: VecDeque<LasapiMidiEvent>,
    pub midi_out: Vec<LasapiMidiEvent>,
    pub release_count: u32,
}

impl InMemoryClientState {
    fn new(name: String) -> Self {
        Self {
            name,
            initialized: false,
            share_mode: 0,
            stream_flags: 0,
            format: WaveFormatEx::default(),
            buffer_frames: 2048,
            period_frames: 256,
            padding_frames: 0,
            started: false,
            master_volume: 1.0,
            written_frames: 0,
            written_bytes: Vec::new(),
            capture_queue: VecDeque::new(),
            midi_in: VecDeque::new(),
            midi_out: Vec::new(),
            release_count: 0,
        }
    }
}

pub struct InMemoryLasapiBackend {
    next_id: AtomicUsize,
    mix_format: WaveFormatEx,
    default_period_frames: u32,
    fail_activate: Mutex<Option<i32>>,
    clients: Mutex<BTreeMap<usize, InMemoryClientState>>,
    traces: Mutex<Vec<InMemoryTrace>>,
}

impl Default for InMemoryLasapiBackend {
    fn default() -> Self {
        Self::new(48_000, 2, 256)
    }
}

impl InMemoryLasapiBackend {
    pub fn new(sample_rate: u32, channels: u16, period_frames: u32) -> Self {
        let block_align = channels * 4;
        Self {
            next_id: AtomicUsize::new(1),
            mix_format: WaveFormatEx {
                w_format_tag: LASAPI_FMT_FLOAT32 as u16,
                n_channels: channels,
                n_samples_per_sec: sample_rate,
                w_bits_per_sample: 32,
                n_block_align: block_align,
                n_avg_bytes_per_sec: sample_rate * u32::from(block_align),
                cb_size: 0,
            },
            default_period_frames: period_frames.max(32),
            fail_activate: Mutex::new(None),
            clients: Mutex::new(BTreeMap::new()),
            traces: Mutex::new(Vec::new()),
        }
    }

    pub fn set_activate_failure(&self, err: Option<i32>) {
        *self.fail_activate.lock().unwrap_or_else(|e| e.into_inner()) = err;
    }

    pub fn push_capture_bytes(&self, client: ClientHandle, data: &[u8]) {
        if let Some(st) = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&client.0)
        {
            st.capture_queue.extend(data);
        }
    }

    pub fn push_midi_in(&self, client: ClientHandle, ev: LasapiMidiEvent) {
        if let Some(st) = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&client.0)
        {
            st.midi_in.push_back(ev);
        }
    }

    pub fn snapshot_client(&self, client: ClientHandle) -> Option<InMemoryClientState> {
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&client.0)
            .cloned()
    }

    pub fn all_client_handles(&self) -> Vec<ClientHandle> {
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .copied()
            .map(ClientHandle)
            .collect()
    }

    pub fn traces(&self) -> Vec<InMemoryTrace> {
        self.traces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl AudioSessionBackend for InMemoryLasapiBackend {
    fn debug_trace(&self, sub: u8, lvl: u8, sid: u32, op: &str, target: &str, detail: &str) {
        self.traces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(InMemoryTrace {
                sub,
                lvl,
                sid,
                op: op.to_string(),
                target: target.to_string(),
                detail: detail.to_string(),
            });
    }

    fn activate_client(&self, client_name: Option<&str>) -> Result<ClientHandle, i32> {
        if let Some(hr) = *self.fail_activate.lock().unwrap_or_else(|e| e.into_inner()) {
            return Err(hr);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let name = client_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("test-client")
            .to_string();
        let mut st = InMemoryClientState::new(name);
        st.period_frames = self.default_period_frames;
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, st);
        Ok(ClientHandle(id))
    }

    fn get_mix_format(&self, client: ClientHandle) -> Result<WaveFormatEx, i32> {
        let map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if !map.contains_key(&client.0) {
            return Err(E_INVALIDARG);
        }
        Ok(self.mix_format)
    }

    fn initialize(
        &self,
        client: ClientHandle,
        share_mode: u32,
        stream_flags: u32,
        buffer_frames: u32,
        period_frames: u32,
        format: &WaveFormatEx,
    ) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return E_INVALIDARG;
        };
        if st.initialized {
            return AUDCLNT_E_ALREADY_INITIALIZED;
        }
        st.initialized = true;
        st.share_mode = share_mode;
        st.stream_flags = stream_flags;
        st.format = *format;
        st.buffer_frames = buffer_frames.max(64);
        st.period_frames = period_frames.clamp(32, st.buffer_frames / 2);
        S_OK
    }

    fn get_buffer_size(&self, client: ClientHandle) -> Result<u32, i32> {
        let map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let st = map.get(&client.0).ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        Ok(st.buffer_frames)
    }

    fn get_device_period(&self, client: ClientHandle) -> Result<(u32, u32), i32> {
        let map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let st = map.get(&client.0).ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        Ok((st.period_frames, st.period_frames))
    }

    fn get_current_padding(&self, client: ClientHandle) -> Result<u32, i32> {
        let map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let st = map.get(&client.0).ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        Ok(st.padding_frames)
    }

    fn set_event_handle(&self, client: ClientHandle) -> Result<i32, i32> {
        let map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if !map.contains_key(&client.0) {
            return Err(AUDCLNT_E_NOT_INITIALIZED);
        }
        Ok(-1)
    }

    fn start(&self, client: ClientHandle) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        st.started = true;
        S_OK
    }

    fn stop(&self, client: ClientHandle) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        st.started = false;
        S_OK
    }

    fn reset(&self, client: ClientHandle) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        st.padding_frames = 0;
        st.capture_queue.clear();
        S_OK
    }

    fn wait_for_period(&self, client: ClientHandle, _timeout_ms: i32) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        st.padding_frames = st.padding_frames.saturating_sub(st.period_frames);
        S_OK
    }

    fn write_frames(&self, client: ClientHandle, src_bytes: &[u8], frames: u32, _src_fmt: u32) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        st.written_frames = st.written_frames.saturating_add(u64::from(frames));
        st.written_bytes.extend_from_slice(src_bytes);
        st.padding_frames = st.padding_frames.saturating_add(frames).min(st.buffer_frames);
        S_OK
    }

    fn get_next_packet_size(&self, client: ClientHandle) -> Result<u32, i32> {
        let map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let st = map.get(&client.0).ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        let fb = usize::from(st.format.n_block_align.max(4));
        Ok((st.capture_queue.len() / fb) as u32)
    }

    fn read_frames(
        &self,
        client: ClientHandle,
        dst_bytes: &mut [u8],
        _frames: u32,
        _dst_fmt: u32,
    ) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        for dst in dst_bytes.iter_mut() {
            *dst = st.capture_queue.pop_front().unwrap_or(0);
        }
        S_OK
    }

    fn set_master_volume(&self, client: ClientHandle, level: f32) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        st.master_volume = level.clamp(0.0, 2.0);
        S_OK
    }

    fn read_midi_events(&self, client: ClientHandle, out: &mut [LasapiMidiEvent]) -> u32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return 0;
        };
        let mut count = 0u32;
        for slot in out.iter_mut() {
            let Some(ev) = st.midi_in.pop_front() else {
                break;
            };
            *slot = ev;
            count += 1;
        }
        count
    }

    fn write_midi_event(&self, client: ClientHandle, ev: &LasapiMidiEvent) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = map.get_mut(&client.0) else {
            return AUDCLNT_E_NOT_INITIALIZED;
        };
        st.midi_out.push(*ev);
        S_OK
    }

    fn release(&self, client: ClientHandle) -> i32 {
        let mut map = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(st) = map.get_mut(&client.0) {
            st.started = false;
            st.release_count += 1;
            S_OK
        } else {
            E_INVALIDARG
        }
    }
}

#[macro_export]
macro_rules! define_lasapi_ffi_backend {
    () => {
        #[cfg(not(test))]
        mod lasapi_ffi_binding {
            use core::ffi::{c_char, c_int, c_void};
            use $crate::c_abi::{LasapiMidiEvent, WaveFormatEx};
            use $crate::client_seam::{AudioSessionBackend, FfiLasapiBackend, LasapiFfiVTable};

            extern "C" {
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
                fn IAudioClient_GetMixFormat(
                    client: *mut c_void,
                    p_format: *mut WaveFormatEx,
                ) -> i32;
                fn IAudioClient_Initialize(
                    client: *mut c_void,
                    share_mode: u32,
                    stream_flags: u32,
                    buffer_frame_count: u32,
                    periodicity_frames: u32,
                    p_format: *const WaveFormatEx,
                ) -> i32;
                fn IAudioClient_GetBufferSize(
                    client: *mut c_void,
                    p_num_buffer_frames: *mut u32,
                ) -> i32;
                fn IAudioClient_GetDevicePeriod(
                    client: *mut c_void,
                    p_default_period_frames: *mut u32,
                    p_minimum_period_frames: *mut u32,
                ) -> i32;
                fn IAudioClient_GetCurrentPadding(
                    client: *mut c_void,
                    p_num_padding_frames: *mut u32,
                ) -> i32;
                fn IAudioClient_SetEventHandle(
                    client: *mut c_void,
                    p_event_fd_out: *mut c_int,
                ) -> i32;
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
                fn ISimpleAudioVolume_SetMasterVolume(client: *mut c_void, f_level: f32) -> i32;
                fn IAudioClient_ReadMidiEvents(
                    client: *mut c_void,
                    events_out: *mut LasapiMidiEvent,
                    max_events: u32,
                ) -> u32;
                fn IAudioClient_WriteMidiEvent(
                    client: *mut c_void,
                    ev: *const LasapiMidiEvent,
                ) -> i32;
            }

            static BACKEND: FfiLasapiBackend = FfiLasapiBackend::from_vtable(LasapiFfiVTable {
                debug_trace: LASAPI_DebugTraceClient,
                activate_audio_client: LASAPI_ActivateAudioClient,
                get_mix_format: IAudioClient_GetMixFormat,
                initialize: IAudioClient_Initialize,
                get_buffer_size: IAudioClient_GetBufferSize,
                get_device_period: IAudioClient_GetDevicePeriod,
                get_current_padding: IAudioClient_GetCurrentPadding,
                set_event_handle: IAudioClient_SetEventHandle,
                start: IAudioClient_Start,
                stop: IAudioClient_Stop,
                reset: IAudioClient_Reset,
                wait_for_period: IAudioClient_WaitForPeriod,
                write_frames: IAudioRenderClient_WriteFrames,
                get_next_packet_size: IAudioCaptureClient_GetNextPacketSize,
                read_frames: IAudioCaptureClient_ReadFrames,
                set_master_volume: ISimpleAudioVolume_SetMasterVolume,
                read_midi_events: IAudioClient_ReadMidiEvents,
                write_midi_event: IAudioClient_WriteMidiEvent,
                release: IAudioClient_Release,
            });

            #[inline]
            pub(crate) fn with_lasapi_backend<R>(
                f: impl FnOnce(&dyn AudioSessionBackend) -> R,
            ) -> R {
                f(&BACKEND)
            }
        }

        #[cfg(test)]
        mod lasapi_ffi_binding {
            use core::cell::RefCell;
            use std::sync::{Arc, OnceLock};
            use $crate::client_seam::{AudioSessionBackend, InMemoryLasapiBackend};

            fn default_test_backend() -> &'static InMemoryLasapiBackend {
                static DEFAULT: OnceLock<InMemoryLasapiBackend> = OnceLock::new();
                DEFAULT.get_or_init(InMemoryLasapiBackend::default)
            }

            thread_local! {
                static TEST_BACKEND: RefCell<Option<Arc<dyn AudioSessionBackend>>> =
                    const { RefCell::new(None) };
            }

            pub(crate) fn with_lasapi_backend<R>(
                f: impl FnOnce(&dyn AudioSessionBackend) -> R,
            ) -> R {
                TEST_BACKEND.with(|cell| {
                    let b = cell.borrow().clone();
                    match b {
                        Some(arc) => f(arc.as_ref()),
                        None => f(default_test_backend()),
                    }
                })
            }

            pub(crate) fn with_test_lasapi_backend<R>(
                backend: Arc<dyn AudioSessionBackend>,
                f: impl FnOnce() -> R,
            ) -> R {
                let prev = TEST_BACKEND.with(|cell| cell.borrow_mut().replace(backend));
                struct Reset(Option<Arc<dyn AudioSessionBackend>>);
                impl Drop for Reset {
                    fn drop(&mut self) {
                        let old = self.0.take();
                        TEST_BACKEND.with(|cell| {
                            *cell.borrow_mut() = old;
                        });
                    }
                }
                let _guard = Reset(prev);
                f()
            }
        }

        use lasapi_ffi_binding::with_lasapi_backend;
        #[cfg(test)]
        use lasapi_ffi_binding::with_test_lasapi_backend;
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_abi::AUDCLNT_SHAREMODE_SHARED;

    #[test]
    fn in_memory_lasapi_backend_lifecycle_audio_and_midi() {
        let mem = InMemoryLasapiBackend::new(48_000, 2, 256);
        let h = mem.activate_client(Some("SeamTest")).unwrap();
        assert!(!h.is_null());

        let wfx = mem.get_mix_format(h).unwrap();
        assert_eq!(wfx.n_samples_per_sec, 48_000);
        assert_eq!(wfx.n_channels, 2);
        assert_eq!(
            mem.initialize(h, AUDCLNT_SHAREMODE_SHARED, 0, 1024, 256, &wfx),
            S_OK
        );
        assert_eq!(mem.get_buffer_size(h), Ok(1024));
        assert_eq!(mem.get_device_period(h), Ok((256, 256)));
        assert_eq!(mem.start(h), S_OK);
        assert_eq!(mem.set_master_volume(h, 0.75), S_OK);

        let tx = [0x11u8; 256 * 8];
        assert_eq!(mem.write_frames(h, &tx, 256, LASAPI_FMT_FLOAT32), S_OK);
        assert_eq!(mem.get_current_padding(h), Ok(256));
        assert_eq!(mem.wait_for_period(h, 10), S_OK);
        assert_eq!(mem.get_current_padding(h), Ok(0));

        let rx_seed = [0x5Au8; 64 * 8];
        mem.push_capture_bytes(h, &rx_seed);
        assert_eq!(mem.get_next_packet_size(h), Ok(64));
        let mut rx_out = vec![0u8; 64 * 8];
        assert_eq!(mem.read_frames(h, &mut rx_out, 64, LASAPI_FMT_FLOAT32), S_OK);
        assert_eq!(rx_out, rx_seed);

        let ev = LasapiMidiEvent {
            frame_offset: 12,
            size: 3,
            data: [0x90, 60, 100, 0],
            ..LasapiMidiEvent::default()
        };
        mem.push_midi_in(h, ev);
        let mut midi_buf = [LasapiMidiEvent::default(); 4];
        assert_eq!(mem.read_midi_events(h, &mut midi_buf), 1);
        assert_eq!(&midi_buf[0].data[..3], &[0x90, 60, 100]);
        assert_eq!(mem.write_midi_event(h, &ev), S_OK);

        mem.debug_trace(1, 2, 0, "SEAM_TEST", "mem", "ok");
        assert_eq!(mem.traces().len(), 1);
        assert_eq!(mem.stop(h), S_OK);
        assert_eq!(mem.release(h), S_OK);
        let snap = mem.snapshot_client(h).unwrap();
        assert_eq!(snap.written_frames, 256);
        assert_eq!(snap.release_count, 1);
        assert_eq!(snap.midi_out.len(), 1);
    }
}
