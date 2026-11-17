#define _GNU_SOURCE
#include "../include/lwasapi_pa_server.h"
#include "../include/salsa_wire.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <poll.h>
#include <math.h>
#include <time.h>
#include <sched.h>
#include <pthread.h>
#include <pwd.h>
#include <arpa/inet.h>
#include <sys/stat.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/eventfd.h>
#include <sys/time.h>

/* ============================================================================
 * PulseAudio Native Protocol v35 Wire Constants
 * ============================================================================ */

#define PA_PROTO_VERSION           35U
#define PA_INVALID_INDEX           0xFFFFFFFFU
#define PA_PSTREAM_CTRL_CHANNEL    0xFFFFFFFFU
#define PA_VOLUME_NORM             0x10000U
#define PA_NATIVE_COOKIE_LEN       256
#define PA_SERVER_COOKIE_ID        0x53414C53U

#define PA_MAX_CLIENTS             32
#define PA_MAX_STREAMS             48
#define PA_MAX_OUT_QUEUE           2048
#define PA_STREAM_RING_FRAMES      32768 /* Power of 2 for lock-free mask */
#define PA_DEFAULT_MAXLENGTH       (4U * 1024U * 1024U)
#define PA_PSTREAM_DESC_BYTES      20U

/* Commands */
enum {
    CMD_ERROR                         = 0,
    CMD_TIMEOUT                       = 1,
    CMD_REPLY                         = 2,
    CMD_CREATE_PLAYBACK_STREAM        = 3,
    CMD_DELETE_PLAYBACK_STREAM        = 4,
    CMD_CREATE_RECORD_STREAM          = 5,
    CMD_DELETE_RECORD_STREAM          = 6,
    CMD_EXIT                          = 7,
    CMD_AUTH                          = 8,
    CMD_SET_CLIENT_NAME               = 9,
    CMD_LOOKUP_SINK                   = 10,
    CMD_LOOKUP_SOURCE                 = 11,
    CMD_DRAIN_PLAYBACK_STREAM         = 12,
    CMD_STAT                          = 13,
    CMD_GET_PLAYBACK_LATENCY          = 14,
    CMD_GET_SERVER_INFO               = 20,
    CMD_GET_SINK_INFO                 = 21,
    CMD_GET_SINK_INFO_LIST            = 22,
    CMD_GET_SOURCE_INFO               = 23,
    CMD_GET_SOURCE_INFO_LIST          = 24,
    CMD_GET_MODULE_INFO               = 25,
    CMD_GET_MODULE_INFO_LIST          = 26,
    CMD_GET_CLIENT_INFO               = 27,
    CMD_GET_CLIENT_INFO_LIST          = 28,
    CMD_GET_SINK_INPUT_INFO           = 29,
    CMD_GET_SINK_INPUT_INFO_LIST      = 30,
    CMD_GET_SOURCE_OUTPUT_INFO        = 31,
    CMD_GET_SOURCE_OUTPUT_INFO_LIST   = 32,
    CMD_GET_SAMPLE_INFO               = 33,
    CMD_GET_SAMPLE_INFO_LIST          = 34,
    CMD_SUBSCRIBE                     = 35,
    CMD_SET_SINK_VOLUME               = 36,
    CMD_SET_SINK_INPUT_VOLUME         = 37,
    CMD_SET_SOURCE_VOLUME             = 38,
    CMD_SET_SINK_MUTE                 = 39,
    CMD_SET_SOURCE_MUTE               = 40,
    CMD_CORK_PLAYBACK_STREAM          = 41,
    CMD_FLUSH_PLAYBACK_STREAM         = 42,
    CMD_TRIGGER_PLAYBACK_STREAM       = 43,
    CMD_SET_DEFAULT_SINK              = 44,
    CMD_SET_DEFAULT_SOURCE            = 45,
    CMD_SET_PLAYBACK_STREAM_NAME      = 46,
    CMD_SET_RECORD_STREAM_NAME        = 47,
    CMD_KILL_CLIENT                   = 48,
    CMD_KILL_SINK_INPUT               = 49,
    CMD_KILL_SOURCE_OUTPUT            = 50,
    CMD_GET_RECORD_LATENCY            = 57,
    CMD_CORK_RECORD_STREAM            = 58,
    CMD_FLUSH_RECORD_STREAM           = 59,
    CMD_PREBUF_PLAYBACK_STREAM        = 60,
    CMD_REQUEST                       = 61,
    CMD_OVERFLOW                      = 62,
    CMD_UNDERFLOW                     = 63,
    CMD_SUBSCRIBE_EVENT               = 66,
    CMD_MOVE_SINK_INPUT               = 67,
    CMD_MOVE_SOURCE_OUTPUT            = 68,
    CMD_SET_SINK_INPUT_MUTE           = 69,
    CMD_SUSPEND_SINK                  = 70,
    CMD_SUSPEND_SOURCE                = 71,
    CMD_SET_PLAYBACK_STREAM_BUFFER_ATTR = 72,
    CMD_SET_RECORD_STREAM_BUFFER_ATTR = 73,
    CMD_UPDATE_PLAYBACK_STREAM_SAMPLE_RATE = 74,
    CMD_UPDATE_RECORD_STREAM_SAMPLE_RATE = 75,
    CMD_UPDATE_RECORD_STREAM_PROPLIST = 80,
    CMD_UPDATE_PLAYBACK_STREAM_PROPLIST = 81,
    CMD_UPDATE_CLIENT_PROPLIST        = 82,
    CMD_REMOVE_RECORD_STREAM_PROPLIST = 83,
    CMD_REMOVE_PLAYBACK_STREAM_PROPLIST = 84,
    CMD_REMOVE_CLIENT_PROPLIST        = 85,
    CMD_STARTED                       = 86,
    CMD_EXTENSION                     = 87,
    CMD_GET_CARD_INFO                 = 88,
    CMD_GET_CARD_INFO_LIST            = 89,
    CMD_SET_CARD_PROFILE              = 90,
    CMD_SET_SINK_PORT                 = 96,
    CMD_SET_SOURCE_PORT               = 97,
    CMD_SET_SOURCE_OUTPUT_VOLUME      = 98,
    CMD_SET_SOURCE_OUTPUT_MUTE        = 99,
    CMD_SET_PORT_LATENCY_OFFSET       = 100
};

/* Tag characters */
enum {
    TAG_INVALID       = 0,
    TAG_STRING        = 't',
    TAG_STRING_NULL   = 'N',
    TAG_U32           = 'L',
    TAG_U8            = 'B',
    TAG_U64           = 'R',
    TAG_S64           = 'r',
    TAG_SAMPLE_SPEC   = 'a',
    TAG_ARBITRARY     = 'x',
    TAG_BOOLEAN_TRUE  = '1',
    TAG_BOOLEAN_FALSE = '0',
    TAG_TIMEVAL       = 'T',
    TAG_USEC          = 'U',
    TAG_CHANNEL_MAP   = 'm',
    TAG_CVOLUME       = 'v',
    TAG_PROPLIST      = 'P',
    TAG_VOLUME        = 'V',
    TAG_FORMAT_INFO   = 'f'
};

/* Errors */
enum {
    ERR_ACCESS        = 1,
    ERR_COMMAND       = 2,
    ERR_INVALID       = 3,
    ERR_NOENTITY      = 5,
    ERR_PROTOCOL      = 7,
    ERR_NOTSUPPORTED  = 19,
    ERR_NOEXTENSION   = 21
};

/* Subscription events */
#define SUB_EVENT_SINK          0x0000U
#define SUB_EVENT_SOURCE        0x0001U
#define SUB_EVENT_SINK_INPUT    0x0002U
#define SUB_EVENT_SOURCE_OUTPUT 0x0003U
#define SUB_EVENT_MODULE        0x0004U
#define SUB_EVENT_CLIENT        0x0005U
#define SUB_EVENT_SERVER        0x0007U
#define SUB_EVENT_CARD          0x0009U
#define SUB_EVENT_NEW           0x0000U
#define SUB_EVENT_CHANGE        0x0010U
#define SUB_EVENT_REMOVE        0x0020U
#define SUB_MASK_ALL            0x02FFU

/* Formats */
enum {
    FMT_U8        = 0,
    FMT_ALAW      = 1,
    FMT_ULAW      = 2,
    FMT_S16LE     = 3,
    FMT_S16BE     = 4,
    FMT_FLOAT32LE = 5,
    FMT_FLOAT32BE = 6,
    FMT_S32LE     = 7,
    FMT_S32BE     = 8,
    FMT_S24LE     = 9,
    FMT_S24BE     = 10,
    FMT_S24_32LE  = 11,
    FMT_S24_32BE  = 12
};

typedef salsa_pa_wire_ss_t   pa_wire_ss_t;
typedef salsa_pa_wire_map_t  pa_wire_map_t;
typedef salsa_pa_wire_cvol_t pa_wire_cvol_t;
typedef salsa_pa_kv_t        pa_kv_t;
typedef salsa_pa_propbag_t   pa_propbag_t;

typedef struct {
    uint32_t channel;
    uint32_t flags;
    uint8_t *data;
    size_t   len;
    size_t   sent;
} pa_out_pkt_t;

typedef struct pa_client pa_client_t;

typedef struct {
    bool              in_use;
    uint32_t          index;          /* Global PulseAudio sink_input / source_output index */
    uint32_t          channel;        /* Per-client channel id */
    pa_client_t      *client;
    bool              is_playback;
    bool              is_monitor;
    bool              peak_detect;
    uint32_t          direct_on_input;
    uint32_t          target_dev_idx;
    pa_wire_ss_t      ss;
    pa_wire_map_t     map;
    pa_wire_cvol_t    vol;
    uint32_t          frame_size;
    uint32_t          maxlength;
    uint32_t          tlength;
    uint32_t          prebuf;
    uint32_t          minreq;
    uint32_t          fragsize;
    pa_propbag_t      props;
    char              name[64];
    char              device_id[64];  /* Matched target sink/source */
    bool              pinned_route;   /* True if user explicitly routed stream */

    _Atomic int       active;
    _Atomic int       corked;
    _Atomic int       muted;
    _Atomic uint32_t  volume_milli;   /* 1000 = 100% linear multiplier */
    _Atomic int       in_prebuf;
    bool              started_sent;
    _Atomic int       draining;
    uint32_t          drain_tag;
    _Atomic uint32_t  sample_rate_atomic;

    /* Lock-free SPSC ring (stream sample rate, stereo float) */
    float             ring[PA_STREAM_RING_FRAMES * 2];
    _Atomic uint64_t  wr_frame;
    _Atomic uint64_t  rd_frame;
    double            resamp_phase;
    float             hist_l[2];
    float             hist_r[2];
    bool              have_hist;
    float             cap_hist_l;
    float             cap_hist_r;
    bool              have_cap_hist;
    float             peak_accum_l;
    float             peak_accum_r;
    float             mix_peak_l;
    float             mix_peak_r;
    float             last_mix_l;
    float             last_mix_r;
    float             cap_smooth_gain;
#define PA_MAX_DOI_PERIOD 2048U
    float             period_pcm[PA_MAX_DOI_PERIOD * 2];
    uint32_t          period_frames;

    /* Byte accounting (in stream native bytes) */
    _Atomic uint64_t  write_index;
    _Atomic uint64_t  read_index;
    _Atomic uint64_t  underrun_baseline_bytes;
    uint64_t          requested_bytes_total;
    _Atomic uint32_t  need_notify;
} pa_stream_t;

struct pa_client {
    bool         in_use;
    uint32_t     index;
    int          fd;
    uint32_t     version;
    bool         authed;
    uint32_t     sub_mask;
    pa_propbag_t props;
    char         name[64];
    uint32_t     pid;
    uint32_t     uid;
    bool         uid_valid;
    uint32_t     next_channel;

    /* RX framing state */
    uint8_t      rx_hdr[PA_PSTREAM_DESC_BYTES];
    size_t       rx_hdr_got;
    uint8_t     *rx_body;
    size_t       rx_body_len;
    size_t       rx_body_got;
    uint32_t     rx_channel;

    /* TX queue */
    pa_out_pkt_t out_q[PA_MAX_OUT_QUEUE];
    int          out_head;
    int          out_tail;
    int          out_count;
};

#define PA_MAX_STABLE_IDS 128U
#define PA_FAC_SINK       0U
#define PA_FAC_SOURCE     1U
#define PA_FAC_CARD       2U

typedef struct {
    bool     used;
    uint32_t facility;
    uint32_t idx;
    char     key[96];
} pa_stable_id_t;

struct lwasapi_pa_server {
    lwasapi_pa_callbacks_t cbs;
    int              listen_fd;
    int              wake_efd;
    char             sock_path[256];
    char             pid_path[256];
    uint8_t          cookie[PA_NATIVE_COOKIE_LEN];
    bool             cookie_valid;
    pthread_t        thread;
    volatile int     running;
    pthread_mutex_t  lock;

    pa_client_t      clients[PA_MAX_CLIENTS];
    pa_stream_t      streams[PA_MAX_STREAMS];
    uint32_t         next_client_idx;
    uint32_t         next_stream_idx;

    pa_stable_id_t   stable_ids[PA_MAX_STABLE_IDS];
    uint32_t         next_fac_idx[3];

    lwasapi_engine_status_t cached_status;
    lwasapi_device_list_t   cached_devices;
    bool                    devices_cache_valid;
    uint64_t                last_refresh_ns;

    bool             sink_pa_vol_valid;
    uint32_t         sink_pa_vol;
    bool             hw_src_pa_vol_valid;
    uint32_t         hw_src_pa_vol;
    uint32_t         mon_pa_vol;
    _Atomic uint32_t mon_vol_milli;
    _Atomic int      mon_muted;

    /* Dominant Stream Rate */
    _Atomic uint32_t dom_stream_rate;
    _Atomic uint32_t dom_stream_bits;

#define PA_MAX_APP_VOLUMES 256
    struct {
        char     key[128];
        uint32_t volume_milli;
        uint8_t  muted;
    }                app_vols[PA_MAX_APP_VOLUMES];
    uint32_t         num_app_vols;
    char             app_vols_path[320];
    pthread_mutex_t  vol_file_lock;
};

static void pa_init_pi_mutex(pthread_mutex_t *mtx) {
    pthread_mutexattr_t attr;
    pthread_mutexattr_init(&attr);
    pthread_mutexattr_setprotocol(&attr, PTHREAD_PRIO_INHERIT);
    pthread_mutex_init(mtx, &attr);
    pthread_mutexattr_destroy(&attr);
}

static void pa_enable_realtime(int prio, int fallback_nice) {
    struct sched_param sp;
    memset(&sp, 0, sizeof(sp));
    sp.sched_priority = prio;
    if (pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp) != 0) {
        setpriority(PRIO_PROCESS, 0, fallback_nice);
    }
}

static uint32_t stable_pa_idx(lwasapi_pa_server_t *srv, uint32_t fac, const char *key) {
    if (!srv || !key || !key[0] || fac >= 3U) return PA_INVALID_INDEX;
    for (uint32_t i = 0; i < PA_MAX_STABLE_IDS; i++) {
        if (srv->stable_ids[i].used && srv->stable_ids[i].facility == fac &&
            strcmp(srv->stable_ids[i].key, key) == 0) {
            return srv->stable_ids[i].idx;
        }
    }
    for (uint32_t i = 0; i < PA_MAX_STABLE_IDS; i++) {
        if (!srv->stable_ids[i].used) {
            srv->stable_ids[i].used = true;
            srv->stable_ids[i].facility = fac;
            srv->stable_ids[i].idx = srv->next_fac_idx[fac]++;
            snprintf(srv->stable_ids[i].key, sizeof(srv->stable_ids[i].key), "%.95s", key);
            return srv->stable_ids[i].idx;
        }
    }
    return srv->next_fac_idx[fac]++;
}

static void enforce_playback_attr(pa_stream_t *s, uint32_t maxl, uint32_t tl, uint32_t minr, uint32_t preb) {
    if (!s || s->frame_size == 0) return;
    uint32_t rate = s->ss.rate ? s->ss.rate : 48000U;
    uint32_t max_ring = PA_STREAM_RING_FRAMES * s->frame_size;
    uint32_t bytes_10ms = (rate >= 100U) ? ((rate / 100U) * s->frame_size) : s->frame_size;
    if (rate >= 1000U && bytes_10ms < s->frame_size * 256U) bytes_10ms = s->frame_size * 256U;
    bytes_10ms = (bytes_10ms / s->frame_size) * s->frame_size;
    if (bytes_10ms == 0) bytes_10ms = s->frame_size;
    uint32_t bytes_5ms = ((bytes_10ms / 2U) / s->frame_size) * s->frame_size;
    if (bytes_5ms < s->frame_size * 128U && rate >= 24000U) bytes_5ms = s->frame_size * 128U;
    if (bytes_5ms == 0) bytes_5ms = s->frame_size;
    uint32_t bytes_20ms = bytes_10ms * 2U;

    s->maxlength = (maxl == PA_INVALID_INDEX || maxl > PA_DEFAULT_MAXLENGTH || maxl > max_ring) ? max_ring : maxl;
    if (s->maxlength < bytes_20ms * 2U && max_ring >= bytes_20ms * 2U) s->maxlength = bytes_20ms * 2U;
    if (s->maxlength > max_ring) s->maxlength = max_ring;
    s->maxlength = (s->maxlength / s->frame_size) * s->frame_size;
    if (s->maxlength == 0) s->maxlength = s->frame_size;

    s->tlength = (tl == PA_INVALID_INDEX || tl == 0) ? (bytes_20ms * 4U) : tl;
    uint32_t min_tlength = bytes_20ms;
    if (s->tlength < min_tlength) s->tlength = min_tlength;
    if (s->tlength > s->maxlength) s->tlength = s->maxlength;
    s->tlength = (s->tlength / s->frame_size) * s->frame_size;
    if (s->tlength == 0) s->tlength = s->frame_size;

    uint32_t def_minreq = (s->tlength / 4U / s->frame_size) * s->frame_size;
    uint32_t min_floor = (s->tlength <= bytes_20ms * 2U) ? bytes_5ms : bytes_10ms;
    if (def_minreq < min_floor) def_minreq = min_floor;

    s->minreq = (minr == PA_INVALID_INDEX || minr == 0) ? def_minreq : minr;
    if (s->minreq < bytes_5ms) s->minreq = bytes_5ms;
    if (s->minreq > s->tlength / 2U && s->tlength >= s->frame_size * 2U) s->minreq = s->tlength / 2U;
    s->minreq = (s->minreq / s->frame_size) * s->frame_size;
    if (s->minreq == 0) s->minreq = s->frame_size;
    if (s->tlength < s->minreq * 2U && s->maxlength >= s->minreq * 2U) {
        s->tlength = s->minreq * 2U;
    } else if (s->tlength < s->minreq) {
        s->tlength = s->minreq;
    }

    s->prebuf = (preb == PA_INVALID_INDEX) ? s->minreq : preb;
    s->prebuf = (s->prebuf / s->frame_size) * s->frame_size;
    if (s->prebuf > s->tlength) s->prebuf = s->tlength;
}

static void enforce_record_attr(pa_stream_t *s, uint32_t frags) {
    if (!s || s->frame_size == 0) return;
    uint32_t rate = s->ss.rate ? s->ss.rate : 48000U;
    uint32_t bytes_10ms = (rate >= 100U) ? ((rate / 100U) * s->frame_size) : s->frame_size;
    uint32_t bytes_20ms = bytes_10ms * 2U;
    if (rate >= 1000U) {
        if (bytes_10ms < s->frame_size * 256U) bytes_10ms = s->frame_size * 256U;
        if (bytes_20ms < s->frame_size * 512U) bytes_20ms = s->frame_size * 512U;
    } else {
        if (bytes_20ms < s->frame_size) bytes_20ms = s->frame_size;
    }
    s->fragsize = (frags == PA_INVALID_INDEX || frags == 0) ? bytes_20ms : frags;
    if (!s->peak_detect && rate >= 1000U && s->fragsize < bytes_10ms) {
        s->fragsize = bytes_10ms;
    }
    if (s->fragsize < s->frame_size) s->fragsize = s->frame_size;
    s->fragsize = (s->fragsize / s->frame_size) * s->frame_size;
    if (s->fragsize == 0) s->fragsize = s->frame_size;
    if (s->maxlength >= s->frame_size * 2U && s->fragsize > s->maxlength / 2U) {
        s->fragsize = (s->maxlength / 2U / s->frame_size) * s->frame_size;
    }
}

/* ============================================================================
 * Tagstruct Dynamic Buffer Helper
 * ============================================================================ */

typedef struct {
    uint8_t *buf;
    size_t   len;
    size_t   cap;
} tagbuf_t;

static void tb_init(tagbuf_t *tb, size_t init_cap) {
    tb->cap = init_cap ? init_cap : 512;
    tb->buf = (uint8_t *)malloc(tb->cap);
    tb->len = 0;
}

static void tb_free(tagbuf_t *tb) {
    free(tb->buf);
    tb->buf = NULL;
    tb->len = tb->cap = 0;
}

static void tb_ensure(tagbuf_t *tb, size_t add) {
    if (tb->len + add <= tb->cap) return;
    size_t ncap = tb->cap * 2;
    while (ncap < tb->len + add) ncap *= 2;
    uint8_t *nb = (uint8_t *)realloc(tb->buf, ncap);
    if (nb) {
        tb->buf = nb;
        tb->cap = ncap;
    }
}

static void tb_bytes(tagbuf_t *tb, const void *p, size_t n) {
    tb_ensure(tb, n);
    if (tb->buf && tb->len + n <= tb->cap) {
        memcpy(tb->buf + tb->len, p, n);
        tb->len += n;
    }
}

static void tb_put_u8(tagbuf_t *tb, uint8_t tag_c, uint8_t v) {
    uint8_t b[2] = { tag_c, v };
    tb_bytes(tb, b, 2);
}

static void tb_put_u32(tagbuf_t *tb, uint8_t tag_c, uint32_t v) {
    uint8_t b[5];
    b[0] = tag_c;
    uint32_t be = htonl(v);
    memcpy(b + 1, &be, 4);
    tb_bytes(tb, b, 5);
}

static void tb_put_u64_raw(tagbuf_t *tb, uint8_t tag_c, uint64_t v) {
    uint8_t b[9];
    b[0] = tag_c;
    uint32_t hi = htonl((uint32_t)(v >> 32));
    uint32_t lo = htonl((uint32_t)(v & 0xFFFFFFFFU));
    memcpy(b + 1, &hi, 4);
    memcpy(b + 5, &lo, 4);
    tb_bytes(tb, b, 9);
}

static void tb_put_bool(tagbuf_t *tb, bool v) {
    uint8_t c = v ? TAG_BOOLEAN_TRUE : TAG_BOOLEAN_FALSE;
    tb_bytes(tb, &c, 1);
}

static void tb_put_str(tagbuf_t *tb, const char *s) {
    if (!s) {
        uint8_t c = TAG_STRING_NULL;
        tb_bytes(tb, &c, 1);
        return;
    }
    uint8_t c = TAG_STRING;
    tb_bytes(tb, &c, 1);
    tb_bytes(tb, s, strlen(s) + 1);
}

static void tb_put_arbitrary(tagbuf_t *tb, const void *p, uint32_t len) {
    tb_put_u32(tb, TAG_ARBITRARY, len);
    if (len > 0 && p) tb_bytes(tb, p, len);
}

static void tb_put_timeval(tagbuf_t *tb, uint32_t sec, uint32_t usec) {
    uint8_t b[9];
    b[0] = TAG_TIMEVAL;
    uint32_t s = htonl(sec), u = htonl(usec);
    memcpy(b + 1, &s, 4);
    memcpy(b + 5, &u, 4);
    tb_bytes(tb, b, 9);
}

static void tb_put_ss(tagbuf_t *tb, const pa_wire_ss_t *ss) {
    uint8_t b[7];
    b[0] = TAG_SAMPLE_SPEC;
    b[1] = ss->format;
    b[2] = ss->channels;
    uint32_t r = htonl(ss->rate);
    memcpy(b + 3, &r, 4);
    tb_bytes(tb, b, 7);
}

static void tb_put_map(tagbuf_t *tb, const pa_wire_map_t *m) {
    uint8_t hdr[2] = { TAG_CHANNEL_MAP, m->channels };
    tb_bytes(tb, hdr, 2);
    if (m->channels > 0) tb_bytes(tb, m->map, m->channels);
}

static void tb_put_cvol(tagbuf_t *tb, const pa_wire_cvol_t *cv) {
    uint8_t hdr[2] = { TAG_CVOLUME, cv->channels };
    tb_bytes(tb, hdr, 2);
    for (uint8_t i = 0; i < cv->channels && i < 32; i++) {
        uint32_t v = htonl(cv->values[i]);
        tb_bytes(tb, &v, 4);
    }
}

