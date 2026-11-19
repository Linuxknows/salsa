#![allow(clippy::too_many_lines)]

use core::ffi::c_char;
use core::mem;
use core::sync::atomic::AtomicU64;
use std::env;

use salsa_wire::c_abi::{
    LwasapiDbgDgram, LwasapiHwAnalogCfg, LwasapiMicApoCfg, LwasapiParamEqExt, SalsaMicCubicState,
    SalsaRuntimeState, LWASAPI_DEBUG_DGRAM_MAGIC,
};
use salsa_wire::debug_server::{salsa_dbg_escape_json_str, salsa_dbg_parse_dgram};
use salsa_wire::dsp::{
    salsa_dsp_pack_pcm, salsa_dsp_resample_mic_cubic, salsa_dsp_scale_cap_clip,
};
use salsa_wire::ffi::{
    salsa_bt_abr_step_aac, salsa_bt_abr_step_ldac, salsa_bt_extract_mac, salsa_bt_hfp_parse_at,
    salsa_bt_ldac_eqmid_to_kbps, salsa_bt_make_batt_path, salsa_bt_msbc_h2_byte,
    salsa_bt_msbc_rx_strip_offset, salsa_bt_pack_s16_to_s24le, salsa_bt_parse_bdaddr,
    salsa_bt_ring_pop_resampled_s16, salsa_bt_ring_write_period, salsa_bt_sco_push_pcm,
    salsa_bt_select_a2dp_config, salsa_bt_write_rtp_header, salsa_bt_write_rtp_sbc_payload_hdr,
    salsa_dbg_trim_cmd, salsa_pa_parse_frame_hdr, salsa_propbag_set, salsa_pwb_fd_quota_check,
    salsa_pwb_parse_param_pod, salsa_pwb_parse_pkt_hdr, salsa_tr_init, salsa_tr_map,
    salsa_tr_proplist, salsa_tr_skip_format_info, salsa_tr_ss, salsa_tr_str_slice,
    salsa_tr_timeval, salsa_tr_u32, SalsaBtHfpAtRes, SalsaPaPropbag, SalsaPaTagread,
    SalsaPaWireCvol, SalsaPaWireMap, SalsaPaWireSs, SalsaPwbParamParsed,
};
use salsa_wire::pa_wire::PA_MAX_FRAME_SZ;
use salsa_wire::pw_wire::PWB_MAX_CLIENT_FDS;
use salsa_wire::hw_analog::{
    lwasapi_hw_analog_clamp_cfg, lwasapi_hw_analog_default_cfg, lwasapi_hw_analog_score_card,
};
use salsa_wire::state_conf::{
    salsa_dsp_clamp_mic_apo_cfg, salsa_dsp_clamp_param_eq_ext, salsa_state_init_defaults,
};

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

    unsafe {
        // 1. PA frame header bounds check
        if size >= 21 {
            let mut r_len = 0u32;
            let mut r_ch = 0u32;
            let mut r_fl = 0u32;
            if salsa_pa_parse_frame_hdr(data.as_ptr().add(1), &mut r_len, &mut r_ch, &mut r_fl) {
                assert!(r_len <= PA_MAX_FRAME_SZ);
            }
        }

        // 2. PW packet header + FD quota invariant check
        {
            let mut ro = 0u32;
            let mut rop = 0u8;
            let mut rp = 0u32;
            let mut rs = 0u32;
            let mut rn = 0u32;
            let mut rpl = 0usize;
            if salsa_pwb_parse_pkt_hdr(
                data.as_ptr(),
                size,
                &mut ro,
                &mut rop,
                &mut rp,
                &mut rs,
                &mut rn,
                &mut rpl,
            ) {
                assert!(rpl >= 16 && rpl <= size);
                assert!(rn <= PWB_MAX_CLIENT_FDS);
            }
        }
        if size >= 8 {
            let cur_n = ((u32::from(data[0])) << 1) | u32::from(data[1] & 1);
            let inc_n = u32::from(data[2]) | (u32::from(data[3] & 1) << 8);
            let mut ra = 999u32;
            if salsa_pwb_fd_quota_check(cur_n, inc_n, &mut ra) {
                assert!(ra <= inc_n);
                assert!(cur_n + ra <= PWB_MAX_CLIENT_FDS);
            }
        }

        // 3. PW SPA Param Pod parser check
        {
            let mut rp: SalsaPwbParamParsed = mem::zeroed();
            salsa_pwb_parse_param_pod(data.as_ptr(), size, &mut rp);
        }

        // 4. PA tagstruct sequential decode monotonicity & slice bounds check
        {
            let mut rtr: SalsaPaTagread = mem::zeroed();
            salsa_tr_init(&mut rtr, data.as_ptr(), size);
            for step in 0..8usize {
                if rtr.pos >= size {
                    break;
                }
                let prev_pos = rtr.pos;
                let sel = data[step % size] & 7;
                match sel {
                    0 => {
                        let mut rv = 0u32;
                        if salsa_tr_u32(&mut rtr, &mut rv) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        }
                    }
                    1 => {
                        let mut roff = 0usize;
                        let mut rlen = 0usize;
                        let mut rnull = false;
                        if salsa_tr_str_slice(&mut rtr, &mut roff, &mut rlen, &mut rnull) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                            if !rnull {
                                assert!(roff <= size && rlen <= size - roff);
                            }
                        }
                    }
                    2 => {
                        let mut rss: SalsaPaWireSs = mem::zeroed();
                        if salsa_tr_ss(&mut rtr, &mut rss) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        }
                    }
                    3 => {
                        let mut rm: SalsaPaWireMap = mem::zeroed();
                        if salsa_tr_map(&mut rtr, &mut rm) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        }
                    }
                    4 => {
                        let mut rcv: SalsaPaWireCvol = mem::zeroed();
                        if salsa_wire::ffi::salsa_tr_cvol(&mut rtr, &mut rcv) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        }
                    }
                    5 => {
                        let mut rpb: SalsaPaPropbag = mem::zeroed();
                        if salsa_tr_proplist(&mut rtr, &mut rpb) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        }
                    }
                    6 => {
                        let mut rsec = 0u32;
                        let mut rusec = 0u32;
                        if salsa_tr_timeval(&mut rtr, &mut rsec, &mut rusec) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        }
                    }
                    _ => {
                        if salsa_tr_skip_format_info(&mut rtr) {
                            assert!(rtr.pos > prev_pos && rtr.pos <= size);
                        }
                    }
                }
            }
            let mut pb: SalsaPaPropbag = mem::zeroed();
            let k = b"test.key\0";
            let v = b"test.val\0";
            salsa_propbag_set(
                &mut pb,
                k.as_ptr().cast::<c_char>(),
                v.as_ptr().cast::<c_char>(),
            );
        }

        // 5. BlueZ A2DP SEP negotiation + BDADDR parser
        {
            let kind = i32::from(data[0] % 5);
            let mut r_cfg = [0u8; 16];
            let mut r_len = 0usize;
            let mut r_bit = 0u32;
            let rr = salsa_bt_select_a2dp_config(
                kind,
                data.as_ptr().add(1),
                size - 1,
                r_cfg.as_mut_ptr(),
                r_cfg.len(),
                &mut r_len,
                &mut r_bit,
            );
            if rr == 0 {
                assert!(r_len <= r_cfg.len());
            }
            let mut rb = [0u8; 6];
            let _ = salsa_bt_parse_bdaddr(
                data.as_ptr().cast::<c_char>(),
                size.min(32),
                rb.as_mut_ptr(),
            );
        }

        // 6. BlueZ HFP AT parser, paths, RTP/SBC/aptX, mSBC, ABR, ring resampler
        {
            let mut r_at: SalsaBtHfpAtRes = mem::zeroed();
            let _ = salsa_bt_hfp_parse_at(data.as_ptr().cast::<c_char>(), size, &mut r_at);

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
            synth_at[pfx.len() + copy_n] = 0;
            let _ = salsa_bt_hfp_parse_at(
                synth_at.as_ptr().cast::<c_char>(),
                pfx.len() + copy_n,
                &mut r_at,
            );

            let mut zstr = [0u8; 129];
            let zlen = size.min(128);
            zstr[..zlen].copy_from_slice(&data[..zlen]);
            zstr[zlen] = 0;

            let mut r_bpath = [0xAAu8; 64];
            let bcap = (data[0] as usize) % r_bpath.len();
            if salsa_bt_make_batt_path(
                zstr.as_ptr().cast::<c_char>(),
                r_bpath.as_mut_ptr().cast::<c_char>(),
                bcap,
            ) && bcap > 0
            {
                assert!(r_bpath[..bcap].contains(&0));
            }

            let mut r_mac = [0x55u8; 24];
            let mcap = (data[1] as usize) % r_mac.len();
            if salsa_bt_extract_mac(
                zstr.as_ptr().cast::<c_char>(),
                r_mac.as_mut_ptr().cast::<c_char>(),
                mcap,
            ) && mcap > 0
            {
                assert!(r_mac[..mcap].contains(&0));
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
                salsa_bt_write_rtp_header(
                    r_rtp.as_mut_ptr(),
                    rtp_cap,
                    data[0],
                    (data[1] & 1) != 0,
                    seq,
                    ts,
                    ts ^ 1,
                ) <= rtp_cap
            );

            let mut r_pl = 0u8;
            let plcap = usize::from(data[0] & 1);
            assert!(salsa_bt_write_rtp_sbc_payload_hdr(&mut r_pl, plcap, data[1]) <= plcap);

            let mut pcm_in = [0i16; 32];
            let nsamp = (size / 2).min(32);
            for (i, item) in pcm_in[..nsamp].iter_mut().enumerate() {
                *item = i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
            }
            let mut r_s24 = [0u8; 96];
            let s24_cap = (data[1] as usize) % r_s24.len();
            assert!(
                salsa_bt_pack_s16_to_s24le(
                    pcm_in.as_ptr(),
                    nsamp,
                    r_s24.as_mut_ptr(),
                    s24_cap
                ) <= s24_cap
            );

            let sf = if (data[0] & 1) != 0 {
                44100
            } else if (data[0] & 2) != 0 {
                88200
            } else {
                48000
            };
            let eqm = i32::from(data[1] % 5) - 1;
            let _ = salsa_bt_ldac_eqmid_to_kbps(sf, eqm);
            let _ = salsa_bt_msbc_h2_byte(ts);
            assert!(salsa_bt_msbc_rx_strip_offset(data.as_ptr(), size) <= size);

            let mut r_eq = 0i32;
            let mut r_cc = false;
            let mut r_cl = false;
            salsa_bt_abr_step_ldac(
                eqm,
                u32::from(data[0] & 7),
                ts & 511,
                &mut r_eq,
                &mut r_cc,
                &mut r_cl,
            );

            let cur_bps = u32::from(data[0] & 7) * 64_000;
            let max_bps = u32::from(data[1] & 7) * 64_000;
            let mut r_nb = 0u32;
            salsa_bt_abr_step_aac(
                cur_bps,
                max_bps,
                u32::from(data[0] & 7),
                ts & 511,
                &mut r_nb,
                &mut r_cc,
                &mut r_cl,
            );

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
                let _ = salsa_bt_ring_write_period(
                    r_tx_ring.as_mut_ptr(),
                    &r_wr,
                    paused,
                    f_in.as_ptr(),
                    frames,
                    ch,
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
                let mut r_rem = 0u32;
                assert!(
                    salsa_bt_ring_pop_resampled_s16(
                        r_tx_ring.as_ptr(),
                        &r_wr,
                        &r_rd,
                        in_r,
                        out_r,
                        muted,
                        vol,
                        avrcp,
                        r_out.as_mut_ptr(),
                        need_fr,
                        bt_ch,
                        &mut r_frac,
                        drift,
                        &mut r_rem,
                    ) <= need_fr as usize
                );

                let r_cwr = AtomicU64::new(0);
                let mut r_rp = 0.25f64;
                let mut r_prev = 0.1f32;
                let sco_r = if (data[13] & 1) != 0 { 16000 } else { 8000 };
                let eng_r = if (data[14] & 1) != 0 { 44100 } else { 48000 };
                salsa_bt_sco_push_pcm(
                    r_cap_ring.as_mut_ptr(),
                    &r_cwr,
                    pcm_in.as_ptr(),
                    nsamp as u32,
                    sco_r,
                    eng_r,
                    &mut r_rp,
                    &mut r_prev,
                );
            }
        }

        // 7. Debug datagram parser, command trimmer, and JSON string escaper
        {
            assert!(salsa_dbg_trim_cmd(data.as_ptr(), size) <= size);

            let mut r_dg: LwasapiDbgDgram = mem::zeroed();
            let _ = salsa_dbg_parse_dgram(data.as_ptr(), size, &mut r_dg);

            if size >= mem::size_of::<LwasapiDbgDgram>() && (data[0] & 1) != 0 {
                let mut dgram_buf = [0u8; mem::size_of::<LwasapiDbgDgram>()];
                dgram_buf.copy_from_slice(&data[..mem::size_of::<LwasapiDbgDgram>()]);
                let magic_le = LWASAPI_DEBUG_DGRAM_MAGIC.to_le_bytes();
                dgram_buf[0..4].copy_from_slice(&magic_le);
                assert!(salsa_dbg_parse_dgram(
                    dgram_buf.as_ptr(),
                    dgram_buf.len(),
                    &mut r_dg
                ));
            }

            let mut jsrc = [0u8; 129];
            let jlen = size.min(128);
            jsrc[..jlen].copy_from_slice(&data[..jlen]);
            jsrc[jlen] = 0;
            let mut r_json = [0xCCu8; 1024];
            let rj = salsa_dbg_escape_json_str(
                jsrc.as_ptr().cast::<c_char>(),
                r_json.as_mut_ptr().cast::<c_char>(),
                r_json.len(),
            );
            assert!(rj < r_json.len() && r_json[rj] == 0);

            let mut asc_src = [0u8; 65];
            let alen = size.min(64);
            for i in 0..alen {
                asc_src[i] = data[i] & 0x7F;
            }
            asc_src[alen] = 0;
            let tcap = usize::from(data[0] % 64);
            let mut r_trunc = [0xDDu8; 64];
            let rjt = salsa_dbg_escape_json_str(
                asc_src.as_ptr().cast::<c_char>(),
                r_trunc.as_mut_ptr().cast::<c_char>(),
                tcap,
            );
            if tcap > 0 {
                assert!(rjt < tcap && r_trunc[rjt] == 0);
            }
        }

        // 8. State defaults, DSP clampers, HW Analog scoring, resampler, and PCM packer
        {
            let mut rst: SalsaRuntimeState = mem::zeroed();
            salsa_state_init_defaults(&mut rst);
            assert!(rst.sample_rate >= 44100 && rst.sample_rate <= 384000);

            let mut hwa: LwasapiHwAnalogCfg = mem::zeroed();
            lwasapi_hw_analog_default_cfg(&mut hwa);
            assert_eq!(hwa.hw_dac_unity_vol, 1);
            lwasapi_hw_analog_clamp_cfg(&mut hwa);
            let cname = b"Scarlett 2i2 4th Gen\0";
            let _ = lwasapi_hw_analog_score_card(99, cname.as_ptr().cast::<c_char>());

            if size >= mem::size_of::<LwasapiParamEqExt>() {
                let mut r_ext: LwasapiParamEqExt = mem::zeroed();
                core::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    (&mut r_ext as *mut LwasapiParamEqExt).cast::<u8>(),
                    mem::size_of::<LwasapiParamEqExt>(),
                );
                salsa_dsp_clamp_param_eq_ext(&mut r_ext);
                assert!(r_ext.crossfeed_cutoff_hz >= 300 && r_ext.crossfeed_cutoff_hz <= 1500);
            }
            if size >= mem::size_of::<LwasapiMicApoCfg>() {
                let mut r_mic: LwasapiMicApoCfg = mem::zeroed();
                core::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    (&mut r_mic as *mut LwasapiMicApoCfg).cast::<u8>(),
                    mem::size_of::<LwasapiMicApoCfg>(),
                );
                salsa_dsp_clamp_mic_apo_cfg(&mut r_mic);
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
                salsa_dsp_resample_mic_cubic(r_in.as_ptr(), 8, r_res.as_mut_ptr(), out_fr, &mut rst_c);
                for &v in &r_res[..(out_fr as usize) * 2] {
                    assert!(v.is_finite());
                }
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
            buf[0] &= 0xF0; // exercise ring resampler branch frequently
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
