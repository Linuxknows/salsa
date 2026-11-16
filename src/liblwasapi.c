#define _GNU_SOURCE
#include "../include/lwasapi.h"
#include "../include/lwasapi_debug.h"
#include "../include/lwasapi_paths.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <poll.h>
#include <stdatomic.h>
#include <stdarg.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>

struct IAudioClient {
    char               client_name[64];
    uint32_t           client_pid;
    uint32_t           session_id;
    bool               initialized;
    int                sock_fd;
    int                shm_fd;
    int                event_fd;
    lwasapi_shm_ring_t *ring;
    uint32_t           locked_frames;
    float              *staging_buf;
};

static _Atomic int g_dbg_dgram_fd = -1;

static void dbg_emit_dgram(uint8_t sub, uint8_t lvl, uint32_t sid,
                           int32_t v1, int32_t v2, int32_t hr,
                           const char *op, const char *target, const char *fmt, ...)
    __attribute__((format(printf, 9, 10)));

static void dbg_emit_dgram(uint8_t sub, uint8_t lvl, uint32_t sid,
                           int32_t v1, int32_t v2, int32_t hr,
                           const char *op, const char *target, const char *fmt, ...) {
    int fd = atomic_load_explicit(&g_dbg_dgram_fd, memory_order_acquire);
    if (fd < 0) {
        int nfd = socket(AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
        if (nfd < 0) return;
        int expected = -1;
        if (atomic_compare_exchange_strong_explicit(&g_dbg_dgram_fd, &expected, nfd,
                memory_order_release, memory_order_acquire)) {
            fd = nfd;
        } else {
            close(nfd);
            fd = expected;
        }
        if (fd < 0) return;
    }

    lwasapi_dbg_dgram_t dg;
    memset(&dg, 0, sizeof(dg));
    dg.magic = LWASAPI_DEBUG_DGRAM_MAGIC;
    dg.pid = (uint32_t)getpid();
    dg.session_id = sid;
    dg.v1 = v1;
    dg.v2 = v2;
    dg.hr = hr;
    dg.sub = sub;
    dg.lvl = lvl;
    if (op) snprintf(dg.op, sizeof(dg.op), "%s", op);
    if (target) snprintf(dg.target, sizeof(dg.target), "%s", target);
    if (fmt && fmt[0] != '\0') {
        va_list ap;
        va_start(ap, fmt);
        vsnprintf(dg.detail, sizeof(dg.detail), fmt, ap);
        va_end(ap);
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    lwasapi_debug_in_sock_path(addr.sun_path, sizeof(addr.sun_path), LWASAPI_DEBUG_IN_SOCK_DEFAULT);

    ssize_t rc = sendto(fd, &dg, sizeof(dg), MSG_DONTWAIT | MSG_NOSIGNAL,
                        (const struct sockaddr *)&addr, sizeof(addr));
    (void)rc;
}

void LWASAPI_DebugTraceClient(uint8_t sub, uint8_t lvl, uint32_t sid,
                              const char *op, const char *target, const char *detail) {
    dbg_emit_dgram(sub, lvl, sid, 0, 0, 0, op, target, "%s", detail ? detail : "");
}

static const char *get_socket_path(void) {
    static _Thread_local char s_sock_buf[108];
    lwasapi_audiodg_sock_path(s_sock_buf, sizeof(s_sock_buf), LWASAPI_SOCKET_DEFAULT);
    return s_sock_buf;
}

static void get_process_comm(char *out, size_t out_len) {
    out[0] = '\0';
    int fd = open("/proc/self/comm", O_RDONLY | O_CLOEXEC);
    if (fd >= 0) {
        ssize_t n = read(fd, out, out_len - 1);
        close(fd);
        if (n > 0) {
            out[n] = '\0';
            for (ssize_t i = 0; i < n; i++) {
                if (out[i] == '\n' || out[i] == '\r') {
                    out[i] = '\0';
                    break;
                }
            }
        }
    }
    if (!out[0]) {
        snprintf(out, out_len, "app");
    }
}

static int recv_response_with_fds(int sock, lwasapi_ipc_resp_t *resp, int *fd1, int *fd2) {
    struct msghdr msg;
    struct iovec iov;
    memset(&msg, 0, sizeof(msg));

    iov.iov_base = resp;
    iov.iov_len = sizeof(*resp);
    msg.msg_iov = &iov;
    msg.msg_iovlen = 1;

    char cmsg_buf[CMSG_SPACE(sizeof(int) * 2)];
    memset(cmsg_buf, 0, sizeof(cmsg_buf));
    msg.msg_control = cmsg_buf;
    msg.msg_controllen = sizeof(cmsg_buf);

    *fd1 = -1;
    *fd2 = -1;

    ssize_t n = recvmsg(sock, &msg, 0);
    if (n != (ssize_t)sizeof(*resp)) {
        return -1;
    }

    for (struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg); cmsg != NULL; cmsg = CMSG_NXTHDR(&msg, cmsg)) {
        if (cmsg->cmsg_level == SOL_SOCKET && cmsg->cmsg_type == SCM_RIGHTS) {
            int fds[2];
            memcpy(fds, CMSG_DATA(cmsg), sizeof(fds));
            *fd1 = fds[0];
            *fd2 = fds[1];
            break;
        }
    }
    return 0;
}

HRESULT LWASAPI_ActivateAudioClient(const char *client_name, IAudioClient **ppAudioClient) {
    if (!ppAudioClient) return E_INVALIDARG;

    IAudioClient *c = calloc(1, sizeof(IAudioClient));
    if (!c) return E_OUTOFMEMORY;

    c->sock_fd = -1;
    c->shm_fd = -1;
    c->event_fd = -1;
    c->client_pid = (uint32_t)getpid();

    char proc_comm[32];
    get_process_comm(proc_comm, sizeof(proc_comm));

    if (!client_name || !client_name[0] ||
        strcmp(client_name, "default") == 0 ||
        strcmp(client_name, "LWASAPI-Client") == 0 ||
        strcmp(client_name, proc_comm) == 0) {
        snprintf(c->client_name, sizeof(c->client_name), "%s", proc_comm);
    } else if (strncmp(client_name, proc_comm, strlen(proc_comm)) == 0) {
        snprintf(c->client_name, sizeof(c->client_name), "%s", client_name);
    } else {
        snprintf(c->client_name, sizeof(c->client_name), "%s:%s", proc_comm, client_name);
    }

    *ppAudioClient = c;
    return S_OK;
}

HRESULT IAudioClient_GetMixFormat(IAudioClient *client, WAVEFORMATEX *pFormat) {
    if (!client || !pFormat) return E_INVALIDARG;
    memset(pFormat, 0, sizeof(*pFormat));
    pFormat->wFormatTag = LWASAPI_FMT_FLOAT32;
    pFormat->nChannels = 2;
    pFormat->nSamplesPerSec = 48000;
    pFormat->wBitsPerSample = 32;
    pFormat->nBlockAlign = 2 * sizeof(float);
    pFormat->nAvgBytesPerSec = 48000 * pFormat->nBlockAlign;
    return S_OK;
}

HRESULT IAudioClient_Initialize(
    IAudioClient *client,
    AUDCLNT_SHAREMODE ShareMode,
    uint32_t StreamFlags,
    uint32_t BufferFrameCount,
    uint32_t PeriodicityFrames,
    const WAVEFORMATEX *pFormat
) {
    if (!client || !pFormat) return E_INVALIDARG;
    if (client->initialized) return AUDCLNT_E_ALREADY_INITIALIZED;

    int sock = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (sock < 0) return E_FAIL;

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", get_socket_path());

    if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        int saved_errno = errno;
        close(sock);
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_ERR, 0,
                       (int32_t)pFormat->nSamplesPerSec, (int32_t)pFormat->nChannels, E_FAIL,
                       "WASAPI_INIT_FAIL", client->client_name,
                       "connect(%s) failed: %s", get_socket_path(), strerror(saved_errno));
        return E_FAIL;
    }

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_CREATE_SESSION;
    snprintf(req.client_name, sizeof(req.client_name), "%s", client->client_name);
    req.client_pid = client->client_pid;
    req.share_mode = (uint32_t)ShareMode;
    req.stream_flags = StreamFlags;
    req.sample_rate = pFormat->nSamplesPerSec ? pFormat->nSamplesPerSec : 48000;
    req.channels = pFormat->nChannels ? pFormat->nChannels : 2;
    req.format = pFormat->wFormatTag ? pFormat->wFormatTag : LWASAPI_FMT_FLOAT32;
    req.requested_buffer_frames = BufferFrameCount ? BufferFrameCount : 2048;
    req.requested_period_frames = PeriodicityFrames ? PeriodicityFrames : 256;

    if (send(sock, &req, sizeof(req), MSG_NOSIGNAL) != (ssize_t)sizeof(req)) {
        close(sock);
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_ERR, 0,
                       (int32_t)req.sample_rate, (int32_t)req.channels, E_FAIL,
                       "WASAPI_INIT_FAIL", client->client_name, "IPC send CREATE_SESSION failed");
        return E_FAIL;
    }

    lwasapi_ipc_resp_t resp;
    int shm_fd = -1, ev_fd = -1;
    if (recv_response_with_fds(sock, &resp, &shm_fd, &ev_fd) < 0) {
        close(sock);
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_ERR, 0,
                       (int32_t)req.sample_rate, (int32_t)req.channels, E_FAIL,
                       "WASAPI_INIT_FAIL", client->client_name, "recv_response_with_fds failed");
        return E_FAIL;
    }

    if (resp.hr != S_OK || shm_fd < 0 || ev_fd < 0) {
        if (shm_fd >= 0) close(shm_fd);
        if (ev_fd >= 0) close(ev_fd);
        close(sock);
        HRESULT fail_hr = (resp.hr != S_OK) ? resp.hr : E_FAIL;
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_ERR, resp.session_id,
                       (int32_t)req.sample_rate, (int32_t)req.channels, fail_hr,
                       "WASAPI_INIT_FAIL", client->client_name,
                       "audiodg rejected hr=0x%08x shm=%d ev=%d",
                       (unsigned)resp.hr, shm_fd, ev_fd);
        return fail_hr;
    }

    lwasapi_shm_ring_t *ring = mmap(NULL, sizeof(lwasapi_shm_ring_t),
                                    PROT_READ | PROT_WRITE, MAP_SHARED, shm_fd, 0);
    if (ring == MAP_FAILED) {
        close(shm_fd);
        close(ev_fd);
        close(sock);
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_ERR, resp.session_id,
                       0, 0, E_OUTOFMEMORY, "WASAPI_INIT_FAIL", client->client_name,
                       "mmap shm ring failed");
        return E_OUTOFMEMORY;
    }

    client->sock_fd = sock;
    client->shm_fd = shm_fd;
    client->event_fd = ev_fd;
    client->ring = ring;
    client->session_id = resp.session_id;
    uint32_t sb_frames = resp.actual_buffer_frames ? resp.actual_buffer_frames : LWASAPI_RING_FRAMES;
    if (sb_frames > LWASAPI_RING_FRAMES) sb_frames = LWASAPI_RING_FRAMES;
    uint32_t sb_ch = resp.actual_channels ? resp.actual_channels : 2U;
    if (sb_ch > LWASAPI_MAX_CHANNELS) sb_ch = LWASAPI_MAX_CHANNELS;
    client->staging_buf = calloc((size_t)sb_frames * sb_ch, sizeof(float));
    if (!client->staging_buf) {
        munmap(ring, sizeof(lwasapi_shm_ring_t));
        close(shm_fd);
        close(ev_fd);
        close(sock);
        return E_OUTOFMEMORY;
    }
    client->initialized = true;
    dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_INFO, resp.session_id,
                   (int32_t)resp.actual_sample_rate, (int32_t)resp.actual_period_frames, S_OK,
                   "WASAPI_INIT", client->client_name,
                   "rate=%u ch=%u buf=%u per=%u flags=0x%x mode=%u",
                   resp.actual_sample_rate, resp.actual_channels,
                   resp.actual_buffer_frames, resp.actual_period_frames,
                   StreamFlags, (unsigned)ShareMode);
    return S_OK;
}

