#define _GNU_SOURCE
#include "lwasapi_hw_analog.h"
#include "lwasapi_debug.h"

#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/hidraw.h>
#include <linux/input.h>
#include <math.h>
#include <pthread.h>
#include <sound/asound.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

typedef struct {
    bool                  valid;
    struct snd_ctl_elem_id id;
    uint32_t              type;
    uint32_t              count;
    long                  imin;
    long                  imax;
    uint32_t              items;
    char                  name[64];
} hw_ctl_elem_t;

struct lwasapi_hw_analog_ctx {
    pthread_mutex_t      lock;
    int                  pb_card;
    int                  cap_card;
    uint32_t             caps_mask;
    char                 card_arch_name[64];
    char                 diag_note[64];

    /* Discovered ALSA controls on pb_card / cap_card */
    hw_ctl_elem_t        pb_dac_vol;
    hw_ctl_elem_t        pb_dac_sw;
    hw_ctl_elem_t        pb_hp_vol;
    hw_ctl_elem_t        pb_hp_sw;
    hw_ctl_elem_t        pb_spk_vol;
    hw_ctl_elem_t        pb_spk_sw;
    hw_ctl_elem_t        pb_amp_ctl;
    hw_ctl_elem_t        pb_route_ctl;
    hw_ctl_elem_t        cap_boost_ctl;
    hw_ctl_elem_t        cap_vol_ctl;
    hw_ctl_elem_t        cap_sw_ctl;

    /* Discovered ASUS Strix / HID Control Box monitor */
    int                  hid_fd;
    char                 hid_path[64];
    uint16_t             hid_vid;
    uint16_t             hid_pid;
    bool                 hid_has_consumer;
    bool                 hid_has_report_id;
    uint8_t              hid_consumer_report_id;
    bool                 hid_prev_up;
    bool                 hid_prev_dn;
    bool                 hid_prev_mute;

    /* Cached ALSA mixer write state to prevent redundant live ioctl clicks */
    bool                 sync_cache_valid;
    int                  last_pb_card;
    int                  last_cap_card;
    lwasapi_hw_analog_cfg_t last_cfg;
    uint32_t             last_d_vol;
    bool                 last_d_muted;
    bool                 last_m_muted;

    /* Atomic RT-safe audio thread mirrors */
    _Atomic int          dac_unity_active;
    _Atomic int          softknee_active;
    _Atomic uint32_t     drive_ppm;
    _Atomic uint32_t     cap_makeup_ppm;
};

static bool ci_contains(const char *hay, const char *needle) {
    if (!hay || !needle || !needle[0]) return false;
    size_t nlen = strlen(needle);
    for (const char *p = hay; *p; p++) {
        size_t i = 0;
        while (i < nlen && p[i] &&
               tolower((unsigned char)p[i]) == tolower((unsigned char)needle[i])) {
            i++;
        }
        if (i == nlen) return true;
    }
    return false;
}

static bool ci_eq(const char *a, const char *b) {
    if (!a || !b) return false;
    while (*a && *b) {
        if (tolower((unsigned char)*a) != tolower((unsigned char)*b)) return false;
        a++;
        b++;
    }
    return *a == '\0' && *b == '\0';
}

static bool read_small_text(const char *path, char *out, size_t out_len) {
    if (!path || !out || out_len == 0) return false;
    out[0] = '\0';
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return false;
    ssize_t n = read(fd, out, out_len - 1);
    close(fd);
    if (n <= 0) {
        out[0] = '\0';
        return false;
    }
    out[n] = '\0';
    while (n > 0 && (out[n - 1] == '\n' || out[n - 1] == '\r' || out[n - 1] == ' ')) {
        out[--n] = '\0';
    }
    return n > 0;
}

void lwasapi_hw_analog_default_cfg(lwasapi_hw_analog_cfg_t *cfg) {
    if (!cfg) return;
    memset(cfg, 0, sizeof(*cfg));
    cfg->hw_dac_unity_vol = 1U;                  /* Hardware DAC vol when card exposes Master/PCM/HP */
    cfg->hp_amp_tier      = LWASAPI_HW_AMP_AUTO; /* Preserve card amp firmware until user selects */
    cfg->output_route     = LWASAPI_HW_ROUTE_AUTO;
    cfg->unlock_0db_rails = 0U;                  /* Never touch existing mixer rails until requested */
    cfg->analog_boost_db  = 0.0f;
    cfg->mic_hw_boost_db  = 0.0f;
}

void lwasapi_hw_analog_clamp_cfg(lwasapi_hw_analog_cfg_t *cfg) {
    if (!cfg) return;
    cfg->hw_dac_unity_vol = cfg->hw_dac_unity_vol ? 1U : 0U;
    if (cfg->hp_amp_tier > LWASAPI_HW_AMP_HIGH) cfg->hp_amp_tier = LWASAPI_HW_AMP_AUTO;
    if (cfg->output_route > LWASAPI_HW_ROUTE_SPK) cfg->output_route = LWASAPI_HW_ROUTE_AUTO;
    cfg->unlock_0db_rails = cfg->unlock_0db_rails ? 1U : 0U;
    if (!(cfg->analog_boost_db >= 0.0f && cfg->analog_boost_db <= 12.0f)) {
        cfg->analog_boost_db = (cfg->analog_boost_db > 12.0f) ? 12.0f : 0.0f;
    }
    if (!(cfg->mic_hw_boost_db >= 0.0f && cfg->mic_hw_boost_db <= 24.0f)) {
        cfg->mic_hw_boost_db = (cfg->mic_hw_boost_db > 24.0f) ? 24.0f : 0.0f;
    }
}

