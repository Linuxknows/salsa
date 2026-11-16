#ifndef LWASAPI_H
#define LWASAPI_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <stdatomic.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ============================================================================
 * LWASAPI: Linux Windows Audio Session API & Universal Compatibility Engine
 * Encapsulates Windows WASAPI / WaveRT / MMCSS / AudioGraph semantics on Linux
 * Full-Duplex Playback + Microphone/Instrument Capture + DAW MIDI + Loopback
 * ============================================================================ */

#define LWASAPI_SOCKET_DEFAULT "/tmp/lwasapi-audiodg.sock"
#define LWASAPI_MAX_SESSIONS   32
#define LWASAPI_MAX_CHANNELS   8
#define LWASAPI_RING_FRAMES    32768U /* Power of 2 (170ms @ 192kHz / 682ms @ 48kHz) */
#define LWASAPI_RING_MASK      (LWASAPI_RING_FRAMES - 1U)
#define LWASAPI_EQ_BANDS       10

/* Windows-style HRESULT codes */
typedef int32_t HRESULT;
#define S_OK                            ((HRESULT)0x00000000L)
#define E_FAIL                          ((HRESULT)0x80004005L)
#define E_INVALIDARG                    ((HRESULT)0x80070057L)
#define E_OUTOFMEMORY                   ((HRESULT)0x8007000EL)
#define AUDCLNT_E_NOT_INITIALIZED       ((HRESULT)0x88890001L)
#define AUDCLNT_E_ALREADY_INITIALIZED   ((HRESULT)0x88890002L)
#define AUDCLNT_E_DEVICE_IN_USE         ((HRESULT)0x8889000AL)
#define AUDCLNT_E_BUFFER_TOO_LARGE      ((HRESULT)0x88890006L)
#define AUDCLNT_E_UNSUPPORTED_FORMAT    ((HRESULT)0x88890008L)

/* Share Modes (Matches Windows AUDCLNT_SHAREMODE) */
typedef enum {
    AUDCLNT_SHAREMODE_SHARED    = 0,
    AUDCLNT_SHAREMODE_EXCLUSIVE = 1
} AUDCLNT_SHAREMODE;

/* Stream Flags (Matches Windows AUDCLNT_STREAMFLAGS_*) */
#define AUDCLNT_STREAMFLAGS_CROSSPROCESS        0x00010000U
#define AUDCLNT_STREAMFLAGS_LOOPBACK            0x00020000U
#define AUDCLNT_STREAMFLAGS_EVENTCALLBACK       0x00040000U
#define AUDCLNT_STREAMFLAGS_NOPERSIST           0x00080000U
#define AUDCLNT_STREAMFLAGS_RATEADJUST          0x00100000U
#define AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM      0x80000000U
#define AUDCLNT_STREAMFLAGS_CAPTURE             0x08000000U

/* Buffer Flags */
#define AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY  0x01U
#define AUDCLNT_BUFFERFLAGS_SILENT              0x02U

/* Sample Formats */
typedef enum {
    LWASAPI_FMT_FLOAT32 = 1,
    LWASAPI_FMT_S16_LE  = 2,
    LWASAPI_FMT_S32_LE  = 3
} lwasapi_sample_format_t;

/* Windows WAVEFORMATEX Equivalent */
typedef struct {
    uint16_t wFormatTag;
    uint16_t nChannels;
    uint32_t nSamplesPerSec;
    uint32_t nAvgBytesPerSec;
    uint16_t nBlockAlign;
    uint16_t wBitsPerSample;
    uint16_t cbSize;
} WAVEFORMATEX;

/* ============================================================================
 * Salsa (Linux WASAPI) 10-Band Real-Time Dynamic EQ APO Configuration
 * ============================================================================ */

typedef struct {
    uint32_t enabled;                         /* 0 = Bypass, 1 = Active */
    uint32_t dynamic_mode;                    /* 0 = Static Parametric, 1 = Dynamic Compress, 2 = Dynamic Expand */
    float    preamp_db;                       /* -12.0f .. +12.0f */
    float    dyn_threshold_db;                /* -48.0f .. 0.0f (default -18.0f) */
    float    dyn_ratio;                       /* 1.0f .. 8.0f (default 2.5f) */
    uint32_t limiter_enabled;                 /* 1 = Soft-Knee Mastering Limiter */
    float    band_gains_db[LWASAPI_EQ_BANDS]; /* -18.0f .. +18.0f for 31..16k Hz */
    float    band_q[LWASAPI_EQ_BANDS];        /* Default 1.414f per band */
    char     preset_name[32];
} lwasapi_eq_config_t;

