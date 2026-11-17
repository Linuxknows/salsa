#define _GNU_SOURCE
#include "../include/lwasapi_bluez.h"
#include "../include/lwasapi_debug.h"
#include "../include/salsa_wire.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <dlfcn.h>
#include <poll.h>
#include <math.h>
#include <time.h>
#include <sched.h>
#include <arpa/inet.h>
#include <sys/ioctl.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sound/asound.h>
#include <dbus/dbus.h>

/* Portable Linux Bluetooth SCO kernel socket definitions (zero bluez-libs-dev dependency) */
#ifndef AF_BLUETOOTH
#define AF_BLUETOOTH 31
#endif
#ifndef PF_BLUETOOTH
#define PF_BLUETOOTH AF_BLUETOOTH
#endif
#ifndef BTPROTO_SCO
#define BTPROTO_SCO 2
#endif
#ifndef SOL_BLUETOOTH
#define SOL_BLUETOOTH 274
#endif
#ifndef BT_VOICE
#define BT_VOICE 11
#endif
#ifndef BT_VOICE_TRANSPARENT
#define BT_VOICE_TRANSPARENT 0x0003
#endif
#ifndef BT_VOICE_CVSD_16BIT
#define BT_VOICE_CVSD_16BIT 0x0060
#endif

typedef struct __attribute__((packed)) {
    uint8_t b[6];
} lwasapi_bdaddr_t;

struct lwasapi_sockaddr_sco {
    sa_family_t      sco_family;
    lwasapi_bdaddr_t sco_bdaddr;
};

struct lwasapi_bt_voice {
    uint16_t setting;
};

/* ============================================================================
 * A2DP Codec & RTP Protocol Wire Structs (No External BlueZ Dev Headers Needed)
 * ============================================================================ */

#define A2DP_SOURCE_UUID          "0000110a-0000-1000-8000-00805f9b34fb"
#define A2DP_SINK_UUID            "0000110b-0000-1000-8000-00805f9b34fb"
#define A2DP_CODEC_SBC            0x00
#define A2DP_CODEC_MPEG24         0x02
#define A2DP_CODEC_VENDOR         0xFF

#define LWASAPI_BT_CODEC_SBC      0
#define LWASAPI_BT_CODEC_AAC      1
#define LWASAPI_BT_CODEC_APTX     2
#define LWASAPI_BT_CODEC_APTX_HD  3
#define LWASAPI_BT_CODEC_LDAC     4

#define LWASAPI_EP_LDAC           "/org/lwasapi/A2DP/LDAC/Source"
#define LWASAPI_EP_APTXHD         "/org/lwasapi/A2DP/APTXHD/Source"
#define LWASAPI_EP_APTX           "/org/lwasapi/A2DP/APTX/Source"
#define LWASAPI_EP_AAC            "/org/lwasapi/A2DP/AAC/Source"
#define LWASAPI_EP_SBC            "/org/lwasapi/A2DP/SBC/Source"
#define HFP_HF_UUID               "0000111e-0000-1000-8000-00805f9b34fb"
#define HFP_AG_UUID               "0000111f-0000-1000-8000-00805f9b34fb"
#define LWASAPI_HFP_AG_PATH       "/org/lwasapi/HFP/AG"
#define LWASAPI_BATTERY_ROOT      "/org/lwasapi/Battery"

#define SBC_SAMPLING_FREQ_16000   (1 << 3)
#define SBC_SAMPLING_FREQ_32000   (1 << 2)
#define SBC_SAMPLING_FREQ_44100   (1 << 1)
#define SBC_SAMPLING_FREQ_48000   (1 << 0)

#define SBC_CHANNEL_MODE_MONO         (1 << 3)
#define SBC_CHANNEL_MODE_DUAL_CHANNEL (1 << 2)
#define SBC_CHANNEL_MODE_STEREO       (1 << 1)
#define SBC_CHANNEL_MODE_JOINT_STEREO (1 << 0)

#define SBC_BLOCK_LENGTH_4        (1 << 3)
#define SBC_BLOCK_LENGTH_8        (1 << 2)
#define SBC_BLOCK_LENGTH_12       (1 << 1)
#define SBC_BLOCK_LENGTH_16       (1 << 0)

#define SBC_SUBBANDS_4            (1 << 1)
#define SBC_SUBBANDS_8            (1 << 0)

#define SBC_ALLOCATION_SNR        (1 << 1)
#define SBC_ALLOCATION_LOUDNESS   (1 << 0)

#define AAC_OBJECT_TYPE_MPEG2_AAC_LC 0x80
#define AAC_OBJECT_TYPE_MPEG4_AAC_LC 0x40
#define AAC_SAMPLING_FREQ_44100      0x0010
#define AAC_SAMPLING_FREQ_48000      0x0008
#define AAC_CHANNELS_1               0x08
#define AAC_CHANNELS_2               0x04

typedef struct __attribute__((packed)) {
    uint8_t channel_mode:4;
    uint8_t frequency:4;
    uint8_t allocation_method:2;
    uint8_t subbands:2;
    uint8_t block_length:4;
    uint8_t min_bitpool;
    uint8_t max_bitpool;
} a2dp_sbc_t;

typedef struct __attribute__((packed)) {
    uint8_t object_type;
    uint8_t frequency1;
    uint8_t channels:4;
    uint8_t frequency2:4;
    uint8_t bitrate1:7;
    uint8_t vbr:1;
    uint8_t bitrate2;
    uint8_t bitrate3;
} a2dp_aac_t;

typedef struct __attribute__((packed)) {
    uint8_t  cc:4;
    uint8_t  x:1;
    uint8_t  p:1;
    uint8_t  v:2;
    uint8_t  pt:7;
    uint8_t  m:1;
    uint16_t sequence_number;
    uint32_t timestamp;
    uint32_t ssrc;
} rtp_header_t;

typedef struct __attribute__((packed)) {
    uint8_t frame_count:4;
    uint8_t rfa0:1;
    uint8_t is_last_fragment:1;
    uint8_t is_first_fragment:1;
    uint8_t is_fragmented:1;
} rtp_sbc_payload_t;

/* ============================================================================
 * Dynamic FDK-AAC ABI (loaded via dlopen("libfdk-aac.so.2"))
 * ============================================================================ */

#define FDK_AACENC_OK            0x0000
#define FDK_AOT_AAC_LC           2
#define FDK_AOT_MP2_AAC_LC       129
#define FDK_TT_MP4_LATM_MCP1     6
#define FDK_IN_AUDIO_DATA        0
#define FDK_OUT_BITSTREAM_DATA   3

#define FDK_PARAM_AOT            0x0100
#define FDK_PARAM_BITRATE        0x0101
#define FDK_PARAM_BITRATEMODE    0x0102
#define FDK_PARAM_SAMPLERATE     0x0103
#define FDK_PARAM_CHANNELMODE    0x0106
#define FDK_PARAM_AFTERBURNER    0x0200
#define FDK_PARAM_BANDWIDTH      0x0203
#define FDK_PARAM_PEAK_BITRATE   0x0207
#define FDK_PARAM_TRANSMUX       0x0300
#define FDK_PARAM_HEADER_PERIOD  0x0301
#define FDK_PARAM_SIGNALING_MODE 0x0302
#define FDK_PARAM_AUDIOMUXVER    0x0304

typedef struct {
    unsigned int maxOutBufBytes;
    unsigned int maxAncBytes;
    unsigned int inBufFillLevel;
    unsigned int inputChannels;
    unsigned int frameLength;
    unsigned int nDelay;
    unsigned int nDelayCore;
    unsigned char confBuf[64];
    unsigned int confSize;
} fdk_info_t;

typedef struct {
    int   numBufs;
    void **bufs;
    int  *bufferIdentifiers;
    int  *bufSizes;
    int  *bufElSizes;
} fdk_buf_desc_t;

typedef struct {
    int numInSamples;
    int numAncBytes;
} fdk_in_args_t;

typedef struct {
    int numOutBytes;
    int numInSamples;
    int numAncBytes;
    int bitResState;
} fdk_out_args_t;

typedef int (*pfn_aacEncOpen)(void **ph, const unsigned int encModules, const unsigned int maxChannels);
typedef int (*pfn_aacEncClose)(void **ph);
typedef int (*pfn_aacEncoder_SetParam)(void *h, const unsigned int param, const unsigned int value);
typedef int (*pfn_aacEncInfo)(void *h, fdk_info_t *pInfo);
typedef int (*pfn_aacEncEncode)(void *h, const fdk_buf_desc_t *inBufDesc, const fdk_buf_desc_t *outBufDesc,
                                const fdk_in_args_t *inArgs, fdk_out_args_t *outArgs);

static struct {
    void *dl;
    pfn_aacEncOpen          open;
    pfn_aacEncClose         close;
    pfn_aacEncoder_SetParam set_param;
    pfn_aacEncInfo          info;
    pfn_aacEncEncode        encode;
    bool                    checked;
    bool                    available;
} g_fdk = {0};

static lwasapi_bluez_t *g_bt_inst = NULL;
static pthread_mutex_t  g_dbus_enum_lock = PTHREAD_MUTEX_INITIALIZER;
static _Atomic int      g_need_acquire = 0;
static _Atomic int      g_need_reregister = 0;
static _Atomic int      g_need_avrcp_sync = 0;
static _Atomic int      g_pending_avrcp_vol = -1;
static _Atomic int      g_avrcp_supported = 0;
static _Atomic int      g_bt_recovery_active = 0;
static _Atomic uint64_t g_bt_last_recover_ns = 0;
static bool             g_ep_ldac_obj_registered = false;
static bool             g_ep_aptxhd_obj_registered = false;
static bool             g_ep_aptx_obj_registered = false;
static bool             g_ep_aac_obj_registered = false;
static bool             g_ep_sbc_obj_registered = false;
static char             g_bt_startup_autoconnect_dev[128] = {0};
static char             g_bt_kick_dev[128] = {0};

/* ============================================================================
 * Helpers: Timing, FDK-AAC Loader, Kernel ALSA Capture, Procfs Enumeration
 * ============================================================================ */

static uint64_t bt_monotonic_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static bool load_fdk_aac_library(void) {
    if (g_fdk.checked) return g_fdk.available;
    g_fdk.checked = true;
    const char *cands[] = {
        "libfdk-aac.so.2",
        "/usr/lib/libfdk-aac.so.2",
        "/usr/lib64/libfdk-aac.so.2",
        "/usr/lib/x86_64-linux-gnu/libfdk-aac.so.2",
        "/usr/lib/aarch64-linux-gnu/libfdk-aac.so.2",
        NULL
    };
    for (int i = 0; cands[i]; i++) {
        g_fdk.dl = dlopen(cands[i], RTLD_LAZY);
        if (g_fdk.dl) break;
    }
    if (!g_fdk.dl) return false;

    g_fdk.open      = (pfn_aacEncOpen)dlsym(g_fdk.dl, "aacEncOpen");
    g_fdk.close     = (pfn_aacEncClose)dlsym(g_fdk.dl, "aacEncClose");
    g_fdk.set_param = (pfn_aacEncoder_SetParam)dlsym(g_fdk.dl, "aacEncoder_SetParam");
    g_fdk.info      = (pfn_aacEncInfo)dlsym(g_fdk.dl, "aacEncInfo");
    g_fdk.encode    = (pfn_aacEncEncode)dlsym(g_fdk.dl, "aacEncEncode");
    if (!g_fdk.open || !g_fdk.close || !g_fdk.set_param || !g_fdk.info || !g_fdk.encode) {
        dlclose(g_fdk.dl);
        g_fdk.dl = NULL;
        return false;
    }
    g_fdk.available = true;
    return true;
}

/* ============================================================================
 * Dynamic LDAC (libldacBT_enc.so.2) & aptX / aptX HD (libfreeaptx.so.0) ABI
 * ============================================================================ */

#define APTX_VENDOR_ID            0x0000004FU
#define APTX_CODEC_ID             0x0001U
#define APTX_SAMPLING_FREQ_16000  0x08U
#define APTX_SAMPLING_FREQ_32000  0x04U
#define APTX_SAMPLING_FREQ_44100  0x02U
#define APTX_SAMPLING_FREQ_48000  0x01U
#define APTX_CHANNEL_MODE_MONO    0x01U
#define APTX_CHANNEL_MODE_STEREO  0x02U

#define APTX_HD_VENDOR_ID         0x000000D7U
#define APTX_HD_CODEC_ID          0x0024U

#define LDAC_VENDOR_ID            0x0000012DU
#define LDAC_CODEC_ID             0x00AAU
#define LDAC_SAMPLING_FREQ_44100  0x20U
#define LDAC_SAMPLING_FREQ_48000  0x10U
#define LDAC_SAMPLING_FREQ_88200  0x08U
#define LDAC_SAMPLING_FREQ_96000  0x04U
#define LDAC_CHANNEL_MODE_MONO    0x04U
#define LDAC_CHANNEL_MODE_DUAL    0x02U
#define LDAC_CHANNEL_MODE_STEREO  0x01U

#define LDACBT_ENC_LSU            128
#define LDACBT_EQMID_HQ           0
#define LDACBT_EQMID_SQ           1
#define LDACBT_EQMID_MQ           2
#define LDACBT_SMPL_FMT_S16       2

typedef struct __attribute__((packed)) {
    uint8_t vendor_id4;
    uint8_t vendor_id3;
    uint8_t vendor_id2;
    uint8_t vendor_id1;
    uint8_t codec_id2;
    uint8_t codec_id1;
} a2dp_vendor_codec_t;

#define A2DP_GET_VENDOR_ID(a) ( \
    (((uint32_t)(a).vendor_id4) <<  0) | \
    (((uint32_t)(a).vendor_id3) <<  8) | \
    (((uint32_t)(a).vendor_id2) << 16) | \
    (((uint32_t)(a).vendor_id1) << 24))
#define A2DP_GET_CODEC_ID(a) ((uint16_t)((a).codec_id2 | (((uint16_t)(a).codec_id1) << 8)))
#define A2DP_SET_VENDOR_HEADER(info, vid, cid) do { \
    (info).vendor_id4 = (uint8_t)(((vid) >>  0) & 0xFFU); \
    (info).vendor_id3 = (uint8_t)(((vid) >>  8) & 0xFFU); \
    (info).vendor_id2 = (uint8_t)(((vid) >> 16) & 0xFFU); \
    (info).vendor_id1 = (uint8_t)(((vid) >> 24) & 0xFFU); \
    (info).codec_id2  = (uint8_t)(((cid) >>  0) & 0xFFU); \
    (info).codec_id1  = (uint8_t)(((cid) >>  8) & 0xFFU); \
} while (0)

typedef struct __attribute__((packed)) {
    a2dp_vendor_codec_t info;
    uint8_t frequency;
    uint8_t channel_mode;
} a2dp_ldac_t;

typedef struct __attribute__((packed)) {
    a2dp_vendor_codec_t info;
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
    uint8_t frequency:4;
    uint8_t channel_mode:4;
#else
    uint8_t channel_mode:4;
    uint8_t frequency:4;
#endif
} a2dp_aptx_t;

typedef struct __attribute__((packed)) {
    a2dp_aptx_t aptx;
    uint8_t reserved0;
    uint8_t reserved1;
    uint8_t reserved2;
    uint8_t reserved3;
} a2dp_aptx_hd_t;

typedef void *(*pfn_ldac_get_handle)(void);
typedef void  (*pfn_ldac_free_handle)(void *h);
typedef int   (*pfn_ldac_init_handle_encode)(void *h, int mtu, int eqmid, int cm, int fmt, int sf);
typedef int   (*pfn_ldac_encode)(void *h, void *pcm, int *pcm_used, unsigned char *stream, int *stream_sz, int *frame_num);
typedef int   (*pfn_ldac_set_eqmid)(void *h, int eqmid);
typedef int   (*pfn_ldac_get_eqmid)(void *h);
typedef int   (*pfn_ldac_get_bitrate)(void *h);
typedef int   (*pfn_ldac_alter_eqmid_priority)(void *h, int priority);

static struct {
    void *dl;
    pfn_ldac_get_handle           get_handle;
    pfn_ldac_free_handle          free_handle;
    pfn_ldac_init_handle_encode   init_encode;
    pfn_ldac_encode               encode;
    pfn_ldac_set_eqmid            set_eqmid;
    pfn_ldac_get_eqmid            get_eqmid;
    pfn_ldac_get_bitrate          get_bitrate;
    pfn_ldac_alter_eqmid_priority alter_prio;
    bool checked;
    bool available;
} g_ldac = {0};

typedef void *(*pfn_ldac_abr_get_handle)(void);
typedef void  (*pfn_ldac_abr_free_handle)(void *h_abr);
typedef int   (*pfn_ldac_abr_init)(void *h_abr, unsigned int interval_ms);
typedef int   (*pfn_ldac_abr_set_thresholds)(void *h_abr, unsigned int crit, unsigned int dang, unsigned int saf);
typedef int   (*pfn_ldac_abr_proc)(void *h_ldac, void *h_abr, unsigned int tx_qdepth, unsigned int flag_enable);

static struct {
    void *dl;
    pfn_ldac_abr_get_handle     get_handle;
    pfn_ldac_abr_free_handle    free_handle;
    pfn_ldac_abr_init           init;
    pfn_ldac_abr_set_thresholds set_thresholds;
    pfn_ldac_abr_proc           proc;
    bool checked;
    bool available;
} g_ldac_abr = {0};

typedef void  *(*pfn_aptx_init)(int hd);
typedef void   (*pfn_aptx_finish)(void *ctx);
typedef size_t (*pfn_aptx_encode)(void *ctx, const unsigned char *in, size_t in_sz,
                                  unsigned char *out, size_t out_sz, size_t *written);

static struct {
    void *dl;
    pfn_aptx_init   init;
    pfn_aptx_finish finish;
    pfn_aptx_encode encode;
    bool checked;
    bool available;
} g_aptx = {0};

static bool load_ldac_abr_library(void) {
    if (g_ldac_abr.checked) return g_ldac_abr.available;
    g_ldac_abr.checked = true;
    const char *cands[] = {
        "/usr/lib/libldacBT_abr.so.2",
        "/usr/lib64/libldacBT_abr.so.2",
        "/usr/lib/x86_64-linux-gnu/libldacBT_abr.so.2",
        "/usr/lib/aarch64-linux-gnu/libldacBT_abr.so.2",
        "libldacBT_abr.so.2",
        NULL
    };
    for (int i = 0; cands[i]; i++) {
        g_ldac_abr.dl = dlopen(cands[i], RTLD_NOW | RTLD_LOCAL);
        if (g_ldac_abr.dl) break;
    }
    if (!g_ldac_abr.dl) return false;
    g_ldac_abr.get_handle     = (pfn_ldac_abr_get_handle)dlsym(g_ldac_abr.dl, "ldac_ABR_get_handle");
    g_ldac_abr.free_handle    = (pfn_ldac_abr_free_handle)dlsym(g_ldac_abr.dl, "ldac_ABR_free_handle");
    g_ldac_abr.init           = (pfn_ldac_abr_init)dlsym(g_ldac_abr.dl, "ldac_ABR_Init");
    g_ldac_abr.set_thresholds = (pfn_ldac_abr_set_thresholds)dlsym(g_ldac_abr.dl, "ldac_ABR_set_thresholds");
    g_ldac_abr.proc           = (pfn_ldac_abr_proc)dlsym(g_ldac_abr.dl, "ldac_ABR_Proc");
    if (!g_ldac_abr.get_handle || !g_ldac_abr.free_handle || !g_ldac_abr.init ||
        !g_ldac_abr.set_thresholds || !g_ldac_abr.proc) {
        dlclose(g_ldac_abr.dl);
        g_ldac_abr.dl = NULL;
        return false;
    }
    g_ldac_abr.available = true;
    return true;
}

static bool load_ldac_library(void) {
    if (g_ldac.checked) return g_ldac.available;
    g_ldac.checked = true;
    const char *cands[] = {
        "/usr/lib/libldacBT_enc.so.2",
        "/usr/lib64/libldacBT_enc.so.2",
        "/usr/lib/x86_64-linux-gnu/libldacBT_enc.so.2",
        "/usr/lib/aarch64-linux-gnu/libldacBT_enc.so.2",
        "libldacBT_enc.so.2",
        NULL
    };
    for (int i = 0; cands[i]; i++) {
        g_ldac.dl = dlopen(cands[i], RTLD_NOW | RTLD_LOCAL);
        if (g_ldac.dl) break;
    }
    if (!g_ldac.dl) return false;
    g_ldac.get_handle  = (pfn_ldac_get_handle)dlsym(g_ldac.dl, "ldacBT_get_handle");
    g_ldac.free_handle = (pfn_ldac_free_handle)dlsym(g_ldac.dl, "ldacBT_free_handle");
    g_ldac.init_encode = (pfn_ldac_init_handle_encode)dlsym(g_ldac.dl, "ldacBT_init_handle_encode");
    g_ldac.encode      = (pfn_ldac_encode)dlsym(g_ldac.dl, "ldacBT_encode");
    g_ldac.set_eqmid   = (pfn_ldac_set_eqmid)dlsym(g_ldac.dl, "ldacBT_set_eqmid");
    g_ldac.get_eqmid   = (pfn_ldac_get_eqmid)dlsym(g_ldac.dl, "ldacBT_get_eqmid");
    g_ldac.get_bitrate = (pfn_ldac_get_bitrate)dlsym(g_ldac.dl, "ldacBT_get_bitrate");
    g_ldac.alter_prio  = (pfn_ldac_alter_eqmid_priority)dlsym(g_ldac.dl, "ldacBT_alter_eqmid_priority");
    if (!g_ldac.get_handle || !g_ldac.free_handle || !g_ldac.init_encode || !g_ldac.encode) {
        dlclose(g_ldac.dl);
        g_ldac.dl = NULL;
        return false;
    }
    load_ldac_abr_library();
    g_ldac.available = true;
    return true;
}

static bool load_aptx_library(void) {
    if (g_aptx.checked) return g_aptx.available;
    g_aptx.checked = true;
    const char *cands[] = {
        "/usr/lib/libfreeaptx.so.0",
        "/usr/lib64/libfreeaptx.so.0",
        "/usr/lib/x86_64-linux-gnu/libfreeaptx.so.0",
        "/usr/lib/aarch64-linux-gnu/libfreeaptx.so.0",
        "libfreeaptx.so.0",
        NULL
    };
    for (int i = 0; cands[i]; i++) {
        g_aptx.dl = dlopen(cands[i], RTLD_NOW | RTLD_LOCAL);
        if (g_aptx.dl) break;
    }
    if (!g_aptx.dl) return false;
    g_aptx.init   = (pfn_aptx_init)dlsym(g_aptx.dl, "aptx_init");
    g_aptx.finish = (pfn_aptx_finish)dlsym(g_aptx.dl, "aptx_finish");
    g_aptx.encode = (pfn_aptx_encode)dlsym(g_aptx.dl, "aptx_encode");
    if (!g_aptx.init || !g_aptx.finish || !g_aptx.encode) {
        dlclose(g_aptx.dl);
        g_aptx.dl = NULL;
        return false;
    }
    g_aptx.available = true;
    return true;
}

static void alsa_hw_params_any(struct snd_pcm_hw_params *params) {
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

static void alsa_hw_set_mask(struct snd_pcm_hw_params *params, int param, unsigned int bit) {
    struct snd_mask *m = &params->masks[param - SNDRV_PCM_HW_PARAM_FIRST_MASK];
    memset(m, 0, sizeof(*m));
    m->bits[bit >> 5] |= (1U << (bit & 31));
    params->cmask |= (1U << param);
}

static void alsa_hw_set_minmax(struct snd_pcm_hw_params *params, int param, unsigned int minv, unsigned int maxv) {
    struct snd_interval *iv = &params->intervals[param - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL];
    iv->min = minv;
    iv->max = maxv;
    iv->openmin = 0;
    iv->openmax = 0;
    iv->integer = 0;
    params->cmask |= (1U << param);
}

static void parse_asound_cards(char card_names[32][64], int card_is_usb[32]) {
    for (int i = 0; i < 32; i++) {
        card_names[i][0] = '\0';
        card_is_usb[i] = 0;
    }
    FILE *fp = fopen("/proc/asound/cards", "r");
    if (!fp) return;
    char line[256];
    int last_idx = -1;
    while (fgets(line, sizeof(line), fp)) {
        char *p = line;
        while (*p == ' ') p++;
        if (isdigit((unsigned char)*p)) {
            int idx = atoi(p);
            if (idx >= 0 && idx < 32) {
                last_idx = idx;
                if (strstr(p, "USB-Audio")) card_is_usb[idx] = 1;
                char *dash = strstr(p, " - ");
                if (dash) {
                    dash += 3;
                    size_t n = strcspn(dash, "\r\n");
                    if (n > 63) n = 63;
                    memcpy(card_names[idx], dash, n);
                    card_names[idx][n] = '\0';
                }
            }
        } else if (last_idx >= 0 && last_idx < 32) {
            char *at = strstr(p, " at ");
            if (at && at > p) {
                size_t n = (size_t)(at - p);
                if (n > 63) n = 63;
                memcpy(card_names[last_idx], p, n);
                card_names[last_idx][n] = '\0';
            }
            last_idx = -1;
        }
    }
    fclose(fp);
}

#define LWASAPI_MAX_CAP_CHANNELS 32U

static int try_configure_capture_pcm(int fd, int fmt, uint32_t rate,
                                     uint32_t ch_min, uint32_t ch_max,
                                     uint32_t per_min, uint32_t per_max,
                                     uint32_t buf_min, uint32_t buf_max,
                                     uint32_t *out_rate, uint32_t *out_ch,
                                     uint32_t *out_per, uint32_t *out_buf) {
    struct snd_pcm_hw_params params;
    alsa_hw_params_any(&params);
    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_REFINE, &params) < 0) return -1;
    alsa_hw_set_mask(&params, SNDRV_PCM_HW_PARAM_ACCESS, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
    alsa_hw_set_mask(&params, SNDRV_PCM_HW_PARAM_FORMAT, (unsigned int)fmt);
    alsa_hw_set_minmax(&params, SNDRV_PCM_HW_PARAM_CHANNELS, ch_min, ch_max);
    alsa_hw_set_minmax(&params, SNDRV_PCM_HW_PARAM_RATE, rate, rate);
    alsa_hw_set_minmax(&params, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, per_min, per_max);
    alsa_hw_set_minmax(&params, SNDRV_PCM_HW_PARAM_BUFFER_SIZE, buf_min, buf_max);
    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS, &params) < 0) return -1;

    *out_rate = params.intervals[SNDRV_PCM_HW_PARAM_RATE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    *out_ch   = params.intervals[SNDRV_PCM_HW_PARAM_CHANNELS - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    *out_per  = params.intervals[SNDRV_PCM_HW_PARAM_PERIOD_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    *out_buf  = params.intervals[SNDRV_PCM_HW_PARAM_BUFFER_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    return 0;
}

static int query_capture_min_channels(int fd, int fmt, uint32_t rate, uint32_t *out_min_ch) {
    struct snd_pcm_hw_params params;
    alsa_hw_params_any(&params);
    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_REFINE, &params) < 0) return -1;
    alsa_hw_set_mask(&params, SNDRV_PCM_HW_PARAM_ACCESS, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
    alsa_hw_set_mask(&params, SNDRV_PCM_HW_PARAM_FORMAT, (unsigned int)fmt);
    alsa_hw_set_minmax(&params, SNDRV_PCM_HW_PARAM_RATE, rate, rate);
    alsa_hw_set_minmax(&params, SNDRV_PCM_HW_PARAM_CHANNELS, 1U, LWASAPI_MAX_CAP_CHANNELS);
    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_REFINE, &params) < 0) return -1;

    uint32_t cmin = params.intervals[SNDRV_PCM_HW_PARAM_CHANNELS - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    if (cmin < 1U || cmin > LWASAPI_MAX_CAP_CHANNELS) return -1;
    *out_min_ch = cmin;
    return 0;
}

static int open_kernel_capture_pcm(const char *dev_path, uint32_t req_rate,
                                   uint32_t *out_rate, uint32_t *out_ch, int *out_fmt) {
    int fd = open(dev_path, O_RDWR | O_CLOEXEC);
    if (fd < 0) return -1;

    const int fmts[] = { SNDRV_PCM_FORMAT_S32_LE, SNDRV_PCM_FORMAT_S24_3LE, SNDRV_PCM_FORMAT_S16_LE };
    const uint32_t chs[] = { 2U, 1U, 4U, 6U, 8U, 10U, 12U, 16U, 18U };
    const uint32_t rates[] = { req_rate, 48000U, 44100U, 96000U, 192000U, 32000U, 16000U };

    int chosen_fmt = -1;
    uint32_t actual_rate = req_rate;
    uint32_t actual_ch = 2;
    uint32_t actual_per = 256;
    uint32_t actual_buf = 1024;

    for (size_t r = 0; r < sizeof(rates) / sizeof(rates[0]) && chosen_fmt < 0; r++) {
        if (rates[r] == 0) continue;
        if (r > 0 && rates[r] == req_rate) continue;
        for (size_t f = 0; f < sizeof(fmts) / sizeof(fmts[0]) && chosen_fmt < 0; f++) {
            for (size_t c = 0; c < sizeof(chs) / sizeof(chs[0]) && chosen_fmt < 0; c++) {
                if (try_configure_capture_pcm(fd, fmts[f], rates[r], chs[c], chs[c],
                                              64U, 256U, 1024U, 8192U,
                                              &actual_rate, &actual_ch, &actual_per, &actual_buf) == 0 ||
                    try_configure_capture_pcm(fd, fmts[f], rates[r], chs[c], chs[c],
                                              64U, 2048U, 512U, 8192U,
                                              &actual_rate, &actual_ch, &actual_per, &actual_buf) == 0) {
                    chosen_fmt = fmts[f];
                }
            }
            if (chosen_fmt < 0) {
                uint32_t min_ch = 0;
                if (query_capture_min_channels(fd, fmts[f], rates[r], &min_ch) == 0 &&
                    (try_configure_capture_pcm(fd, fmts[f], rates[r], min_ch, min_ch,
                                               64U, 256U, 1024U, 8192U,
                                               &actual_rate, &actual_ch, &actual_per, &actual_buf) == 0 ||
                     try_configure_capture_pcm(fd, fmts[f], rates[r], min_ch, min_ch,
                                               64U, 2048U, 512U, 8192U,
                                               &actual_rate, &actual_ch, &actual_per, &actual_buf) == 0)) {
                    chosen_fmt = fmts[f];
                }
            }
        }
    }

    if (chosen_fmt < 0) {
        close(fd);
        return -1;
    }

    struct snd_pcm_sw_params sw;
    memset(&sw, 0, sizeof(sw));
    sw.tstamp_mode     = SNDRV_PCM_TSTAMP_ENABLE;
    sw.period_step     = 1;
    sw.avail_min       = actual_per ? actual_per : 256;
    sw.start_threshold = 1;
    sw.stop_threshold  = actual_buf ? actual_buf : 2048;
    sw.xfer_align      = 1;
    ioctl(fd, SNDRV_PCM_IOCTL_SW_PARAMS, &sw);

    if (ioctl(fd, SNDRV_PCM_IOCTL_PREPARE, 0) < 0) {
        close(fd);
        return -1;
    }

    *out_rate = actual_rate ? actual_rate : req_rate;
    *out_ch   = (actual_ch >= 1U && actual_ch <= LWASAPI_MAX_CAP_CHANNELS) ? actual_ch : 2U;
    *out_fmt  = chosen_fmt;
    return fd;
}

static inline float convert_alsa_sample_to_float(const uint8_t *raw, size_t idx, int fmt) {
    if (fmt == SNDRV_PCM_FORMAT_S32_LE) {
        int32_t v;
        memcpy(&v, raw + idx * 4, 4);
        return (float)v * (1.0f / 2147483648.0f);
    } else if (fmt == SNDRV_PCM_FORMAT_S24_3LE) {
        const uint8_t *p = raw + idx * 3;
        int32_t v = (int32_t)((uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16));
        if (v & 0x00800000) v |= (int32_t)0xFF000000U;
        return (float)v * (1.0f / 8388608.0f);
    } else {
        int16_t v;
        memcpy(&v, raw + idx * 2, 2);
        return (float)v * (1.0f / 32768.0f);
    }
}

/* ============================================================================
 * Direct Kernel ALSA Capture Thread (/dev/snd/pcmC*D*c)
 * ============================================================================ */

static void bt_init_pi_mutex(pthread_mutex_t *mtx) {
    pthread_mutexattr_t attr;
    pthread_mutexattr_init(&attr);
    pthread_mutexattr_setprotocol(&attr, PTHREAD_PRIO_INHERIT);
    pthread_mutex_init(mtx, &attr);
    pthread_mutexattr_destroy(&attr);
}

static void bt_enable_realtime(int prio, int fallback_nice) {
    struct sched_param sp;
    memset(&sp, 0, sizeof(sp));
    sp.sched_priority = prio;
    if (pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp) != 0) {
        setpriority(PRIO_PROCESS, 0, fallback_nice);
    }
}

