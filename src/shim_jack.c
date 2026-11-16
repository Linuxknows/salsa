#define _GNU_SOURCE
#include "../include/lwasapi.h"
#include "../include/lwasapi_debug.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>

/* ============================================================================
 * Hollow ABI Shim for JACK Pro-Audio & MIDI (libjack.so.0 -> LWASAPI Ring)
 * Drives JACK process() callbacks + sample-accurate MIDI directly off LWASAPI!
 * ============================================================================ */

typedef uint32_t      jack_nframes_t;
typedef float         jack_default_audio_sample_t;
typedef unsigned char jack_midi_data_t;
typedef int (*JackProcessCallback)(jack_nframes_t nframes, void *arg);

#define JACK_MAX_PORT_MIDI_EVENTS 128

typedef struct _jack_midi_event {
    jack_nframes_t    time;
    size_t            size;
    jack_midi_data_t *buffer;
} jack_midi_event_t;

typedef struct {
    uint32_t             magic;
    IAudioClient        *lwasapi_client;
    uint32_t             count;
    lwasapi_midi_event_t events[JACK_MAX_PORT_MIDI_EVENTS];
} jack_midi_buf_t;

typedef struct _jack_port {
    char            name[64];
    char            full_name[128];
    bool            is_midi;
    bool            is_input;
    unsigned long   flags;
    float          *audio_buffer;
    jack_midi_buf_t midi_buffer;
} jack_port_t;

typedef struct _jack_client {
    char                name[64];
    IAudioClient       *lwasapi_client;
    uint32_t            sample_rate;
    uint32_t            period_frames;
    JackProcessCallback process_cb;
    void               *process_arg;
    jack_port_t         ports[LWASAPI_MAX_CHANNELS * 2];
    int                 num_ports;
    volatile int        active;
    volatile int        transport_rolling;
    uint64_t            frame_counter;
    pthread_t           rt_thread;
} jack_client_t;

static void *jack_wavert_callback_thread(void *arg) {
    jack_client_t *jc = (jack_client_t *)arg;
    uint32_t nframes = jc->period_frames;
    float *interleaved = calloc(nframes * 2, sizeof(float));
    float *cap_interleaved = calloc(nframes * 2, sizeof(float));

    while (jc->active) {
        uint32_t pad = 0;
        IAudioClient_GetCurrentPadding(jc->lwasapi_client, &pad);
        if (pad > nframes * 2) {
            IAudioClient_WaitForPeriod(jc->lwasapi_client, 15);
            continue;
        }

        /* Read live microphone / audio interface capture frames for JACK input ports */
        IAudioCaptureClient_ReadFrames(jc->lwasapi_client, cap_interleaved, nframes, LWASAPI_FMT_FLOAT32);

        /* Read any incoming hardware/sequencer MIDI events from LWASAPI shared memory ring */
        lwasapi_midi_event_t incoming_midi[JACK_MAX_PORT_MIDI_EVENTS];
        uint32_t midi_count = IAudioClient_ReadMidiEvents(
            jc->lwasapi_client, incoming_midi, JACK_MAX_PORT_MIDI_EVENTS);

        int in_ch = 0;
        for (int p = 0; p < jc->num_ports; p++) {
            if (jc->ports[p].is_midi) {
                if (jc->ports[p].is_input) {
                    jc->ports[p].midi_buffer.count = midi_count;
                    if (midi_count > 0) {
                        memcpy(jc->ports[p].midi_buffer.events, incoming_midi,
                               midi_count * sizeof(lwasapi_midi_event_t));
                    }
                } else {
                    jc->ports[p].midi_buffer.count = 0;
                }
            } else if (jc->ports[p].audio_buffer) {
                if (jc->ports[p].is_input) {
                    int ch = in_ch & 1;
                    for (uint32_t f = 0; f < nframes; f++) {
                        jc->ports[p].audio_buffer[f] = cap_interleaved[f * 2 + ch];
                    }
                    in_ch++;
                } else {
                    memset(jc->ports[p].audio_buffer, 0, nframes * sizeof(float));
                }
            }
        }

        if (jc->process_cb) {
            jc->process_cb(nframes, jc->process_arg);
        }

        /* Gather first two audio output ports as Stereo L/R output */
        float *L = NULL, *R = NULL;
        for (int p = 0; p < jc->num_ports; p++) {
            if (!jc->ports[p].is_midi && !jc->ports[p].is_input && jc->ports[p].audio_buffer) {
                if (!L) L = jc->ports[p].audio_buffer;
                else if (!R) { R = jc->ports[p].audio_buffer; break; }
            }
        }
        if (!R) R = L;

        for (uint32_t f = 0; f < nframes; f++) {
            interleaved[f * 2 + 0] = L ? L[f] : 0.0f;
            interleaved[f * 2 + 1] = R ? R[f] : 0.0f;
        }

        IAudioRenderClient_WriteFrames(jc->lwasapi_client, interleaved, nframes, LWASAPI_FMT_FLOAT32);
        jc->frame_counter += nframes;
    }

    free(cap_interleaved);
    free(interleaved);
    return NULL;
}