HRESULT IAudioClient_GetBufferSize(IAudioClient *client, uint32_t *pNumBufferFrames) {
    if (!client || !client->initialized || !pNumBufferFrames) return AUDCLNT_E_NOT_INITIALIZED;
    *pNumBufferFrames = client->ring->buffer_frames;
    return S_OK;
}

HRESULT IAudioClient_GetDevicePeriod(IAudioClient *client, uint32_t *pDefaultPeriodFrames, uint32_t *pMinimumPeriodFrames) {
    if (!client || !client->initialized) return AUDCLNT_E_NOT_INITIALIZED;
    if (pDefaultPeriodFrames) *pDefaultPeriodFrames = client->ring->period_frames;
    if (pMinimumPeriodFrames) *pMinimumPeriodFrames = client->ring->period_frames;
    return S_OK;
}

HRESULT IAudioClient_GetCurrentPadding(IAudioClient *client, uint32_t *pNumPaddingFrames) {
    if (!client || !client->initialized || !pNumPaddingFrames) return AUDCLNT_E_NOT_INITIALIZED;
    uint64_t w = atomic_load_explicit(&client->ring->write_frame, memory_order_relaxed);
    uint64_t r = atomic_load_explicit(&client->ring->read_frame, memory_order_acquire);
    *pNumPaddingFrames = (w >= r) ? (uint32_t)(w - r) : 0;
    return S_OK;
}

