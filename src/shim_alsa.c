#define _GNU_SOURCE
#include "../include/lwasapi.h"
#include "../include/lwasapi_debug.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <dlfcn.h>
#include <poll.h>

/* ============================================================================
 * Hollow ABI Shim for ALSA Userspace (libasound.so.2 -> LWASAPI Ring Buffer)
 * - Routes PCM audio (snd_pcm_*) directly into LWASAPI shared memory rings.
 * - Forwards ALSA Sequencer (snd_seq_*), MIDI Event (snd_midi_event_*), and
 *   RawMIDI (snd_rawmidi_*) to /usr/lib/libasound.so.2.0.0 so DAWs and
 *   Firefox WebMIDI have full hardware MIDI access (FLkey 37 + LWASAPI Hub).
 * ============================================================================ */

static void *get_real_asound(void) {
    static void *dl = NULL;
    if (!dl) {
        static const char *const cands[] = {
            "/usr/lib/libasound.so.2.0.0",
            "/usr/lib/libasound.so.2",
            "/usr/lib64/libasound.so.2.0.0",
            "/usr/lib64/libasound.so.2",
            "/usr/lib/x86_64-linux-gnu/libasound.so.2.0.0",
            "/usr/lib/x86_64-linux-gnu/libasound.so.2",
            "/usr/lib/aarch64-linux-gnu/libasound.so.2.0.0",
            "/usr/lib/aarch64-linux-gnu/libasound.so.2",
            "/lib/x86_64-linux-gnu/libasound.so.2",
            "/lib/aarch64-linux-gnu/libasound.so.2",
            "/lib64/libasound.so.2",
            "/lib/libasound.so.2",
            NULL
        };
        for (int i = 0; cands[i]; i++) {
            dl = dlopen(cands[i], RTLD_LAZY | RTLD_LOCAL);
            if (dl) break;
        }
    }
    return dl;
}

