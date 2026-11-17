#define _GNU_SOURCE
#include "../include/lwasapi.h"
#include "../include/lwasapi_bluez.h"
#include "../include/lwasapi_debug.h"
#include "../include/lwasapi_paths.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <netinet/in.h>
#include <arpa/inet.h>

static const char *get_socket_path(void) {
    static char s_sock_buf[108];
    lwasapi_audiodg_sock_path(s_sock_buf, sizeof(s_sock_buf), LWASAPI_SOCKET_DEFAULT);
    return s_sock_buf;
}

static int connect_engine(void) {
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

static int get_full_status(lwasapi_engine_status_t *out_st) {
    int sock = connect_engine();
    if (sock < 0) return -1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_GET_STATUS;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    ssize_t n = recv(sock, out_st, sizeof(*out_st), MSG_WAITALL);
    close(sock);
    return (n == (ssize_t)sizeof(*out_st)) ? 0 : -1;
}

static int send_payload_cmd(uint32_t cmd, const void *payload, uint32_t payload_bytes,
                            lwasapi_ipc_resp_t *out_resp) {
    int sock = connect_engine();
    if (sock < 0) return 1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = cmd;
    req.requested_buffer_frames = payload_bytes;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);
    if (payload && payload_bytes > 0) {
        send(sock, payload, payload_bytes, MSG_NOSIGNAL);
    }

    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    resp.hr = E_FAIL;
    recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);
    if (out_resp) *out_resp = resp;
    return resp.hr == S_OK ? 0 : 1;
}

static const char *bt_pref_name(uint32_t pref) {
    switch (pref) {
        case LWASAPI_BT_PREF_LDAC:    return "LDAC";
        case LWASAPI_BT_PREF_APTX_HD: return "aptX HD";
        case LWASAPI_BT_PREF_APTX:    return "aptX";
        case LWASAPI_BT_PREF_AAC:     return "AAC";
        case LWASAPI_BT_PREF_SBC:     return "SBC";
        default:                      return "Auto";
    }
}

static const char *ldac_tier_name(uint32_t mode) {
    switch (mode) {
        case LWASAPI_LDAC_MODE_990:  return "990 kbps (HQ)";
        case LWASAPI_LDAC_MODE_660:  return "660 kbps (SQ)";
        case LWASAPI_LDAC_MODE_330:  return "330 kbps (MQ)";
        default:                     return "Adaptive ABR";
    }
}

static void format_bt_caps(uint32_t mask, char *buf, size_t len) {
    buf[0] = '\0';
    if (mask & LWASAPI_BT_CAP_LDAC)    snprintf(buf + strlen(buf), len - strlen(buf), "%sLDAC", buf[0] ? ", " : "");
    if (mask & LWASAPI_BT_CAP_APTX_HD) snprintf(buf + strlen(buf), len - strlen(buf), "%saptX HD", buf[0] ? ", " : "");
    if (mask & LWASAPI_BT_CAP_APTX)    snprintf(buf + strlen(buf), len - strlen(buf), "%saptX", buf[0] ? ", " : "");
    if (mask & LWASAPI_BT_CAP_AAC)     snprintf(buf + strlen(buf), len - strlen(buf), "%sAAC", buf[0] ? ", " : "");
    if (mask & LWASAPI_BT_CAP_SBC)     snprintf(buf + strlen(buf), len - strlen(buf), "%sSBC", buf[0] ? ", " : "");
    if (!buf[0]) snprintf(buf, len, "Probing / None");
}

static int cmd_status(void) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) {
        fprintf(stderr, "Error: Could not read status from lwasapi-audiodg on %s\n", get_socket_path());
        return 1;
    }

    double period_ms = (1000.0 * st.hw_period_frames) / (st.hw_sample_rate ? st.hw_sample_rate : 48000);
    double buffer_ms = (1000.0 * st.hw_buffer_frames) / (st.hw_sample_rate ? st.hw_sample_rate : 48000);

    static const char *dyn_names[] = {
        "Static Parametric EQ",
        "Dynamic EQ Compress (Downward Peak Control)",
        "Dynamic EQ Expand (Upward Transient Punch)"
    };
    static const char *shape_tags[] = { "PK", "LS", "HS" };
    static const char *band_labels[LWASAPI_EQ_BANDS] = {
        "31Hz", "62Hz", "125Hz", "250Hz", "500Hz", "1kHz", "2kHz", "4kHz", "8kHz", "16kHz"
    };

    printf("================================================================================\n");
    printf(" Salsa v6.7-beta — Linux WASAPI Studio Engine (Fixed/Split Clock + HW Analog)\n");
    printf("================================================================================\n");
    printf(" Render / Sink Endpoint : %s [%s]\n",
           st.hw_device, st.active_playback_id[0] ? st.active_playback_id : "auto");
    printf(" Capture / Mic Endpoint : %s [%s]\n",
           st.hw_capture_device[0] ? st.hw_capture_device : "No Capture Device",
           st.active_capture_id[0] ? st.active_capture_id : "none");
    printf(" Hardware Clock & Depth : %u Hz (Fixed Master) | %u Channels | %u-bit PCM\n",
           st.hw_sample_rate, st.hw_channels, st.hw_format_bits);
    if (st.hw_analog_info.reserved >= 8000U && st.hw_analog_info.reserved <= 384000U &&
        st.hw_analog_info.reserved != st.hw_sample_rate) {
        printf(" Capture / Mic Clock    : %u Hz (Independent Mic Clock)\n",
               st.hw_analog_info.reserved);
    }
    printf(" Quantum / Latency      : %u frames (%.2f ms period / %.2f ms buffer)\n",
           st.hw_period_frames, period_ms, buffer_ms);
    printf(" Native PA Socket       : %s ($XDG_RUNTIME_DIR/pulse/native | %u connected client%s)\n",
           st.pa_server_running ? "Active (PipeWire Bypassed)" : "Offline",
           st.pa_clients, st.pa_clients == 1 ? "" : "s");
    if (st.bt_active || st.bt_alias[0]) {
        char bt_caps[96];
        format_bt_caps(st.bt_codec_cfg.supported_codecs_mask, bt_caps, sizeof(bt_caps));
        printf(" Bluetooth A2DP Native  : %s | Codec: %s @ %u kbps (ABR:%s) | Delay: %u ms (%s)\n",
               st.bt_alias[0] ? st.bt_alias : "BlueZ Device",
               st.bt_codec[0] ? st.bt_codec : "A2DP",
               st.bt_codec_cfg.effective_kbps,
               st.bt_codec_cfg.wireless_abr_enabled ? "ON" : "OFF",
               st.bt_delay_ms,
               st.bt_active ? "Active Sink" : "Standby");
        printf("   BT Policy & HFP SCO  : Pref=%s | LDAC=%s | HFP Mic=%s%s%s%s\n",
               bt_pref_name(st.bt_codec_cfg.preferred_codec),
               ldac_tier_name(st.bt_codec_cfg.ldac_mode),
               st.bt_codec_cfg.hfp_sco_enabled ? (st.bt_codec_cfg.hfp_sco_active ? "ACTIVE" : "Enabled/Idle") : "OFF",
               st.bt_codec_cfg.bt_sco_codec[0] ? " (" : "",
               st.bt_codec_cfg.bt_sco_codec[0] ? st.bt_codec_cfg.bt_sco_codec : "",
               st.bt_codec_cfg.bt_sco_codec[0] ? ")" : "");
        printf("   BT Supported SEPs    : %s | %s\n",
               bt_caps,
               st.bt_codec_cfg.neg_reason[0] ? st.bt_codec_cfg.neg_reason : "Negotiation nominal");
    }
    printf(" Master Speaker Mixer   : Volume %u%% (%s) [Pure Native + BlueZ AVRCP]\n",
           st.desktop_volume_pct, st.desktop_muted ? "MUTED" : "Unmuted");
    printf(" Master Mic Controls    : Gain   %u%% (%s) [Direct Kernel /dev/snd/pcm Capture]\n",
           st.desktop_mic_volume_pct, st.desktop_mic_muted ? "MUTED" : "Unmuted");
    {
        static const char *amp_names[] = { "Auto (Preserve)", "Low (IEM)", "Med (64-150R)", "High (600R TPA6120A2)" };
        static const char *route_names[] = { "Auto (Preserve)", "Headphones", "Line/Speakers" };
        uint32_t at = (st.hw_analog_cfg.hp_amp_tier <= 3U) ? st.hw_analog_cfg.hp_amp_tier : 0U;
        uint32_t rt = (st.hw_analog_cfg.output_route <= 2U) ? st.hw_analog_cfg.output_route : 0U;
        printf(" HW Analog Stage        : %s | DAC Unity: %s (%s)\n",
               st.hw_analog_info.card_arch_name[0] ? st.hw_analog_info.card_arch_name : "Virtual/Unprobed",
               st.hw_analog_info.dac_unity_active ? "ACTIVE (Bit-Perfect PCM)" :
               (st.hw_analog_cfg.hw_dac_unity_vol ? "Standby/Software Gain" : "Disabled (Software)"),
               st.hw_analog_info.dac_ctl_name[0] ? st.hw_analog_info.dac_ctl_name : "none");
        printf("   Amp / Route / Rails  : Gain=%s | Route=%s | 0dB Rails=%s | SoftKnee=%+.1fdB | MicBoost=%+.1fdB%s\n",
               amp_names[at], route_names[rt],
               st.hw_analog_cfg.unlock_0db_rails ? "UNLOCKED" : "Preserve",
               st.hw_analog_cfg.analog_boost_db,
               st.hw_analog_cfg.mic_hw_boost_db,
               st.hw_analog_info.hid_box_connected ? " | Strix HID: Connected" : "");
        if (st.hw_analog_info.diag_note[0]) {
            printf("   HW Analog Telemetry  : %s\n", st.hw_analog_info.diag_note);
        }
    }
    printf(" DAW MIDI Hub & HW      : %s (MIDI Key Presses: %lu)\n",
           st.midi_device[0] ? st.midi_device : "LWASAPI-MIDI-Hub",
           (unsigned long)st.midi_events_routed);
    printf(" MMCSS Real-Time Thread : %s (Priority: %u)\n",
           st.realtime_priority ? "SCHED_FIFO Active" : "Standard CFS", st.realtime_priority);
    printf(" Engine Mode            : %s\n",
           st.exclusive_owner_id >= 0 ? "EXCLUSIVE MODE (Hardware Locked)" : "SHARED MODE (Multi-Stream APO Mixer)");
    printf(" Total Period Cycles    : %lu (Hardware XRUNs: %lu)\n",
           (unsigned long)st.total_cycles, (unsigned long)st.hw_xruns);
    printf("--------------------------------------------------------------------------------\n");
    printf(" Salsa 10-Band Parametric Dynamic EQ APO:\n");
    printf("   Status / Mode        : %s | %s\n",
           st.eq_cfg.enabled ? "ACTIVE" : "BYPASSED",
           dyn_names[st.eq_cfg.dynamic_mode <= 2 ? st.eq_cfg.dynamic_mode : 0]);
    printf("   Preset & Dynamics    : %s | Preamp: %+.1f dB | Thresh: %.1f dB | Ratio: %.1f:1 | Limiter: %s\n",
           st.eq_cfg.preset_name[0] ? st.eq_cfg.preset_name : "Custom",
           st.eq_cfg.preamp_db, st.eq_cfg.dyn_threshold_db, st.eq_cfg.dyn_ratio,
           st.eq_cfg.limiter_enabled ? "ON" : "OFF");
    printf("   Crossfeed (Bauer)    : %s (Level: %.0f%%, Cutoff: %u Hz)\n",
           st.param_eq.crossfeed_enabled ? "ON" : "OFF",
           st.param_eq.crossfeed_level * 100.0f,
           st.param_eq.crossfeed_cutoff_hz ? st.param_eq.crossfeed_cutoff_hz : 700);
    printf("   Parametric Bands     :");
    for (int b = 0; b < LWASAPI_EQ_BANDS; b++) {
        uint32_t sh = st.param_eq.band_type[b] <= 2 ? st.param_eq.band_type[b] : 0;
        float freq = st.param_eq.band_freq_hz[b] > 1.0f ? st.param_eq.band_freq_hz[b] : 0.0f;
        float q = st.eq_cfg.band_q[b] > 0.05f ? st.eq_cfg.band_q[b] : 1.414f;
        if (freq > 1.0f) {
            printf("\n     #%d [%s|%.0fHz|Q=%.2f]: %+.1fdB",
                   b, shape_tags[sh], freq, q, st.eq_cfg.band_gains_db[b]);
        } else {
            printf(" %s:%+.1fdB", band_labels[b], st.eq_cfg.band_gains_db[b]);
        }
    }
    printf("\n");
    printf("--------------------------------------------------------------------------------\n");
    printf(" Capture Microphone APO : %s | Peak In: %.2f -> Out: %.2f\n",
           st.mic_apo.enabled ? "ACTIVE" : "BYPASSED (Raw Scarlett/HW Passthrough)",
           st.mic_apo_peak_in, st.mic_apo_peak_out);
    printf("   HPF: %s (%.0fHz) | Gate: %s (%.0fdB) | RNNoise: %s (%s) | Comp: %s (%.0fdB %.1f:1) | Clarity: %+.1fdB | Sidetone: %s (%u%%)\n",
           st.mic_apo.hpf_enabled ? "ON" : "OFF", st.mic_apo.hpf_hz,
           st.mic_apo.gate_enabled ? "ON" : "OFF", st.mic_apo.gate_thresh_db,
           st.mic_apo.rnnoise_enabled ? "ON" : "OFF",
           st.mic_rnnoise_active ? "Neural" : (st.mic_apo.rnnoise_enabled ? "Spectral" : "Idle"),
           st.mic_apo.comp_enabled ? "ON" : "OFF", st.mic_apo.comp_thresh_db, st.mic_apo.comp_ratio,
           st.mic_apo.clarity_db,
           st.mic_apo.sidetone_enabled ? "ON" : "OFF", st.mic_apo.sidetone_pct);
    printf("--------------------------------------------------------------------------------\n");
    printf(" Active Audio & PulseAudio Client Sessions (%u):\n", st.num_sessions);
    if (st.num_sessions == 0) {
        printf("   (No active client streams)\n");
    } else {
        for (uint32_t i = 0; i < st.num_sessions; i++) {
            lwasapi_session_info_t *s = &st.sessions[i];
            printf("   [#%-5u | PID %-7u] [%s] %-24s | %-6s | %6u Hz %uch | Vol:%3u%%%s | %-6s%s%s\n",
                   s->session_id,
                   s->client_pid,
                   s->is_capture ? "CAPT" : "PLAY",
                   s->client_name,
                   s->share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE ? "EXCL" : "SHARED",
                   s->sample_rate,
                   s->channels,
                   s->volume_milli / 10,
                   s->muted ? " [MUTE]" : "",
                   s->active ? "ACTIVE" : "IDLE",
                   s->target_device[0] ? " -> " : "",
                   s->target_device[0] ? s->target_device : "");
        }
    }
    printf("================================================================================\n");
    return 0;
}

