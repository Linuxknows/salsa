#ifndef LWASAPI_PA_SERVER_H
#define LWASAPI_PA_SERVER_H

#include <stdint.h>
#include <stdbool.h>
#include "lwasapi.h"
#include "lwasapi_bluez.h"

typedef struct lwasapi_pa_server lwasapi_pa_server_t;

typedef struct {
    void *user_ctx;
    void (*get_status)(void *ctx, lwasapi_engine_status_t *out_st);
    void (*get_devices)(void *ctx, lwasapi_device_list_t *out_list);
    int  (*set_devices)(void *ctx, const char *playback_id, const char *capture_id);
    int  (*set_master_volume)(void *ctx, int is_capture, int vol_pct, int mute_flag);
    int  (*set_mic_gain_ppm)(void *ctx, uint32_t gain_ppm, int vol_pct, int mute_flag);
} lwasapi_pa_callbacks_t;

/**
 * Start built-in PulseAudio native protocol v35 server on $XDG_RUNTIME_DIR/pulse/native.
 * Runs socket multiplexing on its own non-RT thread while exposing lock-free playback/capture
 * mixing hooks for audio_engine_thread.
 */
int  lwasapi_pa_server_start(lwasapi_pa_server_t **out_srv, lwasapi_pa_callbacks_t cbs);

/**
 * Stop built-in PulseAudio server and unlink socket cleanly.
 */
void lwasapi_pa_server_stop(lwasapi_pa_server_t *srv);

/**
 * Notify subscribed PulseAudio clients (KDE plasma-pa, pavucontrol, Firefox)
 * that sinks, sources, cards, or server defaults have changed.
 */
void lwasapi_pa_server_notify_change(lwasapi_pa_server_t *srv);

/**
 * Lightweight volume/mute notification that avoids D-Bus/ALSA device re-enumeration.
 */
void lwasapi_pa_server_notify_volume(lwasapi_pa_server_t *srv, bool is_capture);

/**
 * RT-safe lock-free mix of all running PulseAudio playback streams into stereo_out.
 * Called every period from audio_engine_thread.
 */
void lwasapi_pa_server_mix_playback(lwasapi_pa_server_t *srv, float *stereo_out,
                                    uint32_t frames, uint32_t engine_rate);

/**
 * RT-safe lock-free feed of microphone capture + master monitor loopback into
 * all running PulseAudio record streams. Called every period from audio_engine_thread.
 */
void lwasapi_pa_server_push_capture(lwasapi_pa_server_t *srv,
                                    const float *mic_stereo, uint32_t mic_frames, uint32_t mic_rate,
                                    const float *monitor_stereo, uint32_t mon_frames, uint32_t mon_rate);

/**
 * Append active PulseAudio client streams into engine status session table and update stats.
 */
void lwasapi_pa_server_fill_sessions(lwasapi_pa_server_t *srv, lwasapi_engine_status_t *st,
                                     uint32_t *out_num_clients);

/**
 * Adjust per-stream PulseAudio sink-input volume (when controlled from Salsa GUI/CLI).
 */
int  lwasapi_pa_server_set_stream_volume(lwasapi_pa_server_t *srv, uint32_t session_id, uint32_t vol_pct);
int  lwasapi_pa_server_control_stream(lwasapi_pa_server_t *srv, uint32_t session_id,
                                      int32_t vol_pct, int32_t mute_mode, const char *target_device_id);

uint32_t lwasapi_pa_server_get_dominant_rate(lwasapi_pa_server_t *srv, uint32_t *out_bits);

struct lwasapi_dbg_pa_snapshot_tag;
#include "lwasapi_debug.h"
/**
 * Populate deep per-client and per-stream PulseAudio v35 telemetry snapshot.
 */
void lwasapi_pa_server_fill_debug(lwasapi_pa_server_t *srv, lwasapi_dbg_pa_snapshot_t *out);

#endif /* LWASAPI_PA_SERVER_H */
