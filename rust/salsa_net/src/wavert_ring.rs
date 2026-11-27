// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

//! Safe shared-memory WaveRT Ring trust seam shared by `lasapi-audiodg` and `lasapi_client`.
//!
//! All raw `LasapiShmRing` pointer arithmetic, cross-process atomic cursor loads/stores,
//! header bounds validation, and `period_frames` updates live inside this single module.
//! No reference (`&LasapiShmRing` or `&mut LasapiShmRing`) is ever formed over mapped
//! shared memory, eliminating cross-process aliasing UB at the trust seam while keeping
//! the 1,323,200-byte binary ring layout identical (ADR-0001).

#![allow(unsafe_code)]

use core::mem::{self, offset_of};
use core::ptr;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};

use crate::c_abi::{
    copy_str_to_c, LasapiMidiEvent, LasapiShmRing, E_FAIL, E_OUTOFMEMORY, LASAPI_FMT_FLOAT32,
    LASAPI_MAX_CHANNELS, LASAPI_MIDI_RING_EVENTS, LASAPI_MIDI_RING_MASK, LASAPI_RING_FRAMES,
    LASAPI_RING_MAGIC, LASAPI_RING_MASK,
};
use crate::unix_sys::{create_memfd, memfd_size_valid, seal_memfd_size, MappedMem};

#[derive(Clone, Copy, Debug)]
pub struct SessionRingSpec {
    pub session_id: u32,
    pub client_pid: u32,
    pub client_name: [u8; 64],
    pub share_mode: u32,
    pub stream_flags: u32,
    pub is_capture: bool,
    pub sample_rate: u32,
    pub channels: u32,
    pub format: u32,
    pub buffer_frames: u32,
    pub period_frames: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RingMuteOp {
    #[default]
    Keep,
    Unmute,
    Mute,
    Toggle,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RingTelemetry {
    pub active: u32,
    pub muted: u32,
    pub volume_milli: u32,
    pub buffered_frames: u64,
    pub xruns: u64,
    pub period_frames: u32,
    pub stream_flags: u32,
    pub buffer_frames: u32,
    pub write_frame: u64,
    pub read_frame: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SessionResampleState {
    pub phase: f64,
    pub stream_live: bool,
    pub underrun_pending: bool,
    pub last_audio_ns: u64,
}

impl SessionResampleState {
    pub const fn new() -> Self {
        Self {
            phase: 0.0,
            stream_live: false,
            underrun_pending: false,
            last_audio_ns: 0,
        }
    }

    pub fn reset_idle(&mut self) {
        self.stream_live = false;
        self.underrun_pending = false;
        self.phase = 0.0;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ConsumeReport {
    pub pre_active: u32,
    pub pre_live: bool,
    pub pre_pending: bool,
    pub pre_avail: u64,
    pub pre_buf: u32,
    pub pre_xruns: u64,
    pub post_xruns: u64,
    pub post_pending: bool,
}

struct RingMem {
    map: MappedMem,
}

impl RingMem {
    #[inline]
    fn base(&self) -> *mut LasapiShmRing {
        self.map.as_ptr() as *mut LasapiShmRing
    }

    #[inline]
    fn active(&self) -> &AtomicU32 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).active) }
    }

    #[inline]
    fn muted(&self) -> &AtomicU32 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).muted) }
    }

    #[inline]
    fn volume_milli(&self) -> &AtomicU32 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).volume_milli) }
    }

    #[inline]
    fn xruns(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).xruns) }
    }

    #[inline]
    fn hw_clock_ns(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).hw_clock_ns) }
    }

    #[inline]
    fn write_frame(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).write_frame) }
    }

    #[inline]
    fn read_frame(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).read_frame) }
    }

    #[inline]
    fn cap_write_frame(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).cap_write_frame) }
    }

    #[inline]
    fn cap_read_frame(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).cap_read_frame) }
    }

    #[inline]
    fn midi_write_idx(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).midi_write_idx) }
    }

    #[inline]
    fn midi_read_idx(&self) -> &AtomicU64 {
        let b = self.base();
        unsafe { &*ptr::addr_of!((*b).midi_read_idx) }
    }

    #[inline]
    fn period_frames_atomic(&self) -> &AtomicU32 {
        let b = self.base();
        unsafe { AtomicU32::from_ptr(ptr::addr_of_mut!((*b).period_frames)) }
    }

    #[inline]
    fn pcm_base_ptr(&self) -> *mut f32 {
        let b = self.base();
        unsafe { ptr::addr_of_mut!((*b).pcm_data.0).cast::<f32>() }
    }

    #[inline]
    fn cap_pcm_base_ptr(&self) -> *mut f32 {
        let b = self.base();
        unsafe { ptr::addr_of_mut!((*b).cap_pcm_data.0).cast::<f32>() }
    }

    #[inline]
    fn midi_ring_slot_ptr(&self, slot: usize) -> *mut LasapiMidiEvent {
        let b = self.base();
        unsafe { ptr::addr_of_mut!((*b).midi_ring[slot & LASAPI_MIDI_RING_MASK]) }
    }
}

