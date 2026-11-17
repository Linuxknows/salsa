#ifndef SALSA_WIRE_H
#define SALSA_WIRE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "lwasapi.h"

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

typedef struct {
    size_t   reply_len;
    int32_t  battery_pct;
    uint8_t  sco_codec;
    bool     handled;
    bool     has_msbc_update;
    bool     supports_msbc;
    char     reply[192];
} salsa_bt_hfp_at_res_t;

bool salsa_bt_hfp_parse_at(const char *line, size_t len, salsa_bt_hfp_at_res_t *out);
bool salsa_bt_make_batt_path(const char *dev_path, char *out, size_t out_cap);
bool salsa_bt_extract_mac(const char *id, char *out_mac, size_t out_cap);

size_t salsa_bt_write_rtp_header(uint8_t *out, size_t cap, uint8_t payload_type,
                                 bool marker, uint16_t seq, uint32_t timestamp,
                                 uint32_t ssrc);
size_t salsa_bt_write_rtp_sbc_payload_hdr(uint8_t *out, size_t cap, uint8_t frame_count);
size_t salsa_bt_pack_s16_to_s24le(const int16_t *pcm_s16, size_t nsamp,
                                  uint8_t *out_s24, size_t out_cap);
uint32_t salsa_bt_ldac_eqmid_to_kbps(uint32_t sf, int32_t eqmid);
uint8_t  salsa_bt_msbc_h2_byte(uint32_t seq);
size_t   salsa_bt_msbc_rx_strip_offset(const uint8_t *rx, size_t rx_len);
void     salsa_bt_abr_step_ldac(int32_t cur_eqmid, uint32_t congest_streak,
                                uint32_t clean_streak, int32_t *out_new_eqmid,
                                bool *out_clear_congest, bool *out_clear_clean);
void     salsa_bt_abr_step_aac(uint32_t cur_bps, uint32_t max_bps,
                               uint32_t congest_streak, uint32_t clean_streak,
                               uint32_t *out_new_bps, bool *out_clear_congest,
                               bool *out_clear_clean);

int salsa_bt_ring_write_period(float *tx_ring, const void *wr_frame_atomic,
                               bool tx_paused, const float *interleaved,
                               uint32_t frames, uint32_t channels);
size_t salsa_bt_ring_pop_resampled_s16(const float *tx_ring,
                                       const void *wr_frame_atomic,
                                       const void *rd_frame_atomic,
                                       uint32_t in_rate, uint32_t out_rate,
                                       bool muted, uint32_t vol_pct,
                                       bool avrcp_supported,
                                       int16_t *out_s16, uint32_t needed_frames,
                                       uint32_t bt_ch, double *io_frac,
                                       double drift_scale, uint32_t *out_avail_after);
void salsa_bt_sco_push_pcm(float *cap_ring, const void *wr_frame_atomic,
                           const int16_t *dec_pcm, uint32_t nsamp,
                           uint32_t sco_rate, uint32_t eng_rate,
                           double *io_resamp_pos, float *io_prev_s);

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

/* ============================================================================
 * 6. Debug Telemetry & Datagram IPC Helpers
 * ============================================================================ */

#define SALSA_DBG_DGRAM_MAGIC 0x4C444247u
#define SALSA_DBG_DGRAM_SZ    244u

void   salsa_dbg_emit_raw(uint8_t sub, uint8_t lvl, uint32_t pid, uint32_t sid,
                          int32_t v1, int32_t v2, int32_t hr,
                          const char *op, const char *target, const char *detail);
bool   salsa_dbg_parse_dgram(const uint8_t *buf, size_t len, void *out_dgram);
size_t salsa_dbg_escape_json_str(const char *src, char *out, size_t out_cap);
size_t salsa_dbg_trim_cmd(const uint8_t *raw, size_t len);

/* ============================================================================
 * 7. Audio DSP, Capture Mic APO, Session Ring Mixer & State Persistence
 * ============================================================================ */

enum {
    SALSA_PACK_S32_LE  = 0u,
    SALSA_PACK_S24_LE  = 1u,
    SALSA_PACK_S24_3LE = 2u,
    SALSA_PACK_S16_LE  = 3u,
    SALSA_PACK_F32_LE  = 4u
};

typedef struct {
    float hist_l[3];
    float hist_r[3];
    bool  valid;
} salsa_mic_cubic_state_t;

typedef struct {
    uint32_t                sample_rate;
    uint32_t                period_frames;
    uint32_t                format_bits;
    uint32_t                cap_sample_rate;
    char                    target_playback_id[64];
    char                    target_capture_id[64];
    lwasapi_eq_config_t     eq_cfg;
    lwasapi_param_eq_ext_t  param_eq;
    lwasapi_mic_apo_cfg_t   mic_apo;
    lwasapi_bt_codec_cfg_t  bt_cfg;
    lwasapi_hw_analog_cfg_t hw_analog_cfg;
    uint32_t                desktop_volume_pct;
    uint32_t                desktop_muted;
    uint32_t                desktop_mic_volume_pct;
    uint32_t                desktop_mic_muted;
    uint32_t                desktop_mic_gain_ppm;
    bool                    has_saved_vol_state;
} salsa_runtime_state_t;

void salsa_dsp_clamp_param_eq_ext(lwasapi_param_eq_ext_t *ext);
void salsa_dsp_clamp_mic_apo_cfg(lwasapi_mic_apo_cfg_t *m);
void salsa_dsp_process_dynamic_eq(float *buf, uint32_t frames, uint32_t ch, uint32_t rate,
                                  const lwasapi_eq_config_t *eq_cfg,
                                  const lwasapi_param_eq_ext_t *param_eq);