static uint64_t cap_monotonic_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static uint32_t cap_snap_measured_rate(double actual_hz) {
    static const uint32_t std_rates[] = { 44100U, 48000U, 88200U, 96000U, 176400U, 192000U };
    for (size_t i = 0; i < sizeof(std_rates) / sizeof(std_rates[0]); i++) {
        double r = (double)std_rates[i];
        if (fabs(actual_hz - r) <= r * 0.035) {
            return std_rates[i];
        }
    }
    return (actual_hz >= 8000.0 && actual_hz <= 384000.0) ? (uint32_t)(actual_hz + 0.5) : 48000U;
}

/* Briefly stream 1 period at an alternate rate so snd-usb-audio clears cached bClockID state */
static int prime_usb_capture_clock(const char *dev_path, uint32_t target_rate) {
    if (!dev_path || !dev_path[0]) return -1;
    uint32_t alt_rate = (target_rate == 96000U) ? 48000U : 96000U;
    uint32_t got_rate = 0, got_ch = 0;
    int got_fmt = -1;
    int alt_fd = open_kernel_capture_pcm(dev_path, alt_rate, &got_rate, &got_ch, &got_fmt);
    if (alt_fd < 0 && alt_rate != 48000U) {
        alt_rate = 48000U;
        alt_fd = open_kernel_capture_pcm(dev_path, alt_rate, &got_rate, &got_ch, &got_fmt);
    }
    if (alt_fd < 0) return -1;

    uint32_t ch = (got_ch >= 1U && got_ch <= LWASAPI_MAX_CAP_CHANNELS) ? got_ch : 2U;
    uint8_t dummy[128 * LWASAPI_MAX_CAP_CHANNELS * 4];
    ioctl(alt_fd, SNDRV_PCM_IOCTL_START, 0);
    struct pollfd pfd = { .fd = alt_fd, .events = POLLIN | POLLERR | POLLHUP };
    if (poll(&pfd, 1, 100) > 0 && (pfd.revents & POLLIN)) {
        struct snd_xferi xf;
        memset(&xf, 0, sizeof(xf));
        xf.buf = dummy;
        xf.frames = (ch <= LWASAPI_MAX_CAP_CHANNELS) ? 64U : 16U;
        if (ioctl(alt_fd, SNDRV_PCM_IOCTL_READI_FRAMES, &xf) < 0) {}
    }
    close(alt_fd);
    usleep(25000);
    return 0;
}

static int kick_usb_capture_clock(lwasapi_bluez_t *bt, uint32_t target_rate) {
    if (!bt || !bt->cap_pcm_path[0]) return -1;
    if (bt->cap_pcm_fd >= 0) {
        close(bt->cap_pcm_fd);
        bt->cap_pcm_fd = -1;
    }
    prime_usb_capture_clock(bt->cap_pcm_path, target_rate);
    if (!bt->cap_thread_running) return -1;

    uint32_t new_rate = target_rate, new_ch = 2;
    int new_fmt = SNDRV_PCM_FORMAT_S32_LE;
    int fd = open_kernel_capture_pcm(bt->cap_pcm_path, target_rate, &new_rate, &new_ch, &new_fmt);
    if (fd < 0) return -1;

    bt->cap_pcm_fd = fd;
    __atomic_store_n(&bt->cap_hw_rate, new_rate, __ATOMIC_RELAXED);
    __atomic_store_n(&bt->cap_hw_channels, new_ch, __ATOMIC_RELAXED);
    __atomic_store_n(&bt->cap_hw_format, new_fmt, __ATOMIC_RELAXED);
    return 0;
}

static void *alsa_cap_thread_fn(void *arg) {
    lwasapi_bluez_t *bt = (lwasapi_bluez_t *)arg;
    bt_enable_realtime(62, -15);
    const uint32_t chunk = 128;
    uint8_t raw_buf[256 * LWASAPI_MAX_CAP_CHANNELS * 4];
    float   src_float[256 * 2];
    float   smooth_gain = 1.0f;

    uint32_t eff_in_rate_override = 0;
    uint32_t kick_attempts = 0;
    uint32_t mismatch_streak = 0;
    bool     rate_locked_fallback = false;
    uint64_t warmup_until_ns = cap_monotonic_ns() + 150000000ULL;
    uint64_t meas_start_ns = 0;
    uint64_t meas_frames = 0;
    uint32_t meas_ticks = 0;

    while (bt->cap_thread_running) {
        if (bt->cap_pcm_fd < 0) {
            meas_start_ns = 0;
            meas_frames = 0;
            mismatch_streak = 0;
            usleep(20000);
            continue;
        }

        struct snd_xferi xf;
        memset(&xf, 0, sizeof(xf));
        xf.buf    = raw_buf;
        xf.frames = chunk;

        if (ioctl(bt->cap_pcm_fd, SNDRV_PCM_IOCTL_READI_FRAMES, &xf) < 0) {
            if (errno == EPIPE || errno == ESTRPIPE) {
                ioctl(bt->cap_pcm_fd, SNDRV_PCM_IOCTL_PREPARE, 0);
                meas_start_ns = 0;
                meas_frames = 0;
                mismatch_streak = 0;
                continue;
            }
            if (errno == EINTR || errno == EAGAIN) {
                meas_start_ns = 0;
                meas_frames = 0;
                mismatch_streak = 0;
                usleep(500);
                continue;
            }
            /* Device unplugged (ENODEV) */
            close(bt->cap_pcm_fd);
            bt->cap_pcm_fd = -1;
            bt->has_capture_hw = false;
            snprintf(bt->cap_device_id, sizeof(bt->cap_device_id), "none");
            snprintf(bt->capture_device_alias, sizeof(bt->capture_device_alias), "Capture Disconnected");
            break;
        }

        uint32_t got = (uint32_t)xf.frames;
        if (got == 0) continue;
        if (got > chunk) got = chunk;

        uint32_t nominal_rate = __atomic_load_n(&bt->cap_hw_rate, __ATOMIC_RELAXED);
        if (!nominal_rate) nominal_rate = 48000U;

        meas_frames += got;
        if ((++meas_ticks & 15U) == 0U) {
            uint64_t now_ns = cap_monotonic_ns();
            if (now_ns >= warmup_until_ns) {
                if (meas_start_ns == 0) {
                    meas_start_ns = now_ns;
                    meas_frames = 0;
                } else if (now_ns - meas_start_ns >= 350000000ULL) {
                    uint64_t dt_ns = now_ns - meas_start_ns;
                    double actual_hz = (double)meas_frames * 1e9 / (double)dt_ns;
                    __atomic_store_n(&bt->cap_measured_rate, (uint32_t)(actual_hz + 0.5), __ATOMIC_RELAXED);
                    meas_start_ns = now_ns;
                    meas_frames = 0;
                    if (bt->cap_is_usb && !rate_locked_fallback) {
                        if (actual_hz < (double)nominal_rate * 0.78 ||
                            actual_hz > (double)nominal_rate * 1.25) {
                            mismatch_streak++;
                            if (mismatch_streak >= 2U) {
                                if (kick_attempts < 2U) {
                                    kick_attempts++;
                                    __atomic_fetch_add(&bt->cap_kick_count, 1U, __ATOMIC_RELAXED);
                                    fprintf(stderr,
                                            "[LWASAPI-Capture] Clock rate mismatch on %s (nominal=%uHz actual=%.0fHz) -> cycling UAC2 clock (attempt %u/2)\n",
                                            bt->cap_pcm_path, nominal_rate, actual_hz, kick_attempts);
                                    lwasapi_debug_emit(LWASAPI_DBG_ALSA, LWASAPI_DBG_WARN, 0, 0,
                                                       "ALSA_CAP_CLK_KICK", bt->cap_pcm_path,
                                                       "nominal=%uHz actual=%.0fHz attempt=%u",
                                                       nominal_rate, actual_hz, kick_attempts);
                                    if (kick_usb_capture_clock(bt, nominal_rate) == 0) {
                                        meas_start_ns = 0;
                                        meas_frames = 0;
                                        mismatch_streak = 0;
                                        warmup_until_ns = cap_monotonic_ns() + 150000000ULL;
                                        continue;
                                    }
                                }
                                uint32_t snapped = cap_snap_measured_rate(actual_hz);
                                eff_in_rate_override = snapped;
                                __atomic_store_n(&bt->cap_hw_rate, snapped, __ATOMIC_RELAXED);
                                rate_locked_fallback = true;
                                mismatch_streak = 0;
                                fprintf(stderr,
                                        "[LWASAPI-Capture] Using measured hardware clock %uHz (actual=%.0fHz) on %s for artifact-free capture\n",
                                        snapped, actual_hz, bt->cap_pcm_path);
                                lwasapi_debug_emit(LWASAPI_DBG_ALSA, LWASAPI_DBG_WARN, 0, 0,
                                                   "ALSA_CAP_CLK_FALLBACK", bt->cap_pcm_path,
                                                   "snapped=%uHz actual=%.0fHz", snapped, actual_hz);
                            }
                        } else {
                            mismatch_streak = 0;
                        }
                    }
                }
            } else {
                meas_frames = 0;
            }
        }

        bool mic_muted = __atomic_load_n(&bt->desktop_mic_muted, __ATOMIC_RELAXED);
        uint32_t ppm = atomic_load_explicit(&bt->desktop_mic_gain_ppm, memory_order_relaxed);
        float target_gain = mic_muted ? 0.0f : ((float)ppm * (1.0f / 1000000.0f));
        float g_step = (target_gain - smooth_gain) / (float)got;
        uint32_t ch = __atomic_load_n(&bt->cap_hw_channels, __ATOMIC_RELAXED);
        if (!ch) ch = 2;
        if (ch > LWASAPI_MAX_CAP_CHANNELS) ch = LWASAPI_MAX_CAP_CHANNELS;
        int cap_fmt = __atomic_load_n(&bt->cap_hw_format, __ATOMIC_RELAXED);
        for (uint32_t i = 0; i < got; i++) {
            smooth_gain += g_step;
            float l = convert_alsa_sample_to_float(raw_buf, i * ch, cap_fmt) * smooth_gain;
            float r = (ch >= 2) ? convert_alsa_sample_to_float(raw_buf, i * ch + 1, cap_fmt) * smooth_gain : l;
            if (l > 1.0f) l = 1.0f; else if (l < -1.0f) l = -1.0f;
            if (r > 1.0f) r = 1.0f; else if (r < -1.0f) r = -1.0f;
            src_float[i * 2 + 0] = l;
            src_float[i * 2 + 1] = r;
        }
        smooth_gain = target_gain;

        uint32_t in_rate = eff_in_rate_override ? eff_in_rate_override : nominal_rate;
        __atomic_store_n(&bt->cap_ring_rate, in_rate, __ATOMIC_RELEASE);
        uint64_t wr = atomic_load_explicit(&bt->cap_hw_write_frame, memory_order_relaxed);
        for (uint32_t i = 0; i < got; i++) {
            uint32_t idx = (uint32_t)((wr + i) & (LWASAPI_RING_FRAMES - 1));
            bt->cap_ring[idx * 2 + 0] = src_float[i * 2 + 0];
            bt->cap_ring[idx * 2 + 1] = src_float[i * 2 + 1];
        }
        wr += got;
        atomic_store_explicit(&bt->cap_hw_write_frame, wr, memory_order_release);
    }
    return NULL;
}

static void stop_capture_thread(lwasapi_bluez_t *bt) {
    bt->cap_thread_running = 0;
    if (bt->cap_thread) {
        pthread_join(bt->cap_thread, NULL);
        bt->cap_thread = 0;
    }
    if (bt->cap_pcm_fd >= 0) {
        close(bt->cap_pcm_fd);
        bt->cap_pcm_fd = -1;
    }
    bt->has_capture_hw = false;
}

#define MAX_ALSA_CARDS_SCAN 32

/** Per-card hardware Jack and HDMI/DP ELD probe state read via /dev/snd/controlC*. */
typedef struct {
    bool checked;
    int  hdmi_jack[64];         /* -1 = no jack ctl, 0 = unplugged, 1 = plugged */
    char hdmi_name[64][32];     /* ELD monitor product name when present */
    int  out_jack_count;
    int  out_jack_on;
    int  speaker_jack;          /* -1 = absent, 0 = off, 1 = on */
    int  headphone_jack;        /* -1 = absent, 0 = off, 1 = on */
    int  lineout_jack;          /* -1 = absent, 0 = off, 1 = on */
    int  in_jack_count;
    int  in_jack_on;
    int  in_jack_by_idx[16];    /* -1 = absent, 0 = off, 1 = on */
} alsa_card_jacks_t;

/** Snapshot of kernel ALSA PCM counts and Jack/ELD plug states across cards 0..31. */
typedef struct {
    alsa_card_jacks_t jacks[MAX_ALSA_CARDS_SCAN];
    int               pb_nonhdmi_cnt[MAX_ALSA_CARDS_SCAN];
    int               cap_cnt[MAX_ALSA_CARDS_SCAN];
    bool              has_card[MAX_ALSA_CARDS_SCAN];
    uint64_t          pcm_hash;
} alsa_topology_t;

static uint64_t fnv1a_step(uint64_t h, const void *data, size_t len) {
    const uint8_t *p = (const uint8_t *)data;
    for (size_t i = 0; i < len; i++) {
        h ^= (uint64_t)p[i];
        h *= 1099511628211ULL;
    }
    return h;
}

static bool card_eld_any_present(unsigned card, bool *out_has_eld_files) {
    if (out_has_eld_files) *out_has_eld_files = false;
    bool any_on = false;
    for (int k = 0; k < 8; k++) {
        char path[64];
        snprintf(path, sizeof(path), "/proc/asound/card%u/eld#0.%d", card, k);
        FILE *fp = fopen(path, "r");
        if (!fp) continue;
        if (out_has_eld_files) *out_has_eld_files = true;
        char line[128];
        while (fgets(line, sizeof(line), fp)) {
            if (strncmp(line, "monitor_present", 15) == 0) {
                const char *v = line + 15;
                while (*v == ' ' || *v == '\t') v++;
                if (*v == '1') any_on = true;
                break;
            }
        }
        fclose(fp);
    }
    return any_on;
}

static void probe_card_jacks(unsigned card, alsa_card_jacks_t *cj) {
    memset(cj, 0, sizeof(*cj));
    for (int i = 0; i < 64; i++) cj->hdmi_jack[i] = -1;
    cj->speaker_jack = cj->headphone_jack = cj->lineout_jack = -1;
    for (int i = 0; i < 16; i++) cj->in_jack_by_idx[i] = -1;
    if (card >= MAX_ALSA_CARDS_SCAN) return;

    char ctl_path[64];
    snprintf(ctl_path, sizeof(ctl_path), "/dev/snd/controlC%u", card);
    int fd = open(ctl_path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) return;
    cj->checked = true;

    struct snd_ctl_elem_list el;
    memset(&el, 0, sizeof(el));
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &el) < 0 || el.count == 0) {
        close(fd);
        return;
    }

    uint32_t cnt = el.count > 512U ? 512U : el.count;
    struct snd_ctl_elem_id *pids = (struct snd_ctl_elem_id *)calloc(cnt, sizeof(*pids));
    if (!pids) {
        close(fd);
        return;
    }
    el.offset = 0;
    el.space  = cnt;
    el.pids   = pids;
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &el) < 0) {
        free(pids);
        close(fd);
        return;
    }

    for (uint32_t i = 0; i < el.used; i++) {
        char nm[SNDRV_CTL_ELEM_ID_NAME_MAXLEN + 1];
        memcpy(nm, pids[i].name, SNDRV_CTL_ELEM_ID_NAME_MAXLEN);
        nm[SNDRV_CTL_ELEM_ID_NAME_MAXLEN] = '\0';

        if (pids[i].iface == SNDRV_CTL_ELEM_IFACE_PCM && strcmp(nm, "ELD") == 0 && pids[i].device < 64) {
            struct snd_ctl_elem_info info;
            memset(&info, 0, sizeof(info));
            info.id = pids[i];
            if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_INFO, &info) == 0 &&
                info.type == SNDRV_CTL_ELEM_TYPE_BYTES && info.count >= 20 && info.count <= 512) {
                struct snd_ctl_elem_value ev;
                memset(&ev, 0, sizeof(ev));
                ev.id = pids[i];
                if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_READ, &ev) == 0) {
                    uint32_t mnl = ev.value.bytes.data[4] & 0x1FU;
                    if (mnl > 0 && mnl <= 16 && 20U + mnl <= info.count) {
                        char mname[32];
                        uint32_t out_n = 0;
                        for (uint32_t k = 0; k < mnl && out_n + 1 < sizeof(mname); k++) {
                            unsigned char ch = ev.value.bytes.data[20 + k];
                            if (ch >= 0x20 && ch < 0x7f) mname[out_n++] = (char)ch;
                        }
                        while (out_n > 0 && mname[out_n - 1] == ' ') out_n--;
                        mname[out_n] = '\0';
                        snprintf(cj->hdmi_name[pids[i].device],
                                 sizeof(cj->hdmi_name[pids[i].device]), "%.31s", mname);
                    }
                }
            }
            continue;
        }

        if (!strstr(nm, "Jack")) continue;

        struct snd_ctl_elem_info info;
        memset(&info, 0, sizeof(info));
        info.id = pids[i];
        if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_INFO, &info) < 0 ||
            info.type != SNDRV_CTL_ELEM_TYPE_BOOLEAN) {
            continue;
        }

        struct snd_ctl_elem_value ev;
        memset(&ev, 0, sizeof(ev));
        ev.id = pids[i];
        if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_READ, &ev) < 0) continue;
        int on = (ev.value.integer.value[0] != 0) ? 1 : 0;

        const char *pcm_p = strstr(nm, "pcm=");
        if (pcm_p && (strstr(nm, "HDMI") || strstr(nm, "DP") || strstr(nm, "DisplayPort"))) {
            unsigned pdev = 0;
            if (sscanf(pcm_p, "pcm=%u", &pdev) == 1 && pdev < 64) {
                if (cj->hdmi_jack[pdev] < 0) cj->hdmi_jack[pdev] = on;
                else if (on) cj->hdmi_jack[pdev] = 1;
            }
            continue;
        }

        bool is_in = (strcasestr(nm, "Mic") != NULL ||
                      strcasestr(nm, "Input") != NULL ||
                      strcasestr(nm, "Line In") != NULL ||
                      strcasestr(nm, "Line-In") != NULL ||
                      strcasestr(nm, "Capture") != NULL ||
                      strcasestr(nm, "Aux") != NULL ||
                      strcasestr(nm, "CD") != NULL);
        if (is_in) {
            cj->in_jack_count++;
            if (on) cj->in_jack_on++;
            unsigned idx = pids[i].index;
            if (idx < 16) {
                if (cj->in_jack_by_idx[idx] < 0) cj->in_jack_by_idx[idx] = on;
                else if (on) cj->in_jack_by_idx[idx] = 1;
            }
        } else {
            cj->out_jack_count++;
            if (on) cj->out_jack_on++;
            if (strcasestr(nm, "Speaker")) {
                cj->speaker_jack = (cj->speaker_jack == 1 || on) ? 1 : 0;
            }
            if (strcasestr(nm, "Headphone")) {
                cj->headphone_jack = (cj->headphone_jack == 1 || on) ? 1 : 0;
            }
            if (strcasestr(nm, "Line Out") || strcasestr(nm, "Line-Out") || strcasestr(nm, "Front")) {
                cj->lineout_jack = (cj->lineout_jack == 1 || on) ? 1 : 0;
            }
        }
    }

    free(pids);
    close(fd);
}

static void collect_alsa_topology(alsa_topology_t *top) {
    memset(top, 0, sizeof(*top));
    top->pcm_hash = 14695981039346656037ULL;

    FILE *fp = fopen("/proc/asound/pcm", "r");
    if (fp) {
        char line[256];
        while (fgets(line, sizeof(line), fp)) {
            top->pcm_hash = fnv1a_step(top->pcm_hash, line, strlen(line));
            unsigned c = 0, d = 0;
            char pcm_name[64] = {0};
            if (sscanf(line, "%u-%u: %63[^:]", &c, &d, pcm_name) < 2 || c >= MAX_ALSA_CARDS_SCAN) {
                continue;
            }
            top->has_card[c] = true;
            if (strstr(line, "playback") &&
                !strcasestr(pcm_name, "HDMI") && !strcasestr(pcm_name, "DisplayPort")) {
                top->pb_nonhdmi_cnt[c]++;
            }
            if (strstr(line, "capture")) {
                top->cap_cnt[c]++;
            }
        }
        fclose(fp);
    }

    for (unsigned c = 0; c < MAX_ALSA_CARDS_SCAN; c++) {
        if (top->has_card[c]) probe_card_jacks(c, &top->jacks[c]);
    }
}

static bool is_alsa_pcm_plugged(const alsa_topology_t *top, const int card_is_usb[MAX_ALSA_CARDS_SCAN],
                                unsigned c, unsigned d, bool is_capture, const char *pcm_name) {
    if (!top || c >= MAX_ALSA_CARDS_SCAN) return true;
    const alsa_card_jacks_t *cj = &top->jacks[c];

    if (!is_capture) {
        bool is_hdmi = (d < 64 && (cj->hdmi_jack[d] != -1 ||
                                    strcasestr(pcm_name, "HDMI") != NULL ||
                                    strcasestr(pcm_name, "DisplayPort") != NULL));
        if (is_hdmi) {
            if (d < 64 && cj->hdmi_jack[d] != -1) return cj->hdmi_jack[d] != 0;
            bool has_eld_files = false;
            bool eld_on = card_eld_any_present(c, &has_eld_files);
            return has_eld_files ? eld_on : true;
        }

        if (!cj->checked || cj->out_jack_count == 0) return true;
        if (cj->out_jack_on == 0) return false;

        /* Multi-PCM USB motherboard codecs (e.g. ALC4080) split Speaker=pcm0 and Headphone=pcm1. */
        if (card_is_usb && card_is_usb[c] && top->pb_nonhdmi_cnt[c] >= 2 &&
            cj->speaker_jack != -1 && cj->headphone_jack != -1) {
            if (d == 0) return (cj->speaker_jack != 0 || cj->lineout_jack == 1);
            if (d == 1) return (cj->headphone_jack != 0);
            return false;
        }
        return true;
    }

    if (!cj->checked || cj->in_jack_count == 0) return true;
    if (cj->in_jack_on == 0) return false;

    if (top->cap_cnt[c] > 1 && d < 16 && cj->in_jack_by_idx[d] != -1) {
        return cj->in_jack_by_idx[d] != 0;
    }
    if (top->cap_cnt[c] > 1 && cj->in_jack_by_idx[0] != -1 && d >= 2) {
        return false;
    }
    return true;
}

uint64_t lwasapi_bluez_hw_fingerprint(const lwasapi_bluez_t *bt) {
    alsa_topology_t top;
    collect_alsa_topology(&top);
    uint64_t h = top.pcm_hash;
    for (unsigned c = 0; c < MAX_ALSA_CARDS_SCAN; c++) {
        if (!top.has_card[c]) continue;
        const alsa_card_jacks_t *cj = &top.jacks[c];
        h = fnv1a_step(h, &c, sizeof(c));
        h = fnv1a_step(h, cj->hdmi_jack, sizeof(cj->hdmi_jack));
        h = fnv1a_step(h, cj->hdmi_name, sizeof(cj->hdmi_name));
        h = fnv1a_step(h, &cj->out_jack_count, sizeof(cj->out_jack_count));
        h = fnv1a_step(h, &cj->out_jack_on, sizeof(cj->out_jack_on));
        h = fnv1a_step(h, &cj->speaker_jack, sizeof(cj->speaker_jack));
        h = fnv1a_step(h, &cj->headphone_jack, sizeof(cj->headphone_jack));
        h = fnv1a_step(h, &cj->lineout_jack, sizeof(cj->lineout_jack));
        h = fnv1a_step(h, &cj->in_jack_count, sizeof(cj->in_jack_count));
        h = fnv1a_step(h, &cj->in_jack_on, sizeof(cj->in_jack_on));
        h = fnv1a_step(h, cj->in_jack_by_idx, sizeof(cj->in_jack_by_idx));
    }
    if (bt) {
        pthread_mutex_lock((pthread_mutex_t *)&bt->state_lock);
        bool bt_live = (bt->has_bluez_transport && bt->direct_socket_mode && bt->bt_fd >= 0);
        uint8_t flags[4] = {
            bt_live ? 1 : 0,
            bt->tx_stream_active ? 1 : 0,
            bt->has_capture_hw ? 1 : 0,
            (uint8_t)bt->codec_id
        };
        char mac[32], cap_id[64];
        snprintf(mac, sizeof(mac), "%s", bt->mac_address);
        snprintf(cap_id, sizeof(cap_id), "%s", bt->cap_device_id);
        pthread_mutex_unlock((pthread_mutex_t *)&bt->state_lock);
        h = fnv1a_step(h, flags, sizeof(flags));
        if (bt_live) h = fnv1a_step(h, mac, strlen(mac));
        h = fnv1a_step(h, cap_id, strlen(cap_id));
    }
    return h;
}

static int  bt_open_sco_capture(lwasapi_bluez_t *bt, const char *mac_or_empty);
static void bt_stop_sco_capture(lwasapi_bluez_t *bt);

int lwasapi_bluez_open_capture(lwasapi_bluez_t *bt, const char *preferred_cap_id, uint32_t capture_sample_rate) {
    if (!bt) return -1;
    stop_capture_thread(bt);
    bt_stop_sco_capture(bt);
    uint32_t eng_rate = __atomic_load_n(&bt->engine_sample_rate, __ATOMIC_RELAXED);
    if (!eng_rate) {
        eng_rate = capture_sample_rate ? capture_sample_rate : 48000U;
        __atomic_store_n(&bt->engine_sample_rate, eng_rate, __ATOMIC_RELEASE);
    }
    uint32_t req_cap_rate = capture_sample_rate ? capture_sample_rate : eng_rate;

    if (preferred_cap_id && strncmp(preferred_cap_id, "bluez_sco:", 10) == 0 && bt->hfp_sco_enabled) {
        if (bt_open_sco_capture(bt, preferred_cap_id + 10) == 0) {
            return 0;
        }
    }

    char card_names[MAX_ALSA_CARDS_SCAN][64];
    int  card_is_usb[MAX_ALSA_CARDS_SCAN];
    parse_asound_cards(card_names, card_is_usb);

    alsa_topology_t top;
    collect_alsa_topology(&top);

    typedef struct {
        int card, dev, score;
        char id[64], path[64], alias[64];
    } cap_cand_t;

    cap_cand_t cands[32];
    int n_cands = 0;

    FILE *fp = fopen("/proc/asound/pcm", "r");
    if (fp) {
        char line[256];
        while (fgets(line, sizeof(line), fp) && n_cands < 32) {
            if (!strstr(line, "capture")) continue;
            unsigned c = 0, d = 0;
            char pcm_name[64] = {0};
            if (sscanf(line, "%u-%u: %63[^:]", &c, &d, pcm_name) < 2) continue;
            if (c >= MAX_ALSA_CARDS_SCAN) continue;
            char dev_path[64];
            snprintf(dev_path, sizeof(dev_path), "/dev/snd/pcmC%uD%uc", c, d);
            if (access(dev_path, F_OK) != 0) continue;
            if (!is_alsa_pcm_plugged(&top, card_is_usb, c, d, true, pcm_name)) continue;

            cap_cand_t *cc = &cands[n_cands++];
            cc->card = (int)c;
            cc->dev  = (int)d;
            char id_tmp[32];
            snprintf(id_tmp, sizeof(id_tmp), "hw:%u,%u", c, d);
            snprintf(cc->id, sizeof(cc->id), "%s", id_tmp);
            snprintf(cc->path, sizeof(cc->path), "%s", dev_path);
            if (card_names[c][0]) snprintf(cc->alias, sizeof(cc->alias), "%s", card_names[c]);
            else snprintf(cc->alias, sizeof(cc->alias), "%.40s (%.19s)", pcm_name, id_tmp);

            int sc = 10;
            if (preferred_cap_id && strcmp(preferred_cap_id, "auto") != 0 && strcmp(preferred_cap_id, cc->id) == 0) {
                sc = 10000;
            } else {
                if (card_is_usb[c]) sc += 200;
                if (strcasestr(cc->alias, "Scarlett") || strcasestr(cc->alias, "Blue") ||
                    strcasestr(cc->alias, "Shure") || strcasestr(cc->alias, "Rode") ||
                    strcasestr(cc->alias, "Audient")) sc += 300;
                if (d == 0) sc += 20;
                sc -= (int)(c + d);
            }
            cc->score = sc;
        }
        fclose(fp);
    }

    for (int i = 0; i < n_cands; i++) {
        for (int j = i + 1; j < n_cands; j++) {
            if (cands[j].score > cands[i].score) {
                cap_cand_t tmp = cands[i];
                cands[i] = cands[j];
                cands[j] = tmp;
            }
        }
    }

    for (int i = 0; i < n_cands; i++) {
        bool is_usb = (card_is_usb[cands[i].card] != 0);
        if (is_usb && req_cap_rate > 48000U) {
            prime_usb_capture_clock(cands[i].path, req_cap_rate);
        }
        uint32_t actual_rate = 48000, actual_ch = 2;
        int actual_fmt = SNDRV_PCM_FORMAT_S32_LE;
        int fd = open_kernel_capture_pcm(cands[i].path, req_cap_rate, &actual_rate, &actual_ch, &actual_fmt);
        if (fd >= 0) {
            bt->cap_pcm_fd = fd;
            snprintf(bt->cap_device_id, sizeof(bt->cap_device_id), "%s", cands[i].id);
            snprintf(bt->cap_pcm_path, sizeof(bt->cap_pcm_path), "%s", cands[i].path);
            snprintf(bt->capture_device_alias, sizeof(bt->capture_device_alias), "%s", cands[i].alias);
            bt->cap_hw_rate = actual_rate;
            bt->cap_hw_channels = actual_ch;
            bt->cap_hw_format = actual_fmt;
            bt->cap_is_usb = is_usb;
            __atomic_store_n(&bt->cap_ring_rate, actual_rate, __ATOMIC_RELEASE);
            __atomic_store_n(&bt->cap_measured_rate, 0U, __ATOMIC_RELAXED);
            __atomic_store_n(&bt->cap_kick_count, 0U, __ATOMIC_RELAXED);
            bt->has_capture_hw = true;
            atomic_store(&bt->cap_hw_write_frame, 0);
            bt->cap_thread_running = 1;
            pthread_create(&bt->cap_thread, NULL, alsa_cap_thread_fn, bt);
            fprintf(stderr, "[LWASAPI-Capture] Direct Kernel ALSA Capture active: %s [%s @ %uHz %uch]\n",
                    bt->capture_device_alias, bt->cap_pcm_path, actual_rate, actual_ch);
            lwasapi_debug_emit(LWASAPI_DBG_ALSA, LWASAPI_DBG_INFO, 0, 0,
                               "ALSA_CAP_OPEN", bt->cap_pcm_path,
                               "%s (%s) @ %uHz %uch fmt=%d",
                               bt->capture_device_alias, bt->cap_device_id,
                               actual_rate, actual_ch, actual_fmt);
            return 0;
        }
    }

    snprintf(bt->cap_device_id, sizeof(bt->cap_device_id), "none");
    snprintf(bt->capture_device_alias, sizeof(bt->capture_device_alias), "No Hardware Mic");
    lwasapi_debug_emit(LWASAPI_DBG_ALSA, LWASAPI_DBG_WARN, 0, 0,
                       "ALSA_CAP_NONE", preferred_cap_id ? preferred_cap_id : "auto",
                       "No plugged ALSA capture PCM available (%d candidates tried)", n_cands);
    return -1;
}

/* ============================================================================
 * A2DP Codec Initialization & Negotiation (MPEG-2,4 AAC LATM + SBC)
 * ============================================================================ */

static uint32_t ldac_eqmid_to_kbps(uint32_t sf, int eqmid) {
    bool is_44k_fam = (sf == 44100 || sf == 88200);
    if (eqmid == LDACBT_EQMID_HQ) return is_44k_fam ? 909U : 990U;
    if (eqmid == LDACBT_EQMID_SQ) return is_44k_fam ? 606U : 660U;
    return is_44k_fam ? 303U : 330U;
}

static void close_bt_encoders(lwasapi_bluez_t *bt) {
    if (bt->ldac_abr && g_ldac_abr.free_handle) {
        g_ldac_abr.free_handle(bt->ldac_abr);
        bt->ldac_abr = NULL;
    }
    if (bt->fdk_enc && g_fdk.close) {
        g_fdk.close(&bt->fdk_enc);
        bt->fdk_enc = NULL;
    }
    if (bt->ldac_enc && g_ldac.free_handle) {
        g_ldac.free_handle(bt->ldac_enc);
        bt->ldac_enc = NULL;
    }
    if (bt->aptx_enc && g_aptx.finish) {
        g_aptx.finish(bt->aptx_enc);
        bt->aptx_enc = NULL;
    }
    if (bt->sbc_enc_init) {
        sbc_finish(&bt->sbc_enc);
        bt->sbc_enc_init = false;
    }
}

