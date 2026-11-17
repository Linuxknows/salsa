#define _GNU_SOURCE
#include "../include/lwasapi_debug.h"
#include "../include/lwasapi_paths.h"
#include "../include/salsa_wire.h"

#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#define DBG_RING_CAP       2048U
#define DBG_HISTORY_CAP    512U
#define DBG_MAX_CLIENTS    16
#define DBG_CLIENT_MAX_OUT (1024U * 1024U)
#define DBG_TELEM_INTERVAL_MS 200ULL

static char g_dbg_sock_path[108]    = LWASAPI_DEBUG_SOCK_DEFAULT;
static char g_dbg_in_sock_path[108] = LWASAPI_DEBUG_IN_SOCK_DEFAULT;

static bool dbg_peer_uid_allowed(uid_t peer_uid) {
    if (peer_uid == getuid() || peer_uid == 0) return true;
    const char *allow_all = getenv("LWASAPI_DEBUG_ALLOW_ALL");
    return (allow_all && strcmp(allow_all, "1") == 0);
}

typedef struct {
    uint64_t id;
    uint64_t ts_mono_ns;
    int64_t  ts_real_ms;
    uint32_t pid;
    uint32_t sid;
    int32_t  v1;
    int32_t  v2;
    int32_t  hr;
    uint8_t  sub;
    uint8_t  lvl;
    char     op[32];
    char     target[64];
    char     detail[192];
} dbg_event_t;

typedef struct {
    _Atomic uint64_t turn;
    dbg_event_t      ev;
} dbg_slot_t;

typedef struct {
    int    fd;
    char   inbuf[512];
    size_t inlen;
    char  *outbuf;
    size_t outlen;
    size_t outcap;
} dbg_client_t;

typedef struct {
    char  *data;
    size_t len;
    size_t cap;
} dbg_jbuf_t;

/* Global RT-safe event ring */
static dbg_slot_t         g_ring[DBG_RING_CAP];
static _Atomic uint64_t   g_ring_head = 0;
static uint64_t           g_ring_tail = 0; /* owned solely by debug thread */
static _Atomic uint64_t   g_next_ev_id = 1;
static _Atomic uint64_t   g_dropped_events = 0;

/* Atomic RT telemetry */
static _Atomic uint64_t   g_rt_cycle_count   = 0;
static _Atomic uint64_t   g_rt_last_cycle_ns = 0;
static _Atomic uint64_t   g_rt_avg_cycle_ns  = 0;
static _Atomic uint64_t   g_rt_max_cycle_ns  = 0;
static _Atomic uint64_t   g_rt_min_cycle_ns  = UINT64_MAX;
static _Atomic int64_t    g_rt_last_jitter_ns = 0;
static _Atomic uint64_t   g_rt_max_jitter_ns = 0;
static _Atomic uint64_t   g_rt_eq_ns         = 0;
static _Atomic uint64_t   g_rt_pa_mix_ns     = 0;
static _Atomic uint64_t   g_rt_hw_write_ns   = 0;
static _Atomic uint64_t   g_rt_budget_ns     = 0;
static _Atomic uint64_t   g_rt_hw_xruns      = 0;

/* Server lifecycle & state */
static _Atomic int        g_dbg_running = 0;
static pthread_t          g_dbg_thread;
static lwasapi_dbg_callbacks_t g_cbs;
static int                g_listen_fd = -1;
static int                g_dgram_fd  = -1;

/* CFS-thread-only partitioned replay history (keeps INFO/WARN/ERR across TRACE bursts) */
typedef struct {
    uint64_t id;
    char    *line;
} dbg_hist_entry_t;

static dbg_hist_entry_t   g_imp_hist[DBG_HISTORY_CAP];
static size_t             g_imp_head = 0;
static size_t             g_imp_count = 0;
static dbg_hist_entry_t   g_trace_hist[DBG_HISTORY_CAP];
static size_t             g_trace_head = 0;
static size_t             g_trace_count = 0;
static dbg_client_t       g_clients[DBG_MAX_CLIENTS];

static inline uint64_t dbg_mono_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static inline int64_t dbg_real_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec * 1000LL + (int64_t)(ts.tv_nsec / 1000000L);
}

static const char *dbg_sub_name(uint8_t sub) {
    switch (sub) {
        case LWASAPI_DBG_ENGINE:   return "ENGINE";
        case LWASAPI_DBG_WASAPI:   return "WASAPI";
        case LWASAPI_DBG_PA:       return "PA";
        case LWASAPI_DBG_BLUEZ:    return "BLUEZ";
        case LWASAPI_DBG_MIDI:     return "MIDI";
        case LWASAPI_DBG_SHIM:     return "SHIM";
        case LWASAPI_DBG_ALSA:     return "ALSA";
        case LWASAPI_DBG_PW_VIDEO: return "PW_VIDEO";
        default:                   return "UNKNOWN";
    }
}

static const char *dbg_lvl_name(uint8_t lvl) {
    switch (lvl) {
        case LWASAPI_DBG_TRACE: return "TRACE";
        case LWASAPI_DBG_INFO:  return "INFO";
        case LWASAPI_DBG_WARN:  return "WARN";
        case LWASAPI_DBG_ERR:   return "ERR";
        default:                return "INFO";
    }
}

static inline void safe_cstr_copy(char *dst, size_t dst_sz, const char *src) {
    if (!dst || dst_sz == 0) return;
    if (!src) {
        dst[0] = '\0';
        return;
    }
    size_t i = 0;
    while (i + 1 < dst_sz && src[i] != '\0') {
        dst[i] = src[i];
        i++;
    }
    dst[i] = '\0';
}

/* ============================================================================
 * RT-Safe Lock-Free Event Emission & Stats Recording
 * ========================================================================== */

static void ring_push_event(const dbg_event_t *src_ev) {
    uint64_t pos = atomic_load_explicit(&g_ring_head, memory_order_relaxed);
    for (;;) {
        dbg_slot_t *slot = &g_ring[pos & (DBG_RING_CAP - 1U)];
        uint64_t turn = atomic_load_explicit(&slot->turn, memory_order_acquire);
        int64_t diff = (int64_t)(turn - pos);
        if (diff == 0) {
            if (atomic_compare_exchange_weak_explicit(
                    &g_ring_head, &pos, pos + 1U,
                    memory_order_relaxed, memory_order_relaxed)) {
                slot->ev = *src_ev;
                slot->ev.id = atomic_fetch_add_explicit(&g_next_ev_id, 1U, memory_order_relaxed);
                atomic_store_explicit(&slot->turn, pos + 1U, memory_order_release);
                return;
            }
        } else if (diff < 0) {
            atomic_fetch_add_explicit(&g_dropped_events, 1U, memory_order_relaxed);
            return;
        } else {
            pos = atomic_load_explicit(&g_ring_head, memory_order_relaxed);
        }
    }
}

