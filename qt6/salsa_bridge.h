// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum {
    SALSA_EQ_BANDS = 10,
    SALSA_MAX_SESSIONS = 32,
    SALSA_MAX_DEVICES = 32,
};

enum {
    SALSA_FILTER_PEAK = 0,
    SALSA_FILTER_LOW_SHELF = 1,
    SALSA_FILTER_HIGH_SHELF = 2,
};

enum {
    SALSA_BT_PREF_AUTO = 0,
    SALSA_BT_PREF_LDAC = 1,
    SALSA_BT_PREF_APTX_HD = 2,
    SALSA_BT_PREF_APTX = 3,
    SALSA_BT_PREF_AAC = 4,
    SALSA_BT_PREF_SBC = 5,
};

enum {
    SALSA_LDAC_MODE_ABR = 0,
    SALSA_LDAC_MODE_990 = 1,
    SALSA_LDAC_MODE_660 = 2,
    SALSA_LDAC_MODE_330 = 3,
};

enum {
    SALSA_HW_AMP_AUTO = 0,
    SALSA_HW_AMP_LOW = 1,
    SALSA_HW_AMP_MED = 2,
    SALSA_HW_AMP_HIGH = 3,
};

enum {
    SALSA_HW_ROUTE_AUTO = 0,
    SALSA_HW_ROUTE_HP = 1,
    SALSA_HW_ROUTE_SPK = 2,
};

enum {
    SALSA_HW_CAP_DAC_VOL = 1 << 0,
    SALSA_HW_CAP_HP_AMP = 1 << 1,
    SALSA_HW_CAP_OUT_ROUTE = 1 << 2,
    SALSA_HW_CAP_MIC_BOOST = 1 << 3,
    SALSA_HW_CAP_LINE_AIR = 1 << 4,
    SALSA_HW_CAP_HID_BOX = 1 << 5,
    SALSA_HW_CAP_STRIX_ARCH = 1 << 6,
    SALSA_HW_CAP_HIGH_RES = 1 << 7,
};

typedef struct SalsaEqConfig {
    uint32_t enabled;
    uint32_t dynamic_mode;
    float preamp_db;
    float dyn_threshold_db;
    float dyn_ratio;
    uint32_t limiter_enabled;
    float band_gains_db[SALSA_EQ_BANDS];
    float band_q[SALSA_EQ_BANDS];
    uint8_t preset_name[32];
} SalsaEqConfig;

typedef struct SalsaParamEqExt {
    float band_freq_hz[SALSA_EQ_BANDS];
    uint32_t band_type[SALSA_EQ_BANDS];
    uint32_t crossfeed_enabled;
    float crossfeed_level;
    uint32_t crossfeed_cutoff_hz;
    uint32_t reserved;
} SalsaParamEqExt;

typedef struct SalsaMicApoCfg {
    uint32_t enabled;
    uint32_t hpf_enabled;
    float hpf_hz;
    uint32_t gate_enabled;
    float gate_thresh_db;
    uint32_t rnnoise_enabled;
    uint32_t comp_enabled;
    float comp_thresh_db;
    float comp_ratio;
    float clarity_db;
    uint32_t sidetone_enabled;
    uint32_t sidetone_pct;
} SalsaMicApoCfg;

typedef struct SalsaBtCodecCfg {
    uint32_t preferred_codec;
    uint32_t ldac_mode;
    uint32_t wireless_abr_enabled;
    uint32_t effective_kbps;
    uint32_t supported_codecs_mask;
    uint32_t hfp_sco_enabled;
    uint32_t hfp_sco_active;
    uint8_t bt_sco_codec[8];
    uint8_t neg_reason[64];
} SalsaBtCodecCfg;

typedef struct SalsaHwAnalogCfg {
    uint32_t hw_dac_unity_vol;
    uint32_t hp_amp_tier;
    uint32_t output_route;
    uint32_t unlock_0db_rails;
    float analog_boost_db;
    float mic_hw_boost_db;
} SalsaHwAnalogCfg;

typedef struct SalsaHwAnalogInfo {
    uint32_t caps_mask;
    uint32_t dac_unity_active;
    uint32_t hid_box_connected;
    int32_t active_pb_card;
    int32_t active_cap_card;
    uint32_t reserved;
    uint8_t card_arch_name[64];
    uint8_t dac_ctl_name[32];
    uint8_t amp_ctl_name[32];
    uint8_t cap_ctl_name[32];
    uint8_t diag_note[64];
} SalsaHwAnalogInfo;

typedef struct SalsaSessionInfo {
    uint32_t session_id;
    uint32_t client_pid;
    uint8_t client_name[64];
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
    uint8_t target_device[64];
} SalsaSessionInfo;

typedef struct SalsaDeviceEntry {
    uint8_t id[64];
    uint8_t name[64];
    uint8_t driver[32];
    uint32_t is_capture;
    uint32_t is_bluetooth;
    uint32_t is_active;
    uint32_t card_index;
    uint32_t device_index;
    uint32_t min_rate;
    uint32_t max_rate;
} SalsaDeviceEntry;

