// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::{c_char, c_int};
use core::mem;
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use salsa_wire::unix_sys::audiodg_sock_path;

use crate::host::{ensure_dir_0700, read_text_lossy, state_dir, write_atomic_secure};
use crate::json::JsonValue;

pub use salsa_wire::c_abi::{
    LwasapiBtCodecCfg, LwasapiDeviceEntry, LwasapiDeviceList, LwasapiEngineStatus, LwasapiEqConfig,
    LwasapiHwAnalogCfg, LwasapiHwAnalogInfo, LwasapiIpcReq, LwasapiIpcResp, LwasapiMicApoCfg,
    LwasapiParamEqExt, LwasapiSessionInfo, LWASAPI_BT_CAP_AAC, LWASAPI_BT_CAP_APTX,
    LWASAPI_BT_CAP_APTX_HD, LWASAPI_BT_CAP_LDAC, LWASAPI_BT_CAP_SBC, LWASAPI_BT_PREF_AAC,
    LWASAPI_BT_PREF_APTX, LWASAPI_BT_PREF_APTX_HD, LWASAPI_BT_PREF_AUTO, LWASAPI_BT_PREF_LDAC,
    LWASAPI_BT_PREF_SBC, LWASAPI_CFG_FLAG_CAPTURE_RATE, LWASAPI_CMD_SET_VOLUME, LWASAPI_EQ_BANDS,
    LWASAPI_FILTER_HIGH_SHELF, LWASAPI_FILTER_LOW_SHELF, LWASAPI_FILTER_PEAK, LWASAPI_MAX_DEVICES,
    LWASAPI_MAX_SESSIONS,
};

pub const LWASAPI_LDAC_MODE_ABR: u32 = 0;
pub const LWASAPI_LDAC_MODE_HQ_990: u32 = 1;
pub const LWASAPI_LDAC_MODE_SQ_660: u32 = 2;
pub const LWASAPI_LDAC_MODE_MQ_330: u32 = 3;

pub const LWASAPI_HW_AMP_AUTO: u32 = 0;
pub const LWASAPI_HW_AMP_LOW: u32 = 1;
pub const LWASAPI_HW_AMP_MED: u32 = 2;
pub const LWASAPI_HW_AMP_HIGH: u32 = 3;

pub const LWASAPI_HW_ROUTE_AUTO: u32 = 0;
pub const LWASAPI_HW_ROUTE_HP: u32 = 1;
pub const LWASAPI_HW_ROUTE_SPK: u32 = 2;

pub const LWASAPI_HW_CAP_DAC_VOL: u32 = 1 << 0;
pub const LWASAPI_HW_CAP_HP_AMP: u32 = 1 << 1;
pub const LWASAPI_HW_CAP_OUT_ROUTE: u32 = 1 << 2;
pub const LWASAPI_HW_CAP_MIC_BOOST: u32 = 1 << 3;
pub const LWASAPI_HW_CAP_LINE_AIR: u32 = 1 << 4;
pub const LWASAPI_HW_CAP_HID_BOX: u32 = 1 << 5;
pub const LWASAPI_HW_CAP_STRIX_ARCH: u32 = 1 << 6;
pub const LWASAPI_HW_CAP_HIGH_RES: u32 = 1 << 7;

pub const DEFAULT_BAND_FREQS_HZ: [f32; 10] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
pub const SUPPORTED_SAMPLE_RATES: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];
pub const DEFAULT_VOL_STEP_PCT: u32 = 5;
pub const SUPPORTED_VOL_STEPS: [u32; 4] = [1, 2, 5, 10];

pub struct EqPreset {
    pub name: &'static str,
    pub mode: u32,
    pub preamp: f32,
    pub thresh: f32,
    pub ratio: f32,
    pub gains: [f32; 10],
}