static int init_bt_encoder_for_transport(lwasapi_bluez_t *bt) {
    close_bt_encoders(bt);
    bt->tx_congest_streak = 0;
    bt->tx_clean_streak = 0;
    bt->abr_last_ms = 0;

    if (bt->active_codec_kind == LWASAPI_BT_CODEC_LDAC) {
        if (!load_ldac_library() || bt->a2dp_config_len < sizeof(a2dp_ldac_t)) return -1;
        const a2dp_ldac_t *cfg = (const a2dp_ldac_t *)bt->a2dp_config;
        uint32_t sf = 48000;
        if (cfg->frequency & LDAC_SAMPLING_FREQ_96000) sf = 96000;
        else if (cfg->frequency & LDAC_SAMPLING_FREQ_88200) sf = 88200;
        else if (cfg->frequency & LDAC_SAMPLING_FREQ_48000) sf = 48000;
        else if (cfg->frequency & LDAC_SAMPLING_FREQ_44100) sf = 44100;

        int cm = LDAC_CHANNEL_MODE_STEREO;
        uint32_t ch = 2;
        if (cfg->channel_mode & LDAC_CHANNEL_MODE_STEREO) {
            cm = LDAC_CHANNEL_MODE_STEREO;
            ch = 2;
        } else if (cfg->channel_mode & LDAC_CHANNEL_MODE_DUAL) {
            cm = LDAC_CHANNEL_MODE_DUAL;
            ch = 2;
        } else if (cfg->channel_mode & LDAC_CHANNEL_MODE_MONO) {
            cm = LDAC_CHANNEL_MODE_MONO;
            ch = 1;
        }

        uint16_t mtu = bt->write_mtu ? bt->write_mtu : 679;
        uint16_t payload_mtu = (mtu > (uint16_t)(sizeof(rtp_header_t) + sizeof(rtp_sbc_payload_t) + 64U))
                             ? (uint16_t)(mtu - sizeof(rtp_header_t) - sizeof(rtp_sbc_payload_t))
                             : 660U;
        if (payload_mtu > 1024U) payload_mtu = 1024U;
        int eqmid = (payload_mtu >= 660U) ? LDACBT_EQMID_HQ : LDACBT_EQMID_SQ;
        if (bt->ldac_mode == LWASAPI_LDAC_MODE_990) eqmid = LDACBT_EQMID_HQ;
        else if (bt->ldac_mode == LWASAPI_LDAC_MODE_660) eqmid = LDACBT_EQMID_SQ;
        else if (bt->ldac_mode == LWASAPI_LDAC_MODE_330) eqmid = LDACBT_EQMID_MQ;

        void *lh = g_ldac.get_handle();
        if (!lh) return -1;
        if (g_ldac.init_encode(lh, (int)payload_mtu, eqmid, cm, LDACBT_SMPL_FMT_S16, (int)sf) < 0) {
            g_ldac.free_handle(lh);
            return -1;
        }
        bt->ldac_enc = lh;
        bt->ldac_eqmid = eqmid;
        bt->bt_sample_rate = sf;
        bt->bt_channels = ch;
        bt->effective_kbps = ldac_eqmid_to_kbps(sf, eqmid);
        if (bt->ldac_mode == LWASAPI_LDAC_MODE_ADAPTIVE && bt->wireless_abr_enabled &&
            g_ldac_abr.available && g_ldac_abr.get_handle) {
            void *abr = g_ldac_abr.get_handle();
            if (abr) {
                g_ldac_abr.init(abr, 20);
                g_ldac_abr.set_thresholds(abr, 5, 3, 1);
                bt->ldac_abr = abr;
            }
        }
        snprintf(bt->codec_name, sizeof(bt->codec_name), "LDAC");
        return 0;
    }

    if (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX_HD ||
        bt->active_codec_kind == LWASAPI_BT_CODEC_APTX) {
        bool is_hd = (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX_HD);
        size_t need_len = is_hd ? sizeof(a2dp_aptx_hd_t) : sizeof(a2dp_aptx_t);
        if (!load_aptx_library() || bt->a2dp_config_len < need_len) return -1;
        const a2dp_aptx_t *cfg = (const a2dp_aptx_t *)bt->a2dp_config;
        uint32_t sf = 48000;
        if (cfg->frequency & APTX_SAMPLING_FREQ_48000) sf = 48000;
        else if (cfg->frequency & APTX_SAMPLING_FREQ_44100) sf = 44100;
        else if (cfg->frequency & APTX_SAMPLING_FREQ_32000) sf = 32000;
        else if (cfg->frequency & APTX_SAMPLING_FREQ_16000) sf = 16000;

        void *ah = g_aptx.init(is_hd ? 1 : 0);
        if (!ah) return -1;
        bt->aptx_enc = ah;
        bt->bt_sample_rate = sf;
        bt->bt_channels = 2;
        bt->effective_kbps = is_hd ? 576U : 352U;
        snprintf(bt->codec_name, sizeof(bt->codec_name), "%s", is_hd ? "aptX HD" : "aptX");
        return 0;
    }

    if (bt->codec_id == A2DP_CODEC_MPEG24 || bt->active_codec_kind == LWASAPI_BT_CODEC_AAC) {
        if (!load_fdk_aac_library()) return -1;
        const a2dp_aac_t *cfg = (const a2dp_aac_t *)bt->a2dp_config;
        uint16_t freq_mask = ((uint16_t)cfg->frequency1 << 4) | (cfg->frequency2 & 0x0F);
        bt->bt_sample_rate = (freq_mask & AAC_SAMPLING_FREQ_48000) ? 48000 : 44100;
        bt->bt_channels    = (cfg->channels & AAC_CHANNELS_2) ? 2 : 1;

        uint32_t remote_bps = ((uint32_t)cfg->bitrate1 << 16) |
                              ((uint32_t)cfg->bitrate2 << 8)  |
                              (uint32_t)cfg->bitrate3;
        if (remote_bps == 0) remote_bps = 192000;

        uint16_t mtu = bt->write_mtu ? bt->write_mtu : 672;
        uint32_t max_bps = (mtu > sizeof(rtp_header_t))
                           ? (((uint32_t)(mtu - sizeof(rtp_header_t)) * 8U * bt->bt_sample_rate) / 1024U)
                           : 192000;
        uint32_t target_bps = remote_bps < max_bps ? remote_bps : max_bps;
        if (target_bps > 256000) target_bps = 256000;
        if (target_bps < 64000) target_bps = 64000;

        void *enc = NULL;
        if (g_fdk.open(&enc, 0, bt->bt_channels) != FDK_AACENC_OK) return -1;

        int aot = (cfg->object_type & AAC_OBJECT_TYPE_MPEG4_AAC_LC)
                  ? FDK_AOT_AAC_LC : FDK_AOT_MP2_AAC_LC;
        if ((cfg->object_type & AAC_OBJECT_TYPE_MPEG2_AAC_LC) &&
            !(cfg->object_type & AAC_OBJECT_TYPE_MPEG4_AAC_LC)) {
            aot = FDK_AOT_MP2_AAC_LC;
        }

        g_fdk.set_param(enc, FDK_PARAM_AOT, (unsigned int)aot);
        g_fdk.set_param(enc, FDK_PARAM_SAMPLERATE, bt->bt_sample_rate);
        g_fdk.set_param(enc, FDK_PARAM_CHANNELMODE, bt->bt_channels);
        if (cfg->vbr) g_fdk.set_param(enc, FDK_PARAM_BITRATEMODE, 5);
        g_fdk.set_param(enc, FDK_PARAM_AUDIOMUXVER, 2);
        g_fdk.set_param(enc, FDK_PARAM_SIGNALING_MODE, 1);
        g_fdk.set_param(enc, FDK_PARAM_BITRATE, target_bps);
        g_fdk.set_param(enc, FDK_PARAM_PEAK_BITRATE, max_bps);
        if (!cfg->vbr && target_bps > 110000) {
            g_fdk.set_param(enc, FDK_PARAM_BANDWIDTH, 19293);
        }
        g_fdk.set_param(enc, FDK_PARAM_TRANSMUX, FDK_TT_MP4_LATM_MCP1);
        g_fdk.set_param(enc, FDK_PARAM_HEADER_PERIOD, 1);
        g_fdk.set_param(enc, FDK_PARAM_AFTERBURNER, 1);

        if (g_fdk.encode(enc, NULL, NULL, NULL, NULL) != FDK_AACENC_OK) {
            g_fdk.close(&enc);
            return -1;
        }

        fdk_info_t info;
        memset(&info, 0, sizeof(info));
        if (g_fdk.info(enc, &info) != FDK_AACENC_OK) {
            g_fdk.close(&enc);
            return -1;
        }
        bt->fdk_enc = enc;
        bt->fdk_frame_length = info.frameLength ? info.frameLength : 1024;
        bt->aac_max_bps = target_bps;
        bt->aac_target_bps = target_bps;
        bt->effective_kbps = target_bps / 1000U;
        snprintf(bt->codec_name, sizeof(bt->codec_name), "AAC");
        return 0;
    }

    /* Fallback / baseline SBC encoder via libsbc */
    a2dp_sbc_t scfg;
    if (bt->a2dp_config_len >= sizeof(a2dp_sbc_t)) {
        memcpy(&scfg, bt->a2dp_config, sizeof(a2dp_sbc_t));
    } else {
        scfg.frequency = SBC_SAMPLING_FREQ_48000;
        scfg.channel_mode = SBC_CHANNEL_MODE_JOINT_STEREO;
        scfg.block_length = SBC_BLOCK_LENGTH_16;
        scfg.subbands = SBC_SUBBANDS_8;
        scfg.allocation_method = SBC_ALLOCATION_LOUDNESS;
        scfg.min_bitpool = 2;
        scfg.max_bitpool = 53;
    }

    sbc_init_a2dp(&bt->sbc_enc, 0, &scfg, sizeof(scfg));
    bt->sbc_enc.endian = SBC_LE;
    bt->sbc_enc_init = true;

    bt->bt_sample_rate = (scfg.frequency == SBC_SAMPLING_FREQ_44100) ? 44100 :
                         (scfg.frequency == SBC_SAMPLING_FREQ_32000) ? 32000 :
                         (scfg.frequency == SBC_SAMPLING_FREQ_16000) ? 16000 : 48000;
    bt->bt_channels = (scfg.channel_mode == SBC_CHANNEL_MODE_MONO) ? 1 : 2;
    bt->effective_kbps = (scfg.max_bitpool >= 50) ? 328U : 229U;
    snprintf(bt->codec_name, sizeof(bt->codec_name), "SBC");
    return 0;
}

/* ============================================================================
 * Paced A2DP Bluetooth TX Thread (RTP + FDK-AAC LATM / SBC)
 * ============================================================================ */

static inline int16_t float_to_s16_clamped(float x) {
    if (x > 1.0f) x = 1.0f;
    if (x < -1.0f) x = -1.0f;
    return (int16_t)(x * 32767.0f);
}

static size_t pop_bt_resampled_s16(lwasapi_bluez_t *bt, int16_t *out_s16,
                                   uint32_t needed_frames, uint32_t bt_ch,
                                   double *io_frac, double drift_scale, uint32_t *out_avail_after) {
    uint64_t wr = atomic_load_explicit(&bt->tx_write_frame, memory_order_acquire);
    uint64_t rd = atomic_load_explicit(&bt->tx_read_frame, memory_order_relaxed);
    uint64_t avail = (wr >= rd) ? (wr - rd) : 0;

    uint32_t in_rate  = __atomic_load_n(&bt->engine_sample_rate, __ATOMIC_RELAXED);
    if (!in_rate) in_rate = 48000;
    uint32_t out_rate = bt->bt_sample_rate ? bt->bt_sample_rate : 48000;
    double   step     = ((double)in_rate / (double)out_rate) * drift_scale;

    if (avail > LWASAPI_BT_RING_FRAMES - 64) {
        uint64_t need_est  = (uint64_t)ceil(*io_frac + (double)needed_frames * step);
        uint64_t margin_in = ((uint64_t)in_rate * 20ULL) / 1000ULL;
        uint64_t keep = need_est + margin_in;
        if (keep > LWASAPI_BT_RING_FRAMES - 128U) keep = LWASAPI_BT_RING_FRAMES - 128U;
        rd = wr - keep;
        avail = keep;
        *io_frac = 0.0;
    }

    /* Determine software trim: if AVRCP handles 1..100% cleanly on headphone DAC, stay at unity 1.0f;
     * apply software zero on mute/0% or software scaling if boost >100% or no AVRCP support. */
    float sw_gain = 1.0f;
    bool muted = __atomic_load_n(&bt->desktop_muted, __ATOMIC_RELAXED);
    uint32_t vol_pct = __atomic_load_n(&bt->desktop_volume_pct, __ATOMIC_RELAXED);
    if (muted || vol_pct == 0) {
        sw_gain = 0.0f;
    } else if (!atomic_load(&g_avrcp_supported) || vol_pct > 100) {
        sw_gain = (float)vol_pct / 100.0f;
    }

    if (avail == 0) {
        memset(out_s16, 0, (size_t)needed_frames * bt_ch * sizeof(int16_t));
        if (out_avail_after) *out_avail_after = 0;
        return 0;
    }

    uint64_t need_in_min = (uint64_t)ceil(*io_frac + (double)needed_frames * step);
    bool underrun = (avail < need_in_min);
    uint32_t valid_out = needed_frames;
    if (underrun) {
        double est = ((double)avail - *io_frac) / (step > 0.0 ? step : 1.0);
        valid_out = (est > 0.0) ? (uint32_t)est : 0;
        if (valid_out > needed_frames) valid_out = needed_frames;
    }

    for (uint32_t i = 0; i < needed_frames; i++) {
        float l = 0.0f, r = 0.0f;
        if (i < valid_out) {
            uint64_t f0 = rd + (uint64_t)(*io_frac);
            if (f0 + 1 < wr) {
                double frac = *io_frac - floor(*io_frac);
                uint32_t p0 = (uint32_t)(f0 & (LWASAPI_BT_RING_FRAMES - 1));
                uint32_t p1 = (uint32_t)((f0 + 1) & (LWASAPI_BT_RING_FRAMES - 1));
                l = (float)(bt->tx_ring[p0 * 2 + 0] * (1.0 - frac) + bt->tx_ring[p1 * 2 + 0] * frac);
                r = (float)(bt->tx_ring[p0 * 2 + 1] * (1.0 - frac) + bt->tx_ring[p1 * 2 + 1] * frac);
            } else if (f0 < wr) {
                uint32_t p0 = (uint32_t)(f0 & (LWASAPI_BT_RING_FRAMES - 1));
                l = bt->tx_ring[p0 * 2 + 0];
                r = bt->tx_ring[p0 * 2 + 1];
            }
            if (underrun && valid_out > 0) {
                uint32_t fade_len = (valid_out < 32U) ? valid_out : 32U;
                uint32_t fade_start = valid_out - fade_len;
                if (i >= fade_start) {
                    float g = (float)(valid_out - i) / (float)fade_len;
                    l *= g;
                    r *= g;
                }
            }
            *io_frac += step;
        }
        l *= sw_gain;
        r *= sw_gain;
        if (bt_ch == 1) {
            out_s16[i] = float_to_s16_clamped(0.5f * (l + r));
        } else {
            out_s16[i * 2 + 0] = float_to_s16_clamped(l);
            out_s16[i * 2 + 1] = float_to_s16_clamped(r);
        }
    }

    if (underrun) {
        rd = wr;
        *io_frac = 0.0;
    } else {
        uint64_t consumed = (uint64_t)floor(*io_frac);
        if (rd + consumed <= wr) {
            rd += consumed;
            *io_frac -= (double)consumed;
        } else {
            rd = wr;
            *io_frac = 0.0;
        }
    }
    atomic_store_explicit(&bt->tx_read_frame, rd, memory_order_release);
    if (out_avail_after) *out_avail_after = (wr >= rd) ? (uint32_t)(wr - rd) : 0;
    return valid_out;
}

static void *bt_tx_thread_fn(void *arg) {
    lwasapi_bluez_t *bt = (lwasapi_bluez_t *)arg;
    bt_enable_realtime(58, -14);

    uint8_t  pkt[2048];
    int16_t  pcm[4096];
    double   frac_pos = 0.0;
    uint64_t next_tick_ns = bt_monotonic_ns();
    bool     prebuffering = true;
    uint32_t prebuf_waits = 0;
    double   avg_avail = 960.0;

    while (__atomic_load_n(&bt->tx_thread_running, __ATOMIC_RELAXED)) {
        pthread_mutex_lock(&bt->state_lock);
        bool     active   = (bt->tx_stream_active && bt->bt_fd >= 0 && bt->has_bluez_transport);
        uint32_t in_rate  = __atomic_load_n(&bt->engine_sample_rate, __ATOMIC_RELAXED);
        if (!in_rate) in_rate = 48000;
        uint32_t rate     = bt->bt_sample_rate ? bt->bt_sample_rate : 48000;
        uint32_t ch       = bt->bt_channels ? bt->bt_channels : 2;
        uint32_t mtu      = bt->write_mtu ? bt->write_mtu : 672;
        uint8_t  codec_id = bt->codec_id;
        uint8_t  codec_kind = bt->active_codec_kind;
        uint32_t fdk_flen = bt->fdk_frame_length ? bt->fdk_frame_length : 1024U;
        pthread_mutex_unlock(&bt->state_lock);

        uint64_t margin_in = ((uint64_t)in_rate * 20ULL) / 1000ULL;

        if (atomic_load_explicit(&bt->tx_paused, memory_order_relaxed) || !active) {
            prebuffering = true;
            prebuf_waits = 0;
            frac_pos = 0.0;
            avg_avail = (double)margin_in;
            next_tick_ns = bt_monotonic_ns();
            usleep(10000);
            continue;
        }

        if (mtu > sizeof(pkt)) mtu = sizeof(pkt);

        uint32_t est_need_out = (codec_kind == LWASAPI_BT_CODEC_LDAC) ? LDACBT_ENC_LSU :
                                (codec_kind == LWASAPI_BT_CODEC_APTX_HD || codec_kind == LWASAPI_BT_CODEC_APTX) ? 256U :
                                (codec_id == A2DP_CODEC_MPEG24 || codec_kind == LWASAPI_BT_CODEC_AAC) ? fdk_flen : 512U;
        uint64_t est_need_in = (uint64_t)ceil((double)est_need_out * ((double)in_rate / (double)rate)) + 4ULL;

        {
            uint64_t wr = atomic_load_explicit(&bt->tx_write_frame, memory_order_acquire);
            uint64_t rd = atomic_load_explicit(&bt->tx_read_frame, memory_order_relaxed);
            uint64_t avail = (wr >= rd) ? (wr - rd) : 0;

            if (prebuffering) {
                uint64_t goal = est_need_in + (margin_in * 3ULL) / 4ULL;
                if (goal > LWASAPI_BT_RING_FRAMES - 1024U) goal = LWASAPI_BT_RING_FRAMES - 1024U;
                if (avail < goal && prebuf_waits < 40) {
                    prebuf_waits++;
                    usleep(1500);
                    continue;
                }
                prebuffering = false;
                prebuf_waits = 0;
                avg_avail = (avail > est_need_in) ? (double)(avail - est_need_in) : (double)(margin_in / 2ULL);
                next_tick_ns = bt_monotonic_ns();
            } else if (avail > 0 && avail < est_need_in) {
                for (int w = 0; w < 5 && __atomic_load_n(&bt->tx_thread_running, __ATOMIC_RELAXED); w++) {
                    usleep(1200);
                    wr = atomic_load_explicit(&bt->tx_write_frame, memory_order_acquire);
                    rd = atomic_load_explicit(&bt->tx_read_frame, memory_order_relaxed);
                    avail = (wr >= rd) ? (wr - rd) : 0;
                    if (avail >= est_need_in) break;
                }
            }
        }

        double drift_scale = 1.0;
        if (avg_avail > (double)margin_in * 1.50) drift_scale = 1.001;
        else if (avg_avail < (double)margin_in * 0.45) drift_scale = 0.999;

        pthread_mutex_lock(&bt->state_lock);
        if (bt->bt_fd < 0 || !bt->tx_stream_active) {
            pthread_mutex_unlock(&bt->state_lock);
            continue;
        }

        size_t pkt_len = 0;
        uint32_t ts_step = 0;
        uint32_t expected_pop = 0;
        size_t popped = 0;
        uint32_t rem_avail = 0;

        if (bt->active_codec_kind == LWASAPI_BT_CODEC_LDAC && bt->ldac_enc) {
            uint32_t flen = LDACBT_ENC_LSU;
            size_t hdr_off = sizeof(rtp_header_t) + sizeof(rtp_sbc_payload_t);
            expected_pop = flen;
            popped = pop_bt_resampled_s16(bt, pcm, flen, ch, &frac_pos, drift_scale, &rem_avail);

            if (mtu > hdr_off) {
                int pcm_used = (int)(flen * ch * sizeof(int16_t));
                int stream_sz = (int)(mtu - hdr_off);
                int frame_num = 0;
                if (g_ldac.encode(bt->ldac_enc, pcm, &pcm_used,
                                  pkt + hdr_off, &stream_sz, &frame_num) == 0 &&
                    frame_num > 0 && stream_sz > 0 && hdr_off + (size_t)stream_sz <= mtu) {
                    rtp_header_t *rtp = (rtp_header_t *)pkt;
                    rtp_sbc_payload_t *pl = (rtp_sbc_payload_t *)(pkt + sizeof(rtp_header_t));
                    memset(rtp, 0, sizeof(*rtp));
                    memset(pl, 0, sizeof(*pl));
                    rtp->v = 2;
                    rtp->pt = 96;
                    rtp->m = 0;
                    rtp->sequence_number = htons(bt->rtp_seq++);
                    rtp->timestamp = htonl(bt->rtp_timestamp);
                    rtp->ssrc = htonl(1);
                    pl->frame_count = (uint8_t)(frame_num & 0x0F);
                    pkt_len = hdr_off + (size_t)stream_sz;
                }
            }
            ts_step = flen;
        } else if ((bt->active_codec_kind == LWASAPI_BT_CODEC_APTX_HD ||
                    bt->active_codec_kind == LWASAPI_BT_CODEC_APTX) && bt->aptx_enc) {
            bool is_hd = (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX_HD);
            size_t hdr_off = is_hd ? sizeof(rtp_header_t) : 0;
            size_t enc_blk_bytes = is_hd ? 6U : 4U;
            size_t avail_out = (mtu > hdr_off) ? (mtu - hdr_off) : 0;
            uint32_t max_blocks = (uint32_t)(avail_out / enc_blk_bytes);
            if (max_blocks > 96U) max_blocks = 96U;
            uint32_t flen = max_blocks * 4U;
            if (flen == 0) flen = 4U;
            expected_pop = flen;
            popped = pop_bt_resampled_s16(bt, pcm, flen, 2, &frac_pos, drift_scale, &rem_avail);

            uint8_t s24_buf[96U * 24U];
            uint32_t nsamp = flen * 2U;
            for (uint32_t s = 0; s < nsamp; s++) {
                int32_t v = ((int32_t)pcm[s]) << 8;
                s24_buf[s * 3U + 0] = (uint8_t)(v & 0xFF);
                s24_buf[s * 3U + 1] = (uint8_t)((v >> 8) & 0xFF);
                s24_buf[s * 3U + 2] = (uint8_t)((v >> 16) & 0xFF);
            }

            size_t written = 0;
            if (avail_out >= enc_blk_bytes) {
                g_aptx.encode(bt->aptx_enc, s24_buf, (size_t)nsamp * 3U,
                              pkt + hdr_off, avail_out, &written);
                if (written > 0 && hdr_off + written <= mtu) {
                    if (is_hd) {
                        rtp_header_t *rtp = (rtp_header_t *)pkt;
                        memset(rtp, 0, sizeof(*rtp));
                        rtp->v = 2;
                        rtp->pt = 96;
                        rtp->m = 0;
                        rtp->sequence_number = htons(bt->rtp_seq++);
                        rtp->timestamp = htonl(bt->rtp_timestamp);
                        rtp->ssrc = htonl(1);
                    }
                    pkt_len = hdr_off + written;
                }
            }
            ts_step = flen;
        } else if ((bt->codec_id == A2DP_CODEC_MPEG24 || bt->active_codec_kind == LWASAPI_BT_CODEC_AAC) && bt->fdk_enc) {
            uint32_t flen = bt->fdk_frame_length ? bt->fdk_frame_length : 1024;
            expected_pop = flen;
            popped = pop_bt_resampled_s16(bt, pcm, flen, ch, &frac_pos, drift_scale, &rem_avail);

            void *in_ptr  = pcm;
            void *out_ptr = pkt + sizeof(rtp_header_t);
            int   in_id   = FDK_IN_AUDIO_DATA;
            int   out_id  = FDK_OUT_BITSTREAM_DATA;
            int   in_sz   = (int)(flen * ch * sizeof(int16_t));
            int   out_sz  = (int)(mtu - sizeof(rtp_header_t));
            int   in_el   = (int)sizeof(int16_t);
            int   out_el  = 1;

            fdk_buf_desc_t in_desc  = { 1, &in_ptr,  &in_id,  &in_sz,  &in_el };
            fdk_buf_desc_t out_desc = { 1, &out_ptr, &out_id, &out_sz, &out_el };
            fdk_in_args_t  in_args  = { (int)(flen * ch), 0 };
            fdk_out_args_t out_args = { 0, 0, 0, 0 };

            if (g_fdk.encode(bt->fdk_enc, &in_desc, &out_desc, &in_args, &out_args) == FDK_AACENC_OK &&
                out_args.numOutBytes > 0) {
                rtp_header_t *rtp = (rtp_header_t *)pkt;
                memset(rtp, 0, sizeof(*rtp));
                rtp->v  = 2;
                rtp->pt = 96;
                rtp->m  = 1;
                rtp->sequence_number = htons(bt->rtp_seq++);
                rtp->timestamp       = htonl(bt->rtp_timestamp);
                rtp->ssrc            = htonl(1);
                pkt_len = sizeof(rtp_header_t) + (size_t)out_args.numOutBytes;
            }
            ts_step = flen;
        } else if (bt->sbc_enc_init) {
            size_t codesize = sbc_get_codesize(&bt->sbc_enc);
            uint32_t sbc_frames_per_pkt = 4;
            uint32_t samples_per_frame = (uint32_t)(codesize / (ch * sizeof(int16_t)));
            if (samples_per_frame == 0) samples_per_frame = 128;

            uint32_t max_frames = sbc_frames_per_pkt;
            if (max_frames * samples_per_frame * ch > 4096) max_frames = 2;

            expected_pop = max_frames * samples_per_frame;
            popped = pop_bt_resampled_s16(bt, pcm, expected_pop, ch, &frac_pos, drift_scale, &rem_avail);

            size_t header_len = sizeof(rtp_header_t) + sizeof(rtp_sbc_payload_t);
            size_t off = header_len;
            uint32_t encoded_cnt = 0;

            for (uint32_t f = 0; f < max_frames; f++) {
                ssize_t enc_written = 0;
                ssize_t in_consumed = sbc_encode(&bt->sbc_enc,
                                                 pcm + f * samples_per_frame * ch, codesize,
                                                 pkt + off, mtu - off, &enc_written);
                if (in_consumed <= 0 || enc_written <= 0) break;
                off += (size_t)enc_written;
                encoded_cnt++;
                if (off + (size_t)enc_written > mtu) break;
            }
            if (encoded_cnt > 0) {
                rtp_header_t *rtp = (rtp_header_t *)pkt;
                rtp_sbc_payload_t *pl = (rtp_sbc_payload_t *)(pkt + sizeof(rtp_header_t));
                memset(rtp, 0, sizeof(*rtp));
                memset(pl, 0, sizeof(*pl));
                rtp->v  = 2;
                rtp->pt = 96;
                rtp->m  = 0;
                rtp->sequence_number = htons(bt->rtp_seq++);
                rtp->timestamp       = htonl(bt->rtp_timestamp);
                rtp->ssrc            = htonl(1);
                pl->frame_count      = (uint8_t)(encoded_cnt & 0x0F);
                pkt_len = off;
            }
            ts_step = encoded_cnt * samples_per_frame;
        }

        bt->rtp_timestamp += ts_step;
        int tx_fd = bt->bt_fd;
        pthread_mutex_unlock(&bt->state_lock);

        if (popped == expected_pop) {
            avg_avail = avg_avail * 0.90 + (double)rem_avail * 0.10;
        } else if (popped > 0) {
            atomic_fetch_add_explicit(&bt->tx_underruns, 1ULL, memory_order_relaxed);
            lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                                    (int32_t)popped, (int32_t)expected_pop, 0,
                                    "BT_TX_UNDERRUN", bt->device_alias,
                                    "TX ring partial pop (%zu/%u frames) -> re-prebuffering",
                                    popped, expected_pop);
            prebuffering = true;
            prebuf_waits = 0;
        }

        if (pkt_len > 0 && tx_fd >= 0) {
            bool had_eagain = false;
            ssize_t sres = send(tx_fd, pkt, pkt_len, MSG_DONTWAIT | MSG_NOSIGNAL);
            if (sres < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
                had_eagain = true;
                struct pollfd pfd = { .fd = tx_fd, .events = POLLOUT };
                if (poll(&pfd, 1, 15) > 0 && (pfd.revents & POLLOUT)) {
                    sres = send(tx_fd, pkt, pkt_len, MSG_DONTWAIT | MSG_NOSIGNAL);
                }
            }
            if (sres > 0) {
                atomic_fetch_add_explicit(&bt->tx_packets_sent, 1ULL, memory_order_relaxed);
                uint64_t now_ms = bt_monotonic_ns() / 1000000ULL;
                pthread_mutex_lock(&bt->state_lock);
                if (had_eagain) {
                    bt->tx_congest_streak++;
                    bt->tx_clean_streak = 0;
                } else {
                    if (bt->tx_congest_streak > 0) bt->tx_congest_streak--;
                    bt->tx_clean_streak++;
                }
                if (bt->active_codec_kind == LWASAPI_BT_CODEC_LDAC && bt->ldac_enc &&
                    bt->ldac_mode == LWASAPI_LDAC_MODE_ADAPTIVE && bt->wireless_abr_enabled &&
                    (now_ms - bt->abr_last_ms) >= 20ULL) {
                    bt->abr_last_ms = now_ms;
                    unsigned int qdepth = had_eagain ? (4U + (bt->tx_congest_streak > 6U ? 6U : bt->tx_congest_streak)) : 0U;
                    int prev_eq = bt->ldac_eqmid;
                    int new_eq = prev_eq;
                    if (bt->ldac_abr && g_ldac_abr.proc) {
                        int ret = g_ldac_abr.proc(bt->ldac_enc, bt->ldac_abr, qdepth, 1U);
                        if (ret >= LDACBT_EQMID_HQ && ret <= LDACBT_EQMID_MQ) new_eq = ret;
                    } else {
                        if (bt->tx_congest_streak >= 4U && prev_eq < LDACBT_EQMID_MQ && g_ldac.set_eqmid) {
                            new_eq = prev_eq + 1;
                            if (g_ldac.set_eqmid(bt->ldac_enc, new_eq) == 0) bt->tx_congest_streak = 0;
                        } else if (bt->tx_clean_streak >= 250U && prev_eq > LDACBT_EQMID_HQ && g_ldac.set_eqmid) {
                            new_eq = prev_eq - 1;
                            if (g_ldac.set_eqmid(bt->ldac_enc, new_eq) == 0) bt->tx_clean_streak = 0;
                        }
                    }
                    if (new_eq != prev_eq && new_eq >= LDACBT_EQMID_HQ && new_eq <= LDACBT_EQMID_MQ) {
                        bt->ldac_eqmid = new_eq;
                        bt->effective_kbps = ldac_eqmid_to_kbps(bt->bt_sample_rate, new_eq);
                        snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                                 "LDAC ABR adapted -> %ukbps", bt->effective_kbps);
                        lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                                prev_eq, new_eq, (int32_t)bt->effective_kbps,
                                                "BT_LDAC_ABR", bt->device_alias,
                                                "eqmid %d -> %d (%ukbps)", prev_eq, new_eq, bt->effective_kbps);
                    }
                } else if ((bt->codec_id == A2DP_CODEC_MPEG24 || bt->active_codec_kind == LWASAPI_BT_CODEC_AAC) &&
                           bt->fdk_enc && bt->wireless_abr_enabled && g_fdk.set_param) {
                    uint32_t cur_bps = bt->aac_target_bps ? bt->aac_target_bps : 192000U;
                    uint32_t max_bps = bt->aac_max_bps ? bt->aac_max_bps : 256000U;
                    uint32_t next_bps = cur_bps;
                    if (bt->tx_congest_streak >= 4U && cur_bps > 128000U) {
                        next_bps = (cur_bps > 192000U) ? 192000U : 128000U;
                        bt->tx_congest_streak = 0;
                    } else if (bt->tx_clean_streak >= 300U && cur_bps < max_bps) {
                        next_bps = cur_bps + 32000U;
                        if (next_bps > max_bps) next_bps = max_bps;
                        bt->tx_clean_streak = 0;
                    }
                    if (next_bps != cur_bps) {
                        if (g_fdk.set_param(bt->fdk_enc, FDK_PARAM_BITRATE, next_bps) == FDK_AACENC_OK) {
                            bt->aac_target_bps = next_bps;
                            bt->effective_kbps = next_bps / 1000U;
                            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                                     "AAC ABR adapted -> %ukbps", bt->effective_kbps);
                        }
                    }
                }
                pthread_mutex_unlock(&bt->state_lock);
            } else if (sres < 0 && (errno == EPIPE || errno == ECONNRESET || errno == EBADF)) {
                int saved_err = errno;
                atomic_fetch_add_explicit(&bt->tx_errors, 1ULL, memory_order_relaxed);
                lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_ERR, 0, 0,
                                        tx_fd, saved_err, -saved_err,
                                        "BT_TX_ERR", bt->device_alias,
                                        "L2CAP send failed on fd=%d: %s",
                                        tx_fd, strerror(saved_err));
                pthread_mutex_lock(&bt->state_lock);
                if (bt->bt_fd == tx_fd) {
                    close(bt->bt_fd);
                    bt->bt_fd = -1;
                    bt->direct_socket_mode = false;
                    bt->tx_stream_active = 0;
                    bt->has_bluez_transport = false;
                }
                pthread_mutex_unlock(&bt->state_lock);
                atomic_store_explicit(&bt->tx_drop_event_ns, bt_monotonic_ns(), memory_order_relaxed);
                continue;
            } else if (sres < 0 && errno != EAGAIN && errno != EWOULDBLOCK) {
                atomic_fetch_add_explicit(&bt->tx_errors, 1ULL, memory_order_relaxed);
            }
        }

        if (ts_step == 0) ts_step = 1024;
        uint64_t period_ns = ((uint64_t)ts_step * 1000000000ULL) / (uint64_t)rate;
        next_tick_ns += period_ns;
        uint64_t now_ns = bt_monotonic_ns();
        if (now_ns > next_tick_ns + period_ns * 4ULL) {
            next_tick_ns = now_ns + period_ns;
        }
        struct timespec ts_wake = {
            .tv_sec  = (time_t)(next_tick_ns / 1000000000ULL),
            .tv_nsec = (long)(next_tick_ns % 1000000000ULL)
        };
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &ts_wake, NULL);
    }
    return NULL;
}