HRESULT IAudioClient_SetEventHandle(IAudioClient *client, int *pEventFdOut) {
    if (!client || !client->initialized || !pEventFdOut) return AUDCLNT_E_NOT_INITIALIZED;
    *pEventFdOut = client->event_fd;
    return S_OK;
}

HRESULT IAudioClient_Start(IAudioClient *client) {
    if (!client || !client->initialized) return AUDCLNT_E_NOT_INITIALIZED;
    atomic_store_explicit(&client->ring->active, 1, memory_order_release);
    dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_INFO, client->session_id,
                   0, 0, S_OK, "WASAPI_START", client->client_name, "stream active=1");
    return S_OK;
}

HRESULT IAudioClient_Stop(IAudioClient *client) {
    if (!client || !client->initialized) return AUDCLNT_E_NOT_INITIALIZED;
    atomic_store_explicit(&client->ring->active, 0, memory_order_release);
    dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_INFO, client->session_id,
                   0, 0, S_OK, "WASAPI_STOP", client->client_name, "stream active=0");
    return S_OK;
}

HRESULT IAudioClient_Reset(IAudioClient *client) {
    if (!client || !client->initialized) return AUDCLNT_E_NOT_INITIALIZED;
    if (client->ring->is_capture) {
        uint64_t cw = atomic_load_explicit(&client->ring->cap_write_frame, memory_order_acquire);
        atomic_store_explicit(&client->ring->cap_read_frame, cw, memory_order_release);
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_TRACE, client->session_id,
                       0, 0, S_OK, "WASAPI_RESET", client->client_name,
                       "cap_read_frame=cap_write_frame=%llu", (unsigned long long)cw);
    } else {
        uint64_t r = atomic_load_explicit(&client->ring->read_frame, memory_order_acquire);
        atomic_store_explicit(&client->ring->write_frame, r, memory_order_release);
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_TRACE, client->session_id,
                       0, 0, S_OK, "WASAPI_RESET", client->client_name,
                       "write_frame=read_frame=%llu", (unsigned long long)r);
    }
    return S_OK;
}