lwasapi_hw_analog_ctx_t *lwasapi_hw_analog_create(void) {
    lwasapi_hw_analog_ctx_t *ctx = (lwasapi_hw_analog_ctx_t *)calloc(1, sizeof(*ctx));
    if (!ctx) return NULL;
    pthread_mutex_init(&ctx->lock, NULL);
    ctx->pb_card = -1;
    ctx->cap_card = -1;
    ctx->hid_fd = -1;
    atomic_init(&ctx->dac_unity_active, 0);
    atomic_init(&ctx->softknee_active, 0);
    atomic_init(&ctx->drive_ppm, 1000000U);
    atomic_init(&ctx->cap_makeup_ppm, 1000000U);
    snprintf(ctx->card_arch_name, sizeof(ctx->card_arch_name), "Virtual / Standby");
    snprintf(ctx->diag_note, sizeof(ctx->diag_note), "Preserve active hardware defaults");
    return ctx;
}

void lwasapi_hw_analog_destroy(lwasapi_hw_analog_ctx_t *ctx) {
    if (!ctx) return;
    pthread_mutex_lock(&ctx->lock);
    if (ctx->hid_fd >= 0) {
        close(ctx->hid_fd);
        ctx->hid_fd = -1;
    }
    pthread_mutex_unlock(&ctx->lock);
    pthread_mutex_destroy(&ctx->lock);
    free(ctx);
}

static void identify_card_architecture(int card, char *out_arch, size_t out_len, uint32_t *io_caps) {
    if (card < 0 || !out_arch || out_len == 0) return;
    char p_id[64], p_usb[64], p_drv[96];
    char cid[64] = {0}, usbid[32] = {0}, uevent[256] = {0};
    snprintf(p_id, sizeof(p_id), "/proc/asound/card%d/id", card);
    snprintf(p_usb, sizeof(p_usb), "/proc/asound/card%d/usbid", card);
    snprintf(p_drv, sizeof(p_drv), "/sys/class/sound/card%d/device/uevent", card);
    read_small_text(p_id, cid, sizeof(cid));
    read_small_text(p_usb, usbid, sizeof(usbid));
    read_small_text(p_drv, uevent, sizeof(uevent));

    /* Fallback check /proc/asound/cards name via stream0/id */
    char stream_head[256] = {0};
    char p_stream[64];
    snprintf(p_stream, sizeof(p_stream), "/proc/asound/card%d/stream0", card);
    read_small_text(p_stream, stream_head, sizeof(stream_head));

    if (ci_eq(usbid, "0b05:180c") || ci_contains(stream_head, "STRIX RAID DLX") || ci_contains(cid, "DLX")) {
        *io_caps |= (LWASAPI_HW_CAP_STRIX_ARCH | LWASAPI_HW_CAP_HIGH_RES);
        snprintf(out_arch, out_len, "ASUS STRIX RAID DLX (ES9016 + TPA6120A2)");
    } else if (ci_eq(usbid, "0b05:180d") || ci_contains(stream_head, "STRIX") || ci_contains(cid, "STRIX")) {
        *io_caps |= (LWASAPI_HW_CAP_STRIX_ARCH | LWASAPI_HW_CAP_HIGH_RES);
        snprintf(out_arch, out_len, "ASUS STRIX Sound Card (ES9016 + TPA6120A2)");
    } else if (ci_eq(usbid, "0b05:1808") || ci_eq(usbid, "0b05:1809") ||
               ci_contains(uevent, "virtuoso") || ci_contains(cid, "Xonar")) {
        *io_caps |= LWASAPI_HW_CAP_HIGH_RES;
        snprintf(out_arch, out_len, "ASUS Xonar Discrete DAC/Amp");
    } else if (strncmp(usbid, "1235:", 5) == 0 || ci_contains(stream_head, "Scarlett") || ci_contains(cid, "Gen")) {
        *io_caps |= LWASAPI_HW_CAP_HIGH_RES;
        snprintf(out_arch, out_len, "Focusrite Scarlett Pro Audio DAC/Preamp");
    } else if (strncmp(usbid, "041e:", 5) == 0 || ci_contains(uevent, "ca0132") || ci_contains(cid, "Creative")) {
        *io_caps |= LWASAPI_HW_CAP_HIGH_RES;
        snprintf(out_arch, out_len, "Creative Sound Blaster DAC / HP Amp");
    } else if (ci_contains(uevent, "snd_hda_intel") || ci_contains(cid, "Generic")) {
        snprintf(out_arch, out_len, "HD-Audio Codec (%.28s)", cid[0] ? cid : "Analog");
    } else if (usbid[0]) {
        *io_caps |= LWASAPI_HW_CAP_HIGH_RES;
        snprintf(out_arch, out_len, "USB Audio Class DAC (%s)", usbid);
    } else {
        snprintf(out_arch, out_len, "ALSA Card %d (%s)", card, cid[0] ? cid : "PCM");
    }
}

static bool hid_desc_has_consumer_vol(const uint8_t *desc, uint32_t len,
                                      bool *out_has_rid, uint8_t *out_rid) {
    if (out_has_rid) *out_has_rid = false;
    if (out_rid) *out_rid = 0;

    bool found_consumer = false;
    bool in_consumer_page = false;
    bool has_any_rid = false;
    uint8_t cur_rid = 0;
    bool cons_has_rid = false;
    uint8_t cons_rid = 0;

    for (uint32_t i = 0; i < len; ) {
        uint8_t b = desc[i];
        if (b == 0xFEU) {
            if (i + 1U >= len) break;
            uint32_t step = 3U + (uint32_t)desc[i + 1U];
            if (i + step > len) break;
            i += step;
            continue;
        }
        uint32_t sz = (uint32_t)(b & 0x03U);
        if (sz == 3U) sz = 4U;
        uint32_t step = 1U + sz;
        if (i + step > len) break;

        uint8_t tag_type = (uint8_t)(b & 0xFCU);
        if (tag_type == 0x04U && sz >= 1U) {
            uint32_t page = (sz >= 2U) ? ((uint32_t)desc[i + 1U] | ((uint32_t)desc[i + 2U] << 8))
                                       : (uint32_t)desc[i + 1U];
            in_consumer_page = (page == 0x0CU);
            if (in_consumer_page) {
                found_consumer = true;
                if (has_any_rid && !cons_has_rid) {
                    cons_has_rid = true;
                    cons_rid = cur_rid;
                }
            }
        } else if (tag_type == 0x84U && sz >= 1U) {
            has_any_rid = true;
            cur_rid = desc[i + 1U];
            if (in_consumer_page) {
                cons_has_rid = true;
                cons_rid = cur_rid;
            }
        } else if (in_consumer_page && (tag_type == 0x08U || tag_type == 0x80U)) {
            found_consumer = true;
            if (has_any_rid) {
                cons_has_rid = true;
                cons_rid = cur_rid;
            }
        }
        i += step;
    }

    if (found_consumer && cons_has_rid) {
        if (out_has_rid) *out_has_rid = true;
        if (out_rid) *out_rid = cons_rid;
    }
    return found_consumer;
}