/* ============================================================================
 * BlueZ System D-Bus MediaEndpoint1 Server & Properties/AVRCP Management
 * ============================================================================ */

static void add_dict_entry_str(DBusMessageIter *dict, const char *k, const char *v) {
    DBusMessageIter entry, var;
    dbus_message_iter_open_container(dict, DBUS_TYPE_DICT_ENTRY, NULL, &entry);
    dbus_message_iter_append_basic(&entry, DBUS_TYPE_STRING, &k);
    dbus_message_iter_open_container(&entry, DBUS_TYPE_VARIANT, "s", &var);
    dbus_message_iter_append_basic(&var, DBUS_TYPE_STRING, &v);
    dbus_message_iter_close_container(&entry, &var);
    dbus_message_iter_close_container(dict, &entry);
}

static void add_dict_entry_byte(DBusMessageIter *dict, const char *k, uint8_t v) {
    DBusMessageIter entry, var;
    dbus_message_iter_open_container(dict, DBUS_TYPE_DICT_ENTRY, NULL, &entry);
    dbus_message_iter_append_basic(&entry, DBUS_TYPE_STRING, &k);
    dbus_message_iter_open_container(&entry, DBUS_TYPE_VARIANT, "y", &var);
    dbus_message_iter_append_basic(&var, DBUS_TYPE_BYTE, &v);
    dbus_message_iter_close_container(&entry, &var);
    dbus_message_iter_close_container(dict, &entry);
}

static void add_dict_entry_bool(DBusMessageIter *dict, const char *k, dbus_bool_t v) {
    DBusMessageIter entry, var;
    dbus_message_iter_open_container(dict, DBUS_TYPE_DICT_ENTRY, NULL, &entry);
    dbus_message_iter_append_basic(&entry, DBUS_TYPE_STRING, &k);
    dbus_message_iter_open_container(&entry, DBUS_TYPE_VARIANT, "b", &var);
    dbus_message_iter_append_basic(&var, DBUS_TYPE_BOOLEAN, &v);
    dbus_message_iter_close_container(&entry, &var);
    dbus_message_iter_close_container(dict, &entry);
}

static void add_dict_entry_bytes(DBusMessageIter *dict, const char *k, const uint8_t *buf, int len) {
    DBusMessageIter entry, var, arr;
    dbus_message_iter_open_container(dict, DBUS_TYPE_DICT_ENTRY, NULL, &entry);
    dbus_message_iter_append_basic(&entry, DBUS_TYPE_STRING, &k);
    dbus_message_iter_open_container(&entry, DBUS_TYPE_VARIANT, "ay", &var);
    dbus_message_iter_open_container(&var, DBUS_TYPE_ARRAY, "y", &arr);
    dbus_message_iter_append_fixed_array(&arr, DBUS_TYPE_BYTE, &buf, len);
    dbus_message_iter_close_container(&var, &arr);
    dbus_message_iter_close_container(&entry, &var);
    dbus_message_iter_close_container(dict, &entry);
}

static void add_dict_entry_u16(DBusMessageIter *dict, const char *k, uint16_t v) {
    DBusMessageIter entry, var;
    dbus_message_iter_open_container(dict, DBUS_TYPE_DICT_ENTRY, NULL, &entry);
    dbus_message_iter_append_basic(&entry, DBUS_TYPE_STRING, &k);
    dbus_message_iter_open_container(&entry, DBUS_TYPE_VARIANT, "q", &var);
    dbus_message_iter_append_basic(&var, DBUS_TYPE_UINT16, &v);
    dbus_message_iter_close_container(&entry, &var);
    dbus_message_iter_close_container(dict, &entry);
}

static void add_dict_entry_obj_path(DBusMessageIter *dict, const char *k, const char *v) {
    DBusMessageIter entry, var;
    dbus_message_iter_open_container(dict, DBUS_TYPE_DICT_ENTRY, NULL, &entry);
    dbus_message_iter_append_basic(&entry, DBUS_TYPE_STRING, &k);
    dbus_message_iter_open_container(&entry, DBUS_TYPE_VARIANT, "o", &var);
    dbus_message_iter_append_basic(&var, DBUS_TYPE_OBJECT_PATH, &v);
    dbus_message_iter_close_container(&entry, &var);
    dbus_message_iter_close_container(dict, &entry);
}

static bool dbus_get_str_prop(DBusConnection *conn, const char *path,
                              const char *iface, const char *prop, char *out, size_t out_len) {
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", path,
                                                     "org.freedesktop.DBus.Properties", "Get");
    if (!msg) return false;
    dbus_message_append_args(msg, DBUS_TYPE_STRING, &iface, DBUS_TYPE_STRING, &prop, DBUS_TYPE_INVALID);
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 1500, NULL);
    dbus_message_unref(msg);
    if (!rep) return false;

    DBusMessageIter it, var;
    bool ok = false;
    if (dbus_message_iter_init(rep, &it) && dbus_message_iter_get_arg_type(&it) == DBUS_TYPE_VARIANT) {
        dbus_message_iter_recurse(&it, &var);
        if (dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_STRING) {
            const char *s = NULL;
            dbus_message_iter_get_basic(&var, &s);
            if (s) {
                snprintf(out, out_len, "%s", s);
                ok = true;
            }
        }
    }
    dbus_message_unref(rep);
    return ok;
}

static bool dbus_get_u16_prop(DBusConnection *conn, const char *path,
                              const char *iface, const char *prop, uint16_t *out_val) {
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", path,
                                                     "org.freedesktop.DBus.Properties", "Get");
    if (!msg) return false;
    dbus_message_append_args(msg, DBUS_TYPE_STRING, &iface, DBUS_TYPE_STRING, &prop, DBUS_TYPE_INVALID);
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 1000, NULL);
    dbus_message_unref(msg);
    if (!rep) return false;

    DBusMessageIter it, var;
    bool ok = false;
    if (dbus_message_iter_init(rep, &it) && dbus_message_iter_get_arg_type(&it) == DBUS_TYPE_VARIANT) {
        dbus_message_iter_recurse(&it, &var);
        if (dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_UINT16) {
            dbus_message_iter_get_basic(&var, out_val);
            ok = true;
        }
    }
    dbus_message_unref(rep);
    return ok;
}

static void dbus_set_avrcp_volume(DBusConnection *conn, const char *transport_path, uint16_t vol_u16) {
    if (!conn || !transport_path || !transport_path[0]) return;
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", transport_path,
                                                     "org.freedesktop.DBus.Properties", "Set");
    if (!msg) return;
    const char *iface = "org.bluez.MediaTransport1";
    const char *prop  = "Volume";
    DBusMessageIter it, var;
    dbus_message_iter_init_append(msg, &it);
    dbus_message_iter_append_basic(&it, DBUS_TYPE_STRING, &iface);
    dbus_message_iter_append_basic(&it, DBUS_TYPE_STRING, &prop);
    dbus_message_iter_open_container(&it, DBUS_TYPE_VARIANT, "q", &var);
    dbus_message_iter_append_basic(&var, DBUS_TYPE_UINT16, &vol_u16);
    dbus_message_iter_close_container(&it, &var);
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 1000, NULL);
    dbus_message_unref(msg);
    if (rep) dbus_message_unref(rep);
}

static bool dbus_get_bool_prop(DBusConnection *conn, const char *path,
                               const char *iface, const char *prop, bool *out_val) {
    if (!conn || !path || !path[0] || !out_val) return false;
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", path,
                                                     "org.freedesktop.DBus.Properties", "Get");
    if (!msg) return false;
    dbus_message_append_args(msg, DBUS_TYPE_STRING, &iface, DBUS_TYPE_STRING, &prop, DBUS_TYPE_INVALID);
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 1500, NULL);
    dbus_message_unref(msg);
    if (!rep) return false;

    DBusMessageIter it, var;
    bool ok = false;
    if (dbus_message_iter_init(rep, &it) && dbus_message_iter_get_arg_type(&it) == DBUS_TYPE_VARIANT) {
        dbus_message_iter_recurse(&it, &var);
        if (dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_BOOLEAN) {
            dbus_bool_t b = FALSE;
            dbus_message_iter_get_basic(&var, &b);
            *out_val = (b != FALSE);
            ok = true;
        }
    }
    dbus_message_unref(rep);
    return ok;
}

static void dbus_set_str_prop(DBusConnection *conn, const char *path,
                              const char *iface, const char *prop, const char *val) {
    if (!conn || !path || !path[0] || !val) return;
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", path,
                                                     "org.freedesktop.DBus.Properties", "Set");
    if (!msg) return;
    DBusMessageIter it, var;
    dbus_message_iter_init_append(msg, &it);
    dbus_message_iter_append_basic(&it, DBUS_TYPE_STRING, &iface);
    dbus_message_iter_append_basic(&it, DBUS_TYPE_STRING, &prop);
    dbus_message_iter_open_container(&it, DBUS_TYPE_VARIANT, "s", &var);
    dbus_message_iter_append_basic(&var, DBUS_TYPE_STRING, &val);
    dbus_message_iter_close_container(&it, &var);
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 1500, NULL);
    dbus_message_unref(msg);
    if (rep) dbus_message_unref(rep);
}

static bool extract_mac_from_id(const char *id, char *out_mac, size_t out_len) {
    if (!id || !id[0] || !out_mac || out_len < 18) return false;
    const char *p = id;
    if (strncasecmp(p, "bluez:", 6) == 0) p += 6;
    if (strlen(p) < 17) return false;
    size_t colons = 0;
    for (size_t i = 0; i < 17; i++) {
        char ch = p[i];
        if ((i % 3) == 2) {
            if (ch != ':' && ch != '_' && ch != '-') return false;
            out_mac[i] = ':';
            colons++;
        } else {
            if (!isxdigit((unsigned char)ch)) return false;
            out_mac[i] = (char)toupper((unsigned char)ch);
        }
    }
    if (colons != 5) return false;
    out_mac[17] = '\0';
    return true;
}

static bool find_a2dp_device_path(DBusConnection *conn, const char *mac_or_empty,
                                  char *out_dev_path, size_t dev_sz,
                                  char *out_alias, size_t alias_sz,
                                  char *out_addr, size_t addr_sz,
                                  bool *out_connected) {
    if (!conn || !out_dev_path || dev_sz == 0) return false;
    out_dev_path[0] = '\0';
    if (out_alias && alias_sz) out_alias[0] = '\0';
    if (out_addr && addr_sz) out_addr[0] = '\0';
    if (out_connected) *out_connected = false;

    char want_mac[32] = {0};
    bool have_want = extract_mac_from_id(mac_or_empty, want_mac, sizeof(want_mac));

    DBusMessage *msg = dbus_message_new_method_call("org.bluez", "/",
                                                     "org.freedesktop.DBus.ObjectManager", "GetManagedObjects");
    if (!msg) return false;
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 2000, NULL);
    dbus_message_unref(msg);
    if (!rep) return false;

    int best_score = -1;
    DBusMessageIter root, obj_arr, obj_entry, if_arr, if_entry, prop_arr, prop_entry, var;
    if (dbus_message_iter_init(rep, &root) && dbus_message_iter_get_arg_type(&root) == DBUS_TYPE_ARRAY) {
        dbus_message_iter_recurse(&root, &obj_arr);
        while (dbus_message_iter_get_arg_type(&obj_arr) == DBUS_TYPE_DICT_ENTRY) {
            const char *obj_path = NULL;
            dbus_message_iter_recurse(&obj_arr, &obj_entry);
            dbus_message_iter_get_basic(&obj_entry, &obj_path);
            dbus_message_iter_next(&obj_entry);
            dbus_message_iter_recurse(&obj_entry, &if_arr);
            while (dbus_message_iter_get_arg_type(&if_arr) == DBUS_TYPE_DICT_ENTRY) {
                const char *iface = NULL;
                dbus_message_iter_recurse(&if_arr, &if_entry);
                dbus_message_iter_get_basic(&if_entry, &iface);
                dbus_message_iter_next(&if_entry);

                if (iface && strcmp(iface, "org.bluez.Device1") == 0 && obj_path) {
                    dbus_bool_t connected = FALSE, paired = FALSE, trusted = FALSE;
                    bool has_a2dp_sink = false;
                    char addr[32] = {0}, alias[64] = {0};
                    dbus_message_iter_recurse(&if_entry, &prop_arr);
                    while (dbus_message_iter_get_arg_type(&prop_arr) == DBUS_TYPE_DICT_ENTRY) {
                        const char *pk = NULL;
                        dbus_message_iter_recurse(&prop_arr, &prop_entry);
                        dbus_message_iter_get_basic(&prop_entry, &pk);
                        dbus_message_iter_next(&prop_entry);
                        dbus_message_iter_recurse(&prop_entry, &var);
                        int vtype = dbus_message_iter_get_arg_type(&var);
                        if (pk && strcmp(pk, "Connected") == 0 && vtype == DBUS_TYPE_BOOLEAN) {
                            dbus_message_iter_get_basic(&var, &connected);
                        } else if (pk && strcmp(pk, "Paired") == 0 && vtype == DBUS_TYPE_BOOLEAN) {
                            dbus_message_iter_get_basic(&var, &paired);
                        } else if (pk && strcmp(pk, "Trusted") == 0 && vtype == DBUS_TYPE_BOOLEAN) {
                            dbus_message_iter_get_basic(&var, &trusted);
                        } else if (pk && strcmp(pk, "Address") == 0 && vtype == DBUS_TYPE_STRING) {
                            const char *s = NULL;
                            dbus_message_iter_get_basic(&var, &s);
                            if (s) snprintf(addr, sizeof(addr), "%s", s);
                        } else if (pk && strcmp(pk, "Alias") == 0 && vtype == DBUS_TYPE_STRING) {
                            const char *s = NULL;
                            dbus_message_iter_get_basic(&var, &s);
                            if (s) snprintf(alias, sizeof(alias), "%s", s);
                        } else if (pk && strcmp(pk, "UUIDs") == 0 && vtype == DBUS_TYPE_ARRAY) {
                            DBusMessageIter uarr;
                            dbus_message_iter_recurse(&var, &uarr);
                            while (dbus_message_iter_get_arg_type(&uarr) == DBUS_TYPE_STRING) {
                                const char *u = NULL;
                                dbus_message_iter_get_basic(&uarr, &u);
                                if (u && strncasecmp(u, "0000110b", 8) == 0) has_a2dp_sink = true;
                                dbus_message_iter_next(&uarr);
                            }
                        }
                        dbus_message_iter_next(&prop_arr);
                    }
                    if (has_a2dp_sink && paired) {
                        bool exact = (have_want && addr[0] && strcasecmp(addr, want_mac) == 0);
                        if (!have_want || exact) {
                            int score = (exact ? 100 : 0) + (connected ? 10 : 0) + (trusted ? 2 : 0);
                            if (score > best_score) {
                                best_score = score;
                                snprintf(out_dev_path, dev_sz, "%s", obj_path);
                                if (out_alias && alias_sz) snprintf(out_alias, alias_sz, "%s", alias[0] ? alias : addr);
                                if (out_addr && addr_sz) snprintf(out_addr, addr_sz, "%s", addr);
                                if (out_connected) *out_connected = (connected != FALSE);
                            }
                        }
                    }
                }
                dbus_message_iter_next(&if_arr);
            }
            dbus_message_iter_next(&obj_arr);
        }
    }
    dbus_message_unref(rep);
    return out_dev_path[0] != '\0';
}

static bool device_has_active_transport(DBusConnection *conn, const char *dev_path) {
    if (!conn || !dev_path || !dev_path[0]) return false;
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", "/",
                                                     "org.freedesktop.DBus.ObjectManager", "GetManagedObjects");
    if (!msg) return false;
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 1500, NULL);
    dbus_message_unref(msg);
    if (!rep) return false;

    bool found = false;
    DBusMessageIter root, obj_arr, obj_entry, if_arr, if_entry, prop_arr, prop_entry, var;
    if (dbus_message_iter_init(rep, &root) && dbus_message_iter_get_arg_type(&root) == DBUS_TYPE_ARRAY) {
        dbus_message_iter_recurse(&root, &obj_arr);
        while (!found && dbus_message_iter_get_arg_type(&obj_arr) == DBUS_TYPE_DICT_ENTRY) {
            dbus_message_iter_recurse(&obj_arr, &obj_entry);
            dbus_message_iter_next(&obj_entry);
            dbus_message_iter_recurse(&obj_entry, &if_arr);
            while (!found && dbus_message_iter_get_arg_type(&if_arr) == DBUS_TYPE_DICT_ENTRY) {
                const char *iface = NULL;
                dbus_message_iter_recurse(&if_arr, &if_entry);
                dbus_message_iter_get_basic(&if_entry, &iface);
                dbus_message_iter_next(&if_entry);
                if (iface && strcmp(iface, "org.bluez.MediaTransport1") == 0) {
                    char tdev[128] = {0}, tstate[32] = {0};
                    dbus_message_iter_recurse(&if_entry, &prop_arr);
                    while (dbus_message_iter_get_arg_type(&prop_arr) == DBUS_TYPE_DICT_ENTRY) {
                        const char *pk = NULL;
                        dbus_message_iter_recurse(&prop_arr, &prop_entry);
                        dbus_message_iter_get_basic(&prop_entry, &pk);
                        dbus_message_iter_next(&prop_entry);
                        dbus_message_iter_recurse(&prop_entry, &var);
                        if (pk && strcmp(pk, "Device") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_OBJECT_PATH) {
                            const char *s = NULL;
                            dbus_message_iter_get_basic(&var, &s);
                            if (s) snprintf(tdev, sizeof(tdev), "%s", s);
                        } else if (pk && strcmp(pk, "State") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_STRING) {
                            const char *s = NULL;
                            dbus_message_iter_get_basic(&var, &s);
                            if (s) snprintf(tstate, sizeof(tstate), "%s", s);
                        }
                        dbus_message_iter_next(&prop_arr);
                    }
                    if (strcmp(tdev, dev_path) == 0 &&
                        (strcmp(tstate, "active") == 0 || strcmp(tstate, "idle") == 0)) {
                        found = true;
                    }
                }
                dbus_message_iter_next(&if_arr);
            }
            dbus_message_iter_next(&obj_arr);
        }
    }
    dbus_message_unref(rep);
    return found;
}

static int cycle_device_bredr_reconnect(DBusConnection *conn, const char *dev_path,
                                        const char *alias, bool wait_for_transport,
                                        char *out_err, size_t out_err_len) {
    if (!conn || !dev_path || !dev_path[0]) return -1;
    const char *label = (alias && alias[0]) ? alias : dev_path;

    bool is_conn = false;
    if (dbus_get_bool_prop(conn, dev_path, "org.bluez.Device1", "Connected", &is_conn) && is_conn) {
        DBusError derr;
        dbus_error_init(&derr);
        DBusMessage *dmsg = dbus_message_new_method_call("org.bluez", dev_path,
                                                          "org.bluez.Device1", "Disconnect");
        if (dmsg) {
            DBusMessage *drep = dbus_connection_send_with_reply_and_block(conn, dmsg, 5000, &derr);
            dbus_message_unref(dmsg);
            if (drep) dbus_message_unref(drep);
        }
        dbus_error_free(&derr);
        for (int i = 0; i < 20; i++) {
            bool now_conn = true;
            if (dbus_get_bool_prop(conn, dev_path, "org.bluez.Device1", "Connected", &now_conn) && !now_conn) {
                break;
            }
            usleep(250000);
        }
        usleep(400000);
    }

    dbus_set_str_prop(conn, dev_path, "org.bluez.Device1", "PreferredBearer", "bredr");

    DBusError cerr;
    dbus_error_init(&cerr);
    DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", dev_path,
                                                      "org.bluez.Device1", "Connect");
    if (!cmsg) {
        if (out_err && out_err_len) snprintf(out_err, out_err_len, "Out of memory creating Connect D-Bus call");
        return -1;
    }
    DBusMessage *crep = dbus_connection_send_with_reply_and_block(conn, cmsg, 12000, &cerr);
    dbus_message_unref(cmsg);
    if (!crep) {
        const char *emsg = cerr.message ? cerr.message : "Connect failed";
        if (strstr(emsg, "br-connection-key-missing") != NULL) {
            if (out_err && out_err_len) {
                snprintf(out_err, out_err_len,
                         "Bluetooth link key mismatch (br-connection-key-missing) for %s. Please re-pair headphones.",
                         label);
            }
            dbus_error_free(&cerr);
            return -2;
        }
        /* Tolerate AlreadyConnected / InProgress and verify Connected property below */
        if (strstr(emsg, "AlreadyConnected") == NULL && strstr(emsg, "InProgress") == NULL) {
            if (out_err && out_err_len) {
                snprintf(out_err, out_err_len, "Failed to connect %s: %s", label, emsg);
            }
            dbus_error_free(&cerr);
            return -1;
        }
        dbus_error_free(&cerr);
    } else {
        dbus_error_free(&cerr);
        dbus_message_unref(crep);
    }

    bool connected_ok = false;
    for (int i = 0; i < 24; i++) {
        if (dbus_get_bool_prop(conn, dev_path, "org.bluez.Device1", "Connected", &connected_ok) && connected_ok) {
            break;
        }
        usleep(250000);
    }
    if (!connected_ok) {
        if (out_err && out_err_len) {
            snprintf(out_err, out_err_len, "Failed to connect %s. Ensure headphones are powered on.", label);
        }
        return -1;
    }

    if (wait_for_transport) {
        bool transport_ok = false;
        for (int i = 0; i < 30; i++) {
            if (device_has_active_transport(conn, dev_path)) {
                transport_ok = true;
                break;
            }
            usleep(200000);
        }
        if (!transport_ok) {
            if (out_err && out_err_len) {
                snprintf(out_err, out_err_len,
                         "%s connected on BR/EDR, but A2DP audio transport is still initializing.",
                         label);
            }
            return -1;
        }
    }
    return 0;
}

static int select_a2dp_config_for_endpoint(const char *path, const uint8_t *rcaps, int rlen,
                                          uint8_t *out_cfg, int *out_len, uint32_t *out_cap_bit) {
    if (!path || !rcaps || rlen <= 0 || !out_cfg || !out_len) return -1;
    *out_len = 0;
    if (out_cap_bit) *out_cap_bit = 0;

    int kind = -1;
    if (strcmp(path, LWASAPI_EP_LDAC) == 0)        kind = SALSA_BT_WIRE_LDAC;
    else if (strcmp(path, LWASAPI_EP_APTXHD) == 0) kind = SALSA_BT_WIRE_APTX_HD;
    else if (strcmp(path, LWASAPI_EP_APTX) == 0)   kind = SALSA_BT_WIRE_APTX;
    else if (strcmp(path, LWASAPI_EP_AAC) == 0)    kind = SALSA_BT_WIRE_AAC;
    else if (strcmp(path, LWASAPI_EP_SBC) == 0)    kind = SALSA_BT_WIRE_SBC;
    else return -1;

    if (kind == SALSA_BT_WIRE_SBC && rlen >= 4) {
        if (rcaps[2] < 2u || rcaps[3] > 53u || rcaps[2] > rcaps[3]) {
            salsa_failsafe_record_trip(SALSA_FS_BT_FALLBACK,
                                       "Clamped malformed A2DP SBC bitpool bounds");
        }
    }

    size_t ol = 0;
    int rc = salsa_bt_select_a2dp_config(kind, rcaps, (size_t)rlen, out_cfg, 16u, &ol, out_cap_bit);
    if (rc == 0) {
        *out_len = (int)ol;
    } else {
        salsa_failsafe_record_trip(SALSA_FS_BT_FALLBACK,
                                   "Rejected malformed BlueZ A2DP endpoint capability payload");
    }
    return rc;
}

static void acquire_bluez_transport(DBusConnection *conn, lwasapi_bluez_t *bt) {
    if (!conn || !bt->transport_path[0]) return;
    DBusError err;
    dbus_error_init(&err);

    DBusMessage *msg = dbus_message_new_method_call("org.bluez", bt->transport_path,
                                                     "org.bluez.MediaTransport1", "Acquire");
    if (!msg) return;
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 3000, &err);
    dbus_message_unref(msg);

    if (!rep) {
        dbus_error_free(&err);
        dbus_error_init(&err);
        msg = dbus_message_new_method_call("org.bluez", bt->transport_path,
                                           "org.bluez.MediaTransport1", "TryAcquire");
        if (!msg) return;
        rep = dbus_connection_send_with_reply_and_block(conn, msg, 2000, &err);
        dbus_message_unref(msg);
    }

    if (!rep) {
        printf("[LWASAPI-BlueZ] Transport Acquire deferred (%s)\n", err.message ? err.message : "unknown");
        lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                           "BT_ACQ_DEFER", bt->transport_path,
                           "MediaTransport1.Acquire deferred: %s",
                           err.message ? err.message : "unknown");
        dbus_error_free(&err);
        return;
    }

    int fd = -1;
    uint16_t rmtu = 0, wmtu = 0;
    if (dbus_message_get_args(rep, &err,
                              DBUS_TYPE_UNIX_FD, &fd,
                              DBUS_TYPE_UINT16, &rmtu,
                              DBUS_TYPE_UINT16, &wmtu,
                              DBUS_TYPE_INVALID) && fd >= 0) {
        pthread_mutex_lock(&bt->state_lock);
        if (bt->bt_fd >= 0) close(bt->bt_fd);
        bt->bt_fd = fd;
        bt->read_mtu = rmtu;
        bt->write_mtu = wmtu;
        bt->direct_socket_mode = true;
        bt->has_bluez_transport = true;
        init_bt_encoder_for_transport(bt);
        bt->tx_stream_active = 1;
        pthread_mutex_unlock(&bt->state_lock);
        printf("[LWASAPI-BlueZ] Acquired L2CAP fd=%d MTU=%u/%u Codec=%s (%s @ %uHz)\n",
               fd, rmtu, wmtu, bt->codec_name, bt->device_alias, bt->bt_sample_rate);
        lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                (int32_t)rmtu, (int32_t)wmtu, fd,
                                "BT_L2CAP_ACQ", bt->device_alias,
                                "L2CAP fd=%d mtu=%u/%u codec=%s rate=%u",
                                fd, rmtu, wmtu, bt->codec_name, bt->bt_sample_rate);
    }
    dbus_error_free(&err);
    dbus_message_unref(rep);
}

static DBusHandlerResult endpoint_message_handler(DBusConnection *conn, DBusMessage *msg, void *user_data) {
    lwasapi_bluez_t *bt = (lwasapi_bluez_t *)user_data;
    const char *path   = dbus_message_get_path(msg);
    const char *member = dbus_message_get_member(msg);
    const char *iface  = dbus_message_get_interface(msg);
    if (!path || !member || !iface) return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    if (strcmp(iface, "org.bluez.MediaEndpoint1") != 0) return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;

    if (strcmp(member, "SelectConfiguration") == 0) {
        const uint8_t *rcaps = NULL;
        int rlen = 0;
        if (!dbus_message_get_args(msg, NULL, DBUS_TYPE_ARRAY, DBUS_TYPE_BYTE, &rcaps, &rlen, DBUS_TYPE_INVALID)) {
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        }
        uint8_t out_cfg[16];
        int out_len = 0;
        uint32_t cap_bit = 0;
        select_a2dp_config_for_endpoint(path, rcaps, rlen, out_cfg, &out_len, &cap_bit);

        if (out_len > 0 && cap_bit) {
            pthread_mutex_lock(&bt->state_lock);
            bt->supported_codecs_mask |= cap_bit;
            pthread_mutex_unlock(&bt->state_lock);
        }

        DBusMessage *rep = (out_len > 0)
                         ? dbus_message_new_method_return(msg)
                         : dbus_message_new_error(msg, "org.bluez.Error.InvalidArguments", "Unsupported codec capabilities");
        if (rep) {
            if (out_len > 0) {
                const uint8_t *p = out_cfg;
                dbus_message_append_args(rep, DBUS_TYPE_ARRAY, DBUS_TYPE_BYTE, &p, out_len, DBUS_TYPE_INVALID);
            }
            dbus_connection_send(conn, rep, NULL);
            dbus_message_unref(rep);
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if (strcmp(member, "SetConfiguration") == 0) {
        DBusMessageIter args, dict, entry, var;
        const char *transport_path = NULL;
        if (!dbus_message_iter_init(msg, &args) || dbus_message_iter_get_arg_type(&args) != DBUS_TYPE_OBJECT_PATH) {
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        }
        dbus_message_iter_get_basic(&args, &transport_path);
        dbus_message_iter_next(&args);

        char dev_path[128] = {0};
        uint8_t cfg[16] = {0};
        uint32_t cfg_len = 0;
        uint16_t delay = 1500;
        uint16_t vol = 127;
        bool has_vol = false;

        if (dbus_message_iter_get_arg_type(&args) == DBUS_TYPE_ARRAY) {
            dbus_message_iter_recurse(&args, &dict);
            while (dbus_message_iter_get_arg_type(&dict) == DBUS_TYPE_DICT_ENTRY) {
                const char *key = NULL;
                dbus_message_iter_recurse(&dict, &entry);
                dbus_message_iter_get_basic(&entry, &key);
                dbus_message_iter_next(&entry);
                dbus_message_iter_recurse(&entry, &var);
                if (key && strcmp(key, "Device") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_OBJECT_PATH) {
                    const char *dp = NULL;
                    dbus_message_iter_get_basic(&var, &dp);
                    if (dp) snprintf(dev_path, sizeof(dev_path), "%s", dp);
                } else if (key && strcmp(key, "Configuration") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_ARRAY) {
                    DBusMessageIter arr;
                    const uint8_t *ptr = NULL;
                    int n = 0;
                    dbus_message_iter_recurse(&var, &arr);
                    dbus_message_iter_get_fixed_array(&arr, &ptr, &n);
                    if (ptr && n > 0) {
                        cfg_len = (uint32_t)(n > 16 ? 16 : n);
                        memcpy(cfg, ptr, cfg_len);
                    }
                } else if (key && strcmp(key, "Delay") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_UINT16) {
                    dbus_message_iter_get_basic(&var, &delay);
                } else if (key && strcmp(key, "Volume") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_UINT16) {
                    dbus_message_iter_get_basic(&var, &vol);
                    has_vol = true;
                }
                dbus_message_iter_next(&dict);
            }
        }

        char alias[64] = "Bluetooth A2DP Sink", addr[32] = "00:00:00:00:00:00";
        if (dev_path[0]) {
            dbus_get_str_prop(conn, dev_path, "org.bluez.Device1", "Alias", alias, sizeof(alias));
            dbus_get_str_prop(conn, dev_path, "org.bluez.Device1", "Address", addr, sizeof(addr));
        }

        pthread_mutex_lock(&bt->state_lock);
        snprintf(bt->transport_path, sizeof(bt->transport_path), "%s", transport_path ? transport_path : "");
        snprintf(bt->device_path, sizeof(bt->device_path), "%s", dev_path);
        snprintf(bt->endpoint_path, sizeof(bt->endpoint_path), "%s", path);
        snprintf(bt->device_alias, sizeof(bt->device_alias), "%s", alias);
        snprintf(bt->mac_address, sizeof(bt->mac_address), "%s", addr);
        if (strcmp(path, LWASAPI_EP_LDAC) == 0) {
            bt->codec_id = A2DP_CODEC_VENDOR;
            bt->active_codec_kind = LWASAPI_BT_CODEC_LDAC;
            bt->supported_codecs_mask |= LWASAPI_BT_CAP_LDAC;
            snprintf(bt->codec_name, sizeof(bt->codec_name), "LDAC");
        } else if (strcmp(path, LWASAPI_EP_APTXHD) == 0) {
            bt->codec_id = A2DP_CODEC_VENDOR;
            bt->active_codec_kind = LWASAPI_BT_CODEC_APTX_HD;
            bt->supported_codecs_mask |= LWASAPI_BT_CAP_APTX_HD;
            snprintf(bt->codec_name, sizeof(bt->codec_name), "aptX HD");
        } else if (strcmp(path, LWASAPI_EP_APTX) == 0) {
            bt->codec_id = A2DP_CODEC_VENDOR;
            bt->active_codec_kind = LWASAPI_BT_CODEC_APTX;
            bt->supported_codecs_mask |= LWASAPI_BT_CAP_APTX;
            snprintf(bt->codec_name, sizeof(bt->codec_name), "aptX");
        } else if (strcmp(path, LWASAPI_EP_AAC) == 0) {
            bt->codec_id = A2DP_CODEC_MPEG24;
            bt->active_codec_kind = LWASAPI_BT_CODEC_AAC;
            bt->supported_codecs_mask |= LWASAPI_BT_CAP_AAC;
            snprintf(bt->codec_name, sizeof(bt->codec_name), "AAC");
        } else {
            bt->codec_id = A2DP_CODEC_SBC;
            bt->active_codec_kind = LWASAPI_BT_CODEC_SBC;
            bt->supported_codecs_mask |= LWASAPI_BT_CAP_SBC;
            snprintf(bt->codec_name, sizeof(bt->codec_name), "SBC");
        }
        /* Peer headset always supports baseline SBC alongside any higher codecs discovered */
        bt->supported_codecs_mask |= LWASAPI_BT_CAP_SBC;
        char codec_label[16];
        snprintf(codec_label, sizeof(codec_label), "%.15s", bt->codec_name);
        if (bt->pref_codec != LWASAPI_BT_PREF_AUTO) {
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "Switched to %.15s per user codec preference", codec_label);
        } else if (bt->active_codec_kind == LWASAPI_BT_CODEC_LDAC) {
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "Negotiated LDAC (peer + host SEP match)");
        } else if (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX_HD ||
                   bt->active_codec_kind == LWASAPI_BT_CODEC_APTX) {
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "Negotiated %.15s (headset lacks LDAC SEP)", codec_label);
        } else if (bt->active_codec_kind == LWASAPI_BT_CODEC_AAC) {
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "AAC active: peer lacked LDAC/aptX or chose AAC");
        } else {
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "SBC fallback: peer offered baseline A2DP only");
        }
        atomic_store(&bt->bt_renegotiate_fallback, 0U);
        memcpy(bt->a2dp_config, cfg, sizeof(cfg));
        bt->a2dp_config_len = cfg_len;
        bt->delay_tenths_ms = delay;
        bt->has_bluez_transport = true;
        if (has_vol) {
            bt->avrcp_volume = vol;
            bt->desktop_volume_pct = (uint32_t)((vol * 100U + 63U) / 127U);
            atomic_store(&g_avrcp_supported, 1);
        }
        pthread_mutex_unlock(&bt->state_lock);

        lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                (int32_t)delay, (int32_t)(has_vol ? vol : 0), (int32_t)bt->codec_id,
                                "BT_SET_CFG", alias,
                                "addr=%s codec=%s ep=%s pref=%u delay=%.1fms vol=%u",
                                addr, bt->codec_name, path, bt->pref_codec, (double)delay / 10.0,
                                (unsigned)(has_vol ? vol : bt->avrcp_volume));

        DBusMessage *rep = dbus_message_new_method_return(msg);
        if (rep) {
            dbus_connection_send(conn, rep, NULL);
            dbus_message_unref(rep);
        }
        /* Acquire AFTER replying to SetConfiguration so bluetoothd never deadlocks! */
        atomic_store(&g_need_acquire, 1);
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if (strcmp(member, "ClearConfiguration") == 0 || strcmp(member, "Release") == 0) {
        const char *tp = NULL;
        dbus_message_get_args(msg, NULL, DBUS_TYPE_OBJECT_PATH, &tp, DBUS_TYPE_INVALID);

        pthread_mutex_lock(&bt->state_lock);
        if (strcmp(member, "Release") == 0 || !tp || strcmp(tp, bt->transport_path) == 0) {
            if (bt->bt_fd >= 0) { close(bt->bt_fd); bt->bt_fd = -1; }
            bt->direct_socket_mode = false;
            bt->has_bluez_transport = false;
            bt->tx_stream_active = 0;
            bt->transport_path[0] = '\0';
            close_bt_encoders(bt);
        }
        pthread_mutex_unlock(&bt->state_lock);

        lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                           "BT_CLEAR_CFG", tp ? tp : path,
                           "Endpoint %s by BlueZ", member);

        DBusMessage *rep = dbus_message_new_method_return(msg);
        if (rep) {
            dbus_connection_send(conn, rep, NULL);
            dbus_message_unref(rep);
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
}