pub const EQ_PRESETS: &[EqPreset] = &[
    EqPreset {
        name: "Flat / Studio Reference",
        mode: 0,
        preamp: 0.0,
        thresh: -18.0,
        ratio: 2.0,
        gains: [0.0; 10],
    },
    EqPreset {
        name: "Bose QC Ultra 2 Audiophile",
        mode: 0,
        preamp: -1.5,
        thresh: -16.0,
        ratio: 2.0,
        gains: [1.0, 0.5, -1.5, -0.5, 0.5, 0.5, 0.5, 0.0, -1.5, 0.5],
    },
    EqPreset {
        name: "Dynamic Bass & Sub Control",
        mode: 1,
        preamp: -3.0,
        thresh: -16.0,
        ratio: 3.5,
        gains: [6.5, 5.5, 2.5, 0.0, -1.0, 0.0, 1.0, 2.0, 2.5, 2.0],
    },
    EqPreset {
        name: "192kHz Hi-Res Mastering",
        mode: 2,
        preamp: -1.0,
        thresh: -20.0,
        ratio: 2.2,
        gains: [1.5, 1.0, 0.0, -0.5, 0.0, 0.5, 1.5, 2.5, 3.5, 4.5],
    },
    EqPreset {
        name: "FPS Gaming Footstep Radar",
        mode: 1,
        preamp: -2.0,
        thresh: -22.0,
        ratio: 4.0,
        gains: [-4.0, -2.0, 1.0, 2.5, 3.0, 4.5, 6.0, 5.5, 3.0, 1.0],
    },
    EqPreset {
        name: "Vocal & Podcast Clarity",
        mode: 1,
        preamp: -1.5,
        thresh: -18.0,
        ratio: 3.0,
        gains: [-5.0, -3.0, -1.0, 1.5, 3.5, 4.5, 4.0, 2.5, 1.5, 0.5],
    },
    EqPreset {
        name: "Late-Night Dynamic Loudness",
        mode: 1,
        preamp: -2.5,
        thresh: -24.0,
        ratio: 4.5,
        gains: [4.5, 3.5, 1.0, 0.0, 0.5, 1.5, 2.5, 3.0, 3.5, 3.0],
    },
];

pub struct MicApoPreset {
    pub name: &'static str,
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
}

pub const MIC_APO_PRESETS: &[MicApoPreset] = &[
    MicApoPreset {
        name: "Discord Voice (48k + RNNoise)",
        enabled: 1,
        hpf_enabled: 1,
        hpf_hz: 65.0,
        gate_enabled: 1,
        gate_thresh_db: -50.0,
        rnnoise_enabled: 1,
        comp_enabled: 1,
        comp_thresh_db: -14.0,
        comp_ratio: 2.0,
        clarity_db: 1.5,
    },
    MicApoPreset {
        name: "Studio Broadcast (Warm AI)",
        enabled: 1,
        hpf_enabled: 1,
        hpf_hz: 75.0,
        gate_enabled: 1,
        gate_thresh_db: -45.0,
        rnnoise_enabled: 1,
        comp_enabled: 1,
        comp_thresh_db: -15.0,
        comp_ratio: 2.2,
        clarity_db: 2.0,
    },
    MicApoPreset {
        name: "Clean Vocal (No RNNoise)",
        enabled: 1,
        hpf_enabled: 1,
        hpf_hz: 65.0,
        gate_enabled: 1,
        gate_thresh_db: -52.0,
        rnnoise_enabled: 0,
        comp_enabled: 1,
        comp_thresh_db: -16.0,
        comp_ratio: 1.8,
        clarity_db: 1.0,
    },
    MicApoPreset {
        name: "Bypass (Raw Mic)",
        enabled: 0,
        hpf_enabled: 0,
        hpf_hz: 65.0,
        gate_enabled: 0,
        gate_thresh_db: -50.0,
        rnnoise_enabled: 0,
        comp_enabled: 0,
        comp_thresh_db: -14.0,
        comp_ratio: 2.0,
        clarity_db: 0.0,
    },
];

pub fn normalize_vol_step_pct(val: &str) -> u32 {
    let cleaned = val.trim().trim_end_matches('%');
    if let Ok(v) = cleaned.parse::<u32>() {
        if SUPPORTED_VOL_STEPS.contains(&v) {
            return v;
        }
    }
    DEFAULT_VOL_STEP_PCT
}

pub fn get_engine_status() -> Option<LwasapiEngineStatus> {
    let mut st: LwasapiEngineStatus = unsafe { mem::zeroed() };
    let hr = unsafe { lwasapi::LWASAPI_GetEngineStatus(&raw mut st) };
    (hr == 0).then_some(st)
}

pub fn set_engine_config(sample_rate: u32, period_frames: u32, format_bits: u32) -> i32 {
    lwasapi::LWASAPI_SetEngineConfig(sample_rate, period_frames, format_bits)
}

pub fn set_capture_sample_rate(sample_rate: u32) -> i32 {
    lwasapi::LWASAPI_SetCaptureSampleRate(sample_rate)
}

pub fn set_dynamic_eq(eq: &LwasapiEqConfig) -> i32 {
    unsafe { lwasapi::LWASAPI_SetDynamicEQ(eq) }
}

pub fn set_parametric_eq(eq: &LwasapiEqConfig, ext: &LwasapiParamEqExt) -> i32 {
    unsafe { lwasapi::LWASAPI_SetParametricEQ(eq, ext) }
}

pub fn set_mic_apo(cfg: &LwasapiMicApoCfg) -> i32 {
    unsafe { lwasapi::LWASAPI_SetMicAPO(cfg) }
}