/* ============================================================================
 * Sample-Accurate MIDI Event Structure for DAWs (ALSA Seq / JACK MIDI / RawMIDI)
 * ============================================================================ */

#define LWASAPI_MIDI_RING_EVENTS 512U
#define LWASAPI_MIDI_RING_MASK   (LWASAPI_MIDI_RING_EVENTS - 1U)

typedef struct {
    uint32_t frame_offset;   /* Sample offset within current audio period */
    uint8_t  size;           /* 1..3 bytes (standard MIDI message) */
    uint8_t  port;           /* Source/destination MIDI port index */
    uint8_t  data[4];        /* Raw MIDI status + data1 + data2 */
    uint64_t timestamp_ns;   /* Monotonic hardware timestamp */
} lwasapi_midi_event_t;

/* ============================================================================
 * Lock-Free Shared Memory Cyclic Ring Buffer (WaveRT / audiodg equivalent)
 * Mapped into both the Client Process and lwasapi-audiodg via memfd_create()
 * ============================================================================ */

typedef struct {
    uint32_t magic;                      /* 0x4C574153 ('LWAS') */
    uint32_t session_id;
    uint32_t client_pid;
    char     client_name[64];
    uint32_t share_mode;                 /* AUDCLNT_SHAREMODE */
    uint32_t stream_flags;               /* AUDCLNT_STREAMFLAGS_* */
    uint32_t is_capture;                 /* 0 = Render, 1 = Capture, 2 = Duplex */
    uint32_t sample_rate;
    uint32_t channels;
    uint32_t format;                     /* lwasapi_sample_format_t */
    uint32_t buffer_frames;              /* Total ring capacity in frames */
    uint32_t period_frames;              /* Hardware/Engine period quantum */

    _Atomic uint64_t write_frame;        /* Monotonic render frames written by client */
    _Atomic uint64_t read_frame;         /* Monotonic render frames consumed by engine */
    _Atomic uint64_t cap_write_frame;    /* Monotonic capture frames written by engine */
    _Atomic uint64_t cap_read_frame;     /* Monotonic capture frames read by client */
    _Atomic uint32_t active;             /* 1 = started, 0 = stopped */
    _Atomic uint32_t muted;              /* 1 = muted, 0 = unmuted */
    _Atomic uint32_t volume_milli;       /* 1000 = 1.0f unity gain */
    _Atomic uint64_t xruns;              /* Underrun/Overrun counter */
    _Atomic uint64_t hw_clock_ns;        /* Latest hardware DMA clock timestamp */

    /* Lock-free MIDI Ring Buffer (Hardware Controller / Sequencer <-> DAW) */
    _Atomic uint64_t midi_write_idx;
    _Atomic uint64_t midi_read_idx;
    lwasapi_midi_event_t midi_ring[LWASAPI_MIDI_RING_EVENTS];

    /* Interleaved float32 render & capture ring buffers */
    _Alignas(64) float pcm_data[LWASAPI_RING_FRAMES * LWASAPI_MAX_CHANNELS];
    _Alignas(64) float cap_pcm_data[LWASAPI_RING_FRAMES * 2];
} lwasapi_shm_ring_t;

/* ============================================================================
 * Control Socket IPC Messages (Used only during session setup/control)
 * ============================================================================ */

/* Rev 6.1 Constants & Enumerations */

#define LWASAPI_FILTER_PEAK        0U
#define LWASAPI_FILTER_LOW_SHELF   1U
#define LWASAPI_FILTER_HIGH_SHELF  2U

#define LWASAPI_BT_PREF_AUTO       0U
#define LWASAPI_BT_PREF_LDAC       1U
#define LWASAPI_BT_PREF_APTX_HD    2U
#define LWASAPI_BT_PREF_APTX       3U
#define LWASAPI_BT_PREF_AAC        4U
#define LWASAPI_BT_PREF_SBC        5U

#define LWASAPI_LDAC_MODE_ADAPTIVE 0U
#define LWASAPI_LDAC_MODE_990      1U
#define LWASAPI_LDAC_MODE_660      2U
#define LWASAPI_LDAC_MODE_330      3U