static DBusHandlerResult bluez_signal_filter(DBusConnection *conn, DBusMessage *msg, void *user_data) {
    (void)conn;
    lwasapi_bluez_t *bt = (lwasapi_bluez_t *)user_data;
    if (dbus_message_get_type(msg) != DBUS_MESSAGE_TYPE_SIGNAL) return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;

    const char *path   = dbus_message_get_path(msg);
    const char *iface  = dbus_message_get_interface(msg);
    const char *member = dbus_message_get_member(msg);
    if (!iface || !member) return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;

    if (strcmp(iface, "org.freedesktop.DBus.Properties") == 0 &&
        strcmp(member, "PropertiesChanged") == 0 &&
        path && bt->transport_path[0] && strcmp(path, bt->transport_path) == 0) {
        DBusMessageIter it, dict, entry, var;
        const char *prop_iface = NULL;
        if (dbus_message_iter_init(msg, &it) && dbus_message_iter_get_arg_type(&it) == DBUS_TYPE_STRING) {
            dbus_message_iter_get_basic(&it, &prop_iface);
            if (prop_iface && strcmp(prop_iface, "org.bluez.MediaTransport1") == 0 && dbus_message_iter_next(&it)) {
                dbus_message_iter_recurse(&it, &dict);
                while (dbus_message_iter_get_arg_type(&dict) == DBUS_TYPE_DICT_ENTRY) {
                    const char *k = NULL;
                    dbus_message_iter_recurse(&dict, &entry);
                    dbus_message_iter_get_basic(&entry, &k);
                    dbus_message_iter_next(&entry);
                    dbus_message_iter_recurse(&entry, &var);
                    if (k && strcmp(k, "Volume") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_UINT16) {
                        uint16_t v = 127;
                        dbus_message_iter_get_basic(&var, &v);
                        pthread_mutex_lock(&bt->state_lock);
                        bt->avrcp_volume = v;
                        bt->desktop_volume_pct = (uint32_t)((v * 100U + 63U) / 127U);
                        atomic_store(&g_avrcp_supported, 1);
                        uint32_t pct = bt->desktop_volume_pct;
                        pthread_mutex_unlock(&bt->state_lock);
                        lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                                (int32_t)v, (int32_t)pct, 0,
                                                "BT_AVRCP_VOL", bt->device_alias,
                                                "AVRCP Volume=%u (%u%%)", v, pct);
                    } else if (k && strcmp(k, "Delay") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_UINT16) {
                        uint16_t d = 0;
                        dbus_message_iter_get_basic(&var, &d);
                        bt->delay_tenths_ms = d;
                        lwasapi_debug_emit_vals(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_TRACE, 0, 0,
                                                (int32_t)d, 0, 0,
                                                "BT_DELAY_CHG", bt->device_alias,
                                                "Delay=%.1fms", (double)d / 10.0);
                    }
                    dbus_message_iter_next(&dict);
                }
            }
        }
    } else if (strcmp(iface, "org.freedesktop.DBus") == 0 &&
               strcmp(member, "NameOwnerChanged") == 0) {
        const char *name = NULL, *old_owner = NULL, *new_owner = NULL;
        if (dbus_message_get_args(msg, NULL,
                                  DBUS_TYPE_STRING, &name,
                                  DBUS_TYPE_STRING, &old_owner,
                                  DBUS_TYPE_STRING, &new_owner,
                                  DBUS_TYPE_INVALID) &&
            name && strcmp(name, "org.bluez") == 0) {
            (void)old_owner;
            pthread_mutex_lock(&bt->state_lock);
            if (bt->bt_fd >= 0) { close(bt->bt_fd); bt->bt_fd = -1; }
            bt->direct_socket_mode = false;
            bt->has_bluez_transport = false;
            bt->tx_stream_active = 0;
            bt->transport_path[0] = '\0';
            bt->adapter_path[0] = '\0';
            bt->endpoints_registered = false;
            close_bt_encoders(bt);
            pthread_mutex_unlock(&bt->state_lock);

            if (new_owner && new_owner[0] != '\0') {
                atomic_store(&g_need_reregister, 1);
                lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                   "BT_BLUEZ_UP", new_owner,
                                   "org.bluez acquired/restarted; scheduling MediaEndpoint1 re-registration");
            } else {
                atomic_store(&g_need_reregister, 0);
                lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                                   "BT_BLUEZ_DOWN", "org.bluez",
                                   "org.bluez exited; waiting for bluetoothd restart");
            }
        }
    } else if (strcmp(iface, "org.freedesktop.DBus.ObjectManager") == 0 &&
               strcmp(member, "InterfacesAdded") == 0) {
        const char *obj = NULL;
        DBusMessageIter it, if_arr, if_entry;
        if (dbus_message_iter_init(msg, &it) && dbus_message_iter_get_arg_type(&it) == DBUS_TYPE_OBJECT_PATH) {
            dbus_message_iter_get_basic(&it, &obj);
            if (dbus_message_iter_next(&it) && dbus_message_iter_get_arg_type(&it) == DBUS_TYPE_ARRAY) {
                dbus_message_iter_recurse(&it, &if_arr);
                while (dbus_message_iter_get_arg_type(&if_arr) == DBUS_TYPE_DICT_ENTRY) {
                    const char *added_iface = NULL;
                    dbus_message_iter_recurse(&if_arr, &if_entry);
                    dbus_message_iter_get_basic(&if_entry, &added_iface);
                    if (added_iface && strcmp(added_iface, "org.bluez.Media1") == 0) {
                        if (!bt->endpoints_registered || (obj && strcmp(obj, bt->adapter_path) != 0)) {
                            atomic_store(&g_need_reregister, 1);
                            lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                               "BT_MEDIA_ADD", obj ? obj : "/org/bluez",
                                               "org.bluez.Media1 appeared; scheduling endpoint registration");
                        }
                        break;
                    }
                    dbus_message_iter_next(&if_arr);
                }
            }
        }
    } else if (strcmp(iface, "org.freedesktop.DBus.ObjectManager") == 0 &&
               strcmp(member, "InterfacesRemoved") == 0) {
        const char *obj = NULL;
        DBusMessageIter it;
        if (dbus_message_iter_init(msg, &it) && dbus_message_iter_get_arg_type(&it) == DBUS_TYPE_OBJECT_PATH) {
            dbus_message_iter_get_basic(&it, &obj);
            if (obj && (strcmp(obj, bt->transport_path) == 0 || strcmp(obj, bt->device_path) == 0)) {
                pthread_mutex_lock(&bt->state_lock);
                if (bt->bt_fd >= 0) { close(bt->bt_fd); bt->bt_fd = -1; }
                bt->direct_socket_mode = false;
                bt->has_bluez_transport = false;
                bt->tx_stream_active = 0;
                bt->transport_path[0] = '\0';
                close_bt_encoders(bt);
                pthread_mutex_unlock(&bt->state_lock);
                lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                                   "BT_IFACE_REM", obj,
                                   "BlueZ ObjectManager removed device/transport");
            }
        }
    }
    return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
}

/* ============================================================================
 * HFP AG (0000111f) Battery Monitor & BlueZ BatteryProvider1 (/org/lwasapi/Battery)
 * ============================================================================ */

#define MAX_HFP_DEVS 8

typedef struct {
    bool    in_use;
    bool    rfcomm_active;
    bool    published;
    bool    pending_add;
    bool    pending_chg;
    bool    pending_rem;
    bool    supports_msbc;
    int     negotiated_sco_codec; /* 1=CVSD, 2=mSBC */
    int     fd;
    int     pct;
    char    dev_path[128];
    char    batt_path[160];
} hfp_battery_dev_t;

static hfp_battery_dev_t g_hfp_devs[MAX_HFP_DEVS];
static pthread_mutex_t   g_hfp_lock = PTHREAD_MUTEX_INITIALIZER;
static atomic_int        g_hfp_battery_dirty = 0;
static bool              g_hfp_obj_registered = false;
static bool              g_batt_obj_registered = false;

static void hfp_make_batt_path(const char *dev_path, char *out, size_t out_len) {
    const char *tail = strrchr(dev_path, '/');
    const char *name = (tail && tail[1]) ? (tail + 1) : "dev";
    snprintf(out, out_len, "%s/%s", LWASAPI_BATTERY_ROOT, name);
    for (size_t i = strlen(LWASAPI_BATTERY_ROOT) + 1; out[i]; i++) {
        char c = out[i];
        if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_')) {
            out[i] = '_';
        }
    }
}

static hfp_battery_dev_t *hfp_find_or_alloc_locked(const char *dev_path) {
    int free_idx = -1;
    for (int i = 0; i < MAX_HFP_DEVS; i++) {
        if (g_hfp_devs[i].in_use && strcmp(g_hfp_devs[i].dev_path, dev_path) == 0) {
            return &g_hfp_devs[i];
        }
        if (!g_hfp_devs[i].in_use && free_idx < 0) free_idx = i;
    }
    if (free_idx < 0) return NULL;
    hfp_battery_dev_t *d = &g_hfp_devs[free_idx];
    memset(d, 0, sizeof(*d));
    d->in_use = true;
    d->fd = -1;
    d->pct = -1;
    snprintf(d->dev_path, sizeof(d->dev_path), "%s", dev_path);
    hfp_make_batt_path(dev_path, d->batt_path, sizeof(d->batt_path));
    return d;
}

static void hfp_set_battery_pct(const char *dev_path, int pct) {
    if (pct < 0) pct = 0;
    if (pct > 100) pct = 100;
    pthread_mutex_lock(&g_hfp_lock);
    hfp_battery_dev_t *d = hfp_find_or_alloc_locked(dev_path);
    if (d) {
        if (!d->published) {
            d->pct = pct;
            d->pending_add = true;
            d->pending_rem = false;
            atomic_store(&g_hfp_battery_dirty, 1);
        } else if (d->pct != pct) {
            d->pct = pct;
            d->pending_chg = true;
            atomic_store(&g_hfp_battery_dirty, 1);
        }
    }
    pthread_mutex_unlock(&g_hfp_lock);
    fprintf(stderr, "[LWASAPI-HFP] Battery update for %s: %d%%\n", dev_path, pct);
}

static void hfp_mark_disconnected(const char *dev_path) {
    pthread_mutex_lock(&g_hfp_lock);
    for (int i = 0; i < MAX_HFP_DEVS; i++) {
        if (g_hfp_devs[i].in_use && strcmp(g_hfp_devs[i].dev_path, dev_path) == 0) {
            if (g_hfp_devs[i].fd >= 0) {
                close(g_hfp_devs[i].fd);
                g_hfp_devs[i].fd = -1;
            }
            g_hfp_devs[i].rfcomm_active = false;
            if (g_hfp_devs[i].published) {
                g_hfp_devs[i].pending_rem = true;
                g_hfp_devs[i].pending_add = false;
                g_hfp_devs[i].pending_chg = false;
                atomic_store(&g_hfp_battery_dirty, 1);
            } else {
                g_hfp_devs[i].in_use = false;
            }
        }
    }
    pthread_mutex_unlock(&g_hfp_lock);
}

static bool hfp_is_connected(const char *dev_path) {
    bool active = false;
    pthread_mutex_lock(&g_hfp_lock);
    for (int i = 0; i < MAX_HFP_DEVS; i++) {
        if (g_hfp_devs[i].in_use && strcmp(g_hfp_devs[i].dev_path, dev_path) == 0 &&
            g_hfp_devs[i].rfcomm_active && g_hfp_devs[i].fd >= 0) {
            active = true;
            break;
        }
    }
    pthread_mutex_unlock(&g_hfp_lock);
    return active;
}

static void append_battery_props_dict(DBusMessageIter *dict, const char *dev_path, uint8_t pct) {
    add_dict_entry_obj_path(dict, "Device", dev_path);
    add_dict_entry_byte(dict, "Percentage", pct);
    add_dict_entry_str(dict, "Source", "HFP");
}

static void hfp_flush_battery_dbus(DBusConnection *conn) {
    if (!conn || !atomic_exchange(&g_hfp_battery_dirty, 0)) return;

    pthread_mutex_lock(&g_hfp_lock);
    for (int i = 0; i < MAX_HFP_DEVS; i++) {
        hfp_battery_dev_t *d = &g_hfp_devs[i];
        if (!d->in_use) continue;

        if (d->pending_add && d->pct >= 0) {
            d->pending_add = false;
            d->pending_chg = false;
            d->published = true;
            DBusMessage *sig = dbus_message_new_signal(LWASAPI_BATTERY_ROOT,
                                                       "org.freedesktop.DBus.ObjectManager",
                                                       "InterfacesAdded");
            if (sig) {
                DBusMessageIter it, if_arr, if_ent, props;
                const char *bp = d->batt_path;
                const char *iface = "org.bluez.BatteryProvider1";
                dbus_message_iter_init_append(sig, &it);
                dbus_message_iter_append_basic(&it, DBUS_TYPE_OBJECT_PATH, &bp);
                dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "{sa{sv}}", &if_arr);
                dbus_message_iter_open_container(&if_arr, DBUS_TYPE_DICT_ENTRY, NULL, &if_ent);
                dbus_message_iter_append_basic(&if_ent, DBUS_TYPE_STRING, &iface);
                dbus_message_iter_open_container(&if_ent, DBUS_TYPE_ARRAY, "{sv}", &props);
                append_battery_props_dict(&props, d->dev_path, (uint8_t)d->pct);
                dbus_message_iter_close_container(&if_ent, &props);
                dbus_message_iter_close_container(&if_arr, &if_ent);
                dbus_message_iter_close_container(&it, &if_arr);
                dbus_connection_send(conn, sig, NULL);
                dbus_message_unref(sig);
            }
        } else if (d->pending_chg && d->published && d->pct >= 0) {
            d->pending_chg = false;
            DBusMessage *sig = dbus_message_new_signal(d->batt_path,
                                                       "org.freedesktop.DBus.Properties",
                                                       "PropertiesChanged");
            if (sig) {
                DBusMessageIter it, props, inv;
                const char *iface = "org.bluez.BatteryProvider1";
                dbus_message_iter_init_append(sig, &it);
                dbus_message_iter_append_basic(&it, DBUS_TYPE_STRING, &iface);
                dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "{sv}", &props);
                add_dict_entry_byte(&props, "Percentage", (uint8_t)d->pct);
                dbus_message_iter_close_container(&it, &props);
                dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "s", &inv);
                dbus_message_iter_close_container(&it, &inv);
                dbus_connection_send(conn, sig, NULL);
                dbus_message_unref(sig);
            }
        } else if (d->pending_rem) {
            d->pending_rem = false;
            if (d->published) {
                d->published = false;
                DBusMessage *sig = dbus_message_new_signal(LWASAPI_BATTERY_ROOT,
                                                           "org.freedesktop.DBus.ObjectManager",
                                                           "InterfacesRemoved");
                if (sig) {
                    DBusMessageIter it, arr;
                    const char *bp = d->batt_path;
                    const char *iface = "org.bluez.BatteryProvider1";
                    dbus_message_iter_init_append(sig, &it);
                    dbus_message_iter_append_basic(&it, DBUS_TYPE_OBJECT_PATH, &bp);
                    dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "s", &arr);
                    dbus_message_iter_append_basic(&arr, DBUS_TYPE_STRING, &iface);
                    dbus_message_iter_close_container(&it, &arr);
                    dbus_connection_send(conn, sig, NULL);
                    dbus_message_unref(sig);
                }
            }
            if (!d->rfcomm_active) d->in_use = false;
        }
    }
    pthread_mutex_unlock(&g_hfp_lock);
}

static DBusHandlerResult hfp_battery_message_handler(DBusConnection *conn, DBusMessage *msg, void *user_data) {
    (void)user_data;
    const char *path = dbus_message_get_path(msg);
    const char *iface = dbus_message_get_interface(msg);
    const char *member = dbus_message_get_member(msg);
    if (!path || !member) return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;

    if (strcmp(path, LWASAPI_BATTERY_ROOT) == 0 &&
        iface && strcmp(iface, "org.freedesktop.DBus.ObjectManager") == 0 &&
        strcmp(member, "GetManagedObjects") == 0) {
        DBusMessage *rep = dbus_message_new_method_return(msg);
        if (!rep) return DBUS_HANDLER_RESULT_HANDLED;
        DBusMessageIter it, obj_arr;
        dbus_message_iter_init_append(rep, &it);
        dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "{oa{sa{sv}}}", &obj_arr);
        pthread_mutex_lock(&g_hfp_lock);
        for (int i = 0; i < MAX_HFP_DEVS; i++) {
            hfp_battery_dev_t *d = &g_hfp_devs[i];
            if (!d->in_use || d->pct < 0 || d->pending_rem) continue;
            d->published = true;
            d->pending_add = false;
            DBusMessageIter obj_ent, if_arr, if_ent, props;
            const char *bp = d->batt_path;
            const char *biface = "org.bluez.BatteryProvider1";
            dbus_message_iter_open_container(&obj_arr, DBUS_TYPE_DICT_ENTRY, NULL, &obj_ent);
            dbus_message_iter_append_basic(&obj_ent, DBUS_TYPE_OBJECT_PATH, &bp);
            dbus_message_iter_open_container(&obj_ent, DBUS_TYPE_ARRAY, "{sa{sv}}", &if_arr);
            dbus_message_iter_open_container(&if_arr, DBUS_TYPE_DICT_ENTRY, NULL, &if_ent);
            dbus_message_iter_append_basic(&if_ent, DBUS_TYPE_STRING, &biface);
            dbus_message_iter_open_container(&if_ent, DBUS_TYPE_ARRAY, "{sv}", &props);
            append_battery_props_dict(&props, d->dev_path, (uint8_t)d->pct);
            dbus_message_iter_close_container(&if_ent, &props);
            dbus_message_iter_close_container(&if_arr, &if_ent);
            dbus_message_iter_close_container(&obj_ent, &if_arr);
            dbus_message_iter_close_container(&obj_arr, &obj_ent);
        }
        pthread_mutex_unlock(&g_hfp_lock);
        dbus_message_iter_close_container(&it, &obj_arr);
        dbus_connection_send(conn, rep, NULL);
        dbus_message_unref(rep);
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if (iface && strcmp(iface, "org.freedesktop.DBus.Properties") == 0 &&
        strcmp(member, "GetAll") == 0) {
        const char *req_iface = NULL;
        dbus_message_get_args(msg, NULL, DBUS_TYPE_STRING, &req_iface, DBUS_TYPE_INVALID);
        DBusMessage *rep = dbus_message_new_method_return(msg);
        if (!rep) return DBUS_HANDLER_RESULT_HANDLED;
        DBusMessageIter it, props;
        dbus_message_iter_init_append(rep, &it);
        dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "{sv}", &props);
        if (req_iface && strcmp(req_iface, "org.bluez.BatteryProvider1") == 0) {
            pthread_mutex_lock(&g_hfp_lock);
            for (int i = 0; i < MAX_HFP_DEVS; i++) {
                if (g_hfp_devs[i].in_use && strcmp(g_hfp_devs[i].batt_path, path) == 0 && g_hfp_devs[i].pct >= 0) {
                    append_battery_props_dict(&props, g_hfp_devs[i].dev_path, (uint8_t)g_hfp_devs[i].pct);
                    break;
                }
            }
            pthread_mutex_unlock(&g_hfp_lock);
        }
        dbus_message_iter_close_container(&it, &props);
        dbus_connection_send(conn, rep, NULL);
        dbus_message_unref(rep);
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
}

static void hfp_write_str(int fd, const char *s) {
    size_t len = strlen(s);
    size_t off = 0;
    while (off < len) {
        ssize_t w = write(fd, s + off, len - off);
        if (w <= 0) break;
        off += (size_t)w;
    }
}

static void hfp_set_bac_codecs(const char *dev_path, const char *list_str) {
    bool has_msbc = false;
    const char *p = list_str;
    while (p && *p) {
        if (*p == '2' && (p == list_str || p[-1] == ',' || p[-1] == ' ') &&
            (p[1] == '\0' || p[1] == ',' || p[1] == ' ')) {
            has_msbc = true;
            break;
        }
        p++;
    }
    pthread_mutex_lock(&g_hfp_lock);
    hfp_battery_dev_t *d = hfp_find_or_alloc_locked(dev_path);
    if (d) d->supports_msbc = has_msbc;
    pthread_mutex_unlock(&g_hfp_lock);
}

static void hfp_handle_at_line(int fd, const char *dev_path, const char *line) {
    while (*line == ' ' || *line == '\t') line++;
    if (!*line) return;
    fprintf(stderr, "[LWASAPI-HFP] AT << %s\n", line);

    if (strncmp(line, "AT+BRSF=", 8) == 0) {
        /* Bit 9 (512): Codec Negotiation | Bit 10 (1024): HF Indicators */
        hfp_write_str(fd, "\r\n+BRSF: 1536\r\n\r\nOK\r\n");
    } else if (strncmp(line, "AT+BAC=", 7) == 0) {
        hfp_set_bac_codecs(dev_path, line + 7);
        hfp_write_str(fd, "\r\nOK\r\n");
    } else if (strncmp(line, "AT+BCS=", 7) == 0) {
        int c = atoi(line + 7);
        pthread_mutex_lock(&g_hfp_lock);
        hfp_battery_dev_t *d = hfp_find_or_alloc_locked(dev_path);
        if (d) d->negotiated_sco_codec = (c == 2) ? 2 : 1;
        pthread_mutex_unlock(&g_hfp_lock);
        hfp_write_str(fd, "\r\nOK\r\n");
    } else if (strcmp(line, "AT+CIND=?") == 0) {
        hfp_write_str(fd, "\r\n+CIND: (\"service\",(0,1)),(\"call\",(0,1)),(\"callsetup\",(0,3)),(\"callheld\",(0,2)),(\"signal\",(0,5)),(\"roam\",(0,1)),(\"battchg\",(0,5))\r\n\r\nOK\r\n");
    } else if (strcmp(line, "AT+CIND?") == 0) {
        hfp_write_str(fd, "\r\n+CIND: 1,0,0,0,5,0,5\r\n\r\nOK\r\n");
    } else if (strncmp(line, "AT+CMER=", 8) == 0) {
        hfp_write_str(fd, "\r\nOK\r\n");
    } else if (strcmp(line, "AT+CHLD=?") == 0) {
        hfp_write_str(fd, "\r\n+CHLD: (0,1,2,3)\r\n\r\nOK\r\n");
    } else if (strcmp(line, "AT+BIND=?") == 0) {
        hfp_write_str(fd, "\r\n+BIND: (1,2)\r\n\r\nOK\r\n");
    } else if (strcmp(line, "AT+BIND?") == 0) {
        /* Enable HF indicator 2 (Battery Level) */
        hfp_write_str(fd, "\r\n+BIND: 2,1\r\n\r\nOK\r\n");
    } else if (strncmp(line, "AT+BIND=", 8) == 0) {
        hfp_write_str(fd, "\r\nOK\r\n");
    } else if (strncmp(line, "AT+BIEV=", 8) == 0) {
        int ind = 0, val = -1;
        if (sscanf(line + 8, "%d,%d", &ind, &val) == 2 && ind == 2 && val >= 0 && val <= 100) {
            hfp_set_battery_pct(dev_path, val);
        }
        hfp_write_str(fd, "\r\nOK\r\n");
    } else if (strncmp(line, "AT+XAPL=", 8) == 0) {
        /* Advertise battery reporting support (bit 1 = 2, bit 2 = 4 -> 7) */
        hfp_write_str(fd, "\r\n+XAPL=iPhone,7\r\n\r\nOK\r\n");
    } else if (strncmp(line, "AT+IPHONEACCEV=", 15) == 0) {
        const char *p = line + 15;
        char *end = NULL;
        long count = strtol(p, &end, 10);
        if (end && count > 0 && count <= 16) {
            p = end;
            for (long i = 0; i < count; i++) {
                while (*p == ',' || *p == ' ') p++;
                long key = strtol(p, &end, 10);
                if (!end || end == p) break;
                p = end;
                while (*p == ',' || *p == ' ') p++;
                long val = strtol(p, &end, 10);
                if (!end || end == p) break;
                p = end;
                if (key == 1 && val >= 0 && val <= 9) {
                    hfp_set_battery_pct(dev_path, (int)((val + 1) * 10));
                }
            }
        }
        hfp_write_str(fd, "\r\nOK\r\n");
    } else if (strncmp(line, "AT", 2) == 0) {
        hfp_write_str(fd, "\r\nOK\r\n");
    }
}

typedef struct {
    int  fd;
    char dev_path[128];
} hfp_conn_arg_t;

static void *hfp_rfcomm_thread_fn(void *arg) {
    hfp_conn_arg_t *ca = (hfp_conn_arg_t *)arg;
    int fd = ca->fd;
    char dev_path[128];
    snprintf(dev_path, sizeof(dev_path), "%s", ca->dev_path);
    free(ca);

    char line[512];
    size_t line_len = 0;
    char buf[256];

    fprintf(stderr, "[LWASAPI-HFP] RFCOMM connected for %s (fd=%d)\n", dev_path, fd);
    for (;;) {
        struct pollfd pfd = { .fd = fd, .events = POLLIN, .revents = 0 };
        int pr = poll(&pfd, 1, 1000);
        if (pr < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (pr == 0) continue;
        if (pfd.revents & (POLLERR | POLLHUP | POLLNVAL)) {
            if (!(pfd.revents & POLLIN)) break;
        }
        ssize_t n = read(fd, buf, sizeof(buf));
        if (n < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR) continue;
            fprintf(stderr, "[LWASAPI-HFP] RFCOMM read error on %s: %s\n", dev_path, strerror(errno));
            break;
        }
        if (n == 0) break;
        for (ssize_t i = 0; i < n; i++) {
            char c = buf[i];
            if (c == '\r' || c == '\n') {
                if (line_len > 0) {
                    line[line_len] = '\0';
                    hfp_handle_at_line(fd, dev_path, line);
                    line_len = 0;
                }
            } else if (line_len + 1 < sizeof(line)) {
                line[line_len++] = c;
            }
        }
    }

    fprintf(stderr, "[LWASAPI-HFP] RFCOMM disconnected for %s\n", dev_path);
    hfp_mark_disconnected(dev_path);
    return NULL;
}

