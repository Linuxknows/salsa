#include "../include/salsa_wire.h"
#include "../include/lwasapi.h"

#include <limits.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>

/* ============================================================================
 * Shared Helpers
 * ============================================================================ */

static uint32_t sw_read_be32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) |
           ((uint32_t)p[1] << 16) |
           ((uint32_t)p[2] << 8)  |
           ((uint32_t)p[3]);
}

static uint32_t sw_read_le32(const uint8_t *p) {
    return ((uint32_t)p[0])       |
           ((uint32_t)p[1] << 8)  |
           ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static uint64_t sw_read_le64(const uint8_t *p) {
    return ((uint64_t)sw_read_le32(p)) | ((uint64_t)sw_read_le32(p + 4) << 32);
}

static void sw_safe_copy(char *dst, size_t cap, const char *src) {
    if (!dst || cap == 0) return;
    if (!src) {
        dst[0] = '\0';
        return;
    }
    size_t i = 0;
    while (i + 1u < cap && src[i] != '\0') {
        dst[i] = src[i];
        i++;
    }
    dst[i] = '\0';
}

/* ============================================================================
 * 1. PulseAudio v35 Wire Tagstruct Parser
 * ============================================================================ */

void salsa_tr_init(salsa_pa_tagread_t *tr, const uint8_t *p, size_t len) {
    if (!tr) return;
    tr->p = p;
    tr->len = p ? len : 0u;
    tr->pos = 0u;
}

bool salsa_tr_u32(salsa_pa_tagread_t *tr, uint32_t *out) {
    if (!tr || !tr->p || tr->pos > tr->len) return false;
    if (tr->len - tr->pos < 5u || tr->p[tr->pos] != SALSA_PA_TAG_U32) return false;
    if (out) *out = sw_read_be32(tr->p + tr->pos + 1u);
    tr->pos += 5u;
    return true;
}

bool salsa_tr_u8(salsa_pa_tagread_t *tr, uint8_t *out) {
    if (!tr || !tr->p || tr->pos > tr->len) return false;
    if (tr->len - tr->pos < 2u || tr->p[tr->pos] != SALSA_PA_TAG_U8) return false;
    if (out) *out = tr->p[tr->pos + 1u];
    tr->pos += 2u;
    return true;
}

bool salsa_tr_bool(salsa_pa_tagread_t *tr, bool *out) {
    if (!tr || !tr->p || tr->pos >= tr->len) return false;
    uint8_t t = tr->p[tr->pos];
    if (t == SALSA_PA_TAG_BOOLEAN_TRUE) {
        if (out) *out = true;
        tr->pos++;
        return true;
    }
    if (t == SALSA_PA_TAG_BOOLEAN_FALSE) {
        if (out) *out = false;
        tr->pos++;
        return true;
    }
    return false;
}

bool salsa_tr_str_slice(salsa_pa_tagread_t *tr, size_t *out_off, size_t *out_len, bool *out_is_null) {
    if (!tr || !tr->p || tr->pos >= tr->len) return false;
    uint8_t t = tr->p[tr->pos];
    if (t == SALSA_PA_TAG_STRING_NULL) {
        if (out_off) *out_off = 0u;
        if (out_len) *out_len = 0u;
        if (out_is_null) *out_is_null = true;
        tr->pos++;
        return true;
    }
    if (t != SALSA_PA_TAG_STRING) return false;
    size_t start = tr->pos + 1u;
    if (start >= tr->len) return false;
    size_t rem = tr->len - start;
    const uint8_t *nul = (const uint8_t *)memchr(tr->p + start, '\0', rem);
    if (!nul) return false;
    size_t slen = (size_t)(nul - (tr->p + start));
    if (out_off) *out_off = start;
    if (out_len) *out_len = slen;
    if (out_is_null) *out_is_null = false;
    tr->pos = start + slen + 1u;
    return true;
}

bool salsa_tr_arbitrary_slice(salsa_pa_tagread_t *tr, size_t *out_off, size_t *out_len) {
    if (!tr || !tr->p || tr->pos > tr->len) return false;
    size_t avail = tr->len - tr->pos;
    if (avail < 5u || tr->p[tr->pos] != SALSA_PA_TAG_ARBITRARY) return false;
    uint32_t blen = sw_read_be32(tr->p + tr->pos + 1u);
    if ((size_t)blen > avail - 5u) return false;
    if (out_off) *out_off = tr->pos + 5u;
    if (out_len) *out_len = (size_t)blen;
    tr->pos += 5u + (size_t)blen;
    return true;
}

bool salsa_tr_ss(salsa_pa_tagread_t *tr, salsa_pa_wire_ss_t *ss) {
    if (!tr || !tr->p || tr->pos > tr->len) return false;
    if (tr->len - tr->pos < 7u || tr->p[tr->pos] != SALSA_PA_TAG_SAMPLE_SPEC) return false;
    if (ss) {
        ss->format   = tr->p[tr->pos + 1u];
        ss->channels = tr->p[tr->pos + 2u];
        ss->rate     = sw_read_be32(tr->p + tr->pos + 3u);
    }
    tr->pos += 7u;
    return true;
}

bool salsa_tr_map(salsa_pa_tagread_t *tr, salsa_pa_wire_map_t *m) {
    if (!tr || !tr->p || tr->pos > tr->len) return false;
    size_t avail = tr->len - tr->pos;
    if (avail < 2u || tr->p[tr->pos] != SALSA_PA_TAG_CHANNEL_MAP) return false;
    uint8_t ch = tr->p[tr->pos + 1u];
    if ((size_t)ch > avail - 2u) return false;
    if (m) {
        uint8_t copy_n = (ch > 32u) ? 32u : ch;
        memset(m->map, 0, sizeof(m->map));
        m->channels = copy_n;
        if (copy_n > 0u) memcpy(m->map, tr->p + tr->pos + 2u, copy_n);
    }
    tr->pos += 2u + (size_t)ch;
    return true;
}

bool salsa_tr_cvol(salsa_pa_tagread_t *tr, salsa_pa_wire_cvol_t *cv) {
    if (!tr || !tr->p || tr->pos > tr->len) return false;
    size_t avail = tr->len - tr->pos;
    if (avail < 2u || tr->p[tr->pos] != SALSA_PA_TAG_CVOLUME) return false;
    uint8_t ch = tr->p[tr->pos + 1u];
    size_t need = (size_t)ch * 4u;
    if (need > avail - 2u) return false;
    if (cv) {
        uint8_t copy_n = (ch > 32u) ? 32u : ch;
        memset(cv->values, 0, sizeof(cv->values));
        cv->channels = copy_n;
        for (uint8_t i = 0; i < copy_n; i++) {
            cv->values[i] = sw_read_be32(tr->p + tr->pos + 2u + (size_t)i * 4u);
        }
    }
    tr->pos += 2u + need;
    return true;
}

bool salsa_tr_timeval(salsa_pa_tagread_t *tr, uint32_t *sec, uint32_t *usec) {
    if (!tr || !tr->p || tr->pos > tr->len) return false;
    if (tr->len - tr->pos < 9u || tr->p[tr->pos] != SALSA_PA_TAG_TIMEVAL) return false;
    if (sec)  *sec  = sw_read_be32(tr->p + tr->pos + 1u);
    if (usec) *usec = sw_read_be32(tr->p + tr->pos + 5u);
    tr->pos += 9u;
    return true;
}

void salsa_propbag_set(salsa_pa_propbag_t *pb, const char *k, const char *v) {
    if (!pb || !k || !v) return;
    int32_t cnt = pb->count;
    if (cnt < 0) cnt = 0;
    if (cnt > 24) cnt = 24;
    for (int32_t i = 0; i < cnt; i++) {
        if (strncmp(pb->items[i].key, k, sizeof(pb->items[i].key)) == 0) {
            sw_safe_copy(pb->items[i].val, sizeof(pb->items[i].val), v);
            return;
        }
    }
    if (cnt < 24) {
        sw_safe_copy(pb->items[cnt].key, sizeof(pb->items[cnt].key), k);
        sw_safe_copy(pb->items[cnt].val, sizeof(pb->items[cnt].val), v);
        pb->count = cnt + 1;
    }
}

bool salsa_tr_proplist(salsa_pa_tagread_t *tr, salsa_pa_propbag_t *pb) {
    if (!tr || !tr->p || tr->pos >= tr->len || tr->p[tr->pos] != SALSA_PA_TAG_PROPLIST) return false;
    tr->pos++;
    /* Cap decoded proplist pairs at 512 to prevent adversarial CPU amplification */
    for (size_t iters = 0; iters < 512u && tr->pos < tr->len; iters++) {
        size_t k_off = 0, k_len = 0;
        bool k_null = false;
        if (!salsa_tr_str_slice(tr, &k_off, &k_len, &k_null)) return false;
        if (k_null) return true;
        uint32_t vlen = 0;
        size_t a_off = 0, a_len = 0;
        if (!salsa_tr_u32(tr, &vlen)) return false;
        if (!salsa_tr_arbitrary_slice(tr, &a_off, &a_len)) return false;
        if (pb && a_len > 0u) {
            char kbuf[64];
            size_t kc = k_len < sizeof(kbuf) - 1u ? k_len : sizeof(kbuf) - 1u;
            memcpy(kbuf, tr->p + k_off, kc);
            kbuf[kc] = '\0';

            char tmp[128];
            size_t c = a_len < sizeof(tmp) - 1u ? a_len : sizeof(tmp) - 1u;
            memcpy(tmp, tr->p + a_off, c);
            tmp[c] = '\0';
            salsa_propbag_set(pb, kbuf, tmp);
        }
    }
    return false;
}

bool salsa_tr_skip_format_info(salsa_pa_tagread_t *tr) {
    if (!tr || !tr->p || tr->pos >= tr->len || tr->p[tr->pos] != SALSA_PA_TAG_FORMAT_INFO) return false;
    tr->pos++;
    uint8_t enc = 0;
    if (!salsa_tr_u8(tr, &enc)) return false;
    return salsa_tr_proplist(tr, NULL);
}

/* ============================================================================
 * 2. PipeWire Video Broker Wire Framing & SPA Pod Parser
 * ============================================================================ */

static size_t sw_pad8(size_t n) {
    if (n > SIZE_MAX - 7u) return n;
    return (n + 7u) & ~(size_t)7u;
}

bool salsa_pwb_parse_pkt_hdr(const uint8_t *buf, size_t avail,
                             uint32_t *out_oid, uint8_t *out_op,
                             uint32_t *out_psz, uint32_t *out_seq,
                             uint32_t *out_nfds, size_t *out_pkt_len) {
    if (!buf || avail < 16u) return false;
    uint32_t oid  = sw_read_le32(buf);
    uint32_t w1   = sw_read_le32(buf + 4u);
    uint32_t seq  = sw_read_le32(buf + 8u);
    uint32_t nfds = sw_read_le32(buf + 12u);
    uint8_t  op   = (uint8_t)((w1 >> 24) & 0xffu);
    uint32_t psz  = w1 & 0x00ffffffu;
    if (nfds > 256u || psz > SALSA_PWB_MAX_PACKET_SZ) return false;
    size_t pkt_len = 16u + sw_pad8((size_t)psz);
    if (avail < pkt_len) return false;
    if (out_oid)     *out_oid = oid;
    if (out_op)      *out_op  = op;
    if (out_psz)     *out_psz = psz;
    if (out_seq)     *out_seq = seq;
    if (out_nfds)    *out_nfds = nfds;
    if (out_pkt_len) *out_pkt_len = pkt_len;
    return true;
}

void salsa_pwb_cur_init(salsa_pwb_cur_t *c, const void *data, size_t len) {
    if (!c) return;
    c->base = (const uint8_t *)data;
    c->off = 0u;
    c->end = data ? len : 0u;
}

bool salsa_pwb_cur_next_pod(salsa_pwb_cur_t *c, uint32_t *out_sz, uint32_t *out_tp,
                            size_t *out_body_off, size_t *out_pod_total) {
    if (!c || !c->base || c->off > c->end || c->end - c->off < 8u) return false;
    uint32_t sz = sw_read_le32(c->base + c->off);
    uint32_t tp = sw_read_le32(c->base + c->off + 4u);
    size_t rem_after_hdr = c->end - c->off - 8u;
    if ((size_t)sz > rem_after_hdr) return false;
    size_t unpadded = 8u + (size_t)sz;
    size_t total = sw_pad8(unpadded);
    if (total < unpadded || total > c->end - c->off) total = unpadded;
    if (out_sz)        *out_sz = sz;
    if (out_tp)        *out_tp = tp;
    if (out_body_off)  *out_body_off = c->off + 8u;
    if (out_pod_total) *out_pod_total = unpadded;
    c->off += total;
    return true;
}

bool salsa_pwb_cur_enter_struct(salsa_pwb_cur_t *c, salsa_pwb_cur_t *inner) {
    if (!c || !inner) return false;
    uint32_t sz = 0, tp = 0;
    size_t body_off = 0;
    if (!salsa_pwb_cur_next_pod(c, &sz, &tp, &body_off, NULL)) return false;
    if (tp != SALSA_PWB_SPA_STRUCT) return false;
    inner->base = c->base + body_off;
    inner->off = 0u;
    inner->end = (size_t)sz;
    return true;
}

bool salsa_pwb_cur_int(salsa_pwb_cur_t *c, int32_t *out) {
    uint32_t sz = 0, tp = 0;
    size_t body_off = 0;
    if (!salsa_pwb_cur_next_pod(c, &sz, &tp, &body_off, NULL)) return false;
    if ((tp != SALSA_PWB_SPA_INT && tp != SALSA_PWB_SPA_ID && tp != SALSA_PWB_SPA_BOOL) || sz < 4u) {
        return false;
    }
    if (out) *out = (int32_t)sw_read_le32(c->base + body_off);
    return true;
}

bool salsa_pwb_cur_u32(salsa_pwb_cur_t *c, uint32_t *out) {
    int32_t v = 0;
    if (!salsa_pwb_cur_int(c, &v)) return false;
    if (out) *out = (uint32_t)v;
    return true;
}

bool salsa_pwb_cur_long(salsa_pwb_cur_t *c, int64_t *out) {
    uint32_t sz = 0, tp = 0;
    size_t body_off = 0;
    if (!salsa_pwb_cur_next_pod(c, &sz, &tp, &body_off, NULL)) return false;
    if ((tp != SALSA_PWB_SPA_LONG && tp != SALSA_PWB_SPA_FD) || sz < 8u) return false;
    if (out) *out = (int64_t)sw_read_le64(c->base + body_off);
    return true;
}

bool salsa_pwb_cur_str(salsa_pwb_cur_t *c, char *out, size_t out_sz) {
    uint32_t sz = 0, tp = 0;
    size_t body_off = 0;
    if (!salsa_pwb_cur_next_pod(c, &sz, &tp, &body_off, NULL)) return false;
    if (tp == SALSA_PWB_SPA_NONE) {
        if (out && out_sz > 0u) out[0] = '\0';
        return true;
    }
    if (tp != SALSA_PWB_SPA_STRING) return false;
    if (out && out_sz > 0u) {
        size_t n = 0u;
        if (sz > 0u) {
            const uint8_t *body = c->base + body_off;
            const uint8_t *nul = (const uint8_t *)memchr(body, '\0', (size_t)sz);
            n = nul ? (size_t)(nul - body) : (size_t)sz;
        }
        if (n >= out_sz) n = out_sz - 1u;
        if (n > 0u) memcpy(out, c->base + body_off, n);
        out[n] = '\0';
    }
    return true;
}

void salsa_pwb_dict_set(salsa_pwb_dict_t *d, const char *key, const char *val) {
    if (!d || !key || !key[0]) return;
    if (!val) val = "";
    uint32_t cnt = d->count;
    if (cnt > SALSA_PWB_MAX_PROPS) cnt = SALSA_PWB_MAX_PROPS;
    for (uint32_t i = 0; i < cnt; i++) {
        if (strncmp(d->items[i].key, key, sizeof(d->items[i].key)) == 0) {
            sw_safe_copy(d->items[i].val, sizeof(d->items[i].val), val);
            return;
        }
    }
    if (cnt < SALSA_PWB_MAX_PROPS) {
        sw_safe_copy(d->items[cnt].key, sizeof(d->items[cnt].key), key);
        sw_safe_copy(d->items[cnt].val, sizeof(d->items[cnt].val), val);
        d->count = cnt + 1u;
    }
}

bool salsa_pwb_cur_dict(salsa_pwb_cur_t *c, salsa_pwb_dict_t *d, bool wrapped_in_struct) {
    salsa_pwb_cur_t dc;
    salsa_pwb_cur_t *p = c;
    if (wrapped_in_struct) {
        if (!salsa_pwb_cur_enter_struct(c, &dc)) return false;
        p = &dc;
    }
    int32_t n = 0;
    if (!salsa_pwb_cur_int(p, &n) || n < 0 || n > 1024) return false;
    for (int32_t i = 0; i < n; i++) {
        char k[64], v[160];
        if (!salsa_pwb_cur_str(p, k, sizeof(k)) || !salsa_pwb_cur_str(p, v, sizeof(v))) return false;
        salsa_pwb_dict_set(d, k, v);
    }
    return true;
}

static void parse_choice_u32(uint32_t tp, const uint8_t *body, uint32_t sz,
                             uint32_t *arr, uint32_t max_arr, uint32_t *out_cnt,
                             uint32_t *out_def, uint32_t *out_min, uint32_t *out_max,
                             bool *out_is_range) {
    if (!body) return;
    if (tp == SALSA_PWB_SPA_ID || tp == SALSA_PWB_SPA_INT) {
        if (sz >= 4u) {
            uint32_t v = sw_read_le32(body);
            if (out_def) *out_def = v;
            if (out_min) *out_min = v;
            if (out_max) *out_max = v;
            if (arr && max_arr > 0u) {
                arr[0] = v;
                if (out_cnt) *out_cnt = 1u;
            }
        }
        return;
    }
    if (tp == SALSA_PWB_SPA_CHOICE && sz >= 16u) {
        uint32_t ctype = sw_read_le32(body);
        uint32_t esz   = sw_read_le32(body + 8u);
        uint32_t etp   = sw_read_le32(body + 12u);
        if ((etp == SALSA_PWB_SPA_ID || etp == SALSA_PWB_SPA_INT) && esz == 4u) {
            uint32_t n = (sz - 16u) / 4u;
            if (n > 0u) {
                uint32_t v0 = sw_read_le32(body + 16u);
                if (out_def) *out_def = v0;
                if (ctype == SALSA_PWB_CHOICE_Range && n >= 3u) {
                    uint32_t vmin = sw_read_le32(body + 20u);
                    uint32_t vmax = sw_read_le32(body + 24u);
                    if (out_min) *out_min = vmin;
                    if (out_max) *out_max = vmax;
                    if (out_is_range) *out_is_range = true;
                } else {
                    if (out_min) *out_min = v0;
                    if (out_max) *out_max = v0;
                    if (arr && max_arr > 0u) {
                        uint32_t c = 0u;
                        uint32_t start = (n > 1u) ? 1u : 0u;
                        arr[c++] = v0;
                        for (uint32_t i = start; i < n && c < max_arr; i++) {
                            uint32_t vi = sw_read_le32(body + 16u + (size_t)i * 4u);
                            bool dup = false;
                            for (uint32_t k = 0; k < c; k++) {
                                if (arr[k] == vi) { dup = true; break; }
                            }
                            if (!dup) arr[c++] = vi;
                        }
                        if (out_cnt) *out_cnt = c;
                    }
                }
            }
        }
    }
}

static void parse_choice_long(uint32_t tp, const uint8_t *body, uint32_t sz,
                              uint64_t *arr, uint32_t max_arr, uint32_t *out_cnt) {
    if (!body) return;
    if (tp == SALSA_PWB_SPA_LONG && sz >= 8u) {
        if (arr && max_arr > 0u) {
            arr[0] = sw_read_le64(body);
            if (out_cnt) *out_cnt = 1u;
        }
        return;
    }
    if (tp == SALSA_PWB_SPA_CHOICE && sz >= 16u) {
        uint32_t esz = sw_read_le32(body + 8u);
        uint32_t etp = sw_read_le32(body + 12u);
        if (etp == SALSA_PWB_SPA_LONG && esz == 8u) {
            uint32_t n = (sz - 16u) / 8u;
            uint32_t c = 0u;
            for (uint32_t i = 0; i < n && arr && c < max_arr; i++) {
                uint64_t vi = sw_read_le64(body + 16u + (size_t)i * 8u);
                bool dup = false;
                for (uint32_t k = 0; k < c; k++) {
                    if (arr[k] == vi) { dup = true; break; }
                }
                if (!dup) arr[c++] = vi;
            }
            if (out_cnt) *out_cnt = c;
        }
    }
}

static void parse_choice_rect(uint32_t tp, const uint8_t *body, uint32_t sz,
                              uint32_t *wd, uint32_t *wmin, uint32_t *wmax,
                              uint32_t *hd, uint32_t *hmin, uint32_t *hmax) {
    if (!body) return;
    if (tp == SALSA_PWB_SPA_RECT && sz >= 8u) {
        uint32_t w = sw_read_le32(body);
        uint32_t h = sw_read_le32(body + 4u);
        *wd = *wmin = *wmax = w;
        *hd = *hmin = *hmax = h;
        return;
    }
    if (tp == SALSA_PWB_SPA_CHOICE && sz >= 16u) {
        uint32_t ctype = sw_read_le32(body);
        uint32_t esz   = sw_read_le32(body + 8u);
        uint32_t etp   = sw_read_le32(body + 12u);
        if (etp == SALSA_PWB_SPA_RECT && esz == 8u) {
            uint32_t n = (sz - 16u) / 8u;
            if (n >= 1u) {
                uint32_t w0 = sw_read_le32(body + 16u);
                uint32_t h0 = sw_read_le32(body + 20u);
                *wd = *wmin = *wmax = w0;
                *hd = *hmin = *hmax = h0;
                if (ctype == SALSA_PWB_CHOICE_Range && n >= 3u) {
                    *wmin = sw_read_le32(body + 24u);
                    *hmin = sw_read_le32(body + 28u);
                    *wmax = sw_read_le32(body + 32u);
                    *hmax = sw_read_le32(body + 36u);
                } else if (ctype == SALSA_PWB_CHOICE_Enum && n > 1u) {
                    for (uint32_t i = 1u; i < n; i++) {
                        uint32_t wi = sw_read_le32(body + 16u + (size_t)i * 8u);
                        uint32_t hi = sw_read_le32(body + 20u + (size_t)i * 8u);
                        if (wi < *wmin) *wmin = wi;
                        if (wi > *wmax) *wmax = wi;
                        if (hi < *hmin) *hmin = hi;
                        if (hi > *hmax) *hmax = hi;
                    }
                }
            }
        }
    }
}

static void parse_choice_frac(uint32_t tp, const uint8_t *body, uint32_t sz,
                              salsa_pwb_frac_t *fdef, salsa_pwb_frac_t *fmin, salsa_pwb_frac_t *fmax) {
    if (!body) return;
    if (tp == SALSA_PWB_SPA_FRAC && sz >= 8u) {
        salsa_pwb_frac_t f = { sw_read_le32(body), sw_read_le32(body + 4u) };
        if (f.denom == 0u) f.denom = 1u;
        *fdef = *fmin = *fmax = f;
        return;
    }
    if (tp == SALSA_PWB_SPA_CHOICE && sz >= 16u) {
        uint32_t ctype = sw_read_le32(body);
        uint32_t esz   = sw_read_le32(body + 8u);
        uint32_t etp   = sw_read_le32(body + 12u);
        if (etp == SALSA_PWB_SPA_FRAC && esz == 8u) {
            uint32_t n = (sz - 16u) / 8u;
            if (n >= 1u) {
                salsa_pwb_frac_t f0 = { sw_read_le32(body + 16u), sw_read_le32(body + 20u) };
                if (f0.denom == 0u) f0.denom = 1u;
                *fdef = *fmin = *fmax = f0;
                if (ctype == SALSA_PWB_CHOICE_Range && n >= 3u) {
                    fmin->num   = sw_read_le32(body + 24u);
                    fmin->denom = sw_read_le32(body + 28u);
                    fmax->num   = sw_read_le32(body + 32u);
                    fmax->denom = sw_read_le32(body + 36u);
                    if (fmin->denom == 0u) fmin->denom = 1u;
                    if (fmax->denom == 0u) fmax->denom = 1u;
                    if ((uint64_t)fmin->num * (uint64_t)fmax->denom >
                        (uint64_t)fmax->num * (uint64_t)fmin->denom) {
                        salsa_pwb_frac_t tmp = *fmin;
                        *fmin = *fmax;
                        *fmax = tmp;
                    }
                } else if (ctype == SALSA_PWB_CHOICE_Enum && n > 1u) {
                    for (uint32_t i = 1u; i < n; i++) {
                        salsa_pwb_frac_t fi = {
                            sw_read_le32(body + 16u + (size_t)i * 8u),
                            sw_read_le32(body + 20u + (size_t)i * 8u)
                        };
                        if (fi.denom == 0u) fi.denom = 1u;
                        if ((uint64_t)fi.num * (uint64_t)fmin->denom < (uint64_t)fmin->num * (uint64_t)fi.denom) {
                            *fmin = fi;
                        }
                        if ((uint64_t)fi.num * (uint64_t)fmax->denom > (uint64_t)fmax->num * (uint64_t)fi.denom) {
                            *fmax = fi;
                        }
                    }
                }
            }
        }
    }
}

void salsa_pwb_parse_param_pod(const uint8_t *pod, size_t pod_sz, salsa_pwb_param_parsed_t *out) {
    if (!out) return;
    memset(out, 0, sizeof(*out));
    if (!pod || pod_sz < 16u) return;
    uint32_t sz = sw_read_le32(pod);
    uint32_t tp = sw_read_le32(pod + 4u);
    /* Prevent uint32 wrap on 8u + sz and underflow on sz - 8u */
    if (tp != SALSA_PWB_SPA_OBJECT || sz < 8u || (size_t)sz > pod_sz - 8u) return;
    uint32_t otype = sw_read_le32(pod + 8u);
    uint32_t oid   = sw_read_le32(pod + 12u);

    if (otype == SALSA_PWB_OBJ_FORMAT &&
        (oid == SALSA_PWB_PARAM_EnumFormat || oid == SALSA_PWB_PARAM_Format)) {
        salsa_pwb_vfmt_caps_t c;
        memset(&c, 0, sizeof(c));
        c.w_def = 1920u; c.w_min = 1u; c.w_max = 16384u;
        c.h_def = 1080u; c.h_min = 1u; c.h_max = 16384u;
        c.fps_def = (salsa_pwb_frac_t){0u, 1u};
        c.fps_min = (salsa_pwb_frac_t){0u, 1u};
        c.fps_max = (salsa_pwb_frac_t){0u, 1u};
        c.max_fps_def = (salsa_pwb_frac_t){60u, 1u};
        c.max_fps_min = (salsa_pwb_frac_t){1u, 1u};
        c.max_fps_max = (salsa_pwb_frac_t){240u, 1u};
        uint32_t mtype = 0u, msub = 0u;
        salsa_pwb_cur_t p;
        salsa_pwb_cur_init(&p, pod + 16u, (size_t)sz - 8u);
        while (p.off <= p.end && p.end - p.off >= 8u) {
            uint32_t k  = sw_read_le32(p.base + p.off);
            uint32_t fl = sw_read_le32(p.base + p.off + 4u);
            p.off += 8u;
            uint32_t vsz = 0u, vtp = 0u;
            size_t body_off = 0u;
            if (!salsa_pwb_cur_next_pod(&p, &vsz, &vtp, &body_off, NULL)) break;
            const uint8_t *vbody = p.base + body_off;
            if (k == 1u) {
                parse_choice_u32(vtp, vbody, vsz, NULL, 0u, NULL, &mtype, NULL, NULL, NULL);
            } else if (k == 2u) {
                parse_choice_u32(vtp, vbody, vsz, NULL, 0u, NULL, &msub, NULL, NULL, NULL);
            } else if (k == 0x20001u) {
                parse_choice_u32(vtp, vbody, vsz, c.formats, 32u, &c.n_formats, NULL, NULL, NULL, NULL);
            } else if (k == 0x20002u) {
                parse_choice_long(vtp, vbody, vsz, c.modifiers, SALSA_PWB_MAX_MODIFIERS, &c.n_modifiers);
                c.has_modifier = (c.n_modifiers > 0u);
                c.mod_dont_fixate = c.has_modifier && ((fl & SALSA_PWB_POD_PROP_FLAG_DONT_FIXATE) != 0u);
            } else if (k == 0x20003u) {
                parse_choice_rect(vtp, vbody, vsz, &c.w_def, &c.w_min, &c.w_max, &c.h_def, &c.h_min, &c.h_max);
            } else if (k == 0x20004u) {
                parse_choice_frac(vtp, vbody, vsz, &c.fps_def, &c.fps_min, &c.fps_max);
                c.has_fps = true;
            } else if (k == 0x20005u) {
                parse_choice_frac(vtp, vbody, vsz, &c.max_fps_def, &c.max_fps_min, &c.max_fps_max);
                c.has_max_fps = true;
            }
        }
        if ((mtype == 0u || mtype == SALSA_PWB_MEDIA_Video) && (msub == 0u || msub == SALSA_PWB_SUB_Raw)) {
            if (c.n_formats == 0u) {
                c.formats[0] = SALSA_PWB_VF_BGRx;
                c.formats[1] = SALSA_PWB_VF_BGRA;
                c.n_formats = 2u;
            }
            c.valid = true;
            out->kind = SALSA_PWB_PARSED_VFMT;
            out->vfmt = c;
        }
    } else if (otype == SALSA_PWB_OBJ_PARAM_BUFS && oid == SALSA_PWB_PARAM_Buffers) {
        salsa_pwb_buf_caps_t bc;
        memset(&bc, 0, sizeof(bc));
        bc.buf_def = 3u; bc.buf_min = 2u; bc.buf_max = 8u;
        bc.datatype_mask = (1u << SALSA_PWB_DATA_MemFd);
        salsa_pwb_cur_t p;
        salsa_pwb_cur_init(&p, pod + 16u, (size_t)sz - 8u);
        while (p.off <= p.end && p.end - p.off >= 8u) {
            uint32_t k = sw_read_le32(p.base + p.off);
            p.off += 8u;
            uint32_t vsz = 0u, vtp = 0u;
            size_t body_off = 0u;
            if (!salsa_pwb_cur_next_pod(&p, &vsz, &vtp, &body_off, NULL)) break;
            const uint8_t *vbody = p.base + body_off;
            if (k == 1u) parse_choice_u32(vtp, vbody, vsz, NULL, 0u, NULL, &bc.buf_def, &bc.buf_min, &bc.buf_max, NULL);
            else if (k == 3u) parse_choice_u32(vtp, vbody, vsz, NULL, 0u, NULL, &bc.size_def, &bc.size_min, &bc.size_max, NULL);
            else if (k == 4u) parse_choice_u32(vtp, vbody, vsz, NULL, 0u, NULL, &bc.stride_def, NULL, NULL, NULL);
            else if (k == 6u) parse_choice_u32(vtp, vbody, vsz, NULL, 0u, NULL, &bc.datatype_mask, NULL, NULL, NULL);
        }
        bc.valid = true;
        out->kind = SALSA_PWB_PARSED_BUFS;
        out->bcaps = bc;
    }
}

static uint32_t vfmt_bpp_num(uint32_t f) {
    switch (f) {
        case SALSA_PWB_VF_BGRx: case SALSA_PWB_VF_BGRA:
        case SALSA_PWB_VF_RGBx: case SALSA_PWB_VF_RGBA:
        case SALSA_PWB_VF_xRGB: case SALSA_PWB_VF_xBGR:
        case SALSA_PWB_VF_ARGB: case SALSA_PWB_VF_ABGR: return 4u;
        case SALSA_PWB_VF_RGB:  case SALSA_PWB_VF_BGR:  return 3u;
        case SALSA_PWB_VF_YUY2:                         return 2u;
        case SALSA_PWB_VF_NV12: case SALSA_PWB_VF_I420: return 3u;
        default: return 4u;
    }
}

uint32_t salsa_pwb_vfmt_stride(uint32_t f, uint32_t w) {
    if (f == SALSA_PWB_VF_NV12 || f == SALSA_PWB_VF_I420) return w;
    uint64_t s = (uint64_t)w * (uint64_t)vfmt_bpp_num(f);
    return (s > UINT32_MAX) ? UINT32_MAX : (uint32_t)s;
}

uint32_t salsa_pwb_vfmt_size(uint32_t f, uint32_t w, uint32_t h) {
    uint64_t total;
    if (f == SALSA_PWB_VF_NV12 || f == SALSA_PWB_VF_I420) {
        total = ((uint64_t)w * (uint64_t)h * 3ULL) / 2ULL;
    } else {
        total = (uint64_t)salsa_pwb_vfmt_stride(f, w) * (uint64_t)h;
    }
    return (total > UINT32_MAX) ? UINT32_MAX : (uint32_t)total;
}

/* ============================================================================
 * 3. BlueZ A2DP Capability Selector & Strict MAC Parser
 * ============================================================================ */

#define SW_SBC_FREQ_16000   (1u << 3)
#define SW_SBC_FREQ_32000   (1u << 2)
#define SW_SBC_FREQ_44100   (1u << 1)
#define SW_SBC_FREQ_48000   (1u << 0)

#define SW_SBC_CH_MONO         (1u << 3)
#define SW_SBC_CH_DUAL         (1u << 2)
#define SW_SBC_CH_STEREO       (1u << 1)
#define SW_SBC_CH_JOINT_STEREO (1u << 0)

#define SW_SBC_BLK_4        (1u << 3)
#define SW_SBC_BLK_8        (1u << 2)
#define SW_SBC_BLK_12       (1u << 1)
#define SW_SBC_BLK_16       (1u << 0)

#define SW_SBC_SUB_4        (1u << 1)
#define SW_SBC_SUB_8        (1u << 0)
#define SW_SBC_ALLOC_SNR    (1u << 1)
#define SW_SBC_ALLOC_LOUD   (1u << 0)

#define SW_AAC_OBJ_MP2_LC   0x80u
#define SW_AAC_OBJ_MP4_LC   0x40u
#define SW_AAC_FREQ_44100   0x0010u
#define SW_AAC_FREQ_48000   0x0008u
#define SW_AAC_CH_1         0x08u
#define SW_AAC_CH_2         0x04u

#define SW_APTX_VENDOR_ID   0x0000004Fu
#define SW_APTX_CODEC_ID    0x0001u
#define SW_APTX_HD_VID      0x000000D7u
#define SW_APTX_HD_CID      0x0024u
#define SW_APTX_FREQ_16000  0x08u
#define SW_APTX_FREQ_32000  0x04u
#define SW_APTX_FREQ_44100  0x02u
#define SW_APTX_FREQ_48000  0x01u
#define SW_APTX_CH_MONO     0x01u
#define SW_APTX_CH_STEREO   0x02u

#define SW_LDAC_VENDOR_ID   0x0000012Du
#define SW_LDAC_CODEC_ID    0x00AAu
#define SW_LDAC_FREQ_44100  0x20u
#define SW_LDAC_FREQ_48000  0x10u
#define SW_LDAC_FREQ_88200  0x08u
#define SW_LDAC_FREQ_96000  0x04u
#define SW_LDAC_CH_MONO     0x04u
#define SW_LDAC_CH_DUAL     0x02u
#define SW_LDAC_CH_STEREO   0x01u

static uint32_t sw_a2dp_vid(const uint8_t *hdr) {
    return ((uint32_t)hdr[0])       |
           ((uint32_t)hdr[1] << 8)  |
           ((uint32_t)hdr[2] << 16) |
           ((uint32_t)hdr[3] << 24);
}

static uint16_t sw_a2dp_cid(const uint8_t *hdr) {
    return (uint16_t)(((uint16_t)hdr[4]) | (((uint16_t)hdr[5]) << 8));
}

static void sw_a2dp_write_hdr(uint8_t *hdr, uint32_t vid, uint16_t cid) {
    hdr[0] = (uint8_t)((vid)       & 0xFFu);
    hdr[1] = (uint8_t)((vid >> 8)  & 0xFFu);
    hdr[2] = (uint8_t)((vid >> 16) & 0xFFu);
    hdr[3] = (uint8_t)((vid >> 24) & 0xFFu);
    hdr[4] = (uint8_t)((cid)       & 0xFFu);
    hdr[5] = (uint8_t)((cid >> 8)  & 0xFFu);
}

int salsa_bt_select_a2dp_config(int codec_kind, const uint8_t *rcaps, size_t rlen,
                                uint8_t *out_cfg, size_t out_cap,
                                size_t *out_len, uint32_t *out_cap_bit) {
    if (!rcaps || rlen == 0u || !out_cfg || !out_len) return -1;
    *out_len = 0u;
    if (out_cap_bit) *out_cap_bit = 0u;

    if (codec_kind == SALSA_BT_WIRE_LDAC && rlen >= 8u && out_cap >= 8u) {
        if (sw_a2dp_vid(rcaps) == SW_LDAC_VENDOR_ID && sw_a2dp_cid(rcaps) == SW_LDAC_CODEC_ID) {
            if (out_cap_bit) *out_cap_bit = LWASAPI_BT_CAP_LDAC;
            uint8_t in_f = rcaps[6], in_c = rcaps[7];
            uint8_t freq = (in_f & SW_LDAC_FREQ_96000) ? SW_LDAC_FREQ_96000
                         : (in_f & SW_LDAC_FREQ_88200) ? SW_LDAC_FREQ_88200
                         : (in_f & SW_LDAC_FREQ_48000) ? SW_LDAC_FREQ_48000
                         : (in_f & SW_LDAC_FREQ_44100) ? SW_LDAC_FREQ_44100 : 0u;
            uint8_t ch   = (in_c & SW_LDAC_CH_STEREO) ? SW_LDAC_CH_STEREO
                         : (in_c & SW_LDAC_CH_DUAL)   ? SW_LDAC_CH_DUAL
                         : (in_c & SW_LDAC_CH_MONO)   ? SW_LDAC_CH_MONO : 0u;
            if (freq && ch) {
                memset(out_cfg, 0, 8u);
                sw_a2dp_write_hdr(out_cfg, SW_LDAC_VENDOR_ID, SW_LDAC_CODEC_ID);
                out_cfg[6] = freq;
                out_cfg[7] = ch;
                *out_len = 8u;
                return 0;
            }
        }
    } else if (codec_kind == SALSA_BT_WIRE_APTX_HD && rlen >= 11u && out_cap >= 11u) {
        if (sw_a2dp_vid(rcaps) == SW_APTX_HD_VID && sw_a2dp_cid(rcaps) == SW_APTX_HD_CID) {
            if (out_cap_bit) *out_cap_bit = LWASAPI_BT_CAP_APTX_HD;
            uint8_t in_b = rcaps[6];
            uint8_t in_c = (uint8_t)(in_b & 0x0Fu);
            uint8_t in_f = (uint8_t)((in_b >> 4) & 0x0Fu);
            uint8_t freq = (in_f & SW_APTX_FREQ_48000) ? SW_APTX_FREQ_48000
                         : (in_f & SW_APTX_FREQ_44100) ? SW_APTX_FREQ_44100
                         : (in_f & SW_APTX_FREQ_32000) ? SW_APTX_FREQ_32000
                         : (in_f & SW_APTX_FREQ_16000) ? SW_APTX_FREQ_16000 : 0u;
            uint8_t ch   = (in_c & SW_APTX_CH_STEREO) ? SW_APTX_CH_STEREO
                         : (in_c & SW_APTX_CH_MONO)   ? SW_APTX_CH_MONO : 0u;
            if (freq && ch) {
                memset(out_cfg, 0, 11u);
                sw_a2dp_write_hdr(out_cfg, SW_APTX_HD_VID, SW_APTX_HD_CID);
                out_cfg[6] = (uint8_t)(((freq & 0x0Fu) << 4) | (ch & 0x0Fu));
                *out_len = 11u;
                return 0;
            }
        }
    } else if (codec_kind == SALSA_BT_WIRE_APTX && rlen >= 7u && out_cap >= 7u) {
        if (sw_a2dp_vid(rcaps) == SW_APTX_VENDOR_ID && sw_a2dp_cid(rcaps) == SW_APTX_CODEC_ID) {
            if (out_cap_bit) *out_cap_bit = LWASAPI_BT_CAP_APTX;
            uint8_t in_b = rcaps[6];
            uint8_t in_c = (uint8_t)(in_b & 0x0Fu);
            uint8_t in_f = (uint8_t)((in_b >> 4) & 0x0Fu);
            uint8_t freq = (in_f & SW_APTX_FREQ_48000) ? SW_APTX_FREQ_48000
                         : (in_f & SW_APTX_FREQ_44100) ? SW_APTX_FREQ_44100
                         : (in_f & SW_APTX_FREQ_32000) ? SW_APTX_FREQ_32000
                         : (in_f & SW_APTX_FREQ_16000) ? SW_APTX_FREQ_16000 : 0u;
            uint8_t ch   = (in_c & SW_APTX_CH_STEREO) ? SW_APTX_CH_STEREO
                         : (in_c & SW_APTX_CH_MONO)   ? SW_APTX_CH_MONO : 0u;
            if (freq && ch) {
                memset(out_cfg, 0, 7u);
                sw_a2dp_write_hdr(out_cfg, SW_APTX_VENDOR_ID, SW_APTX_CODEC_ID);
                out_cfg[6] = (uint8_t)(((freq & 0x0Fu) << 4) | (ch & 0x0Fu));
                *out_len = 7u;
                return 0;
            }
        }
    } else if (codec_kind == SALSA_BT_WIRE_AAC && rlen >= 6u && out_cap >= 6u) {
        if (out_cap_bit) *out_cap_bit = LWASAPI_BT_CAP_AAC;
        uint8_t obj_in = rcaps[0];
        uint8_t obj = (obj_in & SW_AAC_OBJ_MP2_LC) ? SW_AAC_OBJ_MP2_LC
                    : (obj_in & SW_AAC_OBJ_MP4_LC) ? SW_AAC_OBJ_MP4_LC : obj_in;
        uint16_t infreq = (uint16_t)(((uint16_t)rcaps[1] << 4) | (((uint16_t)rcaps[2] >> 4) & 0x0Fu));
        uint16_t sfreq  = (infreq & SW_AAC_FREQ_48000) ? SW_AAC_FREQ_48000
                        : (infreq & SW_AAC_FREQ_44100) ? SW_AAC_FREQ_44100 : SW_AAC_FREQ_48000;
        uint8_t ch_in = (uint8_t)(rcaps[2] & 0x0Fu);
        uint8_t ch = (ch_in & SW_AAC_CH_2) ? SW_AAC_CH_2 : SW_AAC_CH_1;
        uint8_t br1_in = (uint8_t)(rcaps[3] & 0x7Fu);
        uint8_t vbr    = (uint8_t)((rcaps[3] >> 7) & 0x01u);
        uint32_t max_bps = ((uint32_t)br1_in << 16) | ((uint32_t)rcaps[4] << 8) | (uint32_t)rcaps[5];
        if (max_bps == 0u || max_bps > 256000u) max_bps = 256000u;
        out_cfg[0] = obj;
        out_cfg[1] = (uint8_t)((sfreq >> 4) & 0xFFu);
        out_cfg[2] = (uint8_t)(((sfreq & 0x0Fu) << 4) | (ch & 0x0Fu));
        out_cfg[3] = (uint8_t)(((vbr & 0x01u) << 7) | ((max_bps >> 16) & 0x7Fu));
        out_cfg[4] = (uint8_t)((max_bps >> 8) & 0xFFu);
        out_cfg[5] = (uint8_t)(max_bps & 0xFFu);
        *out_len = 6u;
        return 0;
    } else if (codec_kind == SALSA_BT_WIRE_SBC && rlen >= 4u && out_cap >= 4u) {
        if (out_cap_bit) *out_cap_bit = LWASAPI_BT_CAP_SBC;
        uint8_t b0 = rcaps[0], b1 = rcaps[1];
        uint8_t in_ch   = (uint8_t)(b0 & 0x0Fu);
        uint8_t in_freq = (uint8_t)((b0 >> 4) & 0x0Fu);
        uint8_t in_alloc = (uint8_t)(b1 & 0x03u);
        uint8_t in_sub   = (uint8_t)((b1 >> 2) & 0x03u);
        uint8_t in_blk   = (uint8_t)((b1 >> 4) & 0x0Fu);
        uint8_t freq = (in_freq & SW_SBC_FREQ_48000) ? SW_SBC_FREQ_48000
                     : (in_freq & SW_SBC_FREQ_44100) ? SW_SBC_FREQ_44100
                     : (in_freq & SW_SBC_FREQ_32000) ? SW_SBC_FREQ_32000 : SW_SBC_FREQ_16000;
        uint8_t ch   = (in_ch & SW_SBC_CH_JOINT_STEREO) ? SW_SBC_CH_JOINT_STEREO
                     : (in_ch & SW_SBC_CH_STEREO)       ? SW_SBC_CH_STEREO
                     : (in_ch & SW_SBC_CH_DUAL)         ? SW_SBC_CH_DUAL : SW_SBC_CH_MONO;
        uint8_t blk  = (in_blk & SW_SBC_BLK_16) ? SW_SBC_BLK_16
                     : (in_blk & SW_SBC_BLK_12) ? SW_SBC_BLK_12
                     : (in_blk & SW_SBC_BLK_8)  ? SW_SBC_BLK_8 : SW_SBC_BLK_4;
        uint8_t sub   = (in_sub & SW_SBC_SUB_8) ? SW_SBC_SUB_8 : SW_SBC_SUB_4;
        uint8_t alloc = (in_alloc & SW_SBC_ALLOC_LOUD) ? SW_SBC_ALLOC_LOUD : SW_SBC_ALLOC_SNR;
        uint8_t min_bp = rcaps[2] < 2u ? 2u : (rcaps[2] > 53u ? 53u : rcaps[2]);
        uint8_t max_bp = rcaps[3] > 53u ? 53u : (rcaps[3] < 2u ? 2u : rcaps[3]);
        if (max_bp < min_bp) max_bp = min_bp;
        out_cfg[0] = (uint8_t)(((freq & 0x0Fu) << 4) | (ch & 0x0Fu));
        out_cfg[1] = (uint8_t)(((blk & 0x0Fu) << 4) | ((sub & 0x03u) << 2) | (alloc & 0x03u));
        out_cfg[2] = min_bp;
        out_cfg[3] = max_bp;
        *out_len = 4u;
        return 0;
    }
    return -1;
}

static int sw_hex_val(uint8_t ch) {
    if (ch >= '0' && ch <= '9') return ch - '0';
    if (ch >= 'a' && ch <= 'f') return ch - 'a' + 10;
    if (ch >= 'A' && ch <= 'F') return ch - 'A' + 10;
    return -1;
}

bool salsa_bt_parse_bdaddr(const char *str, size_t len, uint8_t out_b[6]) {
    if (!str || !out_b || len != 17u) return false;
    uint8_t tmp[6];
    for (int idx = 0; idx < 6; idx++) {
        size_t pos = (size_t)idx * 3u;
        int hi = sw_hex_val((uint8_t)str[pos]);
        int lo = sw_hex_val((uint8_t)str[pos + 1u]);
        if (hi < 0 || lo < 0) return false;
        if (idx < 5 && str[pos + 2u] != ':') return false;
        tmp[5 - idx] = (uint8_t)((hi << 4) | lo);
    }
    memcpy(out_b, tmp, 6u);
    return true;
}

/* ============================================================================
 * 4. PulseAudio Frame Header Validator & PipeWire FD Quota Guard
 * ============================================================================ */

bool salsa_pa_parse_frame_hdr(const uint8_t hdr[20], uint32_t *out_body_len,
                              uint32_t *out_channel, uint32_t *out_flags) {
    if (!hdr) return false;
    uint32_t len   = sw_read_be32(hdr);
    uint32_t chan  = sw_read_be32(hdr + 4u);
    uint32_t flags = sw_read_be32(hdr + 16u);
    if (len > SALSA_PA_MAX_FRAME_SZ) return false;
    if (out_body_len) *out_body_len = len;
    if (out_channel)  *out_channel  = chan;
    if (out_flags)    *out_flags    = flags;
    return true;
}

bool salsa_pwb_fd_quota_check(uint32_t current_nfds, uint32_t incoming_cnt,
                              uint32_t *out_accept) {
    if (current_nfds >= SALSA_PWB_MAX_CLIENT_FDS) {
        if (out_accept) *out_accept = 0u;
        return false;
    }
    uint32_t avail = SALSA_PWB_MAX_CLIENT_FDS - current_nfds;
    if (incoming_cnt > avail) {
        if (out_accept) *out_accept = avail;
        return false;
    }
    if (out_accept) *out_accept = incoming_cnt;
    return true;
}

/* ============================================================================
 * 5. Wire Backend Identity & Silent Network Failsafe Telemetry (C Fallback)
 * ============================================================================ */

static _Atomic uint64_t g_fs_trips_total     = 0;
static _Atomic uint64_t g_fs_pa_quarantined  = 0;
static _Atomic uint64_t g_fs_pw_quarantined  = 0;
static _Atomic uint64_t g_fs_bt_fallbacks    = 0;
static _Atomic uint64_t g_fs_fd_leaks        = 0;
static _Atomic uint64_t g_fs_manual_resets   = 0;
static pthread_mutex_t  g_fs_reason_lock     = PTHREAD_MUTEX_INITIALIZER;
static char             g_fs_last_reason[96] = {0};

bool salsa_wire_is_rust(void) {
    return false;
}

void salsa_failsafe_record_trip(uint32_t kind, const char *reason) {
    atomic_fetch_add_explicit(&g_fs_trips_total, 1u, memory_order_relaxed);
    switch (kind) {
        case SALSA_FS_PA_BAD_PKT:
            atomic_fetch_add_explicit(&g_fs_pa_quarantined, 1u, memory_order_relaxed);
            break;
        case SALSA_FS_PW_BAD_PKT:
            atomic_fetch_add_explicit(&g_fs_pw_quarantined, 1u, memory_order_relaxed);
            break;
        case SALSA_FS_FD_QUOTA:
            atomic_fetch_add_explicit(&g_fs_fd_leaks, 1u, memory_order_relaxed);
            break;
        case SALSA_FS_BT_FALLBACK:
            atomic_fetch_add_explicit(&g_fs_bt_fallbacks, 1u, memory_order_relaxed);
            break;
        case SALSA_FS_MANUAL_RESET:
            atomic_fetch_add_explicit(&g_fs_manual_resets, 1u, memory_order_relaxed);
            break;
        default:
            break;
    }
    if (reason && reason[0]) {
        pthread_mutex_lock(&g_fs_reason_lock);
        sw_safe_copy(g_fs_last_reason, sizeof(g_fs_last_reason), reason);
        pthread_mutex_unlock(&g_fs_reason_lock);
    }
}

void salsa_failsafe_get_stats(salsa_failsafe_stats_t *out) {
    if (!out) return;
    memset(out, 0, sizeof(*out));
    out->is_rust          = false;
    out->armed            = true;
    out->trips_total      = atomic_load_explicit(&g_fs_trips_total, memory_order_relaxed);
    out->pa_quarantined   = atomic_load_explicit(&g_fs_pa_quarantined, memory_order_relaxed);
    out->pw_quarantined   = atomic_load_explicit(&g_fs_pw_quarantined, memory_order_relaxed);
    out->bt_fallbacks     = atomic_load_explicit(&g_fs_bt_fallbacks, memory_order_relaxed);
    out->fd_leaks_blocked = atomic_load_explicit(&g_fs_fd_leaks, memory_order_relaxed);
    out->manual_resets    = atomic_load_explicit(&g_fs_manual_resets, memory_order_relaxed);
    pthread_mutex_lock(&g_fs_reason_lock);
    sw_safe_copy(out->last_reason, sizeof(out->last_reason), g_fs_last_reason);
    pthread_mutex_unlock(&g_fs_reason_lock);
}

void salsa_failsafe_reset_stats(void) {
    atomic_store_explicit(&g_fs_trips_total, 0u, memory_order_relaxed);
    atomic_store_explicit(&g_fs_pa_quarantined, 0u, memory_order_relaxed);
    atomic_store_explicit(&g_fs_pw_quarantined, 0u, memory_order_relaxed);
    atomic_store_explicit(&g_fs_bt_fallbacks, 0u, memory_order_relaxed);
    atomic_store_explicit(&g_fs_fd_leaks, 0u, memory_order_relaxed);
    atomic_store_explicit(&g_fs_manual_resets, 0u, memory_order_relaxed);
    pthread_mutex_lock(&g_fs_reason_lock);
    g_fs_last_reason[0] = '\0';
    pthread_mutex_unlock(&g_fs_reason_lock);
}

