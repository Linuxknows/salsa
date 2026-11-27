// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

//! Kernel ALSA `/dev/snd/*` hardware adapter seam.
//!
//! Quarantines Linux `<sound/asound.h>` UAPI ioctl structs, PCM device negotiation,
//! and real-time `WRITEI_FRAMES` / `PREPARE` XRUN recovery behind the safe
//! `AlsaPlaybackBackend` seam so `audiodg_engine` operates with `#![deny(unsafe_code)]`.

#![allow(unsafe_code)]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_int, c_long, c_uint, c_ulong, c_void};
use core::mem;
use core::ptr;
use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::sync::{Arc, Mutex};

use crate::c_abi::{
    copy_str_to_c, LASAPI_MAX_PERIOD, SALSA_PACK_F32_LE, SALSA_PACK_S16_LE, SALSA_PACK_S24_3LE,
    SALSA_PACK_S24_LE, SALSA_PACK_S32_LE,
};
use crate::dsp::pack_pcm_bytes;
use crate::unix_sys::{
    close_raw_fd, ioctl_raw, open_chrdev_rw_cloexec, ERRNO_EBADFD, ERRNO_EIO, ERRNO_ENODEV,
    ERRNO_EPIPE, ERRNO_ESTRPIPE,
};

// Linux <sound/asound.h> PCM kernel ABI constants & ioctls
pub(crate) const SNDRV_PCM_IOCTL_HW_REFINE: c_ulong = 0xc260_4110;
pub(crate) const SNDRV_PCM_IOCTL_HW_PARAMS: c_ulong = 0xc260_4111;
pub(crate) const SNDRV_PCM_IOCTL_SW_PARAMS: c_ulong = 0xc088_4113;
pub(crate) const SNDRV_PCM_IOCTL_PREPARE: c_ulong = 0x4140;
pub(crate) const SNDRV_PCM_IOCTL_START: c_ulong = 0x4142;
const SNDRV_PCM_IOCTL_WRITEI_FRAMES: c_ulong = 0x4018_4150;
pub(crate) const SNDRV_PCM_IOCTL_READI_FRAMES: c_ulong = 0x8018_4151;

pub(crate) const SNDRV_PCM_HW_PARAM_ACCESS: usize = 0;
pub(crate) const SNDRV_PCM_HW_PARAM_FORMAT: usize = 1;
pub(crate) const SNDRV_PCM_HW_PARAM_FIRST_MASK: usize = 0;
pub(crate) const SNDRV_PCM_HW_PARAM_LAST_MASK: usize = 2;
pub(crate) const SNDRV_PCM_HW_PARAM_FIRST_INTERVAL: usize = 8;
pub(crate) const SNDRV_PCM_HW_PARAM_LAST_INTERVAL: usize = 19;
pub(crate) const SNDRV_PCM_HW_PARAM_CHANNELS: usize = 10;
pub(crate) const SNDRV_PCM_HW_PARAM_RATE: usize = 11;
pub(crate) const SNDRV_PCM_HW_PARAM_PERIOD_SIZE: usize = 13;
pub(crate) const SNDRV_PCM_HW_PARAM_BUFFER_SIZE: usize = 17;

