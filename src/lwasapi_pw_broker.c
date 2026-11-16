#define _GNU_SOURCE
#include "../include/lwasapi_pw_broker.h"
#include "../include/lwasapi_debug.h"
#include "../include/lwasapi_paths.h"

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/dma-buf.h>
#include <poll.h>
#include <pthread.h>
#include <pwd.h>
#include <signal.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

_Static_assert(offsetof(lwasapi_pw_preview_shm_t, pixels) == 520u,
               "lwasapi_pw_preview_shm_t header must remain 520 bytes");
#include <string.h>
#include <sys/eventfd.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/un.h>
#include <sys/utsname.h>
#include <time.h>
#include <unistd.h>

/* ============================================================================
 * Salsa Built-in Pure-C Video-Only PipeWire-0 ScreenCast Broker (ABI 1.6.x)
 * Zero runtime or compile-time linkage to libpipewire / wireplumber.
 * ============================================================================ */

#define PWB_MAX_CLIENTS       64
#define PWB_MAX_CLIENT_OBJS   256
#define PWB_MAX_MEMS          1024
#define PWB_MAX_PROPS         48
#define PWB_MAX_NODES         32
#define PWB_MAX_LINKS         32
#define PWB_MAX_BUFFERS       16
#define PWB_MAX_PARAMS        48
#define PWB_MAX_VFMT          32
#define PWB_MAX_MODIFIERS     64
#define PWB_ACT_SIZE          2312u
#define PWB_ACT_POS_OFFSET    560u
#define PWB_ACT_CLOCK_SIZE    160u
#define PWB_ACT_POS_SIZE      1688u
#define PWB_IO_BUFFERS_SIZE   16u
#define PWB_META_STRIDE       4096u

/* SPA types & enums (verified against spa-0.2 / pipewire 1.6.11) */
enum {
    PWB_SPA_NONE      = 1,
    PWB_SPA_BOOL      = 2,
    PWB_SPA_ID        = 3,
    PWB_SPA_INT       = 4,
    PWB_SPA_LONG      = 5,
    PWB_SPA_FLOAT     = 6,
    PWB_SPA_DOUBLE    = 7,
    PWB_SPA_STRING    = 8,
    PWB_SPA_BYTES     = 9,
    PWB_SPA_RECT      = 10,
    PWB_SPA_FRAC      = 11,
    PWB_SPA_ARRAY     = 13,
    PWB_SPA_STRUCT    = 14,
    PWB_SPA_OBJECT    = 15,
    PWB_SPA_FD        = 18,
    PWB_SPA_CHOICE    = 19,
};

enum {
    PWB_OBJ_PROP_INFO    = 0x40001,
    PWB_OBJ_PROPS        = 0x40002,
    PWB_OBJ_FORMAT       = 0x40003,
    PWB_OBJ_PARAM_BUFS   = 0x40004,
    PWB_OBJ_PARAM_META   = 0x40005,
    PWB_OBJ_PARAM_IO     = 0x40006,
    PWB_EVT_NODE         = 0x20002,
    PWB_CMD_NODE         = 0x30002,
};

enum {
    PWB_PARAM_EnumFormat = 3,
    PWB_PARAM_Format     = 4,
    PWB_PARAM_Buffers    = 5,
    PWB_PARAM_Meta       = 6,
    PWB_PARAM_IO         = 7,
};

enum {
    PWB_DATA_Invalid = 0,
    PWB_DATA_MemPtr  = 1,
    PWB_DATA_MemFd   = 2,
    PWB_DATA_DmaBuf  = 3,
    PWB_DATA_MemId   = 4,
};

enum {
    PWB_META_Header = 1,
    PWB_META_Busy   = 7,
};

enum {
    PWB_IO_Buffers  = 1,
    PWB_IO_Clock    = 3,
    PWB_IO_Position = 7,
};

enum {
    PWB_STATUS_OK        = 0,
    PWB_STATUS_NEED_DATA = 1 << 0,
    PWB_STATUS_HAVE_DATA = 1 << 1,
};

enum {
    PWB_DIR_IN  = 0,
    PWB_DIR_OUT = 1,
};

enum {
    PWB_CHOICE_None  = 0,
    PWB_CHOICE_Range = 1,
    PWB_CHOICE_Step  = 2,
    PWB_CHOICE_Enum  = 3,
    PWB_CHOICE_Flags = 4,
};

#define PWB_POD_PROP_FLAG_DONT_FIXATE (1u << 4)

enum {
    PWB_MEDIA_Video = 2,
    PWB_SUB_Raw     = 1,
};

enum {
    PWB_VF_UNKNOWN = 0,
    PWB_VF_I420    = 2,
    PWB_VF_YUY2    = 4,
    PWB_VF_RGBx    = 7,
    PWB_VF_BGRx    = 8,
    PWB_VF_xRGB    = 9,
    PWB_VF_xBGR    = 10,
    PWB_VF_RGBA    = 11,
    PWB_VF_BGRA    = 12,
    PWB_VF_ARGB    = 13,
    PWB_VF_ABGR    = 14,
    PWB_VF_RGB     = 15,
    PWB_VF_BGR     = 16,
    PWB_VF_NV12    = 23,
};

/* PipeWire opcodes */
enum {
    /* Core methods & events */
    PW_CORE_M_HELLO         = 1,
    PW_CORE_M_SYNC          = 2,
    PW_CORE_M_PONG          = 3,
    PW_CORE_M_ERROR         = 4,
    PW_CORE_M_GET_REGISTRY  = 5,
    PW_CORE_M_CREATE_OBJ    = 6,
    PW_CORE_M_DESTROY       = 7,

    PW_CORE_E_INFO          = 0,
    PW_CORE_E_DONE          = 1,
    PW_CORE_E_PING          = 2,
    PW_CORE_E_ERROR         = 3,
    PW_CORE_E_REMOVE_ID     = 4,
    PW_CORE_E_BOUND_ID      = 5,
    PW_CORE_E_ADD_MEM       = 6,
    PW_CORE_E_REMOVE_MEM    = 7,
    PW_CORE_E_BOUND_PROPS   = 8,

    /* Registry */
    PW_REG_M_BIND           = 1,
    PW_REG_M_DESTROY        = 2,
    PW_REG_E_GLOBAL         = 0,
    PW_REG_E_GLOBAL_REMOVE  = 1,

    /* Client */
    PW_CLIENT_M_UPDATE_PROPS = 2,
    PW_CLIENT_M_GET_PERMS    = 3,
    PW_CLIENT_M_UPDATE_PERMS = 4,
    PW_CLIENT_E_INFO         = 0,
    PW_CLIENT_E_PERMS        = 1,

    /* Factory / Metadata */
    PW_FACTORY_E_INFO        = 0,
    PW_META_M_SET_PROPERTY   = 1,
    PW_META_E_PROPERTY       = 0,

    /* Node */
    PW_NODE_M_SUBSCRIBE     = 1,
    PW_NODE_M_ENUM_PARAMS   = 2,
    PW_NODE_E_INFO          = 0,
    PW_NODE_E_PARAM         = 1,

    /* Port */
    PW_PORT_M_SUBSCRIBE     = 1,
    PW_PORT_M_ENUM_PARAMS   = 2,
    PW_PORT_E_INFO          = 0,
    PW_PORT_E_PARAM         = 1,

    /* Link */
    PW_LINK_E_INFO          = 0,

    /* ClientNode */
    PW_CN_M_GET_NODE        = 1,
    PW_CN_M_UPDATE          = 2,
    PW_CN_M_PORT_UPDATE     = 3,
    PW_CN_M_SET_ACTIVE      = 4,
    PW_CN_M_EVENT           = 5,
    PW_CN_M_PORT_BUFFERS    = 6,

    PW_CN_E_TRANSPORT       = 0,
    PW_CN_E_SET_PARAM       = 1,
    PW_CN_E_SET_IO          = 2,
    PW_CN_E_EVENT           = 3,
    PW_CN_E_COMMAND         = 4,
    PW_CN_E_ADD_PORT        = 5,
    PW_CN_E_REMOVE_PORT     = 6,
    PW_CN_E_PORT_SET_PARAM  = 7,
    PW_CN_E_PORT_USE_BUFS   = 8,
    PW_CN_E_PORT_SET_IO     = 9,
    PW_CN_E_SET_ACTIVATION  = 10,
    PW_CN_E_PORT_SET_MIX_INFO = 11,
};

#define PWB_PERM_ALL         0x1c8u
#define PWB_SPA_ID_INVALID   0xffffffffu
#define PWB_DRM_MOD_INVALID  0x00ffffffffffffffULL
#define PWB_SPA_ASYNC_BIT    0x40000000u

/* PipeWire node activation state (exact 2312B layout verified against 1.6.11) */
typedef struct {
    int32_t status;
    int32_t required;
    int32_t pending;
} pwb_act_state_t;

typedef struct {
    uint8_t raw[PWB_ACT_SIZE];
} pwb_activation_mem_t;

typedef struct {
    int32_t  status;
    uint32_t buffer_id;
} pwb_io_buffers_t;

typedef struct {
    uint32_t offset;
    uint32_t size;
    int32_t  stride;
    int32_t  flags;
} pwb_spa_chunk_t;

typedef struct {
    uint32_t width;
    uint32_t height;
} pwb_rect_t;

typedef struct {
    uint32_t num;
    uint32_t denom;
} pwb_frac_t;

typedef struct {
    char key[64];
    char val[160];
} pwb_kv_t;

typedef struct {
    pwb_kv_t items[PWB_MAX_PROPS];
    uint32_t count;
} pwb_dict_t;

typedef struct {
    uint32_t id;
    uint32_t flags;
} pwb_param_info_t;

typedef struct {
    uint32_t id;
    uint8_t *pod;
    uint32_t size;
} pwb_raw_param_t;

typedef struct {
    uint32_t formats[32];
    uint32_t n_formats;
    uint64_t modifiers[PWB_MAX_MODIFIERS];
    uint32_t n_modifiers;
    bool     has_modifier;
    bool     mod_dont_fixate;
    uint32_t w_def, w_min, w_max;
    uint32_t h_def, h_min, h_max;
    pwb_frac_t fps_def, fps_min, fps_max;
    pwb_frac_t max_fps_def, max_fps_min, max_fps_max;
    bool     has_fps;
    bool     has_max_fps;
    bool     valid;
} pwb_vfmt_caps_t;

typedef struct {
    uint32_t buf_def, buf_min, buf_max;
    uint32_t size_def, size_min, size_max;
    uint32_t stride_def;
    uint32_t datatype_mask;
    bool     valid;
} pwb_buf_caps_t;

typedef struct {
    int      fd;
    uint32_t mem_type; /* MemFd or DmaBuf */
    uint32_t flags;
    uint32_t mapoffset;
    uint32_t maxsize;
    void    *mapped;   /* for preview reading if MemFd or mappable linear */
    size_t   mapped_len;
} pwb_node_buf_t;

struct pwb_client;

typedef struct {
    bool     in_use;
    uint32_t proxy_id;
    uint32_t global_id;
    uint8_t  kind;     /* 1=Registry, 2=Client, 3=ClientNode, 4=Node, 5=Port, 6=Link, 7=Factory, 8=Metadata */
    uint32_t target_node_id;
    uint32_t port_dir;
    uint32_t sub_node_params[16];
    uint32_t n_sub_node;
} pwb_proxy_t;

typedef struct {
    bool              in_use;
    uint32_t          global_id;
    uint32_t          serial;
    uint32_t          client_idx;
    uint32_t          cn_proxy_id;
    uint32_t          node_proxy_id; /* GetNode proxy */
    uint32_t          port_global_id;
    bool              port_registered;
    uint8_t           direction;     /* PWB_DIR_OUT (1) or PWB_DIR_IN (0) */
    bool              is_driver;
    bool              alloc_buffers;
    bool              autoconnect;
    bool              active;
    uint8_t           state;         /* PW_NODE_STATE_* */
    pwb_dict_t        node_props;
    pwb_dict_t        port_props;
    char              name[64];
    char              media_class[48];
    char              target_str[64];
    uint32_t          target_id_req;

    /* Raw parameter storage for EnumParams queries */
    pwb_raw_param_t   node_params[PWB_MAX_PARAMS];
    uint32_t          n_node_params;
    pwb_raw_param_t   port_params[PWB_MAX_PARAMS];
    uint32_t          n_port_params;

    /* Parsed video + buffers capability */
    pwb_vfmt_caps_t   vfmt_candidates[PWB_MAX_VFMT];
    uint32_t          n_vfmt;
    pwb_buf_caps_t    bcaps;

    /* Negotiated stream state */
    uint32_t          neg_format;
    uint32_t          neg_w, neg_h, neg_stride;
    uint32_t          neg_fps_num, neg_fps_denom;
    uint64_t          neg_modifier;
    bool              neg_has_modifier;
    bool              neg_is_dmabuf;

    /* Shared Activation MemFd + EventFDs */
    int               act_mem_fd;
    uint32_t          act_mem_id;
    uint8_t          *act_map;
    int               act_readfd;    /* Broker -> Client trigger */
    int               act_writefd;

    /* Broker-mediated Target EventFD (Client writes here when done!) */
    int               done_efd;
    int               done_act_fd;
    uint32_t          done_act_memid;
    uint8_t          *done_act_map;

    /* Port Buffers & IO */
    int               meta_mem_fd;
    uint32_t          meta_mem_id;
    uint8_t          *meta_map;
    size_t            meta_map_len;
    int               io_mem_fd;
    uint32_t          io_mem_id;
    pwb_io_buffers_t *io_map;

    pwb_node_buf_t    bufs[PWB_MAX_BUFFERS];
    uint32_t          n_bufs;
    uint32_t          buf_mem_ids[PWB_MAX_BUFFERS];
    bool              destroying;
    bool              awaiting_port_buffers;
    bool              mix_ready;
    uint32_t          pending_link_idx;
    uint64_t          frames_count;
    uint64_t          relay_next_due_ns;
    uint32_t          relay_applied_fps;
} pwb_node_t;

typedef struct {
    bool     in_use;
    uint32_t global_id;
    uint32_t serial;
    uint32_t out_node_idx;
    uint32_t in_node_idx;
    uint8_t  state; /* PW_LINK_STATE_* */
    uint8_t  format_pod[2048];
    uint32_t format_pod_sz;
    uint32_t owner_client_idx; /* if created explicitly via link-factory */
    uint32_t owner_proxy_id;
    uint64_t frames_relayed;
    bool     buffers_ready;
    bool     io_ready;
    bool     awaiting_mod_fixation;
    bool     mod_fixated;
    uint32_t last_buf_id;
} pwb_link_t;

typedef struct pwb_client {
    bool        in_use;
    bool        disconnecting;
    int         fd;
    uint32_t    client_idx;
    uint32_t    global_id;
    uint32_t    serial;
    uint32_t    pid;
    uint32_t    uid;
    uint32_t    gid;
    uint32_t    tx_seq;
    uint32_t    next_mem_id;
    uint32_t    mem_free_head;
    uint32_t    mem_free_next[PWB_MAX_MEMS];
    bool        hello_done;
    bool        is_portal;
    char        app_name[64];
    char        binary[64];
    char        portal_app_id[64];
    pwb_dict_t  props;

    uint8_t    *rx_buf;
    size_t      rx_len;
    size_t      rx_cap;
    int         rx_fds[256];
    uint32_t    rx_nfds;

    pwb_proxy_t objs[PWB_MAX_CLIENT_OBJS];
} pwb_client_t;

typedef struct {
    char subject[32];
    char key[64];
    char type[32];
    char val[160];
} pwb_meta_entry_t;

struct lwasapi_pw_broker {
    char             runtime_dir[256];
    char             socket_path[384];
    char             manager_socket_path[384];
    char             preview_shm_name[128];
    bool             ignore_zero_flag;
    atomic_bool      running;
    atomic_bool      zero_pw_mode;
    atomic_bool      broker_listening;

    int              listen_fd;
    int              mgr_fd;
    int              wake_efd;

    pthread_t        thread;
    pthread_mutex_t  lock;

    uint32_t         next_global_id;
    uint32_t         next_serial;
    uint64_t         generation;

    pwb_client_t     clients[PWB_MAX_CLIENTS];
    pwb_node_t       nodes[PWB_MAX_NODES];
    pwb_link_t       links[PWB_MAX_LINKS];
    pwb_meta_entry_t metas[16];
    uint32_t         n_metas;

    int              preview_shm_fd;
    lwasapi_pw_preview_shm_t *preview_shm;
    atomic_uint_fast64_t preview_expiry_ns;
    atomic_uint_fast32_t preview_target_w;
    atomic_uint_fast32_t preview_target_h;
    atomic_uint_fast32_t preview_target_fps;
    atomic_uint_fast32_t preview_actual_fps_x100_pub;
    atomic_uint_fast32_t cast_limit_w;
    atomic_uint_fast32_t cast_limit_h;
    atomic_uint_fast32_t cast_limit_fps;
    uint64_t             preview_next_due_ns;
    uint64_t             preview_fps_win_ns;
    uint32_t             preview_fps_win_cnt;
    uint32_t             preview_actual_fps_x100;
    uint32_t             preview_applied_fps;
    atomic_uint_fast64_t frames_relayed_total;
    atomic_uint_fast64_t frames_dropped;
    atomic_uint_fast32_t audio_rejects;
};

static inline uint64_t pwb_mono_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static inline size_t pwb_pad8(size_t n) {
    return (n + 7u) & ~(size_t)7u;
}

static void pwb_safe_copy(char *dst, size_t dst_sz, const char *src) {
    if (!dst || dst_sz == 0) return;
    if (!src) { dst[0] = '\0'; return; }
    size_t len = strlen(src);
    if (len >= dst_sz) len = dst_sz - 1u;
    if (len > 0) memcpy(dst, src, len);
    dst[len] = '\0';
}

/* ============================================================================
 * Minimal Self-Contained SPA POD Builder & Reader
 * ============================================================================ */

typedef struct {
    uint8_t *buf;
    size_t   cap;
    size_t   len;
    size_t   frame_off[12];
    uint32_t frame_hdr_sz[12];
    int      depth;
    bool     overflow;
} pwb_pod_b_t;

static void pb_init(pwb_pod_b_t *b, uint8_t *buf, size_t cap) {
    b->buf = buf;
    b->cap = cap;
    b->len = 0;
    b->depth = 0;
    b->overflow = false;
}

static void pb_write(pwb_pod_b_t *b, const void *src, size_t n) {
    if (b->overflow || b->len + n > b->cap) { b->overflow = true; return; }
    if (n > 0 && src) memcpy(b->buf + b->len, src, n);
    b->len += n;
}

static void pb_pad(pwb_pod_b_t *b) {
    size_t p = pwb_pad8(b->len);
    if (b->overflow || p > b->cap) { b->overflow = true; return; }
    if (p > b->len) memset(b->buf + b->len, 0, p - b->len);
    b->len = p;
}

static void pb_hdr(pwb_pod_b_t *b, uint32_t sz, uint32_t tp) {
    uint32_t h[2] = { sz, tp };
    pb_write(b, h, 8);
}

static void pb_none(pwb_pod_b_t *b) {
    pb_hdr(b, 0, PWB_SPA_NONE);
}

static void __attribute__((unused)) pb_bool(pwb_pod_b_t *b, bool v) {
    uint32_t iv = v ? 1u : 0u;
    pb_hdr(b, 4, PWB_SPA_BOOL);
    pb_write(b, &iv, 4);
    pb_pad(b);
}

static void pb_id(pwb_pod_b_t *b, uint32_t v) {
    pb_hdr(b, 4, PWB_SPA_ID);
    pb_write(b, &v, 4);
    pb_pad(b);
}

static void pb_int(pwb_pod_b_t *b, int32_t v) {
    pb_hdr(b, 4, PWB_SPA_INT);
    pb_write(b, &v, 4);
    pb_pad(b);
}

static void pb_long(pwb_pod_b_t *b, int64_t v) {
    pb_hdr(b, 8, PWB_SPA_LONG);
    pb_write(b, &v, 8);
}

static void pb_fd(pwb_pod_b_t *b, int64_t idx) {
    pb_hdr(b, 8, PWB_SPA_FD);
    pb_write(b, &idx, 8);
}

static void pb_rect(pwb_pod_b_t *b, uint32_t w, uint32_t h) {
    uint32_t r[2] = { w, h };
    pb_hdr(b, 8, PWB_SPA_RECT);
    pb_write(b, r, 8);
}

static void pb_frac(pwb_pod_b_t *b, uint32_t num, uint32_t den) {
    uint32_t f[2] = { num, den ? den : 1u };
    pb_hdr(b, 8, PWB_SPA_FRAC);
    pb_write(b, f, 8);
}

static void pb_str(pwb_pod_b_t *b, const char *s) {
    if (!s) { pb_none(b); return; }
    size_t sl = strlen(s) + 1u;
    pb_hdr(b, (uint32_t)sl, PWB_SPA_STRING);
    pb_write(b, s, sl);
    pb_pad(b);
}

static void pb_raw_pod(pwb_pod_b_t *b, const uint8_t *pod, uint32_t pod_sz) {
    if (!pod || pod_sz < 8) { pb_none(b); return; }
    pb_write(b, pod, pod_sz);
    pb_pad(b);
}

static void pb_push_struct(pwb_pod_b_t *b) {
    if (b->depth >= 12) { b->overflow = true; return; }
    b->frame_off[b->depth] = b->len;
    b->frame_hdr_sz[b->depth] = 8u;
    b->depth++;
    pb_hdr(b, 0, PWB_SPA_STRUCT);
}

static void pb_push_object(pwb_pod_b_t *b, uint32_t obj_type, uint32_t obj_id) {
    if (b->depth >= 12) { b->overflow = true; return; }
    b->frame_off[b->depth] = b->len;
    b->frame_hdr_sz[b->depth] = 8u;
    b->depth++;
    pb_hdr(b, 8, PWB_SPA_OBJECT);
    uint32_t ot[2] = { obj_type, obj_id };
    pb_write(b, ot, 8);
}

static void pb_push_choice(pwb_pod_b_t *b, uint32_t ctype, uint32_t cflags,
                           uint32_t esz, uint32_t etp) {
    if (b->depth >= 12) { b->overflow = true; return; }
    b->frame_off[b->depth] = b->len;
    b->frame_hdr_sz[b->depth] = 8u;
    b->depth++;
    pb_hdr(b, 16, PWB_SPA_CHOICE);
    uint32_t ch[4] = { ctype, cflags, esz, etp };
    pb_write(b, ch, 16);
}

static void pb_prop(pwb_pod_b_t *b, uint32_t key, uint32_t flags) {
    uint32_t kf[2] = { key, flags };
    pb_write(b, kf, 8);
}

static void pb_pop(pwb_pod_b_t *b) {
    if (b->depth <= 0) { b->overflow = true; return; }
    b->depth--;
    size_t foff = b->frame_off[b->depth];
    if (!b->overflow && b->len >= foff + b->frame_hdr_sz[b->depth]) {
        uint32_t sz = (uint32_t)(b->len - (foff + b->frame_hdr_sz[b->depth]));
        memcpy(b->buf + foff, &sz, 4);
    }
    pb_pad(b);
}

static void pb_dict(pwb_pod_b_t *b, const pwb_dict_t *d) {
    pb_push_struct(b);
    uint32_t cnt = d ? d->count : 0u;
    pb_int(b, (int32_t)cnt);
    for (uint32_t i = 0; i < cnt; i++) {
        pb_str(b, d->items[i].key);
        const char *v = d->items[i].val;
        if (strncmp(v, "pointer:", 8) == 0) v = "";
        pb_str(b, v);
    }
    pb_pop(b);
}

static void pb_param_info(pwb_pod_b_t *b, const pwb_param_info_t *pi, uint32_t n) {
    pb_push_struct(b);
    pb_int(b, (int32_t)n);
    for (uint32_t i = 0; i < n; i++) {
        pb_id(b, pi[i].id);
        pb_int(b, (int32_t)pi[i].flags);
    }
    pb_pop(b);
}

/* Dictionary helper */
static const char *dict_get(const pwb_dict_t *d, const char *key) {
    if (!d || !key) return NULL;
    for (uint32_t i = 0; i < d->count; i++) {
        if (strcmp(d->items[i].key, key) == 0) return d->items[i].val;
    }
    return NULL;
}

static void dict_set(pwb_dict_t *d, const char *key, const char *val) {
    if (!d || !key || !key[0]) return;
    if (!val) val = "";
    for (uint32_t i = 0; i < d->count; i++) {
        if (strcmp(d->items[i].key, key) == 0) {
            pwb_safe_copy(d->items[i].val, sizeof(d->items[i].val), val);
            return;
        }
    }
    if (d->count < PWB_MAX_PROPS) {
        pwb_safe_copy(d->items[d->count].key, sizeof(d->items[d->count].key), key);
        pwb_safe_copy(d->items[d->count].val, sizeof(d->items[d->count].val), val);
        d->count++;
    }
}

/* POD Reader */
typedef struct {
    const uint8_t *base;
    size_t         off;
    size_t         end;
} pwb_cur_t;

static void cur_init(pwb_cur_t *c, const void *data, size_t len) {
    c->base = (const uint8_t *)data;
    c->off = 0;
    c->end = len;
}

static bool cur_next_pod(pwb_cur_t *c, uint32_t *out_sz, uint32_t *out_tp,
                         const uint8_t **out_body, size_t *out_pod_total,
                         const uint8_t **out_pod_ptr) {
    if (c->off + 8 > c->end) return false;
    uint32_t sz = 0, tp = 0;
    memcpy(&sz, c->base + c->off, 4);
    memcpy(&tp, c->base + c->off + 4, 4);
    size_t total = pwb_pad8(8u + (size_t)sz);
    if (c->off + 8u + (size_t)sz > c->end) return false;
    if (c->off + total > c->end) total = 8u + (size_t)sz;
    if (out_sz) *out_sz = sz;
    if (out_tp) *out_tp = tp;
    if (out_body) *out_body = c->base + c->off + 8u;
    if (out_pod_ptr) *out_pod_ptr = c->base + c->off;
    if (out_pod_total) *out_pod_total = 8u + (size_t)sz;
    c->off += total;
    return true;
}

static bool cur_enter_struct(pwb_cur_t *c, pwb_cur_t *inner) {
    uint32_t sz = 0, tp = 0;
    const uint8_t *body = NULL;
    if (!cur_next_pod(c, &sz, &tp, &body, NULL, NULL)) return false;
    if (tp != PWB_SPA_STRUCT) return false;
    inner->base = body;
    inner->off = 0;
    inner->end = sz;
    return true;
}

static bool cur_int(pwb_cur_t *c, int32_t *out) {
    uint32_t sz = 0, tp = 0;
    const uint8_t *body = NULL;
    if (!cur_next_pod(c, &sz, &tp, &body, NULL, NULL)) return false;
    if ((tp != PWB_SPA_INT && tp != PWB_SPA_ID && tp != PWB_SPA_BOOL) || sz < 4) return false;
    memcpy(out, body, 4);
    return true;
}

