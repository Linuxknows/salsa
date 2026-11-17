#define _GNU_SOURCE
#include "../include/lwasapi.h"
#include "../include/lwasapi_bluez.h"
#include "../include/lwasapi_midi.h"
#include "../include/lwasapi_pa_server.h"
#include "../include/lwasapi_pw_broker.h"
#include "../include/lwasapi_debug.h"
#include "../include/lwasapi_paths.h"
#include "../include/lwasapi_hw_analog.h"
#include "../include/salsa_wire.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <dlfcn.h>
#include <errno.h>
#include <math.h>
#include <time.h>
#include <signal.h>
#include <sched.h>
#include <pthread.h>
#include <poll.h>
#include <sys/stat.h>
#include <sys/mman.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/eventfd.h>
#include <sound/asound.h>

/* ============================================================================
 * Direct Linux Kernel /dev/snd/pcm*, BlueZ A2DP & MIDI Hub WaveRT Backend
 * ============================================================================ */

typedef struct {
    int      fd;
    char     device_path[64];
    bool     is_virtual;
    bool     is_bluetooth;
    uint32_t sample_rate;
    uint32_t channels;
    uint32_t period_frames;
    uint32_t buffer_frames;
    int      sndrv_format;
    uint32_t format_bits;
} kernel_wavert_hw_t;

typedef struct {
    bool               in_use;
    uint32_t           session_id;
    int                client_sock;
    int                shm_fd;
    int                event_fd;
    lwasapi_shm_ring_t *ring;
    double             resample_phase;
    uint32_t           sample_rate;
    uint32_t           channels;
    bool               is_capture;
    bool               is_loopback;
    bool               stream_live;
    bool               underrun_pending;
    uint64_t           last_audio_ns;
} engine_session_t;

static volatile sig_atomic_t g_running = 1;
static pthread_mutex_t       g_sessions_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t       g_hw_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t       g_hw_io_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t       g_reconf_lock = PTHREAD_MUTEX_INITIALIZER;
static engine_session_t      g_sessions[LWASAPI_MAX_SESSIONS];
static kernel_wavert_hw_t    g_hw;
static lwasapi_bluez_t       g_bt;
static lwasapi_midi_hub_t    g_midi;
static lwasapi_pa_server_t  *g_pa_srv = NULL;
static lwasapi_pw_broker_t  *g_pw_brk = NULL;
static char                  g_target_playback_id[64] = "auto";
static char                  g_target_capture_id[64]  = "auto";
static char                  g_active_playback_id[64] = "virtual";
static char                  g_active_capture_id[64]  = "none";
static uint32_t              g_pref_sample_rate = 192000;
static uint32_t              g_pref_cap_sample_rate = 0; /* 0 = follow master engine rate */
static uint32_t              g_pref_period_frames = 512;
static uint32_t              g_pref_format_bits = 32;
static uint32_t              g_next_session_id = 1;
static int32_t               g_exclusive_session_id = -1;
static uint32_t              g_rt_priority = 0;
static _Atomic uint64_t      g_total_cycles = 0;
static _Atomic uint64_t      g_hw_xruns = 0;
static _Atomic uint64_t      g_cap_read_cursor = 0;

static inline uint32_t resolve_cap_sample_rate(uint32_t engine_rate, uint32_t pref_cap_rate) {
    if (pref_cap_rate >= 44100U && pref_cap_rate <= 192000U) {
        return pref_cap_rate;
    }
    return engine_rate ? engine_rate : 48000U;
}

/* ============================================================================
 * Salsa (Linux WASAPI) 10-Band Full Parametric EQ, Crossfeed & Mic APO
 * ============================================================================ */

#define LWASAPI_MAX_PERIOD 4096U

static const float k_eq_freqs[LWASAPI_EQ_BANDS] = {
    31.25f, 62.5f, 125.0f, 250.0f, 500.0f,
    1000.0f, 2000.0f, 4000.0f, 8000.0f, 16000.0f
};

typedef struct {
    double   b0, b1, b2, a1, a2;
    double   z1[LWASAPI_MAX_CHANNELS];
    double   z2[LWASAPI_MAX_CHANNELS];
    uint32_t last_rate;
    uint32_t last_type;
    float    last_freq_hz;
    float    last_q;
    float    last_gain_db;
    bool     coeffs_valid;
} biquad_filter_t;

static lwasapi_eq_config_t    g_eq_cfg;
static lwasapi_param_eq_ext_t g_param_eq;
static lwasapi_mic_apo_cfg_t  g_mic_apo;
static lwasapi_bt_codec_cfg_t g_saved_bt_cfg;
static lwasapi_hw_analog_ctx_t *g_hw_analog = NULL;
static lwasapi_hw_analog_cfg_t  g_hw_analog_cfg;
static bool                     g_has_saved_vol_state = false;
static uint32_t                 g_saved_desktop_vol_pct = 100U;
static uint32_t                 g_saved_desktop_muted = 0U;
static uint32_t                 g_saved_mic_vol_pct = 100U;
static uint32_t                 g_saved_mic_muted = 0U;
static uint32_t                 g_saved_mic_gain_ppm = 1000000U;
static _Atomic bool             g_state_dirty = false;
static _Atomic uint64_t         g_state_dirty_ns = 0ULL;

