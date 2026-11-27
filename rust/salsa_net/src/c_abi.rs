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
pub const LASAPI_CMD_CONTROL_V2: u32 = 0x5343_5432;

pub const LASAPI_BT_CONN_QUERY: u32 = 0;
pub const LASAPI_BT_CONN_SET_OPTIONAL: u32 = 1;
pub const LASAPI_BT_CONN_SET_AUTOCONN: u32 = 2;
pub const LASAPI_BT_CONN_CONNECT: u32 = 3;
pub const LASAPI_BT_CONN_DISCONNECT: u32 = 4;
pub const LASAPI_BT_CONN_RESET: u32 = 5;

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
    pub session_num: u32,
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

impl LasapiEngineStatus {
    pub fn effective_capture_rate_hz(&self) -> u32 {
        if (8_000..=384_000).contains(&self.hw_analog_info.reserved) {
            self.hw_analog_info.reserved
        } else {
            self.hw_sample_rate
        }
    }
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

impl Default for LasapiMidiHub {
    fn default() -> Self {
        Self {
            initialized: false,
            rawmidi_fd: -1,
            controller_name: [0; 64],
            dev_path: [0; 64],
            alsa_dl: core::ptr::null_mut(),
            seq_handle: core::ptr::null_mut(),
            seq_client_id: 0,
            seq_in_port: 0,
            seq_out_port: 0,
            running_status: 0,
            msg_buf: [0; 3],
            msg_idx: 0,
            msg_expected: 0,
            total_events: AtomicU64::new(0),
        }
    }
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

mod sealed_pod {
    pub trait Sealed {}
}

pub trait PodBytes: Copy + sealed_pod::Sealed {
    fn zeroed() -> Self {
        unsafe { core::mem::zeroed() }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let sz = core::mem::size_of::<Self>();
        let mut out = vec![0u8; sz];
        unsafe {
            core::ptr::copy_nonoverlapping((self as *const Self).cast::<u8>(), out.as_mut_ptr(), sz);
        }
        out
    }

    fn from_bytes(src: &[u8]) -> Option<Self> {
        let sz = core::mem::size_of::<Self>();
        if src.len() != sz {
            return None;
        }
        let mut val: Self = unsafe { core::mem::zeroed() };
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), (&raw mut val).cast::<u8>(), sz);
        }
        Some(val)
    }
}

macro_rules! impl_pod_bytes {
    ($($t:ty),* $(,)?) => {
        $(
            impl sealed_pod::Sealed for $t {}
            impl PodBytes for $t {}
        )*
    };
}

impl_pod_bytes!(
    LasapiEqConfig,
    LasapiParamEqExt,
    LasapiParamEqPayload,
    LasapiMicApoCfg,
    LasapiBtCodecCfg,
    LasapiHwAnalogCfg,
    LasapiHwAnalogInfo,
    LasapiSessionInfo,
    LasapiMidiEvent,
    LasapiDeviceEntry,
    LasapiDeviceList,
    LasapiEngineStatus,
    LasapiIpcReq,
    LasapiIpcResp,
    SalsaBtSnapshot,
    SalsaMicCubicState,
);

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

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn c_abi_struct_sizes_match_gcc_headers() {
        assert_eq!(size_of::<LasapiEngineStatus>(), 7136);
        assert_eq!(size_of::<LasapiDeviceList>(), 6020);
        assert_eq!(size_of::<LasapiSessionInfo>(), 184);
        assert_eq!(size_of::<LasapiDbgDgram>(), 244);
        assert_eq!(size_of::<LasapiDbgRtStats>(), 96);
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
        assert_eq!(size_of::<WaveFormatEx>(), 20);
    }
}