static DBusHandlerResult hfp_profile_message_handler(DBusConnection *conn, DBusMessage *msg, void *user_data) {
    (void)user_data;
    const char *iface = dbus_message_get_interface(msg);
    const char *member = dbus_message_get_member(msg);
    if (!iface || strcmp(iface, "org.bluez.Profile1") != 0 || !member) {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    }

    if (strcmp(member, "Release") == 0) {
        DBusMessage *rep = dbus_message_new_method_return(msg);
        if (rep) { dbus_connection_send(conn, rep, NULL); dbus_message_unref(rep); }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if (strcmp(member, "RequestDisconnection") == 0) {
        const char *dev_path = NULL;
        if (dbus_message_get_args(msg, NULL, DBUS_TYPE_OBJECT_PATH, &dev_path, DBUS_TYPE_INVALID) && dev_path) {
            hfp_mark_disconnected(dev_path);
        }
        DBusMessage *rep = dbus_message_new_method_return(msg);
        if (rep) { dbus_connection_send(conn, rep, NULL); dbus_message_unref(rep); }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if (strcmp(member, "NewConnection") == 0) {
        const char *dev_path = NULL;
        int fd = -1;
        if (dbus_message_get_args(msg, NULL,
                                  DBUS_TYPE_OBJECT_PATH, &dev_path,
                                  DBUS_TYPE_UNIX_FD, &fd,
                                  DBUS_TYPE_INVALID) && dev_path && fd >= 0) {
            hfp_mark_disconnected(dev_path);
            hfp_conn_arg_t *ca = (hfp_conn_arg_t *)malloc(sizeof(hfp_conn_arg_t));
            if (ca) {
                ca->fd = fd;
                snprintf(ca->dev_path, sizeof(ca->dev_path), "%s", dev_path);
                pthread_mutex_lock(&g_hfp_lock);
                hfp_battery_dev_t *d = hfp_find_or_alloc_locked(dev_path);
                if (d) {
                    d->fd = fd;
                    d->rfcomm_active = true;
                }
                pthread_mutex_unlock(&g_hfp_lock);

                pthread_t tid;
                pthread_attr_t attr;
                pthread_attr_init(&attr);
                pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
                if (pthread_create(&tid, &attr, hfp_rfcomm_thread_fn, ca) != 0) {
                    free(ca);
                    hfp_mark_disconnected(dev_path);
                }
                pthread_attr_destroy(&attr);
            } else {
                close(fd);
            }
        }
        DBusMessage *rep = dbus_message_new_method_return(msg);
        if (rep) { dbus_connection_send(conn, rep, NULL); dbus_message_unref(rep); }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
}

static void register_hfp_and_battery_provider(DBusConnection *conn, const char adapters[][64], int n_adapters) {
    static const DBusObjectPathVTable hfp_vtbl = { .message_function = hfp_profile_message_handler };
    static const DBusObjectPathVTable batt_vtbl = { .message_function = hfp_battery_message_handler };

    if (!g_hfp_obj_registered) {
        if (dbus_connection_register_object_path(conn, LWASAPI_HFP_AG_PATH, &hfp_vtbl, NULL)) {
            g_hfp_obj_registered = true;
        }
    }
    if (!g_batt_obj_registered) {
        if (dbus_connection_register_fallback(conn, LWASAPI_BATTERY_ROOT, &batt_vtbl, NULL)) {
            g_batt_obj_registered = true;
        }
    }

    /* Register HFP AG Profile1 with BlueZ ProfileManager1 */
    DBusMessage *pmsg = dbus_message_new_method_call("org.bluez", "/org/bluez",
                                                      "org.bluez.ProfileManager1", "RegisterProfile");
    if (pmsg) {
        const char *ppath = LWASAPI_HFP_AG_PATH;
        const char *uuid = HFP_AG_UUID;
        DBusMessageIter it, dict;
        dbus_message_iter_init_append(pmsg, &it);
        dbus_message_iter_append_basic(&it, DBUS_TYPE_OBJECT_PATH, &ppath);
        dbus_message_iter_append_basic(&it, DBUS_TYPE_STRING, &uuid);
        dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "{sv}", &dict);
        add_dict_entry_str(&dict, "Name", "LWASAPI HFP Battery");
        add_dict_entry_u16(&dict, "Version", 0x0107);
        add_dict_entry_u16(&dict, "Features", 0x0009);
        add_dict_entry_bool(&dict, "AutoConnect", TRUE);
        dbus_message_iter_close_container(&it, &dict);
        DBusError err;
        dbus_error_init(&err);
        DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, pmsg, 2000, &err);
        dbus_message_unref(pmsg);
        if (rep) {
            dbus_message_unref(rep);
            printf("[LWASAPI-HFP] Registered HFP AG profile %s\n", LWASAPI_HFP_AG_PATH);
            fflush(stdout);
        } else {
            fprintf(stderr, "[LWASAPI-HFP] RegisterProfile failed: %s\n", err.message ? err.message : "unknown");
            dbus_error_free(&err);
        }
    }

    /* Register BatteryProvider1 with each adapter's BatteryProviderManager1 */
    for (int i = 0; i < n_adapters; i++) {
        DBusMessage *bmsg = dbus_message_new_method_call("org.bluez", adapters[i],
                                                          "org.bluez.BatteryProviderManager1", "RegisterBatteryProvider");
        if (!bmsg) continue;
        const char *broot = LWASAPI_BATTERY_ROOT;
        dbus_message_append_args(bmsg, DBUS_TYPE_OBJECT_PATH, &broot, DBUS_TYPE_INVALID);
        DBusError err;
        dbus_error_init(&err);
        DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, bmsg, 2000, &err);
        dbus_message_unref(bmsg);
        if (rep) {
            dbus_message_unref(rep);
            printf("[LWASAPI-HFP] Registered BatteryProvider %s on %s\n", LWASAPI_BATTERY_ROOT, adapters[i]);
            fflush(stdout);
        } else {
            fprintf(stderr, "[LWASAPI-HFP] RegisterBatteryProvider on %s failed: %s\n",
                    adapters[i], err.message ? err.message : "unknown");
            dbus_error_free(&err);
        }
    }
}

static void register_endpoint_on_adapter(DBusConnection *conn, const char *adapter,
                                         const char *ep_path, uint8_t codec,
                                         const uint8_t *caps, int caps_len) {
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", adapter,
                                                     "org.bluez.Media1", "RegisterEndpoint");
    if (!msg) return;
    DBusMessageIter it, dict;
    dbus_message_iter_init_append(msg, &it);
    dbus_message_iter_append_basic(&it, DBUS_TYPE_OBJECT_PATH, &ep_path);
    dbus_message_iter_open_container(&it, DBUS_TYPE_ARRAY, "{sv}", &dict);
    add_dict_entry_str(&dict, "UUID", A2DP_SOURCE_UUID);
    add_dict_entry_byte(&dict, "Codec", codec);
    add_dict_entry_bool(&dict, "DelayReporting", TRUE);
    add_dict_entry_bytes(&dict, "Capabilities", caps, caps_len);
    dbus_message_iter_close_container(&it, &dict);

    DBusError err;
    dbus_error_init(&err);
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 2000, &err);
    dbus_message_unref(msg);
    const char *cname = (strcmp(ep_path, LWASAPI_EP_LDAC) == 0) ? "LDAC" :
                        (strcmp(ep_path, LWASAPI_EP_APTXHD) == 0) ? "aptX HD" :
                        (strcmp(ep_path, LWASAPI_EP_APTX) == 0) ? "aptX" :
                        (codec == A2DP_CODEC_MPEG24) ? "AAC" : "SBC";
    if (rep) {
        dbus_message_unref(rep);
        printf("[LWASAPI-BlueZ] Registered A2DP %s endpoint %s on %s\n", cname, ep_path, adapter);
        lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                           "BT_EP_REG", adapter, "Registered %s (%s)", ep_path, cname);
    } else {
        lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                           "BT_EP_ERR", adapter, "RegisterEndpoint %s failed: %s",
                           ep_path, err.message ? err.message : "unknown");
        dbus_error_free(&err);
    }
}

static void register_all_adapters_and_kick_connected(DBusConnection *conn, lwasapi_bluez_t *bt) {
    g_bt_kick_dev[0] = '\0';
    g_bt_startup_autoconnect_dev[0] = '\0';

    DBusMessage *msg = dbus_message_new_method_call("org.bluez", "/",
                                                     "org.freedesktop.DBus.ObjectManager", "GetManagedObjects");
    if (!msg) return;
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 2000, NULL);
    dbus_message_unref(msg);
    if (!rep) return;

    char adapters[4][64];
    int  n_adapters = 0;
    char kick_devices[4][128];
    int  n_kick = 0;
    char hfp_devices[4][128];
    int  n_hfp = 0;
    bool kick_matches_pref = false;

    pthread_mutex_lock(&bt->state_lock);
    char pref_mac[32];
    snprintf(pref_mac, sizeof(pref_mac), "%s", bt->preferred_mac);
    pthread_mutex_unlock(&bt->state_lock);

    DBusMessageIter root, obj_arr, obj_entry, if_arr, if_entry, prop_arr, prop_entry, var;
    if (dbus_message_iter_init(rep, &root) && dbus_message_iter_get_arg_type(&root) == DBUS_TYPE_ARRAY) {
        dbus_message_iter_recurse(&root, &obj_arr);
        while (dbus_message_iter_get_arg_type(&obj_arr) == DBUS_TYPE_DICT_ENTRY) {
            const char *obj_path = NULL;
            dbus_message_iter_recurse(&obj_arr, &obj_entry);
            dbus_message_iter_get_basic(&obj_entry, &obj_path);
            dbus_message_iter_next(&obj_entry);
            dbus_message_iter_recurse(&obj_entry, &if_arr);
            while (dbus_message_iter_get_arg_type(&if_arr) == DBUS_TYPE_DICT_ENTRY) {
                const char *iface = NULL;
                dbus_message_iter_recurse(&if_arr, &if_entry);
                dbus_message_iter_get_basic(&if_entry, &iface);
                dbus_message_iter_next(&if_entry);

                if (iface && strcmp(iface, "org.bluez.Media1") == 0 && n_adapters < 4 && obj_path) {
                    snprintf(adapters[n_adapters++], 64, "%s", obj_path);
                } else if (iface && strcmp(iface, "org.bluez.Device1") == 0 && obj_path) {
                    dbus_bool_t connected = FALSE, paired = FALSE, trusted = FALSE;
                    bool has_a2dp_sink = false;
                    bool has_hfp_hf = false;
                    char addr[32] = {0};
                    dbus_message_iter_recurse(&if_entry, &prop_arr);
                    while (dbus_message_iter_get_arg_type(&prop_arr) == DBUS_TYPE_DICT_ENTRY) {
                        const char *pk = NULL;
                        dbus_message_iter_recurse(&prop_arr, &prop_entry);
                        dbus_message_iter_get_basic(&prop_entry, &pk);
                        dbus_message_iter_next(&prop_entry);
                        dbus_message_iter_recurse(&prop_entry, &var);
                        int vtype = dbus_message_iter_get_arg_type(&var);
                        if (pk && strcmp(pk, "Connected") == 0 && vtype == DBUS_TYPE_BOOLEAN) {
                            dbus_message_iter_get_basic(&var, &connected);
                        } else if (pk && strcmp(pk, "Paired") == 0 && vtype == DBUS_TYPE_BOOLEAN) {
                            dbus_message_iter_get_basic(&var, &paired);
                        } else if (pk && strcmp(pk, "Trusted") == 0 && vtype == DBUS_TYPE_BOOLEAN) {
                            dbus_message_iter_get_basic(&var, &trusted);
                        } else if (pk && strcmp(pk, "Address") == 0 && vtype == DBUS_TYPE_STRING) {
                            const char *s = NULL;
                            dbus_message_iter_get_basic(&var, &s);
                            if (s) snprintf(addr, sizeof(addr), "%s", s);
                        } else if (pk && strcmp(pk, "UUIDs") == 0 && vtype == DBUS_TYPE_ARRAY) {
                            DBusMessageIter uarr;
                            dbus_message_iter_recurse(&var, &uarr);
                            while (dbus_message_iter_get_arg_type(&uarr) == DBUS_TYPE_STRING) {
                                const char *u = NULL;
                                dbus_message_iter_get_basic(&uarr, &u);
                                if (u && strncasecmp(u, "0000110b", 8) == 0) has_a2dp_sink = true;
                                if (u && strncasecmp(u, "0000111e", 8) == 0) has_hfp_hf = true;
                                dbus_message_iter_next(&uarr);
                            }
                        }
                        dbus_message_iter_next(&prop_arr);
                    }
                    bool matches_pref = (pref_mac[0] && addr[0] && strcasecmp(addr, pref_mac) == 0);
                    if (connected && has_hfp_hf && n_hfp < 4) {
                        snprintf(hfp_devices[n_hfp++], 128, "%s", obj_path);
                    }
                    if (connected && has_a2dp_sink && n_kick < 4) {
                        snprintf(kick_devices[n_kick++], 128, "%s", obj_path);
                        if (!g_bt_kick_dev[0] || (matches_pref && !kick_matches_pref)) {
                            snprintf(g_bt_kick_dev, sizeof(g_bt_kick_dev), "%s", obj_path);
                            kick_matches_pref = matches_pref;
                        }
                    } else if (!connected && has_a2dp_sink && paired && trusted && matches_pref && !g_bt_startup_autoconnect_dev[0]) {
                        snprintf(g_bt_startup_autoconnect_dev, sizeof(g_bt_startup_autoconnect_dev), "%s", obj_path);
                    }
                }
                dbus_message_iter_next(&if_arr);
            }
            dbus_message_iter_next(&obj_arr);
        }
    }
    dbus_message_unref(rep);

    bool ldac_ok = load_ldac_library();
    bool aptx_ok = load_aptx_library();
    bool aac_ok  = load_fdk_aac_library();

    a2dp_ldac_t ldac_caps;
    memset(&ldac_caps, 0, sizeof(ldac_caps));
    A2DP_SET_VENDOR_HEADER(ldac_caps.info, LDAC_VENDOR_ID, LDAC_CODEC_ID);
    ldac_caps.frequency    = LDAC_SAMPLING_FREQ_44100 | LDAC_SAMPLING_FREQ_48000 |
                             LDAC_SAMPLING_FREQ_88200 | LDAC_SAMPLING_FREQ_96000;
    ldac_caps.channel_mode = LDAC_CHANNEL_MODE_MONO | LDAC_CHANNEL_MODE_DUAL |
                             LDAC_CHANNEL_MODE_STEREO;

    a2dp_aptx_hd_t aptx_hd_caps;
    memset(&aptx_hd_caps, 0, sizeof(aptx_hd_caps));
    A2DP_SET_VENDOR_HEADER(aptx_hd_caps.aptx.info, APTX_HD_VENDOR_ID, APTX_HD_CODEC_ID);
    aptx_hd_caps.aptx.frequency    = APTX_SAMPLING_FREQ_16000 | APTX_SAMPLING_FREQ_32000 |
                                     APTX_SAMPLING_FREQ_44100 | APTX_SAMPLING_FREQ_48000;
    aptx_hd_caps.aptx.channel_mode = APTX_CHANNEL_MODE_MONO | APTX_CHANNEL_MODE_STEREO;

    a2dp_aptx_t aptx_caps;
    memset(&aptx_caps, 0, sizeof(aptx_caps));
    A2DP_SET_VENDOR_HEADER(aptx_caps.info, APTX_VENDOR_ID, APTX_CODEC_ID);
    aptx_caps.frequency    = APTX_SAMPLING_FREQ_16000 | APTX_SAMPLING_FREQ_32000 |
                             APTX_SAMPLING_FREQ_44100 | APTX_SAMPLING_FREQ_48000;
    aptx_caps.channel_mode = APTX_CHANNEL_MODE_MONO | APTX_CHANNEL_MODE_STEREO;

    /* Capabilities: MPEG2+MPEG4 LC (0xC0), 44.1/48kHz (0x01, 0x80), 1ch+2ch (0x0C) => 0x8C, VBR+256kbps (0x83, 0xE8, 0x00) */
    const uint8_t aac_caps[6] = { 0xC0, 0x01, 0x8C, 0x83, 0xE8, 0x00 };
    const uint8_t sbc_caps[4] = { 0x3F, 0xFF, 0x02, 0x53 };

    static const DBusObjectPathVTable vtbl = { .message_function = endpoint_message_handler };

    if (ldac_ok && !g_ep_ldac_obj_registered) {
        if (dbus_connection_register_object_path(conn, LWASAPI_EP_LDAC, &vtbl, bt)) {
            g_ep_ldac_obj_registered = true;
        }
    }
    if (aptx_ok && !g_ep_aptxhd_obj_registered) {
        if (dbus_connection_register_object_path(conn, LWASAPI_EP_APTXHD, &vtbl, bt)) {
            g_ep_aptxhd_obj_registered = true;
        }
    }
    if (aptx_ok && !g_ep_aptx_obj_registered) {
        if (dbus_connection_register_object_path(conn, LWASAPI_EP_APTX, &vtbl, bt)) {
            g_ep_aptx_obj_registered = true;
        }
    }
    if (aac_ok && !g_ep_aac_obj_registered) {
        if (dbus_connection_register_object_path(conn, LWASAPI_EP_AAC, &vtbl, bt)) {
            g_ep_aac_obj_registered = true;
        }
    }
    if (!g_ep_sbc_obj_registered) {
        if (dbus_connection_register_object_path(conn, LWASAPI_EP_SBC, &vtbl, bt)) {
            g_ep_sbc_obj_registered = true;
        }
    }

    for (int i = 0; i < n_adapters; i++) {
        if (i == 0) snprintf(bt->adapter_path, sizeof(bt->adapter_path), "%.63s", adapters[i]);
        if (ldac_ok) {
            register_endpoint_on_adapter(conn, adapters[i], LWASAPI_EP_LDAC, A2DP_CODEC_VENDOR,
                                         (const uint8_t *)&ldac_caps, (int)sizeof(ldac_caps));
        }
        if (aptx_ok) {
            register_endpoint_on_adapter(conn, adapters[i], LWASAPI_EP_APTXHD, A2DP_CODEC_VENDOR,
                                         (const uint8_t *)&aptx_hd_caps, (int)sizeof(aptx_hd_caps));
            register_endpoint_on_adapter(conn, adapters[i], LWASAPI_EP_APTX, A2DP_CODEC_VENDOR,
                                         (const uint8_t *)&aptx_caps, (int)sizeof(aptx_caps));
        }
        if (aac_ok) register_endpoint_on_adapter(conn, adapters[i], LWASAPI_EP_AAC, A2DP_CODEC_MPEG24, aac_caps, 6);
        register_endpoint_on_adapter(conn, adapters[i], LWASAPI_EP_SBC, A2DP_CODEC_SBC, sbc_caps, 4);
    }
    if (n_adapters > 0) {
        register_hfp_and_battery_provider(conn, adapters, n_adapters);
    }
    bt->endpoints_registered = (n_adapters > 0);

    /* Dispatch any immediate SetConfiguration before re-kicking unconfigured A2DP devices */
    for (int t = 0; t < 10; t++) dbus_connection_read_write_dispatch(conn, 20);

    for (int i = 0; i < n_hfp; i++) {
        if (!hfp_is_connected(hfp_devices[i])) {
            const char *uuid = HFP_HF_UUID;
            DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", hfp_devices[i],
                                                              "org.bluez.Device1", "ConnectProfile");
            if (cmsg) {
                dbus_message_append_args(cmsg, DBUS_TYPE_STRING, &uuid, DBUS_TYPE_INVALID);
                dbus_connection_send(conn, cmsg, NULL);
                dbus_message_unref(cmsg);
            }
        }
    }

    if (!bt->has_bluez_transport) {
        for (int i = 0; i < n_kick; i++) {
            dbus_set_str_prop(conn, kick_devices[i], "org.bluez.Device1", "PreferredBearer", "bredr");
            const char *uuid = A2DP_SINK_UUID;
            DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", kick_devices[i],
                                                              "org.bluez.Device1", "ConnectProfile");
            if (cmsg) {
                dbus_message_append_args(cmsg, DBUS_TYPE_STRING, &uuid, DBUS_TYPE_INVALID);
                dbus_connection_send(conn, cmsg, NULL);
                dbus_message_unref(cmsg);
            }
        }
    }
}

static bool probe_remote_seps_and_match(DBusConnection *conn, lwasapi_bluez_t *bt,
                                        uint32_t want_cap, char *out_sep_path, size_t sep_sz,
                                        uint8_t *out_caps, int *out_caps_len) {
    if (out_sep_path && sep_sz > 0) out_sep_path[0] = '\0';
    if (out_caps_len) *out_caps_len = 0;
    if (!conn || !bt) return false;

    DBusMessage *msg = dbus_message_new_method_call("org.bluez", "/",
                                                     "org.freedesktop.DBus.ObjectManager", "GetManagedObjects");
    if (!msg) return false;
    DBusMessage *rep = dbus_connection_send_with_reply_and_block(conn, msg, 1500, NULL);
    dbus_message_unref(msg);
    if (!rep) return false;

    pthread_mutex_lock(&bt->state_lock);
    char target_dev[128];
    snprintf(target_dev, sizeof(target_dev), "%s", bt->device_path);
    pthread_mutex_unlock(&bt->state_lock);

    uint32_t mask = 0;
    bool found_match = false;
    DBusMessageIter root, obj_arr, obj_entry, if_arr, if_entry, prop_arr, prop_entry, var;
    if (dbus_message_iter_init(rep, &root) && dbus_message_iter_get_arg_type(&root) == DBUS_TYPE_ARRAY) {
        dbus_message_iter_recurse(&root, &obj_arr);
        while (dbus_message_iter_get_arg_type(&obj_arr) == DBUS_TYPE_DICT_ENTRY) {
            const char *obj_path = NULL;
            dbus_message_iter_recurse(&obj_arr, &obj_entry);
            dbus_message_iter_get_basic(&obj_entry, &obj_path);
            dbus_message_iter_next(&obj_entry);
            dbus_message_iter_recurse(&obj_entry, &if_arr);
            while (dbus_message_iter_get_arg_type(&if_arr) == DBUS_TYPE_DICT_ENTRY) {
                const char *iface = NULL;
                dbus_message_iter_recurse(&if_arr, &if_entry);
                dbus_message_iter_get_basic(&if_entry, &iface);
                dbus_message_iter_next(&if_entry);
                if (iface && strcmp(iface, "org.bluez.MediaEndpoint1") == 0 && obj_path &&
                    (!target_dev[0] || strncmp(obj_path, target_dev, strlen(target_dev)) == 0)) {
                    uint8_t codec = 0xFF;
                    uint8_t caps[16] = {0};
                    int caps_len = 0;
                    bool is_sink_sep = false;
                    dbus_message_iter_recurse(&if_entry, &prop_arr);
                    while (dbus_message_iter_get_arg_type(&prop_arr) == DBUS_TYPE_DICT_ENTRY) {
                        const char *pk = NULL;
                        dbus_message_iter_recurse(&prop_arr, &prop_entry);
                        dbus_message_iter_get_basic(&prop_entry, &pk);
                        dbus_message_iter_next(&prop_entry);
                        dbus_message_iter_recurse(&prop_entry, &var);
                        if (pk && strcmp(pk, "UUID") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_STRING) {
                            const char *u = NULL;
                            dbus_message_iter_get_basic(&var, &u);
                            if (u && strncasecmp(u, "0000110b", 8) == 0) is_sink_sep = true;
                        } else if (pk && strcmp(pk, "Codec") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_BYTE) {
                            dbus_message_iter_get_basic(&var, &codec);
                        } else if (pk && strcmp(pk, "Capabilities") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_ARRAY) {
                            DBusMessageIter arr;
                            const uint8_t *ptr = NULL;
                            int n = 0;
                            dbus_message_iter_recurse(&var, &arr);
                            dbus_message_iter_get_fixed_array(&arr, &ptr, &n);
                            if (ptr && n > 0) {
                                caps_len = n > 16 ? 16 : n;
                                memcpy(caps, ptr, (size_t)caps_len);
                            }
                        }
                        dbus_message_iter_next(&prop_arr);
                    }
                    if (is_sink_sep) {
                        uint32_t sep_cap = 0;
                        if (codec == A2DP_CODEC_SBC) sep_cap = LWASAPI_BT_CAP_SBC;
                        else if (codec == A2DP_CODEC_MPEG24) sep_cap = LWASAPI_BT_CAP_AAC;
                        else if (codec == A2DP_CODEC_VENDOR && caps_len >= 6) {
                            a2dp_vendor_codec_t vc;
                            memcpy(&vc, caps, sizeof(vc));
                            uint32_t vid = A2DP_GET_VENDOR_ID(vc);
                            uint16_t cid = A2DP_GET_CODEC_ID(vc);
                            if (vid == LDAC_VENDOR_ID && cid == LDAC_CODEC_ID) sep_cap = LWASAPI_BT_CAP_LDAC;
                            else if (vid == APTX_HD_VENDOR_ID && cid == APTX_HD_CODEC_ID) sep_cap = LWASAPI_BT_CAP_APTX_HD;
                            else if (vid == APTX_VENDOR_ID && cid == APTX_CODEC_ID) sep_cap = LWASAPI_BT_CAP_APTX;
                        }
                        mask |= sep_cap;
                        if (want_cap && sep_cap == want_cap && !found_match && caps_len > 0) {
                            found_match = true;
                            if (out_sep_path && sep_sz > 0) {
                                snprintf(out_sep_path, sep_sz, "%s", obj_path);
                            }
                            if (out_caps && out_caps_len) {
                                memcpy(out_caps, caps, (size_t)caps_len);
                                *out_caps_len = caps_len;
                            }
                        }
                    }
                }
                dbus_message_iter_next(&if_arr);
            }
            dbus_message_iter_next(&obj_arr);
        }
    }
    dbus_message_unref(rep);
    if (mask) {
        pthread_mutex_lock(&bt->state_lock);
        bt->supported_codecs_mask = mask;
        pthread_mutex_unlock(&bt->state_lock);
    }
    return found_match;
}

static void probe_remote_seps_on_dbus(DBusConnection *conn, lwasapi_bluez_t *bt) {
    (void)probe_remote_seps_and_match(conn, bt, 0U, NULL, 0, NULL, NULL);
}

static int send_remote_sep_set_config(DBusConnection *conn, const char *sep_path,
                                      const char *local_ep, const uint8_t *cfg_bytes, int cfg_len,
                                      DBusPendingCall **out_pending) {
    if (!conn || !sep_path || !sep_path[0] || !local_ep || !cfg_bytes || cfg_len <= 0 || !out_pending) {
        return -1;
    }
    *out_pending = NULL;
    DBusMessage *msg = dbus_message_new_method_call("org.bluez", sep_path,
                                                    "org.bluez.MediaEndpoint1", "SetConfiguration");
    if (!msg) return -1;

    DBusMessageIter args, dict, entry, var, arr;
    dbus_message_iter_init_append(msg, &args);
    dbus_message_iter_append_basic(&args, DBUS_TYPE_OBJECT_PATH, &local_ep);
    dbus_message_iter_open_container(&args, DBUS_TYPE_ARRAY, "{sv}", &dict);
    dbus_message_iter_open_container(&dict, DBUS_TYPE_DICT_ENTRY, NULL, &entry);
    const char *k = "Capabilities";
    dbus_message_iter_append_basic(&entry, DBUS_TYPE_STRING, &k);
    dbus_message_iter_open_container(&entry, DBUS_TYPE_VARIANT, "ay", &var);
    dbus_message_iter_open_container(&var, DBUS_TYPE_ARRAY, "y", &arr);
    dbus_message_iter_append_fixed_array(&arr, DBUS_TYPE_BYTE, &cfg_bytes, cfg_len);
    dbus_message_iter_close_container(&var, &arr);
    dbus_message_iter_close_container(&entry, &var);
    dbus_message_iter_close_container(&dict, &entry);
    dbus_message_iter_close_container(&args, &dict);

    dbus_bool_t ok = dbus_connection_send_with_reply(conn, msg, out_pending, 4000);
    dbus_message_unref(msg);
    dbus_connection_flush(conn);
    return (ok && *out_pending) ? 0 : -1;
}

