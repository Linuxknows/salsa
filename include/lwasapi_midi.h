#ifndef LWASAPI_MIDI_H
#define LWASAPI_MIDI_H

#include "lwasapi.h"

typedef struct {
    bool     initialized;
    int      rawmidi_fd;
    char     controller_name[64];
    char     dev_path[64];
    void    *alsa_dl;
    void    *seq_handle;
    int      seq_client_id;
    int      seq_in_port;
    int      seq_out_port;
    uint8_t  running_status;
    uint8_t  msg_buf[3];
    int      msg_idx;
    int      msg_expected;
    _Atomic uint64_t total_events;
} lwasapi_midi_hub_t;

static inline bool lwasapi_midi_is_key_press(const lwasapi_midi_event_t *ev) {
    if (!ev || ev->size < 3U) return false;
    return ((ev->data[0] & 0xF0U) == 0x90U) &&
           ((ev->data[1] & 0x80U) == 0U) &&
           ((ev->data[2] & 0x80U) == 0U) &&
           ((ev->data[2] & 0x7FU) > 0U);
}

int  lwasapi_midi_init(lwasapi_midi_hub_t *m);
int  lwasapi_midi_poll_events(lwasapi_midi_hub_t *m, lwasapi_midi_event_t *out_events, int max_events, uint64_t period_start_ns, uint32_t sample_rate, uint32_t period_frames);
void lwasapi_midi_push_to_ring(lwasapi_shm_ring_t *ring, const lwasapi_midi_event_t *ev);
void lwasapi_midi_close(lwasapi_midi_hub_t *m);

#endif /* LWASAPI_MIDI_H */