pub fn set_bt_codec_config(cfg: &LwasapiBtCodecCfg) -> i32 {
    unsafe { lwasapi::LWASAPI_SetBtCodecConfig(cfg) }
}

pub fn set_hw_analog(cfg: &LwasapiHwAnalogCfg) -> i32 {
    unsafe { lwasapi::LWASAPI_SetHwAnalog(cfg) }
}

pub fn bt_connection_ctl(action: u32, mac: Option<&str>) -> (i32, u32) {
    let c_mac = mac.and_then(|s| CString::new(s).ok());
    let mac_ptr = c_mac.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    let mut flags = 0u32;
    let hr = unsafe { lwasapi::LWASAPI_BtConnectionCtl(action, mac_ptr, &raw mut flags) };
    (hr, flags)
}

pub fn list_devices() -> Option<LwasapiDeviceList> {
    let mut list: LwasapiDeviceList = unsafe { mem::zeroed() };
    let hr = unsafe { lwasapi::LWASAPI_ListDevices(&raw mut list) };
    (hr == 0).then_some(list)
}

pub fn set_devices(playback_id: Option<&str>, capture_id: Option<&str>) -> i32 {
    let c_pb = playback_id.and_then(|s| CString::new(s).ok());
    let c_cap = capture_id.and_then(|s| CString::new(s).ok());
    let pb_ptr = c_pb.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    let cap_ptr = c_cap.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    unsafe { lwasapi::LWASAPI_SetDevices(pb_ptr, cap_ptr) }
}

pub fn set_session_control(
    session_id: u32,
    volume_pct: c_int,
    muted: c_int,
    target_device: Option<&str>,
) -> i32 {
    let c_dev = target_device.and_then(|s| CString::new(s).ok());
    let dev_ptr: *const c_char = c_dev.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    unsafe { lwasapi::LWASAPI_SetSessionControl(session_id, volume_pct, muted, dev_ptr) }
}

fn send_volume_cmd(target_id: u32, pct: i32, mute_mode: i32) -> bool {
    let mut req: LwasapiIpcReq = unsafe { mem::zeroed() };
    req.cmd = LWASAPI_CMD_SET_VOLUME;
    req.client_pid = std::process::id();
    if let Ok(comm) = fs::read_to_string("/proc/self/comm") {
        let trimmed = comm.trim();
        let bytes = trimmed.as_bytes();
        let n = bytes.len().min(req.client_name.len().saturating_sub(1));
        req.client_name[..n].copy_from_slice(&bytes[..n]);
    }
    req.share_mode = (mute_mode + 1).clamp(0, 3) as u32;
    req.target_session_id = target_id;
    req.volume_milli = if pct >= 0 {
        (pct.clamp(0, 150) * 10) as u32
    } else {
        u32::MAX
    };

    let sock_path = audiodg_sock_path();
    let Ok(mut stream) = UnixStream::connect(&sock_path) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let req_bytes = unsafe {
        core::slice::from_raw_parts(
            (&req as *const LwasapiIpcReq).cast::<u8>(),
            mem::size_of::<LwasapiIpcReq>(),
        )
    };
    if stream.write_all(req_bytes).is_err() {
        return false;
    }
    let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
    let resp_bytes = unsafe {
        core::slice::from_raw_parts_mut(
            (&mut resp as *mut LwasapiIpcResp).cast::<u8>(),
            mem::size_of::<LwasapiIpcResp>(),
        )
    };
    match stream.read_exact(resp_bytes) {
        Ok(()) => resp.hr == 0,
        Err(_) => false,
    }
}

pub fn set_master_volume(pct: i32, mute_mode: i32) -> bool {
    send_volume_cmd(0xFFFF_FFFF, pct, mute_mode)
}

pub fn set_mic_volume(pct: i32, mute_mode: i32) -> bool {
    send_volume_cmd(0xFFFF_FFFE, pct, mute_mode)
}

pub fn config_dir() -> PathBuf {
    state_dir()
}

pub fn salsa_state_path() -> PathBuf {
    config_dir().join("salsa_state.bin")
}

pub fn gui_settings_path() -> PathBuf {
    config_dir().join("gui_settings.json")
}

fn sanitize_eq_device_profiles(val: &JsonValue) -> JsonValue {
    let Some(entries) = val.as_object() else {
        return JsonValue::Object(Vec::new());
    };
    let mut clean = Vec::new();
    for (k, v) in entries {
        let trimmed = k.trim();
        if !trimmed.is_empty() && trimmed.len() <= 64 && matches!(v, JsonValue::Object(_)) {
            clean.push((trimmed.to_string(), v.clone()));
        }
    }
    JsonValue::Object(clean)
}