static void mark_salsa_state_dirty(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    uint64_t now_ns = (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
    atomic_store_explicit(&g_state_dirty_ns, now_ns, memory_order_release);
    atomic_store_explicit(&g_state_dirty, true, memory_order_release);
}

static int parse_card_idx_from_id(const char *id) {
    unsigned c = 0, d = 0;
    if (!id || !id[0]) return -1;
    if (sscanf(id, "hw:%u,%u", &c, &d) >= 1) return (int)c;
    if (sscanf(id, "/dev/snd/pcmC%u", &c) == 1) return (int)c;
    return -1;
}

static void hw_analog_probe_and_sync(bool re_probe) {
    if (!g_hw_analog) return;
    int pb_card = -1, cap_card = -1;
    pthread_mutex_lock(&g_hw_lock);
    if (!g_hw.is_virtual && !g_hw.is_bluetooth) {
        pb_card = parse_card_idx_from_id(g_active_playback_id);
        if (pb_card < 0) pb_card = parse_card_idx_from_id(g_hw.device_path);
    }
    cap_card = parse_card_idx_from_id(g_active_capture_id);
    pthread_mutex_unlock(&g_hw_lock);

    lwasapi_hw_analog_cfg_t cfg;
    pthread_mutex_lock(&g_sessions_lock);
    cfg = g_hw_analog_cfg;
    pthread_mutex_unlock(&g_sessions_lock);

    uint32_t d_vol   = __atomic_load_n(&g_bt.desktop_volume_pct, __ATOMIC_RELAXED);
    bool     d_muted = __atomic_load_n(&g_bt.desktop_muted, __ATOMIC_RELAXED);
    uint32_t m_vol   = __atomic_load_n(&g_bt.desktop_mic_volume_pct, __ATOMIC_RELAXED);
    bool     m_muted = __atomic_load_n(&g_bt.desktop_mic_muted, __ATOMIC_RELAXED);

    if (re_probe) {
        lwasapi_hw_analog_probe(g_hw_analog, pb_card, cap_card);
    }
    lwasapi_hw_analog_sync(g_hw_analog, &cfg, d_vol, d_muted, m_vol, m_muted);
}
static biquad_filter_t        g_eq_filters[LWASAPI_EQ_BANDS];
static biquad_filter_t        g_spec_filters[LWASAPI_EQ_BANDS];
static float                  g_spec_last_freqs[LWASAPI_EQ_BANDS];
static uint32_t               g_spec_last_rate = 0;
static float                  g_band_env[LWASAPI_EQ_BANDS];
static float                  g_eq_band_levels_db[LWASAPI_EQ_BANDS];
static float                  g_eq_dyn_delta_db[LWASAPI_EQ_BANDS];
static float                  g_output_peak_l = 0.0f;
static float                  g_output_peak_r = 0.0f;

/* Crossfeed state */
static double   g_xfeed_lp_l = 0.0;
static double   g_xfeed_lp_r = 0.0;
static float    g_xfeed_dly_l[64];
static float    g_xfeed_dly_r[64];
static uint32_t g_xfeed_dly_idx = 0;

/* Capture Mic APO telemetry & DSP state */
static float           g_mic_apo_peak_in = 0.0f;
static float           g_mic_apo_peak_out = 0.0f;
static uint32_t        g_mic_rnnoise_active = 0U;
static biquad_filter_t g_mic_hpf_flt;
static biquad_filter_t g_mic_demud_flt;
static biquad_filter_t g_mic_pres_flt;
static biquad_filter_t g_mic_clarity_flt;
static float           g_mic_gate_env = 0.0f;
static float           g_mic_gate_gain = 1.0f;
static float           g_mic_comp_env = 0.0f;
static float           g_mic_comp_gain = 1.0f;
static float           g_mic_vad_env = 0.0f;
static uint32_t        g_mic_vad_hold_samples = 0U;
static float           g_mic_silence_gain = 1.0f;
static double          g_mic_dc_x1 = 0.0;
static double          g_mic_dc_y1 = 0.0;

#define MIC_RN_FRAME 480U
#define MIC_RN_CAP   8192U
typedef struct {
    void  *lib;
    void *(*create_fn)(void *model);
    void  (*destroy_fn)(void *st);
    float (*process_fn)(void *st, float *out, const float *in);
    void  *st;
    bool   probed;
    bool   available;

    float    in_ring[MIC_RN_CAP];
    uint32_t in_count;
    float    out_ring[MIC_RN_CAP];
    float    dry_ring[MIC_RN_CAP];
    float    vad_ring[MIC_RN_CAP];
    uint32_t out_rd;
    uint32_t out_wr;
    float    in_last;
    float    out_last;
    float    dry_last;
    float    vad_last;
    double   in_phase;
    double   out_phase;
    uint32_t last_rate;
} mic_rnnoise_ctx_t;

static mic_rnnoise_ctx_t g_mic_rn;

typedef struct {
    uint32_t            magic;
    uint32_t            sample_rate;
    uint32_t            period_frames;
    uint32_t            format_bits;
    lwasapi_eq_config_t eq_cfg;
} salsa_persisted_state_v1_t;

typedef struct {
    uint32_t            magic;
    uint32_t            sample_rate;
    uint32_t            period_frames;
    uint32_t            format_bits;
    lwasapi_eq_config_t eq_cfg;
    char                target_playback_id[64];
    char                target_capture_id[64];
} salsa_persisted_state_t;

typedef struct {
    uint32_t               magic; /* 0x53414C55 ('SALU') */
    uint32_t               sample_rate;
    uint32_t               period_frames;
    uint32_t               format_bits;
    lwasapi_eq_config_t    eq_cfg;
    char                   target_playback_id[64];
    char                   target_capture_id[64];
    uint32_t               reserved_0;
    uint32_t               reserved_1;
    lwasapi_param_eq_ext_t param_eq;
    lwasapi_mic_apo_cfg_t  mic_apo;
    lwasapi_bt_codec_cfg_t bt_cfg;
} salsa_persisted_state_v3_t;

typedef struct {
    uint32_t                magic; /* 0x53414C56 ('SALV') */
    uint32_t                sample_rate;
    uint32_t                period_frames;
    uint32_t                format_bits;
    lwasapi_eq_config_t     eq_cfg;
    char                    target_playback_id[64];
    char                    target_capture_id[64];
    uint32_t                reserved_0;
    uint32_t                reserved_1;
    lwasapi_param_eq_ext_t  param_eq;
    lwasapi_mic_apo_cfg_t   mic_apo;
    lwasapi_bt_codec_cfg_t  bt_cfg;
    lwasapi_hw_analog_cfg_t hw_analog_cfg;
} salsa_persisted_state_v4_t;

typedef struct {
    uint32_t                magic; /* 0x53414C57 ('SALW') */
    uint32_t                sample_rate;
    uint32_t                period_frames;
    uint32_t                format_bits;
    lwasapi_eq_config_t     eq_cfg;
    char                    target_playback_id[64];
    char                    target_capture_id[64];
    uint32_t                reserved_0;
    uint32_t                reserved_1;
    lwasapi_param_eq_ext_t  param_eq;
    lwasapi_mic_apo_cfg_t   mic_apo;
    lwasapi_bt_codec_cfg_t  bt_cfg;
    lwasapi_hw_analog_cfg_t hw_analog_cfg;
    uint32_t                desktop_volume_pct;
    uint32_t                desktop_muted;
    uint32_t                desktop_mic_volume_pct;
    uint32_t                desktop_mic_muted;
    uint32_t                desktop_mic_gain_ppm;
} salsa_persisted_state_v5_t;

static void get_salsa_state_path(char *out, size_t out_len) {
    const char *home = getenv("HOME");
    if (!home) home = "/tmp";
    char dir[256];
    snprintf(dir, sizeof(dir), "%s/.config/lwasapi", home);
    mkdir(dir, 0700);
    chmod(dir, 0700);
    snprintf(out, out_len, "%s/salsa_state.bin", dir);
}

static void save_salsa_state(void) {
    atomic_store_explicit(&g_state_dirty, false, memory_order_release);
    salsa_persisted_state_v5_t st;
    memset(&st, 0, sizeof(st));
    st.magic = 0x53414C57U; /* 'SALW' v5 (v6.3-beta) */

    pthread_mutex_lock(&g_hw_lock);
    st.sample_rate   = g_pref_sample_rate ? g_pref_sample_rate : g_hw.sample_rate;
    st.period_frames = g_pref_period_frames ? g_pref_period_frames : g_hw.period_frames;
    st.format_bits   = (g_pref_format_bits == 16U || g_pref_format_bits == 24U || g_pref_format_bits == 32U)
                       ? g_pref_format_bits : g_hw.format_bits;
    st.reserved_0    = (g_pref_cap_sample_rate >= 44100U && g_pref_cap_sample_rate <= 192000U)
                       ? g_pref_cap_sample_rate : 0U;
    st.reserved_1    = 0U;
    snprintf(st.target_playback_id, sizeof(st.target_playback_id), "%s",
             g_target_playback_id[0] ? g_target_playback_id : "auto");
    snprintf(st.target_capture_id, sizeof(st.target_capture_id), "%s",
             g_target_capture_id[0] ? g_target_capture_id : "auto");
    pthread_mutex_unlock(&g_hw_lock);

    pthread_mutex_lock(&g_sessions_lock);
    st.eq_cfg        = g_eq_cfg;
    st.param_eq      = g_param_eq;
    st.mic_apo       = g_mic_apo;
    st.hw_analog_cfg = g_hw_analog_cfg;
    pthread_mutex_unlock(&g_sessions_lock);

    if (g_bt.initialized) {
        lwasapi_bluez_get_codec_config(&g_bt, &st.bt_cfg);
        g_saved_bt_cfg = st.bt_cfg;
        g_saved_desktop_vol_pct = __atomic_load_n(&g_bt.desktop_volume_pct, __ATOMIC_RELAXED);
        g_saved_desktop_muted   = __atomic_load_n(&g_bt.desktop_muted, __ATOMIC_RELAXED) ? 1U : 0U;
        g_saved_mic_vol_pct     = __atomic_load_n(&g_bt.desktop_mic_volume_pct, __ATOMIC_RELAXED);
        g_saved_mic_muted       = __atomic_load_n(&g_bt.desktop_mic_muted, __ATOMIC_RELAXED) ? 1U : 0U;
        g_saved_mic_gain_ppm    = __atomic_load_n(&g_bt.desktop_mic_gain_ppm, __ATOMIC_RELAXED);
        g_has_saved_vol_state   = true;
    } else {
        st.bt_cfg = g_saved_bt_cfg;
    }
    st.desktop_volume_pct     = g_saved_desktop_vol_pct;
    st.desktop_muted          = g_saved_desktop_muted;
    st.desktop_mic_volume_pct = g_saved_mic_vol_pct;
    st.desktop_mic_muted      = g_saved_mic_muted;
    st.desktop_mic_gain_ppm   = g_saved_mic_gain_ppm;

    char path[512], tmp[540];
    get_salsa_state_path(path, sizeof(path));
    snprintf(tmp, sizeof(tmp), "%s.tmp.%d", path, (int)getpid());
    unlink(tmp);
    int fd = open(tmp, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
    if (fd >= 0) {
        FILE *fp = fdopen(fd, "wb");
        if (fp) {
            size_t nw = fwrite(&st, sizeof(st), 1, fp);
            fflush(fp);
            fsync(fileno(fp));
            fclose(fp);
            if (nw == 1) {
                if (rename(tmp, path) != 0) unlink(tmp);
            } else {
                unlink(tmp);
            }
        } else {
            close(fd);
            unlink(tmp);
        }
    }
}

static void init_default_eq(void) {
    memset(&g_eq_cfg, 0, sizeof(g_eq_cfg));
    g_eq_cfg.enabled = 1;
    g_eq_cfg.dynamic_mode = 0; /* Clean Static Parametric EQ default (transparent, zero pumping) */
    g_eq_cfg.preamp_db = 0.0f;
    g_eq_cfg.dyn_threshold_db = -16.0f;
    g_eq_cfg.dyn_ratio = 2.2f;
    g_eq_cfg.limiter_enabled = 1;

    memset(&g_param_eq, 0, sizeof(g_param_eq));
    for (int i = 0; i < LWASAPI_EQ_BANDS; i++) {
        g_eq_cfg.band_gains_db[i] = 0.0f;
        g_eq_cfg.band_q[i] = 1.414f;
        g_param_eq.band_freq_hz[i] = k_eq_freqs[i];
        g_param_eq.band_type[i] = LWASAPI_FILTER_PEAK;
        g_eq_band_levels_db[i] = -60.0f;
        g_eq_dyn_delta_db[i] = 0.0f;
    }
    g_param_eq.crossfeed_enabled = 0;
    g_param_eq.crossfeed_level = 0.45f;
    g_param_eq.crossfeed_cutoff_hz = 700;
    snprintf(g_eq_cfg.preset_name, sizeof(g_eq_cfg.preset_name), "Flat / Studio Reference");

    memset(&g_mic_apo, 0, sizeof(g_mic_apo));
    g_mic_apo.enabled = 0; /* Raw Scarlett / mic pass-through unless enabled */
    g_mic_apo.hpf_enabled = 1;
    g_mic_apo.hpf_hz = 80.0f;
    g_mic_apo.gate_enabled = 1;
    g_mic_apo.gate_thresh_db = -50.0f;
    g_mic_apo.rnnoise_enabled = 1;
    g_mic_apo.comp_enabled = 1;
    g_mic_apo.comp_thresh_db = -15.0f;
    g_mic_apo.comp_ratio = 2.2f;
    g_mic_apo.clarity_db = 1.5f;
    g_mic_apo.sidetone_enabled = 0;
    g_mic_apo.sidetone_pct = 0;

    memset(&g_saved_bt_cfg, 0, sizeof(g_saved_bt_cfg));
    g_saved_bt_cfg.preferred_codec = LWASAPI_BT_PREF_AUTO;
    g_saved_bt_cfg.ldac_mode = LWASAPI_LDAC_MODE_ADAPTIVE;
    g_saved_bt_cfg.wireless_abr_enabled = 1;
    g_saved_bt_cfg.hfp_sco_enabled = 0;
}

static void clamp_param_eq_ext(lwasapi_param_eq_ext_t *ext) {
    if (!ext) return;
    for (int i = 0; i < LWASAPI_EQ_BANDS; i++) {
        if (!(ext->band_freq_hz[i] >= 20.0f && ext->band_freq_hz[i] <= 20000.0f)) {
            ext->band_freq_hz[i] = k_eq_freqs[i];
        }
        if (ext->band_type[i] > LWASAPI_FILTER_HIGH_SHELF) {
            ext->band_type[i] = LWASAPI_FILTER_PEAK;
        }
    }
    ext->crossfeed_enabled = ext->crossfeed_enabled ? 1U : 0U;
    if (!(ext->crossfeed_level >= 0.05f && ext->crossfeed_level <= 1.0f)) {
        ext->crossfeed_level = 0.45f;
    }
    if (ext->crossfeed_cutoff_hz < 300U || ext->crossfeed_cutoff_hz > 1500U) {
        ext->crossfeed_cutoff_hz = 700U;
    }
}

static void clamp_mic_apo_cfg(lwasapi_mic_apo_cfg_t *m) {
    if (!m) return;
    m->enabled          = m->enabled ? 1U : 0U;
    m->hpf_enabled      = m->hpf_enabled ? 1U : 0U;
    m->gate_enabled     = m->gate_enabled ? 1U : 0U;
    m->rnnoise_enabled  = m->rnnoise_enabled ? 1U : 0U;
    m->comp_enabled     = m->comp_enabled ? 1U : 0U;
    m->sidetone_enabled = m->sidetone_enabled ? 1U : 0U;
    if (!(m->hpf_hz >= 40.0f && m->hpf_hz <= 250.0f)) m->hpf_hz = 80.0f;
    if (!(m->gate_thresh_db >= -85.0f && m->gate_thresh_db <= -10.0f)) m->gate_thresh_db = -50.0f;
    if (!(m->comp_thresh_db >= -45.0f && m->comp_thresh_db <= -1.0f)) m->comp_thresh_db = -15.0f;
    if (!(m->comp_ratio >= 1.0f && m->comp_ratio <= 12.0f)) m->comp_ratio = 2.2f;
    if (!(m->clarity_db >= -6.0f && m->clarity_db <= 12.0f)) m->clarity_db = 1.5f;
    if (m->sidetone_pct > 100U) m->sidetone_pct = 100U;
}

static void load_salsa_state(uint32_t *io_rate, uint32_t *io_period, uint32_t *io_bits) {
    init_default_eq();
    lwasapi_hw_analog_default_cfg(&g_hw_analog_cfg);
    g_has_saved_vol_state   = false;
    g_saved_desktop_vol_pct = 100U;
    g_saved_desktop_muted   = 0U;
    g_saved_mic_vol_pct     = 100U;
    g_saved_mic_muted       = 0U;
    g_saved_mic_gain_ppm    = 1000000U;
    g_pref_cap_sample_rate  = 0U;

    char path[512];
    get_salsa_state_path(path, sizeof(path));
    int fd = open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    if (fd < 0) return;
    struct stat fst;
    if (fstat(fd, &fst) != 0 || !S_ISREG(fst.st_mode) || fst.st_size <= 0 ||
        fst.st_size > (off_t)(sizeof(salsa_persisted_state_v5_t) * 2)) {
        close(fd);
        return;
    }
    FILE *fp = fdopen(fd, "rb");
    if (!fp) {
        close(fd);
        return;
    }

    salsa_persisted_state_v5_t st;
    memset(&st, 0, sizeof(st));
    size_t nread = fread(&st, 1, sizeof(st), fp);
    fclose(fp);

    bool valid_v5 = (nread == sizeof(salsa_persisted_state_v5_t) && st.magic == 0x53414C57U);
    bool valid_v4 = (nread >= sizeof(salsa_persisted_state_v4_t) && st.magic == 0x53414C56U);
    bool valid_v3 = (nread >= sizeof(salsa_persisted_state_v3_t) && st.magic == 0x53414C55U);
    bool valid_v2 = (nread >= sizeof(salsa_persisted_state_t) && st.magic == 0x53414C54U);
    bool valid_v1 = (nread >= sizeof(salsa_persisted_state_v1_t) && st.magic == 0x53414C53U);
    if (valid_v5 || valid_v4 || valid_v3 || valid_v2 || valid_v1) {
        if (st.sample_rate >= 44100 && st.sample_rate <= 192000 && io_rate) {
            *io_rate = st.sample_rate;
        }
        if (st.period_frames >= 64 && st.period_frames <= 2048 && io_period) {
            *io_period = st.period_frames;
        }
        if ((st.format_bits == 16 || st.format_bits == 24 || st.format_bits == 32) && io_bits) {
            *io_bits = st.format_bits;
        }
        g_eq_cfg = st.eq_cfg;
        for (int i = 0; i < LWASAPI_EQ_BANDS; i++) {
            if (g_eq_cfg.band_q[i] < 0.2f || g_eq_cfg.band_q[i] > 10.0f) {
                g_eq_cfg.band_q[i] = 1.414f;
            }
        }
        if (valid_v5 || valid_v4 || valid_v3 || valid_v2) {
            if (st.target_playback_id[0]) {
                snprintf(g_target_playback_id, sizeof(g_target_playback_id), "%s", st.target_playback_id);
            }
            if (st.target_capture_id[0]) {
                snprintf(g_target_capture_id, sizeof(g_target_capture_id), "%s", st.target_capture_id);
            }
        }
        if (valid_v5 || valid_v4 || valid_v3) {
            g_param_eq = st.param_eq;
            clamp_param_eq_ext(&g_param_eq);
            g_mic_apo = st.mic_apo;
            clamp_mic_apo_cfg(&g_mic_apo);
            g_saved_bt_cfg = st.bt_cfg;
            if (g_saved_bt_cfg.preferred_codec > LWASAPI_BT_PREF_SBC) {
                g_saved_bt_cfg.preferred_codec = LWASAPI_BT_PREF_AUTO;
            }
            if (g_saved_bt_cfg.ldac_mode > LWASAPI_LDAC_MODE_330) {
                g_saved_bt_cfg.ldac_mode = LWASAPI_LDAC_MODE_ADAPTIVE;
            }
        }
        if (valid_v5 || valid_v4) {
            g_hw_analog_cfg = st.hw_analog_cfg;
            lwasapi_hw_analog_clamp_cfg(&g_hw_analog_cfg);
        }
        if (valid_v5) {
            g_saved_desktop_vol_pct = (st.desktop_volume_pct <= 150U) ? st.desktop_volume_pct : 100U;
            g_saved_desktop_muted   = st.desktop_muted ? 1U : 0U;
            g_saved_mic_vol_pct     = (st.desktop_mic_volume_pct <= 150U) ? st.desktop_mic_volume_pct : 100U;
            g_saved_mic_muted       = st.desktop_mic_muted ? 1U : 0U;
            g_saved_mic_gain_ppm    = (st.desktop_mic_gain_ppm >= 100000U && st.desktop_mic_gain_ppm <= 4000000U)
                                      ? st.desktop_mic_gain_ppm : 1000000U;
            g_pref_cap_sample_rate  = (st.reserved_0 >= 44100U && st.reserved_0 <= 192000U) ? st.reserved_0 : 0U;
            g_has_saved_vol_state   = true;
        }
    }
}

static void compute_parametric_biquad(biquad_filter_t *f, uint32_t ftype,
                                      double freq, double rate, float q_in, float gain_db) {
    if (freq < 10.0) freq = 10.0;
    if (freq >= rate * 0.48) freq = rate * 0.48;
    double q = (q_in < 0.2f) ? 0.2 : (double)q_in;
    double A = pow(10.0, (double)gain_db / 40.0);
    double w0 = 2.0 * M_PI * freq / rate;
    double cos_w0 = cos(w0);
    double sin_w0 = sin(w0);
    double alpha = sin_w0 / (2.0 * q);

    double b0, b1, b2, a0, a1, a2;
    if (ftype == LWASAPI_FILTER_LOW_SHELF) {
        double two_sqrtA_alpha = 2.0 * sqrt(A) * alpha;
        b0 =      A * ((A + 1.0) - (A - 1.0) * cos_w0 + two_sqrtA_alpha);
        b1 =  2.0*A * ((A - 1.0) - (A + 1.0) * cos_w0);
        b2 =      A * ((A + 1.0) - (A - 1.0) * cos_w0 - two_sqrtA_alpha);
        a0 =           (A + 1.0) + (A - 1.0) * cos_w0 + two_sqrtA_alpha;
        a1 = -2.0 *   ((A - 1.0) + (A + 1.0) * cos_w0);
        a2 =           (A + 1.0) + (A - 1.0) * cos_w0 - two_sqrtA_alpha;
    } else if (ftype == LWASAPI_FILTER_HIGH_SHELF) {
        double two_sqrtA_alpha = 2.0 * sqrt(A) * alpha;
        b0 =      A * ((A + 1.0) + (A - 1.0) * cos_w0 + two_sqrtA_alpha);
        b1 = -2.0*A * ((A - 1.0) + (A + 1.0) * cos_w0);
        b2 =      A * ((A + 1.0) + (A - 1.0) * cos_w0 - two_sqrtA_alpha);
        a0 =           (A + 1.0) - (A - 1.0) * cos_w0 + two_sqrtA_alpha;
        a1 =  2.0 *   ((A - 1.0) - (A + 1.0) * cos_w0);
        a2 =           (A + 1.0) - (A - 1.0) * cos_w0 - two_sqrtA_alpha;
    } else {
        b0 = 1.0 + alpha * A;
        b1 = -2.0 * cos_w0;
        b2 = 1.0 - alpha * A;
        a0 = 1.0 + alpha / A;
        a1 = -2.0 * cos_w0;
        a2 = 1.0 - alpha / A;
    }

    double inv_a0 = 1.0 / a0;
    f->b0 = b0 * inv_a0;
    f->b1 = b1 * inv_a0;
    f->b2 = b2 * inv_a0;
    f->a1 = a1 * inv_a0;
    f->a2 = a2 * inv_a0;
}

static void compute_hpf_biquad(biquad_filter_t *f, double freq, double rate) {
    if (freq < 20.0) freq = 20.0;
    if (freq >= rate * 0.45) freq = rate * 0.45;
    double w0 = 2.0 * M_PI * freq / rate;
    double cos_w0 = cos(w0);
    double sin_w0 = sin(w0);
    double alpha = sin_w0 / (2.0 * 0.70710678);
    double b0 =  (1.0 + cos_w0) * 0.5;
    double b1 = -(1.0 + cos_w0);
    double b2 =  (1.0 + cos_w0) * 0.5;
    double a0 =   1.0 + alpha;
    double a1 =  -2.0 * cos_w0;
    double a2 =   1.0 - alpha;
    double inv_a0 = 1.0 / a0;
    f->b0 = b0 * inv_a0;
    f->b1 = b1 * inv_a0;
    f->b2 = b2 * inv_a0;
    f->a1 = a1 * inv_a0;
    f->a2 = a2 * inv_a0;
}

static void compute_bandpass_biquad(biquad_filter_t *f, double freq, double rate, float q_in) {
    if (freq < 10.0) freq = 10.0;
    if (freq >= rate * 0.48) freq = rate * 0.48;
    double q = (q_in < 0.2f) ? 0.2 : (double)q_in;
    double w0 = 2.0 * M_PI * freq / rate;
    double cos_w0 = cos(w0);
    double sin_w0 = sin(w0);
    double alpha = sin_w0 / (2.0 * q);

    double b0 = alpha;
    double b1 = 0.0;
    double b2 = -alpha;
    double a0 = 1.0 + alpha;
    double a1 = -2.0 * cos_w0;
    double a2 = 1.0 - alpha;

    double inv_a0 = 1.0 / a0;
    f->b0 = b0 * inv_a0;
    f->b1 = b1 * inv_a0;
    f->b2 = b2 * inv_a0;
    f->a1 = a1 * inv_a0;
    f->a2 = a2 * inv_a0;
}

static void process_crossfeed(float *buf, uint32_t frames, uint32_t ch, uint32_t rate) {
    if (ch < 2 || !g_eq_cfg.enabled || !g_param_eq.crossfeed_enabled) {
        g_xfeed_lp_l = 0.0;
        g_xfeed_lp_r = 0.0;
        return;
    }
    float lvl = g_param_eq.crossfeed_level;
    if (lvl < 0.02f) return;
    if (lvl > 1.0f) lvl = 1.0f;
    uint32_t fc = g_param_eq.crossfeed_cutoff_hz;
    if (fc < 300U || fc > 1500U) fc = 700U;

    double lp_a = exp(-2.0 * M_PI * (double)fc / (double)rate);
    double lp_b = 1.0 - lp_a;
    float feed = 0.26f * lvl;
    float direct = 1.0f - 0.14f * lvl;
    uint32_t dly_len = (uint32_t)(0.00026 * (double)rate); /* ~260us ILD/ITD */
    if (dly_len < 1U) dly_len = 1U;
    if (dly_len > 63U) dly_len = 63U;

    for (uint32_t f = 0; f < frames; f++) {
        float xl = buf[f * ch + 0];
        float xr = buf[f * ch + 1];
        g_xfeed_dly_l[g_xfeed_dly_idx & 63U] = xl;
        g_xfeed_dly_r[g_xfeed_dly_idx & 63U] = xr;
        uint32_t r_pos = (g_xfeed_dly_idx - dly_len) & 63U;
        float dl = g_xfeed_dly_l[r_pos];
        float dr = g_xfeed_dly_r[r_pos];
        g_xfeed_dly_idx++;

        g_xfeed_lp_l = lp_b * (double)dl + lp_a * g_xfeed_lp_l;
        g_xfeed_lp_r = lp_b * (double)dr + lp_a * g_xfeed_lp_r;
        if (fabs(g_xfeed_lp_l) < 1e-18) g_xfeed_lp_l = 0.0;
        if (fabs(g_xfeed_lp_r) < 1e-18) g_xfeed_lp_r = 0.0;

        buf[f * ch + 0] = xl * direct + (float)g_xfeed_lp_r * feed;
        buf[f * ch + 1] = xr * direct + (float)g_xfeed_lp_l * feed;
    }
}

static void process_salsa_dynamic_eq(float *buf, uint32_t frames, uint32_t ch, uint32_t rate) {
    if (frames == 0 || ch == 0) return;
    if (!rate) rate = 48000;
    double d_rate = (double)rate;

    for (int b = 0; b < LWASAPI_EQ_BANDS; b++) {
        float bfreq = (g_param_eq.band_freq_hz[b] >= 20.0f && g_param_eq.band_freq_hz[b] <= 20000.0f)
                    ? g_param_eq.band_freq_hz[b] : k_eq_freqs[b];
        if (g_spec_last_rate != rate || fabsf(g_spec_last_freqs[b] - bfreq) > 0.5f) {
            memset(&g_spec_filters[b], 0, sizeof(g_spec_filters[b]));
            compute_bandpass_biquad(&g_spec_filters[b], (double)bfreq, d_rate, 1.8f);
            g_spec_last_freqs[b] = bfreq;
            if (g_spec_last_rate != rate) {
                memset(&g_eq_filters[b], 0, sizeof(g_eq_filters[b]));
            }
        }
    }
    g_spec_last_rate = rate;

    float preamp_lin = g_eq_cfg.enabled ? powf(10.0f, g_eq_cfg.preamp_db / 20.0f) : 1.0f;
    if (g_eq_cfg.enabled && fabsf(g_eq_cfg.preamp_db) > 0.01f) {
        for (uint32_t i = 0; i < frames * ch; i++) {
            buf[i] *= preamp_lin;
        }
    }

    /* 1. Measure per-band pre-EQ envelope for Spectrum Analyzer & smooth Dynamic EQ ballistics */
    for (int b = 0; b < LWASAPI_EQ_BANDS; b++) {
        biquad_filter_t *sf = &g_spec_filters[b];
        double z1 = sf->z1[0];
        double z2 = sf->z2[0];
        double sum_sq = 0.0;

        for (uint32_t f = 0; f < frames; f++) {
            double mono = (ch >= 2) ? 0.5 * ((double)buf[f * ch + 0] + (double)buf[f * ch + 1]) : (double)buf[f * ch];
            double y = sf->b0 * mono + z1;
            z1 = sf->b1 * mono - sf->a1 * y + z2;
            z2 = sf->b2 * mono - sf->a2 * y;
            sum_sq += y * y;
        }
        if (fabs(z1) < 1e-18) z1 = 0.0;
        if (fabs(z2) < 1e-18) z2 = 0.0;
        sf->z1[0] = z1;
        sf->z2[0] = z2;

        float band_rms = sqrtf((float)(sum_sq / (double)frames)) * 1.4142135f;
        float coeff = (band_rms > g_band_env[b]) ? 0.18f : 0.05f;
        g_band_env[b] = g_band_env[b] + coeff * (band_rms - g_band_env[b]);
        if (g_band_env[b] < 1e-10f) g_band_env[b] = 0.0f;
        float db = 20.0f * log10f(g_band_env[b] + 1e-5f);
        if (db < -60.0f) db = -60.0f;
        if (db > 6.0f) db = 6.0f;
        g_eq_band_levels_db[b] = db;

        float target_delta = 0.0f;
        if (g_eq_cfg.enabled && g_eq_cfg.dynamic_mode != 0) {
            float thresh = g_eq_cfg.dyn_threshold_db;
            float ratio = (g_eq_cfg.dyn_ratio >= 1.0f) ? g_eq_cfg.dyn_ratio : 2.0f;
            if (db > thresh) {
                float over = db - thresh;
                if (g_eq_cfg.dynamic_mode == 1) {
                    target_delta = -over * (1.0f - (1.0f / ratio)) * 0.65f;
                    if (target_delta < -10.0f) target_delta = -10.0f;
                } else if (g_eq_cfg.dynamic_mode == 2) {
                    target_delta = over * ((ratio - 1.0f) * 0.12f);
                    if (target_delta > 3.5f) target_delta = 3.5f;
                }
            }
        }
        g_eq_dyn_delta_db[b] = 0.80f * g_eq_dyn_delta_db[b] + 0.20f * target_delta;
    }

    /* 2. Apply 10-band Full Parametric Biquad EQ (Peaking / Low-Shelf / High-Shelf) */
    for (int b = 0; b < LWASAPI_EQ_BANDS; b++) {
        biquad_filter_t *ef = &g_eq_filters[b];
        if (g_eq_cfg.enabled) {
            float eff_gain = g_eq_cfg.band_gains_db[b] + g_eq_dyn_delta_db[b];
            if (eff_gain > 24.0f) eff_gain = 24.0f;
            if (eff_gain < -24.0f) eff_gain = -24.0f;

            if (fabsf(eff_gain) > 0.05f) {
                float q = g_eq_cfg.band_q[b];
                if (q < 0.2f || q > 10.0f) q = 1.414f;
                float freq_hz = g_spec_last_freqs[b];
                uint32_t ftype = g_param_eq.band_type[b];
                if (!ef->coeffs_valid || ef->last_rate != rate || ef->last_type != ftype ||
                    fabsf(ef->last_freq_hz - freq_hz) > 0.5f ||
                    fabsf(ef->last_q - q) > 1e-4f || fabsf(ef->last_gain_db - eff_gain) > 0.08f) {
                    compute_parametric_biquad(ef, ftype, (double)freq_hz, d_rate, q, eff_gain);
                    ef->last_rate = rate;
                    ef->last_type = ftype;
                    ef->last_freq_hz = freq_hz;
                    ef->last_q = q;
                    ef->last_gain_db = eff_gain;
                    ef->coeffs_valid = true;
                }
                for (uint32_t c = 0; c < ch && c < LWASAPI_MAX_CHANNELS; c++) {
                    double ez1 = ef->z1[c];
                    double ez2 = ef->z2[c];
                    for (uint32_t f = 0; f < frames; f++) {
                        double x = (double)buf[f * ch + c];
                        double y = ef->b0 * x + ez1;
                        ez1 = ef->b1 * x - ef->a1 * y + ez2;
                        ez2 = ef->b2 * x - ef->a2 * y;
                        buf[f * ch + c] = (float)y;
                    }
                    if (fabs(ez1) < 1e-18) ez1 = 0.0;
                    if (fabs(ez2) < 1e-18) ez2 = 0.0;
                    ef->z1[c] = ez1;
                    ef->z2[c] = ez2;
                }
            } else if (ef->coeffs_valid) {
                memset(ef->z1, 0, sizeof(ef->z1));
                memset(ef->z2, 0, sizeof(ef->z2));
                ef->coeffs_valid = false;
            }
        } else if (ef->coeffs_valid) {
            memset(ef->z1, 0, sizeof(ef->z1));
            memset(ef->z2, 0, sizeof(ef->z2));
            ef->coeffs_valid = false;
        }
    }

    /* Optional Bauer/BS2B Stereo Headphone Crossfeed */
    process_crossfeed(buf, frames, ch, rate);

    /* 3. Soft-knee mastering limiter & Stereo Output Peak VU metering */
    float pk_l = 0.0f, pk_r = 0.0f;
    bool lim = (!g_eq_cfg.enabled || g_eq_cfg.limiter_enabled != 0);
    for (uint32_t f = 0; f < frames; f++) {
        for (uint32_t c = 0; c < ch; c++) {
            float x = buf[f * ch + c];
            if (lim) {
                if (x > 0.92f) x = 0.92f + 0.08f * tanhf((x - 0.92f) / 0.08f);
                else if (x < -0.92f) x = -0.92f + 0.08f * tanhf((x + 0.92f) / 0.08f);
                buf[f * ch + c] = x;
            }
            float ax = fabsf(x);
            if (c == 0 && ax > pk_l) pk_l = ax;
            if (c == 1 && ax > pk_r) pk_r = ax;
        }
    }
    if (ch == 1) pk_r = pk_l;
    g_output_peak_l = (pk_l > g_output_peak_l) ? pk_l : (g_output_peak_l * 0.85f + pk_l * 0.15f);
    g_output_peak_r = (pk_r > g_output_peak_r) ? pk_r : (g_output_peak_r * 0.85f + pk_r * 0.15f);
}

static void ensure_mic_rnnoise_loaded(void) {
    if (g_mic_rn.probed) return;
    g_mic_rn.probed = true;
    const char *cands[] = { "librnnoise.so.0", "/usr/lib/librnnoise.so.0", "/usr/local/lib/librnnoise.so.0", NULL };
    for (int i = 0; cands[i]; i++) {
        g_mic_rn.lib = dlopen(cands[i], RTLD_LAZY);
        if (g_mic_rn.lib) break;
    }
    if (!g_mic_rn.lib) return;
    void *c_sym = dlsym(g_mic_rn.lib, "rnnoise_create");
    void *d_sym = dlsym(g_mic_rn.lib, "rnnoise_destroy");
    void *p_sym = dlsym(g_mic_rn.lib, "rnnoise_process_frame");
    memcpy(&g_mic_rn.create_fn, &c_sym, sizeof(c_sym));
    memcpy(&g_mic_rn.destroy_fn, &d_sym, sizeof(d_sym));
    memcpy(&g_mic_rn.process_fn, &p_sym, sizeof(p_sym));
    if (g_mic_rn.create_fn && g_mic_rn.destroy_fn && g_mic_rn.process_fn) {
        g_mic_rn.st = g_mic_rn.create_fn(NULL);
        g_mic_rn.available = (g_mic_rn.st != NULL);
    }
    if (g_mic_rn.available) {
        fprintf(stderr, "[LWASAPI-Salsa] Capture Mic APO: Neural RNNoise denoiser loaded (librnnoise.so.0).\n");
    }
}

static void run_mic_rnnoise_mono(float *mono, float *out_dry_aligned, float *out_vad_aligned,
                                 uint32_t frames, uint32_t rate) {
    if (!mono || frames == 0 || !g_mic_rn.available || !g_mic_rn.st) return;
    if (g_mic_rn.last_rate != rate) {
        g_mic_rn.in_count = 0;
        g_mic_rn.out_rd = 0;
        g_mic_rn.out_wr = MIC_RN_FRAME;
        memset(g_mic_rn.out_ring, 0, MIC_RN_FRAME * sizeof(float));
        memset(g_mic_rn.dry_ring, 0, MIC_RN_FRAME * sizeof(float));
        memset(g_mic_rn.vad_ring, 0, MIC_RN_FRAME * sizeof(float));
        g_mic_rn.in_last = 0.0f;
        g_mic_rn.out_last = 0.0f;
        g_mic_rn.dry_last = 0.0f;
        g_mic_rn.vad_last = 0.0f;
        g_mic_rn.in_phase = 0.0;
        g_mic_rn.out_phase = 0.0;
        g_mic_rn.last_rate = rate;
    }

    /* 1. Resample mono input to 48000 Hz * 32768.0f scale into in_ring */
    if (rate == 48000U) {
        uint32_t room = MIC_RN_CAP - g_mic_rn.in_count;
        uint32_t take = (frames < room) ? frames : room;
        for (uint32_t f = 0; f < take; f++) {
            g_mic_rn.in_ring[g_mic_rn.in_count++] = mono[f] * 32768.0f;
        }
    } else {
        double step_in = (double)rate / 48000.0;
        double ph = g_mic_rn.in_phase;
        while (ph < (double)frames && g_mic_rn.in_count < MIC_RN_CAP) {
            int64_t i0 = (int64_t)floor(ph);
            double frac = ph - (double)i0;
            float s0 = (i0 < 0) ? g_mic_rn.in_last : mono[i0];
            uint32_t i1 = (uint32_t)(i0 + 1);
            float s1 = (i1 < frames) ? mono[i1] : mono[frames - 1];
            float y = s0 + (s1 - s0) * (float)frac;
            g_mic_rn.in_ring[g_mic_rn.in_count++] = y * 32768.0f;
            ph += step_in;
        }
        g_mic_rn.in_phase = ph - (double)frames;
        g_mic_rn.in_last = mono[frames - 1];
    }

    /* 2. Run RNNoise 0.2 per 480-sample (10ms) frame, capturing exact dry & VAD lockstep */
    float tmp_out[MIC_RN_FRAME];
    const uint32_t mask = MIC_RN_CAP - 1U;
    while (g_mic_rn.in_count >= MIC_RN_FRAME) {
        float vad = g_mic_rn.process_fn(g_mic_rn.st, tmp_out, g_mic_rn.in_ring);
        if (vad < 0.0f) vad = 0.0f;
        if (vad > 1.0f) vad = 1.0f;
        for (uint32_t k = 0; k < MIC_RN_FRAME; k++) {
            if ((g_mic_rn.out_wr - g_mic_rn.out_rd) < MIC_RN_CAP) {
                uint32_t slot = g_mic_rn.out_wr & mask;
                g_mic_rn.out_ring[slot] = tmp_out[k] * (1.0f / 32768.0f);
                g_mic_rn.dry_ring[slot] = g_mic_rn.in_ring[k] * (1.0f / 32768.0f);
                g_mic_rn.vad_ring[slot] = vad;
                g_mic_rn.out_wr++;
            }
        }
        uint32_t rem = g_mic_rn.in_count - MIC_RN_FRAME;
        if (rem > 0) memmove(g_mic_rn.in_ring, g_mic_rn.in_ring + MIC_RN_FRAME, rem * sizeof(float));
        g_mic_rn.in_count = rem;
    }

    /* 3. Read cleaned + zero-skew aligned dry + VAD back to engine sample rate */
    if (rate == 48000U) {
        for (uint32_t f = 0; f < frames; f++) {
            if (g_mic_rn.out_wr > g_mic_rn.out_rd) {
                uint32_t slot = g_mic_rn.out_rd & mask;
                float y = g_mic_rn.out_ring[slot];
                float d = g_mic_rn.dry_ring[slot];
                float v = g_mic_rn.vad_ring[slot];
                g_mic_rn.out_rd++;
                g_mic_rn.out_last = y;
                g_mic_rn.dry_last = d;
                g_mic_rn.vad_last = v;
                mono[f] = y;
                if (out_dry_aligned) out_dry_aligned[f] = d;
                if (out_vad_aligned) out_vad_aligned[f] = v;
            } else {
                mono[f] = g_mic_rn.out_last;
                if (out_dry_aligned) out_dry_aligned[f] = g_mic_rn.dry_last;
                if (out_vad_aligned) out_vad_aligned[f] = g_mic_rn.vad_last;
            }
        }
        if ((g_mic_rn.out_wr - g_mic_rn.out_rd) > MIC_RN_FRAME * 2U) {
            g_mic_rn.out_rd = g_mic_rn.out_wr - MIC_RN_FRAME;
        }
    } else {
        double step_out = 48000.0 / (double)rate;
        double ph = g_mic_rn.out_phase;
        for (uint32_t f = 0; f < frames; f++) {
            uint32_t avail = g_mic_rn.out_wr - g_mic_rn.out_rd;
            float y = g_mic_rn.out_last;
            float d = g_mic_rn.dry_last;
            float v = g_mic_rn.vad_last;
            if (avail >= 2U) {
                uint32_t s0_idx = g_mic_rn.out_rd & mask;
                uint32_t s1_idx = (g_mic_rn.out_rd + 1U) & mask;
                float y0 = g_mic_rn.out_ring[s0_idx], y1 = g_mic_rn.out_ring[s1_idx];
                float d0 = g_mic_rn.dry_ring[s0_idx], d1 = g_mic_rn.dry_ring[s1_idx];
                y = y0 + (y1 - y0) * (float)ph;
                d = d0 + (d1 - d0) * (float)ph;
                v = g_mic_rn.vad_ring[s0_idx];
                g_mic_rn.out_last = y;
                g_mic_rn.dry_last = d;
                g_mic_rn.vad_last = v;
            } else if (avail == 1U) {
                uint32_t s0_idx = g_mic_rn.out_rd & mask;
                y = g_mic_rn.out_ring[s0_idx];
                d = g_mic_rn.dry_ring[s0_idx];
                v = g_mic_rn.vad_ring[s0_idx];
                g_mic_rn.out_last = y;
                g_mic_rn.dry_last = d;
                g_mic_rn.vad_last = v;
            }
            mono[f] = y;
            if (out_dry_aligned) out_dry_aligned[f] = d;
            if (out_vad_aligned) out_vad_aligned[f] = v;
            ph += step_out;
            uint32_t adv = (uint32_t)floor(ph);
            if (adv > 0U) {
                if (adv > avail) adv = avail;
                g_mic_rn.out_rd += adv;
                ph -= (double)adv;
            }
        }
        g_mic_rn.out_phase = ph;
        if ((g_mic_rn.out_wr - g_mic_rn.out_rd) > MIC_RN_FRAME * 2U) {
            g_mic_rn.out_rd = g_mic_rn.out_wr - MIC_RN_FRAME;
        }
    }
}

/* Snapshot of g_mic_apo passed by caller (already locked or copied cleanly) */
static void process_capture_mic_apo(float *mic_buf, uint32_t frames, uint32_t rate,
                                    const lwasapi_mic_apo_cfg_t *cfg) {
    if (!mic_buf || frames == 0 || !cfg) return;
    if (!rate) rate = 48000U;

    float pk_in = 0.0f;
    for (uint32_t f = 0; f < frames; f++) {
        float al = fabsf(mic_buf[f * 2 + 0]);
        float ar = fabsf(mic_buf[f * 2 + 1]);
        float m = (al > ar) ? al : ar;
        if (m > pk_in) pk_in = m;
    }
    g_mic_apo_peak_in = (pk_in > g_mic_apo_peak_in) ? pk_in : (g_mic_apo_peak_in * 0.85f + pk_in * 0.15f);

    if (!cfg->enabled) {
        g_mic_apo_peak_out = g_mic_apo_peak_in;
        g_mic_rnnoise_active = 0U;
        g_mic_vad_env = 0.0f;
        g_mic_vad_hold_samples = 0U;
        g_mic_silence_gain = 1.0f;
        return;
    }

    static float s_mono[LWASAPI_MAX_PERIOD];
    static float s_dry_aligned[LWASAPI_MAX_PERIOD];
    static float s_vad_aligned[LWASAPI_MAX_PERIOD];
    static float s_ch_env_l = 0.0f;
    static float s_ch_env_r = 0.0f;
    float p_abs_l = 0.0f, p_abs_r = 0.0f;
    for (uint32_t f = 0; f < frames; f++) {
        float al = fabsf(mic_buf[f * 2 + 0]);
        float ar = fabsf(mic_buf[f * 2 + 1]);
        if (al > p_abs_l) p_abs_l = al;
        if (ar > p_abs_r) p_abs_r = ar;
    }
    s_ch_env_l = 0.85f * s_ch_env_l + 0.15f * p_abs_l;
    s_ch_env_r = 0.85f * s_ch_env_r + 0.15f * p_abs_r;
    bool use_left_only  = (s_ch_env_l > 1e-5f && s_ch_env_r < 0.12f * s_ch_env_l);
    bool use_right_only = (s_ch_env_r > 1e-5f && s_ch_env_l < 0.12f * s_ch_env_r);

    /* 0. Unity mono auto-selection + 10 Hz DC-blocker so ADC offset never biases neural VAD */
    double dc_r = 1.0 - (2.0 * M_PI * 10.0) / (double)rate;
    if (dc_r < 0.90) dc_r = 0.90;
    if (dc_r > 0.9995) dc_r = 0.9995;
    for (uint32_t f = 0; f < frames; f++) {
        float l = mic_buf[f * 2 + 0];
        float r = mic_buf[f * 2 + 1];
        float raw = use_left_only ? l : (use_right_only ? r : ((fabsf(l - r) < 1e-6f) ? l : 0.5f * (l + r)));
        double x = (double)raw;
        double y = x - g_mic_dc_x1 + dc_r * g_mic_dc_y1;
        g_mic_dc_x1 = x;
        if (fabs(y) < 1e-18) y = 0.0;
        g_mic_dc_y1 = y;
        s_mono[f] = (float)y;
    }

    /* 1. Subsonic Voice Rumble Filter (80 Hz Butterworth HPF) before RNNoise */
    if (cfg->hpf_enabled) {
        float hpf_hz = (cfg->hpf_hz >= 40.0f && cfg->hpf_hz <= 250.0f) ? cfg->hpf_hz : 80.0f;
        if (!g_mic_hpf_flt.coeffs_valid || g_mic_hpf_flt.last_rate != rate ||
            fabsf(g_mic_hpf_flt.last_freq_hz - hpf_hz) > 0.5f) {
            compute_hpf_biquad(&g_mic_hpf_flt, (double)hpf_hz, (double)rate);
            g_mic_hpf_flt.last_rate = rate;
            g_mic_hpf_flt.last_freq_hz = hpf_hz;
            g_mic_hpf_flt.coeffs_valid = true;
        }
        double z1 = g_mic_hpf_flt.z1[0], z2 = g_mic_hpf_flt.z2[0];
        for (uint32_t f = 0; f < frames; f++) {
            double x = (double)s_mono[f];
            double y = g_mic_hpf_flt.b0 * x + z1;
            z1 = g_mic_hpf_flt.b1 * x - g_mic_hpf_flt.a1 * y + z2;
            z2 = g_mic_hpf_flt.b2 * x - g_mic_hpf_flt.a2 * y;
            s_mono[f] = (float)y;
        }
        if (fabs(z1) < 1e-18) z1 = 0.0;
        if (fabs(z2) < 1e-18) z2 = 0.0;
        g_mic_hpf_flt.z1[0] = z1;
        g_mic_hpf_flt.z2[0] = z2;
    } else if (g_mic_hpf_flt.coeffs_valid) {
        memset(g_mic_hpf_flt.z1, 0, sizeof(g_mic_hpf_flt.z1));
        memset(g_mic_hpf_flt.z2, 0, sizeof(g_mic_hpf_flt.z2));
        g_mic_hpf_flt.coeffs_valid = false;
    }

    /* 2. Krisp-Grade RNNoise 0.2 Neural Denoiser + Zero-Skew Aligned Air + 220ms VAD Holdover */
    bool used_rn = false;
    if (cfg->rnnoise_enabled) {
        ensure_mic_rnnoise_loaded();
        if (g_mic_rn.available) {
            run_mic_rnnoise_mono(s_mono, s_dry_aligned, s_vad_aligned, frames, rate);
            float vad_atk = expf(-1.0f / (0.003f * (float)rate));
            float vad_rel = expf(-1.0f / (0.140f * (float)rate));
            float sil_atk = expf(-1.0f / (0.004f * (float)rate));
            float sil_rel = expf(-1.0f / (0.085f * (float)rate));
            uint32_t hold_reset = (uint32_t)(0.220f * (float)rate);
            for (uint32_t f = 0; f < frames; f++) {
                float v_raw = s_vad_aligned[f];
                if (v_raw >= 0.35f) {
                    g_mic_vad_hold_samples = hold_reset;
                } else if (g_mic_vad_hold_samples > 0U) {
                    g_mic_vad_hold_samples--;
                }
                float v_tgt = (g_mic_vad_hold_samples > 0U && v_raw < 0.48f) ? 0.48f : v_raw;
                float vc = (v_tgt > g_mic_vad_env) ? vad_atk : vad_rel;
                g_mic_vad_env = vc * g_mic_vad_env + (1.0f - vc) * v_tgt;
                s_vad_aligned[f] = g_mic_vad_env;

                /* Blend 0..10% exact zero-skew aligned dry voice ONLY when neural VAD confirms speech */
                float dry_mix = 0.0f;
                if (g_mic_vad_env > 0.25f) {
                    float t = (g_mic_vad_env - 0.25f) / 0.60f;
                    if (t > 1.0f) t = 1.0f;
                    dry_mix = 0.10f * t;
                }
                float sil_tgt = (g_mic_vad_env >= 0.20f) ? 1.0f : 0.125f; /* -18 dB silent neural floor */
                float sc = (sil_tgt > g_mic_silence_gain) ? sil_atk : sil_rel;
                g_mic_silence_gain = sc * g_mic_silence_gain + (1.0f - sc) * sil_tgt;
                s_mono[f] = ((1.0f - dry_mix) * s_mono[f] + dry_mix * s_dry_aligned[f]) * g_mic_silence_gain;
            }
            used_rn = true;
        }
    }
    g_mic_rnnoise_active = used_rn ? 1U : 0U;

    /* 3. Speech-Aware Neural VAD + Hysteresis Noise Gate (rejects keyboard/mouse clicks) */
    if (cfg->gate_enabled) {
        float thresh_lin = powf(10.0f, cfg->gate_thresh_db / 20.0f);
        float close_lin  = thresh_lin * 0.63f; /* ~4 dB hysteresis */
        float env_atk = expf(-1.0f / (0.002f * (float)rate));
        float env_rel = expf(-1.0f / (0.120f * (float)rate));
        float g_atk   = expf(-1.0f / (0.0025f * (float)rate));
        float g_rel   = expf(-1.0f / (0.140f * (float)rate));
        for (uint32_t f = 0; f < frames; f++) {
            float ax = fabsf(s_mono[f]);
            float ec = (ax > g_mic_gate_env) ? env_atk : env_rel;
            g_mic_gate_env = ec * g_mic_gate_env + (1.0f - ec) * ax;
            float tgt;
            if (used_rn) {
                float v = s_vad_aligned[f];
                bool open_vad  = (v >= 0.28f) || (g_mic_gate_env >= thresh_lin && v >= 0.12f);
                bool close_vad = (v < 0.16f && g_mic_gate_env <= close_lin) || (v < 0.09f);
                tgt = open_vad ? 1.0f : (close_vad ? 0.012f : g_mic_gate_gain);
            } else {
                tgt = (g_mic_gate_env >= thresh_lin) ? 1.0f :
                      (g_mic_gate_env <= close_lin)  ? 0.015f : g_mic_gate_gain;
            }
            float gc = (tgt > g_mic_gate_gain) ? g_atk : g_rel;
            g_mic_gate_gain = gc * g_mic_gate_gain + (1.0f - gc) * tgt;
            s_mono[f] *= g_mic_gate_gain;
        }
    }

    /* 4. Krisp Broadcast Voice Contour: De-Mud @ 250Hz + Split Presence/Air + Speech-Gated Leveler */
    if (cfg->comp_enabled) {
        if (!g_mic_demud_flt.coeffs_valid || g_mic_demud_flt.last_rate != rate) {
            compute_parametric_biquad(&g_mic_demud_flt, LWASAPI_FILTER_PEAK,
                                      250.0, (double)rate, 1.10f, -1.4f);
            g_mic_demud_flt.last_rate = rate;
            g_mic_demud_flt.coeffs_valid = true;
        }
        double dz1 = g_mic_demud_flt.z1[0], dz2 = g_mic_demud_flt.z2[0];
        for (uint32_t f = 0; f < frames; f++) {
            double x = (double)s_mono[f];
            double y = g_mic_demud_flt.b0 * x + dz1;
            dz1 = g_mic_demud_flt.b1 * x - g_mic_demud_flt.a1 * y + dz2;
            dz2 = g_mic_demud_flt.b2 * x - g_mic_demud_flt.a2 * y;
            s_mono[f] = (float)y;
        }
        if (fabs(dz1) < 1e-18) dz1 = 0.0;
        if (fabs(dz2) < 1e-18) dz2 = 0.0;
        g_mic_demud_flt.z1[0] = dz1;
        g_mic_demud_flt.z2[0] = dz2;

        if (fabsf(cfg->clarity_db) > 0.1f) {
            if (!g_mic_pres_flt.coeffs_valid || g_mic_pres_flt.last_rate != rate ||
                fabsf(g_mic_pres_flt.last_gain_db - cfg->clarity_db) > 0.05f) {
                compute_parametric_biquad(&g_mic_pres_flt, LWASAPI_FILTER_PEAK,
                                          3400.0, (double)rate, 0.80f, cfg->clarity_db * 0.40f);
                g_mic_pres_flt.last_rate = rate;
                g_mic_pres_flt.last_gain_db = cfg->clarity_db;
                g_mic_pres_flt.coeffs_valid = true;
            }
            if (!g_mic_clarity_flt.coeffs_valid || g_mic_clarity_flt.last_rate != rate ||
                fabsf(g_mic_clarity_flt.last_gain_db - cfg->clarity_db) > 0.05f) {
                compute_parametric_biquad(&g_mic_clarity_flt, LWASAPI_FILTER_HIGH_SHELF,
                                          6800.0, (double)rate, 0.707f, cfg->clarity_db * 0.60f);
                g_mic_clarity_flt.last_rate = rate;
                g_mic_clarity_flt.last_gain_db = cfg->clarity_db;
                g_mic_clarity_flt.coeffs_valid = true;
            }
            double pz1 = g_mic_pres_flt.z1[0], pz2 = g_mic_pres_flt.z2[0];
            double sz1 = g_mic_clarity_flt.z1[0], sz2 = g_mic_clarity_flt.z2[0];
            for (uint32_t f = 0; f < frames; f++) {
                double x = (double)s_mono[f];
                double p_y = g_mic_pres_flt.b0 * x + pz1;
                pz1 = g_mic_pres_flt.b1 * x - g_mic_pres_flt.a1 * p_y + pz2;
                pz2 = g_mic_pres_flt.b2 * x - g_mic_pres_flt.a2 * p_y;
                double s_y = g_mic_clarity_flt.b0 * p_y + sz1;
                sz1 = g_mic_clarity_flt.b1 * p_y - g_mic_clarity_flt.a1 * s_y + sz2;
                sz2 = g_mic_clarity_flt.b2 * p_y - g_mic_clarity_flt.a2 * s_y;
                s_mono[f] = (float)s_y;
            }
            if (fabs(pz1) < 1e-18) pz1 = 0.0;
            if (fabs(pz2) < 1e-18) pz2 = 0.0;
            if (fabs(sz1) < 1e-18) sz1 = 0.0;
            if (fabs(sz2) < 1e-18) sz2 = 0.0;
            g_mic_pres_flt.z1[0] = pz1;
            g_mic_pres_flt.z2[0] = pz2;
            g_mic_clarity_flt.z1[0] = sz1;
            g_mic_clarity_flt.z2[0] = sz2;
        }

        float cthresh = cfg->comp_thresh_db;
        float ratio   = (cfg->comp_ratio >= 1.0f) ? cfg->comp_ratio : 2.2f;
        float atk = expf(-1.0f / (0.004f * (float)rate));
        float rel = expf(-1.0f / (0.090f * (float)rate));
        for (uint32_t f = 0; f < frames; f++) {
            float x = s_mono[f];
            float ax = fabsf(x);
            float ec = (ax > g_mic_comp_env) ? atk : rel;
            g_mic_comp_env = ec * g_mic_comp_env + (1.0f - ec) * ax;
            float env_db = 20.0f * log10f(g_mic_comp_env + 1e-6f);
            /* Gentle +1.8 dB speech makeup ONLY when actively speaking so silence never rises */
            float speech_ok = used_rn ? ((s_vad_aligned[f] > 0.30f) ? 1.0f : 0.0f)
                                      : ((env_db > -34.0f) ? 1.0f : 0.0f);
            float gain_db = 1.8f * speech_ok;
            if (env_db > cthresh) {
                gain_db -= (env_db - cthresh) * (1.0f - 1.0f / ratio);
            }
            float tgt = powf(10.0f, gain_db / 20.0f);
            g_mic_comp_gain = 0.90f * g_mic_comp_gain + 0.10f * tgt;
            x *= g_mic_comp_gain;
            if (x > 0.94f) x = 0.94f + 0.05f * tanhf((x - 0.94f) / 0.05f);
            else if (x < -0.94f) x = -0.94f + 0.05f * tanhf((x + 0.94f) / 0.05f);
            s_mono[f] = x;
        }
    }

    float pk_out = 0.0f;
    for (uint32_t f = 0; f < frames; f++) {
        float y = s_mono[f];
        float ay = fabsf(y);
        if (ay > pk_out) pk_out = ay;
        mic_buf[f * 2 + 0] = y;
        mic_buf[f * 2 + 1] = y;
    }
    g_mic_apo_peak_out = (pk_out > g_mic_apo_peak_out) ? pk_out : (g_mic_apo_peak_out * 0.85f + pk_out * 0.15f);
}

static void handle_sigint(int sig) {
    (void)sig;
    g_running = 0;
}

static uint64_t monotonic_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

/* Helpers to initialize raw kernel struct snd_pcm_hw_params masks & intervals */
static void hw_params_any(struct snd_pcm_hw_params *params) {
    memset(params, 0, sizeof(*params));
    for (int i = 0; i <= SNDRV_PCM_HW_PARAM_LAST_MASK - SNDRV_PCM_HW_PARAM_FIRST_MASK; i++) {
        memset(&params->masks[i], 0xff, sizeof(struct snd_mask));
    }
    for (int i = 0; i <= SNDRV_PCM_HW_PARAM_LAST_INTERVAL - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL; i++) {
        params->intervals[i].min = 0;
        params->intervals[i].max = ~0U;
    }
    params->rmask = ~0U;
}

static void hw_param_set_mask(struct snd_pcm_hw_params *params, int param, unsigned int bit) {
    struct snd_mask *m = &params->masks[param - SNDRV_PCM_HW_PARAM_FIRST_MASK];
    memset(m, 0, sizeof(*m));
    m->bits[bit >> 5] |= (1U << (bit & 31));
    params->cmask |= (1U << param);
}

static void hw_param_set_minmax(struct snd_pcm_hw_params *params, int param, unsigned int minv, unsigned int maxv) {
    struct snd_interval *iv = &params->intervals[param - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL];
    iv->min = minv;
    iv->max = maxv;
    iv->openmin = 0;
    iv->openmax = 0;
    iv->integer = 0;
    params->cmask |= (1U << param);
}

static int open_kernel_pcm_device(const char *dev_path, uint32_t req_rate, uint32_t req_period,
                                  uint32_t req_bits, kernel_wavert_hw_t *hw) {
    int fd = open(dev_path, O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        return -1;
    }

    int candidate_formats[5];
    uint32_t candidate_bits[5];
    if (req_bits == 16U) {
        candidate_formats[0] = SNDRV_PCM_FORMAT_S16_LE;  candidate_bits[0] = 16;
        candidate_formats[1] = SNDRV_PCM_FORMAT_S24_3LE; candidate_bits[1] = 24;
        candidate_formats[2] = SNDRV_PCM_FORMAT_S24_LE;  candidate_bits[2] = 24;
        candidate_formats[3] = SNDRV_PCM_FORMAT_S32_LE;  candidate_bits[3] = 32;
        candidate_formats[4] = SNDRV_PCM_FORMAT_FLOAT_LE;candidate_bits[4] = 32;
    } else if (req_bits == 24U) {
        candidate_formats[0] = SNDRV_PCM_FORMAT_S24_3LE; candidate_bits[0] = 24;
        candidate_formats[1] = SNDRV_PCM_FORMAT_S24_LE;  candidate_bits[1] = 24;
        candidate_formats[2] = SNDRV_PCM_FORMAT_S32_LE;  candidate_bits[2] = 32;
        candidate_formats[3] = SNDRV_PCM_FORMAT_FLOAT_LE;candidate_bits[3] = 32;
        candidate_formats[4] = SNDRV_PCM_FORMAT_S16_LE;  candidate_bits[4] = 16;
    } else {
        candidate_formats[0] = SNDRV_PCM_FORMAT_S32_LE;  candidate_bits[0] = 32;
        candidate_formats[1] = SNDRV_PCM_FORMAT_FLOAT_LE;candidate_bits[1] = 32;
        candidate_formats[2] = SNDRV_PCM_FORMAT_S24_3LE; candidate_bits[2] = 24;
        candidate_formats[3] = SNDRV_PCM_FORMAT_S24_LE;  candidate_bits[3] = 24;
        candidate_formats[4] = SNDRV_PCM_FORMAT_S16_LE;  candidate_bits[4] = 16;
    }
    const uint32_t candidate_chs[] = { 2, 1 };
    uint32_t candidate_rates[4] = { req_rate, 0, 48000, 44100 };
    if (req_rate == 192000U) {
        candidate_rates[1] = 96000U;
    } else if (req_rate == 176400U) {
        candidate_rates[1] = 88200U;
        candidate_rates[2] = 44100U;
        candidate_rates[3] = 48000U;
    } else if (req_rate == 88200U || req_rate == 44100U) {
        candidate_rates[2] = 44100U;
        candidate_rates[3] = 48000U;
    }

    struct snd_pcm_hw_params params;
    int chosen_fmt = -1;
    uint32_t chosen_bits = 0;

    const uint32_t buf_mults[] = { 4U, 2U };
    for (size_t bm = 0; bm < 2 && chosen_fmt < 0; bm++) {
        for (size_t r = 0; r < 4 && chosen_fmt < 0; r++) {
            if (candidate_rates[r] < 44100U) continue;
            if (r > 0 && candidate_rates[r] == req_rate) continue;
            for (size_t c = 0; c < 2 && chosen_fmt < 0; c++) {
                for (size_t f = 0; f < 5 && chosen_fmt < 0; f++) {
                    hw_params_any(&params);
                    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_REFINE, &params) < 0) {
                        continue;
                    }
                    hw_param_set_mask(&params, SNDRV_PCM_HW_PARAM_ACCESS, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
                    hw_param_set_mask(&params, SNDRV_PCM_HW_PARAM_FORMAT, candidate_formats[f]);
                    hw_param_set_minmax(&params, SNDRV_PCM_HW_PARAM_CHANNELS, candidate_chs[c], candidate_chs[c]);
                    hw_param_set_minmax(&params, SNDRV_PCM_HW_PARAM_RATE, candidate_rates[r], candidate_rates[r]);
                    hw_param_set_minmax(&params, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, req_period, req_period * 2);
                    hw_param_set_minmax(&params, SNDRV_PCM_HW_PARAM_BUFFER_SIZE, req_period * buf_mults[bm], req_period * 8);

                    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS, &params) == 0) {
                        chosen_fmt = candidate_formats[f];
                        chosen_bits = candidate_bits[f];
                    }
                }
            }
        }
    }

    if (chosen_fmt < 0) {
        close(fd);
        return -1;
    }

    uint32_t actual_rate = params.intervals[SNDRV_PCM_HW_PARAM_RATE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    uint32_t actual_ch   = params.intervals[SNDRV_PCM_HW_PARAM_CHANNELS - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    uint32_t actual_per  = params.intervals[SNDRV_PCM_HW_PARAM_PERIOD_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    uint32_t actual_buf  = params.intervals[SNDRV_PCM_HW_PARAM_BUFFER_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;

    struct snd_pcm_sw_params sw;
    memset(&sw, 0, sizeof(sw));
    sw.tstamp_mode = SNDRV_PCM_TSTAMP_ENABLE;
    sw.period_step = 1;
    sw.avail_min   = actual_per ? actual_per : req_period;
    uint32_t base_per = actual_per ? actual_per : req_period;
    sw.start_threshold = (actual_buf >= base_per * 3U) ? (base_per * 2U) : base_per;
    sw.stop_threshold  = actual_buf ? actual_buf : base_per * 4U;
    sw.xfer_align      = 1;
    ioctl(fd, SNDRV_PCM_IOCTL_SW_PARAMS, &sw);

    if (ioctl(fd, SNDRV_PCM_IOCTL_PREPARE, 0) < 0) {
        close(fd);
        return -1;
    }

    hw->fd = fd;
    snprintf(hw->device_path, sizeof(hw->device_path), "%s", dev_path);
    hw->is_virtual = false;
    hw->is_bluetooth = false;
    hw->sample_rate = actual_rate ? actual_rate : req_rate;
    hw->channels = actual_ch ? actual_ch : 2;
    hw->period_frames = actual_per ? actual_per : req_period;
    hw->buffer_frames = actual_buf ? actual_buf : req_period * 4;
    hw->sndrv_format = chosen_fmt;
    hw->format_bits = chosen_bits;
    return 0;
}

static void format_bt_hw_label(bool has_tr, const char *alias, const char *mac, const char *codec,
                                 char *dev_path, size_t dev_len,
                                 char *pb_id, size_t pb_len) {
    if (!has_tr) return;
    if (dev_path && dev_len > 0) {
        snprintf(dev_path, dev_len, "BlueZ: %.32s (%s)",
                 (alias && alias[0]) ? alias : (mac ? mac : "Bluetooth"),
                 (codec && codec[0]) ? codec : "A2DP");
    }
    if (pb_id && pb_len > 0 && mac && mac[0]) {
        snprintf(pb_id, pb_len, "bluez:%.48s", mac);
    }
}

static void resolve_dev_id_to_path(const char *id, char *out_path, size_t out_len) {
    unsigned c = 0, d = 0;
    if (id && sscanf(id, "hw:%u,%u", &c, &d) == 2) {
        snprintf(out_path, out_len, "/dev/snd/pcmC%uD%up", c, d);
    } else if (id) {
        snprintf(out_path, out_len, "%s", id);
    } else {
        out_path[0] = '\0';
    }
}

static int apply_playback_endpoint(const char *requested_id, uint32_t rate, uint32_t period,
                                   uint32_t req_bits, bool force_virtual, bool force_bt,
                                   const lwasapi_device_list_t *pre_dl) {
    const char *req = (requested_id && requested_id[0]) ? requested_id : "auto";
    bool auto_mode  = (strcmp(req, "auto") == 0);
    bool want_bt    = (force_bt || strcmp(req, "bluetooth") == 0 || strncmp(req, "bluez:", 6) == 0);
    uint32_t target_bits = (req_bits == 16U || req_bits == 24U || req_bits == 32U)
                           ? req_bits
                           : ((g_pref_format_bits == 16U || g_pref_format_bits == 24U || g_pref_format_bits == 32U)
                              ? g_pref_format_bits : 32U);

    int old_fd = -1;
    pthread_mutex_lock(&g_hw_io_lock);
    pthread_mutex_lock(&g_hw_lock);
    old_fd = g_hw.fd;
    if (old_fd >= 0) {
        g_hw.fd = -1;
        g_hw.is_virtual = true;
        snprintf(g_hw.device_path, sizeof(g_hw.device_path), "WaveRT-Virtual-Clock");
    }
    pthread_mutex_unlock(&g_hw_lock);
    if (old_fd >= 0) close(old_fd);
    pthread_mutex_unlock(&g_hw_io_lock);

    kernel_wavert_hw_t next_hw;
    memset(&next_hw, 0, sizeof(next_hw));
    next_hw.fd = -1;
    char next_id[64] = "virtual";

    if (!force_virtual && (want_bt || auto_mode)) {
        pthread_mutex_lock(&g_bt.state_lock);
        bool bt_ready = (g_bt.has_bluez_transport && g_bt.direct_socket_mode && g_bt.bt_fd >= 0);
        char bt_mac[32], bt_alias[64], bt_codec[16];
        uint32_t bt_rate = g_bt.bt_sample_rate;
        snprintf(bt_mac, sizeof(bt_mac), "%s", g_bt.mac_address);
        snprintf(bt_alias, sizeof(bt_alias), "%s", g_bt.device_alias);
        snprintf(bt_codec, sizeof(bt_codec), "%s", g_bt.codec_name);
        pthread_mutex_unlock(&g_bt.state_lock);

        if (bt_ready && strncmp(req, "bluez:", 6) == 0 && bt_mac[0] &&
            strcasecmp(req + 6, bt_mac) != 0) {
            bt_ready = false;
        }
        if (bt_ready) {
            next_hw.fd = -1;
            next_hw.is_virtual = false;
            next_hw.is_bluetooth = true;
            next_hw.sample_rate = rate;
            next_hw.channels = 2;
            next_hw.period_frames = period;
            next_hw.buffer_frames = period * 4;
            next_hw.sndrv_format = SNDRV_PCM_FORMAT_FLOAT_LE;
            next_hw.format_bits = target_bits;
            format_bt_hw_label(true, bt_alias, bt_mac, bt_codec,
                               next_hw.device_path, sizeof(next_hw.device_path),
                               next_id, sizeof(next_id));
            if (strcmp(next_id, "virtual") == 0) {
                snprintf(next_id, sizeof(next_id), "%s", bt_mac[0] ? bt_mac : "bluetooth");
            }
            lwasapi_bluez_reconfigure_rate(&g_bt, rate);
            lwasapi_bluez_set_tx_enabled(&g_bt, true);

            pthread_mutex_lock(&g_hw_lock);
            g_hw = next_hw;
            snprintf(g_active_playback_id, sizeof(g_active_playback_id), "%s", next_id);
            pthread_mutex_unlock(&g_hw_lock);

            fprintf(stderr, "[LWASAPI-audiodg] Active Pure BlueZ A2DP endpoint: %s (%u Hz -> BT %u Hz %s)\n",
                    next_hw.device_path, next_hw.sample_rate, bt_rate,
                    bt_codec[0] ? bt_codec : "A2DP");
            lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                               "ENGINE_ENDPOINT", next_id,
                               "driver=bluez-a2dp dev=%s rate=%u codec=%s bt_rate=%u",
                               next_hw.device_path, next_hw.sample_rate,
                               bt_codec[0] ? bt_codec : "A2DP", bt_rate);
            return 0;
        }
        if (force_bt) {
            fprintf(stderr, "[LWASAPI-audiodg] Notice: BlueZ A2DP transport not yet acquired, probing kernel endpoints...\n");
        }
    }

    lwasapi_bluez_set_tx_enabled(&g_bt, false);

    if (!force_virtual && !auto_mode && !want_bt && strcmp(req, "virtual") != 0) {
        char dev_path[64];
        resolve_dev_id_to_path(req, dev_path, sizeof(dev_path));
        if (open_kernel_pcm_device(dev_path, rate, period, target_bits, &next_hw) == 0) {
            unsigned c = 0, d = 0;
            if (sscanf(dev_path, "/dev/snd/pcmC%uD%up", &c, &d) == 2) {
                snprintf(next_id, sizeof(next_id), "hw:%u,%u", c, d);
            } else {
                snprintf(next_id, sizeof(next_id), "%s", req);
            }
            pthread_mutex_lock(&g_hw_lock);
            g_hw = next_hw;
            snprintf(g_active_playback_id, sizeof(g_active_playback_id), "%s", next_id);
            pthread_mutex_unlock(&g_hw_lock);

            fprintf(stderr, "[LWASAPI-audiodg] Opened direct kernel WaveRT endpoint: %s (%u Hz, %u ch, %u-bit, period=%u frames / %.2f ms)\n",
                    next_hw.device_path, next_hw.sample_rate, next_hw.channels, next_hw.format_bits,
                    next_hw.period_frames, (1000.0 * next_hw.period_frames) / next_hw.sample_rate);
            lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                               "ENGINE_ENDPOINT", next_id,
                               "driver=kernel-pcm req=%s dev=%s rate=%u ch=%u bits=%u per=%u",
                               req, next_hw.device_path, next_hw.sample_rate,
                               next_hw.channels, next_hw.format_bits, next_hw.period_frames);
            return 0;
        }
        fprintf(stderr, "[LWASAPI-audiodg] Notice: %s busy or unavailable (%s), probing fallback endpoints...\n",
                dev_path, strerror(errno));
        lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_WARN, 0, 0,
                           "ENDPOINT_BUSY", req, "dev=%s err=%s", dev_path, strerror(errno));
        if (strncmp(req, "hw:", 3) == 0 || strncmp(req, "/dev/snd/", 9) == 0) {
            /* Fall through to dynamic probe if requested device failed */
        }
    }

    if (!force_virtual && strcmp(req, "virtual") != 0) {
        lwasapi_device_list_t local_dl;
        const lwasapi_device_list_t *dlp = pre_dl;
        if (!dlp) {
            lwasapi_bluez_enumerate_devices(&g_bt, NULL, NULL, &local_dl);
            dlp = &local_dl;
        }

        int idx_order[LWASAPI_MAX_DEVICES];
        int scores[LWASAPI_MAX_DEVICES];
        int cand_count = 0;

        for (uint32_t i = 0; i < dlp->num_devices; i++) {
            const lwasapi_device_entry_t *e = &dlp->devices[i];
            if (e->is_capture || e->is_bluetooth) continue;
            int sc = lwasapi_hw_analog_score_card(e->card_index, e->name);
            if (e->device_index == 0) sc += 10;
            idx_order[cand_count] = (int)i;
            scores[cand_count] = sc;
            cand_count++;
        }

        for (int a = 0; a < cand_count; a++) {
            for (int b = a + 1; b < cand_count; b++) {
                if (scores[b] > scores[a]) {
                    int ts = scores[a]; scores[a] = scores[b]; scores[b] = ts;
                    int ti = idx_order[a]; idx_order[a] = idx_order[b]; idx_order[b] = ti;
                }
            }
        }

        for (int k = 0; k < cand_count; k++) {
            const lwasapi_device_entry_t *e = &dlp->devices[idx_order[k]];
            char dev_path[64];
            snprintf(dev_path, sizeof(dev_path), "/dev/snd/pcmC%uD%up", e->card_index, e->device_index);
            if (open_kernel_pcm_device(dev_path, rate, period, target_bits, &next_hw) == 0) {
                snprintf(next_id, sizeof(next_id), "%s", e->id);
                pthread_mutex_lock(&g_hw_lock);
                g_hw = next_hw;
                snprintf(g_active_playback_id, sizeof(g_active_playback_id), "%s", next_id);
                pthread_mutex_unlock(&g_hw_lock);

                fprintf(stderr, "[LWASAPI-audiodg] Auto-bound direct kernel WaveRT endpoint: %s (%s | %u Hz, %u ch, %u-bit, period=%u frames / %.2f ms)\n",
                        next_hw.device_path, e->name, next_hw.sample_rate, next_hw.channels, next_hw.format_bits,
                        next_hw.period_frames, (1000.0 * next_hw.period_frames) / next_hw.sample_rate);
                lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                                   "ENGINE_ENDPOINT", next_id,
                                   "driver=auto-kernel name=%s dev=%s rate=%u ch=%u bits=%u per=%u",
                                   e->name, next_hw.device_path, next_hw.sample_rate,
                                   next_hw.channels, next_hw.format_bits, next_hw.period_frames);
                return 0;
            }
        }
    }

    /* High-precision monotonic WaveRT virtual hardware clock fallback */
    next_hw.fd = -1;
    snprintf(next_hw.device_path, sizeof(next_hw.device_path), "WaveRT-Virtual-Clock");
    snprintf(next_id, sizeof(next_id), "virtual");
    next_hw.is_virtual = true;
    next_hw.is_bluetooth = false;
    next_hw.sample_rate = rate;
    next_hw.channels = 2;
    next_hw.period_frames = period;
    next_hw.buffer_frames = period * 4;
    next_hw.sndrv_format = SNDRV_PCM_FORMAT_FLOAT_LE;
    next_hw.format_bits = target_bits;

    pthread_mutex_lock(&g_hw_lock);
    g_hw = next_hw;
    snprintf(g_active_playback_id, sizeof(g_active_playback_id), "%s", next_id);
    pthread_mutex_unlock(&g_hw_lock);

    fprintf(stderr, "[LWASAPI-audiodg] Initialized deterministic WaveRT clock (%u Hz, period=%u frames / %.2f ms)\n",
            next_hw.sample_rate, next_hw.period_frames, (1000.0 * next_hw.period_frames) / next_hw.sample_rate);
    lwasapi_debug_emit(LWASAPI_DBG_ENGINE,
                       force_virtual ? LWASAPI_DBG_INFO : LWASAPI_DBG_WARN,
                       0, 0, "ENGINE_ENDPOINT", "virtual",
                       "driver=virtual-clock req=%s rate=%u per=%u",
                       req ? req : "auto", next_hw.sample_rate, next_hw.period_frames);
    return 0;
}