HRESULT IAudioRenderClient_GetBuffer(IAudioClient *client, uint32_t NumFramesRequested, float **ppData) {
    if (!client || !client->initialized || !ppData) return AUDCLNT_E_NOT_INITIALIZED;

    uint32_t padding = 0;
    IAudioClient_GetCurrentPadding(client, &padding);
    if (padding + NumFramesRequested > client->ring->buffer_frames) {
        return AUDCLNT_E_BUFFER_TOO_LARGE;
    }

    client->locked_frames = NumFramesRequested;
    *ppData = client->staging_buf;
    return S_OK;
}

HRESULT IAudioRenderClient_ReleaseBuffer(IAudioClient *client, uint32_t NumFramesWritten, uint32_t dwFlags) {
    if (!client || !client->initialized) return AUDCLNT_E_NOT_INITIALIZED;
    if (NumFramesWritten > client->locked_frames) return E_INVALIDARG;

    lwasapi_shm_ring_t *r = client->ring;
    uint32_t ch = r->channels;
    uint64_t w = atomic_load_explicit(&r->write_frame, memory_order_relaxed);

    if (dwFlags & AUDCLNT_BUFFERFLAGS_SILENT) {
        for (uint32_t f = 0; f < NumFramesWritten; f++) {
            uint32_t idx = (uint32_t)((w + f) & LWASAPI_RING_MASK) * ch;
            for (uint32_t c = 0; c < ch; c++) {
                r->pcm_data[idx + c] = 0.0f;
            }
        }
    } else {
        for (uint32_t f = 0; f < NumFramesWritten; f++) {
            uint32_t idx = (uint32_t)((w + f) & LWASAPI_RING_MASK) * ch;
            for (uint32_t c = 0; c < ch; c++) {
                r->pcm_data[idx + c] = client->staging_buf[f * ch + c];
            }
        }
    }

    atomic_store_explicit(&r->write_frame, w + NumFramesWritten, memory_order_release);
    client->locked_frames = 0;
    return S_OK;
}

HRESULT IAudioClient_WaitForPeriod(IAudioClient *client, int timeout_ms) {
    if (!client || !client->initialized) return AUDCLNT_E_NOT_INITIALIZED;
    struct pollfd pfd;
    pfd.fd = client->event_fd;
    pfd.events = POLLIN;
    int r = poll(&pfd, 1, timeout_ms);
    if (r > 0 && (pfd.revents & POLLIN)) {
        uint64_t val = 0;
        if (read(client->event_fd, &val, sizeof(val)) < 0) {}
    }
    return S_OK;
}

