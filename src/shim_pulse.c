#define _GNU_SOURCE
#include "../include/lwasapi.h"
#include "../include/lwasapi_debug.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>
#include <time.h>
#include <pulse/pulseaudio.h>
#include <pulse/simple.h>

/* ============================================================================
 * Hollow ABI Shim for PulseAudio (libpulse.so.0 & libpulse-simple.so.0)
 * Implements both the Simple API and the full Threaded Mainloop / Stream API
 * used by Firefox (cubeb pulse-rust), Chromium, Discord, and SDL2.
 * ============================================================================ */

#define MAX_PA_STREAMS 16

struct pa_threaded_mainloop {
    pthread_mutex_t  mutex;
    pthread_cond_t   cond;
    pthread_t        thread;
    volatile int     running;
    int              in_thread;
    pa_mainloop_api  api;
    struct pa_context *ctx;
};

struct pa_operation {
    int                         ref;
    pa_operation_state_t        state;
    pa_operation_notify_cb_t    cb;
    void                       *userdata;
};

struct pa_proplist {
    char name[64];
};

struct pa_context {
    int                         ref;
    pa_threaded_mainloop       *mainloop;
    pa_context_state_t          state;
    pa_context_notify_cb_t      state_cb;
    void                       *state_userdata;
    pa_context_subscribe_cb_t   sub_cb;
    void                       *sub_userdata;
    struct pa_stream           *streams[MAX_PA_STREAMS];
};

struct pa_stream {
    int                         ref;
    pa_context                 *ctx;
    char                        name[64];
    pa_sample_spec              ss;
    pa_channel_map              cmap;
    pa_buffer_attr              battr;
    pa_stream_state_t           state;
    int                         corked;
    bool                        is_record;
    uint32_t                    index;
    IAudioClient               *lwasapi;
    lwasapi_sample_format_t     lfmt;
    uint32_t                    frame_bytes;
    void                       *write_scratch;
    size_t                      write_scratch_cap;
    void                       *read_scratch;
    size_t                      read_scratch_cap;
    size_t                      read_scratch_len;
    uint64_t                    bytes_written;
    pa_timing_info              timing;
    pa_stream_notify_cb_t       state_cb;
    void                       *state_userdata;
    pa_stream_request_cb_t      write_cb;
    void                       *write_userdata;
    pa_stream_request_cb_t      read_cb;
    void                       *read_userdata;
};

struct pa_simple {
    IAudioClient           *client;
    uint32_t                rate;
    uint32_t                channels;
    lwasapi_sample_format_t fmt;
    uint32_t                frame_bytes;
    bool                    is_record;
};

static lwasapi_sample_format_t map_pa_fmt(pa_sample_format_t pa_fmt, uint32_t *bytes_per_sample) {
    if (pa_fmt == PA_SAMPLE_S16LE || pa_fmt == PA_SAMPLE_S16BE) {
        *bytes_per_sample = 2;
        return LWASAPI_FMT_S16_LE;
    }
    if (pa_fmt == PA_SAMPLE_S32LE || pa_fmt == PA_SAMPLE_S32BE) {
        *bytes_per_sample = 4;
        return LWASAPI_FMT_S32_LE;
    }
    *bytes_per_sample = 4;
    return LWASAPI_FMT_FLOAT32;
}

static pa_operation *alloc_done_op(void) {
    pa_operation *op = calloc(1, sizeof(pa_operation));
    if (op) {
        op->ref = 1;
        op->state = PA_OPERATION_DONE;
    }
    return op;
}

/* ============================================================================
 * Threaded Mainloop Worker: Drives Firefox/WebRTC write & read callbacks at 256f
 * ============================================================================ */

static void *mainloop_worker(void *arg) {
    pa_threaded_mainloop *m = (pa_threaded_mainloop *)arg;

    while (m->running) {
        usleep(4000); /* ~4ms period check matching 256 frames @ 48kHz */

        pthread_mutex_lock(&m->mutex);
        m->in_thread = 1;

        pa_context *ctx = m->ctx;
        if (ctx && ctx->state == PA_CONTEXT_READY) {
            for (int i = 0; i < MAX_PA_STREAMS; i++) {
                pa_stream *s = ctx->streams[i];
                if (!s || s->state != PA_STREAM_READY || s->corked || !s->lwasapi) {
                    continue;
                }
                if (s->is_record) {
                    if (s->read_cb) {
                        uint32_t pkt_frames = 0;
                        IAudioCaptureClient_GetNextPacketSize(s->lwasapi, &pkt_frames);
                        if (pkt_frames >= 256) {
                            size_t readable_bytes = (size_t)pkt_frames * s->frame_bytes;
                            s->read_cb(s, readable_bytes, s->read_userdata);
                        }
                    }
                } else if (s->write_cb) {
                    uint32_t buf_frames = 2048, pad_frames = 0;
                    IAudioClient_GetBufferSize(s->lwasapi, &buf_frames);
                    IAudioClient_GetCurrentPadding(s->lwasapi, &pad_frames);
                    uint32_t avail_frames = (buf_frames > pad_frames) ? (buf_frames - pad_frames) : 0;
                    if (avail_frames >= 256) {
                        size_t writable_bytes = (size_t)avail_frames * s->frame_bytes;
                        s->write_cb(s, writable_bytes, s->write_userdata);
                    }
                }
            }
        }

        m->in_thread = 0;
        pthread_cond_broadcast(&m->cond);
        pthread_mutex_unlock(&m->mutex);
    }
    return NULL;
}