static bool cur_u32(pwb_cur_t *c, uint32_t *out) {
    int32_t v = 0;
    if (!cur_int(c, &v)) return false;
    *out = (uint32_t)v;
    return true;
}

static bool cur_long(pwb_cur_t *c, int64_t *out) {
    uint32_t sz = 0, tp = 0;
    const uint8_t *body = NULL;
    if (!cur_next_pod(c, &sz, &tp, &body, NULL, NULL)) return false;
    if ((tp != PWB_SPA_LONG && tp != PWB_SPA_FD) || sz < 8) return false;
    memcpy(out, body, 8);
    return true;
}

static bool cur_str(pwb_cur_t *c, char *out, size_t out_sz) {
    uint32_t sz = 0, tp = 0;
    const uint8_t *body = NULL;
    if (!cur_next_pod(c, &sz, &tp, &body, NULL, NULL)) return false;
    if (tp == PWB_SPA_NONE) { if (out_sz) out[0] = '\0'; return true; }
    if (tp != PWB_SPA_STRING) return false;
    size_t n = strnlen((const char *)body, sz);
    if (out_sz > 0) {
        if (n >= out_sz) n = out_sz - 1u;
        memcpy(out, body, n);
        out[n] = '\0';
    }
    return true;
}

static bool cur_dict(pwb_cur_t *c, pwb_dict_t *d, bool wrapped_in_struct) {
    pwb_cur_t dc;
    pwb_cur_t *p = c;
    if (wrapped_in_struct) {
        if (!cur_enter_struct(c, &dc)) return false;
        p = &dc;
    }
    int32_t n = 0;
    if (!cur_int(p, &n) || n < 0 || n > 1024) return false;
    for (int32_t i = 0; i < n; i++) {
        char k[64], v[160];
        if (!cur_str(p, k, sizeof(k)) || !cur_str(p, v, sizeof(v))) return false;
        dict_set(d, k, v);
    }
    return true;
}

/* ============================================================================
 * Video Format / Param Parsing & Intersection Helpers
 * ============================================================================ */

static const char *pwb_vfmt_name(uint32_t f) {
    switch (f) {
        case PWB_VF_BGRx: return "BGRx";
        case PWB_VF_BGRA: return "BGRA";
        case PWB_VF_RGBx: return "RGBx";
        case PWB_VF_RGBA: return "RGBA";
        case PWB_VF_xRGB: return "xRGB";
        case PWB_VF_xBGR: return "xBGR";
        case PWB_VF_ARGB: return "ARGB";
        case PWB_VF_ABGR: return "ABGR";
        case PWB_VF_RGB:  return "RGB";
        case PWB_VF_BGR:  return "BGR";
        case PWB_VF_YUY2: return "YUY2";
        case PWB_VF_NV12: return "NV12";
        case PWB_VF_I420: return "I420";
        default:          return "RAW";
    }
}

static uint32_t pwb_vfmt_bpp_num(uint32_t f) {
    switch (f) {
        case PWB_VF_BGRx: case PWB_VF_BGRA: case PWB_VF_RGBx: case PWB_VF_RGBA:
        case PWB_VF_xRGB: case PWB_VF_xBGR: case PWB_VF_ARGB: case PWB_VF_ABGR: return 4u;
        case PWB_VF_RGB: case PWB_VF_BGR: return 3u;
        case PWB_VF_YUY2: return 2u;
        case PWB_VF_NV12: case PWB_VF_I420: return 3u; /* /2 */
        default: return 4u;
    }
}

static uint32_t pwb_vfmt_stride(uint32_t f, uint32_t w) {
    if (f == PWB_VF_NV12 || f == PWB_VF_I420) return w;
    return w * pwb_vfmt_bpp_num(f);
}

static uint32_t pwb_vfmt_size(uint32_t f, uint32_t w, uint32_t h) {
    if (f == PWB_VF_NV12 || f == PWB_VF_I420) return (w * h * 3u) / 2u;
    return pwb_vfmt_stride(f, w) * h;
}

static void parse_choice_u32(uint32_t tp, const uint8_t *body, uint32_t sz,
                             uint32_t *arr, uint32_t max_arr, uint32_t *out_cnt,
                             uint32_t *out_def, uint32_t *out_min, uint32_t *out_max,
                             bool *out_is_range) {
    if (tp == PWB_SPA_ID || tp == PWB_SPA_INT) {
        if (sz >= 4) {
            uint32_t v = 0; memcpy(&v, body, 4);
            if (out_def) *out_def = v;
            if (out_min) *out_min = v;
            if (out_max) *out_max = v;
            if (arr && max_arr > 0) { arr[0] = v; if (out_cnt) *out_cnt = 1; }
        }
        return;
    }
    if (tp == PWB_SPA_CHOICE && sz >= 16) {
        uint32_t ctype = 0, cflags = 0, esz = 0, etp = 0;
        memcpy(&ctype, body, 4);
        memcpy(&cflags, body + 4, 4);
        memcpy(&esz, body + 8, 4);
        memcpy(&etp, body + 12, 4);
        (void)cflags;
        if ((etp == PWB_SPA_ID || etp == PWB_SPA_INT) && esz == 4) {
            uint32_t n = (sz - 16u) / 4u;
            if (n > 0) {
                uint32_t v0 = 0; memcpy(&v0, body + 16, 4);
                if (out_def) *out_def = v0;
                if (ctype == PWB_CHOICE_Range && n >= 3) {
                    uint32_t vmin = 0, vmax = 0;
                    memcpy(&vmin, body + 20, 4);
                    memcpy(&vmax, body + 24, 4);
                    if (out_min) *out_min = vmin;
                    if (out_max) *out_max = vmax;
                    if (out_is_range) *out_is_range = true;
                } else {
                    if (out_min) *out_min = v0;
                    if (out_max) *out_max = v0;
                    uint32_t c = 0;
                    uint32_t start = (n > 1) ? 1u : 0u; /* In PipeWire Enum/None, [0] is default, [1..] are enum or same */
                    if (arr) {
                        arr[c++] = v0;
                        for (uint32_t i = start; i < n && c < max_arr; i++) {
                            uint32_t vi = 0; memcpy(&vi, body + 16 + i * 4u, 4);
                            bool dup = false;
                            for (uint32_t k = 0; k < c; k++) if (arr[k] == vi) { dup = true; break; }
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
    if (tp == PWB_SPA_LONG && sz >= 8) {
        if (arr && max_arr > 0) {
            memcpy(&arr[0], body, 8);
            if (out_cnt) *out_cnt = 1;
        }
        return;
    }
    if (tp == PWB_SPA_CHOICE && sz >= 16) {
        uint32_t esz = 0, etp = 0;
        memcpy(&esz, body + 8, 4);
        memcpy(&etp, body + 12, 4);
        if (etp == PWB_SPA_LONG && esz == 8) {
            uint32_t n = (sz - 16u) / 8u;
            uint32_t c = 0;
            for (uint32_t i = 0; i < n && arr && c < max_arr; i++) {
                uint64_t vi = 0; memcpy(&vi, body + 16 + i * 8u, 8);
                bool dup = false;
                for (uint32_t k = 0; k < c; k++) if (arr[k] == vi) { dup = true; break; }
                if (!dup) arr[c++] = vi;
            }
            if (out_cnt) *out_cnt = c;
        }
    }
}

static void parse_choice_rect(uint32_t tp, const uint8_t *body, uint32_t sz,
                              uint32_t *wd, uint32_t *wmin, uint32_t *wmax,
                              uint32_t *hd, uint32_t *hmin, uint32_t *hmax) {
    if (tp == PWB_SPA_RECT && sz >= 8) {
        pwb_rect_t r; memcpy(&r, body, 8);
        *wd = *wmin = *wmax = r.width;
        *hd = *hmin = *hmax = r.height;
        return;
    }
    if (tp == PWB_SPA_CHOICE && sz >= 16) {
        uint32_t ctype = 0, esz = 0, etp = 0;
        memcpy(&ctype, body, 4);
        memcpy(&esz, body + 8, 4);
        memcpy(&etp, body + 12, 4);
        if (etp == PWB_SPA_RECT && esz == 8) {
            uint32_t n = (sz - 16u) / 8u;
            if (n >= 1) {
                pwb_rect_t r0; memcpy(&r0, body + 16, 8);
                *wd = r0.width; *hd = r0.height;
                *wmin = r0.width; *wmax = r0.width;
                *hmin = r0.height; *hmax = r0.height;
                if (ctype == PWB_CHOICE_Range && n >= 3) {
                    pwb_rect_t r1, r2;
                    memcpy(&r1, body + 24, 8);
                    memcpy(&r2, body + 32, 8);
                    *wmin = r1.width; *wmax = r2.width;
                    *hmin = r1.height; *hmax = r2.height;
                } else if (ctype == PWB_CHOICE_Enum && n > 1) {
                    for (uint32_t i = 1; i < n; i++) {
                        pwb_rect_t ri; memcpy(&ri, body + 16 + i * 8u, 8);
                        if (ri.width < *wmin) *wmin = ri.width;
                        if (ri.width > *wmax) *wmax = ri.width;
                        if (ri.height < *hmin) *hmin = ri.height;
                        if (ri.height > *hmax) *hmax = ri.height;
                    }
                }
            }
        }
    }
}

static void parse_choice_frac(uint32_t tp, const uint8_t *body, uint32_t sz,
                              pwb_frac_t *fdef, pwb_frac_t *fmin, pwb_frac_t *fmax) {
    if (tp == PWB_SPA_FRAC && sz >= 8) {
        pwb_frac_t f; memcpy(&f, body, 8);
        if (f.denom == 0) f.denom = 1;
        *fdef = *fmin = *fmax = f;
        return;
    }
    if (tp == PWB_SPA_CHOICE && sz >= 16) {
        uint32_t ctype = 0, esz = 0, etp = 0;
        memcpy(&ctype, body, 4);
        memcpy(&esz, body + 8, 4);
        memcpy(&etp, body + 12, 4);
        if (etp == PWB_SPA_FRAC && esz == 8) {
            uint32_t n = (sz - 16u) / 8u;
            if (n >= 1) {
                pwb_frac_t f0; memcpy(&f0, body + 16, 8);
                if (f0.denom == 0) f0.denom = 1;
                *fdef = *fmin = *fmax = f0;
                if (ctype == PWB_CHOICE_Range && n >= 3) {
                    memcpy(fmin, body + 24, 8);
                    memcpy(fmax, body + 32, 8);
                    if (fmin->denom == 0) fmin->denom = 1;
                    if (fmax->denom == 0) fmax->denom = 1;
                    if (((uint64_t)fmin->num * 1000ULL) / (uint64_t)fmin->denom >
                        ((uint64_t)fmax->num * 1000ULL) / (uint64_t)fmax->denom) {
                        pwb_frac_t tmp = *fmin;
                        *fmin = *fmax;
                        *fmax = tmp;
                    }
                } else if (ctype == PWB_CHOICE_Enum && n > 1) {
                    for (uint32_t i = 1; i < n; i++) {
                        pwb_frac_t fi;
                        memcpy(&fi, body + 16 + i * 8u, 8);
                        if (fi.denom == 0) fi.denom = 1;
                        uint64_t fm = ((uint64_t)fi.num * 1000ULL) / (uint64_t)fi.denom;
                        uint64_t min_m = ((uint64_t)fmin->num * 1000ULL) / (uint64_t)fmin->denom;
                        uint64_t max_m = ((uint64_t)fmax->num * 1000ULL) / (uint64_t)fmax->denom;
                        if (fm < min_m) *fmin = fi;
                        if (fm > max_m) *fmax = fi;
                    }
                }
            }
        }
    }
}

static void parse_param_pod(pwb_node_t *nd, const uint8_t *pod, uint32_t pod_sz) {
    if (!nd || !pod || pod_sz < 16) return;
    uint32_t sz = 0, tp = 0;
    memcpy(&sz, pod, 4);
    memcpy(&tp, pod + 4, 4);
    if (tp != PWB_SPA_OBJECT || 8u + sz > pod_sz) return;
    uint32_t otype = 0, oid = 0;
    memcpy(&otype, pod + 8, 4);
    memcpy(&oid, pod + 12, 4);

    if (otype == PWB_OBJ_FORMAT && (oid == PWB_PARAM_EnumFormat || oid == PWB_PARAM_Format)) {
        pwb_vfmt_caps_t c = {0};
        c.w_def = 1920; c.w_min = 1; c.w_max = 16384;
        c.h_def = 1080; c.h_min = 1; c.h_max = 16384;
        c.fps_def = (pwb_frac_t){0, 1};
        c.fps_min = (pwb_frac_t){0, 1};
        c.fps_max = (pwb_frac_t){0, 1};
        c.max_fps_def = (pwb_frac_t){60, 1};
        c.max_fps_min = (pwb_frac_t){1, 1};
        c.max_fps_max = (pwb_frac_t){240, 1};
        uint32_t mtype = 0, msub = 0;
        pwb_cur_t p;
        cur_init(&p, pod + 16, sz - 8);
        while (p.off + 8 <= p.end) {
            uint32_t k = 0, fl = 0;
            memcpy(&k, p.base + p.off, 4);
            memcpy(&fl, p.base + p.off + 4, 4);
            p.off += 8;
            uint32_t vsz = 0, vtp = 0;
            const uint8_t *vbody = NULL;
            if (!cur_next_pod(&p, &vsz, &vtp, &vbody, NULL, NULL)) break;
            if (k == 1) { /* mediaType */
                parse_choice_u32(vtp, vbody, vsz, NULL, 0, NULL, &mtype, NULL, NULL, NULL);
            } else if (k == 2) { /* mediaSubtype */
                parse_choice_u32(vtp, vbody, vsz, NULL, 0, NULL, &msub, NULL, NULL, NULL);
            } else if (k == 0x20001) { /* VIDEO_format */
                parse_choice_u32(vtp, vbody, vsz, c.formats, 32, &c.n_formats, NULL, NULL, NULL, NULL);
            } else if (k == 0x20002) { /* VIDEO_modifier */
                parse_choice_long(vtp, vbody, vsz, c.modifiers, PWB_MAX_MODIFIERS, &c.n_modifiers);
                c.has_modifier = (c.n_modifiers > 0);
                c.mod_dont_fixate = c.has_modifier && ((fl & PWB_POD_PROP_FLAG_DONT_FIXATE) != 0);
            } else if (k == 0x20003) { /* VIDEO_size */
                parse_choice_rect(vtp, vbody, vsz, &c.w_def, &c.w_min, &c.w_max, &c.h_def, &c.h_min, &c.h_max);
            } else if (k == 0x20004) { /* VIDEO_framerate */
                parse_choice_frac(vtp, vbody, vsz, &c.fps_def, &c.fps_min, &c.fps_max);
                c.has_fps = true;
            } else if (k == 0x20005) { /* VIDEO_maxFramerate */
                parse_choice_frac(vtp, vbody, vsz, &c.max_fps_def, &c.max_fps_min, &c.max_fps_max);
                c.has_max_fps = true;
            }
        }
        if ((mtype == 0 || mtype == PWB_MEDIA_Video) && (msub == 0 || msub == PWB_SUB_Raw)) {
            if (c.n_formats == 0) {
                c.formats[0] = PWB_VF_BGRx;
                c.formats[1] = PWB_VF_BGRA;
                c.n_formats = 2;
            }
            c.valid = true;
            if (nd->n_vfmt < PWB_MAX_VFMT) nd->vfmt_candidates[nd->n_vfmt++] = c;
        }
    } else if (otype == PWB_OBJ_PARAM_BUFS && oid == PWB_PARAM_Buffers) {
        pwb_buf_caps_t bc = {0};
        bc.buf_def = 3; bc.buf_min = 2; bc.buf_max = 8;
        bc.datatype_mask = (1u << PWB_DATA_MemFd);
        pwb_cur_t p;
        cur_init(&p, pod + 16, sz - 8);
        while (p.off + 8 <= p.end) {
            uint32_t k = 0;
            memcpy(&k, p.base + p.off, 4);
            p.off += 8;
            uint32_t vsz = 0, vtp = 0;
            const uint8_t *vbody = NULL;
            if (!cur_next_pod(&p, &vsz, &vtp, &vbody, NULL, NULL)) break;
            if (k == 1) parse_choice_u32(vtp, vbody, vsz, NULL, 0, NULL, &bc.buf_def, &bc.buf_min, &bc.buf_max, NULL);
            else if (k == 3) parse_choice_u32(vtp, vbody, vsz, NULL, 0, NULL, &bc.size_def, &bc.size_min, &bc.size_max, NULL);
            else if (k == 4) parse_choice_u32(vtp, vbody, vsz, NULL, 0, NULL, &bc.stride_def, NULL, NULL, NULL);
            else if (k == 6) parse_choice_u32(vtp, vbody, vsz, NULL, 0, NULL, &bc.datatype_mask, NULL, NULL, NULL);
        }
        bc.valid = true;
        nd->bcaps = bc;
    }
}

static void store_raw_param(pwb_raw_param_t *arr, uint32_t *cnt, uint32_t id, const uint8_t *pod, uint32_t sz) {
    if (!pod || sz < 8) return;
    if (id == PWB_PARAM_EnumFormat) {
        /* Append new EnumFormat entries; cleared only when client calls empty/update start cleanly if needed */
    }
    if (*cnt >= PWB_MAX_PARAMS) {
        free(arr[0].pod);
        memmove(&arr[0], &arr[1], (PWB_MAX_PARAMS - 1u) * sizeof(arr[0]));
        (*cnt)--;
    }
    uint8_t *copy = (uint8_t *)malloc(sz);
    if (!copy) return;
    memcpy(copy, pod, sz);
    arr[*cnt].id = id;
    arr[*cnt].pod = copy;
    arr[*cnt].size = sz;
    (*cnt)++;
}

static void clear_raw_params(pwb_raw_param_t *arr, uint32_t *cnt) {
    for (uint32_t i = 0; i < *cnt; i++) {
        free(arr[i].pod);
        arr[i].pod = NULL;
        arr[i].size = 0;
    }
    *cnt = 0;
}

/* ============================================================================
 * Wire Send Helpers (Unix Socket + SCM_RIGHTS)
 * ============================================================================ */

static int pwb_send_msg(pwb_client_t *c, uint32_t obj_id, uint8_t op,
                        const void *payload, uint32_t len,
                        const int *fds, uint32_t nfds, uint32_t *out_seq) {
    if (!c || !c->in_use || c->fd < 0) return -1;
    uint32_t seq = c->tx_seq++;
    if (out_seq) *out_seq = seq;

    size_t pad_len = pwb_pad8(len);
    size_t total = 16u + pad_len;
    uint8_t *pkt = (uint8_t *)malloc(total);
    if (!pkt) return -1;
    uint32_t hdr[4] = {
        obj_id,
        (((uint32_t)op & 0xffu) << 24) | (len & 0x00ffffffu),
        seq,
        nfds
    };
    memcpy(pkt, hdr, 16);
    if (len > 0 && payload) memcpy(pkt + 16, payload, len);
    if (pad_len > len) memset(pkt + 16 + len, 0, pad_len - len);

    struct msghdr msg = {0};
    struct iovec iov = { .iov_base = pkt, .iov_len = total };
    msg.msg_iov = &iov;
    msg.msg_iovlen = 1;

    uint8_t cbuf[CMSG_SPACE(sizeof(int) * 32)];
    if (nfds > 0 && nfds <= 32 && fds) {
        memset(cbuf, 0, sizeof(cbuf));
        msg.msg_control = cbuf;
        msg.msg_controllen = CMSG_SPACE(sizeof(int) * nfds);
        struct cmsghdr *cm = CMSG_FIRSTHDR(&msg);
        cm->cmsg_level = SOL_SOCKET;
        cm->cmsg_type  = SCM_RIGHTS;
        cm->cmsg_len   = CMSG_LEN(sizeof(int) * nfds);
        memcpy(CMSG_DATA(cm), fds, sizeof(int) * nfds);
    }

    size_t sent = 0;
    int tries = 0;
    while (sent < total) {
        iov.iov_base = pkt + sent;
        iov.iov_len = total - sent;
        ssize_t n = sendmsg(c->fd, &msg, MSG_NOSIGNAL);
        if (n < 0) {
            if (errno == EINTR) continue;
            if ((errno == EAGAIN || errno == EWOULDBLOCK) && tries++ < 40) {
                struct pollfd pfd = { .fd = c->fd, .events = POLLOUT };
                poll(&pfd, 1, 5);
                continue;
            }
            free(pkt);
            return -1;
        }
        sent += (size_t)n;
        /* Only pass SCM_RIGHTS on the very first write chunk */
        msg.msg_control = NULL;
        msg.msg_controllen = 0;
    }

    free(pkt);
    return 0;
}

static int pwb_add_mem(pwb_client_t *c, uint32_t mem_type, int fd, uint32_t flags, uint32_t *out_memid) {
    if (!c || !c->in_use || c->disconnecting || fd < 0) return -1;
    uint32_t mid;
    bool from_free = (c->mem_free_head != PWB_SPA_ID_INVALID);
    if (from_free) {
        mid = c->mem_free_head;
    } else {
        if (c->next_mem_id >= PWB_MAX_MEMS) return -1;
        mid = c->next_mem_id;
    }
    uint8_t buf[128];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)mid);
    pb_id(&b, mem_type);
    pb_fd(&b, 0);
    pb_int(&b, (int32_t)flags);
    pb_pop(&b);
    int fds[1] = { fd };
    if (pwb_send_msg(c, 0, PW_CORE_E_ADD_MEM, buf, (uint32_t)b.len, fds, 1, NULL) < 0)
        return -1;
    if (from_free) {
        c->mem_free_head = (mid < PWB_MAX_MEMS) ? c->mem_free_next[mid] : PWB_SPA_ID_INVALID;
    } else {
        c->next_mem_id++;
    }
    if (out_memid) *out_memid = mid;
    return 0;
}

static void pwb_remove_mem(pwb_client_t *c, uint32_t *mem_id) {
    if (!mem_id || *mem_id == PWB_SPA_ID_INVALID) return;
    uint32_t mid = *mem_id;
    *mem_id = PWB_SPA_ID_INVALID;
    if (!c || !c->in_use || c->disconnecting || c->fd < 0) return;
    uint8_t buf[64];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)mid);
    pb_pop(&b);
    if (pwb_send_msg(c, 0, PW_CORE_E_REMOVE_MEM, buf, (uint32_t)b.len, NULL, 0, NULL) == 0 &&
        mid < PWB_MAX_MEMS) {
        c->mem_free_next[mid] = c->mem_free_head;
        c->mem_free_head = mid;
    }
}

static void pwb_remove_node_buf_mems(pwb_client_t *c, pwb_node_t *nd) {
    if (!nd) return;
    for (uint32_t i = 0; i < PWB_MAX_BUFFERS; i++) {
        pwb_remove_mem(c, &nd->buf_mem_ids[i]);
    }
}

static void pwb_send_remove_id(pwb_client_t *c, uint32_t obj_id) {
    if (!c || !c->in_use || c->disconnecting || c->fd < 0 || obj_id == 0) return;
    uint8_t buf[64];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)obj_id);
    pb_pop(&b);
    pwb_send_msg(c, 0, PW_CORE_E_REMOVE_ID, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void init_node_mem_ids(pwb_node_t *nd) {
    nd->act_mem_id = PWB_SPA_ID_INVALID;
    nd->done_act_memid = PWB_SPA_ID_INVALID;
    nd->meta_mem_id = PWB_SPA_ID_INVALID;
    nd->io_mem_id = PWB_SPA_ID_INVALID;
    for (uint32_t k = 0; k < PWB_MAX_BUFFERS; k++) {
        nd->buf_mem_ids[k] = PWB_SPA_ID_INVALID;
    }
}

static pwb_client_t *find_client_by_idx(struct lwasapi_pw_broker *brk, uint32_t idx) {
    if (idx >= PWB_MAX_CLIENTS) return NULL;
    return brk->clients[idx].in_use ? &brk->clients[idx] : NULL;
}

static pwb_node_t *find_node_by_global(struct lwasapi_pw_broker *brk, uint32_t gid, uint32_t *out_idx) {
    for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
        if (brk->nodes[i].in_use && brk->nodes[i].global_id == gid) {
            if (out_idx) *out_idx = i;
            return &brk->nodes[i];
        }
    }
    return NULL;
}

static bool parse_u32_strict(const char *s, uint32_t *out) {
    if (!s || s[0] < '0' || s[0] > '9') return false;
    char *ep = NULL;
    unsigned long v = strtoul(s, &ep, 10);
    if (!ep || *ep != '\0' || v >= (unsigned long)PWB_SPA_ID_INVALID) return false;
    *out = (uint32_t)v;
    return (v > 0);
}

static pwb_node_t *find_node_by_ref(struct lwasapi_pw_broker *brk, const char *ref,
                                    uint8_t want_dir, uint32_t *out_idx) {
    if (!ref || !ref[0]) return NULL;
    uint32_t id = 0;
    bool is_num = parse_u32_strict(ref, &id);
    for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
        pwb_node_t *nd = &brk->nodes[i];
        if (!nd->in_use) continue;
        if (is_num &&
            (nd->global_id == id || nd->serial == id ||
             (nd->port_registered && nd->port_global_id == id && nd->direction == want_dir))) {
            if (out_idx) *out_idx = i;
            return nd;
        }
        if (strcmp(nd->name, ref) == 0 && nd->direction == want_dir) {
            if (out_idx) *out_idx = i;
            return nd;
        }
    }
    return NULL;
}

static void apply_node_target_props(pwb_node_t *nd) {
    const char *ac = dict_get(&nd->node_props, "node.autoconnect");
    if (ac && ac[0]) nd->autoconnect = (strcmp(ac, "true") == 0);
    const char *t = dict_get(&nd->node_props, "target.object");
    if (!t || !t[0]) t = dict_get(&nd->node_props, "node.target");
    if (t && t[0] && strcmp(t, "-1") != 0) {
        pwb_safe_copy(nd->target_str, sizeof(nd->target_str), t);
        uint32_t tid = 0;
        nd->target_id_req = parse_u32_strict(t, &tid) ? tid : 0u;
    } else if (t && strcmp(t, "-1") == 0) {
        nd->target_str[0] = '\0';
        nd->target_id_req = 0u;
    }
}

static const char * __attribute__((unused)) node_state_name(uint8_t st) {
    switch (st) {
        case 0: return "error";
        case 1: return "creating";
        case 2: return "suspended";
        case 3: return "idle";
        case 4: return "running";
        default: return "unknown";
    }
}

/* ============================================================================
 * Protocol Emitters (Registry, Core, Client, Factory, Metadata, Node, Port, Link)
 * ============================================================================ */

