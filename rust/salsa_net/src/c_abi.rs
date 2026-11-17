#![allow(unsafe_code)]
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_int, c_void};

pub const LWASAPI_MAX_SESSIONS: usize = 32;
pub const LWASAPI_EQ_BANDS: usize = 10;
pub const LWASAPI_MAX_DEVICES: usize = 32;
pub const LWASAPI_DEBUG_MAX_PA_CLIENTS: usize = 32;
pub const LWASAPI_DEBUG_MAX_PA_STREAMS: usize = 48;
pub const LWASAPI_DEBUG_MAX_PW_CLIENTS: usize = 32;
pub const LWASAPI_DEBUG_MAX_PW_NODES: usize = 32;
pub const LWASAPI_DEBUG_MAX_PW_LINKS: usize = 32;

pub const LWASAPI_DBG_PA: u8 = 2;
pub const LWASAPI_DBG_PW_VIDEO: u8 = 7;

pub const LWASAPI_DBG_TRACE: u8 = 0;
pub const LWASAPI_DBG_INFO: u8 = 1;
pub const LWASAPI_DBG_WARN: u8 = 2;

pub const LWASAPI_PW_PREVIEW_MAGIC: u32 = 0x534C_5057;
pub const LWASAPI_PW_PREVIEW_VERSION: u32 = 2;
pub const LWASAPI_PW_PREVIEW_MAX_W: u32 = 3840;
pub const LWASAPI_PW_PREVIEW_MAX_H: u32 = 2160;
pub const LWASAPI_PW_PREVIEW_MAX_BYTES: usize =
    (LWASAPI_PW_PREVIEW_MAX_W as usize) * (LWASAPI_PW_PREVIEW_MAX_H as usize) * 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiEqConfig {
    pub enabled: u32,
    pub dynamic_mode: u32,
    pub preamp_db: f32,
    pub dyn_threshold_db: f32,
    pub dyn_ratio: f32,
    pub limiter_enabled: u32,
    pub band_gains_db: [f32; LWASAPI_EQ_BANDS],
    pub band_q: [f32; LWASAPI_EQ_BANDS],
    pub preset_name: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiParamEqExt {
    pub band_freq_hz: [f32; LWASAPI_EQ_BANDS],
    pub band_type: [u32; LWASAPI_EQ_BANDS],
    pub crossfeed_enabled: u32,
    pub crossfeed_level: f32,
    pub crossfeed_cutoff_hz: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiMicApoCfg {
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
pub struct LwasapiBtCodecCfg {
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
pub struct LwasapiHwAnalogCfg {
    pub hw_dac_unity_vol: u32,
    pub hp_amp_tier: u32,
    pub output_route: u32,
    pub unlock_0db_rails: u32,
    pub analog_boost_db: f32,
    pub mic_hw_boost_db: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiHwAnalogInfo {
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
pub struct LwasapiSessionInfo {
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
pub struct LwasapiDeviceEntry {
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
pub struct LwasapiDeviceList {
    pub num_devices: u32,
    pub devices: [LwasapiDeviceEntry; LWASAPI_MAX_DEVICES],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiEngineStatus {
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
    pub eq_cfg: LwasapiEqConfig,
    pub eq_band_levels_db: [f32; LWASAPI_EQ_BANDS],
    pub eq_dyn_delta_db: [f32; LWASAPI_EQ_BANDS],
    pub output_peak_l: f32,
    pub output_peak_r: f32,
    pub num_sessions: u32,
    pub sessions: [LwasapiSessionInfo; LWASAPI_MAX_SESSIONS],
    pub bt_active: u32,
    pub bt_delay_ms: u32,
    pub bt_alias: [u8; 64],
    pub bt_codec: [u8; 16],
    pub pa_server_running: u32,
    pub pa_clients: u32,
    pub active_playback_id: [u8; 64],
    pub active_capture_id: [u8; 64],
    pub param_eq: LwasapiParamEqExt,
    pub mic_apo: LwasapiMicApoCfg,
    pub bt_codec_cfg: LwasapiBtCodecCfg,
    pub mic_apo_peak_in: f32,
    pub mic_apo_peak_out: f32,
    pub mic_rnnoise_active: u32,
    pub hw_analog_cfg: LwasapiHwAnalogCfg,
    pub hw_analog_info: LwasapiHwAnalogInfo,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiPaCallbacks {
    pub user_ctx: *mut c_void,
    pub get_status: Option<unsafe extern "C" fn(*mut c_void, *mut LwasapiEngineStatus)>,
    pub get_devices: Option<unsafe extern "C" fn(*mut c_void, *mut LwasapiDeviceList)>,
    pub set_devices: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int>,
    pub set_master_volume: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int>,
    pub set_mic_gain_ppm: Option<unsafe extern "C" fn(*mut c_void, u32, c_int, c_int) -> c_int>,
}

unsafe impl Send for LwasapiPaCallbacks {}
unsafe impl Sync for LwasapiPaCallbacks {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiDbgPaClient {
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
pub struct LwasapiDbgPaStream {
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
pub struct LwasapiDbgPaSnapshot {
    pub num_clients: u32,
    pub clients: [LwasapiDbgPaClient; LWASAPI_DEBUG_MAX_PA_CLIENTS],
    pub num_streams: u32,
    pub streams: [LwasapiDbgPaStream; LWASAPI_DEBUG_MAX_PA_STREAMS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiDbgPwClient {
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
pub struct LwasapiDbgPwNode {
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
pub struct LwasapiDbgPwLink {
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
pub struct LwasapiDbgPwSnapshot {
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
    pub clients: [LwasapiDbgPwClient; LWASAPI_DEBUG_MAX_PW_CLIENTS],
    pub num_nodes: u32,
    pub nodes: [LwasapiDbgPwNode; LWASAPI_DEBUG_MAX_PW_NODES],
    pub num_links: u32,
    pub links: [LwasapiDbgPwLink; LWASAPI_DEBUG_MAX_PW_LINKS],
}

#[repr(C)]
pub struct LwasapiPwPreviewShm {
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
    pub pixels: [u8; LWASAPI_PW_PREVIEW_MAX_BYTES],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LwasapiPwBrokerOpts {
    pub runtime_dir: *const c_char,
    pub socket_name: *const c_char,
    pub ignore_zero_flag: bool,
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
        assert_eq!(size_of::<LwasapiEngineStatus>(), 7136);
        assert_eq!(size_of::<LwasapiDeviceList>(), 6020);
        assert_eq!(size_of::<LwasapiSessionInfo>(), 184);
        assert_eq!(size_of::<LwasapiDbgPaClient>(), 152);
        assert_eq!(size_of::<LwasapiDbgPaStream>(), 232);
        assert_eq!(size_of::<LwasapiDbgPaSnapshot>(), 16008);
        assert_eq!(size_of::<LwasapiDbgPwClient>(), 208);
        assert_eq!(size_of::<LwasapiDbgPwNode>(), 216);
        assert_eq!(size_of::<LwasapiDbgPwLink>(), 72);
        assert_eq!(size_of::<LwasapiDbgPwSnapshot>(), 16272);
        assert_eq!(offset_of!(LwasapiPwPreviewShm, pixels), 520);
        assert_eq!(size_of::<LwasapiPwPreviewShm>(), 33_178_120);
    }
}