pa_threaded_mainloop *pa_threaded_mainloop_new(void) {
    pa_threaded_mainloop *m = calloc(1, sizeof(pa_threaded_mainloop));
    if (!m) return NULL;
    pthread_mutexattr_t attr;
    pthread_mutexattr_init(&attr);
    pthread_mutexattr_settype(&attr, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&m->mutex, &attr);
    pthread_mutexattr_destroy(&attr);
    pthread_cond_init(&m->cond, NULL);
    m->api.userdata = m;
    return m;
}

void pa_threaded_mainloop_free(pa_threaded_mainloop *m) {
    if (!m) return;
    if (m->running) {
        m->running = 0;
        pthread_join(m->thread, NULL);
    }
    pthread_mutex_destroy(&m->mutex);
    pthread_cond_destroy(&m->cond);
    free(m);
}

int pa_threaded_mainloop_start(pa_threaded_mainloop *m) {
    if (!m) return -1;
    if (!m->running) {
        m->running = 1;
        pthread_create(&m->thread, NULL, mainloop_worker, m);
    }
    return 0;
}

void pa_threaded_mainloop_stop(pa_threaded_mainloop *m) {
    if (!m || !m->running) return;
    m->running = 0;
    pthread_join(m->thread, NULL);
}

void pa_threaded_mainloop_lock(pa_threaded_mainloop *m) {
    if (m) pthread_mutex_lock(&m->mutex);
}

void pa_threaded_mainloop_unlock(pa_threaded_mainloop *m) {
    if (m) pthread_mutex_unlock(&m->mutex);
}

void pa_threaded_mainloop_wait(pa_threaded_mainloop *m) {
    if (!m) return;
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    ts.tv_nsec += 10000000L; /* 10ms max wait safety */
    if (ts.tv_nsec >= 1000000000L) {
        ts.tv_nsec -= 1000000000L;
        ts.tv_sec += 1;
    }
    pthread_cond_timedwait(&m->cond, &m->mutex, &ts);
}

void pa_threaded_mainloop_signal(pa_threaded_mainloop *m, int wait_for_accept) {
    (void)wait_for_accept;
    if (m) pthread_cond_broadcast(&m->cond);
}

void pa_threaded_mainloop_accept(pa_threaded_mainloop *m) {
    (void)m;
}

int pa_threaded_mainloop_in_thread(pa_threaded_mainloop *m) {
    return m ? m->in_thread : 0;
}

pa_mainloop_api *pa_threaded_mainloop_get_api(pa_threaded_mainloop *m) {
    return m ? &m->api : NULL;
}

void pa_mainloop_api_once(pa_mainloop_api *a, void (*callback)(pa_mainloop_api *m, void *userdata), void *userdata) {
    if (a && callback) callback(a, userdata);
}

/* ============================================================================
 * Context & Introspection API (Used by Firefox cubeb device enumeration)
 * ============================================================================ */

pa_context *pa_context_new(pa_mainloop_api *mainloop, const char *name) {
    (void)name;
    pa_context *c = calloc(1, sizeof(pa_context));
    if (!c) return NULL;
    c->ref = 1;
    c->state = PA_CONTEXT_UNCONNECTED;
    if (mainloop && mainloop->userdata) {
        c->mainloop = (pa_threaded_mainloop *)mainloop->userdata;
        c->mainloop->ctx = c;
    }
    return c;
}

pa_context *pa_context_new_with_proplist(pa_mainloop_api *mainloop, const char *name, const pa_proplist *proplist) {
    (void)proplist;
    return pa_context_new(mainloop, name);
}

pa_context *pa_context_ref(pa_context *c) {
    if (c) c->ref++;
    return c;
}

void pa_context_unref(pa_context *c) {
    if (!c) return;
    if (--c->ref <= 0) {
        if (c->mainloop && c->mainloop->ctx == c) c->mainloop->ctx = NULL;
        free(c);
    }
}

void pa_context_set_state_callback(pa_context *c, pa_context_notify_cb_t cb, void *userdata) {
    if (!c) return;
    c->state_cb = cb;
    c->state_userdata = userdata;
}

int pa_context_connect(pa_context *c, const char *server, pa_context_flags_t flags, const pa_spawn_api *api) {
    (void)flags; (void)api;
    if (!c) return -1;
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_PA_CTX", server ? server : "default",
                             "libpulse shim pa_context_connect READY");
    c->state = PA_CONTEXT_READY;
    if (c->state_cb) c->state_cb(c, c->state_userdata);
    if (c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return 0;
}

void pa_context_disconnect(pa_context *c) {
    if (!c) return;
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_PA_DISC", "libpulse-shim",
                             "pa_context_disconnect");
    c->state = PA_CONTEXT_TERMINATED;
    if (c->state_cb) c->state_cb(c, c->state_userdata);
}

pa_context_state_t pa_context_get_state(const pa_context *c) {
    return c ? c->state : PA_CONTEXT_FAILED;
}

int pa_context_errno(const pa_context *c) {
    (void)c;
    return 0;
}

pa_operation *pa_context_drain(pa_context *c, pa_context_notify_cb_t cb, void *userdata) {
    if (cb) cb(c, userdata);
    return alloc_done_op();
}

pa_time_event *pa_context_rttime_new(const pa_context *c, pa_usec_t usec, pa_time_event_cb_t cb, void *userdata) {
    (void)c; (void)usec; (void)cb; (void)userdata;
    return NULL;
}