#define RESOLVE_ASOUND(ret_type, name, args_decl, args_call, fallback_ret) \
    ret_type name args_decl { \
        static ret_type (*fn) args_decl = NULL; \
        if (!fn) { \
            void *dl = get_real_asound(); \
            if (dl) fn = (ret_type (*) args_decl)dlsym(dl, #name); \
        } \
        if (fn) return fn args_call; \
        return fallback_ret; \
    }

#define RESOLVE_ASOUND_VOID(name, args_decl, args_call) \
    void name args_decl { \
        static void (*fn) args_decl = NULL; \
        if (!fn) { \
            void *dl = get_real_asound(); \
            if (dl) fn = (void (*) args_decl)dlsym(dl, #name); \
        } \
        if (fn) fn args_call; \
    }

typedef struct _snd_pcm {
    IAudioClient           *client;
    int                     event_fd;
    uint32_t                sample_rate;
    uint32_t                channels;
    lwasapi_sample_format_t fmt;
    uint32_t                buffer_frames;
    uint32_t                period_frames;
    bool                    started;
    bool                    paused;
    bool                    is_capture;
} snd_pcm_t;

typedef struct _snd_pcm_hw_params {
    uint32_t rate;
    uint32_t channels;
    int      alsa_format;
    uint32_t buffer_frames;
    uint32_t period_frames;
} snd_pcm_hw_params_t;

typedef struct _snd_pcm_sw_params {
    uint32_t start_threshold;
    uint32_t avail_min;
} snd_pcm_sw_params_t;

typedef long snd_pcm_sframes_t;
typedef unsigned long snd_pcm_uframes_t;

#define SND_PCM_FORMAT_S16_LE   2
#define SND_PCM_FORMAT_S32_LE   10
#define SND_PCM_FORMAT_FLOAT_LE 14

#define SND_PCM_STATE_OPEN      0
#define SND_PCM_STATE_SETUP     1
#define SND_PCM_STATE_PREPARED  2
#define SND_PCM_STATE_RUNNING   3
#define SND_PCM_STATE_XRUN      4
#define SND_PCM_STATE_DRAINING  5
#define SND_PCM_STATE_PAUSED    6

/* Dummy snd_config symbol & stubs for Firefox cubeb_alsa */
void *snd_config = (void *)0x1;

int snd_config_update_free_global(void) { return 0; }
int snd_config_copy(void **dst, const void *src) { (void)src; if (dst) *dst = (void *)0x1; return 0; }
int snd_config_delete(void *top) { (void)top; return 0; }
int snd_config_search(const void *config, const char *key, void **result) { (void)config; (void)key; (void)result; return -2; }
int snd_config_search_definition(const void *config, const char *base, const char *key, void **result) { (void)config; (void)base; (void)key; (void)result; return -2; }
int snd_config_get_id(const void *config, const char **value) { (void)config; if (value) *value = "default"; return 0; }
int snd_config_get_string(const void *config, const char **value) { (void)config; if (value) *value = "default"; return 0; }
int snd_config_imake_integer(void **config, const char *key, long value) { (void)key; (void)value; if (config) *config = (void *)0x1; return 0; }
int snd_config_add(void *parent, void *child) { (void)parent; (void)child; return 0; }
int snd_lib_error_set_handler(void *handler) { (void)handler; return 0; }

/* ============================================================================
 * Native ALSA Sequencer, MIDI Event & RawMIDI Pass-Through for DAWs & WebMIDI
 * ============================================================================ */

RESOLVE_ASOUND(int, snd_midi_event_new, (size_t bufsize, void **rdev), (bufsize, rdev), -19)
RESOLVE_ASOUND_VOID(snd_midi_event_free, (void *dev), (dev))
RESOLVE_ASOUND_VOID(snd_midi_event_no_status, (void *dev, int on), (dev, on))
RESOLVE_ASOUND(int, snd_midi_event_resize_buffer, (void *dev, size_t bufsize), (dev, bufsize), 0)
RESOLVE_ASOUND(long, snd_midi_event_encode, (void *dev, const unsigned char *buf, long count, void *ev), (dev, buf, count, ev), count)
RESOLVE_ASOUND(long, snd_midi_event_decode, (void *dev, unsigned char *buf, long count, const void *ev), (dev, buf, count, ev), 0)

RESOLVE_ASOUND(int, snd_seq_open, (void **handle, const char *name, int streams, int mode), (handle, name, streams, mode), -19)
RESOLVE_ASOUND(int, snd_seq_close, (void *handle), (handle), 0)
RESOLVE_ASOUND(int, snd_seq_client_id, (void *handle), (handle), 0)
RESOLVE_ASOUND(int, snd_seq_set_client_name, (void *handle, const char *name), (handle, name), 0)
RESOLVE_ASOUND(int, snd_seq_alloc_named_queue, (void *seq, const char *name), (seq, name), 0)
RESOLVE_ASOUND(int, snd_seq_free_queue, (void *handle, int q), (handle, q), 0)
RESOLVE_ASOUND(int, snd_seq_control_queue, (void *seq, int q, int type, int value, void *ev), (seq, q, type, value, ev), 0)
RESOLVE_ASOUND(int, snd_seq_create_port, (void *handle, void *info), (handle, info), 0)
RESOLVE_ASOUND(int, snd_seq_create_simple_port, (void *seq, const char *name, unsigned int caps, unsigned int type), (seq, name, caps, type), 0)
RESOLVE_ASOUND(int, snd_seq_delete_port, (void *handle, int port), (handle, port), 0)
RESOLVE_ASOUND(int, snd_seq_drain_output, (void *handle), (handle), 0)
RESOLVE_ASOUND(int, snd_seq_event_input, (void *handle, void **ev), (handle, ev), -11)
RESOLVE_ASOUND(int, snd_seq_event_input_pending, (void *seq, int fetch_sequencer), (seq, fetch_sequencer), 0)
RESOLVE_ASOUND(int, snd_seq_event_output, (void *handle, void *ev), (handle, ev), 0)
RESOLVE_ASOUND(int, snd_seq_event_output_direct, (void *handle, void *ev), (handle, ev), 0)
RESOLVE_ASOUND(int, snd_seq_get_any_client_info, (void *handle, int client, void *info), (handle, client, info), -2)
RESOLVE_ASOUND(int, snd_seq_get_any_port_info, (void *handle, int client, int port, void *info), (handle, client, port, info), -2)
RESOLVE_ASOUND(int, snd_seq_poll_descriptors_count, (void *handle, short events), (handle, events), 0)
RESOLVE_ASOUND(int, snd_seq_poll_descriptors, (void *handle, struct pollfd *pfds, unsigned int space, short events), (handle, pfds, space, events), 0)
RESOLVE_ASOUND(int, snd_seq_query_next_client, (void *handle, void *info), (handle, info), -2)
RESOLVE_ASOUND(int, snd_seq_query_next_port, (void *handle, void *info), (handle, info), -2)
RESOLVE_ASOUND(int, snd_seq_set_queue_tempo, (void *handle, int q, void *tempo), (handle, q, tempo), 0)
RESOLVE_ASOUND(int, snd_seq_subscribe_port, (void *handle, void *sub), (handle, sub), 0)
RESOLVE_ASOUND(int, snd_seq_unsubscribe_port, (void *handle, void *sub), (handle, sub), 0)
RESOLVE_ASOUND(int, snd_seq_connect_from, (void *seq, int my_port, int src_client, int src_port), (seq, my_port, src_client, src_port), 0)
RESOLVE_ASOUND(int, snd_seq_connect_to, (void *seq, int my_port, int dest_client, int dest_port), (seq, my_port, dest_client, dest_port), 0)

RESOLVE_ASOUND(size_t, snd_seq_client_info_sizeof, (void), (), 256)
RESOLVE_ASOUND(int, snd_seq_client_info_malloc, (void **ptr), (ptr), 0)
RESOLVE_ASOUND_VOID(snd_seq_client_info_free, (void *ptr), (ptr))
RESOLVE_ASOUND(int, snd_seq_client_info_get_client, (const void *info), (info), 0)
RESOLVE_ASOUND(const char *, snd_seq_client_info_get_name, (void *info), (info), "LWASAPI-Seq")
RESOLVE_ASOUND_VOID(snd_seq_client_info_set_client, (void *info, int client), (info, client))

RESOLVE_ASOUND(size_t, snd_seq_port_info_sizeof, (void), (), 256)
RESOLVE_ASOUND(int, snd_seq_port_info_malloc, (void **ptr), (ptr), 0)
RESOLVE_ASOUND_VOID(snd_seq_port_info_free, (void *ptr), (ptr))
RESOLVE_ASOUND(unsigned int, snd_seq_port_info_get_capability, (const void *info), (info), 0)
RESOLVE_ASOUND(int, snd_seq_port_info_get_client, (const void *info), (info), 0)
RESOLVE_ASOUND(const char *, snd_seq_port_info_get_name, (const void *info), (info), "Port")
RESOLVE_ASOUND(int, snd_seq_port_info_get_port, (const void *info), (info), 0)
RESOLVE_ASOUND(unsigned int, snd_seq_port_info_get_type, (const void *info), (info), 0)
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_capability, (void *info, unsigned int capability), (info, capability))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_client, (void *info, int client), (info, client))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_midi_channels, (void *info, int channels), (info, channels))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_name, (void *info, const char *name), (info, name))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_port, (void *info, int port), (info, port))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_timestamping, (void *info, int enable), (info, enable))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_timestamp_queue, (void *info, int queue), (info, queue))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_timestamp_real, (void *info, int enable), (info, enable))
RESOLVE_ASOUND_VOID(snd_seq_port_info_set_type, (void *info, unsigned int type), (info, type))