static void probe_strix_hid_box(lwasapi_hw_analog_ctx_t *ctx) {
    if (ctx->hid_fd >= 0) {
        close(ctx->hid_fd);
        ctx->hid_fd = -1;
    }
    ctx->hid_path[0] = '\0';
    ctx->hid_vid = 0;
    ctx->hid_pid = 0;
    ctx->hid_has_consumer = false;
    ctx->hid_has_report_id = false;
    ctx->hid_consumer_report_id = 0;
    ctx->hid_prev_up = false;
    ctx->hid_prev_dn = false;
    ctx->hid_prev_mute = false;

    for (int idx = 0; idx < 24; idx++) {
        char hpath[32];
        snprintf(hpath, sizeof(hpath), "/dev/hidraw%d", idx);
        int fd = open(hpath, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
        if (fd < 0) continue;

        struct hidraw_devinfo rinfo;
        memset(&rinfo, 0, sizeof(rinfo));
        if (ioctl(fd, HIDIOCGRAWINFO, &rinfo) == 0 &&
            rinfo.bustype == BUS_USB &&
            (uint16_t)rinfo.vendor == 0x0B05U &&
            ((uint16_t)rinfo.product == 0x180CU || (uint16_t)rinfo.product == 0x180DU)) {
            int dsize = 0;
            bool has_cons = false;
            bool has_rid = false;
            uint8_t cons_rid = 0;
            if (ioctl(fd, HIDIOCGRDESCSIZE, &dsize) == 0 && dsize > 0 && dsize <= HID_MAX_DESCRIPTOR_SIZE) {
                struct hidraw_report_descriptor rdesc;
                memset(&rdesc, 0, sizeof(rdesc));
                rdesc.size = (uint32_t)dsize;
                if (ioctl(fd, HIDIOCGRDESC, &rdesc) == 0) {
                    has_cons = hid_desc_has_consumer_vol(rdesc.value, rdesc.size, &has_rid, &cons_rid);
                }
            }
            ctx->hid_fd = fd;
            snprintf(ctx->hid_path, sizeof(ctx->hid_path), "%s", hpath);
            ctx->hid_vid = (uint16_t)rinfo.vendor;
            ctx->hid_pid = (uint16_t)rinfo.product;
            ctx->hid_has_consumer = has_cons;
            ctx->hid_has_report_id = has_rid;
            ctx->hid_consumer_report_id = cons_rid;
            ctx->caps_mask |= LWASAPI_HW_CAP_HID_BOX;
            lwasapi_debug_emit(LWASAPI_DBG_ENGINE, LWASAPI_DBG_INFO, 0, 0,
                               "HW_STRIX_HID", hpath,
                               "Detected ASUS Strix HID Control Box %04x:%04x (consumer=%d rid=%d:%u)",
                               ctx->hid_vid, ctx->hid_pid, has_cons ? 1 : 0,
                               has_rid ? 1 : 0, (unsigned)cons_rid);
            return;
        }
        close(fd);
    }
}

static void record_elem(hw_ctl_elem_t *dst, const struct snd_ctl_elem_id *id,
                        const struct snd_ctl_elem_info *info, const char *name) {
    if (!dst || !id || !info || !name) return;
    dst->valid = true;
    dst->id    = *id;
    dst->type  = (uint32_t)info->type;
    dst->count = (uint32_t)info->count;
    if (info->type == SNDRV_CTL_ELEM_TYPE_INTEGER) {
        dst->imin = info->value.integer.min;
        dst->imax = info->value.integer.max;
    } else if (info->type == SNDRV_CTL_ELEM_TYPE_ENUMERATED) {
        dst->items = info->value.enumerated.items;
    }
    snprintf(dst->name, sizeof(dst->name), "%s", name);
}

static void scan_alsa_card_mixers(int card, bool for_capture, lwasapi_hw_analog_ctx_t *ctx) {
    if (card < 0) return;
    char ctl_path[32];
    snprintf(ctl_path, sizeof(ctl_path), "/dev/snd/controlC%d", card);
    int fd = open(ctl_path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) return;

    struct snd_ctl_elem_list el;
    memset(&el, 0, sizeof(el));
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &el) < 0 || el.count == 0) {
        close(fd);
        return;
    }
    uint32_t cnt = (el.count > 256U) ? 256U : el.count;
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

    int dac_prio = -1;
    for (uint32_t i = 0; i < el.used; i++) {
        if (pids[i].iface != SNDRV_CTL_ELEM_IFACE_MIXER) continue;
        struct snd_ctl_elem_info info;
        memset(&info, 0, sizeof(info));
        info.id = pids[i];
        if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_INFO, &info) < 0) continue;

        char nm[SNDRV_CTL_ELEM_ID_NAME_MAXLEN + 1];
        memcpy(nm, pids[i].name, SNDRV_CTL_ELEM_ID_NAME_MAXLEN);
        nm[SNDRV_CTL_ELEM_ID_NAME_MAXLEN] = '\0';

        if (!for_capture) {
            /* DAC Playback Volume / Switch elements (strictly avoiding Scarlett PCM 01..04 matrix routing) */
            if (info.type == SNDRV_CTL_ELEM_TYPE_INTEGER && info.value.integer.max > info.value.integer.min) {
                int pr = -1;
                if (ci_eq(nm, "Master Playback Volume") || ci_eq(nm, "Master")) pr = 100;
                else if (ci_eq(nm, "DAC Playback Volume") || ci_eq(nm, "DAC")) pr = 95;
                else if (ci_eq(nm, "PCM Playback Volume") || ci_eq(nm, "PCM")) pr = 90;
                else if (ci_eq(nm, "Headphone Playback Volume") || ci_eq(nm, "Headphone")) {
                    pr = 85;
                    record_elem(&ctx->pb_hp_vol, &pids[i], &info, nm);
                } else if (ci_eq(nm, "Front Playback Volume") || ci_eq(nm, "Speaker Playback Volume") ||
                           ci_eq(nm, "Line Out Playback Volume")) {
                    pr = 80;
                    record_elem(&ctx->pb_spk_vol, &pids[i], &info, nm);
                }
                if (pr > dac_prio) {
                    dac_prio = pr;
                    record_elem(&ctx->pb_dac_vol, &pids[i], &info, nm);
                    ctx->caps_mask |= LWASAPI_HW_CAP_DAC_VOL;
                }
            } else if (info.type == SNDRV_CTL_ELEM_TYPE_BOOLEAN) {
                if (ci_eq(nm, "Master Playback Switch") || ci_eq(nm, "PCM Playback Switch") ||
                    ci_eq(nm, "DAC Playback Switch")) {
                    record_elem(&ctx->pb_dac_sw, &pids[i], &info, nm);
                } else if (ci_eq(nm, "Headphone Playback Switch") || ci_eq(nm, "Headphone")) {
                    record_elem(&ctx->pb_hp_sw, &pids[i], &info, nm);
                } else if (ci_eq(nm, "Front Playback Switch") || ci_eq(nm, "Speaker Playback Switch") ||
                           ci_eq(nm, "Line Out Playback Switch")) {
                    record_elem(&ctx->pb_spk_sw, &pids[i], &info, nm);
                }
            }
            /* Headphone Amp / Impedance / Output Route across Xonar, Sound Blaster & HDA */
            if (ci_contains(nm, "HP Amp") || ci_contains(nm, "Headphone Gain") ||
                ci_contains(nm, "Headphone Amplifier") || ci_contains(nm, "Impedance") ||
                ci_contains(nm, "Output Gain")) {
                record_elem(&ctx->pb_amp_ctl, &pids[i], &info, nm);
                ctx->caps_mask |= LWASAPI_HW_CAP_HP_AMP;
            }
            if (ci_eq(nm, "Analog Output") || ci_eq(nm, "Output Select") ||
                ci_contains(nm, "Output Select") || ci_contains(nm, "Headphone Jack Mode")) {
                record_elem(&ctx->pb_route_ctl, &pids[i], &info, nm);
                ctx->caps_mask |= LWASAPI_HW_CAP_OUT_ROUTE;
            }
        } else {
            /* Capture Mic Boost / Preamp / Pro Interface detection */
            if (ci_contains(nm, "Line In 1 Air") || ci_contains(nm, "Phantom Power") ||
                ci_contains(nm, "Autogain")) {
                ctx->caps_mask |= LWASAPI_HW_CAP_LINE_AIR;
            }
            if (ci_contains(nm, "Mic Boost") && info.type == SNDRV_CTL_ELEM_TYPE_INTEGER) {
                if (!ctx->cap_boost_ctl.valid) {
                    record_elem(&ctx->cap_boost_ctl, &pids[i], &info, nm);
                    ctx->caps_mask |= LWASAPI_HW_CAP_MIC_BOOST;
                }
            } else if ((ci_eq(nm, "Capture Volume") || ci_eq(nm, "Mic Capture Volume") ||
                        ci_eq(nm, "Mic")) && info.type == SNDRV_CTL_ELEM_TYPE_INTEGER) {
                if (!ctx->cap_vol_ctl.valid) {
                    record_elem(&ctx->cap_vol_ctl, &pids[i], &info, nm);
                }
            } else if ((ci_eq(nm, "Capture Switch") || ci_eq(nm, "Mic Capture Switch")) &&
                       info.type == SNDRV_CTL_ELEM_TYPE_BOOLEAN) {
                if (!ctx->cap_sw_ctl.valid) {
                    record_elem(&ctx->cap_sw_ctl, &pids[i], &info, nm);
                }
            }
        }
    }

    if (!for_capture && ctx->pb_hp_sw.valid && ctx->pb_spk_sw.valid) {
        ctx->caps_mask |= LWASAPI_HW_CAP_OUT_ROUTE;
    }

    free(pids);
    close(fd);
}