void pa_context_set_subscribe_callback(pa_context *c, pa_context_subscribe_cb_t cb, void *userdata) {
    if (!c) return;
    c->sub_cb = cb;
    c->sub_userdata = userdata;
}

pa_operation *pa_context_subscribe(pa_context *c, pa_subscription_mask_t m, pa_context_success_cb_t cb, void *userdata) {
    (void)m;
    if (cb) cb(c, 1, userdata);
    return alloc_done_op();
}

pa_operation *pa_context_get_server_info(pa_context *c, pa_server_info_cb_t cb, void *userdata) {
    if (cb) {
        pa_server_info info;
        memset(&info, 0, sizeof(info));
        info.user_name = "will";
        info.host_name = "CachyOS-LWASAPI";
        info.server_version = "16.1.0";
        info.server_name = "LWASAPI-Universal-AudioEngine";
        info.sample_spec.format = PA_SAMPLE_FLOAT32LE;
        info.sample_spec.rate = 48000;
        info.sample_spec.channels = 2;
        info.default_sink_name = "lwasapi_default_sink";
        info.default_source_name = "lwasapi_default_source";
        pa_channel_map_init_stereo(&info.channel_map);
        cb(c, &info, userdata);
    }
    if (c && c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return alloc_done_op();
}

static void fill_default_sink_info(pa_sink_info *si) {
    memset(si, 0, sizeof(*si));
    si->name = "lwasapi_default_sink";
    si->index = 0;
    si->description = "LWASAPI Universal Windows-Style Audio Sink";
    si->sample_spec.format = PA_SAMPLE_FLOAT32LE;
    si->sample_spec.rate = 48000;
    si->sample_spec.channels = 2;
    pa_channel_map_init_stereo(&si->channel_map);
    pa_cvolume_set(&si->volume, 2, PA_VOLUME_NORM);
    si->n_volume_steps = 65536;
    si->card = 0;
    si->monitor_source = 1;
    si->monitor_source_name = "lwasapi_default_sink.monitor";
    si->state = PA_SINK_RUNNING;
    si->flags = PA_SINK_HARDWARE | PA_SINK_LATENCY;
}

static void fill_source_info(pa_source_info *src, int is_monitor) {
    memset(src, 0, sizeof(*src));
    if (is_monitor) {
        src->name = "lwasapi_default_sink.monitor";
        src->index = 1;
        src->description = "Monitor of LWASAPI Universal Windows-Style Audio Sink (Desktop Loopback)";
        src->monitor_of_sink = 0;
        src->monitor_of_sink_name = "lwasapi_default_sink";
    } else {
        src->name = "lwasapi_default_source";
        src->index = 0;
        src->description = "LWASAPI Universal Microphone Input (Direct Kernel ALSA)";
        src->monitor_of_sink = PA_INVALID_INDEX;
        src->monitor_of_sink_name = NULL;
    }
    src->sample_spec.format = PA_SAMPLE_FLOAT32LE;
    src->sample_spec.rate = 48000;
    src->sample_spec.channels = 2;
    pa_channel_map_init_stereo(&src->channel_map);
    pa_cvolume_set(&src->volume, 2, PA_VOLUME_NORM);
    src->n_volume_steps = 65536;
    src->card = 0;
    src->state = PA_SOURCE_RUNNING;
    src->flags = PA_SOURCE_HARDWARE | PA_SOURCE_LATENCY;
}

pa_operation *pa_context_get_sink_info_by_name(pa_context *c, const char *name, pa_sink_info_cb_t cb, void *userdata) {
    (void)name;
    if (cb) {
        pa_sink_info si;
        fill_default_sink_info(&si);
        cb(c, &si, 0, userdata);
        cb(c, NULL, 1, userdata);
    }
    if (c && c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return alloc_done_op();
}

pa_operation *pa_context_get_sink_info_by_index(pa_context *c, uint32_t idx, pa_sink_info_cb_t cb, void *userdata) {
    (void)idx;
    return pa_context_get_sink_info_by_name(c, NULL, cb, userdata);
}

pa_operation *pa_context_get_sink_info_list(pa_context *c, pa_sink_info_cb_t cb, void *userdata) {
    return pa_context_get_sink_info_by_name(c, NULL, cb, userdata);
}

pa_operation *pa_context_get_source_info_by_name(pa_context *c, const char *name, pa_source_info_cb_t cb, void *userdata) {
    if (cb) {
        pa_source_info src;
        int is_mon = (name && (strstr(name, "monitor") || strstr(name, "loopback"))) ? 1 : 0;
        fill_source_info(&src, is_mon);
        cb(c, &src, 0, userdata);
        cb(c, NULL, 1, userdata);
    }
    if (c && c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return alloc_done_op();
}

pa_operation *pa_context_get_source_info_by_index(pa_context *c, uint32_t idx, pa_source_info_cb_t cb, void *userdata) {
    if (cb) {
        pa_source_info src;
        fill_source_info(&src, idx == 1 ? 1 : 0);
        cb(c, &src, 0, userdata);
        cb(c, NULL, 1, userdata);
    }
    if (c && c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return alloc_done_op();
}

pa_operation *pa_context_get_source_info_list(pa_context *c, pa_source_info_cb_t cb, void *userdata) {
    if (cb) {
        pa_source_info mic_src, mon_src;
        fill_source_info(&mic_src, 0);
        fill_source_info(&mon_src, 1);
        cb(c, &mic_src, 0, userdata);
        cb(c, &mon_src, 0, userdata);
        cb(c, NULL, 1, userdata);
    }
    if (c && c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return alloc_done_op();
}

pa_operation *pa_context_get_sink_input_info(pa_context *c, uint32_t idx, pa_sink_input_info_cb_t cb, void *userdata) {
    if (cb) {
        pa_sink_input_info ii;
        memset(&ii, 0, sizeof(ii));
        ii.index = idx;
        ii.name = "Firefox-Stream";
        ii.sample_spec.format = PA_SAMPLE_FLOAT32LE;
        ii.sample_spec.rate = 48000;
        ii.sample_spec.channels = 2;
        pa_channel_map_init_stereo(&ii.channel_map);
        pa_cvolume_set(&ii.volume, 2, PA_VOLUME_NORM);
        ii.has_volume = 1;
        ii.volume_writable = 1;
        cb(c, &ii, 0, userdata);
        cb(c, NULL, 1, userdata);
    }
    if (c && c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return alloc_done_op();
}

pa_operation *pa_context_set_sink_input_volume(pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata) {
    if (c && volume && volume->channels > 0) {
        float linear = (float)volume->values[0] / (float)PA_VOLUME_NORM;
        for (int i = 0; i < MAX_PA_STREAMS; i++) {
            if (c->streams[i] && c->streams[i]->lwasapi && c->streams[i]->index == idx) {
                ISimpleAudioVolume_SetMasterVolume(c->streams[i]->lwasapi, linear);
            }
        }
    }
    if (cb) cb(c, 1, userdata);
    if (c && c->mainloop) pthread_cond_broadcast(&c->mainloop->cond);
    return alloc_done_op();
}

/* ============================================================================
 * Stream API (Maps Firefox cubeb streams directly into LWASAPI Shared Mode)
 * ============================================================================ */

pa_stream *pa_stream_new(pa_context *c, const char *name, const pa_sample_spec *ss, const pa_channel_map *map) {
    if (!c || !ss) return NULL;
    pa_stream *s = calloc(1, sizeof(pa_stream));
    if (!s) return NULL;
    s->ref = 1;
    s->ctx = c;
    snprintf(s->name, sizeof(s->name), "%s", name ? name : "AudioStream");
    s->ss = *ss;
    if (map) {
        s->cmap = *map;
    } else {
        pa_channel_map_init_auto(&s->cmap, ss->channels, PA_CHANNEL_MAP_DEFAULT);
    }
    uint32_t bps = 4;
    s->lfmt = map_pa_fmt(ss->format, &bps);
    s->frame_bytes = (ss->channels ? ss->channels : 2) * bps;
    s->state = PA_STREAM_UNCONNECTED;
    s->write_scratch_cap = 65536;
    s->write_scratch = calloc(1, s->write_scratch_cap);
    s->read_scratch_cap = 65536;
    s->read_scratch = calloc(1, s->read_scratch_cap);

    for (int i = 0; i < MAX_PA_STREAMS; i++) {
        if (!c->streams[i]) {
            c->streams[i] = s;
            s->index = (uint32_t)(i + 1);
            break;
        }
    }
    return s;
}

pa_stream *pa_stream_new_with_proplist(pa_context *c, const char *name, const pa_sample_spec *ss, const pa_channel_map *map, pa_proplist *p) {
    (void)p;
    return pa_stream_new(c, name, ss, map);
}

pa_stream *pa_stream_ref(pa_stream *s) {
    if (s) s->ref++;
    return s;
}

void pa_stream_unref(pa_stream *s) {
    if (!s) return;
    if (--s->ref <= 0) {
        if (s->ctx) {
            for (int i = 0; i < MAX_PA_STREAMS; i++) {
                if (s->ctx->streams[i] == s) s->ctx->streams[i] = NULL;
            }
        }
        if (s->lwasapi) {
            IAudioClient_Release(s->lwasapi);
            s->lwasapi = NULL;
        }
        free(s->write_scratch);
        free(s->read_scratch);
        free(s);
    }
}

pa_stream_state_t pa_stream_get_state(const pa_stream *s) {
    return s ? s->state : PA_STREAM_FAILED;
}

pa_context *pa_stream_get_context(const pa_stream *s) {
    return s ? s->ctx : NULL;
}

uint32_t pa_stream_get_index(const pa_stream *s) {
    return s ? s->index : 0;
}

const char *pa_stream_get_device_name(const pa_stream *s) {
    if (s && s->is_record) return "lwasapi_default_source";
    return "lwasapi_default_sink";
}

void pa_stream_set_state_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    if (!s) return;
    s->state_cb = cb;
    s->state_userdata = userdata;
}

void pa_stream_set_write_callback(pa_stream *s, pa_stream_request_cb_t cb, void *userdata) {
    if (!s) return;
    s->write_cb = cb;
    s->write_userdata = userdata;
}

void pa_stream_set_read_callback(pa_stream *s, pa_stream_request_cb_t cb, void *userdata) {
    if (!s) return;
    s->read_cb = cb;
    s->read_userdata = userdata;
}

int pa_stream_connect_playback(
    pa_stream *s,
    const char *dev,
    const pa_buffer_attr *attr,
    pa_stream_flags_t flags,
    const pa_cvolume *volume,
    pa_stream *sync_stream
) {
    (void)dev; (void)sync_stream;
    if (!s) return -1;
    s->is_record = false;

    if (LWASAPI_ActivateAudioClient(s->name, &s->lwasapi) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "SHIM_PA_ERR", s->name, "ActivateAudioClient failed (playback)");
        s->state = PA_STREAM_FAILED;
        return -1;
    }

    WAVEFORMATEX wfx;
    memset(&wfx, 0, sizeof(wfx));
    wfx.wFormatTag = (uint16_t)s->lfmt;
    wfx.nChannels = s->ss.channels ? s->ss.channels : 2;
    wfx.nSamplesPerSec = s->ss.rate ? s->ss.rate : 48000;
    wfx.wBitsPerSample = (s->lfmt == LWASAPI_FMT_S16_LE) ? 16 : 32;
    wfx.nBlockAlign = (uint16_t)s->frame_bytes;
    wfx.nAvgBytesPerSec = wfx.nSamplesPerSec * s->frame_bytes;

    if (IAudioClient_Initialize(s->lwasapi, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 4096, 256, &wfx) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "SHIM_PA_ERR", s->name, "IAudioClient_Initialize failed (playback)");
        IAudioClient_Release(s->lwasapi);
        s->lwasapi = NULL;
        s->state = PA_STREAM_FAILED;
        return -1;
    }

    if (volume && volume->channels > 0) {
        float linear = (float)volume->values[0] / (float)PA_VOLUME_NORM;
        ISimpleAudioVolume_SetMasterVolume(s->lwasapi, linear);
    }

    s->battr.maxlength = 4096 * s->frame_bytes;
    s->battr.tlength   = (attr && attr->tlength != (uint32_t)-1) ? attr->tlength : (1024 * s->frame_bytes);
    s->battr.prebuf    = 0;
    s->battr.minreq    = 256 * s->frame_bytes;
    s->battr.fragsize  = 256 * s->frame_bytes;

    s->corked = (flags & PA_STREAM_START_CORKED) ? 1 : 0;
    if (!s->corked) {
        IAudioClient_Start(s->lwasapi);
    }

    char detail[128];
    snprintf(detail, sizeof(detail), "rate=%u ch=%u tlen=%u corked=%d dev=%s",
             wfx.nSamplesPerSec, wfx.nChannels, s->battr.tlength, s->corked,
             dev ? dev : "default");
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_PA_PLAY", s->name, detail);

    s->state = PA_STREAM_READY;
    if (s->state_cb) s->state_cb(s, s->state_userdata);
    if (s->ctx && s->ctx->mainloop) pthread_cond_broadcast(&s->ctx->mainloop->cond);
    return 0;
}

int pa_stream_connect_record(pa_stream *s, const char *dev, const pa_buffer_attr *attr, pa_stream_flags_t flags) {
    if (!s) return -1;
    s->is_record = true;

    if (LWASAPI_ActivateAudioClient(s->name, &s->lwasapi) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "SHIM_PA_ERR", s->name, "ActivateAudioClient failed (record)");
        s->state = PA_STREAM_FAILED;
        return -1;
    }

    WAVEFORMATEX wfx;
    memset(&wfx, 0, sizeof(wfx));
    wfx.wFormatTag = (uint16_t)s->lfmt;
    wfx.nChannels = s->ss.channels ? s->ss.channels : 2;
    wfx.nSamplesPerSec = s->ss.rate ? s->ss.rate : 48000;
    wfx.wBitsPerSample = (s->lfmt == LWASAPI_FMT_S16_LE) ? 16 : 32;
    wfx.nBlockAlign = (uint16_t)s->frame_bytes;
    wfx.nAvgBytesPerSec = wfx.nSamplesPerSec * s->frame_bytes;

    uint32_t lflags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if (dev && (strstr(dev, "monitor") || strstr(dev, "loopback"))) {
        lflags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    } else {
        lflags |= AUDCLNT_STREAMFLAGS_CAPTURE;
    }

    if (IAudioClient_Initialize(s->lwasapi, AUDCLNT_SHAREMODE_SHARED, lflags, 4096, 256, &wfx) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "SHIM_PA_ERR", s->name, "IAudioClient_Initialize failed (record)");
        IAudioClient_Release(s->lwasapi);
        s->lwasapi = NULL;
        s->state = PA_STREAM_FAILED;
        return -1;
    }

    s->battr.maxlength = 4096 * s->frame_bytes;
    s->battr.fragsize  = (attr && attr->fragsize != (uint32_t)-1) ? attr->fragsize : (256 * s->frame_bytes);

    s->corked = (flags & PA_STREAM_START_CORKED) ? 1 : 0;
    if (!s->corked) {
        IAudioClient_Start(s->lwasapi);
    }

    char detail[128];
    snprintf(detail, sizeof(detail), "rate=%u ch=%u flags=0x%x dev=%s",
             wfx.nSamplesPerSec, wfx.nChannels, lflags, dev ? dev : "default");
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_PA_REC", s->name, detail);

    s->state = PA_STREAM_READY;
    if (s->state_cb) s->state_cb(s, s->state_userdata);
    if (s->ctx && s->ctx->mainloop) pthread_cond_broadcast(&s->ctx->mainloop->cond);
    return 0;
}

int pa_stream_disconnect(pa_stream *s) {
    if (!s) return -1;
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_PA_CLOSE", s->name, "pa_stream_disconnect");
    if (s->lwasapi) {
        IAudioClient_Stop(s->lwasapi);
        IAudioClient_Release(s->lwasapi);
        s->lwasapi = NULL;
    }
    s->state = PA_STREAM_TERMINATED;
    if (s->state_cb) s->state_cb(s, s->state_userdata);
    if (s->ctx && s->ctx->mainloop) pthread_cond_broadcast(&s->ctx->mainloop->cond);
    return 0;
}

size_t pa_stream_writable_size(const pa_stream *s) {
    if (!s || !s->lwasapi || s->is_record) return 0;
    uint32_t buf = 2048, pad = 0;
    IAudioClient_GetBufferSize(s->lwasapi, &buf);
    IAudioClient_GetCurrentPadding(s->lwasapi, &pad);
    return (size_t)((buf > pad) ? (buf - pad) : 0) * s->frame_bytes;
}

size_t pa_stream_readable_size(const pa_stream *s) {
    if (!s || !s->lwasapi || !s->is_record) return 0;
    uint32_t pkt = 0;
    IAudioCaptureClient_GetNextPacketSize(s->lwasapi, &pkt);
    return (size_t)pkt * s->frame_bytes;
}

int pa_stream_peek(pa_stream *s, const void **data, size_t *nbytes) {
    if (!s || !s->lwasapi || !data || !nbytes) return -1;
    if (s->read_scratch_len == 0) {
        uint32_t pkt = 0;
        IAudioCaptureClient_GetNextPacketSize(s->lwasapi, &pkt);
        if (pkt == 0) {
            *data = NULL;
            *nbytes = 0;
            return 0;
        }
        uint32_t max_frames = (uint32_t)(s->read_scratch_cap / s->frame_bytes);
        if (pkt > max_frames) pkt = max_frames;
        IAudioCaptureClient_ReadFrames(s->lwasapi, s->read_scratch, pkt, s->lfmt);
        s->read_scratch_len = (size_t)pkt * s->frame_bytes;
    }
    *data = s->read_scratch;
    *nbytes = s->read_scratch_len;
    return 0;
}

int pa_stream_drop(pa_stream *s) {
    if (!s) return -1;
    s->read_scratch_len = 0;
    return 0;
}

int pa_stream_begin_write(pa_stream *s, void **data, size_t *nbytes) {
    if (!s || !data || !nbytes) return -1;
    size_t req = (*nbytes == (size_t)-1 || *nbytes == 0) ? (1024 * s->frame_bytes) : *nbytes;
    if (req > s->write_scratch_cap) {
        void *nb = realloc(s->write_scratch, req);
        if (!nb) return -1;
        s->write_scratch = nb;
        s->write_scratch_cap = req;
    }
    *data = s->write_scratch;
    *nbytes = req;
    return 0;
}

int pa_stream_cancel_write(pa_stream *s) {
    (void)s;
    return 0;
}

int pa_stream_write(pa_stream *s, const void *data, size_t nbytes, pa_free_cb_t free_cb, int64_t offset, pa_seek_mode_t seek) {
    (void)offset; (void)seek;
    if (!s || !data || !s->lwasapi || s->frame_bytes == 0) return -1;
    uint32_t frames = (uint32_t)(nbytes / s->frame_bytes);
    IAudioRenderClient_WriteFrames(s->lwasapi, data, frames, s->lfmt);
    s->bytes_written += nbytes;
    if (free_cb && data != s->write_scratch) {
        free_cb((void *)data);
    }
    return 0;
}

pa_operation *pa_stream_cork(pa_stream *s, int b, pa_stream_success_cb_t cb, void *userdata) {
    if (s) {
        s->corked = b ? 1 : 0;
        if (s->lwasapi) {
            if (s->corked) IAudioClient_Stop(s->lwasapi);
            else IAudioClient_Start(s->lwasapi);
        }
    }
    if (cb) cb(s, 1, userdata);
    if (s && s->ctx && s->ctx->mainloop) pthread_cond_broadcast(&s->ctx->mainloop->cond);
    return alloc_done_op();
}

int pa_stream_is_corked(const pa_stream *s) {
    return s ? s->corked : 0;
}

int pa_stream_is_suspended(const pa_stream *s) {
    (void)s;
    return 0;
}

const pa_buffer_attr *pa_stream_get_buffer_attr(pa_stream *s) {
    return s ? &s->battr : NULL;
}

const pa_sample_spec *pa_stream_get_sample_spec(pa_stream *s) {
    return s ? &s->ss : NULL;
}

const pa_channel_map *pa_stream_get_channel_map(pa_stream *s) {
    return s ? &s->cmap : NULL;
}

pa_operation *pa_stream_set_name(pa_stream *s, const char *name, pa_stream_success_cb_t cb, void *userdata) {
    if (s && name) snprintf(s->name, sizeof(s->name), "%s", name);
    if (cb) cb(s, 1, userdata);
    return alloc_done_op();
}

pa_operation *pa_stream_update_timing_info(pa_stream *s, pa_stream_success_cb_t cb, void *userdata) {
    if (cb) cb(s, 1, userdata);
    if (s && s->ctx && s->ctx->mainloop) pthread_cond_broadcast(&s->ctx->mainloop->cond);
    return alloc_done_op();
}

int pa_stream_get_time(pa_stream *s, pa_usec_t *r_usec) {
    if (!s || !r_usec || s->frame_bytes == 0 || s->ss.rate == 0) return -1;
    uint32_t pad = 0;
    if (s->lwasapi) IAudioClient_GetCurrentPadding(s->lwasapi, &pad);
    uint64_t total_frames = s->bytes_written / s->frame_bytes;
    uint64_t played_frames = (total_frames > pad) ? (total_frames - pad) : 0;
    *r_usec = (played_frames * 1000000ULL) / s->ss.rate;
    return 0;
}

int pa_stream_get_latency(pa_stream *s, pa_usec_t *r_usec, int *negative) {
    if (negative) *negative = 0;
    if (!s || !r_usec || s->ss.rate == 0) return -1;
    uint32_t pad = 0;
    if (s->lwasapi) IAudioClient_GetCurrentPadding(s->lwasapi, &pad);
    *r_usec = ((uint64_t)pad * 1000000ULL) / s->ss.rate;
    return 0;
}

/* ============================================================================
 * Operation, Proplist, ChannelMap, CVolume, and Utility Helpers
 * ============================================================================ */

pa_operation *pa_operation_ref(pa_operation *o) {
    if (o) o->ref++;
    return o;
}

void pa_operation_unref(pa_operation *o) {
    if (!o) return;
    if (--o->ref <= 0) free(o);
}

void pa_operation_cancel(pa_operation *o) {
    if (o) o->state = PA_OPERATION_CANCELED;
}

pa_operation_state_t pa_operation_get_state(const pa_operation *o) {
    return o ? o->state : PA_OPERATION_DONE;
}

void pa_operation_set_state_callback(pa_operation *o, pa_operation_notify_cb_t cb, void *userdata) {
    if (!o) return;
    o->cb = cb;
    o->userdata = userdata;
}

pa_proplist *pa_proplist_new(void) {
    return calloc(1, sizeof(pa_proplist));
}

void pa_proplist_free(pa_proplist *p) {
    free(p);
}

int pa_proplist_sets(pa_proplist *p, const char *key, const char *value) {
    (void)key;
    if (p && value) snprintf(p->name, sizeof(p->name), "%s", value);
    return 0;
}

const char *pa_proplist_gets(const pa_proplist *p, const char *key) {
    (void)key;
    return p ? p->name : NULL;
}

pa_channel_map *pa_channel_map_init(pa_channel_map *m) {
    if (m) memset(m, 0, sizeof(*m));
    return m;
}

pa_channel_map *pa_channel_map_init_stereo(pa_channel_map *m) {
    if (!m) return NULL;
    m->channels = 2;
    m->map[0] = PA_CHANNEL_POSITION_FRONT_LEFT;
    m->map[1] = PA_CHANNEL_POSITION_FRONT_RIGHT;
    return m;
}

pa_channel_map *pa_channel_map_init_auto(pa_channel_map *m, unsigned channels, pa_channel_map_def_t def) {
    (void)def;
    if (!m) return NULL;
    m->channels = (uint8_t)(channels ? channels : 2);
    for (unsigned i = 0; i < m->channels && i < PA_CHANNELS_MAX; i++) {
        m->map[i] = (i == 0) ? PA_CHANNEL_POSITION_FRONT_LEFT : PA_CHANNEL_POSITION_FRONT_RIGHT;
    }
    return m;
}

int pa_channel_map_can_balance(const pa_channel_map *map) {
    (void)map;
    return 1;
}

pa_cvolume *pa_cvolume_set(pa_cvolume *a, unsigned channels, pa_volume_t v) {
    if (!a) return NULL;
    a->channels = (uint8_t)channels;
    for (unsigned i = 0; i < channels && i < PA_CHANNELS_MAX; i++) {
        a->values[i] = v;
    }
    return a;
}

pa_cvolume *pa_cvolume_set_balance(pa_cvolume *v, const pa_channel_map *map, float new_balance) {
    (void)map; (void)new_balance;
    return v;
}

pa_volume_t pa_sw_volume_from_linear(double v) {
    if (v <= 0.0) return PA_VOLUME_MUTED;
    return (pa_volume_t)(v * (double)PA_VOLUME_NORM);
}

size_t pa_frame_size(const pa_sample_spec *spec) {
    if (!spec) return 8;
    uint32_t bps = 4;
    map_pa_fmt(spec->format, &bps);
    return (size_t)spec->channels * bps;
}

size_t pa_sample_size(const pa_sample_spec *spec) {
    if (!spec) return 4;
    uint32_t bps = 4;
    map_pa_fmt(spec->format, &bps);
    return bps;
}

size_t pa_usec_to_bytes(pa_usec_t t, const pa_sample_spec *spec) {
    if (!spec) return 0;
    return (size_t)((t * spec->rate / 1000000ULL) * pa_frame_size(spec));
}

pa_usec_t pa_rtclock_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (pa_usec_t)ts.tv_sec * 1000000ULL + (pa_usec_t)(ts.tv_nsec / 1000ULL);
}