#define LWASAPI_BT_CAP_LDAC        (1U << 0)
#define LWASAPI_BT_CAP_APTX_HD     (1U << 1)
#define LWASAPI_BT_CAP_APTX        (1U << 2)
#define LWASAPI_BT_CAP_AAC         (1U << 3)
#define LWASAPI_BT_CAP_SBC         (1U << 4)

/* Rev 6.2 Hardware Analog Architecture, DAC Unity & Amp Stage Constants */
#define LWASAPI_HW_AMP_AUTO        0U /* Preserve card firmware/existing state (default) */
#define LWASAPI_HW_AMP_LOW         1U /* Low Gain / IEM (<=32 Ohm) */
#define LWASAPI_HW_AMP_MED         2U /* Medium Gain (64-150 Ohm) */
#define LWASAPI_HW_AMP_HIGH        3U /* High Gain (300-600 Ohm Discrete Amp / TPA6120A2) */

#define LWASAPI_HW_ROUTE_AUTO      0U /* Preserve firmware/jack routing */
#define LWASAPI_HW_ROUTE_HP        1U /* Analog Headphones / Front HP Amp */
#define LWASAPI_HW_ROUTE_SPK       2U /* Analog Line Out / Rear Speakers */

#define LWASAPI_HW_CAP_DAC_VOL     (1U << 0) /* ALSA DAC playback mixer (Master/PCM/Front/HP) */
#define LWASAPI_HW_CAP_HP_AMP      (1U << 1) /* ALSA discrete HP Amp / Impedance control */
#define LWASAPI_HW_CAP_OUT_ROUTE   (1U << 2) /* ALSA Analog Output / Route selector */
#define LWASAPI_HW_CAP_MIC_BOOST   (1U << 3) /* ALSA Mic Boost / Capture preamp control */
#define LWASAPI_HW_CAP_LINE_AIR    (1U << 4) /* Pro interface Air / Autogain / Phantom controls */
#define LWASAPI_HW_CAP_HID_BOX     (1U << 5) /* ASUS Strix Control Box / USB HID node detected */
#define LWASAPI_HW_CAP_STRIX_ARCH  (1U << 6) /* ASUS Strix Raid DLX / Soar (CM6632AE+ES9016+TPA6120A2) */
#define LWASAPI_HW_CAP_HIGH_RES    (1U << 7) /* Native 192 kHz / S32_LE / S24_3LE enthusiast DAC */

typedef struct {
    float    band_freq_hz[LWASAPI_EQ_BANDS]; /* Per-band center frequency Hz (20..20000) */
    uint32_t band_type[LWASAPI_EQ_BANDS];    /* LWASAPI_FILTER_PEAK / LOW_SHELF / HIGH_SHELF */
    uint32_t crossfeed_enabled;              /* 0 = Bypass, 1 = Bauer/BS2B Headphone Crossfeed */
    float    crossfeed_level;                /* 0.10 .. 1.00 (default 0.45) */
    uint32_t crossfeed_cutoff_hz;            /* 300 .. 1500 Hz (default 700) */
    uint32_t reserved;                       /* Zero-padded to 96 bytes */
} lwasapi_param_eq_ext_t;

typedef struct {
    uint32_t enabled;          /* 0 = Raw mic pass-through, 1 = Capture APO enabled */
    uint32_t hpf_enabled;      /* 1 = Voice High-Pass Filter */
    float    hpf_hz;           /* 60.0 .. 200.0 Hz (default 95.0 Hz) */
    uint32_t gate_enabled;     /* 1 = Hysteresis Noise Gate */
    float    gate_thresh_db;   /* -72.0 .. -12.0 dBFS (default -48.0 dB) */
    uint32_t rnnoise_enabled;  /* 1 = Neural RNNoise suppression (+ spectral fallback) */
    uint32_t comp_enabled;     /* 1 = Voice Clarity EQ + Soft Compressor */
    float    comp_thresh_db;   /* -36.0 .. -4.0 dBFS (default -18.0 dB) */
    float    comp_ratio;       /* 1.0 .. 8.0 (default 2.8) */
    float    clarity_db;       /* 0.0 .. +9.0 dB vocal presence boost (default 2.5 dB) */
    uint32_t sidetone_enabled; /* 1 = Low-latency headphone sidetone monitor */
    uint32_t sidetone_pct;     /* 0 .. 100% sidetone mix */
} lwasapi_mic_apo_cfg_t;