static void build_factory_dict(int idx, const char **out_name, const char **out_type,
                               uint32_t *out_ver, pwb_dict_t *d) {
    memset(d, 0, sizeof(*d));
    if (idx == 1) {
        *out_name = "client-node";
        *out_type = "PipeWire:Interface:ClientNode";
        *out_ver = 6;
        dict_set(d, "factory.name", "client-node");
        dict_set(d, "factory.type.name", "PipeWire:Interface:ClientNode");
        dict_set(d, "factory.type.version", "6");
        dict_set(d, "module.id", "2");
    } else if (idx == 2) {
        *out_name = "link-factory";
        *out_type = "PipeWire:Interface:Link";
        *out_ver = 3;
        dict_set(d, "factory.name", "link-factory");
        dict_set(d, "factory.type.name", "PipeWire:Interface:Link");
        dict_set(d, "factory.type.version", "3");
        dict_set(d, "module.id", "2");
    } else if (idx == 3) {
        *out_name = "adapter";
        *out_type = "PipeWire:Interface:Node";
        *out_ver = 3;
        dict_set(d, "factory.name", "adapter");
        dict_set(d, "factory.type.name", "PipeWire:Interface:Node");
        dict_set(d, "factory.type.version", "3");
        dict_set(d, "module.id", "2");
    } else {
        *out_name = "metadata";
        *out_type = "PipeWire:Interface:Metadata";
        *out_ver = 3;
        dict_set(d, "factory.name", "metadata");
        dict_set(d, "factory.type.name", "PipeWire:Interface:Metadata");
        dict_set(d, "factory.type.version", "3");
        dict_set(d, "module.id", "2");
    }
}

static void build_metadata_dict(uint32_t gid, pwb_dict_t *d) {
    memset(d, 0, sizeof(*d));
    dict_set(d, "metadata.name", (gid == 5) ? "default" : "settings");
    dict_set(d, "object.serial", (gid == 5) ? "5" : "6");
}

static void build_node_public_props(const pwb_node_t *nd, pwb_dict_t *d) {
    *d = nd->node_props;
    char tmp[32];
    snprintf(tmp, sizeof(tmp), "%u", nd->serial);
    dict_set(d, "object.serial", tmp);
    dict_set(d, "object.id", (snprintf(tmp, sizeof(tmp), "%u", nd->global_id), tmp));
    dict_set(d, "factory.id", "1");
    if (!dict_get(d, "node.name")) dict_set(d, "node.name", nd->name);
    if (!dict_get(d, "media.class")) dict_set(d, "media.class", nd->media_class);
    if (!dict_get(d, "media.type")) dict_set(d, "media.type", "Video");
    if (!dict_get(d, "media.category")) dict_set(d, "media.category", "Capture");
}

static void emit_registry_global(pwb_client_t *c, uint32_t reg_proxy_id,
                                 uint32_t gid, uint32_t perms, const char *type,
                                 uint32_t ver, const pwb_dict_t *props) {
    uint8_t buf[4096];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)gid);
    pb_int(&b, (int32_t)perms);
    pb_str(&b, type);
    pb_int(&b, (int32_t)ver);
    pb_dict(&b, props);
    pb_pop(&b);
    if (!b.overflow)
        pwb_send_msg(c, reg_proxy_id, PW_REG_E_GLOBAL, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void broadcast_global(struct lwasapi_pw_broker *brk, uint32_t gid,
                             const char *type, uint32_t ver, const pwb_dict_t *props) {
    for (uint32_t ci = 0; ci < PWB_MAX_CLIENTS; ci++) {
        pwb_client_t *c = &brk->clients[ci];
        if (!c->in_use) continue;
        for (uint32_t oi = 0; oi < PWB_MAX_CLIENT_OBJS; oi++) {
            if (c->objs[oi].in_use && c->objs[oi].kind == 1) {
                emit_registry_global(c, c->objs[oi].proxy_id, gid, PWB_PERM_ALL, type, ver, props);
            }
        }
    }
}

static void broadcast_global_remove(struct lwasapi_pw_broker *brk, uint32_t gid) {
    uint8_t buf[64];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)gid);
    pb_pop(&b);
    for (uint32_t ci = 0; ci < PWB_MAX_CLIENTS; ci++) {
        pwb_client_t *c = &brk->clients[ci];
        if (!c->in_use) continue;
        for (uint32_t oi = 0; oi < PWB_MAX_CLIENT_OBJS; oi++) {
            if (c->objs[oi].in_use && c->objs[oi].kind == 1) {
                pwb_send_msg(c, c->objs[oi].proxy_id, PW_REG_E_GLOBAL_REMOVE, buf, (uint32_t)b.len, NULL, 0, NULL);
            }
        }
    }
}

static void emit_node_info(pwb_client_t *c, uint32_t proxy_id, const pwb_node_t *nd) {
    uint8_t buf[4096];
    pwb_pod_b_t b;
    pwb_dict_t props;
    build_node_public_props(nd, &props);

    pwb_param_info_t pis[6];
    uint32_t npi = 0;
    if (nd->n_vfmt > 0) pis[npi++] = (pwb_param_info_t){ PWB_PARAM_EnumFormat, 2 };
    if (nd->neg_w > 0)  pis[npi++] = (pwb_param_info_t){ PWB_PARAM_Format, 6 };

    /* pw_node_state enum: creating=0, suspended=1, idle=2, running=3 */
    uint32_t pw_st = (nd->state >= 1) ? (uint32_t)(nd->state - 1) : 1u;

    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)nd->global_id);
    pb_int(&b, (nd->direction == PWB_DIR_IN) ? 1 : 0);
    pb_int(&b, (nd->direction == PWB_DIR_OUT) ? 1 : 0);
    pb_long(&b, 31); /* PROPS|INPUT_PORTS|OUTPUT_PORTS|STATE|PARAMS */
    pb_int(&b, (nd->direction == PWB_DIR_IN && nd->port_registered) ? 1 : 0);
    pb_int(&b, (nd->direction == PWB_DIR_OUT && nd->port_registered) ? 1 : 0);
    pb_id(&b, pw_st);
    pb_none(&b); /* error NULL */
    pb_dict(&b, &props);
    pb_param_info(&b, pis, npi);
    pb_pop(&b);
    if (!b.overflow)
        pwb_send_msg(c, proxy_id, PW_NODE_E_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void emit_port_info(pwb_client_t *c, uint32_t proxy_id, const pwb_node_t *nd) {
    uint8_t buf[2048];
    pwb_pod_b_t b;
    pwb_param_info_t pis[4];
    uint32_t npi = 0;
    if (nd->n_port_params > 0) {
        pis[npi++] = (pwb_param_info_t){ PWB_PARAM_EnumFormat, 2 };
        if (nd->neg_w > 0) pis[npi++] = (pwb_param_info_t){ PWB_PARAM_Format, 6 };
    }
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)nd->port_global_id);
    pb_int(&b, (int32_t)nd->direction);
    pb_long(&b, 3); /* PROPS | PARAMS */
    pb_dict(&b, &nd->port_props);
    pb_param_info(&b, pis, npi);
    pb_pop(&b);
    if (!b.overflow)
        pwb_send_msg(c, proxy_id, PW_PORT_E_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void emit_link_info(pwb_client_t *c, uint32_t proxy_id, const struct lwasapi_pw_broker *brk,
                           const pwb_link_t *lk) {
    const pwb_node_t *on = &brk->nodes[lk->out_node_idx];
    const pwb_node_t *in = &brk->nodes[lk->in_node_idx];
    uint8_t buf[2048];
    pwb_pod_b_t b;
    pwb_dict_t d = {0};
    char tmp[32];
    dict_set(&d, "object.serial", (snprintf(tmp, sizeof(tmp), "%u", lk->serial), tmp));
    dict_set(&d, "link.output.node", (snprintf(tmp, sizeof(tmp), "%u", on->global_id), tmp));
    dict_set(&d, "link.output.port", (snprintf(tmp, sizeof(tmp), "%u", on->port_global_id), tmp));
    dict_set(&d, "link.input.node", (snprintf(tmp, sizeof(tmp), "%u", in->global_id), tmp));
    dict_set(&d, "link.input.port", (snprintf(tmp, sizeof(tmp), "%u", in->port_global_id), tmp));

    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)lk->global_id);
    pb_int(&b, (int32_t)on->global_id);
    pb_int(&b, (int32_t)on->port_global_id);
    pb_int(&b, (int32_t)in->global_id);
    pb_int(&b, (int32_t)in->port_global_id);
    pb_long(&b, 7); /* STATE | FORMAT | PROPS */
    pb_int(&b, (int32_t)lk->state);
    pb_none(&b); /* error NULL */
    if (lk->format_pod_sz >= 8) pb_raw_pod(&b, lk->format_pod, lk->format_pod_sz);
    else pb_none(&b);
    pb_dict(&b, &d);
    pb_pop(&b);
    if (!b.overflow)
        pwb_send_msg(c, proxy_id, PW_LINK_E_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void emit_metadata_prop(pwb_client_t *c, uint32_t proxy_id, uint32_t subj,
                               const char *key, const char *type, const char *val) {
    uint8_t buf[512]; pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)subj);
    if (key && key[0]) pb_str(&b, key); else pb_none(&b);
    if (type && type[0]) pb_str(&b, type); else pb_none(&b);
    if (val && val[0]) pb_str(&b, val); else pb_none(&b);
    pb_pop(&b);
    if (!b.overflow)
        pwb_send_msg(c, proxy_id, PW_META_E_PROPERTY, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void broadcast_node_updates(struct lwasapi_pw_broker *brk, const pwb_node_t *nd) {
    for (uint32_t ci = 0; ci < PWB_MAX_CLIENTS; ci++) {
        pwb_client_t *c = &brk->clients[ci];
        if (!c->in_use) continue;
        for (uint32_t oi = 0; oi < PWB_MAX_CLIENT_OBJS; oi++) {
            pwb_proxy_t *px = &c->objs[oi];
            if (!px->in_use) continue;
            if (px->kind == 4 && px->global_id == nd->global_id) {
                emit_node_info(c, px->proxy_id, nd);
            } else if (px->kind == 5 && px->global_id == nd->port_global_id) {
                emit_port_info(c, px->proxy_id, nd);
            }
        }
    }
}

/* ============================================================================
 * Negotiation, Buffer Allocation & Graph Activation Relay
 * ============================================================================ */

static void pwb_update_activation_pos(pwb_node_t *nd, uint32_t driver_gid) {
    if (!nd->act_map) return;
    uint32_t w = nd->neg_w ? nd->neg_w : 1920u;
    uint32_t h = nd->neg_h ? nd->neg_h : 1080u;
    uint32_t stride = nd->neg_stride ? nd->neg_stride : (w * 4u);
    uint32_t fn = nd->neg_fps_num ? nd->neg_fps_num : 60u;
    uint32_t fd = nd->neg_fps_denom ? nd->neg_fps_denom : 1u;

    /* server_version @ 544, active_driver_id @ 548, driver_id @ 552, flags @ 556 */
    uint32_t one = 1u;
    memcpy(nd->act_map + 544, &one, 4);
    memcpy(nd->act_map + 548, &driver_gid, 4);
    memcpy(nd->act_map + 552, &driver_gid, 4);

    /* spa_io_position @ 560: clock.id @ +4, clock.name[64] @ +8, clock.rate @ +80,
     * clock.duration @ +96, video.flags @ +160, video.stride @ +164, video.size @ +168,
     * video.framerate @ +176, state @ +208 (2 = RUNNING), n_segments @ +212 */
    uint32_t clk_id = nd->is_driver ? nd->global_id : driver_gid;
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 4, &clk_id, 4);
    char cname[64];
    memset(cname, 0, sizeof(cname));
    snprintf(cname, sizeof(cname), "salsa-pw-%u", clk_id);
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 8, cname, sizeof(cname));
    pwb_frac_t clk_rate = { 1u, 48000u };
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 80, &clk_rate, 8);
    uint64_t clk_dur = 1600u;
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 96, &clk_dur, 8);
    uint32_t vflags = 1u; /* SPA_IO_VIDEO_SIZE_VALID */
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 160, &vflags, 4);
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 164, &stride, 4);
    pwb_rect_t vr = { w, h };
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 168, &vr, 8);
    pwb_frac_t vf = { fn, fd };
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 176, &vf, 8);
    uint32_t pos_state = 2u; /* SPA_IO_POSITION_STATE_RUNNING */
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 208, &pos_state, 4);
    uint32_t nseg = 1u;
    memcpy(nd->act_map + PWB_ACT_POS_OFFSET + 212, &nseg, 4);
}

static inline uint64_t pwb_frac_mhz(pwb_frac_t f) {
    if (f.denom == 0u) return 0ULL;
    return ((uint64_t)f.num * 1000ULL) / (uint64_t)f.denom;
}

static bool pwb_pace_due(uint64_t now, uint64_t *next_due_ns, uint32_t fps) {
    if (fps == 0u) {
        *next_due_ns = 0ULL;
        return true;
    }
    uint64_t interval = 1000000000ULL / (uint64_t)fps;
    uint64_t tol = interval / 8u;
    if (tol > 2000000ULL) tol = 2000000ULL;
    if (*next_due_ns == 0ULL) {
        *next_due_ns = now + interval;
        return true;
    }
    if (now + tol < *next_due_ns) return false;
    if (now > *next_due_ns + interval * 2ULL) {
        *next_due_ns = now + interval;
    } else {
        *next_due_ns += interval;
    }
    return true;
}

static void pwb_update_fps_meter(uint64_t now, uint64_t *win_ns, uint32_t *win_cnt, uint32_t *out_x100) {
    if (*win_ns == 0ULL) {
        *win_ns = now;
        *win_cnt = 1u;
        return;
    }
    (*win_cnt)++;
    uint64_t elapsed = (now > *win_ns) ? (now - *win_ns) : 0ULL;
    if (elapsed >= 500000000ULL) {
        uint64_t x100 = ((uint64_t)(*win_cnt - 1u) * 100000000000ULL) / elapsed;
        if (x100 > 999999ULL) x100 = 999999ULL;
        *out_x100 = (uint32_t)x100;
        *win_ns = now;
        *win_cnt = 1u;
    }
}

static pwb_frac_t sanitize_non_zero_fps(pwb_frac_t f) {
    if (f.num == 0 || f.denom == 0) return (pwb_frac_t){ 60u, 1u };
    uint64_t mhz = ((uint64_t)f.num * 1000ULL) / (uint64_t)f.denom;
    if (mhz == 0ULL) return (pwb_frac_t){ 1u, 1u };
    if (mhz > 1000000ULL) return (pwb_frac_t){ 1000u, 1u };
    if ((uint64_t)f.num * 1000ULL > 0xffffffffULL) {
        return (pwb_frac_t){ (uint32_t)mhz, 1000u };
    }
    return f;
}

static void extract_vfmt_fps_caps(const pwb_vfmt_caps_t *v,
                                  uint64_t *out_min_mhz, uint64_t *out_max_mhz,
                                  pwb_frac_t *out_min_frac, pwb_frac_t *out_max_frac,
                                  bool *out_flex, pwb_frac_t *out_pref_frac) {
    uint64_t min_mhz = 1000ULL;
    uint64_t max_mhz = 1000000ULL;
    pwb_frac_t min_f = { 1u, 1u };
    pwb_frac_t max_f = { 1000u, 1u };
    pwb_frac_t pref_f = { 0u, 1u };
    bool flex = true;

    if (v->has_fps) {
        uint64_t def_m = pwb_frac_mhz(v->fps_def);
        uint64_t fmin_m = pwb_frac_mhz(v->fps_min);
        uint64_t fmax_m = pwb_frac_mhz(v->fps_max);
        if (v->fps_def.num == 0u && fmax_m < 1000ULL) {
            /* Variable framerate (0/1); constraints come from maxFramerate if present */
            flex = true;
        } else if (fmin_m < fmax_m) {
            flex = true;
            if (fmin_m >= 1000ULL) { min_mhz = fmin_m; min_f = v->fps_min; }
            if (fmax_m >= 1000ULL && fmax_m < max_mhz) { max_mhz = fmax_m; max_f = v->fps_max; }
            if (def_m >= 1000ULL) pref_f = v->fps_def;
        } else if (def_m >= 1000ULL) {
            flex = false;
            min_mhz = max_mhz = def_m;
            min_f = max_f = pref_f = v->fps_def;
        }
    }

    if (v->has_max_fps) {
        uint64_t mdef_m = pwb_frac_mhz(v->max_fps_def);
        uint64_t mmin_m = pwb_frac_mhz(v->max_fps_min);
        uint64_t mmax_m = pwb_frac_mhz(v->max_fps_max);
        if (mmax_m >= 1000ULL && mmax_m < max_mhz) {
            max_mhz = mmax_m;
            max_f = v->max_fps_max;
        }
        if (mmin_m >= 1000ULL && mmin_m > min_mhz && (!v->has_fps || v->fps_def.num == 0u)) {
            min_mhz = mmin_m;
            min_f = v->max_fps_min;
        }
        if (mmin_m < mmax_m) {
            flex = true;
        } else if (!v->has_fps && mdef_m >= 1000ULL) {
            flex = false;
            min_mhz = max_mhz = mdef_m;
            min_f = max_f = v->max_fps_def;
        }
        if (pref_f.num == 0u && mdef_m >= 1000ULL) {
            pref_f = v->max_fps_def;
        }
    }

    if (min_mhz > max_mhz) {
        min_mhz = max_mhz;
        min_f = max_f;
    }
    *out_min_mhz = min_mhz;
    *out_max_mhz = max_mhz;
    *out_min_frac = min_f;
    *out_max_frac = max_f;
    *out_flex = flex;
    *out_pref_frac = pref_f;
}

static bool intersect_vfmt(const pwb_vfmt_caps_t *a, const pwb_vfmt_caps_t *b,
                           bool want_mod, uint32_t lim_w, uint32_t lim_h, uint32_t lim_fps,
                           uint32_t *out_fmt, uint32_t *out_w, uint32_t *out_h,
                           uint32_t *out_fn, uint32_t *out_fd,
                           uint64_t *out_mods, uint32_t *out_n_mods) {
    if (!a->valid || !b->valid) return false;
    if (want_mod && (!a->has_modifier || !b->has_modifier)) return false;
    if (!want_mod && (a->has_modifier && b->has_modifier)) return false;
    /* Wait: if producer only sent with modifier and consumer omitted modifier, disallow want_mod==false matching modifier-only unless fallback */

    /* Intersect format IDs */
    uint32_t chosen_fmt = 0;
    for (uint32_t i = 0; i < a->n_formats && !chosen_fmt; i++) {
        for (uint32_t j = 0; j < b->n_formats; j++) {
            if (a->formats[i] == b->formats[j]) {
                chosen_fmt = a->formats[i];
                break;
            }
        }
    }
    if (!chosen_fmt) return false;

    /* Intersect rectangle */
    uint32_t minw = (a->w_min > b->w_min) ? a->w_min : b->w_min;
    uint32_t maxw = (a->w_max < b->w_max) ? a->w_max : b->w_max;
    uint32_t minh = (a->h_min > b->h_min) ? a->h_min : b->h_min;
    uint32_t maxh = (a->h_max < b->h_max) ? a->h_max : b->h_max;
    if (minw > maxw || minh > maxh) return false;
    uint32_t w = a->w_def;
    if (w < minw) w = minw;
    if (w > maxw) w = maxw;
    uint32_t h = a->h_def;
    if (h < minh) h = minh;
    if (h > maxh) h = maxh;
    if (lim_w > 0u && lim_h > 0u && (w > lim_w || h > lim_h) && (minw < maxw || minh < maxh)) {
        uint32_t nw = w, nh = h;
        if (nw > lim_w) { nh = (uint32_t)(((uint64_t)nh * lim_w) / nw); nw = lim_w; }
        if (nh > lim_h) { nw = (uint32_t)(((uint64_t)nw * lim_h) / nh); nh = lim_h; }
        if (nw >= minw && nw <= maxw && nh >= minh && nh <= maxh && nw > 0u && nh > 0u) {
            w = nw;
            h = nh;
        }
    }

    /* Intersect all mutual modifiers ordered: explicit > linear (0) > INVALID */
    uint32_t n_mods = 0;
    if (want_mod) {
        if (!out_mods || !out_n_mods) return false;
        for (int tier = 0; tier < 3; tier++) {
            for (uint32_t i = 0; i < a->n_modifiers && n_mods < PWB_MAX_MODIFIERS; i++) {
                uint64_t m = a->modifiers[i];
                if (tier == 0 && (m == 0ULL || m == PWB_DRM_MOD_INVALID)) continue;
                if (tier == 1 && m != 0ULL) continue;
                if (tier == 2 && m != PWB_DRM_MOD_INVALID) continue;
                bool in_b = false;
                for (uint32_t j = 0; j < b->n_modifiers; j++) {
                    if (b->modifiers[j] == m) { in_b = true; break; }
                }
                if (!in_b) continue;
                bool dup = false;
                for (uint32_t k = 0; k < n_mods; k++) {
                    if (out_mods[k] == m) { dup = true; break; }
                }
                if (!dup) out_mods[n_mods++] = m;
            }
        }
        if (n_mods == 0) return false;
    } else if (out_mods) {
        out_mods[0] = PWB_DRM_MOD_INVALID;
    }

    uint64_t a_min_mhz = 1000ULL, a_max_mhz = 1000000ULL;
    uint64_t b_min_mhz = 1000ULL, b_max_mhz = 1000000ULL;
    pwb_frac_t a_min_f, a_max_f, a_pref_f, b_min_f, b_max_f, b_pref_f;
    bool a_flex = true, b_flex = true;
    extract_vfmt_fps_caps(a, &a_min_mhz, &a_max_mhz, &a_min_f, &a_max_f, &a_flex, &a_pref_f);
    extract_vfmt_fps_caps(b, &b_min_mhz, &b_max_mhz, &b_min_f, &b_max_f, &b_flex, &b_pref_f);

    uint64_t lo_mhz = (a_min_mhz > b_min_mhz) ? a_min_mhz : b_min_mhz;
    uint64_t hi_mhz = (a_max_mhz < b_max_mhz) ? a_max_mhz : b_max_mhz;
    pwb_frac_t lo_f = (a_min_mhz >= b_min_mhz) ? a_min_f : b_min_f;
    pwb_frac_t hi_f = (a_max_mhz <= b_max_mhz) ? a_max_f : b_max_f;
    if (lo_mhz > hi_mhz) {
        lo_mhz = a_min_mhz; lo_f = a_min_f;
        hi_mhz = a_max_mhz; hi_f = a_max_f;
    }

    /* Pick a sensible default without letting a consumer's dummy range default (e.g. 25/1
     * in [0..1000]) drag down a variable-rate producer below 60 fps. */
    pwb_frac_t chosen_fps = (pwb_frac_t){ 60u, 1u };
    if (!a_flex && pwb_frac_mhz(a_pref_f) >= 1000ULL) {
        chosen_fps = a_pref_f;
    } else if (!b_flex && pwb_frac_mhz(b_pref_f) >= 1000ULL) {
        chosen_fps = b_pref_f;
    } else if (pwb_frac_mhz(a_pref_f) >= 1000ULL) {
        chosen_fps = a_pref_f;
    }

    uint64_t ch_mhz = pwb_frac_mhz(chosen_fps);
    if (ch_mhz < lo_mhz) chosen_fps = lo_f;
    else if (ch_mhz > hi_mhz) chosen_fps = hi_f;

    if (a_flex && lim_fps > 0u &&
        (uint64_t)lim_fps * 1000ULL < pwb_frac_mhz(chosen_fps) &&
        (uint64_t)lim_fps * 1000ULL >= a_min_mhz) {
        chosen_fps = (pwb_frac_t){ lim_fps, 1u };
    }
    chosen_fps = sanitize_non_zero_fps(chosen_fps);

    *out_fmt = chosen_fmt;
    *out_w = w;
    *out_h = h;
    *out_fn = chosen_fps.num;
    *out_fd = chosen_fps.denom;
    if (out_n_mods) *out_n_mods = n_mods;
    return true;
}

static size_t build_negotiated_format_pod(uint8_t *dst, size_t cap,
                                          uint32_t fmt, uint32_t w, uint32_t h,
                                          uint32_t fn, uint32_t fd,
                                          bool has_mod, bool dont_fixate,
                                          const uint64_t *mods, uint32_t n_mods) {
    pwb_frac_t maxf = sanitize_non_zero_fps((pwb_frac_t){ fn, fd });
    pwb_pod_b_t b;
    pb_init(&b, dst, cap);
    pb_push_object(&b, PWB_OBJ_FORMAT, PWB_PARAM_Format);
    pb_prop(&b, 1, 0); pb_id(&b, PWB_MEDIA_Video);
    pb_prop(&b, 2, 0); pb_id(&b, PWB_SUB_Raw);
    pb_prop(&b, 0x20001, 0); pb_id(&b, fmt);
    if (has_mod && mods && n_mods > 0) {
        pb_prop(&b, 0x20002, dont_fixate ? PWB_POD_PROP_FLAG_DONT_FIXATE : 0u);
        if (dont_fixate) {
            pb_push_choice(&b, PWB_CHOICE_Enum, 0, 8u, PWB_SPA_LONG);
            uint64_t defm = mods[0];
            pb_write(&b, &defm, 8);
            for (uint32_t i = 0; i < n_mods; i++) {
                uint64_t vi = mods[i];
                pb_write(&b, &vi, 8);
            }
            pb_pop(&b);
        } else {
            pb_push_choice(&b, PWB_CHOICE_None, 0, 8u, PWB_SPA_LONG);
            uint64_t m0 = mods[0];
            pb_write(&b, &m0, 8);
            pb_pop(&b);
        }
    }
    pb_prop(&b, 0x20003, 0); pb_rect(&b, w, h);
    pb_prop(&b, 0x20004, 0); pb_frac(&b, fn, fd);
    /* KWin ScreenCastStream::onStreamParamChanged leaves m_videoFormat.max_framerate
     * uninitialized when 0x20005 is omitted, which can return 0 from framerate() and
     * cause SIGFPE in KWin::VsyncSource::arm(). Always populate valid maxFramerate. */
    pb_prop(&b, 0x20005, 0); pb_frac(&b, maxf.num, maxf.denom);
    pb_pop(&b);
    return b.overflow ? 0u : b.len;
}

