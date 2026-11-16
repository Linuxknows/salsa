#ifndef LWASAPI_BLUEZ_H
#define LWASAPI_BLUEZ_H

#include <stdint.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <pthread.h>
#include <sbc/sbc.h>
#include "lwasapi.h"

#define LWASAPI_BT_RING_FRAMES 16384

typedef struct {
    bool      initialized;
    bool      endpoints_registered;     /* true = org.bluez.MediaEndpoint1 AAC+SBC active */
    bool      direct_socket_mode;       /* true = raw BlueZ L2CAP A2DP transport socket fd acquired */
    bool      has_bluez_transport;      /* true = active BlueZ MediaTransport1 discovered/acquired */
    bool      has_capture_hw;           /* true = direct kernel ALSA /dev/snd/pcmC*D*c active */

    /* Active Bluetooth A2DP endpoint metadata */
    char      adapter_path[64];         /* e.g. /org/bluez/hci0 */
    char      device_path[128];         /* e.g. /org/bluez/hci0/dev_00_11_22_33_44_55 */
    char      endpoint_path[64];        /* e.g. /org/lwasapi/A2DP/AAC/Source */
    char      transport_path[160];      /* e.g. /org/bluez/hci0/dev_00_11_22_33_44_55/sep6/fd0 */
    char      device_alias[64];         /* e.g. Bose QC Ultra 2 HP */
    char      mac_address[32];          /* e.g. 00:11:22:33:44:55 */
    char      codec_name[16];           /* e.g. LDAC, aptX HD, aptX, AAC or SBC */
    uint8_t   codec_id;                 /* 0x00 = SBC, 0x02 = AAC, 0xFF = Vendor */
    uint8_t   active_codec_kind;        /* 0=SBC, 1=AAC, 2=aptX, 3=aptX HD, 4=LDAC */
    uint8_t   a2dp_config[16];
    uint32_t  a2dp_config_len;
    uint32_t  bt_sample_rate;           /* Negotiated A2DP transport sample rate */
    uint32_t  bt_channels;
    uint32_t  engine_sample_rate;       /* Mixer sample rate feeding BT resampler */
    uint16_t  delay_tenths_ms;          /* MediaTransport1.Delay (1/10 ms) */
    uint16_t  avrcp_volume;             /* 0..127 synchronized over BlueZ */
    int       bt_fd;                    /* Acquired L2CAP socket fd */
    uint16_t  read_mtu;
    uint16_t  write_mtu;

    /* Encoders (SBC + libfdk-aac.so.2 + libldacBT_enc.so.2 + libfreeaptx.so.0) */
    sbc_t     sbc_enc;
    bool      sbc_enc_init;
    void     *fdk_dl;
    void     *fdk_enc;
    uint32_t  fdk_frame_length;
    void     *ldac_dl;
    void     *ldac_enc;
    int       ldac_eqmid;
    void     *aptx_dl;
    void     *aptx_enc;
    uint16_t  rtp_seq;
    uint32_t  rtp_timestamp;

    /* Paced A2DP TX ring + thread (keeps RT audio engine non-blocking) */
    float            tx_ring[LWASAPI_BT_RING_FRAMES * 2];
    _Atomic uint64_t tx_write_frame;
    _Atomic uint64_t tx_read_frame;
    _Atomic uint64_t tx_packets_sent;
    _Atomic uint64_t tx_underruns;
    _Atomic uint64_t tx_errors;
    pthread_t        tx_thread;
    volatile int     tx_thread_running;
    volatile int     tx_stream_active;
    _Atomic int      tx_paused;

    /* System D-Bus service thread handling MediaEndpoint1 + PropertiesChanged */
    void            *dbus_conn;
    pthread_t        dbus_thread;
    volatile int     dbus_thread_running;
    pthread_mutex_t  state_lock;

    /* Direct Kernel ALSA Capture (/dev/snd/pcmC*D*c) */
    int              cap_pcm_fd;
    char             cap_device_id[64];         /* e.g. "hw:3,0" or "auto" */
    char             cap_pcm_path[64];          /* e.g. "/dev/snd/pcmC3D0c" */
    char             capture_device_alias[64];  /* e.g. "Scarlett 2i2 4th Gen" */
    uint32_t         cap_hw_rate;
    uint32_t         cap_hw_channels;
    int              cap_hw_format;             /* SNDRV_PCM_FORMAT_* */
    bool             cap_is_usb;
    uint32_t         cap_ring_rate;             /* Native sample rate stored inside cap_ring */
    uint32_t         cap_measured_rate;         /* Live measured ALSA capture Hz */
    uint32_t         cap_kick_count;            /* UAC2 clock watchdog kicks triggered */
    volatile int     cap_thread_running;
    pthread_t        cap_thread;
    _Atomic uint64_t cap_hw_write_frame;
    float            cap_ring[LWASAPI_RING_FRAMES * 2];

    /* Preferred playback Bluetooth MAC (e.g. "00:11:22:33:44:55") for auto-connect & recovery */
    char              preferred_mac[32];
    _Atomic uint64_t  tx_drop_event_ns;

    /* Master software gain/mute state (0..150%) */
    uint32_t          desktop_volume_pct;
    bool              desktop_muted;
    uint32_t          desktop_mic_volume_pct;
    bool              desktop_mic_muted;
    _Atomic uint32_t  desktop_mic_gain_ppm; /* 1000000 = 1.0x linear gain */

    /* Rev 6.1 Active Codec Switcher, Wireless ABR & Diagnostics */
    void             *ldac_abr;
    uint32_t          pref_codec;            /* LWASAPI_BT_PREF_* */
    uint32_t          ldac_mode;             /* LWASAPI_LDAC_MODE_* */
    uint32_t          wireless_abr_enabled;  /* 1 = Adaptive Wireless Bitrate (LDAC+AAC) */
    uint32_t          effective_kbps;        /* Current negotiated/adapted bitrate (kbps) */
    uint32_t          supported_codecs_mask; /* LWASAPI_BT_CAP_* bitmask */
    uint32_t          aac_target_bps;        /* Current FDK-AAC target bitrate (bps) */
    uint32_t          aac_max_bps;           /* Negotiated remote/MTU AAC ceiling (bps) */
    uint64_t          abr_last_ms;
    uint32_t          tx_congest_streak;
    uint32_t          tx_clean_streak;
    char              neg_reason[64];
    _Atomic uint32_t  bt_renegotiate_req;    /* 1 = DBus thread should switch codec & reconnect */
    _Atomic uint32_t  bt_renegotiate_fallback;
    uint64_t          renegotiate_start_ns;

    /* Rev 6.1 Optional Two-Way HFP BTPROTO_SCO Headset Mic (default OFF) */
    uint32_t          hfp_sco_enabled;       /* 0 = Scarlett default preserved, 1 = HFP SCO permitted */
    _Atomic uint32_t  hfp_sco_active;        /* 1 = BTPROTO_SCO mSBC/CVSD connected */
    char              bt_sco_codec[8];       /* "mSBC" / "CVSD" / "" */
    int               sco_fd;
    pthread_t         sco_thread;
    volatile int      sco_thread_running;
    uint32_t          sco_sample_rate;
    _Atomic uint64_t  sco_cap_write_frame;
    float             sco_cap_ring[LWASAPI_RING_FRAMES * 2];
    char              sco_cap_device_id[64]; /* "bluez_sco:XX:XX:..." */
} lwasapi_bluez_t;