static void init_hardware(const char *requested_dev, uint32_t rate, uint32_t period, uint32_t bits,
                          bool force_virtual, bool force_bt) {
    pthread_mutex_lock(&g_reconf_lock);
    pthread_mutex_lock(&g_hw_lock);
    g_hw.fd = -1;
    if (requested_dev && requested_dev[0] && strcmp(requested_dev, "auto") != 0) {
        snprintf(g_target_playback_id, sizeof(g_target_playback_id), "%s", requested_dev);
    }
    char target_cap[64], target_pb[64];
    uint32_t pref_cap = g_pref_cap_sample_rate;
    snprintf(target_cap, sizeof(target_cap), "%s", g_target_capture_id);
    snprintf(target_pb, sizeof(target_pb), "%s", g_target_playback_id);
    pthread_mutex_unlock(&g_hw_lock);

    if (!force_virtual) {
        uint32_t cap_rate = resolve_cap_sample_rate(rate, pref_cap);
        lwasapi_bluez_init_rates(&g_bt, rate, cap_rate, target_cap, target_pb);
        lwasapi_bluez_set_sco_enabled(&g_bt, g_saved_bt_cfg.hfp_sco_enabled != 0);
        lwasapi_bluez_set_codec_config(&g_bt, &g_saved_bt_cfg);
        if (g_has_saved_vol_state) {
            lwasapi_bluez_set_volume(&g_bt, (int)g_saved_desktop_vol_pct, g_saved_desktop_muted ? 1 : 0);
            lwasapi_bluez_set_mic_volume(&g_bt, (int)g_saved_mic_vol_pct, g_saved_mic_muted ? 1 : 0);
            lwasapi_bluez_set_mic_gain_ppm(&g_bt, g_saved_mic_gain_ppm, (int)g_saved_mic_vol_pct, g_saved_mic_muted ? 1 : 0);
        }
        pthread_mutex_lock(&g_bt.state_lock);
        char cap_id[64];
        snprintf(cap_id, sizeof(cap_id), "%s", g_bt.cap_device_id);
        pthread_mutex_unlock(&g_bt.state_lock);
        if (cap_id[0]) {
            pthread_mutex_lock(&g_hw_lock);
            snprintf(g_active_capture_id, sizeof(g_active_capture_id), "%s", cap_id);
            pthread_mutex_unlock(&g_hw_lock);
        }
    }

    apply_playback_endpoint(force_bt ? "bluetooth" : target_pb,
                            rate, period, bits, force_virtual, force_bt, NULL);
    hw_analog_probe_and_sync(true);
    pthread_mutex_unlock(&g_reconf_lock);
}