pub fn load_gui_settings() -> JsonValue {
    let p = gui_settings_path();
    let Ok(meta) = fs::symlink_metadata(&p) else {
        return JsonValue::Object(Vec::new());
    };
    if meta.file_type().is_symlink() {
        return JsonValue::Object(Vec::new());
    }
    let raw = read_text_lossy(&p, 262_144);
    match JsonValue::parse(&raw) {
        Ok(mut obj @ JsonValue::Object(_)) => {
            if let Some(prof) = obj.get("eq_device_profiles").cloned() {
                obj.set("eq_device_profiles", sanitize_eq_device_profiles(&prof));
            }
            obj
        }
        _ => JsonValue::Object(Vec::new()),
    }
}

pub fn save_studio_gui_settings(data: &JsonValue) -> Result<(), String> {
    let Some(new_entries) = data.as_object() else {
        return Err("gui settings must be a JSON object".into());
    };
    let p = gui_settings_path();
    let mut merged = match fs::symlink_metadata(&p) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(format!("refusing symlinked settings file: {}", p.display()));
        }
        Ok(_) => {
            let raw = read_text_lossy(&p, 262_144);
            match JsonValue::parse(&raw) {
                Ok(obj @ JsonValue::Object(_)) => obj,
                _ => JsonValue::Object(Vec::new()),
            }
        }
        Err(_) => JsonValue::Object(Vec::new()),
    };

    for (k, v) in new_entries {
        if !k.starts_with("tray_") || merged.get(k).is_none() {
            merged.set(k.as_str(), v.clone());
        }
    }
    if let Some(epd) = data
        .get("eq_per_device")
        .and_then(JsonValue::as_bool)
        .or_else(|| data.get("eq_per_device_enabled").and_then(JsonValue::as_bool))
    {
        merged.set("eq_per_device", JsonValue::Bool(epd));
        merged.set("eq_per_device_enabled", JsonValue::Bool(epd));
    }
    if let Some(prof) = merged.get("eq_device_profiles").cloned() {
        merged.set("eq_device_profiles", sanitize_eq_device_profiles(&prof));
    }
    save_gui_settings(&merged)
}

pub fn save_gui_settings(data: &JsonValue) -> Result<(), String> {
    let JsonValue::Object(_) = data else {
        return Err("gui settings must be a JSON object".into());
    };
    let dir = config_dir();
    ensure_dir_0700(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let p = gui_settings_path();
    if let Ok(meta) = fs::symlink_metadata(&p) {
        if meta.file_type().is_symlink() {
            return Err(format!("refusing symlink at {}", p.display()));
        }
    }
    let mut text = data.to_json_pretty(2);
    text.push('\n');
    write_atomic_secure(&p, text.as_bytes(), 0o600)
        .map_err(|e| format!("write {}: {e}", p.display()))
}

pub fn update_gui_settings(updates: &[(&str, JsonValue)]) -> Result<(), String> {
    let p = gui_settings_path();
    let mut cur = match fs::symlink_metadata(&p) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(format!("refusing symlinked settings file: {}", p.display()));
        }
        Ok(_) => {
            let raw = fs::read_to_string(&p)
                .map_err(|e| format!("read {}: {e}", p.display()))?;
            let parsed = JsonValue::parse(&raw)
                .map_err(|e| format!("parse {}: {e}", p.display()))?;
            if !matches!(parsed, JsonValue::Object(_)) {
                return Err(format!("{}: expected JSON object", p.display()));
            }
            parsed
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => JsonValue::obj(Vec::new()),
        Err(e) => return Err(format!("stat {}: {e}", p.display())),
    };
    for (k, v) in updates {
        cur.set(*k, v.clone());
    }
    save_gui_settings(&cur)
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
        assert_eq!(size_of::<LwasapiEqConfig>(), 136);
        assert_eq!(size_of::<LwasapiDeviceEntry>(), 188);
        assert_eq!(size_of::<LwasapiDeviceList>(), 6020);
        assert_eq!(size_of::<LwasapiSessionInfo>(), 184);
        assert_eq!(size_of::<LwasapiParamEqExt>(), 96);
        assert_eq!(size_of::<LwasapiMicApoCfg>(), 48);
        assert_eq!(size_of::<LwasapiBtCodecCfg>(), 100);
        assert_eq!(size_of::<LwasapiHwAnalogCfg>(), 24);
        assert_eq!(size_of::<LwasapiHwAnalogInfo>(), 248);
        assert_eq!(size_of::<LwasapiEngineStatus>(), 7136);
        assert_eq!(offset_of!(LwasapiEngineStatus, output_peak_l), 480);
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