typedef struct {
    uint32_t preferred_codec;       /* LWASAPI_BT_PREF_* */
    uint32_t ldac_mode;             /* LWASAPI_LDAC_MODE_* */
    uint32_t wireless_abr_enabled;  /* 1 = Adaptive Wireless Bitrate (LDAC+AAC) */
    uint32_t effective_kbps;        /* Live negotiated/adapted bitrate (kbps) */
    uint32_t supported_codecs_mask; /* LWASAPI_BT_CAP_* peer/host bitmask */
    uint32_t hfp_sco_enabled;       /* 0 = Off (preserve Scarlett default), 1 = Enable SCO mic */
    uint32_t hfp_sco_active;        /* 1 = BTPROTO_SCO active */
    char     bt_sco_codec[8];       /* "mSBC" / "CVSD" / "" */
    char     neg_reason[64];        /* Negotiation & codec diagnostics string */
} lwasapi_bt_codec_cfg_t;

typedef struct {
    lwasapi_eq_config_t    eq;
    lwasapi_param_eq_ext_t ext;
} lwasapi_param_eq_payload_t;

typedef struct {
    uint32_t hw_dac_unity_vol;   /* 1 = Drive ALSA hardware DAC volume, keep PCM digital 1.0x unity */
    uint32_t hp_amp_tier;        /* LWASAPI_HW_AMP_* (0=Auto/Preserve -> untouched!) */
    uint32_t output_route;       /* LWASAPI_HW_ROUTE_* (0=Preserve) */
    uint32_t unlock_0db_rails;   /* 1 = Unmute & normalize ALSA DAC playback switches @ 0 dB */
    float    analog_boost_db;    /* 0.0 .. +12.0 dB clean analog-modeled amp headroom */
    float    mic_hw_boost_db;    /* 0.0 .. +24.0 dB ALSA Mic Boost + clean capture preamp makeup */
} lwasapi_hw_analog_cfg_t;

typedef struct {
    uint32_t caps_mask;          /* LWASAPI_HW_CAP_* discovered card capabilities */
    uint32_t dac_unity_active;   /* 1 = Hardware ALSA DAC attenuator driving output volume */
    uint32_t hid_box_connected;  /* 1 = Strix Raid DLX / Soar USB HID control box node detected */
    int32_t  active_pb_card;     /* Active ALSA playback card index or -1 */
    int32_t  active_cap_card;    /* Active ALSA capture card index or -1 */
    uint32_t reserved;           /* Active/configured capture HW sample rate Hz (0 = follow master) */
    char     card_arch_name[64]; /* Detected card/DAC/amp architecture label */
    char     dac_ctl_name[32];   /* Matched ALSA playback DAC mixer elem */
    char     amp_ctl_name[32];   /* Matched ALSA amp/route elem or UAC status */
    char     cap_ctl_name[32];   /* Matched ALSA Mic Boost/Capture elem */
    char     diag_note[64];      /* Live hardware boost/analog telemetry note */
} lwasapi_hw_analog_info_t;

typedef enum {
    LWASAPI_CMD_CREATE_SESSION      = 1,
    LWASAPI_CMD_DESTROY_SESSION     = 2,
    LWASAPI_CMD_GET_STATUS          = 3,
    LWASAPI_CMD_SET_VOLUME          = 4,
    LWASAPI_CMD_INJECT_MIDI         = 5,
    LWASAPI_CMD_SET_ENGINE_CONFIG   = 6,
    LWASAPI_CMD_SET_EQ              = 7,
    LWASAPI_CMD_LIST_DEVICES        = 8,
    LWASAPI_CMD_SET_DEVICES         = 9,
    LWASAPI_CMD_SET_SESSION_CONTROL = 10,
    LWASAPI_CMD_SET_PARAM_EQ        = 11,
    LWASAPI_CMD_SET_MIC_APO         = 12,
    LWASAPI_CMD_SET_BT_CODEC        = 14,
    LWASAPI_CMD_SET_HW_ANALOG       = 15
} lwasapi_cmd_type_t;

#define LWASAPI_MAX_DEVICES 32

typedef struct {
    char     id[64];          /* e.g. "bluez:00:11:22:33:44:55", "hw:3,0", "virtual" */
    char     name[64];        /* Friendly device name */
    char     driver[32];      /* "bluez-a2dp", "alsa-kernel", "virtual" */
    uint32_t is_capture;      /* 0 = playback sink, 1 = capture source */
    uint32_t is_bluetooth;    /* 1 = BlueZ A2DP, 0 = wired/virtual */
    uint32_t is_active;       /* 1 = active engine target */
    uint32_t card_index;
    uint32_t device_index;
    uint32_t min_rate;
    uint32_t max_rate;
} lwasapi_device_entry_t;