HRESULT IAudioRenderClient_WriteFrames(IAudioClient *client, const void *src, uint32_t frames, lwasapi_sample_format_t src_fmt) {
    if (!client || !client->initialized || !src) return E_INVALIDARG;

    uint32_t ch = client->ring->channels;
    uint32_t remaining = frames;
    const uint8_t *ptr = (const uint8_t *)src;

    while (remaining > 0) {
        uint32_t padding = 0;
        IAudioClient_GetCurrentPadding(client, &padding);
        uint32_t space = (client->ring->buffer_frames > padding)
                         ? (client->ring->buffer_frames - padding) : 0;

        if (space == 0) {
            IAudioClient_WaitForPeriod(client, 20);
            continue;
        }

        uint32_t chunk = (remaining < space) ? remaining : space;
        float *dst = NULL;
        if (IAudioRenderClient_GetBuffer(client, chunk, &dst) != S_OK) {
            break;
        }

        uint32_t samples = chunk * ch;
        if (src_fmt == LWASAPI_FMT_FLOAT32) {
            memcpy(dst, ptr, samples * sizeof(float));
            ptr += samples * sizeof(float);
        } else if (src_fmt == LWASAPI_FMT_S16_LE) {
            const int16_t *s16 = (const int16_t *)ptr;
            for (uint32_t i = 0; i < samples; i++) {
                dst[i] = (float)s16[i] / 32768.0f;
            }
            ptr += samples * sizeof(int16_t);
        } else if (src_fmt == LWASAPI_FMT_S32_LE) {
            const int32_t *s32 = (const int32_t *)ptr;
            for (uint32_t i = 0; i < samples; i++) {
                dst[i] = (float)((double)s32[i] / 2147483648.0);
            }
            ptr += samples * sizeof(int32_t);
        }

        IAudioRenderClient_ReleaseBuffer(client, chunk, 0);
        remaining -= chunk;
    }
    return S_OK;
}

HRESULT IAudioCaptureClient_GetNextPacketSize(IAudioClient *client, uint32_t *pNumFramesInNextPacket) {
    if (!client || !client->initialized || !pNumFramesInNextPacket) return AUDCLNT_E_NOT_INITIALIZED;
    uint64_t cw = atomic_load_explicit(&client->ring->cap_write_frame, memory_order_acquire);
    uint64_t cr = atomic_load_explicit(&client->ring->cap_read_frame, memory_order_relaxed);
    *pNumFramesInNextPacket = (cw >= cr) ? (uint32_t)(cw - cr) : 0;
    return S_OK;
}

HRESULT IAudioCaptureClient_ReadFrames(IAudioClient *client, void *dst, uint32_t frames, lwasapi_sample_format_t dst_fmt) {
    if (!client || !client->initialized || !dst) return E_INVALIDARG;

    lwasapi_shm_ring_t *r = client->ring;
    uint32_t ch = r->channels ? r->channels : 2;
    uint32_t bps = (dst_fmt == LWASAPI_FMT_S16_LE) ? 2 : 4;
    uint8_t *out_ptr = (uint8_t *)dst;

    uint64_t cw = atomic_load_explicit(&r->cap_write_frame, memory_order_acquire);
    uint64_t cr = atomic_load_explicit(&r->cap_read_frame, memory_order_relaxed);
    if (cw <= cr) {
        IAudioClient_WaitForPeriod(client, 10);
        cw = atomic_load_explicit(&r->cap_write_frame, memory_order_acquire);
        cr = atomic_load_explicit(&r->cap_read_frame, memory_order_relaxed);
    }

    uint64_t avail = (cw >= cr) ? (cw - cr) : 0;
    if (avail > LWASAPI_RING_FRAMES) {
        cr = cw - (LWASAPI_RING_FRAMES / 2);
        avail = cw - cr;
    }

    uint32_t to_copy = (avail < frames) ? (uint32_t)avail : frames;
    float vol = (atomic_load_explicit(&r->muted, memory_order_relaxed) != 0)
                ? 0.0f
                : ((float)atomic_load_explicit(&r->volume_milli, memory_order_relaxed) / 1000.0f);

    for (uint32_t f = 0; f < to_copy; f++) {
        uint32_t idx = (uint32_t)((cr + f) & LWASAPI_RING_MASK) * 2;
        for (uint32_t c = 0; c < ch; c++) {
            float sample = r->cap_pcm_data[idx + (c & 1)] * vol;
            if (sample > 1.0f) sample = 1.0f;
            if (sample < -1.0f) sample = -1.0f;

            if (dst_fmt == LWASAPI_FMT_FLOAT32) {
                ((float *)out_ptr)[f * ch + c] = sample;
            } else if (dst_fmt == LWASAPI_FMT_S16_LE) {
                ((int16_t *)out_ptr)[f * ch + c] = (int16_t)(sample * 32767.0f);
            } else if (dst_fmt == LWASAPI_FMT_S32_LE) {
                ((int32_t *)out_ptr)[f * ch + c] = (int32_t)(sample * 2147483520.0f);
            }
        }
    }

    if (to_copy < frames) {
        memset(out_ptr + (size_t)to_copy * ch * bps, 0, (size_t)(frames - to_copy) * ch * bps);
    }

    if (to_copy > 0) {
        atomic_store_explicit(&r->cap_read_frame, cr + to_copy, memory_order_release);
    }
    return S_OK;
}

