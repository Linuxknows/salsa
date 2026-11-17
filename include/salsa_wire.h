#ifndef SALSA_WIRE_H
#define SALSA_WIRE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ============================================================================
 * 1. PulseAudio v35 Wire Tagstruct Parser Boundary
 * ============================================================================ */

enum {
    SALSA_PA_TAG_INVALID       = 0,
    SALSA_PA_TAG_STRING        = 't',
    SALSA_PA_TAG_STRING_NULL   = 'N',
    SALSA_PA_TAG_U32           = 'L',
    SALSA_PA_TAG_U8            = 'B',
    SALSA_PA_TAG_U64           = 'R',
    SALSA_PA_TAG_S64           = 'r',
    SALSA_PA_TAG_SAMPLE_SPEC   = 'a',
    SALSA_PA_TAG_ARBITRARY     = 'x',
    SALSA_PA_TAG_BOOLEAN_TRUE  = '1',
    SALSA_PA_TAG_BOOLEAN_FALSE = '0',
    SALSA_PA_TAG_TIMEVAL       = 'T',
    SALSA_PA_TAG_USEC          = 'U',
    SALSA_PA_TAG_CHANNEL_MAP   = 'm',
    SALSA_PA_TAG_CVOLUME       = 'v',
    SALSA_PA_TAG_PROPLIST      = 'P',
    SALSA_PA_TAG_VOLUME        = 'V',
    SALSA_PA_TAG_FORMAT_INFO   = 'f'
};

typedef struct {
    const uint8_t *p;
    size_t         len;
    size_t         pos;
} salsa_pa_tagread_t;

typedef struct {
    uint8_t  format;
    uint8_t  channels;
    uint32_t rate;
} salsa_pa_wire_ss_t;

typedef struct {
    uint8_t channels;
    uint8_t map[32];
} salsa_pa_wire_map_t;

typedef struct {
    uint8_t  channels;
    uint32_t values[32];
} salsa_pa_wire_cvol_t;

typedef struct {
    char key[64];
    char val[128];
} salsa_pa_kv_t;

typedef struct {
    salsa_pa_kv_t items[24];
    int32_t       count;
} salsa_pa_propbag_t;

void salsa_tr_init(salsa_pa_tagread_t *tr, const uint8_t *p, size_t len);
bool salsa_tr_u32(salsa_pa_tagread_t *tr, uint32_t *out);
bool salsa_tr_u8(salsa_pa_tagread_t *tr, uint8_t *out);
bool salsa_tr_bool(salsa_pa_tagread_t *tr, bool *out);
bool salsa_tr_str_slice(salsa_pa_tagread_t *tr, size_t *out_off, size_t *out_len, bool *out_is_null);
bool salsa_tr_arbitrary_slice(salsa_pa_tagread_t *tr, size_t *out_off, size_t *out_len);
bool salsa_tr_ss(salsa_pa_tagread_t *tr, salsa_pa_wire_ss_t *ss);
bool salsa_tr_map(salsa_pa_tagread_t *tr, salsa_pa_wire_map_t *m);
bool salsa_tr_cvol(salsa_pa_tagread_t *tr, salsa_pa_wire_cvol_t *cv);
bool salsa_tr_timeval(salsa_pa_tagread_t *tr, uint32_t *sec, uint32_t *usec);
void salsa_propbag_set(salsa_pa_propbag_t *pb, const char *k, const char *v);
bool salsa_tr_proplist(salsa_pa_tagread_t *tr, salsa_pa_propbag_t *pb);
bool salsa_tr_skip_format_info(salsa_pa_tagread_t *tr);

/* ============================================================================
 * 2. PipeWire Video Broker Wire Framing & SPA Pod Parser Boundary
 * ============================================================================ */

#define SALSA_PWB_MAX_PROPS     48u
#define SALSA_PWB_MAX_MODIFIERS 64u
#define SALSA_PWB_MAX_PACKET_SZ (16u * 1024u * 1024u)