RESOLVE_ASOUND(size_t, snd_seq_port_subscribe_sizeof, (void), (), 256)
RESOLVE_ASOUND(int, snd_seq_port_subscribe_malloc, (void **ptr), (ptr), 0)
RESOLVE_ASOUND_VOID(snd_seq_port_subscribe_free, (void *ptr), (ptr))
RESOLVE_ASOUND(const void *, snd_seq_port_subscribe_get_dest, (const void *info), (info), info)
RESOLVE_ASOUND(const void *, snd_seq_port_subscribe_get_sender, (const void *info), (info), info)
RESOLVE_ASOUND_VOID(snd_seq_port_subscribe_set_dest, (void *info, const void *addr), (info, addr))
RESOLVE_ASOUND_VOID(snd_seq_port_subscribe_set_sender, (void *info, const void *addr), (info, addr))
RESOLVE_ASOUND_VOID(snd_seq_port_subscribe_set_time_real, (void *info, int val), (info, val))
RESOLVE_ASOUND_VOID(snd_seq_port_subscribe_set_time_update, (void *info, int val), (info, val))

RESOLVE_ASOUND(size_t, snd_seq_queue_tempo_sizeof, (void), (), 256)
RESOLVE_ASOUND(int, snd_seq_queue_tempo_malloc, (void **ptr), (ptr), 0)
RESOLVE_ASOUND_VOID(snd_seq_queue_tempo_free, (void *ptr), (ptr))
RESOLVE_ASOUND_VOID(snd_seq_queue_tempo_set_ppq, (void *info, int ppq), (info, ppq))
RESOLVE_ASOUND_VOID(snd_seq_queue_tempo_set_tempo, (void *info, unsigned int tempo), (info, tempo))