typedef struct {
    uint32_t num_devices;
    lwasapi_device_entry_t devices[LWASAPI_MAX_DEVICES];
} lwasapi_device_list_t;

#define LWASAPI_CFG_FLAG_CAPTURE_RATE 0x43415052U /* 'CAPR': independent capture sample rate in SET_ENGINE_CONFIG */

typedef struct {
    uint32_t cmd;
    uint32_t client_pid;
    char     client_name[64];
    uint32_t share_mode;
    uint32_t stream_flags;
    uint32_t sample_rate;
    uint32_t channels;
    uint32_t format;
    uint32_t requested_buffer_frames;
    uint32_t requested_period_frames;
    uint32_t target_session_id;
    uint32_t volume_milli;
    lwasapi_midi_event_t midi_ev;
    lwasapi_eq_config_t  eq_cfg;
    char     target_device[64];
    char     target_capture_device[64];
} lwasapi_ipc_req_t;

typedef struct {
    int32_t  hr;                         /* HRESULT status */
    uint32_t session_id;
    uint32_t actual_sample_rate;
    uint32_t actual_channels;
    uint32_t actual_buffer_frames;
    uint32_t actual_period_frames;
} lwasapi_ipc_resp_t;

typedef struct {
    uint32_t session_id;
    uint32_t client_pid;
    char     client_name[64];
    uint32_t share_mode;
    uint32_t is_capture;
    uint32_t sample_rate;
    uint32_t channels;
    uint32_t active;
    uint32_t volume_milli;
    uint64_t buffered_frames;
    uint64_t xruns;
    uint32_t muted;
    uint32_t routing_flags;
    char     target_device[64];
} lwasapi_session_info_t;

typedef struct {
    char     hw_device[64];
    char     hw_capture_device[64];
    char     midi_device[64];
    uint32_t hw_sample_rate;
    uint32_t hw_channels;
    uint32_t hw_period_frames;
    uint32_t hw_buffer_frames;
    uint32_t hw_format_bits;
    uint32_t desktop_volume_pct;
    uint32_t desktop_muted;
    uint32_t desktop_mic_volume_pct;
    uint32_t desktop_mic_muted;
    uint32_t realtime_priority;
    int32_t  exclusive_owner_id;
    uint64_t total_cycles;
    uint64_t hw_xruns;
    uint64_t midi_events_routed;
    lwasapi_eq_config_t eq_cfg;
    float    eq_band_levels_db[LWASAPI_EQ_BANDS];
    float    eq_dyn_delta_db[LWASAPI_EQ_BANDS];
    float    output_peak_l;
    float    output_peak_r;
    uint32_t num_sessions;
    lwasapi_session_info_t sessions[LWASAPI_MAX_SESSIONS];
    uint32_t bt_active;
    uint32_t bt_delay_ms;
    char     bt_alias[64];
    char     bt_codec[16];
    uint32_t pa_server_running;
    uint32_t pa_clients;
    char     active_playback_id[64];
    char     active_capture_id[64];

    /* Rev 6.1+ Extensions */
    lwasapi_param_eq_ext_t param_eq;                    /* Per-band freq/type + Crossfeed */
    lwasapi_mic_apo_cfg_t  mic_apo;                     /* Capture Microphone APO settings */
    lwasapi_bt_codec_cfg_t bt_codec_cfg;                /* Codec Switcher / LDAC ABR / HFP SCO telemetry */
    float                  mic_apo_peak_in;             /* Pre-APO mic peak */
    float                  mic_apo_peak_out;            /* Post-APO mic peak */
    uint32_t               mic_rnnoise_active;          /* 1 if librnnoise.so.0 loaded */

    /* Rev 6.2 Hardware Analog Architecture, DAC Unity & Amp Telemetry */
    lwasapi_hw_analog_cfg_t  hw_analog_cfg;
    lwasapi_hw_analog_info_t hw_analog_info;
} lwasapi_engine_status_t;

/* ============================================================================
 * Public Windows-Style Client API (liblwasapi.so)
 * ============================================================================ */

typedef struct IAudioClient IAudioClient;