void lwasapi_debug_emit_vals(uint8_t sub, uint8_t lvl, uint32_t pid, uint32_t sid,
                             int32_t v1, int32_t v2, int32_t hr,
                             const char *op, const char *target, const char *fmt, ...) {
    if (!atomic_load_explicit(&g_dbg_running, memory_order_relaxed)) return;

    dbg_event_t ev;
    ev.id = 0;
    ev.ts_mono_ns = dbg_mono_ns();
    ev.ts_real_ms = dbg_real_ms();
    ev.pid = pid;
    ev.sid = sid;
    ev.v1 = v1;
    ev.v2 = v2;
    ev.hr = hr;
    ev.sub = sub;
    ev.lvl = lvl;
    safe_cstr_copy(ev.op, sizeof(ev.op), op);
    safe_cstr_copy(ev.target, sizeof(ev.target), target);

    if (fmt && fmt[0] != '\0') {
        va_list ap;
        va_start(ap, fmt);
        vsnprintf(ev.detail, sizeof(ev.detail), fmt, ap);
        va_end(ap);
        ev.detail[sizeof(ev.detail) - 1] = '\0';
    } else {
        ev.detail[0] = '\0';
    }

    ring_push_event(&ev);
}

void lwasapi_debug_emit(uint8_t sub, uint8_t lvl, uint32_t pid, uint32_t sid,
                        const char *op, const char *target, const char *fmt, ...) {
    if (!atomic_load_explicit(&g_dbg_running, memory_order_relaxed)) return;

    dbg_event_t ev;
    ev.id = 0;
    ev.ts_mono_ns = dbg_mono_ns();
    ev.ts_real_ms = dbg_real_ms();
    ev.pid = pid;
    ev.sid = sid;
    ev.v1 = 0;
    ev.v2 = 0;
    ev.hr = 0;
    ev.sub = sub;
    ev.lvl = lvl;
    safe_cstr_copy(ev.op, sizeof(ev.op), op);
    safe_cstr_copy(ev.target, sizeof(ev.target), target);

    if (fmt && fmt[0] != '\0') {
        va_list ap;
        va_start(ap, fmt);
        vsnprintf(ev.detail, sizeof(ev.detail), fmt, ap);
        va_end(ap);
        ev.detail[sizeof(ev.detail) - 1] = '\0';
    } else {
        ev.detail[0] = '\0';
    }

    ring_push_event(&ev);
}

void lwasapi_debug_record_rt_period(uint64_t cycle_ns, int64_t wake_jitter_ns,
                                    uint64_t budget_ns, uint64_t eq_ns,
                                    uint64_t pa_mix_ns, uint64_t hw_write_ns) {
    uint64_t c = atomic_fetch_add_explicit(&g_rt_cycle_count, 1U, memory_order_relaxed) + 1U;
    atomic_store_explicit(&g_rt_last_cycle_ns, cycle_ns, memory_order_relaxed);
    atomic_store_explicit(&g_rt_last_jitter_ns, wake_jitter_ns, memory_order_relaxed);
    atomic_store_explicit(&g_rt_budget_ns, budget_ns, memory_order_relaxed);
    atomic_store_explicit(&g_rt_eq_ns, eq_ns, memory_order_relaxed);
    atomic_store_explicit(&g_rt_pa_mix_ns, pa_mix_ns, memory_order_relaxed);
    atomic_store_explicit(&g_rt_hw_write_ns, hw_write_ns, memory_order_relaxed);

    uint64_t prev_avg = atomic_load_explicit(&g_rt_avg_cycle_ns, memory_order_relaxed);
    uint64_t new_avg = (c == 1U || prev_avg == 0U)
        ? cycle_ns
        : ((prev_avg * 31ULL + cycle_ns) >> 5);
    atomic_store_explicit(&g_rt_avg_cycle_ns, new_avg, memory_order_relaxed);

    uint64_t prev_max = atomic_load_explicit(&g_rt_max_cycle_ns, memory_order_relaxed);
    if (cycle_ns > prev_max) {
        atomic_store_explicit(&g_rt_max_cycle_ns, cycle_ns, memory_order_relaxed);
    }

    uint64_t prev_min = atomic_load_explicit(&g_rt_min_cycle_ns, memory_order_relaxed);
    if (cycle_ns < prev_min) {
        atomic_store_explicit(&g_rt_min_cycle_ns, cycle_ns, memory_order_relaxed);
    }

    uint64_t abs_jit = (wake_jitter_ns < 0)
        ? (uint64_t)(-(wake_jitter_ns + 1)) + 1ULL
        : (uint64_t)wake_jitter_ns;
    uint64_t prev_max_j = atomic_load_explicit(&g_rt_max_jitter_ns, memory_order_relaxed);
    if (abs_jit > prev_max_j) {
        atomic_store_explicit(&g_rt_max_jitter_ns, abs_jit, memory_order_relaxed);
    }
}

void lwasapi_debug_get_rt_stats(lwasapi_dbg_rt_stats_t *out_rt) {
    if (!out_rt) return;
    memset(out_rt, 0, sizeof(*out_rt));
    uint64_t count = atomic_load_explicit(&g_rt_cycle_count, memory_order_relaxed);
    out_rt->cycle_count    = count;
    out_rt->last_cycle_ns  = atomic_load_explicit(&g_rt_last_cycle_ns, memory_order_relaxed);
    out_rt->avg_cycle_ns   = atomic_load_explicit(&g_rt_avg_cycle_ns, memory_order_relaxed);
    out_rt->max_cycle_ns   = atomic_load_explicit(&g_rt_max_cycle_ns, memory_order_relaxed);
    uint64_t min_v         = atomic_load_explicit(&g_rt_min_cycle_ns, memory_order_relaxed);
    out_rt->min_cycle_ns   = (count > 0 && min_v != UINT64_MAX) ? min_v : 0ULL;
    out_rt->last_jitter_ns = atomic_load_explicit(&g_rt_last_jitter_ns, memory_order_relaxed);
    out_rt->max_jitter_ns  = atomic_load_explicit(&g_rt_max_jitter_ns, memory_order_relaxed);
    out_rt->eq_ns          = atomic_load_explicit(&g_rt_eq_ns, memory_order_relaxed);
    out_rt->pa_mix_ns      = atomic_load_explicit(&g_rt_pa_mix_ns, memory_order_relaxed);
    out_rt->hw_write_ns    = atomic_load_explicit(&g_rt_hw_write_ns, memory_order_relaxed);
    out_rt->budget_ns      = atomic_load_explicit(&g_rt_budget_ns, memory_order_relaxed);
    out_rt->hw_xruns       = atomic_load_explicit(&g_rt_hw_xruns, memory_order_relaxed);
}

void lwasapi_debug_reset_rt_stats(void) {
    atomic_store_explicit(&g_rt_cycle_count, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_last_cycle_ns, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_avg_cycle_ns, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_max_cycle_ns, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_min_cycle_ns, UINT64_MAX, memory_order_relaxed);
    atomic_store_explicit(&g_rt_last_jitter_ns, 0LL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_max_jitter_ns, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_eq_ns, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_pa_mix_ns, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_hw_write_ns, 0ULL, memory_order_relaxed);
    atomic_store_explicit(&g_rt_hw_xruns, 0ULL, memory_order_relaxed);
}

/* ============================================================================
 * CFS-Thread JSON Builder & Network Helpers
 * ========================================================================== */

static int jbuf_ensure(dbg_jbuf_t *jb, size_t add) {
    if (jb->len + add + 1U <= jb->cap) return 0;
    size_t new_cap = jb->cap ? jb->cap : 1024U;
    while (new_cap < jb->len + add + 1U) {
        new_cap *= 2U;
    }
    char *p = (char *)realloc(jb->data, new_cap);
    if (!p) return -1;
    jb->data = p;
    jb->cap = new_cap;
    return 0;
}