/* RawMIDI Pass-Through for DAWs */
RESOLVE_ASOUND(int, snd_rawmidi_open, (void **in_rmidi, void **out_rmidi, const char *name, int mode), (in_rmidi, out_rmidi, name, mode), -19)
RESOLVE_ASOUND(int, snd_rawmidi_close, (void *rmidi), (rmidi), 0)
RESOLVE_ASOUND(ssize_t, snd_rawmidi_read, (void *rmidi, void *buffer, size_t size), (rmidi, buffer, size), -11)
RESOLVE_ASOUND(ssize_t, snd_rawmidi_write, (void *rmidi, const void *buffer, size_t size), (rmidi, buffer, size), (ssize_t)size)
RESOLVE_ASOUND(int, snd_rawmidi_drain, (void *rmidi), (rmidi), 0)
RESOLVE_ASOUND(int, snd_rawmidi_drop, (void *rmidi), (rmidi), 0)
RESOLVE_ASOUND(int, snd_rawmidi_nonblock, (void *rmidi, int nonblock), (rmidi, nonblock), 0)
RESOLVE_ASOUND(int, snd_rawmidi_poll_descriptors_count, (void *rmidi), (rmidi), 0)
RESOLVE_ASOUND(int, snd_rawmidi_poll_descriptors, (void *rmidi, struct pollfd *pfds, unsigned int space), (rmidi, pfds, space), 0)
RESOLVE_ASOUND(int, snd_rawmidi_poll_descriptors_revents, (void *rmidi, struct pollfd *pfds, unsigned int nfds, unsigned short *revent), (rmidi, pfds, nfds, revent), 0)

/* Device Enumeration, Card, Control & Mixer Pass-Through for Wine/Proton, DAWs & Browsers */
RESOLVE_ASOUND(int, snd_device_name_hint, (int card, const char *iface, void ***hints), (card, iface, hints), -2)
RESOLVE_ASOUND(char *, snd_device_name_get_hint, (const void *hint, const char *id), (hint, id), NULL)
RESOLVE_ASOUND(int, snd_device_name_free_hint, (void **hints), (hints), 0)

RESOLVE_ASOUND(int, snd_card_next, (int *rcard), (rcard), 0)
RESOLVE_ASOUND(int, snd_card_get_index, (const char *string), (string), 0)
RESOLVE_ASOUND(int, snd_card_get_name, (int card, char **name), (card, name), 0)
RESOLVE_ASOUND(int, snd_card_get_longname, (int card, char **name), (card, name), 0)

RESOLVE_ASOUND(int, snd_ctl_open, (void **ctl, const char *name, int mode), (ctl, name, mode), -2)
RESOLVE_ASOUND(int, snd_ctl_close, (void *ctl), (ctl), 0)
RESOLVE_ASOUND(int, snd_ctl_card_info, (void *ctl, void *info), (ctl, info), 0)
RESOLVE_ASOUND(size_t, snd_ctl_card_info_sizeof, (void), (), 512)
RESOLVE_ASOUND(int, snd_ctl_card_info_malloc, (void **ptr), (ptr), 0)
RESOLVE_ASOUND_VOID(snd_ctl_card_info_free, (void *obj), (obj))
RESOLVE_ASOUND(const char *, snd_ctl_card_info_get_name, (const void *obj), (obj), "LWASAPI")
RESOLVE_ASOUND(const char *, snd_ctl_card_info_get_id, (const void *obj), (obj), "lwasapi")
RESOLVE_ASOUND(int, snd_ctl_pcm_next_device, (void *ctl, int *device), (ctl, device), 0)
RESOLVE_ASOUND(int, snd_ctl_pcm_info, (void *ctl, void *info), (ctl, info), 0)

