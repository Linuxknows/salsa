#define _GNU_SOURCE
#include "../include/lwasapi.h"
#include "../include/lwasapi_debug.h"
#include "../include/lwasapi_paths.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#include <alsa/asoundlib.h>
#include <alsa/pcm_external.h>

/* ============================================================================
 * Native ALSA External PCM I/O Plugin (libasound_module_pcm_lwasapi.so)
 * Allows any ALSA application opened normally (zero lwasapi-run / LD_PRELOAD)
 * to route directly into lwasapi-audiodg via ~/.asoundrc, with automatic
 * fallback to sysdefault if lwasapi-audiodg is stopped.
 * ============================================================================ */

typedef struct {
    snd_pcm_ioplug_t        io;
    IAudioClient           *client;
    uint64_t                hw_Frames;
    lwasapi_sample_format_t fmt;
    int                     event_fd;
} lwasapi_alsa_ioplug_t;

static bool is_system_audio_daemon(void) {
    char comm[32] = {0};
    int fd = open("/proc/self/comm", O_RDONLY | O_CLOEXEC);
    if (fd >= 0) {
        ssize_t n = read(fd, comm, sizeof(comm) - 1);
        close(fd);
        if (n > 0) {
            comm[strcspn(comm, "\r\n")] = '\0';
        }
    }
    if (strcmp(comm, "pipewire") == 0 ||
        strcmp(comm, "wireplumber") == 0 ||
        strcmp(comm, "pipewire-pulse") == 0 ||
        strcmp(comm, "pulseaudio") == 0 ||
        strcmp(comm, "lwasapi-audiodg") == 0 ||
        strcmp(comm, "plasmashell") == 0) {
        return true;
    }
    return false;
}

static int lwasapi_io_start(snd_pcm_ioplug_t *io) {
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (!p || !p->client) return -EIO;
    IAudioClient_Start(p->client);
    return 0;
}

static int lwasapi_io_stop(snd_pcm_ioplug_t *io) {
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (!p || !p->client) return -EIO;
    IAudioClient_Stop(p->client);
    return 0;
}

static snd_pcm_sframes_t lwasapi_io_pointer(snd_pcm_ioplug_t *io) {
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (!p || !p->client || io->buffer_size == 0) return 0;

    if (io->stream == SND_PCM_STREAM_CAPTURE) {
        uint32_t pkt = 0;
        IAudioCaptureClient_GetNextPacketSize(p->client, &pkt);
        if (pkt > io->buffer_size) pkt = (uint32_t)io->buffer_size;
        uint64_t captured = p->hw_Frames + pkt;
        return (snd_pcm_sframes_t)(captured % io->buffer_size);
    }

    uint32_t pad = 0;
    IAudioClient_GetCurrentPadding(p->client, &pad);
    uint64_t played = (p->hw_Frames > pad) ? (p->hw_Frames - pad) : 0;
    return (snd_pcm_sframes_t)(played % io->buffer_size);
}

static snd_pcm_sframes_t lwasapi_io_transfer(
    snd_pcm_ioplug_t *io,
    const snd_pcm_channel_area_t *areas,
    snd_pcm_uframes_t offset,
    snd_pcm_uframes_t size
) {
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (!p || !p->client || !areas) return -EIO;

    if (io->stream == SND_PCM_STREAM_CAPTURE) {
        uint8_t *dst = (uint8_t *)areas[0].addr + (areas[0].first + areas[0].step * offset) / 8;
        if (IAudioCaptureClient_ReadFrames(p->client, dst, (uint32_t)size, p->fmt) != S_OK) {
            return -EIO;
        }
        p->hw_Frames += size;
        return (snd_pcm_sframes_t)size;
    }

    const uint8_t *src = (const uint8_t *)areas[0].addr + (areas[0].first + areas[0].step * offset) / 8;
    if (IAudioRenderClient_WriteFrames(p->client, src, (uint32_t)size, p->fmt) != S_OK) {
        return -EIO;
    }
    p->hw_Frames += size;
    return (snd_pcm_sframes_t)size;
}