const char *pa_get_library_version(void) {
    return "16.1.0";
}

void *pa_xrealloc(void *ptr, size_t size) {
    return realloc(ptr, size ? size : 1);
}

/* ============================================================================
 * Simple API (pa_simple_new / pa_simple_write / pa_simple_read)
 * ============================================================================ */

pa_simple *pa_simple_new(
    const char *server,
    const char *name,
    pa_stream_direction_t dir,
    const char *dev,
    const char *stream_name,
    const pa_sample_spec *ss,
    const pa_channel_map *map,
    const pa_buffer_attr *attr,
    int *error
) {
    (void)server; (void)map;
    if (!ss) {
        if (error) *error = -1;
        return NULL;
    }

    pa_simple *s = calloc(1, sizeof(pa_simple));
    if (!s) {
        if (error) *error = -2;
        return NULL;
    }

    uint32_t bps = 4;
    s->fmt = map_pa_fmt(ss->format, &bps);
    s->rate = ss->rate ? ss->rate : 48000;
    s->channels = ss->channels ? ss->channels : 2;
    s->frame_bytes = s->channels * bps;
    s->is_record = (dir == PA_STREAM_RECORD);

    const char *label = (stream_name && stream_name[0]) ? stream_name : (name ? name : "stream");
    if (LWASAPI_ActivateAudioClient(label, &s->client) != S_OK) {
        free(s);
        if (error) *error = -3;
        return NULL;
    }

    WAVEFORMATEX wfx;
    memset(&wfx, 0, sizeof(wfx));
    wfx.wFormatTag = (uint16_t)s->fmt;
    wfx.nChannels = (uint16_t)s->channels;
    wfx.nSamplesPerSec = s->rate;
    wfx.wBitsPerSample = (uint16_t)(bps * 8);
    wfx.nBlockAlign = (uint16_t)s->frame_bytes;
    wfx.nAvgBytesPerSec = s->rate * s->frame_bytes;

    uint32_t flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if (s->is_record) {
        if (dev && (strstr(dev, "monitor") || strstr(dev, "loopback"))) {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        } else {
            flags |= AUDCLNT_STREAMFLAGS_CAPTURE;
        }
    }

    uint32_t buf_frames = 2048;
    uint32_t period_frames = 256;
    if (attr && s->frame_bytes > 0) {
        if (attr->maxlength != (uint32_t)-1 && attr->maxlength >= s->frame_bytes) {
            buf_frames = attr->maxlength / s->frame_bytes;
        } else if (attr->tlength != (uint32_t)-1 && attr->tlength >= s->frame_bytes) {
            buf_frames = attr->tlength / s->frame_bytes;
        }
        if (buf_frames < 512U) buf_frames = 512U;
        if (buf_frames > LWASAPI_RING_FRAMES) buf_frames = LWASAPI_RING_FRAMES;

        if (!s->is_record && attr->minreq != (uint32_t)-1 && attr->minreq >= s->frame_bytes) {
            period_frames = attr->minreq / s->frame_bytes;
        } else if (s->is_record && attr->fragsize != (uint32_t)-1 && attr->fragsize >= s->frame_bytes) {
            period_frames = attr->fragsize / s->frame_bytes;
        }
        if (period_frames < 64U) period_frames = 64U;
        if (period_frames > buf_frames / 2U) period_frames = buf_frames / 2U;
    }

    if (IAudioClient_Initialize(
            s->client,
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            buf_frames,
            period_frames,
            &wfx) != S_OK) {
        IAudioClient_Release(s->client);
        free(s);
        if (error) *error = -4;
        return NULL;
    }

    IAudioClient_Start(s->client);
    if (error) *error = 0;
    return s;
}