static void tb_put_proplist(tagbuf_t *tb, const pa_propbag_t *pb) {
    uint8_t c = TAG_PROPLIST;
    tb_bytes(tb, &c, 1);
    if (pb) {
        for (int i = 0; i < pb->count; i++) {
            uint32_t vlen = (uint32_t)strlen(pb->items[i].val) + 1U;
            tb_put_str(tb, pb->items[i].key);
            tb_put_u32(tb, TAG_U32, vlen);
            tb_put_arbitrary(tb, pb->items[i].val, vlen);
        }
    }
    tb_put_str(tb, NULL);
}

static void tb_put_pcm_format(tagbuf_t *tb) {
    uint8_t c = TAG_FORMAT_INFO;
    tb_bytes(tb, &c, 1);
    tb_put_u8(tb, TAG_U8, 1); /* PA_ENCODING_PCM */
    tb_put_proplist(tb, NULL);
}

/* ============================================================================
 * Tagstruct Reader
 * ============================================================================ */

typedef salsa_pa_tagread_t tagread_t;

static void tr_init(tagread_t *tr, const uint8_t *p, size_t len) {
    salsa_tr_init(tr, p, len);
}

static bool tr_u32(tagread_t *tr, uint32_t *out) {
    return salsa_tr_u32(tr, out);
}

static bool tr_u8(tagread_t *tr, uint8_t *out) {
    return salsa_tr_u8(tr, out);
}

static bool tr_bool(tagread_t *tr, bool *out) {
    return salsa_tr_bool(tr, out);
}

static bool tr_str(tagread_t *tr, const char **out) {
    size_t off = 0, slen = 0;
    bool is_null = false;
    if (!salsa_tr_str_slice(tr, &off, &slen, &is_null)) return false;
    (void)slen;
    *out = is_null ? NULL : (const char *)(tr->p + off);
    return true;
}

static bool tr_arbitrary(tagread_t *tr, const void **out, size_t *out_len) {
    size_t off = 0, blen = 0;
    if (!salsa_tr_arbitrary_slice(tr, &off, &blen)) return false;
    *out = tr->p + off;
    *out_len = blen;
    return true;
}

static bool tr_ss(tagread_t *tr, pa_wire_ss_t *ss) {
    return salsa_tr_ss(tr, ss);
}

static bool tr_map(tagread_t *tr, pa_wire_map_t *m) {
    return salsa_tr_map(tr, m);
}

static bool tr_cvol(tagread_t *tr, pa_wire_cvol_t *cv) {
    return salsa_tr_cvol(tr, cv);
}

static bool tr_timeval(tagread_t *tr, uint32_t *sec, uint32_t *usec) {
    return salsa_tr_timeval(tr, sec, usec);
}

static void propbag_set(pa_propbag_t *pb, const char *k, const char *v) {
    salsa_propbag_set(pb, k, v);
}

static const char *propbag_get(const pa_propbag_t *pb, const char *k) {
    if (!pb || !k) return NULL;
    for (int i = 0; i < pb->count; i++) {
        if (strcmp(pb->items[i].key, k) == 0) return pb->items[i].val;
    }
    return NULL;
}

static bool tr_proplist(tagread_t *tr, pa_propbag_t *pb) {
    return salsa_tr_proplist(tr, pb);
}

static bool tr_skip_format_info(tagread_t *tr) {
    return salsa_tr_skip_format_info(tr);
}

/* ============================================================================
 * Utilities: Helpers, Sample Conversion & Socket Queueing
 * ============================================================================ */

static uint32_t pa_format_sample_size(uint8_t fmt) {
    switch (fmt) {
        case FMT_U8:
        case FMT_ALAW:
        case FMT_ULAW:    return 1;
        case FMT_S16LE:
        case FMT_S16BE:   return 2;
        case FMT_S24LE:
        case FMT_S24BE:   return 3;
        default:          return 4;
    }
}

static uint32_t pa_format_sample_bits(uint8_t fmt) {
    switch (fmt) {
        case FMT_U8:
        case FMT_ALAW:
        case FMT_ULAW:     return 16;
        case FMT_S16LE:
        case FMT_S16BE:    return 16;
        case FMT_S24LE:
        case FMT_S24BE:
        case FMT_S24_32LE:
        case FMT_S24_32BE: return 24;
        case FMT_S32LE:
        case FMT_S32BE:
        case FMT_FLOAT32LE:
        case FMT_FLOAT32BE:return 32;
        default:           return 24;
    }
}

static uint32_t pct_to_pa_vol(uint32_t pct) {
    double lin = (double)pct / 100.0;
    if (lin <= 0.0) return 0;
    /* PA uses cubic dB curve: v = cbrt(lin) * PA_VOLUME_NORM */
    double sw = cbrt(lin) * (double)PA_VOLUME_NORM;
    if (sw > (double)PA_VOLUME_NORM * 1.5) sw = (double)PA_VOLUME_NORM * 1.5;
    return (uint32_t)(sw + 0.5);
}


static uint32_t cvol_to_milli(const pa_wire_cvol_t *cv) {
    if (!cv || cv->channels == 0) return 1000;
    uint64_t sum = 0;
    for (uint8_t i = 0; i < cv->channels; i++) sum += cv->values[i];
    double norm = ((double)sum / (double)cv->channels) / (double)PA_VOLUME_NORM;
    double lin = norm * norm * norm;
    uint32_t m = (uint32_t)(lin * 1000.0 + 0.5);
    if (m > 2000) m = 2000;
    return m;
}

static pa_wire_map_t std_channel_map(uint8_t ch) {
    pa_wire_map_t m;
    memset(&m, 0, sizeof(m));
    m.channels = ch ? ch : 2;
    if (m.channels == 1) {
        m.map[0] = 0; /* MONO */
    } else {
        m.map[0] = 1; /* FRONT_LEFT */
        m.map[1] = 2; /* FRONT_RIGHT */
        for (uint8_t i = 2; i < m.channels && i < 32; i++) m.map[i] = (uint8_t)(i + 1);
    }
    return m;
}

static inline float decode_pa_sample(const uint8_t *p, uint8_t fmt) {
    switch (fmt) {
        case FMT_FLOAT32LE: {
            float v; memcpy(&v, p, 4); return v;
        }
        case FMT_FLOAT32BE: {
            uint32_t b = ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
            float v; memcpy(&v, &b, 4); return v;
        }
        case FMT_S16LE: {
            int16_t v = (int16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
            return (float)v * (1.0f / 32768.0f);
        }
        case FMT_S16BE: {
            int16_t v = (int16_t)(((uint16_t)p[0] << 8) | p[1]);
            return (float)v * (1.0f / 32768.0f);
        }
        case FMT_S32LE: {
            int32_t v; memcpy(&v, p, 4);
            return (float)v * (1.0f / 2147483648.0f);
        }
        case FMT_S32BE: {
            int32_t v = (int32_t)(((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3]);
            return (float)v * (1.0f / 2147483648.0f);
        }
        case FMT_S24LE: {
            int32_t v = (int32_t)((uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16));
            if (v & 0x00800000) v |= (int32_t)0xFF000000U;
            return (float)v * (1.0f / 8388608.0f);
        }
        case FMT_S24_32LE: {
            int32_t v; memcpy(&v, p, 4);
            v = (v << 8) >> 8;
            return (float)v * (1.0f / 8388608.0f);
        }
        case FMT_U8:
            return ((float)p[0] - 128.0f) * (1.0f / 128.0f);
        default:
            return 0.0f;
    }
}

static inline void encode_pa_sample(uint8_t *p, uint8_t fmt, float x) {
    if (x > 1.0f) x = 1.0f;
    if (x < -1.0f) x = -1.0f;
    switch (fmt) {
        case FMT_FLOAT32LE: memcpy(p, &x, 4); break;
        case FMT_S16LE: {
            int16_t v = (int16_t)(x * 32767.0f);
            memcpy(p, &v, 2);
            break;
        }
        case FMT_S32LE: {
            int32_t v = (int32_t)(x * 2147483647.0f);
            memcpy(p, &v, 4);
            break;
        }
        case FMT_S24LE: {
            int32_t v = (int32_t)(x * 8388607.0f);
            p[0] = (uint8_t)(v & 0xFF);
            p[1] = (uint8_t)((v >> 8) & 0xFF);
            p[2] = (uint8_t)((v >> 16) & 0xFF);
            break;
        }
        case FMT_S24_32LE: {
            int32_t v = (int32_t)(x * 8388607.0f);
            memcpy(p, &v, 4);
            break;
        }
        case FMT_U8:
            p[0] = (uint8_t)((x * 127.0f) + 128.0f);
            break;
        default: {
            int16_t v = (int16_t)(x * 32767.0f);
            memcpy(p, &v, 2);
            break;
        }
    }
}

static bool queue_packet(pa_client_t *c, uint32_t channel, const uint8_t *payload, size_t len) {
    if (!c || !c->in_use || c->out_count >= PA_MAX_OUT_QUEUE) return false;
    uint8_t *pkt = (uint8_t *)malloc(PA_PSTREAM_DESC_BYTES + len);
    if (!pkt) return false;

    uint32_t hdr[5] = {
        htonl((uint32_t)len),
        htonl(channel),
        0,
        0,
        0
    };
    memcpy(pkt, hdr, PA_PSTREAM_DESC_BYTES);
    if (len > 0 && payload) memcpy(pkt + PA_PSTREAM_DESC_BYTES, payload, len);

    pa_out_pkt_t *q = &c->out_q[c->out_tail];
    q->channel = channel;
    q->flags   = 0;
    q->data    = pkt;
    q->len     = PA_PSTREAM_DESC_BYTES + len;
    q->sent    = 0;
    c->out_tail = (c->out_tail + 1) % PA_MAX_OUT_QUEUE;
    c->out_count++;
    return true;
}

static void queue_tb(pa_client_t *c, tagbuf_t *tb) {
    queue_packet(c, PA_PSTREAM_CTRL_CHANNEL, tb->buf, tb->len);
    tb_free(tb);
}

static void reply_simple_ack(pa_client_t *c, uint32_t tag) {
    tagbuf_t tb;
    tb_init(&tb, 32);
    tb_put_u32(&tb, TAG_U32, CMD_REPLY);
    tb_put_u32(&tb, TAG_U32, tag);
    queue_tb(c, &tb);
}

static const char *pa_cmd_name(uint32_t cmd) {
    switch (cmd) {
        case CMD_CREATE_PLAYBACK_STREAM:        return "PA_CMD_CREATE_PB";
        case CMD_DELETE_PLAYBACK_STREAM:        return "PA_CMD_DEL_PB";
        case CMD_CREATE_RECORD_STREAM:          return "PA_CMD_CREATE_REC";
        case CMD_DELETE_RECORD_STREAM:          return "PA_CMD_DEL_REC";
        case CMD_EXIT:                          return "PA_CMD_EXIT";
        case CMD_AUTH:                          return "PA_CMD_AUTH";
        case CMD_SET_CLIENT_NAME:               return "PA_CMD_SET_CNAME";
        case CMD_LOOKUP_SINK:                   return "PA_CMD_LOOKUP_SINK";
        case CMD_LOOKUP_SOURCE:                 return "PA_CMD_LOOKUP_SRC";
        case CMD_DRAIN_PLAYBACK_STREAM:         return "PA_CMD_DRAIN_PB";
        case CMD_STAT:                          return "PA_CMD_STAT";
        case CMD_GET_PLAYBACK_LATENCY:          return "PA_CMD_GET_PB_LAT";
        case CMD_GET_SERVER_INFO:               return "PA_CMD_SERVER_INFO";
        case CMD_GET_SINK_INFO:                 return "PA_CMD_SINK_INFO";
        case CMD_GET_SINK_INFO_LIST:            return "PA_CMD_SINK_LIST";
        case CMD_GET_SOURCE_INFO:               return "PA_CMD_SRC_INFO";
        case CMD_GET_SOURCE_INFO_LIST:          return "PA_CMD_SRC_LIST";
        case CMD_GET_MODULE_INFO:               return "PA_CMD_MOD_INFO";
        case CMD_GET_MODULE_INFO_LIST:          return "PA_CMD_MOD_LIST";
        case CMD_GET_CLIENT_INFO:               return "PA_CMD_CLI_INFO";
        case CMD_GET_CLIENT_INFO_LIST:          return "PA_CMD_CLI_LIST";
        case CMD_GET_SINK_INPUT_INFO:           return "PA_CMD_SI_INFO";
        case CMD_GET_SINK_INPUT_INFO_LIST:      return "PA_CMD_SI_LIST";
        case CMD_GET_SOURCE_OUTPUT_INFO:        return "PA_CMD_SO_INFO";
        case CMD_GET_SOURCE_OUTPUT_INFO_LIST:   return "PA_CMD_SO_LIST";
        case CMD_GET_SAMPLE_INFO_LIST:          return "PA_CMD_SAMPLE_LIST";
        case CMD_SUBSCRIBE:                     return "PA_CMD_SUBSCRIBE";
        case CMD_SET_SINK_VOLUME:               return "PA_CMD_SET_SINK_VOL";
        case CMD_SET_SINK_INPUT_VOLUME:         return "PA_CMD_SET_SI_VOL";
        case CMD_SET_SOURCE_VOLUME:             return "PA_CMD_SET_SRC_VOL";
        case CMD_SET_SINK_MUTE:                 return "PA_CMD_SET_SINK_MUTE";
        case CMD_SET_SOURCE_MUTE:               return "PA_CMD_SET_SRC_MUTE";
        case CMD_CORK_PLAYBACK_STREAM:          return "PA_CMD_CORK_PB";
        case CMD_FLUSH_PLAYBACK_STREAM:         return "PA_CMD_FLUSH_PB";
        case CMD_TRIGGER_PLAYBACK_STREAM:       return "PA_CMD_TRIG_PB";
        case CMD_SET_DEFAULT_SINK:              return "PA_CMD_SET_DEF_SINK";
        case CMD_SET_DEFAULT_SOURCE:            return "PA_CMD_SET_DEF_SRC";
        case CMD_SET_PLAYBACK_STREAM_NAME:      return "PA_CMD_SET_PB_NAME";
        case CMD_SET_RECORD_STREAM_NAME:        return "PA_CMD_SET_REC_NAME";
        case CMD_KILL_CLIENT:                   return "PA_CMD_KILL_CLI";
        case CMD_KILL_SINK_INPUT:               return "PA_CMD_KILL_SI";
        case CMD_KILL_SOURCE_OUTPUT:            return "PA_CMD_KILL_SO";
        case CMD_GET_RECORD_LATENCY:            return "PA_CMD_GET_REC_LAT";
        case CMD_CORK_RECORD_STREAM:            return "PA_CMD_CORK_REC";
        case CMD_FLUSH_RECORD_STREAM:           return "PA_CMD_FLUSH_REC";
        case CMD_PREBUF_PLAYBACK_STREAM:        return "PA_CMD_PREBUF_PB";
        case CMD_MOVE_SINK_INPUT:               return "PA_CMD_MOVE_SI";
        case CMD_MOVE_SOURCE_OUTPUT:            return "PA_CMD_MOVE_SO";
        case CMD_SET_SINK_INPUT_MUTE:           return "PA_CMD_SET_SI_MUTE";
        case CMD_SET_PLAYBACK_STREAM_BUFFER_ATTR: return "PA_CMD_SET_PB_ATTR";
        case CMD_SET_RECORD_STREAM_BUFFER_ATTR:   return "PA_CMD_SET_REC_ATTR";
        case CMD_GET_CARD_INFO:                 return "PA_CMD_CARD_INFO";
        case CMD_GET_CARD_INFO_LIST:            return "PA_CMD_CARD_LIST";
        case CMD_SET_CARD_PROFILE:              return "PA_CMD_SET_PROFILE";
        case CMD_SET_SOURCE_OUTPUT_VOLUME:      return "PA_CMD_SET_SO_VOL";
        case CMD_SET_SOURCE_OUTPUT_MUTE:        return "PA_CMD_SET_SO_MUTE";
        default:                                return "PA_CMD_OTHER";
    }
}

static bool pa_is_lifecycle_cmd(uint32_t cmd) {
    return cmd == CMD_AUTH || cmd == CMD_SET_CLIENT_NAME || cmd == CMD_EXIT ||
           cmd == CMD_CREATE_PLAYBACK_STREAM || cmd == CMD_CREATE_RECORD_STREAM ||
           cmd == CMD_DELETE_PLAYBACK_STREAM || cmd == CMD_DELETE_RECORD_STREAM ||
           cmd == CMD_CORK_PLAYBACK_STREAM || cmd == CMD_CORK_RECORD_STREAM;
}

static void reply_err(pa_client_t *c, uint32_t tag, uint32_t pa_err) {
    lwasapi_dbg_lvl_t lvl = (pa_err == ERR_NOEXTENSION) ? LWASAPI_DBG_TRACE : LWASAPI_DBG_WARN;
    lwasapi_debug_emit_vals(LWASAPI_DBG_PA, lvl, c ? c->pid : 0, c ? c->index : 0,
                            (int32_t)pa_err, (int32_t)tag, -(int32_t)pa_err,
                            "PA_ERR", (c && c->name[0]) ? c->name : "pa-client",
                            "PA reply error=%u on tag=%u", pa_err, tag);
    tagbuf_t tb;
    tb_init(&tb, 32);
    tb_put_u32(&tb, TAG_U32, CMD_ERROR);
    tb_put_u32(&tb, TAG_U32, tag);
    tb_put_u32(&tb, TAG_U32, pa_err);
    queue_tb(c, &tb);
}

static void tb_begin_reply(tagbuf_t *tb, uint32_t tag, size_t cap) {
    tb_init(tb, cap);
    tb_put_u32(tb, TAG_U32, CMD_REPLY);
    tb_put_u32(tb, TAG_U32, tag);
}

/* ============================================================================
 * Device Naming Bridge: Salsa Devices <-> PulseAudio Sinks / Sources / Cards
 * ============================================================================ */

static void make_pa_sink_name(const char *salsa_id, char *out, size_t out_len) {
    if (!salsa_id || !salsa_id[0] || strcmp(salsa_id, "virtual") == 0) {
        snprintf(out, out_len, "salsa_out.virtual");
        return;
    }
    char clean[64];
    snprintf(clean, sizeof(clean), "%s", salsa_id);
    for (int i = 0; clean[i]; i++) {
        if (clean[i] == ':' || clean[i] == ',') clean[i] = '_';
    }
    snprintf(out, out_len, "salsa_out.%s", clean);
}

static void make_pa_source_name(const char *salsa_id, char *out, size_t out_len) {
    if (!salsa_id || !salsa_id[0]) {
        snprintf(out, out_len, "salsa_in.none");
        return;
    }
    char clean[64];
    snprintf(clean, sizeof(clean), "%s", salsa_id);
    for (int i = 0; clean[i]; i++) {
        if (clean[i] == ':' || clean[i] == ',') clean[i] = '_';
    }
    snprintf(out, out_len, "salsa_in.%s", clean);
}

static bool pa_name_to_salsa_id(const lwasapi_device_list_t *dl, const char *pa_name,
                                bool is_capture, char *out_id, size_t out_len) {
    if (!pa_name || !dl) return false;
    if (strcmp(pa_name, "salsa_out.virtual") == 0 && !is_capture) {
        snprintf(out_id, out_len, "virtual");
        return true;
    }
    for (uint32_t i = 0; i < dl->num_devices; i++) {
        if ((bool)dl->devices[i].is_capture != is_capture) continue;
        char candidate[128];
        if (is_capture) make_pa_source_name(dl->devices[i].id, candidate, sizeof(candidate));
        else make_pa_sink_name(dl->devices[i].id, candidate, sizeof(candidate));
        if (strcmp(candidate, pa_name) == 0 || strcmp(dl->devices[i].id, pa_name) == 0) {
            snprintf(out_id, out_len, "%s", dl->devices[i].id);
            return true;
        }
    }
    return false;
}

static void refresh_status_only(lwasapi_pa_server_t *srv) {
    if (srv && srv->cbs.get_status) {
        srv->cbs.get_status(srv->cbs.user_ctx, &srv->cached_status);
    }
}

static void refresh_device_cache_force(lwasapi_pa_server_t *srv) {
    refresh_status_only(srv);
    if (srv && srv->cbs.get_devices) {
        srv->cbs.get_devices(srv->cbs.user_ctx, &srv->cached_devices);
        srv->devices_cache_valid = true;
    }
}

static void refresh_device_cache(lwasapi_pa_server_t *srv) {
    refresh_status_only(srv);
    if (srv && !srv->devices_cache_valid && srv->cbs.get_devices) {
        srv->cbs.get_devices(srv->cbs.user_ctx, &srv->cached_devices);
        srv->devices_cache_valid = true;
    }
}

static void broadcast_sub_event(lwasapi_pa_server_t *srv, uint32_t facility, uint32_t ev_type, uint32_t idx) {
    uint32_t ev = facility | ev_type;
    for (int i = 0; i < PA_MAX_CLIENTS; i++) {
        pa_client_t *c = &srv->clients[i];
        if (!c->in_use || !c->authed) continue;
        if (!(c->sub_mask & (1U << (facility & 0x0FU)))) continue;
        tagbuf_t tb;
        tb_init(&tb, 48);
        tb_put_u32(&tb, TAG_U32, CMD_SUBSCRIBE_EVENT);
        tb_put_u32(&tb, TAG_U32, PA_INVALID_INDEX);
        tb_put_u32(&tb, TAG_U32, ev);
        tb_put_u32(&tb, TAG_U32, idx);
        queue_tb(c, &tb);
    }
}

/* ============================================================================
 * Introspection Response Builders
 * ============================================================================ */

static void get_default_sink_pa_name(lwasapi_pa_server_t *srv, char *out, size_t len) {
    if (srv->cached_status.active_playback_id[0]) {
        make_pa_sink_name(srv->cached_status.active_playback_id, out, len);
        return;
    }
    for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
        if (!srv->cached_devices.devices[i].is_capture && srv->cached_devices.devices[i].is_active) {
            make_pa_sink_name(srv->cached_devices.devices[i].id, out, len);
            return;
        }
    }
    snprintf(out, len, "salsa_out.virtual");
}

static void get_default_source_pa_name(lwasapi_pa_server_t *srv, char *out, size_t len) {
    if (srv->cached_status.active_capture_id[0]) {
        make_pa_source_name(srv->cached_status.active_capture_id, out, len);
        return;
    }
    for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
        if (srv->cached_devices.devices[i].is_capture) {
            make_pa_source_name(srv->cached_devices.devices[i].id, out, len);
            return;
        }
    }
    char sink_n[128];
    get_default_sink_pa_name(srv, sink_n, sizeof(sink_n));
    snprintf(out, len, "%.118s.monitor", sink_n);
}

static void make_mon_key(const char *dev_id, char *out, size_t len) {
    snprintf(out, len, "mon:%.58s", (dev_id && dev_id[0]) ? dev_id : "virtual");
}

static uint32_t find_card_idx_for_id(lwasapi_pa_server_t *srv, const lwasapi_device_list_t *dl, const char *dev_id) {
    if (!srv || !dl || !dev_id || !dev_id[0] || strcmp(dev_id, "virtual") == 0) return PA_INVALID_INDEX;
    for (uint32_t i = 0; i < dl->num_devices; i++) {
        if (strcmp(dl->devices[i].id, dev_id) == 0) {
            return stable_pa_idx(srv, PA_FAC_CARD, dev_id);
        }
    }
    return PA_INVALID_INDEX;
}

static uint32_t find_monitor_src_idx_for_sink(lwasapi_pa_server_t *srv, uint32_t sink_idx) {
    if (!srv) return PA_INVALID_INDEX;
    uint32_t pb_cnt = 0;
    for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
        const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
        if (d->is_capture) continue;
        pb_cnt++;
        if (stable_pa_idx(srv, PA_FAC_SINK, d->id) == sink_idx) {
            char mk[64];
            make_mon_key(d->id, mk, sizeof(mk));
            return stable_pa_idx(srv, PA_FAC_SOURCE, mk);
        }
    }
    if (pb_cnt == 0) {
        return stable_pa_idx(srv, PA_FAC_SOURCE, "mon:virtual");
    }
    return PA_INVALID_INDEX;
}

static void resolve_pa_target_name(lwasapi_pa_server_t *srv, bool is_source,
                                   const char *req_nm, char *out, size_t len) {
    if (!req_nm) {
        if (is_source) get_default_source_pa_name(srv, out, len);
        else get_default_sink_pa_name(srv, out, len);
        return;
    }
    if (strcmp(req_nm, "@DEFAULT_SINK@") == 0) {
        get_default_sink_pa_name(srv, out, len);
    } else if (is_source && strcmp(req_nm, "@DEFAULT_SOURCE@") == 0) {
        get_default_source_pa_name(srv, out, len);
    } else if (is_source && strcmp(req_nm, "@DEFAULT_MONITOR@") == 0) {
        char sink_n[128];
        get_default_sink_pa_name(srv, sink_n, sizeof(sink_n));
        snprintf(out, len, "%.118s.monitor", sink_n);
    } else {
        snprintf(out, len, "%s", req_nm);
    }
}

