#include <assert.h>
#include <math.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "../../include/lwasapi_debug.h"
#include "../../include/lwasapi_hw_analog.h"
#include "../../include/salsa_wire.h"

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    if (size < 2) return 0;

    /* 1. PA frame header bounds check */
    if (size >= 21) {
        uint32_t r_len = 0, r_ch = 0, r_fl = 0;
        if (salsa_pa_parse_frame_hdr(data + 1, &r_len, &r_ch, &r_fl)) {
            assert(r_len <= SALSA_PA_MAX_FRAME_SZ);
        }
    }

    /* 2. PW packet header + FD quota invariant check */
    {
        uint32_t ro = 0, rp = 0, rs = 0, rn = 0;
        uint8_t rop = 0;
        size_t rpl = 0;
        if (salsa_pwb_parse_pkt_hdr(data, size, &ro, &rop, &rp, &rs, &rn, &rpl)) {
            assert(rpl >= 16u && rpl <= size);
            assert(rn <= SALSA_PWB_MAX_CLIENT_FDS);
        }
    }
    if (size >= 8) {
        uint32_t cur_n = ((uint32_t)data[0] << 1) | (data[1] & 1u);
        uint32_t inc_n = (uint32_t)data[2] | (((uint32_t)data[3] & 1u) << 8);
        uint32_t ra = 999u;
        if (salsa_pwb_fd_quota_check(cur_n, inc_n, &ra)) {
            assert(ra <= inc_n);
            assert(cur_n + ra <= SALSA_PWB_MAX_CLIENT_FDS);
        }
    }

    /* 3. PW SPA Param Pod parser check */
    {
        salsa_pwb_param_parsed_t rp;
        memset(&rp, 0, sizeof(rp));
        salsa_pwb_parse_param_pod(data, size, &rp);
    }

    /* 4. PA tagstruct sequential decode monotonicity & slice bounds check */
    {
        salsa_pa_tagread_t rtr;
        salsa_tr_init(&rtr, data, size);
        for (int step = 0; step < 8 && rtr.pos < size; step++) {
            size_t prev_pos = rtr.pos;
            uint8_t sel = data[step % size] & 7u;
            if (sel == 0) {
                uint32_t rv = 0;
                if (salsa_tr_u32(&rtr, &rv)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                }
            } else if (sel == 1) {
                size_t roff = 0, rlen = 0;
                bool rnull = false;
                if (salsa_tr_str_slice(&rtr, &roff, &rlen, &rnull)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                    if (!rnull) {
                        assert(roff <= size && rlen <= size - roff);
                    }
                }
            } else if (sel == 2) {
                salsa_pa_wire_ss_t rss;
                memset(&rss, 0, sizeof(rss));
                if (salsa_tr_ss(&rtr, &rss)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                }
            } else if (sel == 3) {
                salsa_pa_wire_map_t rm;
                memset(&rm, 0, sizeof(rm));
                if (salsa_tr_map(&rtr, &rm)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                }
            } else if (sel == 4) {
                salsa_pa_wire_cvol_t rcv;
                memset(&rcv, 0, sizeof(rcv));
                if (salsa_tr_cvol(&rtr, &rcv)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                }
            } else if (sel == 5) {
                salsa_pa_propbag_t rpb;
                memset(&rpb, 0, sizeof(rpb));
                if (salsa_tr_proplist(&rtr, &rpb)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                }
            } else if (sel == 6) {
                uint32_t rsec = 0, rusec = 0;
                if (salsa_tr_timeval(&rtr, &rsec, &rusec)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                }
            } else {
                if (salsa_tr_skip_format_info(&rtr)) {
                    assert(rtr.pos > prev_pos && rtr.pos <= size);
                }
            }
        }
    }

    /* 5. BlueZ A2DP SEP negotiation + BDADDR parser */
    {
        int kind = (int)(data[0] % 5u);
        uint8_t r_cfg[16];
        size_t r_len = 0;
        uint32_t r_bit = 0;
        memset(r_cfg, 0, sizeof(r_cfg));
        int rr = salsa_bt_select_a2dp_config(kind, data + 1, size - 1, r_cfg, 16u, &r_len, &r_bit);
        if (rr == 0) {
            assert(r_len <= sizeof(r_cfg));
        }
        uint8_t rb[6] = {0};
        (void)salsa_bt_parse_bdaddr((const char *)data, size > 32 ? 32 : size, rb);
    }

    /* 6. BlueZ HFP AT parser, paths, RTP/SBC/aptX, mSBC, ABR, ring resampler */
    {
        salsa_bt_hfp_at_res_t r_at;
        memset(&r_at, 0, sizeof(r_at));
        (void)salsa_bt_hfp_parse_at((const char *)data, size, &r_at);

        static const char *const hfp_prefixes[] = {
            "AT+BAC=", "AT+BCS=", "AT+BIEV=", "AT+IPHONEACCEV=", "AT+CIND=?", "AT+BRSF="
        };
        char synth_at[160];
        const char *pfx = hfp_prefixes[data[0] % 6u];
        size_t plen = strlen(pfx);
        size_t copy_n = (size > sizeof(synth_at) - 1u - plen) ? (sizeof(synth_at) - 1u - plen) : size;
        memcpy(synth_at, pfx, plen);
        memcpy(synth_at + plen, data, copy_n);
        synth_at[plen + copy_n] = '\0';
        (void)salsa_bt_hfp_parse_at(synth_at, plen + copy_n, &r_at);

        char zstr[129];
        size_t zlen = size > 128u ? 128u : size;
        memcpy(zstr, data, zlen);
        zstr[zlen] = '\0';

        char r_bpath[64];
        size_t bcap = (size_t)data[0] % sizeof(r_bpath);
        memset(r_bpath, 0xAA, sizeof(r_bpath));
        if (salsa_bt_make_batt_path(zstr, r_bpath, bcap) && bcap > 0) {
            assert(memchr(r_bpath, '\0', bcap) != NULL);
        }

        char r_mac[24];
        size_t mcap = (size_t)data[1] % sizeof(r_mac);
        memset(r_mac, 0x55, sizeof(r_mac));
        if (salsa_bt_extract_mac(zstr, r_mac, mcap) && mcap > 0) {
            assert(memchr(r_mac, '\0', mcap) != NULL);
        }

        uint8_t r_rtp[16];
        size_t rtp_cap = (size_t)data[0] % sizeof(r_rtp);
        uint16_t seq = (uint16_t)(((uint16_t)data[0] << 8) | data[1]);
        uint32_t ts = (size >= 6) ? ((uint32_t)data[2] | ((uint32_t)data[3] << 8) |
                                     ((uint32_t)data[4] << 16) | ((uint32_t)data[5] << 24)) : seq;
        memset(r_rtp, 0, sizeof(r_rtp));
        assert(salsa_bt_write_rtp_header(r_rtp, rtp_cap, data[0], (data[1] & 1u) != 0, seq, ts, ts ^ 1u) <= rtp_cap);

        uint8_t r_pl = 0;
        size_t plcap = (size_t)(data[0] & 1u);
        assert(salsa_bt_write_rtp_sbc_payload_hdr(&r_pl, plcap, data[1]) <= plcap);

        int16_t pcm_in[32];
        size_t nsamp = (size / 2u > 32u) ? 32u : (size / 2u);
        for (size_t i = 0; i < nsamp; i++) {
            pcm_in[i] = (int16_t)((uint16_t)data[i * 2u] | ((uint16_t)data[i * 2u + 1u] << 8));
        }
        uint8_t r_s24[96];
        size_t s24_cap = (size_t)data[1] % sizeof(r_s24);
        memset(r_s24, 0, sizeof(r_s24));
        assert(salsa_bt_pack_s16_to_s24le(pcm_in, nsamp, r_s24, s24_cap) <= s24_cap);

        uint32_t sf = (data[0] & 1u) ? 44100u : ((data[0] & 2u) ? 88200u : 48000u);
        int32_t eqm = (int32_t)(data[1] % 5u) - 1;
        (void)salsa_bt_ldac_eqmid_to_kbps(sf, eqm);
        (void)salsa_bt_msbc_h2_byte(ts);
        assert(salsa_bt_msbc_rx_strip_offset(data, size) <= size);

        int32_t r_eq = 0;
        bool r_cc = false, r_cl = false;
        salsa_bt_abr_step_ldac(eqm, data[0] & 7u, ts & 511u, &r_eq, &r_cc, &r_cl);

        uint32_t cur_bps = ((uint32_t)(data[0] & 7u)) * 64000u;
        uint32_t max_bps = ((uint32_t)(data[1] & 7u)) * 64000u;
        uint32_t r_nb = 0;
        salsa_bt_abr_step_aac(cur_bps, max_bps, data[0] & 7u, ts & 511u, &r_nb, &r_cc, &r_cl);

        if (size >= 16 && (data[0] & 0x0Fu) == 0u) {
            static float r_tx_ring[16384 * 2];
            static float r_cap_ring[32768 * 2];
            static const uint32_t rates[] = { 0u, 8000u, 16000u, 44100u, 48000u, 96000u };

            uint32_t ch = (data[1] & 1u) ? 2u : 1u;
            uint32_t frames = (uint32_t)(data[2] % 64u);
            float f_in[64 * 2];
            for (uint32_t i = 0; i < frames * ch; i++) {
                uint8_t b0 = data[(3u + i * 2u) % size];
                uint8_t b1 = data[(4u + i * 2u) % size];
                int16_t sv = (int16_t)((uint16_t)b0 | ((uint16_t)b1 << 8));
                f_in[i] = (float)sv / 16384.0f;
            }
            _Atomic uint64_t r_wr = 0, r_rd = 0;
            bool paused = (data[3] & 1u) != 0;
            (void)salsa_bt_ring_write_period(r_tx_ring, &r_wr, paused, f_in, frames, ch);

            uint32_t in_r = rates[data[4] % 6u];
            uint32_t out_r = rates[data[5] % 6u];
            uint32_t need_fr = (uint32_t)(data[6] % 48u);
            uint32_t bt_ch = (data[7] & 1u) ? 2u : 1u;
            bool muted = (data[8] & 1u) != 0;
            uint32_t vol = (uint32_t)(data[9] % 150u);
            bool avrcp = (data[10] & 1u) != 0;
            double r_frac = (double)(data[11] & 7u) * 0.125;
            double drift = (data[12] & 1u) ? 1.001 : 0.999;
            int16_t r_out[48 * 2];
            uint32_t r_rem = 0;
            memset(r_out, 0, sizeof(r_out));

            assert(salsa_bt_ring_pop_resampled_s16(
                r_tx_ring, &r_wr, &r_rd, in_r, out_r, muted, vol, avrcp,
                r_out, need_fr, bt_ch, &r_frac, drift, &r_rem) <= need_fr);

            _Atomic uint64_t r_cwr = 0;
            double r_rp = 0.25;
            float r_prev = 0.1f;
            uint32_t sco_r = (data[13] & 1u) ? 16000u : 8000u;
            uint32_t eng_r = (data[14] & 1u) ? 44100u : 48000u;
            salsa_bt_sco_push_pcm(r_cap_ring, &r_cwr, pcm_in, (uint32_t)nsamp, sco_r, eng_r, &r_rp, &r_prev);
        }
    }

    /* 7. Debug datagram parser, command trimmer, and JSON string escaper */
    {
        assert(salsa_dbg_trim_cmd(data, size) <= size);

        lwasapi_dbg_dgram_t r_dg;
        memset(&r_dg, 0, sizeof(r_dg));
        (void)salsa_dbg_parse_dgram(data, size, &r_dg);

        if (size >= sizeof(lwasapi_dbg_dgram_t) && (data[0] & 1u)) {
            uint8_t dgram_buf[sizeof(lwasapi_dbg_dgram_t)];
            memcpy(dgram_buf, data, sizeof(dgram_buf));
            dgram_buf[0] = (uint8_t)(SALSA_DBG_DGRAM_MAGIC & 0xFFu);
            dgram_buf[1] = (uint8_t)((SALSA_DBG_DGRAM_MAGIC >> 8) & 0xFFu);
            dgram_buf[2] = (uint8_t)((SALSA_DBG_DGRAM_MAGIC >> 16) & 0xFFu);
            dgram_buf[3] = (uint8_t)((SALSA_DBG_DGRAM_MAGIC >> 24) & 0xFFu);
            memset(&r_dg, 0, sizeof(r_dg));
            assert(salsa_dbg_parse_dgram(dgram_buf, sizeof(dgram_buf), &r_dg));
        }

        char jsrc[129];
        size_t jlen = size > 128u ? 128u : size;
        memcpy(jsrc, data, jlen);
        jsrc[jlen] = '\0';
        char r_json[1024];
        memset(r_json, 0xCC, sizeof(r_json));
        size_t rj = salsa_dbg_escape_json_str(jsrc, r_json, sizeof(r_json));
        assert(rj < sizeof(r_json) && r_json[rj] == '\0');

        char asc_src[65];
        size_t alen = size > 64u ? 64u : size;
        for (size_t i = 0; i < alen; i++) {
            asc_src[i] = (char)(data[i] & 0x7Fu);
        }
        asc_src[alen] = '\0';
        size_t tcap = (size_t)(data[0] % 64u);
        char r_trunc[64];
        memset(r_trunc, 0xDD, sizeof(r_trunc));
        size_t rjt = salsa_dbg_escape_json_str(asc_src, r_trunc, tcap);
        if (tcap > 0) {
            assert(rjt < tcap && r_trunc[rjt] == '\0');
        }
    }

    /* 8. State defaults, DSP clampers, HW Analog scoring, resampler, and PCM packer */
    {
        salsa_runtime_state_t rst;
        memset(&rst, 0, sizeof(rst));
        salsa_state_init_defaults(&rst);
        assert(rst.sample_rate >= 44100u && rst.sample_rate <= 384000u);

        lwasapi_hw_analog_cfg_t hwa;
        lwasapi_hw_analog_default_cfg(&hwa);
        assert(hwa.hw_dac_unity_vol == 1);
        lwasapi_hw_analog_clamp_cfg(&hwa);
        (void)lwasapi_hw_analog_score_card(99, "Scarlett 2i2 4th Gen");

        if (size >= sizeof(lwasapi_param_eq_ext_t)) {
            lwasapi_param_eq_ext_t r_ext;
            memcpy(&r_ext, data, sizeof(r_ext));
            salsa_dsp_clamp_param_eq_ext(&r_ext);
            assert(r_ext.crossfeed_cutoff_hz >= 300u && r_ext.crossfeed_cutoff_hz <= 1500u);
        }
        if (size >= sizeof(lwasapi_mic_apo_cfg_t)) {
            lwasapi_mic_apo_cfg_t r_mic;
            memcpy(&r_mic, data, sizeof(r_mic));
            salsa_dsp_clamp_mic_apo_cfg(&r_mic);
            assert(r_mic.clarity_db >= -6.0f && r_mic.clarity_db <= 12.0f);
        }

        if (size >= 34) {
            float r_in[16], r_clip[16];
            for (size_t i = 0; i < 16; i++) {
                float v = ((float)(int16_t)(((uint16_t)data[(i * 2u) % size] << 8) | data[(i * 2u + 1u) % size])) / 16384.0f;
                r_in[i] = v;
                r_clip[i] = v;
            }
            float gain = 0.25f + (float)(data[0] & 0x3Fu) * 0.05f;
            bool softknee = (data[1] & 1u) != 0;
            salsa_dsp_scale_cap_clip(r_clip, 16u, gain);
            for (size_t i = 0; i < 16; i++) {
                assert(isfinite(r_clip[i]));
                if (fabsf(gain - 1.0f) > 0.002f) {
                    assert(r_clip[i] >= -1.0f && r_clip[i] <= 1.0f);
                }
            }

            uint32_t fmt = (uint32_t)(data[2] % 5u);
            uint8_t r_out[64];
            memset(r_out, 0, sizeof(r_out));
            salsa_dsp_pack_pcm(r_in, 16u, gain, softknee, fmt, r_out);

            salsa_mic_cubic_state_t rst_c;
            memset(&rst_c, 0, sizeof(rst_c));
            float r_res[32];
            uint32_t out_fr = 8u + (uint32_t)(data[3] % 9u);
            salsa_dsp_resample_mic_cubic(r_in, 8u, r_res, out_fr, &rst_c);
            for (size_t i = 0; i < (size_t)out_fr * 2u; i++) {
                assert(isfinite(r_res[i]));
            }
        }
    }

    return 0;
}
