#ifndef LWASAPI_PW_BROKER_H
#define LWASAPI_PW_BROKER_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include "lwasapi_debug.h"

#ifdef __cplusplus
extern "C" {
#endif

#define LWASAPI_PW_PREVIEW_MAGIC   0x534C5057u /* "SLPW" */
#define LWASAPI_PW_PREVIEW_VERSION 2u
#define LWASAPI_PW_PREVIEW_MAX_W   3840u
#define LWASAPI_PW_PREVIEW_MAX_H   2160u
#define LWASAPI_PW_PREVIEW_MAX_BYTES (LWASAPI_PW_PREVIEW_MAX_W * LWASAPI_PW_PREVIEW_MAX_H * 4u)

/* Lockless seqlock-protected preview frame SHM (/dev/shm/lwasapi_pw_preview_<uid>) */
typedef struct {
    uint32_t magic;
    uint32_t version;
    uint64_t seqlock;            /* odd = writer in progress, even = coherent frame */
    uint64_t last_gui_ping_ns;   /* monotonic ns timestamp set by debugger UI/socket */
    uint32_t preview_requested;  /* 1 when debugger live feed is toggled ON */
    uint32_t active_stream;      /* 1 when a real ScreenCast stream is relaying */
    uint32_t is_dmabuf;          /* 1 if stream buffers are SPA_DATA_DmaBuf */
    uint32_t dmabuf_mappable;    /* 1 if linear DMA-BUF CPU mmap succeeded */
    uint32_t src_width;
    uint32_t src_height;
    uint32_t src_stride;
    uint32_t src_spa_format;
    uint32_t drm_modifier_lo;
    uint32_t drm_modifier_hi;
    uint32_t dst_width;
    uint32_t dst_height;
    uint32_t dst_stride;         /* BGRA / BGRx (QImage::Format_RGB32) */
    uint32_t prod_node_id;
    uint32_t cons_count;
    uint64_t frame_seq;
    uint64_t pts_ns;
    char     prod_name[128];
    char     format_name[32];
    uint32_t target_max_w;       /* 0 = Source (up to LWASAPI_PW_PREVIEW_MAX_W) */
    uint32_t target_max_h;       /* 0 = Source (up to LWASAPI_PW_PREVIEW_MAX_H) */
    uint32_t target_fps;         /* 0 = Source (uncapped), else 15/30/60/... */
    uint32_t actual_fps_x100;    /* rolling measured frame rate * 100 */
    uint32_t cast_fps_cap;       /* active broker stream FPS cap (0 = uncapped) */
    uint8_t  reserved[236];
    uint8_t  pixels[LWASAPI_PW_PREVIEW_MAX_BYTES];
} lwasapi_pw_preview_shm_t;

typedef struct lwasapi_pw_broker lwasapi_pw_broker_t;

typedef struct {
    const char *runtime_dir;     /* NULL -> $XDG_RUNTIME_DIR or /run/user/<uid> */
    const char *socket_name;     /* NULL -> "pipewire-0" (or $LWASAPI_PW_SOCKET_NAME) */
    bool        ignore_zero_flag;/* true for isolated test harness sockets */
} lwasapi_pw_broker_opts_t;

/* Start Salsa's built-in Video-Only PipeWire-0 & Manager socket broker */
int  lwasapi_pw_broker_start(lwasapi_pw_broker_t **out_brk, const lwasapi_pw_broker_opts_t *opts);

/* Cleanly tear down sockets, worker thread, and preview SHM */
void lwasapi_pw_broker_stop(lwasapi_pw_broker_t *brk);

/* Re-evaluate ~/.config/lwasapi/pipewire_zero_mode (e.g. live enable-auto/disable-auto) */
int  lwasapi_pw_broker_sync_mode(lwasapi_pw_broker_t *brk);

/* Enable or refresh the debugger live preview tap (ttl_ms == 0 disables immediately) */
void lwasapi_pw_broker_set_preview(lwasapi_pw_broker_t *brk, bool enabled, uint32_t ttl_ms);

/* Extended preview control with Discord-matching live max resolution & target FPS (0 = Source/uncapped) */
void lwasapi_pw_broker_set_preview_ex(lwasapi_pw_broker_t *brk, bool enabled, uint32_t ttl_ms,
                                      uint32_t max_w, uint32_t max_h, uint32_t target_fps);

/* Optional broker-wide PipeWire stream resolution/FPS ceiling (0 = uncapped/negotiated) */
void lwasapi_pw_broker_set_cast_limits(lwasapi_pw_broker_t *brk,
                                        uint32_t max_w, uint32_t max_h, uint32_t max_fps);

/* Fill ScreenCast/Video Broker snapshot for lwasapi_debug telemetry */
void lwasapi_pw_broker_fill_debug(lwasapi_pw_broker_t *brk, lwasapi_dbg_pw_snapshot_t *out);

/* Query active preview SHM path (e.g. /dev/shm/lwasapi_pw_preview_1000) */
void lwasapi_pw_preview_shm_name(char *out_buf, size_t out_len);

/* Run built-in producer->consumer loopback self-test through broker's socket */
int  lwasapi_pw_broker_self_test(lwasapi_pw_broker_t *brk, char *out_msg, size_t out_len);

/* Flush and disconnect active PipeWire video broker client sockets cleanly */
int  lwasapi_pw_broker_reset_clients(lwasapi_pw_broker_t *brk);

#ifdef __cplusplus
}
#endif

#endif /* LWASAPI_PW_BROKER_H */