static void jbuf_append_raw(dbg_jbuf_t *jb, const char *s, size_t n) {
    if (!s || n == 0) return;
    if (jbuf_ensure(jb, n) < 0) return;
    memcpy(jb->data + jb->len, s, n);
    jb->len += n;
    jb->data[jb->len] = '\0';
}

static void jbuf_append_cstr(dbg_jbuf_t *jb, const char *s) {
    if (!s) return;
    jbuf_append_raw(jb, s, strlen(s));
}

static void jbuf_appendf(dbg_jbuf_t *jb, const char *fmt, ...)
    __attribute__((format(printf, 2, 3)));

static void jbuf_appendf(dbg_jbuf_t *jb, const char *fmt, ...) {
    va_list ap, ap2;
    va_start(ap, fmt);
    va_copy(ap2, ap);
    int needed = vsnprintf(NULL, 0, fmt, ap);
    va_end(ap);
    if (needed < 0) {
        va_end(ap2);
        return;
    }
    if (jbuf_ensure(jb, (size_t)needed) == 0) {
        vsnprintf(jb->data + jb->len, jb->cap - jb->len, fmt, ap2);
        jb->len += (size_t)needed;
    }
    va_end(ap2);
}

static void jbuf_append_json_str(dbg_jbuf_t *jb, const char *s) {
    jbuf_append_raw(jb, "\"", 1);
    if (s) {
        for (const unsigned char *p = (const unsigned char *)s; *p != '\0'; p++) {
            unsigned char c = *p;
            switch (c) {
                case '"':  jbuf_append_raw(jb, "\\\"", 2); break;
                case '\\': jbuf_append_raw(jb, "\\\\", 2); break;
                case '\b': jbuf_append_raw(jb, "\\b", 2);  break;
                case '\f': jbuf_append_raw(jb, "\\f", 2);  break;
                case '\n': jbuf_append_raw(jb, "\\n", 2);  break;
                case '\r': jbuf_append_raw(jb, "\\r", 2);  break;
                case '\t': jbuf_append_raw(jb, "\\t", 2);  break;
                default:
                    if (c < 0x20) {
                        jbuf_appendf(jb, "\\u%04x", (unsigned)c);
                    } else {
                        jbuf_append_raw(jb, (const char *)&c, 1);
                    }
                    break;
            }
        }
    }
    jbuf_append_raw(jb, "\"", 1);
}

static inline double finite_d(double v) {
    return isfinite(v) ? v : 0.0;
}

static void serialize_event_json(dbg_jbuf_t *jb, const dbg_event_t *ev) {
    jbuf_appendf(jb,
        "{\"type\":\"event\",\"id\":%llu,\"ts_mono_ns\":%llu,\"ts_ms\":%lld,"
        "\"sub\":",
        (unsigned long long)ev->id,
        (unsigned long long)ev->ts_mono_ns,
        (long long)ev->ts_real_ms);
    jbuf_append_json_str(jb, dbg_sub_name(ev->sub));
    jbuf_append_cstr(jb, ",\"lvl\":");
    jbuf_append_json_str(jb, dbg_lvl_name(ev->lvl));
    jbuf_appendf(jb, ",\"pid\":%u,\"sid\":%u,\"v1\":%d,\"v2\":%d,\"hr\":%d,\"op\":",
                 ev->pid, ev->sid, ev->v1, ev->v2, ev->hr);
    jbuf_append_json_str(jb, ev->op);
    jbuf_append_cstr(jb, ",\"target\":");
    jbuf_append_json_str(jb, ev->target);
    jbuf_append_cstr(jb, ",\"detail\":");
    jbuf_append_json_str(jb, ev->detail);
    jbuf_append_cstr(jb, "}");
}