RESOLVE_ASOUND(int, snd_mixer_open, (void **mixer, int mode), (mixer, mode), 0)
RESOLVE_ASOUND(int, snd_mixer_close, (void *mixer), (mixer), 0)
RESOLVE_ASOUND(int, snd_mixer_attach, (void *mixer, const char *name), (mixer, name), 0)
RESOLVE_ASOUND(int, snd_mixer_load, (void *mixer), (mixer), 0)
RESOLVE_ASOUND(int, snd_mixer_selem_register, (void *mixer, void *options, void **classp), (mixer, options, classp), 0)
RESOLVE_ASOUND(void *, snd_mixer_first_elem, (void *mixer), (mixer), NULL)
RESOLVE_ASOUND(void *, snd_mixer_elem_next, (void *elem), (elem), NULL)
RESOLVE_ASOUND(void *, snd_mixer_find_selem, (void *mixer, const void *id), (mixer, id), NULL)
RESOLVE_ASOUND(const char *, snd_mixer_selem_get_name, (void *elem), (elem), "Master")
RESOLVE_ASOUND(int, snd_mixer_selem_has_playback_volume, (void *elem), (elem), 1)
RESOLVE_ASOUND(int, snd_mixer_selem_has_capture_volume, (void *elem), (elem), 1)
RESOLVE_ASOUND(int, snd_mixer_selem_get_playback_volume_range, (void *elem, long *min, long *max), (elem, min, max), 0)
RESOLVE_ASOUND(int, snd_mixer_selem_get_playback_volume, (void *elem, int channel, long *value), (elem, channel, value), 0)
RESOLVE_ASOUND(int, snd_mixer_selem_set_playback_volume_all, (void *elem, long value), (elem, value), 0)

/* ============================================================================
 * ALSA PCM Shim -> LWASAPI Shared Memory Engine
 * ============================================================================ */

static lwasapi_sample_format_t map_alsa_fmt(int alsa_fmt) {
    if (alsa_fmt == SND_PCM_FORMAT_S16_LE) return LWASAPI_FMT_S16_LE;
    if (alsa_fmt == SND_PCM_FORMAT_S32_LE) return LWASAPI_FMT_S32_LE;
    return LWASAPI_FMT_FLOAT32;
}

static int ensure_lwasapi_initialized(snd_pcm_t *pcm) {
    if (!pcm) return -22;
    WAVEFORMATEX wfx;
    memset(&wfx, 0, sizeof(wfx));
    wfx.wFormatTag = (uint16_t)pcm->fmt;
    wfx.nChannels = (uint16_t)pcm->channels;
    wfx.nSamplesPerSec = pcm->sample_rate;
    wfx.wBitsPerSample = (pcm->fmt == LWASAPI_FMT_S16_LE) ? 16 : 32;
    wfx.nBlockAlign = wfx.nChannels * (wfx.wBitsPerSample / 8);
    wfx.nAvgBytesPerSec = wfx.nSamplesPerSec * wfx.nBlockAlign;

    uint32_t flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if (pcm->is_capture) {
        flags |= AUDCLNT_STREAMFLAGS_CAPTURE;
    }

    HRESULT hr = IAudioClient_Initialize(
        pcm->client,
        AUDCLNT_SHAREMODE_SHARED,
        flags,
        pcm->buffer_frames,
        pcm->period_frames,
        &wfx
    );
    if (hr != S_OK && hr != AUDCLNT_E_ALREADY_INITIALIZED) {
        return -5;
    }
    IAudioClient_SetEventHandle(pcm->client, &pcm->event_fd);
    IAudioClient_GetBufferSize(pcm->client, &pcm->buffer_frames);
    IAudioClient_GetDevicePeriod(pcm->client, &pcm->period_frames, NULL);
    IAudioClient_Start(pcm->client);
    pcm->started = true;
    pcm->paused = false;
    return 0;
}

int snd_pcm_open(snd_pcm_t **pcmp, const char *name, int stream, int mode) {
    (void)mode;
    if (!pcmp) return -22;

    snd_pcm_t *pcm = calloc(1, sizeof(snd_pcm_t));
    if (!pcm) return -12;

    pcm->is_capture = (stream == 1); /* SND_PCM_STREAM_CAPTURE == 1 */

    char cname[64];
    snprintf(cname, sizeof(cname), "ALSA-%s(%s)",
             pcm->is_capture ? "Cap" : "Play",
             name ? name : "default");
    if (LWASAPI_ActivateAudioClient(cname, &pcm->client) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "SHIM_ALSA_ERR", cname, "LWASAPI_ActivateAudioClient failed");
        free(pcm);
        return -19;
    }

    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_ALSA_OPEN", cname,
                             pcm->is_capture ? "stream=CAPTURE" : "stream=PLAYBACK");

    pcm->event_fd = -1;
    pcm->sample_rate = 48000;
    pcm->channels = 2;
    pcm->fmt = LWASAPI_FMT_FLOAT32;
    pcm->buffer_frames = 2048;
    pcm->period_frames = 256;
    *pcmp = pcm;
    return 0;
}

int snd_pcm_open_lconf(snd_pcm_t **pcmp, const char *name, int stream, int mode, void *lconf) {
    (void)lconf;
    return snd_pcm_open(pcmp, name, stream, mode);
}