HRESULT ISimpleAudioVolume_SetMasterVolume(IAudioClient *client, float fLevel) {
    if (!client || !client->initialized) return AUDCLNT_E_NOT_INITIALIZED;
    if (fLevel < 0.0f) fLevel = 0.0f;
    if (fLevel > 2.0f) fLevel = 2.0f;
    atomic_store_explicit(&client->ring->volume_milli, (uint32_t)(fLevel * 1000.0f), memory_order_relaxed);
    return S_OK;
}

HRESULT ISimpleAudioVolume_GetMasterVolume(IAudioClient *client, float *pfLevel) {
    if (!client || !client->initialized || !pfLevel) return AUDCLNT_E_NOT_INITIALIZED;
    *pfLevel = (float)atomic_load_explicit(&client->ring->volume_milli, memory_order_relaxed) / 1000.0f;
    return S_OK;
}

uint32_t IAudioClient_ReadMidiEvents(IAudioClient *client, lwasapi_midi_event_t *events_out, uint32_t max_events) {
    if (!client || !client->initialized || !client->ring || !events_out || max_events == 0) {
        return 0;
    }

    lwasapi_shm_ring_t *r = client->ring;
    uint64_t w  = atomic_load_explicit(&r->midi_write_idx, memory_order_acquire);
    uint64_t rd = atomic_load_explicit(&r->midi_read_idx, memory_order_relaxed);
    if (rd > w) {
        rd = w;
    } else if (w - rd > LWASAPI_MIDI_RING_EVENTS) {
        rd = w - LWASAPI_MIDI_RING_EVENTS;
    }

    uint32_t count = 0;
    while (rd < w && count < max_events) {
        events_out[count++] = r->midi_ring[rd & LWASAPI_MIDI_RING_MASK];
        rd++;
    }
    if (count > 0 || rd != atomic_load_explicit(&r->midi_read_idx, memory_order_relaxed)) {
        atomic_store_explicit(&r->midi_read_idx, rd, memory_order_release);
    }
    return count;
}

HRESULT IAudioClient_WriteMidiEvent(IAudioClient *client, const lwasapi_midi_event_t *ev) {
    if (!ev || ev->size == 0 || ev->size > 3) return E_INVALIDARG;

    /* Also push directly into local ring if initialized, plus notify daemon */
    int sock = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (sock < 0) return E_FAIL;

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", get_socket_path());

    if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(sock);
        return E_FAIL;
    }

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_INJECT_MIDI;
    req.midi_ev = *ev;
    if (client && client->client_name[0]) {
        snprintf(req.client_name, sizeof(req.client_name), "%s", client->client_name);
    }

    send(sock, &req, sizeof(req), MSG_NOSIGNAL);
    lwasapi_ipc_resp_t resp;
    recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    return S_OK;
}

HRESULT IAudioClient_Release(IAudioClient *client) {
    if (!client) return E_INVALIDARG;
    if (client->initialized) {
        dbg_emit_dgram(LWASAPI_DBG_WASAPI, LWASAPI_DBG_INFO, client->session_id,
                       0, 0, S_OK, "WASAPI_RELEASE", client->client_name,
                       "closing session shm/eventfd");
        atomic_store(&client->ring->active, 0);
        munmap(client->ring, sizeof(lwasapi_shm_ring_t));
        if (client->shm_fd >= 0) close(client->shm_fd);
        if (client->event_fd >= 0) close(client->event_fd);
        if (client->sock_fd >= 0) close(client->sock_fd);
        free(client->staging_buf);
    }
    free(client);
    return S_OK;
}