/// Daemon-side shared-memory **WaveRT Ring** handle owning the authoritative negotiated
/// `SessionRingSpec` geometry (never trusting peer-writable header copies).
pub struct WaveRtEngineRing {
    mem: RingMem,
    spec: SessionRingSpec,
}

unsafe impl Send for WaveRtEngineRing {}

impl WaveRtEngineRing {
    /// Creates an anonymous `"lasapi_ring"` memfd, maps it RW, initializes the ring header,
    /// seals its size (`F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL`), and returns both the
    /// daemon ring handle and the dup-ready memfd to send over `SCM_RIGHTS`.
    pub fn create_sealed(mut spec: SessionRingSpec) -> Option<(Self, OwnedFd)> {
        let ring_bytes = mem::size_of::<LasapiShmRing>();
        let shm_fd = create_memfd("lasapi_ring", ring_bytes)?;
        let map = MappedMem::map_fd(shm_fd.as_raw_fd(), ring_bytes, true, 0)?;

        spec.channels = spec.channels.clamp(1, LASAPI_MAX_CHANNELS as u32);
        spec.buffer_frames = spec.buffer_frames.clamp(1, LASAPI_RING_FRAMES as u32);
        if spec.format == 0 {
            spec.format = LASAPI_FMT_FLOAT32;
        }

        let base = map.as_ptr() as *mut LasapiShmRing;
        unsafe {
            ptr::write_bytes(map.as_ptr().cast_mut(), 0, offset_of!(LasapiShmRing, pcm_data));
            ptr::addr_of_mut!((*base).magic).write(LASAPI_RING_MAGIC);
            ptr::addr_of_mut!((*base).session_id).write(spec.session_id);
            ptr::addr_of_mut!((*base).client_pid).write(spec.client_pid);
            let cname = crate::c_abi::c_str_to_string(&spec.client_name);
            let cname_s = if cname.is_empty() { "AudioSession" } else { &cname };
            copy_str_to_c(&mut *ptr::addr_of_mut!((*base).client_name), cname_s);
            copy_str_to_c(&mut spec.client_name, cname_s);
            ptr::addr_of_mut!((*base).share_mode).write(spec.share_mode);
            ptr::addr_of_mut!((*base).stream_flags).write(spec.stream_flags);
            ptr::addr_of_mut!((*base).is_capture).write(u32::from(spec.is_capture));
            ptr::addr_of_mut!((*base).sample_rate).write(spec.sample_rate);
            ptr::addr_of_mut!((*base).channels).write(spec.channels);
            ptr::addr_of_mut!((*base).format).write(spec.format);
            ptr::addr_of_mut!((*base).buffer_frames).write(spec.buffer_frames);
        }

        let mem = RingMem { map };
        mem.period_frames_atomic()
            .store(spec.period_frames, Ordering::Release);
        mem.volume_milli().store(1000, Ordering::SeqCst);
        seal_memfd_size(shm_fd.as_raw_fd());

        Some((Self { mem, spec }, shm_fd))
    }

    /// Creates a connected daemon+client memfd pair for isolated tests without IPC sockets.
    #[cfg(test)]
    pub fn new_memfd_pair(spec: SessionRingSpec) -> Option<(Self, WaveRtClientRing, OwnedFd)> {
        let (eng, fd) = Self::create_sealed(spec)?;
        let (cli, _, _) = WaveRtClientRing::attach_fd(
            fd.as_raw_fd(),
            eng.spec.buffer_frames,
            eng.spec.channels,
        )
        .ok()?;
        Some((eng, cli, fd))
    }