static void serialize_snapshot_json(dbg_jbuf_t *jb, const lwasapi_dbg_snapshot_t *snap) {
    const lwasapi_engine_status_t *st = &snap->engine_st;
    const lwasapi_dbg_rt_stats_t  *rt = &snap->rt;
    double cpu_pct = (rt->budget_ns > 0)
        ? finite_d((double)rt->last_cycle_ns * 100.0 / (double)rt->budget_ns)
        : 0.0;

    jbuf_appendf(jb,
        "{\"type\":\"telemetry\",\"ts_ms\":%lld,\"ts_mono_ns\":%llu,"
        "\"dropped_events\":%llu,",
        (long long)dbg_real_ms(),
        (unsigned long long)dbg_mono_ns(),
        (unsigned long long)atomic_load_explicit(&g_dropped_events, memory_order_relaxed));

    /* engine summary */
    jbuf_append_cstr(jb, "\"engine\":{\"hw_device\":");
    jbuf_append_json_str(jb, st->hw_device);
    jbuf_append_cstr(jb, ",\"hw_capture_device\":");
    jbuf_append_json_str(jb, st->hw_capture_device);
    jbuf_append_cstr(jb, ",\"midi_device\":");
    jbuf_append_json_str(jb, st->midi_device);
    jbuf_append_cstr(jb, ",\"active_playback_id\":");
    jbuf_append_json_str(jb, st->active_playback_id[0] ? st->active_playback_id : snap->target_playback_id);
    jbuf_append_cstr(jb, ",\"active_capture_id\":");
    jbuf_append_json_str(jb, st->active_capture_id[0] ? st->active_capture_id : snap->target_capture_id);
    jbuf_appendf(jb,
        ",\"sample_rate\":%u,\"channels\":%u,\"period_frames\":%u,\"buffer_frames\":%u,"
        "\"format_bits\":%u,\"desktop_volume_pct\":%u,\"desktop_muted\":%u,"
        "\"desktop_mic_volume_pct\":%u,\"desktop_mic_muted\":%u,"
        "\"realtime_priority\":%u,\"exclusive_owner_id\":%d,"
        "\"total_cycles\":%llu,\"hw_xruns\":%llu,\"midi_events_routed\":%llu,"
        "\"eq_enabled\":%u,\"eq_dynamic_mode\":%u,\"eq_preamp_db\":%.2f,"
        "\"eq_dyn_threshold_db\":%.2f,\"eq_dyn_ratio\":%.2f,\"eq_limiter_enabled\":%u,"
        "\"output_peak_l\":%.4f,\"output_peak_r\":%.4f,"
        "\"pa_server_running\":%u,\"pa_clients\":%u,\"eq_preset\":",
        st->hw_sample_rate, st->hw_channels, st->hw_period_frames, st->hw_buffer_frames,
        st->hw_format_bits, st->desktop_volume_pct, st->desktop_muted,
        st->desktop_mic_volume_pct, st->desktop_mic_muted,
        st->realtime_priority, st->exclusive_owner_id,
        (unsigned long long)st->total_cycles,
        (unsigned long long)st->hw_xruns,
        (unsigned long long)st->midi_events_routed,
        st->eq_cfg.enabled, st->eq_cfg.dynamic_mode, finite_d(st->eq_cfg.preamp_db),
        finite_d(st->eq_cfg.dyn_threshold_db), finite_d(st->eq_cfg.dyn_ratio),
        st->eq_cfg.limiter_enabled,
        finite_d(st->output_peak_l), finite_d(st->output_peak_r),
        st->pa_server_running, st->pa_clients);
    jbuf_append_json_str(jb, st->eq_cfg.preset_name);
    jbuf_append_cstr(jb, "},");

    /* RT timing metrics */
    jbuf_appendf(jb,
        "\"rt\":{\"cycle_count\":%llu,\"last_cycle_us\":%.1f,\"avg_cycle_us\":%.1f,"
        "\"max_cycle_us\":%.1f,\"min_cycle_us\":%.1f,\"last_jitter_us\":%.1f,"
        "\"max_jitter_us\":%.1f,\"eq_us\":%.1f,\"pa_mix_us\":%.1f,\"hw_write_us\":%.1f,"
        "\"budget_us\":%.1f,\"cpu_load_pct\":%.2f},",
        (unsigned long long)rt->cycle_count,
        finite_d((double)rt->last_cycle_ns / 1000.0),
        finite_d((double)rt->avg_cycle_ns / 1000.0),
        finite_d((double)rt->max_cycle_ns / 1000.0),
        finite_d((double)rt->min_cycle_ns / 1000.0),
        finite_d((double)rt->last_jitter_ns / 1000.0),
        finite_d((double)rt->max_jitter_ns / 1000.0),
        finite_d((double)rt->eq_ns / 1000.0),
        finite_d((double)rt->pa_mix_ns / 1000.0),
        finite_d((double)rt->hw_write_ns / 1000.0),
        finite_d((double)rt->budget_ns / 1000.0),
        cpu_pct);

    /* BlueZ */
    jbuf_appendf(jb,
        "\"bluez\":{\"active\":%u,\"initialized\":%u,\"endpoints_registered\":%u,"
        "\"direct_socket\":%u,\"has_transport\":%u,\"tx_paused\":%u,"
        "\"sample_rate\":%u,\"channels\":%u,\"delay_ms\":%u,"
        "\"mtu_r\":%u,\"mtu_w\":%u,\"avrcp_vol\":%u,"
        "\"tx_ring_fill\":%llu,\"tx_ring_cap\":%u,"
        "\"tx_packets\":%llu,\"tx_underruns\":%llu,\"tx_errors\":%llu,"
        "\"alias\":",
        st->bt_active, snap->bt_initialized, snap->bt_endpoints_registered,
        snap->bt_direct_socket, snap->bt_has_transport, snap->bt_tx_paused,
        snap->bt_sample_rate, snap->bt_channels, st->bt_delay_ms,
        snap->bt_mtu_r, snap->bt_mtu_w, snap->bt_avrcp_vol,
        (unsigned long long)snap->bt_tx_ring_fill, snap->bt_tx_ring_cap,
        (unsigned long long)snap->bt_tx_packets,
        (unsigned long long)snap->bt_tx_underruns,
        (unsigned long long)snap->bt_tx_errors);
    jbuf_append_json_str(jb, st->bt_alias);
    jbuf_append_cstr(jb, ",\"codec\":");
    jbuf_append_json_str(jb, st->bt_codec);
    jbuf_append_cstr(jb, ",\"device_path\":");
    jbuf_append_json_str(jb, snap->bt_device_path);
    jbuf_append_cstr(jb, ",\"transport_path\":");
    jbuf_append_json_str(jb, snap->bt_transport_path);
    jbuf_append_cstr(jb, ",\"endpoint_path\":");
    jbuf_append_json_str(jb, snap->bt_endpoint_path);
    jbuf_append_cstr(jb, "},");

    /* Kernel ALSA Capture */
    jbuf_appendf(jb,
        "\"capture\":{\"active\":%u,\"hw_rate\":%u,\"hw_channels\":%u,"
        "\"hw_format\":%d,\"ring_fill\":%llu,\"pref_rate\":%u,"
        "\"measured_rate\":%u,\"kick_count\":%u,\"is_usb\":%u,"
        "\"duplex_locked\":%u,\"device_id\":",
        snap->cap_active, snap->cap_hw_rate, snap->cap_hw_channels,
        snap->cap_hw_format, (unsigned long long)snap->cap_ring_fill,
        snap->cap_pref_rate, snap->cap_measured_rate, snap->cap_kick_count,
        (unsigned)snap->cap_is_usb, (unsigned)snap->cap_duplex_locked);
    jbuf_append_json_str(jb, snap->cap_device_id);
    jbuf_append_cstr(jb, ",\"pcm_path\":");
    jbuf_append_json_str(jb, snap->cap_pcm_path);
    jbuf_append_cstr(jb, ",\"alias\":");
    jbuf_append_json_str(jb, snap->cap_alias);
    jbuf_append_cstr(jb, "},");

    /* MIDI Hub */
    jbuf_appendf(jb,
        "\"midi\":{\"initialized\":%u,\"rawmidi_open\":%u,\"seq_client\":%d,"
        "\"seq_in_port\":%d,\"events_routed\":%llu,\"controller\":",
        snap->midi_initialized, snap->midi_rawmidi_open,
        snap->midi_seq_client, snap->midi_seq_in_port,
        (unsigned long long)snap->midi_events_routed);
    jbuf_append_json_str(jb, snap->midi_controller);
    jbuf_append_cstr(jb, ",\"dev_path\":");
    jbuf_append_json_str(jb, snap->midi_dev_path);
    jbuf_append_cstr(jb, "},");

    /* Native WASAPI Sessions */
    jbuf_append_cstr(jb, "\"wasapi_sessions\":[");
    uint32_t nw = snap->num_wasapi;
    if (nw > LWASAPI_MAX_SESSIONS) nw = LWASAPI_MAX_SESSIONS;
    for (uint32_t i = 0; i < nw; i++) {
        const lwasapi_dbg_wasapi_session_t *ws = &snap->wasapi[i];
        if (i > 0) jbuf_append_raw(jb, ",", 1);
        jbuf_appendf(jb,
            "{\"session_id\":%u,\"pid\":%u,\"name\":",
            ws->session_id, ws->client_pid);
        jbuf_append_json_str(jb, ws->client_name);
        jbuf_appendf(jb,
            ",\"share_mode\":%u,\"stream_flags\":%u,\"is_capture\":%u,"
            "\"rate\":%u,\"channels\":%u,\"active\":%u,\"muted\":%u,"
            "\"volume_milli\":%u,\"buffer_frames\":%u,\"period_frames\":%u,"
            "\"buffered_frames\":%llu,\"xruns\":%llu}",
            ws->share_mode, ws->stream_flags, ws->is_capture,
            ws->sample_rate, ws->channels, ws->active, ws->muted,
            ws->volume_milli, ws->buffer_frames, ws->period_frames,
            (unsigned long long)ws->buffered_frames,
            (unsigned long long)ws->xruns);
    }
    jbuf_append_cstr(jb, "],");

    /* PulseAudio v35 clients */
    jbuf_append_cstr(jb, "\"pa_clients\":[");
    uint32_t nc = snap->pa.num_clients;
    if (nc > LWASAPI_DEBUG_MAX_PA_CLIENTS) nc = LWASAPI_DEBUG_MAX_PA_CLIENTS;
    for (uint32_t i = 0; i < nc; i++) {
        const lwasapi_dbg_pa_client_t *pc = &snap->pa.clients[i];
        if (i > 0) jbuf_append_raw(jb, ",", 1);
        jbuf_appendf(jb, "{\"index\":%u,\"pid\":%u,\"name\":", pc->index, pc->pid);
        jbuf_append_json_str(jb, pc->name);
        jbuf_append_cstr(jb, ",\"binary\":");
        jbuf_append_json_str(jb, pc->binary);
        jbuf_appendf(jb, ",\"version\":%u,\"authed\":%u,\"sub_mask\":%u,\"out_count\":%u}",
                     pc->version, pc->authed, pc->sub_mask, pc->out_count);
    }
    jbuf_append_cstr(jb, "],");

    /* PulseAudio v35 streams */
    jbuf_append_cstr(jb, "\"pa_streams\":[");
    uint32_t ns = snap->pa.num_streams;
    if (ns > LWASAPI_DEBUG_MAX_PA_STREAMS) ns = LWASAPI_DEBUG_MAX_PA_STREAMS;
    for (uint32_t i = 0; i < ns; i++) {
        const lwasapi_dbg_pa_stream_t *ps = &snap->pa.streams[i];
        if (i > 0) jbuf_append_raw(jb, ",", 1);
        jbuf_appendf(jb,
            "{\"index\":%u,\"client_index\":%u,\"pid\":%u,\"name\":",
            ps->index, ps->client_index, ps->pid);
        jbuf_append_json_str(jb, ps->name);
        jbuf_append_cstr(jb, ",\"device_id\":");
        jbuf_append_json_str(jb, ps->device_id);
        jbuf_appendf(jb,
            ",\"is_playback\":%u,\"is_monitor\":%u,\"peak_detect\":%u,"
            "\"direct_on_input\":%u,\"target_dev_idx\":%u,\"rate\":%u,"
            "\"channels\":%u,\"format\":%u,\"active\":%u,\"corked\":%u,"
            "\"in_prebuf\":%u,\"draining\":%u,\"muted\":%u,\"volume_milli\":%u,"
            "\"frame_size\":%u,\"buffered_frames\":%llu,\"ring_cap_frames\":%u,"
            "\"tlength_frames\":%u,\"prebuf_frames\":%u,\"minreq_frames\":%u,"
            "\"fragsize_frames\":%u,\"mix_peak_l\":%.4f,\"mix_peak_r\":%.4f,"
            "\"write_index\":%llu,\"read_index\":%llu}",
            ps->is_playback, ps->is_monitor, ps->peak_detect,
            ps->direct_on_input, ps->target_dev_idx, ps->rate,
            ps->channels, ps->format, ps->active, ps->corked,
            ps->in_prebuf, ps->draining, ps->muted, ps->volume_milli,
            ps->frame_size, (unsigned long long)ps->buffered_frames,
            ps->ring_cap_frames, ps->tlength_frames, ps->prebuf_frames,
            ps->minreq_frames, ps->fragsize_frames,
            finite_d(ps->mix_peak_l), finite_d(ps->mix_peak_r),
            (unsigned long long)ps->write_index,
            (unsigned long long)ps->read_index);
    }
    jbuf_append_cstr(jb, "]");
    jbuf_appendf(jb,
        ",\"pw\":{\"broker_running\":%u,\"zero_pw_mode\":%u,"
        "\"preview_requested\":%u,\"active_casting\":%u,"
        "\"frames_relayed_total\":%llu,\"frames_dropped\":%llu,"
        "\"dmabuf_streams\":%u,\"memfd_streams\":%u,\"audio_rejects\":%u,"
        "\"preview_target_w\":%u,\"preview_target_h\":%u,\"preview_target_fps\":%u,"
        "\"cast_limit_w\":%u,\"cast_limit_h\":%u,\"cast_limit_fps\":%u,\"actual_fps_x100\":%u,"
        "\"socket_path\":",
        snap->pw.broker_running, snap->pw.zero_pw_mode,
        snap->pw.preview_requested, snap->pw.active_casting,
        (unsigned long long)snap->pw.frames_relayed_total,
        (unsigned long long)snap->pw.frames_dropped,
        snap->pw.dmabuf_streams, snap->pw.memfd_streams, snap->pw.audio_rejects,
        snap->pw.preview_target_w, snap->pw.preview_target_h, snap->pw.preview_target_fps,
        snap->pw.cast_limit_w, snap->pw.cast_limit_h, snap->pw.cast_limit_fps,
        snap->pw.actual_fps_x100);
    jbuf_append_json_str(jb, snap->pw.socket_path);
    jbuf_append_cstr(jb, ",\"manager_socket_path\":");
    jbuf_append_json_str(jb, snap->pw.manager_socket_path);
    jbuf_append_cstr(jb, ",\"preview_shm_name\":");
    jbuf_append_json_str(jb, snap->pw.preview_shm_name);

    jbuf_append_cstr(jb, ",\"clients\":[");
    for (uint32_t i = 0; i < snap->pw.num_clients && i < LWASAPI_DEBUG_MAX_PW_CLIENTS; i++) {
        const lwasapi_dbg_pw_client_t *pc = &snap->pw.clients[i];
        if (i > 0) jbuf_append_cstr(jb, ",");
        jbuf_appendf(jb, "{\"id\":%u,\"pid\":%u,\"is_portal\":%u,\"num_nodes\":%u,\"app_name\":",
                     pc->id, pc->pid, pc->is_portal, pc->num_nodes);
        jbuf_append_json_str(jb, pc->app_name);
        jbuf_append_cstr(jb, ",\"binary\":");
        jbuf_append_json_str(jb, pc->binary);
        jbuf_append_cstr(jb, ",\"portal_app_id\":");
        jbuf_append_json_str(jb, pc->portal_app_id);
        jbuf_append_cstr(jb, "}");
    }

    jbuf_append_cstr(jb, "],\"nodes\":[");
    for (uint32_t i = 0; i < snap->pw.num_nodes && i < LWASAPI_DEBUG_MAX_PW_NODES; i++) {
        const lwasapi_dbg_pw_node_t *pn = &snap->pw.nodes[i];
        if (i > 0) jbuf_append_cstr(jb, ",");
        jbuf_appendf(jb,
            "{\"id\":%u,\"client_id\":%u,\"client_pid\":%u,"
            "\"target_node_id\":%u,\"linked_node_id\":%u,"
            "\"direction\":%u,\"state\":%u,\"is_driver\":%u,\"alloc_buffers\":%u,"
            "\"width\":%u,\"height\":%u,\"fps_num\":%u,\"fps_denom\":%u,"
            "\"n_buffers\":%u,\"drm_modifier\":\"0x%llx\",\"frames\":%llu,\"name\":",
            pn->id, pn->client_id, pn->client_pid,
            pn->target_node_id, pn->linked_node_id,
            pn->direction, pn->state, pn->is_driver, pn->alloc_buffers,
            pn->width, pn->height, pn->fps_num, pn->fps_denom,
            pn->n_buffers, (unsigned long long)pn->drm_modifier,
            (unsigned long long)pn->frames);
        jbuf_append_json_str(jb, pn->name);
        jbuf_append_cstr(jb, ",\"media_class\":");
        jbuf_append_json_str(jb, pn->media_class);
        jbuf_append_cstr(jb, ",\"format_name\":");
        jbuf_append_json_str(jb, pn->format_name);
        jbuf_append_cstr(jb, ",\"mem_type\":");
        jbuf_append_json_str(jb, pn->mem_type);
        jbuf_append_cstr(jb, "}");
    }

    jbuf_append_cstr(jb, "],\"links\":[");
    for (uint32_t i = 0; i < snap->pw.num_links && i < LWASAPI_DEBUG_MAX_PW_LINKS; i++) {
        const lwasapi_dbg_pw_link_t *pl = &snap->pw.links[i];
        if (i > 0) jbuf_append_cstr(jb, ",");
        jbuf_appendf(jb,
            "{\"id\":%u,\"out_node_id\":%u,\"in_node_id\":%u,"
            "\"state\":%u,\"is_dmabuf\":%u,\"frames_relayed\":%llu,\"format_summary\":",
            pl->id, pl->out_node_id, pl->in_node_id,
            pl->state, pl->is_dmabuf, (unsigned long long)pl->frames_relayed);
        jbuf_append_json_str(jb, pl->format_summary);
        jbuf_append_cstr(jb, "}");
    }
    salsa_failsafe_stats_t fs;
    salsa_failsafe_get_stats(&fs);
    jbuf_appendf(jb,
        "]},\"wire\":{\"backend\":\"%s\",\"is_rust\":%d,\"armed\":%d,"
        "\"trips_total\":%llu,\"pa_quarantined\":%llu,\"pw_quarantined\":%llu,"
        "\"bt_fallbacks\":%llu,\"fd_leaks_blocked\":%llu,\"manual_resets\":%llu,"
        "\"last_reason\":",
        fs.is_rust ? "rust" : "c_fallback",
        fs.is_rust ? 1 : 0,
        fs.armed ? 1 : 0,
        (unsigned long long)fs.trips_total,
        (unsigned long long)fs.pa_quarantined,
        (unsigned long long)fs.pw_quarantined,
        (unsigned long long)fs.bt_fallbacks,
        (unsigned long long)fs.fd_leaks_blocked,
        (unsigned long long)fs.manual_resets);
    jbuf_append_json_str(jb, fs.last_reason);
    jbuf_append_cstr(jb, "}}");
}