static int cmd_devices(void) {
    lwasapi_device_list_t dlist;
    memset(&dlist, 0, sizeof(dlist));

    bool live = false;
    int sock = connect_engine();
    if (sock >= 0) {
        lwasapi_ipc_req_t req;
        memset(&req, 0, sizeof(req));
        req.cmd = LWASAPI_CMD_LIST_DEVICES;
        send(sock, &req, sizeof(req), MSG_NOSIGNAL);
        if (recv(sock, &dlist, sizeof(dlist), MSG_WAITALL) == (ssize_t)sizeof(dlist)) {
            live = true;
        }
        close(sock);
    }
    if (!live) {
        lwasapi_bluez_enumerate_devices(NULL, NULL, NULL, &dlist);
    }

    printf("Salsa Native Playback Endpoints (use 'lwasapi-ctl set-sink <id>'):\n");
    uint32_t pb_cnt = 0;
    for (uint32_t i = 0; i < dlist.num_devices; i++) {
        const lwasapi_device_entry_t *e = &dlist.devices[i];
        if (e->is_capture) continue;
        printf("  %-24s | %-10s | %s%s\n",
               e->id, e->driver, e->name,
               e->is_active ? "  [ACTIVE]" : "");
        pb_cnt++;
    }
    if (pb_cnt == 0) printf("  (No playback hardware found - WaveRT Virtual Clock available)\n");

    printf("\nSalsa Native Capture / Microphone Endpoints (use 'lwasapi-ctl set-source <id>'):\n");
    uint32_t cap_cnt = 0;
    for (uint32_t i = 0; i < dlist.num_devices; i++) {
        const lwasapi_device_entry_t *e = &dlist.devices[i];
        if (!e->is_capture) continue;
        printf("  %-24s | %-10s | %s%s\n",
               e->id, e->driver, e->name,
               e->is_active ? "  [ACTIVE]" : "");
        cap_cnt++;
    }
    if (cap_cnt == 0) printf("  (No kernel ALSA capture devices detected)\n");

    printf("\nAvailable Hardware & Sequencer MIDI Endpoints for DAWs:\n");
    for (int c = 0; c < 16; c++) {
        char mpath[64];
        snprintf(mpath, sizeof(mpath), "/dev/snd/midiC%dD0", c);
        if (access(mpath, R_OK) == 0) {
            char id_path[64], card_id[64] = "MIDI";
            snprintf(id_path, sizeof(id_path), "/proc/asound/card%d/id", c);
            FILE *idf = fopen(id_path, "r");
            if (idf) {
                if (fgets(card_id, sizeof(card_id), idf)) {
                    card_id[strcspn(card_id, "\r\n")] = '\0';
                }
                fclose(idf);
            }
            printf("  %-18s ->  Card %d Hardware RawMIDI (%s)\n", mpath, c, card_id);
        }
    }
    if (access("/dev/snd/seq", R_OK | W_OK) == 0) {
        printf("  /dev/snd/seq       ->  ALSA Sequencer ('LWASAPI-MIDI-Hub' + Hardware MIDI Ports)\n");
    }
    return 0;
}

static int cmd_set_device(bool is_capture, const char *dev_id) {
    int sock = connect_engine();
    if (sock < 0) {
        fprintf(stderr, "Error: Could not connect to lwasapi-audiodg on %s\n", get_socket_path());
        return 1;
    }

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_DEVICES;
    if (is_capture) snprintf(req.target_capture_device, sizeof(req.target_capture_device), "%s", dev_id);
    else snprintf(req.target_device, sizeof(req.target_device), "%s", dev_id);
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    ssize_t n = recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);

    if (n != (ssize_t)sizeof(resp) || resp.hr != S_OK) {
        fprintf(stderr, "Error: Failed to switch %s device to '%s'\n",
                is_capture ? "capture" : "playback", dev_id);
        return 1;
    }
    printf("Switched Salsa active %s endpoint to: %s\n",
           is_capture ? "capture" : "playback", dev_id);
    return 0;
}

static int cmd_bt_reset(const char *mac_or_empty) {
    char target_hint[64] = {0};
    if (mac_or_empty && mac_or_empty[0]) {
        snprintf(target_hint, sizeof(target_hint), "%s", mac_or_empty);
    } else {
        int sock = connect_engine();
        if (sock >= 0) {
            lwasapi_ipc_req_t req;
            memset(&req, 0, sizeof(req));
            req.cmd = LWASAPI_CMD_GET_STATUS;
            send(sock, &req, sizeof(req), MSG_NOSIGNAL);
            lwasapi_engine_status_t st;
            ssize_t n = recv(sock, &st, sizeof(st), 0);
            close(sock);
            if (n == (ssize_t)sizeof(st) && strncasecmp(st.active_playback_id, "bluez:", 6) == 0) {
                snprintf(target_hint, sizeof(target_hint), "%s", st.active_playback_id);
            }
        }
    }

    char msg[256] = {0};
    int rc = lwasapi_bluez_reset_device(target_hint, msg, sizeof(msg));
    if (rc == 0) {
        printf("%s\n", msg[0] ? msg : "Bluetooth audio reset completed.");
        return 0;
    }
    fprintf(stderr, "Error: %s\n", msg[0] ? msg : "Bluetooth reset failed.");
    return (rc == -2) ? 2 : 1;
}

static int cmd_volume(uint32_t session_id, float vol) {
    if (vol > 2.0f) vol = vol / 100.0f; /* Allow either 0.0-1.0 or 0-100 */
    int sock = connect_engine();
    if (sock < 0) return 1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_VOLUME;
    req.target_session_id = session_id;
    req.volume_milli = (uint32_t)(vol * 1000.0f);
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    printf("Session #%u volume set to %.0f%% (HRESULT=0x%08x)\n", session_id, vol * 100.0f, resp.hr);
    return resp.hr == S_OK ? 0 : 1;
}

static int cmd_session_control(uint32_t session_id, int32_t vol_pct, int32_t mute_mode, const char *dev_id) {
    int sock = connect_engine();
    if (sock < 0) return 1;

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
    if (dev_id && dev_id[0]) {
        snprintf(req.target_device, sizeof(req.target_device), "%s", dev_id);
    }
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    ssize_t n = recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);
    HRESULT hr = (n == (ssize_t)sizeof(resp)) ? resp.hr : E_FAIL;
    printf("Session #%u updated (vol=%d%% mute=%d dev=%s) -> HRESULT=0x%08x\n",
           session_id, vol_pct, mute_mode, (dev_id && dev_id[0]) ? dev_id : "-", (unsigned)hr);
    return (hr == S_OK) ? 0 : 1;
}

static int cmd_master_volume(int pct, int mute_mode) {
    int sock = connect_engine();
    if (sock < 0) return 1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_VOLUME;
    req.target_session_id = 0xFFFFFFFFU;
    req.volume_milli = (pct >= 0) ? (uint32_t)(pct * 10) : 0xFFFFFFFFU;
    req.share_mode = (uint32_t)(mute_mode + 1);
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    printf("Desktop Master Audio Controls updated (HRESULT=0x%08x)\n", resp.hr);
    return resp.hr == S_OK ? 0 : 1;
}

static int cmd_mic_volume(int pct, int mute_mode) {
    int sock = connect_engine();
    if (sock < 0) return 1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_VOLUME;
    req.target_session_id = 0xFFFFFFFEU;
    req.volume_milli = (pct >= 0) ? (uint32_t)(pct * 10) : 0xFFFFFFFFU;
    req.share_mode = (uint32_t)(mute_mode + 1);
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    printf("Desktop Microphone Controls updated (HRESULT=0x%08x)\n", resp.hr);
    return resp.hr == S_OK ? 0 : 1;
}

static int cmd_midi_send(uint8_t note, uint8_t vel) {
    int sock = connect_engine();
    if (sock < 0) return 1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_INJECT_MIDI;
    snprintf(req.client_name, sizeof(req.client_name), "lwasapi-ctl");
    req.midi_ev.size = 3;
    req.midi_ev.data[0] = 0x90; /* Note On, Channel 1 */
    req.midi_ev.data[1] = note & 0x7F;
    req.midi_ev.data[2] = vel & 0x7F;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    printf("Injected MIDI Note-On (status=0x90, note=%u, velocity=%u) -> HRESULT=0x%08x\n",
           note, vel, resp.hr);
    return resp.hr == S_OK ? 0 : 1;
}

