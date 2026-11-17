#ifndef LWASAPI_HW_ANALOG_H
#define LWASAPI_HW_ANALOG_H

#include "lwasapi.h"
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Salsa v6.2-beta Hardware Analog Architecture, DAC Unity Attenuation,
 * Discrete Headphone Amp / Impedance Stage & USB-HID Control Box Support.
 *
 * Non-invasive by default: leaves active devices untouched until hardware
 * mixers are explicitly enabled or the user requests an amp/boost tier.
 */
typedef struct lwasapi_hw_analog_ctx lwasapi_hw_analog_ctx_t;

void lwasapi_hw_analog_default_cfg(lwasapi_hw_analog_cfg_t *cfg);
void lwasapi_hw_analog_clamp_cfg(lwasapi_hw_analog_cfg_t *cfg);

lwasapi_hw_analog_ctx_t *lwasapi_hw_analog_create(void);
void lwasapi_hw_analog_destroy(lwasapi_hw_analog_ctx_t *ctx);

/**
 * Probes /proc/asound/cardN, /sys/class/sound, /dev/snd/controlCN, and
 * /dev/hidraw* (ASUS Strix Control Box) for the active playback/capture cards.
 * Pass -1 when endpoint is virtual or Bluetooth A2DP.
 */
void lwasapi_hw_analog_probe(lwasapi_hw_analog_ctx_t *ctx, int pb_card, int cap_card);

/**
 * Applies config and current master volume/mute to kernel ALSA hardware mixer
 * elements cleanly via SNDRV_CTL_IOCTL_ELEM_WRITE without touching unrelated controls.
 */
void lwasapi_hw_analog_sync(lwasapi_hw_analog_ctx_t *ctx,
                            const lwasapi_hw_analog_cfg_t *cfg,
                            uint32_t desktop_vol_pct,
                            bool desktop_muted,
                            uint32_t mic_vol_pct,
                            bool mic_muted);

/** Non-blocking poll for ASUS Strix / USB-HID volume/mute dial events (outside RT thread). */
int lwasapi_hw_analog_poll_hid(lwasapi_hw_analog_ctx_t *ctx, int *out_vol_delta, int *out_mute_toggle);

/** Thread-safe snapshot of current hardware analog telemetry. */
void lwasapi_hw_analog_get_info(lwasapi_hw_analog_ctx_t *ctx, lwasapi_hw_analog_info_t *out_info);

/**
 * Lock-free audio-thread helpers updated atomically on probe/sync:
 * - is_dac_unity_active(): true when hardware ALSA DAC attenuator handles 0..100% volume.
 * - get_analog_drive_linear(): linear amp boost multiplier (1.0f when disabled/hardware-handled).
 * - get_analog_softknee_active(): true when analog soft-knee TPA6120/op-amp curve should shape boosts.
 * - get_cap_makeup_linear(): low-noise capture makeup multiplier (1.0f when mic_hw_boost_db == 0).
 */
bool  lwasapi_hw_analog_is_dac_unity_active(const lwasapi_hw_analog_ctx_t *ctx);
float lwasapi_hw_analog_get_drive_linear(const lwasapi_hw_analog_ctx_t *ctx);
bool  lwasapi_hw_analog_softknee_active(const lwasapi_hw_analog_ctx_t *ctx);
float lwasapi_hw_analog_get_cap_makeup_linear(const lwasapi_hw_analog_ctx_t *ctx);

/** Card scoring boost for ASUS Strix Raid DLX/Soar, Xonar, Sound Blaster, Scarlett & USB DACs. */
int   lwasapi_hw_analog_score_card(uint32_t card_index, const char *dev_name);

#ifdef __cplusplus
}
#endif

#endif /* LWASAPI_HW_ANALOG_H */