static void cn_send_port_set_param(pwb_client_t *c, uint32_t cn_proxy_id,
                                   uint32_t dir, uint32_t port_id, uint32_t param_id,
                                   const uint8_t *pod, uint32_t pod_sz) {
    uint8_t buf[4096];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)dir);
    pb_int(&b, (int32_t)port_id);
    pb_id(&b, param_id);
    pb_int(&b, 0);
    if (pod && pod_sz >= 8) pb_raw_pod(&b, pod, pod_sz);
    else pb_none(&b);
    pb_pop(&b);
    if (!b.overflow)
        pwb_send_msg(c, cn_proxy_id, PW_CN_E_PORT_SET_PARAM, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void cn_send_command(pwb_client_t *c, uint32_t cn_proxy_id, uint32_t cmd_id) {
    uint8_t buf[128];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_push_object(&b, PWB_CMD_NODE, cmd_id);
    pb_pop(&b);
    pb_pop(&b);
    pwb_send_msg(c, cn_proxy_id, PW_CN_E_COMMAND, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void cn_send_set_activation(pwb_client_t *c, uint32_t cn_proxy_id,
                                   uint32_t node_id, int signalfd, uint32_t memid,
                                   uint32_t off, uint32_t sz) {
    uint8_t buf[160];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)node_id);
    pb_fd(&b, (signalfd >= 0) ? 0 : -1);
    pb_int(&b, (int32_t)memid);
    pb_int(&b, (int32_t)off);
    pb_int(&b, (int32_t)sz);
    pb_pop(&b);
    if (signalfd >= 0) {
        int fds[1] = { signalfd };
        pwb_send_msg(c, cn_proxy_id, PW_CN_E_SET_ACTIVATION, buf, (uint32_t)b.len, fds, 1, NULL);
    } else {
        pwb_send_msg(c, cn_proxy_id, PW_CN_E_SET_ACTIVATION, buf, (uint32_t)b.len, NULL, 0, NULL);
    }
}

static void cn_send_port_set_io(pwb_client_t *c, uint32_t cn_proxy_id,
                                uint32_t dir, uint32_t port_id, uint32_t mix_id,
                                uint32_t io_id, uint32_t memid, uint32_t off, uint32_t sz) {
    uint8_t buf[160];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)dir);
    pb_int(&b, (int32_t)port_id);
    pb_int(&b, (int32_t)mix_id);
    pb_id(&b, io_id);
    pb_int(&b, (int32_t)memid);
    pb_int(&b, (int32_t)off);
    pb_int(&b, (int32_t)sz);
    pb_pop(&b);
    pwb_send_msg(c, cn_proxy_id, PW_CN_E_PORT_SET_IO, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void cn_send_port_use_buffers_clear(pwb_client_t *c, pwb_node_t *nd, uint32_t mix_id) {
    uint8_t buf[128];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)nd->direction);
    pb_int(&b, 0);
    pb_int(&b, (int32_t)mix_id);
    pb_int(&b, 0);
    pb_int(&b, 0);
    pb_pop(&b);
    pwb_send_msg(c, nd->cn_proxy_id, PW_CN_E_PORT_USE_BUFS, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void cn_send_port_set_mix_info(pwb_client_t *c, pwb_node_t *nd, uint32_t dir,
                                      uint32_t port_id, uint32_t mix_id, uint32_t peer_id) {
    uint8_t buf[256];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)dir);
    pb_int(&b, (int32_t)port_id);
    pb_int(&b, (int32_t)mix_id);
    pb_int(&b, (int32_t)peer_id);
    pwb_dict_t empty = {0};
    pb_dict(&b, &empty);
    pb_pop(&b);
    pwb_send_msg(c, nd->cn_proxy_id, PW_CN_E_PORT_SET_MIX_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void cn_send_port_use_buffers(pwb_client_t *c, pwb_node_t *nd, uint32_t mix_id,
                                     uint32_t flags, uint32_t n_buffers,
                                     const uint32_t *data_types, const uint32_t *data_ptrs,
                                     const uint32_t *data_flags, const uint32_t *data_mapoffs,
                                     const uint32_t *data_maxsizes) {
    uint8_t buf[4096];
    pwb_pod_b_t b;
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, (int32_t)nd->direction);
    pb_int(&b, 0);
    pb_int(&b, (int32_t)mix_id);
    pb_int(&b, (int32_t)flags);
    pb_int(&b, (int32_t)n_buffers);
    for (uint32_t i = 0; i < n_buffers; i++) {
        pb_int(&b, (int32_t)nd->meta_mem_id);
        pb_int(&b, (int32_t)(i * PWB_META_STRIDE));
        pb_int(&b, 56); /* Header(32) + Busy(8) + Chunk(16) */
        pb_int(&b, 2);  /* n_metas = 2 */
        pb_id(&b, PWB_META_Header);
        pb_int(&b, 32);
        pb_id(&b, PWB_META_Busy);
        pb_int(&b, 8);
        pb_int(&b, 1);  /* n_datas = 1 */
        pb_id(&b, data_types[i]);
        pb_int(&b, (int32_t)data_ptrs[i]);
        pb_int(&b, (int32_t)data_flags[i]);
        pb_int(&b, (int32_t)data_mapoffs[i]);
        pb_int(&b, (int32_t)data_maxsizes[i]);
    }
    pb_pop(&b);
    if (!b.overflow)
        pwb_send_msg(c, nd->cn_proxy_id, PW_CN_E_PORT_USE_BUFS, buf, (uint32_t)b.len, NULL, 0, NULL);
}

static void free_node_buffers(pwb_node_t *nd) {
    for (uint32_t i = 0; i < PWB_MAX_BUFFERS; i++) {
        if (nd->bufs[i].mapped && nd->bufs[i].mapped != MAP_FAILED)
            munmap(nd->bufs[i].mapped, nd->bufs[i].mapped_len);
        nd->bufs[i].mapped = NULL;
        nd->bufs[i].mapped_len = 0;
        if (nd->bufs[i].fd >= 0) close(nd->bufs[i].fd);
        nd->bufs[i].fd = -1;
    }
    nd->n_bufs = 0;
}

static void complete_link_with_prod_buffers(struct lwasapi_pw_broker *brk, pwb_link_t *lk) {
    pwb_node_t *prod = &brk->nodes[lk->out_node_idx];
    pwb_node_t *cons = &brk->nodes[lk->in_node_idx];
    pwb_client_t *pc = find_client_by_idx(brk, prod->client_idx);
    pwb_client_t *cc = find_client_by_idx(brk, cons->client_idx);
    if (!pc || !cc || !prod->in_use || !cons->in_use || prod->n_bufs == 0) return;

    uint32_t n = prod->n_bufs;
    /* Allocate consumer metadata MemFd if not yet allocated */
    size_t msz = (size_t)n * PWB_META_STRIDE;
    pwb_remove_mem(cc, &cons->meta_mem_id);
    if (cons->meta_mem_fd >= 0) {
        if (cons->meta_map) munmap(cons->meta_map, cons->meta_map_len);
        close(cons->meta_mem_fd);
        cons->meta_mem_fd = -1;
        cons->meta_map = NULL;
        cons->meta_map_len = 0;
    }
    cons->meta_mem_fd = memfd_create("salsa-pw-cmeta", MFD_CLOEXEC);
    if (cons->meta_mem_fd < 0 || ftruncate(cons->meta_mem_fd, (off_t)msz) < 0) return;
    cons->meta_map = mmap(NULL, msz, PROT_READ | PROT_WRITE, MAP_SHARED, cons->meta_mem_fd, 0);
    cons->meta_map_len = (cons->meta_map == MAP_FAILED) ? 0 : msz;
    if (cons->meta_map == MAP_FAILED) cons->meta_map = NULL;
    pwb_add_mem(cc, PWB_DATA_MemFd, cons->meta_mem_fd, 3, &cons->meta_mem_id);

    /* Register each producer buffer FD on consumer connection via Core.AddMem */
    pwb_remove_node_buf_mems(cc, cons);
    uint32_t dtypes[PWB_MAX_BUFFERS], dptrs[PWB_MAX_BUFFERS], dflags[PWB_MAX_BUFFERS],
             dmoff[PWB_MAX_BUFFERS], dmsz[PWB_MAX_BUFFERS];

    for (uint32_t i = 0; i < n; i++) {
        pwb_node_buf_t *pb = &prod->bufs[i];
        int dupfd = fcntl(pb->fd, F_DUPFD_CLOEXEC, 3);
        if (dupfd < 0) return;
        uint32_t cmid = 0;
        uint32_t mtype = pb->mem_type ? pb->mem_type : PWB_DATA_MemFd;
        pwb_add_mem(cc, mtype, dupfd, 3, &cmid);
        close(dupfd);
        cons->buf_mem_ids[i] = cmid;

        dtypes[i]  = PWB_DATA_MemId;
        dptrs[i]   = cmid;
        dflags[i]  = (mtype == PWB_DATA_DmaBuf) ? 3u : 11u;
        dmoff[i]   = pb->mapoffset;
        dmsz[i]    = pb->maxsize;
    }

    if (!prod->mix_ready) {
        cn_send_port_set_mix_info(pc, prod, PWB_DIR_OUT, 0, 0, cons->global_id);
        prod->mix_ready = true;
    }
    if (!cons->mix_ready) {
        cn_send_port_set_mix_info(cc, cons, PWB_DIR_IN, 0, 0, prod->global_id);
        cons->mix_ready = true;
    }

    cn_send_port_use_buffers(cc, cons, 0, 0, n, dtypes, dptrs, dflags, dmoff, dmsz);

    /* Create decoupled SPA_IO_Buffers for producer and consumer, relayed by Broker */
    if (prod->io_mem_fd < 0) {
        prod->io_mem_fd = memfd_create("salsa-pw-pio", MFD_CLOEXEC);
        if (prod->io_mem_fd >= 0 && ftruncate(prod->io_mem_fd, PWB_IO_BUFFERS_SIZE) == 0) {
            prod->io_map = mmap(NULL, PWB_IO_BUFFERS_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, prod->io_mem_fd, 0);
            if (prod->io_map == MAP_FAILED) prod->io_map = NULL;
            if (prod->io_map) { prod->io_map->status = PWB_STATUS_OK; prod->io_map->buffer_id = PWB_SPA_ID_INVALID; }
            pwb_add_mem(pc, PWB_DATA_MemFd, prod->io_mem_fd, 3, &prod->io_mem_id);
        }
    }
    if (cons->io_mem_fd < 0) {
        cons->io_mem_fd = memfd_create("salsa-pw-cio", MFD_CLOEXEC);
        if (cons->io_mem_fd >= 0 && ftruncate(cons->io_mem_fd, PWB_IO_BUFFERS_SIZE) == 0) {
            cons->io_map = mmap(NULL, PWB_IO_BUFFERS_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, cons->io_mem_fd, 0);
            if (cons->io_map == MAP_FAILED) cons->io_map = NULL;
            if (cons->io_map) { cons->io_map->status = PWB_STATUS_OK; cons->io_map->buffer_id = PWB_SPA_ID_INVALID; }
            pwb_add_mem(cc, PWB_DATA_MemFd, cons->io_mem_fd, 3, &cons->io_mem_id);
        }
    }

    /* Reset activation state BEFORE wiring SetActivation */
    if (prod->done_act_map) {
        uint32_t one = 1u, zero = 0u;
        memcpy(prod->done_act_map + 544, &one, 4); /* server_version = 1 */
        memcpy(prod->done_act_map + 0, &zero, 4);  /* NOT_TRIGGERED */
        pwb_act_state_t st0 = { 0, 1, 1 };
        memcpy(prod->done_act_map + 8, &st0, sizeof(st0));
    }
    if (cons->done_act_map) {
        uint32_t one = 1u, zero = 0u;
        memcpy(cons->done_act_map + 544, &one, 4);
        memcpy(cons->done_act_map + 0, &zero, 4);
        pwb_act_state_t st0 = { 0, 1, 1 };
        memcpy(cons->done_act_map + 8, &st0, sizeof(st0));
    }

    /* Update clocks/positions for both nodes */
    pwb_update_activation_pos(prod, prod->global_id);
    pwb_update_activation_pos(cons, prod->global_id);

    /* Wire Broker in the middle of SetActivation so Broker taps + relays frames */
    if (prod->done_act_fd >= 0 && prod->done_efd >= 0) {
        pwb_remove_mem(pc, &prod->done_act_memid);
        pwb_add_mem(pc, PWB_DATA_MemFd, prod->done_act_fd, 3, &prod->done_act_memid);
        cn_send_set_activation(pc, prod->cn_proxy_id, cons->global_id,
                               prod->done_efd, prod->done_act_memid, 0, PWB_ACT_SIZE);
    }
    if (cons->done_act_fd >= 0 && cons->done_efd >= 0) {
        pwb_remove_mem(cc, &cons->done_act_memid);
        pwb_add_mem(cc, PWB_DATA_MemFd, cons->done_act_fd, 3, &cons->done_act_memid);
        cn_send_set_activation(cc, cons->cn_proxy_id, prod->global_id,
                               cons->done_efd, cons->done_act_memid, 0, PWB_ACT_SIZE);
    }

    /* Send PortSetIo(SPA_IO_Buffers) */
    cn_send_port_set_io(cc, cons->cn_proxy_id, PWB_DIR_IN, 0, 0, PWB_IO_Buffers, cons->io_mem_id, 0, 16);
    cn_send_port_set_io(pc, prod->cn_proxy_id, PWB_DIR_OUT, 0, 0, PWB_IO_Buffers, prod->io_mem_id, 0, 16);

    /* Start nodes & mark link active */
    cons->state = 4; /* running */
    prod->state = 4; /* running */
    lk->state = 4;   /* active */
    lk->buffers_ready = true;
    lk->io_ready = true;

    cn_send_command(cc, cons->cn_proxy_id, 2 /* SPA_NODE_COMMAND_Start */);
    cn_send_command(pc, prod->cn_proxy_id, 2 /* SPA_NODE_COMMAND_Start */);

    /* Re-notify node info + link info */
    broadcast_node_updates(brk, prod);
    broadcast_node_updates(brk, cons);
    pwb_dict_t ld = {0};
    char tmp[32];
    dict_set(&ld, "link.output.node", (snprintf(tmp, sizeof(tmp), "%u", prod->global_id), tmp));
    dict_set(&ld, "link.input.node", (snprintf(tmp, sizeof(tmp), "%u", cons->global_id), tmp));
    broadcast_global(brk, lk->global_id, "PipeWire:Interface:Link", 3, &ld);

    lwasapi_debug_emit(LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_INFO, pc->pid, prod->global_id,
                       "PW_LINK_ACTIVE", cons->name,
                       "prod=%s(%u) -> cons=%s(%u) %ux%u@%u/%u %s %s nbuf=%u",
                       prod->name, prod->global_id, cons->name, cons->global_id,
                       prod->neg_w, prod->neg_h, prod->neg_fps_num, prod->neg_fps_denom,
                       pwb_vfmt_name(prod->neg_format),
                       prod->neg_is_dmabuf ? "DmaBuf" : "MemFd", n);
}

static int negotiate_and_activate_link(struct lwasapi_pw_broker *brk, pwb_link_t *lk) {
    pwb_node_t *prod = &brk->nodes[lk->out_node_idx];
    pwb_node_t *cons = &brk->nodes[lk->in_node_idx];
    pwb_client_t *pc = find_client_by_idx(brk, prod->client_idx);
    pwb_client_t *cc = find_client_by_idx(brk, cons->client_idx);
    if (!pc || !cc || prod->n_vfmt == 0) {
        fprintf(stderr, "[LWASAPI-PW] neg fail early: pc=%d cc=%d prod=%u(%s) nvfmt=%u cons=%u(%s)\n",
                pc != NULL, cc != NULL, prod->global_id, prod->name, prod->n_vfmt, cons->global_id, cons->name);
        return -1;
    }

    uint32_t fmt = 0, w = 0, h = 0, fn = 60, fd = 1;
    uint32_t lim_w = atomic_load(&brk->cast_limit_w);
    uint32_t lim_h = atomic_load(&brk->cast_limit_h);
    uint32_t lim_fps = atomic_load(&brk->cast_limit_fps);
    uint64_t mods[PWB_MAX_MODIFIERS];
    uint32_t n_mods = 0;
    bool has_mod = false;
    bool prod_dont_fixate = false;
    bool matched = false;

    /* Pass 1: try DMA-BUF modifier intersection if consumer supports DmaBuf */
    bool c_allows_dmabuf = (!cons->bcaps.valid) || ((cons->bcaps.datatype_mask & (1u << PWB_DATA_DmaBuf)) != 0);
    bool p_allows_dmabuf = (!prod->bcaps.valid) || ((prod->bcaps.datatype_mask & (1u << PWB_DATA_DmaBuf)) != 0);
    if (c_allows_dmabuf && p_allows_dmabuf) {
        for (uint32_t i = 0; i < prod->n_vfmt && !matched; i++) {
            for (uint32_t j = 0; j < cons->n_vfmt && !matched; j++) {
                if (intersect_vfmt(&prod->vfmt_candidates[i], &cons->vfmt_candidates[j], true,
                                   lim_w, lim_h, lim_fps,
                                   &fmt, &w, &h, &fn, &fd, mods, &n_mods)) {
                    matched = true;
                    has_mod = true;
                    prod_dont_fixate = prod->vfmt_candidates[i].mod_dont_fixate;
                }
            }
        }
    }

    /* Pass 2: clean MemFd intersection (without modifier) */
    if (!matched) {
        for (uint32_t i = 0; i < prod->n_vfmt && !matched; i++) {
            for (uint32_t j = 0; j < cons->n_vfmt && !matched; j++) {
                if (!prod->vfmt_candidates[i].has_modifier && !cons->vfmt_candidates[j].has_modifier) {
                    if (intersect_vfmt(&prod->vfmt_candidates[i], &cons->vfmt_candidates[j], false,
                                       lim_w, lim_h, lim_fps,
                                       &fmt, &w, &h, &fn, &fd, mods, &n_mods)) {
                        matched = true;
                    }
                }
            }
        }
    }

    /* Pass 3: relaxed fallback ignoring modifier if one side omitted modifier */
    if (!matched) {
        for (uint32_t i = 0; i < prod->n_vfmt && !matched; i++) {
            pwb_vfmt_caps_t pa = prod->vfmt_candidates[i];
            pa.has_modifier = false;
            for (uint32_t j = 0; j < cons->n_vfmt && !matched; j++) {
                pwb_vfmt_caps_t pb = cons->vfmt_candidates[j];
                pb.has_modifier = false;
                if (intersect_vfmt(&pa, &pb, false, lim_w, lim_h, lim_fps,
                                   &fmt, &w, &h, &fn, &fd, mods, &n_mods)) {
                    matched = true;
                }
            }
        }
    }

    /* Pass 4: consumer omitted EnumFormat (e.g. generic pipewiresrc); derive concrete format from producer */
    if (!matched && cons->n_vfmt == 0) {
        for (uint32_t i = 0; i < prod->n_vfmt && !matched; i++) {
            if (!prod->vfmt_candidates[i].has_modifier &&
                intersect_vfmt(&prod->vfmt_candidates[i], &prod->vfmt_candidates[i], false,
                               lim_w, lim_h, lim_fps,
                               &fmt, &w, &h, &fn, &fd, mods, &n_mods)) {
                matched = true;
            }
        }
        if (!matched && c_allows_dmabuf) {
            for (uint32_t i = 0; i < prod->n_vfmt && !matched; i++) {
                if (intersect_vfmt(&prod->vfmt_candidates[i], &prod->vfmt_candidates[i],
                                   prod->vfmt_candidates[i].has_modifier,
                                   lim_w, lim_h, lim_fps,
                                   &fmt, &w, &h, &fn, &fd, mods, &n_mods)) {
                    has_mod = prod->vfmt_candidates[i].has_modifier;
                    prod_dont_fixate = has_mod && prod->vfmt_candidates[i].mod_dont_fixate;
                    matched = true;
                }
            }
        }
    }

    if (!matched) {
        const pwb_vfmt_caps_t *pf = &prod->vfmt_candidates[0];
        const pwb_vfmt_caps_t *cf = cons->n_vfmt ? &cons->vfmt_candidates[0] : NULL;
        fprintf(stderr,
                "[LWASAPI-PW] neg mismatch prod=%u(%s nv=%u fmt0=%u w=%u..%u h=%u..%u mod=%u dmabuf=%d) "
                "cons=%u(%s nv=%u fmt0=%u w=%u..%u h=%u..%u mod=%u dmabuf=%d)\n",
                prod->global_id, prod->name, prod->n_vfmt, pf->n_formats ? pf->formats[0] : 0u,
                pf->w_min, pf->w_max, pf->h_min, pf->h_max, pf->n_modifiers, p_allows_dmabuf ? 1 : 0,
                cons->global_id, cons->name, cons->n_vfmt, (cf && cf->n_formats) ? cf->formats[0] : 0u,
                cf ? cf->w_min : 0u, cf ? cf->w_max : 0u, cf ? cf->h_min : 0u, cf ? cf->h_max : 0u,
                cf ? cf->n_modifiers : 0u, c_allows_dmabuf ? 1 : 0);
        lwasapi_debug_emit(LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_WARN, cc->pid, cons->global_id,
                           "PW_NEG_MISMATCH", cons->name,
                           "prod=%s(%u nv=%u w=%u..%u) cons=%s(%u nv=%u w=%u..%u)",
                           prod->name, prod->global_id, prod->n_vfmt, pf->w_min, pf->w_max,
                           cons->name, cons->global_id, cons->n_vfmt, cf ? cf->w_min : 0u, cf ? cf->w_max : 0u);
        return -1;
    }

    bool send_dont_fixate = (has_mod && prod_dont_fixate && !lk->mod_fixated);
    uint64_t mod = (has_mod && n_mods > 0) ? mods[0] : PWB_DRM_MOD_INVALID;

    uint32_t stride = prod->bcaps.stride_def ? prod->bcaps.stride_def : pwb_vfmt_stride(fmt, w);
    uint32_t sz = pwb_vfmt_size(fmt, w, h);
    if (sz < 4096) sz = 4096;

    prod->neg_format = cons->neg_format = fmt;
    prod->neg_w = cons->neg_w = w;
    prod->neg_h = cons->neg_h = h;
    prod->neg_stride = cons->neg_stride = stride;
    prod->neg_fps_num = cons->neg_fps_num = fn;
    prod->neg_fps_denom = cons->neg_fps_denom = fd;
    prod->neg_modifier = cons->neg_modifier = mod;
    prod->neg_has_modifier = cons->neg_has_modifier = has_mod;
    prod->neg_is_dmabuf = cons->neg_is_dmabuf = has_mod;

    lk->format_pod_sz = (uint32_t)build_negotiated_format_pod(
        lk->format_pod, sizeof(lk->format_pod), fmt, w, h, fn, fd,
        has_mod, send_dont_fixate, mods, n_mods);
    if (lk->format_pod_sz < 8) return -1;

    if (send_dont_fixate) {
        lk->awaiting_mod_fixation = true;
        lk->state = 1;
        cn_send_port_set_param(pc, prod->cn_proxy_id, PWB_DIR_OUT, 0, PWB_PARAM_Format,
                               lk->format_pod, lk->format_pod_sz);
        lwasapi_debug_emit(LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_INFO, pc->pid, prod->global_id,
                           "PW_MOD_FIXATE_WAIT", cons->name,
                           "prod=%s(%u) -> cons=%s(%u) %ux%u n_mods=%u first=0x%llx",
                           prod->name, prod->global_id, cons->name, cons->global_id,
                           w, h, n_mods, (unsigned long long)mod);
        return 0;
    }

    lk->awaiting_mod_fixation = false;

    /* Send SPA_PARAM_Format to both and initialize mix 0 on each port */
    cn_send_port_set_param(pc, prod->cn_proxy_id, PWB_DIR_OUT, 0, PWB_PARAM_Format, lk->format_pod, lk->format_pod_sz);
    cn_send_port_set_param(cc, cons->cn_proxy_id, PWB_DIR_IN, 0, PWB_PARAM_Format, lk->format_pod, lk->format_pod_sz);
    if (!prod->mix_ready) {
        cn_send_port_set_mix_info(pc, prod, PWB_DIR_OUT, 0, 0, cons->global_id);
        prod->mix_ready = true;
    }
    if (!cons->mix_ready) {
        cn_send_port_set_mix_info(cc, cons, PWB_DIR_IN, 0, 0, prod->global_id);
        cons->mix_ready = true;
    }

    uint32_t nbuf = 3u;
    if (prod->bcaps.valid && prod->bcaps.buf_def >= 2 && prod->bcaps.buf_def <= 8) nbuf = prod->bcaps.buf_def;

    /* Allocate producer metadata MemFd */
    size_t msz = (size_t)nbuf * PWB_META_STRIDE;
    pwb_remove_mem(pc, &prod->meta_mem_id);
    if (prod->meta_mem_fd >= 0) {
        if (prod->meta_map) munmap(prod->meta_map, prod->meta_map_len);
        close(prod->meta_mem_fd);
        prod->meta_mem_fd = -1;
        prod->meta_map = NULL;
        prod->meta_map_len = 0;
    }
    prod->meta_mem_fd = memfd_create("salsa-pw-pmeta", MFD_CLOEXEC);
    if (prod->meta_mem_fd < 0 || ftruncate(prod->meta_mem_fd, (off_t)msz) < 0) return -1;
    prod->meta_map = mmap(NULL, msz, PROT_READ | PROT_WRITE, MAP_SHARED, prod->meta_mem_fd, 0);
    prod->meta_map_len = (prod->meta_map == MAP_FAILED) ? 0 : msz;
    if (prod->meta_map == MAP_FAILED) prod->meta_map = NULL;
    pwb_add_mem(pc, PWB_DATA_MemFd, prod->meta_mem_fd, 3, &prod->meta_mem_id);

    if (prod->alloc_buffers) {
        /* Producer (like KWin::ScreenCastStream) allocates MemFd or DmaBuf FDs via add_buffer & PortBuffers */
        uint32_t dtypes[PWB_MAX_BUFFERS], dptrs[PWB_MAX_BUFFERS], dflags[PWB_MAX_BUFFERS],
                 dmoff[PWB_MAX_BUFFERS], dmsz[PWB_MAX_BUFFERS];
        uint32_t type_mask = has_mod ? ((1u << PWB_DATA_DmaBuf) | (1u << PWB_DATA_MemFd)) : (1u << PWB_DATA_MemFd);
        for (uint32_t i = 0; i < nbuf; i++) {
            dtypes[i] = type_mask;
            dptrs[i] = PWB_SPA_ID_INVALID;
            dflags[i] = 0;
            dmoff[i] = 0;
            dmsz[i] = 0;
        }
        prod->awaiting_port_buffers = true;
        prod->pending_link_idx = (uint32_t)(lk - brk->links);
        lk->state = 2; /* allocating */
        cn_send_port_use_buffers(pc, prod, 0, 1u /* SPA_NODE_BUFFERS_FLAG_ALLOC */,
                                 nbuf, dtypes, dptrs, dflags, dmoff, dmsz);
        return 0;
    }

    /* Server-allocated MemFd buffers (e.g. generic pw_stream producers) */
    pwb_remove_node_buf_mems(pc, prod);
    free_node_buffers(prod);
    uint32_t dtypes[PWB_MAX_BUFFERS], dptrs[PWB_MAX_BUFFERS], dflags[PWB_MAX_BUFFERS],
             dmoff[PWB_MAX_BUFFERS], dmsz[PWB_MAX_BUFFERS];
    for (uint32_t i = 0; i < nbuf; i++) {
        int bfd = memfd_create("salsa-pw-vbuf", MFD_CLOEXEC);
        if (bfd < 0 || ftruncate(bfd, (off_t)sz) < 0) { if (bfd >= 0) close(bfd); return -1; }
        void *mp = mmap(NULL, sz, PROT_READ | PROT_WRITE, MAP_SHARED, bfd, 0);
        prod->bufs[i].fd = bfd;
        prod->bufs[i].mem_type = PWB_DATA_MemFd;
        prod->bufs[i].flags = 11u;
        prod->bufs[i].mapoffset = 0;
        prod->bufs[i].maxsize = sz;
        prod->bufs[i].mapped = (mp == MAP_FAILED) ? NULL : mp;
        prod->bufs[i].mapped_len = (mp == MAP_FAILED) ? 0 : sz;

        uint32_t pmid = 0;
        pwb_add_mem(pc, PWB_DATA_MemFd, bfd, 3, &pmid);
        prod->buf_mem_ids[i] = pmid;
        dtypes[i] = PWB_DATA_MemId;
        dptrs[i] = pmid;
        dflags[i] = 11u;
        dmoff[i] = 0;
        dmsz[i] = sz;
    }
    prod->n_bufs = nbuf;
    cn_send_port_use_buffers(pc, prod, 0, 0, nbuf, dtypes, dptrs, dflags, dmoff, dmsz);
    complete_link_with_prod_buffers(brk, lk);
    return 0;
}