void lwasapi_hw_analog_probe(lwasapi_hw_analog_ctx_t *ctx, int pb_card, int cap_card) {
    if (!ctx) return;
    pthread_mutex_lock(&ctx->lock);
    ctx->pb_card  = pb_card;
    ctx->cap_card = cap_card;
    ctx->caps_mask = 0;
    memset(&ctx->pb_dac_vol, 0, sizeof(ctx->pb_dac_vol));
    memset(&ctx->pb_dac_sw, 0, sizeof(ctx->pb_dac_sw));
    memset(&ctx->pb_hp_vol, 0, sizeof(ctx->pb_hp_vol));
    memset(&ctx->pb_hp_sw, 0, sizeof(ctx->pb_hp_sw));
    memset(&ctx->pb_spk_vol, 0, sizeof(ctx->pb_spk_vol));
    memset(&ctx->pb_spk_sw, 0, sizeof(ctx->pb_spk_sw));
    memset(&ctx->pb_amp_ctl, 0, sizeof(ctx->pb_amp_ctl));
    memset(&ctx->pb_route_ctl, 0, sizeof(ctx->pb_route_ctl));
    memset(&ctx->cap_boost_ctl, 0, sizeof(ctx->cap_boost_ctl));
    memset(&ctx->cap_vol_ctl, 0, sizeof(ctx->cap_vol_ctl));
    memset(&ctx->cap_sw_ctl, 0, sizeof(ctx->cap_sw_ctl));

    if (pb_card >= 0) {
        identify_card_architecture(pb_card, ctx->card_arch_name, sizeof(ctx->card_arch_name), &ctx->caps_mask);
        scan_alsa_card_mixers(pb_card, false, ctx);
    } else {
        snprintf(ctx->card_arch_name, sizeof(ctx->card_arch_name), "BlueZ / Virtual Stream");
    }
    if (cap_card >= 0) {
        if (pb_card < 0) {
            identify_card_architecture(cap_card, ctx->card_arch_name, sizeof(ctx->card_arch_name), &ctx->caps_mask);
        } else {
            uint32_t cap_flags = 0;
            char dummy_arch[64] = {0};
            identify_card_architecture(cap_card, dummy_arch, sizeof(dummy_arch), &cap_flags);
            ctx->caps_mask |= (cap_flags & (LWASAPI_HW_CAP_STRIX_ARCH | LWASAPI_HW_CAP_HIGH_RES));
        }
        scan_alsa_card_mixers(cap_card, true, ctx);
    }

    if ((ctx->caps_mask & LWASAPI_HW_CAP_STRIX_ARCH) != 0U) {
        probe_strix_hid_box(ctx);
    } else if (ctx->hid_fd >= 0) {
        close(ctx->hid_fd);
        ctx->hid_fd = -1;
        ctx->hid_path[0] = '\0';
    }

    if ((ctx->caps_mask & LWASAPI_HW_CAP_STRIX_ARCH) && !(ctx->caps_mask & LWASAPI_HW_CAP_HP_AMP)) {
        snprintf(ctx->diag_note, sizeof(ctx->diag_note),
                 "Strix CM6632AE UAC: DAC/Analog Soft-Amp + Preamp Ready");
    } else if (ctx->caps_mask & LWASAPI_HW_CAP_DAC_VOL) {
        snprintf(ctx->diag_note, sizeof(ctx->diag_note),
                 "ALSA Hardware DAC Unity Stage Active (%.24s)", ctx->pb_dac_vol.name);
    } else {
        snprintf(ctx->diag_note, sizeof(ctx->diag_note),
                 "Preserving Hardware Stage (Digital/AVRCP Nominal)");
    }

    ctx->sync_cache_valid = false;
    pthread_mutex_unlock(&ctx->lock);
}