HRESULT LWASAPI_ActivateAudioClient(const char *client_name, IAudioClient **ppAudioClient);
HRESULT IAudioClient_Initialize(
    IAudioClient *client,
    AUDCLNT_SHAREMODE ShareMode,
    uint32_t StreamFlags,
    uint32_t BufferFrameCount,
    uint32_t PeriodicityFrames,
    const WAVEFORMATEX *pFormat
);
HRESULT IAudioClient_GetMixFormat(IAudioClient *client, WAVEFORMATEX *pFormat);
HRESULT IAudioClient_GetBufferSize(IAudioClient *client, uint32_t *pNumBufferFrames);
HRESULT IAudioClient_GetDevicePeriod(IAudioClient *client, uint32_t *pDefaultPeriodFrames, uint32_t *pMinimumPeriodFrames);
HRESULT IAudioClient_GetCurrentPadding(IAudioClient *client, uint32_t *pNumPaddingFrames);
HRESULT IAudioClient_SetEventHandle(IAudioClient *client, int *pEventFdOut);
HRESULT IAudioClient_Start(IAudioClient *client);
HRESULT IAudioClient_Stop(IAudioClient *client);
HRESULT IAudioClient_Reset(IAudioClient *client);
HRESULT IAudioClient_Release(IAudioClient *client);

/* IAudioRenderClient Equivalent (Playback) */
HRESULT IAudioRenderClient_GetBuffer(IAudioClient *client, uint32_t NumFramesRequested, float **ppData);
HRESULT IAudioRenderClient_ReleaseBuffer(IAudioClient *client, uint32_t NumFramesWritten, uint32_t dwFlags);
HRESULT IAudioRenderClient_WriteFrames(IAudioClient *client, const void *src, uint32_t frames, lwasapi_sample_format_t src_fmt);

/* IAudioCaptureClient Equivalent (Microphone / Line-In / Loopback Capture) */
HRESULT IAudioCaptureClient_GetNextPacketSize(IAudioClient *client, uint32_t *pNumFramesInNextPacket);
HRESULT IAudioCaptureClient_ReadFrames(IAudioClient *client, void *dst, uint32_t frames, lwasapi_sample_format_t dst_fmt);

HRESULT IAudioClient_WaitForPeriod(IAudioClient *client, int timeout_ms);

/* ISimpleAudioVolume Equivalent */
HRESULT ISimpleAudioVolume_SetMasterVolume(IAudioClient *client, float fLevel);
HRESULT ISimpleAudioVolume_GetMasterVolume(IAudioClient *client, float *pfLevel);

/* Real-Time Lock-Free MIDI API for DAWs & Synths */
uint32_t IAudioClient_ReadMidiEvents(IAudioClient *client, lwasapi_midi_event_t *events_out, uint32_t max_events);
HRESULT  IAudioClient_WriteMidiEvent(IAudioClient *client, const lwasapi_midi_event_t *ev);

/* Salsa (Linux WASAPI) Engine Configuration & 10-Band Real-Time Dynamic EQ API */
HRESULT  LWASAPI_GetEngineStatus(lwasapi_engine_status_t *out_status);
HRESULT  LWASAPI_SetEngineConfig(uint32_t sample_rate, uint32_t period_frames, uint32_t format_bits);
HRESULT  LWASAPI_SetCaptureSampleRate(uint32_t sample_rate);
HRESULT  LWASAPI_SetDynamicEQ(const lwasapi_eq_config_t *eq_cfg);
HRESULT  LWASAPI_ListDevices(lwasapi_device_list_t *out_list);
HRESULT  LWASAPI_SetDevices(const char *playback_id, const char *capture_id);
HRESULT  LWASAPI_SetSessionControl(uint32_t session_id, int32_t vol_pct, int32_t mute_mode, const char *target_device);
HRESULT  LWASAPI_SetParametricEQ(const lwasapi_eq_config_t *eq_cfg, const lwasapi_param_eq_ext_t *param_ext);
HRESULT  LWASAPI_SetMicAPO(const lwasapi_mic_apo_cfg_t *mic_cfg);
HRESULT  LWASAPI_SetBtCodecConfig(const lwasapi_bt_codec_cfg_t *bt_cfg);
HRESULT  LWASAPI_SetHwAnalog(const lwasapi_hw_analog_cfg_t *hw_cfg);

#ifdef __cplusplus
}
#endif

#endif /* LWASAPI_H */