    pub fn spec(&self) -> &SessionRingSpec {
        &self.spec
    }

    pub fn set_period_frames(&self, period_frames: u32) {
        self.mem
            .period_frames_atomic()
            .store(period_frames, Ordering::Release);
    }

    pub fn set_hw_clock_ns(&self, now_ns: u64) {
        self.mem.hw_clock_ns().store(now_ns, Ordering::Relaxed);
    }

    pub fn set_volume_and_mute(&self, volume_milli: u32, muted: u32) {
        self.mem.volume_milli().store(volume_milli, Ordering::SeqCst);
        self.mem.muted().store(muted, Ordering::SeqCst);
    }

    pub fn set_volume_milli_query_muted(&self, volume_milli: u32) -> bool {
        self.mem.volume_milli().store(volume_milli, Ordering::SeqCst);
        self.mem.muted().load(Ordering::SeqCst) != 0
    }

    pub fn apply_control(&self, volume_milli: Option<u32>, mute: RingMuteOp) -> (u32, bool) {
        if let Some(vm) = volume_milli {
            self.mem.volume_milli().store(vm.min(1500), Ordering::SeqCst);
        }
        match mute {
            RingMuteOp::Unmute => self.mem.muted().store(0, Ordering::SeqCst),
            RingMuteOp::Mute => self.mem.muted().store(1, Ordering::SeqCst),
            RingMuteOp::Toggle => {
                let cur = self.mem.muted().load(Ordering::SeqCst);
                self.mem.muted().store(u32::from(cur == 0), Ordering::SeqCst);
            }
            RingMuteOp::Keep => {}
        }
        let vm = self.mem.volume_milli().load(Ordering::SeqCst);
        let mu = self.mem.muted().load(Ordering::SeqCst) != 0;
        (vm, mu)
    }

    pub fn telemetry(&self) -> RingTelemetry {
        let active = self.mem.active().load(Ordering::Relaxed);
        let muted = self.mem.muted().load(Ordering::Relaxed);
        let volume_milli = self.mem.volume_milli().load(Ordering::Relaxed);
        let (write_frame, read_frame) = if self.spec.is_capture {
            (
                self.mem.cap_write_frame().load(Ordering::Relaxed),
                self.mem.cap_read_frame().load(Ordering::Relaxed),
            )
        } else {
            (
                self.mem.write_frame().load(Ordering::Relaxed),
                self.mem.read_frame().load(Ordering::Relaxed),
            )
        };
        RingTelemetry {
            active,
            muted,
            volume_milli,
            buffered_frames: write_frame.saturating_sub(read_frame),
            xruns: self.mem.xruns().load(Ordering::Relaxed),
            period_frames: self.mem.period_frames_atomic().load(Ordering::Relaxed),
            stream_flags: self.spec.stream_flags,
            buffer_frames: self.spec.buffer_frames,
            write_frame,
            read_frame,
        }
    }

    /// Pushes one MIDI event into the session MIDI ring without blocking.
    pub fn push_midi(&self, ev: &LasapiMidiEvent) -> bool {
        let w = self.mem.midi_write_idx().load(Ordering::Relaxed);
        let r = self.mem.midi_read_idx().load(Ordering::Acquire);
        if w.wrapping_sub(r) >= LASAPI_MIDI_RING_EVENTS as u64 {
            return false;
        }
        let slot = (w as usize) & LASAPI_MIDI_RING_MASK;
        unsafe {
            self.mem.midi_ring_slot_ptr(slot).write(*ev);
        }
        self.mem
            .midi_write_idx()
            .store(w.wrapping_add(1), Ordering::Release);
        true
    }

    /// Pushes interleaved stereo (`f32` L,R) frames into the daemon-to-client capture ring.
    pub fn push_capture_stereo(&self, stereo: &[f32], frames: usize) {
        if frames == 0 || stereo.len() < frames * 2 {
            return;
        }
        let cwf = self.mem.cap_write_frame();
        let cw = cwf.load(Ordering::Relaxed);
        let cap_base = self.mem.cap_pcm_base_ptr();
        for f in 0..frames {
            let idx = ((cw.wrapping_add(f as u64) as usize) & LASAPI_RING_MASK) * 2;
            unsafe {
                cap_base.add(idx).write(stereo[f * 2]);
                cap_base.add(idx + 1).write(stereo[f * 2 + 1]);
            }
        }
        cwf.store(cw.wrapping_add(frames as u64), Ordering::Release);
    }

