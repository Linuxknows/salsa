#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "../../include/salsa_wire.h"

/* Satisfy libsalsa_wire.a debug emit references without linking audiodg/debug daemon */
void lwasapi_debug_emit(uint8_t sub, uint8_t lvl, uint32_t pid, uint32_t sid,
                        const char *op, const char *tgt, const char *fmt, ...) {
    (void)sub; (void)lvl; (void)pid; (void)sid; (void)op; (void)tgt; (void)fmt;
}
void lwasapi_debug_emit_vals(uint8_t sub, uint8_t lvl, uint32_t pid, uint32_t sid,
                             int32_t v1, int32_t v2, int32_t hr,
                             const char *op, const char *tgt, const char *fmt, ...) {
    (void)sub; (void)lvl; (void)pid; (void)sid; (void)v1; (void)v2; (void)hr;
    (void)op; (void)tgt; (void)fmt;
}

/* Prefixed C fallback declarations from src/salsa_wire.c */
void c_salsa_tr_init(salsa_pa_tagread_t *tr, const uint8_t *p, size_t len);
bool c_salsa_tr_u32(salsa_pa_tagread_t *tr, uint32_t *out);
bool c_salsa_tr_u8(salsa_pa_tagread_t *tr, uint8_t *out);
bool c_salsa_tr_bool(salsa_pa_tagread_t *tr, bool *out);
bool c_salsa_tr_str_slice(salsa_pa_tagread_t *tr, size_t *out_off, size_t *out_len, bool *out_is_null);
bool c_salsa_tr_arbitrary_slice(salsa_pa_tagread_t *tr, size_t *out_off, size_t *out_len);
bool c_salsa_tr_ss(salsa_pa_tagread_t *tr, salsa_pa_wire_ss_t *ss);
bool c_salsa_tr_map(salsa_pa_tagread_t *tr, salsa_pa_wire_map_t *m);
bool c_salsa_tr_cvol(salsa_pa_tagread_t *tr, salsa_pa_wire_cvol_t *cv);
bool c_salsa_tr_timeval(salsa_pa_tagread_t *tr, uint32_t *sec, uint32_t *usec);
bool c_salsa_tr_proplist(salsa_pa_tagread_t *tr, salsa_pa_propbag_t *pb);
bool c_salsa_tr_skip_format_info(salsa_pa_tagread_t *tr);

bool c_salsa_pwb_parse_pkt_hdr(const uint8_t *buf, size_t avail,
                               uint32_t *out_oid, uint8_t *out_op,
                               uint32_t *out_psz, uint32_t *out_seq,
                               uint32_t *out_nfds, size_t *out_pkt_len);
void c_salsa_pwb_parse_param_pod(const uint8_t *pod, size_t pod_sz, salsa_pwb_param_parsed_t *out);
bool c_salsa_pwb_fd_quota_check(uint32_t current_nfds, uint32_t incoming_cnt, uint32_t *out_accept);
bool c_salsa_pa_parse_frame_hdr(const uint8_t hdr[20], uint32_t *out_body_len,
                                uint32_t *out_channel, uint32_t *out_flags);
int  c_salsa_bt_select_a2dp_config(int codec_kind, const uint8_t *rcaps, size_t rlen,
                                   uint8_t *out_cfg, size_t out_cap,
                                   size_t *out_len, uint32_t *out_cap_bit);