int pa_simple_write(pa_simple *s, const void *data, size_t bytes, int *error) {
    if (!s || !data || s->frame_bytes == 0) {
        if (error) *error = -1;
        return -1;
    }
    uint32_t frames = (uint32_t)(bytes / s->frame_bytes);
    if (IAudioRenderClient_WriteFrames(s->client, data, frames, s->fmt) != S_OK) {
        if (error) *error = -5;
        return -1;
    }
    if (error) *error = 0;
    return 0;
}

int pa_simple_read(pa_simple *s, void *data, size_t bytes, int *error) {
    if (!s || !data || s->frame_bytes == 0) {
        if (error) *error = -1;
        return -1;
    }
    uint32_t frames = (uint32_t)(bytes / s->frame_bytes);
    uint32_t pkt = 0;
    IAudioCaptureClient_GetNextPacketSize(s->client, &pkt);
    if (pkt < frames) {
        IAudioClient_WaitForPeriod(s->client, 15);
    }
    if (IAudioCaptureClient_ReadFrames(s->client, data, frames, s->fmt) != S_OK) {
        if (error) *error = -5;
        return -1;
    }
    if (error) *error = 0;
    return 0;
}

int pa_simple_drain(pa_simple *s, int *error) {
    if (!s) return -1;
    if (!s->is_record) {
        for (int i = 0; i < 50; i++) {
            uint32_t pad = 0;
            IAudioClient_GetCurrentPadding(s->client, &pad);
            if (pad == 0) break;
            IAudioClient_WaitForPeriod(s->client, 10);
        }
    }
    if (error) *error = 0;
    return 0;
}