static bool write_alsa_int(int fd, const hw_ctl_elem_t *el, long target_val) {
    if (fd < 0 || !el || !el->valid) return false;
    if (el->type != SNDRV_CTL_ELEM_TYPE_INTEGER && el->type != SNDRV_CTL_ELEM_TYPE_BOOLEAN) return false;
    struct snd_ctl_elem_value ev;
    memset(&ev, 0, sizeof(ev));
    ev.id = el->id;
    long v = target_val;
    if (el->type == SNDRV_CTL_ELEM_TYPE_INTEGER) {
        if (v < el->imin) v = el->imin;
        if (v > el->imax) v = el->imax;
    } else {
        v = v ? 1L : 0L;
    }
    uint32_t cnt = (el->count > 0 && el->count <= 16) ? el->count : 1U;
    for (uint32_t i = 0; i < cnt; i++) {
        ev.value.integer.value[i] = v;
    }
    return ioctl(fd, SNDRV_CTL_IOCTL_ELEM_WRITE, &ev) == 0;
}

static bool write_alsa_enum_match(int fd, const hw_ctl_elem_t *el, const char *kw1, const char *kw2, uint32_t fallback_idx) {
    if (fd < 0 || !el || !el->valid || el->type != SNDRV_CTL_ELEM_TYPE_ENUMERATED || el->items == 0) return false;
    uint32_t chosen = (fallback_idx < el->items) ? fallback_idx : 0U;
    for (uint32_t i = 0; i < el->items && i < 32U; i++) {
        struct snd_ctl_elem_info info;
        memset(&info, 0, sizeof(info));
        info.id = el->id;
        info.value.enumerated.item = i;
        if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_INFO, &info) == 0) {
            if ((kw1 && ci_contains(info.value.enumerated.name, kw1)) ||
                (kw2 && ci_contains(info.value.enumerated.name, kw2))) {
                chosen = i;
                break;
            }
        }
    }
    struct snd_ctl_elem_value ev;
    memset(&ev, 0, sizeof(ev));
    ev.id = el->id;
    uint32_t cnt = (el->count > 0 && el->count <= 16) ? el->count : 1U;
    for (uint32_t i = 0; i < cnt; i++) {
        ev.value.enumerated.item[i] = chosen;
    }
    return ioctl(fd, SNDRV_CTL_IOCTL_ELEM_WRITE, &ev) == 0;
}

static bool hw_analog_cfg_eq(const lwasapi_hw_analog_cfg_t *a, const lwasapi_hw_analog_cfg_t *b) {
    return a->hw_dac_unity_vol == b->hw_dac_unity_vol &&
           a->hp_amp_tier == b->hp_amp_tier &&
           a->output_route == b->output_route &&
           a->unlock_0db_rails == b->unlock_0db_rails &&
           fabsf(a->analog_boost_db - b->analog_boost_db) < 0.001f &&
           fabsf(a->mic_hw_boost_db - b->mic_hw_boost_db) < 0.001f;
}