static void *bt_dbus_thread_fn(void *arg) {
    lwasapi_bluez_t *bt = (lwasapi_bluez_t *)arg;
    dbus_threads_init_default();
    g_ep_ldac_obj_registered = false;
    g_ep_aptxhd_obj_registered = false;
    g_ep_aptx_obj_registered = false;
    g_ep_aac_obj_registered = false;
    g_ep_sbc_obj_registered = false;
    g_hfp_obj_registered = false;
    g_batt_obj_registered = false;
    atomic_store(&g_need_reregister, 0);

    DBusError err;
    dbus_error_init(&err);
    DBusConnection *conn = dbus_bus_get_private(DBUS_BUS_SYSTEM, &err);
    if (!conn) {
        dbus_error_free(&err);
        return NULL;
    }
    dbus_connection_set_exit_on_disconnect(conn, FALSE);
    bt->dbus_conn = conn;

    dbus_bus_add_match(conn, "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.bluez'", NULL);
    dbus_bus_add_match(conn, "type='signal',sender='org.bluez',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'", NULL);
    dbus_bus_add_match(conn, "type='signal',sender='org.bluez',interface='org.freedesktop.DBus.ObjectManager',member='InterfacesAdded'", NULL);
    dbus_bus_add_match(conn, "type='signal',sender='org.bluez',interface='org.freedesktop.DBus.ObjectManager',member='InterfacesRemoved'", NULL);
    dbus_connection_add_filter(conn, bluez_signal_filter, bt, NULL);

    register_all_adapters_and_kick_connected(conn, bt);
    probe_remote_seps_on_dbus(conn, bt);

    uint64_t thread_start_ns = bt_monotonic_ns();
    uint64_t no_transport_since_ns = thread_start_ns;
    uint64_t last_rereg_attempt_ns = thread_start_ns;
    bool autoconnect_done = false;
    bool kick_retried = false;
    bool kick_cycled = false;
    uint64_t last_acquire_retry_ns = 0;
    DBusPendingCall *reneg_pending = NULL;

    while (bt->dbus_thread_running) {
        dbus_connection_read_write_dispatch(conn, 50);
        hfp_flush_battery_dbus(conn);

        uint64_t loop_now_ns = bt_monotonic_ns();

        if (reneg_pending && dbus_pending_call_get_completed(reneg_pending)) {
            DBusMessage *rrep = dbus_pending_call_steal_reply(reneg_pending);
            dbus_pending_call_unref(reneg_pending);
            reneg_pending = NULL;
            if (rrep) {
                if (dbus_message_get_type(rrep) == DBUS_MESSAGE_TYPE_ERROR) {
                    const char *emsg = NULL;
                    dbus_message_get_args(rrep, NULL, DBUS_TYPE_STRING, &emsg, DBUS_TYPE_INVALID);
                    lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                                       "BT_RENEG_ERR", bt->device_path,
                                       "Remote SetConfiguration failed: %s", emsg ? emsg : "BlueZ error");
                    pthread_mutex_lock(&bt->state_lock);
                    snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                             "Codec rejected (%.28s); restoring",
                             emsg ? emsg : "BlueZ error");
                    bt->pref_codec = LWASAPI_BT_PREF_AUTO;
                    char dpath[128];
                    snprintf(dpath, sizeof(dpath), "%s", bt->device_path);
                    pthread_mutex_unlock(&bt->state_lock);
                    if (dpath[0] && !bt->has_bluez_transport) {
                        const char *uuid = A2DP_SINK_UUID;
                        DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", dpath,
                                                                          "org.bluez.Device1", "ConnectProfile");
                        if (cmsg) {
                            dbus_message_append_args(cmsg, DBUS_TYPE_STRING, &uuid, DBUS_TYPE_INVALID);
                            dbus_connection_send(conn, cmsg, NULL);
                            dbus_message_unref(cmsg);
                        }
                    }
                }
                dbus_message_unref(rrep);
            }
            atomic_store(&bt->bt_renegotiate_fallback, 0U);
        }

        if (atomic_exchange(&bt->bt_renegotiate_req, 0)) {
            if (reneg_pending) {
                dbus_pending_call_cancel(reneg_pending);
                dbus_pending_call_unref(reneg_pending);
                reneg_pending = NULL;
            }
            probe_remote_seps_on_dbus(conn, bt);

            pthread_mutex_lock(&bt->state_lock);
            char dpath[128], old_tp[160];
            snprintf(dpath, sizeof(dpath), "%s", bt->device_path);
            snprintf(old_tp, sizeof(old_tp), "%s", bt->transport_path);
            uint32_t want_pref = bt->pref_codec;
            uint32_t sup_mask  = bt->supported_codecs_mask;
            uint32_t cur_kind  = bt->active_codec_kind;
            bool     had_tp    = bt->has_bluez_transport;
            pthread_mutex_unlock(&bt->state_lock);

            uint32_t target_cap = 0;
            const char *local_ep = LWASAPI_EP_SBC;
            if (want_pref == LWASAPI_BT_PREF_AUTO) {
                if ((sup_mask & LWASAPI_BT_CAP_LDAC) && load_ldac_library()) {
                    target_cap = LWASAPI_BT_CAP_LDAC;
                    local_ep = LWASAPI_EP_LDAC;
                } else if ((sup_mask & LWASAPI_BT_CAP_APTX_HD) && load_aptx_library()) {
                    target_cap = LWASAPI_BT_CAP_APTX_HD;
                    local_ep = LWASAPI_EP_APTXHD;
                } else if ((sup_mask & LWASAPI_BT_CAP_APTX) && load_aptx_library()) {
                    target_cap = LWASAPI_BT_CAP_APTX;
                    local_ep = LWASAPI_EP_APTX;
                } else if ((sup_mask & LWASAPI_BT_CAP_AAC) && load_fdk_aac_library()) {
                    target_cap = LWASAPI_BT_CAP_AAC;
                    local_ep = LWASAPI_EP_AAC;
                } else {
                    target_cap = LWASAPI_BT_CAP_SBC;
                    local_ep = LWASAPI_EP_SBC;
                }
            } else if (want_pref == LWASAPI_BT_PREF_LDAC) {
                target_cap = LWASAPI_BT_CAP_LDAC;
                local_ep = LWASAPI_EP_LDAC;
            } else if (want_pref == LWASAPI_BT_PREF_APTX_HD) {
                target_cap = LWASAPI_BT_CAP_APTX_HD;
                local_ep = LWASAPI_EP_APTXHD;
            } else if (want_pref == LWASAPI_BT_PREF_APTX) {
                target_cap = LWASAPI_BT_CAP_APTX;
                local_ep = LWASAPI_EP_APTX;
            } else if (want_pref == LWASAPI_BT_PREF_AAC) {
                target_cap = LWASAPI_BT_CAP_AAC;
                local_ep = LWASAPI_EP_AAC;
            } else {
                target_cap = LWASAPI_BT_CAP_SBC;
                local_ep = LWASAPI_EP_SBC;
            }

            uint32_t cur_cap = (cur_kind == LWASAPI_BT_CODEC_LDAC)    ? LWASAPI_BT_CAP_LDAC :
                               (cur_kind == LWASAPI_BT_CODEC_APTX_HD) ? LWASAPI_BT_CAP_APTX_HD :
                               (cur_kind == LWASAPI_BT_CODEC_APTX)    ? LWASAPI_BT_CAP_APTX :
                               (cur_kind == LWASAPI_BT_CODEC_AAC)     ? LWASAPI_BT_CAP_AAC :
                                                                        LWASAPI_BT_CAP_SBC;

            char sep_path[160] = {0};
            uint8_t remote_caps[16] = {0};
            int remote_len = 0;
            uint8_t sel_cfg[16] = {0};
            int sel_len = 0;
            bool can_switch = (target_cap != cur_cap || !had_tp) && dpath[0] &&
                              probe_remote_seps_and_match(conn, bt, target_cap, sep_path, sizeof(sep_path),
                                                          remote_caps, &remote_len) &&
                              (select_a2dp_config_for_endpoint(local_ep, remote_caps, remote_len,
                                                               sel_cfg, &sel_len, NULL) == 0);

            if (target_cap == cur_cap && had_tp) {
                pthread_mutex_lock(&bt->state_lock);
                snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                         "Active codec %.15s already optimal for Auto", bt->codec_name);
                pthread_mutex_unlock(&bt->state_lock);
            } else if (!can_switch) {
                /* Keep active stream untouched if peer lacks the requested SEP */
                lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                                   "BT_RENEG_SKIP", sep_path[0] ? sep_path : local_ep,
                                   "can_switch=0 target_cap=0x%x cur_cap=0x%x", target_cap, cur_cap);
                pthread_mutex_lock(&bt->state_lock);
                bt->pref_codec = LWASAPI_BT_PREF_AUTO;
                snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                         "Headset lacks requested SEP; kept %.15s",
                         bt->codec_name[0] ? bt->codec_name : "active");
                pthread_mutex_unlock(&bt->state_lock);
            } else {
                lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                   "BT_RENEG_SEND", sep_path,
                                   "Switching SEP -> %s (pref=%u)", local_ep, want_pref);
                /* Release current MediaTransport1 cleanly WITHOUT UnregisterEndpoint or Device1.Disconnect */
                pthread_mutex_lock(&bt->state_lock);
                lwasapi_bluez_set_tx_enabled(bt, false);
                if (bt->bt_fd >= 0) { close(bt->bt_fd); bt->bt_fd = -1; }
                bt->direct_socket_mode = false;
                bt->has_bluez_transport = false;
                bt->tx_stream_active = 0;
                close_bt_encoders(bt);
                pthread_mutex_unlock(&bt->state_lock);

                if (old_tp[0]) {
                    DBusMessage *rel = dbus_message_new_method_call("org.bluez", old_tp,
                                                                     "org.bluez.MediaTransport1", "Release");
                    if (rel) {
                        dbus_connection_send(conn, rel, NULL);
                        dbus_message_unref(rel);
                    }
                    dbus_connection_flush(conn);
                    for (int t = 0; t < 4; t++) dbus_connection_read_write_dispatch(conn, 20);
                }

                if (send_remote_sep_set_config(conn, sep_path, local_ep, sel_cfg, sel_len, &reneg_pending) == 0) {
                    bt->renegotiate_start_ns = bt_monotonic_ns();
                    atomic_store(&bt->bt_renegotiate_fallback, 1U);
                    no_transport_since_ns = bt->renegotiate_start_ns;
                } else {
                    const char *uuid = A2DP_SINK_UUID;
                    DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", dpath,
                                                                      "org.bluez.Device1", "ConnectProfile");
                    if (cmsg) {
                        dbus_message_append_args(cmsg, DBUS_TYPE_STRING, &uuid, DBUS_TYPE_INVALID);
                        dbus_connection_send(conn, cmsg, NULL);
                        dbus_message_unref(cmsg);
                    }
                }
            }
        }

        uint64_t reneg_now_ns = bt_monotonic_ns();
        if (atomic_load(&bt->bt_renegotiate_fallback) &&
            !bt->has_bluez_transport &&
            bt->renegotiate_start_ns > 0 &&
            reneg_now_ns > bt->renegotiate_start_ns &&
            (reneg_now_ns - bt->renegotiate_start_ns) >= 4500000000ULL) {
            atomic_store(&bt->bt_renegotiate_fallback, 0U);
            lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                               "BT_RENEG_TMO", bt->device_path,
                               "Peer timed out codec switch; restoring Auto");
            pthread_mutex_lock(&bt->state_lock);
            bt->pref_codec = LWASAPI_BT_PREF_AUTO;
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "Peer timed out codec switch; restored Auto");
            char dpath[128];
            snprintf(dpath, sizeof(dpath), "%s", bt->device_path);
            pthread_mutex_unlock(&bt->state_lock);
            if (dpath[0]) {
                const char *uuid = A2DP_SINK_UUID;
                DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", dpath,
                                                                  "org.bluez.Device1", "ConnectProfile");
                if (cmsg) {
                    dbus_message_append_args(cmsg, DBUS_TYPE_STRING, &uuid, DBUS_TYPE_INVALID);
                    dbus_connection_send(conn, cmsg, NULL);
                    dbus_message_unref(cmsg);
                }
            }
        }

        if (atomic_exchange(&g_need_reregister, 0) ||
            (!bt->endpoints_registered && (loop_now_ns - last_rereg_attempt_ns) >= 1500000000ULL)) {
            last_rereg_attempt_ns = loop_now_ns;
            register_all_adapters_and_kick_connected(conn, bt);
            probe_remote_seps_on_dbus(conn, bt);
            no_transport_since_ns = bt_monotonic_ns();
            kick_retried = false;
            kick_cycled = false;
        }

        if (atomic_exchange(&g_need_acquire, 0)) {
            last_acquire_retry_ns = loop_now_ns;
            acquire_bluez_transport(conn, bt);
            probe_remote_seps_on_dbus(conn, bt);
            uint16_t cur_v = 127;
            if (bt->transport_path[0] &&
                dbus_get_u16_prop(conn, bt->transport_path, "org.bluez.MediaTransport1", "Volume", &cur_v)) {
                pthread_mutex_lock(&bt->state_lock);
                bt->avrcp_volume = cur_v;
                __atomic_store_n(&bt->desktop_volume_pct, (uint32_t)((cur_v * 100U + 63U) / 127U), __ATOMIC_RELAXED);
                atomic_store(&g_avrcp_supported, 1);
                pthread_mutex_unlock(&bt->state_lock);
            }
        }

        if (atomic_exchange(&g_need_avrcp_sync, 0)) {
            int pct = atomic_load(&g_pending_avrcp_vol);
            if (pct >= 0 && bt->transport_path[0]) {
                uint16_t clamped = (uint16_t)(pct > 100 ? 100 : pct);
                uint16_t v127 = (uint16_t)((clamped * 127U + 50U) / 100U);
                bt->avrcp_volume = v127;
                dbus_set_avrcp_volume(conn, bt->transport_path, v127);
            }
        }

        uint64_t now_ns = bt_monotonic_ns();
        pthread_mutex_lock(&bt->state_lock);
        bool bt_live   = (bt->has_bluez_transport && bt->direct_socket_mode &&
                          bt->bt_fd >= 0 && bt->tx_stream_active);
        bool bt_cfg    = bt->has_bluez_transport;
        bool bt_has_tp = (bt->transport_path[0] != '\0');
        pthread_mutex_unlock(&bt->state_lock);

        if (bt_live) {
            no_transport_since_ns = now_ns;
            last_acquire_retry_ns = 0;
            kick_retried = false;
            kick_cycled = false;
            g_bt_kick_dev[0] = '\0';
            continue;
        }

        if (bt_cfg && bt_has_tp && (now_ns - last_acquire_retry_ns) >= 300000000ULL) {
            last_acquire_retry_ns = now_ns;
            acquire_bluez_transport(conn, bt);
            pthread_mutex_lock(&bt->state_lock);
            bt_live = (bt->has_bluez_transport && bt->direct_socket_mode &&
                       bt->bt_fd >= 0 && bt->tx_stream_active);
            pthread_mutex_unlock(&bt->state_lock);
            if (bt_live) {
                no_transport_since_ns = bt_monotonic_ns();
                last_acquire_retry_ns = 0;
                kick_retried = false;
                kick_cycled = false;
                g_bt_kick_dev[0] = '\0';
                continue;
            }
        }

        /* One-shot startup auto-connect for pinned preferred BlueZ playback device */
        if (!autoconnect_done && g_bt_startup_autoconnect_dev[0] &&
            (now_ns - thread_start_ns) >= 1200000000ULL) {
            autoconnect_done = true;
            bool already_conn = false;
            if (!dbus_get_bool_prop(conn, g_bt_startup_autoconnect_dev, "org.bluez.Device1", "Connected", &already_conn) || !already_conn) {
                dbus_set_str_prop(conn, g_bt_startup_autoconnect_dev, "org.bluez.Device1", "PreferredBearer", "bredr");
                DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", g_bt_startup_autoconnect_dev,
                                                                  "org.bluez.Device1", "Connect");
                if (cmsg) {
                    dbus_connection_send(conn, cmsg, NULL);
                    dbus_message_unref(cmsg);
                    lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                                       "BT_AUTOCONN", g_bt_startup_autoconnect_dev,
                                       "Startup BR/EDR auto-connect triggered for preferred device");
                }
            }
        }

        /* Gentle A2DP-only profile nudge when Connected==true without MediaTransport1 (never ACL Disconnect!) */
        uint64_t drop_ns = atomic_load_explicit(&bt->tx_drop_event_ns, memory_order_relaxed);
        if (drop_ns > no_transport_since_ns) {
            no_transport_since_ns = drop_ns;
            kick_retried = false;
            kick_cycled = false;
        }
        const char *cand = g_bt_kick_dev[0] ? g_bt_kick_dev : bt->device_path;
        if (bt->endpoints_registered && cand[0] &&
            !reneg_pending && !atomic_load(&bt->bt_renegotiate_fallback) &&
            !atomic_load_explicit(&g_bt_recovery_active, memory_order_acquire)) {
            uint64_t elapsed_ns = (now_ns > no_transport_since_ns) ? (now_ns - no_transport_since_ns) : 0ULL;
            uint64_t last_rec = atomic_load_explicit(&g_bt_last_recover_ns, memory_order_relaxed);
            bool cooldown_ok = (last_rec == 0) || (now_ns > last_rec && (now_ns - last_rec) >= 30000000000ULL);
            if (cooldown_ok && elapsed_ns >= 1800000000ULL && !kick_retried) {
                kick_retried = true;
                bool is_conn = false;
                if (dbus_get_bool_prop(conn, cand, "org.bluez.Device1", "Connected", &is_conn) && is_conn) {
                    dbus_set_str_prop(conn, cand, "org.bluez.Device1", "PreferredBearer", "bredr");
                    const char *uuid = A2DP_SINK_UUID;
                    DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", cand,
                                                                      "org.bluez.Device1", "ConnectProfile");
                    if (cmsg) {
                        dbus_message_append_args(cmsg, DBUS_TYPE_STRING, &uuid, DBUS_TYPE_INVALID);
                        dbus_connection_send(conn, cmsg, NULL);
                        dbus_message_unref(cmsg);
                    }
                } else {
                    kick_cycled = true;
                }
            } else if (cooldown_ok && elapsed_ns >= 6000000000ULL && !kick_cycled) {
                kick_cycled = true;
                atomic_store_explicit(&g_bt_last_recover_ns, now_ns, memory_order_relaxed);
                bool is_conn = false;
                if (dbus_get_bool_prop(conn, cand, "org.bluez.Device1", "Connected", &is_conn) && is_conn) {
                    const char *uuid = A2DP_SINK_UUID;
                    DBusMessage *cmsg = dbus_message_new_method_call("org.bluez", cand,
                                                                      "org.bluez.Device1", "ConnectProfile");
                    if (cmsg) {
                        dbus_message_append_args(cmsg, DBUS_TYPE_STRING, &uuid, DBUS_TYPE_INVALID);
                        dbus_connection_send(conn, cmsg, NULL);
                        dbus_message_unref(cmsg);
                    }
                }
            }
        }
    }

    dbus_connection_close(conn);
    dbus_connection_unref(conn);
    bt->dbus_conn = NULL;
    return NULL;
}

/* ============================================================================
 * Public Pure Native BlueZ + Kernel Capture API
 * ============================================================================ */

void lwasapi_bluez_set_preferred_target(lwasapi_bluez_t *bt, const char *preferred_pb_id) {
    if (!bt) return;
    char mac[32] = {0};
    if (preferred_pb_id && strncasecmp(preferred_pb_id, "bluez:", 6) == 0) {
        extract_mac_from_id(preferred_pb_id, mac, sizeof(mac));
    }
    pthread_mutex_lock(&bt->state_lock);
    snprintf(bt->preferred_mac, sizeof(bt->preferred_mac), "%s", mac);
    pthread_mutex_unlock(&bt->state_lock);
}

int lwasapi_bluez_reset_device(const char *mac_or_empty, char *out_msg, size_t out_len) {
    DBusError err;
    dbus_error_init(&err);
    DBusConnection *conn = dbus_bus_get_private(DBUS_BUS_SYSTEM, &err);
    if (!conn) {
        if (out_msg && out_len) {
            snprintf(out_msg, out_len, "Failed to connect to system D-Bus: %s",
                     err.message ? err.message : "unknown");
        }
        dbus_error_free(&err);
        return -1;
    }
    dbus_connection_set_exit_on_disconnect(conn, FALSE);

    char dev_path[128] = {0}, alias[64] = {0}, addr[32] = {0};
    bool is_conn = false;
    int rc = -1;
    if (!find_a2dp_device_path(conn, mac_or_empty, dev_path, sizeof(dev_path),
                               alias, sizeof(alias), addr, sizeof(addr), &is_conn)) {
        if (out_msg && out_len) {
            snprintf(out_msg, out_len,
                     (mac_or_empty && mac_or_empty[0])
                         ? "No paired Bluetooth A2DP sink found matching '%s'."
                         : "No paired Bluetooth A2DP sink device found in BlueZ.",
                     mac_or_empty ? mac_or_empty : "");
        }
    } else {
        char errmsg[256] = {0};
        rc = cycle_device_bredr_reconnect(conn, dev_path, alias, true, errmsg, sizeof(errmsg));
        if (rc == 0) {
            if (out_msg && out_len) {
                snprintf(out_msg, out_len, "%s (%s) reconnected on BR/EDR A2DP.",
                         alias[0] ? alias : "Bluetooth device", addr);
            }
        } else if (out_msg && out_len) {
            snprintf(out_msg, out_len, "%s", errmsg[0] ? errmsg : "Bluetooth reset failed.");
        }
    }

    dbus_connection_close(conn);
    dbus_connection_unref(conn);
    return rc;
}

int lwasapi_bluez_init_rates(lwasapi_bluez_t *bt, uint32_t engine_sample_rate, uint32_t capture_sample_rate,
                             const char *preferred_cap_id, const char *preferred_pb_id) {
    if (!bt) return -1;
    memset(bt, 0, sizeof(*bt));
    bt_init_pi_mutex(&bt->state_lock);
    bt->bt_fd = -1;
    bt->cap_pcm_fd = -1;
    bt->sco_fd = -1;
    bt->engine_sample_rate = engine_sample_rate ? engine_sample_rate : 48000;
    bt->bt_sample_rate = 48000;
    bt->bt_channels = 2;
    bt->desktop_volume_pct = 100;
    bt->desktop_mic_volume_pct = 100;
    bt->wireless_abr_enabled = 1;
    bt->ldac_mode = LWASAPI_LDAC_MODE_ADAPTIVE;
    bt->pref_codec = LWASAPI_BT_PREF_AUTO;
    bt->hfp_sco_enabled = 0; /* Preserve Focusrite Scarlett / native ALSA mic default */
    snprintf(bt->neg_reason, sizeof(bt->neg_reason), "Waiting for A2DP endpoint negotiation");
    atomic_init(&bt->desktop_mic_gain_ppm, 1000000U);
    atomic_init(&bt->tx_drop_event_ns, 0ULL);
    bt->avrcp_volume = 127;
    if (preferred_pb_id && strncasecmp(preferred_pb_id, "bluez:", 6) == 0) {
        extract_mac_from_id(preferred_pb_id, bt->preferred_mac, sizeof(bt->preferred_mac));
    }
    bt->initialized = true;
    g_bt_inst = bt;
    setvbuf(stdout, NULL, _IOLBF, 0);

    bt->dbus_thread_running = 1;
    pthread_create(&bt->dbus_thread, NULL, bt_dbus_thread_fn, bt);

    bt->tx_thread_running = 1;
    pthread_create(&bt->tx_thread, NULL, bt_tx_thread_fn, bt);

    /* Wait for D-Bus A2DP registration + initial headphone Acquire (up to 1.5s when BT is preferred) */
    bool want_bt_pref = (bt->preferred_mac[0] != '\0') ||
                        (preferred_pb_id && (strcasecmp(preferred_pb_id, "bluetooth") == 0 ||
                                             strcasecmp(preferred_pb_id, "bluez") == 0));
    int max_wait_ticks = want_bt_pref ? 125 : 20;
    bool bt_ready_now = false;
    for (int i = 0; i < max_wait_ticks; i++) {
        pthread_mutex_lock(&bt->state_lock);
        bt_ready_now = (bt->has_bluez_transport && bt->direct_socket_mode && bt->bt_fd >= 0);
        pthread_mutex_unlock(&bt->state_lock);
        if (bt_ready_now) break;
        usleep(20000);
    }

    uint32_t req_cap = capture_sample_rate ? capture_sample_rate : bt->engine_sample_rate;
    lwasapi_bluez_open_capture(bt, preferred_cap_id, req_cap);
    return bt_ready_now ? 0 : -1;
}

int lwasapi_bluez_init(lwasapi_bluez_t *bt, uint32_t engine_sample_rate,
                       const char *preferred_cap_id, const char *preferred_pb_id) {
    return lwasapi_bluez_init_rates(bt, engine_sample_rate, engine_sample_rate, preferred_cap_id, preferred_pb_id);
}

int lwasapi_bluez_reconfigure_rate(lwasapi_bluez_t *bt, uint32_t new_engine_sample_rate) {
    if (!bt || !bt->initialized) return -1;
    if (new_engine_sample_rate >= 44100 && new_engine_sample_rate <= 192000) {
        pthread_mutex_lock(&bt->state_lock);
        __atomic_store_n(&bt->engine_sample_rate, new_engine_sample_rate, __ATOMIC_RELEASE);
        pthread_mutex_unlock(&bt->state_lock);
    }
    return 0;
}

void lwasapi_bluez_set_tx_enabled(lwasapi_bluez_t *bt, bool enabled) {
    if (!bt) return;
    if (!enabled) {
        atomic_store_explicit(&bt->tx_paused, 1, memory_order_release);
    } else {
        uint64_t wr = atomic_load_explicit(&bt->tx_write_frame, memory_order_acquire);
        atomic_store_explicit(&bt->tx_read_frame, wr, memory_order_release);
        atomic_store_explicit(&bt->tx_paused, 0, memory_order_release);
    }
}

int lwasapi_bluez_write_period(lwasapi_bluez_t *bt, const float *interleaved_float, uint32_t frames, uint32_t channels) {
    if (!bt || !bt->initialized || !interleaved_float || frames == 0) return -1;
    if (!bt->has_bluez_transport || bt->bt_fd < 0) return -1;
    if (atomic_load_explicit(&bt->tx_paused, memory_order_relaxed)) return 0;

    uint64_t wr = atomic_load_explicit(&bt->tx_write_frame, memory_order_relaxed);
    for (uint32_t i = 0; i < frames; i++) {
        uint32_t idx = (uint32_t)((wr + i) & (LWASAPI_BT_RING_FRAMES - 1));
        float l = interleaved_float[i * channels + 0];
        float r = (channels >= 2) ? interleaved_float[i * channels + 1] : l;
        bt->tx_ring[idx * 2 + 0] = l;
        bt->tx_ring[idx * 2 + 1] = r;
    }
    atomic_store_explicit(&bt->tx_write_frame, wr + frames, memory_order_release);
    return 0;
}

static inline float cap_clamp_unit(float v) {
    if (v > 1.0f) return 1.0f;
    if (v < -1.0f) return -1.0f;
    return v;
}

static inline float cap_catmull_rom(float p0, float p1, float p2, float p3, float t) {
    float t2 = t * t;
    float t3 = t2 * t;
    return 0.5f * ((2.0f * p1) +
                   (-p0 + p2) * t +
                   (2.0f * p0 - 5.0f * p1 + 4.0f * p2 - p3) * t2 +
                   (-p0 + 3.0f * p1 - 3.0f * p2 + p3) * t3);
}

int lwasapi_bluez_read_capture_native(lwasapi_bluez_t *bt,
                                      float *out_stereo_float,
                                      uint32_t max_frames,
                                      uint32_t *out_frames,
                                      uint32_t *out_rate,
                                      uint32_t eng_frames,
                                      uint32_t eng_rate,
                                      uint64_t *io_read_cursor) {
    if (!out_stereo_float || max_frames == 0 || eng_frames == 0 || eng_rate == 0) return -1;

    uint32_t ring_rate = 0U;
    if (bt && bt->initialized && bt->has_capture_hw) {
        ring_rate = __atomic_load_n(&bt->cap_ring_rate, __ATOMIC_ACQUIRE);
        if (ring_rate < 8000U) {
            ring_rate = __atomic_load_n(&bt->cap_hw_rate, __ATOMIC_RELAXED);
        }
    }
    if (ring_rate < 8000U) ring_rate = eng_rate;

    typedef struct {
        const uint64_t *cursor_key;
        double frac;
        double avg_avail;
        double native_phase;
        uint32_t last_ring_rate;
        float last_l;
        float last_r;
        bool in_underrun;
        bool primed;
    } cap_drift_state_t;

    static __thread cap_drift_state_t tls_states[4];
    cap_drift_state_t *st = &tls_states[0];
    if (io_read_cursor) {
        int found = -1, empty = -1;
        for (int s = 0; s < 4; s++) {
            if (tls_states[s].cursor_key == io_read_cursor) { found = s; break; }
            if (!tls_states[s].cursor_key && empty < 0) empty = s;
        }
        int slot = (found >= 0) ? found : (empty >= 0 ? empty : 0);
        st = &tls_states[slot];
        if (st->cursor_key != io_read_cursor) {
            memset(st, 0, sizeof(*st));
            st->cursor_key = io_read_cursor;
        }
    }

    if (st->last_ring_rate != ring_rate) {
        st->last_ring_rate = ring_rate;
        st->native_phase = 0.0;
        st->frac = 0.0;
        st->in_underrun = false;
        st->primed = false;
        if (io_read_cursor) *io_read_cursor = 0;
    }

    double exact_n = ((double)eng_frames * (double)ring_rate) / (double)eng_rate + st->native_phase;
    uint32_t frames = (uint32_t)floor(exact_n + 1e-9);
    if (frames < 1U) frames = 1U;
    if (frames > max_frames) frames = max_frames;
    st->native_phase = exact_n - (double)frames;
    if (st->native_phase < 0.0) st->native_phase = 0.0;

    if (out_frames) *out_frames = frames;
    if (out_rate) *out_rate = ring_rate;

    if (!bt || !bt->initialized || !bt->has_capture_hw) {
        memset(out_stereo_float, 0, (size_t)frames * 2 * sizeof(float));
        return -1;
    }

    uint32_t min_target = (ring_rate <= 64000U) ? 192U : 512U;
    uint32_t target_fill = frames * 4U;
    if (target_fill < min_target) target_fill = min_target;
    uint32_t max_fill = target_fill * 6U;
    if (max_fill > LWASAPI_RING_FRAMES - 1024U) max_fill = LWASAPI_RING_FRAMES - 1024U;

    uint64_t wr = atomic_load_explicit(&bt->cap_hw_write_frame, memory_order_acquire);
    uint64_t rd = io_read_cursor ? *io_read_cursor : 0;

    if (rd == 0) {
        if (wr < target_fill) {
            memset(out_stereo_float, 0, (size_t)frames * 2 * sizeof(float));
            return 0;
        }
        rd = wr - target_fill;
        st->frac = 0.0;
        st->avg_avail = (double)target_fill;
        st->last_l = 0.0f;
        st->last_r = 0.0f;
        st->in_underrun = false;
        st->primed = true;
    } else if (wr < rd || (wr - rd) > max_fill) {
        rd = (wr > target_fill) ? (wr - target_fill) : 0;
        st->frac = 0.0;
        st->avg_avail = (double)target_fill;
        st->in_underrun = false;
        st->primed = true;
    }

    uint64_t avail = (wr >= rd) ? (wr - rd) : 0;
    if (st->in_underrun) {
        if (avail < target_fill) {
            memset(out_stereo_float, 0, (size_t)frames * 2 * sizeof(float));
            if (io_read_cursor) *io_read_cursor = rd;
            return 0;
        }
        st->in_underrun = false;
        st->frac = 0.0;
        st->avg_avail = (double)avail;
    }

    if (avail < frames) {
        uint32_t to_copy = (uint32_t)avail;
        float last_l = st->last_l;
        float last_r = st->last_r;
        for (uint32_t i = 0; i < to_copy; i++) {
            uint32_t idx = (uint32_t)((rd + i) & (LWASAPI_RING_FRAMES - 1));
            last_l = bt->cap_ring[idx * 2 + 0];
            last_r = bt->cap_ring[idx * 2 + 1];
            out_stereo_float[i * 2 + 0] = last_l;
            out_stereo_float[i * 2 + 1] = last_r;
        }
        for (uint32_t j = 0; j < frames - to_copy; j++) {
            if (j < 16U) {
                float g = (float)(15U - j) / 16.0f;
                out_stereo_float[(to_copy + j) * 2 + 0] = last_l * g;
                out_stereo_float[(to_copy + j) * 2 + 1] = last_r * g;
            } else {
                out_stereo_float[(to_copy + j) * 2 + 0] = 0.0f;
                out_stereo_float[(to_copy + j) * 2 + 1] = 0.0f;
            }
        }
        st->last_l = 0.0f;
        st->last_r = 0.0f;
        st->frac = 0.0;
        st->in_underrun = true;
        if (io_read_cursor) *io_read_cursor = rd + to_copy;
        return 0;
    }

    if (!st->primed) {
        st->avg_avail = (double)avail;
        st->primed = true;
    } else {
        st->avg_avail = st->avg_avail * 0.96 + (double)avail * 0.04;
    }

    double err = st->avg_avail - (double)target_fill;
    double dead = (double)target_fill * 0.08;
    double step = 1.0;
    if (err > dead) {
        double adj = (err - dead) / ((double)target_fill * 1.5);
        if (adj > 0.0025) adj = 0.0025;
        step = 1.0 + adj;
    } else if (err < -dead) {
        double adj = (-err - dead) / ((double)target_fill * 1.5);
        if (adj > 0.0025) adj = 0.0025;
        step = 1.0 - adj;
    }

    if (step == 1.0 && fabs(st->frac) < 1e-9) {
        for (uint32_t i = 0; i < frames; i++) {
            uint32_t idx = (uint32_t)((rd + i) & (LWASAPI_RING_FRAMES - 1));
            st->last_l = bt->cap_ring[idx * 2 + 0];
            st->last_r = bt->cap_ring[idx * 2 + 1];
            out_stereo_float[i * 2 + 0] = st->last_l;
            out_stereo_float[i * 2 + 1] = st->last_r;
        }
        st->frac = 0.0;
        if (io_read_cursor) *io_read_cursor = rd + frames;
        return 0;
    }

    double pos = st->frac;
    for (uint32_t i = 0; i < frames; i++) {
        uint64_t i0 = (uint64_t)pos;
        if (i0 + 1U < avail) {
            float alpha = (float)(pos - (double)i0);
            uint64_t im1 = (i0 > 0U) ? (i0 - 1U) : 0U;
            uint64_t ip1 = i0 + 1U;
            uint64_t ip2 = (i0 + 2U < avail) ? (i0 + 2U) : ip1;
            uint32_t km1 = (uint32_t)((rd + im1) & (LWASAPI_RING_FRAMES - 1));
            uint32_t k0  = (uint32_t)((rd + i0)  & (LWASAPI_RING_FRAMES - 1));
            uint32_t kp1 = (uint32_t)((rd + ip1) & (LWASAPI_RING_FRAMES - 1));
            uint32_t kp2 = (uint32_t)((rd + ip2) & (LWASAPI_RING_FRAMES - 1));
            st->last_l = cap_clamp_unit(cap_catmull_rom(
                bt->cap_ring[km1 * 2 + 0], bt->cap_ring[k0 * 2 + 0],
                bt->cap_ring[kp1 * 2 + 0], bt->cap_ring[kp2 * 2 + 0], alpha));
            st->last_r = cap_clamp_unit(cap_catmull_rom(
                bt->cap_ring[km1 * 2 + 1], bt->cap_ring[k0 * 2 + 1],
                bt->cap_ring[kp1 * 2 + 1], bt->cap_ring[kp2 * 2 + 1], alpha));
        } else {
            uint64_t c0 = (avail > 0U) ? (avail - 1U) : 0U;
            uint32_t idx0 = (uint32_t)((rd + c0) & (LWASAPI_RING_FRAMES - 1));
            st->last_l = bt->cap_ring[idx0 * 2 + 0];
            st->last_r = bt->cap_ring[idx0 * 2 + 1];
        }
        out_stereo_float[i * 2 + 0] = st->last_l;
        out_stereo_float[i * 2 + 1] = st->last_r;
        pos += step;
    }

    uint64_t whole = (uint64_t)pos;
    if (whole > avail) {
        whole = avail;
        pos = (double)whole;
    }
    st->frac = pos - (double)whole;
    if (io_read_cursor) *io_read_cursor = rd + whole;
    return 0;
}

int lwasapi_bluez_read_capture_period(lwasapi_bluez_t *bt, float *out_stereo_float, uint32_t frames, uint64_t *io_read_cursor) {
    if (!out_stereo_float || frames == 0) return -1;
    uint32_t eng_rate = (bt && bt->initialized) ? __atomic_load_n(&bt->engine_sample_rate, __ATOMIC_RELAXED) : 48000U;
    if (!eng_rate) eng_rate = 48000U;

    float native_tmp[4096 * 2];
    uint32_t got_n = 0, nat_rate = eng_rate;
    int rc = lwasapi_bluez_read_capture_native(bt, native_tmp, 4096U, &got_n, &nat_rate, frames, eng_rate, io_read_cursor);
    if (got_n == frames || got_n == 0) {
        if (got_n == frames) memcpy(out_stereo_float, native_tmp, (size_t)frames * 2 * sizeof(float));
        else memset(out_stereo_float, 0, (size_t)frames * 2 * sizeof(float));
        return rc;
    }
    double step = (double)got_n / (double)frames;
    for (uint32_t i = 0; i < frames; i++) {
        double p = (double)i * step;
        uint32_t i0 = (uint32_t)p;
        if (i0 >= got_n) i0 = got_n - 1U;
        uint32_t im1 = (i0 > 0U) ? (i0 - 1U) : 0U;
        uint32_t ip1 = (i0 + 1U < got_n) ? (i0 + 1U) : i0;
        uint32_t ip2 = (i0 + 2U < got_n) ? (i0 + 2U) : ip1;
        float t = (float)(p - (double)i0);
        out_stereo_float[i * 2 + 0] = cap_clamp_unit(cap_catmull_rom(
            native_tmp[im1 * 2 + 0], native_tmp[i0 * 2 + 0], native_tmp[ip1 * 2 + 0], native_tmp[ip2 * 2 + 0], t));
        out_stereo_float[i * 2 + 1] = cap_clamp_unit(cap_catmull_rom(
            native_tmp[im1 * 2 + 1], native_tmp[i0 * 2 + 1], native_tmp[ip1 * 2 + 1], native_tmp[ip2 * 2 + 1], t));
    }
    return rc;
}