int snd_pcm_close(snd_pcm_t *pcm) {
    if (!pcm) return -22;
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_ALSA_CLOSE", pcm->is_capture ? "ALSA-Cap" : "ALSA-Play",
                             "snd_pcm_close");
    if (pcm->client) IAudioClient_Release(pcm->client);
    free(pcm);
    return 0;
}

int snd_pcm_nonblock(snd_pcm_t *pcm, int nonblock) {
    (void)pcm; (void)nonblock;
    return 0;
}

int snd_pcm_set_params(snd_pcm_t *pcm, int format, int access, unsigned int channels,
                       unsigned int rate, int soft_resample, unsigned int latency) {
    (void)access; (void)soft_resample;
    if (!pcm) return -22;
    pcm->fmt = map_alsa_fmt(format);
    pcm->channels = channels ? channels : 2;
    pcm->sample_rate = rate ? rate : 48000;
    pcm->buffer_frames = (latency > 0) ? (uint32_t)((uint64_t)pcm->sample_rate * latency / 1000000ULL) : 2048;
    if (pcm->buffer_frames < 512) pcm->buffer_frames = 1024;
    pcm->period_frames = 256;
    return ensure_lwasapi_initialized(pcm);
}

int snd_pcm_get_params(snd_pcm_t *pcm, snd_pcm_uframes_t *buffer_size, snd_pcm_uframes_t *period_size) {
    if (!pcm) return -22;
    if (buffer_size) *buffer_size = pcm->buffer_frames;
    if (period_size) *period_size = pcm->period_frames;
    return 0;
}

size_t snd_pcm_hw_params_sizeof(void) { return sizeof(snd_pcm_hw_params_t); }
int snd_pcm_hw_params_malloc(snd_pcm_hw_params_t **ptr) {
    *ptr = calloc(1, sizeof(snd_pcm_hw_params_t));
    return *ptr ? 0 : -12;
}
void snd_pcm_hw_params_free(snd_pcm_hw_params_t *obj) { free(obj); }
int snd_pcm_hw_params_any(snd_pcm_t *pcm, snd_pcm_hw_params_t *params) {
    (void)pcm;
    if (!params) return -22;
    params->rate = 48000;
    params->channels = 2;
    params->alsa_format = SND_PCM_FORMAT_FLOAT_LE;
    params->buffer_frames = 2048;
    params->period_frames = 256;
    return 0;
}
int snd_pcm_hw_params_set_access(snd_pcm_t *pcm, snd_pcm_hw_params_t *params, int access) {
    (void)pcm; (void)params; (void)access;
    return 0;
}
int snd_pcm_hw_params_set_format(snd_pcm_t *pcm, snd_pcm_hw_params_t *params, int val) {
    (void)pcm;
    if (params) params->alsa_format = val;
    return 0;
}
int snd_pcm_hw_params_set_channels(snd_pcm_t *pcm, snd_pcm_hw_params_t *params, unsigned int val) {
    (void)pcm;
    if (params) params->channels = val;
    return 0;
}
int snd_pcm_hw_params_get_channels(const snd_pcm_hw_params_t *params, unsigned int *val) {
    if (val) *val = (params && params->channels) ? params->channels : 2;
    return 0;
}
int snd_pcm_hw_params_get_channels_max(const snd_pcm_hw_params_t *params, unsigned int *val) {
    (void)params;
    if (val) *val = 2;
    return 0;
}
int snd_pcm_hw_params_get_rate(const snd_pcm_hw_params_t *params, unsigned int *val, int *dir) {
    if (val) *val = (params && params->rate) ? params->rate : 48000;
    if (dir) *dir = 0;
    return 0;
}
int snd_pcm_hw_params_get_period_size(const snd_pcm_hw_params_t *params, snd_pcm_uframes_t *val, int *dir) {
    if (val) *val = (params && params->period_frames) ? params->period_frames : 256;
    if (dir) *dir = 0;
    return 0;
}
int snd_pcm_hw_params_get_buffer_size(const snd_pcm_hw_params_t *params, snd_pcm_uframes_t *val) {
    if (val) *val = (params && params->buffer_frames) ? params->buffer_frames : 2048;
    return 0;
}
int snd_pcm_hw_params_set_rate_near(snd_pcm_t *pcm, snd_pcm_hw_params_t *params, unsigned int *val, int *dir) {
    (void)pcm; (void)dir;
    if (params && val) params->rate = *val;
    return 0;
}
int snd_pcm_hw_params_set_period_size_near(snd_pcm_t *pcm, snd_pcm_hw_params_t *params, snd_pcm_uframes_t *val, int *dir) {
    (void)pcm; (void)dir;
    if (params && val) params->period_frames = (uint32_t)*val;
    return 0;
}
int snd_pcm_hw_params_set_buffer_size_near(snd_pcm_t *pcm, snd_pcm_hw_params_t *params, snd_pcm_uframes_t *val) {
    (void)pcm;
    if (params && val) params->buffer_frames = (uint32_t)*val;
    return 0;
}
int snd_pcm_hw_params(snd_pcm_t *pcm, snd_pcm_hw_params_t *params) {
    if (!pcm || !params) return -22;
    pcm->sample_rate = params->rate ? params->rate : 48000;
    pcm->channels = params->channels ? params->channels : 2;
    pcm->fmt = map_alsa_fmt(params->alsa_format);
    pcm->buffer_frames = params->buffer_frames ? params->buffer_frames : 2048;
    pcm->period_frames = params->period_frames ? params->period_frames : 256;
    char detail[128];
    snprintf(detail, sizeof(detail), "%s rate=%u ch=%u buf=%u per=%u",
             pcm->is_capture ? "CAPTURE" : "PLAYBACK",
             pcm->sample_rate, pcm->channels, pcm->buffer_frames, pcm->period_frames);
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "SHIM_ALSA_HW", pcm->is_capture ? "ALSA-Cap" : "ALSA-Play",
                             detail);
    return ensure_lwasapi_initialized(pcm);
}

