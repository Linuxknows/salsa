#define _GNU_SOURCE
#include "../include/lwasapi_midi.h"
#include "../include/lwasapi_debug.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <time.h>
#include <dlfcn.h>

/* ============================================================================
 * LWASAPI Hardware & Virtual DAW MIDI Router
 *
 * 1. Scans /proc/asound/cards and /dev/snd/midiC*D* to auto-connect hardware
 *    MIDI controllers (e.g. Novation FLkey 37 on /dev/snd/midiC5D0).
 * 2. Registers an ALSA Sequencer hub ("LWASAPI-MIDI-Hub") on /dev/snd/seq and
 *    subscribes to hardware MIDI ports (Client 36 "FLkey 37", Client 14 "Midi Through").
 * 3. Decodes MIDI messages with nanosecond timestamps and sample-accurate
 *    period frame offsets into each DAW session's lock-free shared-memory ring.
 * ============================================================================ */

static uint64_t now_monotonic_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static int midi_expected_bytes(uint8_t status) {
    uint8_t hi = status & 0xF0;
    if (hi == 0x80 || hi == 0x90 || hi == 0xA0 || hi == 0xB0 || hi == 0xE0) return 3;
    if (hi == 0xC0 || hi == 0xD0) return 2;
    if (status >= 0xF8) return 1; /* Real-time clock/transport */
    return 0;
}

static void discover_midi_card_name(int card_num, char *out, size_t out_len) {
    snprintf(out, out_len, "Hardware MIDI Card %d", card_num);
    FILE *fp = fopen("/proc/asound/cards", "r");
    if (!fp) return;
    char line[256];
    while (fgets(line, sizeof(line), fp)) {
        int c = -1;
        char id[32], desc[128];
        if (sscanf(line, " %d [%31[^]]]: %127[^\n]", &c, id, desc) == 3 && c == card_num) {
            char *dash = strstr(desc, " - ");
            snprintf(out, out_len, "%.*s", (int)(out_len - 1), dash ? (dash + 3) : desc);
            break;
        }
    }
    fclose(fp);
}

typedef int  (*fn_seq_event_input_t)(void *, void **);
typedef int  (*fn_seq_free_event_t)(void *);
typedef int  (*fn_midi_event_new_t)(size_t, void **);
typedef void (*fn_midi_event_free_t)(void *);
typedef void (*fn_midi_event_no_status_t)(void *, int);
typedef long (*fn_midi_event_decode_t)(void *, unsigned char *, long, const void *);

static fn_seq_event_input_t      g_p_seq_input  = NULL;
static fn_seq_free_event_t       g_p_seq_free   = NULL;
static fn_midi_event_new_t       g_p_midi_new   = NULL;
static fn_midi_event_free_t      g_p_midi_free  = NULL;
static fn_midi_event_no_status_t g_p_midi_nost  = NULL;
static fn_midi_event_decode_t    g_p_midi_dec   = NULL;
static void                     *g_seq_coder    = NULL;