void salsa_dsp_process_capture_mic_apo(float *mic_buf, uint32_t frames, uint32_t rate,
                                       const lwasapi_mic_apo_cfg_t *cfg);
void salsa_dsp_get_telem(float *out_band_levels_db, float *out_dyn_delta_db,
                         float *out_pk_l, float *out_pk_r,
                         float *out_mic_pk_in, float *out_mic_pk_out,
                         uint32_t *out_rnnoise_active);
void salsa_dsp_consume_session_frames(lwasapi_shm_ring_t *r, double *io_phase,
                                      uint32_t s_rate, uint32_t s_ch,
                                      bool *io_stream_live, bool *io_underrun_pending,
                                      uint64_t *io_last_audio_ns,
                                      float *mix_buf, uint32_t out_frames,
                                      uint32_t out_channels, uint32_t hw_rate,
                                      bool replace_output, uint64_t now_ns);
void salsa_dsp_resample_mic_cubic(const float *in_buf, uint32_t in_frames,
                                  float *out_buf, uint32_t out_frames,
                                  salsa_mic_cubic_state_t *st);
void salsa_dsp_scale_cap_clip(float *buf, uint32_t samples, float gain);
void salsa_dsp_pack_pcm(const float *mix_buf, uint32_t total_samples,
                        float master_gain, bool softknee,
                        uint32_t pack_fmt, void *out_bytes);

void salsa_state_init_defaults(salsa_runtime_state_t *out);
void salsa_state_load(salsa_runtime_state_t *io_st);
void salsa_state_save(const salsa_runtime_state_t *st);

/* ============================================================================
 * 8. Audiodg WaveRT Daemon Entrypoint & BlueZ Adapter Snapshot
 * ============================================================================ */

typedef struct {
    bool     initialized;
    bool     has_bluez_transport;
    bool     direct_socket_mode;
    bool     has_capture_hw;
    bool     endpoints_registered;
    bool     tx_paused;
    bool     cap_is_usb;
    bool     desktop_muted;
    bool     desktop_mic_muted;
    int      bt_fd;
    int      cap_pcm_fd;
    uint16_t delay_tenths_ms;
    uint16_t read_mtu;
    uint16_t write_mtu;
    uint16_t avrcp_volume;
    uint32_t bt_sample_rate;
    uint32_t bt_channels;
    uint32_t cap_hw_rate;
    uint32_t cap_hw_channels;
    int32_t  cap_hw_format;
    uint32_t cap_measured_rate;
    uint32_t cap_kick_count;
    uint32_t desktop_volume_pct;
    uint32_t desktop_mic_volume_pct;
    uint32_t desktop_mic_gain_ppm;
    uint64_t tx_ring_fill;
    uint64_t tx_packets_sent;
    uint64_t tx_underruns;
    uint64_t tx_errors;
    uint64_t cap_hw_write_frame;
    char     mac_address[32];
    char     device_alias[64];
    char     codec_name[16];
    char     device_path[128];
    char     transport_path[160];
    char     endpoint_path[64];
    char     cap_device_id[64];
    char     cap_pcm_path[64];
    char     capture_device_alias[64];
} salsa_bt_snapshot_t;

typedef struct {
    void *(*bt_ptr)(void);
    void (*get_snapshot)(const void *bt, salsa_bt_snapshot_t *out);
    void (*rt_master_vol)(const void *bt, uint32_t *out_vol_pct, bool *out_muted);
    int  (*init_rates)(void *bt, uint32_t engine_sample_rate, uint32_t capture_sample_rate,
                       const char *preferred_cap_id, const char *preferred_pb_id);
    void (*set_preferred_target)(void *bt, const char *preferred_pb_id);
    int  (*reconfigure_rate)(void *bt, uint32_t new_engine_sample_rate);
    void (*set_tx_enabled)(void *bt, bool enabled);
    int  (*write_period)(void *bt, const float *interleaved_float, uint32_t frames, uint32_t channels);
    int  (*open_capture)(void *bt, const char *preferred_cap_id, uint32_t capture_sample_rate);
    int  (*read_capture_native)(void *bt, float *out_stereo_float, uint32_t max_frames,
                                uint32_t *out_frames, uint32_t *out_rate,
                                uint32_t eng_frames, uint32_t eng_rate, uint64_t *io_read_cursor);
    int  (*enumerate_devices)(const void *bt, const char *active_pb_id,
                              const char *active_cap_id, lwasapi_device_list_t *out_list);
    uint64_t (*hw_fingerprint)(const void *bt);
    int  (*set_volume)(void *bt, int vol_pct, int mute_flag);
    int  (*set_mic_volume)(void *bt, int vol_pct, int mute_flag);
    int  (*set_mic_gain_ppm)(void *bt, uint32_t gain_ppm, int vol_pct, int mute_flag);
    int  (*set_codec_config)(void *bt, const lwasapi_bt_codec_cfg_t *cfg);
    void (*get_codec_config)(const void *bt, lwasapi_bt_codec_cfg_t *out_cfg);
    int  (*set_sco_enabled)(void *bt, bool enabled);
    void (*close)(void *bt);
    int  (*reset_device)(const char *mac_or_empty, char *out_msg, size_t out_len);
} salsa_bt_ops_t;

void  salsa_register_bt_ops(const salsa_bt_ops_t *ops);
void *salsa_audiodg_bt_ptr(void);
void  salsa_bt_get_snapshot(const void *bt, salsa_bt_snapshot_t *out);
void  salsa_bt_rt_master_vol(const void *bt, uint32_t *out_vol_pct, bool *out_muted);
int   salsa_audiodg_main(int argc, char **argv);

#ifdef __cplusplus
}
#endif

#endif /* SALSA_WIRE_H */