size_t snd_pcm_sw_params_sizeof(void) { return sizeof(snd_pcm_sw_params_t); }
int snd_pcm_sw_params_malloc(snd_pcm_sw_params_t **ptr) {
    *ptr = calloc(1, sizeof(snd_pcm_sw_params_t));
    return *ptr ? 0 : -12;
}
void snd_pcm_sw_params_free(snd_pcm_sw_params_t *obj) { free(obj); }
int snd_pcm_sw_params_current(snd_pcm_t *pcm, snd_pcm_sw_params_t *params) {
    (void)pcm; (void)params;
    return 0;
}
int snd_pcm_sw_params(snd_pcm_t *pcm, snd_pcm_sw_params_t *params) {
    (void)pcm; (void)params;
    return 0;
}

int snd_pcm_prepare(snd_pcm_t *pcm) {
    if (!pcm) return -22;
    if (!pcm->started) return ensure_lwasapi_initialized(pcm);
    return 0;
}

int snd_pcm_start(snd_pcm_t *pcm) {
    if (!pcm) return -22;
    if (!pcm->started && ensure_lwasapi_initialized(pcm) < 0) return -5;
    IAudioClient_Start(pcm->client);
    pcm->paused = false;
    return 0;
}

int snd_pcm_pause(snd_pcm_t *pcm, int enable) {
    if (!pcm || !pcm->started) return -22;
    if (enable) {
        IAudioClient_Stop(pcm->client);
        pcm->paused = true;
    } else {
        IAudioClient_Start(pcm->client);
        pcm->paused = false;
    }
    return 0;
}

int snd_pcm_state(snd_pcm_t *pcm) {
    if (!pcm || !pcm->started) return SND_PCM_STATE_PREPARED;
    if (pcm->paused) return SND_PCM_STATE_PAUSED;
    return SND_PCM_STATE_RUNNING;
}

int snd_pcm_delay(snd_pcm_t *pcm, snd_pcm_sframes_t *delayp) {
    if (!pcm || !pcm->started) return -22;
    uint32_t pad = 0;
    IAudioClient_GetCurrentPadding(pcm->client, &pad);
    if (delayp) *delayp = (snd_pcm_sframes_t)pad;
    return 0;
}

ssize_t snd_pcm_frames_to_bytes(snd_pcm_t *pcm, snd_pcm_sframes_t frames) {
    if (!pcm) return 0;
    ssize_t bps = (pcm->fmt == LWASAPI_FMT_S16_LE) ? 2 : 4;
    return (ssize_t)frames * (ssize_t)pcm->channels * bps;
}

int snd_pcm_poll_descriptors_count(snd_pcm_t *pcm) {
    (void)pcm;
    return 1;
}

int snd_pcm_poll_descriptors(snd_pcm_t *pcm, struct pollfd *pfds, unsigned int space) {
    if (!pcm || !pfds || space < 1) return -22;
    if (!pcm->started) ensure_lwasapi_initialized(pcm);
    pfds[0].fd = pcm->event_fd;
    pfds[0].events = POLLIN;
    pfds[0].revents = 0;
    return 1;
}