jack_client_t *jack_client_open(const char *client_name, int options, int *status, ...) {
    (void)options;
    jack_client_t *jc = calloc(1, sizeof(jack_client_t));
    if (!jc) return NULL;

    snprintf(jc->name, sizeof(jc->name), "JACK(%s)", client_name ? client_name : "daw");
    if (LWASAPI_ActivateAudioClient(jc->name, &jc->lwasapi_client) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "SHIM_JACK_ERR", jc->name, "LWASAPI_ActivateAudioClient failed");
        free(jc);
        return NULL;
    }

    WAVEFORMATEX wfx;
    IAudioClient_GetMixFormat(jc->lwasapi_client, &wfx);

    if (IAudioClient_Initialize(
            jc->lwasapi_client,
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            1024,
            256,
            &wfx) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "SHIM_JACK_ERR", jc->name, "IAudioClient_Initialize failed");
        IAudioClient_Release(jc->lwasapi_client);
        free(jc);
        return NULL;
    }

    uint32_t def_per = 256;
    IAudioClient_GetDevicePeriod(jc->lwasapi_client, &def_per, NULL);
    jc->sample_rate = wfx.nSamplesPerSec;
    jc->period_frames = def_per;
    jc->transport_rolling = 1;

    char detail[96];
    snprintf(detail, sizeof(detail), "rate=%u period=%u", jc->sample_rate, jc->period_frames);
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_JACK_OPEN", jc->name, detail);

    if (status) *status = 0;
    return jc;
}

int jack_client_close(jack_client_t *jc) {
    if (!jc) return -1;
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_JACK_CLOSE", jc->name, "jack_client_close");
    if (jc->active) {
        jc->active = 0;
        pthread_join(jc->rt_thread, NULL);
    }
    for (int i = 0; i < jc->num_ports; i++) {
        free(jc->ports[i].audio_buffer);
    }
    if (jc->lwasapi_client) IAudioClient_Release(jc->lwasapi_client);
    free(jc);
    return 0;
}

char *jack_get_client_name(jack_client_t *jc) {
    return jc ? jc->name : "LWASAPI-JACK";
}

int jack_set_process_callback(jack_client_t *jc, JackProcessCallback cb, void *arg) {
    if (!jc) return -1;
    jc->process_cb = cb;
    jc->process_arg = arg;
    return 0;
}

int jack_set_sample_rate_callback(jack_client_t *jc, int (*cb)(jack_nframes_t, void *), void *arg) {
    if (jc && cb) cb(jc->sample_rate, arg);
    return 0;
}

int jack_set_buffer_size_callback(jack_client_t *jc, int (*cb)(jack_nframes_t, void *), void *arg) {
    if (jc && cb) cb(jc->period_frames, arg);
    return 0;
}

int jack_set_xrun_callback(jack_client_t *jc, int (*cb)(void *), void *arg) {
    (void)jc; (void)cb; (void)arg;
    return 0;
}

void jack_on_shutdown(jack_client_t *jc, void (*cb)(void *), void *arg) {
    (void)jc; (void)cb; (void)arg;
}

void jack_on_info_shutdown(jack_client_t *jc, void (*cb)(int, const char *, void *), void *arg) {
    (void)jc; (void)cb; (void)arg;
}

