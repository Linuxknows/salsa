// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::{c_char, c_int};
use core::mem;
use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use salsa_wire::control_ipc::{AudioDir, ControlClient, ControlRequest, MuteOp};
use salsa_wire::state_conf::{
    load_unified_settings_json_in, save_unified_settings_json_in,
    update_unified_settings_json_in, SALSA_SETTINGS_FILENAME,
};

use crate::host::state_dir;
use crate::json::JsonValue;

pub use salsa_wire::c_abi::{
    LasapiBtCodecCfg, LasapiDeviceEntry, LasapiDeviceList, LasapiEngineStatus, LasapiEqConfig,
    LasapiHwAnalogCfg, LasapiHwAnalogInfo, LasapiIpcReq, LasapiIpcResp, LasapiMicApoCfg,
    LasapiParamEqExt, LasapiSessionInfo, LASAPI_BT_CAP_AAC, LASAPI_BT_CAP_APTX,
    LASAPI_BT_CAP_APTX_HD, LASAPI_BT_CAP_LDAC, LASAPI_BT_CAP_SBC, LASAPI_BT_PREF_AAC,
    LASAPI_BT_PREF_APTX, LASAPI_BT_PREF_APTX_HD, LASAPI_BT_PREF_AUTO, LASAPI_BT_PREF_LDAC,
    LASAPI_BT_PREF_SBC, LASAPI_CFG_FLAG_CAPTURE_RATE, LASAPI_CMD_SET_VOLUME, LASAPI_EQ_BANDS,
    LASAPI_FILTER_HIGH_SHELF, LASAPI_FILTER_LOW_SHELF, LASAPI_FILTER_PEAK, LASAPI_MAX_DEVICES,
    LASAPI_MAX_SESSIONS,
};

pub const LASAPI_LDAC_MODE_ABR: u32 = 0;
pub const LASAPI_LDAC_MODE_HQ_990: u32 = 1;
pub const LASAPI_LDAC_MODE_SQ_660: u32 = 2;
pub const LASAPI_LDAC_MODE_MQ_330: u32 = 3;

pub const LASAPI_HW_AMP_AUTO: u32 = 0;
pub const LASAPI_HW_AMP_LOW: u32 = 1;
pub const LASAPI_HW_AMP_MED: u32 = 2;
pub const LASAPI_HW_AMP_HIGH: u32 = 3;

pub const LASAPI_HW_ROUTE_AUTO: u32 = 0;
pub const LASAPI_HW_ROUTE_HP: u32 = 1;
pub const LASAPI_HW_ROUTE_SPK: u32 = 2;

pub const LASAPI_HW_CAP_DAC_VOL: u32 = 1 << 0;
pub const LASAPI_HW_CAP_HP_AMP: u32 = 1 << 1;
pub const LASAPI_HW_CAP_OUT_ROUTE: u32 = 1 << 2;
pub const LASAPI_HW_CAP_MIC_BOOST: u32 = 1 << 3;
pub const LASAPI_HW_CAP_LINE_AIR: u32 = 1 << 4;
pub const LASAPI_HW_CAP_HID_BOX: u32 = 1 << 5;
pub const LASAPI_HW_CAP_STRIX_ARCH: u32 = 1 << 6;
pub const LASAPI_HW_CAP_HIGH_RES: u32 = 1 << 7;

pub const SUPPORTED_SAMPLE_RATES: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];
pub const DEFAULT_VOL_STEP_PCT: u32 = 5;
pub const SUPPORTED_VOL_STEPS: [u32; 4] = [1, 2, 5, 10];

pub use salsa_wire::settings::{
    sanitize_device_eq_profiles_map, EqPreset, MicApoPreset, DEFAULT_BAND_FREQS_HZ, EQ_PRESETS,
    MIC_APO_PRESETS,
};

pub fn normalize_vol_step_pct(val: &str) -> u32 {
    let cleaned = val.trim().trim_end_matches('%');
    if let Ok(v) = cleaned.parse::<u32>() {
        if SUPPORTED_VOL_STEPS.contains(&v) {
            return v;
        }
    }
    DEFAULT_VOL_STEP_PCT
}

