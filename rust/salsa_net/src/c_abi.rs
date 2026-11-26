// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicU32, AtomicU64};

pub const LASAPI_RING_FRAMES: usize = 32_768;
pub const LASAPI_MAX_CHANNELS: usize = 8;
pub const LASAPI_MIDI_RING_EVENTS: usize = 512;
pub const LASAPI_MIDI_RING_MASK: usize = LASAPI_MIDI_RING_EVENTS - 1;
pub const LASAPI_MAX_SESSIONS: usize = 32;
pub const LASAPI_EQ_BANDS: usize = 10;
pub const LASAPI_MAX_DEVICES: usize = 32;
pub const LASAPI_DEBUG_MAX_PA_CLIENTS: usize = 32;
pub const LASAPI_DEBUG_MAX_PA_STREAMS: usize = 48;
pub const LASAPI_DEBUG_MAX_PW_CLIENTS: usize = 32;
pub const LASAPI_DEBUG_MAX_PW_NODES: usize = 32;
pub const LASAPI_DEBUG_MAX_PW_LINKS: usize = 32;

pub const LASAPI_DBG_ENGINE: u8 = 0;
pub const LASAPI_DBG_PA: u8 = 2;
pub const LASAPI_DBG_MIDI: u8 = 4;
pub const LASAPI_DBG_SHIM: u8 = 5;
pub const LASAPI_DBG_PW_VIDEO: u8 = 7;

pub const LASAPI_DBG_TRACE: u8 = 0;
pub const LASAPI_DBG_INFO: u8 = 1;
pub const LASAPI_DBG_WARN: u8 = 2;
pub const LASAPI_DBG_ERR: u8 = 3;

pub const LASAPI_DEBUG_DGRAM_MAGIC: u32 = 0x4C44_4247;
pub const LASAPI_DEBUG_IN_SOCK_DEFAULT: &str = "/tmp/lasapi-debug-in.sock";

pub const S_OK: i32 = 0;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const AUDCLNT_SHAREMODE_SHARED: u32 = 0;
pub const AUDCLNT_SHAREMODE_EXCLUSIVE: u32 = 1;
pub const LASAPI_CFG_FLAG_CAPTURE_RATE: u32 = 0x4341_5052;

pub const LASAPI_CMD_GET_STATUS: u32 = 3;
pub const LASAPI_CMD_SET_VOLUME: u32 = 4;
pub const LASAPI_CMD_INJECT_MIDI: u32 = 5;
pub const LASAPI_CMD_SET_ENGINE_CONFIG: u32 = 6;
pub const LASAPI_CMD_SET_EQ: u32 = 7;
pub const LASAPI_CMD_LIST_DEVICES: u32 = 8;
pub const LASAPI_CMD_SET_DEVICES: u32 = 9;
pub const LASAPI_CMD_SET_SESSION_CONTROL: u32 = 10;
pub const LASAPI_CMD_SET_PARAM_EQ: u32 = 11;
pub const LASAPI_CMD_SET_MIC_APO: u32 = 12;
pub const LASAPI_CMD_SET_BT_CODEC: u32 = 14;
pub const LASAPI_CMD_SET_HW_ANALOG: u32 = 15;
pub const LASAPI_CMD_BT_CONNECTION: u32 = 16;
pub const LASAPI_CMD_SAVE_CONFIG: u32 = 17;
pub const LASAPI_CMD_RELOAD_CONFIG: u32 = 18;

pub const LASAPI_BT_CONN_QUERY: u32 = 0;
pub const LASAPI_BT_CONN_SET_OPTIONAL: u32 = 1;
pub const LASAPI_BT_CONN_SET_AUTOCONN: u32 = 2;
pub const LASAPI_BT_CONN_CONNECT: u32 = 3;
pub const LASAPI_BT_CONN_DISCONNECT: u32 = 4;

pub const LASAPI_BT_STAT_OPTIONAL: u32 = 1 << 0;
pub const LASAPI_BT_STAT_CONNECTED: u32 = 1 << 1;
pub const LASAPI_BT_STAT_TRANSPORT: u32 = 1 << 2;
pub const LASAPI_BT_STAT_USB_MATCHES_BT: u32 = 1 << 3;

pub const LASAPI_FILTER_PEAK: u32 = 0;
pub const LASAPI_FILTER_LOW_SHELF: u32 = 1;
pub const LASAPI_FILTER_HIGH_SHELF: u32 = 2;

pub const LASAPI_BT_PREF_AUTO: u32 = 0;
pub const LASAPI_BT_PREF_LDAC: u32 = 1;
pub const LASAPI_BT_PREF_APTX_HD: u32 = 2;
pub const LASAPI_BT_PREF_APTX: u32 = 3;
pub const LASAPI_BT_PREF_AAC: u32 = 4;
pub const LASAPI_BT_PREF_SBC: u32 = 5;

pub const LASAPI_LDAC_MODE_ADAPTIVE: u32 = 0;
pub const LASAPI_LDAC_MODE_990: u32 = 1;
pub const LASAPI_LDAC_MODE_660: u32 = 2;
pub const LASAPI_LDAC_MODE_330: u32 = 3;

pub const LASAPI_BT_CAP_LDAC: u32 = 1 << 0;
pub const LASAPI_BT_CAP_APTX_HD: u32 = 1 << 1;
pub const LASAPI_BT_CAP_APTX: u32 = 1 << 2;
pub const LASAPI_BT_CAP_AAC: u32 = 1 << 3;
pub const LASAPI_BT_CAP_SBC: u32 = 1 << 4;

