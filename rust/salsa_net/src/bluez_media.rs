// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![deny(unsafe_code)]

use core::sync::atomic::{AtomicU64, Ordering};

pub const BT_TX_RING_FRAMES: usize = 16_384;
pub const BT_TX_RING_MASK: usize = BT_TX_RING_FRAMES - 1;
pub const BT_TX_TARGET_MARGIN_MS: u64 = 45;
pub const CAP_RING_FRAMES: usize = 32_768;
pub const CAP_RING_MASK: usize = CAP_RING_FRAMES - 1;

/// Returns the target TX ring jitter headroom in engine input frames.
#[inline]
pub fn bt_tx_margin_frames(in_rate_raw: u32, packet_in_frames: u64) -> u64 {
    let rate = if in_rate_raw == 0 {
        48_000
    } else {
        in_rate_raw
    };
    let time_margin = (u64::from(rate) * BT_TX_TARGET_MARGIN_MS) / 1000;
    time_margin
        .max(packet_in_frames.saturating_mul(2))
        .min((BT_TX_RING_FRAMES as u64) / 2)
}

#[inline]
pub fn float_to_s16_clamped(x: f32) -> i16 {
    let c = if x.is_finite() {
        x.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    (c * 32767.0) as i16
}

pub fn write_tx_period(
    ring: &mut [f32],
    wr_frame: &AtomicU64,
    tx_paused: bool,
    interleaved: &[f32],
    frames: usize,
    channels: usize,
) -> i32 {
    if tx_paused {
        return 0;
    }
    if frames == 0 || channels == 0 || ring.len() < BT_TX_RING_FRAMES * 2 {
        return -1;
    }
    let Some(need) = frames.checked_mul(channels) else {
        return -1;
    };
    if interleaved.len() < need {
        return -1;
    }

    let wr = wr_frame.load(Ordering::Relaxed);
    for i in 0..frames {
        let idx = ((wr.wrapping_add(i as u64)) as usize) & BT_TX_RING_MASK;
        let base = i * channels;
        let l = interleaved[base];
        let r = if channels >= 2 {
            interleaved[base + 1]
        } else {
            l
        };
        ring[idx * 2] = l;
        ring[idx * 2 + 1] = r;
    }
    wr_frame.store(wr.wrapping_add(frames as u64), Ordering::Release);
    0
}

#[allow(clippy::too_many_arguments)]
pub fn pop_bt_resampled_s16(
    ring: &[f32],
    wr_frame: &AtomicU64,
    rd_frame: &AtomicU64,
    in_rate_raw: u32,
    out_rate_raw: u32,
    muted: bool,
    vol_pct: u32,
    avrcp_supported: bool,
    out_s16: &mut [i16],
    needed_frames: usize,
    bt_ch: u32,
    io_frac: &mut f64,
    drift_scale: f64,
) -> (usize, u32) {
    if ring.len() < BT_TX_RING_FRAMES * 2 || bt_ch == 0 {
        return (0, 0);
    }
    let ch = usize::from(bt_ch != 1) + 1; // 1 -> 1ch, >=2 -> 2ch
    let Some(out_needed_elems) = needed_frames.checked_mul(ch) else {
        return (0, 0);
    };
    if out_s16.len() < out_needed_elems {
        return (0, 0);
    }
    if needed_frames == 0 {
        let wr = wr_frame.load(Ordering::Acquire);
        let rd = rd_frame.load(Ordering::Relaxed);
        let rem = wr.saturating_sub(rd).min(u32::MAX as u64) as u32;
        return (0, rem);
    }

    let wr = wr_frame.load(Ordering::Acquire);
    let mut rd = rd_frame.load(Ordering::Relaxed);
    let mut avail = wr.saturating_sub(rd);

    let in_rate = if in_rate_raw == 0 { 48_000 } else { in_rate_raw };
    let out_rate = if out_rate_raw == 0 { 48_000 } else { out_rate_raw };
    let step = ((in_rate as f64) / (out_rate as f64)) * drift_scale;

    if avail > (BT_TX_RING_FRAMES as u64) - 64 {
        let need_est = (*io_frac + (needed_frames as f64) * step).ceil().max(0.0) as u64;
        let margin_in = bt_tx_margin_frames(in_rate, need_est);
        let keep = need_est
            .saturating_add(margin_in)
            .min((BT_TX_RING_FRAMES as u64) - 128);
        rd = wr.saturating_sub(keep);
        avail = keep;
        *io_frac = 0.0;
    }

    let sw_gain = if muted || vol_pct == 0 {
        0.0f32
    } else if !avrcp_supported || vol_pct > 100 {
        (vol_pct as f32) / 100.0f32
    } else {
        1.0f32
    };

    if avail == 0 {
        out_s16[..out_needed_elems].fill(0);
        return (0, 0);
    }

    let need_in_min = (*io_frac + (needed_frames as f64) * step).ceil().max(0.0) as u64;
    let underrun = avail < need_in_min;
    let mut valid_out = needed_frames;
    if underrun {
        let denom = if step > 0.0 { step } else { 1.0 };
        let est = ((avail as f64) - *io_frac) / denom;
        valid_out = if est > 0.0 {
            (est as usize).min(needed_frames)
        } else {
            0
        };
    }

    for i in 0..needed_frames {
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        if i < valid_out {
            let frac_int = if *io_frac >= 0.0 {
                *io_frac as u64
            } else {
                0
            };
            let f0 = rd.saturating_add(frac_int);
            if f0 + 1 < wr {
                let frac = *io_frac - io_frac.floor();
                let p0 = (f0 as usize) & BT_TX_RING_MASK;
                let p1 = ((f0 + 1) as usize) & BT_TX_RING_MASK;
                l = ((ring[p0 * 2] as f64) * (1.0 - frac) + (ring[p1 * 2] as f64) * frac) as f32;
                r = ((ring[p0 * 2 + 1] as f64) * (1.0 - frac)
                    + (ring[p1 * 2 + 1] as f64) * frac) as f32;
            } else if f0 < wr {
                let p0 = (f0 as usize) & BT_TX_RING_MASK;
                l = ring[p0 * 2];
                r = ring[p0 * 2 + 1];
            }
            if underrun && valid_out > 0 {
                let fade_len = valid_out.min(32);
                let fade_start = valid_out - fade_len;
                if i >= fade_start {
                    let g = ((valid_out - i) as f32) / (fade_len as f32);
                    l *= g;
                    r *= g;
                }
            }
            *io_frac += step;
        }
        l *= sw_gain;
        r *= sw_gain;
        if ch == 1 {
            out_s16[i] = float_to_s16_clamped(0.5 * (l + r));
        } else {
            out_s16[i * 2] = float_to_s16_clamped(l);
            out_s16[i * 2 + 1] = float_to_s16_clamped(r);
        }
    }

    if underrun {
        rd = wr;
        *io_frac = 0.0;
    } else {
        let consumed = if *io_frac >= 0.0 {
            io_frac.floor() as u64
        } else {
            0
        };
        if rd.saturating_add(consumed) <= wr {
            rd += consumed;
            *io_frac -= consumed as f64;
        } else {
            rd = wr;
            *io_frac = 0.0;
        }
    }

    rd_frame.store(rd, Ordering::Release);
    let rem = wr.saturating_sub(rd).min(u32::MAX as u64) as u32;
    (valid_out, rem)
}

pub fn sco_push_pcm_to_cap_ring(
    cap_ring: &mut [f32],
    wr_frame: &AtomicU64,
    dec_pcm: &[i16],
    sco_rate_raw: u32,
    eng_rate_raw: u32,
    resamp_pos: &mut f64,
    prev_s: &mut f32,
) {
    if dec_pcm.is_empty() || cap_ring.len() < CAP_RING_FRAMES * 2 {
        return;
    }
    let sco_rate = if sco_rate_raw == 0 { 8_000 } else { sco_rate_raw };
    let eng_rate = if eng_rate_raw == 0 { 48_000 } else { eng_rate_raw };
    let ratio = (eng_rate as f64) / (sco_rate as f64);
    let mut wr = wr_frame.load(Ordering::Relaxed);

    for &sample in dec_pcm {
        let cur_s = (sample as f32) / 32768.0f32;
        *resamp_pos += ratio;
        let out_n = if *resamp_pos >= 0.0 {
            *resamp_pos as u32
        } else {
            0
        };
        *resamp_pos -= f64::from(out_n);
        for k in 0..out_n {
            let t = if out_n > 1 {
                ((k + 1) as f32) / (out_n as f32)
            } else {
                1.0
            };
            let v = *prev_s + (cur_s - *prev_s) * t;
            let idx = (wr as usize) & CAP_RING_MASK;
            wr = wr.wrapping_add(1);
            cap_ring[idx * 2] = v;
            cap_ring[idx * 2 + 1] = v;
        }
        *prev_s = cur_s;
    }
    wr_frame.store(wr, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_ring_write_pop_and_underrun_fade() {
        let mut ring = vec![0.0f32; BT_TX_RING_FRAMES * 2];
        let wr = AtomicU64::new(0);
        let rd = AtomicU64::new(0);

        let mut in_buf = vec![0.0f32; 256 * 2];
        for i in 0..256 {
            in_buf[i * 2] = 0.5;
            in_buf[i * 2 + 1] = -0.5;
        }
        assert_eq!(write_tx_period(&mut ring, &wr, false, &in_buf, 256, 2), 0);
        assert_eq!(wr.load(Ordering::Acquire), 256);

        let mut out_s16 = vec![0i16; 128 * 2];
        let mut frac = 0.0;
        let (popped, rem) = pop_bt_resampled_s16(
            &ring,
            &wr,
            &rd,
            48_000,
            48_000,
            false,
            100,
            true,
            &mut out_s16,
            128,
            2,
            &mut frac,
            1.0,
        );
        assert_eq!(popped, 128);
        assert_eq!(rem, 128);
        assert!((out_s16[0] - 16_383).abs() <= 2);
        assert!((out_s16[1] + 16_383).abs() <= 2);

        // Underrun when requesting more frames than remain
        let mut out_under = vec![0i16; 256 * 2];
        let (pop2, rem2) = pop_bt_resampled_s16(
            &ring,
            &wr,
            &rd,
            48_000,
            48_000,
            false,
            100,
            true,
            &mut out_under,
            256,
            2,
            &mut frac,
            1.0,
        );
        assert!(pop2 <= 128);
        assert_eq!(rem2, 0);
        assert_eq!(out_under[255 * 2], 0);
    }

    #[test]
    fn sco_resampler_upsamples_msbc_cleanly() {
        let mut cap = vec![0.0f32; CAP_RING_FRAMES * 2];
        let wr = AtomicU64::new(0);
        let dec = [16_384i16; 120];
        let mut pos = 0.0;
        let mut prev = 0.0;
        sco_push_pcm_to_cap_ring(&mut cap, &wr, &dec, 16_000, 48_000, &mut pos, &mut prev);
        assert_eq!(wr.load(Ordering::Acquire), 360);
        assert!((cap[10] - 0.5).abs() < 0.001);
    }

    #[test]
    fn bt_tx_margin_scales_for_codec_and_engine_rate() {
        // 48kHz AAC packet (1024 frames = 21.3ms) gets 45ms (2160 frames), >= 2x packet.
        assert_eq!(bt_tx_margin_frames(48_000, 1024), 2160);
        // 96kHz LDAC packet (128 input frames) keeps 45ms (4320 frames).
        assert_eq!(bt_tx_margin_frames(96_000, 128), 4320);
        // 192kHz engine pushing 4096 input frames per AAC packet caps safely at half-ring.
        assert_eq!(bt_tx_margin_frames(192_000, 4096), 8192);
    }

    #[test]
    fn tx_ring_overflow_trim_preserves_target_margin() {
        let ring = vec![0.25f32; BT_TX_RING_FRAMES * 2];
        let wr = AtomicU64::new((BT_TX_RING_FRAMES - 16) as u64);
        let rd = AtomicU64::new(0);
        let mut out_s16 = vec![0i16; 1024 * 2];
        let target = bt_tx_margin_frames(48_000, 1024);

        let mut frac_zero = 0.0;
        let (popped0, rem0) = pop_bt_resampled_s16(
            &ring,
            &wr,
            &rd,
            48_000,
            48_000,
            false,
            100,
            true,
            &mut out_s16,
            1024,
            2,
            &mut frac_zero,
            1.0,
        );
        assert_eq!(popped0, 1024);
        assert_eq!(rem0 as u64, target);

        let rd_half = AtomicU64::new(0);
        let mut frac_half = 0.5;
        let (popped1, rem1) = pop_bt_resampled_s16(
            &ring,
            &wr,
            &rd_half,
            48_000,
            48_000,
            false,
            100,
            true,
            &mut out_s16,
            1024,
            2,
            &mut frac_half,
            1.0,
        );
        assert_eq!(popped1, 1024);
        assert!(rem1 as u64 >= target && rem1 as u64 <= target + 1);
    }
}