static bool node_is_linked(const struct lwasapi_pw_broker *brk, uint32_t nidx, bool is_out) {
    for (uint32_t i = 0; i < PWB_MAX_LINKS; i++) {
        if (!brk->links[i].in_use) continue;
        if (is_out && brk->links[i].out_node_idx == nidx) return true;
        if (!is_out && brk->links[i].in_node_idx == nidx) return true;
    }
    return false;
}

static void destroy_link(struct lwasapi_pw_broker *brk, uint32_t li);

static void try_autolink_all(struct lwasapi_pw_broker *brk) {
    for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
        pwb_node_t *cons = &brk->nodes[i];
        if (!cons->in_use || cons->direction != PWB_DIR_IN)
            continue;
        if (!cons->port_registered) {
            fprintf(stderr, "[LWASAPI-PW] autolink skip cons=%u(%s): port_registered=0 tgt=%u state=%u\n",
                    cons->global_id, cons->name, cons->target_id_req, cons->state);
            continue;
        }
        if (node_is_linked(brk, i, false)) continue;

        /* Find matching producer (prefer explicit target, then unlinked Video/Source, then any Video producer) */
        pwb_node_t *best_prod = NULL;
        uint32_t best_idx = 0;
        bool best_is_src = false;
        bool best_unlinked = false;
        for (uint32_t j = 0; j < PWB_MAX_NODES; j++) {
            pwb_node_t *prod = &brk->nodes[j];
            if (!prod->in_use || prod->direction != PWB_DIR_OUT || !prod->port_registered || prod->n_vfmt == 0)
                continue;
            if (cons->target_id_req && cons->target_id_req != PWB_SPA_ID_INVALID) {
                if (prod->global_id == cons->target_id_req || prod->serial == cons->target_id_req) {
                    best_prod = prod; best_idx = j; break;
                }
            } else if (cons->target_str[0]) {
                uint32_t tid = 0;
                if ((parse_u32_strict(cons->target_str, &tid) && tid > 0 &&
                     (prod->global_id == tid || prod->serial == tid)) ||
                    strcmp(prod->name, cons->target_str) == 0) {
                    best_prod = prod; best_idx = j; break;
                }
            } else if (cons->autoconnect) {
                bool is_src = (strstr(prod->media_class, "Video/Source") != NULL);
                bool is_vid = is_src || (strstr(prod->media_class, "Video") != NULL) || prod->n_vfmt > 0;
                if (!is_vid) continue;
                bool unlinked = !node_is_linked(brk, j, true);
                if (!best_prod ||
                    (is_src && !best_is_src) ||
                    (is_src == best_is_src && unlinked && !best_unlinked)) {
                    best_prod = prod;
                    best_idx = j;
                    best_is_src = is_src;
                    best_unlinked = unlinked;
                    if (is_src && unlinked) break;
                }
            }
        }
        if (!best_prod) {
            pwb_node_t *tgt_nd = (cons->target_id_req && cons->target_id_req != PWB_SPA_ID_INVALID)
                ? find_node_by_global(brk, cons->target_id_req, NULL) : NULL;
            fprintf(stderr,
                    "[LWASAPI-PW] autolink no_prod cons=%u(%s st=%u nv=%u ac=%d tgt=%u tgt_str='%s') "
                    "tgt_nd=[found=%d dir=%u reg=%u nv=%u st=%u]\n",
                    cons->global_id, cons->name, cons->state, cons->n_vfmt, cons->autoconnect ? 1 : 0,
                    cons->target_id_req, cons->target_str, tgt_nd ? 1 : 0,
                    tgt_nd ? tgt_nd->direction : 0u, tgt_nd ? (tgt_nd->port_registered ? 1u : 0u) : 0u,
                    tgt_nd ? tgt_nd->n_vfmt : 0u, tgt_nd ? tgt_nd->state : 0u);
            continue;
        }

        /* Allocate link slot */
        for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
            if (!brk->links[li].in_use) {
                pwb_link_t *lk = &brk->links[li];
                memset(lk, 0, sizeof(*lk));
                lk->in_use = true;
                lk->global_id = brk->next_global_id++;
                lk->serial = brk->next_serial++;
                lk->out_node_idx = best_idx;
                lk->in_node_idx = i;
                lk->state = 1;
                lk->last_buf_id = PWB_SPA_ID_INVALID;
                if (negotiate_and_activate_link(brk, lk) < 0) {
                    destroy_link(brk, li);
                }
                break;
            }
        }
    }
}

/* ============================================================================
 * Live Video Frame Tap -> Lockless Preview SHM
 * ============================================================================ */

static inline uint8_t clamp_u8(int v) {
    return (v < 0) ? 0 : (v > 255 ? 255 : (uint8_t)v);
}

static void copy_frame_to_preview_shm(struct lwasapi_pw_broker *brk, pwb_node_t *prod,
                                      uint32_t buf_id, uint32_t cons_count) {
    lwasapi_pw_preview_shm_t *shm = brk->preview_shm;
    if (!shm || buf_id >= prod->n_bufs) return;
    if (__atomic_load_n(&shm->preview_requested, __ATOMIC_RELAXED) == 0) return;

    uint64_t now = pwb_mono_ns();
    uint64_t exp_ns = atomic_load(&brk->preview_expiry_ns);
    uint64_t last_ping = __atomic_load_n(&shm->last_gui_ping_ns, __ATOMIC_RELAXED);
    if ((exp_ns > 0 && now > exp_ns) ||
        (exp_ns == 0 && last_ping > 0 && now > last_ping && now - last_ping > 8000000000ULL)) {
        __atomic_store_n(&shm->preview_requested, 0, __ATOMIC_RELAXED);
        return;
    }

    uint32_t tw = atomic_load(&brk->preview_target_w);
    uint32_t th_lim = atomic_load(&brk->preview_target_h);
    uint32_t tfps = atomic_load(&brk->preview_target_fps);
    if (tfps != brk->preview_applied_fps) {
        brk->preview_next_due_ns = 0ULL;
        brk->preview_fps_win_ns = 0ULL;
        brk->preview_fps_win_cnt = 0u;
        brk->preview_applied_fps = tfps;
    }
    if (!pwb_pace_due(now, &brk->preview_next_due_ns, tfps)) return;

    pwb_update_fps_meter(now, &brk->preview_fps_win_ns, &brk->preview_fps_win_cnt,
                         &brk->preview_actual_fps_x100);
    atomic_store(&brk->preview_actual_fps_x100_pub, brk->preview_actual_fps_x100);

    pwb_node_buf_t *pb = &prod->bufs[buf_id];
    uint32_t sw = prod->neg_w, sh = prod->neg_h;
    if (sw == 0 || sh == 0) return;

    /* Try lazy mmap of MemFd or linear DmaBuf */
    bool mappable = (pb->mapped != NULL);
    if (!mappable && pb->fd >= 0 && pb->maxsize > 0 &&
        pb->maxsize <= (256u * 1024u * 1024u) &&
        (pb->mem_type == PWB_DATA_MemFd ||
         (pb->mem_type == PWB_DATA_DmaBuf && (!prod->neg_has_modifier || prod->neg_modifier == 0ULL)))) {
        bool size_ok = true;
        if (pb->mem_type == PWB_DATA_MemFd) {
            struct stat bst;
            if (fstat(pb->fd, &bst) != 0 || bst.st_size <= 0 ||
                (uint64_t)pb->mapoffset > (uint64_t)bst.st_size ||
                (uint64_t)pb->maxsize > (uint64_t)bst.st_size - (uint64_t)pb->mapoffset) {
                size_ok = false;
            }
        }
        if (size_ok) {
            void *mp = mmap(NULL, pb->maxsize, PROT_READ, MAP_SHARED, pb->fd, (off_t)pb->mapoffset);
            if (mp != MAP_FAILED) {
                pb->mapped = mp;
                pb->mapped_len = pb->maxsize;
                mappable = true;
            }
        }
    }

    /* Begin seqlock odd */
    uint64_t seq = __atomic_load_n(&shm->seqlock, __ATOMIC_RELAXED);
    __atomic_store_n(&shm->seqlock, seq + 1u, __ATOMIC_RELEASE);

    shm->active_stream = 1u;
    shm->is_dmabuf = (pb->mem_type == PWB_DATA_DmaBuf) ? 1u : 0u;
    shm->dmabuf_mappable = (shm->is_dmabuf && mappable) ? 1u : 0u;
    shm->src_width = sw;
    shm->src_height = sh;
    shm->src_stride = prod->neg_stride;
    shm->src_spa_format = prod->neg_format;
    shm->drm_modifier_lo = (uint32_t)(prod->neg_modifier & 0xffffffffULL);
    shm->drm_modifier_hi = (uint32_t)(prod->neg_modifier >> 32);
    shm->prod_node_id = prod->global_id;
    shm->cons_count = cons_count;
    shm->frame_seq++;
    shm->pts_ns = now;
    pwb_safe_copy(shm->prod_name, sizeof(shm->prod_name), prod->name);
    pwb_safe_copy(shm->format_name, sizeof(shm->format_name), pwb_vfmt_name(prod->neg_format));
    shm->target_max_w = tw;
    shm->target_max_h = th_lim;
    shm->target_fps = tfps;
    shm->actual_fps_x100 = brk->preview_actual_fps_x100;
    shm->cast_fps_cap = atomic_load(&brk->cast_limit_fps);

    if (mappable && pb->mapped && sw > 0 && sh > 0) {
        if (pb->mem_type == PWB_DATA_DmaBuf) {
            struct dma_buf_sync dbs = { .flags = DMA_BUF_SYNC_START | DMA_BUF_SYNC_READ };
            (void)ioctl(pb->fd, DMA_BUF_IOCTL_SYNC, &dbs);
        }

        uint32_t cap_w = (tw > 0u && tw <= LWASAPI_PW_PREVIEW_MAX_W) ? tw : LWASAPI_PW_PREVIEW_MAX_W;
        uint32_t cap_h = (th_lim > 0u && th_lim <= LWASAPI_PW_PREVIEW_MAX_H) ? th_lim : LWASAPI_PW_PREVIEW_MAX_H;
        uint32_t dw = sw, dh = sh;
        if (dw > cap_w) {
            dh = (uint32_t)(((uint64_t)dh * cap_w) / dw);
            dw = cap_w;
        }
        if (dh > cap_h) {
            dw = (uint32_t)(((uint64_t)dw * cap_h) / dh);
            dh = cap_h;
        }
        if (dw == 0u) dw = 1u;
        if (dh == 0u) dh = 1u;
        shm->dst_width = dw;
        shm->dst_height = dh;
        shm->dst_stride = dw * 4u;

        uint32_t coff = 0;
        int32_t cstride = 0;
        if (prod->meta_map && (size_t)buf_id * PWB_META_STRIDE + 56u <= prod->meta_map_len) {
            const uint8_t *m = prod->meta_map + (size_t)buf_id * PWB_META_STRIDE;
            memcpy(&coff, m + 40, 4);
            memcpy(&cstride, m + 48, 4);
        }
        if (coff >= pb->mapped_len) coff = 0;
        const uint8_t *src = (const uint8_t *)pb->mapped + coff;
        size_t avail_len = pb->mapped_len - coff;
        size_t min_row_bytes = pwb_vfmt_stride(prod->neg_format, sw);
        size_t sstride = (cstride > 0) ? (size_t)cstride : (prod->neg_stride ? prod->neg_stride : min_row_bytes);
        if (sstride < min_row_bytes || min_row_bytes == 0 || avail_len < min_row_bytes || avail_len < sstride) {
            sh = 0u;
        } else if ((size_t)sh * sstride > avail_len) {
            sh = (uint32_t)(avail_len / sstride);
        }
        const uint64_t x_step = (((uint64_t)sw) << 16) / dw;
        for (uint32_t y = 0; y < dh && sh > 0u; y++) {
            uint32_t sy = (uint32_t)(((uint64_t)y * sh) / dh);
            uint8_t *dst_row = shm->pixels + (size_t)y * shm->dst_stride;
            if (prod->neg_format == PWB_VF_BGRx || prod->neg_format == PWB_VF_BGRA) {
                const uint8_t *srow = src + (size_t)sy * sstride;
                if (dw == sw && (size_t)sy * sstride + (size_t)sw * 4u <= avail_len) {
                    memcpy(dst_row, srow, (size_t)dw * 4u);
                } else {
                    uint64_t xp = 0;
                    for (uint32_t x = 0; x < dw; x++) {
                        uint32_t sx = (uint32_t)(xp >> 16);
                        if (sx >= sw) sx = sw - 1u;
                        xp += x_step;
                        const uint8_t *px = srow + (size_t)sx * 4u;
                        dst_row[x * 4u + 0] = px[0];
                        dst_row[x * 4u + 1] = px[1];
                        dst_row[x * 4u + 2] = px[2];
                        dst_row[x * 4u + 3] = 255;
                    }
                }
            } else if (prod->neg_format == PWB_VF_RGBx || prod->neg_format == PWB_VF_RGBA) {
                const uint8_t *srow = src + (size_t)sy * sstride;
                uint64_t xp = 0;
                for (uint32_t x = 0; x < dw; x++) {
                    uint32_t sx = (uint32_t)(xp >> 16);
                    if (sx >= sw) sx = sw - 1u;
                    xp += x_step;
                    const uint8_t *px = srow + (size_t)sx * 4u;
                    dst_row[x * 4u + 0] = px[2];
                    dst_row[x * 4u + 1] = px[1];
                    dst_row[x * 4u + 2] = px[0];
                    dst_row[x * 4u + 3] = 255;
                }
            } else if (prod->neg_format == PWB_VF_YUY2) {
                const uint8_t *srow = src + (size_t)sy * sstride;
                uint64_t xp = 0;
                for (uint32_t x = 0; x < dw; x++) {
                    uint32_t sx = (uint32_t)(xp >> 16);
                    if (sx >= sw) sx = sw - 1u;
                    xp += x_step;
                    uint32_t pair = (sx & ~1u) * 2u;
                    int Y = srow[sx * 2u];
                    int U = srow[pair + 1] - 128;
                    int V = srow[pair + 3] - 128;
                    dst_row[x * 4u + 2] = clamp_u8(Y + ((359 * V) >> 8));
                    dst_row[x * 4u + 1] = clamp_u8(Y - ((88 * U + 183 * V) >> 8));
                    dst_row[x * 4u + 0] = clamp_u8(Y + ((454 * U) >> 8));
                    dst_row[x * 4u + 3] = 255;
                }
            } else if (prod->neg_format == PWB_VF_NV12) {
                const uint8_t *yplane = src + (size_t)sy * sstride;
                const uint8_t *uvplane = src + (size_t)sh * sstride + (size_t)(sy >> 1) * sstride;
                uint64_t xp = 0;
                for (uint32_t x = 0; x < dw; x++) {
                    uint32_t sx = (uint32_t)(xp >> 16);
                    if (sx >= sw) sx = sw - 1u;
                    xp += x_step;
                    int Y = yplane[sx];
                    int U = uvplane[(sx & ~1u) + 0] - 128;
                    int V = uvplane[(sx & ~1u) + 1] - 128;
                    dst_row[x * 4u + 2] = clamp_u8(Y + ((359 * V) >> 8));
                    dst_row[x * 4u + 1] = clamp_u8(Y - ((88 * U + 183 * V) >> 8));
                    dst_row[x * 4u + 0] = clamp_u8(Y + ((454 * U) >> 8));
                    dst_row[x * 4u + 3] = 255;
                }
            }
        }

        if (pb->mem_type == PWB_DATA_DmaBuf) {
            struct dma_buf_sync dbs = { .flags = DMA_BUF_SYNC_END | DMA_BUF_SYNC_READ };
            (void)ioctl(pb->fd, DMA_BUF_IOCTL_SYNC, &dbs);
        }
    } else {
        shm->dst_width = 0;
        shm->dst_height = 0;
    }

    __atomic_store_n(&shm->seqlock, seq + 2u, __ATOMIC_RELEASE);
}

/* ============================================================================
 * Incoming Wire Request Handlers
 * ============================================================================ */

static pwb_proxy_t *find_proxy(pwb_client_t *c, uint32_t proxy_id) {
    for (uint32_t i = 0; i < PWB_MAX_CLIENT_OBJS; i++) {
        if (c->objs[i].in_use && c->objs[i].proxy_id == proxy_id) return &c->objs[i];
    }
    return NULL;
}

static pwb_proxy_t *alloc_proxy(pwb_client_t *c, uint32_t proxy_id, uint32_t gid, uint8_t kind) {
    pwb_proxy_t *free_slot = NULL;
    for (uint32_t i = 0; i < PWB_MAX_CLIENT_OBJS; i++) {
        if (c->objs[i].in_use && c->objs[i].proxy_id == proxy_id) {
            free_slot = &c->objs[i];
            break;
        }
        if (!c->objs[i].in_use && !free_slot) {
            free_slot = &c->objs[i];
        }
    }
    if (!free_slot) return NULL;
    memset(free_slot, 0, sizeof(*free_slot));
    free_slot->in_use = true;
    free_slot->proxy_id = proxy_id;
    free_slot->global_id = gid;
    free_slot->kind = kind;
    return free_slot;
}

static void update_preview_active_state(struct lwasapi_pw_broker *brk) {
    bool any_active = false;
    for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
        if (brk->links[li].in_use && brk->links[li].state == 4) { any_active = true; break; }
    }
    if (!any_active && brk->preview_shm) {
        __atomic_store_n(&brk->preview_shm->active_stream, 0, __ATOMIC_RELEASE);
    }
}

static void unwire_node_link_side(struct lwasapi_pw_broker *brk, uint32_t nidx,
                                  uint32_t peer_gid, bool is_out) {
    if (nidx >= PWB_MAX_NODES || !brk->nodes[nidx].in_use) return;
    pwb_node_t *nd = &brk->nodes[nidx];
    pwb_client_t *c = find_client_by_idx(brk, nd->client_idx);
    bool can_send = (c && c->in_use && !c->disconnecting && c->fd >= 0);

    if (can_send && !nd->destroying && nd->cn_proxy_id != 0) {
        if (peer_gid != 0) {
            cn_send_set_activation(c, nd->cn_proxy_id, peer_gid, -1, PWB_SPA_ID_INVALID, 0, 0);
        }
        if (!node_is_linked(brk, nidx, is_out)) {
            if (nd->state == 4) {
                cn_send_command(c, nd->cn_proxy_id, 1 /* SPA_NODE_COMMAND_Pause */);
            }
            cn_send_port_use_buffers_clear(c, nd, 0);
            cn_send_port_set_io(c, nd->cn_proxy_id, is_out ? PWB_DIR_OUT : PWB_DIR_IN,
                                0, 0, PWB_IO_Buffers, PWB_SPA_ID_INVALID, 0, 0);
        }
    }

    if (!node_is_linked(brk, nidx, is_out)) {
        if (can_send) {
            pwb_remove_mem(c, &nd->meta_mem_id);
            pwb_remove_mem(c, &nd->io_mem_id);
            pwb_remove_mem(c, &nd->done_act_memid);
            pwb_remove_node_buf_mems(c, nd);
        }
        if (nd->meta_map && nd->meta_map != MAP_FAILED) munmap(nd->meta_map, nd->meta_map_len);
        nd->meta_map = NULL;
        nd->meta_map_len = 0;
        if (nd->meta_mem_fd >= 0) { close(nd->meta_mem_fd); nd->meta_mem_fd = -1; }
        if (nd->io_map && nd->io_map != MAP_FAILED) munmap(nd->io_map, PWB_IO_BUFFERS_SIZE);
        nd->io_map = NULL;
        if (nd->io_mem_fd >= 0) { close(nd->io_mem_fd); nd->io_mem_fd = -1; }
        if (is_out) free_node_buffers(nd);
        nd->mix_ready = false;
        nd->awaiting_port_buffers = false;
        nd->state = nd->active ? 3 : 2;
    }
}

static void destroy_link(struct lwasapi_pw_broker *brk, uint32_t li) {
    if (li >= PWB_MAX_LINKS || !brk->links[li].in_use) return;
    pwb_link_t *lk = &brk->links[li];
    uint32_t lgid = lk->global_id;
    uint32_t out_idx = lk->out_node_idx;
    uint32_t in_idx = lk->in_node_idx;
    uint32_t prod_gid = (out_idx < PWB_MAX_NODES && brk->nodes[out_idx].in_use) ? brk->nodes[out_idx].global_id : 0u;
    uint32_t cons_gid = (in_idx < PWB_MAX_NODES && brk->nodes[in_idx].in_use) ? brk->nodes[in_idx].global_id : 0u;

    lk->in_use = false;
    broadcast_global_remove(brk, lgid);

    unwire_node_link_side(brk, in_idx, prod_gid, false);
    unwire_node_link_side(brk, out_idx, cons_gid, true);
    update_preview_active_state(brk);
}

static void destroy_node(struct lwasapi_pw_broker *brk, uint32_t nidx) {
    if (nidx >= PWB_MAX_NODES || !brk->nodes[nidx].in_use) return;
    pwb_node_t *nd = &brk->nodes[nidx];
    nd->destroying = true;

    for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
        if (brk->links[li].in_use &&
            (brk->links[li].out_node_idx == nidx || brk->links[li].in_node_idx == nidx)) {
            destroy_link(brk, li);
        }
    }

    pwb_client_t *owner = find_client_by_idx(brk, nd->client_idx);
    if (owner && owner->in_use && !owner->disconnecting && owner->fd >= 0) {
        pwb_remove_mem(owner, &nd->act_mem_id);
        pwb_remove_mem(owner, &nd->done_act_memid);
        pwb_remove_mem(owner, &nd->meta_mem_id);
        pwb_remove_mem(owner, &nd->io_mem_id);
        pwb_remove_node_buf_mems(owner, nd);
    }

    if (nd->port_registered) broadcast_global_remove(brk, nd->port_global_id);
    broadcast_global_remove(brk, nd->global_id);

    free_node_buffers(nd);
    clear_raw_params(nd->node_params, &nd->n_node_params);
    clear_raw_params(nd->port_params, &nd->n_port_params);

    if (nd->act_map && nd->act_map != MAP_FAILED) munmap(nd->act_map, PWB_ACT_SIZE);
    if (nd->act_mem_fd >= 0) close(nd->act_mem_fd);
    if (nd->act_readfd >= 0) close(nd->act_readfd);
    if (nd->act_writefd >= 0) close(nd->act_writefd);
    if (nd->done_act_map && nd->done_act_map != MAP_FAILED) munmap(nd->done_act_map, PWB_ACT_SIZE);
    if (nd->done_act_fd >= 0) close(nd->done_act_fd);
    if (nd->done_efd >= 0) close(nd->done_efd);
    if (nd->meta_map && nd->meta_map != MAP_FAILED) munmap(nd->meta_map, nd->meta_map_len);
    if (nd->meta_mem_fd >= 0) close(nd->meta_mem_fd);
    if (nd->io_map && nd->io_map != MAP_FAILED) munmap(nd->io_map, PWB_IO_BUFFERS_SIZE);
    if (nd->io_mem_fd >= 0) close(nd->io_mem_fd);

    memset(nd, 0, sizeof(*nd));
    nd->act_mem_fd = nd->act_readfd = nd->act_writefd = -1;
    nd->done_efd = nd->done_act_fd = nd->meta_mem_fd = nd->io_mem_fd = -1;
    init_node_mem_ids(nd);

    /* Clear preview active flag if no active link left */
    update_preview_active_state(brk);
}

static void handle_client_disconnect(struct lwasapi_pw_broker *brk, pwb_client_t *c) {
    if (!c || !c->in_use) return;
    c->disconnecting = true;
    uint32_t cidx = c->client_idx;
    uint32_t cgid = c->global_id;
    for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
        if (brk->nodes[i].in_use && brk->nodes[i].client_idx == cidx) destroy_node(brk, i);
    }
    for (uint32_t i = 0; i < PWB_MAX_LINKS; i++) {
        if (brk->links[i].in_use && brk->links[i].owner_client_idx == cidx) {
            destroy_link(brk, i);
        }
    }
    broadcast_global_remove(brk, cgid);
    if (c->fd >= 0) close(c->fd);
    for (uint32_t i = 0; i < c->rx_nfds; i++) if (c->rx_fds[i] >= 0) close(c->rx_fds[i]);
    free(c->rx_buf);
    memset(c, 0, sizeof(*c));
    c->fd = -1;
}

static void send_core_info_and_client_bound(struct lwasapi_pw_broker *brk, pwb_client_t *c) {
    struct passwd *pw = getpwuid(getuid());
    struct utsname un; uname(&un);
    uint8_t buf[2048];
    pwb_pod_b_t b;
    pwb_dict_t props = {0};
    dict_set(&props, "core.name", "pipewire-0");
    dict_set(&props, "core.daemon", "true");
    dict_set(&props, "application.name", "Salsa Video Broker");
    dict_set(&props, "default.clock.rate", "48000");
    dict_set(&props, "link.max-buffers", "16");

    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, 0);
    pb_int(&b, (int32_t)(0x534C5057u ^ c->global_id));
    pb_str(&b, pw ? pw->pw_name : "user");
    pb_str(&b, un.nodename[0] ? un.nodename : "localhost");
    pb_str(&b, "1.6.11"); /* Required by OBS/Chromium/libpipewire version parser */
    pb_str(&b, "pipewire-0");
    pb_long(&b, 1); /* PW_CORE_CHANGE_MASK_PROPS */
    pb_dict(&b, &props);
    pb_pop(&b);
    pwb_send_msg(c, 0, PW_CORE_E_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);

    /* Send BoundProps for proxy 1 (the Client object itself) */
    pb_init(&b, buf, sizeof(buf));
    pb_push_struct(&b);
    pb_int(&b, 1);
    pb_int(&b, (int32_t)c->global_id);
    pb_dict(&b, &c->props);
    pb_pop(&b);
    pwb_send_msg(c, 0, PW_CORE_E_BOUND_PROPS, buf, (uint32_t)b.len, NULL, 0, NULL);
    (void)brk;
}