pub(crate) const SNDRV_PCM_ACCESS_RW_INTERLEAVED: u32 = 3;
pub(crate) const SNDRV_PCM_FORMAT_S16_LE: i32 = 2;
pub(crate) const SNDRV_PCM_FORMAT_S24_LE: i32 = 6;
pub(crate) const SNDRV_PCM_FORMAT_S32_LE: i32 = 10;
pub const SNDRV_PCM_FORMAT_FLOAT_LE: i32 = 14;
pub(crate) const SNDRV_PCM_FORMAT_S24_3LE: i32 = 32;
pub(crate) const SNDRV_PCM_TSTAMP_ENABLE: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndMask {
    pub(crate) bits: [u32; 8],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndInterval {
    pub(crate) min: u32,
    pub(crate) max: u32,
    pub(crate) flags: u32,
}

#[repr(C)]
pub(crate) struct SndPcmHwParams {
    pub(crate) flags: u32,
    pub(crate) masks: [SndMask; 8],
    pub(crate) intervals: [SndInterval; 21],
    pub(crate) rmask: u32,
    pub(crate) cmask: u32,
    pub(crate) info: u32,
    pub(crate) msbits: u32,
    pub(crate) rate_num: u32,
    pub(crate) rate_den: u32,
    pub(crate) fifo_size: u64,
    pub(crate) reserved: [u8; 64],
}

#[repr(C)]
pub(crate) struct SndPcmSwParams {
    pub(crate) tstamp_mode: c_int,
    pub(crate) period_step: c_uint,
    pub(crate) sleep_min: c_uint,
    pub(crate) pad0: c_uint,
    pub(crate) avail_min: u64,
    pub(crate) xfer_align: u64,
    pub(crate) start_threshold: u64,
    pub(crate) stop_threshold: u64,
    pub(crate) silence_threshold: u64,
    pub(crate) silence_size: u64,
    pub(crate) boundary: u64,
    pub(crate) proto: u32,
    pub(crate) tstamp_type: u32,
    pub(crate) reserved: [u8; 56],
}

#[repr(C)]
pub(crate) struct SndXferi {
    pub(crate) result: c_long,
    pub(crate) buf: *mut c_void,
    pub(crate) frames: c_ulong,
}

#[derive(Clone, Copy, Debug)]
pub struct KernelWaveRtHw {
    pub fd: RawFd,
    pub device_path: [u8; 64],
    pub is_virtual: bool,
    pub is_bluetooth: bool,
    pub sample_rate: u32,
    pub channels: u32,
    pub period_frames: u32,
    pub buffer_frames: u32,
    pub sndrv_format: i32,
    pub format_bits: u32,
}

impl Default for KernelWaveRtHw {
    fn default() -> Self {
        let mut device_path = [0u8; 64];
        copy_str_to_c(&mut device_path, "WaveRT-Virtual-Clock");
        Self {
            fd: -1,
            device_path,
            is_virtual: true,
            is_bluetooth: false,
            sample_rate: 48_000,
            channels: 2,
            period_frames: 256,
            buffer_frames: 1024,
            sndrv_format: SNDRV_PCM_FORMAT_FLOAT_LE,
            format_bits: 32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlsaWriteOutcome {
    Ok,
    RecoveredXrun { xerr: i32 },
    DeviceLost { dead_err: i32, xerr: Option<i32> },
    TransientErr { had_xrun: bool, xerr: i32 },
}

pub trait AlsaPlaybackBackend: Send + Sync {
    fn open_pcm(
        &self,
        dev_path: &str,
        req_rate: u32,
        req_period: u32,
        req_bits: u32,
    ) -> Option<KernelWaveRtHw>;

    fn write_packed_period(
        &self,
        hw: &KernelWaveRtHw,
        mix_f32: &[f32],
        frames: u32,
        ch: u32,
        master_gain: f32,
        softknee: bool,
        scratch_bytes: &mut [u8],
    ) -> AlsaWriteOutcome;

    fn close_pcm(&self, fd: RawFd);
}

#[derive(Default)]
pub struct KernelAlsaBackend;

impl KernelAlsaBackend {
    pub fn global() -> Arc<Self> {
        Arc::new(Self)
    }
}

fn pack_pcm_into_scratch(
    sndrv_format: i32,
    mix_f32: &[f32],
    samples: usize,
    master_gain: f32,
    softknee: bool,
    scratch_bytes: &mut [u8],
) -> Option<usize> {
    let (pack_fmt, need_bytes) = match sndrv_format {
        SNDRV_PCM_FORMAT_S32_LE => (SALSA_PACK_S32_LE, samples.checked_mul(4)?),
        SNDRV_PCM_FORMAT_S24_LE => (SALSA_PACK_S24_LE, samples.checked_mul(4)?),
        SNDRV_PCM_FORMAT_S24_3LE => (SALSA_PACK_S24_3LE, samples.checked_mul(3)?),
        SNDRV_PCM_FORMAT_S16_LE => (SALSA_PACK_S16_LE, samples.checked_mul(2)?),
        _ => (SALSA_PACK_F32_LE, samples.checked_mul(4)?),
    };
    if mix_f32.len() < samples || scratch_bytes.len() < need_bytes {
        return None;
    }
    pack_pcm_bytes(
        &mix_f32[..samples],
        master_gain,
        softknee,
        pack_fmt,
        &mut scratch_bytes[..need_bytes],
    );
    Some(need_bytes)
}

impl AlsaPlaybackBackend for KernelAlsaBackend {
    fn open_pcm(
        &self,
        dev_path: &str,
        req_rate: u32,
        req_period: u32,
        req_bits: u32,
    ) -> Option<KernelWaveRtHw> {
        open_kernel_pcm_device(dev_path, req_rate, req_period, req_bits)
    }

    fn write_packed_period(
        &self,
        hw: &KernelWaveRtHw,
        mix_f32: &[f32],
        frames: u32,
        ch: u32,
        master_gain: f32,
        softknee: bool,
        scratch_bytes: &mut [u8],
    ) -> AlsaWriteOutcome {
        let Some(samples) = (frames as usize).checked_mul(ch as usize) else {
            return AlsaWriteOutcome::TransientErr {
                had_xrun: false,
                xerr: 22,
            };
        };
        if pack_pcm_into_scratch(
            hw.sndrv_format,
            mix_f32,
            samples,
            master_gain,
            softknee,
            scratch_bytes,
        )
        .is_none()
        {
            return AlsaWriteOutcome::TransientErr {
                had_xrun: false,
                xerr: 22,
            };
        }

        let mut xfer = SndXferi {
            result: 0,
            buf: scratch_bytes.as_mut_ptr().cast::<c_void>(),
            frames: frames as c_ulong,
        };
        let mut had_xrun = false;
        let mut xrun_err = None;
        let mut wr = unsafe {
            ioctl_raw(
                hw.fd,
                SNDRV_PCM_IOCTL_WRITEI_FRAMES,
                (&mut xfer as *mut SndXferi).cast::<c_void>(),
            )
        };
        if wr < 0 {
            let xerr = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if xerr == ERRNO_EPIPE || xerr == ERRNO_ESTRPIPE {
                had_xrun = true;
                xrun_err = Some(xerr);
                unsafe {
                    let _ = ioctl_raw(hw.fd, SNDRV_PCM_IOCTL_PREPARE, ptr::null_mut());
                }
                xfer.result = 0;
                wr = unsafe {
                    ioctl_raw(
                        hw.fd,
                        SNDRV_PCM_IOCTL_WRITEI_FRAMES,
                        (&mut xfer as *mut SndXferi).cast::<c_void>(),
                    )
                };
            } else if xerr == ERRNO_ENODEV || xerr == ERRNO_EBADFD || xerr == ERRNO_EIO {
                return AlsaWriteOutcome::DeviceLost {
                    dead_err: xerr,
                    xerr: None,
                };
            } else {
                return AlsaWriteOutcome::TransientErr {
                    had_xrun: false,
                    xerr,
                };
            }
        }

        if wr < 0 {
            let dead_err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if dead_err == ERRNO_ENODEV || dead_err == ERRNO_EBADFD || dead_err == ERRNO_EIO {
                AlsaWriteOutcome::DeviceLost {
                    dead_err,
                    xerr: xrun_err,
                }
            } else {
                AlsaWriteOutcome::TransientErr {
                    had_xrun,
                    xerr: dead_err,
                }
            }
        } else if had_xrun {
            AlsaWriteOutcome::RecoveredXrun {
                xerr: xrun_err.unwrap_or(ERRNO_EPIPE),
            }
        } else {
            AlsaWriteOutcome::Ok
        }
    }

    fn close_pcm(&self, fd: RawFd) {
        if fd >= 0 {
            close_raw_fd(fd);
        }
    }
}

/// Deterministic in-memory test adapter for [`AlsaPlaybackBackend`].
#[derive(Default)]
pub struct InMemoryAlsaBackend {
    inner: Mutex<InMemoryAlsaState>,
}

#[derive(Default)]
struct InMemoryAlsaState {
    next_fd: i32,
    opened_paths: Vec<String>,
    closed_fds: Vec<RawFd>,
    written_packets: Vec<Vec<u8>>,
    faults: VecDeque<AlsaWriteOutcome>,
}

impl InMemoryAlsaBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inject_fault(&self, fault: AlsaWriteOutcome) {
        self.inner.lock().unwrap().faults.push_back(fault);
    }

    pub fn written_packet_count(&self) -> usize {
        self.inner.lock().unwrap().written_packets.len()
    }

    pub fn opened_paths(&self) -> Vec<String> {
        self.inner.lock().unwrap().opened_paths.clone()
    }

    pub fn closed_fds(&self) -> Vec<RawFd> {
        self.inner.lock().unwrap().closed_fds.clone()
    }
}

impl AlsaPlaybackBackend for InMemoryAlsaBackend {
    fn open_pcm(
        &self,
        dev_path: &str,
        req_rate: u32,
        req_period: u32,
        req_bits: u32,
    ) -> Option<KernelWaveRtHw> {
        if !is_valid_alsa_pcm_path(dev_path) {
            return None;
        }
        let mut st = self.inner.lock().unwrap();
        st.next_fd += 1;
        let fd = 1000 + st.next_fd;
        st.opened_paths.push(dev_path.to_string());
        let mut device_path = [0u8; 64];
        copy_str_to_c(&mut device_path, dev_path);
        let (sndrv_format, format_bits) = match req_bits {
            16 => (SNDRV_PCM_FORMAT_S16_LE, 16),
            24 => (SNDRV_PCM_FORMAT_S24_3LE, 24),
            _ => (SNDRV_PCM_FORMAT_S32_LE, 32),
        };
        Some(KernelWaveRtHw {
            fd,
            device_path,
            is_virtual: false,
            is_bluetooth: false,
            sample_rate: req_rate.max(44_100),
            channels: 2,
            period_frames: req_period.clamp(64, LASAPI_MAX_PERIOD as u32),
            buffer_frames: req_period.clamp(64, LASAPI_MAX_PERIOD as u32) * 4,
            sndrv_format,
            format_bits,
        })
    }

    fn write_packed_period(
        &self,
        hw: &KernelWaveRtHw,
        mix_f32: &[f32],
        frames: u32,
        ch: u32,
        master_gain: f32,
        softknee: bool,
        scratch_bytes: &mut [u8],
    ) -> AlsaWriteOutcome {
        let Some(samples) = (frames as usize).checked_mul(ch as usize) else {
            return AlsaWriteOutcome::TransientErr {
                had_xrun: false,
                xerr: 22,
            };
        };
        let Some(need) = pack_pcm_into_scratch(
            hw.sndrv_format,
            mix_f32,
            samples,
            master_gain,
            softknee,
            scratch_bytes,
        ) else {
            return AlsaWriteOutcome::TransientErr {
                had_xrun: false,
                xerr: 22,
            };
        };
        let mut st = self.inner.lock().unwrap();
        if let Some(fault) = st.faults.pop_front() {
            if matches!(fault, AlsaWriteOutcome::Ok | AlsaWriteOutcome::RecoveredXrun { .. }) {
                st.written_packets.push(scratch_bytes[..need].to_vec());
            }
            return fault;
        }
        st.written_packets.push(scratch_bytes[..need].to_vec());
        AlsaWriteOutcome::Ok
    }

    fn close_pcm(&self, fd: RawFd) {
        if fd >= 0 {
            self.inner.lock().unwrap().closed_fds.push(fd);
        }
    }
}

pub(crate) fn hw_params_any(params: &mut SndPcmHwParams) {
    unsafe { ptr::write_bytes(params, 0, 1) };
    let mask_cnt = SNDRV_PCM_HW_PARAM_LAST_MASK - SNDRV_PCM_HW_PARAM_FIRST_MASK + 1;
    for m in params.masks.iter_mut().take(mask_cnt) {
        m.bits.fill(u32::MAX);
    }
    let int_cnt = SNDRV_PCM_HW_PARAM_LAST_INTERVAL - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL + 1;
    for iv in params.intervals.iter_mut().take(int_cnt) {
        iv.min = 0;
        iv.max = u32::MAX;
    }
    params.rmask = u32::MAX;
}

pub(crate) fn hw_param_set_mask(params: &mut SndPcmHwParams, param: usize, bit: u32) {
    let idx = param - SNDRV_PCM_HW_PARAM_FIRST_MASK;
    params.masks[idx].bits.fill(0);
    params.masks[idx].bits[(bit >> 5) as usize] |= 1u32 << (bit & 31);
    params.cmask |= 1u32 << (param as u32);
}

pub(crate) fn hw_param_set_minmax(params: &mut SndPcmHwParams, param: usize, minv: u32, maxv: u32) {
    let idx = param - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL;
    params.intervals[idx].min = minv;
    params.intervals[idx].max = maxv;
    params.intervals[idx].flags = 0;
    params.cmask |= 1u32 << (param as u32);
}

pub fn is_valid_alsa_pcm_path(p: &str) -> bool {
    let Some(rest) = p.strip_prefix("/dev/snd/pcmC") else {
        return false;
    };
    let mut chars = rest.chars().peekable();
    let mut card_digits = 0usize;
    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
        chars.next();
        card_digits += 1;
    }
    if card_digits == 0 || chars.next() != Some('D') {
        return false;
    }
    let mut dev_digits = 0usize;
    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
        chars.next();
        dev_digits += 1;
    }
    dev_digits > 0 && matches!(chars.next(), Some('p' | 'c')) && chars.next().is_none()
}

pub fn open_kernel_pcm_device(
    dev_path: &str,
    req_rate: u32,
    req_period: u32,
    req_bits: u32,
) -> Option<KernelWaveRtHw> {
    if !is_valid_alsa_pcm_path(dev_path) {
        return None;
    }
    let fd = open_chrdev_rw_cloexec(dev_path);
    if fd < 0 {
        return None;
    }

    let (candidate_formats, candidate_bits): ([i32; 5], [u32; 5]) = match req_bits {
        16 => (
            [
                SNDRV_PCM_FORMAT_S16_LE,
                SNDRV_PCM_FORMAT_S24_3LE,
                SNDRV_PCM_FORMAT_S24_LE,
                SNDRV_PCM_FORMAT_S32_LE,
                SNDRV_PCM_FORMAT_FLOAT_LE,
            ],
            [16, 24, 24, 32, 32],
        ),
        24 => (
            [
                SNDRV_PCM_FORMAT_S24_3LE,
                SNDRV_PCM_FORMAT_S24_LE,
                SNDRV_PCM_FORMAT_S32_LE,
                SNDRV_PCM_FORMAT_FLOAT_LE,
                SNDRV_PCM_FORMAT_S16_LE,
            ],
            [24, 24, 32, 32, 16],
        ),
        _ => (
            [
                SNDRV_PCM_FORMAT_S32_LE,
                SNDRV_PCM_FORMAT_FLOAT_LE,
                SNDRV_PCM_FORMAT_S24_3LE,
                SNDRV_PCM_FORMAT_S24_LE,
                SNDRV_PCM_FORMAT_S16_LE,
            ],
            [32, 32, 24, 24, 16],
        ),
    };

    let candidate_chs = [2u32, 1u32];
    let mut candidate_rates = [req_rate, 0u32, 48_000, 44_100];
    if req_rate == 192_000 {
        candidate_rates[1] = 96_000;
    } else if req_rate == 176_400 {
        candidate_rates[1] = 88_200;
        candidate_rates[2] = 44_100;
        candidate_rates[3] = 48_000;
    } else if req_rate == 88_200 || req_rate == 44_100 {
        candidate_rates[2] = 44_100;
        candidate_rates[3] = 48_000;
    }

    let mut params: SndPcmHwParams = unsafe { mem::zeroed() };
    let mut chosen_fmt = -1i32;
    let mut chosen_bits = 0u32;

    'outer: for &bm in &[4u32, 2u32] {
        for (r_idx, &cand_r) in candidate_rates.iter().enumerate() {
            if cand_r < 44_100 {
                continue;
            }
            if r_idx > 0 && cand_r == req_rate {
                continue;
            }
            for &ch in &candidate_chs {
                for (&fmt, &bits) in candidate_formats.iter().zip(candidate_bits.iter()) {
                    hw_params_any(&mut params);
                    if unsafe {
                        ioctl_raw(
                            fd,
                            SNDRV_PCM_IOCTL_HW_REFINE,
                            (&mut params as *mut SndPcmHwParams).cast::<c_void>(),
                        )
                    } < 0
                    {
                        continue;
                    }
                    hw_param_set_mask(
                        &mut params,
                        SNDRV_PCM_HW_PARAM_ACCESS,
                        SNDRV_PCM_ACCESS_RW_INTERLEAVED,
                    );
                    hw_param_set_mask(&mut params, SNDRV_PCM_HW_PARAM_FORMAT, fmt as u32);
                    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_CHANNELS, ch, ch);
                    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_RATE, cand_r, cand_r);
                    let max_p = LASAPI_MAX_PERIOD as u32;
                    let min_per = req_period.clamp(64, max_p);
                    let max_per = req_period.saturating_mul(2).clamp(min_per, max_p);
                    hw_param_set_minmax(
                        &mut params,
                        SNDRV_PCM_HW_PARAM_PERIOD_SIZE,
                        min_per,
                        max_per,
                    );
                    hw_param_set_minmax(
                        &mut params,
                        SNDRV_PCM_HW_PARAM_BUFFER_SIZE,
                        min_per.saturating_mul(bm),
                        min_per.saturating_mul(8),
                    );
                    if unsafe {
                        ioctl_raw(
                            fd,
                            SNDRV_PCM_IOCTL_HW_PARAMS,
                            (&mut params as *mut SndPcmHwParams).cast::<c_void>(),
                        )
                    } == 0
                    {
                        chosen_fmt = fmt;
                        chosen_bits = bits;
                        break 'outer;
                    }
                }
            }
        }
    }

    if chosen_fmt < 0 {
        close_raw_fd(fd);
        return None;
    }

    let actual_rate =
        params.intervals[SNDRV_PCM_HW_PARAM_RATE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    let actual_ch =
        params.intervals[SNDRV_PCM_HW_PARAM_CHANNELS - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    let actual_per =
        params.intervals[SNDRV_PCM_HW_PARAM_PERIOD_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    let actual_buf =
        params.intervals[SNDRV_PCM_HW_PARAM_BUFFER_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;

    let base_per = if actual_per != 0 { actual_per } else { req_period };
    let mut sw: SndPcmSwParams = unsafe { mem::zeroed() };
    sw.tstamp_mode = SNDRV_PCM_TSTAMP_ENABLE;
    sw.period_step = 1;
    sw.avail_min = base_per as u64;
    sw.start_threshold = if actual_buf >= base_per.saturating_mul(3) {
        (base_per.saturating_mul(2)) as u64
    } else {
        base_per as u64
    };
    sw.stop_threshold = if actual_buf != 0 {
        actual_buf as u64
    } else {
        base_per.saturating_mul(4) as u64
    };
    sw.xfer_align = 1;
    unsafe {
        let _ = ioctl_raw(
            fd,
            SNDRV_PCM_IOCTL_SW_PARAMS,
            (&mut sw as *mut SndPcmSwParams).cast::<c_void>(),
        );
        if ioctl_raw(fd, SNDRV_PCM_IOCTL_PREPARE, ptr::null_mut()) < 0 {
            close_raw_fd(fd);
            return None;
        }
    }

    let mut device_path = [0u8; 64];
    copy_str_to_c(&mut device_path, dev_path);
    Some(KernelWaveRtHw {
        fd,
        device_path,
        is_virtual: false,
        is_bluetooth: false,
        sample_rate: if actual_rate != 0 { actual_rate } else { req_rate },
        channels: if actual_ch != 0 { actual_ch } else { 2 },
        period_frames: base_per,
        buffer_frames: if actual_buf != 0 {
            actual_buf
        } else {
            req_period.saturating_mul(4)
        },
        sndrv_format: chosen_fmt,
        format_bits: chosen_bits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn alsa_pcm_uapi_layouts_match_kernel() {
        assert_eq!(size_of::<SndMask>(), 32);
        assert_eq!(size_of::<SndInterval>(), 12);
        assert_eq!(size_of::<SndPcmHwParams>(), 608);
        assert_eq!(offset_of!(SndPcmHwParams, masks), 4);
        assert_eq!(offset_of!(SndPcmHwParams, intervals), 260);
        assert_eq!(offset_of!(SndPcmHwParams, rmask), 512);
        assert_eq!(offset_of!(SndPcmHwParams, fifo_size), 536);

        assert_eq!(size_of::<SndPcmSwParams>(), 136);
        assert_eq!(offset_of!(SndPcmSwParams, avail_min), 16);
        assert_eq!(offset_of!(SndPcmSwParams, xfer_align), 24);
        assert_eq!(offset_of!(SndPcmSwParams, start_threshold), 32);
        assert_eq!(offset_of!(SndPcmSwParams, stop_threshold), 40);
        assert_eq!(offset_of!(SndPcmSwParams, silence_size), 56);
        assert_eq!(offset_of!(SndPcmSwParams, proto), 72);

        assert_eq!(size_of::<SndXferi>(), 24);
        assert_eq!(offset_of!(SndXferi, buf), 8);
        assert_eq!(offset_of!(SndXferi, frames), 16);
    }

    #[test]
    fn in_memory_alsa_backend_handles_open_pack_xrun_and_device_lost() {
        let alsa = InMemoryAlsaBackend::new();
        assert!(alsa.open_pcm("/tmp/evil", 48_000, 256, 32).is_none());
        let hw = alsa.open_pcm("/dev/snd/pcmC0D0p", 96_000, 256, 24).unwrap();
        assert_eq!(hw.sample_rate, 96_000);
        assert_eq!(hw.format_bits, 24);

        let mix = vec![0.25f32; 256 * 2];
        let mut scratch = vec![0u8; 256 * 2 * 4];
        assert_eq!(
            alsa.write_packed_period(&hw, &mix, 256, 2, 1.0, false, &mut scratch),
            AlsaWriteOutcome::Ok
        );
        assert_eq!(alsa.written_packet_count(), 1);

        alsa.inject_fault(AlsaWriteOutcome::RecoveredXrun { xerr: ERRNO_EPIPE });
        assert_eq!(
            alsa.write_packed_period(&hw, &mix, 256, 2, 1.0, false, &mut scratch),
            AlsaWriteOutcome::RecoveredXrun { xerr: ERRNO_EPIPE }
        );
        assert_eq!(alsa.written_packet_count(), 2);

        alsa.inject_fault(AlsaWriteOutcome::DeviceLost {
            dead_err: ERRNO_ENODEV,
            xerr: None,
        });
        assert_eq!(
            alsa.write_packed_period(&hw, &mix, 256, 2, 1.0, false, &mut scratch),
            AlsaWriteOutcome::DeviceLost {
                dead_err: ERRNO_ENODEV,
                xerr: None
            }
        );

        alsa.close_pcm(hw.fd);
        assert_eq!(alsa.closed_fds(), vec![hw.fd]);
    }
}