static bool resolve_sink_idx_and_name(lwasapi_pa_server_t *srv, uint32_t req_idx, const char *req_nm,
                                      uint32_t *out_idx, char *out_nm, size_t out_len) {
    char tgt[160] = {0};
    if (req_idx == PA_INVALID_INDEX) {
        resolve_pa_target_name(srv, false, req_nm, tgt, sizeof(tgt));
    }
    uint32_t pb_cnt = 0;
    for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
        const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
        if (d->is_capture) continue;
        pb_cnt++;
        uint32_t sidx = stable_pa_idx(srv, PA_FAC_SINK, d->id);
        char pa_n[128];
        make_pa_sink_name(d->id, pa_n, sizeof(pa_n));
        if ((req_idx != PA_INVALID_INDEX && req_idx == sidx) ||
            (req_idx == PA_INVALID_INDEX && strcmp(tgt, pa_n) == 0)) {
            if (out_idx) *out_idx = sidx;
            if (out_nm) snprintf(out_nm, out_len, "%s", pa_n);
            return true;
        }
    }
    if (pb_cnt == 0) {
        uint32_t vidx = stable_pa_idx(srv, PA_FAC_SINK, "virtual");
        if (req_idx == vidx || (req_idx == PA_INVALID_INDEX && (!req_nm || strcmp(tgt, "salsa_out.virtual") == 0))) {
            if (out_idx) *out_idx = vidx;
            if (out_nm) snprintf(out_nm, out_len, "salsa_out.virtual");
            return true;
        }
    }
    return false;
}

static bool resolve_source_idx_and_name(lwasapi_pa_server_t *srv, uint32_t req_idx, const char *req_nm,
                                        uint32_t *out_idx, char *out_nm, size_t out_len, bool *out_is_mon) {
    char tgt[160] = {0};
    if (req_idx == PA_INVALID_INDEX) {
        resolve_pa_target_name(srv, true, req_nm, tgt, sizeof(tgt));
    }
    for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
        const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
        if (!d->is_capture) continue;
        uint32_t src_idx = stable_pa_idx(srv, PA_FAC_SOURCE, d->id);
        char pa_n[128];
        make_pa_source_name(d->id, pa_n, sizeof(pa_n));
        if ((req_idx != PA_INVALID_INDEX && req_idx == src_idx) ||
            (req_idx == PA_INVALID_INDEX && strcmp(tgt, pa_n) == 0)) {
            if (out_idx) *out_idx = src_idx;
            if (out_nm) snprintf(out_nm, out_len, "%s", pa_n);
            if (out_is_mon) *out_is_mon = false;
            return true;
        }
    }
    uint32_t pb_cnt = 0;
    for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
        const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
        if (d->is_capture) continue;
        pb_cnt++;
        char mk[64], sink_n[128], mon_n[160];
        make_mon_key(d->id, mk, sizeof(mk));
        uint32_t src_idx = stable_pa_idx(srv, PA_FAC_SOURCE, mk);
        make_pa_sink_name(d->id, sink_n, sizeof(sink_n));
        snprintf(mon_n, sizeof(mon_n), "%.118s.monitor", sink_n);
        if ((req_idx != PA_INVALID_INDEX && req_idx == src_idx) ||
            (req_idx == PA_INVALID_INDEX && strcmp(tgt, mon_n) == 0)) {
            if (out_idx) *out_idx = src_idx;
            if (out_nm) snprintf(out_nm, out_len, "%s", mon_n);
            if (out_is_mon) *out_is_mon = true;
            return true;
        }
    }
    if (pb_cnt == 0) {
        uint32_t vmon = stable_pa_idx(srv, PA_FAC_SOURCE, "mon:virtual");
        if (req_idx == vmon || (req_idx == PA_INVALID_INDEX && (!req_nm || strcmp(tgt, "salsa_out.virtual.monitor") == 0))) {
            if (out_idx) *out_idx = vmon;
            if (out_nm) snprintf(out_nm, out_len, "salsa_out.virtual.monitor");
            if (out_is_mon) *out_is_mon = true;
            return true;
        }
    }
    return false;
}

static uint32_t find_active_sink_idx(lwasapi_pa_server_t *srv) {
    uint32_t idx = 0;
    char nm[128];
    if (resolve_sink_idx_and_name(srv, PA_INVALID_INDEX, NULL, &idx, nm, sizeof(nm))) return idx;
    return 0;
}

static uint32_t find_active_source_idx(lwasapi_pa_server_t *srv) {
    uint32_t idx = 0;
    char nm[160];
    bool is_mon = false;
    if (resolve_source_idx_and_name(srv, PA_INVALID_INDEX, NULL, &idx, nm, sizeof(nm), &is_mon)) return idx;
    return 0;
}

static bool source_idx_to_salsa_id(lwasapi_pa_server_t *srv, const lwasapi_device_list_t *dl,
                                   uint32_t target_src_idx, char *out_id, size_t out_len) {
    if (!srv || !dl || !out_id || out_len == 0) return false;
    for (uint32_t i = 0; i < dl->num_devices; i++) {
        if (!dl->devices[i].is_capture) continue;
        if (stable_pa_idx(srv, PA_FAC_SOURCE, dl->devices[i].id) == target_src_idx) {
            snprintf(out_id, out_len, "%s", dl->devices[i].id);
            return true;
        }
    }
    return false;
}

static bool sink_idx_to_salsa_id(lwasapi_pa_server_t *srv, const lwasapi_device_list_t *dl,
                                 uint32_t target_sink_idx, char *out_id, size_t out_len) {
    if (!srv || !dl || !out_id || out_len == 0) return false;
    uint32_t pb_cnt = 0;
    for (uint32_t i = 0; i < dl->num_devices; i++) {
        if (dl->devices[i].is_capture) continue;
        pb_cnt++;
        if (stable_pa_idx(srv, PA_FAC_SINK, dl->devices[i].id) == target_sink_idx) {
            snprintf(out_id, out_len, "%s", dl->devices[i].id);
            return true;
        }
    }
    if (pb_cnt == 0 && target_sink_idx == stable_pa_idx(srv, PA_FAC_SINK, "virtual")) {
        snprintf(out_id, out_len, "virtual");
        return true;
    }
    return false;
}

static void stream_update_device_id_locked(lwasapi_pa_server_t *srv, pa_stream_t *s) {
    if (!srv || !s) return;
    s->device_id[0] = '\0';
    if (s->is_playback) {
        if (!sink_idx_to_salsa_id(srv, &srv->cached_devices, s->target_dev_idx,
                                  s->device_id, sizeof(s->device_id))) {
            snprintf(s->device_id, sizeof(s->device_id), "%s",
                     srv->cached_status.active_playback_id[0] ? srv->cached_status.active_playback_id : "virtual");
        }
    } else if (s->is_monitor) {
        snprintf(s->device_id, sizeof(s->device_id), "monitor");
    } else if (!source_idx_to_salsa_id(srv, &srv->cached_devices, s->target_dev_idx,
                                       s->device_id, sizeof(s->device_id))) {
        snprintf(s->device_id, sizeof(s->device_id), "%s",
                 srv->cached_status.active_capture_id[0] ? srv->cached_status.active_capture_id : "virtual");
    }
}

static void init_app_vols_path(lwasapi_pa_server_t *srv) {
    if (!srv) return;
    const char *home = getenv("HOME");
    if (!home || home[0] != '/') {
        struct passwd *pw = getpwuid(getuid());
        home = (pw && pw->pw_dir && pw->pw_dir[0] == '/') ? pw->pw_dir : NULL;
    }
    if (!home) {
        srv->app_vols_path[0] = '\0';
        return;
    }
    char cfg_dir[256], app_dir[288];
    snprintf(cfg_dir, sizeof(cfg_dir), "%s/.config", home);
    mkdir(cfg_dir, 0700);
    snprintf(app_dir, sizeof(app_dir), "%s/lwasapi", cfg_dir);
    mkdir(app_dir, 0700);
    snprintf(srv->app_vols_path, sizeof(srv->app_vols_path), "%s/app_volumes.conf", app_dir);
}

static bool sanitize_app_vol_key(bool is_playback, const char *raw, char *out, size_t out_sz) {
    if (!out || out_sz < 16) return false;
    out[0] = '\0';
    if (!raw || !raw[0]) return false;
    const char *base = strrchr(raw, '/');
    if (base && base[1]) raw = base + 1;

    int n = snprintf(out, out_sz, "%s", is_playback ? "play:" : "rec:");
    if (n < 0 || (size_t)n >= out_sz) return false;
    size_t pos = (size_t)n;
    size_t added = 0;
    for (const unsigned char *p = (const unsigned char *)raw; *p && pos + 1 < out_sz; p++) {
        unsigned char c = *p;
        if (c >= 'A' && c <= 'Z') c = (unsigned char)(c + ('a' - 'A'));
        if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-') {
            out[pos++] = (char)c;
            added++;
        } else if (c == ' ') {
            out[pos++] = '_';
            added++;
        }
    }
    out[pos] = '\0';
    return added > 0;
}

static bool build_stream_app_vol_key(const pa_stream_t *s, char *out, size_t out_sz) {
    if (!s || s->peak_detect) return false;
    const char *raw = propbag_get(&s->props, "application.process.binary");
    if ((!raw || !raw[0]) && s->client) raw = propbag_get(&s->client->props, "application.process.binary");
    if (!raw || !raw[0]) raw = propbag_get(&s->props, "application.name");
    if ((!raw || !raw[0]) && s->client) raw = propbag_get(&s->client->props, "application.name");
    if ((!raw || !raw[0]) && s->client && s->client->name[0]) raw = s->client->name;
    if (!raw || !raw[0]) raw = s->name;
    return sanitize_app_vol_key(s->is_playback, raw, out, out_sz);
}

static void load_app_volumes_from_disk(lwasapi_pa_server_t *srv) {
    if (!srv || !srv->app_vols_path[0]) return;
    int fd = open(srv->app_vols_path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (fd < 0) return;
    struct stat st;
    if (fstat(fd, &st) != 0 || !S_ISREG(st.st_mode) || st.st_size <= 0 || st.st_size > 262144) {
        close(fd);
        return;
    }
    FILE *fp = fdopen(fd, "r");
    if (!fp) {
        close(fd);
        return;
    }
    char line[256];
    srv->num_app_vols = 0;
    while (fgets(line, sizeof(line), fp) && srv->num_app_vols < PA_MAX_APP_VOLUMES) {
        line[strcspn(line, "\r\n")] = '\0';
        if (!line[0] || line[0] == '#') continue;
        char *eq = strchr(line, '=');
        if (!eq) continue;
        *eq = '\0';
        const char *k = line;
        const char *v = eq + 1;
        char clean_k[128] = {0};
        bool is_pb = true;
        const char *raw_k = k;
        if (strncmp(k, "play:", 5) == 0) {
            is_pb = true;
            raw_k = k + 5;
        } else if (strncmp(k, "rec:", 4) == 0) {
            is_pb = false;
            raw_k = k + 4;
        } else {
            continue;
        }
        if (!sanitize_app_vol_key(is_pb, raw_k, clean_k, sizeof(clean_k))) continue;

        char *endp = NULL;
        unsigned long milli = strtoul(v, &endp, 10);
        if (!endp || endp == v || milli > 1500UL) continue;
        uint8_t muted = 0;
        if (*endp == ',') {
            unsigned long m = strtoul(endp + 1, NULL, 10);
            muted = (m != 0UL) ? 1U : 0U;
        }
        uint32_t idx = srv->num_app_vols++;
        snprintf(srv->app_vols[idx].key, sizeof(srv->app_vols[idx].key), "%s", clean_k);
        srv->app_vols[idx].volume_milli = (uint32_t)milli;
        srv->app_vols[idx].muted = muted;
    }
    fclose(fp);
}

static void save_app_volumes_locked_or_snap(pthread_mutex_t *file_lock, const char *path,
                                            const char keys[][128], const uint32_t *millis,
                                            const uint8_t *mutes, uint32_t count) {
    if (!path || !path[0] || !file_lock) return;
    char tmp[360];
    snprintf(tmp, sizeof(tmp), "%s.tmp.%d", path, (int)getpid());
    pthread_mutex_lock(file_lock);
    unlink(tmp);
    int fd = open(tmp, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
    if (fd < 0) {
        pthread_mutex_unlock(file_lock);
        return;
    }
    FILE *fp = fdopen(fd, "w");
    if (!fp) {
        close(fd);
        unlink(tmp);
        pthread_mutex_unlock(file_lock);
        return;
    }
    fprintf(fp, "# Salsa Persistent Per-App Volumes (key=volume_milli,muted)\n");
    for (uint32_t i = 0; i < count; i++) {
        fprintf(fp, "%s=%u,%u\n", keys[i], millis[i], (unsigned)mutes[i]);
    }
    fflush(fp);
    fsync(fileno(fp));
    fclose(fp);
    if (rename(tmp, path) != 0) {
        unlink(tmp);
    }
    pthread_mutex_unlock(file_lock);
}

static uint32_t collect_app_volumes_snapshot_locked(const lwasapi_pa_server_t *srv,
                                                    char keys[][128], uint32_t *millis,
                                                    uint8_t *mutes, char *out_path, size_t path_sz) {
    if (!srv || !srv->app_vols_path[0]) return 0;
    uint32_t cnt = srv->num_app_vols;
    if (cnt > PA_MAX_APP_VOLUMES) cnt = PA_MAX_APP_VOLUMES;
    for (uint32_t i = 0; i < cnt; i++) {
        snprintf(keys[i], 128, "%s", srv->app_vols[i].key);
        millis[i] = srv->app_vols[i].volume_milli;
        mutes[i] = srv->app_vols[i].muted;
    }
    if (out_path && path_sz > 0) snprintf(out_path, path_sz, "%s", srv->app_vols_path);
    return cnt;
}

static void write_app_volumes_from_srv_locked(lwasapi_pa_server_t *srv) {
    if (!srv || !srv->app_vols_path[0]) return;
    char keys[PA_MAX_APP_VOLUMES][128];
    uint32_t millis[PA_MAX_APP_VOLUMES];
    uint8_t mutes[PA_MAX_APP_VOLUMES];
    char save_path[320] = {0};
    uint32_t cnt = collect_app_volumes_snapshot_locked(srv, keys, millis, mutes, save_path, sizeof(save_path));
    if (save_path[0]) {
        save_app_volumes_locked_or_snap(&srv->vol_file_lock, save_path, keys, millis, mutes, cnt);
    }
}

static void upsert_app_vol_kv_locked(lwasapi_pa_server_t *srv, const char *key,
                                     uint32_t milli, uint8_t muted) {
    if (!srv || !key || !key[0]) return;
    if (milli > 1500U) milli = 1500U;
    int found = -1;
    for (uint32_t i = 0; i < srv->num_app_vols; i++) {
        if (strcmp(srv->app_vols[i].key, key) == 0) {
            found = (int)i;
            break;
        }
    }
    if (found < 0) {
        if (srv->num_app_vols < PA_MAX_APP_VOLUMES) {
            found = (int)(srv->num_app_vols++);
        } else {
            memmove(&srv->app_vols[0], &srv->app_vols[1],
                    (PA_MAX_APP_VOLUMES - 1) * sizeof(srv->app_vols[0]));
            found = PA_MAX_APP_VOLUMES - 1;
        }
        snprintf(srv->app_vols[found].key, sizeof(srv->app_vols[found].key), "%s", key);
    }
    srv->app_vols[found].volume_milli = milli;
    srv->app_vols[found].muted = muted ? 1U : 0U;
}

static void record_app_volume_locked(lwasapi_pa_server_t *srv, const pa_stream_t *s) {
    if (!srv || !s || s->peak_detect) return;
    char key[128];
    if (!build_stream_app_vol_key(s, key, sizeof(key))) return;
    uint32_t milli = atomic_load_explicit(&s->volume_milli, memory_order_relaxed);
    uint8_t muted = atomic_load_explicit(&s->muted, memory_order_relaxed) ? 1U : 0U;
    upsert_app_vol_kv_locked(srv, key, milli, muted);
}

static void apply_saved_app_volume_locked(const lwasapi_pa_server_t *srv, pa_stream_t *s, bool vol_explicit) {
    if (!srv || !s || s->peak_detect) return;
    char key[128];
    if (!build_stream_app_vol_key(s, key, sizeof(key))) return;
    for (uint32_t i = 0; i < srv->num_app_vols; i++) {
        if (strcmp(srv->app_vols[i].key, key) == 0) {
            uint32_t cur_milli = atomic_load_explicit(&s->volume_milli, memory_order_relaxed);
            if (!vol_explicit || cur_milli == 1000U) {
                uint32_t milli = srv->app_vols[i].volume_milli;
                if (milli > 1500U) milli = 1500U;
                uint32_t pv = pct_to_pa_vol(milli / 10U);
                for (uint8_t k = 0; k < s->vol.channels; k++) s->vol.values[k] = pv;
                atomic_store_explicit(&s->volume_milli, milli, memory_order_relaxed);
                atomic_store_explicit(&s->muted, srv->app_vols[i].muted ? 1 : 0, memory_order_relaxed);
            }
            break;
        }
    }
}

static void sync_active_capture_if_explicit(lwasapi_pa_server_t *srv, uint32_t actual_src_idx,
                                            bool is_mon, bool peak_detect, bool explicit_req) {
    if (!srv || is_mon || peak_detect || !explicit_req || !srv->cbs.set_devices) return;
    char salsa_id[64] = {0};
    if (!source_idx_to_salsa_id(srv, &srv->cached_devices, actual_src_idx, salsa_id, sizeof(salsa_id))) return;
    if (salsa_id[0] && strcmp(salsa_id, srv->cached_status.active_capture_id) != 0) {
        srv->cbs.set_devices(srv->cbs.user_ctx, NULL, salsa_id);
        refresh_device_cache(srv);
    }
}

static bool is_voice_comms_stream(const pa_stream_t *s) {
    if (!s || s->is_playback || s->is_monitor || s->peak_detect || s->direct_on_input != PA_INVALID_INDEX) {
        return false;
    }
    const char *role = propbag_get(&s->props, "media.role");
    if (role && (strcasecmp(role, "phone") == 0 || strcasecmp(role, "communication") == 0)) {
        return true;
    }
    if (strcasecmp(s->name, "recStream") == 0) return true;
    const char *cname = (s->client && s->client->name[0]) ? s->client->name : "";
    return (strcasestr(cname, "Discord") != NULL || strcasestr(cname, "WEBRTC") != NULL ||
            strcasestr(cname, "Zoom") != NULL || strcasestr(cname, "TeamSpeak") != NULL ||
            strcasestr(cname, "Mumble") != NULL || strcasestr(cname, "Skype") != NULL ||
            strcasestr(cname, "Slack") != NULL);
}

static void put_sink_entry(tagbuf_t *tb, uint32_t ver, uint32_t sink_idx,
                           uint32_t mon_src_idx, uint32_t card_idx,
                           const char *pa_name, const char *desc, const char *salsa_id,
                           bool is_bt, uint32_t rate, uint32_t vol_pct,
                           uint32_t pa_vol_override, bool muted) {
    pa_wire_ss_t ss = { FMT_FLOAT32LE, 2, rate ? rate : 48000 };
    pa_wire_map_t mp = std_channel_map(2);
    uint32_t pv = (pa_vol_override != PA_INVALID_INDEX) ? pa_vol_override : pct_to_pa_vol(vol_pct);
    pa_wire_cvol_t cv = { 2, { pv, pv } };
    char mon_name[160];
    snprintf(mon_name, sizeof(mon_name), "%s.monitor", pa_name);

    uint32_t flags = 0x0002U | 0x0004U | 0x0020U | 0x0080U; /* LATENCY | HARDWARE | DECIBEL | DYN_LATENCY */
    if (is_bt) flags |= 0x0001U; /* HW_VOLUME_CTRL */

    tb_put_u32(tb, TAG_U32, sink_idx);
    tb_put_str(tb, pa_name);
    tb_put_str(tb, desc);
    tb_put_ss(tb, &ss);
    tb_put_map(tb, &mp);
    tb_put_u32(tb, TAG_U32, 0);     /* module index */
    tb_put_cvol(tb, &cv);
    tb_put_bool(tb, muted);
    tb_put_u32(tb, TAG_U32, mon_src_idx); /* monitor_source index matching source list */
    tb_put_str(tb, mon_name);
    tb_put_u64_raw(tb, TAG_USEC, is_bt ? 20000ULL : 2000ULL);
    tb_put_str(tb, "Salsa-LWASAPI");
    tb_put_u32(tb, TAG_U32, flags);

    if (ver >= 13) {
        pa_propbag_t pb;
        memset(&pb, 0, sizeof(pb));
        propbag_set(&pb, "device.description", desc);
        propbag_set(&pb, "device.class", "sound");
        propbag_set(&pb, "device.api", is_bt ? "bluez" : "alsa");
        propbag_set(&pb, "salsa.device.id", salsa_id);
        propbag_set(&pb, "node.driver", "lwasapi-audiodg");
        tb_put_proplist(tb, &pb);
        tb_put_u64_raw(tb, TAG_USEC, 2000ULL);
    }
    if (ver >= 15) {
        tb_put_u32(tb, TAG_VOLUME, PA_VOLUME_NORM);
        tb_put_u32(tb, TAG_U32, 0); /* PA_SINK_RUNNING */
        tb_put_u32(tb, TAG_U32, 65536);
        tb_put_u32(tb, TAG_U32, card_idx); /* matching card index */
    }
    if (ver >= 16) {
        tb_put_u32(tb, TAG_U32, 1); /* 1 port */
        const char *pname = is_bt ? "headset-output" : "analog-output";
        tb_put_str(tb, pname);
        tb_put_str(tb, desc);
        tb_put_u32(tb, TAG_U32, 9900);
        if (ver >= 24) tb_put_u32(tb, TAG_U32, 2); /* PA_PORT_AVAILABLE_YES */
        if (ver >= 34) {
            tb_put_str(tb, NULL);
            tb_put_u32(tb, TAG_U32, is_bt ? 6 : 2);
        }
        tb_put_str(tb, pname);
    }
    if (ver >= 21) {
        tb_put_u8(tb, TAG_U8, 1);
        tb_put_pcm_format(tb);
    }
}

static void put_source_entry(tagbuf_t *tb, uint32_t ver, uint32_t src_idx, uint32_t card_idx,
                             const char *pa_name, const char *desc, const char *salsa_id,
                             uint32_t mon_of_sink_idx, const char *mon_of_sink_name,
                             uint32_t rate, uint32_t vol_pct,
                             uint32_t pa_vol_override, bool muted) {
    pa_wire_ss_t ss = { FMT_FLOAT32LE, 2, rate ? rate : 48000 };
    pa_wire_map_t mp = std_channel_map(2);
    uint32_t pv = (pa_vol_override != PA_INVALID_INDEX) ? pa_vol_override : pct_to_pa_vol(vol_pct);
    pa_wire_cvol_t cv = { 2, { pv, pv } };
    bool is_mon = (mon_of_sink_idx != PA_INVALID_INDEX);

    uint32_t flags = 0x0002U | 0x0020U | 0x0040U; /* LATENCY | DECIBEL | DYN_LATENCY */
    if (!is_mon) flags |= 0x0004U; /* PA_SOURCE_HARDWARE */

    tb_put_u32(tb, TAG_U32, src_idx);
    tb_put_str(tb, pa_name);
    tb_put_str(tb, desc);
    tb_put_ss(tb, &ss);
    tb_put_map(tb, &mp);
    tb_put_u32(tb, TAG_U32, 0);
    tb_put_cvol(tb, &cv);
    tb_put_bool(tb, muted);
    tb_put_u32(tb, TAG_U32, mon_of_sink_idx);
    tb_put_str(tb, mon_of_sink_name);
    tb_put_u64_raw(tb, TAG_USEC, 2000ULL);
    tb_put_str(tb, "Salsa-LWASAPI");
    tb_put_u32(tb, TAG_U32, flags);

    if (ver >= 13) {
        pa_propbag_t pb;
        memset(&pb, 0, sizeof(pb));
        propbag_set(&pb, "device.description", desc);
        propbag_set(&pb, "device.class", is_mon ? "monitor" : "sound");
        propbag_set(&pb, "device.api", is_mon ? "salsa-monitor" : "alsa");
        if (salsa_id) propbag_set(&pb, "salsa.device.id", salsa_id);
        tb_put_proplist(tb, &pb);
        tb_put_u64_raw(tb, TAG_USEC, 2000ULL);
    }
    if (ver >= 15) {
        tb_put_u32(tb, TAG_VOLUME, PA_VOLUME_NORM);
        tb_put_u32(tb, TAG_U32, 0); /* PA_SOURCE_RUNNING */
        tb_put_u32(tb, TAG_U32, 65536);
        tb_put_u32(tb, TAG_U32, is_mon ? PA_INVALID_INDEX : card_idx);
    }
    if (ver >= 16) {
        if (!is_mon) {
            tb_put_u32(tb, TAG_U32, 1);
            tb_put_str(tb, "analog-input");
            tb_put_str(tb, desc);
            tb_put_u32(tb, TAG_U32, 9900);
            if (ver >= 24) tb_put_u32(tb, TAG_U32, 2);
            if (ver >= 34) {
                tb_put_str(tb, NULL);
                tb_put_u32(tb, TAG_U32, 4);
            }
            tb_put_str(tb, "analog-input");
        } else {
            tb_put_u32(tb, TAG_U32, 0);
            tb_put_str(tb, NULL);
        }
    }
    if (ver >= 22) {
        tb_put_u8(tb, TAG_U8, 1);
        tb_put_pcm_format(tb);
    }
}

static void put_card_entry(tagbuf_t *tb, uint32_t ver, uint32_t card_idx,
                           const lwasapi_device_list_t *dl, const lwasapi_device_entry_t *d) {
    char cname[96];
    if (d->is_bluetooth) snprintf(cname, sizeof(cname), "bluez_card.%s", d->id + 6);
    else snprintf(cname, sizeof(cname), "alsa_card.%s", d->id);
    for (int i = 0; cname[i]; i++) if (cname[i] == ':' || cname[i] == ',') cname[i] = '_';

    bool has_pb = false, has_cap = false;
    if (dl) {
        for (uint32_t i = 0; i < dl->num_devices; i++) {
            if (strcmp(dl->devices[i].id, d->id) == 0) {
                if (dl->devices[i].is_capture) has_cap = true;
                else has_pb = true;
            }
        }
    }
    if (!has_pb && !has_cap) {
        has_pb = !d->is_capture;
        has_cap = d->is_capture != 0;
    }

    tb_put_u32(tb, TAG_U32, card_idx);
    tb_put_str(tb, cname);
    tb_put_u32(tb, TAG_U32, 0);
    tb_put_str(tb, d->is_bluetooth ? "module-bluez5-device.c" : "module-alsa-card.c");

    const char *prof_name = d->is_bluetooth ? "a2dp-sink"
                          : ((has_pb && has_cap) ? "output+input" : (has_cap ? "input" : "output"));
    const char *prof_desc = d->is_bluetooth ? "Native BlueZ A2DP High Fidelity (SBC/AAC)"
                          : ((has_pb && has_cap) ? "Direct Kernel ALSA Duplex" : "Direct Kernel ALSA");

    tb_put_u32(tb, TAG_U32, 1); /* 1 profile */
    tb_put_str(tb, prof_name);
    tb_put_str(tb, prof_desc);
    tb_put_u32(tb, TAG_U32, has_pb ? 1 : 0);
    tb_put_u32(tb, TAG_U32, has_cap ? 1 : 0);
    tb_put_u32(tb, TAG_U32, 100);
    if (ver >= 29) tb_put_u32(tb, TAG_U32, 1); /* available = yes */

    tb_put_str(tb, prof_name);
    pa_propbag_t pb;
    memset(&pb, 0, sizeof(pb));
    propbag_set(&pb, "device.description", d->name);
    propbag_set(&pb, "device.api", d->is_bluetooth ? "bluez" : "alsa");
    propbag_set(&pb, "salsa.device.id", d->id);
    tb_put_proplist(tb, &pb);

    if (ver >= 26) {
        uint32_t n_ports = (has_pb ? 1U : 0U) + (has_cap ? 1U : 0U);
        tb_put_u32(tb, TAG_U32, n_ports);
        if (has_pb) {
            const char *port_name = d->is_bluetooth ? "headset-output" : "analog-output";
            tb_put_str(tb, port_name);
            tb_put_str(tb, d->name);
            tb_put_u32(tb, TAG_U32, 9900);
            tb_put_u32(tb, TAG_U32, 2); /* PA_PORT_AVAILABLE_YES */
            tb_put_u8(tb, TAG_U8, 1);   /* PA_DIRECTION_OUTPUT */
            tb_put_proplist(tb, &pb);
            tb_put_u32(tb, TAG_U32, 1);
            tb_put_str(tb, prof_name);
            if (ver >= 27) tb_put_u64_raw(tb, TAG_S64, 0);
            if (ver >= 34) {
                tb_put_str(tb, NULL);
                tb_put_u32(tb, TAG_U32, d->is_bluetooth ? 6 : 2);
            }
        }
        if (has_cap) {
            tb_put_str(tb, "analog-input");
            tb_put_str(tb, d->name);
            tb_put_u32(tb, TAG_U32, 9900);
            tb_put_u32(tb, TAG_U32, 2); /* PA_PORT_AVAILABLE_YES */
            tb_put_u8(tb, TAG_U8, 2);   /* PA_DIRECTION_INPUT */
            tb_put_proplist(tb, &pb);
            tb_put_u32(tb, TAG_U32, 1);
            tb_put_str(tb, prof_name);
            if (ver >= 27) tb_put_u64_raw(tb, TAG_S64, 0);
            if (ver >= 34) {
                tb_put_str(tb, NULL);
                tb_put_u32(tb, TAG_U32, 4);
            }
        }
    }
}

static void put_sink_input_entry(tagbuf_t *tb, uint32_t ver, const pa_stream_t *s, uint32_t sink_idx) {
    tb_put_u32(tb, TAG_U32, s->index);
    tb_put_str(tb, s->name[0] ? s->name : (s->client ? s->client->name : "Salsa Stream"));
    tb_put_u32(tb, TAG_U32, 0);
    tb_put_u32(tb, TAG_U32, s->client ? s->client->index : 0);
    tb_put_u32(tb, TAG_U32, sink_idx);
    tb_put_ss(tb, &s->ss);
    tb_put_map(tb, &s->map);
    tb_put_cvol(tb, &s->vol);
    tb_put_u64_raw(tb, TAG_USEC, 2000ULL);
    tb_put_u64_raw(tb, TAG_USEC, 2000ULL);
    tb_put_str(tb, "linear");
    tb_put_str(tb, "Salsa-LWASAPI");
    if (ver >= 11) tb_put_bool(tb, atomic_load(&s->muted) != 0);
    if (ver >= 13) tb_put_proplist(tb, &s->props);
    if (ver >= 19) tb_put_bool(tb, atomic_load(&s->corked) != 0);
    if (ver >= 20) {
        tb_put_bool(tb, true);
        tb_put_bool(tb, true);
    }
    if (ver >= 21) tb_put_pcm_format(tb);
}

static void put_source_output_entry(tagbuf_t *tb, uint32_t ver, const pa_stream_t *s, uint32_t src_idx) {
    tb_put_u32(tb, TAG_U32, s->index);
    tb_put_str(tb, s->name[0] ? s->name : (s->client ? s->client->name : "Salsa Capture"));
    tb_put_u32(tb, TAG_U32, 0);
    tb_put_u32(tb, TAG_U32, s->client ? s->client->index : 0);
    tb_put_u32(tb, TAG_U32, src_idx);
    tb_put_ss(tb, &s->ss);
    tb_put_map(tb, &s->map);
    tb_put_u64_raw(tb, TAG_USEC, 2000ULL);
    tb_put_u64_raw(tb, TAG_USEC, 2000ULL);
    tb_put_str(tb, "linear");
    tb_put_str(tb, "Salsa-LWASAPI");
    if (ver >= 13) tb_put_proplist(tb, &s->props);
    if (ver >= 19) tb_put_bool(tb, atomic_load(&s->corked) != 0);
    if (ver >= 22) {
        tb_put_cvol(tb, &s->vol);
        tb_put_bool(tb, atomic_load(&s->muted) != 0);
        tb_put_bool(tb, true);
        tb_put_bool(tb, true);
        tb_put_pcm_format(tb);
    }
}

/* ============================================================================
 * Command Dispatcher
 * ============================================================================ */

static pa_stream_t *alloc_stream(lwasapi_pa_server_t *srv, pa_client_t *c) {
    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        if (!srv->streams[i].in_use) {
            pa_stream_t *s = &srv->streams[i];
            memset(s, 0, sizeof(*s));
            s->in_use          = true;
            s->index           = srv->next_stream_idx++;
            s->channel         = c->next_channel++;
            s->client          = c;
            s->direct_on_input = PA_INVALID_INDEX;
            s->cap_smooth_gain = 1.0f;
            atomic_store(&s->volume_milli, 1000);
            return s;
        }
    }
    return NULL;
}

static pa_stream_t *find_client_stream(lwasapi_pa_server_t *srv, pa_client_t *c, uint32_t channel) {
    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        if (srv->streams[i].in_use && srv->streams[i].client == c && srv->streams[i].channel == channel) {
            return &srv->streams[i];
        }
    }
    return NULL;
}