pub const LASAPI_PW_PREVIEW_MAGIC: u32 = 0x534C_5057;
pub const LASAPI_PW_PREVIEW_VERSION: u32 = 2;
pub const LASAPI_PW_PREVIEW_MAX_W: u32 = 3840;
pub const LASAPI_PW_PREVIEW_MAX_H: u32 = 2160;
pub const LASAPI_PW_PREVIEW_MAX_BYTES: usize =
    (LASAPI_PW_PREVIEW_MAX_W as usize) * (LASAPI_PW_PREVIEW_MAX_H as usize) * 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiEqConfig {
    pub enabled: u32,
    pub dynamic_mode: u32,
    pub preamp_db: f32,
    pub dyn_threshold_db: f32,
    pub dyn_ratio: f32,
    pub limiter_enabled: u32,
    pub band_gains_db: [f32; LASAPI_EQ_BANDS],
    pub band_q: [f32; LASAPI_EQ_BANDS],
    pub preset_name: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiParamEqExt {
    pub band_freq_hz: [f32; LASAPI_EQ_BANDS],
    pub band_type: [u32; LASAPI_EQ_BANDS],
    pub crossfeed_enabled: u32,
    pub crossfeed_level: f32,
    pub crossfeed_cutoff_hz: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiMicApoCfg {
    pub enabled: u32,
    pub hpf_enabled: u32,
    pub hpf_hz: f32,
    pub gate_enabled: u32,
    pub gate_thresh_db: f32,
    pub rnnoise_enabled: u32,
    pub comp_enabled: u32,
    pub comp_thresh_db: f32,
    pub comp_ratio: f32,
    pub clarity_db: f32,
    pub sidetone_enabled: u32,
    pub sidetone_pct: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiBtCodecCfg {
    pub preferred_codec: u32,
    pub ldac_mode: u32,
    pub wireless_abr_enabled: u32,
    pub effective_kbps: u32,
    pub supported_codecs_mask: u32,
    pub hfp_sco_enabled: u32,
    pub hfp_sco_active: u32,
    pub bt_sco_codec: [u8; 8],
    pub neg_reason: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiHwAnalogCfg {
    pub hw_dac_unity_vol: u32,
    pub hp_amp_tier: u32,
    pub output_route: u32,
    pub unlock_0db_rails: u32,
    pub analog_boost_db: f32,
    pub mic_hw_boost_db: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiHwAnalogInfo {
    pub caps_mask: u32,
    pub dac_unity_active: u32,
    pub hid_box_connected: u32,
    pub active_pb_card: i32,
    pub active_cap_card: i32,
    pub reserved: u32,
    pub card_arch_name: [u8; 64],
    pub dac_ctl_name: [u8; 32],
    pub amp_ctl_name: [u8; 32],
    pub cap_ctl_name: [u8; 32],
    pub diag_note: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiSessionInfo {
    pub session_id: u32,
    pub client_pid: u32,
    pub client_name: [u8; 64],
    pub share_mode: u32,
    pub is_capture: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub active: u32,
    pub volume_milli: u32,
    pub buffered_frames: u64,
    pub xruns: u64,
    pub muted: u32,
    pub routing_flags: u32,
    pub target_device: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDeviceEntry {
    pub id: [u8; 64],
    pub name: [u8; 64],
    pub driver: [u8; 32],
    pub is_capture: u32,
    pub is_bluetooth: u32,
    pub is_active: u32,
    pub card_index: u32,
    pub device_index: u32,
    pub min_rate: u32,
    pub max_rate: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDeviceList {
    pub num_devices: u32,
    pub devices: [LasapiDeviceEntry; LASAPI_MAX_DEVICES],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiEngineStatus {
    pub hw_device: [u8; 64],
    pub hw_capture_device: [u8; 64],
    pub midi_device: [u8; 64],
    pub hw_sample_rate: u32,
    pub hw_channels: u32,
    pub hw_period_frames: u32,
    pub hw_buffer_frames: u32,
    pub hw_format_bits: u32,
    pub desktop_volume_pct: u32,
    pub desktop_muted: u32,
    pub desktop_mic_volume_pct: u32,
    pub desktop_mic_muted: u32,
    pub realtime_priority: u32,
    pub exclusive_owner_id: i32,
    pub total_cycles: u64,
    pub hw_xruns: u64,
    pub midi_events_routed: u64,
    pub eq_cfg: LasapiEqConfig,
    pub eq_band_levels_db: [f32; LASAPI_EQ_BANDS],
    pub eq_dyn_delta_db: [f32; LASAPI_EQ_BANDS],
    pub output_peak_l: f32,
    pub output_peak_r: f32,
    pub num_sessions: u32,
    pub sessions: [LasapiSessionInfo; LASAPI_MAX_SESSIONS],
    pub bt_active: u32,
    pub bt_delay_ms: u32,
    pub bt_alias: [u8; 64],
    pub bt_codec: [u8; 16],
    pub pa_server_running: u32,
    pub pa_clients: u32,
    pub active_playback_id: [u8; 64],
    pub active_capture_id: [u8; 64],
    pub param_eq: LasapiParamEqExt,
    pub mic_apo: LasapiMicApoCfg,
    pub bt_codec_cfg: LasapiBtCodecCfg,
    pub mic_apo_peak_in: f32,
    pub mic_apo_peak_out: f32,
    pub mic_rnnoise_active: u32,
    pub hw_analog_cfg: LasapiHwAnalogCfg,
    pub hw_analog_info: LasapiHwAnalogInfo,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiPaCallbacks {
    pub user_ctx: *mut c_void,
    pub get_status: Option<unsafe extern "C" fn(*mut c_void, *mut LasapiEngineStatus)>,
    pub get_devices: Option<unsafe extern "C" fn(*mut c_void, *mut LasapiDeviceList)>,
    pub set_devices: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int>,
    pub set_master_volume: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int>,
    pub set_mic_gain_ppm: Option<unsafe extern "C" fn(*mut c_void, u32, c_int, c_int) -> c_int>,
}

unsafe impl Send for LasapiPaCallbacks {}
unsafe impl Sync for LasapiPaCallbacks {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgPaClient {
    pub index: u32,
    pub pid: u32,
    pub name: [u8; 64],
    pub binary: [u8; 64],
    pub version: u32,
    pub authed: u8,
    pub sub_mask: u32,
    pub out_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgPaStream {
    pub index: u32,
    pub client_index: u32,
    pub pid: u32,
    pub name: [u8; 64],
    pub device_id: [u8; 64],
    pub is_playback: u8,
    pub is_monitor: u8,
    pub peak_detect: u8,
    pub direct_on_input: u32,
    pub target_dev_idx: u32,
    pub rate: u32,
    pub channels: u8,
    pub format: u8,
    pub active: u8,
    pub corked: u8,
    pub in_prebuf: u8,
    pub draining: u8,
    pub muted: u8,
    pub volume_milli: u32,
    pub frame_size: u32,
    pub buffered_frames: u64,
    pub ring_cap_frames: u32,
    pub tlength_frames: u32,
    pub prebuf_frames: u32,
    pub minreq_frames: u32,
    pub fragsize_frames: u32,
    pub mix_peak_l: f32,
    pub mix_peak_r: f32,
    pub write_index: u64,
    pub read_index: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgPaSnapshot {
    pub num_clients: u32,
    pub clients: [LasapiDbgPaClient; LASAPI_DEBUG_MAX_PA_CLIENTS],
    pub num_streams: u32,
    pub streams: [LasapiDbgPaStream; LASAPI_DEBUG_MAX_PA_STREAMS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgPwClient {
    pub id: u32,
    pub pid: u32,
    pub app_name: [u8; 64],
    pub binary: [u8; 64],
    pub portal_app_id: [u8; 64],
    pub is_portal: u8,
    pub pad: [u8; 3],
    pub num_nodes: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgPwNode {
    pub id: u32,
    pub client_id: u32,
    pub client_pid: u32,
    pub target_node_id: u32,
    pub linked_node_id: u32,
    pub name: [u8; 64],
    pub media_class: [u8; 48],
    pub format_name: [u8; 24],
    pub mem_type: [u8; 16],
    pub direction: u8,
    pub state: u8,
    pub is_driver: u8,
    pub alloc_buffers: u8,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_denom: u32,
    pub n_buffers: u32,
    pub drm_modifier: u64,
    pub frames: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgPwLink {
    pub id: u32,
    pub out_node_id: u32,
    pub in_node_id: u32,
    pub state: u8,
    pub is_dmabuf: u8,
    pub pad: [u8; 2],
    pub format_summary: [u8; 48],
    pub frames_relayed: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgPwSnapshot {
    pub broker_running: u8,
    pub zero_pw_mode: u8,
    pub preview_requested: u8,
    pub active_casting: u8,
    pub socket_path: [u8; 128],
    pub manager_socket_path: [u8; 128],
    pub preview_shm_name: [u8; 64],
    pub frames_relayed_total: u64,
    pub frames_dropped: u64,
    pub dmabuf_streams: u32,
    pub memfd_streams: u32,
    pub audio_rejects: u32,
    pub preview_target_w: u32,
    pub preview_target_h: u32,
    pub preview_target_fps: u32,
    pub cast_limit_w: u32,
    pub cast_limit_h: u32,
    pub cast_limit_fps: u32,
    pub actual_fps_x100: u32,
    pub num_clients: u32,
    pub clients: [LasapiDbgPwClient; LASAPI_DEBUG_MAX_PW_CLIENTS],
    pub num_nodes: u32,
    pub nodes: [LasapiDbgPwNode; LASAPI_DEBUG_MAX_PW_NODES],
    pub num_links: u32,
    pub links: [LasapiDbgPwLink; LASAPI_DEBUG_MAX_PW_LINKS],
}

#[repr(C)]
pub struct LasapiPwPreviewShm {
    pub magic: u32,
    pub version: u32,
    pub seqlock: u64,
    pub last_gui_ping_ns: u64,
    pub preview_requested: u32,
    pub active_stream: u32,
    pub is_dmabuf: u32,
    pub dmabuf_mappable: u32,
    pub src_width: u32,
    pub src_height: u32,
    pub src_stride: u32,
    pub src_spa_format: u32,
    pub drm_modifier_lo: u32,
    pub drm_modifier_hi: u32,
    pub dst_width: u32,
    pub dst_height: u32,
    pub dst_stride: u32,
    pub prod_node_id: u32,
    pub cons_count: u32,
    pub frame_seq: u64,
    pub pts_ns: u64,
    pub prod_name: [u8; 128],
    pub format_name: [u8; 32],
    pub target_max_w: u32,
    pub target_max_h: u32,
    pub target_fps: u32,
    pub actual_fps_x100: u32,
    pub cast_fps_cap: u32,
    pub reserved: [u8; 236],
    pub pixels: [u8; LASAPI_PW_PREVIEW_MAX_BYTES],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiPwBrokerOpts {
    pub runtime_dir: *const c_char,
    pub socket_name: *const c_char,
    pub ignore_zero_flag: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgDgram {
    pub magic: u32,
    pub pid: u32,
    pub session_id: u32,
    pub v1: i32,
    pub v2: i32,
    pub hr: i32,
    pub sub: u8,
    pub lvl: u8,
    pub reserved: u16,
    pub op: [u8; 28],
    pub target: [u8; 48],
    pub detail: [u8; 140],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LasapiDbgRtStats {
    pub cycle_count: u64,
    pub last_cycle_ns: u64,
    pub avg_cycle_ns: u64,
    pub max_cycle_ns: u64,
    pub min_cycle_ns: u64,
    pub last_jitter_ns: i64,
    pub max_jitter_ns: u64,
    pub eq_ns: u64,
    pub pa_mix_ns: u64,
    pub hw_write_ns: u64,
    pub budget_ns: u64,
    pub hw_xruns: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgLasapiSession {
    pub session_id: u32,
    pub client_pid: u32,
    pub client_name: [u8; 64],
    pub share_mode: u32,
    pub stream_flags: u32,
    pub is_capture: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub active: u32,
    pub muted: u32,
    pub volume_milli: u32,
    pub buffer_frames: u32,
    pub period_frames: u32,
    pub buffered_frames: u64,
    pub xruns: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgSnapshot {
    pub engine_st: LasapiEngineStatus,
    pub rt: LasapiDbgRtStats,
    pub target_playback_id: [u8; 64],
    pub target_capture_id: [u8; 64],

    pub bt_initialized: u8,
    pub bt_endpoints_registered: u8,
    pub bt_direct_socket: u8,
    pub bt_has_transport: u8,
    pub bt_tx_paused: u8,
    pub bt_device_path: [u8; 128],
    pub bt_transport_path: [u8; 160],
    pub bt_endpoint_path: [u8; 64],
    pub bt_sample_rate: u32,
    pub bt_channels: u32,
    pub bt_mtu_r: u16,
    pub bt_mtu_w: u16,
    pub bt_avrcp_vol: u16,
    pub bt_tx_ring_fill: u64,
    pub bt_tx_ring_cap: u32,
    pub bt_tx_packets: u64,
    pub bt_tx_underruns: u64,
    pub bt_tx_errors: u64,

    pub cap_active: u8,
    pub cap_device_id: [u8; 64],
    pub cap_pcm_path: [u8; 64],
    pub cap_alias: [u8; 64],
    pub cap_hw_rate: u32,
    pub cap_hw_channels: u32,
    pub cap_hw_format: i32,
    pub cap_ring_fill: u64,
    pub cap_pref_rate: u32,
    pub cap_measured_rate: u32,
    pub cap_kick_count: u32,
    pub cap_is_usb: u8,
    pub cap_duplex_locked: u8,

    pub midi_initialized: u8,
    pub midi_rawmidi_open: u8,
    pub midi_controller: [u8; 64],
    pub midi_dev_path: [u8; 64],
    pub midi_seq_client: i32,
    pub midi_seq_in_port: i32,
    pub midi_events_routed: u64,

    pub num_lasapi: u32,
    pub lasapi: [LasapiDbgLasapiSession; LASAPI_MAX_SESSIONS],
    pub pa: LasapiDbgPaSnapshot,
    pub pw: LasapiDbgPwSnapshot,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiDbgCallbacks {
    pub user_ctx: *mut c_void,
    pub fill_snapshot: Option<unsafe extern "C" fn(*mut c_void, *mut LasapiDbgSnapshot)>,
    pub handle_cmd: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char, usize) -> c_int>,
}

unsafe impl Send for LasapiDbgCallbacks {}
unsafe impl Sync for LasapiDbgCallbacks {}

impl Default for LasapiDbgCallbacks {
    fn default() -> Self {
        Self {
            user_ctx: core::ptr::null_mut(),
            fill_snapshot: None,
            handle_cmd: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LasapiMidiEvent {
    pub frame_offset: u32,
    pub size: u8,
    pub port: u8,
    pub data: [u8; 4],
    pub timestamp_ns: u64,
}

#[repr(C)]
pub struct LasapiMidiHub {
    pub initialized: bool,
    pub rawmidi_fd: c_int,
    pub controller_name: [u8; 64],
    pub dev_path: [u8; 64],
    pub alsa_dl: *mut c_void,
    pub seq_handle: *mut c_void,
    pub seq_client_id: c_int,
    pub seq_in_port: c_int,
    pub seq_out_port: c_int,
    pub running_status: u8,
    pub msg_buf: [u8; 3],
    pub msg_idx: c_int,
    pub msg_expected: c_int,
    pub total_events: AtomicU64,
}

#[repr(C, align(64))]
pub struct AlignedShmPcm<const N: usize>(pub [f32; N]);

#[repr(C)]
pub struct LasapiShmRing {
    pub magic: u32,
    pub session_id: u32,
    pub client_pid: u32,
    pub client_name: [u8; 64],
    pub share_mode: u32,
    pub stream_flags: u32,
    pub is_capture: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub format: u32,
    pub buffer_frames: u32,
    pub period_frames: u32,
    pub write_frame: AtomicU64,
    pub read_frame: AtomicU64,
    pub cap_write_frame: AtomicU64,
    pub cap_read_frame: AtomicU64,
    pub active: AtomicU32,
    pub muted: AtomicU32,
    pub volume_milli: AtomicU32,
    pub xruns: AtomicU64,
    pub hw_clock_ns: AtomicU64,
    pub midi_write_idx: AtomicU64,
    pub midi_read_idx: AtomicU64,
    pub midi_ring: [LasapiMidiEvent; LASAPI_MIDI_RING_EVENTS],
    pub pcm_data: AlignedShmPcm<{ LASAPI_RING_FRAMES * LASAPI_MAX_CHANNELS }>,
    pub cap_pcm_data: AlignedShmPcm<{ LASAPI_RING_FRAMES * 2 }>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiParamEqPayload {
    pub eq: LasapiEqConfig,
    pub ext: LasapiParamEqExt,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiIpcReq {
    pub cmd: u32,
    pub client_pid: u32,
    pub client_name: [u8; 64],
    pub share_mode: u32,
    pub stream_flags: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub format: u32,
    pub requested_buffer_frames: u32,
    pub requested_period_frames: u32,
    pub target_session_id: u32,
    pub volume_milli: u32,
    pub midi_ev: LasapiMidiEvent,
    pub eq_cfg: LasapiEqConfig,
    pub target_device: [u8; 64],
    pub target_capture_device: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LasapiIpcResp {
    pub hr: i32,
    pub session_id: u32,
    pub actual_sample_rate: u32,
    pub actual_channels: u32,
    pub actual_buffer_frames: u32,
    pub actual_period_frames: u32,
}

pub const LASAPI_RING_MASK: usize = LASAPI_RING_FRAMES - 1;
pub const LASAPI_MAX_PERIOD: usize = 4096;

pub const SALSA_PACK_S32_LE: u32 = 0;
pub const SALSA_PACK_S24_LE: u32 = 1;
pub const SALSA_PACK_S24_3LE: u32 = 2;
pub const SALSA_PACK_S16_LE: u32 = 3;
pub const SALSA_PACK_F32_LE: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaMicCubicState {
    pub hist_l: [f32; 3],
    pub hist_r: [f32; 3],
    pub valid: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaRuntimeState {
    pub sample_rate: u32,
    pub period_frames: u32,
    pub format_bits: u32,
    pub cap_sample_rate: u32,
    pub target_playback_id: [u8; 64],
    pub target_capture_id: [u8; 64],
    pub eq_cfg: LasapiEqConfig,
    pub param_eq: LasapiParamEqExt,
    pub mic_apo: LasapiMicApoCfg,
    pub bt_cfg: LasapiBtCodecCfg,
    pub hw_analog_cfg: LasapiHwAnalogCfg,
    pub desktop_volume_pct: u32,
    pub desktop_muted: u32,
    pub desktop_mic_volume_pct: u32,
    pub desktop_mic_muted: u32,
    pub desktop_mic_gain_ppm: u32,
    pub has_saved_vol_state: bool,
    pub bt_optional_mode: u32,
}

pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
pub const AUDCLNT_E_NOT_INITIALIZED: i32 = 0x8889_0001u32 as i32;
pub const AUDCLNT_E_ALREADY_INITIALIZED: i32 = 0x8889_0002u32 as i32;
pub const AUDCLNT_E_BUFFER_TOO_LARGE: i32 = 0x8889_0006u32 as i32;
pub const AUDCLNT_E_UNSUPPORTED_FORMAT: i32 = 0x8889_0008u32 as i32;
pub const AUDCLNT_E_DEVICE_IN_USE: i32 = 0x8889_000Au32 as i32;
pub const AUDCLNT_STREAMFLAGS_CROSSPROCESS: u32 = 0x0001_0000;
pub const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x0002_0000;
pub const AUDCLNT_STREAMFLAGS_EVENTCALLBACK: u32 = 0x0004_0000;
pub const AUDCLNT_STREAMFLAGS_NOPERSIST: u32 = 0x0008_0000;
pub const AUDCLNT_STREAMFLAGS_RATEADJUST: u32 = 0x0010_0000;
pub const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x8000_0000;
pub const AUDCLNT_STREAMFLAGS_CAPTURE: u32 = 0x0800_0000;
pub const AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY: u32 = 0x01;
pub const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x02;
pub const LASAPI_FMT_FLOAT32: u32 = 1;
pub const LASAPI_FMT_S16_LE: u32 = 2;
pub const LASAPI_FMT_S32_LE: u32 = 3;
pub const LASAPI_RING_MAGIC: u32 = 0x4C41_5341;
pub const LASAPI_CMD_CREATE_SESSION: u32 = 1;
pub const LASAPI_CMD_DESTROY_SESSION: u32 = 2;
pub const LASAPI_DBG_LASAPI: u8 = 1;
pub const LASAPI_DBG_BLUEZ: u8 = 3;
pub const LASAPI_DBG_ALSA: u8 = 6;
pub const LASAPI_BT_RING_FRAMES: u32 = 16_384;
pub const LASAPI_SOCKET_DEFAULT: &str = "/tmp/lasapi-audiodg.sock";

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaveFormatEx {
    pub w_format_tag: u16,
    pub n_channels: u16,
    pub n_samples_per_sec: u32,
    pub n_avg_bytes_per_sec: u32,
    pub n_block_align: u16,
    pub w_bits_per_sample: u16,
    pub cb_size: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaBtSnapshot {
    pub initialized: bool,
    pub has_bluez_transport: bool,
    pub direct_socket_mode: bool,
    pub has_capture_hw: bool,
    pub endpoints_registered: bool,
    pub tx_paused: bool,
    pub cap_is_usb: bool,
    pub desktop_muted: bool,
    pub desktop_mic_muted: bool,
    pub bt_fd: c_int,
    pub cap_pcm_fd: c_int,
    pub delay_tenths_ms: u16,
    pub read_mtu: u16,
    pub write_mtu: u16,
    pub avrcp_volume: u16,
    pub bt_sample_rate: u32,
    pub bt_channels: u32,
    pub cap_hw_rate: u32,
    pub cap_hw_channels: u32,
    pub cap_hw_format: i32,
    pub cap_measured_rate: u32,
    pub cap_kick_count: u32,
    pub desktop_volume_pct: u32,
    pub desktop_mic_volume_pct: u32,
    pub desktop_mic_gain_ppm: u32,
    pub tx_ring_fill: u64,
    pub tx_packets_sent: u64,
    pub tx_underruns: u64,
    pub tx_errors: u64,
    pub cap_hw_write_frame: u64,
    pub mac_address: [u8; 32],
    pub device_alias: [u8; 64],
    pub codec_name: [u8; 16],
    pub device_path: [u8; 128],
    pub transport_path: [u8; 160],
    pub endpoint_path: [u8; 64],
    pub cap_device_id: [u8; 64],
    pub cap_pcm_path: [u8; 64],
    pub capture_device_alias: [u8; 64],
}

pub fn copy_str_to_c(dst: &mut [u8], s: &str) {
    dst.fill(0);
    if dst.is_empty() {
        return;
    }
    let clean = s.as_bytes();
    let end = clean.iter().position(|&b| b == 0).unwrap_or(clean.len());
    let n = end.min(dst.len() - 1);
    dst[..n].copy_from_slice(&clean[..n]);
}

pub fn c_str_to_string(src: &[u8]) -> String {
    let end = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    String::from_utf8_lossy(&src[..end]).into_owned()
}

/// Converts an optional NUL-terminated C string pointer into a Rust `String`.
///
/// # Safety
/// If `p` is non-null, it must point to a valid NUL-terminated C string for the duration of this call.
pub unsafe fn ptr_to_opt_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        let s = unsafe { core::ffi::CStr::from_ptr(p) };
        Some(s.to_string_lossy().into_owned())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaBtOps {
    pub bt_ptr: Option<unsafe extern "C" fn() -> *mut c_void>,
    pub get_snapshot: Option<unsafe extern "C" fn(*const c_void, *mut SalsaBtSnapshot)>,
    pub rt_master_vol: Option<unsafe extern "C" fn(*const c_void, *mut u32, *mut bool)>,
    pub init_rates: Option<
        unsafe extern "C" fn(*mut c_void, u32, u32, *const c_char, *const c_char) -> c_int,
    >,
    pub set_preferred_target: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    pub reconfigure_rate: Option<unsafe extern "C" fn(*mut c_void, u32) -> c_int>,
    pub set_tx_enabled: Option<unsafe extern "C" fn(*mut c_void, bool)>,
    pub write_period: Option<unsafe extern "C" fn(*mut c_void, *const f32, u32, u32) -> c_int>,
    pub open_capture: Option<unsafe extern "C" fn(*mut c_void, *const c_char, u32) -> c_int>,
    pub read_capture_native: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut f32,
            u32,
            *mut u32,
            *mut u32,
            u32,
            u32,
            *mut u64,
        ) -> c_int,
    >,
    pub enumerate_devices: Option<
        unsafe extern "C" fn(
            *const c_void,
            *const c_char,
            *const c_char,
            *mut LasapiDeviceList,
        ) -> c_int,
    >,
    pub hw_fingerprint: Option<unsafe extern "C" fn(*const c_void) -> u64>,
    pub set_volume: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int>,
    pub set_mic_volume: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int>,
    pub set_mic_gain_ppm: Option<unsafe extern "C" fn(*mut c_void, u32, c_int, c_int) -> c_int>,
    pub set_codec_config: Option<unsafe extern "C" fn(*mut c_void, *const LasapiBtCodecCfg) -> c_int>,
    pub get_codec_config: Option<unsafe extern "C" fn(*const c_void, *mut LasapiBtCodecCfg)>,
    pub set_sco_enabled: Option<unsafe extern "C" fn(*mut c_void, bool) -> c_int>,
    pub close: Option<unsafe extern "C" fn(*mut c_void)>,
    pub reset_device: Option<unsafe extern "C" fn(*const c_char, *mut c_char, usize) -> c_int>,
}

static BT_OPS: std::sync::OnceLock<SalsaBtOps> = std::sync::OnceLock::new();

#[inline]
fn bt_ops() -> Option<&'static SalsaBtOps> {
    BT_OPS.get()
}

/// Register the C BlueZ / ALSA capture callback table before invoking `salsa_audiodg_main`
/// or `salsa_ctl_main`.
///
/// # Safety
/// `ops` must be null or point to a valid `SalsaBtOps` struct for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn salsa_register_bt_ops(ops: *const SalsaBtOps) {
    if !ops.is_null() {
        let _ = BT_OPS.set(unsafe { *ops });
    }
}

pub(crate) unsafe fn salsa_audiodg_bt_ptr() -> *mut c_void {
    if let Some(f) = bt_ops().and_then(|o| o.bt_ptr) {
        unsafe { f() }
    } else {
        #[cfg(not(test))]
        {
            crate::bluez_runtime::audiodg_bt_ptr_impl()
        }
        #[cfg(test)]
        {
            core::ptr::null_mut()
        }
    }
}

pub(crate) unsafe fn salsa_bt_is_initialized(bt: *const c_void) -> bool {
    if bt_ops().is_none() {
        #[cfg(not(test))]
        {
            return unsafe { crate::bluez_runtime::bt_is_initialized_impl(bt) };
        }
    }
    let mut snap: SalsaBtSnapshot = unsafe { core::mem::zeroed() };
    unsafe { salsa_bt_get_snapshot(bt, &mut snap) };
    snap.initialized
}

pub(crate) unsafe fn salsa_bt_get_snapshot(bt: *const c_void, out: *mut SalsaBtSnapshot) {
    if let Some(f) = bt_ops().and_then(|o| o.get_snapshot) {
        unsafe { f(bt, out) };
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bt_get_snapshot_impl(bt, out);
        }
        #[cfg(test)]
        if !out.is_null() {
            unsafe {
                *out = core::mem::zeroed();
                (*out).bt_fd = -1;
                (*out).cap_pcm_fd = -1;
                (*out).desktop_volume_pct = 100;
                (*out).desktop_mic_volume_pct = 100;
                (*out).desktop_mic_gain_ppm = 1_000_000;
            }
        }
    }
}

pub(crate) unsafe fn salsa_bt_rt_master_vol(
    bt: *const c_void,
    out_vol_pct: *mut u32,
    out_muted: *mut bool,
) {
    if let Some(f) = bt_ops().and_then(|o| o.rt_master_vol) {
        unsafe { f(bt, out_vol_pct, out_muted) };
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bt_rt_master_vol_impl(bt, out_vol_pct, out_muted);
        }
        #[cfg(test)]
        {
            if !out_vol_pct.is_null() {
                unsafe { *out_vol_pct = 100 };
            }
            if !out_muted.is_null() {
                unsafe { *out_muted = false };
            }
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_init_rates(
    bt: *mut c_void,
    engine_sample_rate: u32,
    capture_sample_rate: u32,
    preferred_cap_id: *const c_char,
    preferred_pb_id: *const c_char,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.init_rates) {
        unsafe { f(bt, engine_sample_rate, capture_sample_rate, preferred_cap_id, preferred_pb_id) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_init_rates_impl(
                bt,
                engine_sample_rate,
                capture_sample_rate,
                preferred_cap_id,
                preferred_pb_id,
            )
        }
        #[cfg(test)]
        {
            let _ = (bt, engine_sample_rate, capture_sample_rate, preferred_cap_id, preferred_pb_id);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_preferred_target(
    bt: *mut c_void,
    preferred_pb_id: *const c_char,
) {
    if let Some(f) = bt_ops().and_then(|o| o.set_preferred_target) {
        unsafe { f(bt, preferred_pb_id) };
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_set_preferred_target_impl(bt, preferred_pb_id);
        }
        #[cfg(test)]
        let _ = (bt, preferred_pb_id);
    }
}

pub(crate) unsafe fn lasapi_bluez_reconfigure_rate(
    bt: *mut c_void,
    new_engine_sample_rate: u32,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.reconfigure_rate) {
        unsafe { f(bt, new_engine_sample_rate) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_reconfigure_rate_impl(bt, new_engine_sample_rate)
        }
        #[cfg(test)]
        {
            let _ = (bt, new_engine_sample_rate);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_tx_enabled(bt: *mut c_void, enabled: bool) {
    if let Some(f) = bt_ops().and_then(|o| o.set_tx_enabled) {
        unsafe { f(bt, enabled) };
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_set_tx_enabled_impl(bt, enabled);
        }
        #[cfg(test)]
        let _ = (bt, enabled);
    }
}

pub(crate) unsafe fn lasapi_bluez_write_period(
    bt: *mut c_void,
    interleaved_float: *const f32,
    frames: u32,
    channels: u32,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.write_period) {
        unsafe { f(bt, interleaved_float, frames, channels) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_write_period_impl(bt, interleaved_float, frames, channels)
        }
        #[cfg(test)]
        {
            let _ = (bt, interleaved_float, frames, channels);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_open_capture(
    bt: *mut c_void,
    preferred_cap_id: *const c_char,
    capture_sample_rate: u32,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.open_capture) {
        unsafe { f(bt, preferred_cap_id, capture_sample_rate) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_open_capture_impl(
                bt,
                preferred_cap_id,
                capture_sample_rate,
            )
        }
        #[cfg(test)]
        {
            let _ = (bt, preferred_cap_id, capture_sample_rate);
            0
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn lasapi_bluez_read_capture_native(
    bt: *mut c_void,
    out_stereo_float: *mut f32,
    max_frames: u32,
    out_frames: *mut u32,
    out_rate: *mut u32,
    eng_frames: u32,
    eng_rate: u32,
    io_read_cursor: *mut u64,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.read_capture_native) {
        unsafe {
            f(
                bt,
                out_stereo_float,
                max_frames,
                out_frames,
                out_rate,
                eng_frames,
                eng_rate,
                io_read_cursor,
            )
        }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_read_capture_native_impl(
                bt,
                out_stereo_float,
                max_frames,
                out_frames,
                out_rate,
                eng_frames,
                eng_rate,
                io_read_cursor,
            )
        }
        #[cfg(test)]
        {
            let _ = (bt, max_frames, io_read_cursor);
            if !out_frames.is_null() {
                unsafe { *out_frames = eng_frames };
            }
            if !out_rate.is_null() {
                unsafe { *out_rate = eng_rate };
            }
            if !out_stereo_float.is_null() {
                unsafe { core::ptr::write_bytes(out_stereo_float, 0, (eng_frames as usize) * 2) };
            }
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_enumerate_devices(
    bt: *const c_void,
    active_pb_id: *const c_char,
    active_cap_id: *const c_char,
    out_list: *mut LasapiDeviceList,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.enumerate_devices) {
        unsafe { f(bt, active_pb_id, active_cap_id, out_list) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_enumerate_devices_impl(
                bt,
                active_pb_id,
                active_cap_id,
                out_list,
            )
        }
        #[cfg(test)]
        {
            let _ = (bt, active_pb_id, active_cap_id);
            if !out_list.is_null() {
                unsafe { core::ptr::write_bytes(out_list, 0, 1) };
            }
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_hw_fingerprint(bt: *const c_void) -> u64 {
    if let Some(f) = bt_ops().and_then(|o| o.hw_fingerprint) {
        unsafe { f(bt) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_hw_fingerprint_impl(bt)
        }
        #[cfg(test)]
        {
            let _ = bt;
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_volume(
    bt: *mut c_void,
    vol_pct: c_int,
    mute_flag: c_int,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.set_volume) {
        unsafe { f(bt, vol_pct, mute_flag) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_set_volume_impl(bt, vol_pct, mute_flag)
        }
        #[cfg(test)]
        {
            let _ = (bt, vol_pct, mute_flag);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_mic_volume(
    bt: *mut c_void,
    vol_pct: c_int,
    mute_flag: c_int,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.set_mic_volume) {
        unsafe { f(bt, vol_pct, mute_flag) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_set_mic_volume_impl(bt, vol_pct, mute_flag)
        }
        #[cfg(test)]
        {
            let _ = (bt, vol_pct, mute_flag);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_mic_gain_ppm(
    bt: *mut c_void,
    gain_ppm: u32,
    vol_pct: c_int,
    mute_flag: c_int,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.set_mic_gain_ppm) {
        unsafe { f(bt, gain_ppm, vol_pct, mute_flag) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_set_mic_gain_ppm_impl(bt, gain_ppm, vol_pct, mute_flag)
        }
        #[cfg(test)]
        {
            let _ = (bt, gain_ppm, vol_pct, mute_flag);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_codec_config(
    bt: *mut c_void,
    cfg: *const LasapiBtCodecCfg,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.set_codec_config) {
        unsafe { f(bt, cfg) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_set_codec_config_impl(bt, cfg)
        }
        #[cfg(test)]
        {
            let _ = (bt, cfg);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_get_codec_config(
    bt: *const c_void,
    out_cfg: *mut LasapiBtCodecCfg,
) {
    if let Some(f) = bt_ops().and_then(|o| o.get_codec_config) {
        unsafe { f(bt, out_cfg) };
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_get_codec_config_impl(bt, out_cfg);
        }
        #[cfg(test)]
        {
            let _ = bt;
            if !out_cfg.is_null() {
                unsafe { core::ptr::write_bytes(out_cfg, 0, 1) };
            }
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_sco_enabled(bt: *mut c_void, enabled: bool) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.set_sco_enabled) {
        unsafe { f(bt, enabled) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_set_sco_enabled_impl(bt, enabled)
        }
        #[cfg(test)]
        {
            let _ = (bt, enabled);
            0
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_close(bt: *mut c_void) {
    if let Some(f) = bt_ops().and_then(|o| o.close) {
        unsafe { f(bt) };
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_close_impl(bt);
        }
        #[cfg(test)]
        let _ = bt;
    }
}

pub(crate) unsafe fn lasapi_bluez_reset_device(
    mac_or_empty: *const c_char,
    out_msg: *mut c_char,
    out_len: usize,
) -> c_int {
    if let Some(f) = bt_ops().and_then(|o| o.reset_device) {
        unsafe { f(mac_or_empty, out_msg, out_len) }
    } else {
        #[cfg(not(test))]
        unsafe {
            crate::bluez_runtime::bluez_reset_device_impl(mac_or_empty, out_msg, out_len)
        }
        #[cfg(test)]
        {
            let _ = (mac_or_empty, out_msg, out_len);
            -1
        }
    }
}

pub(crate) unsafe fn lasapi_bluez_set_optional_mode(bt: *mut c_void, optional: bool) {
    #[cfg(not(test))]
    unsafe {
        crate::bluez_runtime::bluez_set_optional_mode_impl(bt, optional);
    }
    #[cfg(test)]
    let _ = (bt, optional);
}

pub(crate) unsafe fn lasapi_bluez_is_optional_mode(bt: *const c_void) -> bool {
    #[cfg(not(test))]
    {
        unsafe { crate::bluez_runtime::bluez_is_optional_mode_impl(bt) }
    }
    #[cfg(test)]
    {
        let _ = bt;
        true
    }
}

pub(crate) unsafe fn lasapi_bluez_known_audio_aliases(bt: *const c_void) -> Vec<String> {
    #[cfg(not(test))]
    {
        unsafe { crate::bluez_runtime::bluez_known_audio_aliases_impl(bt) }
    }
    #[cfg(test)]
    {
        let _ = bt;
        Vec::new()
    }
}

pub(crate) unsafe fn lasapi_bluez_connection_ctl(
    bt: *mut c_void,
    action: u32,
    mac_or_empty: *const c_char,
    out_msg: *mut c_char,
    out_len: usize,
    out_flags: *mut u32,
) -> c_int {
    #[cfg(not(test))]
    {
        unsafe {
            crate::bluez_runtime::bluez_connection_ctl_impl(
                bt,
                action,
                mac_or_empty,
                out_msg,
                out_len,
                out_flags,
            )
        }
    }
    #[cfg(test)]
    {
        let _ = (bt, action, mac_or_empty, out_msg, out_len);
        if !out_flags.is_null() {
            unsafe { *out_flags = LASAPI_BT_STAT_OPTIONAL };
        }
        0
    }
}

/// Runs a `extern "C" fn(argc, argv) -> c_int` entrypoint using process arguments and exits with its status code.
pub fn run_c_main(entry: unsafe extern "C" fn(c_int, *const *const c_char) -> c_int) -> ! {
    use std::ffi::CString;
    let c_args: Vec<CString> = std::env::args_os()
        .map(|a| {
            let bytes = a.into_encoded_bytes();
            CString::new(bytes).unwrap_or_else(|_| CString::new("").unwrap())
        })
        .collect();
    let ptrs: Vec<*const c_char> = c_args.iter().map(|s| s.as_ptr()).collect();
    let argc = ptrs.len() as c_int;
    let code = unsafe { entry(argc, ptrs.as_ptr()) };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn c_abi_struct_sizes_match_gcc_headers() {
        assert_eq!(size_of::<LasapiEngineStatus>(), 7136);
        assert_eq!(size_of::<LasapiDeviceList>(), 6020);
        assert_eq!(size_of::<LasapiSessionInfo>(), 184);
        assert_eq!(size_of::<LasapiDbgPaClient>(), 152);
        assert_eq!(size_of::<LasapiDbgPaStream>(), 232);
        assert_eq!(size_of::<LasapiDbgPaSnapshot>(), 16008);
        assert_eq!(size_of::<LasapiDbgPwClient>(), 208);
        assert_eq!(size_of::<LasapiDbgPwNode>(), 216);
        assert_eq!(size_of::<LasapiDbgPwLink>(), 72);
        assert_eq!(size_of::<LasapiDbgPwSnapshot>(), 16272);
        assert_eq!(size_of::<LasapiDbgDgram>(), 244);
        assert_eq!(size_of::<LasapiDbgRtStats>(), 96);
        assert_eq!(size_of::<LasapiDbgLasapiSession>(), 128);
        assert_eq!(size_of::<LasapiDbgCallbacks>(), 24);
        assert_eq!(size_of::<LasapiDbgSnapshot>(), 44536);
        assert_eq!(offset_of!(LasapiDbgSnapshot, target_playback_id), 7232);
        assert_eq!(offset_of!(LasapiDbgSnapshot, bt_initialized), 7360);
        assert_eq!(offset_of!(LasapiDbgSnapshot, cap_active), 7776);
        assert_eq!(offset_of!(LasapiDbgSnapshot, midi_initialized), 8006);
        assert_eq!(offset_of!(LasapiDbgSnapshot, num_lasapi), 8152);
        assert_eq!(offset_of!(LasapiDbgSnapshot, pa), 12256);
        assert_eq!(offset_of!(LasapiDbgSnapshot, pw), 28264);
        assert_eq!(offset_of!(LasapiPwPreviewShm, pixels), 520);
        assert_eq!(size_of::<LasapiPwPreviewShm>(), 33_178_120);
        assert_eq!(size_of::<LasapiMidiEvent>(), 24);
        assert_eq!(size_of::<LasapiMidiHub>(), 184);
        assert_eq!(offset_of!(LasapiMidiHub, dev_path), 72);
        assert_eq!(offset_of!(LasapiMidiHub, alsa_dl), 136);
        assert_eq!(offset_of!(LasapiMidiHub, running_status), 164);
        assert_eq!(offset_of!(LasapiMidiHub, total_events), 176);
        assert_eq!(size_of::<LasapiShmRing>(), 1_323_200);
        assert_eq!(offset_of!(LasapiShmRing, midi_write_idx), 176);
        assert_eq!(offset_of!(LasapiShmRing, midi_read_idx), 184);
        assert_eq!(offset_of!(LasapiShmRing, midi_ring), 192);
        assert_eq!(offset_of!(LasapiShmRing, pcm_data), 12_480);
        assert_eq!(offset_of!(LasapiShmRing, cap_pcm_data), 1_061_056);
        assert_eq!(size_of::<LasapiIpcReq>(), 400);
        assert_eq!(offset_of!(LasapiIpcReq, client_pid), 4);
        assert_eq!(offset_of!(LasapiIpcReq, client_name), 8);
        assert_eq!(offset_of!(LasapiIpcReq, stream_flags), 76);
        assert_eq!(offset_of!(LasapiIpcReq, midi_ev), 112);
        assert_eq!(offset_of!(LasapiIpcReq, eq_cfg), 136);
        assert_eq!(offset_of!(LasapiIpcReq, target_device), 272);
        assert_eq!(offset_of!(LasapiIpcReq, target_capture_device), 336);
        assert_eq!(size_of::<LasapiIpcResp>(), 24);
        assert_eq!(size_of::<LasapiParamEqPayload>(), 232);
        assert_eq!(size_of::<SalsaMicCubicState>(), 28);
        assert_eq!(offset_of!(SalsaMicCubicState, valid), 24);
        assert_eq!(size_of::<SalsaRuntimeState>(), 576);
        assert_eq!(offset_of!(SalsaRuntimeState, eq_cfg), 144);
        assert_eq!(offset_of!(SalsaRuntimeState, hw_analog_cfg), 524);
        assert_eq!(offset_of!(SalsaRuntimeState, has_saved_vol_state), 568);
        assert_eq!(offset_of!(SalsaRuntimeState, bt_optional_mode), 572);
        assert_eq!(size_of::<LasapiEngineStatus>(), 7_136);
        assert_eq!(offset_of!(LasapiEngineStatus, eq_cfg), 264);
        assert_eq!(offset_of!(LasapiEngineStatus, sessions), 496);
        assert_eq!(offset_of!(LasapiEngineStatus, bt_codec_cfg), 6_752);
        assert_eq!(offset_of!(LasapiEngineStatus, hw_analog_cfg), 6_864);
        assert_eq!(size_of::<SalsaBtSnapshot>(), 768);
        assert_eq!(size_of::<SalsaBtOps>(), 160);
        assert_eq!(size_of::<WaveFormatEx>(), 20);
    }
}