/* Elevate audio engine thread to SCHED_FIFO (Windows MMCSS 'Pro Audio' equivalent) */
static void enable_mmcss_realtime(void) {
    static const int prios[] = { 65, 50, 20, 10 };
    struct sched_param sp;
    for (size_t i = 0; i < sizeof(prios) / sizeof(prios[0]); i++) {
        memset(&sp, 0, sizeof(sp));
        sp.sched_priority = prios[i];
        if (pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp) == 0) {
            g_rt_priority = (uint32_t)sp.sched_priority;
            fprintf(stderr, "[LWASAPI-MMCSS] Real-time SCHED_FIFO priority %u enabled for audio graph thread.\n", g_rt_priority);
            return;
        }
    }
    g_rt_priority = 0;
    fprintf(stderr, "[LWASAPI-MMCSS] Running in standard scheduler priority (grant rtprio for SCHED_FIFO).\n");
}

static inline float soft_clip(float x) {
    if (x > 1.0f) return 1.0f;
    if (x < -1.0f) return -1.0f;
    return x;
}

static inline float shape_hw_analog_sample(float x, float gain, bool softknee) {
    float v = x * gain;
    if (softknee) {
        /* Smooth Class-AB / TPA6120A2 soft-knee ceiling preserving analog headroom */
        float av = fabsf(v);
        if (av > 0.72f) {
            float excess = av - 0.72f;
            float shaped = 0.72f + 0.275f * tanhf(excess / 0.45f);
            v = (v >= 0.0f) ? shaped : -shaped;
        }
        return v;
    }
    return soft_clip(v);
}

/* Read frames from a client's lock-free shared memory ring buffer (with linear resampling if needed) */
static void consume_session_frames(engine_session_t *s, float *mix_buf, uint32_t out_frames, uint32_t out_channels, uint32_t hw_rate, bool replace_output, uint64_t now_ns) {
    lwasapi_shm_ring_t *r = s->ring;
    if (!r || !atomic_load_explicit(&r->active, memory_order_acquire)) {
        s->stream_live = false;
        s->underrun_pending = false;
        s->resample_phase = 0.0;
        return;
    }

    uint32_t ch = s->channels;
    if (ch == 0 || ch > LWASAPI_MAX_CHANNELS) {
        return;
    }
    uint32_t client_rate = s->sample_rate ? s->sample_rate : hw_rate;

    uint64_t w = atomic_load_explicit(&r->write_frame, memory_order_acquire);
    uint64_t rd = atomic_load_explicit(&r->read_frame, memory_order_relaxed);
    if (w < rd) {
        s->stream_live = false;
        s->underrun_pending = false;
        s->resample_phase = 0.0;
        return;
    }
    uint64_t avail = w - rd;
    uint64_t min_avail = (client_rate == hw_rate) ? 1ULL : 2ULL;
    if (avail < min_avail) {
        if (s->stream_live) {
            /* Disarm after 500ms of silence so naturally drained/idle streams don't report XRUNs on next sound */
            if (now_ns > s->last_audio_ns && (now_ns - s->last_audio_ns) > 500000000ULL) {
                s->stream_live = false;
                s->underrun_pending = false;
            } else {
                s->underrun_pending = true;
            }
        }
        return;
    }

    if (s->underrun_pending) {
        atomic_fetch_add_explicit(&r->xruns, 1, memory_order_relaxed);
        s->underrun_pending = false;
    }

    float vol = (atomic_load_explicit(&r->muted, memory_order_relaxed) != 0)
                ? 0.0f
                : ((float)atomic_load_explicit(&r->volume_milli, memory_order_relaxed) / 1000.0f);
    if (!(vol >= 0.0f)) vol = 0.0f;
    if (vol > 4.0f) vol = 4.0f;

    if (client_rate == hw_rate) {
        uint32_t to_read = (avail < out_frames) ? (uint32_t)avail : out_frames;
        for (uint32_t f = 0; f < to_read; f++) {
            uint32_t idx = (uint32_t)((rd + f) & LWASAPI_RING_MASK) * ch;
            for (uint32_t c = 0; c < out_channels; c++) {
                float sample = r->pcm_data[idx + (c % ch)] * vol;
                if (replace_output) {
                    mix_buf[f * out_channels + c] = sample;
                } else {
                    mix_buf[f * out_channels + c] += sample;
                }
            }
        }
        atomic_store_explicit(&r->read_frame, rd + to_read, memory_order_release);
    } else {
        /* Real-time fractional linear resampler (Windows Audio Engine APO equivalent) */
        double step = (double)client_rate / (double)hw_rate;
        double phase = s->resample_phase;
        for (uint32_t f = 0; f < out_frames; f++) {
            uint64_t i0 = (uint64_t)phase;
            if (i0 + 1 >= avail) break;
            float frac = (float)(phase - (double)i0);
            uint32_t idx0 = (uint32_t)((rd + i0) & LWASAPI_RING_MASK) * ch;
            uint32_t idx1 = (uint32_t)((rd + i0 + 1) & LWASAPI_RING_MASK) * ch;
            for (uint32_t c = 0; c < out_channels; c++) {
                float s0 = r->pcm_data[idx0 + (c % ch)];
                float s1 = r->pcm_data[idx1 + (c % ch)];
                float sample = (s0 + (s1 - s0) * frac) * vol;
                if (replace_output) {
                    mix_buf[f * out_channels + c] = sample;
                } else {
                    mix_buf[f * out_channels + c] += sample;
                }
            }
            phase += step;
        }
        uint64_t advance = (uint64_t)phase;
        if (advance > avail) advance = avail;
        s->resample_phase = phase - (double)advance;
        atomic_store_explicit(&r->read_frame, rd + advance, memory_order_release);
    }

    s->stream_live = true;
    s->last_audio_ns = now_ns;
}

/* ============================================================================
 * Real-Time Audio & MIDI Graph Thread (Windows audiodg.exe + WaveRT Pump)
 * ============================================================================ */

typedef struct {
    float hist_l[3];
    float hist_r[3];
    bool  valid;
} mic_cubic_state_t;

static inline float eng_catmull_rom(float p0, float p1, float p2, float p3, float t) {
    float t2 = t * t;
    float t3 = t2 * t;
    return 0.5f * ((2.0f * p1) +
                   (-p0 + p2) * t +
                   (2.0f * p0 - 5.0f * p1 + 4.0f * p2 - p3) * t2 +
                   (-p0 + 3.0f * p1 - 3.0f * p2 + p3) * t3);
}

static void resample_mic_native_to_engine(const float *in, uint32_t in_frames,
                                          float *out, uint32_t out_frames,
                                          mic_cubic_state_t *st) {
    if (!in || !out || in_frames == 0U || out_frames == 0U) return;
    if (in_frames == out_frames) {
        memcpy(out, in, (size_t)out_frames * 2U * sizeof(float));
        for (int k = 0; k < 3; k++) {
            int32_t src_i = (int32_t)in_frames - 3 + k;
            uint32_t si = (src_i >= 0) ? (uint32_t)src_i : 0U;
            st->hist_l[k] = in[si * 2U + 0U];
            st->hist_r[k] = in[si * 2U + 1U];
        }
        st->valid = true;
        return;
    }

    if (!st->valid) {
        for (int k = 0; k < 3; k++) {
            st->hist_l[k] = in[0];
            st->hist_r[k] = in[1];
        }
        st->valid = true;
    }

    /* Evaluate at step across [hist[0..2], in[0..in_frames-1]], index base 3 = in[0] */
    double step = (double)in_frames / (double)out_frames;
    int32_t total = (int32_t)in_frames + 3;
    for (uint32_t f = 0; f < out_frames; f++) {
        double p = 2.0 + (double)f * step;
        int32_t i0 = (int32_t)floor(p);
        float t = (float)(p - (double)i0);
        int32_t idx[4] = { i0 - 1, i0, i0 + 1, i0 + 2 };
        float sl[4], sr[4];
        for (int m = 0; m < 4; m++) {
            int32_t j = idx[m];
            if (j < 0) j = 0;
            if (j >= total) j = total - 1;
            if (j < 3) {
                sl[m] = st->hist_l[j];
                sr[m] = st->hist_r[j];
            } else {
                uint32_t u = (uint32_t)(j - 3);
                sl[m] = in[u * 2U + 0U];
                sr[m] = in[u * 2U + 1U];
            }
        }
        out[f * 2U + 0U] = soft_clip(eng_catmull_rom(sl[0], sl[1], sl[2], sl[3], t));
        out[f * 2U + 1U] = soft_clip(eng_catmull_rom(sr[0], sr[1], sr[2], sr[3], t));
    }

    for (int k = 0; k < 3; k++) {
        int32_t j = total - 3 + k;
        if (j < 3) {
            st->hist_l[k] = st->hist_l[(j >= 0) ? j : 0];
            st->hist_r[k] = st->hist_r[(j >= 0) ? j : 0];
        } else {
            uint32_t u = (uint32_t)(j - 3);
            st->hist_l[k] = in[u * 2U + 0U];
            st->hist_r[k] = in[u * 2U + 1U];
        }
    }
}