void jack_set_error_function(void (*func)(const char *)) { (void)func; }
void jack_set_info_function(void (*func)(const char *)) { (void)func; }

jack_port_t *jack_port_register(jack_client_t *jc, const char *port_name, const char *port_type, unsigned long flags, unsigned long buffer_size) {
    (void)buffer_size;
    if (!jc || jc->num_ports >= (LWASAPI_MAX_CHANNELS * 2)) return NULL;
    jack_port_t *p = &jc->ports[jc->num_ports++];
    memset(p, 0, sizeof(*p));
    char cname[sizeof(jc->name)];
    char pname[sizeof(p->name)];
    snprintf(cname, sizeof(cname), "%s", jc->name);
    snprintf(pname, sizeof(pname), "%s", port_name ? port_name : "port");
    memcpy(p->name, pname, sizeof(p->name));
    snprintf(p->full_name, sizeof(p->full_name), "%.63s:%.63s", cname, pname);
    p->flags = flags;
    /* JackPortIsInput == 0x1, JackPortIsOutput == 0x2 */
    p->is_input = ((flags & 0x1) != 0) && ((flags & 0x2) == 0);

    if (port_type && (strstr(port_type, "midi") || strstr(port_type, "MIDI"))) {
        p->is_midi = true;
        p->midi_buffer.magic = 0x4A4D4944U;
        p->midi_buffer.lwasapi_client = jc->lwasapi_client;
    } else {
        p->is_midi = false;
        p->audio_buffer = calloc(jc->period_frames, sizeof(float));
    }
    return p;
}

int jack_port_unregister(jack_client_t *jc, jack_port_t *p) {
    (void)jc; (void)p;
    return 0;
}

void *jack_port_get_buffer(jack_port_t *p, jack_nframes_t nframes) {
    (void)nframes;
    if (!p) return NULL;
    return p->is_midi ? (void *)&p->midi_buffer : (void *)p->audio_buffer;
}

const char *jack_port_name(const jack_port_t *p) {
    return p ? p->full_name : "system:playback_1";
}

const char *jack_port_short_name(const jack_port_t *p) {
    return p ? p->name : "playback_1";
}

int jack_port_flags(const jack_port_t *p) {
    return p ? (int)p->flags : 2;
}

jack_port_t *jack_port_by_name(jack_client_t *jc, const char *port_name) {
    if (!jc || !port_name) return NULL;
    for (int i = 0; i < jc->num_ports; i++) {
        if (strcmp(jc->ports[i].full_name, port_name) == 0 ||
            strcmp(jc->ports[i].name, port_name) == 0) {
            return &jc->ports[i];
        }
    }
    return jc->num_ports > 0 ? &jc->ports[0] : NULL;
}

const char **jack_get_ports(jack_client_t *jc, const char *port_name_pattern, const char *type_name_pattern, unsigned long flags) {
    (void)jc; (void)port_name_pattern; (void)type_name_pattern; (void)flags;
    const char **list = calloc(7, sizeof(char *));
    if (!list) return NULL;
    list[0] = strdup("system:capture_1");
    list[1] = strdup("system:capture_2");
    list[2] = strdup("system:playback_1");
    list[3] = strdup("system:playback_2");
    list[4] = strdup("system:midi_capture_1");
    list[5] = strdup("system:midi_playback_1");
    list[6] = NULL;
    return list;
}

void jack_free(void *ptr) {
    free(ptr);
}

jack_nframes_t jack_get_sample_rate(jack_client_t *jc) {
    return jc ? jc->sample_rate : 48000;
}

jack_nframes_t jack_get_buffer_size(jack_client_t *jc) {
    return jc ? jc->period_frames : 256;
}

jack_nframes_t jack_frame_time(const jack_client_t *jc) {
    return jc ? (jack_nframes_t)jc->frame_counter : 0;
}

jack_nframes_t jack_last_frame_time(const jack_client_t *jc) {
    return jc ? (jack_nframes_t)jc->frame_counter : 0;
}

uint64_t jack_get_time(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000ULL + (uint64_t)(ts.tv_nsec / 1000ULL);
}