enum {
    SALSA_PWB_SPA_NONE   = 1,
    SALSA_PWB_SPA_BOOL   = 2,
    SALSA_PWB_SPA_ID     = 3,
    SALSA_PWB_SPA_INT    = 4,
    SALSA_PWB_SPA_LONG   = 5,
    SALSA_PWB_SPA_FLOAT  = 6,
    SALSA_PWB_SPA_DOUBLE = 7,
    SALSA_PWB_SPA_STRING = 8,
    SALSA_PWB_SPA_BYTES  = 9,
    SALSA_PWB_SPA_RECT   = 10,
    SALSA_PWB_SPA_FRAC   = 11,
    SALSA_PWB_SPA_ARRAY  = 13,
    SALSA_PWB_SPA_STRUCT = 14,
    SALSA_PWB_SPA_OBJECT = 15,
    SALSA_PWB_SPA_FD     = 18,
    SALSA_PWB_SPA_CHOICE = 19
};

enum {
    SALSA_PWB_OBJ_FORMAT     = 0x40003u,
    SALSA_PWB_OBJ_PARAM_BUFS = 0x40004u
};

enum {
    SALSA_PWB_PARAM_EnumFormat = 3u,
    SALSA_PWB_PARAM_Format     = 4u,
    SALSA_PWB_PARAM_Buffers    = 5u
};

enum {
    SALSA_PWB_DATA_MemFd = 2u
};

enum {
    SALSA_PWB_CHOICE_None  = 0u,
    SALSA_PWB_CHOICE_Range = 1u,
    SALSA_PWB_CHOICE_Step  = 2u,
    SALSA_PWB_CHOICE_Enum  = 3u,
    SALSA_PWB_CHOICE_Flags = 4u
};

#define SALSA_PWB_POD_PROP_FLAG_DONT_FIXATE (1u << 4)

enum {
    SALSA_PWB_MEDIA_Video = 2u,
    SALSA_PWB_SUB_Raw     = 1u
};

enum {
    SALSA_PWB_VF_I420 = 2u,
    SALSA_PWB_VF_YUY2 = 4u,
    SALSA_PWB_VF_RGBx = 7u,
    SALSA_PWB_VF_BGRx = 8u,
    SALSA_PWB_VF_xRGB = 9u,
    SALSA_PWB_VF_xBGR = 10u,
    SALSA_PWB_VF_RGBA = 11u,
    SALSA_PWB_VF_BGRA = 12u,
    SALSA_PWB_VF_ARGB = 13u,
    SALSA_PWB_VF_ABGR = 14u,
    SALSA_PWB_VF_RGB  = 15u,
    SALSA_PWB_VF_BGR  = 16u,
    SALSA_PWB_VF_NV12 = 23u
};

typedef struct {
    uint32_t width;
    uint32_t height;
} salsa_pwb_rect_t;

typedef struct {
    uint32_t num;
    uint32_t denom;
} salsa_pwb_frac_t;

typedef struct {
    char key[64];
    char val[160];
} salsa_pwb_kv_t;

typedef struct {
    salsa_pwb_kv_t items[SALSA_PWB_MAX_PROPS];
    uint32_t       count;
} salsa_pwb_dict_t;

typedef struct {
    uint32_t         formats[32];
    uint32_t         n_formats;
    uint64_t         modifiers[SALSA_PWB_MAX_MODIFIERS];
    uint32_t         n_modifiers;
    bool             has_modifier;
    bool             mod_dont_fixate;
    uint32_t         w_def, w_min, w_max;
    uint32_t         h_def, h_min, h_max;
    salsa_pwb_frac_t fps_def, fps_min, fps_max;
    salsa_pwb_frac_t max_fps_def, max_fps_min, max_fps_max;
    bool             has_fps;
    bool             has_max_fps;
    bool             valid;
} salsa_pwb_vfmt_caps_t;

typedef struct {
    uint32_t buf_def, buf_min, buf_max;
    uint32_t size_def, size_min, size_max;
    uint32_t stride_def;
    uint32_t datatype_mask;
    bool     valid;
} salsa_pwb_buf_caps_t;

enum {
    SALSA_PWB_PARSED_NONE = 0,
    SALSA_PWB_PARSED_VFMT = 1,
    SALSA_PWB_PARSED_BUFS = 2
};

typedef struct {
    uint32_t              kind;
    salsa_pwb_vfmt_caps_t vfmt;
    salsa_pwb_buf_caps_t  bcaps;
} salsa_pwb_param_parsed_t;

typedef struct {
    const uint8_t *base;
    size_t         off;
    size_t         end;
} salsa_pwb_cur_t;