pub fn get_engine_status() -> Option<LasapiEngineStatus> {
    let mut st: LasapiEngineStatus = unsafe { mem::zeroed() };
    let hr = unsafe { lasapi::LASAPI_GetEngineStatus(&raw mut st) };
    (hr == 0).then_some(st)
}

pub fn set_engine_config(sample_rate: u32, period_frames: u32, format_bits: u32) -> i32 {
    lasapi::LASAPI_SetEngineConfig(sample_rate, period_frames, format_bits)
}

pub fn set_capture_sample_rate(sample_rate: u32) -> i32 {
    lasapi::LASAPI_SetCaptureSampleRate(sample_rate)
}

pub fn set_dynamic_eq(eq: &LasapiEqConfig) -> i32 {
    unsafe { lasapi::LASAPI_SetDynamicEQ(eq) }
}

pub fn set_parametric_eq(eq: &LasapiEqConfig, ext: &LasapiParamEqExt) -> i32 {
    unsafe { lasapi::LASAPI_SetParametricEQ(eq, ext) }
}

pub fn set_mic_apo(cfg: &LasapiMicApoCfg) -> i32 {
    unsafe { lasapi::LASAPI_SetMicAPO(cfg) }
}

pub fn set_bt_codec_config(cfg: &LasapiBtCodecCfg) -> i32 {
    unsafe { lasapi::LASAPI_SetBtCodecConfig(cfg) }
}

pub fn set_hw_analog(cfg: &LasapiHwAnalogCfg) -> i32 {
    unsafe { lasapi::LASAPI_SetHwAnalog(cfg) }
}

pub fn save_config_now() -> i32 {
    unsafe { lasapi::LASAPI_SaveConfig() }
}

pub fn reload_config_now() -> i32 {
    unsafe { lasapi::LASAPI_ReloadConfig() }
}

pub fn bt_connection_ctl(action: u32, mac: Option<&str>) -> (i32, u32) {
    let c_mac = mac.and_then(|s| CString::new(s).ok());
    let mac_ptr = c_mac.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    let mut flags = 0u32;
    let hr = unsafe { lasapi::LASAPI_BtConnectionCtl(action, mac_ptr, &raw mut flags) };
    (hr, flags)
}

pub fn list_devices() -> Option<LasapiDeviceList> {
    let mut list: LasapiDeviceList = unsafe { mem::zeroed() };
    let hr = unsafe { lasapi::LASAPI_ListDevices(&raw mut list) };
    (hr == 0).then_some(list)
}

pub fn set_devices(playback_id: Option<&str>, capture_id: Option<&str>) -> i32 {
    let c_pb = playback_id.and_then(|s| CString::new(s).ok());
    let c_cap = capture_id.and_then(|s| CString::new(s).ok());
    let pb_ptr = c_pb.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    let cap_ptr = c_cap.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    unsafe { lasapi::LASAPI_SetDevices(pb_ptr, cap_ptr) }
}

pub fn set_session_control(
    session_id: u32,
    volume_pct: c_int,
    muted: c_int,
    target_device: Option<&str>,
) -> i32 {
    let c_dev = target_device.and_then(|s| CString::new(s).ok());
    let dev_ptr: *const c_char = c_dev.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    unsafe { lasapi::LASAPI_SetSessionControl(session_id, volume_pct, muted, dev_ptr) }
}

static PROC_COMM_CACHE: OnceLock<String> = OnceLock::new();

fn cached_proc_comm() -> &'static str {
    PROC_COMM_CACHE
        .get_or_init(|| {
            fs::read_to_string("/proc/self/comm")
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        })
        .as_str()
}

fn send_volume_cmd(dir: AudioDir, pct: i32, mute_mode: i32) -> bool {
    let req = ControlRequest::SetMasterVolume {
        dir,
        volume_pct: (pct >= 0).then(|| pct.clamp(0, 150) as u32),
        mute: MuteOp::from_ui_mode(mute_mode),
        client_tag: cached_proc_comm().to_string(),
    };
    ControlClient::send_fast(&req).is_ok_and(|resp| resp.hr == 0)
}