static void handle_core_msg(struct lwasapi_pw_broker *brk, pwb_client_t *c,
                            uint8_t op, uint32_t seq, const uint8_t *payload, uint32_t sz) {
    (void)seq;
    pwb_cur_t cur;
    cur_init(&cur, payload, sz);
    pwb_cur_t st;
    if (!cur_enter_struct(&cur, &st)) return;

    if (op == PW_CORE_M_HELLO) {
        bool re_hello = c->hello_done;
        c->hello_done = true;
        if (re_hello) {
            memset(c->objs, 0, sizeof(c->objs));
            fprintf(stderr, "[LWASAPI-PW] client %u re-HELLO (stolen fd handoff, cleared stale proxies)\n", c->global_id);
        }
        send_core_info_and_client_bound(brk, c);
        if (!re_hello) {
            broadcast_global(brk, c->global_id, "PipeWire:Interface:Client", 3, &c->props);
        }
    } else if (op == PW_CORE_M_SYNC) {
        uint32_t id = 0, sseq = 0;
        if (!cur_u32(&st, &id) || !cur_u32(&st, &sseq)) return;
        uint8_t buf[64]; pwb_pod_b_t b;
        pb_init(&b, buf, sizeof(buf));
        pb_push_struct(&b);
        pb_int(&b, (int32_t)id);
        pb_int(&b, (int32_t)sseq);
        pb_pop(&b);
        pwb_send_msg(c, 0, PW_CORE_E_DONE, buf, (uint32_t)b.len, NULL, 0, NULL);
    } else if (op == PW_CORE_M_GET_REGISTRY) {
        uint32_t ver = 3, new_id = 0;
        if (!cur_u32(&st, &ver) || !cur_u32(&st, &new_id)) return;
        alloc_proxy(c, new_id, 0, 1);

        /* Global 0: Core */
        pwb_dict_t d0 = {0};
        dict_set(&d0, "core.name", "pipewire-0");
        dict_set(&d0, "object.serial", "0");
        emit_registry_global(c, new_id, 0, PWB_PERM_ALL, "PipeWire:Interface:Core", 4, &d0);

        /* Globals 1..4: Required Factories */
        for (int fi = 1; fi <= 4; fi++) {
            const char *fn = NULL, *ft = NULL; uint32_t fv = 3; pwb_dict_t fd = {0};
            build_factory_dict(fi, &fn, &ft, &fv, &fd);
            emit_registry_global(c, new_id, (uint32_t)fi, PWB_PERM_ALL, "PipeWire:Interface:Factory", 3, &fd);
        }
        /* Globals 5..6: Metadata */
        for (uint32_t mi = 5; mi <= 6; mi++) {
            pwb_dict_t md = {0};
            build_metadata_dict(mi, &md);
            emit_registry_global(c, new_id, mi, PWB_PERM_ALL, "PipeWire:Interface:Metadata", 3, &md);
        }
        /* All active Clients */
        for (uint32_t i = 0; i < PWB_MAX_CLIENTS; i++) {
            if (brk->clients[i].in_use && brk->clients[i].hello_done) {
                emit_registry_global(c, new_id, brk->clients[i].global_id, PWB_PERM_ALL,
                                     "PipeWire:Interface:Client", 3, &brk->clients[i].props);
            }
        }
        /* All active Nodes & Ports */
        for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
            pwb_node_t *nd = &brk->nodes[i];
            if (!nd->in_use) continue;
            pwb_dict_t ndd; build_node_public_props(nd, &ndd);
            emit_registry_global(c, new_id, nd->global_id, PWB_PERM_ALL, "PipeWire:Interface:Node", 3, &ndd);
            if (nd->port_registered)
                emit_registry_global(c, new_id, nd->port_global_id, PWB_PERM_ALL,
                                     "PipeWire:Interface:Port", 3, &nd->port_props);
        }
        /* Active Links */
        for (uint32_t i = 0; i < PWB_MAX_LINKS; i++) {
            if (brk->links[i].in_use) {
                pwb_dict_t ld = {0};
                pwb_node_t *on = &brk->nodes[brk->links[i].out_node_idx];
                pwb_node_t *in = &brk->nodes[brk->links[i].in_node_idx];
                char tmp[32];
                dict_set(&ld, "link.output.node", (snprintf(tmp, sizeof(tmp), "%u", on->global_id), tmp));
                dict_set(&ld, "link.input.node", (snprintf(tmp, sizeof(tmp), "%u", in->global_id), tmp));
                emit_registry_global(c, new_id, brk->links[i].global_id, PWB_PERM_ALL,
                                     "PipeWire:Interface:Link", 3, &ld);
            }
        }
    } else if (op == PW_CORE_M_CREATE_OBJ) {
        char fac[64], tp[96]; uint32_t ver = 0, new_id = 0;
        pwb_dict_t props = {0};
        if (!cur_str(&st, fac, sizeof(fac)) || !cur_str(&st, tp, sizeof(tp)) ||
            !cur_u32(&st, &ver) || !cur_dict(&st, &props, true) || !cur_u32(&st, &new_id))
            return;

        if (strcmp(fac, "client-node") == 0 || strstr(tp, "ClientNode") != NULL) {
            const char *mtype = dict_get(&props, "media.type");
            const char *mclass = dict_get(&props, "media.class");
            if ((mtype && strcasecmp(mtype, "Audio") == 0) ||
                (mclass && strstr(mclass, "Audio") != NULL)) {
                atomic_fetch_add(&brk->audio_rejects, 1u);
                uint8_t eb[256]; pwb_pod_b_t b;
                pb_init(&b, eb, sizeof(eb));
                pb_push_struct(&b);
                pb_int(&b, (int32_t)new_id);
                pb_int(&b, 0);
                pb_int(&b, -EPROTONOSUPPORT);
                pb_str(&b, "Salsa pipewire-0 broker is Video-Only (audio routes via Salsa PA/ALSA/WASAPI)");
                pb_pop(&b);
                pwb_send_msg(c, 0, PW_CORE_E_ERROR, eb, (uint32_t)b.len, NULL, 0, NULL);
                lwasapi_debug_emit(LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_WARN, c->pid, new_id,
                                   "PW_AUDIO_REJECTED", mclass ? mclass : "Audio",
                                   "client=%s rejected audio node on video-only broker", c->app_name);
                return;
            }

            /* Allocate Video Node slot */
            int free_i = -1;
            for (int i = 0; i < PWB_MAX_NODES; i++) if (!brk->nodes[i].in_use) { free_i = i; break; }
            if (free_i < 0) return;

            pwb_node_t *nd = &brk->nodes[free_i];
            memset(nd, 0, sizeof(*nd));
            nd->in_use = true;
            nd->global_id = brk->next_global_id++;
            nd->serial = brk->next_serial++;
            nd->client_idx = c->client_idx;
            nd->cn_proxy_id = new_id;
            nd->node_props = props;
            nd->state = 2; /* suspended */
            nd->act_mem_fd = nd->act_readfd = nd->act_writefd = -1;
            nd->done_efd = nd->done_act_fd = nd->meta_mem_fd = nd->io_mem_fd = -1;
            for (uint32_t k = 0; k < PWB_MAX_BUFFERS; k++) nd->bufs[k].fd = -1;
            init_node_mem_ids(nd);

            const char *nn = dict_get(&props, "node.name");
            const char *mn = dict_get(&props, "media.name");
            pwb_safe_copy(nd->name, sizeof(nd->name), nn ? nn : (mn ? mn : c->app_name));
            pwb_safe_copy(nd->media_class, sizeof(nd->media_class), mclass ? mclass : "Video");

            const char *drv = dict_get(&props, "node.driver");
            const char *ac = dict_get(&props, "node.autoconnect");
            const char *alloc = dict_get(&props, "adapt.follower.spa-node");
            (void)alloc;
            nd->is_driver = (drv && strcmp(drv, "true") == 0) || (mclass && strstr(mclass, "Video/Source") != NULL);
            nd->autoconnect = (!ac || strcmp(ac, "true") == 0);
            nd->direction = (mclass && (strstr(mclass, "Input") != NULL || strstr(mclass, "Sink") != NULL))
                            ? PWB_DIR_IN : PWB_DIR_OUT;
            apply_node_target_props(nd);

            /* Allocate node's own 2312B activation memfd & read/write eventfds */
            nd->act_mem_fd = memfd_create("salsa-pw-act", MFD_CLOEXEC);
            if (nd->act_mem_fd < 0 || ftruncate(nd->act_mem_fd, PWB_ACT_SIZE) < 0) { destroy_node(brk, (uint32_t)free_i); return; }
            nd->act_map = mmap(NULL, PWB_ACT_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, nd->act_mem_fd, 0);
            if (nd->act_map == MAP_FAILED) nd->act_map = NULL;
            if (nd->act_map) {
                memset(nd->act_map, 0, PWB_ACT_SIZE);
                uint32_t three = 3u; memcpy(nd->act_map, &three, 4); /* FINISHED */
                pwb_act_state_t st0 = { 0, 1, 1 };
                memcpy(nd->act_map + 8, &st0, sizeof(st0));
                pwb_update_activation_pos(nd, nd->global_id);
            }
            nd->act_readfd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
            nd->act_writefd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);

            nd->done_efd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
            nd->done_act_fd = memfd_create("salsa-pw-dact", MFD_CLOEXEC);
            if (nd->done_act_fd >= 0 && ftruncate(nd->done_act_fd, PWB_ACT_SIZE) == 0) {
                nd->done_act_map = mmap(NULL, PWB_ACT_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, nd->done_act_fd, 0);
                if (nd->done_act_map == MAP_FAILED) nd->done_act_map = NULL;
                if (nd->done_act_map) {
                    memset(nd->done_act_map, 0, PWB_ACT_SIZE);
                    uint32_t one = 1u; memcpy(nd->done_act_map + 544, &one, 4);
                    pwb_act_state_t st0 = { 0, 1, 1 }; memcpy(nd->done_act_map + 8, &st0, sizeof(st0));
                }
            }

            alloc_proxy(c, new_id, nd->global_id, 3);

            /* 1. Core.AddMem for node's activation */
            pwb_add_mem(c, PWB_DATA_MemFd, nd->act_mem_fd, 3, &nd->act_mem_id);
            /* 2. ClientNode.SetActivation(self) */
            cn_send_set_activation(c, new_id, nd->global_id, nd->act_readfd, nd->act_mem_id, 0, PWB_ACT_SIZE);
            /* 3. Core.BoundProps(proxy -> node_global_id) */
            pwb_dict_t ndpub; build_node_public_props(nd, &ndpub);
            uint8_t bp[2048]; pwb_pod_b_t b;
            pb_init(&b, bp, sizeof(bp));
            pb_push_struct(&b);
            pb_int(&b, (int32_t)new_id);
            pb_int(&b, (int32_t)nd->global_id);
            pb_dict(&b, &ndpub);
            pb_pop(&b);
            pwb_send_msg(c, 0, PW_CORE_E_BOUND_PROPS, bp, (uint32_t)b.len, NULL, 0, NULL);
            /* 4. ClientNode.Transport(readfd, writefd, act_mem_id) */
            pb_init(&b, bp, sizeof(bp));
            pb_push_struct(&b);
            pb_fd(&b, 0);
            pb_fd(&b, 1);
            pb_int(&b, (int32_t)nd->act_mem_id);
            pb_int(&b, 0);
            pb_int(&b, (int32_t)PWB_ACT_SIZE);
            pb_pop(&b);
            int tfds[2] = { nd->act_readfd, nd->act_writefd };
            pwb_send_msg(c, new_id, PW_CN_E_TRANSPORT, bp, (uint32_t)b.len, tfds, 2, NULL);
            /* 5. ClientNode.SetIo(SPA_IO_Clock) & SetIo(SPA_IO_Position) */
            uint32_t io_ids[2] = { PWB_IO_Clock, PWB_IO_Position };
            uint32_t io_szs[2] = { PWB_ACT_CLOCK_SIZE, PWB_ACT_POS_SIZE };
            for (int k = 0; k < 2; k++) {
                pb_init(&b, bp, sizeof(bp));
                pb_push_struct(&b);
                pb_id(&b, io_ids[k]);
                pb_int(&b, (int32_t)nd->act_mem_id);
                pb_int(&b, (int32_t)PWB_ACT_POS_OFFSET);
                pb_int(&b, (int32_t)io_szs[k]);
                pb_pop(&b);
                pwb_send_msg(c, new_id, PW_CN_E_SET_IO, bp, (uint32_t)b.len, NULL, 0, NULL);
            }
            /* 6. Broadcast Node global */
            broadcast_global(brk, nd->global_id, "PipeWire:Interface:Node", 3, &ndpub);

            lwasapi_debug_emit(LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_INFO, c->pid, nd->global_id,
                               "PW_NODE_CREATED", nd->name,
                               "class=%s dir=%s driver=%d app=%s",
                               nd->media_class, nd->direction ? "OUT" : "IN", nd->is_driver ? 1 : 0, c->app_name);
        } else if (strcmp(fac, "link-factory") == 0 || strstr(tp, "Link") != NULL) {
            const char *os = dict_get(&props, "link.output.node");
            if (!os || !os[0]) os = dict_get(&props, "link.output.port");
            const char *is = dict_get(&props, "link.input.node");
            if (!is || !is[0]) is = dict_get(&props, "link.input.port");
            uint32_t oi = 0, ii = 0;
            if (find_node_by_ref(brk, os, PWB_DIR_OUT, &oi) &&
                find_node_by_ref(brk, is, PWB_DIR_IN, &ii) &&
                brk->nodes[oi].direction == PWB_DIR_OUT &&
                brk->nodes[ii].direction == PWB_DIR_IN) {
                for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
                    if (!brk->links[li].in_use) {
                        pwb_link_t *lk = &brk->links[li];
                        memset(lk, 0, sizeof(*lk));
                        lk->in_use = true;
                        lk->global_id = brk->next_global_id++;
                        lk->serial = brk->next_serial++;
                        lk->out_node_idx = oi;
                        lk->in_node_idx = ii;
                        lk->owner_client_idx = c->client_idx;
                        lk->owner_proxy_id = new_id;
                        lk->last_buf_id = PWB_SPA_ID_INVALID;
                        alloc_proxy(c, new_id, lk->global_id, 6);
                        pwb_dict_t ld = props;
                        uint8_t bp[1024]; pwb_pod_b_t b;
                        pb_init(&b, bp, sizeof(bp));
                        pb_push_struct(&b);
                        pb_int(&b, (int32_t)new_id);
                        pb_int(&b, (int32_t)lk->global_id);
                        pb_dict(&b, &ld);
                        pb_pop(&b);
                        pwb_send_msg(c, 0, PW_CORE_E_BOUND_PROPS, bp, (uint32_t)b.len, NULL, 0, NULL);
                        negotiate_and_activate_link(brk, lk);
                        break;
                    }
                }
            }
        }
    } else if (op == PW_CORE_M_DESTROY) {
        uint32_t id = 0;
        if (!cur_u32(&st, &id)) return;
        pwb_proxy_t *px = find_proxy(c, id);
        if (px) {
            if (px->kind == 3) {
                uint32_t nidx = 0;
                if (find_node_by_global(brk, px->global_id, &nidx)) destroy_node(brk, nidx);
            } else if (px->kind == 6) {
                for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
                    if (brk->links[li].in_use && brk->links[li].global_id == px->global_id) {
                        destroy_link(brk, li);
                    }
                }
            }
            px->in_use = false;
        }
        pwb_send_remove_id(c, id);
    }
}

static void handle_registry_msg(struct lwasapi_pw_broker *brk, pwb_client_t *c,
                                pwb_proxy_t *px, uint8_t op, const uint8_t *payload, uint32_t sz) {
    (void)px;
    pwb_cur_t cur, st;
    cur_init(&cur, payload, sz);
    if (!cur_enter_struct(&cur, &st)) return;

    if (op == PW_REG_M_BIND) {
        uint32_t gid = 0, ver = 0, new_id = 0;
        char tp[96];
        if (!cur_u32(&st, &gid) || !cur_str(&st, tp, sizeof(tp)) ||
            !cur_u32(&st, &ver) || !cur_u32(&st, &new_id))
            return;

        if (strcmp(tp, "PipeWire:Interface:Factory") == 0 && gid >= 1 && gid <= 4) {
            alloc_proxy(c, new_id, gid, 7);
            const char *fn = NULL, *ft = NULL; uint32_t fv = 3; pwb_dict_t fd;
            build_factory_dict((int)gid, &fn, &ft, &fv, &fd);
            uint8_t buf[1024]; pwb_pod_b_t b;
            pb_init(&b, buf, sizeof(buf));
            pb_push_struct(&b);
            pb_int(&b, (int32_t)gid);
            pb_str(&b, fn);
            pb_str(&b, ft);
            pb_int(&b, (int32_t)fv);
            pb_long(&b, 1);
            pb_dict(&b, &fd);
            pb_pop(&b);
            pwb_send_msg(c, new_id, PW_FACTORY_E_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);
        } else if (strcmp(tp, "PipeWire:Interface:Metadata") == 0) {
            alloc_proxy(c, new_id, gid, 8);
            if (gid == 5) {
                for (uint32_t mi = 0; mi < brk->n_metas; mi++) {
                    const pwb_meta_entry_t *e = &brk->metas[mi];
                    uint32_t s = (uint32_t)strtoul(e->subject, NULL, 10);
                    emit_metadata_prop(c, new_id, s, e->key, e->type, e->val);
                }
            }
        } else if (strcmp(tp, "PipeWire:Interface:Client") == 0) {
            alloc_proxy(c, new_id, gid, 2);
            for (uint32_t i = 0; i < PWB_MAX_CLIENTS; i++) {
                if (brk->clients[i].in_use && brk->clients[i].global_id == gid) {
                    uint8_t buf[1024]; pwb_pod_b_t b;
                    pb_init(&b, buf, sizeof(buf));
                    pb_push_struct(&b);
                    pb_int(&b, (int32_t)gid);
                    pb_long(&b, 1);
                    pb_dict(&b, &brk->clients[i].props);
                    pb_pop(&b);
                    pwb_send_msg(c, new_id, PW_CLIENT_E_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);
                    break;
                }
            }
        } else if (strcmp(tp, "PipeWire:Interface:Node") == 0) {
            alloc_proxy(c, new_id, gid, 4);
            pwb_node_t *nd = find_node_by_global(brk, gid, NULL);
            if (nd) emit_node_info(c, new_id, nd);
        } else if (strcmp(tp, "PipeWire:Interface:Port") == 0) {
            alloc_proxy(c, new_id, gid, 5);
            for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
                if (brk->nodes[i].in_use && brk->nodes[i].port_registered &&
                    brk->nodes[i].port_global_id == gid) {
                    emit_port_info(c, new_id, &brk->nodes[i]);
                    break;
                }
            }
        } else if (strcmp(tp, "PipeWire:Interface:Link") == 0) {
            alloc_proxy(c, new_id, gid, 6);
            for (uint32_t i = 0; i < PWB_MAX_LINKS; i++) {
                if (brk->links[i].in_use && brk->links[i].global_id == gid) {
                    emit_link_info(c, new_id, brk, &brk->links[i]);
                    break;
                }
            }
        } else {
            alloc_proxy(c, new_id, gid, 2);
        }
    } else if (op == PW_REG_M_DESTROY) {
        uint32_t gid = 0;
        if (!cur_u32(&st, &gid)) return;
        for (uint32_t i = 0; i < PWB_MAX_LINKS; i++) {
            if (brk->links[i].in_use && brk->links[i].global_id == gid) {
                destroy_link(brk, i);
            }
        }
    }
}

static void handle_client_obj_msg(struct lwasapi_pw_broker *brk, pwb_client_t *c,
                                  pwb_proxy_t *px, uint8_t op, const uint8_t *payload, uint32_t sz) {
    (void)px;
    pwb_cur_t cur, st;
    cur_init(&cur, payload, sz);
    if (!cur_enter_struct(&cur, &st)) return;

    if (op == PW_CLIENT_M_UPDATE_PROPS) {
        if (cur_dict(&st, &c->props, true)) {
            const char *an = dict_get(&c->props, "application.name");
            const char *bn = dict_get(&c->props, "application.process.binary");
            const char *pi = dict_get(&c->props, "pipewire.access.portal.app_id");
            const char *ip = dict_get(&c->props, "pipewire.access.portal.is_portal");
            if (an && an[0]) pwb_safe_copy(c->app_name, sizeof(c->app_name), an);
            if (bn && bn[0]) pwb_safe_copy(c->binary, sizeof(c->binary), bn);
            if (pi && pi[0]) pwb_safe_copy(c->portal_app_id, sizeof(c->portal_app_id), pi);
            if (ip && strcmp(ip, "true") == 0) c->is_portal = true;

            /* Re-emit Client.Info */
            uint8_t buf[2048]; pwb_pod_b_t b;
            pb_init(&b, buf, sizeof(buf));
            pb_push_struct(&b);
            pb_int(&b, (int32_t)c->global_id);
            pb_long(&b, 1);
            pb_dict(&b, &c->props);
            pb_pop(&b);
            pwb_send_msg(c, 1, PW_CLIENT_E_INFO, buf, (uint32_t)b.len, NULL, 0, NULL);
        }
    } else if (op == PW_CLIENT_M_GET_PERMS) {
        uint8_t buf[128]; pwb_pod_b_t b;
        pb_init(&b, buf, sizeof(buf));
        pb_push_struct(&b);
        pb_int(&b, 0);
        pb_push_struct(&b);
        pb_int(&b, 1);
        pb_int(&b, -1);
        pb_int(&b, (int32_t)PWB_PERM_ALL);
        pb_pop(&b);
        pb_pop(&b);
        pwb_send_msg(c, px->proxy_id, PW_CLIENT_E_PERMS, buf, (uint32_t)b.len, NULL, 0, NULL);
    } else if (op == PW_CLIENT_M_UPDATE_PERMS) {
        /* Accepted cleanly so xdg-desktop-portal permissions flow never fails */
        (void)brk;
    }
}

static void handle_node_or_port_enum(struct lwasapi_pw_broker *brk, pwb_client_t *c,
                                     pwb_proxy_t *px, uint8_t op, uint32_t req_seq,
                                     const uint8_t *payload, uint32_t sz, bool is_port) {
    pwb_cur_t cur, st;
    cur_init(&cur, payload, sz);
    if (!cur_enter_struct(&cur, &st)) return;

    pwb_node_t *nd = NULL;
    if (is_port) {
        for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
            if (brk->nodes[i].in_use && brk->nodes[i].port_registered &&
                brk->nodes[i].port_global_id == px->global_id) { nd = &brk->nodes[i]; break; }
        }
    } else {
        nd = find_node_by_global(brk, px->global_id, NULL);
    }
    if (!nd) return;

    const pwb_raw_param_t *arr = is_port ? nd->port_params : nd->node_params;
    uint32_t arr_n = is_port ? nd->n_port_params : nd->n_node_params;
    uint8_t ev_param = is_port ? PW_PORT_E_PARAM : PW_NODE_E_PARAM;

    if (op == (is_port ? PW_PORT_M_ENUM_PARAMS : PW_NODE_M_ENUM_PARAMS)) {
        int32_t eseq = 0; uint32_t pid = 0, index = 0, num = 100;
        if (!cur_int(&st, &eseq) || !cur_u32(&st, &pid)) return;
        (void)cur_u32(&st, &index);
        (void)cur_u32(&st, &num);
        if (!is_port && arr_n == 0 && nd->n_port_params > 0) {
            arr = nd->port_params;
            arr_n = nd->n_port_params;
        } else if (!is_port && pid != 0) {
            bool any_in_node = false;
            for (uint32_t i = 0; i < arr_n; i++) {
                if (arr[i].id == pid) { any_in_node = true; break; }
            }
            if (!any_in_node && nd->n_port_params > 0) {
                arr = nd->port_params;
                arr_n = nd->n_port_params;
            }
        }
        uint32_t found_idx = 0;
        for (uint32_t i = 0; i < arr_n; i++) {
            if (pid == 0 || arr[i].id == pid) {
                if (found_idx >= index && found_idx < index + num) {
                    uint8_t buf[2048]; pwb_pod_b_t b;
                    pb_init(&b, buf, sizeof(buf));
                    pb_push_struct(&b);
                    pb_int(&b, eseq);
                    pb_id(&b, arr[i].id);
                    pb_int(&b, (int32_t)found_idx);
                    pb_int(&b, (int32_t)(found_idx + 1u));
                    pb_raw_pod(&b, arr[i].pod, arr[i].size);
                    pb_pop(&b);
                    if (!b.overflow)
                        pwb_send_msg(c, px->proxy_id, ev_param, buf, (uint32_t)b.len, NULL, 0, NULL);
                }
                found_idx++;
            }
        }
        uint8_t db[64]; pwb_pod_b_t b;
        pb_init(&b, db, sizeof(db));
        pb_push_struct(&b);
        pb_int(&b, (int32_t)px->proxy_id);
        pb_int(&b, (int32_t)(req_seq | PWB_SPA_ASYNC_BIT));
        pb_pop(&b);
        pwb_send_msg(c, 0, PW_CORE_E_DONE, db, (uint32_t)b.len, NULL, 0, NULL);
    } else if (op == (is_port ? PW_PORT_M_SUBSCRIBE : PW_NODE_M_SUBSCRIBE)) {
        int32_t n_ids = 0;
        if (!cur_int(&st, &n_ids) || n_ids <= 0 || n_ids > 32) return;
        for (int32_t k = 0; k < n_ids; k++) {
            uint32_t pid = 0;
            if (!cur_u32(&st, &pid)) break;
            const pwb_raw_param_t *s_arr = arr;
            uint32_t s_n = arr_n;
            if (!is_port) {
                bool any_in_node = false;
                for (uint32_t i = 0; i < s_n; i++) {
                    if (s_arr[i].id == pid) { any_in_node = true; break; }
                }
                if (!any_in_node && nd->n_port_params > 0) {
                    s_arr = nd->port_params;
                    s_n = nd->n_port_params;
                }
            }
            uint32_t idx = 0;
            for (uint32_t i = 0; i < s_n; i++) {
                if (s_arr[i].id == pid) {
                    uint8_t buf[2048]; pwb_pod_b_t b;
                    pb_init(&b, buf, sizeof(buf));
                    pb_push_struct(&b);
                    pb_int(&b, 0);
                    pb_id(&b, s_arr[i].id);
                    pb_int(&b, (int32_t)idx);
                    pb_int(&b, (int32_t)(idx + 1u));
                    pb_raw_pod(&b, s_arr[i].pod, s_arr[i].size);
                    pb_pop(&b);
                    if (!b.overflow)
                        pwb_send_msg(c, px->proxy_id, ev_param, buf, (uint32_t)b.len, NULL, 0, NULL);
                    idx++;
                }
            }
        }
    }
}