static void history_push_ring(dbg_hist_entry_t *ring, size_t *head, size_t *count,
                              uint64_t id, const char *json_line, size_t len) {
    char *copy = (char *)malloc(len + 1U);
    if (!copy) return;
    memcpy(copy, json_line, len);
    copy[len] = '\0';

    size_t idx = (*head + *count) % DBG_HISTORY_CAP;
    if (*count == DBG_HISTORY_CAP) {
        free(ring[*head].line);
        ring[*head].id = id;
        ring[*head].line = copy;
        *head = (*head + 1U) % DBG_HISTORY_CAP;
    } else {
        ring[idx].id = id;
        ring[idx].line = copy;
        (*count)++;
    }
}

static void history_push(uint64_t id, uint8_t lvl, const char *json_line, size_t len) {
    if (lvl <= LWASAPI_DBG_INFO) {
        history_push_ring(g_imp_hist, &g_imp_head, &g_imp_count, id, json_line, len);
    } else {
        history_push_ring(g_trace_hist, &g_trace_head, &g_trace_count, id, json_line, len);
    }
}

static void client_close(dbg_client_t *c) {
    if (c->fd >= 0) {
        close(c->fd);
        c->fd = -1;
    }
    free(c->outbuf);
    c->outbuf = NULL;
    c->outlen = 0;
    c->outcap = 0;
    c->inlen = 0;
}