    /// Consumes session playback frames from shared memory into `mix`, applying gain,
    /// optional linear fractional rate conversion, partial-read 32-frame ramp-out tapering,
    /// NaN/Inf scrubbing, and idle/xrun tracking.
    #[allow(clippy::too_many_arguments)]
    pub fn consume_playback(
        &self,
        st: &mut SessionResampleState,
        mix: &mut [f32],
        out_frames: u32,
        out_channels: u32,
        hw_rate: u32,
        replace_output: bool,
        now_ns: u64,
    ) -> ConsumeReport {
        let pre_live = st.stream_live;
        let pre_pending = st.underrun_pending;
        let pre_active = self.mem.active().load(Ordering::Acquire);
        let w = self.mem.write_frame().load(Ordering::Acquire);
        let mut rd = self.mem.read_frame().load(Ordering::Relaxed);
        let pre_xruns = self.mem.xruns().load(Ordering::Relaxed);
        let mut report = ConsumeReport {
            pre_active,
            pre_live,
            pre_pending,
            pre_avail: w.saturating_sub(rd),
            pre_buf: self.spec.buffer_frames,
            pre_xruns,
            post_xruns: pre_xruns,
            post_pending: st.underrun_pending,
        };

        if out_frames == 0 || out_channels == 0 {
            return report;
        }
        if pre_active == 0 {
            st.reset_idle();
            report.post_pending = false;
            return report;
        }

        let ch = (self.spec.channels as usize).clamp(1, LASAPI_MAX_CHANNELS);
        let out_ch = out_channels as usize;
        let out_fr = out_frames as usize;
        let Some(mix_len) = out_fr.checked_mul(out_ch) else {
            return report;
        };
        if mix.len() < mix_len {
            return report;
        }

        let client_rate = if self.spec.sample_rate != 0 {
            self.spec.sample_rate
        } else {
            hw_rate
        };
        if client_rate == 0 || hw_rate == 0 {
            return report;
        }

        if w < rd {
            self.mem.read_frame().store(w, Ordering::Release);
            st.reset_idle();
            report.post_pending = false;
            return report;
        }

        if w <= rd {
            if st.stream_live {
                if now_ns > st.last_audio_ns && (now_ns - st.last_audio_ns) > 500_000_000 {
                    st.reset_idle();
                } else {
                    st.underrun_pending = true;
                }
            }
            report.post_pending = st.underrun_pending;
            return report;
        }

        let mut avail = w - rd;
        if avail > LASAPI_RING_FRAMES as u64 {
            let keep = (out_fr as u64).saturating_mul(2).min(LASAPI_RING_FRAMES as u64);
            rd = w.saturating_sub(keep);
            avail = w - rd;
            self.mem.read_frame().store(rd, Ordering::Release);
        }

        let min_avail = if client_rate == hw_rate { 1 } else { 2 };
        if avail < min_avail {
            if st.stream_live {
                if now_ns > st.last_audio_ns && (now_ns - st.last_audio_ns) > 500_000_000 {
                    st.reset_idle();
                } else {
                    st.underrun_pending = true;
                }
            }
            report.post_pending = st.underrun_pending;
            return report;
        }

        if st.underrun_pending {
            let post_x = self.mem.xruns().fetch_add(1, Ordering::Relaxed) + 1;
            report.post_xruns = post_x;
            st.underrun_pending = false;
        }

        let mut vol = if self.mem.muted().load(Ordering::Relaxed) != 0 {
            0.0f32
        } else {
            (self.mem.volume_milli().load(Ordering::Relaxed) as f32) / 1000.0
        };
        if !(0.0..=4.0).contains(&vol) {
            vol = if vol > 4.0 { 4.0 } else { 0.0 };
        }

        let pcm_base = self.mem.pcm_base_ptr() as *const f32;
        if client_rate == hw_rate {
            let to_read = (avail.min(out_fr as u64)) as usize;
            let fade_frames = if st.stream_live && to_read > 0 && to_read < out_fr {
                to_read.min(32)
            } else {
                0
            };
            let fade_start = to_read.saturating_sub(fade_frames);
            for f in 0..to_read {
                let slot = ((rd.wrapping_add(f as u64) as usize) & LASAPI_RING_MASK) * ch;
                let env = if fade_frames > 0 && f >= fade_start {
                    ((to_read - 1 - f) as f32) / (fade_frames as f32)
                } else {
                    1.0f32
                };
                for c in 0..out_ch {
                    let raw = unsafe { pcm_base.add(slot + (c % ch)).read() };
                    let sample = if raw.is_finite() { raw * vol * env } else { 0.0 };
                    let dst = &mut mix[f * out_ch + c];
                    if replace_output {
                        *dst = sample;
                    } else {
                        *dst += sample;
                    }
                }
            }
            self.mem
                .read_frame()
                .store(rd.wrapping_add(to_read as u64), Ordering::Release);
        } else {
            let step = (client_rate as f64) / (hw_rate as f64);
            let mut phase = st.phase;
            for f in 0..out_fr {
                let i0 = phase as u64;
                if i0 + 1 >= avail {
                    break;
                }
                let frac = (phase - (i0 as f64)) as f32;
                let slot0 = ((rd.wrapping_add(i0) as usize) & LASAPI_RING_MASK) * ch;
                let slot1 = ((rd.wrapping_add(i0 + 1) as usize) & LASAPI_RING_MASK) * ch;
                for c in 0..out_ch {
                    let ch_idx = c % ch;
                    let s0_raw = unsafe { pcm_base.add(slot0 + ch_idx).read() };
                    let s1_raw = unsafe { pcm_base.add(slot1 + ch_idx).read() };
                    let s0 = if s0_raw.is_finite() { s0_raw } else { 0.0 };
                    let s1 = if s1_raw.is_finite() { s1_raw } else { 0.0 };
                    let sample = (s0 + (s1 - s0) * frac) * vol;
                    let dst = &mut mix[f * out_ch + c];
                    if replace_output {
                        *dst = sample;
                    } else {
                        *dst += sample;
                    }
                }
                phase += step;
            }
            let advance = (phase as u64).min(avail);
            st.phase = phase - (advance as f64);
            self.mem
                .read_frame()
                .store(rd.wrapping_add(advance), Ordering::Release);
        }

        st.stream_live = true;
        st.last_audio_ns = now_ns;
        report.post_pending = false;
        report
    }
}