void lwasapi_hw_analog_sync(lwasapi_hw_analog_ctx_t *ctx,
                            const lwasapi_hw_analog_cfg_t *cfg_in,
                            uint32_t desktop_vol_pct,
                            bool desktop_muted,
                            uint32_t mic_vol_pct,
                            bool mic_muted) {
    if (!ctx || !cfg_in) return;
    lwasapi_hw_analog_cfg_t cfg = *cfg_in;
    lwasapi_hw_analog_clamp_cfg(&cfg);
    (void)mic_vol_pct;

    pthread_mutex_lock(&ctx->lock);

    if (ctx->sync_cache_valid &&
        ctx->last_pb_card == ctx->pb_card &&
        ctx->last_cap_card == ctx->cap_card &&
        hw_analog_cfg_eq(&ctx->last_cfg, &cfg) &&
        ctx->last_d_vol == desktop_vol_pct &&
        ctx->last_d_muted == desktop_muted &&
        ctx->last_m_muted == mic_muted) {
        pthread_mutex_unlock(&ctx->lock);
        return;
    }

    bool dac_unity_ok = false;
    if (ctx->pb_card >= 0) {
        char ctl_path[32];
        snprintf(ctl_path, sizeof(ctl_path), "/dev/snd/controlC%d", ctx->pb_card);
        int pfd = open(ctl_path, O_RDWR | O_NONBLOCK | O_CLOEXEC);
        if (pfd >= 0) {
            /* 1. Explicit Output Route (when requested by user; never altered when AUTO) */
            if (cfg.output_route != LWASAPI_HW_ROUTE_AUTO) {
                if (ctx->pb_route_ctl.valid) {
                    if (cfg.output_route == LWASAPI_HW_ROUTE_HP) {
                        write_alsa_enum_match(pfd, &ctx->pb_route_ctl, "Headphone", "FP", 1U);
                    } else {
                        write_alsa_enum_match(pfd, &ctx->pb_route_ctl, "Speaker", "Line", 0U);
                    }
                }
                if (ctx->pb_hp_sw.valid && ctx->pb_spk_sw.valid) {
                    write_alsa_int(pfd, &ctx->pb_hp_sw, (cfg.output_route == LWASAPI_HW_ROUTE_HP) ? 1L : 0L);
                    write_alsa_int(pfd, &ctx->pb_spk_sw, (cfg.output_route == LWASAPI_HW_ROUTE_SPK) ? 1L : 0L);
                }
            }

            /* 2. Discrete Headphone Amp Gain / Impedance Stage (when explicitly selected) */
            bool hw_amp_applied = false;
            if (cfg.hp_amp_tier != LWASAPI_HW_AMP_AUTO && ctx->pb_amp_ctl.valid) {
                if (ctx->pb_amp_ctl.type == SNDRV_CTL_ELEM_TYPE_ENUMERATED) {
                    if (cfg.hp_amp_tier == LWASAPI_HW_AMP_LOW) {
                        hw_amp_applied = write_alsa_enum_match(pfd, &ctx->pb_amp_ctl, "Low", "32", 0U);
                    } else if (cfg.hp_amp_tier == LWASAPI_HW_AMP_MED) {
                        hw_amp_applied = write_alsa_enum_match(pfd, &ctx->pb_amp_ctl, "Med", "150", 1U);
                    } else {
                        hw_amp_applied = write_alsa_enum_match(pfd, &ctx->pb_amp_ctl, "High", "600", 2U);
                    }
                } else if (ctx->pb_amp_ctl.type == SNDRV_CTL_ELEM_TYPE_INTEGER) {
                    long span = ctx->pb_amp_ctl.imax - ctx->pb_amp_ctl.imin;
                    long target = (cfg.hp_amp_tier == LWASAPI_HW_AMP_LOW) ? ctx->pb_amp_ctl.imin :
                                  (cfg.hp_amp_tier == LWASAPI_HW_AMP_MED) ? (ctx->pb_amp_ctl.imin + span / 2L) :
                                                                            ctx->pb_amp_ctl.imax;
                    hw_amp_applied = write_alsa_int(pfd, &ctx->pb_amp_ctl, target);
                }
            }
            (void)hw_amp_applied;

            /* 3. Optional 0dB rail normalize (only when explicitly enabled by user or HP tier) */
            if (cfg.unlock_0db_rails) {
                if (ctx->pb_hp_vol.valid && (!ctx->pb_dac_vol.valid || !ci_eq(ctx->pb_hp_vol.name, ctx->pb_dac_vol.name))) {
                    write_alsa_int(pfd, &ctx->pb_hp_vol, ctx->pb_hp_vol.imax);
                }
                if (ctx->pb_spk_vol.valid && (!ctx->pb_dac_vol.valid || !ci_eq(ctx->pb_spk_vol.name, ctx->pb_dac_vol.name))) {
                    write_alsa_int(pfd, &ctx->pb_spk_vol, ctx->pb_spk_vol.imax);
                }
                if (cfg.output_route == LWASAPI_HW_ROUTE_AUTO) {
                    if (ctx->pb_hp_sw.valid) write_alsa_int(pfd, &ctx->pb_hp_sw, 1L);
                    if (ctx->pb_spk_sw.valid) write_alsa_int(pfd, &ctx->pb_spk_sw, 1L);
                }
            }

            /* 4. Bit-Perfect Unity Hardware DAC Volume (0..100% pushed to ALSA DAC mixer) */
            if (cfg.hw_dac_unity_vol && ctx->pb_dac_vol.valid) {
                uint32_t hw_pct = desktop_muted ? 0U : ((desktop_vol_pct > 100U) ? 100U : desktop_vol_pct);
                long span = ctx->pb_dac_vol.imax - ctx->pb_dac_vol.imin;
                long val = ctx->pb_dac_vol.imin + (long)(((double)hw_pct * (double)span) / 100.0 + 0.5);
                if (write_alsa_int(pfd, &ctx->pb_dac_vol, val)) {
                    dac_unity_ok = true;
                }
                if (ctx->pb_dac_sw.valid) {
                    write_alsa_int(pfd, &ctx->pb_dac_sw, desktop_muted ? 0L : 1L);
                }
            }
            close(pfd);
        }
    }

    /* Compute lock-free RT playback analog drive boost */
    float amp_stage_db = cfg.analog_boost_db;
    if (cfg.hp_amp_tier != LWASAPI_HW_AMP_AUTO && !ctx->pb_amp_ctl.valid) {
        /* Emulate TPA6120A2 / discrete HP stage gain tiers cleanly when UAC driver hides relay */
        if (cfg.hp_amp_tier == LWASAPI_HW_AMP_MED) amp_stage_db += 3.5f;
        else if (cfg.hp_amp_tier == LWASAPI_HW_AMP_HIGH) amp_stage_db += 7.0f;
    }
    if (amp_stage_db > 15.0f) amp_stage_db = 15.0f;
    float drive_lin = (amp_stage_db > 0.01f) ? powf(10.0f, amp_stage_db / 20.0f) : 1.0f;
    uint32_t drive_ppm = (uint32_t)(drive_lin * 1000000.0f + 0.5f);
    if (drive_ppm < 1000000U) drive_ppm = 1000000U;

    /* 5. Hardware Mic Preamp & Clean Strix UAC Capture Makeup (untouched when mic_hw_boost_db == 0) */
    float cap_makeup_db = cfg.mic_hw_boost_db;
    if (ctx->cap_card >= 0 && cfg.mic_hw_boost_db > 0.1f) {
        char cpath[32];
        snprintf(cpath, sizeof(cpath), "/dev/snd/controlC%d", ctx->cap_card);
        int cfd = open(cpath, O_RDWR | O_NONBLOCK | O_CLOEXEC);
        if (cfd >= 0) {
            if (ctx->cap_sw_ctl.valid) {
                write_alsa_int(cfd, &ctx->cap_sw_ctl, mic_muted ? 0L : 1L);
            }
            if (ctx->cap_vol_ctl.valid) {
                /* Open capture rail to max hardware headroom before makeup */
                write_alsa_int(cfd, &ctx->cap_vol_ctl, ctx->cap_vol_ctl.imax);
            }
            if (ctx->cap_boost_ctl.valid && ctx->cap_boost_ctl.imax > ctx->cap_boost_ctl.imin) {
                long span = ctx->cap_boost_ctl.imax - ctx->cap_boost_ctl.imin;
                long step = (cfg.mic_hw_boost_db >= 16.0f) ? ctx->cap_boost_ctl.imax :
                            (ctx->cap_boost_ctl.imin + (long)((cfg.mic_hw_boost_db / 24.0f) * (float)span + 0.5f));
                if (write_alsa_int(cfd, &ctx->cap_boost_ctl, step)) {
                    /* Native ALSA Mic Boost absorbed ~12..20 dB cleanly in hardware */
                    float absorbed = (step >= ctx->cap_boost_ctl.imax) ? 18.0f : 10.0f;
                    cap_makeup_db -= absorbed;
                    if (cap_makeup_db < 0.0f) cap_makeup_db = 0.0f;
                }
            }
            close(cfd);
        }
    }
    float cap_lin = (cap_makeup_db > 0.05f) ? powf(10.0f, cap_makeup_db / 20.0f) : 1.0f;
    uint32_t cap_ppm = (uint32_t)(cap_lin * 1000000.0f + 0.5f);
    if (cap_ppm < 1000000U) cap_ppm = 1000000U;

    atomic_store_explicit(&ctx->dac_unity_active, dac_unity_ok ? 1 : 0, memory_order_release);
    atomic_store_explicit(&ctx->softknee_active, (amp_stage_db > 0.05f) ? 1 : 0, memory_order_release);
    atomic_store_explicit(&ctx->drive_ppm, drive_ppm, memory_order_release);
    atomic_store_explicit(&ctx->cap_makeup_ppm, cap_ppm, memory_order_release);

    if (dac_unity_ok && amp_stage_db > 0.05f) {
        snprintf(ctx->diag_note, sizeof(ctx->diag_note),
                 "DAC Unity (%s) + %.1fdB Analog Amp Drive", ctx->pb_dac_vol.name, amp_stage_db);
    } else if (dac_unity_ok) {
        snprintf(ctx->diag_note, sizeof(ctx->diag_note),
                 "Bit-Perfect DAC Unity -> %s", ctx->pb_dac_vol.name);
    } else if (amp_stage_db > 0.05f || cfg.mic_hw_boost_db > 0.1f) {
        snprintf(ctx->diag_note, sizeof(ctx->diag_note),
                 "Amp +%.1fdB | HW Mic Preamp +%.1fdB", amp_stage_db, cfg.mic_hw_boost_db);
    } else {
        snprintf(ctx->diag_note, sizeof(ctx->diag_note),
                 "Hardware Defaults Preserved (Zero Alteration)");
    }

    ctx->last_pb_card = ctx->pb_card;
    ctx->last_cap_card = ctx->cap_card;
    ctx->last_cfg = cfg;
    ctx->last_d_vol = desktop_vol_pct;
    ctx->last_d_muted = desktop_muted;
    ctx->last_m_muted = mic_muted;
    ctx->sync_cache_valid = true;

    pthread_mutex_unlock(&ctx->lock);
}