static void client_queue_line(dbg_client_t *c, const char *line, size_t len) {
    if (c->fd < 0) return;
    size_t need = len + 1U;
    if (c->outlen + need > DBG_CLIENT_MAX_OUT) {
        /* Drop slow consumer rather than exhausting memory */
        client_close(c);
        return;
    }
    if (c->outlen + need > c->outcap) {
        size_t ncap = c->outcap ? c->outcap : 16384U;
        while (ncap < c->outlen + need) ncap *= 2U;
        char *p = (char *)realloc(c->outbuf, ncap);
        if (!p) return;
        c->outbuf = p;
        c->outcap = ncap;
    }
    memcpy(c->outbuf + c->outlen, line, len);
    c->outbuf[c->outlen + len] = '\n';
    c->outlen += need;
}

static void broadcast_line(const char *line, size_t len) {
    for (int i = 0; i < DBG_MAX_CLIENTS; i++) {
        if (g_clients[i].fd >= 0) {
            client_queue_line(&g_clients[i], line, len);
        }
    }
}

static void client_flush(dbg_client_t *c) {
    while (c->fd >= 0 && c->outlen > 0) {
        ssize_t n = send(c->fd, c->outbuf, c->outlen, MSG_NOSIGNAL | MSG_DONTWAIT);
        if (n > 0) {
            size_t sent = (size_t)n;
            if (sent < c->outlen) {
                memmove(c->outbuf, c->outbuf + sent, c->outlen - sent);
            }
            c->outlen -= sent;
        } else if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            break;
        } else {
            client_close(c);
            break;
        }
    }
}

static int num_active_clients(void) {
    int cnt = 0;
    for (int i = 0; i < DBG_MAX_CLIENTS; i++) {
        if (g_clients[i].fd >= 0) cnt++;
    }
    return cnt;
}

static void send_snapshot_to_client(dbg_client_t *target_client) {
    if (!g_cbs.fill_snapshot) return;
    lwasapi_dbg_snapshot_t *snap = (lwasapi_dbg_snapshot_t *)calloc(1, sizeof(*snap));
    if (!snap) return;

    lwasapi_debug_get_rt_stats(&snap->rt);
    g_cbs.fill_snapshot(g_cbs.user_ctx, snap);

    dbg_jbuf_t jb = {0};
    serialize_snapshot_json(&jb, snap);
    if (jb.data && jb.len > 0) {
        if (target_client) {
            client_queue_line(target_client, jb.data, jb.len);
        } else {
            broadcast_line(jb.data, jb.len);
        }
    }
    free(jb.data);
    free(snap);
}

static void handle_client_command(dbg_client_t *c, char *cmd) {
    /* Trim whitespace */
    size_t n = strlen(cmd);
    while (n > 0 && (cmd[n - 1] == '\r' || cmd[n - 1] == '\n' ||
                     cmd[n - 1] == ' '  || cmd[n - 1] == '\t')) {
        cmd[--n] = '\0';
    }
    if (n == 0) return;

    if (strcmp(cmd, "SNAPSHOT") == 0) {
        send_snapshot_to_client(c);
    } else if (strcmp(cmd, "RESET_STATS") == 0) {
        lwasapi_debug_reset_rt_stats();
        lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, (uint32_t)getpid(), 0,
                           "RESET_STATS", "rt_telemetry", "RT min/max/jitter statistics reset via debugger");
        send_snapshot_to_client(c);
    } else if (strncmp(cmd, "PING", 4) == 0 && (cmd[4] == '\0' || cmd[4] == ' ')) {
        const char *tag = (cmd[4] == ' ') ? (cmd + 5) : "";
        dbg_jbuf_t jb = {0};
        jbuf_appendf(&jb, "{\"type\":\"pong\",\"ts_ms\":%lld,\"ts_mono_ns\":%llu,\"tag\":",
                     (long long)dbg_real_ms(), (unsigned long long)dbg_mono_ns());
        jbuf_append_json_str(&jb, tag);
        jbuf_append_cstr(&jb, "}");
        if (jb.data && jb.len > 0) {
            client_queue_line(c, jb.data, jb.len);
        }
        free(jb.data);
    } else if (g_cbs.handle_cmd) {
        char reply[512];
        reply[0] = '\0';
        int rc = g_cbs.handle_cmd(g_cbs.user_ctx, cmd, reply, sizeof(reply));
        dbg_jbuf_t jb = {0};
        jbuf_appendf(&jb, "{\"type\":\"cmd_reply\",\"ok\":%s,\"rc\":%d,\"cmd\":",
                     (rc == 0) ? "true" : "false", rc);
        jbuf_append_json_str(&jb, cmd);
        jbuf_append_cstr(&jb, ",\"msg\":");
        jbuf_append_json_str(&jb, reply);
        jbuf_append_cstr(&jb, "}");
        if (jb.data && jb.len > 0) {
            client_queue_line(c, jb.data, jb.len);
        }
        free(jb.data);
    }
}