int lwasapi_midi_init(lwasapi_midi_hub_t *m) {
    memset(m, 0, sizeof(*m));
    m->rawmidi_fd = -1;
    m->seq_client_id = -1;
    m->seq_in_port = -1;
    snprintf(m->controller_name, sizeof(m->controller_name), "Virtual MIDI Hub");

    /* 1. Discover hardware USB/DIN MIDI controllers (e.g. FLkey 37 on C5D0) */
    for (int c = 0; c < 16; c++) {
        char path[64];
        snprintf(path, sizeof(path), "/dev/snd/midiC%dD0", c);
        if (access(path, R_OK) == 0) {
            discover_midi_card_name(c, m->controller_name, sizeof(m->controller_name));
            snprintf(m->dev_path, sizeof(m->dev_path), "%s", path);
            break;
        }
    }

    /* 2. Open /dev/snd/seq via system libasound to create "LWASAPI-MIDI-Hub" */
    static const char *const alsa_cands[] = {
        "/usr/lib/libasound.so.2.0.0",
        "/usr/lib/libasound.so.2",
        "/usr/lib64/libasound.so.2.0.0",
        "/usr/lib64/libasound.so.2",
        "/usr/lib/x86_64-linux-gnu/libasound.so.2.0.0",
        "/usr/lib/x86_64-linux-gnu/libasound.so.2",
        "/usr/lib/aarch64-linux-gnu/libasound.so.2.0.0",
        "/usr/lib/aarch64-linux-gnu/libasound.so.2",
        "/lib/x86_64-linux-gnu/libasound.so.2",
        "/lib64/libasound.so.2",
        "libasound.so.2",
        NULL
    };
    for (int i = 0; alsa_cands[i]; i++) {
        m->alsa_dl = dlopen(alsa_cands[i], RTLD_NOW | RTLD_LOCAL);
        if (m->alsa_dl) break;
    }
    if (m->alsa_dl) {
        typedef int (*fn_seq_open)(void **, const char *, int, int);
        typedef int (*fn_set_name)(void *, const char *);
        typedef int (*fn_create_port)(void *, const char *, unsigned int, unsigned int);
        typedef int (*fn_connect_from)(void *, int, int, int);
        typedef int (*fn_nonblock)(void *, int);

        typedef int (*fn_seq_client_id)(void *);
        fn_seq_open p_open = (fn_seq_open)dlsym(m->alsa_dl, "snd_seq_open");
        fn_set_name p_name = (fn_set_name)dlsym(m->alsa_dl, "snd_seq_set_client_name");
        fn_create_port p_port = (fn_create_port)dlsym(m->alsa_dl, "snd_seq_create_simple_port");
        fn_connect_from p_conn = (fn_connect_from)dlsym(m->alsa_dl, "snd_seq_connect_from");
        fn_nonblock p_nb = (fn_nonblock)dlsym(m->alsa_dl, "snd_seq_nonblock");
        fn_seq_client_id p_cid = (fn_seq_client_id)dlsym(m->alsa_dl, "snd_seq_client_id");
        g_p_seq_input = (fn_seq_event_input_t)dlsym(m->alsa_dl, "snd_seq_event_input");
        g_p_seq_free  = (fn_seq_free_event_t)dlsym(m->alsa_dl, "snd_seq_free_event");
        g_p_midi_new  = (fn_midi_event_new_t)dlsym(m->alsa_dl, "snd_midi_event_new");
        g_p_midi_free = (fn_midi_event_free_t)dlsym(m->alsa_dl, "snd_midi_event_free");
        g_p_midi_nost = (fn_midi_event_no_status_t)dlsym(m->alsa_dl, "snd_midi_event_no_status");
        g_p_midi_dec  = (fn_midi_event_decode_t)dlsym(m->alsa_dl, "snd_midi_event_decode");

        if (p_open && p_name && p_port) {
            if (p_open(&m->seq_handle, "default", 3 /* DUPLEX */, 1 /* NONBLOCK */) == 0 && m->seq_handle) {
                if (p_nb) p_nb(m->seq_handle, 1);
                p_name(m->seq_handle, "LWASAPI-MIDI-Hub");
                m->seq_client_id = p_cid ? p_cid(m->seq_handle) : -1;
                /* Capabilities: WRITE | SUBS_WRITE | READ | SUBS_READ */
                m->seq_in_port = p_port(m->seq_handle, "LWASAPI DAW MIDI In/Out", 0x20 | 0x40 | 0x01 | 0x02, 0x02 | 0x100000);
                if (g_p_midi_new && g_p_midi_dec) {
                    if (g_seq_coder && g_p_midi_free) {
                        g_p_midi_free(g_seq_coder);
                        g_seq_coder = NULL;
                    }
                    if (g_p_midi_new(64, &g_seq_coder) == 0 && g_seq_coder && g_p_midi_nost) {
                        g_p_midi_nost(g_seq_coder, 1);
                    }
                }
                if (p_conn && m->seq_in_port >= 0) {
                    /* Auto-subscribe to Midi Through (14:0) and hardware MIDI ports (e.g. FLkey 37 on 36:0, 36:1) */
                    p_conn(m->seq_handle, m->seq_in_port, 14, 0);
                    for (int client = 16; client <= 64; client++) {
                        if (client == m->seq_client_id) continue;
                        p_conn(m->seq_handle, m->seq_in_port, client, 0);
                        p_conn(m->seq_handle, m->seq_in_port, client, 1);
                    }
                }
                fprintf(stderr, "[LWASAPI-MIDI] Registered kernel ALSA Sequencer hub 'LWASAPI-MIDI-Hub' (auto-subscribed to '%s')\n",
                        m->controller_name);
            }
        }
    }

    /* 3. Fallback to direct RawMIDI fd if ALSA Sequencer decode is unavailable */
    if ((!m->seq_handle || !g_p_seq_input || !g_p_midi_dec || !g_seq_coder) && m->dev_path[0]) {
        m->rawmidi_fd = open(m->dev_path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    }

    m->initialized = true;
    lwasapi_debug_emit_vals(LWASAPI_DBG_MIDI, LWASAPI_DBG_INFO, 0, 0,
                            m->seq_client_id, m->seq_in_port, m->rawmidi_fd,
                            "MIDI_INIT", m->controller_name,
                            "dev=%s seq_client=%d port=%d rawmidi_fd=%d",
                            m->dev_path[0] ? m->dev_path : "virtual",
                            m->seq_client_id, m->seq_in_port, m->rawmidi_fd);
    return 0;
}

static uint32_t compute_frame_offset(uint64_t ev_ns, uint64_t period_start_ns, uint32_t sample_rate, uint32_t period_frames) {
    if (ev_ns <= period_start_ns) return 0;
    uint64_t delta_ns = ev_ns - period_start_ns;
    uint32_t frame = (uint32_t)((delta_ns * sample_rate) / 1000000000ULL);
    if (frame >= period_frames) frame = period_frames > 0 ? (period_frames - 1) : 0;
    return frame;
}

int lwasapi_midi_poll_events(lwasapi_midi_hub_t *m, lwasapi_midi_event_t *out_events, int max_events, uint64_t period_start_ns, uint32_t sample_rate, uint32_t period_frames) {
    if (!m || !m->initialized || !out_events || max_events <= 0) return 0;
    int count = 0;

    /* 1. Drain ALSA Sequencer hub ("LWASAPI-MIDI-Hub") if active */
    if (m->seq_handle && g_p_seq_input && g_p_midi_dec && g_seq_coder) {
        while (count < max_events) {
            void *seq_ev = NULL;
            int sr = g_p_seq_input(m->seq_handle, &seq_ev);
            if (sr < 0 || !seq_ev) break;

            unsigned char buf[64];
            long dec_len = g_p_midi_dec(g_seq_coder, buf, (long)sizeof(buf), seq_ev);
            if (g_p_seq_free) g_p_seq_free(seq_ev);

            if (dec_len >= 1 && dec_len <= 3) {
                lwasapi_midi_event_t *ev = &out_events[count++];
                ev->timestamp_ns = now_monotonic_ns();
                ev->frame_offset = compute_frame_offset(ev->timestamp_ns, period_start_ns, sample_rate, period_frames);
                ev->size = (uint8_t)dec_len;
                ev->port = 0;
                ev->data[0] = buf[0];
                ev->data[1] = (dec_len >= 2) ? buf[1] : 0;
                ev->data[2] = (dec_len >= 3) ? buf[2] : 0;
                ev->data[3] = 0;
                if (lwasapi_midi_is_key_press(ev)) {
                    atomic_fetch_add_explicit(&m->total_events, 1, memory_order_relaxed);
                }
                if (dec_len == 1 && buf[0] >= 0xF8) {
                    if (buf[0] != 0xF8 && buf[0] != 0xFE) {
                        lwasapi_debug_emit_vals(LWASAPI_DBG_MIDI, LWASAPI_DBG_TRACE, 0, 0,
                                                (int32_t)buf[0], (int32_t)ev->frame_offset, 0,
                                                "MIDI_RT", m->controller_name,
                                                "rt_byte=0x%02X off=%u", buf[0], ev->frame_offset);
                    }
                } else {
                    lwasapi_debug_emit_vals(LWASAPI_DBG_MIDI, LWASAPI_DBG_INFO, 0, 0,
                                            (int32_t)ev->data[0], (int32_t)ev->data[1], (int32_t)ev->data[2],
                                            "MIDI_RX", m->controller_name,
                                            "[%02X %02X %02X] ch=%u off=%u",
                                            ev->data[0], ev->data[1], ev->data[2],
                                            (unsigned)(ev->data[0] & 0x0F) + 1U, ev->frame_offset);
                }
            }
        }
    }

    /* 2. Poll direct /dev/snd/midiC*D0 byte stream if open */
    if (m->rawmidi_fd >= 0 && count < max_events) {
        uint8_t buf[64];
        ssize_t n = read(m->rawmidi_fd, buf, sizeof(buf));
        for (ssize_t i = 0; i < n && count < max_events; i++) {
            uint8_t b = buf[i];
            if (b >= 0xF8) {
                /* Single-byte real-time message (clock/sensing/transport: never a key press) */
                lwasapi_midi_event_t *ev = &out_events[count++];
                ev->timestamp_ns = now_monotonic_ns();
                ev->frame_offset = compute_frame_offset(ev->timestamp_ns, period_start_ns, sample_rate, period_frames);
                ev->size = 1;
                ev->port = 0;
                ev->data[0] = b;
                ev->data[1] = 0;
                ev->data[2] = 0;
                ev->data[3] = 0;
                if (b != 0xF8 && b != 0xFE) {
                    lwasapi_debug_emit_vals(LWASAPI_DBG_MIDI, LWASAPI_DBG_TRACE, 0, 0,
                                            (int32_t)b, (int32_t)ev->frame_offset, 0,
                                            "MIDI_RT", m->controller_name,
                                            "rt_byte=0x%02X off=%u", b, ev->frame_offset);
                }
                continue;
            }
            if (b & 0x80) {
                m->running_status = b;
                m->msg_expected = midi_expected_bytes(b);
                m->msg_buf[0] = b;
                m->msg_idx = 1;
            } else if (m->running_status != 0 && m->msg_expected > 1) {
                if (m->msg_idx == 0) {
                    m->msg_buf[0] = m->running_status;
                    m->msg_idx = 1;
                }
                m->msg_buf[m->msg_idx++] = b;
                if (m->msg_idx >= m->msg_expected) {
                    lwasapi_midi_event_t *ev = &out_events[count++];
                    ev->timestamp_ns = now_monotonic_ns();
                    ev->frame_offset = compute_frame_offset(ev->timestamp_ns, period_start_ns, sample_rate, period_frames);
                    ev->size = (uint8_t)m->msg_expected;
                    ev->port = 0;
                    ev->data[0] = m->msg_buf[0];
                    ev->data[1] = m->msg_buf[1];
                    ev->data[2] = (m->msg_expected > 2) ? m->msg_buf[2] : 0;
                    ev->data[3] = 0;
                    m->msg_idx = 0;
                    if (lwasapi_midi_is_key_press(ev)) {
                        atomic_fetch_add_explicit(&m->total_events, 1, memory_order_relaxed);
                    }
                    lwasapi_debug_emit_vals(LWASAPI_DBG_MIDI, LWASAPI_DBG_INFO, 0, 0,
                                            (int32_t)ev->data[0], (int32_t)ev->data[1], (int32_t)ev->data[2],
                                            "MIDI_RX", m->controller_name,
                                            "[%02X %02X %02X] ch=%u off=%u",
                                            ev->data[0], ev->data[1], ev->data[2],
                                            (unsigned)(ev->data[0] & 0x0F) + 1U, ev->frame_offset);
                }
            }
        }
    }

    return count;
}

void lwasapi_midi_push_to_ring(lwasapi_shm_ring_t *ring, const lwasapi_midi_event_t *ev) {
    if (!ring || !ev) return;
    uint64_t w = atomic_load_explicit(&ring->midi_write_idx, memory_order_relaxed);
    uint64_t r = atomic_load_explicit(&ring->midi_read_idx, memory_order_acquire);
    if (w - r >= LWASAPI_MIDI_RING_EVENTS) {
        return; /* Ring full */
    }
    ring->midi_ring[w & LWASAPI_MIDI_RING_MASK] = *ev;
    atomic_store_explicit(&ring->midi_write_idx, w + 1, memory_order_release);
}

void lwasapi_midi_close(lwasapi_midi_hub_t *m) {
    if (!m || !m->initialized) return;
    if (m->rawmidi_fd >= 0) {
        close(m->rawmidi_fd);
        m->rawmidi_fd = -1;
    }
    if (g_seq_coder && g_p_midi_free) {
        g_p_midi_free(g_seq_coder);
        g_seq_coder = NULL;
    }
    if (m->seq_handle && m->alsa_dl) {
        typedef int (*fn_seq_close)(void *);
        fn_seq_close p_close = (fn_seq_close)dlsym(m->alsa_dl, "snd_seq_close");
        if (p_close) p_close(m->seq_handle);
        m->seq_handle = NULL;
    }
    g_p_seq_input = NULL;
    g_p_seq_free  = NULL;
    g_p_midi_new  = NULL;
    g_p_midi_free = NULL;
    g_p_midi_nost = NULL;
    g_p_midi_dec  = NULL;
    if (m->alsa_dl) {
        dlclose(m->alsa_dl);
        m->alsa_dl = NULL;
    }
    m->initialized = false;
}