int lwasapi_hw_analog_poll_hid(lwasapi_hw_analog_ctx_t *ctx, int *out_vol_delta, int *out_mute_toggle) {
    if (out_vol_delta) *out_vol_delta = 0;
    if (out_mute_toggle) *out_mute_toggle = 0;
    if (!ctx) return 0;

    pthread_mutex_lock(&ctx->lock);
    int fd = ctx->hid_fd;
    if (fd < 0 || !ctx->hid_has_consumer) {
        pthread_mutex_unlock(&ctx->lock);
        return 0;
    }
    uint8_t buf[64];
    int events = 0;
    for (int iter = 0; iter < 16; iter++) {
        ssize_t n = read(fd, buf, sizeof(buf));
        if (n <= 0) break;
        uint8_t bits = 0;
        if (ctx->hid_has_report_id) {
            if (n < 2 || n > 8 || buf[0] != ctx->hid_consumer_report_id) continue;
            bits = buf[1];
        } else {
            if (n < 1 || n > 8) continue;
            bits = (n >= 2) ? buf[1] : buf[0];
        }
        bool up   = (bits & 0x01U) != 0;
        bool dn   = (bits & 0x02U) != 0;
        bool mute = (bits & 0x04U) != 0;
        if (up && !ctx->hid_prev_up && out_vol_delta) { *out_vol_delta += 2; events++; }
        if (dn && !ctx->hid_prev_dn && out_vol_delta) { *out_vol_delta -= 2; events++; }
        if (mute && !ctx->hid_prev_mute && out_mute_toggle) { *out_mute_toggle = 1; events++; }
        ctx->hid_prev_up = up;
        ctx->hid_prev_dn = dn;
        ctx->hid_prev_mute = mute;
    }
    pthread_mutex_unlock(&ctx->lock);
    return events;
}