static void accept_new_client(void) {
    int cfd = accept4(g_listen_fd, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC);
    if (cfd < 0) return;

    struct ucred uc = {0};
    socklen_t ulen = sizeof(uc);
    if (getsockopt(cfd, SOL_SOCKET, SO_PEERCRED, &uc, &ulen) != 0 || !dbg_peer_uid_allowed(uc.uid)) {
        close(cfd);
        return;
    }

    int slot = -1;
    for (int i = 0; i < DBG_MAX_CLIENTS; i++) {
        if (g_clients[i].fd < 0) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        close(cfd);
        return;
    }

    dbg_client_t *c = &g_clients[slot];
    memset(c, 0, sizeof(*c));
    c->fd = cfd;

    /* Send hello header */
    dbg_jbuf_t jb = {0};
    jbuf_appendf(&jb,
        "{\"type\":\"hello\",\"version\":1,\"pid\":%u,\"ts_ms\":%lld,"
        "\"sock\":\"%s\",\"dropped_events\":%llu,\"history_count\":%zu}",
        (unsigned)getpid(),
        (long long)dbg_real_ms(),
        g_dbg_sock_path,
        (unsigned long long)atomic_load_explicit(&g_dropped_events, memory_order_relaxed),
        g_imp_count + g_trace_count);
    if (jb.data && jb.len > 0) {
        client_queue_line(c, jb.data, jb.len);
    }
    free(jb.data);

    /* Replay recent important + trace events chronologically by id */
    size_t ii = 0, ti = 0;
    while (ii < g_imp_count || ti < g_trace_count) {
        const dbg_hist_entry_t *pick = NULL;
        if (ii < g_imp_count && ti < g_trace_count) {
            const dbg_hist_entry_t *ei = &g_imp_hist[(g_imp_head + ii) % DBG_HISTORY_CAP];
            const dbg_hist_entry_t *et = &g_trace_hist[(g_trace_head + ti) % DBG_HISTORY_CAP];
            if (ei->id <= et->id) {
                pick = ei;
                ii++;
            } else {
                pick = et;
                ti++;
            }
        } else if (ii < g_imp_count) {
            pick = &g_imp_hist[(g_imp_head + ii) % DBG_HISTORY_CAP];
            ii++;
        } else {
            pick = &g_trace_hist[(g_trace_head + ti) % DBG_HISTORY_CAP];
            ti++;
        }
        if (pick && pick->line) {
            client_queue_line(c, pick->line, strlen(pick->line));
        }
    }

    /* Send immediate telemetry snapshot */
    send_snapshot_to_client(c);
    client_flush(c);
}

static void drain_dgram_socket(void) {
    if (g_dgram_fd < 0) return;
    lwasapi_dbg_dgram_t dg;
    char cbuf[CMSG_SPACE(sizeof(struct ucred))];
    for (int iter = 0; iter < 64; iter++) {
        struct iovec iov = { .iov_base = &dg, .iov_len = sizeof(dg) };
        struct msghdr msg = {0};
        msg.msg_iov = &iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf;
        msg.msg_controllen = sizeof(cbuf);

        ssize_t n = recvmsg(g_dgram_fd, &msg, MSG_DONTWAIT);
        if (n < (ssize_t)sizeof(dg)) break;
        if (dg.magic != LWASAPI_DEBUG_DGRAM_MAGIC) continue;

        bool has_cred = false;
        struct ucred uc = {0};
        for (struct cmsghdr *cm = CMSG_FIRSTHDR(&msg); cm; cm = CMSG_NXTHDR(&msg, cm)) {
            if (cm->cmsg_level == SOL_SOCKET && cm->cmsg_type == SCM_CREDENTIALS &&
                cm->cmsg_len >= CMSG_LEN(sizeof(struct ucred))) {
                memcpy(&uc, CMSG_DATA(cm), sizeof(uc));
                has_cred = true;
                break;
            }
        }
        if (!has_cred || !dbg_peer_uid_allowed(uc.uid)) continue;

        dg.op[sizeof(dg.op) - 1] = '\0';
        dg.target[sizeof(dg.target) - 1] = '\0';
        dg.detail[sizeof(dg.detail) - 1] = '\0';

        dbg_event_t ev;
        ev.id = atomic_fetch_add_explicit(&g_next_ev_id, 1U, memory_order_relaxed);
        ev.ts_mono_ns = dbg_mono_ns();
        ev.ts_real_ms = dbg_real_ms();
        ev.pid = dg.pid;
        ev.sid = dg.session_id;
        ev.v1  = dg.v1;
        ev.v2  = dg.v2;
        ev.hr  = dg.hr;
        ev.sub = dg.sub;
        ev.lvl = dg.lvl;
        safe_cstr_copy(ev.op, sizeof(ev.op), dg.op);
        safe_cstr_copy(ev.target, sizeof(ev.target), dg.target);
        safe_cstr_copy(ev.detail, sizeof(ev.detail), dg.detail);

        dbg_jbuf_t jb = {0};
        serialize_event_json(&jb, &ev);
        if (jb.data && jb.len > 0) {
            history_push(ev.id, ev.lvl, jb.data, jb.len);
            broadcast_line(jb.data, jb.len);
        }
        free(jb.data);
    }
}

static void drain_rt_ring(void) {
    dbg_jbuf_t jb = {0};
    for (int iter = 0; iter < 512; iter++) {
        dbg_slot_t *slot = &g_ring[g_ring_tail & (DBG_RING_CAP - 1U)];
        uint64_t turn = atomic_load_explicit(&slot->turn, memory_order_acquire);
        if (turn != g_ring_tail + 1U) break;

        dbg_event_t ev = slot->ev;
        atomic_store_explicit(&slot->turn, g_ring_tail + DBG_RING_CAP, memory_order_release);
        g_ring_tail++;

        jb.len = 0;
        serialize_event_json(&jb, &ev);
        if (jb.data && jb.len > 0) {
            history_push(ev.id, ev.lvl, jb.data, jb.len);
            broadcast_line(jb.data, jb.len);
        }
    }
    free(jb.data);
}