static int lwasapi_io_hw_params(snd_pcm_ioplug_t *io, snd_pcm_hw_params_t *params) {
    (void)params;
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (!p || !p->client) return -EIO;

    if (io->format == SND_PCM_FORMAT_S16_LE) p->fmt = LWASAPI_FMT_S16_LE;
    else if (io->format == SND_PCM_FORMAT_S32_LE) p->fmt = LWASAPI_FMT_S32_LE;
    else p->fmt = LWASAPI_FMT_FLOAT32;

    WAVEFORMATEX wfx;
    memset(&wfx, 0, sizeof(wfx));
    wfx.wFormatTag = (uint16_t)p->fmt;
    wfx.nChannels = (uint16_t)(io->channels ? io->channels : 2);
    wfx.nSamplesPerSec = io->rate ? io->rate : 48000;
    wfx.wBitsPerSample = (p->fmt == LWASAPI_FMT_S16_LE) ? 16 : 32;
    wfx.nBlockAlign = wfx.nChannels * (wfx.wBitsPerSample / 8);
    wfx.nAvgBytesPerSec = wfx.nSamplesPerSec * wfx.nBlockAlign;

    uint32_t flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if (io->stream == SND_PCM_STREAM_CAPTURE) {
        flags |= AUDCLNT_STREAMFLAGS_CAPTURE;
    }

    HRESULT hr = IAudioClient_Initialize(
        p->client,
        AUDCLNT_SHAREMODE_SHARED,
        flags,
        io->buffer_size ? (uint32_t)io->buffer_size : 2048,
        io->period_size ? (uint32_t)io->period_size : 256,
        &wfx
    );
    if (hr != S_OK && hr != AUDCLNT_E_ALREADY_INITIALIZED) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                                 "ALSA_PLG_ERR", "lwasapi-ioplug",
                                 "IAudioClient_Initialize failed in hw_params");
        return -EIO;
    }
    char detail[128];
    snprintf(detail, sizeof(detail), "%s rate=%u ch=%u buf=%lu per=%lu",
             io->stream == SND_PCM_STREAM_CAPTURE ? "CAPTURE" : "PLAYBACK",
             wfx.nSamplesPerSec, wfx.nChannels,
             (unsigned long)io->buffer_size, (unsigned long)io->period_size);
    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "ALSA_PLG_HW", p->io.name, detail);
    IAudioClient_SetEventHandle(p->client, &p->event_fd);
    if (p->event_fd >= 0) {
        int fl = fcntl(p->event_fd, F_GETFL, 0);
        if (fl >= 0 && !(fl & O_NONBLOCK)) {
            (void)fcntl(p->event_fd, F_SETFL, fl | O_NONBLOCK);
        }
    }
    p->io.poll_fd = p->event_fd;
    p->io.poll_events = POLLIN;
    snd_pcm_ioplug_reinit_status(&p->io);
    return 0;
}

static int lwasapi_io_prepare(snd_pcm_ioplug_t *io) {
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (!p || !p->client) return -EIO;
    p->hw_Frames = 0;
    IAudioClient_Stop(p->client);
    IAudioClient_Reset(p->client);
    if (p->event_fd >= 0) {
        uint64_t val = 0;
        if (read(p->event_fd, &val, sizeof(val)) < 0) {}
    }
    return 0;
}

static int lwasapi_io_drain(snd_pcm_ioplug_t *io) {
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (!p || !p->client) return 0;
    if (io->stream == SND_PCM_STREAM_CAPTURE) {
        IAudioClient_Stop(p->client);
        return 0;
    }
    uint32_t prev_pad = UINT32_MAX;
    int stall = 0;
    for (int i = 0; i < 200; i++) {
        uint32_t pad = 0;
        IAudioClient_GetCurrentPadding(p->client, &pad);
        if (pad == 0) break;
        if (pad == prev_pad) {
            if (++stall >= 3) break;
        } else {
            stall = 0;
            prev_pad = pad;
        }
        IAudioClient_WaitForPeriod(p->client, 10);
    }
    IAudioClient_Stop(p->client);
    return 0;
}

static int lwasapi_io_poll_revents(snd_pcm_ioplug_t *io, struct pollfd *pfd, unsigned int nfds, unsigned short *revents) {
    if (!revents) return -EINVAL;
    *revents = 0;
    if (!io || !pfd || nfds == 0) return 0;
    if (pfd[0].revents & POLLIN) {
        uint64_t val = 0;
        if (read(pfd[0].fd, &val, sizeof(val)) < 0) {}
        *revents = (io->stream == SND_PCM_STREAM_CAPTURE) ? POLLIN : POLLOUT;
    }
    *revents |= (pfd[0].revents & (POLLERR | POLLHUP | POLLNVAL));
    return 0;
}

