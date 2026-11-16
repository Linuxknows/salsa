#ifndef LWASAPI_DEBUG_H
#define LWASAPI_DEBUG_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include "lwasapi.h"

#ifdef __cplusplus
extern "C" {
#endif

#define LWASAPI_DEBUG_SOCK_DEFAULT    "/tmp/lwasapi-debug.sock"
#define LWASAPI_DEBUG_IN_SOCK_DEFAULT "/tmp/lwasapi-debug-in.sock"
#define LWASAPI_DEBUG_DGRAM_MAGIC     0x4C444247U /* 'LDBG' */
#define LWASAPI_DEBUG_MAX_PA_CLIENTS  32
#define LWASAPI_DEBUG_MAX_PA_STREAMS  48
#define LWASAPI_DEBUG_MAX_PW_CLIENTS  32
#define LWASAPI_DEBUG_MAX_PW_NODES    32
#define LWASAPI_DEBUG_MAX_PW_LINKS    32

typedef enum {
    LWASAPI_DBG_ENGINE   = 0,
    LWASAPI_DBG_WASAPI   = 1,
    LWASAPI_DBG_PA       = 2,
    LWASAPI_DBG_BLUEZ    = 3,
    LWASAPI_DBG_MIDI     = 4,
    LWASAPI_DBG_SHIM     = 5,
    LWASAPI_DBG_ALSA     = 6,
    LWASAPI_DBG_PW_VIDEO = 7
} lwasapi_dbg_sub_t;

typedef enum {
    LWASAPI_DBG_TRACE = 0,
    LWASAPI_DBG_INFO  = 1,
    LWASAPI_DBG_WARN  = 2,
    LWASAPI_DBG_ERR   = 3
} lwasapi_dbg_lvl_t;

/* Non-blocking datagram emitted from liblwasapi / shims / alsa plugin / ctl */
typedef struct {
    uint32_t magic;
    uint32_t pid;
    uint32_t session_id;
    int32_t  v1;
    int32_t  v2;
    int32_t  hr;
    uint8_t  sub;
    uint8_t  lvl;
    uint16_t reserved;
    char     op[28];
    char     target[48];
    char     detail[140];
} lwasapi_dbg_dgram_t;

typedef struct {
    uint64_t cycle_count;
    uint64_t last_cycle_ns;
    uint64_t avg_cycle_ns;
    uint64_t max_cycle_ns;
    uint64_t min_cycle_ns;
    int64_t  last_jitter_ns;
    uint64_t max_jitter_ns;
    uint64_t eq_ns;
    uint64_t pa_mix_ns;
    uint64_t hw_write_ns;
    uint64_t budget_ns;
    uint64_t hw_xruns;
} lwasapi_dbg_rt_stats_t;

typedef struct {
    uint32_t session_id;
    uint32_t client_pid;
    char     client_name[64];
    uint32_t share_mode;
    uint32_t stream_flags;
    uint32_t is_capture;
    uint32_t sample_rate;
    uint32_t channels;
    uint32_t active;
    uint32_t muted;
    uint32_t volume_milli;
    uint32_t buffer_frames;
    uint32_t period_frames;
    uint64_t buffered_frames;
    uint64_t xruns;
} lwasapi_dbg_wasapi_session_t;

typedef struct {
    uint32_t index;
    uint32_t pid;
    char     name[64];
    char     binary[64];
    uint32_t version;
    uint8_t  authed;
    uint32_t sub_mask;
    uint32_t out_count;
} lwasapi_dbg_pa_client_t;

typedef struct {
    uint32_t index;
    uint32_t client_index;
    uint32_t pid;
    char     name[64];
    char     device_id[64];
    uint8_t  is_playback;
    uint8_t  is_monitor;
    uint8_t  peak_detect;
    uint32_t direct_on_input;
    uint32_t target_dev_idx;
    uint32_t rate;
    uint8_t  channels;
    uint8_t  format;
    uint8_t  active;
    uint8_t  corked;
    uint8_t  in_prebuf;
    uint8_t  draining;
    uint8_t  muted;
    uint32_t volume_milli;
    uint32_t frame_size;
    uint64_t buffered_frames;
    uint32_t ring_cap_frames;
    uint32_t tlength_frames;
    uint32_t prebuf_frames;
    uint32_t minreq_frames;
    uint32_t fragsize_frames;
    float    mix_peak_l;
    float    mix_peak_r;
    uint64_t write_index;
    uint64_t read_index;
} lwasapi_dbg_pa_stream_t;

typedef struct {
    uint32_t num_clients;
    lwasapi_dbg_pa_client_t clients[LWASAPI_DEBUG_MAX_PA_CLIENTS];
    uint32_t num_streams;
    lwasapi_dbg_pa_stream_t streams[LWASAPI_DEBUG_MAX_PA_STREAMS];
} lwasapi_dbg_pa_snapshot_t;

typedef struct {
    uint32_t id;
    uint32_t pid;
    char     app_name[64];
    char     binary[64];
    char     portal_app_id[64];
    uint8_t  is_portal;
    uint8_t  pad[3];
    uint32_t num_nodes;
} lwasapi_dbg_pw_client_t;

typedef struct {
    uint32_t id;
    uint32_t client_id;
    uint32_t client_pid;
    uint32_t target_node_id;
    uint32_t linked_node_id;
    char     name[64];
    char     media_class[48];
    char     format_name[24];
    char     mem_type[16];
    uint8_t  direction;     /* 0 = input (consumer), 1 = output (producer) */
    uint8_t  state;         /* 0=error,1=creating,2=suspended,3=idle,4=running */
    uint8_t  is_driver;
    uint8_t  alloc_buffers;
    uint32_t width;
    uint32_t height;
    uint32_t fps_num;
    uint32_t fps_denom;
    uint32_t n_buffers;
    uint64_t drm_modifier;
    uint64_t frames;
} lwasapi_dbg_pw_node_t;

typedef struct {
    uint32_t id;
    uint32_t out_node_id;
    uint32_t in_node_id;
    uint8_t  state;         /* 0=init,1=negotiating,2=allocating,3=paused,4=active,5=error */
    uint8_t  is_dmabuf;
    uint8_t  pad[2];
    char     format_summary[48];
    uint64_t frames_relayed;
} lwasapi_dbg_pw_link_t;

typedef struct {
    uint8_t  broker_running;
    uint8_t  zero_pw_mode;
    uint8_t  preview_requested;
    uint8_t  active_casting;
    char     socket_path[128];
    char     manager_socket_path[128];
    char     preview_shm_name[64];
    uint64_t frames_relayed_total;
    uint64_t frames_dropped;
    uint32_t dmabuf_streams;
    uint32_t memfd_streams;
    uint32_t audio_rejects;
    uint32_t preview_target_w;
    uint32_t preview_target_h;
    uint32_t preview_target_fps;
    uint32_t cast_limit_w;
    uint32_t cast_limit_h;
    uint32_t cast_limit_fps;
    uint32_t actual_fps_x100;
    uint32_t num_clients;
    lwasapi_dbg_pw_client_t clients[LWASAPI_DEBUG_MAX_PW_CLIENTS];
    uint32_t num_nodes;
    lwasapi_dbg_pw_node_t   nodes[LWASAPI_DEBUG_MAX_PW_NODES];
    uint32_t num_links;
    lwasapi_dbg_pw_link_t   links[LWASAPI_DEBUG_MAX_PW_LINKS];
} lwasapi_dbg_pw_snapshot_t;

typedef struct {
    lwasapi_engine_status_t engine_st;
    lwasapi_dbg_rt_stats_t  rt;
    char     target_playback_id[64];
    char     target_capture_id[64];

    /* BlueZ detailed */
    uint8_t  bt_initialized;
    uint8_t  bt_endpoints_registered;
    uint8_t  bt_direct_socket;
    uint8_t  bt_has_transport;
    uint8_t  bt_tx_paused;
    char     bt_device_path[128];
    char     bt_transport_path[160];
    char     bt_endpoint_path[64];
    uint32_t bt_sample_rate;
    uint32_t bt_channels;
    uint16_t bt_mtu_r;
    uint16_t bt_mtu_w;
    uint16_t bt_avrcp_vol;
    uint64_t bt_tx_ring_fill;
    uint32_t bt_tx_ring_cap;
    uint64_t bt_tx_packets;
    uint64_t bt_tx_underruns;
    uint64_t bt_tx_errors;

    /* Kernel ALSA Capture detailed */
    uint8_t  cap_active;
    char     cap_device_id[64];
    char     cap_pcm_path[64];
    char     cap_alias[64];
    uint32_t cap_hw_rate;
    uint32_t cap_hw_channels;
    int32_t  cap_hw_format;
    uint64_t cap_ring_fill;
    uint32_t cap_pref_rate;
    uint32_t cap_measured_rate;
    uint32_t cap_kick_count;
    uint8_t  cap_is_usb;
    uint8_t  cap_duplex_locked;

    /* MIDI Hub detailed */
    uint8_t  midi_initialized;
    uint8_t  midi_rawmidi_open;
    char     midi_controller[64];
    char     midi_dev_path[64];
    int32_t  midi_seq_client;
    int32_t  midi_seq_in_port;
    uint64_t midi_events_routed;

    /* Native WASAPI Sessions */
    uint32_t num_wasapi;
    lwasapi_dbg_wasapi_session_t wasapi[LWASAPI_MAX_SESSIONS];

    /* PA v35 Server Snapshot */
    lwasapi_dbg_pa_snapshot_t pa;

    /* Built-in Video-Only PipeWire-0 ScreenCast Broker Snapshot */
    lwasapi_dbg_pw_snapshot_t pw;
} lwasapi_dbg_snapshot_t;

typedef struct {
    void *user_ctx;
    void (*fill_snapshot)(void *ctx, lwasapi_dbg_snapshot_t *out_snap);
    int  (*handle_cmd)(void *ctx, const char *cmd, char *reply, size_t reply_len);
} lwasapi_dbg_callbacks_t;

/* Engine-side RT-safe telemetry & event stream server (linked in lwasapi-audiodg) */
int  lwasapi_debug_start(lwasapi_dbg_callbacks_t cbs);
void lwasapi_debug_stop(void);
void lwasapi_debug_emit(uint8_t sub, uint8_t lvl, uint32_t pid, uint32_t sid,
                        const char *op, const char *target, const char *fmt, ...)
                        __attribute__((format(printf, 7, 8)));
void lwasapi_debug_emit_vals(uint8_t sub, uint8_t lvl, uint32_t pid, uint32_t sid,
                             int32_t v1, int32_t v2, int32_t hr,
                             const char *op, const char *target, const char *fmt, ...)
                             __attribute__((format(printf, 10, 11)));
void lwasapi_debug_record_rt_period(uint64_t cycle_ns, int64_t wake_jitter_ns,
                                    uint64_t budget_ns, uint64_t eq_ns,
                                    uint64_t pa_mix_ns, uint64_t hw_write_ns);
void lwasapi_debug_get_rt_stats(lwasapi_dbg_rt_stats_t *out_rt);
void lwasapi_debug_reset_rt_stats(void);

/* Client/Shim non-blocking datagram trace helper (exported by liblwasapi.so) */
void LWASAPI_DebugTraceClient(uint8_t sub, uint8_t lvl, uint32_t sid,
                              const char *op, const char *target, const char *detail);

#ifdef __cplusplus
}
#endif

#endif /* LWASAPI_DEBUG_H */