static void *audio_engine_thread(void *arg) {
    (void)arg;
    enable_mmcss_realtime();

    float   *mix_buf        = calloc(LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS, sizeof(float));
    float   *pa_stereo      = calloc(LWASAPI_MAX_PERIOD * 2, sizeof(float));
    float   *mic_buf        = calloc(LWASAPI_MAX_PERIOD * 2, sizeof(float));
    float   *native_mic_buf = calloc(LWASAPI_MAX_PERIOD * 2, sizeof(float));
    int32_t *s32_buf        = calloc(LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS, sizeof(int32_t));
    uint8_t *s24_buf        = calloc(LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS, 3);
    int16_t *s16_buf        = calloc(LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS, sizeof(int16_t));
    uint64_t mic_read_cursor = 0;
    mic_cubic_state_t mic_up_st;
    memset(&mic_up_st, 0, sizeof(mic_up_st));
    uint32_t last_mic_up_rate = 0;

    struct timespec next_wakeup;
    clock_gettime(CLOCK_MONOTONIC, &next_wakeup);
    uint64_t expected_wake_ns = 0;
    bool have_expected_wake = false;

    while (g_running) {
        pthread_mutex_lock(&g_hw_lock);
        uint32_t period = g_hw.period_frames;
        uint32_t ch = g_hw.channels ? g_hw.channels : 2;
        uint32_t rate = g_hw.sample_rate ? g_hw.sample_rate : 48000;
        pthread_mutex_unlock(&g_hw_lock);

        if (period < 64) period = 64;
        if (period > LWASAPI_MAX_PERIOD) period = LWASAPI_MAX_PERIOD;
        if (ch > LWASAPI_MAX_CHANNELS) ch = LWASAPI_MAX_CHANNELS;
        uint64_t period_ns = (1000000000ULL * period) / rate;

        memset(mix_buf, 0, period * ch * sizeof(float));
        uint64_t now_ns = monotonic_ns();
        int64_t wake_jitter_ns = have_expected_wake ? (int64_t)(now_ns - expected_wake_ns) : 0;

        /* 1. Read non-blocking hardware capture at native device rate + run Mic APO at native rate */
        uint32_t mic_frames = period, mic_rate = rate;
        lwasapi_bluez_read_capture_native(&g_bt, native_mic_buf, LWASAPI_MAX_PERIOD,
                                          &mic_frames, &mic_rate, period, rate, &mic_read_cursor);
        atomic_store_explicit(&g_cap_read_cursor, mic_read_cursor, memory_order_relaxed);
        if (mic_rate != last_mic_up_rate) {
            memset(&mic_up_st, 0, sizeof(mic_up_st));
            last_mic_up_rate = mic_rate;
        }
        if (g_hw_analog) {
            float cap_makeup = lwasapi_hw_analog_get_cap_makeup_linear(g_hw_analog);
            if (fabsf(cap_makeup - 1.0f) > 0.001f) {
                for (uint32_t k = 0; k < mic_frames * 2U; k++) {
                    native_mic_buf[k] = soft_clip(native_mic_buf[k] * cap_makeup);
                }
            }
        }
        lwasapi_mic_apo_cfg_t mic_cfg;
        pthread_mutex_lock(&g_sessions_lock);
        mic_cfg = g_mic_apo;
        pthread_mutex_unlock(&g_sessions_lock);
        process_capture_mic_apo(native_mic_buf, mic_frames, mic_rate, &mic_cfg);

        /* 2. Poll hardware RawMIDI and ALSA Sequencer MIDI Hub non-blockingly */
        lwasapi_midi_event_t midi_ev[64];
        uint32_t num_midi = lwasapi_midi_poll_events(&g_midi, midi_ev, 64, now_ns, rate, period);

        pthread_mutex_lock(&g_sessions_lock);
        if (num_midi > 0) {
            for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
                if (g_sessions[i].in_use && g_sessions[i].ring) {
                    for (uint32_t m = 0; m < num_midi; m++) {
                        lwasapi_midi_push_to_ring(g_sessions[i].ring, &midi_ev[m]);
                    }
                }
            }
        }

        uint64_t pa_mix_ns = 0;
        bool exclusive_active = (g_exclusive_session_id >= 0);
        if (exclusive_active) {
            /* WASAPI EXCLUSIVE MODE: Only the exclusive owner drives hardware */
            for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
                if (g_sessions[i].in_use && (int32_t)g_sessions[i].session_id == g_exclusive_session_id) {
                    atomic_store_explicit(&g_sessions[i].ring->hw_clock_ns, now_ns, memory_order_relaxed);
                    consume_session_frames(&g_sessions[i], mix_buf, period, ch, rate, true, now_ns);
                    break;
                }
            }
        } else {
            /* WASAPI SHARED MODE: Mix all active LWASAPI shared sessions */
            for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
                if (g_sessions[i].in_use && g_sessions[i].ring) {
                    atomic_store_explicit(&g_sessions[i].ring->hw_clock_ns, now_ns, memory_order_relaxed);
                    if (!g_sessions[i].is_capture) {
                        consume_session_frames(&g_sessions[i], mix_buf, period, ch, rate, false, now_ns);
                    }
                }
            }
            /* Mix native PulseAudio socket clients (Firefox, Discord, KDE, Steam, paplay) */
            if (g_pa_srv) {
                uint64_t t_pa0 = monotonic_ns();
                if (ch == 2) {
                    lwasapi_pa_server_mix_playback(g_pa_srv, mix_buf, period, rate);
                } else {
                    memset(pa_stereo, 0, period * 2 * sizeof(float));
                    lwasapi_pa_server_mix_playback(g_pa_srv, pa_stereo, period, rate);
                    for (uint32_t f = 0; f < period; f++) {
                        if (ch == 1) {
                            mix_buf[f] += 0.5f * (pa_stereo[f * 2 + 0] + pa_stereo[f * 2 + 1]);
                        } else {
                            for (uint32_t c = 0; c < ch; c++) {
                                mix_buf[f * ch + c] += pa_stereo[f * 2 + (c & 1U)];
                            }
                        }
                    }
                }
                pa_mix_ns += monotonic_ns() - t_pa0;
            }
        }

        /* 3. Run Salsa 10-Band Real-Time Dynamic EQ & Spectrum Analyzer APO */
        uint64_t t_eq0 = monotonic_ns();
        process_salsa_dynamic_eq(mix_buf, period, ch, rate);
        uint64_t eq_ns = monotonic_ns() - t_eq0;

        /* Prepare stereo loopback view of post-EQ mix for loopback/monitor captures */
        const float *monitor_stereo = mix_buf;
        if (ch != 2) {
            for (uint32_t f = 0; f < period; f++) {
                pa_stereo[f * 2 + 0] = mix_buf[f * ch + 0];
                pa_stereo[f * 2 + 1] = (ch > 1) ? mix_buf[f * ch + 1] : mix_buf[f * ch + 0];
            }
            monitor_stereo = pa_stereo;
        }

        /* 4. Push native capture to PA, and upsample to engine rate only when WASAPI/sidetone needs it */
        bool need_eng_mic = (mic_cfg.enabled && mic_cfg.sidetone_enabled && mic_cfg.sidetone_pct > 0U);
        if (!need_eng_mic) {
            for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
                if (g_sessions[i].in_use && g_sessions[i].ring && !g_sessions[i].is_loopback) {
                    need_eng_mic = true;
                    break;
                }
            }
        }
        if (need_eng_mic) {
            resample_mic_native_to_engine(native_mic_buf, mic_frames, mic_buf, period, &mic_up_st);
        }

        for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
            if (g_sessions[i].in_use && g_sessions[i].ring) {
                lwasapi_shm_ring_t *r = g_sessions[i].ring;
                const float *cap_src = g_sessions[i].is_loopback ? monitor_stereo : mic_buf;
                uint64_t cw = atomic_load_explicit(&r->cap_write_frame, memory_order_relaxed);
                for (uint32_t f = 0; f < period; f++) {
                    uint32_t idx = (uint32_t)((cw + f) & LWASAPI_RING_MASK) * 2;
                    r->cap_pcm_data[idx + 0] = cap_src[f * 2 + 0];
                    r->cap_pcm_data[idx + 1] = cap_src[f * 2 + 1];
                }
                atomic_store_explicit(&r->cap_write_frame, cw + period, memory_order_release);

                uint64_t one = 1;
                if (write(g_sessions[i].event_fd, &one, sizeof(one)) < 0) {}
            }
        }
        pthread_mutex_unlock(&g_sessions_lock);

        if (g_pa_srv) {
            uint64_t t_pa1 = monotonic_ns();
            lwasapi_pa_server_push_capture(g_pa_srv, native_mic_buf, mic_frames, mic_rate,
                                           monitor_stereo, period, rate);
            pa_mix_ns += monotonic_ns() - t_pa1;
        }

        if (mic_cfg.enabled && mic_cfg.sidetone_enabled && mic_cfg.sidetone_pct > 0U) {
            float sg = (float)mic_cfg.sidetone_pct / 100.0f;
            if (sg > 1.0f) sg = 1.0f;
            for (uint32_t f = 0; f < period; f++) {
                float sl = mic_buf[f * 2 + 0] * sg;
                float sr = mic_buf[f * 2 + 1] * sg;
                if (ch == 1U) {
                    mix_buf[f] += 0.5f * (sl + sr);
                } else {
                    for (uint32_t c = 0; c < ch; c++) {
                        mix_buf[f * ch + c] += (c & 1U) ? sr : sl;
                    }
                }
            }
        }

        /* 5. Write mixed/exclusive period directly to BlueZ A2DP or kernel /dev/snd/pcm* */
        bool need_timer_pace = true;
        uint64_t t_hw0 = monotonic_ns();
        pthread_mutex_lock(&g_hw_io_lock);
        pthread_mutex_lock(&g_hw_lock);
        kernel_wavert_hw_t hw_snap = g_hw;
        pthread_mutex_unlock(&g_hw_lock);

        if (hw_snap.period_frames == period && hw_snap.channels == ch && hw_snap.sample_rate == rate) {
            if (hw_snap.is_bluetooth && g_bt.initialized) {
                for (uint32_t k = 0; k < period * ch; k++) {
                    mix_buf[k] = soft_clip(mix_buf[k]);
                }
                lwasapi_bluez_write_period(&g_bt, mix_buf, period, ch);
            } else if (!hw_snap.is_virtual && hw_snap.fd >= 0) {
                bool d_muted = __atomic_load_n(&g_bt.desktop_muted, __ATOMIC_RELAXED);
                uint32_t d_vol = __atomic_load_n(&g_bt.desktop_volume_pct, __ATOMIC_RELAXED);
                float master_gain = d_muted ? 0.0f : ((float)d_vol / 100.0f);
                bool softknee = false;
                if (g_hw_analog) {
                    if (lwasapi_hw_analog_is_dac_unity_active(g_hw_analog)) {
                        if (d_muted) {
                            master_gain = 0.0f;
                        } else if (d_vol <= 100U) {
                            master_gain = 1.0f;
                        } else {
                            master_gain = (float)d_vol / 100.0f;
                        }
                    }
                    master_gain *= lwasapi_hw_analog_get_drive_linear(g_hw_analog);
                    softknee = lwasapi_hw_analog_softknee_active(g_hw_analog);
                }
                struct snd_xferi xfer;
                memset(&xfer, 0, sizeof(xfer));
                xfer.frames = period;

                if (hw_snap.sndrv_format == SNDRV_PCM_FORMAT_S32_LE) {
                    for (uint32_t k = 0; k < period * ch; k++) {
                        s32_buf[k] = (int32_t)(shape_hw_analog_sample(mix_buf[k], master_gain, softknee) * 2147483520.0f);
                    }
                    xfer.buf = s32_buf;
                } else if (hw_snap.sndrv_format == SNDRV_PCM_FORMAT_S24_LE) {
                    for (uint32_t k = 0; k < period * ch; k++) {
                        s32_buf[k] = (int32_t)(shape_hw_analog_sample(mix_buf[k], master_gain, softknee) * 8388607.0f);
                    }
                    xfer.buf = s32_buf;
                } else if (hw_snap.sndrv_format == SNDRV_PCM_FORMAT_S24_3LE) {
                    for (uint32_t k = 0; k < period * ch; k++) {
                        int32_t v = (int32_t)(shape_hw_analog_sample(mix_buf[k], master_gain, softknee) * 8388607.0f);
                        s24_buf[k * 3 + 0] = (uint8_t)(v & 0xFF);
                        s24_buf[k * 3 + 1] = (uint8_t)((v >> 8) & 0xFF);
                        s24_buf[k * 3 + 2] = (uint8_t)((v >> 16) & 0xFF);
                    }
                    xfer.buf = s24_buf;
                } else if (hw_snap.sndrv_format == SNDRV_PCM_FORMAT_S16_LE) {
                    for (uint32_t k = 0; k < period * ch; k++) {
                        s16_buf[k] = (int16_t)(shape_hw_analog_sample(mix_buf[k], master_gain, softknee) * 32767.0f);
                    }
                    xfer.buf = s16_buf;
                } else {
                    float *f32_out = (float *)s32_buf;
                    for (uint32_t k = 0; k < period * ch; k++) {
                        f32_out[k] = shape_hw_analog_sample(mix_buf[k], master_gain, softknee);
                    }
                    xfer.buf = f32_out;
                }

                int wr = ioctl(hw_snap.fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &xfer);
                if (wr < 0 && (errno == EPIPE || errno == ESTRPIPE)) {
                    int xerr = errno;
                    uint64_t xc = atomic_fetch_add_explicit(&g_hw_xruns, 1, memory_order_relaxed) + 1ULL;
                    lwasapi_debug_emit_vals(LWASAPI_DBG_ALSA, LWASAPI_DBG_WARN, 0, 0,
                                            (int32_t)xc, xerr, -xerr,
                                            "HW_XRUN", hw_snap.device_path,
                                            "ALSA PCM XRUN #%llu (%s)",
                                            (unsigned long long)xc, strerror(xerr));
                    ioctl(hw_snap.fd, SNDRV_PCM_IOCTL_PREPARE, 0);
                    xfer.result = 0;
                    wr = ioctl(hw_snap.fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &xfer);
                }

                if (wr < 0) {
                    if (errno == ENODEV || errno == EBADFD || errno == EIO) {
                        int dead_err = errno;
                        lwasapi_debug_emit_vals(LWASAPI_DBG_ALSA, LWASAPI_DBG_ERR, 0, 0,
                                                dead_err, 0, -dead_err,
                                                "HW_DEV_LOST", hw_snap.device_path,
                                                "Kernel playback device removed (%s) -> Virtual Clock",
                                                strerror(dead_err));
                        fprintf(stderr, "[LWASAPI-audiodg] Kernel playback device %s removed (%s), dropping to virtual clock until failover.\n",
                                hw_snap.device_path, strerror(dead_err));
                        pthread_mutex_lock(&g_hw_lock);
                        if (g_hw.fd == hw_snap.fd && g_hw.fd >= 0) {
                            close(g_hw.fd);
                            g_hw.fd = -1;
                            g_hw.is_virtual = true;
                            snprintf(g_hw.device_path, sizeof(g_hw.device_path), "WaveRT-Virtual-Clock");
                            snprintf(g_active_playback_id, sizeof(g_active_playback_id), "virtual");
                        }
                        pthread_mutex_unlock(&g_hw_lock);
                    }
                } else {
                    need_timer_pace = false;
                    clock_gettime(CLOCK_MONOTONIC, &next_wakeup);
                }
            }
        }
        pthread_mutex_unlock(&g_hw_io_lock);
        uint64_t t_end = monotonic_ns();
        uint64_t hw_write_ns = t_end - t_hw0;
        /* When kernel ALSA ioctl paces by blocking in WRITEI_FRAMES, exclude blocking wait from RT compute load */
        uint64_t cycle_ns = need_timer_pace ? (t_end - now_ns) : (t_hw0 - now_ns);
        lwasapi_debug_record_rt_period(cycle_ns, wake_jitter_ns, period_ns, eq_ns, pa_mix_ns, hw_write_ns);

        if (need_timer_pace) {
            uint64_t next_ns = (uint64_t)next_wakeup.tv_sec * 1000000000ULL +
                               (uint64_t)next_wakeup.tv_nsec + period_ns;
            uint64_t cur_ns = monotonic_ns();
            if (!have_expected_wake || cur_ns >= next_ns + period_ns) {
                next_ns = cur_ns + period_ns;
            }
            next_wakeup.tv_sec  = (time_t)(next_ns / 1000000000ULL);
            next_wakeup.tv_nsec = (long)(next_ns % 1000000000ULL);
            expected_wake_ns = next_ns;
            have_expected_wake = true;
            clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next_wakeup, NULL);
        } else {
            have_expected_wake = false;
        }

        atomic_fetch_add_explicit(&g_total_cycles, 1, memory_order_relaxed);
    }

    free(mix_buf);
    free(pa_stereo);
    free(mic_buf);
    free(native_mic_buf);
    free(s32_buf);
    free(s24_buf);
    free(s16_buf);
    return NULL;
}

/* ============================================================================
 * SCM_RIGHTS File-Descriptor Passing for Zero-Copy memfd + eventfd Handoff
 * ============================================================================ */

static int send_response_with_fds(int sock, const lwasapi_ipc_resp_t *resp, int fd1, int fd2) {
    struct msghdr msg;
    struct iovec iov;
    memset(&msg, 0, sizeof(msg));

    iov.iov_base = (void *)resp;
    iov.iov_len = sizeof(*resp);
    msg.msg_iov = &iov;
    msg.msg_iovlen = 1;

    char cmsg_buf[CMSG_SPACE(sizeof(int) * 2)];
    if (fd1 >= 0 && fd2 >= 0) {
        memset(cmsg_buf, 0, sizeof(cmsg_buf));
        msg.msg_control = cmsg_buf;
        msg.msg_controllen = sizeof(cmsg_buf);

        struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
        cmsg->cmsg_level = SOL_SOCKET;
        cmsg->cmsg_type = SCM_RIGHTS;
        cmsg->cmsg_len = CMSG_LEN(sizeof(int) * 2);
        int fds[2] = { fd1, fd2 };
        memcpy(CMSG_DATA(cmsg), fds, sizeof(fds));
    }

    return sendmsg(sock, &msg, MSG_NOSIGNAL);
}

static void destroy_session_slot(int slot) {
    if (!g_sessions[slot].in_use) return;
    uint32_t sid  = g_sessions[slot].session_id;
    uint32_t cpid = 0;
    char cname[64] = "";
    if (g_sessions[slot].ring && g_sessions[slot].ring != MAP_FAILED) {
        cpid = g_sessions[slot].ring->client_pid;
        snprintf(cname, sizeof(cname), "%s", g_sessions[slot].ring->client_name);
    }
    if ((int32_t)g_sessions[slot].session_id == g_exclusive_session_id) {
        g_exclusive_session_id = -1;
        fprintf(stderr, "[LWASAPI-audiodg] Exclusive mode released by session %u; Shared Mode restored.\n",
                g_sessions[slot].session_id);
        lwasapi_debug_emit(LWASAPI_DBG_WASAPI, LWASAPI_DBG_INFO, cpid, sid,
                           "EXCL_RELEASE", cname[0] ? cname : "session",
                           "exclusive mode released; shared mode restored");
    }
    lwasapi_debug_emit(LWASAPI_DBG_WASAPI, LWASAPI_DBG_INFO, cpid, sid,
                       "WASAPI_DESTROY", cname[0] ? cname : "session",
                       "session slot=%d closed", slot);
    if (g_sessions[slot].ring && g_sessions[slot].ring != MAP_FAILED) {
        munmap(g_sessions[slot].ring, sizeof(lwasapi_shm_ring_t));
    }
    if (g_sessions[slot].shm_fd >= 0) close(g_sessions[slot].shm_fd);
    if (g_sessions[slot].event_fd >= 0) close(g_sessions[slot].event_fd);
    if (g_sessions[slot].client_sock >= 0) close(g_sessions[slot].client_sock);
    memset(&g_sessions[slot], 0, sizeof(engine_session_t));
}

static void handle_create_session(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));

    int shm_fd = memfd_create("lwasapi_ring", MFD_CLOEXEC);
    int ev_fd  = eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK);
    if (shm_fd < 0 || ev_fd < 0 || ftruncate(shm_fd, sizeof(lwasapi_shm_ring_t)) < 0) {
        if (shm_fd >= 0) close(shm_fd);
        if (ev_fd >= 0) close(ev_fd);
        resp.hr = E_OUTOFMEMORY;
        send_response_with_fds(client_sock, &resp, -1, -1);
        close(client_sock);
        return;
    }

    lwasapi_shm_ring_t *ring = mmap(NULL, sizeof(lwasapi_shm_ring_t),
                                    PROT_READ | PROT_WRITE, MAP_SHARED, shm_fd, 0);
    if (ring == MAP_FAILED) {
        close(shm_fd);
        close(ev_fd);
        resp.hr = E_OUTOFMEMORY;
        send_response_with_fds(client_sock, &resp, -1, -1);
        close(client_sock);
        return;
    }

    /* memfd_create + ftruncate already supplies kernel-zeroed pages on first touch;
     * zero only the control/MIDI header so init avoids 300+ synchronous 4 KiB faults. */
    memset(ring, 0, offsetof(lwasapi_shm_ring_t, pcm_data));

    pthread_mutex_lock(&g_sessions_lock);

    /* Enforce Windows WASAPI Exclusive Mode semantics */
    if (g_exclusive_session_id >= 0) {
        resp.hr = AUDCLNT_E_DEVICE_IN_USE;
        int32_t holder = g_exclusive_session_id;
        pthread_mutex_unlock(&g_sessions_lock);
        munmap(ring, sizeof(lwasapi_shm_ring_t));
        close(shm_fd);
        close(ev_fd);
        lwasapi_debug_emit_vals(LWASAPI_DBG_WASAPI, LWASAPI_DBG_WARN, req->client_pid, 0,
                                holder, 0, resp.hr,
                                "EXCL_DENIED", req->client_name[0] ? req->client_name : "session",
                                "exclusive lock held by session #%d", holder);
        send_response_with_fds(client_sock, &resp, -1, -1);
        close(client_sock);
        return;
    }

    int slot = -1;
    for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
        if (!g_sessions[i].in_use) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        resp.hr = E_OUTOFMEMORY;
        pthread_mutex_unlock(&g_sessions_lock);
        munmap(ring, sizeof(lwasapi_shm_ring_t));
        close(shm_fd);
        close(ev_fd);
        send_response_with_fds(client_sock, &resp, -1, -1);
        close(client_sock);
        return;
    }

    ring->magic = 0x4C574153U;
    ring->session_id = g_next_session_id++;
    ring->client_pid = req->client_pid;
    snprintf(ring->client_name, sizeof(ring->client_name), "%s",
             req->client_name[0] ? req->client_name : "AudioSession");
    ring->share_mode = req->share_mode;
    ring->stream_flags = req->stream_flags;
    ring->is_capture = (req->stream_flags & AUDCLNT_STREAMFLAGS_CAPTURE) ? 1U : 0U;
    uint32_t req_rate = req->sample_rate;
    if (req_rate < 8000U || req_rate > 768000U) req_rate = g_hw.sample_rate;
    ring->sample_rate = (req->share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE)
                        ? g_hw.sample_rate
                        : (req->sample_rate ? req_rate : g_hw.sample_rate);
    ring->channels = (req->channels && req->channels <= LWASAPI_MAX_CHANNELS)
                     ? req->channels : g_hw.channels;
    if (ring->channels == 0 || ring->channels > LWASAPI_MAX_CHANNELS) ring->channels = 2;
    ring->format = LWASAPI_FMT_FLOAT32;
    ring->period_frames = g_hw.period_frames;

    uint32_t buf_frames = req->requested_buffer_frames;
    if (buf_frames < g_hw.period_frames * 2) buf_frames = g_hw.period_frames * 4;
    if (buf_frames > LWASAPI_RING_FRAMES) buf_frames = LWASAPI_RING_FRAMES;
    ring->buffer_frames = buf_frames;
    atomic_store(&ring->volume_milli, 1000);

    g_sessions[slot].in_use = true;
    g_sessions[slot].session_id = ring->session_id;
    g_sessions[slot].client_sock = client_sock;
    g_sessions[slot].shm_fd = shm_fd;
    g_sessions[slot].event_fd = ev_fd;
    g_sessions[slot].ring = ring;
    g_sessions[slot].resample_phase = 0.0;
    g_sessions[slot].stream_live = false;
    g_sessions[slot].underrun_pending = false;
    g_sessions[slot].last_audio_ns = 0;
    g_sessions[slot].sample_rate = ring->sample_rate;
    g_sessions[slot].channels = ring->channels;
    g_sessions[slot].is_capture = (ring->is_capture != 0U);
    g_sessions[slot].is_loopback = ((req->stream_flags & AUDCLNT_STREAMFLAGS_LOOPBACK) != 0U);

    if (req->share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE) {
        g_exclusive_session_id = (int32_t)ring->session_id;
    }

    uint32_t sid = ring->session_id;
    uint32_t s_pid = ring->client_pid;
    uint32_t s_rate = ring->sample_rate;
    uint32_t hw_rate = g_hw.sample_rate;
    uint32_t s_ch = ring->channels;
    uint32_t s_buf = ring->buffer_frames;
    uint32_t s_per = ring->period_frames;
    uint32_t s_cap = ring->is_capture;
    char s_name[64];
    snprintf(s_name, sizeof(s_name), "%s", ring->client_name);

    resp.hr = S_OK;
    resp.session_id = sid;
    resp.actual_sample_rate = s_rate;
    resp.actual_channels = s_ch;
    resp.actual_buffer_frames = s_buf;
    resp.actual_period_frames = s_per;

    pthread_mutex_unlock(&g_sessions_lock);

    send_response_with_fds(client_sock, &resp, shm_fd, ev_fd);

    if (req->share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE) {
        fprintf(stderr, "[LWASAPI-audiodg] Session #%u ('%s', PID %u) acquired WASAPI EXCLUSIVE hardware lock (%u Hz, period=%u)\n",
                sid, s_name, s_pid, s_rate, s_per);
    } else {
        fprintf(stderr, "[LWASAPI-audiodg] Session #%u ('%s', PID %u) initialized in WASAPI %s mode (%u Hz -> %u Hz, %u ch, buf=%u)\n",
                sid, s_name, s_pid, s_cap ? "CAPTURE" : "SHARED",
                s_rate, hw_rate, s_ch, s_buf);
    }

    lwasapi_debug_emit_vals(LWASAPI_DBG_WASAPI, LWASAPI_DBG_INFO, s_pid, sid,
                            (int32_t)s_rate, (int32_t)s_ch, S_OK,
                            "WASAPI_SESSION", s_name,
                            "mode=%s cap=%u rate=%u ch=%u buf=%u per=%u",
                            (req->share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE) ? "EXCLUSIVE" : "SHARED",
                            s_cap, s_rate, s_ch, s_buf, s_per);
}