static int cmd_set_rate(uint32_t rate, uint32_t period, uint32_t bits) {
    int sock = connect_engine();
    if (sock < 0) return 1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_ENGINE_CONFIG;
    req.sample_rate = rate;
    req.requested_period_frames = period;
    req.format = (lwasapi_sample_format_t)bits;
    req.stream_flags = 0U;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);

    uint32_t actual_bits = bits;
    if (resp.hr == S_OK) {
        int s2 = connect_engine();
        if (s2 >= 0) {
            lwasapi_ipc_req_t greq;
            memset(&greq, 0, sizeof(greq));
            greq.cmd = LWASAPI_CMD_GET_STATUS;
            send(s2, &greq, sizeof(greq), MSG_NOSIGNAL);
            lwasapi_engine_status_t st;
            memset(&st, 0, sizeof(st));
            if (recv(s2, &st, sizeof(st), MSG_WAITALL) == (ssize_t)sizeof(st)) {
                actual_bits = st.hw_format_bits;
            }
            close(s2);
        }
    }
    uint32_t disp_bits = actual_bits ? actual_bits : (bits ? bits : 32);
    bool rate_locked = (rate && resp.actual_sample_rate && resp.actual_sample_rate != rate);
    bool bits_locked = (bits && actual_bits && actual_bits != bits);
    bool per_locked  = (period && resp.actual_period_frames && resp.actual_period_frames != period);
    if (rate_locked || bits_locked || per_locked) {
        printf("Salsa Engine Clock -> %u Hz | Period: %u frames | %u-bit (HW Locked; requested %u Hz / %u frames / %u-bit) (HRESULT=0x%08x)\n",
               resp.actual_sample_rate, resp.actual_period_frames, disp_bits,
               rate ? rate : resp.actual_sample_rate,
               period ? period : resp.actual_period_frames,
               bits ? bits : disp_bits, resp.hr);
    } else {
        printf("Salsa Engine Clock -> %u Hz | Period: %u frames | %u-bit (HRESULT=0x%08x)\n",
               resp.actual_sample_rate, resp.actual_period_frames, disp_bits, resp.hr);
    }
    return resp.hr == S_OK ? 0 : 1;
}

static int cmd_set_cap_rate(uint32_t rate) {
    int sock = connect_engine();
    if (sock < 0) return 1;

    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_ENGINE_CONFIG;
    req.sample_rate = rate;
    req.stream_flags = LWASAPI_CFG_FLAG_CAPTURE_RATE;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);

    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    recv(sock, &resp, sizeof(resp), MSG_WAITALL);
    close(sock);

    printf("Salsa Capture / Mic Clock -> %u Hz (%s) (HRESULT=0x%08x)\n",
           resp.actual_sample_rate ? resp.actual_sample_rate : rate,
           rate == 0U ? "Auto (Follow Master Clock)" : "Fixed Independent Mic Clock",
           resp.hr);
    return resp.hr == S_OK ? 0 : 1;
}

static int get_current_eq(lwasapi_eq_config_t *out_eq) {
    int sock = connect_engine();
    if (sock < 0) return -1;
    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_GET_STATUS;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);
    lwasapi_engine_status_t st;
    ssize_t n = recv(sock, &st, sizeof(st), MSG_WAITALL);
    close(sock);
    if (n != (ssize_t)sizeof(st)) return -1;
    *out_eq = st.eq_cfg;
    return 0;
}

static int send_eq(const lwasapi_eq_config_t *eq) {
    int sock = connect_engine();
    if (sock < 0) return 1;
    lwasapi_ipc_req_t req;
    memset(&req, 0, sizeof(req));
    req.cmd = LWASAPI_CMD_SET_EQ;
    req.eq_cfg = *eq;
    send(sock, &req, sizeof(req), MSG_NOSIGNAL);
    lwasapi_ipc_resp_t resp;
    recv(sock, &resp, sizeof(resp), 0);
    close(sock);
    return resp.hr == S_OK ? 0 : 1;
}

static int cmd_eq_preset(const char *preset) {
    lwasapi_eq_config_t eq;
    if (get_current_eq(&eq) < 0) return 1;

    eq.enabled = 1;
    eq.limiter_enabled = 1;
    for (int i = 0; i < LWASAPI_EQ_BANDS; i++) eq.band_q[i] = 1.414f;

    if (strcasecmp(preset, "flat") == 0) {
        snprintf(eq.preset_name, sizeof(eq.preset_name), "Flat / Studio Reference");
        eq.preamp_db = 0.0f;
        eq.dynamic_mode = 0;
        float g[10] = { 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 };
        memcpy(eq.band_gains_db, g, sizeof(g));
    } else if (strcasecmp(preset, "bose") == 0) {
        snprintf(eq.preset_name, sizeof(eq.preset_name), "Bose QC Ultra 2 Audiophile");
        eq.preamp_db = -1.5f;
        eq.dynamic_mode = 0;
        eq.dyn_threshold_db = -16.0f;
        eq.dyn_ratio = 2.0f;
        float g[10] = { 1.0f, 0.5f, -1.5f, -0.5f, 0.5f, 0.5f, 0.5f, 0.0f, -1.5f, 0.5f };
        memcpy(eq.band_gains_db, g, sizeof(g));
    } else if (strcasecmp(preset, "bass") == 0) {
        snprintf(eq.preset_name, sizeof(eq.preset_name), "Dynamic Bass & Sub Control");
        eq.preamp_db = -3.0f;
        eq.dynamic_mode = 1;
        eq.dyn_threshold_db = -15.0f;
        eq.dyn_ratio = 3.2f;
        float g[10] = { 6.5f, 5.5f, 3.0f, 0.5f, 0.0f, 0.0f, 1.0f, 1.5f, 2.0f, 2.0f };
        memcpy(eq.band_gains_db, g, sizeof(g));
    } else if (strcasecmp(preset, "hires") == 0) {
        snprintf(eq.preset_name, sizeof(eq.preset_name), "192kHz Hi-Res Mastering");
        eq.preamp_db = -1.0f;
        eq.dynamic_mode = 2;
        eq.dyn_threshold_db = -20.0f;
        eq.dyn_ratio = 1.8f;
        float g[10] = { 1.5f, 1.0f, 0.0f, -0.5f, 0.0f, 0.5f, 1.5f, 2.5f, 3.5f, 4.5f };
        memcpy(eq.band_gains_db, g, sizeof(g));
    } else if (strcasecmp(preset, "gaming") == 0) {
        snprintf(eq.preset_name, sizeof(eq.preset_name), "FPS Gaming Footstep Radar");
        eq.preamp_db = -2.0f;
        eq.dynamic_mode = 1;
        eq.dyn_threshold_db = -22.0f;
        eq.dyn_ratio = 3.5f;
        float g[10] = { -3.0f, -2.0f, 0.0f, 1.0f, 2.0f, 4.0f, 5.5f, 5.0f, 2.5f, 1.0f };
        memcpy(eq.band_gains_db, g, sizeof(g));
    } else if (strcasecmp(preset, "vocal") == 0) {
        snprintf(eq.preset_name, sizeof(eq.preset_name), "Vocal & Podcast Clarity");
        eq.preamp_db = -1.0f;
        eq.dynamic_mode = 1;
        eq.dyn_threshold_db = -19.0f;
        eq.dyn_ratio = 2.8f;
        float g[10] = { -4.0f, -2.5f, -1.0f, 1.5f, 3.0f, 4.0f, 3.5f, 2.5f, 1.5f, 0.5f };
        memcpy(eq.band_gains_db, g, sizeof(g));
    } else if (strcasecmp(preset, "loudness") == 0) {
        snprintf(eq.preset_name, sizeof(eq.preset_name), "Late-Night Dynamic Loudness");
        eq.preamp_db = -2.0f;
        eq.dynamic_mode = 1;
        eq.dyn_threshold_db = -24.0f;
        eq.dyn_ratio = 4.0f;
        float g[10] = { 4.5f, 3.5f, 1.5f, 0.0f, -0.5f, 0.5f, 2.0f, 3.0f, 4.0f, 4.5f };
        memcpy(eq.band_gains_db, g, sizeof(g));
    } else {
        fprintf(stderr, "Unknown preset '%s'. Use: flat, bose, bass, hires, gaming, vocal, loudness\n", preset);
        return 1;
    }

    int rc = send_eq(&eq);
    printf("Applied Salsa Dynamic EQ preset: '%s' (rc=%d)\n", eq.preset_name, rc);
    return rc;
}

static int cmd_eq_band(int band, float gain_db) {
    if (band < 0 || band >= LWASAPI_EQ_BANDS) {
        fprintf(stderr, "Band index must be 0..9\n");
        return 1;
    }
    lwasapi_eq_config_t eq;
    if (get_current_eq(&eq) < 0) return 1;
    eq.enabled = 1;
    eq.band_gains_db[band] = gain_db;
    snprintf(eq.preset_name, sizeof(eq.preset_name), "Custom Salsa Curve");
    int rc = send_eq(&eq);
    printf("Updated Salsa EQ Band #%d to %+.1f dB\n", band, gain_db);
    return rc;
}

static int cmd_eq_mode(const char *mode_str, float thresh, float ratio) {
    lwasapi_eq_config_t eq;
    if (get_current_eq(&eq) < 0) return 1;

    if (strcasecmp(mode_str, "off") == 0 || strcasecmp(mode_str, "bypass") == 0) {
        eq.enabled = 0;
    } else if (strcasecmp(mode_str, "static") == 0) {
        eq.enabled = 1;
        eq.dynamic_mode = 0;
    } else if (strcasecmp(mode_str, "compress") == 0) {
        eq.enabled = 1;
        eq.dynamic_mode = 1;
    } else if (strcasecmp(mode_str, "expand") == 0) {
        eq.enabled = 1;
        eq.dynamic_mode = 2;
    } else {
        fprintf(stderr, "Unknown mode '%s'. Use: off, static, compress, expand\n", mode_str);
        return 1;
    }

    if (thresh < 0.0f) eq.dyn_threshold_db = thresh;
    if (ratio >= 1.0f) eq.dyn_ratio = ratio;

    int rc = send_eq(&eq);
    printf("Updated Salsa Dynamic EQ Mode -> %s (Threshold: %.1f dB, Ratio: %.1f:1)\n",
           mode_str, eq.dyn_threshold_db, eq.dyn_ratio);
    return rc;
}

static int cmd_bt_codecs(void) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;
    char caps[96];
    format_bt_caps(st.bt_codec_cfg.supported_codecs_mask, caps, sizeof(caps));
    printf("Bluetooth Device : %s\n", st.bt_alias[0] ? st.bt_alias : "(None)");
    printf("Active Codec     : %s @ %u kbps (ABR: %s)\n",
           st.bt_codec[0] ? st.bt_codec : "None",
           st.bt_codec_cfg.effective_kbps,
           st.bt_codec_cfg.wireless_abr_enabled ? "ON" : "OFF");
    printf("Preferred Codec  : %s\n", bt_pref_name(st.bt_codec_cfg.preferred_codec));
    printf("LDAC Tier        : %s\n", ldac_tier_name(st.bt_codec_cfg.ldac_mode));
    printf("Supported SEPs   : %s\n", caps);
    printf("HFP SCO Mic      : %s%s%s%s\n",
           st.bt_codec_cfg.hfp_sco_enabled ? (st.bt_codec_cfg.hfp_sco_active ? "ACTIVE" : "Enabled") : "OFF",
           st.bt_codec_cfg.bt_sco_codec[0] ? " (" : "",
           st.bt_codec_cfg.bt_sco_codec[0] ? st.bt_codec_cfg.bt_sco_codec : "",
           st.bt_codec_cfg.bt_sco_codec[0] ? ")" : "");
    printf("Diagnostics      : %s\n",
           st.bt_codec_cfg.neg_reason[0] ? st.bt_codec_cfg.neg_reason : "Nominal");
    return 0;
}