typedef struct SalsaDeviceList {
    uint32_t num_devices;
    SalsaDeviceEntry devices[SALSA_MAX_DEVICES];
} SalsaDeviceList;

typedef struct SalsaEngineStatus {
    uint8_t hw_device[64];
    uint8_t hw_capture_device[64];
    uint8_t midi_device[64];
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
    int32_t exclusive_owner_id;
    uint64_t total_cycles;
    uint64_t hw_xruns;
    uint64_t midi_events_routed;
    SalsaEqConfig eq_cfg;
    float eq_band_levels_db[SALSA_EQ_BANDS];
    float eq_dyn_delta_db[SALSA_EQ_BANDS];
    float output_peak_l;
    float output_peak_r;
    uint32_t num_sessions;
    SalsaSessionInfo sessions[SALSA_MAX_SESSIONS];
    uint32_t bt_active;
    uint32_t bt_delay_ms;
    uint8_t bt_alias[64];
    uint8_t bt_codec[16];
    uint32_t pa_server_running;
    uint32_t pa_clients;
    uint8_t active_playback_id[64];
    uint8_t active_capture_id[64];
    SalsaParamEqExt param_eq;
    SalsaMicApoCfg mic_apo;
    SalsaBtCodecCfg bt_codec_cfg;
    float mic_apo_peak_in;
    float mic_apo_peak_out;
    uint32_t mic_rnnoise_active;
    SalsaHwAnalogCfg hw_analog_cfg;
    SalsaHwAnalogInfo hw_analog_info;
} SalsaEngineStatus;

typedef struct SalsaScFrameMeta {
    int active_stream;
    int is_dmabuf;
    int dmabuf_mappable;
    uint32_t width;
    uint32_t height;
    uint32_t actual_fps_x100;
    uint32_t prod_node_id;
    uint32_t cons_count;
    uint32_t cast_fps_cap;
    uint64_t seq;
    size_t bytes_written;
    uint8_t prod_name[64];
    uint8_t format_name[32];
} SalsaScFrameMeta;

typedef void (*SalsaSniCb)(void* ctx, int kind, int x_or_delta, int y, const char* ori);

void salsa_str_free(char* ptr);
void salsa_bytes_free(uint8_t* ptr, size_t len);
bool salsa_abi_sizes_ok(
    size_t status_sz,
    size_t devlist_sz,
    size_t eq_sz,
    size_t ext_sz,
    size_t mic_sz,
    size_t bt_sz,
    size_t hwa_sz);
void salsa_reset_sigpipe(void);

int salsa_get_engine_status(SalsaEngineStatus* out);
int salsa_list_devices(SalsaDeviceList* out);
int salsa_set_devices(const char* pb_id, const char* cap_id);
int salsa_set_master_volume(int pct, int muted);
int salsa_set_mic_volume(int pct, int muted);
int salsa_set_session_control(uint32_t sid, int pct, int muted, const char* target_dev);
int salsa_set_engine_config(uint32_t rate, uint32_t period, uint32_t bits);
int salsa_set_capture_sample_rate(uint32_t rate);
int salsa_set_parametric_eq(const SalsaEqConfig* eq, const SalsaParamEqExt* ext);
int salsa_set_dynamic_eq(const SalsaEqConfig* eq);
int salsa_set_mic_apo(const SalsaMicApoCfg* mic);
int salsa_set_bt_codec_config(const SalsaBtCodecCfg* bt);
int salsa_set_hw_analog(const SalsaHwAnalogCfg* hwa);
char* salsa_probe_playback_caps_json(const char* pb_id);
int salsa_send_test_midi(uint8_t note, uint8_t vel);

char* salsa_host_profile_json(void);
char* salsa_install_root(void);
uint64_t salsa_gui_settings_mtime_ns(void);
char* salsa_load_gui_settings_json(void);
int salsa_save_studio_settings_json(const char* json_str);
int salsa_update_gui_settings_json(const char* patch_json);
int salsa_normalize_font_scale(int pct);
int salsa_save_font_scale(int pct);
int salsa_load_font_scale(void);

char* salsa_load_theme_json(bool for_tray);
char* salsa_get_theme_by_name_json(const char* name);
char* salsa_normalize_theme_json(const char* json_str);
char* salsa_derive_harmony_theme_json(const char* json_str);
char* salsa_all_theme_names_json(bool for_tray);
char* salsa_theme_token_groups_json(void);