static int lwasapi_io_close(snd_pcm_ioplug_t *io) {
    lwasapi_alsa_ioplug_t *p = (lwasapi_alsa_ioplug_t *)io->private_data;
    if (p) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                                 "ALSA_PLG_CLOSE", p->io.name ? p->io.name : "lwasapi",
                                 "lwasapi_io_close");
        if (p->client) IAudioClient_Release(p->client);
        free(p);
    }
    return 0;
}

static const snd_pcm_ioplug_callback_t lwasapi_io_callbacks = {
    .start        = lwasapi_io_start,
    .stop         = lwasapi_io_stop,
    .pointer      = lwasapi_io_pointer,
    .transfer     = lwasapi_io_transfer,
    .hw_params    = lwasapi_io_hw_params,
    .prepare      = lwasapi_io_prepare,
    .drain        = lwasapi_io_drain,
    .poll_revents = lwasapi_io_poll_revents,
    .close        = lwasapi_io_close,
};

SND_PCM_PLUGIN_DEFINE_FUNC(lwasapi) {
    (void)conf;
    char sock_path[108];
    lwasapi_audiodg_sock_path(sock_path, sizeof(sock_path), LWASAPI_SOCKET_DEFAULT);
    /* Do not intercept system audio daemons; fall back cleanly to sysdefault */
    if (is_system_audio_daemon() || access(sock_path, F_OK) != 0) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_WARN, 0,
                                 "ALSA_PLG_FALLBACK", name ? name : "default",
                                 "Falling back to ALSA sysdefault (daemon or no audiodg socket)");
        return snd_pcm_open(pcmp, "sysdefault", stream, mode);
    }

    lwasapi_alsa_ioplug_t *p = calloc(1, sizeof(lwasapi_alsa_ioplug_t));
    if (!p) return -ENOMEM;

    if (LWASAPI_ActivateAudioClient(NULL, &p->client) != S_OK) {
        LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_WARN, 0,
                                 "ALSA_PLG_FALLBACK", name ? name : "default",
                                 "ActivateAudioClient failed, falling back to sysdefault");
        free(p);
        return snd_pcm_open(pcmp, "sysdefault", stream, mode);
    }

    p->event_fd = -1;
    p->io.version = SND_PCM_IOPLUG_VERSION;
    p->io.name = (stream == SND_PCM_STREAM_CAPTURE)
        ? "LWASAPI Direct WaveRT Capture Engine"
        : "LWASAPI Direct WaveRT Render Engine";
    p->io.poll_fd = -1;
    p->io.poll_events = 0;
    p->io.callback = &lwasapi_io_callbacks;
    p->io.private_data = p;

    LWASAPI_DebugTraceClient(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                             "ALSA_PLG_OPEN", name ? name : "lwasapi", p->io.name);

    int err = snd_pcm_ioplug_create(&p->io, name, stream, mode);
    if (err < 0) {
        IAudioClient_Release(p->client);
        free(p);
        return err;
    }

    static const unsigned int accesses[] = {
        SND_PCM_ACCESS_RW_INTERLEAVED,
        SND_PCM_ACCESS_MMAP_INTERLEAVED
    };
    static const unsigned int formats[] = {
        SND_PCM_FORMAT_FLOAT_LE,
        SND_PCM_FORMAT_S32_LE,
        SND_PCM_FORMAT_S16_LE
    };

    snd_pcm_ioplug_set_param_list(&p->io, SND_PCM_IOPLUG_HW_ACCESS, 2, accesses);
    snd_pcm_ioplug_set_param_list(&p->io, SND_PCM_IOPLUG_HW_FORMAT, 3, formats);
    snd_pcm_ioplug_set_param_minmax(&p->io, SND_PCM_IOPLUG_HW_CHANNELS, 1, 8);
    snd_pcm_ioplug_set_param_minmax(&p->io, SND_PCM_IOPLUG_HW_RATE, 8000, 192000);
    snd_pcm_ioplug_set_param_minmax(&p->io, SND_PCM_IOPLUG_HW_PERIOD_BYTES, 256, 16384);
    snd_pcm_ioplug_set_param_minmax(&p->io, SND_PCM_IOPLUG_HW_BUFFER_BYTES, 1024, 65536);
    snd_pcm_ioplug_set_param_minmax(&p->io, SND_PCM_IOPLUG_HW_PERIODS, 2, 64);

    *pcmp = p->io.pcm;
    return 0;
}

SND_PCM_PLUGIN_SYMBOL(lwasapi);