void lwasapi_hw_analog_get_info(lwasapi_hw_analog_ctx_t *ctx, lwasapi_hw_analog_info_t *out_info) {
    if (!out_info) return;
    memset(out_info, 0, sizeof(*out_info));
    if (!ctx) {
        out_info->active_pb_card = -1;
        out_info->active_cap_card = -1;
        return;
    }
    pthread_mutex_lock(&ctx->lock);
    out_info->caps_mask         = ctx->caps_mask;
    out_info->dac_unity_active  = (uint32_t)atomic_load_explicit(&ctx->dac_unity_active, memory_order_relaxed);
    out_info->hid_box_connected = (ctx->hid_fd >= 0) ? 1U : 0U;
    out_info->active_pb_card    = ctx->pb_card;
    out_info->active_cap_card   = ctx->cap_card;
    snprintf(out_info->card_arch_name, sizeof(out_info->card_arch_name), "%s", ctx->card_arch_name);
    snprintf(out_info->dac_ctl_name, sizeof(out_info->dac_ctl_name), "%.31s",
             ctx->pb_dac_vol.valid ? ctx->pb_dac_vol.name : "Software/AVRCP");
    if (ctx->pb_amp_ctl.valid) {
        snprintf(out_info->amp_ctl_name, sizeof(out_info->amp_ctl_name), "%.31s", ctx->pb_amp_ctl.name);
    } else if (ctx->pb_route_ctl.valid) {
        snprintf(out_info->amp_ctl_name, sizeof(out_info->amp_ctl_name), "%.31s", ctx->pb_route_ctl.name);
    } else if (ctx->caps_mask & LWASAPI_HW_CAP_STRIX_ARCH) {
        snprintf(out_info->amp_ctl_name, sizeof(out_info->amp_ctl_name), "TPA6120A2 Analog Stage");
    } else {
        snprintf(out_info->amp_ctl_name, sizeof(out_info->amp_ctl_name), "Default Output Stage");
    }
    snprintf(out_info->cap_ctl_name, sizeof(out_info->cap_ctl_name), "%.31s",
             ctx->cap_boost_ctl.valid ? ctx->cap_boost_ctl.name :
             (ctx->cap_vol_ctl.valid ? ctx->cap_vol_ctl.name : "Direct PCM Preamp"));
    snprintf(out_info->diag_note, sizeof(out_info->diag_note), "%s", ctx->diag_note);
    pthread_mutex_unlock(&ctx->lock);
}

bool lwasapi_hw_analog_is_dac_unity_active(const lwasapi_hw_analog_ctx_t *ctx) {
    return ctx && atomic_load_explicit(&ctx->dac_unity_active, memory_order_relaxed) != 0;
}

float lwasapi_hw_analog_get_drive_linear(const lwasapi_hw_analog_ctx_t *ctx) {
    if (!ctx) return 1.0f;
    uint32_t ppm = atomic_load_explicit(&ctx->drive_ppm, memory_order_relaxed);
    return (float)ppm * (1.0f / 1000000.0f);
}

bool lwasapi_hw_analog_softknee_active(const lwasapi_hw_analog_ctx_t *ctx) {
    return ctx && atomic_load_explicit(&ctx->softknee_active, memory_order_relaxed) != 0;
}

float lwasapi_hw_analog_get_cap_makeup_linear(const lwasapi_hw_analog_ctx_t *ctx) {
    if (!ctx) return 1.0f;
    uint32_t ppm = atomic_load_explicit(&ctx->cap_makeup_ppm, memory_order_relaxed);
    return (float)ppm * (1.0f / 1000000.0f);
}

int lwasapi_hw_analog_score_card(uint32_t card_index, const char *dev_name) {
    int sc = 0;
    char p_usb[64], p_drv[96], usbid[32] = {0}, uevent[256] = {0};
    snprintf(p_usb, sizeof(p_usb), "/proc/asound/card%u/usbid", card_index);
    snprintf(p_drv, sizeof(p_drv), "/sys/class/sound/card%u/device/uevent", card_index);
    bool is_usb = read_small_text(p_usb, usbid, sizeof(usbid));
    read_small_text(p_drv, uevent, sizeof(uevent));

    if (is_usb || access(p_usb, F_OK) == 0) sc += 100;
    if (dev_name && !ci_contains(dev_name, "HDMI") && !ci_contains(dev_name, "DisplayPort")) sc += 50;

    /* Enthusiast discrete DAC / studio interface priority (works for both USB and PCIe cards) */
    if (ci_contains(dev_name, "Scarlett") || strncmp(usbid, "1235:", 5) == 0) sc += 140;
    if (ci_contains(dev_name, "STRIX") || ci_eq(usbid, "0b05:180c") || ci_eq(usbid, "0b05:180d")) sc += 130;
    if (ci_contains(dev_name, "Xonar") || ci_contains(uevent, "virtuoso") ||
        ci_contains(dev_name, "Sound Blaster") || ci_contains(uevent, "ca0132") ||
        ci_contains(dev_name, "Essence") || ci_contains(dev_name, "Sabre")) {
        sc += 120;
    }
    if (ci_contains(dev_name, "Generic USB Audio") || ci_eq(usbid, "26ce:0a08")) {
        sc -= 20; /* Keep dedicated interfaces above motherboard generic USB codecs */
    }
    return sc;
}