static void fill_engine_status_impl(lwasapi_engine_status_t *st, bool include_pa_sessions) {
    memset(st, 0, sizeof(*st));

    /* Snapshot BlueZ / capture fields under state_lock FIRST (never hold g_hw_lock simultaneously) */
    pthread_mutex_lock(&g_bt.state_lock);
    bool bt_has_tr = g_bt.has_bluez_transport;
    bool bt_has_cap = g_bt.has_capture_hw;
    uint32_t bt_cap_rate = __atomic_load_n(&g_bt.cap_hw_rate, __ATOMIC_RELAXED);
    uint32_t bt_delay_tenths = g_bt.delay_tenths_ms;
    char bt_mac[32], bt_alias[64], bt_codec[16], cap_alias[64];
    snprintf(bt_mac, sizeof(bt_mac), "%s", g_bt.mac_address);
    snprintf(bt_alias, sizeof(bt_alias), "%s", g_bt.device_alias);
    snprintf(bt_codec, sizeof(bt_codec), "%s", g_bt.codec_name);
    snprintf(cap_alias, sizeof(cap_alias), "%s", g_bt.capture_device_alias);
    pthread_mutex_unlock(&g_bt.state_lock);

    pthread_mutex_lock(&g_hw_lock);
    uint32_t pref_cap = g_pref_cap_sample_rate;
    if (g_hw.is_bluetooth && bt_has_tr) {
        format_bt_hw_label(true, bt_alias, bt_mac, bt_codec,
                           g_hw.device_path, sizeof(g_hw.device_path),
                           g_active_playback_id, sizeof(g_active_playback_id));
    }
    snprintf(st->hw_device, sizeof(st->hw_device), "%s", g_hw.device_path);
    snprintf(st->active_playback_id, sizeof(st->active_playback_id), "%s", g_active_playback_id);
    snprintf(st->active_capture_id, sizeof(st->active_capture_id), "%s", g_active_capture_id);
    st->hw_sample_rate   = g_hw.sample_rate;
    st->hw_channels      = g_hw.channels;
    st->hw_period_frames = g_hw.period_frames;
    st->hw_buffer_frames = g_hw.buffer_frames;
    st->hw_format_bits   = g_hw.format_bits;
    st->bt_active        = (g_hw.is_bluetooth && bt_has_tr) ? 1U : 0U;
    pthread_mutex_unlock(&g_hw_lock);

    st->bt_delay_ms = bt_delay_tenths / 10U;
    snprintf(st->bt_alias, sizeof(st->bt_alias), "%s", bt_alias);
    snprintf(st->bt_codec, sizeof(st->bt_codec), "%s", bt_codec);
    st->pa_server_running = (g_pa_srv != NULL) ? 1U : 0U;

    snprintf(st->hw_capture_device, sizeof(st->hw_capture_device), "%s",
             cap_alias[0] ? cap_alias : "No Capture Device");
    snprintf(st->midi_device, sizeof(st->midi_device), "%s",
             g_midi.controller_name[0] ? g_midi.controller_name : "LWASAPI-MIDI-Hub");

    st->desktop_volume_pct     = __atomic_load_n(&g_bt.desktop_volume_pct, __ATOMIC_RELAXED);
    st->desktop_muted          = __atomic_load_n(&g_bt.desktop_muted, __ATOMIC_RELAXED) ? 1U : 0U;
    st->desktop_mic_volume_pct = __atomic_load_n(&g_bt.desktop_mic_volume_pct, __ATOMIC_RELAXED);
    st->desktop_mic_muted      = __atomic_load_n(&g_bt.desktop_mic_muted, __ATOMIC_RELAXED) ? 1U : 0U;
    st->total_cycles           = atomic_load(&g_total_cycles);
    st->hw_xruns               = atomic_load(&g_hw_xruns);
    st->midi_events_routed     = atomic_load(&g_midi.total_events);

    pthread_mutex_lock(&g_sessions_lock);
    st->realtime_priority  = g_rt_priority;
    st->exclusive_owner_id = g_exclusive_session_id;
    st->eq_cfg             = g_eq_cfg;
    st->param_eq           = g_param_eq;
    st->mic_apo            = g_mic_apo;
    st->mic_apo_peak_in    = g_mic_apo_peak_in;
    st->mic_apo_peak_out   = g_mic_apo_peak_out;
    st->mic_rnnoise_active = g_mic_rnnoise_active;
    memcpy(st->eq_band_levels_db, g_eq_band_levels_db, sizeof(g_eq_band_levels_db));
    memcpy(st->eq_dyn_delta_db, g_eq_dyn_delta_db, sizeof(g_eq_dyn_delta_db));
    st->output_peak_l = g_output_peak_l;
    st->output_peak_r = g_output_peak_r;

    uint32_t count = 0;
    for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
        if (g_sessions[i].in_use && g_sessions[i].ring) {
            lwasapi_shm_ring_t *r = g_sessions[i].ring;
            lwasapi_session_info_t *info = &st->sessions[count++];
            info->session_id   = r->session_id;
            info->client_pid   = r->client_pid;
            info->is_capture   = r->is_capture;
            snprintf(info->client_name, sizeof(info->client_name), "%s", r->client_name);
            info->share_mode   = r->share_mode;
            info->sample_rate  = r->sample_rate;
            info->channels     = r->channels;
            info->active       = atomic_load(&r->active);
            info->volume_milli = atomic_load(&r->volume_milli);
            uint64_t w  = atomic_load(r->is_capture ? &r->cap_write_frame : &r->write_frame);
            uint64_t rd = atomic_load(r->is_capture ? &r->cap_read_frame : &r->read_frame);
            info->buffered_frames = (w >= rd) ? (w - rd) : 0;
            info->xruns           = atomic_load(&r->xruns);
            info->muted           = atomic_load(&r->muted);
            info->routing_flags   = 0;
            snprintf(info->target_device, sizeof(info->target_device), "%s",
                     r->is_capture ? st->active_capture_id : st->active_playback_id);
        }
    }
    st->num_sessions = count;
    st->hw_analog_cfg = g_hw_analog_cfg;
    pthread_mutex_unlock(&g_sessions_lock);

    if (g_hw_analog) {
        lwasapi_hw_analog_get_info(g_hw_analog, &st->hw_analog_info);
    }
    st->hw_analog_info.reserved = (bt_has_cap && bt_cap_rate >= 8000U)
                                  ? bt_cap_rate
                                  : resolve_cap_sample_rate(st->hw_sample_rate, pref_cap);

    if (g_bt.initialized) {
        lwasapi_bluez_get_codec_config(&g_bt, &st->bt_codec_cfg);
    } else {
        st->bt_codec_cfg = g_saved_bt_cfg;
    }

    /* Never re-enter lwasapi_pa_server when called from inside the PA server lock */
    if (include_pa_sessions && g_pa_srv) {
        lwasapi_pa_server_fill_sessions(g_pa_srv, st, &st->pa_clients);
    }
}

static void fill_engine_status(lwasapi_engine_status_t *st) {
    fill_engine_status_impl(st, true);
}

static int switch_engine_devices_impl(const char *pb_id, const char *cap_id, bool notify_pa) {
    pthread_mutex_lock(&g_reconf_lock);

    char target_pb[64] = {0};
    char target_cap[64] = {0};
    pthread_mutex_lock(&g_hw_lock);
    uint32_t rate    = g_pref_sample_rate ? g_pref_sample_rate : (g_hw.sample_rate ? g_hw.sample_rate : 48000);
    uint32_t period  = g_pref_period_frames ? g_pref_period_frames : (g_hw.period_frames ? g_hw.period_frames : 256);
    uint32_t bits    = g_pref_format_bits ? g_pref_format_bits : (g_hw.format_bits ? g_hw.format_bits : 32);
    uint32_t hw_rate = g_hw.sample_rate ? g_hw.sample_rate : rate;
    uint32_t pref_cap = g_pref_cap_sample_rate;
    if (pb_id && pb_id[0]) {
        snprintf(g_target_playback_id, sizeof(g_target_playback_id), "%s", pb_id);
        snprintf(target_pb, sizeof(target_pb), "%s", g_target_playback_id);
    }
    if (cap_id && cap_id[0]) {
        snprintf(g_target_capture_id, sizeof(g_target_capture_id), "%s", cap_id);
        snprintf(target_cap, sizeof(target_cap), "%s", g_target_capture_id);
    }
    pthread_mutex_unlock(&g_hw_lock);

    if (target_pb[0]) {
        if (g_bt.initialized) {
            lwasapi_bluez_set_preferred_target(&g_bt, target_pb);
        }
        apply_playback_endpoint(target_pb, rate, period, bits, false, false, NULL);
        pthread_mutex_lock(&g_hw_lock);
        if (g_hw.sample_rate) hw_rate = g_hw.sample_rate;
        pthread_mutex_unlock(&g_hw_lock);
    }
    if (target_cap[0]) {
        lwasapi_bluez_open_capture(&g_bt, target_cap, resolve_cap_sample_rate(hw_rate, pref_cap));
        char cap_id_now[64];
        pthread_mutex_lock(&g_bt.state_lock);
        snprintf(cap_id_now, sizeof(cap_id_now), "%s", g_bt.cap_device_id);
        pthread_mutex_unlock(&g_bt.state_lock);

        pthread_mutex_lock(&g_hw_lock);
        if (cap_id_now[0]) {
            snprintf(g_active_capture_id, sizeof(g_active_capture_id), "%s", cap_id_now);
        } else {
            snprintf(g_active_capture_id, sizeof(g_active_capture_id), "none");
        }
        pthread_mutex_unlock(&g_hw_lock);
    }

    hw_analog_probe_and_sync(true);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    if (notify_pa && g_pa_srv) {
        lwasapi_pa_server_notify_change(g_pa_srv);
    }
    return 0;
}

static void enumerate_devices_safe(lwasapi_device_list_t *out_list) {
    char pb_id[64], cap_id[64];
    pthread_mutex_lock(&g_hw_lock);
    snprintf(pb_id, sizeof(pb_id), "%s", g_active_playback_id);
    snprintf(cap_id, sizeof(cap_id), "%s", g_active_capture_id);
    pthread_mutex_unlock(&g_hw_lock);
    lwasapi_bluez_enumerate_devices(&g_bt, pb_id, cap_id, out_list);
}

static void pa_cb_get_status(void *ctx, lwasapi_engine_status_t *out_st) {
    (void)ctx;
    fill_engine_status_impl(out_st, false);
}

static void pa_cb_get_devices(void *ctx, lwasapi_device_list_t *out_list) {
    (void)ctx;
    enumerate_devices_safe(out_list);
}

static int pa_cb_set_devices(void *ctx, const char *pb_id, const char *cap_id) {
    (void)ctx;
    return switch_engine_devices_impl(pb_id, cap_id, false);
}

static int pa_cb_set_master_volume(void *ctx, int is_capture, int vol_pct, int mute_flag) {
    (void)ctx;
    /* PA server already holds srv->lock and broadcasts subscribe events right after this returns */
    int rc = is_capture ? lwasapi_bluez_set_mic_volume(&g_bt, vol_pct, mute_flag)
                        : lwasapi_bluez_set_volume(&g_bt, vol_pct, mute_flag);
    hw_analog_probe_and_sync(false);
    if (rc == 0) mark_salsa_state_dirty();
    return rc;
}

static int pa_cb_set_mic_gain_ppm(void *ctx, uint32_t gain_ppm, int vol_pct, int mute_flag) {
    (void)ctx;
    int rc = lwasapi_bluez_set_mic_gain_ppm(&g_bt, gain_ppm, vol_pct, mute_flag);
    hw_analog_probe_and_sync(false);
    if (rc == 0) mark_salsa_state_dirty();
    return rc;
}