static int connect_audiodg_sock(void) {
    int sock = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (sock < 0) return -1;

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", get_socket_path());

    if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(sock);
        return -1;
    }
    return sock;
}

HRESULT LWASAPI_GetEngineStatus(lwasapi_engine_status_t *out_status) {
    if (!out_status) return E_INVALIDARG;
    int sock = connect_audiodg_sock();
    if (sock < 0) return E_FAIL;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_GET_STATUS;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    ssize_t n = recv(sock, out_status, sizeof(*out_status), MSG_WAITALL);
    close(sock);
    return (n == (ssize_t)sizeof(*out_status)) ? S_OK : E_FAIL;
}

static HRESULT send_engine_config_ipc(uint32_t sample_rate, uint32_t period_frames,
                                      uint32_t format_bits, uint32_t stream_flags) {
    int sock = connect_audiodg_sock();
    if (sock < 0) {
        dbg_emit_dgram(LWASAPI_DBG_SHIM, LWASAPI_DBG_ERR, 0,
                       (int32_t)sample_rate, (int32_t)period_frames, E_FAIL,
                       "CTL_SET_CFG_FAIL", "audiodg", "connect failed");
        return E_FAIL;
    }

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_ENGINE_CONFIG;
    req.sample_rate = sample_rate;
    req.requested_period_frames = period_frames;
    req.format = (lwasapi_sample_format_t)format_bits;
    req.stream_flags = stream_flags;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    ssize_t n = recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    HRESULT hr = (n == (ssize_t)sizeof(resp)) ? resp.hr : E_FAIL;
    char pname[64];
    get_process_comm(pname, sizeof(pname));
    dbg_emit_dgram(LWASAPI_DBG_SHIM, (hr == S_OK) ? LWASAPI_DBG_INFO : LWASAPI_DBG_WARN, 0,
                   (int32_t)sample_rate, (int32_t)period_frames, hr,
                   (stream_flags == LWASAPI_CFG_FLAG_CAPTURE_RATE) ? "CTL_SET_CAP_CFG" : "CTL_SET_CFG",
                   pname[0] ? pname : "ctl",
                   "rate=%u period=%u bits=%u flags=0x%x hr=0x%08x",
                   sample_rate, period_frames, format_bits, stream_flags, (unsigned)hr);
    return hr;
}

HRESULT LWASAPI_SetEngineConfig(uint32_t sample_rate, uint32_t period_frames, uint32_t format_bits) {
    return send_engine_config_ipc(sample_rate, period_frames, format_bits, 0U);
}

HRESULT LWASAPI_SetCaptureSampleRate(uint32_t sample_rate) {
    return send_engine_config_ipc(sample_rate, 0U, 0U, LWASAPI_CFG_FLAG_CAPTURE_RATE);
}

HRESULT LWASAPI_SetDynamicEQ(const lwasapi_eq_config_t *eq_cfg) {
    if (!eq_cfg) return E_INVALIDARG;
    int sock = connect_audiodg_sock();
    if (sock < 0) return E_FAIL;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_EQ;
    req.eq_cfg = *eq_cfg;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    ssize_t n = recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    HRESULT hr = (n == (ssize_t)sizeof(resp)) ? resp.hr : E_FAIL;
    char pname[64];
    get_process_comm(pname, sizeof(pname));
    dbg_emit_dgram(LWASAPI_DBG_SHIM, LWASAPI_DBG_INFO, 0,
                   (int32_t)eq_cfg->enabled, (int32_t)eq_cfg->dynamic_mode, hr,
                   "CTL_SET_EQ", pname[0] ? pname : "ctl",
                   "en=%u dyn=%u preamp=%.1fdB thr=%.1fdB ratio=%.1f lim=%u %s",
                   eq_cfg->enabled, eq_cfg->dynamic_mode, eq_cfg->preamp_db,
                   eq_cfg->dyn_threshold_db, eq_cfg->dyn_ratio,
                   eq_cfg->limiter_enabled, eq_cfg->preset_name);
    return hr;
}

HRESULT LWASAPI_ListDevices(lwasapi_device_list_t *out_list) {
    if (!out_list) return E_INVALIDARG;
    int sock = connect_audiodg_sock();
    if (sock < 0) return E_FAIL;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_LIST_DEVICES;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    ssize_t n = recv(sock, out_list, sizeof(*out_list), MSG_WAITALL);
    close(sock);
    return (n == (ssize_t)sizeof(*out_list)) ? S_OK : E_FAIL;
}