static int cmd_set_codec(const char *codec_str, const char *tier_str) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;
    lwasapi_bt_codec_cfg_t cfg = st.bt_codec_cfg;

    if (strcasecmp(codec_str, "auto") == 0) cfg.preferred_codec = LWASAPI_BT_PREF_AUTO;
    else if (strcasecmp(codec_str, "ldac") == 0) cfg.preferred_codec = LWASAPI_BT_PREF_LDAC;
    else if (strcasecmp(codec_str, "aptxhd") == 0 || strcasecmp(codec_str, "aptx_hd") == 0)
        cfg.preferred_codec = LWASAPI_BT_PREF_APTX_HD;
    else if (strcasecmp(codec_str, "aptx") == 0) cfg.preferred_codec = LWASAPI_BT_PREF_APTX;
    else if (strcasecmp(codec_str, "aac") == 0) cfg.preferred_codec = LWASAPI_BT_PREF_AAC;
    else if (strcasecmp(codec_str, "sbc") == 0) cfg.preferred_codec = LWASAPI_BT_PREF_SBC;
    else {
        fprintf(stderr, "Unknown BlueZ codec '%s'. Use: auto|ldac|aptxhd|aptx|aac|sbc\n", codec_str);
        return 1;
    }

    if (tier_str && tier_str[0]) {
        if (strcasecmp(tier_str, "abr") == 0 || strcasecmp(tier_str, "adaptive") == 0) {
            cfg.ldac_mode = LWASAPI_LDAC_MODE_ADAPTIVE;
            cfg.wireless_abr_enabled = 1;
        } else if (strcmp(tier_str, "990") == 0) {
            cfg.ldac_mode = LWASAPI_LDAC_MODE_990;
        } else if (strcmp(tier_str, "660") == 0) {
            cfg.ldac_mode = LWASAPI_LDAC_MODE_660;
        } else if (strcmp(tier_str, "330") == 0) {
            cfg.ldac_mode = LWASAPI_LDAC_MODE_330;
        }
    }

    lwasapi_ipc_resp_t resp;
    int rc = send_payload_cmd(LWASAPI_CMD_SET_BT_CODEC, &cfg, (uint32_t)sizeof(cfg), &resp);
    printf("BlueZ Codec Switcher -> %s (LDAC: %s, ABR: %s, HRESULT=0x%08x)\n",
           bt_pref_name(cfg.preferred_codec), ldac_tier_name(cfg.ldac_mode),
           cfg.wireless_abr_enabled ? "ON" : "OFF", resp.hr);
    return rc;
}

static int cmd_bt_sco(const char *mode_str) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;
    lwasapi_bt_codec_cfg_t cfg = st.bt_codec_cfg;
    cfg.hfp_sco_enabled = (strcasecmp(mode_str, "on") == 0 || strcmp(mode_str, "1") == 0) ? 1u : 0u;
    lwasapi_ipc_resp_t resp;
    int rc = send_payload_cmd(LWASAPI_CMD_SET_BT_CODEC, &cfg, (uint32_t)sizeof(cfg), &resp);
    printf("Bluetooth HFP BTPROTO_SCO Headset Mic -> %s (HRESULT=0x%08x)\n",
           cfg.hfp_sco_enabled ? "ENABLED" : "DISABLED", resp.hr);
    return rc;
}

static int cmd_hw_analog(int argc, char **argv) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;
    lwasapi_hw_analog_cfg_t cfg = st.hw_analog_cfg;

    if (argc == 2 || (argc >= 3 && strcasecmp(argv[2], "status") == 0)) {
        printf("Card Architecture : %s (PB Card=%d, Cap Card=%d)\n",
               st.hw_analog_info.card_arch_name[0] ? st.hw_analog_info.card_arch_name : "Virtual/None",
               st.hw_analog_info.active_pb_card, st.hw_analog_info.active_cap_card);
        printf("DAC Unity Mode    : %s (Active=%s, Elem='%s')\n",
               cfg.hw_dac_unity_vol ? "ON" : "OFF",
               st.hw_analog_info.dac_unity_active ? "YES" : "NO",
               st.hw_analog_info.dac_ctl_name[0] ? st.hw_analog_info.dac_ctl_name : "none");
        printf("HP Amp / Route    : tier=%u | route=%u | unlock_0db=%u (Elem='%s')\n",
               cfg.hp_amp_tier, cfg.output_route, cfg.unlock_0db_rails,
               st.hw_analog_info.amp_ctl_name[0] ? st.hw_analog_info.amp_ctl_name : "none");
        printf("Analog Boost      : %+.1f dB | Mic HW Boost: %+.1f dB (Elem='%s') | Strix HID=%u\n",
               cfg.analog_boost_db, cfg.mic_hw_boost_db,
               st.hw_analog_info.cap_ctl_name[0] ? st.hw_analog_info.cap_ctl_name : "none",
               st.hw_analog_info.hid_box_connected);
        if (st.hw_analog_info.diag_note[0]) {
            printf("Telemetry Note    : %s\n", st.hw_analog_info.diag_note);
        }
        return 0;
    }

    if (strcasecmp(argv[2], "reset") == 0) {
        cfg.hw_dac_unity_vol = 1U;
        cfg.hp_amp_tier      = LWASAPI_HW_AMP_AUTO;
        cfg.output_route     = LWASAPI_HW_ROUTE_AUTO;
        cfg.unlock_0db_rails = 0U;
        cfg.analog_boost_db  = 0.0f;
        cfg.mic_hw_boost_db  = 0.0f;
    } else {
        for (int i = 2; i + 1 < argc; i += 2) {
            const char *k = argv[i];
            const char *v = argv[i + 1];
            if (strcasecmp(k, "dac") == 0) {
                cfg.hw_dac_unity_vol = (strcasecmp(v, "on") == 0 || strcmp(v, "1") == 0) ? 1U : 0U;
            } else if (strcasecmp(k, "amp") == 0) {
                if (strcasecmp(v, "auto") == 0 || strcmp(v, "0") == 0) cfg.hp_amp_tier = LWASAPI_HW_AMP_AUTO;
                else if (strcasecmp(v, "low") == 0 || strcmp(v, "1") == 0) cfg.hp_amp_tier = LWASAPI_HW_AMP_LOW;
                else if (strcasecmp(v, "med") == 0 || strcasecmp(v, "medium") == 0 || strcmp(v, "2") == 0)
                    cfg.hp_amp_tier = LWASAPI_HW_AMP_MED;
                else if (strcasecmp(v, "high") == 0 || strcasecmp(v, "600") == 0 || strcmp(v, "3") == 0)
                    cfg.hp_amp_tier = LWASAPI_HW_AMP_HIGH;
                else {
                    fprintf(stderr, "Unknown amp tier '%s'. Use: auto|low|med|high\n", v);
                    return 1;
                }
            } else if (strcasecmp(k, "route") == 0) {
                if (strcasecmp(v, "auto") == 0 || strcmp(v, "0") == 0) cfg.output_route = LWASAPI_HW_ROUTE_AUTO;
                else if (strcasecmp(v, "hp") == 0 || strcasecmp(v, "headphones") == 0 || strcmp(v, "1") == 0)
                    cfg.output_route = LWASAPI_HW_ROUTE_HP;
                else if (strcasecmp(v, "spk") == 0 || strcasecmp(v, "speakers") == 0 ||
                         strcasecmp(v, "line") == 0 || strcmp(v, "2") == 0)
                    cfg.output_route = LWASAPI_HW_ROUTE_SPK;
                else {
                    fprintf(stderr, "Unknown route '%s'. Use: auto|hp|spk\n", v);
                    return 1;
                }
            } else if (strcasecmp(k, "rails") == 0) {
                cfg.unlock_0db_rails = (strcasecmp(v, "on") == 0 || strcmp(v, "1") == 0) ? 1U : 0U;
            } else if (strcasecmp(k, "boost") == 0) {
                float b = strtof(v, NULL);
                if (b < 0.0f) b = 0.0f;
                if (b > 12.0f) b = 12.0f;
                cfg.analog_boost_db = b;
            } else if (strcasecmp(k, "mic-boost") == 0 || strcasecmp(k, "micboost") == 0) {
                float mb = strtof(v, NULL);
                if (mb < 0.0f) mb = 0.0f;
                if (mb > 24.0f) mb = 24.0f;
                cfg.mic_hw_boost_db = mb;
            } else {
                fprintf(stderr, "Unknown hw-analog option '%s'. Use: dac|amp|route|rails|boost|mic-boost|reset\n", k);
                return 1;
            }
        }
    }

    lwasapi_ipc_resp_t resp;
    int rc = send_payload_cmd(LWASAPI_CMD_SET_HW_ANALOG, &cfg, (uint32_t)sizeof(cfg), &resp);
    printf("HW Analog Architecture -> DAC=%s Amp=%u Route=%u 0dB-Rails=%s SoftKnee=+%.1fdB MicBoost=+%.1fdB (HRESULT=0x%08x)\n",
           cfg.hw_dac_unity_vol ? "ON" : "OFF",
           cfg.hp_amp_tier, cfg.output_route,
           cfg.unlock_0db_rails ? "UNLOCK" : "Preserve",
           cfg.analog_boost_db, cfg.mic_hw_boost_db, resp.hr);
    return rc;
}

static int cmd_param_eq(int band, float freq, float q, const char *shape, float gain) {
    if (band < 0 || band >= LWASAPI_EQ_BANDS) {
        fprintf(stderr, "Band index must be 0..%d\n", LWASAPI_EQ_BANDS - 1);
        return 1;
    }
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;

    lwasapi_param_eq_payload_t pl;
    pl.eq = st.eq_cfg;
    pl.ext = st.param_eq;
    pl.eq.enabled = 1;
    pl.eq.band_gains_db[band] = gain;
    pl.eq.band_q[band] = (q >= 0.2f && q <= 10.0f) ? q : 1.414f;
    pl.ext.band_freq_hz[band] = (freq >= 15.0f && freq <= 22000.0f) ? freq : 1000.0f;
    if (strcasecmp(shape, "lowshelf") == 0 || strcasecmp(shape, "ls") == 0)
        pl.ext.band_type[band] = LWASAPI_FILTER_LOW_SHELF;
    else if (strcasecmp(shape, "highshelf") == 0 || strcasecmp(shape, "hs") == 0)
        pl.ext.band_type[band] = LWASAPI_FILTER_HIGH_SHELF;
    else
        pl.ext.band_type[band] = LWASAPI_FILTER_PEAK;
    snprintf(pl.eq.preset_name, sizeof(pl.eq.preset_name), "Custom Parametric");

    lwasapi_ipc_resp_t resp;
    int rc = send_payload_cmd(LWASAPI_CMD_SET_PARAM_EQ, &pl, (uint32_t)sizeof(pl), &resp);
    printf("Parametric Band #%d -> %.1f Hz | Q=%.2f | %s | %+.1f dB (HRESULT=0x%08x)\n",
           band, pl.ext.band_freq_hz[band], pl.eq.band_q[band], shape, gain, resp.hr);
    return rc;
}

static int cmd_crossfeed(const char *mode_str, float level_pct, uint32_t cutoff_hz) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;
    lwasapi_param_eq_payload_t pl;
    pl.eq = st.eq_cfg;
    pl.ext = st.param_eq;
    pl.ext.crossfeed_enabled = (strcasecmp(mode_str, "on") == 0 || strcmp(mode_str, "1") == 0) ? 1u : 0u;
    if (level_pct >= 0.0f && level_pct <= 100.0f) pl.ext.crossfeed_level = level_pct / 100.0f;
    if (cutoff_hz >= 200 && cutoff_hz <= 2000) pl.ext.crossfeed_cutoff_hz = cutoff_hz;

    lwasapi_ipc_resp_t resp;
    int rc = send_payload_cmd(LWASAPI_CMD_SET_PARAM_EQ, &pl, (uint32_t)sizeof(pl), &resp);
    printf("Bauer Headphone Crossfeed -> %s (Level: %.0f%%, Cutoff: %u Hz, HRESULT=0x%08x)\n",
           pl.ext.crossfeed_enabled ? "ON" : "OFF",
           pl.ext.crossfeed_level * 100.0f, pl.ext.crossfeed_cutoff_hz, resp.hr);
    return rc;
}