pub fn set_master_volume(pct: i32, mute_mode: i32) -> bool {
    send_volume_cmd(AudioDir::Playback, pct, mute_mode)
}

pub fn set_mic_volume(pct: i32, mute_mode: i32) -> bool {
    send_volume_cmd(AudioDir::Capture, pct, mute_mode)
}

pub fn config_dir() -> PathBuf {
    state_dir()
}

pub fn salsa_state_path() -> PathBuf {
    config_dir().join("salsa_state.bin")
}

pub fn gui_settings_path() -> PathBuf {
    config_dir().join(SALSA_SETTINGS_FILENAME)
}

fn sanitize_eq_device_profiles(val: &JsonValue) -> JsonValue {
    sanitize_device_eq_profiles_map(val)
}

pub fn load_gui_settings() -> JsonValue {
    let mut obj = load_unified_settings_json_in(&config_dir());
    if let Some(prof) = obj.get("per_device_eq").cloned() {
        obj.set("per_device_eq", sanitize_eq_device_profiles(&prof));
    } else if let Some(prof) = obj.get("eq_device_profiles").cloned() {
        let clean = sanitize_eq_device_profiles(&prof);
        obj.set("per_device_eq", clean.clone());
        obj.set("eq_device_profiles", clean);
    }
    if let Some(prof) = obj.get("eq_device_profiles").cloned() {
        obj.set("eq_device_profiles", sanitize_eq_device_profiles(&prof));
    }
    obj
}

pub fn save_studio_gui_settings(data: &JsonValue) -> Result<(), String> {
    let Some(new_entries) = data.as_object() else {
        return Err("gui settings must be a JSON object".into());
    };
    let mut patched = JsonValue::Object(new_entries.to_vec());
    if let Some(epd) = data
        .get("eq_per_device")
        .and_then(JsonValue::as_bool)
        .or_else(|| data.get("eq_per_device_enabled").and_then(JsonValue::as_bool))
    {
        patched.set("eq_per_device", JsonValue::Bool(epd));
        patched.set("eq_per_device_enabled", JsonValue::Bool(epd));
    }
    if let Some(prof) = patched.get("per_device_eq").cloned() {
        patched.set("per_device_eq", sanitize_eq_device_profiles(&prof));
    } else if let Some(prof) = patched.get("eq_device_profiles").cloned() {
        let clean = sanitize_eq_device_profiles(&prof);
        patched.set("per_device_eq", clean.clone());
        patched.set("eq_device_profiles", clean);
    }
    if let Some(prof) = patched.get("eq_device_profiles").cloned() {
        patched.set("eq_device_profiles", sanitize_eq_device_profiles(&prof));
    }
    save_unified_settings_json_in(&config_dir(), &patched, true)
}

pub fn save_gui_settings(data: &JsonValue) -> Result<(), String> {
    save_unified_settings_json_in(&config_dir(), data, false)
}

pub fn update_gui_settings(updates: &[(&str, JsonValue)]) -> Result<(), String> {
    update_unified_settings_json_in(&config_dir(), updates)
}

pub fn load_saved_theme_name() -> String {
    let s = load_gui_settings();
    s.get("theme")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "Salsa Midnight (Default)".to_string())
}

pub fn active_theme_name_for_tray() -> String {
    let s = load_gui_settings();
    if let Some(tt) = s.get("tray_theme").and_then(|v| v.as_str()) {
        if !tt.is_empty() && tt != "Follow Studio Theme" {
            return tt.to_string();
        }
    }
    s.get("theme")
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Salsa Midnight (Default)".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCastPrefs {
    pub res_key: String,
    pub fps_val: u32,
    pub cap_real_cast: bool,
}

impl Default for ScreenCastPrefs {
    fn default() -> Self {
        Self {
            res_key: "Source".to_string(),
            fps_val: 0,
            cap_real_cast: false,
        }
    }
}

const VALID_SC_RES: [&str; 6] = ["Source", "2160p", "1440p", "1080p", "720p", "480p"];

fn is_valid_sc_fps(fps: u32) -> bool {
    fps == 0 || (15..=240).contains(&fps)
}