static pa_stream_t *find_stream_by_index(lwasapi_pa_server_t *srv, uint32_t index) {
    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        if (srv->streams[i].in_use && srv->streams[i].index == index) {
            return &srv->streams[i];
        }
    }
    return NULL;
}

static void free_stream(lwasapi_pa_server_t *srv, pa_stream_t *s) {
    if (!s || !s->in_use) return;
    bool is_pb = s->is_playback;
    uint32_t idx = s->index;
    atomic_store(&s->active, 0);
    s->in_use = false;
    broadcast_sub_event(srv, is_pb ? SUB_EVENT_SINK_INPUT : SUB_EVENT_SOURCE_OUTPUT, SUB_EVENT_REMOVE, idx);
}

static void send_stream_request(pa_stream_t *s, uint32_t req_bytes) {
    if (!s || !s->client || req_bytes == 0) return;
    tagbuf_t tb;
    tb_init(&tb, 48);
    tb_put_u32(&tb, TAG_U32, CMD_REQUEST);
    tb_put_u32(&tb, TAG_U32, PA_INVALID_INDEX);
    tb_put_u32(&tb, TAG_U32, s->channel);
    tb_put_u32(&tb, TAG_U32, req_bytes);
    queue_tb(s->client, &tb);
    s->requested_bytes_total += req_bytes;
}

static void send_stream_started(pa_stream_t *s) {
    if (!s || !s->client || s->started_sent) return;
    s->started_sent = true;
    tagbuf_t tb;
    tb_init(&tb, 32);
    tb_put_u32(&tb, TAG_U32, CMD_STARTED);
    tb_put_u32(&tb, TAG_U32, PA_INVALID_INDEX);
    tb_put_u32(&tb, TAG_U32, s->channel);
    queue_tb(s->client, &tb);
}

static void close_pa_client(lwasapi_pa_server_t *srv, pa_client_t *c);