static void handle_get_status(int client_sock) {
    lwasapi_engine_status_t st;
    fill_engine_status(&st);
    send(client_sock, &st, sizeof(st), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_list_devices(int client_sock) {
    lwasapi_device_list_t dlist;
    enumerate_devices_safe(&dlist);
    send(client_sock, &dlist, sizeof(dlist), MSG_NOSIGNAL);
    close(client_sock);
}

static void dbg_fill_snapshot_cb(void *ctx, lwasapi_dbg_snapshot_t *snap) {
    (void)ctx;
    if (!snap) return;

    lwasapi_dbg_rt_stats_t saved_rt = snap->rt;
    memset(snap, 0, sizeof(*snap));
    snap->rt = saved_rt;

    fill_engine_status(&snap->engine_st);

    pthread_mutex_lock(&g_hw_lock);
    snprintf(snap->target_playback_id, sizeof(snap->target_playback_id), "%s", g_target_playback_id);
    snprintf(snap->target_capture_id, sizeof(snap->target_capture_id), "%s", g_target_capture_id);
    uint32_t pref_cap = g_pref_cap_sample_rate;
    uint32_t pref_pb_rate = g_pref_sample_rate;
    bool pb_is_hw = (!g_hw.is_virtual && !g_hw.is_bluetooth);
    char active_pb_copy[64];
    snprintf(active_pb_copy, sizeof(active_pb_copy), "%s", g_active_playback_id);
    pthread_mutex_unlock(&g_hw_lock);

    /* Never hold g_hw_lock and g_bt.state_lock simultaneously */
    pthread_mutex_lock(&g_bt.state_lock);
    snap->bt_initialized          = g_bt.initialized ? 1U : 0U;
    snap->bt_endpoints_registered = g_bt.endpoints_registered ? 1U : 0U;
    snap->bt_direct_socket        = g_bt.direct_socket_mode ? 1U : 0U;
    snap->bt_has_transport        = g_bt.has_bluez_transport ? 1U : 0U;
    snap->bt_tx_paused            = atomic_load_explicit(&g_bt.tx_paused, memory_order_relaxed) ? 1U : 0U;
    snprintf(snap->bt_device_path, sizeof(snap->bt_device_path), "%s", g_bt.device_path);
    snprintf(snap->bt_transport_path, sizeof(snap->bt_transport_path), "%s", g_bt.transport_path);
    snprintf(snap->bt_endpoint_path, sizeof(snap->bt_endpoint_path), "%s", g_bt.endpoint_path);
    snap->bt_sample_rate          = g_bt.bt_sample_rate;
    snap->bt_channels             = g_bt.bt_channels;
    snap->bt_mtu_r                = g_bt.read_mtu;
    snap->bt_mtu_w                = g_bt.write_mtu;
    snap->bt_avrcp_vol            = g_bt.avrcp_volume;
    uint64_t tx_wr                = atomic_load_explicit(&g_bt.tx_write_frame, memory_order_relaxed);
    uint64_t tx_rd                = atomic_load_explicit(&g_bt.tx_read_frame, memory_order_relaxed);
    snap->bt_tx_ring_fill         = (tx_wr >= tx_rd) ? (tx_wr - tx_rd) : 0ULL;
    snap->bt_tx_ring_cap          = LWASAPI_BT_RING_FRAMES;
    snap->bt_tx_packets           = atomic_load_explicit(&g_bt.tx_packets_sent, memory_order_relaxed);
    snap->bt_tx_underruns         = atomic_load_explicit(&g_bt.tx_underruns, memory_order_relaxed);
    snap->bt_tx_errors            = atomic_load_explicit(&g_bt.tx_errors, memory_order_relaxed);

    snap->cap_active              = g_bt.has_capture_hw ? 1U : 0U;
    snprintf(snap->cap_device_id, sizeof(snap->cap_device_id), "%s", g_bt.cap_device_id);
    snprintf(snap->cap_pcm_path, sizeof(snap->cap_pcm_path), "%s", g_bt.cap_pcm_path);
    snprintf(snap->cap_alias, sizeof(snap->cap_alias), "%s", g_bt.capture_device_alias);
    snap->cap_hw_rate             = g_bt.cap_hw_rate;
    snap->cap_hw_channels         = g_bt.cap_hw_channels;
    snap->cap_hw_format           = g_bt.cap_hw_format;
    uint64_t cap_wr               = atomic_load_explicit(&g_bt.cap_hw_write_frame, memory_order_relaxed);
    uint64_t cap_rd               = atomic_load_explicit(&g_cap_read_cursor, memory_order_relaxed);
    snap->cap_ring_fill           = (cap_wr >= cap_rd) ? (cap_wr - cap_rd) : 0ULL;
    snap->cap_pref_rate           = pref_cap;
    snap->cap_measured_rate       = __atomic_load_n(&g_bt.cap_measured_rate, __ATOMIC_RELAXED);
    snap->cap_kick_count          = __atomic_load_n(&g_bt.cap_kick_count, __ATOMIC_RELAXED);
    snap->cap_is_usb              = g_bt.cap_is_usb ? 1U : 0U;
    {
        unsigned pb_c = 0, cap_c = 0;
        snap->cap_duplex_locked = (g_bt.has_capture_hw && pb_is_hw && pref_pb_rate != 0U &&
                                   sscanf(active_pb_copy, "hw:%u,", &pb_c) == 1 &&
                                   sscanf(g_bt.cap_device_id, "hw:%u,", &cap_c) == 1 &&
                                   pb_c == cap_c &&
                                   snap->engine_st.hw_sample_rate != pref_pb_rate) ? 1U : 0U;
    }
    pthread_mutex_unlock(&g_bt.state_lock);

    snap->midi_initialized        = g_midi.initialized ? 1U : 0U;
    snap->midi_rawmidi_open       = (g_midi.rawmidi_fd >= 0) ? 1U : 0U;
    snprintf(snap->midi_controller, sizeof(snap->midi_controller), "%s", g_midi.controller_name);
    snprintf(snap->midi_dev_path, sizeof(snap->midi_dev_path), "%s", g_midi.dev_path);
    snap->midi_seq_client         = g_midi.seq_client_id;
    snap->midi_seq_in_port        = g_midi.seq_in_port;
    snap->midi_events_routed      = atomic_load_explicit(&g_midi.total_events, memory_order_relaxed);

    pthread_mutex_lock(&g_sessions_lock);
    uint32_t wcnt = 0;
    for (int i = 0; i < LWASAPI_MAX_SESSIONS && wcnt < LWASAPI_MAX_SESSIONS; i++) {
        if (g_sessions[i].in_use && g_sessions[i].ring) {
            lwasapi_shm_ring_t *r = g_sessions[i].ring;
            lwasapi_dbg_wasapi_session_t *ws = &snap->wasapi[wcnt++];
            ws->session_id    = r->session_id;
            ws->client_pid    = r->client_pid;
            snprintf(ws->client_name, sizeof(ws->client_name), "%s", r->client_name);
            ws->share_mode    = r->share_mode;
            ws->stream_flags  = r->stream_flags;
            ws->is_capture    = r->is_capture;
            ws->sample_rate   = r->sample_rate;
            ws->channels      = r->channels;
            ws->active        = atomic_load_explicit(&r->active, memory_order_relaxed);
            ws->muted         = atomic_load_explicit(&r->muted, memory_order_relaxed);
            ws->volume_milli  = atomic_load_explicit(&r->volume_milli, memory_order_relaxed);
            ws->buffer_frames = r->buffer_frames;
            ws->period_frames = r->period_frames;
            uint64_t w  = atomic_load_explicit(r->is_capture ? &r->cap_write_frame : &r->write_frame, memory_order_relaxed);
            uint64_t rd = atomic_load_explicit(r->is_capture ? &r->cap_read_frame : &r->read_frame, memory_order_relaxed);
            ws->buffered_frames = (w >= rd) ? (w - rd) : 0ULL;
            ws->xruns           = atomic_load_explicit(&r->xruns, memory_order_relaxed);
        }
    }
    snap->num_wasapi = wcnt;
    pthread_mutex_unlock(&g_sessions_lock);

    if (g_pa_srv) {
        lwasapi_pa_server_fill_debug(g_pa_srv, &snap->pa);
    }
    if (g_pw_brk) {
        lwasapi_pw_broker_fill_debug(g_pw_brk, &snap->pw);
    }
}

static int dbg_handle_cmd_cb(void *ctx, const char *cmd, char *reply, size_t reply_len) {
    (void)ctx;
    if (!cmd) return -EINVAL;
    if (strcmp(cmd, "RESET_NET_CLIENTS") == 0) {
        int pa_n = g_pa_srv ? lwasapi_pa_server_reset_clients(g_pa_srv) : 0;
        int pw_n = g_pw_brk ? lwasapi_pw_broker_reset_clients(g_pw_brk) : 0;
        salsa_failsafe_record_trip(SALSA_FS_MANUAL_RESET,
                                   "Manual PA/PW network client flush via Debugger");
        lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, (uint32_t)getpid(), 0,
                           "NET_FAILSAFE_RESET", "PA/PW",
                           "Flushed %d PA + %d PW network clients (audio engine uninterrupted)",
                           pa_n, pw_n);
        snprintf(reply, reply_len,
                 "Flushed %d PA + %d PW network clients (audio engine uninterrupted)",
                 pa_n, pw_n);
        return 0;
    }
    if (!g_pw_brk) {
        snprintf(reply, reply_len, "PW video broker not initialized");
        return -ENODEV;
    }
    if (strncmp(cmd, "PW_PREVIEW ", 11) == 0) {
        unsigned long en = 0, ttl = 15000, mw = 0, mh = 0, fps = 0;
        int n = sscanf(cmd + 11, "%lu %lu %lu %lu %lu", &en, &ttl, &mw, &mh, &fps);
        if (n >= 1) {
            lwasapi_pw_broker_set_preview_ex(
                g_pw_brk, en != 0,
                (uint32_t)(ttl > 0 ? ttl : 15000),
                (n >= 4) ? (uint32_t)mw : 0u,
                (n >= 4) ? (uint32_t)mh : 0u,
                (n >= 5) ? (uint32_t)fps : 0u);
            snprintf(reply, reply_len, "preview=%s ttl=%lums target=%lux%lu@%lufps",
                     en ? "on" : "off", ttl, mw, mh, fps);
            return 0;
        }
        snprintf(reply, reply_len, "invalid PW_PREVIEW syntax");
        return -EINVAL;
    } else if (strncmp(cmd, "PW_CAST_LIMITS ", 15) == 0) {
        unsigned long mw = 0, mh = 0, fps = 0;
        if (sscanf(cmd + 15, "%lu %lu %lu", &mw, &mh, &fps) >= 3) {
            lwasapi_pw_broker_set_cast_limits(g_pw_brk, (uint32_t)mw, (uint32_t)mh, (uint32_t)fps);
            snprintf(reply, reply_len, "cast_limits=%lux%lu@%lufps", mw, mh, fps);
            return 0;
        }
        snprintf(reply, reply_len, "invalid PW_CAST_LIMITS syntax");
        return -EINVAL;
    } else if (strcmp(cmd, "PW_SELF_TEST") == 0) {
        return lwasapi_pw_broker_self_test(g_pw_brk, reply, reply_len);
    } else if (strcmp(cmd, "PW_SYNC_MODE") == 0) {
        int rc = lwasapi_pw_broker_sync_mode(g_pw_brk);
        snprintf(reply, reply_len, "sync_mode rc=%d", rc);
        return rc;
    }
    snprintf(reply, reply_len, "Unknown debug command: %s", cmd);
    return -ENOSYS;
}

static void handle_set_devices(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));

    const char *pb = req->target_device[0] ? req->target_device : NULL;
    const char *cap = req->target_capture_device[0] ? req->target_capture_device : NULL;
    if (!pb && !cap) {
        resp.hr = E_INVALIDARG;
    } else {
        switch_engine_devices_impl(pb, cap, true);
        pthread_mutex_lock(&g_hw_lock);
        resp.hr                   = S_OK;
        resp.actual_sample_rate   = g_hw.sample_rate;
        resp.actual_channels      = g_hw.channels;
        resp.actual_period_frames = g_hw.period_frames;
        resp.actual_buffer_frames = g_hw.buffer_frames;
        pthread_mutex_unlock(&g_hw_lock);
    }

    lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE,
                            (resp.hr == S_OK) ? LWASAPI_DBG_INFO : LWASAPI_DBG_WARN,
                            req->client_pid, 0,
                            (int32_t)resp.actual_sample_rate,
                            (int32_t)resp.actual_period_frames,
                            resp.hr,
                            "DEV_SWITCH", pb ? pb : (cap ? cap : "none"),
                            "pb=%s cap=%s -> %uHz/%u",
                            pb ? pb : "-", cap ? cap : "-",
                            resp.actual_sample_rate, resp.actual_period_frames);

    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_set_volume(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    resp.hr = E_INVALIDARG;

    if (req->target_session_id == 0xFFFFFFFFU) {
        /* Master Output Volume / Mute synchronization (+ BlueZ AVRCP) */
        int vol_pct   = (req->volume_milli <= 1500) ? (int)(req->volume_milli / 10U) : -1;
        int mute_flag = (int)req->share_mode - 1;
        lwasapi_bluez_set_volume(&g_bt, vol_pct, mute_flag);
        hw_analog_probe_and_sync(false);
        if (g_pa_srv) lwasapi_pa_server_notify_volume(g_pa_srv, false);
        mark_salsa_state_dirty();
        resp.hr = S_OK;
        lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, req->client_pid, 0,
                                vol_pct, mute_flag, resp.hr,
                                "VOL_CHG", "master_out", "vol=%d%% mute=%d", vol_pct, mute_flag);
    } else if (req->target_session_id == 0xFFFFFFFEU) {
        /* Microphone Input Volume / Mute synchronization */
        int vol_pct   = (req->volume_milli <= 1500) ? (int)(req->volume_milli / 10U) : -1;
        int mute_flag = (int)req->share_mode - 1;
        lwasapi_bluez_set_mic_volume(&g_bt, vol_pct, mute_flag);
        hw_analog_probe_and_sync(false);
        if (g_pa_srv) lwasapi_pa_server_notify_volume(g_pa_srv, true);
        mark_salsa_state_dirty();
        resp.hr = S_OK;
        lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, req->client_pid, 0,
                                vol_pct, mute_flag, resp.hr,
                                "VOL_CHG", "master_mic", "vol=%d%% mute=%d", vol_pct, mute_flag);
    } else {
        bool found = false;
        pthread_mutex_lock(&g_sessions_lock);
        for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
            if (g_sessions[i].in_use && g_sessions[i].session_id == req->target_session_id && g_sessions[i].ring) {
                atomic_store(&g_sessions[i].ring->volume_milli, req->volume_milli);
                resp.hr = S_OK;
                found = true;
                break;
            }
        }
        pthread_mutex_unlock(&g_sessions_lock);

        if (!found && g_pa_srv) {
            if (lwasapi_pa_server_set_stream_volume(g_pa_srv, req->target_session_id, req->volume_milli / 10U) == 0) {
                resp.hr = S_OK;
            }
        }
        lwasapi_debug_emit_vals(LWASAPI_DBG_WASAPI,
                                (resp.hr == S_OK) ? LWASAPI_DBG_INFO : LWASAPI_DBG_WARN,
                                req->client_pid, req->target_session_id,
                                (int32_t)req->volume_milli, found ? 1 : 0, resp.hr,
                                "VOL_CHG", "stream", "milli=%u wasapi=%d", req->volume_milli, found ? 1 : 0);
    }

    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_set_session_control(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    resp.hr = E_INVALIDARG;

    char tgt_dev[64];
    memcpy(tgt_dev, req->target_device, sizeof(tgt_dev));
    tgt_dev[sizeof(tgt_dev) - 1] = '\0';

    if (req->target_session_id < 10000U) {
        pthread_mutex_lock(&g_sessions_lock);
        for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
            if (g_sessions[i].in_use && g_sessions[i].session_id == req->target_session_id && g_sessions[i].ring) {
                if (req->volume_milli <= 1500U) {
                    atomic_store(&g_sessions[i].ring->volume_milli, req->volume_milli);
                }
                if (req->share_mode == 1U) {
                    atomic_store(&g_sessions[i].ring->muted, 0);
                } else if (req->share_mode == 2U) {
                    atomic_store(&g_sessions[i].ring->muted, 1);
                } else if (req->share_mode == 3U) {
                    atomic_store(&g_sessions[i].ring->muted,
                                 atomic_load(&g_sessions[i].ring->muted) ? 0 : 1);
                }
                resp.hr = S_OK;
                break;
            }
        }
        pthread_mutex_unlock(&g_sessions_lock);
    } else if (g_pa_srv) {
        int32_t vol_pct = (req->volume_milli <= 1500U) ? (int32_t)(req->volume_milli / 10U) : -1;
        int32_t mute_mode = (req->share_mode >= 1U && req->share_mode <= 3U)
                          ? (int32_t)(req->share_mode - 1U) : -1;
        if (lwasapi_pa_server_control_stream(g_pa_srv, req->target_session_id,
                                             vol_pct, mute_mode,
                                             tgt_dev[0] ? tgt_dev : NULL) == 0) {
            resp.hr = S_OK;
        }
    }

    lwasapi_debug_emit_vals(LWASAPI_DBG_WASAPI,
                            (resp.hr == S_OK) ? LWASAPI_DBG_INFO : LWASAPI_DBG_WARN,
                            req->client_pid, req->target_session_id,
                            (int32_t)req->volume_milli, (int32_t)req->share_mode, resp.hr,
                            "SESS_CTL", tgt_dev[0] ? tgt_dev : "-",
                            "sid=%u vol_m=%u mute_op=%u dev=%s",
                            req->target_session_id, req->volume_milli, req->share_mode,
                            tgt_dev[0] ? tgt_dev : "-");

    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_inject_midi(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));

    lwasapi_midi_event_t ev = req->midi_ev;
    if (ev.size == 0 || ev.size > 3) {
        resp.hr = E_INVALIDARG;
    } else {
        if (ev.timestamp_ns == 0) {
            ev.timestamp_ns = monotonic_ns();
        }
        pthread_mutex_lock(&g_sessions_lock);
        for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
            if (g_sessions[i].in_use && g_sessions[i].ring) {
                lwasapi_midi_push_to_ring(g_sessions[i].ring, &ev);
            }
        }
        if (lwasapi_midi_is_key_press(&ev)) {
            atomic_fetch_add(&g_midi.total_events, 1);
        }
        pthread_mutex_unlock(&g_sessions_lock);
        resp.hr = S_OK;
    }

    lwasapi_debug_emit_vals(LWASAPI_DBG_MIDI,
                            (resp.hr == S_OK) ? LWASAPI_DBG_INFO : LWASAPI_DBG_WARN,
                            req->client_pid, 0,
                            (int32_t)ev.data[0], (int32_t)ev.data[1], resp.hr,
                            "MIDI_INJECT", "ipc",
                            "status=0x%02X d1=%u d2=%u size=%u",
                            (unsigned)ev.data[0], (unsigned)ev.data[1], (unsigned)ev.data[2], (unsigned)ev.size);

    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static int reconfigure_engine_rate_impl(uint32_t new_rate, uint32_t new_period, uint32_t new_bits,
                                        bool update_pref, lwasapi_ipc_resp_t *out_resp,
                                        uint32_t *out_actual_bits) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));

    pthread_mutex_lock(&g_reconf_lock);
    pthread_mutex_lock(&g_hw_lock);
    uint32_t cur_rate   = g_hw.sample_rate;
    uint32_t cur_period = g_hw.period_frames;
    uint32_t cur_bits   = g_hw.format_bits;
    if (!new_rate)   new_rate   = cur_rate;
    if (!new_period) new_period = cur_period;
    if (new_bits != 16U && new_bits != 24U && new_bits != 32U) new_bits = cur_bits;
    bool was_virtual    = (g_hw.is_virtual && strcmp(g_active_playback_id, "virtual") == 0);
    bool is_hw_kernel   = (!g_hw.is_virtual && !g_hw.is_bluetooth);
    char target_pb[64], target_cap[64];
    uint32_t pref_cap = g_pref_cap_sample_rate;
    snprintf(target_pb, sizeof(target_pb), "%s", g_target_playback_id);
    snprintf(target_cap, sizeof(target_cap), "%s", g_target_capture_id);

    if (new_rate < 44100U || new_rate > 192000U || new_period < 64U || new_period > LWASAPI_MAX_PERIOD) {
        pthread_mutex_unlock(&g_hw_lock);
        pthread_mutex_unlock(&g_reconf_lock);
        return -1;
    }

    if (update_pref) {
        g_pref_sample_rate   = new_rate;
        g_pref_period_frames = new_period;
        g_pref_format_bits   = new_bits;
    }
    bool rate_changed   = (new_rate != cur_rate);
    bool period_changed = (new_period != cur_period);
    bool bits_changed   = (new_bits != cur_bits);
    pthread_mutex_unlock(&g_hw_lock);

    if (rate_changed || period_changed || (bits_changed && is_hw_kernel)) {
        apply_playback_endpoint(target_pb, new_rate, new_period, new_bits, was_virtual, false, NULL);
    }

    pthread_mutex_lock(&g_hw_lock);
    if ((g_hw.is_virtual || g_hw.is_bluetooth) && (new_bits == 16U || new_bits == 24U || new_bits == 32U)) {
        g_hw.format_bits = new_bits;
    }
    resp.actual_sample_rate   = g_hw.sample_rate;
    resp.actual_channels      = g_hw.channels;
    resp.actual_period_frames = g_hw.period_frames;
    resp.actual_buffer_frames = g_hw.buffer_frames;
    if (out_actual_bits) *out_actual_bits = g_hw.format_bits;
    pthread_mutex_unlock(&g_hw_lock);

    if (rate_changed && g_bt.initialized) {
        lwasapi_bluez_reconfigure_rate(&g_bt, resp.actual_sample_rate);
        bool cap_already_open = false;
        char cap_id_now[64];
        pthread_mutex_lock(&g_bt.state_lock);
        cap_already_open = (g_bt.has_capture_hw && g_bt.cap_pcm_fd >= 0);
        snprintf(cap_id_now, sizeof(cap_id_now), "%s", g_bt.cap_device_id);
        pthread_mutex_unlock(&g_bt.state_lock);
        if (!cap_already_open) {
            lwasapi_bluez_open_capture(&g_bt, target_cap,
                                       resolve_cap_sample_rate(resp.actual_sample_rate, pref_cap));
            pthread_mutex_lock(&g_bt.state_lock);
            snprintf(cap_id_now, sizeof(cap_id_now), "%s", g_bt.cap_device_id);
            pthread_mutex_unlock(&g_bt.state_lock);
        }
        if (cap_id_now[0]) {
            pthread_mutex_lock(&g_hw_lock);
            snprintf(g_active_capture_id, sizeof(g_active_capture_id), "%s", cap_id_now);
            pthread_mutex_unlock(&g_hw_lock);
        }
    }

    pthread_mutex_lock(&g_sessions_lock);
    for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
        if (g_sessions[i].in_use && g_sessions[i].ring) {
            g_sessions[i].ring->period_frames = resp.actual_period_frames;
        }
    }
    pthread_mutex_unlock(&g_sessions_lock);

    if (rate_changed || period_changed || bits_changed) {
        hw_analog_probe_and_sync(true);
    }
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    if (g_pa_srv) {
        lwasapi_pa_server_notify_change(g_pa_srv);
    }
    if (out_resp) *out_resp = resp;
    return 0;
}

static int reconfigure_capture_rate_impl(uint32_t new_cap_rate, lwasapi_ipc_resp_t *out_resp) {
    if (new_cap_rate != 0U && (new_cap_rate < 44100U || new_cap_rate > 192000U)) {
        return -1;
    }
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));

    pthread_mutex_lock(&g_reconf_lock);
    pthread_mutex_lock(&g_hw_lock);
    uint32_t eng_rate   = g_hw.sample_rate ? g_hw.sample_rate : 48000U;
    uint32_t cur_period = g_hw.period_frames;
    uint32_t cur_buf    = g_hw.buffer_frames;
    char target_cap[64];
    snprintf(target_cap, sizeof(target_cap), "%s", g_target_capture_id);
    g_pref_cap_sample_rate = new_cap_rate;
    pthread_mutex_unlock(&g_hw_lock);

    uint32_t eff_cap = resolve_cap_sample_rate(eng_rate, new_cap_rate);
    bool has_cap_now = false;
    uint32_t cap_rate_now = eff_cap;
    uint32_t cap_ch_now = 2U;
    if (g_bt.initialized) {
        lwasapi_bluez_open_capture(&g_bt, target_cap[0] ? target_cap : "auto", eff_cap);
        char cap_id_now[64] = {0};
        pthread_mutex_lock(&g_bt.state_lock);
        has_cap_now  = g_bt.has_capture_hw;
        cap_rate_now = __atomic_load_n(&g_bt.cap_hw_rate, __ATOMIC_RELAXED);
        cap_ch_now   = g_bt.cap_hw_channels ? g_bt.cap_hw_channels : 2U;
        snprintf(cap_id_now, sizeof(cap_id_now), "%s", g_bt.cap_device_id);
        pthread_mutex_unlock(&g_bt.state_lock);

        pthread_mutex_lock(&g_hw_lock);
        snprintf(g_active_capture_id, sizeof(g_active_capture_id), "%s",
                 cap_id_now[0] ? cap_id_now : "none");
        pthread_mutex_unlock(&g_hw_lock);
    }

    hw_analog_probe_and_sync(true);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    if (g_pa_srv) {
        lwasapi_pa_server_notify_change(g_pa_srv);
    }

    resp.hr                   = S_OK;
    resp.actual_sample_rate   = (has_cap_now && cap_rate_now) ? cap_rate_now : eff_cap;
    resp.actual_channels      = cap_ch_now;
    resp.actual_period_frames = cur_period;
    resp.actual_buffer_frames = cur_buf;
    if (out_resp) *out_resp = resp;
    return 0;
}

static void handle_set_engine_config(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));

    pthread_mutex_lock(&g_hw_lock);
    uint32_t cur_rate   = g_hw.sample_rate;
    uint32_t cur_period = g_hw.period_frames;
    uint32_t cur_bits   = g_hw.format_bits;
    char active_pb_copy[64];
    snprintf(active_pb_copy, sizeof(active_pb_copy), "%s", g_active_playback_id);
    pthread_mutex_unlock(&g_hw_lock);

    if (req->stream_flags == LWASAPI_CFG_FLAG_CAPTURE_RATE) {
        if (reconfigure_capture_rate_impl(req->sample_rate, &resp) != 0) {
            resp.hr = E_INVALIDARG;
        } else {
            fprintf(stderr, "[LWASAPI-Salsa] Capture/Mic Clock reconfigured -> %u Hz (%s)\n",
                    resp.actual_sample_rate,
                    req->sample_rate == 0U ? "Auto/Follow Master" : "Fixed Independent Mic Clock");
            lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, req->client_pid, 0,
                                    (int32_t)resp.actual_sample_rate, (int32_t)resp.actual_period_frames, resp.hr,
                                    "CFG_CAP_RATE", active_pb_copy,
                                    "CAP %uHz (pref=%u)", resp.actual_sample_rate, req->sample_rate);
        }
        send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
        close(client_sock);
        return;
    }

    uint32_t req_rate = req->sample_rate ? req->sample_rate : cur_rate;
    if (req->sample_rate != 0U && (req->sample_rate < 44100U || req->sample_rate > 192000U)) {
        resp.hr = E_INVALIDARG;
        send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
        close(client_sock);
        return;
    }

    uint32_t new_period = req->requested_period_frames ? req->requested_period_frames : cur_period;
    uint32_t new_bits   = req->format ? (uint32_t)req->format : cur_bits;
    uint32_t actual_bits = cur_bits;

    if (reconfigure_engine_rate_impl(req_rate, new_period, new_bits, true, &resp, &actual_bits) != 0) {
        resp.hr = E_INVALIDARG;
        lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_WARN, req->client_pid, 0,
                                (int32_t)req_rate, (int32_t)new_period, resp.hr,
                                "CFG_RECONF", "invalid", "req=%uHz period=%u", req_rate, new_period);
        send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
        close(client_sock);
        return;
    }

    resp.hr = S_OK;
    fprintf(stderr, "[LWASAPI-Salsa] Engine reconfigured -> %u Hz | %u-bit | Quantum %u frames (%.2f ms)\n",
            resp.actual_sample_rate, actual_bits, resp.actual_period_frames,
            (1000.0 * resp.actual_period_frames) / resp.actual_sample_rate);

    lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, req->client_pid, 0,
                            (int32_t)resp.actual_sample_rate, (int32_t)resp.actual_period_frames, resp.hr,
                            "CFG_RECONF", active_pb_copy,
                            "FIXED %uHz %ubit quantum=%u",
                            resp.actual_sample_rate, actual_bits, resp.actual_period_frames);

    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_set_eq(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));

    pthread_mutex_lock(&g_sessions_lock);
    g_eq_cfg = req->eq_cfg;
    for (int b = 0; b < LWASAPI_EQ_BANDS; b++) {
        if (g_eq_cfg.band_q[b] < 0.2f || g_eq_cfg.band_q[b] > 10.0f) {
            g_eq_cfg.band_q[b] = 1.414f;
        }
    }
    lwasapi_eq_config_t eq_copy = g_eq_cfg;
    pthread_mutex_unlock(&g_sessions_lock);

    pthread_mutex_lock(&g_reconf_lock);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, req->client_pid, 0,
                            (int32_t)eq_copy.enabled, (int32_t)eq_copy.dynamic_mode, S_OK,
                            "EQ_SET", eq_copy.preset_name[0] ? eq_copy.preset_name : "custom",
                            "en=%u dyn=%u preamp=%.1fdB lim=%u",
                            eq_copy.enabled, eq_copy.dynamic_mode,
                            (double)eq_copy.preamp_db, eq_copy.limiter_enabled);

    resp.hr = S_OK;
    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_set_param_eq(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    lwasapi_param_eq_payload_t pay;
    if (req->requested_buffer_frames != sizeof(pay) ||
        recv(client_sock, &pay, sizeof(pay), MSG_WAITALL) != (ssize_t)sizeof(pay)) {
        resp.hr = E_INVALIDARG;
        send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
        close(client_sock);
        return;
    }

    for (int b = 0; b < LWASAPI_EQ_BANDS; b++) {
        if (pay.eq.band_q[b] < 0.2f || pay.eq.band_q[b] > 10.0f) pay.eq.band_q[b] = 1.414f;
    }
    clamp_param_eq_ext(&pay.ext);

    pthread_mutex_lock(&g_sessions_lock);
    g_eq_cfg = pay.eq;
    g_param_eq = pay.ext;
    pthread_mutex_unlock(&g_sessions_lock);

    pthread_mutex_lock(&g_reconf_lock);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                            (int32_t)pay.eq.enabled, (int32_t)pay.ext.crossfeed_enabled, S_OK,
                            "PARAM_EQ_SET", pay.eq.preset_name[0] ? pay.eq.preset_name : "parametric",
                            "en=%u dyn=%u xfeed=%u(%.2f)",
                            pay.eq.enabled, pay.eq.dynamic_mode,
                            pay.ext.crossfeed_enabled, (double)pay.ext.crossfeed_level);

    resp.hr = S_OK;
    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_set_mic_apo(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    lwasapi_mic_apo_cfg_t cfg;
    if (req->requested_buffer_frames != sizeof(cfg) ||
        recv(client_sock, &cfg, sizeof(cfg), MSG_WAITALL) != (ssize_t)sizeof(cfg)) {
        resp.hr = E_INVALIDARG;
        send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
        close(client_sock);
        return;
    }

    clamp_mic_apo_cfg(&cfg);
    pthread_mutex_lock(&g_sessions_lock);
    g_mic_apo = cfg;
    pthread_mutex_unlock(&g_sessions_lock);

    pthread_mutex_lock(&g_reconf_lock);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                            (int32_t)cfg.enabled, (int32_t)cfg.rnnoise_enabled, S_OK,
                            "MIC_APO_SET", cfg.enabled ? "active" : "bypass",
                            "hpf=%.0f gate=%.0fdB rnn=%u comp=%.0fdB sidetone=%u%%",
                            (double)cfg.hpf_hz, (double)cfg.gate_thresh_db,
                            cfg.rnnoise_enabled, (double)cfg.comp_thresh_db,
                            cfg.sidetone_enabled ? cfg.sidetone_pct : 0U);

    resp.hr = S_OK;
    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_set_bt_codec(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    lwasapi_bt_codec_cfg_t cfg;
    if (req->requested_buffer_frames != sizeof(cfg) ||
        recv(client_sock, &cfg, sizeof(cfg), MSG_WAITALL) != (ssize_t)sizeof(cfg)) {
        resp.hr = E_INVALIDARG;
        send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
        close(client_sock);
        return;
    }

    if (g_bt.initialized) {
        lwasapi_bluez_set_sco_enabled(&g_bt, cfg.hfp_sco_enabled != 0U);
        lwasapi_bluez_set_codec_config(&g_bt, &cfg);
        lwasapi_bluez_get_codec_config(&g_bt, &g_saved_bt_cfg);

        /* If HFP SCO was turned off while bluez_sco capture was active, restore Scarlett/default capture */
        if (!cfg.hfp_sco_enabled) {
            bool was_sco = false;
            pthread_mutex_lock(&g_hw_lock);
            was_sco = (strncmp(g_active_capture_id, "bluez_sco:", 10) == 0);
            pthread_mutex_unlock(&g_hw_lock);
            if (was_sco) {
                switch_engine_devices_impl(NULL, "auto", false);
            }
        }
    } else {
        g_saved_bt_cfg = cfg;
    }

    pthread_mutex_lock(&g_reconf_lock);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    if (g_pa_srv) {
        lwasapi_pa_server_notify_change(g_pa_srv);
    }

    lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                            (int32_t)g_saved_bt_cfg.preferred_codec,
                            (int32_t)g_saved_bt_cfg.ldac_mode, S_OK,
                            "BT_CODEC_CFG", "a2dp",
                            "pref=%u ldac=%u abr=%u sco=%u kbps=%u",
                            g_saved_bt_cfg.preferred_codec, g_saved_bt_cfg.ldac_mode,
                            g_saved_bt_cfg.wireless_abr_enabled, g_saved_bt_cfg.hfp_sco_enabled,
                            g_saved_bt_cfg.effective_kbps);

    resp.hr = S_OK;
    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static void handle_set_hw_analog(int client_sock, const lwasapi_ipc_req_t *req) {
    lwasapi_ipc_resp_t resp;
    memset(&resp, 0, sizeof(resp));
    lwasapi_hw_analog_cfg_t cfg;
    if (req->requested_buffer_frames != sizeof(cfg) ||
        recv(client_sock, &cfg, sizeof(cfg), MSG_WAITALL) != (ssize_t)sizeof(cfg)) {
        resp.hr = E_INVALIDARG;
        send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
        close(client_sock);
        return;
    }

    lwasapi_hw_analog_clamp_cfg(&cfg);
    pthread_mutex_lock(&g_sessions_lock);
    g_hw_analog_cfg = cfg;
    pthread_mutex_unlock(&g_sessions_lock);

    hw_analog_probe_and_sync(true);

    pthread_mutex_lock(&g_reconf_lock);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);

    if (g_pa_srv) {
        lwasapi_pa_server_notify_change(g_pa_srv);
    }

    lwasapi_debug_emit_vals(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                            (int32_t)cfg.hp_amp_tier, (int32_t)cfg.output_route, S_OK,
                            "HW_ANALOG_CFG", "analog",
                            "unity=%u tier=%u route=%u rails=%u boost=%.1fdB mic=%.1fdB",
                            cfg.hw_dac_unity_vol, cfg.hp_amp_tier, cfg.output_route,
                            cfg.unlock_0db_rails, cfg.analog_boost_db, cfg.mic_hw_boost_db);

    resp.hr = S_OK;
    send(client_sock, &resp, sizeof(resp), MSG_NOSIGNAL);
    close(client_sock);
}