bool salsa_pwb_parse_pkt_hdr(const uint8_t *buf, size_t avail,
                             uint32_t *out_oid, uint8_t *out_op,
                             uint32_t *out_psz, uint32_t *out_seq,
                             uint32_t *out_nfds, size_t *out_pkt_len);
void salsa_pwb_cur_init(salsa_pwb_cur_t *c, const void *data, size_t len);
bool salsa_pwb_cur_next_pod(salsa_pwb_cur_t *c, uint32_t *out_sz, uint32_t *out_tp,
                            size_t *out_body_off, size_t *out_pod_total);
bool salsa_pwb_cur_enter_struct(salsa_pwb_cur_t *c, salsa_pwb_cur_t *inner);
bool salsa_pwb_cur_int(salsa_pwb_cur_t *c, int32_t *out);
bool salsa_pwb_cur_u32(salsa_pwb_cur_t *c, uint32_t *out);
bool salsa_pwb_cur_long(salsa_pwb_cur_t *c, int64_t *out);
bool salsa_pwb_cur_str(salsa_pwb_cur_t *c, char *out, size_t out_sz);
void salsa_pwb_dict_set(salsa_pwb_dict_t *d, const char *key, const char *val);
bool salsa_pwb_cur_dict(salsa_pwb_cur_t *c, salsa_pwb_dict_t *d, bool wrapped_in_struct);
void salsa_pwb_parse_param_pod(const uint8_t *pod, size_t pod_sz, salsa_pwb_param_parsed_t *out);
uint32_t salsa_pwb_vfmt_stride(uint32_t fmt, uint32_t w);
uint32_t salsa_pwb_vfmt_size(uint32_t fmt, uint32_t w, uint32_t h);

/* ============================================================================
 * 3. BlueZ A2DP Capability Negotiation & MAC Parser Boundary
 * ============================================================================ */

enum {
    SALSA_BT_WIRE_SBC     = 0,
    SALSA_BT_WIRE_AAC     = 1,
    SALSA_BT_WIRE_APTX    = 2,
    SALSA_BT_WIRE_APTX_HD = 3,
    SALSA_BT_WIRE_LDAC    = 4
};

int  salsa_bt_select_a2dp_config(int codec_kind, const uint8_t *rcaps, size_t rlen,
                                 uint8_t *out_cfg, size_t out_cap,
                                 size_t *out_len, uint32_t *out_cap_bit);
bool salsa_bt_parse_bdaddr(const char *str, size_t len, uint8_t out_b[6]);

/* ============================================================================
 * 4. PulseAudio Frame Header Validator & PipeWire FD Quota Guard
 * ============================================================================ */

#define SALSA_PA_MAX_FRAME_SZ    (16u * 1024u * 1024u)
#define SALSA_PWB_MAX_CLIENT_FDS 256u

bool salsa_pa_parse_frame_hdr(const uint8_t hdr[20], uint32_t *out_body_len,
                              uint32_t *out_channel, uint32_t *out_flags);
bool salsa_pwb_fd_quota_check(uint32_t current_nfds, uint32_t incoming_cnt,
                              uint32_t *out_accept);

/* ============================================================================
 * 5. Wire Backend Identity & Silent Network Failsafe Telemetry
 * ============================================================================ */

enum {
    SALSA_FS_PA_BAD_PKT   = 1u,
    SALSA_FS_PW_BAD_PKT   = 2u,
    SALSA_FS_FD_QUOTA     = 3u,
    SALSA_FS_BT_FALLBACK  = 4u,
    SALSA_FS_MANUAL_RESET = 5u
};

typedef struct {
    bool     is_rust;
    bool     armed;
    uint64_t trips_total;
    uint64_t pa_quarantined;
    uint64_t pw_quarantined;
    uint64_t bt_fallbacks;
    uint64_t fd_leaks_blocked;
    uint64_t manual_resets;
    char     last_reason[96];
} salsa_failsafe_stats_t;

bool salsa_wire_is_rust(void);
void salsa_failsafe_record_trip(uint32_t kind, const char *reason);
void salsa_failsafe_get_stats(salsa_failsafe_stats_t *out);
void salsa_failsafe_reset_stats(void);

#ifdef __cplusplus
}
#endif

#endif /* SALSA_WIRE_H */