int  lwasapi_bluez_init(lwasapi_bluez_t *bt, uint32_t engine_sample_rate, const char *preferred_cap_id, const char *preferred_pb_id);
int  lwasapi_bluez_init_rates(lwasapi_bluez_t *bt, uint32_t engine_sample_rate, uint32_t capture_sample_rate, const char *preferred_cap_id, const char *preferred_pb_id);
void lwasapi_bluez_set_preferred_target(lwasapi_bluez_t *bt, const char *preferred_pb_id);
int  lwasapi_bluez_reset_device(const char *mac_or_empty, char *out_msg, size_t out_len);
int  lwasapi_bluez_reconfigure_rate(lwasapi_bluez_t *bt, uint32_t new_engine_sample_rate);
void lwasapi_bluez_set_tx_enabled(lwasapi_bluez_t *bt, bool enabled);
int  lwasapi_bluez_write_period(lwasapi_bluez_t *bt, const float *interleaved_float, uint32_t frames, uint32_t channels);
int  lwasapi_bluez_open_capture(lwasapi_bluez_t *bt, const char *preferred_cap_id, uint32_t capture_sample_rate);
int  lwasapi_bluez_read_capture_period(lwasapi_bluez_t *bt, float *out_stereo_float, uint32_t frames, uint64_t *io_read_cursor);
int  lwasapi_bluez_read_capture_native(lwasapi_bluez_t *bt, float *out_stereo_float, uint32_t max_frames,
                                       uint32_t *out_frames, uint32_t *out_rate,
                                       uint32_t eng_frames, uint32_t eng_rate, uint64_t *io_read_cursor);
int  lwasapi_bluez_enumerate_devices(const lwasapi_bluez_t *bt, const char *active_pb_id, const char *active_cap_id, lwasapi_device_list_t *out_list);
uint64_t lwasapi_bluez_hw_fingerprint(const lwasapi_bluez_t *bt);
int  lwasapi_bluez_set_volume(lwasapi_bluez_t *bt, int vol_pct, int mute_flag);
int  lwasapi_bluez_set_mic_volume(lwasapi_bluez_t *bt, int vol_pct, int mute_flag);
int  lwasapi_bluez_set_mic_gain_ppm(lwasapi_bluez_t *bt, uint32_t gain_ppm, int vol_pct, int mute_flag);
int  lwasapi_bluez_set_codec_config(lwasapi_bluez_t *bt, const lwasapi_bt_codec_cfg_t *cfg);
void lwasapi_bluez_get_codec_config(const lwasapi_bluez_t *bt, lwasapi_bt_codec_cfg_t *out_cfg);
int  lwasapi_bluez_set_sco_enabled(lwasapi_bluez_t *bt, bool enabled);
void lwasapi_bluez_close(lwasapi_bluez_t *bt);

#endif /* LWASAPI_BLUEZ_H */
