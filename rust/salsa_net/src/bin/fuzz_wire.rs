// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(clippy::too_many_lines)]

use core::ffi::c_char;
use core::mem;
use core::sync::atomic::AtomicU64;
use std::env;

use salsa_wire::bluez_media::{pop_bt_resampled_s16, sco_push_pcm_to_cap_ring, write_tx_period};
use salsa_wire::bluez_wire::{
    abr_step_aac, abr_step_ldac, extract_mac_from_id, hfp_make_batt_path, ldac_eqmid_to_kbps,
    msbc_h2_byte, msbc_rx_strip_offset, pack_s16_to_s24le, parse_bdaddr, parse_hfp_at_line,
    select_a2dp_config, write_rtp_header, write_rtp_sbc_payload_hdr,
};
use salsa_wire::c_abi::{
    LasapiDbgDgram, LasapiHwAnalogCfg, LasapiMicApoCfg, LasapiParamEqExt, SalsaMicCubicState,
    LASAPI_DEBUG_DGRAM_MAGIC,
};
use salsa_wire::debug_server::{parse_dgram_bytes, trim_cmd_line};
use salsa_wire::dsp::{
    salsa_dsp_pack_pcm, salsa_dsp_resample_mic_cubic, salsa_dsp_scale_cap_clip,
};
use salsa_wire::hw_analog::{
    lasapi_hw_analog_clamp_cfg, lasapi_hw_analog_default_cfg, lasapi_hw_analog_score_card,
};
use salsa_wire::json::JsonValue;
use salsa_wire::pa_wire::{parse_frame_hdr, PaPropBag, TagReader, PA_MAX_FRAME_SZ};
use salsa_wire::pw_wire::{fd_quota_check, parse_param_pod, parse_pkt_hdr, PWB_MAX_CLIENT_FDS};
use salsa_wire::settings::{clamp_mic_apo_cfg, clamp_param_eq_ext};
use salsa_wire::state_conf::default_runtime_state;

struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn fill_bytes(&mut self, out: &mut [u8]) {
        let mut i = 0;
        while i < out.len() {
            let v = self.next_u64().to_le_bytes();
            let n = (out.len() - i).min(8);
            out[i..i + n].copy_from_slice(&v[..n]);
            i += n;
        }
    }
}