char* salsa_builtin_eq_presets_json(void);
char* salsa_mic_apo_presets_json(void);
char* salsa_default_shortcuts_json(void);
char* salsa_saved_autoeq_list_json(void);
int salsa_save_autoeq_list_json(const char* list_json);
char* salsa_import_autoeq_txt_json(const char* path, const char* existing_list_json);
char* salsa_reload_autoeq_preset_json(const char* preset_json);
char* salsa_eq_cfg_to_json(const SalsaEqConfig* eq, const SalsaParamEqExt* ext);
int salsa_json_to_eq_cfg(const char* json_str, SalsaEqConfig* eq, SalsaParamEqExt* ext);
int salsa_apply_builtin_eq_preset(const char* name, SalsaEqConfig* eq, SalsaParamEqExt* ext);
int salsa_apply_saved_autoeq(const char* preset_json, SalsaEqConfig* eq, SalsaParamEqExt* ext);
void salsa_set_eq_preset_name(SalsaEqConfig* eq, const char* name);

char* salsa_manual_chapters_json(void);
char* salsa_render_manual_chapter_html(const char* key, const char* theme_json);
char* salsa_get_integration_status_json(void);
char* salsa_switch_engine_mode(bool to_pipewire);
char* salsa_toggle_pacman_hook(void);
char* salsa_toggle_autosync_watcher(bool enable);
char* salsa_run_sync_modules(bool force);
char* salsa_run_latency_bench(void);
char* salsa_run_health_audit_json(void);
char* salsa_bt_reset_sync(void);
int salsa_ctl_systemd(const char* unit, const char* action);

int salsa_debug_cli_main(int argc, const char* const* argv);
void* salsa_dbg_create(void);
void salsa_dbg_destroy(void* handle);
char* salsa_dbg_stats_json(void* handle);
char* salsa_dbg_latest_snapshot_json(void* handle);
char* salsa_dbg_recent_events_json(void* handle, size_t max_items);
size_t salsa_dbg_drain_events(void* handle, size_t max_items);
int salsa_dbg_send_command(void* handle, const char* cmd);
int salsa_dbg_request_snapshot(void* handle);
char* salsa_dbg_reset_net_clients_json(void* handle);
char* salsa_dbg_ping_json(void* handle, const char* tag);
int salsa_dbg_level_rank(const char* lvl);
char* salsa_dbg_format_ts_ms(int64_t ts_ms);
char* salsa_dbg_subsystems_json(void);
char* salsa_dbg_log_levels_json(void);
char* salsa_dbg_scan_procs_json(void);
char* salsa_dbg_full_audit_json(void);
char* salsa_dbg_run_probes_json(void);
char* salsa_dbg_play_tone(void);
char* salsa_dbg_send_midi(void);
char* salsa_dbg_export_bundle(void* handle);
char* salsa_dbg_ext_logs_json(bool journal, bool autosync, bool dbus);
char* salsa_dbg_load_sc_prefs_json(void);
int salsa_dbg_save_sc_prefs(const char* res_key, uint32_t fps_val, bool cap_real_cast);

void* salsa_dbg_shm_create(void);
void salsa_dbg_shm_destroy(void* ctx);
int salsa_dbg_shm_poll_frame(
    void* ctx,
    const char* shm_name,
    uint32_t max_w,
    uint32_t max_h,
    uint32_t tfps,
    uint32_t cap_fps,
    uint8_t* rgba_out,
    size_t rgba_cap,
    SalsaScFrameMeta* meta_out);
void salsa_dbg_shm_stop(void* ctx);
size_t salsa_dbg_dryrun_frame(void* ctx, uint32_t dw, uint32_t dh, uint8_t* rgba_out, size_t rgba_cap);
char* salsa_dbg_pw_self_test(void);
char* salsa_dbg_save_frame_png(const uint8_t* rgba, uint32_t w, uint32_t h);

void* salsa_sni_start(SalsaSniCb cb, void* user_ctx, char** icon_dir_out);
void salsa_sni_update(void* h, bool online, bool muted, bool glow, const char* title, const char* status);
void salsa_sni_stop(void* h);
uint8_t* salsa_tray_header_icon_png(void* h, size_t* len_out);

void* salsa_acquire_tray_lock(bool replace);
void salsa_release_tray_lock(void* h);
bool salsa_pid_in_tray_unit(void);
bool salsa_tray_unit_active_or_enabled(void);
bool salsa_display_is_x11(void);
void salsa_map_sni_point(int32_t rx, int32_t ry, int32_t* sx, int32_t* sy);
void salsa_plasma_panel_info(int32_t* loc, int32_t* clearance, int32_t* right_pad);
void salsa_monitor_geo_near(int32_t x, int32_t y, int32_t* mx, int32_t* my, int32_t* mw, int32_t* mh);
void salsa_apply_x11_popup_hints(uint64_t xid, bool skip_taskbar, bool above, int32_t w, int32_t h);
void salsa_apply_x11_osd_hints(uint64_t xid, int32_t w, int32_t h);
void salsa_move_x11_window(uint64_t xid, int32_t x, int32_t y);
double salsa_ui_scale(void);

char* salsa_version_str(void);
char* salsa_license_section_text(const char* app_name, int section);
int salsa_handle_legal_cli_flag(const char* app_name, const char* arg);

char* salsa_studio_selftest(void);
char* salsa_tray_selftest(void);
char* salsa_debug_selftest(void);

#ifdef __cplusplus
}
#endif