/// Client-side shared-memory **WaveRT Ring** handle that validates the `memfd` size and header
/// once at attach time and caches immutable session geometry in process-local memory.
pub struct WaveRtClientRing {
    mem: RingMem,
    channels: usize,
    buffer_frames: u32,
    is_capture: bool,
}

unsafe impl Send for WaveRtClientRing {}

impl WaveRtClientRing {
    pub fn attach_fd(
        shm_fd: RawFd,
        expected_buffer_frames: u32,
        expected_channels: u32,
    ) -> Result<(Self, u32, u32), i32> {
        let ring_bytes = mem::size_of::<LasapiShmRing>();
        if !memfd_size_valid(shm_fd, 0, ring_bytes as u64) {
            return Err(E_FAIL);
        }
        let Some(map) = MappedMem::map_fd(shm_fd, ring_bytes, true, 0) else {
            return Err(E_OUTOFMEMORY);
        };
        let hdr = map.as_ptr().cast::<LasapiShmRing>();
        let (ring_magic, ring_ch, ring_buf_frames, is_capture_raw) = unsafe {
            (
                ptr::addr_of!((*hdr).magic).read(),
                ptr::addr_of!((*hdr).channels).read(),
                ptr::addr_of!((*hdr).buffer_frames).read(),
                ptr::addr_of!((*hdr).is_capture).read(),
            )
        };
        if ring_magic != LASAPI_RING_MAGIC
            || ring_ch == 0
            || ring_ch as usize > LASAPI_MAX_CHANNELS
            || ring_buf_frames == 0
            || ring_buf_frames as usize > LASAPI_RING_FRAMES
        {
            return Err(E_FAIL);
        }
        let sb_frames = if expected_buffer_frames != 0 {
            expected_buffer_frames.min(ring_buf_frames)
        } else {
            ring_buf_frames
        };
        let sb_ch = if expected_channels != 0 {
            expected_channels.min(ring_ch)
        } else {
            ring_ch
        };
        if sb_frames == 0 || sb_ch == 0 {
            return Err(E_FAIL);
        }
        Ok((
            Self {
                mem: RingMem { map },
                channels: sb_ch as usize,
                buffer_frames: sb_frames,
                is_capture: is_capture_raw != 0,
            },
            ring_ch,
            ring_buf_frames,
        ))
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn buffer_frames(&self) -> u32 {
        self.buffer_frames
    }

    pub fn is_capture(&self) -> bool {
        self.is_capture
    }

    pub fn period_frames(&self) -> u32 {
        self.mem.period_frames_atomic().load(Ordering::Acquire)
    }

    pub fn current_padding(&self) -> u32 {
        let w = self.mem.write_frame().load(Ordering::Relaxed);
        let rd = self.mem.read_frame().load(Ordering::Acquire);
        w.saturating_sub(rd) as u32
    }

    pub fn is_active(&self) -> bool {
        self.mem.active().load(Ordering::Acquire) != 0
    }

    pub fn start(&self) {
        self.mem.active().store(1, Ordering::Release);
    }

    pub fn stop(&self) {
        self.mem.active().store(0, Ordering::Release);
    }

    pub fn stop_seqcst(&self) {
        self.mem.active().store(0, Ordering::SeqCst);
    }

    pub fn reset(&self) -> (bool, u64) {
        if self.is_capture {
            let cw = self.mem.cap_write_frame().load(Ordering::Acquire);
            self.mem.cap_read_frame().store(cw, Ordering::Release);
            (true, cw)
        } else {
            let rd = self.mem.read_frame().load(Ordering::Acquire);
            self.mem.write_frame().store(rd, Ordering::Release);
            (false, rd)
        }
    }

    pub fn write_playback_f32(&self, src: &[f32], frames: u32, silent: bool) {
        let ch = self.channels;
        let total_frames = frames as usize;
        let w = self.mem.write_frame().load(Ordering::Relaxed);
        if total_frames > 0 && ch > 0 {
            let start = (w as usize) & LASAPI_RING_MASK;
            let first = total_frames.min(LASAPI_RING_FRAMES - start);
            let second = total_frames - first;
            let pcm_base = self.mem.pcm_base_ptr();
            if silent {
                unsafe {
                    ptr::write_bytes(pcm_base.add(start * ch), 0, first * ch * mem::size_of::<f32>());
                    if second > 0 {
                        ptr::write_bytes(pcm_base, 0, second * ch * mem::size_of::<f32>());
                    }
                }
            } else if let Some(need) = total_frames.checked_mul(ch) {
                if src.len() >= need {
                    unsafe {
                        ptr::copy_nonoverlapping(src.as_ptr(), pcm_base.add(start * ch), first * ch);
                        if second > 0 {
                            ptr::copy_nonoverlapping(src.as_ptr().add(first * ch), pcm_base, second * ch);
                        }
                    }
                }
            }
        }
        self.mem
            .write_frame()
            .store(w + u64::from(frames), Ordering::Release);
    }

    pub fn next_capture_packet_frames(&self) -> u32 {
        let cw = self.mem.cap_write_frame().load(Ordering::Acquire);
        let cr = self.mem.cap_read_frame().load(Ordering::Relaxed);
        cw.saturating_sub(cr) as u32
    }

    /// Copies up to `max_frames` interleaved `f32` capture frames (scaled by session volume/mute,
    /// clamped to `[-1.0, 1.0]`, and NaN-scrubbed) into `out_interleaved`. Returns frames copied.
    pub fn read_capture_f32(&self, max_frames: u32, out_interleaved: &mut [f32]) -> u32 {
        let ch = self.channels.min(LASAPI_MAX_CHANNELS);
        if ch == 0 || max_frames == 0 {
            return 0;
        }
        let max_by_buf = (out_interleaved.len() / ch) as u32;
        let req_frames = max_frames.min(max_by_buf);
        if req_frames == 0 {
            return 0;
        }

        let cw = self.mem.cap_write_frame().load(Ordering::Acquire);
        let mut cr = self.mem.cap_read_frame().load(Ordering::Relaxed);
        let mut avail = cw.saturating_sub(cr);
        if avail > LASAPI_RING_FRAMES as u64 {
            cr = cw - (LASAPI_RING_FRAMES as u64 / 2);
            avail = cw - cr;
        }
        let to_copy = avail.min(u64::from(req_frames)) as u32;
        if to_copy == 0 {
            return 0;
        }

        let vol = if self.mem.muted().load(Ordering::Relaxed) != 0 {
            0.0f32
        } else {
            (self.mem.volume_milli().load(Ordering::Relaxed) as f32) / 1000.0
        };
        let cap_base = self.mem.cap_pcm_base_ptr() as *const f32;
        for f in 0..to_copy as usize {
            let idx = (((cr + f as u64) as usize) & LASAPI_RING_MASK) * 2;
            for ch_i in 0..ch {
                let raw = unsafe { cap_base.add(idx + (ch_i & 1)).read() } * vol;
                let sample = if raw.is_finite() {
                    raw.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                out_interleaved[f * ch + ch_i] = sample;
            }
        }
        self.mem
            .cap_read_frame()
            .store(cr + u64::from(to_copy), Ordering::Release);
        to_copy
    }

    pub fn set_master_volume(&self, level: f32) {
        let lvl = level.clamp(0.0, 2.0);
        self.mem
            .volume_milli()
            .store((lvl * 1000.0) as u32, Ordering::Relaxed);
    }

    pub fn master_volume(&self) -> f32 {
        (self.mem.volume_milli().load(Ordering::Relaxed) as f32) / 1000.0
    }

    pub fn read_midi_events(&self, out: &mut [LasapiMidiEvent]) -> u32 {
        if out.is_empty() {
            return 0;
        }
        let w = self.mem.midi_write_idx().load(Ordering::Acquire);
        let mut rd = self.mem.midi_read_idx().load(Ordering::Relaxed);
        if rd > w {
            rd = w;
        } else if w - rd > LASAPI_MIDI_RING_EVENTS as u64 {
            rd = w - LASAPI_MIDI_RING_EVENTS as u64;
        }

        let mut count = 0usize;
        while rd < w && count < out.len() {
            let ev = unsafe { self.mem.midi_ring_slot_ptr(rd as usize).read() };
            out[count] = ev;
            count += 1;
            rd += 1;
        }
        if count > 0 || rd != self.mem.midi_read_idx().load(Ordering::Relaxed) {
            self.mem.midi_read_idx().store(rd, Ordering::Release);
        }
        count as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_abi::AUDCLNT_SHAREMODE_SHARED;

    fn test_spec(is_capture: bool, sample_rate: u32, channels: u32) -> SessionRingSpec {
        let mut client_name = [0u8; 64];
        copy_str_to_c(&mut client_name, "WaveRtTest");
        SessionRingSpec {
            session_id: 7,
            client_pid: 4242,
            client_name,
            share_mode: AUDCLNT_SHAREMODE_SHARED,
            stream_flags: if is_capture { 0x20000 } else { 0 },
            is_capture,
            sample_rate,
            channels,
            format: LASAPI_FMT_FLOAT32,
            buffer_frames: 2048,
            period_frames: 256,
        }
    }

    #[test]
    fn sealed_memfd_attach_rejects_corrupt_header_and_supports_atomic_period_updates() {
        let (eng, cli, fd) =
            WaveRtEngineRing::new_memfd_pair(test_spec(false, 48_000, 2)).expect("memfd pair");
        assert_eq!(cli.buffer_frames(), 2048);
        assert_eq!(cli.channels(), 2);
        assert_eq!(cli.period_frames(), 256);

        // Atomic period_frames update during Master Clock rate switch propagates cleanly.
        eng.set_period_frames(512);
        assert_eq!(cli.period_frames(), 512);
        assert_eq!(eng.telemetry().period_frames, 512);

        // Tampering with header bytes cannot corrupt client/daemon cached geometry, and corrupt magic fails attach.
        let base = eng.mem.base();
        unsafe { ptr::addr_of_mut!((*base).magic).write(0xDEADBEEF) };
        assert!(WaveRtClientRing::attach_fd(fd.as_raw_fd(), 2048, 2).is_err());
        assert_eq!(cli.channels(), 2);
        assert_eq!(eng.telemetry().buffer_frames, 2048);
    }

    #[test]
    fn playback_consume_resample_fade_nan_filter_and_recovery_work() {
        let (eng, cli, _fd) =
            WaveRtEngineRing::new_memfd_pair(test_spec(false, 48_000, 2)).expect("memfd pair");
        cli.start();
        eng.set_volume_and_mute(500, 0);

        let mut src = Vec::with_capacity(256 * 2);
        for _ in 0..256 {
            src.push(0.8f32);
            src.push(-0.4f32);
        }
        cli.write_playback_f32(&src, 256, false);
        assert_eq!(cli.current_padding(), 256);

        let mut rstate = SessionResampleState::new();
        let mut mix = vec![0.0f32; 128 * 2];
        eng.consume_playback(&mut rstate, &mut mix, 128, 2, 48_000, true, 1_000_000_000);
        assert!(rstate.stream_live);
        assert_eq!(cli.current_padding(), 128);
        assert!((mix[0] - 0.4).abs() < 1e-5);
        assert!((mix[1] - (-0.2)).abs() < 1e-5);

        // Partial tail softens last sample smoothly to ~0 when live.
        let (eng2, cli2, _fd2) =
            WaveRtEngineRing::new_memfd_pair(test_spec(false, 48_000, 2)).expect("pair2");
        cli2.start();
        let ones = vec![0.5f32; 64 * 2];
        cli2.write_playback_f32(&ones, 64, false);
        let mut r2 = SessionResampleState {
            phase: 0.0,
            stream_live: true,
            underrun_pending: false,
            last_audio_ns: 999_000_000,
        };
        let mut big = vec![0.0f32; 256 * 2];
        eng2.consume_playback(&mut r2, &mut big, 256, 2, 48_000, true, 1_000_000_000);
        assert!((big[0] - 0.5).abs() < 1e-5);
        assert!(big[63 * 2].abs() < 1e-5);

        // NaN/Inf scrubbed and read_frame > write_frame snaps back safely; resampler + xrun recovery work.
        let (eng3, cli3, _fd3) =
            WaveRtEngineRing::new_memfd_pair(test_spec(false, 44_100, 2)).expect("pair3");
        cli3.start();
        cli3.write_playback_f32(&[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.25], 2, false);
        let mut r3 = SessionResampleState::new();
        let mut out3 = vec![0.0f32; 8];
        eng3.consume_playback(&mut r3, &mut out3, 2, 2, 48_000, true, 1_000_000_000);
        assert!(out3.iter().all(|v| v.is_finite()));

        cli3.reset();
        r3.stream_live = true;
        eng3.consume_playback(&mut r3, &mut out3, 4, 2, 48_000, true, 1_010_000_000);
        assert!(r3.underrun_pending);
        cli3.write_playback_f32(&vec![0.25f32; 40 * 2], 40, false);
        let rep = eng3.consume_playback(&mut r3, &mut out3, 4, 2, 48_000, true, 1_020_000_000);
        assert_eq!(rep.post_xruns, 1);
    }

    #[test]
    fn capture_stereo_and_midi_ring_transfer_roundtrip() {
        let (eng, cli, _fd) =
            WaveRtEngineRing::new_memfd_pair(test_spec(true, 48_000, 2)).expect("memfd pair");
        eng.push_capture_stereo(&[0.5, -0.5, 2.0, f32::NAN], 2);
        assert_eq!(cli.next_capture_packet_frames(), 2);

        let mut dst = [0.0f32; 4];
        let got = cli.read_capture_f32(2, &mut dst);
        assert_eq!(got, 2);
        assert!((dst[0] - 0.5).abs() < 1e-6);
        assert!((dst[1] - (-0.5)).abs() < 1e-6);
        assert!((dst[2] - 1.0).abs() < 1e-6);
        assert_eq!(dst[3], 0.0);

        let ev = LasapiMidiEvent {
            frame_offset: 12,
            size: 3,
            port: 0,
            data: [0x90, 64, 127, 0],
            timestamp_ns: 999,
        };
        assert!(eng.push_midi(&ev));
        let mut rx = [LasapiMidiEvent::default(); 4];
        assert_eq!(cli.read_midi_events(&mut rx), 1);
        assert_eq!(rx[0], ev);
    }
}