fn fuzz_one_input(data: &[u8]) {
    let size = data.len();
    if size < 2 {
        return;
    }

    // 1. PA frame header bounds check
    if size >= 21 {
        if let Some(hdr) = parse_frame_hdr(&data[1..]) {
            assert!(hdr.body_len <= PA_MAX_FRAME_SZ);
        }
    }

    // 2. PW packet header + FD quota invariant check
    if let Some(pkt) = parse_pkt_hdr(data) {
        assert!(pkt.pkt_len >= 16 && pkt.pkt_len <= size);
        assert!(pkt.nfds <= PWB_MAX_CLIENT_FDS);
    }
    if size >= 8 {
        let cur_n = ((u32::from(data[0])) << 1) | u32::from(data[1] & 1);
        let inc_n = u32::from(data[2]) | (u32::from(data[3] & 1) << 8);
        let (ok, allow) = fd_quota_check(cur_n, inc_n);
        if ok {
            assert!(allow <= inc_n);
            assert!(cur_n + allow <= PWB_MAX_CLIENT_FDS);
        }
    }

    // 3. PW SPA Param Pod parser check
    let _ = parse_param_pod(data);

    // 4. PA tagstruct sequential decode monotonicity & slice bounds check
    if let Some(mut rtr) = TagReader::new(data, 0) {
        for step in 0..8usize {
            if rtr.pos >= size {
                break;
            }
            let prev_pos = rtr.pos;
            let sel = data[step % size] & 7;
            match sel {
                0 => {
                    if rtr.read_u32().is_some() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                    }
                }
                1 => {
                    if let Some(slice_opt) = rtr.read_str_slice() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        if let Some((off, len)) = slice_opt {
                            assert!(off <= size && len <= size - off);
                        }
                    }
                }
                2 => {
                    if rtr.read_ss().is_some() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                    }
                }
                3 => {
                    if rtr.read_map().is_some() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                    }
                }
                4 => {
                    if rtr.read_cvol().is_some() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                    }
                }
                5 => {
                    let mut pb = PaPropBag::default();
                    if rtr.read_proplist(Some(&mut pb)).is_some() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                    }
                }
                6 => {
                    if rtr.read_timeval().is_some() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                    }
                }
                _ => {
                    if rtr.skip_format_info().is_some() {
                        assert!(rtr.pos > prev_pos && rtr.pos <= size);
                    }
                }
            }
        }
        let mut pb = PaPropBag::default();
        pb.set_kv("test.key", "test.val");
    }

    // 5. BlueZ A2DP SEP negotiation + BDADDR parser
    {
        let kind = i32::from(data[0] % 5);
        let mut r_cfg = [0u8; 16];
        if let Some((r_len, _r_bit)) = select_a2dp_config(kind, &data[1..], &mut r_cfg) {
            assert!(r_len <= r_cfg.len());
        }
        let _ = parse_bdaddr(&data[..size.min(32)]);
    }

    // 6. BlueZ HFP AT parser, paths, RTP/SBC/aptX, mSBC, ABR, ring resampler
    {
        let _ = parse_hfp_at_line(data);

        let hfp_prefixes: [&[u8]; 6] = [
            b"AT+BAC=",
            b"AT+BCS=",
            b"AT+BIEV=",
            b"AT+IPHONEACCEV=",
            b"AT+CIND=?",
            b"AT+BRSF=",
        ];
        let mut synth_at = [0u8; 160];
        let pfx = hfp_prefixes[(data[0] % 6) as usize];
        let copy_n = size.min(synth_at.len() - 1 - pfx.len());
        synth_at[..pfx.len()].copy_from_slice(pfx);
        synth_at[pfx.len()..pfx.len() + copy_n].copy_from_slice(&data[..copy_n]);
        let _ = parse_hfp_at_line(&synth_at[..pfx.len() + copy_n]);

        let zlen = size.min(128);
        let mut r_bpath = [0xAAu8; 64];
        let bcap = (data[0] as usize) % r_bpath.len();
        if let Some(n) = hfp_make_batt_path(&data[..zlen], &mut r_bpath[..bcap]) {
            assert!(n <= bcap);
        }

        if let Some(mac) = extract_mac_from_id(&data[..zlen]) {
            assert!(mac.iter().all(|&b| b.is_ascii()));
        }

        let mut r_rtp = [0u8; 16];
        let rtp_cap = (data[0] as usize) % r_rtp.len();
        let seq = (u16::from(data[0]) << 8) | u16::from(data[1]);
        let ts = if size >= 6 {
            u32::from_le_bytes([data[2], data[3], data[4], data[5]])
        } else {
            u32::from(seq)
        };
        assert!(
            write_rtp_header(
                &mut r_rtp[..rtp_cap],
                data[0],
                (data[1] & 1) != 0,
                seq,
                ts,
                ts ^ 1,
            )
            .unwrap_or(0)
                <= rtp_cap
        );

        let mut r_pl = [0u8; 1];
        let plcap = usize::from(data[0] & 1);
        assert!(write_rtp_sbc_payload_hdr(&mut r_pl[..plcap], data[1]).unwrap_or(0) <= plcap);

        let mut pcm_in = [0i16; 32];
        let nsamp = (size / 2).min(32);
        for (i, item) in pcm_in[..nsamp].iter_mut().enumerate() {
            *item = i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
        }
        let mut r_s24 = [0u8; 96];
        let s24_cap = (data[1] as usize) % r_s24.len();
        assert!(
            pack_s16_to_s24le(&pcm_in[..nsamp], &mut r_s24[..s24_cap]).unwrap_or(0) <= s24_cap
        );

        let sf = if (data[0] & 1) != 0 {
            44100
        } else if (data[0] & 2) != 0 {
            88200
        } else {
            48000
        };
        let eqm = i32::from(data[1] % 5) - 1;
        let _ = ldac_eqmid_to_kbps(sf, eqm);
        let _ = msbc_h2_byte(ts);
        assert!(msbc_rx_strip_offset(data) <= size);

        let _ = abr_step_ldac(eqm, u32::from(data[0] & 7), ts & 511);

        let cur_bps = u32::from(data[0] & 7) * 64_000;
        let max_bps = u32::from(data[1] & 7) * 64_000;
        let _ = abr_step_aac(cur_bps, max_bps, u32::from(data[0] & 7), ts & 511);

        if size >= 16 && (data[0] & 0x0F) == 0 {
            let mut r_tx_ring = vec![0.0f32; 16_384 * 2];
            let mut r_cap_ring = vec![0.0f32; 32_768 * 2];
            let rates = [0u32, 8000, 16000, 44100, 48000, 96000];
            let ch = if (data[1] & 1) != 0 { 2u32 } else { 1u32 };
            let frames = u32::from(data[2] % 64);
            let mut f_in = [0.0f32; 64 * 2];
            for i in 0..(frames * ch) as usize {
                let b0 = data[(3 + i * 2) % size];
                let b1 = data[(4 + i * 2) % size];
                let sv = i16::from_le_bytes([b0, b1]);
                f_in[i] = f32::from(sv) / 16384.0;
            }
            let r_wr = AtomicU64::new(0);
            let r_rd = AtomicU64::new(0);
            let paused = (data[3] & 1) != 0;
            let _ = write_tx_period(
                &mut r_tx_ring,
                &r_wr,
                paused,
                &f_in[..],
                frames as usize,
                ch as usize,
            );

            let in_r = rates[(data[4] % 6) as usize];
            let out_r = rates[(data[5] % 6) as usize];
            let need_fr = u32::from(data[6] % 48);
            let bt_ch = if (data[7] & 1) != 0 { 2u32 } else { 1u32 };
            let muted = (data[8] & 1) != 0;
            let vol = u32::from(data[9] % 150);
            let avrcp = (data[10] & 1) != 0;
            let mut r_frac = f64::from(data[11] & 7) * 0.125;
            let drift = if (data[12] & 1) != 0 { 1.001 } else { 0.999 };
            let mut r_out = [0i16; 48 * 2];
            let (popped, _rem) = pop_bt_resampled_s16(
                &r_tx_ring,
                &r_wr,
                &r_rd,
                in_r,
                out_r,
                muted,
                vol,
                avrcp,
                &mut r_out[..],
                need_fr as usize,
                bt_ch,
                &mut r_frac,
                drift,
            );
            assert!(popped <= need_fr as usize);

            let r_cwr = AtomicU64::new(0);
            let mut r_rp = 0.25f64;
            let mut r_prev = 0.1f32;
            let sco_r = if (data[13] & 1) != 0 { 16000 } else { 8000 };
            let eng_r = if (data[14] & 1) != 0 { 44100 } else { 48000 };
            sco_push_pcm_to_cap_ring(
                &mut r_cap_ring,
                &r_cwr,
                &pcm_in[..nsamp],
                sco_r,
                eng_r,
                &mut r_rp,
                &mut r_prev,
            );
        }
    }

    // 7. Debug datagram parser, command trimmer, and JsonValue string round-trip
    {
        assert!(trim_cmd_line(data).len() <= size);

        let _ = parse_dgram_bytes(data);

        if size >= mem::size_of::<LasapiDbgDgram>() && (data[0] & 1) != 0 {
            let mut dgram_buf = [0u8; mem::size_of::<LasapiDbgDgram>()];
            dgram_buf.copy_from_slice(&data[..mem::size_of::<LasapiDbgDgram>()]);
            let magic_le = LASAPI_DEBUG_DGRAM_MAGIC.to_le_bytes();
            dgram_buf[0..4].copy_from_slice(&magic_le);
            assert!(parse_dgram_bytes(&dgram_buf).is_some());
        }

        let s = String::from_utf8_lossy(&data[..size.min(128)]).into_owned();
        let encoded = JsonValue::Str(s).to_json_compact();
        assert!(JsonValue::parse(&encoded).is_ok());
    }

    // 8. State defaults, DSP clampers, HW Analog scoring, resampler, and PCM packer
    unsafe {
        let rst = default_runtime_state();
        assert!(rst.sample_rate >= 44100 && rst.sample_rate <= 384000);

        let mut hwa: LasapiHwAnalogCfg = mem::zeroed();
        lasapi_hw_analog_default_cfg(&mut hwa);
        assert_eq!(hwa.hw_dac_unity_vol, 1);
        lasapi_hw_analog_clamp_cfg(&mut hwa);
        let cname = b"Scarlett 2i2 4th Gen\0";
        let _ = lasapi_hw_analog_score_card(99, cname.as_ptr().cast::<c_char>());

        if size >= mem::size_of::<LasapiParamEqExt>() {
            let mut r_ext: LasapiParamEqExt = mem::zeroed();
            core::ptr::copy_nonoverlapping(
                data.as_ptr(),
                (&raw mut r_ext).cast::<u8>(),
                mem::size_of::<LasapiParamEqExt>(),
            );
            clamp_param_eq_ext(&mut r_ext);
            assert!(r_ext.crossfeed_cutoff_hz >= 300 && r_ext.crossfeed_cutoff_hz <= 1500);
        }
        if size >= mem::size_of::<LasapiMicApoCfg>() {
            let mut r_mic: LasapiMicApoCfg = mem::zeroed();
            core::ptr::copy_nonoverlapping(
                data.as_ptr(),
                (&raw mut r_mic).cast::<u8>(),
                mem::size_of::<LasapiMicApoCfg>(),
            );
            clamp_mic_apo_cfg(&mut r_mic);
            assert!(r_mic.clarity_db >= -6.0 && r_mic.clarity_db <= 12.0);
        }

        if size >= 34 {
            let mut r_in = [0.0f32; 16];
            let mut r_clip = [0.0f32; 16];
            for i in 0..16usize {
                let b_hi = data[(i * 2) % size];
                let b_lo = data[(i * 2 + 1) % size];
                let v = f32::from(i16::from_be_bytes([b_hi, b_lo])) / 16384.0;
                r_in[i] = v;
                r_clip[i] = v;
            }
            let gain = 0.25 + f32::from(data[0] & 0x3F) * 0.05;
            let softknee = (data[1] & 1) != 0;
            salsa_dsp_scale_cap_clip(r_clip.as_mut_ptr(), 16, gain);
            for v in r_clip {
                assert!(v.is_finite());
                if (gain - 1.0).abs() > 0.002 {
                    assert!((-1.0..=1.0).contains(&v));
                }
            }

            let fmt = u32::from(data[2] % 5);
            let mut r_out = [0u8; 64];
            salsa_dsp_pack_pcm(
                r_in.as_ptr(),
                16,
                gain,
                softknee,
                fmt,
                r_out.as_mut_ptr(),
            );

            let mut rst_c: SalsaMicCubicState = mem::zeroed();
            let mut r_res = [0.0f32; 32];
            let out_fr = 8 + u32::from(data[3] % 9);
            salsa_dsp_resample_mic_cubic(
                r_in.as_ptr(),
                8,
                r_res.as_mut_ptr(),
                out_fr,
                &raw mut rst_c,
            );
            for &v in &r_res[..(out_fr as usize) * 2] {
                assert!(v.is_finite());
            }
        }
    }
}