static bool dev_id_is_usb_card(const char *dev_id) {
    unsigned c = 0, d = 0;
    if (!dev_id || sscanf(dev_id, "hw:%u,%u", &c, &d) != 2) return false;
    char probe[64];
    snprintf(probe, sizeof(probe), "/proc/asound/card%u/usbid", c);
    return access(probe, F_OK) == 0;
}

static bool device_list_has_type(const lwasapi_device_list_t *dl, const char *dev_id,
                                 uint32_t want_capture) {
    if (!dl || !dev_id || !dev_id[0]) return false;
    bool want_any_bt = (strcmp(dev_id, "bluetooth") == 0);
    for (uint32_t i = 0; i < dl->num_devices; i++) {
        const lwasapi_device_entry_t *e = &dl->devices[i];
        if (e->is_capture != want_capture) continue;
        if (want_any_bt && e->is_bluetooth) return true;
        if (strcmp(e->id, dev_id) == 0) return true;
    }
    return false;
}

static bool device_list_has_any_type(const lwasapi_device_list_t *dl, uint32_t want_capture,
                                     bool usb_only) {
    if (!dl) return false;
    for (uint32_t i = 0; i < dl->num_devices; i++) {
        const lwasapi_device_entry_t *e = &dl->devices[i];
        if (e->is_capture != want_capture) continue;
        if (usb_only && (e->is_bluetooth || !dev_id_is_usb_card(e->id))) continue;
        return true;
    }
    return false;
}

static bool sync_hotplug_topology(bool force_virtual, bool bt_now) {
    bool cap_deferred = false;
    static uint64_t g_usb_cap_appear_ns = 0;

    pthread_mutex_lock(&g_reconf_lock);

    lwasapi_device_list_t dlist;
    enumerate_devices_safe(&dlist);

    pthread_mutex_lock(&g_bt.state_lock);
    bool bt_has_cap = g_bt.has_capture_hw;
    pthread_mutex_unlock(&g_bt.state_lock);

    pthread_mutex_lock(&g_hw_lock);
    uint32_t pref_r = g_pref_sample_rate ? g_pref_sample_rate : g_hw.sample_rate;
    uint32_t pref_p = g_pref_period_frames ? g_pref_period_frames : g_hw.period_frames;
    uint32_t pref_b = g_pref_format_bits ? g_pref_format_bits : g_hw.format_bits;
    uint32_t eng_rate = g_hw.sample_rate ? g_hw.sample_rate : 48000;
    uint32_t pref_cap = g_pref_cap_sample_rate;
    bool hw_is_virtual = g_hw.is_virtual;
    bool hw_is_bt      = g_hw.is_bluetooth;
    char target_pb[64], active_pb[64], target_cap[64], active_cap[64];
    snprintf(target_pb, sizeof(target_pb), "%s", g_target_playback_id);
    snprintf(active_pb, sizeof(active_pb), "%s", g_active_playback_id);
    snprintf(target_cap, sizeof(target_cap), "%s", g_target_capture_id);
    snprintf(active_cap, sizeof(active_cap), "%s", g_active_capture_id);
    pthread_mutex_unlock(&g_hw_lock);

    if (!force_virtual && strcmp(target_pb, "virtual") != 0) {
        bool target_auto = (strcmp(target_pb, "auto") == 0);
        bool target_is_bt = (strcmp(target_pb, "bluetooth") == 0 || strncmp(target_pb, "bluez", 5) == 0);
        bool active_kernel_lost = (!hw_is_virtual && !hw_is_bt &&
                                   !device_list_has_type(&dlist, active_pb, 0));
        bool active_bt_lost = (hw_is_bt && !bt_now);
        bool target_returned = (!target_auto &&
                                (!target_is_bt || bt_now) &&
                                strcmp(active_pb, target_pb) != 0 &&
                                device_list_has_type(&dlist, target_pb, 0));
        bool auto_upgrade_bt = (target_auto && !hw_is_bt && bt_now);
        bool auto_recover_hw = (target_auto && hw_is_virtual && device_list_has_any_type(&dlist, 0, false));
        bool auto_upgrade_usb = (target_auto && !hw_is_bt &&
                                 !dev_id_is_usb_card(active_pb) &&
                                 device_list_has_any_type(&dlist, 0, true));

        if (target_returned) {
            fprintf(stderr, "[LWASAPI-audiodg] Hotplug restored preferred playback endpoint: %s\n",
                    target_pb);
            lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                               "HOTPLUG_PB", target_pb, "restored preferred target");
            apply_playback_endpoint(target_pb, pref_r, pref_p, pref_b, false, false, &dlist);
        } else if (active_kernel_lost || active_bt_lost || auto_upgrade_bt || auto_recover_hw || auto_upgrade_usb) {
            fprintf(stderr, "[LWASAPI-audiodg] Hotplug playback change detected, re-evaluating auto route...\n");
            lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                               "HOTPLUG_PB", target_auto ? "auto" : target_pb,
                               "lost=%d btlost=%d upg_bt=%d rec=%d usb=%d",
                               active_kernel_lost, active_bt_lost, auto_upgrade_bt, auto_recover_hw, auto_upgrade_usb);
            apply_playback_endpoint(target_auto ? "auto" : target_pb,
                                    pref_r, pref_p, pref_b, false, false, &dlist);
        }
        pthread_mutex_lock(&g_hw_lock);
        if (g_hw.sample_rate) eng_rate = g_hw.sample_rate;
        pthread_mutex_unlock(&g_hw_lock);
    }

    if (!force_virtual) {
        bool cap_none = (strcmp(active_cap, "none") == 0 || !active_cap[0]);
        bool cap_still_plugged = (!cap_none && bt_has_cap &&
                                  device_list_has_type(&dlist, active_cap, 1));
        bool target_cap_auto = (strcmp(target_cap, "auto") == 0 || !target_cap[0]);
        bool target_cap_returned = (!target_cap_auto &&
                                    strcmp(active_cap, target_cap) != 0 &&
                                    device_list_has_type(&dlist, target_cap, 1));
        bool cap_appeared = (cap_none && device_list_has_any_type(&dlist, 1, false));
        bool cap_upgrade_usb = (target_cap_auto && cap_still_plugged &&
                                !dev_id_is_usb_card(active_cap) &&
                                device_list_has_any_type(&dlist, 1, true));

        bool cap_lost = (!cap_none && !cap_still_plugged);
        bool cap_new_usb = ((cap_appeared || target_cap_returned || cap_upgrade_usb) &&
                            device_list_has_any_type(&dlist, 1, true));
        if (!cap_new_usb) {
            g_usb_cap_appear_ns = 0;
        }
        if (cap_lost) {
            g_usb_cap_appear_ns = 0;
            pthread_mutex_lock(&g_hw_lock);
            snprintf(g_active_capture_id, sizeof(g_active_capture_id), "none");
            pthread_mutex_unlock(&g_hw_lock);
        }
        if (cap_new_usb) {
            uint64_t now_ns = monotonic_ns();
            if (g_usb_cap_appear_ns == 0) {
                g_usb_cap_appear_ns = now_ns;
                cap_deferred = true;
            } else if (now_ns - g_usb_cap_appear_ns < 500000000ULL) {
                cap_deferred = true;
            } else {
                g_usb_cap_appear_ns = 0;
            }
        }

        if (!cap_deferred && (cap_lost || cap_appeared || target_cap_returned || cap_upgrade_usb)) {
            const char *req_cap = target_cap[0] ? target_cap : "auto";
            int cap_rc = lwasapi_bluez_open_capture(&g_bt, req_cap, resolve_cap_sample_rate(eng_rate, pref_cap));
            pthread_mutex_lock(&g_bt.state_lock);
            bool has_cap_now = g_bt.has_capture_hw;
            char cap_id_now[64];
            snprintf(cap_id_now, sizeof(cap_id_now), "%s", g_bt.cap_device_id);
            pthread_mutex_unlock(&g_bt.state_lock);

            char new_cap_id[64];
            if (cap_rc == 0 && has_cap_now && cap_id_now[0]) {
                snprintf(new_cap_id, sizeof(new_cap_id), "%s", cap_id_now);
            } else {
                snprintf(new_cap_id, sizeof(new_cap_id), "none");
            }
            pthread_mutex_lock(&g_hw_lock);
            snprintf(g_active_capture_id, sizeof(g_active_capture_id), "%s", new_cap_id);
            pthread_mutex_unlock(&g_hw_lock);

            lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                               "HOTPLUG_CAP", new_cap_id,
                               "req=%s active=%d", req_cap, has_cap_now ? 1 : 0);
        }
    }
    hw_analog_probe_and_sync(true);
    pthread_mutex_unlock(&g_reconf_lock);
    return cap_deferred;
}

int main(int argc, char **argv) {
    unsetenv("LD_LIBRARY_PATH");
    {
        pthread_mutexattr_t mattr;
        pthread_mutexattr_init(&mattr);
        pthread_mutexattr_setprotocol(&mattr, PTHREAD_PRIO_INHERIT);
        pthread_mutex_init(&g_sessions_lock, &mattr);
        pthread_mutex_init(&g_hw_lock, &mattr);
        pthread_mutex_init(&g_reconf_lock, &mattr);
        pthread_mutexattr_destroy(&mattr);
    }
    char sock_path[108];
    lwasapi_audiodg_sock_path(sock_path, sizeof(sock_path), LWASAPI_SOCKET_DEFAULT);
    if (strcmp(sock_path, LWASAPI_SOCKET_DEFAULT) != 0) {
        unlink(LWASAPI_SOCKET_DEFAULT);
    }

    const char *device_arg = NULL;
    uint32_t rate_arg      = 48000;
    uint32_t period_arg    = 256;
    uint32_t bits_arg      = 32;
    bool force_virtual     = false;
    bool force_bt          = false;

    load_salsa_state(&rate_arg, &period_arg, &bits_arg);
    g_hw_analog = lwasapi_hw_analog_create();

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--device") == 0 && i + 1 < argc) {
            device_arg = argv[++i];
        } else if (strcmp(argv[i], "--rate") == 0 && i + 1 < argc) {
            rate_arg = (uint32_t)atoi(argv[++i]);
        } else if (strcmp(argv[i], "--period") == 0 && i + 1 < argc) {
            period_arg = (uint32_t)atoi(argv[++i]);
        } else if (strcmp(argv[i], "--virtual") == 0) {
            force_virtual = true;
        } else if (strcmp(argv[i], "--bluetooth") == 0) {
            force_bt = true;
        } else if (strcmp(argv[i], "--socket") == 0 && i + 1 < argc) {
            snprintf(sock_path, sizeof(sock_path), "%s", argv[++i]);
        }
    }

    signal(SIGINT, handle_sigint);
    signal(SIGTERM, handle_sigint);
    signal(SIGPIPE, SIG_IGN);

    g_pref_sample_rate   = rate_arg;
    g_pref_period_frames = period_arg;
    g_pref_format_bits   = bits_arg;
    init_hardware(device_arg ? device_arg : "auto", rate_arg, period_arg, bits_arg, force_virtual, force_bt);
    lwasapi_midi_init(&g_midi);

    lwasapi_pa_callbacks_t pa_cbs = {
        .user_ctx          = NULL,
        .get_status        = pa_cb_get_status,
        .get_devices       = pa_cb_get_devices,
        .set_devices       = pa_cb_set_devices,
        .set_master_volume = pa_cb_set_master_volume,
        .set_mic_gain_ppm  = pa_cb_set_mic_gain_ppm,
    };
    if (lwasapi_pa_server_start(&g_pa_srv, pa_cbs) == 0) {
        fprintf(stderr, "[LWASAPI-audiodg] Built-in PulseAudio native protocol v35 server active.\n");
    }
    if (lwasapi_pw_broker_start(&g_pw_brk, NULL) == 0) {
        fprintf(stderr, "[LWASAPI-audiodg] Built-in Video-Only PipeWire ScreenCast broker initialized.\n");
    }

    unlink(sock_path);
    int listen_fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (listen_fd < 0) {
        perror("socket");
        return 1;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", sock_path);

    if (bind(listen_fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        perror("bind");
        return 1;
    }
    chmod(sock_path, 0600);
    if (listen(listen_fd, 32) < 0) {
        perror("listen");
        return 1;
    }

    lwasapi_dbg_callbacks_t dbg_cbs = {
        .user_ctx = NULL,
        .fill_snapshot = dbg_fill_snapshot_cb,
        .handle_cmd = dbg_handle_cmd_cb,
    };
    lwasapi_debug_start(dbg_cbs);
    {
        pthread_mutex_lock(&g_hw_lock);
        char pb_id[64], devp[64], cap_id[64];
        snprintf(pb_id, sizeof(pb_id), "%s", g_active_playback_id);
        snprintf(devp, sizeof(devp), "%s", g_hw.device_path);
        snprintf(cap_id, sizeof(cap_id), "%s", g_active_capture_id);
        uint32_t r = g_hw.sample_rate, p = g_hw.period_frames, ch = g_hw.channels, b = g_hw.format_bits;
        pthread_mutex_unlock(&g_hw_lock);
        lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, (uint32_t)getpid(), 0,
                           "ENGINE_START", pb_id, "dev=%s cap=%s rate=%u ch=%u bits=%u period=%u",
                           devp, cap_id, r, ch, b, p);
    }

    pthread_t engine_tid;
    pthread_create(&engine_tid, NULL, audio_engine_thread, NULL);

    fprintf(stderr, "[LWASAPI-audiodg] Listening on %s (Ready for WASAPI Shared/Exclusive, MIDI Hub & Native PA/ALSA/JACK)\n", sock_path);

    pthread_mutex_lock(&g_bt.state_lock);
    bool     prev_bt_has    = (g_bt.has_bluez_transport && g_bt.direct_socket_mode);
    uint16_t prev_bt_avrcp  = __atomic_load_n(&g_bt.avrcp_volume, __ATOMIC_RELAXED);
    bool     prev_muted     = __atomic_load_n(&g_bt.desktop_muted, __ATOMIC_RELAXED);
    pthread_mutex_unlock(&g_bt.state_lock);

    uint64_t next_watcher_ns = monotonic_ns() + 250000000ULL;

    while (g_running) {
        struct pollfd pfd[1 + LWASAPI_MAX_SESSIONS];
        int slot_map[1 + LWASAPI_MAX_SESSIONS];
        nfds_t nfds = 0;

        pfd[nfds].fd = listen_fd;
        pfd[nfds].events = POLLIN;
        slot_map[nfds] = -1;
        nfds++;

        pthread_mutex_lock(&g_sessions_lock);
        for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
            if (g_sessions[i].in_use && g_sessions[i].client_sock >= 0) {
                pfd[nfds].fd = g_sessions[i].client_sock;
                pfd[nfds].events = POLLIN | POLLHUP | POLLERR;
                slot_map[nfds] = i;
                nfds++;
            }
        }
        pthread_mutex_unlock(&g_sessions_lock);

        uint64_t now_p_ns = monotonic_ns();
        int poll_ms = 0;
        if (now_p_ns < next_watcher_ns) {
            uint64_t rem_ms = (next_watcher_ns - now_p_ns + 999999ULL) / 1000000ULL;
            poll_ms = (rem_ms > 250ULL) ? 250 : (int)rem_ms;
        }
        int pr = poll(pfd, nfds, poll_ms);
        if (pr < 0) {
            if (errno == EINTR) continue;
            break;
        }

        if (pr > 0) {
            /* Check existing sessions for disconnect */
            for (nfds_t n = 1; n < nfds; n++) {
                if (pfd[n].revents & (POLLIN | POLLHUP | POLLERR)) {
                    char dummy[16];
                    ssize_t r = recv(pfd[n].fd, dummy, sizeof(dummy), MSG_DONTWAIT);
                    if (r <= 0) {
                        pthread_mutex_lock(&g_sessions_lock);
                        int slot = slot_map[n];
                        if (g_sessions[slot].in_use) {
                            fprintf(stderr, "[LWASAPI-audiodg] Session #%u ('%s', PID %u) closed.\n",
                                    g_sessions[slot].session_id, g_sessions[slot].ring->client_name,
                                    g_sessions[slot].ring->client_pid);
                            destroy_session_slot(slot);
                        }
                        pthread_mutex_unlock(&g_sessions_lock);
                    }
                }
            }

            /* Accept new client or control connection */
            if (pfd[0].revents & POLLIN) {
                int csock = accept4(listen_fd, NULL, NULL, SOCK_CLOEXEC);
                if (csock >= 0) {
                    struct ucred uc = {0};
                    socklen_t ulen = sizeof(uc);
                    if (getsockopt(csock, SOL_SOCKET, SO_PEERCRED, &uc, &ulen) != 0 ||
                        (uc.uid != getuid() && uc.uid != 0)) {
                        close(csock);
                    } else {
                        struct timeval rtv = { .tv_sec = 0, .tv_usec = 500000 };
                        (void)setsockopt(csock, SOL_SOCKET, SO_RCVTIMEO, &rtv, sizeof(rtv));
                        lwasapi_ipc_req_t req;
                        ssize_t n = recv(csock, &req, sizeof(req), MSG_WAITALL);
                        if (n == sizeof(req)) {
                            if (req.cmd == LWASAPI_CMD_CREATE_SESSION) {
                                handle_create_session(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_GET_STATUS) {
                                handle_get_status(csock);
                            } else if (req.cmd == LWASAPI_CMD_SET_VOLUME) {
                                handle_set_volume(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_INJECT_MIDI) {
                                handle_inject_midi(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_SET_ENGINE_CONFIG) {
                                handle_set_engine_config(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_SET_EQ) {
                                handle_set_eq(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_LIST_DEVICES) {
                                handle_list_devices(csock);
                            } else if (req.cmd == LWASAPI_CMD_SET_DEVICES) {
                                handle_set_devices(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_SET_SESSION_CONTROL) {
                                handle_set_session_control(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_SET_PARAM_EQ) {
                                handle_set_param_eq(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_SET_MIC_APO) {
                                handle_set_mic_apo(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_SET_BT_CODEC) {
                                handle_set_bt_codec(csock, &req);
                            } else if (req.cmd == LWASAPI_CMD_SET_HW_ANALOG) {
                                handle_set_hw_analog(csock, &req);
                            } else {
                                close(csock);
                            }
                        } else {
                            close(csock);
                        }
                    }
                }
            }
        }

        /* 250ms periodic watcher: ALSA Jack/HDMI/USB + BlueZ hotplug and PA topology sync */
        uint64_t now_w_ns = monotonic_ns();
        if (now_w_ns < next_watcher_ns) {
            continue;
        }
        next_watcher_ns = now_w_ns + 250000000ULL;

        pthread_mutex_lock(&g_bt.state_lock);
        bool bt_now        = (g_bt.has_bluez_transport && g_bt.direct_socket_mode);
        uint16_t avrcp_now = __atomic_load_n(&g_bt.avrcp_volume, __ATOMIC_RELAXED);
        bool muted_now     = __atomic_load_n(&g_bt.desktop_muted, __ATOMIC_RELAXED);
        char bt_alias[64], bt_codec[16];
        snprintf(bt_alias, sizeof(bt_alias), "%s", g_bt.device_alias);
        snprintf(bt_codec, sizeof(bt_codec), "%s", g_bt.codec_name);
        uint32_t bt_rate   = g_bt.bt_sample_rate;
        pthread_mutex_unlock(&g_bt.state_lock);

        static uint64_t prev_hw_fp = 0;
        static bool hw_fp_init = false;
        static bool pending_usb_cap_settle = false;
        uint64_t hw_fp = lwasapi_bluez_hw_fingerprint(&g_bt);
        bool hw_changed = hw_fp_init && (hw_fp != prev_hw_fp);
        if (!hw_fp_init) {
            prev_hw_fp = hw_fp;
            hw_fp_init = true;
        }

        if (hw_changed || bt_now != prev_bt_has || pending_usb_cap_settle) {
            if (bt_now != prev_bt_has) {
                lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                   "BT_STATE", bt_alias[0] ? bt_alias : "bluetooth",
                                   "connected=%d codec=%s rate=%u",
                                   bt_now ? 1 : 0,
                                   bt_codec[0] ? bt_codec : "none",
                                   bt_rate);
            }
            if (hw_changed) {
                lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                                   "HW_TOPOLOGY", "alsa", "fingerprint=0x%llx",
                                   (unsigned long long)hw_fp);
            }
            pending_usb_cap_settle = sync_hotplug_topology(force_virtual, bt_now);
            prev_hw_fp    = lwasapi_bluez_hw_fingerprint(&g_bt);
            prev_bt_has   = bt_now;
            prev_bt_avrcp = avrcp_now;
            prev_muted    = muted_now;
            if (g_pa_srv && !pending_usb_cap_settle) lwasapi_pa_server_notify_change(g_pa_srv);
        } else if (avrcp_now != prev_bt_avrcp || muted_now != prev_muted) {
            lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                               "BT_AVRCP_STATE", bt_alias[0] ? bt_alias : "bluetooth",
                               "avrcp=%u muted=%d", (unsigned)avrcp_now, muted_now ? 1 : 0);
            prev_bt_avrcp = avrcp_now;
            prev_muted    = muted_now;
            if (g_pa_srv) lwasapi_pa_server_notify_volume(g_pa_srv, false);
            mark_salsa_state_dirty();
        }
        if (g_hw_analog) {
            int hid_delta = 0, hid_mute = 0;
            if (lwasapi_hw_analog_poll_hid(g_hw_analog, &hid_delta, &hid_mute) > 0 &&
                (hid_delta != 0 || hid_mute != 0)) {
                uint32_t cur_v = __atomic_load_n(&g_bt.desktop_volume_pct, __ATOMIC_RELAXED);
                bool cur_m = __atomic_load_n(&g_bt.desktop_muted, __ATOMIC_RELAXED);
                int new_v = (int)cur_v + hid_delta * 2;
                if (new_v < 0) new_v = 0;
                if (new_v > 150) new_v = 150;
                int mute_op = hid_mute ? (cur_m ? 0 : 1) : -1;
                lwasapi_bluez_set_volume(&g_bt, (hid_delta != 0) ? new_v : -1, mute_op);
                hw_analog_probe_and_sync(false);
                if (g_pa_srv) lwasapi_pa_server_notify_volume(g_pa_srv, false);
                mark_salsa_state_dirty();
            }
        }
        if (atomic_load_explicit(&g_state_dirty, memory_order_acquire)) {
            struct timespec dts;
            clock_gettime(CLOCK_MONOTONIC, &dts);
            uint64_t now_ns = (uint64_t)dts.tv_sec * 1000000000ULL + (uint64_t)dts.tv_nsec;
            uint64_t dirty_at = atomic_load_explicit(&g_state_dirty_ns, memory_order_acquire);
            if (now_ns >= dirty_at + 400000000ULL) {
                atomic_store_explicit(&g_state_dirty, false, memory_order_release);
                pthread_mutex_lock(&g_reconf_lock);
                save_salsa_state();
                pthread_mutex_unlock(&g_reconf_lock);
            }
        }
    }

    pthread_join(engine_tid, NULL);
    pthread_mutex_lock(&g_reconf_lock);
    save_salsa_state();
    pthread_mutex_unlock(&g_reconf_lock);
    lwasapi_debug_stop();

    if (g_pw_brk) {
        lwasapi_pw_broker_stop(g_pw_brk);
        g_pw_brk = NULL;
    }
    if (g_pa_srv) {
        lwasapi_pa_server_stop(g_pa_srv);
        g_pa_srv = NULL;
    }

    pthread_mutex_lock(&g_sessions_lock);
    for (int i = 0; i < LWASAPI_MAX_SESSIONS; i++) {
        destroy_session_slot(i);
    }
    pthread_mutex_unlock(&g_sessions_lock);

    lwasapi_midi_close(&g_midi);
    if (g_hw_analog) {
        lwasapi_hw_analog_destroy(g_hw_analog);
        g_hw_analog = NULL;
    }
    if (g_bt.initialized) lwasapi_bluez_close(&g_bt);
    if (g_hw.fd >= 0) close(g_hw.fd);
    close(listen_fd);
    unlink(sock_path);
    return 0;
}