int pa_simple_flush(pa_simple *s, int *error) {
    if (!s) return -1;
    IAudioClient_Reset(s->client);
    if (error) *error = 0;
    return 0;
}

pa_usec_t pa_simple_get_latency(pa_simple *s, int *error) {
    if (error) *error = 0;
    if (!s) return 0;
    uint32_t pad = 0;
    IAudioClient_GetCurrentPadding(s->client, &pad);
    return ((pa_usec_t)pad * 1000000ULL) / s->rate;
}

void pa_simple_free(pa_simple *s) {
    if (!s) return;
    if (s->client) IAudioClient_Release(s->client);
    free(s);
}

const char *pa_strerror(int error) {
    (void)error;
    return "LWASAPI PulseAudio Shim Status";
}

/* ============================================================================
 * GLib Mainloop API (libpulse-mainloop-glib.so.0 for GTK, Canberra, Electron)
 * ============================================================================ */

void *pa_glib_mainloop_new(void *glib_context) {
    (void)glib_context;
    pa_threaded_mainloop *m = pa_threaded_mainloop_new();
    if (m) pa_threaded_mainloop_start(m);
    return m;
}

void pa_glib_mainloop_free(void *g) {
    if (g) pa_threaded_mainloop_free((pa_threaded_mainloop *)g);
}

pa_mainloop_api *pa_glib_mainloop_get_api(void *g) {
    return g ? pa_threaded_mainloop_get_api((pa_threaded_mainloop *)g) : NULL;
}