static int cmd_mic_apo(const char *mode_str, int rnn_arg, float gate_db, int side_pct) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;
    lwasapi_mic_apo_cfg_t cfg = st.mic_apo;
    cfg.enabled = (strcasecmp(mode_str, "on") == 0 || strcmp(mode_str, "1") == 0) ? 1u : 0u;
    if (rnn_arg >= 0) cfg.rnnoise_enabled = (uint32_t)(rnn_arg ? 1 : 0);
    if (gate_db <= 0.0f && gate_db >= -80.0f) {
        cfg.gate_enabled = 1;
        cfg.gate_thresh_db = gate_db;
    }
    if (side_pct >= 0 && side_pct <= 100) {
        cfg.sidetone_enabled = (side_pct > 0) ? 1u : 0u;
        cfg.sidetone_pct = (uint32_t)side_pct;
    }

    lwasapi_ipc_resp_t resp;
    int rc = send_payload_cmd(LWASAPI_CMD_SET_MIC_APO, &cfg, (uint32_t)sizeof(cfg), &resp);
    printf("Capture Mic APO -> %s | RNNoise: %s | Gate: %.1f dB | Sidetone: %s (%u%%) (HRESULT=0x%08x)\n",
           cfg.enabled ? "ACTIVE" : "BYPASSED",
           cfg.rnnoise_enabled ? "ON" : "OFF",
           cfg.gate_thresh_db,
           cfg.sidetone_enabled ? "ON" : "OFF", cfg.sidetone_pct, resp.hr);
    return rc;
}

static int cmd_mic_preset(const char *preset) {
    lwasapi_engine_status_t st;
    if (get_full_status(&st) < 0) return 1;
    lwasapi_mic_apo_cfg_t cfg = st.mic_apo;
    if (strcasecmp(preset, "discord") == 0) {
        cfg.enabled = 1u;
        cfg.hpf_enabled = 1u;
        cfg.hpf_hz = 65.0f;
        cfg.gate_enabled = 1u;
        cfg.gate_thresh_db = -50.0f;
        cfg.rnnoise_enabled = 1u;
        cfg.comp_enabled = 1u;
        cfg.comp_thresh_db = -14.0f;
        cfg.comp_ratio = 2.0f;
        cfg.clarity_db = 1.5f;
    } else if (strcasecmp(preset, "broadcast") == 0) {
        cfg.enabled = 1u;
        cfg.hpf_enabled = 1u;
        cfg.hpf_hz = 75.0f;
        cfg.gate_enabled = 1u;
        cfg.gate_thresh_db = -45.0f;
        cfg.rnnoise_enabled = 1u;
        cfg.comp_enabled = 1u;
        cfg.comp_thresh_db = -15.0f;
        cfg.comp_ratio = 2.2f;
        cfg.clarity_db = 2.0f;
    } else if (strcasecmp(preset, "clean") == 0 || strcasecmp(preset, "natural") == 0) {
        cfg.enabled = 1u;
        cfg.hpf_enabled = 1u;
        cfg.hpf_hz = 65.0f;
        cfg.gate_enabled = 1u;
        cfg.gate_thresh_db = -52.0f;
        cfg.rnnoise_enabled = 0u;
        cfg.comp_enabled = 1u;
        cfg.comp_thresh_db = -16.0f;
        cfg.comp_ratio = 1.8f;
        cfg.clarity_db = 1.0f;
    } else if (strcasecmp(preset, "bypass") == 0 || strcasecmp(preset, "raw") == 0 || strcasecmp(preset, "off") == 0) {
        cfg.enabled = 0u;
    } else {
        fprintf(stderr, "Unknown mic preset '%s'. Use: discord, broadcast, clean, bypass\n", preset);
        return 1;
    }

    lwasapi_ipc_resp_t resp;
    int rc = send_payload_cmd(LWASAPI_CMD_SET_MIC_APO, &cfg, (uint32_t)sizeof(cfg), &resp);
    printf("Mic APO Preset '%s' -> %s | HPF %.0fHz | Gate %.0fdB | RNNoise %s | Clarity %+.1fdB (HRESULT=0x%08x)\n",
           preset, cfg.enabled ? "ACTIVE" : "BYPASSED",
           cfg.hpf_hz, cfg.gate_thresh_db,
           cfg.rnnoise_enabled ? "ON" : "OFF", cfg.clarity_db, resp.hr);
    return rc;
}

static void get_install_root(char *out_root, size_t out_len) {
    char exe[512] = {0};
    ssize_t n = readlink("/proc/self/exe", exe, sizeof(exe) - 1);
    if (n > 0) {
        exe[n] = '\0';
        char *slash = strrchr(exe, '/');
        if (slash) {
            *slash = '\0'; /* .../build */
            char *parent = strrchr(exe, '/');
            if (parent && strcmp(slash + 1, "lwasapi-ctl") == 0) {
                /* Check if exe is inside build/ */
                if (strlen(exe) > 6 && strcmp(exe + strlen(exe) - 6, "/build") == 0) {
                    exe[strlen(exe) - 6] = '\0';
                }
            }
        }
        snprintf(out_root, out_len, "%s", exe);
    } else {
        snprintf(out_root, out_len, ".");
    }
}

static void mkdir_p(const char *path) {
    char tmp[600];
    snprintf(tmp, sizeof(tmp), "%s", path);
    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') {
            *p = '\0';
            if (access(tmp, F_OK) != 0) (void)mkdir(tmp, 0755);
            *p = '/';
        }
    }
    if (access(tmp, F_OK) != 0) (void)mkdir(tmp, 0755);
}

static void move_if_exists(const char *src, const char *dst) {
    if (access(src, F_OK) == 0) {
        if (rename(src, dst) != 0) {}
    }
}

static bool unit_exists(const char *unit) {
    char cmd[256], line[256];
    snprintf(cmd, sizeof(cmd), "systemctl --user list-unit-files --full --no-legend '%s' 2>/dev/null", unit);
    FILE *fp = popen(cmd, "r");
    if (!fp) return false;
    bool found = false;
    if (fgets(line, sizeof(line), fp) != NULL) {
        size_t ulen = strlen(unit);
        if (strncmp(line, unit, ulen) == 0 && (line[ulen] == ' ' || line[ulen] == '\t')) {
            found = true;
        }
    }
    pclose(fp);
    return found;
}

static void stop_and_mask_unit(const char *unit) {
    char cmd[256];
    snprintf(cmd, sizeof(cmd), "systemctl --user stop '%s' >/dev/null 2>&1", unit);
    if (system(cmd) != 0) {}
    snprintf(cmd, sizeof(cmd), "systemctl --user mask '%s' >/dev/null 2>&1", unit);
    if (system(cmd) != 0) {}
}

static void unmask_and_enable_unit(const char *unit) {
    char cmd[256];
    snprintf(cmd, sizeof(cmd), "systemctl --user unmask '%s' >/dev/null 2>&1", unit);
    if (system(cmd) != 0) {}
    snprintf(cmd, sizeof(cmd), "systemctl --user enable --now '%s' >/dev/null 2>&1", unit);
    if (system(cmd) != 0) {}
}

static void record_masked_unit(const char *list_path, const char *unit) {
    FILE *rfp = fopen(list_path, "r");
    if (rfp) {
        char line[128];
        while (fgets(line, sizeof(line), rfp)) {
            line[strcspn(line, "\r\n \t")] = '\0';
            if (strcmp(line, unit) == 0) {
                fclose(rfp);
                return;
            }
        }
        fclose(rfp);
    }
    FILE *afp = fopen(list_path, "a");
    if (afp) {
        fprintf(afp, "%s\n", unit);
        fclose(afp);
    }
}

static int trigger_broker_sync_mode(void) {
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    struct timeval tv = { .tv_sec = 0, .tv_usec = 500000 };
    (void)setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    (void)setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));

    char dsock[108];
    lwasapi_debug_sock_path(dsock, sizeof(dsock), LWASAPI_DEBUG_SOCK_DEFAULT);

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", dsock);
    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return -1;
    }

    const char *cmd = "PW_SYNC_MODE\n";
    ssize_t wr = send(fd, cmd, strlen(cmd), MSG_NOSIGNAL);
    if (wr <= 0) {
        close(fd);
        return -1;
    }
    shutdown(fd, SHUT_WR);

    char rbuf[512];
    ssize_t n = recv(fd, rbuf, sizeof(rbuf) - 1, 0);
    close(fd);
    return (n > 0) ? 0 : -1;
}

static void run_quiet_argv(char *const argv[]) {
    pid_t pid = fork();
    if (pid == 0) {
        int devnull = open("/dev/null", O_RDWR);
        if (devnull >= 0) {
            dup2(devnull, STDOUT_FILENO);
            dup2(devnull, STDERR_FILENO);
            if (devnull > STDERR_FILENO) close(devnull);
        }
        execvp(argv[0], argv);
        _exit(127);
    } else if (pid > 0) {
        int status = 0;
        (void)waitpid(pid, &status, 0);
    }
}

static bool is_salsa_pa_dir(const char *seg, size_t len) {
    static const char s64[] = "/build/pa_reconnect";
    static const char s32[] = "/build/pa_reconnect32";
    size_t l64 = sizeof(s64) - 1;
    size_t l32 = sizeof(s32) - 1;
    if (len >= l64 && memcmp(seg + len - l64, s64, l64) == 0) return true;
    if (len >= l32 && memcmp(seg + len - l32, s32, l32) == 0) return true;
    return false;
}

static void build_filtered_ld_path(const char *prepend_dir, char *out, size_t out_sz) {
    if (!out || out_sz == 0) return;
    out[0] = '\0';
    size_t used = 0;

    if (prepend_dir && prepend_dir[0]) {
        int w = snprintf(out, out_sz, "%s", prepend_dir);
        if (w > 0 && (size_t)w < out_sz) used = (size_t)w;
        else return;
    }

    const char *cur = getenv("LD_LIBRARY_PATH");
    if (!cur || !cur[0]) return;

    const char *p = cur;
    while (*p) {
        const char *colon = strchr(p, ':');
        size_t len = colon ? (size_t)(colon - p) : strlen(p);
        if (len > 0 && !is_salsa_pa_dir(p, len)) {
            bool dup = prepend_dir && strlen(prepend_dir) == len && memcmp(p, prepend_dir, len) == 0;
            if (!dup) {
                size_t need = (used > 0 ? 1 : 0) + len + 1;
                if (used + need <= out_sz) {
                    if (used > 0) out[used++] = ':';
                    memcpy(out + used, p, len);
                    used += len;
                    out[used] = '\0';
                }
            }
        }
    if (!colon) break;
        p = colon + 1;
    }
}

static FILE *fopen_nofollow_write(const char *path, mode_t mode) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW | O_CLOEXEC, mode);
    if (fd < 0) return NULL;
    if (fchmod(fd, mode) != 0) {}
    FILE *fp = fdopen(fd, "w");
    if (!fp) close(fd);
    return fp;
}

static bool file_contains_str(const char *path, const char *needle) {
    int fd = open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (fd < 0) return false;
    FILE *fp = fdopen(fd, "r");
    if (!fp) { close(fd); return false; }
    char line[2048];
    bool found = false;
    while (fgets(line, sizeof(line), fp)) {
        if (strstr(line, needle)) { found = true; break; }
    }
    fclose(fp);
    return found;
}

static bool copy_file_nofollow(const char *src, const char *dst, mode_t mode) {
    int sfd = open(src, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (sfd < 0) return false;
    FILE *in = fdopen(sfd, "r");
    if (!in) { close(sfd); return false; }
    FILE *out = fopen_nofollow_write(dst, mode);
    if (!out) { fclose(in); return false; }
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), in)) > 0) {
        if (fwrite(buf, 1, n, out) != n) break;
    }
    fclose(out);
    fclose(in);
    return true;
}