bool c_salsa_bt_parse_bdaddr(const char *str, size_t len, uint8_t out_b[6]);

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    if (size < 2) return 0;

    /* 1. PA frame header differential check */
    if (size >= 21) {
        uint32_t r_len = 0, r_ch = 0, r_fl = 0;
        uint32_t c_len = 0, c_ch = 0, c_fl = 0;
        bool r_ok = salsa_pa_parse_frame_hdr(data + 1, &r_len, &r_ch, &r_fl);
        bool c_ok = c_salsa_pa_parse_frame_hdr(data + 1, &c_len, &c_ch, &c_fl);
        assert(r_ok == c_ok);
        if (r_ok) {
            assert(r_len == c_len && r_ch == c_ch && r_fl == c_fl);
        }
    }

    /* 2. PW packet header + FD quota differential check */
    {
        uint32_t ro = 0, rp = 0, rs = 0, rn = 0, co = 0, cp = 0, cs = 0, cn = 0;
        uint8_t rop = 0, cop = 0;
        size_t rpl = 0, cpl = 0;
        bool r_ok = salsa_pwb_parse_pkt_hdr(data, size, &ro, &rop, &rp, &rs, &rn, &rpl);
        bool c_ok = c_salsa_pwb_parse_pkt_hdr(data, size, &co, &cop, &cp, &cs, &cn, &cpl);
        assert(r_ok == c_ok);
        if (r_ok) {
            assert(ro == co && rop == cop && rp == cp && rs == cs && rn == cn && rpl == cpl);
        }
    }
    if (size >= 8) {
        uint32_t cur_n = ((uint32_t)data[0] << 1) | (data[1] & 1u);
        uint32_t inc_n = (uint32_t)data[2] | (((uint32_t)data[3] & 1u) << 8);
        uint32_t ra = 999u, ca = 999u;
        bool rok = salsa_pwb_fd_quota_check(cur_n, inc_n, &ra);
        bool cok = c_salsa_pwb_fd_quota_check(cur_n, inc_n, &ca);
        assert(rok == cok && ra == ca);
    }

    /* 3. PW SPA Param Pod differential check */
    {
        salsa_pwb_param_parsed_t rp, cp;
        memset(&rp, 0, sizeof(rp));
        memset(&cp, 0, sizeof(cp));
        salsa_pwb_parse_param_pod(data, size, &rp);
        c_salsa_pwb_parse_param_pod(data, size, &cp);
        assert(rp.kind == cp.kind);
        assert(memcmp(&rp.vfmt, &cp.vfmt, sizeof(rp.vfmt)) == 0);
        assert(memcmp(&rp.bcaps, &cp.bcaps, sizeof(rp.bcaps)) == 0);
    }

    /* 4. PA tagstruct sequential decode differential check */
    {
        salsa_pa_tagread_t rtr, ctr;
        salsa_tr_init(&rtr, data, size);
        c_salsa_tr_init(&ctr, data, size);
        for (int step = 0; step < 8 && rtr.pos < size; step++) {
            uint8_t sel = data[step % size] & 7u;
            if (sel == 0) {
                uint32_t rv = 0, cv = 0;
                bool ro = salsa_tr_u32(&rtr, &rv);
                bool co = c_salsa_tr_u32(&ctr, &cv);
                assert(ro == co && rtr.pos == ctr.pos);
                if (ro) assert(rv == cv);
            } else if (sel == 1) {
                size_t roff = 0, rlen = 0, coff = 0, clen = 0;
                bool rnull = false, cnull = false;
                bool ro = salsa_tr_str_slice(&rtr, &roff, &rlen, &rnull);
                bool co = c_salsa_tr_str_slice(&ctr, &coff, &clen, &cnull);
                assert(ro == co && rtr.pos == ctr.pos);
                if (ro) assert(rnull == cnull && roff == coff && rlen == clen);
            } else if (sel == 2) {
                salsa_pa_wire_ss_t rss, css;
                memset(&rss, 0, sizeof(rss));
                memset(&css, 0, sizeof(css));
                bool ro = salsa_tr_ss(&rtr, &rss);
                bool co = c_salsa_tr_ss(&ctr, &css);
                assert(ro == co && rtr.pos == ctr.pos);
                if (ro) assert(memcmp(&rss, &css, sizeof(rss)) == 0);
            } else if (sel == 3) {
                salsa_pa_wire_map_t rm, cm;
                memset(&rm, 0, sizeof(rm));
                memset(&cm, 0, sizeof(cm));
                bool ro = salsa_tr_map(&rtr, &rm);
                bool co = c_salsa_tr_map(&ctr, &cm);
                assert(ro == co && rtr.pos == ctr.pos);
                if (ro) assert(memcmp(&rm, &cm, sizeof(rm)) == 0);
            } else if (sel == 4) {
                salsa_pa_wire_cvol_t rcv, ccv;
                memset(&rcv, 0, sizeof(rcv));
                memset(&ccv, 0, sizeof(ccv));
                bool ro = salsa_tr_cvol(&rtr, &rcv);
                bool co = c_salsa_tr_cvol(&ctr, &ccv);
                assert(ro == co && rtr.pos == ctr.pos);
                if (ro) assert(memcmp(&rcv, &ccv, sizeof(rcv)) == 0);
            } else if (sel == 5) {
                salsa_pa_propbag_t rpb, cpb;
                memset(&rpb, 0, sizeof(rpb));
                memset(&cpb, 0, sizeof(cpb));
                bool ro = salsa_tr_proplist(&rtr, &rpb);
                bool co = c_salsa_tr_proplist(&ctr, &cpb);
                assert(ro == co && rtr.pos == ctr.pos);
                if (ro) assert(memcmp(&rpb, &cpb, sizeof(rpb)) == 0);
            } else if (sel == 6) {
                uint32_t rsec = 0, rusec = 0, csec = 0, cusec = 0;
                bool ro = salsa_tr_timeval(&rtr, &rsec, &rusec);
                bool co = c_salsa_tr_timeval(&ctr, &csec, &cusec);
                assert(ro == co && rtr.pos == ctr.pos);
                if (ro) assert(rsec == csec && rusec == cusec);
            } else {
                bool ro = salsa_tr_skip_format_info(&rtr);
                bool co = c_salsa_tr_skip_format_info(&ctr);
                assert(ro == co && rtr.pos == ctr.pos);
            }
        }
    }

    /* 5. BlueZ A2DP SEP negotiation + BDADDR parser differential check */
    {
        int kind = (int)(data[0] % 5u);
        uint8_t r_cfg[16], c_cfg[16];
        size_t r_len = 0, c_len = 0;
        uint32_t r_bit = 0, c_bit = 0;
        memset(r_cfg, 0, sizeof(r_cfg));
        memset(c_cfg, 0, sizeof(c_cfg));
        int rr = salsa_bt_select_a2dp_config(kind, data + 1, size - 1, r_cfg, 16u, &r_len, &r_bit);
        int cr = c_salsa_bt_select_a2dp_config(kind, data + 1, size - 1, c_cfg, 16u, &c_len, &c_bit);
        assert(rr == cr && r_bit == c_bit && r_len == c_len);
        if (rr == 0) {
            assert(memcmp(r_cfg, c_cfg, r_len) == 0);
        }

        uint8_t rb[6] = {0}, cb[6] = {0};
        bool rmac = salsa_bt_parse_bdaddr((const char *)data, size > 32 ? 32 : size, rb);
        bool cmac = c_salsa_bt_parse_bdaddr((const char *)data, size > 32 ? 32 : size, cb);
        assert(rmac == cmac);
        if (rmac) assert(memcmp(rb, cb, 6) == 0);
    }

    return 0;
}