static void handle_client_node_msg(struct lwasapi_pw_broker *brk, pwb_client_t *c,
                                   pwb_proxy_t *px, uint8_t op, const uint8_t *payload,
                                   uint32_t sz, const int *msg_fds, uint32_t msg_nfds) {
    uint32_t nidx = 0;
    pwb_node_t *nd = find_node_by_global(brk, px->global_id, &nidx);
    if (!nd) return;

    pwb_cur_t cur, st;
    cur_init(&cur, payload, sz);
    if (!cur_enter_struct(&cur, &st)) return;

    if (op == PW_CN_M_GET_NODE) {
        uint32_t ver = 3, new_id = 0;
        if (!cur_u32(&st, &ver) || !cur_u32(&st, &new_id)) return;
        nd->node_proxy_id = new_id;
        alloc_proxy(c, new_id, nd->global_id, 4);
        emit_node_info(c, new_id, nd);
    } else if (op == PW_CN_M_UPDATE) {
        uint32_t change_mask = 0; int32_t n_params = 0;
        if (!cur_u32(&st, &change_mask) || !cur_int(&st, &n_params) || n_params < 0 || n_params > 64) return;
        if (change_mask & 1u) { clear_raw_params(nd->node_params, &nd->n_node_params); }
        for (int32_t i = 0; i < n_params; i++) {
            uint32_t psz = 0, ptp = 0; const uint8_t *pbody = NULL, *pptr = NULL; size_t ptot = 0;
            if (!cur_next_pod(&st, &psz, &ptp, &pbody, &ptot, &pptr)) return;
            if (ptp == PWB_SPA_OBJECT && psz >= 8) {
                uint32_t oid = 0; memcpy(&oid, pbody + 4, 4);
                store_raw_param(nd->node_params, &nd->n_node_params, oid, pptr, (uint32_t)ptot);
            }
        }
        /* Node info struct (optional SPA_NONE or SPA_STRUCT) */
        uint32_t isz = 0, itp = 0; const uint8_t *ibody = NULL;
        if (cur_next_pod(&st, &isz, &itp, &ibody, NULL, NULL) && itp == PWB_SPA_STRUCT) {
            pwb_cur_t inf; inf.base = ibody; inf.off = 0; inf.end = isz;
            int32_t max_in = 0, max_out = 0; int64_t ich = 0, ifl = 0;
            if (cur_int(&inf, &max_in) && cur_int(&inf, &max_out) &&
                cur_long(&inf, &ich) && cur_long(&inf, &ifl)) {
                (void)ifl;
                if (max_in > 0 && max_out == 0) nd->direction = PWB_DIR_IN;
                else if (max_out > 0 && max_in == 0) nd->direction = PWB_DIR_OUT;
                if (ich & 16LL) { /* SPA_NODE_CHANGE_MASK_PROPS */
                    cur_dict(&inf, &nd->node_props, false);
                    apply_node_target_props(nd);
                }
            }
        }
        broadcast_node_updates(brk, nd);
        try_autolink_all(brk);
    } else if (op == PW_CN_M_PORT_UPDATE) {
        uint32_t dir = 0, port_id = 0, change_mask = 0; int32_t n_params = 0;
        if (!cur_u32(&st, &dir) || !cur_u32(&st, &port_id) ||
            !cur_u32(&st, &change_mask) || !cur_int(&st, &n_params) || n_params < 0 || n_params > 64) {
            fprintf(stderr, "[LWASAPI-PW] PORT_UPDATE header parse fail node=%u(%s) sz=%u\n",
                    nd->global_id, nd->name, sz);
            return;
        }
        nd->direction = (uint8_t)dir;
        if (change_mask & 1u) {
            clear_raw_params(nd->port_params, &nd->n_port_params);
            nd->n_vfmt = 0;
            memset(nd->vfmt_candidates, 0, sizeof(nd->vfmt_candidates));
        }
        for (int32_t i = 0; i < n_params; i++) {
            uint32_t psz = 0, ptp = 0; const uint8_t *pbody = NULL, *pptr = NULL; size_t ptot = 0;
            if (!cur_next_pod(&st, &psz, &ptp, &pbody, &ptot, &pptr)) {
                fprintf(stderr, "[LWASAPI-PW] PORT_UPDATE pod fail node=%u(%s) idx=%d/%d off=%zu/%zu\n",
                        nd->global_id, nd->name, i, n_params, st.off, st.end);
                return;
            }
            if (ptp == PWB_SPA_OBJECT && psz >= 8) {
                uint32_t oid = 0; memcpy(&oid, pbody + 4, 4);
                store_raw_param(nd->port_params, &nd->n_port_params, oid, pptr, (uint32_t)ptot);
                parse_param_pod(nd, pptr, (uint32_t)ptot);
            }
        }
        uint32_t isz = 0, itp = 0; const uint8_t *ibody = NULL;
        if (cur_next_pod(&st, &isz, &itp, &ibody, NULL, NULL) && itp == PWB_SPA_STRUCT) {
            pwb_cur_t inf; inf.base = ibody; inf.off = 0; inf.end = isz;
            int64_t ich = 0, ifl = 0; int32_t r1 = 0, r2 = 0;
            if (cur_long(&inf, &ich) && cur_long(&inf, &ifl) && cur_int(&inf, &r1) && cur_int(&inf, &r2)) {
                (void)r1; (void)r2;
                /* bit 2 (4) = SPA_PORT_FLAG_CAN_ALLOC_BUFFERS */
                if (ifl & (1LL << 2)) nd->alloc_buffers = true;
                if (ich & 4LL) cur_dict(&inf, &nd->port_props, false); /* PROPS */
            }
        }
        if (strcmp(nd->name, "plasmashell") != 0) {
            const pwb_vfmt_caps_t *v0 = nd->n_vfmt ? &nd->vfmt_candidates[0] : NULL;
            fprintf(stderr,
                    "[LWASAPI-PW] PORT_UPDATE node=%u(%s) dir=%u pid=%u cm=0x%x np=%d nvfmt=%u "
                    "bcaps=[v=%d dt=0x%x] v0=[fmt=%u w=%u..%u h=%u..%u nmod=%u]\n",
                    nd->global_id, nd->name, dir, port_id, change_mask, n_params, nd->n_vfmt,
                    nd->bcaps.valid ? 1 : 0, nd->bcaps.datatype_mask,
                    (v0 && v0->n_formats) ? v0->formats[0] : 0u,
                    v0 ? v0->w_min : 0u, v0 ? v0->w_max : 0u,
                    v0 ? v0->h_min : 0u, v0 ? v0->h_max : 0u,
                    v0 ? v0->n_modifiers : 0u);
        }
        if (!nd->port_registered) {
            nd->port_registered = true;
            nd->port_global_id = brk->next_global_id++;
            char tmp[32];
            dict_set(&nd->port_props, "node.id", (snprintf(tmp, sizeof(tmp), "%u", nd->global_id), tmp));
            dict_set(&nd->port_props, "port.id", "0");
            dict_set(&nd->port_props, "port.direction", (nd->direction == PWB_DIR_OUT) ? "out" : "in");
            broadcast_global(brk, nd->port_global_id, "PipeWire:Interface:Port", 3, &nd->port_props);
            broadcast_node_updates(brk, nd);
        }
        if ((change_mask & 1u) && n_params > 0) {
            for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
                pwb_link_t *lk = &brk->links[li];
                if (!lk->in_use || lk->state >= 2u) continue;
                if (lk->out_node_idx != (uint32_t)nidx && lk->in_node_idx != (uint32_t)nidx) continue;
                pwb_node_t *prod_n = &brk->nodes[lk->out_node_idx];
                if (prod_n->n_bufs > 0 || prod_n->n_vfmt == 0) continue;
                if (lk->awaiting_mod_fixation) {
                    if ((uint32_t)nidx != lk->out_node_idx) continue;
                    lk->awaiting_mod_fixation = false;
                    lk->mod_fixated = true;
                }
                prod_n->awaiting_port_buffers = false;
                if (negotiate_and_activate_link(brk, lk) < 0) {
                    destroy_link(brk, li);
                }
            }
        }
        try_autolink_all(brk);
    } else if (op == PW_CN_M_SET_ACTIVE) {
        int32_t act = 0;
        if (cur_int(&st, &act)) {
            nd->active = (act != 0);
            nd->state = act ? 3 : 2; /* idle or suspended */
            if (strcmp(nd->name, "plasmashell") != 0) {
                fprintf(stderr, "[LWASAPI-PW] SET_ACTIVE node=%u(%s) act=%d -> state=%u\n",
                        nd->global_id, nd->name, act, nd->state);
            }
            broadcast_node_updates(brk, nd);
        }
    } else if (op == PW_CN_M_EVENT) {
        uint32_t esz = 0, etp = 0; const uint8_t *ebody = NULL;
        if (cur_next_pod(&st, &esz, &etp, &ebody, NULL, NULL) && etp == PWB_SPA_OBJECT && esz >= 8) {
            uint32_t otype = 0, oid = 0;
            memcpy(&otype, ebody, 4); memcpy(&oid, ebody + 4, 4);
            if (otype == PWB_EVT_NODE && oid == 3 /* RequestProcess */) {
                if (nd->act_map) {
                    uint32_t z = 0; memcpy(nd->act_map, &z, 4);
                    pwb_act_state_t s = { 0, 1, 0 }; memcpy(nd->act_map + 8, &s, sizeof(s));
                    uint32_t trg = 1u; memcpy(nd->act_map, &trg, 4);
                }
                if (nd->act_readfd >= 0) {
                    uint64_t one = 1u; ssize_t wr = write(nd->act_readfd, &one, 8); (void)wr;
                }
            }
        }
    } else if (op == PW_CN_M_PORT_BUFFERS) {
        uint32_t dir = 0, port_id = 0, mix_id = 0, n_buffers = 0;
        (void)dir; (void)port_id; (void)mix_id;
        if (!cur_u32(&st, &dir) || !cur_u32(&st, &port_id) ||
            !cur_u32(&st, &mix_id) || !cur_u32(&st, &n_buffers))
            return;
        if (n_buffers > PWB_MAX_BUFFERS) n_buffers = PWB_MAX_BUFFERS;
        free_node_buffers(nd);

        /* Record alloc_buffers from incoming PortBuffers cleanly */
        nd->alloc_buffers = true;
        for (uint32_t i = 0; i < n_buffers; i++) {
            uint32_t n_datas = 0;
            if (!cur_u32(&st, &n_datas) || n_datas == 0) break;
            for (uint32_t j = 0; j < n_datas; j++) {
                uint32_t dt = 0, dfl = 0, dmo = 0, dms = 0; int64_t fd_idx = -1;
                if (!cur_u32(&st, &dt) || !cur_long(&st, &fd_idx) ||
                    !cur_u32(&st, &dfl) || !cur_u32(&st, &dmo) || !cur_u32(&st, &dms))
                    break;
                if (j == 0 && fd_idx >= 0 && (uint32_t)fd_idx < msg_nfds && msg_fds[fd_idx] >= 0) {
                    int dupfd = fcntl(msg_fds[fd_idx], F_DUPFD_CLOEXEC, 3);
                    nd->bufs[i].fd = dupfd;
                    nd->bufs[i].mem_type = dt;
                    nd->bufs[i].flags = dfl;
                    nd->bufs[i].mapoffset = dmo;
                    nd->bufs[i].maxsize = dms;
                }
            }
        }
        nd->n_bufs = n_buffers;
        if (nd->awaiting_port_buffers) {
            nd->awaiting_port_buffers = false;
            if (nd->pending_link_idx < PWB_MAX_LINKS && brk->links[nd->pending_link_idx].in_use) {
                complete_link_with_prod_buffers(brk, &brk->links[nd->pending_link_idx]);
            }
        }
    }
    (void)nidx;
}

static void broadcast_metadata_prop(struct lwasapi_pw_broker *brk, uint32_t meta_gid,
                                    uint32_t subj, const char *key,
                                    const char *type, const char *val) {
    for (uint32_t ci = 0; ci < PWB_MAX_CLIENTS; ci++) {
        pwb_client_t *mc = &brk->clients[ci];
        if (!mc->in_use) continue;
        for (uint32_t oi = 0; oi < PWB_MAX_CLIENT_OBJS; oi++) {
            pwb_proxy_t *mpx = &mc->objs[oi];
            if (mpx->in_use && mpx->kind == 8 && mpx->global_id == meta_gid) {
                emit_metadata_prop(mc, mpx->proxy_id, subj, key, type, val);
            }
        }
    }
}

static void handle_metadata_msg(struct lwasapi_pw_broker *brk, pwb_client_t *c,
                                pwb_proxy_t *px, uint8_t op,
                                const uint8_t *payload, uint32_t sz) {
    (void)c;
    if (op == 2) { /* PW_METADATA_METHOD_CLEAR */
        if (px->global_id == 5) brk->n_metas = 0;
        broadcast_metadata_prop(brk, px->global_id, 0, NULL, NULL, NULL);
        return;
    }
    if (op != PW_META_M_SET_PROPERTY) return;

    pwb_cur_t cur, st;
    cur_init(&cur, payload, sz);
    if (!cur_enter_struct(&cur, &st)) return;

    uint32_t subj = 0;
    char key[64], type[32], val[160];
    if (!cur_u32(&st, &subj) || !cur_str(&st, key, sizeof(key)) ||
        !cur_str(&st, type, sizeof(type)) || !cur_str(&st, val, sizeof(val))) return;

    if (px->global_id == 5) {
        char sbuf[32];
        snprintf(sbuf, sizeof(sbuf), "%u", subj);
        if (!key[0]) {
            uint32_t dst = 0;
            for (uint32_t i = 0; i < brk->n_metas; i++) {
                if (strcmp(brk->metas[i].subject, sbuf) != 0)
                    brk->metas[dst++] = brk->metas[i];
            }
            brk->n_metas = dst;
        } else if (!val[0]) {
            uint32_t dst = 0;
            for (uint32_t i = 0; i < brk->n_metas; i++) {
                if (!(strcmp(brk->metas[i].subject, sbuf) == 0 &&
                      strcmp(brk->metas[i].key, key) == 0))
                    brk->metas[dst++] = brk->metas[i];
            }
            brk->n_metas = dst;
        } else {
            pwb_meta_entry_t *found = NULL;
            for (uint32_t i = 0; i < brk->n_metas; i++) {
                if (strcmp(brk->metas[i].subject, sbuf) == 0 &&
                    strcmp(brk->metas[i].key, key) == 0) {
                    found = &brk->metas[i];
                    break;
                }
            }
            if (!found && brk->n_metas < 16) found = &brk->metas[brk->n_metas++];
            if (found) {
                pwb_safe_copy(found->subject, sizeof(found->subject), sbuf);
                pwb_safe_copy(found->key, sizeof(found->key), key);
                pwb_safe_copy(found->type, sizeof(found->type), type);
                pwb_safe_copy(found->val, sizeof(found->val), val);
            }
        }
    }

    broadcast_metadata_prop(brk, px->global_id, subj,
                            key[0] ? key : NULL,
                            type[0] ? type : NULL,
                            val[0] ? val : NULL);

    if (strcmp(key, "target.object") == 0 || strcmp(key, "target.node") == 0) {
        pwb_node_t *nd = find_node_by_global(brk, subj, NULL);
        if (nd) {
            if (!val[0] || strcmp(val, "-1") == 0) {
                nd->target_str[0] = '\0';
                nd->target_id_req = 0u;
            } else {
                pwb_safe_copy(nd->target_str, sizeof(nd->target_str), val);
                uint32_t tid = 0;
                nd->target_id_req = parse_u32_strict(val, &tid) ? tid : 0u;
            }
            try_autolink_all(brk);
        }
    }
}

static void dispatch_client_packet(struct lwasapi_pw_broker *brk, pwb_client_t *c,
                                   uint32_t obj_id, uint8_t op, uint32_t seq,
                                   const uint8_t *payload, uint32_t sz,
                                   const int *msg_fds, uint32_t msg_nfds) {
    if (obj_id == 0) {
        handle_core_msg(brk, c, op, seq, payload, sz);
        return;
    }
    if (obj_id == 1) {
        pwb_proxy_t dummy = { .in_use = true, .proxy_id = 1, .global_id = c->global_id, .kind = 2 };
        handle_client_obj_msg(brk, c, &dummy, op, payload, sz);
        return;
    }
    pwb_proxy_t *px = find_proxy(c, obj_id);
    if (!px) return;
    switch (px->kind) {
        case 1: handle_registry_msg(brk, c, px, op, payload, sz); break;
        case 2: handle_client_obj_msg(brk, c, px, op, payload, sz); break;
        case 3: handle_client_node_msg(brk, c, px, op, payload, sz, msg_fds, msg_nfds); break;
        case 4: handle_node_or_port_enum(brk, c, px, op, seq, payload, sz, false); break;
        case 5: handle_node_or_port_enum(brk, c, px, op, seq, payload, sz, true); break;
        case 8: handle_metadata_msg(brk, c, px, op, payload, sz); break;
        default: break;
    }
}

static int drain_client_socket(struct lwasapi_pw_broker *brk, pwb_client_t *c) {
    uint8_t tmp[65536];
    uint8_t cbuf[CMSG_SPACE(sizeof(int) * 64)];
    struct msghdr msg = {0};
    struct iovec iov = { .iov_base = tmp, .iov_len = sizeof(tmp) };
    msg.msg_iov = &iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf;
    msg.msg_controllen = sizeof(cbuf);

    ssize_t n = recvmsg(c->fd, &msg, MSG_DONTWAIT);
    if (n == 0) return -1;
    if (n < 0) {
        if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR) return 0;
        return -1;
    }

    for (struct cmsghdr *cm = CMSG_FIRSTHDR(&msg); cm; cm = CMSG_NXTHDR(&msg, cm)) {
        if (cm->cmsg_level == SOL_SOCKET && cm->cmsg_type == SCM_RIGHTS) {
            size_t cnt = (cm->cmsg_len - CMSG_LEN(0)) / sizeof(int);
            const int *rfds = (const int *)CMSG_DATA(cm);
            for (size_t i = 0; i < cnt; i++) {
                if (c->rx_nfds < 256) c->rx_fds[c->rx_nfds++] = rfds[i];
                else close(rfds[i]);
            }
        }
    }

    if (c->rx_len + (size_t)n > (16u * 1024u * 1024u)) {
        return -1;
    }
    if (c->rx_len + (size_t)n > c->rx_cap) {
        size_t ncap = (c->rx_len + (size_t)n) * 2u;
        if (ncap < 65536u) ncap = 65536u;
        if (ncap > (16u * 1024u * 1024u)) ncap = (16u * 1024u * 1024u);
        uint8_t *nb = (uint8_t *)realloc(c->rx_buf, ncap);
        if (!nb) return -1;
        c->rx_buf = nb;
        c->rx_cap = ncap;
    }
    memcpy(c->rx_buf + c->rx_len, tmp, (size_t)n);
    c->rx_len += (size_t)n;

    size_t consumed = 0;
    while (c->rx_len - consumed >= 16u) {
        uint32_t hdr[4];
        memcpy(hdr, c->rx_buf + consumed, 16);
        uint32_t oid = hdr[0];
        uint8_t op = (uint8_t)((hdr[1] >> 24) & 0xffu);
        uint32_t psz = hdr[1] & 0x00ffffffu;
        uint32_t seq = hdr[2];
        uint32_t nfds = hdr[3];
        if (nfds > 256u) return -1;
        size_t pkt_len = 16u + pwb_pad8(psz);
        if (c->rx_len - consumed < pkt_len) break;
        if (c->rx_nfds < nfds) break;

        int local_fds[64];
        uint32_t take_fds = (nfds > 64) ? 64 : nfds;
        for (uint32_t i = 0; i < take_fds; i++) local_fds[i] = c->rx_fds[i];
        if (nfds > take_fds) {
            for (uint32_t i = take_fds; i < nfds; i++) close(c->rx_fds[i]);
        }
        if (nfds > 0 && c->rx_nfds > nfds) {
            memmove(c->rx_fds, c->rx_fds + nfds, (c->rx_nfds - nfds) * sizeof(int));
        }
        c->rx_nfds -= nfds;

        dispatch_client_packet(brk, c, oid, op, seq, c->rx_buf + consumed + 16u, psz,
                               local_fds, take_fds);

        for (uint32_t i = 0; i < take_fds; i++) {
            if (local_fds[i] >= 0) close(local_fds[i]);
        }
        consumed += pkt_len;
    }
    if (consumed > 0) {
        if (consumed < c->rx_len) memmove(c->rx_buf, c->rx_buf + consumed, c->rx_len - consumed);
        c->rx_len -= consumed;
    }
    return 1;
}

/* ============================================================================
 * Relay Cycle Handling (Producer Frame Ready -> Mirror + Wake Consumers -> Recycle)
 * ============================================================================ */

static void on_prod_done_event(struct lwasapi_pw_broker *brk, uint32_t nidx) {
    pwb_node_t *prod = &brk->nodes[nidx];
    uint64_t val = 0;
    ssize_t r = read(prod->done_efd, &val, 8);
    (void)r;

    if (prod->done_act_map) {
        pwb_act_state_t st0 = { 0, 1, 1 };
        memcpy(prod->done_act_map + 8, &st0, sizeof(st0));
        uint32_t nt = 0;
        memcpy(prod->done_act_map, &nt, 4);
    }

    if (!prod->io_map || prod->io_map->status != PWB_STATUS_HAVE_DATA) return;
    uint32_t buf_id = prod->io_map->buffer_id;
    prod->frames_count++;

    uint32_t cl_fps = atomic_load(&brk->cast_limit_fps);
    if (cl_fps != prod->relay_applied_fps) {
        prod->relay_next_due_ns = 0ULL;
        prod->relay_applied_fps = cl_fps;
    }
    if (cl_fps > 0u) {
        bool has_ready_link = false;
        for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
            const pwb_link_t *lk = &brk->links[li];
            if (lk->in_use && lk->out_node_idx == nidx && lk->io_ready) {
                has_ready_link = true;
                break;
            }
        }
        if (has_ready_link && !pwb_pace_due(pwb_mono_ns(), &prod->relay_next_due_ns, cl_fps)) {
            prod->io_map->status = PWB_STATUS_NEED_DATA;
            return;
        }
    }

    uint32_t cons_woken = 0;
    for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
        pwb_link_t *lk = &brk->links[li];
        if (!lk->in_use || lk->out_node_idx != nidx || !lk->io_ready) continue;
        pwb_node_t *cons = &brk->nodes[lk->in_node_idx];
        if (cons->in_use && cons->io_map && cons->act_readfd >= 0) {
            if (prod->meta_map && cons->meta_map &&
                (size_t)buf_id * PWB_META_STRIDE + 56u <= prod->meta_map_len &&
                (size_t)buf_id * PWB_META_STRIDE + 56u <= cons->meta_map_len) {
                const uint8_t *pm = prod->meta_map + (size_t)buf_id * PWB_META_STRIDE;
                uint8_t *cm = cons->meta_map + (size_t)buf_id * PWB_META_STRIDE;
                memcpy(cm, pm, 32);           /* spa_meta_header */
                memcpy(cm + 40, pm + 40, 16); /* spa_chunk */
            }
            cons->io_map->buffer_id = buf_id;
            cons->io_map->status = PWB_STATUS_HAVE_DATA;
            lk->last_buf_id = buf_id;
            lk->frames_relayed++;
            cons->frames_count++;
            if (cons->act_map) {
                uint32_t trig = 1u; /* TRIGGERED */
                memcpy(cons->act_map, &trig, 4);
                pwb_act_state_t s = { 0, 1, 0 };
                memcpy(cons->act_map + 8, &s, sizeof(s));
            }
            uint64_t one = 1u;
            ssize_t wr = write(cons->act_readfd, &one, 8);
            (void)wr;
            cons_woken++;
        }
    }

    if (cons_woken > 0) {
        atomic_fetch_add(&brk->frames_relayed_total, 1u);
        copy_frame_to_preview_shm(brk, prod, buf_id, cons_woken);
    } else {
        /* Recycle directly if no consumers linked */
        prod->io_map->status = PWB_STATUS_NEED_DATA;
    }
}

static void on_cons_done_event(struct lwasapi_pw_broker *brk, uint32_t nidx) {
    pwb_node_t *cons = &brk->nodes[nidx];
    uint64_t val = 0;
    ssize_t r = read(cons->done_efd, &val, 8);
    (void)r;

    if (cons->done_act_map) {
        pwb_act_state_t st0 = { 0, 1, 1 };
        memcpy(cons->done_act_map + 8, &st0, sizeof(st0));
        uint32_t nt = 0;
        memcpy(cons->done_act_map, &nt, 4);
    }

    if (!cons->io_map) return;
    uint32_t rec_id = cons->io_map->buffer_id;

    for (uint32_t li = 0; li < PWB_MAX_LINKS; li++) {
        pwb_link_t *lk = &brk->links[li];
        if (!lk->in_use || lk->in_node_idx != nidx || !lk->io_ready) continue;
        pwb_node_t *prod = &brk->nodes[lk->out_node_idx];
        if (prod->in_use && prod->io_map) {
            if (cons->io_map->status == PWB_STATUS_NEED_DATA && rec_id != PWB_SPA_ID_INVALID) {
                prod->io_map->buffer_id = rec_id;
                prod->io_map->status = PWB_STATUS_NEED_DATA;
            }
        }
    }
}

/* ============================================================================
 * Socket Binding Policy & Zero-PipeWire Mode Detection
 * ============================================================================ */

static void detect_proc_info(uint32_t pid, char *app, size_t alen, char *bin, size_t blen) {
    char path[64];
    snprintf(path, sizeof(path), "/proc/%u/comm", pid);
    FILE *f = fopen(path, "r");
    char buf[128] = {0};
    if (f) {
        if (fgets(buf, sizeof(buf), f)) {
            size_t n = strlen(buf);
            while (n > 0 && (buf[n - 1] == '\n' || buf[n - 1] == '\r')) buf[--n] = '\0';
        }
        fclose(f);
    }
    if (!buf[0]) snprintf(buf, sizeof(buf), "pid-%u", pid);
    pwb_safe_copy(app, alen, buf);
    pwb_safe_copy(bin, blen, buf);
}

static bool pwb_zero_mode_configured(const struct lwasapi_pw_broker *brk) {
    if (brk->ignore_zero_flag) return true;
    const char *env_zero = getenv("LWASAPI_PW_ZERO_MODE");
    if (env_zero && strcmp(env_zero, "1") == 0) return true;
    const char *home = getenv("HOME");
    if (!home) return false;
    char path[512];
    snprintf(path, sizeof(path), "%s/.config/lwasapi/pipewire_zero_mode", home);
    FILE *f = fopen(path, "r");
    if (!f) return false;
    char line[64] = {0};
    if (fgets(line, sizeof(line), f)) {
        fclose(f);
        return (strncmp(line, "enabled", 7) == 0 || line[0] == '1');
    }
    fclose(f);
    return false;
}