static bool rewrite_steam_desktop_exec(const char *src_path, const char *dst_path,
                                       const char *wrap_cmd, bool enable, mode_t mode) {
    int sfd = open(src_path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (sfd < 0) return false;
    FILE *in = fdopen(sfd, "r");
    if (!in) { close(sfd); return false; }
    char tmp_path[640];
    snprintf(tmp_path, sizeof(tmp_path), "%s.tmp", dst_path);
    FILE *out = fopen_nofollow_write(tmp_path, mode);
    if (!out) { fclose(in); return false; }

    char wrap_prefix[640];
    snprintf(wrap_prefix, sizeof(wrap_prefix), "Exec=%s", wrap_cmd);
    size_t wrap_len = strlen(wrap_prefix);

    char line[2048];
    while (fgets(line, sizeof(line), in)) {
        if (enable) {
            if (strncmp(line, "Exec=/usr/bin/steam", 19) == 0 &&
                (line[19] == '\0' || line[19] == ' ' || line[19] == '\t' || line[19] == '\r' || line[19] == '\n')) {
                fprintf(out, "Exec=%s%s", wrap_cmd, line + 19);
                continue;
            }
            if (strncmp(line, "Exec=steam", 10) == 0 &&
                (line[10] == '\0' || line[10] == ' ' || line[10] == '\t' || line[10] == '\r' || line[10] == '\n')) {
                fprintf(out, "Exec=%s%s", wrap_cmd, line + 10);
                continue;
            }
        } else if (strncmp(line, wrap_prefix, wrap_len) == 0 &&
                   (line[wrap_len] == '\0' || line[wrap_len] == ' ' || line[wrap_len] == '\t' ||
                    line[wrap_len] == '\r' || line[wrap_len] == '\n')) {
            fprintf(out, "Exec=/usr/bin/steam%s", line + wrap_len);
            continue;
        }
        fputs(line, out);
    }
    fclose(out);
    fclose(in);
    if (rename(tmp_path, dst_path) != 0) {
        unlink(tmp_path);
        return false;
    }
    return true;
}

static void refresh_desktop_db(const char *apps_dir) {
    char *argv_db[] = { "update-desktop-database", (char *)apps_dir, NULL };
    run_quiet_argv(argv_db);
}

static void install_steam_wrapper(const char *home, const char *root) {
    char bin_dir[512], apps_dir[512], auto_dir[512], cfg_dir[512];
    char wrap_path[600], app_desk[600], app_bak[600], auto_desk[600], auto_bak[600];
    snprintf(bin_dir, sizeof(bin_dir), "%s/.local/bin", home);
    snprintf(apps_dir, sizeof(apps_dir), "%s/.local/share/applications", home);
    snprintf(auto_dir, sizeof(auto_dir), "%s/.config/autostart", home);
    snprintf(cfg_dir, sizeof(cfg_dir), "%s/.config/lwasapi", home);
    mkdir_p(bin_dir);
    mkdir_p(apps_dir);
    mkdir_p(cfg_dir);

    snprintf(wrap_path, sizeof(wrap_path), "%s/salsa-steam-launch", bin_dir);
    FILE *wfp = fopen_nofollow_write(wrap_path, 0700);
    if (!wfp) return;
    fprintf(wfp,
        "#!/bin/sh\n"
        "# Salsa PulseAudio Reconnect wrapper for Steam (propagates inside pressure-vessel)\n"
        "# Revert anytime with: lwasapi-ctl disable-auto\n"
        "RECON_SO=\"%s/build/libsalsa_pa_reconnect.so\"\n"
        "if [ -r \"$RECON_SO\" ]; then\n"
        "    case \":${LD_PRELOAD:-}:\" in\n"
        "        *\":$RECON_SO:\"*) ;;\n"
        "        *) export LD_PRELOAD=\"${RECON_SO}${LD_PRELOAD:+:$LD_PRELOAD}\" ;;\n"
        "    esac\n"
        "fi\n"
        "if [ -x /usr/bin/steam ]; then\n"
        "    exec /usr/bin/steam \"$@\"\n"
        "fi\n"
        "exec steam \"$@\"\n",
        root);
    fclose(wfp);

    snprintf(app_desk, sizeof(app_desk), "%s/steam.desktop", apps_dir);
    snprintf(app_bak, sizeof(app_bak), "%s/steam_app.desktop.bak", cfg_dir);
    const char *sys_desk = "/usr/share/applications/steam.desktop";
    if (access(app_desk, F_OK) == 0 && !file_contains_str(app_desk, "salsa-steam-launch") &&
        access(app_bak, F_OK) != 0) {
        copy_file_nofollow(app_desk, app_bak, 0600);
    }
    const char *app_src = (access(app_bak, R_OK) == 0) ? app_bak : sys_desk;
    if (access(app_src, R_OK) == 0) {
        if (rewrite_steam_desktop_exec(app_src, app_desk, wrap_path, true, 0644)) {
            refresh_desktop_db(apps_dir);
        }
    }

    snprintf(auto_desk, sizeof(auto_desk), "%s/steam.desktop", auto_dir);
    snprintf(auto_bak, sizeof(auto_bak), "%s/steam_autostart.desktop.bak", cfg_dir);
    if (access(auto_desk, F_OK) == 0) {
        if (!file_contains_str(auto_desk, "salsa-steam-launch") && access(auto_bak, F_OK) != 0) {
            copy_file_nofollow(auto_desk, auto_bak, 0600);
        }
        const char *auto_src = (access(auto_bak, R_OK) == 0) ? auto_bak : auto_desk;
        rewrite_steam_desktop_exec(auto_src, auto_desk, wrap_path, true, 0644);
    }
}

static void remove_steam_wrapper(const char *home) {
    char bin_dir[512], apps_dir[512], auto_dir[512], cfg_dir[512];
    char wrap_path[600], app_desk[600], app_bak[600], auto_desk[600], auto_bak[600];
    snprintf(bin_dir, sizeof(bin_dir), "%s/.local/bin", home);
    snprintf(apps_dir, sizeof(apps_dir), "%s/.local/share/applications", home);
    snprintf(auto_dir, sizeof(auto_dir), "%s/.config/autostart", home);
    snprintf(cfg_dir, sizeof(cfg_dir), "%s/.config/lwasapi", home);

    snprintf(wrap_path, sizeof(wrap_path), "%s/salsa-steam-launch", bin_dir);
    unlink(wrap_path);

    snprintf(app_desk, sizeof(app_desk), "%s/steam.desktop", apps_dir);
    snprintf(app_bak, sizeof(app_bak), "%s/steam_app.desktop.bak", cfg_dir);
    if (access(app_bak, F_OK) == 0) {
        if (copy_file_nofollow(app_bak, app_desk, 0644)) {
            unlink(app_bak);
        }
        refresh_desktop_db(apps_dir);
    } else if (file_contains_str(app_desk, "salsa-steam-launch")) {
        unlink(app_desk);
        refresh_desktop_db(apps_dir);
    }

    snprintf(auto_desk, sizeof(auto_desk), "%s/steam.desktop", auto_dir);
    snprintf(auto_bak, sizeof(auto_bak), "%s/steam_autostart.desktop.bak", cfg_dir);
    if (access(auto_bak, F_OK) == 0) {
        if (copy_file_nofollow(auto_bak, auto_desk, 0644)) {
            unlink(auto_bak);
        }
    } else if (file_contains_str(auto_desk, "salsa-steam-launch")) {
        rewrite_steam_desktop_exec(auto_desk, auto_desk, wrap_path, false, 0644);
    }
}

static int cmd_enable_auto(void) {
    const char *home = getenv("HOME");
    if (!home) home = "/tmp";

    char root[512];
    get_install_root(root, sizeof(root));

    char asoundrc[512], backup[512];
    snprintf(asoundrc, sizeof(asoundrc), "%s/.asoundrc", home);
    snprintf(backup, sizeof(backup), "%s/.asoundrc.lwasapi.bak", home);

    if (access(asoundrc, F_OK) == 0 && access(backup, F_OK) != 0) {
        if (rename(asoundrc, backup) != 0) {}
    }

    FILE *fp = fopen(asoundrc, "w");
    if (!fp) {
        perror("fopen ~/.asoundrc");
        return 1;
    }

    fprintf(fp,
        "# LWASAPI Full-Duplex Automatic ALSA Application Recognition Config\n"
        "# Routes both Playback and Capture streams through LWASAPI.\n"
        "# Revert anytime with: lwasapi-ctl disable-auto\n"
        "pcm_type.lwasapi {\n"
        "    lib \"%s/build/libasound_module_pcm_lwasapi.so\"\n"
        "}\n\n"
        "pcm.lwasapi_raw {\n"
        "    type lwasapi\n"
        "}\n\n"
        "pcm.lwasapi {\n"
        "    type plug\n"
        "    slave.pcm \"lwasapi_raw\"\n"
        "}\n\n"
        "pcm.!default {\n"
        "    type asym\n"
        "    playback.pcm \"lwasapi\"\n"
        "    capture.pcm \"lwasapi\"\n"
        "}\n",
        root
    );
    fclose(fp);

    /* Configure OpenAL Soft (~/.config/OpenAL/alsoft.conf) for 3D games */
    char oal_dir[512], oal_conf[600], oal_bak[600];
    snprintf(oal_dir, sizeof(oal_dir), "%s/.config/OpenAL", home);
    snprintf(oal_conf, sizeof(oal_conf), "%s/alsoft.conf", oal_dir);
    snprintf(oal_bak, sizeof(oal_bak), "%s/alsoft.conf.lwasapi.bak", oal_dir);
    mkdir_p(oal_dir);
    if (access(oal_conf, F_OK) == 0 && access(oal_bak, F_OK) != 0) {
        if (rename(oal_conf, oal_bak) != 0) {}
    }
    FILE *ofp = fopen(oal_conf, "w");
    if (ofp) {
        fprintf(ofp,
            "# LWASAPI OpenAL Soft Automatic Routing\n"
            "[general]\n"
            "drivers = pulse,alsa\n"
            "channels = stereo\n"
            "sample-type = float32\n"
            "frequency = 48000\n"
            "period_size = 256\n"
            "periods = 4\n"
        );
        fclose(ofp);
    }

    /* Clean out legacy PipeWire/WirePlumber drop-ins & back up pre-Salsa custom configs */
    char wp_dir[512], wp_lua_main[512], wp_lua_bt[512], pw_conf_dir[512], cfg_dir[512], bak_dir[600];
    snprintf(wp_dir, sizeof(wp_dir), "%s/.config/wireplumber/wireplumber.conf.d", home);
    snprintf(wp_lua_main, sizeof(wp_lua_main), "%s/.config/wireplumber/main.lua.d", home);
    snprintf(wp_lua_bt, sizeof(wp_lua_bt), "%s/.config/wireplumber/bluetooth.lua.d", home);
    snprintf(pw_conf_dir, sizeof(pw_conf_dir), "%s/.config/pipewire/pipewire.conf.d", home);
    snprintf(cfg_dir, sizeof(cfg_dir), "%s/.config/lwasapi", home);
    snprintf(bak_dir, sizeof(bak_dir), "%s/pw_backup", cfg_dir);
    mkdir_p(cfg_dir);
    mkdir_p(bak_dir);

    char legacy1[600], legacy2[600], legacy3[600], legacy4[600];
    snprintf(legacy1, sizeof(legacy1), "%s/99-lwasapi-bridge.conf", pw_conf_dir);
    snprintf(legacy2, sizeof(legacy2), "%s/99-lwasapi-video-only.conf", wp_dir);
    snprintf(legacy3, sizeof(legacy3), "%s/99-lwasapi-no-audio.lua", wp_lua_main);
    snprintf(legacy4, sizeof(legacy4), "%s/99-lwasapi-no-bluez.lua", wp_lua_bt);
    unlink(legacy1);
    unlink(legacy2);
    unlink(legacy3);
    unlink(legacy4);

    char src1[600], src2[600], src3[600], dst1[700], dst2[700], dst3[700];
    snprintf(src1, sizeof(src1), "%s/10-bose-persistent-bridge.conf", pw_conf_dir);
    snprintf(src2, sizeof(src2), "%s/51-bluetooth-autoswitch.conf", wp_dir);
    snprintf(src3, sizeof(src3), "%s/52-bose-keepalive.conf", wp_dir);
    snprintf(dst1, sizeof(dst1), "%s/10-bose-persistent-bridge.conf", bak_dir);
    snprintf(dst2, sizeof(dst2), "%s/51-bluetooth-autoswitch.conf", bak_dir);
    snprintf(dst3, sizeof(dst3), "%s/52-bose-keepalive.conf", bak_dir);
    move_if_exists(src1, dst1);
    move_if_exists(src2, dst2);
    move_if_exists(src3, dst3);

    /* Enable Zero-PipeWire flag for Salsa's built-in pure-C ScreenCast Video Broker */
    char zero_flag[600];
    snprintf(zero_flag, sizeof(zero_flag), "%s/pipewire_zero_mode", cfg_dir);
    FILE *zfp = fopen(zero_flag, "w");
    if (zfp) {
        fprintf(zfp, "enabled\n");
        fclose(zfp);
    }

    /* Configure Steam/Proton/SDL/OpenAL/PulseAudio Reconnect Proxy environment */
    char pa_dir[1300], pa32_so[640], ld_val[2048], ld_env[2100];
    snprintf(pa32_so, sizeof(pa32_so), "%s/build/pa_reconnect32/libpulse.so.0", root);
    if (access(pa32_so, R_OK) == 0) {
        snprintf(pa_dir, sizeof(pa_dir), "%s/build/pa_reconnect:%s/build/pa_reconnect32", root, root);
    } else {
        snprintf(pa_dir, sizeof(pa_dir), "%s/build/pa_reconnect", root);
    }
    build_filtered_ld_path(pa_dir, ld_val, sizeof(ld_val));
    snprintf(ld_env, sizeof(ld_env), "LD_LIBRARY_PATH=%s", ld_val);

    char env_dir[512], env_file[600];
    snprintf(env_dir, sizeof(env_dir), "%s/.config/environment.d", home);
    snprintf(env_file, sizeof(env_file), "%s/99-salsa-direct.conf", env_dir);
    mkdir_p(env_dir);
    FILE *efp = fopen(env_file, "w");
    if (efp) {
        fprintf(efp,
            "# Salsa Zero-PipeWire Direct Steam/Proton/Desktop Audio Environment\n"
            "# Revert anytime with: lwasapi-ctl disable-auto\n"
            "PROTON_USE_PIPEWIRE=0\n"
            "SDL_AUDIODRIVER=pulse\n"
            "ALSOFT_DRIVERS=pulse,alsa\n"
            "LD_LIBRARY_PATH=%s\n",
            ld_val
        );
        fclose(efp);
    }
    {
        char *argv_sys[] = {
            "systemctl", "--user", "set-environment",
            "PROTON_USE_PIPEWIRE=0", "SDL_AUDIODRIVER=pulse",
            "ALSOFT_DRIVERS=pulse,alsa", ld_env, NULL
        };
        char *argv_dbus[] = {
            "dbus-update-activation-environment", "--systemd",
            "PROTON_USE_PIPEWIRE=0", "SDL_AUDIODRIVER=pulse",
            "ALSOFT_DRIVERS=pulse,alsa", ld_env, NULL
        };
        run_quiet_argv(argv_sys);
        run_quiet_argv(argv_dbus);
    }

    install_steam_wrapper(home, root);

    /* Stop & mask sockets FIRST before services so systemd cannot respawn PipeWire */
    static const char *const zero_units[] = {
        "pipewire-pulse.socket",
        "pipewire-pulse.service",
        "wireplumber.service",
        "pipewire.socket",
        "pipewire.service",
        "pulseaudio.socket",
        "pulseaudio.service",
        NULL
    };
    char units_list[600];
    snprintf(units_list, sizeof(units_list), "%s/masked_units.list", cfg_dir);
    for (int i = 0; zero_units[i]; i++) {
        if (unit_exists(zero_units[i])) {
            record_masked_unit(units_list, zero_units[i]);
            stop_and_mask_unit(zero_units[i]);
        }
    }

    /* Switch live lwasapi-audiodg ScreenCast broker onto $XDG_RUNTIME_DIR/pipewire-0 */
    if (system("systemctl --user is-active --quiet lwasapi-audiodg.service >/dev/null 2>&1") == 0) {
        if (trigger_broker_sync_mode() != 0) {
            if (system("systemctl --user try-restart lwasapi-audiodg.service >/dev/null 2>&1") != 0) {}
            const char *rt = getenv("XDG_RUNTIME_DIR");
            char pw_sock[512];
            snprintf(pw_sock, sizeof(pw_sock), "%s/pipewire-0", (rt && rt[0]) ? rt : "/run/user/1000");
            usleep(150000);
            for (int wi = 0; wi < 40 && access(pw_sock, F_OK) != 0; wi++) usleep(50000);
        }
        if (system("systemctl --user try-restart xdg-desktop-portal.service plasma-xdg-desktop-portal-kde.service xdg-desktop-portal-kde.service xdg-desktop-portal-wlr.service xdg-desktop-portal-hyprland.service xdg-desktop-portal-gnome.service xdg-desktop-portal-gtk.service >/dev/null 2>&1") != 0) {}
    }

    printf("Enabled Zero-PipeWire Pure Salsa mode (ScreenCast portals preserved by Salsa Video Broker):\n");
    printf("  - ALSA/OpenAL routing   : %s, %s\n", asoundrc, oal_conf);
    printf("  - Zero-PipeWire flag    : %s (Salsa owns $XDG_RUNTIME_DIR/pipewire-0 for video)\n", zero_flag);
    printf("  - Steam/Proton direct   : %s (PROTON_USE_PIPEWIRE=0, direct PA/ALSA)\n", env_file);
    printf("  - Steam container proxy : %s/.local/bin/salsa-steam-launch (reconnects across engine resets)\n", home);
    printf("  - Upstream PW/WP/PA     : stopped & masked cleanly\n");
    printf("To revert cleanly anytime, run: lwasapi-ctl disable-auto\n");
    return 0;
}

static int cmd_disable_auto(void) {
    const char *home = getenv("HOME");
    if (!home) home = "/tmp";

    char asoundrc[512], backup[512];
    snprintf(asoundrc, sizeof(asoundrc), "%s/.asoundrc", home);
    snprintf(backup, sizeof(backup), "%s/.asoundrc.lwasapi.bak", home);

    unlink(asoundrc);
    if (access(backup, F_OK) == 0) {
        if (rename(backup, asoundrc) != 0) {}
        printf("Restored previous %s from backup.\n", asoundrc);
    } else {
        printf("Removed %s and restored stock ALSA configuration.\n", asoundrc);
    }

    char oal_conf[600], oal_bak[600];
    snprintf(oal_conf, sizeof(oal_conf), "%s/.config/OpenAL/alsoft.conf", home);
    snprintf(oal_bak, sizeof(oal_bak), "%s/.config/OpenAL/alsoft.conf.lwasapi.bak", home);
    unlink(oal_conf);
    if (access(oal_bak, F_OK) == 0) {
        if (rename(oal_bak, oal_conf) != 0) {}
    }

    char wp_dir[512], wp_lua_main[512], wp_lua_bt[512], pw_dir[512], pw_bridge[600], wp_conf[600], wp_lua1[600], wp_lua2[600], cfg_dir[512], bak_dir[600];
    snprintf(wp_dir, sizeof(wp_dir), "%s/.config/wireplumber/wireplumber.conf.d", home);
    snprintf(wp_lua_main, sizeof(wp_lua_main), "%s/.config/wireplumber/main.lua.d", home);
    snprintf(wp_lua_bt, sizeof(wp_lua_bt), "%s/.config/wireplumber/bluetooth.lua.d", home);
    snprintf(pw_dir, sizeof(pw_dir), "%s/.config/pipewire/pipewire.conf.d", home);
    snprintf(pw_bridge, sizeof(pw_bridge), "%s/99-lwasapi-bridge.conf", pw_dir);
    snprintf(wp_conf, sizeof(wp_conf), "%s/99-lwasapi-video-only.conf", wp_dir);
    snprintf(wp_lua1, sizeof(wp_lua1), "%s/99-lwasapi-no-audio.lua", wp_lua_main);
    snprintf(wp_lua2, sizeof(wp_lua2), "%s/99-lwasapi-no-bluez.lua", wp_lua_bt);
    snprintf(cfg_dir, sizeof(cfg_dir), "%s/.config/lwasapi", home);
    snprintf(bak_dir, sizeof(bak_dir), "%s/pw_backup", cfg_dir);
    unlink(pw_bridge);
    unlink(wp_conf);
    unlink(wp_lua1);
    unlink(wp_lua2);

    char zero_flag[600], env_file[600];
    snprintf(zero_flag, sizeof(zero_flag), "%s/pipewire_zero_mode", cfg_dir);
    snprintf(env_file, sizeof(env_file), "%s/.config/environment.d/99-salsa-direct.conf", home);
    unlink(zero_flag);
    unlink(env_file);
    remove_steam_wrapper(home);
    {
        char ld_val[2048];
        build_filtered_ld_path(NULL, ld_val, sizeof(ld_val));
        if (ld_val[0] == '\0') {
            char *argv_unset[] = {
                "systemctl", "--user", "unset-environment",
                "PROTON_USE_PIPEWIRE", "SDL_AUDIODRIVER",
                "ALSOFT_DRIVERS", "LD_LIBRARY_PATH", NULL
            };
            run_quiet_argv(argv_unset);
        } else {
            char ld_env[2100];
            snprintf(ld_env, sizeof(ld_env), "LD_LIBRARY_PATH=%s", ld_val);
            char *argv_unset[] = {
                "systemctl", "--user", "unset-environment",
                "PROTON_USE_PIPEWIRE", "SDL_AUDIODRIVER",
                "ALSOFT_DRIVERS", NULL
            };
            char *argv_set[] = {
                "systemctl", "--user", "set-environment", ld_env, NULL
            };
            char *argv_dbus[] = {
                "dbus-update-activation-environment", "--systemd", ld_env, NULL
            };
            run_quiet_argv(argv_unset);
            run_quiet_argv(argv_set);
            run_quiet_argv(argv_dbus);
        }
    }

    char src2[700], dst2[600];
    snprintf(src2, sizeof(src2), "%s/51-bluetooth-autoswitch.conf", bak_dir);
    snprintf(dst2, sizeof(dst2), "%s/51-bluetooth-autoswitch.conf", wp_dir);
    if (access(src2, F_OK) == 0) mkdir_p(wp_dir);
    move_if_exists(src2, dst2);

    /* Release Salsa sockets cleanly before restoring upstream PipeWire/Pulse */
    (void)trigger_broker_sync_mode();
    if (system("systemctl --user stop lwasapi-audiodg.service >/dev/null 2>&1") != 0) {}

    char units_list[600];
    snprintf(units_list, sizeof(units_list), "%s/masked_units.list", cfg_dir);
    FILE *ufp = fopen(units_list, "r");
    if (ufp) {
        char ubuf[128];
        while (fgets(ubuf, sizeof(ubuf), ufp)) {
            ubuf[strcspn(ubuf, "\r\n \t")] = '\0';
            if (ubuf[0]) unmask_and_enable_unit(ubuf);
        }
        fclose(ufp);
        unlink(units_list);
    }
    static const char *const fallback_units[] = {
        "pipewire.socket",
        "pipewire.service",
        "wireplumber.service",
        "pipewire-pulse.socket",
        "pipewire-pulse.service",
        "pulseaudio.socket",
        "pulseaudio.service",
        NULL
    };
    for (int i = 0; fallback_units[i]; i++) {
        if (unit_exists(fallback_units[i])) {
            unmask_and_enable_unit(fallback_units[i]);
        }
    }

    if (system("systemctl --user reset-failed pipewire.service pipewire.socket wireplumber.service pipewire-pulse.service pipewire-pulse.socket >/dev/null 2>&1") != 0) {}
    if (unit_exists("pipewire.socket")) {
        if (system("systemctl --user start pipewire.socket >/dev/null 2>&1") != 0) {}
    }
    if (unit_exists("pipewire.service")) {
        if (system("systemctl --user restart pipewire.service >/dev/null 2>&1") != 0) {}
    }
    if (unit_exists("wireplumber.service") &&
        system("systemctl --user is-active --quiet wireplumber.service >/dev/null 2>&1") != 0) {
        if (system("systemctl --user start wireplumber.service >/dev/null 2>&1") != 0) {}
    }
    if (system("systemctl --user try-restart xdg-desktop-portal.service plasma-xdg-desktop-portal-kde.service xdg-desktop-portal-kde.service xdg-desktop-portal-wlr.service xdg-desktop-portal-hyprland.service xdg-desktop-portal-gnome.service xdg-desktop-portal-gtk.service >/dev/null 2>&1") != 0) {}

    printf("Restored upstream desktop sound server (PipeWire, PipeWire-Pulse & WirePlumber) cleanly.\n");
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2 || strcmp(argv[1], "status") == 0) {
        return cmd_status();
    }
    if (strcmp(argv[1], "devices") == 0) {
        return cmd_devices();
    }
    if ((strcmp(argv[1], "set-sink") == 0 || strcmp(argv[1], "set-output") == 0) && argc >= 3) {
        return cmd_set_device(false, argv[2]);
    }
    if ((strcmp(argv[1], "set-source") == 0 || strcmp(argv[1], "set-input") == 0) && argc >= 3) {
        return cmd_set_device(true, argv[2]);
    }
    if (strcmp(argv[1], "bt-reset") == 0 || strcmp(argv[1], "reset-bt") == 0 || strcmp(argv[1], "bt-reconnect") == 0) {
        return cmd_bt_reset((argc >= 3) ? argv[2] : "");
    }
    if ((strcmp(argv[1], "volume") == 0 || strcmp(argv[1], "set-volume") == 0 ||
         strcmp(argv[1], "app-volume") == 0) && argc >= 4) {
        if (strcmp(argv[1], "app-volume") == 0) {
            int pct = atoi(argv[3]);
            if (pct < 0) pct = 0;
            if (pct > 150) pct = 150;
            return cmd_session_control((uint32_t)atoi(argv[2]), pct, -1, NULL);
        }
        return cmd_volume((uint32_t)atoi(argv[2]), strtof(argv[3], NULL));
    }
    if (strcmp(argv[1], "app-mute") == 0 && argc >= 4) {
        int m = (strcmp(argv[3], "toggle") == 0) ? 2 : (atoi(argv[3]) ? 1 : 0);
        return cmd_session_control((uint32_t)atoi(argv[2]), -1, m, NULL);
    }
    if (strcmp(argv[1], "move-app") == 0 && argc >= 4) {
        return cmd_session_control((uint32_t)atoi(argv[2]), -1, -1, argv[3]);
    }
    if (strcmp(argv[1], "master-volume") == 0 && argc >= 3) {
        return cmd_master_volume(atoi(argv[2]), -1);
    }
    if (strcmp(argv[1], "mute") == 0 && argc >= 3) {
        int m = (strcmp(argv[2], "toggle") == 0) ? 2 : atoi(argv[2]);
        return cmd_master_volume(-1, m);
    }
    if (strcmp(argv[1], "mic-volume") == 0 && argc >= 3) {
        return cmd_mic_volume(atoi(argv[2]), -1);
    }
    if (strcmp(argv[1], "mic-mute") == 0 && argc >= 3) {
        int m = (strcmp(argv[2], "toggle") == 0) ? 2 : atoi(argv[2]);
        return cmd_mic_volume(-1, m);
    }
    if (strcmp(argv[1], "midi-send") == 0 && argc >= 3) {
        uint8_t note = (uint8_t)atoi(argv[2]);
        uint8_t vel = (argc >= 4) ? (uint8_t)atoi(argv[3]) : 100;
        return cmd_midi_send(note, vel);
    }
    if (strcmp(argv[1], "set-rate") == 0 && argc >= 3) {
        uint32_t period = (argc >= 4) ? (uint32_t)atoi(argv[3]) : 0;
        uint32_t bits = (argc >= 5) ? (uint32_t)atoi(argv[4]) : 0;
        uint32_t rate = (strcasecmp(argv[2], "keep") == 0) ? 0U : (uint32_t)atoi(argv[2]);
        return cmd_set_rate(rate, period, bits);
    }
    if (strcmp(argv[1], "set-cap-rate") == 0 && argc >= 3) {
        uint32_t rate = (strcasecmp(argv[2], "auto") == 0 || strcasecmp(argv[2], "follow") == 0)
                        ? 0U : (uint32_t)atoi(argv[2]);
        if (rate != 0U && (rate < 44100U || rate > 192000U)) {
            fprintf(stderr, "Usage: %s set-cap-rate <44100..192000|auto>\n", argv[0]);
            return 1;
        }
        return cmd_set_cap_rate(rate);
    }
    if (strcmp(argv[1], "bt-codecs") == 0) {
        return cmd_bt_codecs();
    }
    if (strcmp(argv[1], "set-codec") == 0 && argc >= 3) {
        const char *tier = (argc >= 4) ? argv[3] : "";
        return cmd_set_codec(argv[2], tier);
    }
    if (strcmp(argv[1], "bt-sco") == 0 && argc >= 3) {
        return cmd_bt_sco(argv[2]);
    }
    if (strcmp(argv[1], "param-eq") == 0 && argc >= 7) {
        return cmd_param_eq(atoi(argv[2]), strtof(argv[3], NULL), strtof(argv[4], NULL),
                            argv[5], strtof(argv[6], NULL));
    }
    if (strcmp(argv[1], "crossfeed") == 0 && argc >= 3) {
        float lvl = (argc >= 4) ? strtof(argv[3], NULL) : -1.0f;
        uint32_t cut = (argc >= 5) ? (uint32_t)atoi(argv[4]) : 0u;
        return cmd_crossfeed(argv[2], lvl, cut);
    }
    if (strcmp(argv[1], "mic-apo") == 0 && argc >= 3) {
        int rnn = (argc >= 4) ? atoi(argv[3]) : -1;
        float gate = (argc >= 5) ? strtof(argv[4], NULL) : 1.0f;
        int side = (argc >= 6) ? atoi(argv[5]) : -1;
        return cmd_mic_apo(argv[2], rnn, gate, side);
    }
    if (strcmp(argv[1], "mic-preset") == 0 && argc >= 3) {
        return cmd_mic_preset(argv[2]);
    }
    if (strcmp(argv[1], "hw-analog") == 0 && argc >= 2) {
        return cmd_hw_analog(argc, argv);
    }
    if (strcmp(argv[1], "eq-preset") == 0 && argc >= 3) {
        return cmd_eq_preset(argv[2]);
    }
    if (strcmp(argv[1], "eq-band") == 0 && argc >= 4) {
        return cmd_eq_band(atoi(argv[2]), strtof(argv[3], NULL));
    }
    if (strcmp(argv[1], "eq-mode") == 0 && argc >= 3) {
        float thresh = (argc >= 4) ? strtof(argv[3], NULL) : 1.0f;
        float ratio  = (argc >= 5) ? strtof(argv[4], NULL) : 0.0f;
        return cmd_eq_mode(argv[2], thresh, ratio);
    }
    if (strcmp(argv[1], "enable-auto") == 0) {
        return cmd_enable_auto();
    }
    if (strcmp(argv[1], "disable-auto") == 0) {
        return cmd_disable_auto();
    }
    if (strcmp(argv[1], "bench") == 0 || strcmp(argv[1], "benchmark") == 0) {
        char root[512], bpath[600];
        get_install_root(root, sizeof(root));
        snprintf(bpath, sizeof(bpath), "%s/build/lwasapi-bench", root);
        execl(bpath, "lwasapi-bench", NULL);
        perror("execl lwasapi-bench");
        return 1;
    }
    if (strcmp(argv[1], "sync-modules") == 0 || strcmp(argv[1], "sync") == 0) {
        char root[512], spath[600];
        get_install_root(root, sizeof(root));
        snprintf(spath, sizeof(spath), "%s/lwasapi-autosync.sh", root);
        execl(spath, "lwasapi-autosync.sh", "--force", NULL);
        perror("execl lwasapi-autosync.sh");
        return 1;
    }

    fprintf(stderr,
        "Usage: %s <command>\n"
        "  status                                    Show Salsa v6.7-beta engine, BlueZ, PA socket & HW analog status\n"
        "  bench                                     Run live latency benchmark vs. Native PA / ALSA\n"
        "  sync-modules                              Sync ELF symbol versions, rebuild shims & self-heal configs\n"
        "  devices                                   List active WaveRT ALSA, BlueZ A2DP & MIDI endpoints\n"
        "  set-sink <id|auto|virtual>                Switch playback output endpoint (e.g. hw:3,0, bluez:MAC)\n"
        "  set-source <id|auto>                      Switch kernel ALSA capture endpoint (e.g. hw:3,0)\n"
        "  bt-reset [mac]                            Cleanly reset & reconnect Bluetooth A2DP over BR/EDR\n"
        "  bt-codecs                                 Inspect remote BT supported SEPs & negotiation diagnostics\n"
        "  set-codec <auto|ldac|aptxhd|aptx|aac|sbc> [abr|990|660|330]\n"
        "                                            Switch BlueZ A2DP preferred codec & LDAC bitrate tier\n"
        "  bt-sco <on|off>                           Toggle two-way Bluetooth HFP BTPROTO_SCO headset mic\n"
        "  set-rate <44100..192000|keep> [period] [bits]\n"
        "                                            Configure fixed Master Sample Rate Hz, quantum period & bit depth (16/24/32)\n"
        "  set-cap-rate <44100..192000|auto>\n"
        "                                            Pin capture/mic sample rate independently of playback clock (auto = follow master)\n"
        "  hw-analog [status|reset | <dac|amp|route|rails|boost|mic-boost> <val> ...]\n"
        "                                            Configure HW DAC Unity, Discrete Amp Tier (auto|low|med|high), Route & Boost\n"
        "  eq-preset <flat|bose|bass|hires|gaming|vocal|loudness>\n"
        "                                            Load a Salsa 10-band Dynamic EQ studio preset\n"
        "  eq-band <0-9> <gain_db>                   Adjust a specific EQ band (-18.0 to +18.0 dB)\n"
        "  param-eq <0-9> <freq> <q> <peak|lowshelf|highshelf> <gain_db>\n"
        "                                            Configure Parametric EQ frequency, Q & filter shape\n"
        "  crossfeed <on|off> [level_pct] [cutoff]   Toggle Bauer headphone stereo crossfeed\n"
        "  eq-mode <off|static|compress|expand> [thresh_db] [ratio]\n"
        "                                            Configure Dynamic EQ mode, threshold & ratio\n"
        "  mic-apo <on|off> [rnnoise 0|1] [gate_db] [sidetone_pct]\n"
        "                                            Configure Capture Mic APO (HPF, Gate, RNNoise, Comp, Sidetone)\n"
        "  mic-preset <discord|broadcast|clean|bypass>\n"
        "                                            Load a quick reversible Mic APO profile (e.g. discord = RNNoise + -50dB gate)\n"
        "  volume <id> <0.0-1.0|0-100>               Set volume for a specific session or PA client stream\n"
        "  app-volume <id> <0-150>                   Set & persist per-app session volume (0-150%%)\n"
        "  app-mute <id> <0|1|toggle>                Mute, unmute, or toggle a specific session/app\n"
        "  move-app <id> <dev_id|auto>               Route a PA client stream to a specific sink/source\n"
        "  master-volume <0-150>                     Set Salsa master speaker volume percentage (+ AVRCP)\n"
        "  mute <0|1|toggle>                         Set or toggle Salsa master speaker mute\n"
        "  mic-volume <0-150>                        Set Salsa microphone input gain percentage\n"
        "  mic-mute <0|1|toggle>                     Set or toggle Salsa microphone mute\n"
        "  midi-send <note> [velocity]               Inject a MIDI Note-On into the LWASAPI DAW MIDI Hub\n"
        "  enable-auto                               Enable Zero-PipeWire Salsa mode (Video Broker, Steam/ALSA/PA direct)\n"
        "  disable-auto                              Restore upstream PipeWire, PipeWire-Pulse & WirePlumber cleanly\n",
        argv[0]);
    return 1;
}