fn run_fuzz(runs: usize) {
    let mut rng = SplitMix64(0x5341_4C53_415F_5255);
    let seeds: [&[u8]; 6] = [
        b"",
        b"\x00",
        b"\x00\x00\x00\x14\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
        b"AT+BAC=1,2\r\n\x00",
        b"bluez:00:11:22:33:44:55\x00",
        &[0xFF; 256],
    ];
    for s in seeds {
        fuzz_one_input(s);
    }

    let mut buf = [0u8; 512];
    for i in 0..runs {
        let len = match i % 8 {
            0 => (rng.next_u64() as usize) % 24,
            1 => 16 + ((rng.next_u64() as usize) % 64),
            2 => 240 + ((rng.next_u64() as usize) % 64),
            _ => 2 + ((rng.next_u64() as usize) % (buf.len() - 2)),
        };
        rng.fill_bytes(&mut buf[..len]);
        if i % 16 == 0 && len >= 16 {
            buf[0] &= 0xF0;
        }
        fuzz_one_input(&buf[..len]);
    }
    println!("fuzz_wire (pure Rust): {runs} iterations + seeds passed with 0 invariant failures");
}

fn main() {
    salsa_wire::unix_sys::reset_sigpipe_default();
    let mut runs = 250_000usize;
    for arg in env::args().skip(1) {
        if let Some(val) = arg.strip_prefix("-runs=") {
            if let Ok(n) = val.parse::<usize>() {
                runs = n;
            }
        }
    }
    run_fuzz(runs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzz_wire_smoke_500_runs() {
        run_fuzz(500);
    }
}