snd_pcm_sframes_t snd_pcm_avail_update(snd_pcm_t *pcm);

int snd_pcm_poll_descriptors_revents(snd_pcm_t *pcm, struct pollfd *pfds, unsigned int nfds, unsigned short *revents) {
    if (!pcm || !pfds || nfds < 1 || !revents) return -22;
    if (pfds[0].revents & POLLIN) {
        uint64_t val = 0;
        if (read(pcm->event_fd, &val, sizeof(val)) < 0) {}
    }
    snd_pcm_sframes_t avail = snd_pcm_avail_update(pcm);
    if (pcm->is_capture) {
        *revents = (avail >= (snd_pcm_sframes_t)pcm->period_frames) ? POLLIN : 0;
    } else {
        *revents = (avail >= (snd_pcm_sframes_t)pcm->period_frames) ? POLLOUT : 0;
    }
    return 0;
}

snd_pcm_sframes_t snd_pcm_writei(snd_pcm_t *pcm, const void *buffer, snd_pcm_uframes_t size) {
    if (!pcm || !buffer) return -22;
    if (!pcm->started && ensure_lwasapi_initialized(pcm) < 0) return -5;
    if (IAudioRenderClient_WriteFrames(pcm->client, buffer, (uint32_t)size, pcm->fmt) != S_OK) {
        return -5;
    }
    return (snd_pcm_sframes_t)size;
}

snd_pcm_sframes_t snd_pcm_readi(snd_pcm_t *pcm, void *buffer, snd_pcm_uframes_t size) {
    if (!pcm || !buffer) return -22;
    if (!pcm->started && ensure_lwasapi_initialized(pcm) < 0) return -5;
    uint32_t pkt = 0;
    IAudioCaptureClient_GetNextPacketSize(pcm->client, &pkt);
    if (pkt < size) {
        IAudioClient_WaitForPeriod(pcm->client, 15);
    }
    if (IAudioCaptureClient_ReadFrames(pcm->client, buffer, (uint32_t)size, pcm->fmt) != S_OK) {
        return -5;
    }
    return (snd_pcm_sframes_t)size;
}

snd_pcm_sframes_t snd_pcm_avail_update(snd_pcm_t *pcm) {
    if (!pcm || !pcm->started) return 0;
    if (pcm->is_capture) {
        uint32_t pkt = 0;
        IAudioCaptureClient_GetNextPacketSize(pcm->client, &pkt);
        return (snd_pcm_sframes_t)pkt;
    }
    uint32_t buf = 0, pad = 0;
    IAudioClient_GetBufferSize(pcm->client, &buf);
    IAudioClient_GetCurrentPadding(pcm->client, &pad);
    return (snd_pcm_sframes_t)((buf > pad) ? (buf - pad) : 0);
}

snd_pcm_sframes_t snd_pcm_avail(snd_pcm_t *pcm) {
    return snd_pcm_avail_update(pcm);
}

snd_pcm_sframes_t snd_pcm_forward(snd_pcm_t *pcm, snd_pcm_uframes_t frames) {
    (void)pcm;
    return (snd_pcm_sframes_t)frames;
}

int snd_pcm_wait(snd_pcm_t *pcm, int timeout) {
    if (!pcm || !pcm->started) return -22;
    IAudioClient_WaitForPeriod(pcm->client, timeout);
    return 1;
}

int snd_pcm_drain(snd_pcm_t *pcm) {
    if (!pcm || !pcm->started) return 0;
    if (pcm->is_capture) {
        IAudioClient_Stop(pcm->client);
        return 0;
    }
    for (int i = 0; i < 50; i++) {
        uint32_t pad = 0;
        IAudioClient_GetCurrentPadding(pcm->client, &pad);
        if (pad == 0) break;
        IAudioClient_WaitForPeriod(pcm->client, 10);
    }
    IAudioClient_Stop(pcm->client);
    return 0;
}

int snd_pcm_drop(snd_pcm_t *pcm) {
    if (!pcm || !pcm->started) return 0;
    IAudioClient_Stop(pcm->client);
    IAudioClient_Reset(pcm->client);
    return 0;
}

int snd_pcm_recover(snd_pcm_t *pcm, int err, int silent) {
    (void)err; (void)silent;
    return snd_pcm_prepare(pcm);
}

const char *snd_strerror(int errnum) {
    return strerror(errnum < 0 ? -errnum : errnum);
}