pub fn load_screencast_prefs() -> ScreenCastPrefs {
    let mut out = ScreenCastPrefs::default();
    let data = load_gui_settings();
    if let Some(sc) = data.get("screencast_profile") {
        if let Some(rk) = sc.get("res_key").and_then(|v| v.as_str()) {
            if VALID_SC_RES.contains(&rk) {
                out.res_key = rk.to_string();
            }
        }
        if let Some(fv) = sc.get("fps_val").and_then(|v| v.as_u64()) {
            let fv32 = fv as u32;
            if is_valid_sc_fps(fv32) {
                out.fps_val = fv32;
            }
        }
        if let Some(crc) = sc.get("cap_real_cast").and_then(|v| v.as_bool()) {
            out.cap_real_cast = crc;
        }
    }
    out
}

pub fn save_screencast_prefs(prefs: &ScreenCastPrefs) -> Result<(), String> {
    let res_key = if VALID_SC_RES.contains(&prefs.res_key.as_str()) {
        prefs.res_key.as_str()
    } else {
        "Source"
    };
    let fps_val = if is_valid_sc_fps(prefs.fps_val) {
        prefs.fps_val
    } else {
        0
    };
    let profile = JsonValue::obj(vec![
        ("res_key", JsonValue::Str(res_key.to_string())),
        ("fps_val", JsonValue::Int(i64::from(fps_val))),
        ("cap_real_cast", JsonValue::Bool(prefs.cap_real_cast)),
    ]);
    update_gui_settings(&[("screencast_profile", profile)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn abi_struct_sizes_and_offsets_match_c_and_python() {
        assert_eq!(size_of::<LasapiEqConfig>(), 136);
        assert_eq!(size_of::<LasapiDeviceEntry>(), 188);
        assert_eq!(size_of::<LasapiDeviceList>(), 6020);
        assert_eq!(size_of::<LasapiSessionInfo>(), 184);
        assert_eq!(size_of::<LasapiParamEqExt>(), 96);
        assert_eq!(size_of::<LasapiMicApoCfg>(), 48);
        assert_eq!(size_of::<LasapiBtCodecCfg>(), 100);
        assert_eq!(size_of::<LasapiHwAnalogCfg>(), 24);
        assert_eq!(size_of::<LasapiHwAnalogInfo>(), 248);
        assert_eq!(size_of::<LasapiEngineStatus>(), 7136);
        assert_eq!(offset_of!(LasapiEngineStatus, output_peak_l), 480);
    }

    #[test]
    fn vol_step_normalization() {
        assert_eq!(normalize_vol_step_pct("10%"), 10);
        assert_eq!(normalize_vol_step_pct("2"), 2);
        assert_eq!(normalize_vol_step_pct("7"), DEFAULT_VOL_STEP_PCT);
        assert_eq!(normalize_vol_step_pct("bad"), DEFAULT_VOL_STEP_PCT);
    }

    #[test]
    fn settings_0700_0600_and_symlink_safety() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pid = std::process::id();
        let tmp = std::env::temp_dir().join(format!("salsa_ipc_cfg_test_{pid}"));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &tmp);

        let obj = JsonValue::obj(vec![
            ("eq_per_device", JsonValue::Bool(true)),
            (
                "eq_device_profiles",
                JsonValue::obj(vec![("dev0", JsonValue::obj(vec![("enabled", JsonValue::Bool(true))]))]),
            ),
        ]);
        save_gui_settings(&obj).expect("save settings");
        let dmode = fs::metadata(config_dir()).unwrap().permissions().mode() & 0o777;
        let fmode = fs::metadata(gui_settings_path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
        assert_eq!(fmode, 0o600);

        let loaded = load_gui_settings();
        assert_eq!(loaded.get("eq_per_device").and_then(|v| v.as_bool()), Some(true));

        let p = gui_settings_path();
        fs::remove_file(&p).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", &p).unwrap();
        let sym_loaded = load_gui_settings();
        assert_eq!(sym_loaded.as_object().map(|o| o.len()), Some(0));

        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = fs::remove_dir_all(&tmp);
    }
}