static void handle_control_packet(lwasapi_pa_server_t *srv, pa_client_t *c, const uint8_t *buf, size_t len) {
    tagread_t tr;
    tr_init(&tr, buf, len);
    uint32_t cmd = 0, tag = 0;
    if (!tr_u32(&tr, &cmd) || !tr_u32(&tr, &tag)) return;

    if (!pa_is_lifecycle_cmd(cmd)) {
        lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_TRACE, c->pid, c->index,
                                (int32_t)cmd, (int32_t)tag, (int32_t)len,
                                pa_cmd_name(cmd), c->name[0] ? c->name : "pa-client",
                                "len=%zu", len);
    }

    if (cmd == CMD_AUTH) {
        uint32_t cver = 0;
        const void *ck = NULL;
        size_t cklen = 0;
        if (!tr_u32(&tr, &cver) || !tr_arbitrary(&tr, &ck, &cklen)) {
            reply_err(c, tag, ERR_PROTOCOL);
            return;
        }
        bool uid_ok = c->uid_valid && (c->uid == (uint32_t)getuid() || c->uid == 0U);
        bool cookie_ok = false;
        if (srv->cookie_valid && ck && cklen == PA_NATIVE_COOKIE_LEN) {
            const uint8_t *cb = (const uint8_t *)ck;
            uint8_t diff = 0;
            for (size_t i = 0; i < PA_NATIVE_COOKIE_LEN; i++) {
                diff |= (uint8_t)(cb[i] ^ srv->cookie[i]);
            }
            cookie_ok = (diff == 0);
        }
        if (!uid_ok && !cookie_ok) {
            reply_err(c, tag, ERR_ACCESS);
            return;
        }
        c->version = cver & 0xFFFFU;
        c->authed  = true;
        lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_INFO, c->pid, c->index,
                                (int32_t)c->version, (int32_t)tag, 0,
                                "PA_AUTH", "pulse/native",
                                "client#%u authed proto_ver=%u", c->index, c->version);
        tagbuf_t tb;
        tb_begin_reply(&tb, tag, 32);
        /* Do NOT set 0x80000000 SHM flag so clients stream clean inline memblocks */
        tb_put_u32(&tb, TAG_U32, PA_PROTO_VERSION);
        queue_tb(c, &tb);
        return;
    }

    if (!c->authed) {
        reply_err(c, tag, ERR_ACCESS);
        return;
    }

    switch (cmd) {
        case CMD_EXIT:
            close_pa_client(srv, c);
            return;

        case CMD_SET_CLIENT_NAME: {
            if (c->version < 13) {
                const char *nm = NULL;
                if (tr_str(&tr, &nm) && nm) snprintf(c->name, sizeof(c->name), "%s", nm);
            } else {
                tr_proplist(&tr, &c->props);
                const char *an = propbag_get(&c->props, "application.name");
                if (an) snprintf(c->name, sizeof(c->name), "%s", an);
                const char *pid_s = propbag_get(&c->props, "application.process.id");
                if (pid_s) c->pid = (uint32_t)atoi(pid_s);
            }
            const char *bin = propbag_get(&c->props, "application.process.binary");
            lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_INFO, c->pid, c->index,
                                    (int32_t)c->version, 0, 0,
                                    "PA_CLIENT", c->name[0] ? c->name : "pa-client",
                                    "pid=%u bin=%s", c->pid, bin ? bin : "unknown");
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 32);
            if (c->version >= 13) tb_put_u32(&tb, TAG_U32, c->index);
            queue_tb(c, &tb);
            broadcast_sub_event(srv, SUB_EVENT_CLIENT, SUB_EVENT_NEW, c->index);
            break;
        }

        case CMD_GET_SERVER_INFO: {
            refresh_device_cache(srv);
            struct passwd *pw = getpwuid(getuid());
            const char *uname = pw && pw->pw_name ? pw->pw_name : "user";
            char hname[128] = "salsa-host";
            gethostname(hname, sizeof(hname) - 1);
            char def_sink[128], def_src[128];
            get_default_sink_pa_name(srv, def_sink, sizeof(def_sink));
            get_default_source_pa_name(srv, def_src, sizeof(def_src));
            pa_wire_ss_t ss = { FMT_FLOAT32LE, 2, srv->cached_status.hw_sample_rate ? srv->cached_status.hw_sample_rate : 48000 };
            pa_wire_map_t mp = std_channel_map(2);

            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 512);
            tb_put_str(&tb, "PulseAudio (on Salsa LWASAPI 1.0)");
            tb_put_str(&tb, "15.0.0");
            tb_put_str(&tb, uname);
            tb_put_str(&tb, hname);
            tb_put_ss(&tb, &ss);
            tb_put_str(&tb, def_sink);
            tb_put_str(&tb, def_src);
            tb_put_u32(&tb, TAG_U32, PA_SERVER_COOKIE_ID);
            if (c->version >= 15) tb_put_map(&tb, &mp);
            queue_tb(c, &tb);
            break;
        }

        case CMD_STAT: {
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 64);
            tb_put_u32(&tb, TAG_U32, 0);
            tb_put_u32(&tb, TAG_U32, 0);
            tb_put_u32(&tb, TAG_U32, 0);
            tb_put_u32(&tb, TAG_U32, 0);
            tb_put_u32(&tb, TAG_U32, 0);
            queue_tb(c, &tb);
            break;
        }

        case CMD_LOOKUP_SINK:
        case CMD_LOOKUP_SOURCE: {
            const char *nm = NULL;
            if (!tr_str(&tr, &nm)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            refresh_device_cache(srv);
            bool is_sink = (cmd == CMD_LOOKUP_SINK);
            uint32_t found = PA_INVALID_INDEX;
            char resolved[160];
            bool ok = is_sink
                    ? resolve_sink_idx_and_name(srv, PA_INVALID_INDEX, nm, &found, resolved, sizeof(resolved))
                    : resolve_source_idx_and_name(srv, PA_INVALID_INDEX, nm, &found, resolved, sizeof(resolved), NULL);
            if (!ok || found == PA_INVALID_INDEX) {
                reply_err(c, tag, ERR_NOENTITY);
                break;
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 32);
            tb_put_u32(&tb, TAG_U32, found);
            queue_tb(c, &tb);
            break;
        }

        case CMD_GET_SINK_INFO:
        case CMD_GET_SINK_INFO_LIST: {
            refresh_device_cache(srv);
            uint32_t req_idx = PA_INVALID_INDEX;
            const char *req_nm = NULL;
            char resolved_nm[160] = {0};
            if (cmd == CMD_GET_SINK_INFO) {
                if (!tr_u32(&tr, &req_idx) || !tr_str(&tr, &req_nm)) { reply_err(c, tag, ERR_PROTOCOL); break; }
                if (req_nm && req_nm[0]) {
                    resolve_pa_target_name(srv, false, req_nm, resolved_nm, sizeof(resolved_nm));
                }
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 4096);
            uint32_t pb_cnt = 0, added = 0;
            for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
                const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
                if (d->is_capture) continue;
                pb_cnt++;
                uint32_t sidx = stable_pa_idx(srv, PA_FAC_SINK, d->id);
                char mk[64], pa_n[128];
                make_mon_key(d->id, mk, sizeof(mk));
                uint32_t mon_idx = stable_pa_idx(srv, PA_FAC_SOURCE, mk);
                make_pa_sink_name(d->id, pa_n, sizeof(pa_n));
                bool match = (cmd == CMD_GET_SINK_INFO_LIST) ||
                             (req_idx != PA_INVALID_INDEX && req_idx == sidx) ||
                             (resolved_nm[0] && strcmp(resolved_nm, pa_n) == 0);
                if (match) {
                    uint32_t card_idx = find_card_idx_for_id(srv, &srv->cached_devices, d->id);
                    put_sink_entry(&tb, c->version, sidx, mon_idx, card_idx,
                                   pa_n, d->name, d->id,
                                   d->is_bluetooth != 0,
                                   srv->cached_status.hw_sample_rate,
                                   srv->cached_status.desktop_volume_pct,
                                   srv->sink_pa_vol_valid ? srv->sink_pa_vol : PA_INVALID_INDEX,
                                   srv->cached_status.desktop_muted != 0);
                    added++;
                }
            }
            if (pb_cnt == 0) {
                uint32_t vsidx = stable_pa_idx(srv, PA_FAC_SINK, "virtual");
                uint32_t vmon  = stable_pa_idx(srv, PA_FAC_SOURCE, "mon:virtual");
                if (cmd == CMD_GET_SINK_INFO_LIST || req_idx == vsidx ||
                    (resolved_nm[0] && strcmp(resolved_nm, "salsa_out.virtual") == 0)) {
                    put_sink_entry(&tb, c->version, vsidx, vmon, PA_INVALID_INDEX,
                                   "salsa_out.virtual", "Salsa Master Virtual Sink",
                                   "virtual", false, srv->cached_status.hw_sample_rate,
                                   srv->cached_status.desktop_volume_pct,
                                   srv->sink_pa_vol_valid ? srv->sink_pa_vol : PA_INVALID_INDEX,
                                   srv->cached_status.desktop_muted != 0);
                    added++;
                }
            }
            if (cmd == CMD_GET_SINK_INFO && added == 0) {
                tb_free(&tb);
                reply_err(c, tag, ERR_NOENTITY);
            } else {
                queue_tb(c, &tb);
            }
            break;
        }

        case CMD_GET_SOURCE_INFO:
        case CMD_GET_SOURCE_INFO_LIST: {
            refresh_device_cache(srv);
            uint32_t req_idx = PA_INVALID_INDEX;
            const char *req_nm = NULL;
            char resolved_nm[160] = {0};
            if (cmd == CMD_GET_SOURCE_INFO) {
                if (!tr_u32(&tr, &req_idx) || !tr_str(&tr, &req_nm)) { reply_err(c, tag, ERR_PROTOCOL); break; }
                if (req_nm && req_nm[0]) {
                    resolve_pa_target_name(srv, true, req_nm, resolved_nm, sizeof(resolved_nm));
                }
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 4096);
            uint32_t added = 0;

            /* 1. Hardware capture sources */
            for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
                const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
                if (!d->is_capture) continue;
                uint32_t src_idx = stable_pa_idx(srv, PA_FAC_SOURCE, d->id);
                char pa_n[128];
                make_pa_source_name(d->id, pa_n, sizeof(pa_n));
                bool match = (cmd == CMD_GET_SOURCE_INFO_LIST) ||
                             (req_idx != PA_INVALID_INDEX && req_idx == src_idx) ||
                             (resolved_nm[0] && strcmp(resolved_nm, pa_n) == 0);
                if (match) {
                    uint32_t card_idx = find_card_idx_for_id(srv, &srv->cached_devices, d->id);
                    uint32_t src_rate = srv->cached_status.hw_analog_info.reserved;
                    if (src_rate < 8000U || src_rate > 384000U) {
                        src_rate = srv->cached_status.hw_sample_rate;
                    }
                    put_source_entry(&tb, c->version, src_idx, card_idx, pa_n, d->name, d->id,
                                     PA_INVALID_INDEX, NULL,
                                     src_rate,
                                     srv->cached_status.desktop_mic_volume_pct,
                                     srv->hw_src_pa_vol_valid ? srv->hw_src_pa_vol : PA_INVALID_INDEX,
                                     srv->cached_status.desktop_mic_muted != 0);
                    added++;
                }
            }

            /* 2. Monitor sources matching each sink */
            uint32_t mon_pct = atomic_load_explicit(&srv->mon_vol_milli, memory_order_relaxed) / 10U;
            bool mon_muted = (atomic_load_explicit(&srv->mon_muted, memory_order_relaxed) != 0);
            uint32_t pb_cnt = 0;
            for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
                const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
                if (d->is_capture) continue;
                pb_cnt++;
                char mk[64], sink_n[128], mon_n[160], mon_desc[96];
                make_mon_key(d->id, mk, sizeof(mk));
                uint32_t src_idx = stable_pa_idx(srv, PA_FAC_SOURCE, mk);
                uint32_t sidx    = stable_pa_idx(srv, PA_FAC_SINK, d->id);
                make_pa_sink_name(d->id, sink_n, sizeof(sink_n));
                snprintf(mon_n, sizeof(mon_n), "%.118s.monitor", sink_n);
                snprintf(mon_desc, sizeof(mon_desc), "Monitor of %.64s", d->name);
                bool match = (cmd == CMD_GET_SOURCE_INFO_LIST) ||
                             (req_idx != PA_INVALID_INDEX && req_idx == src_idx) ||
                             (resolved_nm[0] && strcmp(resolved_nm, mon_n) == 0);
                if (match) {
                    put_source_entry(&tb, c->version, src_idx, PA_INVALID_INDEX, mon_n, mon_desc, d->id,
                                     sidx, sink_n,
                                     srv->cached_status.hw_sample_rate, mon_pct, srv->mon_pa_vol, mon_muted);
                    added++;
                }
            }
            if (pb_cnt == 0) {
                uint32_t vsidx = stable_pa_idx(srv, PA_FAC_SINK, "virtual");
                uint32_t vmon  = stable_pa_idx(srv, PA_FAC_SOURCE, "mon:virtual");
                bool match = (cmd == CMD_GET_SOURCE_INFO_LIST) ||
                             (req_idx != PA_INVALID_INDEX && req_idx == vmon) ||
                             (resolved_nm[0] && strcmp(resolved_nm, "salsa_out.virtual.monitor") == 0);
                if (match) {
                    put_source_entry(&tb, c->version, vmon, PA_INVALID_INDEX, "salsa_out.virtual.monitor",
                                     "Monitor of Salsa Master Virtual Sink", "virtual",
                                     vsidx, "salsa_out.virtual",
                                     srv->cached_status.hw_sample_rate, mon_pct, srv->mon_pa_vol, mon_muted);
                    added++;
                }
            }
            if (cmd == CMD_GET_SOURCE_INFO && added == 0) {
                tb_free(&tb);
                reply_err(c, tag, ERR_NOENTITY);
            } else {
                queue_tb(c, &tb);
            }
            break;
        }

        case CMD_GET_CARD_INFO:
        case CMD_GET_CARD_INFO_LIST: {
            refresh_device_cache(srv);
            uint32_t req_idx = PA_INVALID_INDEX;
            const char *req_nm = NULL;
            if (cmd == CMD_GET_CARD_INFO) {
                if (!tr_u32(&tr, &req_idx) || !tr_str(&tr, &req_nm)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 4096);
            char seen_ids[LWASAPI_MAX_DEVICES][64];
            uint32_t n_seen = 0, added = 0;
            for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
                const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
                bool dup = false;
                for (uint32_t j = 0; j < n_seen; j++) {
                    if (strcmp(seen_ids[j], d->id) == 0) { dup = true; break; }
                }
                if (dup) continue;
                snprintf(seen_ids[n_seen], sizeof(seen_ids[n_seen]), "%s", d->id);
                n_seen++;

                uint32_t card_idx = stable_pa_idx(srv, PA_FAC_CARD, d->id);
                bool match = (cmd == CMD_GET_CARD_INFO_LIST) ||
                             (req_idx != PA_INVALID_INDEX && req_idx == card_idx) ||
                             (req_nm && strstr(req_nm, d->id + (d->is_bluetooth ? 6 : 0)) != NULL);
                if (match) {
                    put_card_entry(&tb, c->version, card_idx, &srv->cached_devices, d);
                    added++;
                }
            }
            if (cmd == CMD_GET_CARD_INFO && added == 0) {
                tb_free(&tb);
                reply_err(c, tag, ERR_NOENTITY);
            } else {
                queue_tb(c, &tb);
            }
            break;
        }

        case CMD_GET_MODULE_INFO:
        case CMD_GET_MODULE_INFO_LIST: {
            uint32_t req_idx = PA_INVALID_INDEX;
            if (cmd == CMD_GET_MODULE_INFO && (!tr_u32(&tr, &req_idx) || req_idx != 0)) {
                reply_err(c, tag, ERR_NOENTITY);
                break;
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 256);
            tb_put_u32(&tb, TAG_U32, 0);
            tb_put_str(&tb, "module-salsa-lwasapi");
            tb_put_str(&tb, "backend=kernel_alsa+native_bluez");
            tb_put_u32(&tb, TAG_U32, PA_INVALID_INDEX);
            if (c->version < 15) tb_put_bool(&tb, false);
            else {
                pa_propbag_t pb;
                memset(&pb, 0, sizeof(pb));
                propbag_set(&pb, "module.description", "Salsa LWASAPI Direct Audio & BlueZ Server");
                tb_put_proplist(&tb, &pb);
            }
            queue_tb(c, &tb);
            break;
        }

        case CMD_GET_CLIENT_INFO:
        case CMD_GET_CLIENT_INFO_LIST: {
            uint32_t req_idx = PA_INVALID_INDEX;
            if (cmd == CMD_GET_CLIENT_INFO && !tr_u32(&tr, &req_idx)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 1024);
            uint32_t added = 0;
            for (int i = 0; i < PA_MAX_CLIENTS; i++) {
                pa_client_t *cl = &srv->clients[i];
                if (!cl->in_use || !cl->authed) continue;
                if (cmd == CMD_GET_CLIENT_INFO && cl->index != req_idx) continue;
                tb_put_u32(&tb, TAG_U32, cl->index);
                tb_put_str(&tb, cl->name[0] ? cl->name : "PulseAudio Client");
                tb_put_u32(&tb, TAG_U32, 0);
                tb_put_str(&tb, "Salsa-LWASAPI");
                if (c->version >= 13) tb_put_proplist(&tb, &cl->props);
                added++;
            }
            if (cmd == CMD_GET_CLIENT_INFO && added == 0) {
                tb_free(&tb);
                reply_err(c, tag, ERR_NOENTITY);
            } else {
                queue_tb(c, &tb);
            }
            break;
        }

        case CMD_GET_SINK_INPUT_INFO:
        case CMD_GET_SINK_INPUT_INFO_LIST: {
            uint32_t req_idx = PA_INVALID_INDEX;
            if (cmd == CMD_GET_SINK_INPUT_INFO && !tr_u32(&tr, &req_idx)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 2048);
            uint32_t added = 0;
            for (int i = 0; i < PA_MAX_STREAMS; i++) {
                pa_stream_t *s = &srv->streams[i];
                if (!s->in_use || !s->is_playback) continue;
                if (cmd == CMD_GET_SINK_INPUT_INFO && s->index != req_idx) continue;
                put_sink_input_entry(&tb, c->version, s, s->target_dev_idx);
                added++;
            }
            if (cmd == CMD_GET_SINK_INPUT_INFO && added == 0) {
                tb_free(&tb);
                reply_err(c, tag, ERR_NOENTITY);
            } else {
                queue_tb(c, &tb);
            }
            break;
        }

        case CMD_GET_SOURCE_OUTPUT_INFO:
        case CMD_GET_SOURCE_OUTPUT_INFO_LIST: {
            uint32_t req_idx = PA_INVALID_INDEX;
            if (cmd == CMD_GET_SOURCE_OUTPUT_INFO && !tr_u32(&tr, &req_idx)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 2048);
            uint32_t added = 0;
            for (int i = 0; i < PA_MAX_STREAMS; i++) {
                pa_stream_t *s = &srv->streams[i];
                if (!s->in_use || s->is_playback) continue;
                if (cmd == CMD_GET_SOURCE_OUTPUT_INFO && s->index != req_idx) continue;
                put_source_output_entry(&tb, c->version, s, s->target_dev_idx);
                added++;
            }
            if (cmd == CMD_GET_SOURCE_OUTPUT_INFO && added == 0) {
                tb_free(&tb);
                reply_err(c, tag, ERR_NOENTITY);
            } else {
                queue_tb(c, &tb);
            }
            break;
        }

        case CMD_GET_SAMPLE_INFO_LIST: {
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 32);
            queue_tb(c, &tb);
            break;
        }

        case CMD_SUBSCRIBE: {
            uint32_t mask = 0;
            if (!tr_u32(&tr, &mask)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            c->sub_mask = mask & SUB_MASK_ALL;
            reply_simple_ack(c, tag);
            break;
        }

        case CMD_SET_SINK_VOLUME:
        case CMD_SET_SOURCE_VOLUME: {
            uint32_t idx = PA_INVALID_INDEX;
            const char *nm = NULL;
            pa_wire_cvol_t cv;
            if (!tr_u32(&tr, &idx) || !tr_str(&tr, &nm) || !tr_cvol(&tr, &cv)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            uint64_t sum = 0;
            for (uint8_t i = 0; i < cv.channels; i++) sum += cv.values[i];
            uint32_t avg_pv = cv.channels ? (uint32_t)(sum / cv.channels) : PA_VOLUME_NORM;
            if (avg_pv > PA_VOLUME_NORM * 2U) avg_pv = PA_VOLUME_NORM * 2U;
            double norm = (double)avg_pv / (double)PA_VOLUME_NORM;
            double lin  = norm * norm * norm;
            uint32_t gain_ppm = (uint32_t)(lin * 1000000.0 + 0.5);
            if (avg_pv > 0 && gain_ppm == 0) gain_ppm = 1;
            if (gain_ppm > 2000000U) gain_ppm = 2000000U;
            int pct = (int)(lin * 100.0 + 0.5);
            if (avg_pv > 0 && pct == 0) pct = 1;
            if (pct > 150) pct = 150;

            if (cmd == CMD_SET_SOURCE_VOLUME) {
                uint32_t tgt_idx = PA_INVALID_INDEX;
                char tgt_nm[160] = {0};
                bool is_mon = false;
                if (!resolve_source_idx_and_name(srv, idx, nm, &tgt_idx, tgt_nm, sizeof(tgt_nm), &is_mon)) {
                    if (idx != PA_INVALID_INDEX || (nm && nm[0])) {
                        reply_err(c, tag, ERR_NOENTITY);
                        break;
                    }
                    tgt_idx = find_active_source_idx(srv);
                    is_mon = (find_monitor_src_idx_for_sink(srv, find_active_sink_idx(srv)) == tgt_idx);
                }
                if (is_mon) {
                    uint32_t milli = (uint32_t)(lin * 1000.0 + 0.5);
                    if (avg_pv > 0 && milli == 0) milli = 1;
                    if (milli > 2000U) milli = 2000U;
                    srv->mon_pa_vol = avg_pv;
                    atomic_store_explicit(&srv->mon_vol_milli, milli, memory_order_relaxed);
                } else {
                    srv->hw_src_pa_vol = avg_pv;
                    srv->hw_src_pa_vol_valid = true;
                    if (srv->cbs.set_mic_gain_ppm) {
                        srv->cbs.set_mic_gain_ppm(srv->cbs.user_ctx, gain_ppm, pct, -1);
                    } else if (srv->cbs.set_master_volume) {
                        srv->cbs.set_master_volume(srv->cbs.user_ctx, 1, pct, -1);
                    }
                    refresh_status_only(srv);
                }
                reply_simple_ack(c, tag);
                broadcast_sub_event(srv, SUB_EVENT_SOURCE, SUB_EVENT_CHANGE, tgt_idx);
            } else {
                uint32_t tgt_idx = PA_INVALID_INDEX;
                char tgt_nm[160] = {0};
                if (!resolve_sink_idx_and_name(srv, idx, nm, &tgt_idx, tgt_nm, sizeof(tgt_nm))) {
                    tgt_idx = find_active_sink_idx(srv);
                }
                srv->sink_pa_vol = avg_pv;
                srv->sink_pa_vol_valid = true;
                if (srv->cbs.set_master_volume) srv->cbs.set_master_volume(srv->cbs.user_ctx, 0, pct, -1);
                refresh_status_only(srv);
                broadcast_sub_event(srv, SUB_EVENT_SINK, SUB_EVENT_CHANGE, tgt_idx);
                reply_simple_ack(c, tag);
            }
            break;
        }

        case CMD_SET_SINK_MUTE:
        case CMD_SET_SOURCE_MUTE: {
            uint32_t idx = PA_INVALID_INDEX;
            const char *nm = NULL;
            bool mute = false;
            if (!tr_u32(&tr, &idx) || !tr_str(&tr, &nm) || !tr_bool(&tr, &mute)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            if (cmd == CMD_SET_SOURCE_MUTE) {
                uint32_t tgt_idx = PA_INVALID_INDEX;
                char tgt_nm[160] = {0};
                bool is_mon = false;
                if (!resolve_source_idx_and_name(srv, idx, nm, &tgt_idx, tgt_nm, sizeof(tgt_nm), &is_mon)) {
                    if (idx != PA_INVALID_INDEX || (nm && nm[0])) {
                        reply_err(c, tag, ERR_NOENTITY);
                        break;
                    }
                    tgt_idx = find_active_source_idx(srv);
                    is_mon = (find_monitor_src_idx_for_sink(srv, find_active_sink_idx(srv)) == tgt_idx);
                }
                if (is_mon) {
                    atomic_store_explicit(&srv->mon_muted, mute ? 1 : 0, memory_order_relaxed);
                } else {
                    if (srv->cbs.set_master_volume) {
                        srv->cbs.set_master_volume(srv->cbs.user_ctx, 1, -1, mute ? 1 : 0);
                    }
                    refresh_status_only(srv);
                }
                reply_simple_ack(c, tag);
                broadcast_sub_event(srv, SUB_EVENT_SOURCE, SUB_EVENT_CHANGE, tgt_idx);
            } else {
                uint32_t tgt_idx = PA_INVALID_INDEX;
                char tgt_nm[160] = {0};
                if (!resolve_sink_idx_and_name(srv, idx, nm, &tgt_idx, tgt_nm, sizeof(tgt_nm))) {
                    tgt_idx = find_active_sink_idx(srv);
                }
                if (srv->cbs.set_master_volume) {
                    srv->cbs.set_master_volume(srv->cbs.user_ctx, 0, -1, mute ? 1 : 0);
                }
                refresh_status_only(srv);
                broadcast_sub_event(srv, SUB_EVENT_SINK, SUB_EVENT_CHANGE, tgt_idx);
                reply_simple_ack(c, tag);
            }
            break;
        }

        case CMD_SET_SINK_INPUT_VOLUME:
        case CMD_SET_SOURCE_OUTPUT_VOLUME: {
            uint32_t idx = 0;
            pa_wire_cvol_t cv;
            if (!tr_u32(&tr, &idx) || !tr_cvol(&tr, &cv)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_stream_by_index(srv, idx);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            s->vol = cv;
            atomic_store(&s->volume_milli, cvol_to_milli(&cv));
            record_app_volume_locked(srv, s);
            write_app_volumes_from_srv_locked(srv);
            reply_simple_ack(c, tag);
            broadcast_sub_event(srv, s->is_playback ? SUB_EVENT_SINK_INPUT : SUB_EVENT_SOURCE_OUTPUT, SUB_EVENT_CHANGE, idx);
            break;
        }

        case CMD_SET_SINK_INPUT_MUTE:
        case CMD_SET_SOURCE_OUTPUT_MUTE: {
            uint32_t idx = 0;
            bool mute = false;
            if (!tr_u32(&tr, &idx) || !tr_bool(&tr, &mute)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_stream_by_index(srv, idx);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            atomic_store(&s->muted, mute ? 1 : 0);
            record_app_volume_locked(srv, s);
            write_app_volumes_from_srv_locked(srv);
            reply_simple_ack(c, tag);
            broadcast_sub_event(srv, s->is_playback ? SUB_EVENT_SINK_INPUT : SUB_EVENT_SOURCE_OUTPUT, SUB_EVENT_CHANGE, idx);
            break;
        }

        case CMD_SET_DEFAULT_SINK:
        case CMD_SET_DEFAULT_SOURCE: {
            const char *nm = NULL;
            if (!tr_str(&tr, &nm)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            refresh_status_only(srv);
            bool is_cap = (cmd == CMD_SET_DEFAULT_SOURCE);
            char dev_id[64] = {0};
            if (nm && strcmp(nm, "@NONE@") != 0) {
                if (!pa_name_to_salsa_id(&srv->cached_devices, nm, is_cap, dev_id, sizeof(dev_id))) {
                    refresh_device_cache_force(srv);
                }
                if (!pa_name_to_salsa_id(&srv->cached_devices, nm, is_cap, dev_id, sizeof(dev_id))) {
                    /* Support choosing monitor safely */
                    if (is_cap && strstr(nm, ".monitor")) {
                        reply_simple_ack(c, tag);
                        break;
                    }
                    reply_err(c, tag, ERR_NOENTITY);
                    break;
                }
                if (srv->cbs.set_devices) {
                    srv->cbs.set_devices(srv->cbs.user_ctx, is_cap ? NULL : dev_id, is_cap ? dev_id : NULL);
                }
                refresh_status_only(srv);
            }
            reply_simple_ack(c, tag);
            broadcast_sub_event(srv, SUB_EVENT_SERVER, SUB_EVENT_CHANGE, PA_INVALID_INDEX);
            broadcast_sub_event(srv, is_cap ? SUB_EVENT_SOURCE : SUB_EVENT_SINK, SUB_EVENT_CHANGE,
                                is_cap ? find_active_source_idx(srv) : find_active_sink_idx(srv));
            break;
        }

        case CMD_SET_CARD_PROFILE:
        case CMD_SET_SINK_PORT:
        case CMD_SET_SOURCE_PORT:
        case CMD_SUSPEND_SINK:
        case CMD_SUSPEND_SOURCE:
        case CMD_SET_PORT_LATENCY_OFFSET:
            reply_simple_ack(c, tag);
            break;

        case CMD_CREATE_PLAYBACK_STREAM: {
            const char *sname = NULL, *sink_nm = NULL;
            pa_wire_ss_t ss;
            pa_wire_map_t mp;
            uint32_t sink_idx = PA_INVALID_INDEX, maxl = 0, tl = 0, preb = 0, minr = 0, syncid = 0;
            bool corked = false, b_unused = false, muted = false, volume_set = true, adjust_latency = false;
            pa_wire_cvol_t cv;
            memset(&cv, 0, sizeof(cv));

            if (c->version < 13) tr_str(&tr, &sname);
            if (!tr_ss(&tr, &ss) || !tr_map(&tr, &mp) || !tr_u32(&tr, &sink_idx) ||
                !tr_str(&tr, &sink_nm) || !tr_u32(&tr, &maxl) || !tr_bool(&tr, &corked) ||
                !tr_u32(&tr, &tl) || !tr_u32(&tr, &preb) || !tr_u32(&tr, &minr) ||
                !tr_u32(&tr, &syncid) || !tr_cvol(&tr, &cv)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            pa_propbag_t pb;
            memset(&pb, 0, sizeof(pb));
            if (c->version >= 12) {
                for (int k = 0; k < 7; k++) tr_bool(&tr, &b_unused);
            }
            if (c->version >= 13) {
                tr_bool(&tr, &muted);
                tr_bool(&tr, &adjust_latency);
                tr_proplist(&tr, &pb);
            }
            if (c->version >= 14) {
                tr_bool(&tr, &volume_set);
                tr_bool(&tr, &b_unused);
            }
            if (c->version >= 15) {
                bool muted_set = false;
                tr_bool(&tr, &muted_set);
                if (!muted_set) muted = false;
                tr_bool(&tr, &b_unused);
                tr_bool(&tr, &b_unused);
            }
            if (c->version >= 17) tr_bool(&tr, &b_unused);
            if (c->version >= 18) tr_bool(&tr, &b_unused);
            if (c->version >= 21) {
                uint8_t nf = 0;
                if (tr_u8(&tr, &nf)) {
                    for (uint8_t f = 0; f < nf; f++) tr_skip_format_info(&tr);
                }
            }

            refresh_device_cache(srv);
            uint32_t actual_sink_idx = 0;
            char actual_sink_nm[128] = {0};
            if (!resolve_sink_idx_and_name(srv, sink_idx, sink_nm, &actual_sink_idx, actual_sink_nm, sizeof(actual_sink_nm))) {
                actual_sink_idx = find_active_sink_idx(srv);
                get_default_sink_pa_name(srv, actual_sink_nm, sizeof(actual_sink_nm));
            }

            pa_stream_t *s = alloc_stream(srv, c);
            if (!s) { reply_err(c, tag, ERR_INVALID); break; }
            s->is_playback = true;
            s->target_dev_idx = actual_sink_idx;
            if (ss.rate == 0 || ss.rate > 768000) ss.rate = 48000;
            if (ss.channels == 0 || ss.channels > 32) ss.channels = 2;
            s->ss = ss;
            s->map = mp.channels == ss.channels ? mp : std_channel_map(ss.channels);
            s->props = pb;
            const char *mname = propbag_get(&s->props, "media.name");
            if (mname) snprintf(s->name, sizeof(s->name), "%s", mname);
            else if (sname) snprintf(s->name, sizeof(s->name), "%s", sname);
            else snprintf(s->name, sizeof(s->name), "%s", c->name);

            s->frame_size = pa_format_sample_size(ss.format) * ss.channels;
            if (s->frame_size == 0) s->frame_size = 4;
            uint32_t eff_tl = tl;
            if (adjust_latency && eff_tl != PA_INVALID_INDEX && eff_tl > 0) {
                uint64_t sink_bytes = ((5000ULL * (uint64_t)ss.rate) / 1000000ULL) * s->frame_size;
                uint64_t floor_bytes = ((uint64_t)ss.rate / 50U) * s->frame_size;
                if ((uint64_t)eff_tl > sink_bytes + floor_bytes) {
                    eff_tl -= (uint32_t)sink_bytes;
                }
            }
            enforce_playback_attr(s, maxl, eff_tl, minr, preb);
            atomic_store_explicit(&s->in_prebuf, (s->prebuf > 0) ? 1 : 0, memory_order_release);
            atomic_store_explicit(&s->underrun_baseline_bytes, 0ULL, memory_order_relaxed);

            if (volume_set && cv.channels > 0) {
                s->vol = cv;
                atomic_store(&s->volume_milli, cvol_to_milli(&cv));
            } else {
                s->vol.channels = ss.channels;
                for (uint8_t k = 0; k < ss.channels; k++) s->vol.values[k] = PA_VOLUME_NORM;
                atomic_store(&s->volume_milli, 1000);
            }
            atomic_store(&s->muted, muted ? 1 : 0);
            atomic_store(&s->corked, corked ? 1 : 0);
            stream_update_device_id_locked(srv, s);
            apply_saved_app_volume_locked(srv, s, volume_set);
            atomic_store(&s->sample_rate_atomic, ss.rate);
            atomic_store(&s->active, 1);

            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 256);
            tb_put_u32(&tb, TAG_U32, s->channel);
            tb_put_u32(&tb, TAG_U32, s->index);
            tb_put_u32(&tb, TAG_U32, s->tlength); /* Initial requested missing bytes */
            s->requested_bytes_total = s->tlength;

            if (c->version >= 9) {
                tb_put_u32(&tb, TAG_U32, s->maxlength);
                tb_put_u32(&tb, TAG_U32, s->tlength);
                tb_put_u32(&tb, TAG_U32, s->prebuf);
                tb_put_u32(&tb, TAG_U32, s->minreq);
            }
            if (c->version >= 12) {
                tb_put_ss(&tb, &s->ss);
                tb_put_map(&tb, &s->map);
                tb_put_u32(&tb, TAG_U32, actual_sink_idx);
                tb_put_str(&tb, actual_sink_nm);
                tb_put_bool(&tb, false);
            }
            if (c->version >= 13) tb_put_u64_raw(&tb, TAG_USEC, 5000ULL);
            if (c->version >= 21) tb_put_pcm_format(&tb);
            queue_tb(c, &tb);

            lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_INFO, c->pid, 10000U + s->index,
                                    (int32_t)s->ss.rate, (int32_t)s->tlength, (int32_t)s->corked,
                                    "PA_NEW_PLAY", s->name[0] ? s->name : c->name,
                                    "sink=%s ch=%u fmt=%u prebuf=%u minreq=%u",
                                    actual_sink_nm, s->ss.channels, s->ss.format,
                                    s->prebuf, s->minreq);

            broadcast_sub_event(srv, SUB_EVENT_SINK_INPUT, SUB_EVENT_NEW, s->index);
            break;
        }

        case CMD_CREATE_RECORD_STREAM: {
            const char *sname = NULL, *src_nm = NULL;
            pa_wire_ss_t ss;
            pa_wire_map_t mp;
            uint32_t src_idx = PA_INVALID_INDEX, maxl = 0, frags = 0;
            uint32_t doi = PA_INVALID_INDEX;
            bool corked = false, b_unused = false, peak_detect = false;
            bool muted = false, volume_set = false, muted_set = false;
            pa_wire_cvol_t cv;
            memset(&cv, 0, sizeof(cv));

            if (c->version < 13) tr_str(&tr, &sname);
            if (!tr_ss(&tr, &ss) || !tr_map(&tr, &mp) || !tr_u32(&tr, &src_idx) ||
                !tr_str(&tr, &src_nm) || !tr_u32(&tr, &maxl) || !tr_bool(&tr, &corked) ||
                !tr_u32(&tr, &frags)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            pa_propbag_t pb;
            memset(&pb, 0, sizeof(pb));
            if (c->version >= 12) {
                for (int k = 0; k < 7; k++) tr_bool(&tr, &b_unused);
            }
            if (c->version >= 13) {
                tr_bool(&tr, &peak_detect);
                tr_bool(&tr, &b_unused);
                tr_proplist(&tr, &pb);
                if (!tr_u32(&tr, &doi)) doi = PA_INVALID_INDEX;
            }
            if (c->version >= 14) tr_bool(&tr, &b_unused);
            if (c->version >= 15) {
                tr_bool(&tr, &b_unused);
                tr_bool(&tr, &b_unused);
            }
            if (c->version >= 22) {
                uint8_t nf = 0;
                if (tr_u8(&tr, &nf)) {
                    for (uint8_t f = 0; f < nf; f++) tr_skip_format_info(&tr);
                }
                if (tr_cvol(&tr, &cv)) {
                    tr_bool(&tr, &muted);
                    tr_bool(&tr, &volume_set);
                    tr_bool(&tr, &muted_set);
                    tr_bool(&tr, &b_unused); /* relative_volume */
                    tr_bool(&tr, &b_unused); /* passthrough */
                    if (!muted_set) muted = false;
                }
            }

            refresh_device_cache(srv);
            uint32_t actual_src_idx = 0;
            char actual_src_nm[160] = {0};
            bool is_mon = false;

            if (doi != PA_INVALID_INDEX) {
                pa_stream_t *target_pb = find_stream_by_index(srv, doi);
                if (!target_pb || !target_pb->is_playback) {
                    reply_err(c, tag, ERR_NOENTITY);
                    break;
                }
                uint32_t mon_idx = find_monitor_src_idx_for_sink(srv, target_pb->target_dev_idx);
                if (mon_idx == PA_INVALID_INDEX) {
                    mon_idx = stable_pa_idx(srv, PA_FAC_SOURCE, "mon:virtual");
                }
                if (!resolve_source_idx_and_name(srv, mon_idx, NULL, &actual_src_idx, actual_src_nm, sizeof(actual_src_nm), &is_mon)) {
                    actual_src_idx = mon_idx;
                    char def_sink[128];
                    get_default_sink_pa_name(srv, def_sink, sizeof(def_sink));
                    snprintf(actual_src_nm, sizeof(actual_src_nm), "%.118s.monitor", def_sink);
                }
                is_mon = true;
            } else if (!resolve_source_idx_and_name(srv, src_idx, src_nm, &actual_src_idx, actual_src_nm, sizeof(actual_src_nm), &is_mon)) {
                if (src_idx == PA_INVALID_INDEX && (!src_nm || !src_nm[0])) {
                    actual_src_idx = find_active_source_idx(srv);
                    get_default_source_pa_name(srv, actual_src_nm, sizeof(actual_src_nm));
                    is_mon = (strstr(actual_src_nm, ".monitor") != NULL);
                } else {
                    reply_err(c, tag, ERR_NOENTITY);
                    break;
                }
            }

            bool explicit_req = (doi == PA_INVALID_INDEX) &&
                                ((src_idx != PA_INVALID_INDEX) ||
                                 (src_nm && src_nm[0] && strncmp(src_nm, "@DEFAULT_", 9) != 0));
            sync_active_capture_if_explicit(srv, actual_src_idx, is_mon, peak_detect, explicit_req);

            pa_stream_t *s = alloc_stream(srv, c);
            if (!s) { reply_err(c, tag, ERR_INVALID); break; }
            s->is_playback     = false;
            s->is_monitor      = is_mon;
            s->peak_detect     = peak_detect;
            s->direct_on_input = doi;
            s->target_dev_idx  = actual_src_idx;

            /* Preserve low rates (e.g. 25 Hz peak-detect meters!) so libpulse does not abort with PA_ERR_PROTOCOL */
            if (ss.rate == 0 || ss.rate > 768000) ss.rate = 48000;
            if (ss.channels == 0 || ss.channels > 32) ss.channels = 2;
            s->ss = ss;
            s->map = mp.channels == ss.channels ? mp : std_channel_map(ss.channels);
            s->props = pb;
            const char *mname = propbag_get(&s->props, "media.name");
            if (mname) snprintf(s->name, sizeof(s->name), "%s", mname);
            else if (sname) snprintf(s->name, sizeof(s->name), "%s", sname);
            else snprintf(s->name, sizeof(s->name), "%s", c->name);

            s->frame_size = pa_format_sample_size(ss.format) * ss.channels;
            if (s->frame_size == 0) s->frame_size = 4;
            s->maxlength = PA_STREAM_RING_FRAMES * s->frame_size;
            enforce_record_attr(s, frags);

            if (volume_set && cv.channels > 0) {
                s->vol = cv;
                atomic_store(&s->volume_milli, cvol_to_milli(&cv));
            } else {
                s->vol.channels = ss.channels;
                for (uint8_t k = 0; k < ss.channels; k++) s->vol.values[k] = PA_VOLUME_NORM;
                atomic_store(&s->volume_milli, 1000);
            }
            atomic_store(&s->muted, muted ? 1 : 0);
            atomic_store(&s->corked, corked ? 1 : 0);
            stream_update_device_id_locked(srv, s);
            apply_saved_app_volume_locked(srv, s, volume_set);
            atomic_store(&s->sample_rate_atomic, ss.rate);
            atomic_store(&s->active, 1);

            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 256);
            tb_put_u32(&tb, TAG_U32, s->channel);
            tb_put_u32(&tb, TAG_U32, s->index);
            if (c->version >= 9) {
                tb_put_u32(&tb, TAG_U32, s->maxlength);
                tb_put_u32(&tb, TAG_U32, s->fragsize);
            }
            if (c->version >= 12) {
                tb_put_ss(&tb, &s->ss);
                tb_put_map(&tb, &s->map);
                tb_put_u32(&tb, TAG_U32, actual_src_idx);
                tb_put_str(&tb, actual_src_nm);
                tb_put_bool(&tb, false);
            }
            if (c->version >= 13) tb_put_u64_raw(&tb, TAG_USEC, 5000ULL);
            if (c->version >= 22) tb_put_pcm_format(&tb);
            queue_tb(c, &tb);

            lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_INFO, c->pid, 10000U + s->index,
                                    (int32_t)s->ss.rate, (int32_t)s->fragsize, (int32_t)s->is_monitor,
                                    "PA_NEW_REC", s->name[0] ? s->name : c->name,
                                    "src=%s mon=%d peak=%d doi=%u ch=%u cork=%d",
                                    actual_src_nm, (int)s->is_monitor, (int)s->peak_detect,
                                    s->direct_on_input, s->ss.channels, (int)corked);

            broadcast_sub_event(srv, SUB_EVENT_SOURCE_OUTPUT, SUB_EVENT_NEW, s->index);
            break;
        }

        case CMD_DELETE_PLAYBACK_STREAM:
        case CMD_DELETE_RECORD_STREAM: {
            uint32_t ch = 0;
            if (!tr_u32(&tr, &ch)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_INFO, c->pid, 10000U + s->index,
                                    (int32_t)s->is_playback, (int32_t)ch, 0,
                                    "PA_DEL_STREAM", s->name[0] ? s->name : c->name,
                                    "%s dev=%s", s->is_playback ? "playback" : "record",
                                    s->device_id);
            free_stream(srv, s);
            reply_simple_ack(c, tag);
            break;
        }

        case CMD_CORK_PLAYBACK_STREAM:
        case CMD_CORK_RECORD_STREAM: {
            uint32_t ch = 0;
            bool cork = false;
            if (!tr_u32(&tr, &ch) || !tr_bool(&tr, &cork)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            atomic_store(&s->corked, cork ? 1 : 0);
            lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_INFO, c->pid, 10000U + s->index,
                                    (int32_t)(cork ? 1 : 0), (int32_t)s->is_playback, 0,
                                    "PA_CORK", s->name[0] ? s->name : c->name,
                                    "corked=%d (%s)", (int)cork,
                                    s->is_playback ? "playback" : "record");
            reply_simple_ack(c, tag);
            broadcast_sub_event(srv, s->is_playback ? SUB_EVENT_SINK_INPUT : SUB_EVENT_SOURCE_OUTPUT,
                                SUB_EVENT_CHANGE, s->index);
            break;
        }

        case CMD_FLUSH_PLAYBACK_STREAM:
        case CMD_FLUSH_RECORD_STREAM: {
            uint32_t ch = 0;
            if (!tr_u32(&tr, &ch)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            uint64_t wr = atomic_load(&s->wr_frame);
            uint64_t wi = atomic_load(&s->write_index);
            atomic_store(&s->rd_frame, wr);
            atomic_store(&s->read_index, wi);
            s->resamp_phase = 0.0;
            s->have_hist = false;
            s->have_cap_hist = false;
            if (s->is_playback) {
                atomic_store_explicit(&s->in_prebuf, (s->prebuf > 0) ? 1 : 0, memory_order_release);
                atomic_store_explicit(&s->underrun_baseline_bytes, wi, memory_order_relaxed);
            }
            reply_simple_ack(c, tag);
            if (s->is_playback) {
                s->requested_bytes_total = wi;
                send_stream_request(s, s->tlength);
            }
            break;
        }

        case CMD_TRIGGER_PLAYBACK_STREAM:
        case CMD_PREBUF_PLAYBACK_STREAM: {
            uint32_t ch = 0;
            if (!tr_u32(&tr, &ch)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            atomic_store_explicit(&s->in_prebuf, 0, memory_order_release);
            reply_simple_ack(c, tag);
            send_stream_started(s);
            break;
        }

        case CMD_DRAIN_PLAYBACK_STREAM: {
            uint32_t ch = 0;
            if (!tr_u32(&tr, &ch)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            atomic_store_explicit(&s->in_prebuf, 0, memory_order_release);
            uint64_t wr = atomic_load(&s->wr_frame);
            uint64_t rd = atomic_load(&s->rd_frame);
            if (rd >= wr) {
                reply_simple_ack(c, tag);
            } else {
                s->drain_tag = tag;
                atomic_store(&s->draining, 1);
            }
            break;
        }

        case CMD_GET_PLAYBACK_LATENCY:
        case CMD_GET_RECORD_LATENCY: {
            uint32_t ch = 0, ts_s = 0, ts_u = 0;
            if (!tr_u32(&tr, &ch) || !tr_timeval(&tr, &ts_s, &ts_u)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            struct timeval now;
            gettimeofday(&now, NULL);
            /* libpulse already computes buffer latency from (write_index - read_index);
             * sink_usec/source_usec must report ONLY downstream hardware/period latency. */
            uint64_t wi = atomic_load_explicit(&s->write_index, memory_order_acquire);
            uint64_t ri = atomic_load_explicit(&s->read_index, memory_order_acquire);
            if (ri > wi) ri = wi;
            uint64_t hw_pb_usec  = 5000ULL;
            uint64_t hw_rec_usec = 2000ULL;

            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 128);
            if (cmd == CMD_GET_PLAYBACK_LATENCY) {
                tb_put_u64_raw(&tb, TAG_USEC, hw_pb_usec);
                tb_put_u64_raw(&tb, TAG_USEC, 0);
                tb_put_bool(&tb, !atomic_load(&s->corked) && !atomic_load_explicit(&s->in_prebuf, memory_order_acquire));
                tb_put_timeval(&tb, ts_s, ts_u);
                tb_put_timeval(&tb, (uint32_t)now.tv_sec, (uint32_t)now.tv_usec);
                tb_put_u64_raw(&tb, TAG_S64, wi);
                tb_put_u64_raw(&tb, TAG_S64, ri);
                if (c->version >= 13) {
                    uint64_t base = atomic_load_explicit(&s->underrun_baseline_bytes, memory_order_relaxed);
                    uint64_t playing_bytes = (ri >= base) ? (ri - base) : 0ULL;
                    tb_put_u64_raw(&tb, TAG_U64, 0ULL);
                    tb_put_u64_raw(&tb, TAG_U64, playing_bytes);
                }
            } else {
                tb_put_u64_raw(&tb, TAG_USEC, 0);
                tb_put_u64_raw(&tb, TAG_USEC, hw_rec_usec);
                tb_put_bool(&tb, !atomic_load(&s->corked));
                tb_put_timeval(&tb, ts_s, ts_u);
                tb_put_timeval(&tb, (uint32_t)now.tv_sec, (uint32_t)now.tv_usec);
                tb_put_u64_raw(&tb, TAG_S64, wi);
                tb_put_u64_raw(&tb, TAG_S64, ri);
            }
            queue_tb(c, &tb);
            break;
        }

        case CMD_SET_PLAYBACK_STREAM_BUFFER_ATTR: {
            uint32_t ch = 0, maxl = 0, tl = 0, preb = 0, minr = 0;
            if (!tr_u32(&tr, &ch) || !tr_u32(&tr, &maxl) || !tr_u32(&tr, &tl) ||
                !tr_u32(&tr, &preb) || !tr_u32(&tr, &minr)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            enforce_playback_attr(s,
                                  (maxl == PA_INVALID_INDEX) ? s->maxlength : maxl,
                                  (tl   == PA_INVALID_INDEX) ? s->tlength   : tl,
                                  (minr == PA_INVALID_INDEX) ? s->minreq    : minr,
                                  (preb == PA_INVALID_INDEX) ? s->prebuf    : preb);
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 64);
            tb_put_u32(&tb, TAG_U32, s->maxlength);
            tb_put_u32(&tb, TAG_U32, s->tlength);
            tb_put_u32(&tb, TAG_U32, s->prebuf);
            tb_put_u32(&tb, TAG_U32, s->minreq);
            if (c->version >= 13) tb_put_u64_raw(&tb, TAG_USEC, 5000ULL);
            queue_tb(c, &tb);
            break;
        }

        case CMD_SET_RECORD_STREAM_BUFFER_ATTR: {
            uint32_t ch = 0, maxl = 0, frags = 0;
            if (!tr_u32(&tr, &ch) || !tr_u32(&tr, &maxl) || !tr_u32(&tr, &frags)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            enforce_record_attr(s, (frags == PA_INVALID_INDEX) ? s->fragsize : frags);
            tagbuf_t tb;
            tb_begin_reply(&tb, tag, 48);
            tb_put_u32(&tb, TAG_U32, s->maxlength);
            tb_put_u32(&tb, TAG_U32, s->fragsize);
            if (c->version >= 13) tb_put_u64_raw(&tb, TAG_USEC, 2000ULL);
            queue_tb(c, &tb);
            break;
        }

        case CMD_UPDATE_PLAYBACK_STREAM_SAMPLE_RATE:
        case CMD_UPDATE_RECORD_STREAM_SAMPLE_RATE: {
            uint32_t ch = 0, rate = 0;
            if (!tr_u32(&tr, &ch) || !tr_u32(&tr, &rate)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            if (rate >= 8000 && rate <= 384000) {
                s->ss.rate = rate;
                atomic_store(&s->sample_rate_atomic, rate);
            }
            reply_simple_ack(c, tag);
            break;
        }

        case CMD_SET_PLAYBACK_STREAM_NAME:
        case CMD_SET_RECORD_STREAM_NAME: {
            uint32_t ch = 0;
            const char *nm = NULL;
            if (!tr_u32(&tr, &ch) || !tr_str(&tr, &nm)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (s && nm) snprintf(s->name, sizeof(s->name), "%s", nm);
            reply_simple_ack(c, tag);
            break;
        }

        case CMD_UPDATE_PLAYBACK_STREAM_PROPLIST:
        case CMD_UPDATE_RECORD_STREAM_PROPLIST: {
            uint32_t ch = 0, mode = 0;
            if (!tr_u32(&tr, &ch) || !tr_u32(&tr, &mode)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_client_stream(srv, c, ch);
            if (s) {
                tr_proplist(&tr, &s->props);
                const char *mn = propbag_get(&s->props, "media.name");
                if (mn) snprintf(s->name, sizeof(s->name), "%s", mn);
            }
            reply_simple_ack(c, tag);
            break;
        }

        case CMD_UPDATE_CLIENT_PROPLIST: {
            uint32_t mode = 0;
            if (tr_u32(&tr, &mode)) tr_proplist(&tr, &c->props);
            reply_simple_ack(c, tag);
            break;
        }

        case CMD_REMOVE_PLAYBACK_STREAM_PROPLIST:
        case CMD_REMOVE_RECORD_STREAM_PROPLIST:
        case CMD_REMOVE_CLIENT_PROPLIST:
            reply_simple_ack(c, tag);
            break;

        case CMD_MOVE_SINK_INPUT:
        case CMD_MOVE_SOURCE_OUTPUT: {
            uint32_t sidx = 0, dev_idx = PA_INVALID_INDEX;
            const char *dev_nm = NULL;
            if (!tr_u32(&tr, &sidx) || !tr_u32(&tr, &dev_idx) || !tr_str(&tr, &dev_nm)) {
                reply_err(c, tag, ERR_PROTOCOL);
                break;
            }
            pa_stream_t *s = find_stream_by_index(srv, sidx);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            refresh_device_cache(srv);
            if (cmd == CMD_MOVE_SINK_INPUT) {
                uint32_t resolved_idx = 0;
                char resolved_nm[128] = {0};
                if (!resolve_sink_idx_and_name(srv, dev_idx, dev_nm, &resolved_idx, resolved_nm, sizeof(resolved_nm))) {
                    reply_err(c, tag, ERR_NOENTITY);
                    break;
                }
                s->target_dev_idx = resolved_idx;
                s->pinned_route = (dev_idx != PA_INVALID_INDEX) ||
                                  (dev_nm && dev_nm[0] && strncmp(dev_nm, "@DEFAULT_", 9) != 0);
            } else {
                uint32_t resolved_idx = 0;
                char resolved_nm[160] = {0};
                bool is_mon = false;
                if (!resolve_source_idx_and_name(srv, dev_idx, dev_nm, &resolved_idx, resolved_nm, sizeof(resolved_nm), &is_mon)) {
                    reply_err(c, tag, ERR_NOENTITY);
                    break;
                }
                bool explicit_req = (dev_idx != PA_INVALID_INDEX) ||
                                    (dev_nm && dev_nm[0] && strncmp(dev_nm, "@DEFAULT_", 9) != 0);
                sync_active_capture_if_explicit(srv, resolved_idx, is_mon, s->peak_detect, explicit_req);
                s->target_dev_idx = resolved_idx;
                s->is_monitor     = is_mon;
                s->pinned_route   = explicit_req;
            }
            stream_update_device_id_locked(srv, s);
            reply_simple_ack(c, tag);
            broadcast_sub_event(srv, s->is_playback ? SUB_EVENT_SINK_INPUT : SUB_EVENT_SOURCE_OUTPUT,
                                SUB_EVENT_CHANGE, s->index);
            break;
        }

        case CMD_KILL_SINK_INPUT:
        case CMD_KILL_SOURCE_OUTPUT: {
            uint32_t idx = 0;
            if (!tr_u32(&tr, &idx)) { reply_err(c, tag, ERR_PROTOCOL); break; }
            pa_stream_t *s = find_stream_by_index(srv, idx);
            if (!s) { reply_err(c, tag, ERR_NOENTITY); break; }
            free_stream(srv, s);
            reply_simple_ack(c, tag);
            break;
        }

        case CMD_EXTENSION:
            reply_err(c, tag, ERR_NOEXTENSION);
            break;

        default:
            reply_err(c, tag, ERR_COMMAND);
            break;
    }
}

static void handle_data_memblock(lwasapi_pa_server_t *srv, pa_client_t *c,
                                 uint32_t channel, const uint8_t *pcm, size_t len) {
    pa_stream_t *s = find_client_stream(srv, c, channel);
    if (!s || !s->is_playback || s->frame_size == 0) return;

    uint32_t frames = (uint32_t)(len / s->frame_size);
    if (frames == 0) return;

    uint32_t ssz = pa_format_sample_size(s->ss.format);
    uint8_t  ch  = s->ss.channels;
    uint8_t  fmt = s->ss.format;
    uint64_t wr  = atomic_load_explicit(&s->wr_frame, memory_order_relaxed);
    uint64_t rd  = atomic_load_explicit(&s->rd_frame, memory_order_acquire);

    if (wr < rd) wr = rd;
    uint64_t free_f = PA_STREAM_RING_FRAMES - (wr - rd);
    if (frames > free_f) frames = (uint32_t)free_f;

    for (uint32_t i = 0; i < frames; i++) {
        const uint8_t *fp = pcm + (size_t)i * s->frame_size;
        float l = decode_pa_sample(fp, fmt);
        float r = (ch >= 2) ? decode_pa_sample(fp + ssz, fmt) : l;
        uint32_t pos = (uint32_t)((wr + i) & (PA_STREAM_RING_FRAMES - 1));
        s->ring[pos * 2 + 0] = l;
        s->ring[pos * 2 + 1] = r;
    }

    atomic_store_explicit(&s->wr_frame, wr + frames, memory_order_release);
    atomic_fetch_add_explicit(&s->write_index, (uint64_t)frames * s->frame_size, memory_order_relaxed);

    if (atomic_load_explicit(&s->in_prebuf, memory_order_acquire) &&
        ((wr + frames - rd) * s->frame_size >= s->prebuf)) {
        atomic_store_explicit(&s->in_prebuf, 0, memory_order_release);
        send_stream_started(s);
    }
}

static void close_pa_client(lwasapi_pa_server_t *srv, pa_client_t *c) {
    if (!c || !c->in_use) return;
    if (c->authed || c->name[0]) {
        lwasapi_debug_emit_vals(LWASAPI_DBG_PA, LWASAPI_DBG_INFO, c->pid, c->index,
                                (int32_t)c->version, (int32_t)c->out_count, 0,
                                "PA_CLIENT_CLOSE", c->name[0] ? c->name : "pa-client",
                                "closing PA client #%u (pid=%u)", c->index, c->pid);
    }
    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        if (srv->streams[i].in_use && srv->streams[i].client == c) {
            free_stream(srv, &srv->streams[i]);
        }
    }
    while (c->out_count > 0) {
        free(c->out_q[c->out_head].data);
        c->out_head = (c->out_head + 1) % PA_MAX_OUT_QUEUE;
        c->out_count--;
    }
    free(c->rx_body);
    c->rx_body = NULL;
    if (c->fd >= 0) close(c->fd);
    c->fd = -1;
    uint32_t idx = c->index;
    bool was_authed = c->authed;
    c->in_use = false;
    if (was_authed) broadcast_sub_event(srv, SUB_EVENT_CLIENT, SUB_EVENT_REMOVE, idx);
}

static void process_client_io(lwasapi_pa_server_t *srv, pa_client_t *c, short revents) {
    if (revents & (POLLERR | POLLHUP | POLLNVAL)) {
        close_pa_client(srv, c);
        return;
    }

    if (revents & POLLOUT) {
        while (c->out_count > 0) {
            pa_out_pkt_t *q = &c->out_q[c->out_head];
            ssize_t w = send(c->fd, q->data + q->sent, q->len - q->sent, MSG_DONTWAIT | MSG_NOSIGNAL);
            if (w < 0) {
                if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR) break;
                close_pa_client(srv, c);
                return;
            }
            q->sent += (size_t)w;
            if (q->sent < q->len) break;
            free(q->data);
            q->data = NULL;
            c->out_head = (c->out_head + 1) % PA_MAX_OUT_QUEUE;
            c->out_count--;
        }
    }

    if (revents & POLLIN) {
        for (int iter = 0; iter < 64 && c->fd >= 0; iter++) {
            if (c->rx_hdr_got < PA_PSTREAM_DESC_BYTES) {
                ssize_t r = recv(c->fd, c->rx_hdr + c->rx_hdr_got,
                                 PA_PSTREAM_DESC_BYTES - c->rx_hdr_got, MSG_DONTWAIT);
                if (r == 0) { close_pa_client(srv, c); return; }
                if (r < 0) {
                    if (errno == EAGAIN || errno == EWOULDBLOCK) break;
                    if (errno == EINTR) continue;
                    close_pa_client(srv, c);
                    return;
                }
                c->rx_hdr_got += (size_t)r;
                if (c->rx_hdr_got < PA_PSTREAM_DESC_BYTES) break;

                uint32_t blen = 0;
                if (!salsa_pa_parse_frame_hdr(c->rx_hdr, &blen, &c->rx_channel, NULL)) {
                    salsa_failsafe_record_trip(SALSA_FS_PA_BAD_PKT,
                                               "PA client exceeded 16MiB frame limit");
                    close_pa_client(srv, c);
                    return;
                }
                c->rx_body_len = (size_t)blen;
                c->rx_body_got = 0;
                if (c->rx_body_len > 0) {
                    c->rx_body = (uint8_t *)malloc(c->rx_body_len);
                    if (!c->rx_body) { close_pa_client(srv, c); return; }
                }
            }

            if (c->rx_body_got < c->rx_body_len) {
                ssize_t r = recv(c->fd, c->rx_body + c->rx_body_got,
                                 c->rx_body_len - c->rx_body_got, MSG_DONTWAIT);
                if (r == 0) { close_pa_client(srv, c); return; }
                if (r < 0) {
                    if (errno == EAGAIN || errno == EWOULDBLOCK) break;
                    if (errno == EINTR) continue;
                    close_pa_client(srv, c);
                    return;
                }
                c->rx_body_got += (size_t)r;
                if (c->rx_body_got < c->rx_body_len) break;
            }

            if (c->rx_channel == PA_PSTREAM_CTRL_CHANNEL) {
                handle_control_packet(srv, c, c->rx_body, c->rx_body_len);
            } else {
                handle_data_memblock(srv, c, c->rx_channel, c->rx_body, c->rx_body_len);
            }
            free(c->rx_body);
            c->rx_body = NULL;
            c->rx_hdr_got = 0;
            c->rx_body_len = 0;
            c->rx_body_got = 0;
        }
    }
}

static void service_stream_events_and_capture(lwasapi_pa_server_t *srv) {
    uint8_t pcm_tmp[16384];
    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        pa_stream_t *s = &srv->streams[i];
        if (!s->in_use || !s->client || !s->client->in_use) continue;

        if (s->is_playback) {
            uint64_t wr = atomic_load_explicit(&s->wr_frame, memory_order_acquire);
            uint64_t rd = atomic_load_explicit(&s->rd_frame, memory_order_acquire);
            if (atomic_load(&s->draining) && rd >= wr) {
                atomic_store(&s->draining, 0);
                reply_simple_ack(s->client, s->drain_tag);
            }
            uint64_t queued_bytes = (wr >= rd ? (wr - rd) : 0) * s->frame_size;
            uint64_t ack_pending = (s->requested_bytes_total > atomic_load(&s->write_index))
                                   ? (s->requested_bytes_total - atomic_load(&s->write_index)) : 0;
            if (queued_bytes + ack_pending < s->tlength && !atomic_load(&s->draining)) {
                uint32_t need = (uint32_t)(s->tlength - (queued_bytes + ack_pending));
                if (need >= s->minreq) send_stream_request(s, need);
            }
        } else {
            uint64_t wr = atomic_load_explicit(&s->wr_frame, memory_order_acquire);
            uint64_t rd = atomic_load_explicit(&s->rd_frame, memory_order_relaxed);
            if (atomic_load(&s->corked)) {
                atomic_store(&s->rd_frame, wr);
                continue;
            }
            bool voice_fold = is_voice_comms_stream(s);
            while ((wr >= rd) && ((wr - rd) * s->frame_size >= s->fragsize)) {
                uint32_t send_bytes = s->fragsize;
                if (send_bytes > sizeof(pcm_tmp)) send_bytes = sizeof(pcm_tmp) - (sizeof(pcm_tmp) % s->frame_size);
                uint32_t send_frames = send_bytes / s->frame_size;
                uint32_t ssz = pa_format_sample_size(s->ss.format);
                for (uint32_t f = 0; f < send_frames; f++) {
                    uint32_t idx = (uint32_t)((rd + f) & (PA_STREAM_RING_FRAMES - 1));
                    float l = s->ring[idx * 2 + 0];
                    float r = s->ring[idx * 2 + 1];
                    if (voice_fold) {
                        /* Fold 2-input audio interfaces (e.g. Scarlett Input 1 + Input 2) without doubling native 1-ch mics */
                        float m = (l == r) ? l : (l + r);
                        if (m > 1.0f) m = 1.0f;
                        else if (m < -1.0f) m = -1.0f;
                        l = m;
                        r = m;
                    }
                    uint8_t *dp = pcm_tmp + (size_t)f * s->frame_size;
                    if (s->ss.channels == 1) {
                        encode_pa_sample(dp, s->ss.format, voice_fold ? l : (0.5f * (l + r)));
                    } else {
                        encode_pa_sample(dp, s->ss.format, l);
                        encode_pa_sample(dp + ssz, s->ss.format, r);
                        for (uint8_t ch = 2; ch < s->ss.channels; ch++) {
                            encode_pa_sample(dp + (size_t)ch * ssz, s->ss.format, 0.0f);
                        }
                    }
                }
                if (!queue_packet(s->client, s->channel, pcm_tmp, (size_t)send_frames * s->frame_size)) break;
                rd += send_frames;
                atomic_fetch_add(&s->write_index, (uint64_t)send_frames * s->frame_size);
                atomic_fetch_add(&s->read_index, (uint64_t)send_frames * s->frame_size);
            }
            atomic_store_explicit(&s->rd_frame, rd, memory_order_release);
        }
        atomic_store(&s->need_notify, 0);
    }
}

static void *pa_server_thread_fn(void *arg) {
    lwasapi_pa_server_t *srv = (lwasapi_pa_server_t *)arg;
    pa_enable_realtime(60, -15);

    while (srv->running) {
        struct pollfd pfds[PA_MAX_CLIENTS + 2];
        pa_client_t *cmap[PA_MAX_CLIENTS + 2];
        int nfds = 0;

        pfds[nfds].fd = srv->listen_fd;
        pfds[nfds].events = POLLIN;
        cmap[nfds++] = NULL;

        pfds[nfds].fd = srv->wake_efd;
        pfds[nfds].events = POLLIN;
        cmap[nfds++] = NULL;

        pthread_mutex_lock(&srv->lock);
        service_stream_events_and_capture(srv);
        for (int i = 0; i < PA_MAX_CLIENTS; i++) {
            pa_client_t *c = &srv->clients[i];
            if (!c->in_use || c->fd < 0) continue;
            pfds[nfds].fd = c->fd;
            pfds[nfds].events = POLLIN | (c->out_count > 0 ? POLLOUT : 0);
            cmap[nfds++] = c;
        }
        pthread_mutex_unlock(&srv->lock);

        int pr = poll(pfds, (nfds_t)nfds, 50);
        if (pr < 0) {
            if (errno == EINTR) continue;
            break;
        }

        if (pfds[1].revents & POLLIN) {
            eventfd_t val = 0;
            if (eventfd_read(srv->wake_efd, &val) < 0) {}
        }

        pthread_mutex_lock(&srv->lock);
        if (pfds[0].revents & POLLIN) {
            for (;;) {
                int cfd = accept4(srv->listen_fd, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC);
                if (cfd < 0) break;

                struct ucred uc = {0};
                socklen_t ulen = sizeof(uc);
                bool cred_ok = (getsockopt(cfd, SOL_SOCKET, SO_PEERCRED, &uc, &ulen) == 0);
                if (cred_ok && uc.uid != getuid() && uc.uid != 0) {
                    close(cfd);
                    continue;
                }

                int slot = -1;
                for (int i = 0; i < PA_MAX_CLIENTS; i++) {
                    if (!srv->clients[i].in_use) { slot = i; break; }
                }
                if (slot < 0) {
                    close(cfd);
                    continue;
                }
                pa_client_t *nc = &srv->clients[slot];
                memset(nc, 0, sizeof(*nc));
                nc->in_use = true;
                nc->index  = srv->next_client_idx++;
                nc->fd     = cfd;
                if (cred_ok) {
                    nc->uid_valid = true;
                    nc->uid       = (uint32_t)uc.uid;
                    if (uc.pid > 0) nc->pid = (uint32_t)uc.pid;
                }
                nc->next_channel = 0;
            }
        }

        for (int i = 2; i < nfds; i++) {
            if (pfds[i].revents && cmap[i] && cmap[i]->in_use) {
                process_client_io(srv, cmap[i], pfds[i].revents);
            }
        }
        service_stream_events_and_capture(srv);
        pthread_mutex_unlock(&srv->lock);
    }
    return NULL;
}

/* ============================================================================
 * Public Lifecycle & RT-Safe Audio Mixing / Capture Hooks
 * ============================================================================ */

static void ensure_pulse_cookie(uint8_t *out_cookie, bool *out_valid) {
    if (out_valid) *out_valid = false;
    const char *home = getenv("HOME");
    if (!home) {
        struct passwd *pw = getpwuid(getuid());
        home = (pw && pw->pw_dir) ? pw->pw_dir : "/tmp";
    }
    char dir[256], cpath[320];
    snprintf(dir, sizeof(dir), "%s/.config/pulse", home);
    mkdir(dir, 0700);
    snprintf(cpath, sizeof(cpath), "%s/cookie", dir);
    struct stat st;
    if (stat(cpath, &st) == 0 && st.st_size == PA_NATIVE_COOKIE_LEN) {
        int cfd = open(cpath, O_RDONLY | O_CLOEXEC);
        if (cfd >= 0) {
            uint8_t tmp[PA_NATIVE_COOKIE_LEN];
            ssize_t got = read(cfd, tmp, sizeof(tmp));
            close(cfd);
            if (got == PA_NATIVE_COOKIE_LEN && out_cookie && out_valid) {
                memcpy(out_cookie, tmp, sizeof(tmp));
                *out_valid = true;
            }
        }
        return;
    }

    uint8_t cookie[PA_NATIVE_COOKIE_LEN];
    int rfd = open("/dev/urandom", O_RDONLY | O_CLOEXEC);
    ssize_t got = 0;
    if (rfd >= 0) {
        got = read(rfd, cookie, sizeof(cookie));
        close(rfd);
    }
    if (got == PA_NATIVE_COOKIE_LEN) {
        int cfd = open(cpath, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
        if (cfd >= 0) {
            ssize_t wr = write(cfd, cookie, sizeof(cookie));
            close(cfd);
            if (wr == PA_NATIVE_COOKIE_LEN && out_cookie && out_valid) {
                memcpy(out_cookie, cookie, sizeof(cookie));
                *out_valid = true;
            }
        }
    }
}

static uint32_t collect_pa_facility_ids(const lwasapi_device_list_t *dl, uint32_t facility,
                                        char out_ids[LWASAPI_MAX_DEVICES * 2][64]);

int lwasapi_pa_server_start(lwasapi_pa_server_t **out_srv, lwasapi_pa_callbacks_t cbs) {
    if (!out_srv) return -1;

    const char *xdg = getenv("XDG_RUNTIME_DIR");
    char runtime_fallback[64];
    if (!xdg || !xdg[0]) {
        snprintf(runtime_fallback, sizeof(runtime_fallback), "/run/user/%u", (unsigned)getuid());
        xdg = runtime_fallback;
    }

    char pulse_dir[256];
    snprintf(pulse_dir, sizeof(pulse_dir), "%s/pulse", xdg);
    mkdir(pulse_dir, 0700);

    lwasapi_pa_server_t *srv = (lwasapi_pa_server_t *)calloc(1, sizeof(*srv));
    if (!srv) return -1;
    ensure_pulse_cookie(srv->cookie, &srv->cookie_valid);
    srv->cbs = cbs;
    srv->next_client_idx = 1;
    srv->next_stream_idx = 1;
    pa_init_pi_mutex(&srv->lock);
    pthread_mutex_init(&srv->vol_file_lock, NULL);
    atomic_init(&srv->dom_stream_rate, 0U);
    atomic_init(&srv->dom_stream_bits, 0U);

    init_app_vols_path(srv);
    load_app_volumes_from_disk(srv);

    snprintf(srv->sock_path, sizeof(srv->sock_path), "%.248s/native", pulse_dir);
    snprintf(srv->pid_path, sizeof(srv->pid_path), "%.251s/pid", pulse_dir);
    unlink(srv->sock_path);

    srv->listen_fd = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (srv->listen_fd < 0) {
        free(srv);
        return -1;
    }

    struct sockaddr_un sa;
    memset(&sa, 0, sizeof(sa));
    sa.sun_family = AF_UNIX;
    size_t sun_len = strnlen(srv->sock_path, sizeof(sa.sun_path));
    if (sun_len >= sizeof(sa.sun_path)) {
        close(srv->listen_fd);
        free(srv);
        return -1;
    }
    memcpy(sa.sun_path, srv->sock_path, sun_len + 1U);
    if (bind(srv->listen_fd, (struct sockaddr *)&sa, sizeof(sa)) < 0 || listen(srv->listen_fd, 32) < 0) {
        close(srv->listen_fd);
        free(srv);
        return -1;
    }
    chmod(srv->sock_path, 0600);

    FILE *pfp = fopen(srv->pid_path, "w");
    if (pfp) {
        fprintf(pfp, "%d\n", (int)getpid());
        fclose(pfp);
    }

    srv->wake_efd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    srv->mon_pa_vol = PA_VOLUME_NORM;
    atomic_init(&srv->mon_vol_milli, 1000U);
    atomic_init(&srv->mon_muted, 0);
    refresh_device_cache_force(srv);
    {
        char init_ids[LWASAPI_MAX_DEVICES * 2][64];
        uint32_t n;
        n = collect_pa_facility_ids(&srv->cached_devices, SUB_EVENT_CARD, init_ids);
        for (uint32_t i = 0; i < n; i++) stable_pa_idx(srv, PA_FAC_CARD, init_ids[i]);
        n = collect_pa_facility_ids(&srv->cached_devices, SUB_EVENT_SINK, init_ids);
        for (uint32_t i = 0; i < n; i++) stable_pa_idx(srv, PA_FAC_SINK, init_ids[i]);
        n = collect_pa_facility_ids(&srv->cached_devices, SUB_EVENT_SOURCE, init_ids);
        for (uint32_t i = 0; i < n; i++) stable_pa_idx(srv, PA_FAC_SOURCE, init_ids[i]);
    }
    srv->running = 1;
    pthread_create(&srv->thread, NULL, pa_server_thread_fn, srv);
    printf("[Salsa-PA] Native PulseAudio v35 socket bound at %s (no PipeWire)\n", srv->sock_path);
    *out_srv = srv;
    return 0;
}

void lwasapi_pa_server_stop(lwasapi_pa_server_t *srv) {
    if (!srv) return;
    srv->running = 0;
    if (srv->wake_efd >= 0) eventfd_write(srv->wake_efd, 1);
    if (srv->thread) pthread_join(srv->thread, NULL);

    pthread_mutex_lock(&srv->lock);
    for (int i = 0; i < PA_MAX_CLIENTS; i++) {
        if (srv->clients[i].in_use) close_pa_client(srv, &srv->clients[i]);
    }
    pthread_mutex_unlock(&srv->lock);

    if (srv->listen_fd >= 0) close(srv->listen_fd);
    if (srv->wake_efd >= 0) close(srv->wake_efd);
    unlink(srv->sock_path);
    unlink(srv->pid_path);
    pthread_mutex_destroy(&srv->lock);
    pthread_mutex_destroy(&srv->vol_file_lock);
    free(srv);
}

static uint32_t collect_pa_facility_ids(const lwasapi_device_list_t *dl, uint32_t facility,
                                        char out_ids[LWASAPI_MAX_DEVICES * 2][64]) {
    uint32_t n = 0;
    const uint32_t cap = LWASAPI_MAX_DEVICES * 2U;
    if (!dl) return 0;
    if (facility == SUB_EVENT_SINK) {
        for (uint32_t i = 0; i < dl->num_devices && n < cap; i++) {
            if (!dl->devices[i].is_capture) snprintf(out_ids[n++], 64, "%s", dl->devices[i].id);
        }
        if (n == 0 && n < cap) snprintf(out_ids[n++], 64, "virtual");
    } else if (facility == SUB_EVENT_SOURCE) {
        for (uint32_t i = 0; i < dl->num_devices && n < cap; i++) {
            if (dl->devices[i].is_capture) snprintf(out_ids[n++], 64, "%s", dl->devices[i].id);
        }
        uint32_t pb_n = 0;
        for (uint32_t i = 0; i < dl->num_devices && n < cap; i++) {
            if (!dl->devices[i].is_capture) {
                snprintf(out_ids[n++], 64, "mon:%.58s", dl->devices[i].id);
                pb_n++;
            }
        }
        if (pb_n == 0 && n < cap) snprintf(out_ids[n++], 64, "mon:virtual");
    } else if (facility == SUB_EVENT_CARD) {
        for (uint32_t i = 0; i < dl->num_devices; i++) {
            bool dup = false;
            for (uint32_t j = 0; j < n; j++) {
                if (strcmp(out_ids[j], dl->devices[i].id) == 0) { dup = true; break; }
            }
            if (!dup) snprintf(out_ids[n++], 64, "%s", dl->devices[i].id);
        }
    }
    return n;
}

static void emit_facility_diff(lwasapi_pa_server_t *srv, uint32_t facility,
                               const lwasapi_device_list_t *old_dl,
                               const lwasapi_device_list_t *new_dl,
                               uint32_t active_idx) {
    char old_ids[LWASAPI_MAX_DEVICES * 2][64];
    char new_ids[LWASAPI_MAX_DEVICES * 2][64];
    uint32_t old_n = collect_pa_facility_ids(old_dl, facility, old_ids);
    uint32_t new_n = collect_pa_facility_ids(new_dl, facility, new_ids);
    uint32_t fac = (facility == SUB_EVENT_SINK)   ? PA_FAC_SINK   :
                   (facility == SUB_EVENT_SOURCE) ? PA_FAC_SOURCE : PA_FAC_CARD;
    bool emitted_active = false;

    for (uint32_t i = 0; i < old_n; i++) {
        bool still_present = false;
        for (uint32_t j = 0; j < new_n; j++) {
            if (strcmp(old_ids[i], new_ids[j]) == 0) {
                still_present = true;
                break;
            }
        }
        if (!still_present) {
            uint32_t pa_idx = stable_pa_idx(srv, fac, old_ids[i]);
            broadcast_sub_event(srv, facility, SUB_EVENT_REMOVE, pa_idx);
        }
    }

    for (uint32_t j = 0; j < new_n; j++) {
        bool existed = false;
        for (uint32_t i = 0; i < old_n; i++) {
            if (strcmp(new_ids[j], old_ids[i]) == 0) {
                existed = true;
                break;
            }
        }
        uint32_t pa_idx = stable_pa_idx(srv, fac, new_ids[j]);
        if (!existed) {
            broadcast_sub_event(srv, facility, SUB_EVENT_NEW, pa_idx);
            if (pa_idx == active_idx) emitted_active = true;
        }
    }

    if (active_idx != PA_INVALID_INDEX && !emitted_active) {
        broadcast_sub_event(srv, facility, SUB_EVENT_CHANGE, active_idx);
    }
}

void lwasapi_pa_server_notify_change(lwasapi_pa_server_t *srv) {
    if (!srv || !srv->running) return;
    lwasapi_engine_status_t new_st;
    lwasapi_device_list_t new_dl;
    memset(&new_st, 0, sizeof(new_st));
    memset(&new_dl, 0, sizeof(new_dl));
    if (srv->cbs.get_status)  srv->cbs.get_status(srv->cbs.user_ctx, &new_st);
    if (srv->cbs.get_devices) srv->cbs.get_devices(srv->cbs.user_ctx, &new_dl);

    pthread_mutex_lock(&srv->lock);
    lwasapi_device_list_t old_devices = srv->cached_devices;
    srv->cached_status = new_st;
    srv->cached_devices = new_dl;
    srv->devices_cache_valid = true;

    uint32_t sink_idx = find_active_sink_idx(srv);
    uint32_t src_idx  = find_active_source_idx(srv);
    uint32_t card_idx = find_card_idx_for_id(srv, &srv->cached_devices, srv->cached_status.active_playback_id);
    emit_facility_diff(srv, SUB_EVENT_CARD,   &old_devices, &srv->cached_devices, card_idx);
    emit_facility_diff(srv, SUB_EVENT_SINK,   &old_devices, &srv->cached_devices, sink_idx);
    emit_facility_diff(srv, SUB_EVENT_SOURCE, &old_devices, &srv->cached_devices, src_idx);
    uint32_t active_mon_idx = find_monitor_src_idx_for_sink(srv, sink_idx);
    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        pa_stream_t *s = &srv->streams[i];
        if (!s->in_use) continue;
        if (s->is_playback) {
            char dummy_id[64] = {0};
            bool cur_valid = sink_idx_to_salsa_id(srv, &srv->cached_devices, s->target_dev_idx,
                                                  dummy_id, sizeof(dummy_id));
            if (s->pinned_route && cur_valid) {
                stream_update_device_id_locked(srv, s);
                continue;
            }
            if (!cur_valid) s->pinned_route = false;
            if (s->target_dev_idx != sink_idx) {
                s->target_dev_idx = sink_idx;
                stream_update_device_id_locked(srv, s);
                broadcast_sub_event(srv, SUB_EVENT_SINK_INPUT, SUB_EVENT_CHANGE, s->index);
            } else {
                stream_update_device_id_locked(srv, s);
            }
        } else if (!s->is_playback && s->direct_on_input != PA_INVALID_INDEX &&
                   active_mon_idx != PA_INVALID_INDEX && s->target_dev_idx != active_mon_idx) {
            s->target_dev_idx = active_mon_idx;
            stream_update_device_id_locked(srv, s);
            broadcast_sub_event(srv, SUB_EVENT_SOURCE_OUTPUT, SUB_EVENT_CHANGE, s->index);
        } else {
            stream_update_device_id_locked(srv, s);
        }
    }
    broadcast_sub_event(srv, SUB_EVENT_SERVER, SUB_EVENT_CHANGE, PA_INVALID_INDEX);
    pthread_mutex_unlock(&srv->lock);
    if (srv->wake_efd >= 0) eventfd_write(srv->wake_efd, 1);
}

void lwasapi_pa_server_notify_volume(lwasapi_pa_server_t *srv, bool is_capture) {
    if (!srv || !srv->running) return;
    lwasapi_engine_status_t new_st;
    memset(&new_st, 0, sizeof(new_st));
    if (srv->cbs.get_status) srv->cbs.get_status(srv->cbs.user_ctx, &new_st);

    pthread_mutex_lock(&srv->lock);
    srv->cached_status = new_st;
    if (is_capture) srv->hw_src_pa_vol_valid = false;
    else srv->sink_pa_vol_valid = false;
    broadcast_sub_event(srv,
                        is_capture ? SUB_EVENT_SOURCE : SUB_EVENT_SINK,
                        SUB_EVENT_CHANGE,
                        is_capture ? find_active_source_idx(srv) : find_active_sink_idx(srv));
    pthread_mutex_unlock(&srv->lock);
    if (srv->wake_efd >= 0) eventfd_write(srv->wake_efd, 1);
}

static inline float pa_catmull_rom(float ym1, float y0, float y1, float y2, double frac) {
    float t = (float)frac;
    float t2 = t * t;
    float t3 = t2 * t;
    return 0.5f * ((2.0f * y0) +
                   (-ym1 + y1) * t +
                   (2.0f * ym1 - 5.0f * y0 + 4.0f * y1 - y2) * t2 +
                   (-ym1 + 3.0f * y0 - 3.0f * y1 + y2) * t3);
}

static inline void pa_stream_save_hist(pa_stream_t *s, uint64_t consumed, uint64_t new_rd) {
    if (consumed == 0) return;
    uint32_t m1 = (uint32_t)((new_rd - 1ULL) & (PA_STREAM_RING_FRAMES - 1));
    if (consumed >= 2ULL) {
        uint32_t m2 = (uint32_t)((new_rd - 2ULL) & (PA_STREAM_RING_FRAMES - 1));
        s->hist_l[0] = s->ring[m2 * 2 + 0];
        s->hist_r[0] = s->ring[m2 * 2 + 1];
    } else {
        s->hist_l[0] = s->have_hist ? s->hist_l[1] : s->ring[m1 * 2 + 0];
        s->hist_r[0] = s->have_hist ? s->hist_r[1] : s->ring[m1 * 2 + 1];
    }
    s->hist_l[1] = s->ring[m1 * 2 + 0];
    s->hist_r[1] = s->ring[m1 * 2 + 1];
    s->have_hist = true;
}

static inline void pa_stream_emit_fade_out(pa_stream_t *s, float *stereo_out,
                                           uint32_t offset, uint32_t rem,
                                           uint32_t engine_rate, uint32_t doi_cap) {
    if (rem == 0) {
        s->last_mix_l = 0.0f;
        s->last_mix_r = 0.0f;
        return;
    }
    /* ~0.5 ms de-click ramp scaled to engine rate so flush/cork/underrun never clicks the EQ */
    uint32_t target = (engine_rate >= 4000U) ? (engine_rate / 2000U) : 24U;
    if (target < 16U) target = 16U;
    uint32_t fade_n = (rem < target) ? rem : target;
    float l0 = s->last_mix_l, r0 = s->last_mix_r;
    if (l0 == 0.0f && r0 == 0.0f) return;
    for (uint32_t j = 0; j < rem; j++) {
        float fl = 0.0f, fr = 0.0f;
        if (j < fade_n) {
            float g = (float)(fade_n - 1U - j) / (float)fade_n;
            fl = l0 * g;
            fr = r0 * g;
            stereo_out[(offset + j) * 2 + 0] += fl;
            stereo_out[(offset + j) * 2 + 1] += fr;
        }
        if (offset + j < doi_cap) {
            s->period_pcm[(offset + j) * 2 + 0] = fl;
            s->period_pcm[(offset + j) * 2 + 1] = fr;
        }
    }
    if (offset + fade_n > s->period_frames) {
        uint32_t end = offset + fade_n;
        s->period_frames = (end < doi_cap) ? end : doi_cap;
    }
    s->last_mix_l = 0.0f;
    s->last_mix_r = 0.0f;
}

void lwasapi_pa_server_mix_playback(lwasapi_pa_server_t *srv, float *stereo_out,
                                    uint32_t frames, uint32_t engine_rate) {
    if (!srv || !stereo_out || frames == 0 || engine_rate == 0) return;
    bool wake_needed = false;
    uint32_t doi_cap = (frames <= PA_MAX_DOI_PERIOD) ? frames : PA_MAX_DOI_PERIOD;
    uint32_t best_rate = 0, best_bits = 0;

    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        pa_stream_t *s = &srv->streams[i];
        if (!atomic_load_explicit(&s->active, memory_order_acquire) || !s->is_playback) continue;
        s->mix_peak_l = 0.0f;
        s->mix_peak_r = 0.0f;
        s->period_frames = 0;
        if (atomic_load_explicit(&s->in_prebuf, memory_order_acquire) ||
            atomic_load_explicit(&s->corked, memory_order_relaxed)) {
            if (s->last_mix_l != 0.0f || s->last_mix_r != 0.0f) {
                if (doi_cap > 0) memset(s->period_pcm, 0, (size_t)doi_cap * 2 * sizeof(float));
                pa_stream_emit_fade_out(s, stereo_out, 0, frames, engine_rate, doi_cap);
            }
            continue;
        }

        uint64_t wr = atomic_load_explicit(&s->wr_frame, memory_order_acquire);
        uint64_t rd = atomic_load_explicit(&s->rd_frame, memory_order_relaxed);
        uint64_t avail = (wr >= rd) ? (wr - rd) : 0;
        if (avail == 0) {
            if (s->last_mix_l != 0.0f || s->last_mix_r != 0.0f) {
                if (doi_cap > 0) memset(s->period_pcm, 0, (size_t)doi_cap * 2 * sizeof(float));
                pa_stream_emit_fade_out(s, stereo_out, 0, frames, engine_rate, doi_cap);
            }
            if (!atomic_load_explicit(&s->draining, memory_order_relaxed)) {
                atomic_store_explicit(&s->underrun_baseline_bytes,
                                      atomic_load_explicit(&s->read_index, memory_order_relaxed),
                                      memory_order_relaxed);
                wake_needed = true;
            }
            continue;
        }

        bool muted = atomic_load_explicit(&s->muted, memory_order_relaxed) != 0;
        float vol  = muted ? 0.0f : ((float)atomic_load_explicit(&s->volume_milli, memory_order_relaxed) / 1000.0f);
        uint32_t srate = atomic_load_explicit(&s->sample_rate_atomic, memory_order_relaxed);
        if (srate == 0) srate = engine_rate;
        uint32_t sbits = pa_format_sample_bits(s->ss.format);
        if (srate > best_rate || (srate == best_rate && sbits > best_bits)) {
            best_rate = srate;
            best_bits = sbits;
        }
        float pk_l = 0.0f, pk_r = 0.0f;
        float last_sl = s->last_mix_l, last_sr = s->last_mix_r;

        if (srate == engine_rate) {
            uint32_t n = (avail >= frames) ? frames : (uint32_t)avail;
            for (uint32_t f = 0; f < n; f++) {
                uint32_t idx = (uint32_t)((rd + f) & (PA_STREAM_RING_FRAMES - 1));
                float sl = s->ring[idx * 2 + 0] * vol;
                float sr = s->ring[idx * 2 + 1] * vol;
                last_sl = sl;
                last_sr = sr;
                stereo_out[f * 2 + 0] += sl;
                stereo_out[f * 2 + 1] += sr;
                if (f < doi_cap) {
                    s->period_pcm[f * 2 + 0] = sl;
                    s->period_pcm[f * 2 + 1] = sr;
                }
                float al = fabsf(sl), ar = fabsf(sr);
                if (al > pk_l) pk_l = al;
                if (ar > pk_r) pk_r = ar;
            }
            s->last_mix_l = last_sl;
            s->last_mix_r = last_sr;
            if (n < frames) {
                pa_stream_emit_fade_out(s, stereo_out, n, frames - n, engine_rate, doi_cap);
            }
            rd += n;
            pa_stream_save_hist(s, n, rd);
            atomic_fetch_add_explicit(&s->read_index, (uint64_t)n * s->frame_size, memory_order_relaxed);
        } else {
            double step = (double)srate / (double)engine_rate;
            double pos  = s->resamp_phase;
            uint32_t f  = 0;
            while (f < frames && (uint64_t)floor(pos) < avail) {
                uint64_t p0 = rd + (uint64_t)floor(pos);
                double frac = pos - floor(pos);
                uint32_t i0 = (uint32_t)(p0 & (PA_STREAM_RING_FRAMES - 1));
                float l, r;
                if (p0 + 2 < wr && (p0 > rd || s->have_hist)) {
                    float lm1, rm1;
                    if (p0 > rd) {
                        uint32_t im1 = (uint32_t)((p0 - 1ULL) & (PA_STREAM_RING_FRAMES - 1));
                        lm1 = s->ring[im1 * 2 + 0];
                        rm1 = s->ring[im1 * 2 + 1];
                    } else {
                        lm1 = s->hist_l[1];
                        rm1 = s->hist_r[1];
                    }
                    uint32_t i1 = (uint32_t)((p0 + 1ULL) & (PA_STREAM_RING_FRAMES - 1));
                    uint32_t i2 = (uint32_t)((p0 + 2ULL) & (PA_STREAM_RING_FRAMES - 1));
                    l = pa_catmull_rom(lm1, s->ring[i0 * 2 + 0], s->ring[i1 * 2 + 0], s->ring[i2 * 2 + 0], frac);
                    r = pa_catmull_rom(rm1, s->ring[i0 * 2 + 1], s->ring[i1 * 2 + 1], s->ring[i2 * 2 + 1], frac);
                } else {
                    uint32_t i1 = (uint32_t)((p0 + 1 < wr ? p0 + 1 : p0) & (PA_STREAM_RING_FRAMES - 1));
                    l = (float)(s->ring[i0 * 2 + 0] * (1.0 - frac) + s->ring[i1 * 2 + 0] * frac);
                    r = (float)(s->ring[i0 * 2 + 1] * (1.0 - frac) + s->ring[i1 * 2 + 1] * frac);
                }
                float sl = l * vol, sr = r * vol;
                last_sl = sl;
                last_sr = sr;
                stereo_out[f * 2 + 0] += sl;
                stereo_out[f * 2 + 1] += sr;
                if (f < doi_cap) {
                    s->period_pcm[f * 2 + 0] = sl;
                    s->period_pcm[f * 2 + 1] = sr;
                }
                float al = fabsf(sl), ar = fabsf(sr);
                if (al > pk_l) pk_l = al;
                if (ar > pk_r) pk_r = ar;
                pos += step;
                f++;
            }
            s->last_mix_l = last_sl;
            s->last_mix_r = last_sr;
            if (f < frames) {
                pa_stream_emit_fade_out(s, stereo_out, f, frames - f, engine_rate, doi_cap);
            }
            uint64_t adv = (uint64_t)floor(pos);
            if (rd + adv > wr) adv = wr - rd;
            rd += adv;
            pa_stream_save_hist(s, adv, rd);
            s->resamp_phase = pos - (double)adv;
            atomic_fetch_add_explicit(&s->read_index, adv * s->frame_size, memory_order_relaxed);
        }
        s->period_frames = doi_cap;
        s->mix_peak_l = pk_l;
        s->mix_peak_r = pk_r;
        atomic_store_explicit(&s->rd_frame, rd, memory_order_release);

        if ((wr - rd) * s->frame_size < s->tlength) {
            wake_needed = true;
        }
    }

    atomic_store_explicit(&srv->dom_stream_rate, best_rate, memory_order_relaxed);
    atomic_store_explicit(&srv->dom_stream_bits, best_bits, memory_order_relaxed);

    if (wake_needed && srv->wake_efd >= 0) {
        eventfd_write(srv->wake_efd, 1);
    }
}

static inline float clamp_cap_unit(float x) {
    if (x > 1.0f) return 1.0f;
    if (x < -1.0f) return -1.0f;
    return x;
}

void lwasapi_pa_server_push_capture(lwasapi_pa_server_t *srv,
                                    const float *mic_stereo, uint32_t mic_frames, uint32_t mic_rate,
                                    const float *monitor_stereo, uint32_t mon_frames, uint32_t mon_rate) {
    if (!srv) return;
    bool wake_needed = false;

    for (int i = 0; i < PA_MAX_STREAMS; i++) {
        pa_stream_t *s = &srv->streams[i];
        if (!atomic_load_explicit(&s->active, memory_order_acquire)) continue;
        if (s->is_playback || atomic_load_explicit(&s->corked, memory_order_relaxed)) continue;

        bool has_doi = (s->direct_on_input != PA_INVALID_INDEX);
        bool use_mon_src = s->is_monitor || has_doi;
        const float *src = s->is_monitor ? monitor_stereo : mic_stereo;
        uint32_t frames = use_mon_src ? mon_frames : mic_frames;
        uint32_t engine_rate = use_mon_src ? mon_rate : mic_rate;
        if (frames == 0U || engine_rate == 0U) continue;
        if (!has_doi && !s->peak_detect && !src) continue;

        float doi_l = 0.0f, doi_r = 0.0f;
        const float *doi_pcm = NULL;
        if (has_doi) {
            pa_stream_t *target_pb = find_stream_by_index(srv, s->direct_on_input);
            if (target_pb && atomic_load_explicit(&target_pb->active, memory_order_acquire) &&
                target_pb->is_playback && !atomic_load_explicit(&target_pb->corked, memory_order_relaxed)) {
                doi_l = target_pb->mix_peak_l;
                doi_r = target_pb->mix_peak_r;
                if (!s->peak_detect && target_pb->period_frames >= frames) {
                    doi_pcm = target_pb->period_pcm;
                }
            }
            if (!s->peak_detect) {
                src = doi_pcm;
            }
        }

        bool s_muted = (atomic_load_explicit(&s->muted, memory_order_relaxed) != 0);
        uint32_t s_vol_milli = atomic_load_explicit(&s->volume_milli, memory_order_relaxed);
        float s_gain = s_muted ? 0.0f : ((float)s_vol_milli / 1000.0f);
        if (s_gain > 2.0f) s_gain = 2.0f;
        float mon_gain = 1.0f;
        if (s->is_monitor && !has_doi) {
            bool mm = (atomic_load_explicit(&srv->mon_muted, memory_order_relaxed) != 0);
            uint32_t mv = atomic_load_explicit(&srv->mon_vol_milli, memory_order_relaxed);
            mon_gain = mm ? 0.0f : ((float)mv / 1000.0f);
            if (mon_gain > 2.0f) mon_gain = 2.0f;
        }
        float target_cap_gain = s_gain * mon_gain;
        if (target_cap_gain > 2.0f) target_cap_gain = 2.0f;
        float cur_gain = s->cap_smooth_gain;
        float gain_step = (target_cap_gain - cur_gain) / (float)frames;

        uint64_t wr = atomic_load_explicit(&s->wr_frame, memory_order_relaxed);
        uint64_t rd = atomic_load_explicit(&s->rd_frame, memory_order_acquire);
        uint32_t srate = atomic_load_explicit(&s->sample_rate_atomic, memory_order_relaxed);
        if (srate == 0) srate = engine_rate;

        if (s->peak_detect) {
            double step = (double)engine_rate / (double)srate;
            if (step < 1.0) step = 1.0;
            for (uint32_t f = 0; f < frames; f++) {
                cur_gain += gain_step;
                float al = (has_doi ? doi_l : (src ? fabsf(src[f * 2 + 0]) : 0.0f)) * cur_gain;
                float ar = (has_doi ? doi_r : (src ? fabsf(src[f * 2 + 1]) : 0.0f)) * cur_gain;
                if (al > 1.0f) al = 1.0f;
                if (ar > 1.0f) ar = 1.0f;
                if (al > s->peak_accum_l) s->peak_accum_l = al;
                if (ar > s->peak_accum_r) s->peak_accum_r = ar;
                s->resamp_phase += 1.0;
                if (s->resamp_phase >= step) {
                    if (wr >= rd && (wr - rd) < PA_STREAM_RING_FRAMES) {
                        uint32_t idx = (uint32_t)(wr & (PA_STREAM_RING_FRAMES - 1));
                        s->ring[idx * 2 + 0] = s->peak_accum_l;
                        s->ring[idx * 2 + 1] = s->peak_accum_r;
                        wr++;
                    }
                    s->peak_accum_l = 0.0f;
                    s->peak_accum_r = 0.0f;
                    s->resamp_phase -= step;
                }
            }
        } else if (srate == engine_rate) {
            for (uint32_t f = 0; f < frames; f++) {
                cur_gain += gain_step;
                if (wr < rd || (wr - rd) >= PA_STREAM_RING_FRAMES) break;
                uint32_t idx = (uint32_t)(wr & (PA_STREAM_RING_FRAMES - 1));
                s->ring[idx * 2 + 0] = src ? clamp_cap_unit(src[f * 2 + 0] * cur_gain) : 0.0f;
                s->ring[idx * 2 + 1] = src ? clamp_cap_unit(src[f * 2 + 1] * cur_gain) : 0.0f;
                wr++;
            }
            if (src && frames > 0U) {
                s->cap_hist_l = src[(frames - 1U) * 2U + 0U];
                s->cap_hist_r = src[(frames - 1U) * 2U + 1U];
                s->have_cap_hist = true;
            }
        } else if (frames > 0U) {
            double step = (double)engine_rate / (double)srate;
            double pos  = s->resamp_phase;
            double approx_out = (double)frames / step;
            if (approx_out < 1.0) approx_out = 1.0;
            float out_step = (target_cap_gain - cur_gain) / (float)approx_out;
            double limit = (double)(frames - 1U);
            while (pos < limit) {
                if ((out_step >= 0.0f && cur_gain < target_cap_gain) ||
                    (out_step < 0.0f && cur_gain > target_cap_gain)) {
                    cur_gain += out_step;
                }
                if (wr >= rd && (wr - rd) < PA_STREAM_RING_FRAMES) {
                    uint32_t idx = (uint32_t)(wr & (PA_STREAM_RING_FRAMES - 1));
                    if (!src) {
                        s->ring[idx * 2 + 0] = 0.0f;
                        s->ring[idx * 2 + 1] = 0.0f;
                    } else if (pos < 0.0) {
                        double frac = pos + 1.0;
                        if (frac < 0.0) frac = 0.0;
                        if (frac > 1.0) frac = 1.0;
                        float l0 = s->have_cap_hist ? s->cap_hist_l : src[0];
                        float r0 = s->have_cap_hist ? s->cap_hist_r : src[1];
                        float l = (float)(l0 * (1.0 - frac) + src[0] * frac) * cur_gain;
                        float r = (float)(r0 * (1.0 - frac) + src[1] * frac) * cur_gain;
                        s->ring[idx * 2 + 0] = clamp_cap_unit(l);
                        s->ring[idx * 2 + 1] = clamp_cap_unit(r);
                    } else {
                        uint32_t i0 = (uint32_t)pos;
                        uint32_t i1 = (i0 + 1U < frames) ? (i0 + 1U) : i0;
                        double frac = pos - (double)i0;
                        float l = (float)(src[i0 * 2 + 0] * (1.0 - frac) + src[i1 * 2 + 0] * frac) * cur_gain;
                        float r = (float)(src[i0 * 2 + 1] * (1.0 - frac) + src[i1 * 2 + 1] * frac) * cur_gain;
                        s->ring[idx * 2 + 0] = clamp_cap_unit(l);
                        s->ring[idx * 2 + 1] = clamp_cap_unit(r);
                    }
                    wr++;
                }
                pos += step;
            }
            if (src) {
                s->cap_hist_l = src[(frames - 1U) * 2U + 0U];
                s->cap_hist_r = src[(frames - 1U) * 2U + 1U];
                s->have_cap_hist = true;
            }
            s->resamp_phase = pos - (double)frames;
            if (s->resamp_phase < -1.0) s->resamp_phase = -1.0;
        }
        s->cap_smooth_gain = target_cap_gain;
        atomic_store_explicit(&s->wr_frame, wr, memory_order_release);
        if ((wr >= rd) && ((wr - rd) * s->frame_size >= s->fragsize)) {
            wake_needed = true;
        }
    }

    if (wake_needed && srv->wake_efd >= 0) {
        eventfd_write(srv->wake_efd, 1);
    }
}

void lwasapi_pa_server_fill_sessions(lwasapi_pa_server_t *srv, lwasapi_engine_status_t *st,
                                     uint32_t *out_num_clients) {
    if (!srv || !st) return;
    pthread_mutex_lock(&srv->lock);
    uint32_t nc = 0;
    for (int i = 0; i < PA_MAX_CLIENTS; i++) {
        if (srv->clients[i].in_use && srv->clients[i].authed) nc++;
    }
    if (out_num_clients) *out_num_clients = nc;

    for (int i = 0; i < PA_MAX_STREAMS && st->num_sessions < LWASAPI_MAX_SESSIONS; i++) {
        pa_stream_t *s = &srv->streams[i];
        if (!s->in_use) continue;
        lwasapi_session_info_t *dst = &st->sessions[st->num_sessions++];
        memset(dst, 0, sizeof(*dst));
        dst->session_id = 10000U + s->index;
        dst->client_pid = s->client ? s->client->pid : 0;
        snprintf(dst->client_name, sizeof(dst->client_name), "%s",
                 s->name[0] ? s->name : (s->client && s->client->name[0] ? s->client->name : "PA Client"));
        dst->share_mode = 2; /* Desktop PulseAudio Socket Mode */
        dst->is_capture = s->is_playback ? 0 : 1;
        dst->sample_rate = s->ss.rate;
        dst->channels = s->ss.channels;
        dst->active = (atomic_load(&s->active) && !atomic_load(&s->corked)) ? 1 : 0;
        dst->volume_milli = atomic_load(&s->volume_milli);
        dst->muted = atomic_load(&s->muted) ? 1U : 0U;
        dst->routing_flags = s->pinned_route ? 1U : 0U;
        snprintf(dst->target_device, sizeof(dst->target_device), "%s", s->device_id);
        uint64_t wr = atomic_load(&s->wr_frame);
        uint64_t rd = atomic_load(&s->rd_frame);
        dst->buffered_frames = (wr > rd) ? (wr - rd) : 0;
    }
    pthread_mutex_unlock(&srv->lock);
}

int lwasapi_pa_server_control_stream(lwasapi_pa_server_t *srv, uint32_t session_id,
                                     int32_t vol_pct, int32_t mute_mode, const char *target_device_id) {
    if (!srv || session_id < 10000U) return -1;
    uint32_t target_idx = session_id - 10000U;
    if (vol_pct > 150) vol_pct = 150;

    bool save_dirty = false;
    char save_keys[PA_MAX_APP_VOLUMES][128];
    uint32_t save_millis[PA_MAX_APP_VOLUMES];
    uint8_t save_mutes[PA_MAX_APP_VOLUMES];
    uint32_t save_cnt = 0;
    char save_path[320] = {0};

    pthread_mutex_lock(&srv->lock);
    pa_stream_t *s = find_stream_by_index(srv, target_idx);
    if (!s) {
        pthread_mutex_unlock(&srv->lock);
        return -1;
    }

    if (target_device_id && target_device_id[0]) {
        refresh_device_cache(srv);
        if (strcmp(target_device_id, "auto") == 0) {
            s->pinned_route = false;
            s->target_dev_idx = s->is_playback ? find_active_sink_idx(srv) : find_active_source_idx(srv);
        } else {
            bool matched = false;
            uint32_t dir_cnt = 0;
            for (uint32_t i = 0; i < srv->cached_devices.num_devices; i++) {
                const lwasapi_device_entry_t *d = &srv->cached_devices.devices[i];
                if ((d->is_capture == 0) != s->is_playback) continue;
                dir_cnt++;
                if (strcmp(d->id, target_device_id) == 0) {
                    s->target_dev_idx = stable_pa_idx(srv, s->is_playback ? PA_FAC_SINK : PA_FAC_SOURCE, d->id);
                    s->pinned_route = true;
                    matched = true;
                    break;
                }
            }
            if (!matched && s->is_playback && dir_cnt == 0 && strcmp(target_device_id, "virtual") == 0) {
                s->target_dev_idx = stable_pa_idx(srv, PA_FAC_SINK, "virtual");
                s->pinned_route = true;
                matched = true;
            }
            if (!matched) {
                pthread_mutex_unlock(&srv->lock);
                return -1;
            }
        }
        stream_update_device_id_locked(srv, s);
    }

    bool vol_or_mute_changed = false;
    if (vol_pct >= 0) {
        uint32_t pv = pct_to_pa_vol((uint32_t)vol_pct);
        for (uint8_t k = 0; k < s->vol.channels; k++) s->vol.values[k] = pv;
        atomic_store(&s->volume_milli, (uint32_t)vol_pct * 10U);
        vol_or_mute_changed = true;
    }
    if (mute_mode >= 0 && mute_mode <= 2) {
        int new_m = (mute_mode == 2)
                  ? (atomic_load(&s->muted) ? 0 : 1)
                  : (mute_mode ? 1 : 0);
        atomic_store(&s->muted, new_m);
        vol_or_mute_changed = true;
    }
    if (vol_or_mute_changed) {
        record_app_volume_locked(srv, s);
        save_cnt = collect_app_volumes_snapshot_locked(srv, save_keys, save_millis, save_mutes,
                                                       save_path, sizeof(save_path));
        if (save_cnt > 0 && save_path[0]) save_dirty = true;
    }

    broadcast_sub_event(srv, s->is_playback ? SUB_EVENT_SINK_INPUT : SUB_EVENT_SOURCE_OUTPUT,
                        SUB_EVENT_CHANGE, s->index);
    pthread_mutex_unlock(&srv->lock);

    if (save_dirty) {
        save_app_volumes_locked_or_snap(&srv->vol_file_lock, save_path, save_keys, save_millis, save_mutes, save_cnt);
    }
    if (srv->wake_efd >= 0) eventfd_write(srv->wake_efd, 1);
    return 0;
}

int lwasapi_pa_server_set_stream_volume(lwasapi_pa_server_t *srv, uint32_t session_id, uint32_t vol_pct) {
    return lwasapi_pa_server_control_stream(srv, session_id, (int32_t)vol_pct, -1, NULL);
}

uint32_t lwasapi_pa_server_get_dominant_rate(lwasapi_pa_server_t *srv, uint32_t *out_bits) {
    if (!srv) {
        if (out_bits) *out_bits = 0U;
        return 0U;
    }
    if (out_bits) {
        *out_bits = atomic_load_explicit(&srv->dom_stream_bits, memory_order_relaxed);
    }
    return atomic_load_explicit(&srv->dom_stream_rate, memory_order_relaxed);
}

void lwasapi_pa_server_fill_debug(lwasapi_pa_server_t *srv, lwasapi_dbg_pa_snapshot_t *out) {
    if (!out) return;
    memset(out, 0, sizeof(*out));
    if (!srv) return;

    pthread_mutex_lock(&srv->lock);

    for (int i = 0; i < PA_MAX_CLIENTS && out->num_clients < LWASAPI_DEBUG_MAX_PA_CLIENTS; i++) {
        pa_client_t *c = &srv->clients[i];
        if (!c->in_use) continue;
        lwasapi_dbg_pa_client_t *dc = &out->clients[out->num_clients++];
        dc->index = c->index;
        dc->pid   = c->pid;
        snprintf(dc->name, sizeof(dc->name), "%s", c->name);
        const char *bin = propbag_get(&c->props, "application.process.binary");
        snprintf(dc->binary, sizeof(dc->binary), "%s", bin ? bin : "");
        dc->version   = c->version;
        dc->authed    = c->authed ? 1U : 0U;
        dc->sub_mask  = c->sub_mask;
        dc->out_count = (uint32_t)c->out_count;
    }

    for (int i = 0; i < PA_MAX_STREAMS && out->num_streams < LWASAPI_DEBUG_MAX_PA_STREAMS; i++) {
        pa_stream_t *s = &srv->streams[i];
        if (!s->in_use) continue;
        lwasapi_dbg_pa_stream_t *ds = &out->streams[out->num_streams++];
        ds->index           = s->index;
        ds->client_index    = s->client ? s->client->index : 0U;
        ds->pid             = s->client ? s->client->pid : 0U;
        snprintf(ds->name, sizeof(ds->name), "%s", s->name);
        snprintf(ds->device_id, sizeof(ds->device_id), "%s", s->device_id);
        ds->is_playback     = s->is_playback ? 1U : 0U;
        ds->is_monitor      = s->is_monitor ? 1U : 0U;
        ds->peak_detect     = s->peak_detect ? 1U : 0U;
        ds->direct_on_input = s->direct_on_input;
        ds->target_dev_idx  = s->target_dev_idx;
        ds->rate            = s->ss.rate;
        ds->channels        = s->ss.channels;
        ds->format          = s->ss.format;
        ds->active          = (uint8_t)atomic_load_explicit(&s->active, memory_order_relaxed);
        ds->corked          = (uint8_t)atomic_load_explicit(&s->corked, memory_order_relaxed);
        ds->in_prebuf       = atomic_load_explicit(&s->in_prebuf, memory_order_relaxed) ? 1U : 0U;
        ds->draining        = (uint8_t)atomic_load_explicit(&s->draining, memory_order_relaxed);
        ds->muted           = (uint8_t)atomic_load_explicit(&s->muted, memory_order_relaxed);
        ds->volume_milli    = atomic_load_explicit(&s->volume_milli, memory_order_relaxed);

        uint32_t fsz = s->frame_size ? s->frame_size : 4U;
        ds->frame_size      = fsz;
        uint64_t wr = atomic_load_explicit(&s->wr_frame, memory_order_relaxed);
        uint64_t rd = atomic_load_explicit(&s->rd_frame, memory_order_relaxed);
        ds->buffered_frames = (wr >= rd) ? (wr - rd) : 0ULL;
        ds->ring_cap_frames = PA_STREAM_RING_FRAMES;
        ds->tlength_frames  = s->tlength / fsz;
        ds->prebuf_frames   = s->prebuf / fsz;
        ds->minreq_frames   = s->minreq / fsz;
        ds->fragsize_frames = s->fragsize / fsz;
        ds->mix_peak_l      = s->mix_peak_l;
        ds->mix_peak_r      = s->mix_peak_r;
        ds->write_index     = atomic_load_explicit(&s->write_index, memory_order_relaxed);
        ds->read_index      = atomic_load_explicit(&s->read_index, memory_order_relaxed);
    }

    pthread_mutex_unlock(&srv->lock);
}

int lwasapi_pa_server_reset_clients(lwasapi_pa_server_t *srv) {
    if (!srv) return 0;
    int flushed = 0;
    pthread_mutex_lock(&srv->lock);
    for (int i = 0; i < PA_MAX_CLIENTS; i++) {
        if (srv->clients[i].in_use) {
            close_pa_client(srv, &srv->clients[i]);
            flushed++;
        }
    }
    pthread_mutex_unlock(&srv->lock);
    if (srv->wake_efd >= 0) {
        uint64_t one = 1u;
        ssize_t wr = write(srv->wake_efd, &one, 8);
        (void)wr;
    }
    return flushed;
}