static void *debug_server_thread(void *arg) {
    (void)arg;
    pthread_setname_np(pthread_self(), "salsa-debug-srv");

    uint64_t last_telem_ms = 0;

    while (atomic_load_explicit(&g_dbg_running, memory_order_relaxed)) {
        struct pollfd pfds[2 + DBG_MAX_CLIENTS];
        int pfd_client_idx[2 + DBG_MAX_CLIENTS];
        int nfds = 0;

        if (g_listen_fd >= 0) {
            pfds[nfds].fd = g_listen_fd;
            pfds[nfds].events = POLLIN;
            pfds[nfds].revents = 0;
            pfd_client_idx[nfds++] = -1;
        }
        if (g_dgram_fd >= 0) {
            pfds[nfds].fd = g_dgram_fd;
            pfds[nfds].events = POLLIN;
            pfds[nfds].revents = 0;
            pfd_client_idx[nfds++] = -2;
        }
        for (int i = 0; i < DBG_MAX_CLIENTS; i++) {
            if (g_clients[i].fd >= 0) {
                pfds[nfds].fd = g_clients[i].fd;
                pfds[nfds].events = POLLIN | (g_clients[i].outlen > 0 ? POLLOUT : 0);
                pfds[nfds].revents = 0;
                pfd_client_idx[nfds++] = i;
            }
        }

        poll(pfds, (nfds_t)nfds, 25);

        for (int i = 0; i < nfds; i++) {
            int tag = pfd_client_idx[i];
            short rev = pfds[i].revents;
            if (!rev) continue;

            if (tag == -1 && (rev & POLLIN)) {
                accept_new_client();
            } else if (tag == -2 && (rev & POLLIN)) {
                drain_dgram_socket();
            } else if (tag >= 0) {
                dbg_client_t *c = &g_clients[tag];
                if (rev & POLLIN) {
                    for (;;) {
                        if (c->fd < 0) break;
                        size_t avail = sizeof(c->inbuf) - 1U - c->inlen;
                        if (avail == 0) {
                            c->inlen = 0;
                            avail = sizeof(c->inbuf) - 1U;
                        }
                        ssize_t n = recv(c->fd, c->inbuf + c->inlen, avail, MSG_DONTWAIT);
                        if (n > 0) {
                            c->inlen += (size_t)n;
                            c->inbuf[c->inlen] = '\0';
                            char *start = c->inbuf;
                            char *nl;
                            while (c->fd >= 0 && (nl = strchr(start, '\n')) != NULL) {
                                *nl = '\0';
                                handle_client_command(c, start);
                                start = nl + 1;
                            }
                            if (c->fd < 0) break;
                            size_t rem = c->inlen - (size_t)(start - c->inbuf);
                            if (rem > 0 && start != c->inbuf) {
                                memmove(c->inbuf, start, rem);
                            }
                            c->inlen = rem;
                            if (c->inlen >= sizeof(c->inbuf) - 1U) {
                                c->inlen = 0;
                            }
                        } else if (n == 0 || (n < 0 && errno != EAGAIN && errno != EWOULDBLOCK)) {
                            client_close(c);
                            break;
                        } else {
                            break;
                        }
                    }
                }
                if (c->fd >= 0 && (rev & (POLLERR | POLLHUP | POLLNVAL))) {
                    client_close(c);
                    continue;
                }
                if (c->fd >= 0 && (rev & POLLOUT)) {
                    client_flush(c);
                }
            }
        }

        drain_rt_ring();
        drain_dgram_socket();

        uint64_t now_ms = (uint64_t)dbg_real_ms();
        if (num_active_clients() > 0 && (now_ms - last_telem_ms >= DBG_TELEM_INTERVAL_MS)) {
            last_telem_ms = now_ms;
            send_snapshot_to_client(NULL);
        }

        for (int i = 0; i < DBG_MAX_CLIENTS; i++) {
            if (g_clients[i].fd >= 0 && g_clients[i].outlen > 0) {
                client_flush(&g_clients[i]);
            }
        }
    }

    return NULL;
}

static int setup_unix_listen(const char *path) {
    if (!path || strlen(path) >= 108) return -1;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    safe_cstr_copy(addr.sun_path, sizeof(addr.sun_path), path);

    unlink(path);
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return -1;
    }
    chmod(path, 0600);
    if (listen(fd, 8) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

static int setup_unix_dgram(const char *path) {
    if (!path || strlen(path) >= 108) return -1;
    int fd = socket(AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;

    int one = 1;
    (void)setsockopt(fd, SOL_SOCKET, SO_PASSCRED, &one, sizeof(one));

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    safe_cstr_copy(addr.sun_path, sizeof(addr.sun_path), path);

    unlink(path);
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return -1;
    }
    chmod(path, 0600);
    return fd;
}

int lwasapi_debug_start(lwasapi_dbg_callbacks_t cbs) {
    if (atomic_load_explicit(&g_dbg_running, memory_order_relaxed)) {
        return 0;
    }

    g_cbs = cbs;
    g_ring_head = 0;
    g_ring_tail = 0;
    g_next_ev_id = 1;
    g_dropped_events = 0;
    for (uint32_t i = 0; i < DBG_RING_CAP; i++) {
        atomic_init(&g_ring[i].turn, (uint64_t)i);
    }
    for (int i = 0; i < DBG_MAX_CLIENTS; i++) {
        g_clients[i].fd = -1;
        g_clients[i].outbuf = NULL;
        g_clients[i].outlen = 0;
        g_clients[i].outcap = 0;
        g_clients[i].inlen = 0;
    }
    memset(g_imp_hist, 0, sizeof(g_imp_hist));
    g_imp_head = 0;
    g_imp_count = 0;
    memset(g_trace_hist, 0, sizeof(g_trace_hist));
    g_trace_head = 0;
    g_trace_count = 0;

    lwasapi_debug_reset_rt_stats();

    lwasapi_debug_sock_path(g_dbg_sock_path, sizeof(g_dbg_sock_path), LWASAPI_DEBUG_SOCK_DEFAULT);
    lwasapi_debug_in_sock_path(g_dbg_in_sock_path, sizeof(g_dbg_in_sock_path), LWASAPI_DEBUG_IN_SOCK_DEFAULT);
    if (strcmp(g_dbg_sock_path, LWASAPI_DEBUG_SOCK_DEFAULT) != 0) unlink(LWASAPI_DEBUG_SOCK_DEFAULT);
    if (strcmp(g_dbg_in_sock_path, LWASAPI_DEBUG_IN_SOCK_DEFAULT) != 0) unlink(LWASAPI_DEBUG_IN_SOCK_DEFAULT);

    g_listen_fd = setup_unix_listen(g_dbg_sock_path);
    g_dgram_fd  = setup_unix_dgram(g_dbg_in_sock_path);

    atomic_store_explicit(&g_dbg_running, 1, memory_order_release);
    if (pthread_create(&g_dbg_thread, NULL, debug_server_thread, NULL) != 0) {
        atomic_store_explicit(&g_dbg_running, 0, memory_order_release);
        if (g_listen_fd >= 0) { close(g_listen_fd); g_listen_fd = -1; }
        if (g_dgram_fd >= 0)  { close(g_dgram_fd);  g_dgram_fd  = -1; }
        return -1;
    }

    lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, (uint32_t)getpid(), 0,
                       "DEBUG_START", g_dbg_sock_path,
                       "SALSA live telemetry & interaction stream initialized");
    return 0;
}

void lwasapi_debug_stop(void) {
    if (!atomic_exchange_explicit(&g_dbg_running, 0, memory_order_acq_rel)) {
        return;
    }

    pthread_join(g_dbg_thread, NULL);

    if (g_listen_fd >= 0) { close(g_listen_fd); g_listen_fd = -1; }
    if (g_dgram_fd >= 0)  { close(g_dgram_fd);  g_dgram_fd  = -1; }
    unlink(g_dbg_sock_path);
    unlink(g_dbg_in_sock_path);

    for (int i = 0; i < DBG_MAX_CLIENTS; i++) {
        client_close(&g_clients[i]);
    }
    for (size_t i = 0; i < DBG_HISTORY_CAP; i++) {
        free(g_imp_hist[i].line);
        g_imp_hist[i].line = NULL;
        g_imp_hist[i].id = 0;
        free(g_trace_hist[i].line);
        g_trace_hist[i].line = NULL;
        g_trace_hist[i].id = 0;
    }
    g_imp_head = 0;
    g_imp_count = 0;
    g_trace_head = 0;
    g_trace_count = 0;
}