void jack_transport_start(jack_client_t *jc) {
    if (jc) jc->transport_rolling = 1;
}

void jack_transport_stop(jack_client_t *jc) {
    if (jc) jc->transport_rolling = 0;
}

int jack_transport_locate(jack_client_t *jc, jack_nframes_t frame) {
    if (jc) jc->frame_counter = frame;
    return 0;
}

int jack_transport_query(const jack_client_t *jc, void *pos) {
    if (pos) memset(pos, 0, 128);
    return (jc && jc->transport_rolling) ? 1 : 0; /* JackTransportRolling == 1 */
}

int jack_activate(jack_client_t *jc) {
    if (!jc) return -1;
    IAudioClient_Start(jc->lwasapi_client);
    jc->active = 1;
    pthread_create(&jc->rt_thread, NULL, jack_wavert_callback_thread, jc);
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_JACK_ACT", jc->name, "RT callback thread started");
    return 0;
}

int jack_deactivate(jack_client_t *jc) {
    if (!jc) return -1;
    if (jc->active) {
        jc->active = 0;
        pthread_join(jc->rt_thread, NULL);
    }
    IAudioClient_Stop(jc->lwasapi_client);
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_JACK_DEACT", jc->name, "RT callback thread stopped");
    return 0;
}

int jack_connect(jack_client_t *jc, const char *src, const char *dst) {
    (void)jc; (void)src; (void)dst;
    return 0;
}

int jack_disconnect(jack_client_t *jc, const char *src, const char *dst) {
    (void)jc; (void)src; (void)dst;
    return 0;
}

/* ============================================================================
 * JACK MIDI API (Used by Ardour, Reaper, Bitwig, Qtractor, Carla, VCV Rack)
 * ============================================================================ */

uint32_t jack_midi_get_event_count(void *port_buffer) {
    jack_midi_buf_t *mb = (jack_midi_buf_t *)port_buffer;
    if (!mb || mb->magic != 0x4A4D4944U) return 0;
    return mb->count;
}

int jack_midi_event_get(jack_midi_event_t *event, void *port_buffer, uint32_t event_index) {
    jack_midi_buf_t *mb = (jack_midi_buf_t *)port_buffer;
    if (!event || !mb || mb->magic != 0x4A4D4944U || event_index >= mb->count) {
        return -1;
    }
    lwasapi_midi_event_t *src = &mb->events[event_index];
    event->time = src->frame_offset;
    event->size = src->size;
    event->buffer = src->data;
    return 0;
}

void jack_midi_clear_buffer(void *port_buffer) {
    jack_midi_buf_t *mb = (jack_midi_buf_t *)port_buffer;
    if (!mb || mb->magic != 0x4A4D4944U) return;
    mb->count = 0;
}

size_t jack_midi_max_event_size(void *port_buffer) {
    (void)port_buffer;
    return 3;
}

jack_midi_data_t *jack_midi_event_reserve(void *port_buffer, jack_nframes_t time, size_t data_size) {
    jack_midi_buf_t *mb = (jack_midi_buf_t *)port_buffer;
    if (!mb || mb->magic != 0x4A4D4944U || mb->count >= JACK_MAX_PORT_MIDI_EVENTS || data_size > 3) {
        return NULL;
    }
    lwasapi_midi_event_t *ev = &mb->events[mb->count++];
    memset(ev, 0, sizeof(*ev));
    ev->frame_offset = time;
    ev->size = (uint8_t)data_size;
    return ev->data;
}

int jack_midi_event_write(void *port_buffer, jack_nframes_t time, const jack_midi_data_t *data, size_t data_size) {
    jack_midi_buf_t *mb = (jack_midi_buf_t *)port_buffer;
    if (!mb || mb->magic != 0x4A4D4944U || !data || data_size == 0 || data_size > 3) {
        return -1;
    }
    jack_midi_data_t *dst = jack_midi_event_reserve(port_buffer, time, data_size);
    if (!dst) return -1;
    memcpy(dst, data, data_size);

    if (mb->lwasapi_client) {
        IAudioClient_WriteMidiEvent(mb->lwasapi_client, &mb->events[mb->count - 1]);
    }
    return 0;
}