int lwasapi_bluez_enumerate_devices(const lwasapi_bluez_t *bt, const char *active_pb_id,
                                    const char *active_cap_id, lwasapi_device_list_t *out_list) {
    if (!out_list) return -1;
    memset(out_list, 0, sizeof(*out_list));

    char card_names[32][64];
    int  card_is_usb[32];
    parse_asound_cards(card_names, card_is_usb);

    /* 1. Enumerate BlueZ connected A2DP Sink devices via system D-Bus */
    pthread_mutex_lock(&g_dbus_enum_lock);
    dbus_threads_init_default();
    DBusConnection *conn = dbus_bus_get(DBUS_BUS_SYSTEM, NULL);
    if (conn) {
        DBusMessage *msg = dbus_message_new_method_call("org.bluez", "/",
                                                         "org.freedesktop.DBus.ObjectManager", "GetManagedObjects");
        DBusMessage *rep = msg ? dbus_connection_send_with_reply_and_block(conn, msg, 1200, NULL) : NULL;
        if (msg) dbus_message_unref(msg);
        if (rep) {
            bool bt_live = false;
            char bt_mac[32] = {0};
            char bt_codec[16] = {0};
            if (bt) {
                pthread_mutex_lock((pthread_mutex_t *)&bt->state_lock);
                bt_live = (bt->has_bluez_transport && bt->direct_socket_mode && bt->bt_fd >= 0);
                if (bt->mac_address[0]) snprintf(bt_mac, sizeof(bt_mac), "%s", bt->mac_address);
                if (bt->codec_name[0]) snprintf(bt_codec, sizeof(bt_codec), "%s", bt->codec_name);
                pthread_mutex_unlock((pthread_mutex_t *)&bt->state_lock);
            }

            char mt_devs[16][128];
            int  n_mt = 0;
            DBusMessageIter root, obj_arr, obj_entry, if_arr, if_entry, prop_arr, prop_entry, var;
            if (dbus_message_iter_init(rep, &root) && dbus_message_iter_get_arg_type(&root) == DBUS_TYPE_ARRAY) {
                dbus_message_iter_recurse(&root, &obj_arr);
                while (dbus_message_iter_get_arg_type(&obj_arr) == DBUS_TYPE_DICT_ENTRY && n_mt < 16) {
                    const char *obj_path = NULL;
                    dbus_message_iter_recurse(&obj_arr, &obj_entry);
                    dbus_message_iter_get_basic(&obj_entry, &obj_path);
                    dbus_message_iter_next(&obj_entry);
                    dbus_message_iter_recurse(&obj_entry, &if_arr);
                    while (dbus_message_iter_get_arg_type(&if_arr) == DBUS_TYPE_DICT_ENTRY && n_mt < 16) {
                        const char *iface = NULL;
                        dbus_message_iter_recurse(&if_arr, &if_entry);
                        dbus_message_iter_get_basic(&if_entry, &iface);
                        dbus_message_iter_next(&if_entry);
                        if (iface && strcmp(iface, "org.bluez.MediaTransport1") == 0) {
                            char dev_path[128] = {0};
                            dbus_message_iter_recurse(&if_entry, &prop_arr);
                            while (dbus_message_iter_get_arg_type(&prop_arr) == DBUS_TYPE_DICT_ENTRY) {
                                const char *pk = NULL;
                                dbus_message_iter_recurse(&prop_arr, &prop_entry);
                                dbus_message_iter_get_basic(&prop_entry, &pk);
                                dbus_message_iter_next(&prop_entry);
                                dbus_message_iter_recurse(&prop_entry, &var);
                                if (pk && strcmp(pk, "Device") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_OBJECT_PATH) {
                                    const char *dp = NULL;
                                    dbus_message_iter_get_basic(&var, &dp);
                                    if (dp) snprintf(dev_path, sizeof(dev_path), "%s", dp);
                                }
                                dbus_message_iter_next(&prop_arr);
                            }
                            if (!dev_path[0] && obj_path) {
                                const char *sep = strstr(obj_path, "/sep");
                                size_t dlen = sep ? (size_t)(sep - obj_path) : strlen(obj_path);
                                if (dlen >= sizeof(dev_path)) dlen = sizeof(dev_path) - 1;
                                memcpy(dev_path, obj_path, dlen);
                                dev_path[dlen] = '\0';
                            }
                            if (dev_path[0]) snprintf(mt_devs[n_mt++], sizeof(mt_devs[0]), "%s", dev_path);
                        }
                        dbus_message_iter_next(&if_arr);
                    }
                    dbus_message_iter_next(&obj_arr);
                }
            }

            if (dbus_message_iter_init(rep, &root) && dbus_message_iter_get_arg_type(&root) == DBUS_TYPE_ARRAY) {
                dbus_message_iter_recurse(&root, &obj_arr);
                while (dbus_message_iter_get_arg_type(&obj_arr) == DBUS_TYPE_DICT_ENTRY &&
                       out_list->num_devices < LWASAPI_MAX_DEVICES) {
                    const char *obj_path = NULL;
                    dbus_message_iter_recurse(&obj_arr, &obj_entry);
                    dbus_message_iter_get_basic(&obj_entry, &obj_path);
                    dbus_message_iter_next(&obj_entry);
                    dbus_message_iter_recurse(&obj_entry, &if_arr);
                    while (dbus_message_iter_get_arg_type(&if_arr) == DBUS_TYPE_DICT_ENTRY &&
                           out_list->num_devices < LWASAPI_MAX_DEVICES) {
                        const char *iface = NULL;
                        dbus_message_iter_recurse(&if_arr, &if_entry);
                        dbus_message_iter_get_basic(&if_entry, &iface);
                        dbus_message_iter_next(&if_entry);
                        if (iface && strcmp(iface, "org.bluez.Device1") == 0) {
                            dbus_bool_t connected = FALSE;
                            bool is_a2dp = false;
                            bool is_hfp_hf = false;
                            char alias[64] = {0}, addr[32] = {0};
                            dbus_message_iter_recurse(&if_entry, &prop_arr);
                            while (dbus_message_iter_get_arg_type(&prop_arr) == DBUS_TYPE_DICT_ENTRY) {
                                const char *pk = NULL;
                                dbus_message_iter_recurse(&prop_arr, &prop_entry);
                                dbus_message_iter_get_basic(&prop_entry, &pk);
                                dbus_message_iter_next(&prop_entry);
                                dbus_message_iter_recurse(&prop_entry, &var);
                                if (pk && strcmp(pk, "Connected") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_BOOLEAN) {
                                    dbus_message_iter_get_basic(&var, &connected);
                                } else if (pk && strcmp(pk, "Alias") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_STRING) {
                                    const char *s = NULL; dbus_message_iter_get_basic(&var, &s);
                                    if (s) snprintf(alias, sizeof(alias), "%s", s);
                                } else if (pk && strcmp(pk, "Address") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_STRING) {
                                    const char *s = NULL; dbus_message_iter_get_basic(&var, &s);
                                    if (s) snprintf(addr, sizeof(addr), "%s", s);
                                } else if (pk && strcmp(pk, "UUIDs") == 0 && dbus_message_iter_get_arg_type(&var) == DBUS_TYPE_ARRAY) {
                                    DBusMessageIter uarr;
                                    dbus_message_iter_recurse(&var, &uarr);
                                    while (dbus_message_iter_get_arg_type(&uarr) == DBUS_TYPE_STRING) {
                                        const char *u = NULL; dbus_message_iter_get_basic(&uarr, &u);
                                        if (u && strncasecmp(u, "0000110b", 8) == 0) is_a2dp = true;
                                        if (u && strncasecmp(u, "0000111e", 8) == 0) is_hfp_hf = true;
                                        dbus_message_iter_next(&uarr);
                                    }
                                }
                                dbus_message_iter_next(&prop_arr);
                            }
                            bool has_mt = false;
                            if (obj_path) {
                                for (int m = 0; m < n_mt; m++) {
                                    if (strcmp(mt_devs[m], obj_path) == 0) {
                                        has_mt = true;
                                        break;
                                    }
                                }
                            }
                            bool matches_live = (bt_live && bt_mac[0] && addr[0] && strcasecmp(bt_mac, addr) == 0);
                            if (connected && is_a2dp && addr[0] && (matches_live || (!bt && has_mt))) {
                                lwasapi_device_entry_t *e = &out_list->devices[out_list->num_devices++];
                                memset(e, 0, sizeof(*e));
                                snprintf(e->id, sizeof(e->id), "bluez:%s", addr);
                                const char *cn = ((matches_live || (bt_live && !bt_mac[0])) && bt_codec[0]) ? bt_codec : "A2DP";
                                snprintf(e->name, sizeof(e->name), "%s (%s)", alias[0] ? alias : addr, cn);
                                snprintf(e->driver, sizeof(e->driver), "bluez-a2dp");
                                e->is_capture   = 0;
                                e->is_bluetooth = 1;
                                e->is_active    = (active_pb_id && (strcmp(active_pb_id, e->id) == 0 ||
                                                   strcmp(active_pb_id, "bluetooth") == 0 ||
                                                   strcmp(active_pb_id, "bluez") == 0)) ? 1 : 0;
                                e->min_rate     = 44100;
                                e->max_rate     = (strcmp(cn, "LDAC") == 0) ? 96000 : 48000;
                            }
                            if (connected && is_hfp_hf && addr[0] && bt && bt->hfp_sco_enabled &&
                                out_list->num_devices < LWASAPI_MAX_DEVICES) {
                                lwasapi_device_entry_t *sc = &out_list->devices[out_list->num_devices++];
                                memset(sc, 0, sizeof(*sc));
                                snprintf(sc->id, sizeof(sc->id), "bluez_sco:%s", addr);
                                snprintf(sc->name, sizeof(sc->name), "%.42s (HFP SCO Mic)", alias[0] ? alias : addr);
                                snprintf(sc->driver, sizeof(sc->driver), "bluez-hfp-sco");
                                sc->is_capture   = 1;
                                sc->is_bluetooth = 1;
                                sc->is_active    = (active_cap_id && strcmp(active_cap_id, sc->id) == 0) ? 1 : 0;
                                sc->min_rate     = 8000;
                                sc->max_rate     = 16000;
                            }
                        }
                        dbus_message_iter_next(&if_arr);
                    }
                    dbus_message_iter_next(&obj_arr);
                }
            }
            dbus_message_unref(rep);
        }
    }
    pthread_mutex_unlock(&g_dbus_enum_lock);

    /* 2. Enumerate plugged kernel /proc/asound/pcm playback & capture devices */
    alsa_topology_t top;
    collect_alsa_topology(&top);
    FILE *fp = fopen("/proc/asound/pcm", "r");
    if (fp) {
        char line[256];
        while (fgets(line, sizeof(line), fp) && out_list->num_devices < LWASAPI_MAX_DEVICES) {
            unsigned c = 0, d = 0;
            char pcm_name[64] = {0};
            if (sscanf(line, "%u-%u: %63[^:]", &c, &d, pcm_name) < 2) continue;
            if (c >= MAX_ALSA_CARDS_SCAN) continue;
            size_t plen = strlen(pcm_name);
            while (plen > 0 && pcm_name[plen - 1] == ' ') pcm_name[--plen] = '\0';

            for (int is_cap = 0; is_cap <= 1 && out_list->num_devices < LWASAPI_MAX_DEVICES; is_cap++) {
                if (!strstr(line, is_cap ? "capture" : "playback")) continue;
                char devnode[64], idbuf[32];
                snprintf(devnode, sizeof(devnode), "/dev/snd/pcmC%uD%u%c", c, d, is_cap ? 'c' : 'p');
                if (access(devnode, F_OK) != 0) continue;
                if (!is_alsa_pcm_plugged(&top, card_is_usb, c, d, is_cap != 0, pcm_name)) continue;

                snprintf(idbuf, sizeof(idbuf), "hw:%u,%u", c, d);
                const char *disp_sub = pcm_name;
                if (!is_cap && d < 64 && top.jacks[c].hdmi_name[d][0]) {
                    disp_sub = top.jacks[c].hdmi_name[d];
                }
                lwasapi_device_entry_t *e = &out_list->devices[out_list->num_devices++];
                memset(e, 0, sizeof(*e));
                snprintf(e->id, sizeof(e->id), "%s", idbuf);
                if (card_names[c][0]) snprintf(e->name, sizeof(e->name), "%.36s - %.23s", card_names[c], disp_sub);
                else snprintf(e->name, sizeof(e->name), "%.40s (%.19s)", disp_sub, idbuf);
                snprintf(e->driver, sizeof(e->driver), "alsa-kernel");
                e->is_capture   = (uint32_t)is_cap;
                e->is_bluetooth = 0;
                e->card_index   = c;
                e->device_index = d;
                e->min_rate     = 44100;
                e->max_rate     = 192000;
                const char *act_id = is_cap ? active_cap_id : active_pb_id;
                e->is_active    = (act_id && strcmp(act_id, idbuf) == 0) ? 1 : 0;
            }
        }
        fclose(fp);
    }

    return 0;
}

int lwasapi_bluez_set_volume(lwasapi_bluez_t *bt, int vol_pct, int mute_flag) {
    if (!bt) return -1;
    if (vol_pct >= 0) {
        if (vol_pct > 150) vol_pct = 150;
        __atomic_store_n(&bt->desktop_volume_pct, (uint32_t)vol_pct, __ATOMIC_RELAXED);
        atomic_store(&g_pending_avrcp_vol, vol_pct);
        atomic_store(&g_need_avrcp_sync, 1);
    }
    if (mute_flag >= 0) {
        __atomic_store_n(&bt->desktop_muted, (mute_flag != 0), __ATOMIC_RELAXED);
    }
    return 0;
}

int lwasapi_bluez_set_mic_volume(lwasapi_bluez_t *bt, int vol_pct, int mute_flag) {
    if (!bt) return -1;
    if (vol_pct >= 0) {
        if (vol_pct > 150) vol_pct = 150;
        __atomic_store_n(&bt->desktop_mic_volume_pct, (uint32_t)vol_pct, __ATOMIC_RELAXED);
        atomic_store_explicit(&bt->desktop_mic_gain_ppm, (uint32_t)vol_pct * 10000U, memory_order_relaxed);
    }
    if (mute_flag >= 0) {
        __atomic_store_n(&bt->desktop_mic_muted, (mute_flag != 0), __ATOMIC_RELAXED);
    }
    return 0;
}

int lwasapi_bluez_set_mic_gain_ppm(lwasapi_bluez_t *bt, uint32_t gain_ppm, int vol_pct, int mute_flag) {
    if (!bt) return -1;
    if (gain_ppm > 2000000U) gain_ppm = 2000000U;
    atomic_store_explicit(&bt->desktop_mic_gain_ppm, gain_ppm, memory_order_relaxed);
    if (vol_pct >= 0) {
        if (vol_pct > 150) vol_pct = 150;
        __atomic_store_n(&bt->desktop_mic_volume_pct, (uint32_t)vol_pct, __ATOMIC_RELAXED);
    }
    if (mute_flag >= 0) {
        __atomic_store_n(&bt->desktop_mic_muted, (mute_flag != 0), __ATOMIC_RELAXED);
    }
    return 0;
}

static bool parse_bdaddr_str(const char *str, lwasapi_bdaddr_t *out) {
    if (!str || !out) return false;
    return salsa_bt_parse_bdaddr(str, strnlen(str, 32u), out->b);
}

static void bt_stop_sco_capture(lwasapi_bluez_t *bt) {
    if (!bt) return;
    bt->sco_thread_running = 0;
    if (bt->sco_fd >= 0) {
        shutdown(bt->sco_fd, SHUT_RDWR);
    }
    if (bt->sco_thread) {
        pthread_join(bt->sco_thread, NULL);
        bt->sco_thread = 0;
    }
    if (bt->sco_fd >= 0) {
        close(bt->sco_fd);
        bt->sco_fd = -1;
    }
    atomic_store(&bt->hfp_sco_active, 0U);
    bt->bt_sco_codec[0] = '\0';
}

static void *sco_cap_thread_fn(void *arg) {
    lwasapi_bluez_t *bt = (lwasapi_bluez_t *)arg;
    bool use_msbc = (strcmp(bt->bt_sco_codec, "mSBC") == 0);
    uint32_t sco_rate = use_msbc ? 16000U : 8000U;
    sbc_t sbc_dec, sbc_enc;
    if (use_msbc) {
        sbc_init_msbc(&sbc_dec, 0);
        sbc_init_msbc(&sbc_enc, 0);
    }

    uint8_t rx_pkt[256];
    uint8_t tx_pkt[64];
    int16_t dec_pcm[256];
    int16_t zero_pcm[128] = {0};
    static const uint8_t h2_headers[4] = { 0x08, 0x38, 0xc8, 0xf8 };
    uint32_t seq = 0;
    double resamp_pos = 0.0;
    float prev_s = 0.0f;

    while (bt->sco_thread_running && bt->sco_fd >= 0) {
        struct pollfd pfd = { .fd = bt->sco_fd, .events = POLLIN, .revents = 0 };
        int pr = poll(&pfd, 1, 200);
        if (pr < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (pr == 0) continue;
        if (pfd.revents & (POLLERR | POLLHUP | POLLNVAL)) break;

        ssize_t r = recv(bt->sco_fd, rx_pkt, sizeof(rx_pkt), 0);
        if (r <= 0) break;

        uint32_t nsamp = 0;
        if (use_msbc && r >= 59) {
            /* Strip 2-byte H2 synchronization header if present */
            size_t off = (rx_pkt[0] == 0x01 && (rx_pkt[1] & 0x0F) == 0x08) ? 2U : 0U;
            size_t written = 0;
            if (r > (ssize_t)off && sbc_decode(&sbc_dec, rx_pkt + off, (size_t)r - off,
                                               dec_pcm, sizeof(dec_pcm), &written) > 0) {
                nsamp = (uint32_t)(written / sizeof(int16_t));
            }
            /* Keep bidirectional HFP SCO clock alive with silent comfort mSBC frames */
            ssize_t enc_written = 0;
            tx_pkt[0] = 0x01;
            tx_pkt[1] = h2_headers[seq & 3U];
            seq++;
            if (sbc_encode(&sbc_enc, zero_pcm, 240, tx_pkt + 2, sizeof(tx_pkt) - 3, &enc_written) > 0 && enc_written == 57) {
                tx_pkt[59] = 0x00;
                (void)send(bt->sco_fd, tx_pkt, 60, MSG_DONTWAIT | MSG_NOSIGNAL);
            }
        } else if (!use_msbc && r >= 2) {
            nsamp = (uint32_t)(r / 2);
            if (nsamp > 128) nsamp = 128;
            memcpy(dec_pcm, rx_pkt, nsamp * 2U);
            (void)send(bt->sco_fd, zero_pcm, (size_t)(r > 128 ? 128 : r), MSG_DONTWAIT | MSG_NOSIGNAL);
        }

        if (nsamp > 0) {
            uint32_t eng_rate = __atomic_load_n(&bt->engine_sample_rate, __ATOMIC_RELAXED);
            if (!eng_rate) eng_rate = 48000U;
            __atomic_store_n(&bt->cap_ring_rate, eng_rate, __ATOMIC_RELEASE);
            double ratio = (double)eng_rate / (double)sco_rate;
            uint64_t wr = atomic_load_explicit(&bt->cap_hw_write_frame, memory_order_relaxed);
            for (uint32_t i = 0; i < nsamp; i++) {
                float cur_s = (float)dec_pcm[i] / 32768.0f;
                resamp_pos += ratio;
                uint32_t out_n = (uint32_t)resamp_pos;
                resamp_pos -= (double)out_n;
                for (uint32_t k = 0; k < out_n; k++) {
                    float t = (out_n > 1) ? ((float)(k + 1) / (float)out_n) : 1.0f;
                    float v = prev_s + (cur_s - prev_s) * t;
                    uint32_t idx = (uint32_t)((wr++) & (LWASAPI_RING_FRAMES - 1));
                    bt->cap_ring[idx * 2 + 0] = v;
                    bt->cap_ring[idx * 2 + 1] = v;
                }
                prev_s = cur_s;
            }
            atomic_store_explicit(&bt->cap_hw_write_frame, wr, memory_order_release);
        }
    }

    if (use_msbc) {
        sbc_finish(&sbc_dec);
        sbc_finish(&sbc_enc);
    }
    atomic_store(&bt->hfp_sco_active, 0U);
    return NULL;
}

static int bt_open_sco_capture(lwasapi_bluez_t *bt, const char *mac_or_empty) {
    if (!bt || !bt->dbus_conn) return -1;
    char target_mac[32] = {0};
    if (mac_or_empty && extract_mac_from_id(mac_or_empty, target_mac, sizeof(target_mac))) {
        /* ok */
    } else if (bt->mac_address[0]) {
        snprintf(target_mac, sizeof(target_mac), "%s", bt->mac_address);
    } else {
        return -1;
    }

    DBusConnection *conn = (DBusConnection *)bt->dbus_conn;
    char adapter_mac[32] = {0};
    if (bt->adapter_path[0]) {
        dbus_get_str_prop(conn, bt->adapter_path, "org.bluez.Adapter1", "Address", adapter_mac, sizeof(adapter_mac));
    }

    lwasapi_bdaddr_t local_ba = {{0}}, remote_ba = {{0}};
    if (!parse_bdaddr_str(target_mac, &remote_ba)) return -1;

    bool want_msbc = false;
    int rfcomm_fd = -1;
    pthread_mutex_lock(&g_hfp_lock);
    for (int i = 0; i < MAX_HFP_DEVS; i++) {
        if (g_hfp_devs[i].in_use && g_hfp_devs[i].rfcomm_active) {
            want_msbc = g_hfp_devs[i].supports_msbc;
            rfcomm_fd = g_hfp_devs[i].fd;
            break;
        }
    }
    pthread_mutex_unlock(&g_hfp_lock);

    if (rfcomm_fd >= 0 && want_msbc) {
        hfp_write_str(rfcomm_fd, "\r\n+BCS: 2\r\n");
        usleep(120000);
    }

    int sfd = socket(PF_BLUETOOTH, SOCK_SEQPACKET | SOCK_CLOEXEC | SOCK_NONBLOCK, BTPROTO_SCO);
    if (sfd < 0) return -1;

    if (adapter_mac[0] && parse_bdaddr_str(adapter_mac, &local_ba)) {
        struct lwasapi_sockaddr_sco laddr;
        memset(&laddr, 0, sizeof(laddr));
        laddr.sco_family = AF_BLUETOOTH;
        laddr.sco_bdaddr = local_ba;
        (void)bind(sfd, (struct sockaddr *)&laddr, sizeof(laddr));
    }

    struct lwasapi_bt_voice voice;
    voice.setting = want_msbc ? BT_VOICE_TRANSPARENT : BT_VOICE_CVSD_16BIT;
    (void)setsockopt(sfd, SOL_BLUETOOTH, BT_VOICE, &voice, sizeof(voice));

    struct lwasapi_sockaddr_sco raddr;
    memset(&raddr, 0, sizeof(raddr));
    raddr.sco_family = AF_BLUETOOTH;
    raddr.sco_bdaddr = remote_ba;
    if (connect(sfd, (struct sockaddr *)&raddr, sizeof(raddr)) < 0) {
        if (errno != EINPROGRESS && errno != EAGAIN) {
            close(sfd);
            return -1;
        }
        struct pollfd cp = { .fd = sfd, .events = POLLOUT, .revents = 0 };
        int cr = poll(&cp, 1, 1500);
        int soerr = 0;
        socklen_t slen = sizeof(soerr);
        if (cr <= 0 || getsockopt(sfd, SOL_SOCKET, SO_ERROR, &soerr, &slen) != 0 || soerr != 0) {
            close(sfd);
            return -1;
        }
    }

    bt->sco_fd = sfd;
    bt->sco_sample_rate = want_msbc ? 16000U : 8000U;
    snprintf(bt->bt_sco_codec, sizeof(bt->bt_sco_codec), "%s", want_msbc ? "mSBC" : "CVSD");
    snprintf(bt->sco_cap_device_id, sizeof(bt->sco_cap_device_id), "bluez_sco:%s", target_mac);
    snprintf(bt->cap_device_id, sizeof(bt->cap_device_id), "%s", bt->sco_cap_device_id);
    snprintf(bt->capture_device_alias, sizeof(bt->capture_device_alias), "BT HFP Headset (%s)", bt->bt_sco_codec);
    bt->cap_hw_rate = bt->engine_sample_rate;
    bt->cap_hw_channels = 2;
    bt->cap_is_usb = false;
    __atomic_store_n(&bt->cap_ring_rate, bt->engine_sample_rate, __ATOMIC_RELEASE);
    __atomic_store_n(&bt->cap_measured_rate, 0U, __ATOMIC_RELAXED);
    __atomic_store_n(&bt->cap_kick_count, 0U, __ATOMIC_RELAXED);
    bt->has_capture_hw = true;
    atomic_store(&bt->cap_hw_write_frame, 0);
    atomic_store(&bt->hfp_sco_active, 1U);
    bt->sco_thread_running = 1;
    pthread_create(&bt->sco_thread, NULL, sco_cap_thread_fn, bt);
    lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_INFO, 0, 0,
                       "BT_SCO_OPEN", bt->cap_device_id,
                       "codec=%s rate=%uHz", bt->bt_sco_codec, bt->sco_sample_rate);
    return 0;
}

int lwasapi_bluez_set_codec_config(lwasapi_bluez_t *bt, const lwasapi_bt_codec_cfg_t *cfg) {
    if (!bt || !cfg) return -1;
    pthread_mutex_lock(&bt->state_lock);
    uint32_t old_pref = bt->pref_codec;
    uint32_t new_pref = (cfg->preferred_codec <= LWASAPI_BT_PREF_SBC) ? cfg->preferred_codec : LWASAPI_BT_PREF_AUTO;
    uint32_t new_ldac = (cfg->ldac_mode <= LWASAPI_LDAC_MODE_330) ? cfg->ldac_mode : LWASAPI_LDAC_MODE_ADAPTIVE;
    uint32_t new_abr  = cfg->wireless_abr_enabled ? 1U : 0U;
    bool     new_sco  = (cfg->hfp_sco_enabled != 0);

    uint32_t sup_mask = bt->supported_codecs_mask;
    bool     has_link = bt->has_bluez_transport;
    bool     preflight_ok = true;

    /* Pre-flight host encoder + peer SEP validation before mutating active link preference */
    if (new_pref != LWASAPI_BT_PREF_AUTO) {
        uint32_t want_cap = LWASAPI_BT_CAP_SBC;
        bool lib_ok = true;
        const char *codec_label = "SBC";
        switch (new_pref) {
            case LWASAPI_BT_PREF_LDAC:
                want_cap = LWASAPI_BT_CAP_LDAC;
                lib_ok = load_ldac_library();
                codec_label = "LDAC";
                break;
            case LWASAPI_BT_PREF_APTX_HD:
                want_cap = LWASAPI_BT_CAP_APTX_HD;
                lib_ok = load_aptx_library();
                codec_label = "aptX HD";
                break;
            case LWASAPI_BT_PREF_APTX:
                want_cap = LWASAPI_BT_CAP_APTX;
                lib_ok = load_aptx_library();
                codec_label = "aptX";
                break;
            case LWASAPI_BT_PREF_AAC:
                want_cap = LWASAPI_BT_CAP_AAC;
                lib_ok = load_fdk_aac_library();
                codec_label = "AAC";
                break;
            default:
                break;
        }
        const char *cur_name =
            (bt->active_codec_kind == LWASAPI_BT_CODEC_LDAC) ? "LDAC" :
            (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX_HD) ? "aptX HD" :
            (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX) ? "aptX" :
            (bt->active_codec_kind == LWASAPI_BT_CODEC_AAC) ? "AAC" : "SBC";
        if (!lib_ok) {
            preflight_ok = false;
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "%s host encoder library missing — keeping %s", codec_label, cur_name);
            lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                               "BT_CODEC_PREFLIGHT", codec_label,
                               "Rejected host-missing codec without disturbing BlueZ");
        } else if (has_link && sup_mask != 0 && !(sup_mask & want_cap)) {
            preflight_ok = false;
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "Headset does not advertise %s SEP — keeping %s", codec_label, cur_name);
            lwasapi_debug_emit(LWASAPI_DBG_BLUEZ, LWASAPI_DBG_WARN, 0, 0,
                               "BT_CODEC_PREFLIGHT", codec_label,
                               "Headset lacks peer SEP for requested codec — active link preserved");
        }
    }

    if (preflight_ok) {
        bt->pref_codec = new_pref;
    } else {
        bt->pref_codec = old_pref;
    }
    bt->ldac_mode = new_ldac;
    bt->wireless_abr_enabled = new_abr;
    bt->hfp_sco_enabled = new_sco ? 1U : 0U;

    /* Live LDAC EQMID switching when active link is already LDAC */
    if (preflight_ok && bt->active_codec_kind == LWASAPI_BT_CODEC_LDAC && bt->ldac_enc && new_pref == old_pref) {
        if (new_ldac != LWASAPI_LDAC_MODE_ADAPTIVE && g_ldac.set_eqmid) {
            int eq = (new_ldac == LWASAPI_LDAC_MODE_990) ? LDACBT_EQMID_HQ :
                     (new_ldac == LWASAPI_LDAC_MODE_660) ? LDACBT_EQMID_SQ : LDACBT_EQMID_MQ;
            if (g_ldac.set_eqmid(bt->ldac_enc, eq) == 0) {
                bt->ldac_eqmid = eq;
                bt->effective_kbps = ldac_eqmid_to_kbps(bt->bt_sample_rate, eq);
                snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                         "LDAC locked manually -> %ukbps", bt->effective_kbps);
            }
        } else if (new_ldac == LWASAPI_LDAC_MODE_ADAPTIVE && new_abr) {
            if (!bt->ldac_abr && g_ldac_abr.available && g_ldac_abr.get_handle) {
                void *abr = g_ldac_abr.get_handle();
                if (abr) {
                    g_ldac_abr.init(abr, 20);
                    g_ldac_abr.set_thresholds(abr, 5, 3, 1);
                    bt->ldac_abr = abr;
                }
            }
            snprintf(bt->neg_reason, sizeof(bt->neg_reason),
                     "LDAC Adaptive Bitrate active (%ukbps)", bt->effective_kbps);
        }
    }

    /* Check if requested codec differs from currently negotiated active codec */
    uint32_t cur_pref_kind = (bt->active_codec_kind == LWASAPI_BT_CODEC_LDAC) ? LWASAPI_BT_PREF_LDAC :
                             (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX_HD) ? LWASAPI_BT_PREF_APTX_HD :
                             (bt->active_codec_kind == LWASAPI_BT_CODEC_APTX) ? LWASAPI_BT_PREF_APTX :
                             (bt->active_codec_kind == LWASAPI_BT_CODEC_AAC) ? LWASAPI_BT_PREF_AAC :
                             LWASAPI_BT_PREF_SBC;
    uint32_t target_pref = new_pref;
    if (target_pref == LWASAPI_BT_PREF_AUTO && sup_mask != 0) {
        if ((sup_mask & LWASAPI_BT_CAP_LDAC) && load_ldac_library()) target_pref = LWASAPI_BT_PREF_LDAC;
        else if ((sup_mask & LWASAPI_BT_CAP_APTX_HD) && load_aptx_library()) target_pref = LWASAPI_BT_PREF_APTX_HD;
        else if ((sup_mask & LWASAPI_BT_CAP_APTX) && load_aptx_library()) target_pref = LWASAPI_BT_PREF_APTX;
        else if ((sup_mask & LWASAPI_BT_CAP_AAC) && load_fdk_aac_library()) target_pref = LWASAPI_BT_PREF_AAC;
        else target_pref = LWASAPI_BT_PREF_SBC;
    }
    bool need_reneg = preflight_ok && (new_pref != old_pref) && (target_pref != cur_pref_kind);
    pthread_mutex_unlock(&bt->state_lock);

    if (!new_sco && strncmp(bt->cap_device_id, "bluez_sco:", 10) == 0) {
        bt_stop_sco_capture(bt);
    }
    if (need_reneg) {
        atomic_store(&bt->bt_renegotiate_req, 1U);
    }
    return 0;
}

void lwasapi_bluez_get_codec_config(const lwasapi_bluez_t *bt, lwasapi_bt_codec_cfg_t *out_cfg) {
    if (!out_cfg) return;
    memset(out_cfg, 0, sizeof(*out_cfg));
    if (!bt) return;
    pthread_mutex_lock((pthread_mutex_t *)&bt->state_lock);
    out_cfg->preferred_codec = bt->pref_codec;
    out_cfg->ldac_mode = bt->ldac_mode;
    out_cfg->wireless_abr_enabled = bt->wireless_abr_enabled;
    out_cfg->effective_kbps = bt->effective_kbps;
    out_cfg->supported_codecs_mask = bt->supported_codecs_mask;
    out_cfg->hfp_sco_enabled = bt->hfp_sco_enabled;
    out_cfg->hfp_sco_active = atomic_load(&bt->hfp_sco_active);
    snprintf(out_cfg->bt_sco_codec, sizeof(out_cfg->bt_sco_codec), "%s", bt->bt_sco_codec);
    snprintf(out_cfg->neg_reason, sizeof(out_cfg->neg_reason), "%s", bt->neg_reason);
    pthread_mutex_unlock((pthread_mutex_t *)&bt->state_lock);
}

int lwasapi_bluez_set_sco_enabled(lwasapi_bluez_t *bt, bool enabled) {
    if (!bt) return -1;
    pthread_mutex_lock(&bt->state_lock);
    bt->hfp_sco_enabled = enabled ? 1U : 0U;
    pthread_mutex_unlock(&bt->state_lock);
    if (!enabled) bt_stop_sco_capture(bt);
    return 0;
}

void lwasapi_bluez_close(lwasapi_bluez_t *bt) {
    if (!bt) return;
    stop_capture_thread(bt);
    bt_stop_sco_capture(bt);

    bt->tx_thread_running = 0;
    if (bt->tx_thread) {
        pthread_join(bt->tx_thread, NULL);
        bt->tx_thread = 0;
    }

    bt->dbus_thread_running = 0;
    if (bt->dbus_thread) {
        pthread_join(bt->dbus_thread, NULL);
        bt->dbus_thread = 0;
    }

    pthread_mutex_lock(&bt->state_lock);
    if (bt->bt_fd >= 0) {
        close(bt->bt_fd);
        bt->bt_fd = -1;
    }
    close_bt_encoders(bt);
    bt->direct_socket_mode = false;
    bt->has_bluez_transport = false;
    bt->initialized = false;
    pthread_mutex_unlock(&bt->state_lock);
    pthread_mutex_destroy(&bt->state_lock);
    g_bt_inst = NULL;
}