HRESULT LWASAPI_SetDevices(const char *playback_id, const char *capture_id) {
    int sock = connect_audiodg_sock();
    if (sock < 0) return E_FAIL;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_DEVICES;
    if (playback_id && playback_id[0]) {
        snprintf(req.target_device, sizeof(req.target_device), "%s", playback_id);
    }
    if (capture_id && capture_id[0]) {
        snprintf(req.target_capture_device, sizeof(req.target_capture_device), "%s", capture_id);
    }
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    ssize_t n = recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);
    HRESULT hr = (n == (ssize_t)sizeof(resp)) ? resp.hr : E_FAIL;
    dbg_emit_dgram(LWASAPI_DBG_SHIM, (hr == S_OK) ? LWASAPI_DBG_INFO : LWASAPI_DBG_ERR, 0,
                   0, 0, hr, "CTL_SET_DEVICES",
                   (playback_id && playback_id[0]) ? playback_id : "(keep)",
                   "cap=%s hr=0x%08x",
                   (capture_id && capture_id[0]) ? capture_id : "(keep)", (unsigned)hr);
    return hr;
}

HRESULT LWASAPI_SetSessionControl(uint32_t session_id, int32_t vol_pct, int32_t mute_mode, const char *target_device) {
    int sock = connect_audiodg_sock();
    if (sock < 0) return E_FAIL;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_SESSION_CONTROL;
    req.target_session_id = session_id;
    if (vol_pct >= 0) {
        if (vol_pct > 150) vol_pct = 150;
        req.volume_milli = (uint32_t)vol_pct * 10U;
    } else {
        req.volume_milli = 0xFFFFFFFFU;
    }
    if (mute_mode >= 0 && mute_mode <= 2) {
        req.share_mode = (uint32_t)(mute_mode + 1);
    } else {
        req.share_mode = 0;
    }
    if (target_device && target_device[0]) {
        snprintf(req.target_device, sizeof(req.target_device), "%s", target_device);
    }
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    ssize_t n = recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);
    return (n == (ssize_t)sizeof(resp)) ? resp.hr : E_FAIL;
}

static HRESULT send_ext_ipc(uint32_t cmd, const void *payload, size_t payload_sz,
                            const char *target_dev, uint32_t stream_flags) {
    int sock = connect_audiodg_sock();
    if (sock < 0) return E_FAIL;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = cmd;
    req.requested_buffer_frames = (uint32_t)payload_sz;
    req.stream_flags = stream_flags;
    if (target_dev && target_dev[0]) {
        snprintf(req.target_device, sizeof(req.target_device), "%s", target_dev);
    }
    if (send(sock, &req, sizeof(req), MSG_NOSIGNAL) != (ssize_t)sizeof(req)) {
        close(sock);
        return E_FAIL;
    }
    if (payload && payload_sz > 0) {
        if (send(sock, payload, payload_sz, MSG_NOSIGNAL) != (ssize_t)payload_sz) {
            close(sock);
            return E_FAIL;
        }
    }

    lwasapi_ipc_resp_t resp;
    ssize_t n = recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);
    return (n == (ssize_t)sizeof(resp)) ? resp.hr : E_FAIL;
}

HRESULT LWASAPI_SetParametricEQ(const lwasapi_eq_config_t *eq_cfg, const lwasapi_param_eq_ext_t *param_ext) {
    if (!eq_cfg || !param_ext) return E_INVALIDARG;
    lwasapi_param_eq_payload_t pay;
    pay.eq = *eq_cfg;
    pay.ext = *param_ext;
    return send_ext_ipc(LWASAPI_CMD_SET_PARAM_EQ, &pay, sizeof(pay), NULL, 0);
}

HRESULT LWASAPI_SetMicAPO(const lwasapi_mic_apo_cfg_t *mic_cfg) {
    if (!mic_cfg) return E_INVALIDARG;
    return send_ext_ipc(LWASAPI_CMD_SET_MIC_APO, mic_cfg, sizeof(*mic_cfg), NULL, 0);
}

HRESULT LWASAPI_SetBtCodecConfig(const lwasapi_bt_codec_cfg_t *bt_cfg) {
    if (!bt_cfg) return E_INVALIDARG;
    return send_ext_ipc(LWASAPI_CMD_SET_BT_CODEC, bt_cfg, sizeof(*bt_cfg), NULL, 0);
}

HRESULT LWASAPI_SetHwAnalog(const lwasapi_hw_analog_cfg_t *hw_cfg) {
    if (!hw_cfg) return E_INVALIDARG;
    return send_ext_ipc(LWASAPI_CMD_SET_HW_ANALOG, hw_cfg, sizeof(*hw_cfg), NULL, 0);
}