static bool socket_actively_served_by_upstream(const char *sock_path) {
    struct stat st;
    if (lstat(sock_path, &st) != 0) return false;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return false;
    struct sockaddr_un addr = {0};
    addr.sun_family = AF_UNIX;
    pwb_safe_copy(addr.sun_path, sizeof(addr.sun_path), sock_path);
    int rc = connect(fd, (struct sockaddr *)&addr, sizeof(addr));
    bool live = false;
    if (rc == 0) {
        struct ucred uc = {0};
        socklen_t ulen = sizeof(uc);
        if (getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &uc, &ulen) == 0 && uc.pid > 0 && uc.pid != getpid()) {
            live = true;
        }
    }
    close(fd);
    return live;
}

static int bind_unix_sock(const char *path) {
    unlink(path);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    struct sockaddr_un addr = {0};
    addr.sun_family = AF_UNIX;
    if (strlen(path) >= sizeof(addr.sun_path)) { close(fd); return -1; }
    pwb_safe_copy(addr.sun_path, sizeof(addr.sun_path), path);
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0 || listen(fd, 32) < 0) {
        close(fd);
        return -1;
    }
    chmod(path, 0600);
    return fd;
}

static void accept_unix_client(struct lwasapi_pw_broker *brk, int lfd) {
    int cfd = accept4(lfd, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC);
    if (cfd < 0) return;

    int free_i = -1;
    for (int i = 0; i < PWB_MAX_CLIENTS; i++) if (!brk->clients[i].in_use) { free_i = i; break; }
    if (free_i < 0) { close(cfd); return; }

    pwb_client_t *c = &brk->clients[free_i];
    memset(c, 0, sizeof(*c));
    c->in_use = true;
    c->fd = cfd;
    c->mem_free_head = PWB_SPA_ID_INVALID;
    c->client_idx = (uint32_t)free_i;
    c->global_id = brk->next_global_id++;
    c->serial = brk->next_serial++;

    struct ucred uc = {0};
    socklen_t ulen = sizeof(uc);
    if (getsockopt(cfd, SOL_SOCKET, SO_PEERCRED, &uc, &ulen) == 0) {
        c->pid = (uint32_t)uc.pid;
        c->uid = (uint32_t)uc.uid;
        c->gid = (uint32_t)uc.gid;
    }
    detect_proc_info(c->pid, c->app_name, sizeof(c->app_name), c->binary, sizeof(c->binary));
    char tmp[32];
    dict_set(&c->props, "application.name", c->app_name);
    dict_set(&c->props, "application.process.binary", c->binary);
    dict_set(&c->props, "pipewire.sec.pid", (snprintf(tmp, sizeof(tmp), "%u", c->pid), tmp));
    dict_set(&c->props, "pipewire.sec.uid", (snprintf(tmp, sizeof(tmp), "%u", c->uid), tmp));
    dict_set(&c->props, "pipewire.sec.gid", (snprintf(tmp, sizeof(tmp), "%u", c->gid), tmp));
    dict_set(&c->props, "object.serial", (snprintf(tmp, sizeof(tmp), "%u", c->serial), tmp));
    dict_set(&c->props, "module.id", "2");
    dict_set(&c->props, "pipewire.protocol", "protocol-native");
}

static void *broker_thread_main(void *arg) {
    struct lwasapi_pw_broker *brk = (struct lwasapi_pw_broker *)arg;
    struct pollfd pfds[3 + PWB_MAX_CLIENTS + PWB_MAX_NODES * 2];
    int ptype[3 + PWB_MAX_CLIENTS + PWB_MAX_NODES * 2];
    uint32_t pidx[3 + PWB_MAX_CLIENTS + PWB_MAX_NODES * 2];

    while (atomic_load(&brk->running)) {
        nfds_t n = 0;
        pthread_mutex_lock(&brk->lock);

        pfds[n].fd = brk->wake_efd; pfds[n].events = POLLIN; ptype[n] = 0; pidx[n] = 0; n++;
        if (brk->listen_fd >= 0) { pfds[n].fd = brk->listen_fd; pfds[n].events = POLLIN; ptype[n] = 1; pidx[n] = 0; n++; }
        if (brk->mgr_fd >= 0) { pfds[n].fd = brk->mgr_fd; pfds[n].events = POLLIN; ptype[n] = 1; pidx[n] = 1; n++; }

        for (uint32_t i = 0; i < PWB_MAX_CLIENTS; i++) {
            if (brk->clients[i].in_use && brk->clients[i].fd >= 0) {
                pfds[n].fd = brk->clients[i].fd;
                pfds[n].events = POLLIN | POLLHUP | POLLERR;
                ptype[n] = 2; pidx[n] = i; n++;
            }
        }
        for (uint32_t i = 0; i < PWB_MAX_NODES; i++) {
            if (brk->nodes[i].in_use && brk->nodes[i].done_efd >= 0) {
                pfds[n].fd = brk->nodes[i].done_efd;
                pfds[n].events = POLLIN;
                ptype[n] = (brk->nodes[i].direction == PWB_DIR_OUT) ? 3 : 4;
                pidx[n] = i; n++;
            }
        }
        pthread_mutex_unlock(&brk->lock);

        int pr = poll(pfds, n, 250);
        if (pr <= 0) continue;

        pthread_mutex_lock(&brk->lock);
        for (nfds_t k = 0; k < n; k++) {
            if (!pfds[k].revents) continue;
            if (ptype[k] == 0) {
                uint64_t dv = 0; ssize_t r = read(brk->wake_efd, &dv, 8); (void)r;
            } else if (ptype[k] == 1) {
                accept_unix_client(brk, pfds[k].fd);
            } else if (ptype[k] == 2) {
                pwb_client_t *c = &brk->clients[pidx[k]];
                if (c->in_use && ((pfds[k].revents & (POLLERR | POLLHUP)) || drain_client_socket(brk, c) < 0)) {
                    handle_client_disconnect(brk, c);
                }
            } else if (ptype[k] == 3 && brk->nodes[pidx[k]].in_use) {
                on_prod_done_event(brk, pidx[k]);
            } else if (ptype[k] == 4 && brk->nodes[pidx[k]].in_use) {
                on_cons_done_event(brk, pidx[k]);
            }
        }
        pthread_mutex_unlock(&brk->lock);
    }
    return NULL;
}

/* ============================================================================
 * Public Lifecycle, Sync Mode & Debugger Telemetry API
 * ============================================================================ */

void lwasapi_pw_preview_shm_name(char *out_buf, size_t out_len) {
    char dir[256];
    if (lwasapi_ensure_private_dir(dir, sizeof(dir)) && strlen(dir) + 12u < 64u && strlen(dir) + 12u < out_len) {
        snprintf(out_buf, out_len, "%s/pw_preview", dir);
        return;
    }
    snprintf(out_buf, out_len, "/lwasapi_pw_preview_%u", (unsigned)getuid());
}

static int apply_bind_mode(struct lwasapi_pw_broker *brk) {
    bool want_zero = pwb_zero_mode_configured(brk);
    atomic_store(&brk->zero_pw_mode, want_zero);

    if (!want_zero) {
        if (brk->listen_fd >= 0) { close(brk->listen_fd); brk->listen_fd = -1; unlink(brk->socket_path); }
        if (brk->mgr_fd >= 0) { close(brk->mgr_fd); brk->mgr_fd = -1; unlink(brk->manager_socket_path); }
        atomic_store(&brk->broker_listening, false);
        return 0;
    }

    if (brk->listen_fd >= 0) return 0;

    if (!brk->ignore_zero_flag && socket_actively_served_by_upstream(brk->socket_path)) {
        lwasapi_debug_emit(LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_WARN, (uint32_t)getpid(), 0,
                           "PW_BROKER_DEFER", brk->socket_path,
                           "upstream pipewire process still holding %s", brk->socket_path);
        return -EBUSY;
    }

    brk->listen_fd = bind_unix_sock(brk->socket_path);
    brk->mgr_fd = bind_unix_sock(brk->manager_socket_path);
    bool ok = (brk->listen_fd >= 0);
    atomic_store(&brk->broker_listening, ok);
    if (ok) {
        lwasapi_debug_emit(LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_INFO, (uint32_t)getpid(), 0,
                           "PW_BROKER_LISTEN", brk->socket_path,
                           "Built-in Video-Only pipewire-0 ScreenCast broker ready (mgr=%s)",
                           brk->manager_socket_path);
    }
    return ok ? 0 : -1;
}

int lwasapi_pw_broker_start(lwasapi_pw_broker_t **out_brk, const lwasapi_pw_broker_opts_t *opts) {
    if (!out_brk) return -EINVAL;
    struct lwasapi_pw_broker *brk = (struct lwasapi_pw_broker *)calloc(1, sizeof(*brk));
    if (!brk) return -ENOMEM;

    pthread_mutex_init(&brk->lock, NULL);
    brk->listen_fd = -1;
    brk->mgr_fd = -1;
    brk->wake_efd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    brk->next_global_id = 10u;
    brk->next_serial = 100u;
    brk->preview_shm_fd = -1;
    for (int i = 0; i < PWB_MAX_CLIENTS; i++) brk->clients[i].fd = -1;

    const char *rdir = (opts && opts->runtime_dir) ? opts->runtime_dir : getenv("LWASAPI_PW_TEST_DIR");
    if (!rdir || !rdir[0]) rdir = getenv("XDG_RUNTIME_DIR");
    char fb_rdir[64];
    if (!rdir || !rdir[0]) {
        snprintf(fb_rdir, sizeof(fb_rdir), "/run/user/%u", (unsigned)getuid());
        rdir = fb_rdir;
    }
    const char *sname = (opts && opts->socket_name) ? opts->socket_name : getenv("LWASAPI_PW_SOCKET_NAME");
    if (!sname || !sname[0]) sname = "pipewire-0";

    pwb_safe_copy(brk->runtime_dir, sizeof(brk->runtime_dir), rdir);
    snprintf(brk->socket_path, sizeof(brk->socket_path), "%s/%s", brk->runtime_dir, sname);
    snprintf(brk->manager_socket_path, sizeof(brk->manager_socket_path), "%s/%s-manager", brk->runtime_dir, sname);
    brk->ignore_zero_flag = (opts && opts->ignore_zero_flag) || (getenv("LWASAPI_PW_SOCKET_NAME") != NULL);

    /* Remove any legacy /dev/shm preview file and create private 0600 preview backing */
    char legacy_shm[64];
    snprintf(legacy_shm, sizeof(legacy_shm), "/lwasapi_pw_preview_%u", (unsigned)getuid());
    shm_unlink(legacy_shm);

    lwasapi_pw_preview_shm_name(brk->preview_shm_name, sizeof(brk->preview_shm_name));
    if (strncmp(brk->preview_shm_name, "/lwasapi_pw_preview_", 20) == 0) {
        shm_unlink(brk->preview_shm_name);
        brk->preview_shm_fd = shm_open(brk->preview_shm_name, O_CREAT | O_EXCL | O_RDWR, 0600);
    } else {
        unlink(brk->preview_shm_name);
        brk->preview_shm_fd = open(brk->preview_shm_name, O_CREAT | O_EXCL | O_RDWR | O_CLOEXEC, 0600);
    }
    if (brk->preview_shm_fd >= 0 && ftruncate(brk->preview_shm_fd, sizeof(lwasapi_pw_preview_shm_t)) == 0) {
        void *mp = mmap(NULL, sizeof(lwasapi_pw_preview_shm_t), PROT_READ | PROT_WRITE, MAP_SHARED, brk->preview_shm_fd, 0);
        if (mp != MAP_FAILED) {
            brk->preview_shm = (lwasapi_pw_preview_shm_t *)mp;
            memset(brk->preview_shm, 0, offsetof(lwasapi_pw_preview_shm_t, pixels));
            brk->preview_shm->magic = LWASAPI_PW_PREVIEW_MAGIC;
            brk->preview_shm->version = LWASAPI_PW_PREVIEW_VERSION;
        }
    }

    pthread_mutex_lock(&brk->lock);
    apply_bind_mode(brk);
    pthread_mutex_unlock(&brk->lock);

    atomic_store(&brk->running, true);
    pthread_create(&brk->thread, NULL, broker_thread_main, brk);
    *out_brk = brk;
    return 0;
}

void lwasapi_pw_broker_stop(lwasapi_pw_broker_t *brk) {
    if (!brk) return;
    atomic_store(&brk->running, false);
    if (brk->wake_efd >= 0) { uint64_t one = 1u; ssize_t wr = write(brk->wake_efd, &one, 8); (void)wr; }
    pthread_join(brk->thread, NULL);

    pthread_mutex_lock(&brk->lock);
    for (int i = 0; i < PWB_MAX_CLIENTS; i++) {
        if (brk->clients[i].in_use) handle_client_disconnect(brk, &brk->clients[i]);
    }
    if (brk->listen_fd >= 0) { close(brk->listen_fd); unlink(brk->socket_path); }
    if (brk->mgr_fd >= 0) { close(brk->mgr_fd); unlink(brk->manager_socket_path); }
    if (brk->wake_efd >= 0) close(brk->wake_efd);
    if (brk->preview_shm && brk->preview_shm != MAP_FAILED) {
        munmap(brk->preview_shm, sizeof(lwasapi_pw_preview_shm_t));
    }
    if (brk->preview_shm_fd >= 0) close(brk->preview_shm_fd);
    if (strncmp(brk->preview_shm_name, "/lwasapi_pw_preview_", 20) == 0) {
        shm_unlink(brk->preview_shm_name);
    } else if (brk->preview_shm_name[0]) {
        unlink(brk->preview_shm_name);
    }
    pthread_mutex_unlock(&brk->lock);
    pthread_mutex_destroy(&brk->lock);
    free(brk);
}

int lwasapi_pw_broker_sync_mode(lwasapi_pw_broker_t *brk) {
    if (!brk) return -EINVAL;
    pthread_mutex_lock(&brk->lock);
    int rc = apply_bind_mode(brk);
    pthread_mutex_unlock(&brk->lock);
    if (brk->wake_efd >= 0) { uint64_t one = 1u; ssize_t wr = write(brk->wake_efd, &one, 8); (void)wr; }
    return rc;
}

void lwasapi_pw_broker_set_preview_ex(lwasapi_pw_broker_t *brk, bool enabled, uint32_t ttl_ms,
                                      uint32_t max_w, uint32_t max_h, uint32_t target_fps) {
    if (!brk || !brk->preview_shm) return;
    if (max_w > LWASAPI_PW_PREVIEW_MAX_W) max_w = LWASAPI_PW_PREVIEW_MAX_W;
    if (max_h > LWASAPI_PW_PREVIEW_MAX_H) max_h = LWASAPI_PW_PREVIEW_MAX_H;
    if (target_fps > 240u) target_fps = 240u;
    atomic_store(&brk->preview_target_w, max_w);
    atomic_store(&brk->preview_target_h, max_h);
    atomic_store(&brk->preview_target_fps, target_fps);
    uint64_t now = pwb_mono_ns();
    if (enabled) {
        uint64_t ttl_ns = (uint64_t)(ttl_ms ? ttl_ms : 15000u) * 1000000ULL;
        atomic_store(&brk->preview_expiry_ns, now + ttl_ns);
        __atomic_store_n(&brk->preview_shm->last_gui_ping_ns, now, __ATOMIC_RELAXED);
        __atomic_store_n(&brk->preview_shm->preview_requested, 1u, __ATOMIC_RELEASE);
    } else {
        atomic_store(&brk->preview_expiry_ns, 0ULL);
        __atomic_store_n(&brk->preview_shm->preview_requested, 0u, __ATOMIC_RELEASE);
    }
}

void lwasapi_pw_broker_set_preview(lwasapi_pw_broker_t *brk, bool enabled, uint32_t ttl_ms) {
    lwasapi_pw_broker_set_preview_ex(brk, enabled, ttl_ms, 0u, 0u, 0u);
}

void lwasapi_pw_broker_set_cast_limits(lwasapi_pw_broker_t *brk,
                                        uint32_t max_w, uint32_t max_h, uint32_t max_fps) {
    if (!brk) return;
    if (max_fps > 240u) max_fps = 240u;
    atomic_store(&brk->cast_limit_w, max_w);
    atomic_store(&brk->cast_limit_h, max_h);
    atomic_store(&brk->cast_limit_fps, max_fps);
}

void lwasapi_pw_broker_fill_debug(lwasapi_pw_broker_t *brk, lwasapi_dbg_pw_snapshot_t *out) {
    if (!out) return;
    memset(out, 0, sizeof(*out));
    if (!brk) return;

    pthread_mutex_lock(&brk->lock);
    out->broker_running = atomic_load(&brk->broker_listening) ? 1u : 0u;
    out->zero_pw_mode = atomic_load(&brk->zero_pw_mode) ? 1u : 0u;
    out->preview_requested = (brk->preview_shm && __atomic_load_n(&brk->preview_shm->preview_requested, __ATOMIC_RELAXED)) ? 1u : 0u;
    pwb_safe_copy(out->socket_path, sizeof(out->socket_path), brk->socket_path);
    pwb_safe_copy(out->manager_socket_path, sizeof(out->manager_socket_path), brk->manager_socket_path);
    pwb_safe_copy(out->preview_shm_name, sizeof(out->preview_shm_name), brk->preview_shm_name);
    out->frames_relayed_total = atomic_load(&brk->frames_relayed_total);
    out->frames_dropped = atomic_load(&brk->frames_dropped);
    out->audio_rejects = atomic_load(&brk->audio_rejects);
    out->preview_target_w = atomic_load(&brk->preview_target_w);
    out->preview_target_h = atomic_load(&brk->preview_target_h);
    out->preview_target_fps = atomic_load(&brk->preview_target_fps);
    out->cast_limit_w = atomic_load(&brk->cast_limit_w);
    out->cast_limit_h = atomic_load(&brk->cast_limit_h);
    out->cast_limit_fps = atomic_load(&brk->cast_limit_fps);
    out->actual_fps_x100 = atomic_load(&brk->preview_actual_fps_x100_pub);

    for (uint32_t i = 0; i < PWB_MAX_CLIENTS && out->num_clients < LWASAPI_DEBUG_MAX_PW_CLIENTS; i++) {
        const pwb_client_t *c = &brk->clients[i];
        if (!c->in_use) continue;
        lwasapi_dbg_pw_client_t *dc = &out->clients[out->num_clients++];
        dc->id = c->global_id;
        dc->pid = c->pid;
        pwb_safe_copy(dc->app_name, sizeof(dc->app_name), c->app_name);
        pwb_safe_copy(dc->binary, sizeof(dc->binary), c->binary);
        pwb_safe_copy(dc->portal_app_id, sizeof(dc->portal_app_id), c->portal_app_id);
        dc->is_portal = c->is_portal ? 1u : 0u;
        for (uint32_t n = 0; n < PWB_MAX_NODES; n++) {
            if (brk->nodes[n].in_use && brk->nodes[n].client_idx == i) dc->num_nodes++;
        }
    }

    for (uint32_t i = 0; i < PWB_MAX_NODES && out->num_nodes < LWASAPI_DEBUG_MAX_PW_NODES; i++) {
        const pwb_node_t *nd = &brk->nodes[i];
        if (!nd->in_use) continue;
        lwasapi_dbg_pw_node_t *dn = &out->nodes[out->num_nodes++];
        dn->id = nd->global_id;
        const pwb_client_t *ow = &brk->clients[nd->client_idx];
        dn->client_id = ow->in_use ? ow->global_id : 0u;
        dn->client_pid = ow->in_use ? ow->pid : 0u;
        pwb_safe_copy(dn->name, sizeof(dn->name), nd->name);
        pwb_safe_copy(dn->media_class, sizeof(dn->media_class), nd->media_class);
        pwb_safe_copy(dn->format_name, sizeof(dn->format_name),
                      nd->neg_format ? pwb_vfmt_name(nd->neg_format) : "negotiating");
        pwb_safe_copy(dn->mem_type, sizeof(dn->mem_type),
                      nd->neg_w ? (nd->neg_is_dmabuf ? "DmaBuf" : "MemFd") : "idle");
        dn->direction = nd->direction;
        dn->state = nd->state;
        dn->is_driver = nd->is_driver ? 1u : 0u;
        dn->alloc_buffers = nd->alloc_buffers ? 1u : 0u;
        dn->width = nd->neg_w;
        dn->height = nd->neg_h;
        dn->fps_num = nd->neg_fps_num;
        dn->fps_denom = nd->neg_fps_denom;
        dn->n_buffers = nd->n_bufs;
        dn->drm_modifier = nd->neg_modifier;
        dn->frames = nd->frames_count;
        dn->target_node_id = nd->target_id_req;
        if (nd->neg_w && nd->direction == PWB_DIR_OUT) {
            if (nd->neg_is_dmabuf) out->dmabuf_streams++;
            else out->memfd_streams++;
        }
        (void)node_state_name;
    }

    for (uint32_t i = 0; i < PWB_MAX_LINKS && out->num_links < LWASAPI_DEBUG_MAX_PW_LINKS; i++) {
        const pwb_link_t *lk = &brk->links[i];
        if (!lk->in_use) continue;
        lwasapi_dbg_pw_link_t *dl = &out->links[out->num_links++];
        const pwb_node_t *on = &brk->nodes[lk->out_node_idx];
        const pwb_node_t *in = &brk->nodes[lk->in_node_idx];
        dl->id = lk->global_id;
        dl->out_node_id = on->global_id;
        dl->in_node_id = in->global_id;
        dl->state = lk->state;
        dl->is_dmabuf = on->neg_is_dmabuf ? 1u : 0u;
        dl->frames_relayed = lk->frames_relayed;
        if (lk->state == 4) out->active_casting = 1u;
        snprintf(dl->format_summary, sizeof(dl->format_summary), "%ux%u@%u %s (%s)",
                 on->neg_w, on->neg_h, on->neg_fps_denom ? (on->neg_fps_num / on->neg_fps_denom) : 60u,
                 pwb_vfmt_name(on->neg_format), on->neg_is_dmabuf ? "DmaBuf" : "MemFd");
    }
    pthread_mutex_unlock(&brk->lock);
}

/* ============================================================================
 * Pure-C Wire-Protocol Loopback Self-Test (Zero libpipewire Needed!)
 * ============================================================================ */

static int self_test_send(int fd, uint32_t id, uint8_t op, uint32_t seq, const void *pay, uint32_t sz) {
    size_t pad = pwb_pad8(sz);
    uint8_t *pkt = (uint8_t *)calloc(1, 16u + pad);
    if (!pkt) return -1;
    uint32_t h[4] = { id, (((uint32_t)op & 0xffu) << 24) | (sz & 0x00ffffffu), seq, 0 };
    memcpy(pkt, h, 16);
    if (sz && pay) memcpy(pkt + 16, pay, sz);
    ssize_t w = send(fd, pkt, 16u + pad, MSG_NOSIGNAL);
    free(pkt);
    return (w == (ssize_t)(16u + pad)) ? 0 : -1;
}

int lwasapi_pw_broker_self_test(lwasapi_pw_broker_t *brk, char *out_msg, size_t out_len) {
    if (!brk || !atomic_load(&brk->broker_listening)) {
        snprintf(out_msg, out_len, "Video broker socket is not listening (Zero-PipeWire mode inactive).");
        return -1;
    }

    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) { snprintf(out_msg, out_len, "socket failed: %s", strerror(errno)); return -1; }
    struct sockaddr_un addr = {0};
    addr.sun_family = AF_UNIX;
    pwb_safe_copy(addr.sun_path, sizeof(addr.sun_path), brk->socket_path);
    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        snprintf(out_msg, out_len, "connect(%s) failed: %s", brk->socket_path, strerror(errno));
        close(fd);
        return -1;
    }

    uint8_t buf[256]; pwb_pod_b_t b;
    /* 1. Send Core.Hello(4) */
    pb_init(&b, buf, sizeof(buf)); pb_push_struct(&b); pb_int(&b, 4); pb_pop(&b);
    self_test_send(fd, 0, PW_CORE_M_HELLO, 0, buf, (uint32_t)b.len);
    /* 2. Send Core.GetRegistry(3, 2) + Core.Sync(0, 77) */
    pb_init(&b, buf, sizeof(buf)); pb_push_struct(&b); pb_int(&b, 3); pb_int(&b, 2); pb_pop(&b);
    self_test_send(fd, 0, PW_CORE_M_GET_REGISTRY, 1, buf, (uint32_t)b.len);
    pb_init(&b, buf, sizeof(buf)); pb_push_struct(&b); pb_int(&b, 0); pb_int(&b, 77); pb_pop(&b);
    self_test_send(fd, 0, PW_CORE_M_SYNC, 2, buf, (uint32_t)b.len);

    struct pollfd pfd = { .fd = fd, .events = POLLIN };
    uint8_t rx[8192]; size_t rlen = 0;
    bool got_info = false, got_client_node_factory = false, got_done = false;
    uint64_t t0 = pwb_mono_ns();
    while (!got_done && pwb_mono_ns() - t0 < 1000000000ULL) {
        if (poll(&pfd, 1, 100) <= 0) continue;
        ssize_t n = recv(fd, rx + rlen, sizeof(rx) - rlen, 0);
        if (n <= 0) break;
        rlen += (size_t)n;
        size_t off = 0;
        while (rlen - off >= 16u) {
            uint32_t hdr[4]; memcpy(hdr, rx + off, 16);
            uint32_t psz = hdr[1] & 0x00ffffffu; uint8_t op = (uint8_t)(hdr[1] >> 24);
            size_t ptot = 16u + pwb_pad8(psz);
            if (rlen - off < ptot) break;
            if (hdr[0] == 0 && op == PW_CORE_E_INFO) got_info = true;
            if (hdr[0] == 2 && op == PW_REG_E_GLOBAL) {
                for (size_t k = 0; k + 26 < psz; k++) {
                    if (memcmp(rx + off + 16 + k, "PipeWire:Interface:ClientNode", 29) == 0)
                        got_client_node_factory = true;
                }
            }
            if (hdr[0] == 0 && op == PW_CORE_E_DONE) got_done = true;
            off += ptot;
        }
        if (off > 0) { memmove(rx, rx + off, rlen - off); rlen -= off; }
    }
    close(fd);

    if (got_info && got_client_node_factory && got_done) {
        snprintf(out_msg, out_len,
                 "PASS: pipewire-0 native handshake + ClientNode factory + Sync OK (%s)",
                 brk->socket_path);
        return 0;
    }
    snprintf(out_msg, out_len,
             "FAIL: info=%d factory=%d done=%d on %s",
             got_info, got_client_node_factory, got_done, brk->socket_path);
    return -1;
}
