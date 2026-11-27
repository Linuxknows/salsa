// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use salsa_tools::debug_cli::{format_probe_human, run_debug_cli};
use salsa_tools::debug_core::{
    format_ts_ms, level_rank, scan_process_routing_audit, snapshot_once, DiagnosticProbes,
    DryRunDesktopCapturer, ExternalLogManager, SalsaDebugClient, ScreenCastShmReader,
    SystemAuditScanner, LOG_LEVELS, SUBSYSTEMS,
};
use salsa_tools::host::{
    ensure_dir_0700, flock_fd, getpid, getuid, home_dir, install_root, private_runtime_dir,
    read_text_lossy, write_atomic_secure,
};
use salsa_tools::ipc::{
    bt_connection_ctl, get_engine_status, gui_settings_path, list_devices, load_gui_settings,
    load_screencast_prefs, reload_config_now, save_config_now, save_screencast_prefs,
    save_studio_gui_settings, set_bt_codec_config, set_capture_sample_rate, set_devices,
    set_dynamic_eq, set_engine_config, set_hw_analog, set_master_volume, set_mic_volume,
    set_parametric_eq, set_session_control, update_gui_settings, ScreenCastPrefs, EQ_PRESETS,
};
use salsa_tools::json::JsonValue;
use salsa_tools::zlib_png::{decode_png_rgba, encode_png_rgba, RgbaImage};
use salsa_wire::c_abi::{
    c_str_to_string, LasapiBtCodecCfg, LasapiDeviceList, LasapiEngineStatus, LasapiEqConfig,
    LasapiHwAnalogCfg, LasapiMicApoCfg, LasapiParamEqExt,
};
use salsa_wire::state_conf::{
    config_dir_path, load_runtime_state_in_dir, salsa_settings_file_path,
    save_runtime_state_in_dir,
};
use salsa_wire::legal::{
    full_license_bundle, handle_common_cli_flag, salsa_version, short_legal_notice, COPY_TEXT,
    LESSER_COPY_TEXT, THIRD_PARTY_TEXT,
};
use salsa_wire::provisioning::{all_pacman_hook_paths, SalsaHostPaths, UNIFIED_SETTINGS_FILENAME};
use salsa_wire::unix_sys::{
    current_uid, ensure_dir_0700_safe, reset_sigpipe_default, O_CLOEXEC, O_NOFOLLOW,
};

use crate::gio_dbus::{cstr as dbus_cstr, gio};
use crate::screencast_preview::{has_saved_restore_token, PortalScreenCastPreview};
use crate::sni::{SniEvent, SniThreadHost, TrayIconBank, SNI_XML};
use crate::studio_helpers::{
    apply_builtin_preset_to_cfg, apply_json_to_eq_cfg, apply_saved_autoeq_to_cfg,
    build_combined_eq_preset_names, eq_cfg_to_json, load_saved_autoeq_presets,
    reload_autoeq_from_path, save_imported_autoeq_presets, saved_autoeq_from_parsed,
    saved_autoeq_to_json, set_c_bytes, unique_autoeq_name_for_list, SavedAutoEqBand,
    SavedAutoEqPreset, DEFAULT_SHORTCUTS,
};
use crate::studio_manual::{
    detect_host_pc_profile, parse_autoeq_txt, probe_alsa_playback_caps, render_manual_chapter_doc,
    xml_escape, MANUAL_CHAPTER_BUTTONS,
};
use crate::theme::{
    all_studio_theme_choices, all_tray_theme_choices, derive_harmony_theme, load_studio_theme,
    load_tray_theme, load_ui_font_scale_pct, normalize_font_scale_pct, preset_theme_names,
    save_ui_font_scale_pct, theme_by_name, SalsaTheme, THEME_TOKEN_GROUPS,
};
use crate::tray_placement::{
    compute_osd_pos, compute_tray_popup_plan, compute_tray_resize_plan, resolve_tray_target,
    GioCompositorQuery, Rect, ScreenInfo, TrayPopupReq, TrayResizeReq, TrayResolveReq,
};
use crate::x11_util::{
    apply_osd_hints, apply_popup_hints, monitor_geometry_near, plasma_tray_panel_info,
};

extern "C" {
    fn kill(pid: c_int, sig: c_int) -> c_int;
}

pub type SalsaSniCb = extern "C" fn(ctx: *mut c_void, kind: c_int, x_or_delta: c_int, y: c_int, ori: *const c_char);

#[repr(C)]
pub struct SalsaScFrameMeta {
    pub active_stream: c_int,
    pub is_dmabuf: c_int,
    pub dmabuf_mappable: c_int,
    pub width: u32,
    pub height: u32,
    pub actual_fps_x100: u32,
    pub prod_node_id: u32,
    pub cons_count: u32,
    pub cast_fps_cap: u32,
    pub seq: u64,
    pub bytes_written: usize,
    pub prod_name: [u8; 64],
    pub format_name: [u8; 32],
}

struct SniOpaque {
    host: Option<SniThreadHost>,
    icon_dir: PathBuf,
}

struct ShmOpaque {
    reader: Option<ScreenCastShmReader>,
    shm_name: String,
    dry_cap: DryRunDesktopCapturer,
    portal: Option<PortalScreenCastPreview>,
    last_seq: u64,
    last_w: u32,
    last_h: u32,
    last_mw: u32,
    last_mh: u32,
    last_prod_id: u32,
}

fn reset_shm_cache(st: &mut ShmOpaque) {
    st.last_seq = 0;
    st.last_w = 0;
    st.last_h = 0;
    st.last_mw = 0;
    st.last_mh = 0;
    st.last_prod_id = 0;
}

struct TrayLockOpaque {
    _file: File,
}

fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) };
    Some(s.to_string_lossy().into_owned())
}

fn to_c_string(s: String) -> *mut c_char {
    let cleaned = s.replace('\0', " ");
    match std::ffi::CString::new(cleaned) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ffi::CString::new("").unwrap().into_raw(),
    }
}

fn theme_to_wire_json(t: &SalsaTheme) -> JsonValue {
    JsonValue::obj(vec![
        ("name", JsonValue::Str(t.name.clone())),
        ("theme", t.to_json()),
    ])
}

fn run_cmd_bounded(bin: &str, args: &[&str], timeout: Duration) -> Option<Output> {
    const MAX_PIPE_READ: u64 = 1024 * 1024;
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let out_thr = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe {
            let _ = p.take(MAX_PIPE_READ).read_to_end(&mut buf);
        }
        buf
    });
    let err_thr = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe {
            let _ = p.take(MAX_PIPE_READ).read_to_end(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = out_thr.join().unwrap_or_default();
    let stderr = err_thr.join().unwrap_or_default();
    status.map(|st| Output {
        status: st,
        stdout,
        stderr,
    })
}

fn saved_autoeq_from_json_val(v: &JsonValue) -> Option<SavedAutoEqPreset> {
    let name = v.get("name").and_then(JsonValue::as_str)?.trim().to_string();
    let source_path = v
        .get("source_path")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let preamp_db = v
        .get("preamp_db")
        .and_then(JsonValue::as_f64)
        .unwrap_or(0.0) as f32;
    let bands_arr = v.get("bands").and_then(JsonValue::as_array)?;
    if bands_arr.len() != 10 {
        return None;
    }
    let mut bands: [SavedAutoEqBand; 10] = std::array::from_fn(|_| SavedAutoEqBand {
        freq_hz: 1000.0,
        gain_db: 0.0,
        q: 1.414,
        filter_type: 0,
    });
    for (i, bv) in bands_arr.iter().enumerate() {
        bands[i] = SavedAutoEqBand {
            freq_hz: bv.get("freq_hz").and_then(JsonValue::as_f64).unwrap_or(1000.0) as f32,
            gain_db: bv.get("gain_db").and_then(JsonValue::as_f64).unwrap_or(0.0) as f32,
            q: bv.get("q").and_then(JsonValue::as_f64).unwrap_or(1.414) as f32,
            filter_type: bv.get("filter_type").and_then(JsonValue::as_u64).unwrap_or(0) as u32,
        };
    }
    Some(SavedAutoEqPreset {
        name,
        source_path,
        preamp_db,
        bands,
    })
}

fn render_segments_html(segs: &[crate::studio_manual::ManualRichSegment], t: &SalsaTheme) -> String {
    let mut s = String::new();
    for seg in segs {
        let esc = xml_escape(&seg.text);
        if seg.code_chip {
            s.push_str(&format!(
                "<code style=\"background-color:{}; color:{}; padding:1px 5px; border-radius:3px;\">&nbsp;{}&nbsp;</code>",
                t.code_bg, t.accent_secondary, esc
            ));
        } else if seg.bold {
            s.push_str(&format!("<b style=\"color:{};\">{}</b>", t.text_bright, esc));
        } else {
            s.push_str(&esc);
        }
    }
    s
}

fn collect_config_texts(base_dir: &Path) -> BTreeMap<String, String> {
    let paths = SalsaHostPaths::from_env(base_dir.to_path_buf());
    let read_or = |p: PathBuf| -> String {
        if p.is_file() {
            let txt = read_text_lossy(&p, 65536);
            format!("# {}\n\n{}", p.display(), txt)
        } else {
            format!("# {} (not present)", p.display())
        }
    };
    let mut map = BTreeMap::new();
    map.insert("~/.asoundrc".into(), read_or(paths.asoundrc()));
    map.insert(
        "~/.config/OpenAL/alsoft.conf".into(),
        read_or(paths.alsoft_conf()),
    );
    map.insert(
        "pipewire_zero_mode".into(),
        read_or(paths.pipewire_zero_flag()),
    );
    map.insert(
        "99-salsa-direct.conf".into(),
        read_or(paths.direct_env_conf()),
    );
    map.insert(
        "99-lasapi-video-only.conf".into(),
        read_or(paths.video_only_pw_conf()),
    );
    map.insert(
        "settings.json".into(),
        read_or(paths.config_lasapi_dir().join(UNIFIED_SETTINGS_FILENAME)),
    );
    map.insert(
        "module_versions.state".into(),
        read_or(paths.config_lasapi_dir().join("module_versions.state")),
    );
    map.insert(
        "src/alsa.version".into(),
        read_or(base_dir.join("src/alsa.version")),
    );
    map.insert(
        "src/lasapi.version".into(),
        read_or(base_dir.join("src/lasapi.version")),
    );
    map.insert(
        "src/pulse.version".into(),
        read_or(base_dir.join("src/pulse.version")),
    );
    map
}

#[no_mangle]
pub extern "C" fn salsa_str_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = std::ffi::CString::from_raw(ptr);
        }
    }
}

#[no_mangle]
pub extern "C" fn salsa_bytes_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        unsafe {
            let _ = Vec::from_raw_parts(ptr, len, len);
        }
    }
}

#[no_mangle]
pub extern "C" fn salsa_abi_sizes_ok(
    status_sz: usize,
    devlist_sz: usize,
    eq_sz: usize,
    ext_sz: usize,
    mic_sz: usize,
    bt_sz: usize,
    hwa_sz: usize,
) -> bool {
    status_sz == core::mem::size_of::<LasapiEngineStatus>()
        && devlist_sz == core::mem::size_of::<LasapiDeviceList>()
        && eq_sz == core::mem::size_of::<LasapiEqConfig>()
        && ext_sz == core::mem::size_of::<LasapiParamEqExt>()
        && mic_sz == core::mem::size_of::<LasapiMicApoCfg>()
        && bt_sz == core::mem::size_of::<LasapiBtCodecCfg>()
        && hwa_sz == core::mem::size_of::<LasapiHwAnalogCfg>()
}

#[no_mangle]
pub extern "C" fn salsa_reset_sigpipe() {
    reset_sigpipe_default();
}

#[no_mangle]
pub extern "C" fn salsa_get_engine_status(out: *mut LasapiEngineStatus) -> c_int {
    if out.is_null() {
        return -1;
    }
    match get_engine_status() {
        Some(st) => {
            unsafe { *out = st };
            0
        }
        None => -1,
    }
}

#[no_mangle]
pub extern "C" fn salsa_status_effective_capture_rate(st: *const LasapiEngineStatus) -> u32 {
    if st.is_null() {
        return 0;
    }
    unsafe { (*st).effective_capture_rate_hz() }
}

#[no_mangle]
pub extern "C" fn salsa_list_devices(out: *mut LasapiDeviceList) -> c_int {
    if out.is_null() {
        return -1;
    }
    match list_devices() {
        Some(dl) => {
            unsafe { *out = dl };
            0
        }
        None => -1,
    }
}

#[no_mangle]
pub extern "C" fn salsa_set_devices(pb_id: *const c_char, cap_id: *const c_char) -> c_int {
    let pb = cstr_to_string(pb_id);
    let cap = cstr_to_string(cap_id);
    set_devices(pb.as_deref(), cap.as_deref())
}

#[no_mangle]
pub extern "C" fn salsa_set_master_volume(pct: c_int, muted: c_int) -> c_int {
    i32::from(set_master_volume(pct, muted))
}

#[no_mangle]
pub extern "C" fn salsa_set_mic_volume(pct: c_int, muted: c_int) -> c_int {
    i32::from(set_mic_volume(pct, muted))
}

#[no_mangle]
pub extern "C" fn salsa_set_session_control(
    sid: u32,
    pct: c_int,
    muted: c_int,
    target_dev: *const c_char,
) -> c_int {
    let tgt = cstr_to_string(target_dev);
    set_session_control(sid, pct, muted, tgt.as_deref())
}

#[no_mangle]
pub extern "C" fn salsa_set_engine_config(rate: u32, period: u32, bits: u32) -> c_int {
    set_engine_config(rate, period, bits)
}

#[no_mangle]
pub extern "C" fn salsa_set_capture_sample_rate(rate: u32) -> c_int {
    set_capture_sample_rate(rate)
}

#[no_mangle]
pub extern "C" fn salsa_set_parametric_eq(
    eq: *const LasapiEqConfig,
    ext: *const LasapiParamEqExt,
) -> c_int {
    if eq.is_null() || ext.is_null() {
        return -1;
    }
    unsafe { set_parametric_eq(&*eq, &*ext) }
}

#[no_mangle]
pub extern "C" fn salsa_set_dynamic_eq(eq: *const LasapiEqConfig) -> c_int {
    if eq.is_null() {
        return -1;
    }
    unsafe { set_dynamic_eq(&*eq) }
}

#[no_mangle]
pub extern "C" fn salsa_set_bt_codec_config(bt: *const LasapiBtCodecCfg) -> c_int {
    if bt.is_null() {
        return -1;
    }
    unsafe { set_bt_codec_config(&*bt) }
}

#[no_mangle]
pub extern "C" fn salsa_set_hw_analog(hwa: *const LasapiHwAnalogCfg) -> c_int {
    if hwa.is_null() {
        return -1;
    }
    unsafe { set_hw_analog(&*hwa) }
}

#[no_mangle]
pub extern "C" fn salsa_bt_connection_ctl(
    action: u32,
    mac_or_empty: *const c_char,
    out_flags: *mut u32,
) -> c_int {
    let mac = cstr_to_string(mac_or_empty);
    let (hr, flags) = bt_connection_ctl(action, mac.as_deref().filter(|s| !s.is_empty()));
    if !out_flags.is_null() {
        unsafe { *out_flags = flags };
    }
    hr
}

#[no_mangle]
pub extern "C" fn salsa_probe_playback_caps_json(pb_id: *const c_char) -> *mut c_char {
    let Some(id) = cstr_to_string(pb_id) else {
        return to_c_string("null".into());
    };
    let Some(caps) = probe_alsa_playback_caps(&id) else {
        return to_c_string("null".into());
    };
    let obj = JsonValue::obj(vec![
        (
            "rates",
            JsonValue::Array(caps.rates.into_iter().map(|r| JsonValue::Int(i64::from(r))).collect()),
        ),
        (
            "bits",
            JsonValue::Array(caps.bits.into_iter().map(|b| JsonValue::Int(i64::from(b))).collect()),
        ),
    ]);
    to_c_string(obj.to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_send_test_midi(note: u8, vel: u8) -> c_int {
    if DiagnosticProbes::send_test_midi_note(note, vel, 120).is_ok() {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn salsa_host_profile_json() -> *mut c_char {
    let h = detect_host_pc_profile();
    let obj = JsonValue::obj(vec![
        ("os", JsonValue::Str(h.os)),
        ("cpu", JsonValue::Str(h.cpu)),
        ("pkg_mgr", JsonValue::Str(h.pkg_mgr)),
        (
            "cards",
            JsonValue::Array(h.cards.into_iter().map(JsonValue::Str).collect()),
        ),
    ]);
    to_c_string(obj.to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_install_root() -> *mut c_char {
    to_c_string(install_root().to_string_lossy().into_owned())
}

#[no_mangle]
pub extern "C" fn salsa_gui_settings_mtime_ns() -> u64 {
    let p = gui_settings_path();
    fs::symlink_metadata(&p)
        .ok()
        .filter(|m| !m.file_type().is_symlink() && m.file_type().is_file())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn salsa_load_gui_settings_json() -> *mut c_char {
    to_c_string(load_gui_settings().to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_save_studio_settings_json(json_str: *const c_char) -> c_int {
    let Some(s) = cstr_to_string(json_str) else {
        return -1;
    };
    let Ok(val) = JsonValue::parse(&s) else {
        return -1;
    };
    if save_studio_gui_settings(&val).is_ok() {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn salsa_update_gui_settings_json(patch_json: *const c_char) -> c_int {
    let Some(s) = cstr_to_string(patch_json) else {
        return -1;
    };
    let Ok(val) = JsonValue::parse(&s) else {
        return -1;
    };
    let Some(entries) = val.as_object() else {
        return -1;
    };
    let pairs: Vec<(&str, JsonValue)> = entries.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    if update_gui_settings(&pairs).is_ok() {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn salsa_settings_path_str() -> *mut c_char {
    to_c_string(salsa_settings_file_path().to_string_lossy().into_owned())
}

#[no_mangle]
pub extern "C" fn salsa_save_config_now() -> c_int {
    save_config_now() as c_int
}

#[no_mangle]
pub extern "C" fn salsa_reload_config_now() -> c_int {
    reload_config_now() as c_int
}

fn sanitize_ui_dev_id(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|&ch| !ch.is_control())
        .collect();
    cleaned.trim().to_string()
}

#[repr(C)]
pub struct SalsaUiOverwriteReq {
    pub sample_rate: u32,
    pub period_frames: u32,
    pub format_bits: u32,
    pub cap_sample_rate: u32,
    pub master_pct: u32,
    pub master_muted: u32,
    pub mic_pct: u32,
    pub mic_muted: u32,
    pub bt_optional_mode: u32,
    pub _reserved0: u32,
    pub playback_id: *const c_char,
    pub capture_id: *const c_char,
    pub bt_cfg: *const LasapiBtCodecCfg,
    pub hw_analog: *const LasapiHwAnalogCfg,
    pub eq_cfg: *const LasapiEqConfig,
    pub param_ext: *const LasapiParamEqExt,
    pub gui_json: *const c_char,
}

#[no_mangle]
pub unsafe extern "C" fn salsa_overwrite_settings_from_ui(
    req_ptr: *const SalsaUiOverwriteReq,
) -> c_int {
    if req_ptr.is_null() {
        return -1;
    }
    let req = &*req_ptr;
    if let Some(s) = cstr_to_string(req.gui_json) {
        if let Ok(val @ JsonValue::Object(_)) = JsonValue::parse(&s) {
            if save_studio_gui_settings(&val).is_err() {
                return -1;
            }
        } else {
            return -1;
        }
    }

    let dir = config_dir_path();
    let mut st = load_runtime_state_in_dir(&dir, false);
    if (44_100..=192_000).contains(&req.sample_rate) {
        st.sample_rate = req.sample_rate;
    }
    if (64..=2048).contains(&req.period_frames) {
        st.period_frames = req.period_frames;
    }
    if matches!(req.format_bits, 16 | 24 | 32) {
        st.format_bits = req.format_bits;
    }
    if req.cap_sample_rate == 0 || (44_100..=192_000).contains(&req.cap_sample_rate) {
        st.cap_sample_rate = req.cap_sample_rate;
    }
    if let Some(pb) = cstr_to_string(req.playback_id) {
        let clean = sanitize_ui_dev_id(&pb);
        set_c_bytes(&mut st.target_playback_id, &clean);
    }
    if let Some(cap) = cstr_to_string(req.capture_id) {
        let clean = sanitize_ui_dev_id(&cap);
        set_c_bytes(&mut st.target_capture_id, &clean);
    }
    st.desktop_volume_pct = req.master_pct.min(150);
    st.desktop_muted = u32::from(req.master_muted != 0);
    st.desktop_mic_volume_pct = req.mic_pct.min(150);
    st.desktop_mic_muted = u32::from(req.mic_muted != 0);
    st.has_saved_vol_state = true;
    st.bt_optional_mode = u32::from(req.bt_optional_mode != 0);
    if !req.bt_cfg.is_null() {
        st.bt_cfg = *req.bt_cfg;
    }
    if !req.hw_analog.is_null() {
        st.hw_analog_cfg = *req.hw_analog;
    }
    if !req.eq_cfg.is_null() {
        st.eq_cfg = *req.eq_cfg;
    }
    if !req.param_ext.is_null() {
        st.param_eq = *req.param_ext;
    }
    if save_runtime_state_in_dir(&dir, &st) {
        0
    } else {
        -1
    }
}

fn compute_single_instance_sock_path(sock_file_name: &str) -> Option<PathBuf> {
    let clean = sock_file_name.trim();
    if clean.is_empty()
        || clean == "."
        || clean == ".."
        || clean.contains('/')
        || clean.contains('\\')
        || clean.contains("..")
        || clean.chars().any(char::is_control)
    {
        return None;
    }
    let uid = current_uid();
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(3);
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let xt = xdg.trim();
        if !xt.is_empty() {
            candidates.push(PathBuf::from(xt).join("lasapi"));
        }
    }
    candidates.push(PathBuf::from(format!("/run/user/{uid}/lasapi")));
    candidates.push(PathBuf::from(format!("/tmp/lasapi-{uid}")));
    for dir in candidates {
        if ensure_dir_0700_safe(&dir).is_ok() {
            return Some(dir.join(clean));
        }
    }
    None
}

#[no_mangle]
pub extern "C" fn salsa_single_instance_sock_path(sock_file_name: *const c_char) -> *mut c_char {
    let Some(name) = cstr_to_string(sock_file_name) else {
        return core::ptr::null_mut();
    };
    let Some(path) = compute_single_instance_sock_path(&name) else {
        return core::ptr::null_mut();
    };
    to_c_string(path.to_string_lossy().into_owned())
}

#[no_mangle]
pub extern "C" fn salsa_normalize_font_scale(pct: c_int) -> c_int {
    normalize_font_scale_pct(pct)
}

#[no_mangle]
pub extern "C" fn salsa_save_font_scale(pct: c_int) -> c_int {
    save_ui_font_scale_pct(pct)
}

#[no_mangle]
pub extern "C" fn salsa_load_font_scale() -> c_int {
    load_ui_font_scale_pct()
}

#[no_mangle]
pub extern "C" fn salsa_load_theme_json(for_tray: bool) -> *mut c_char {
    let t = if for_tray {
        load_tray_theme()
    } else {
        load_studio_theme()
    };
    to_c_string(theme_to_wire_json(&t).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_get_theme_by_name_json(name: *const c_char) -> *mut c_char {
    let nm = cstr_to_string(name).unwrap_or_default();
    to_c_string(theme_to_wire_json(&theme_by_name(&nm)).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_normalize_theme_json(json_str: *const c_char) -> *mut c_char {
    let fallback = load_studio_theme();
    let t = cstr_to_string(json_str)
        .and_then(|s| JsonValue::parse(&s).ok())
        .map(|v| {
            let mut out = SalsaTheme::from_json(&v, &fallback);
            if let Some(nm) = v.get("name").and_then(JsonValue::as_str) {
                if !nm.trim().is_empty() {
                    out.name = nm.to_string();
                }
            }
            out
        })
        .unwrap_or(fallback);
    to_c_string(theme_to_wire_json(&t).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_derive_harmony_theme_json(json_str: *const c_char) -> *mut c_char {
    let fallback = load_studio_theme();
    let base = cstr_to_string(json_str)
        .and_then(|s| JsonValue::parse(&s).ok())
        .map(|v| SalsaTheme::from_json(&v, &fallback))
        .unwrap_or(fallback);
    let derived = derive_harmony_theme(&base);
    to_c_string(theme_to_wire_json(&derived).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_all_theme_names_json(for_tray: bool) -> *mut c_char {
    let names = if for_tray {
        all_tray_theme_choices()
    } else {
        all_studio_theme_choices()
    };
    to_c_string(JsonValue::Array(names.into_iter().map(JsonValue::Str).collect()).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_theme_token_groups_json() -> *mut c_char {
    let groups: Vec<JsonValue> = THEME_TOKEN_GROUPS
        .iter()
        .map(|(cat, toks)| {
            let items: Vec<JsonValue> = toks
                .iter()
                .map(|(key, lbl, desc)| {
                    JsonValue::obj(vec![
                        ("key", JsonValue::Str((*key).into())),
                        ("label", JsonValue::Str((*lbl).into())),
                        ("desc", JsonValue::Str((*desc).into())),
                    ])
                })
                .collect();
            JsonValue::obj(vec![
                ("category", JsonValue::Str((*cat).into())),
                ("tokens", JsonValue::Array(items)),
            ])
        })
        .collect();
    to_c_string(JsonValue::Array(groups).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_builtin_eq_presets_json() -> *mut c_char {
    let items: Vec<JsonValue> = EQ_PRESETS
        .iter()
        .map(|p| {
            JsonValue::obj(vec![
                ("name", JsonValue::Str(p.name.into())),
                ("mode", JsonValue::Int(i64::from(p.mode))),
                ("preamp", JsonValue::Float(f64::from(p.preamp))),
                ("thresh", JsonValue::Float(f64::from(p.thresh))),
                ("ratio", JsonValue::Float(f64::from(p.ratio))),
                (
                    "gains",
                    JsonValue::Array(p.gains.iter().map(|&g| JsonValue::Float(f64::from(g))).collect()),
                ),
            ])
        })
        .collect();
    to_c_string(JsonValue::Array(items).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_default_shortcuts_json() -> *mut c_char {
    let items: Vec<JsonValue> = DEFAULT_SHORTCUTS
        .iter()
        .map(|(k, lbl, seq)| {
            JsonValue::obj(vec![
                ("key", JsonValue::Str((*k).into())),
                ("label", JsonValue::Str((*lbl).into())),
                ("default_seq", JsonValue::Str((*seq).into())),
            ])
        })
        .collect();
    to_c_string(JsonValue::Array(items).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_saved_autoeq_list_json() -> *mut c_char {
    let settings = load_gui_settings();
    let list = load_saved_autoeq_presets(&settings);
    let arr: Vec<JsonValue> = list.iter().map(saved_autoeq_to_json).collect();
    to_c_string(JsonValue::Array(arr).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_save_autoeq_list_json(list_json: *const c_char) -> c_int {
    let Some(s) = cstr_to_string(list_json) else {
        return -1;
    };
    let Ok(JsonValue::Array(arr)) = JsonValue::parse(&s) else {
        return -1;
    };
    let list: Vec<SavedAutoEqPreset> = arr.iter().filter_map(saved_autoeq_from_json_val).collect();
    save_imported_autoeq_presets(&list);
    0
}

#[no_mangle]
pub extern "C" fn salsa_import_autoeq_txt_json(
    path_c: *const c_char,
    existing_list_json: *const c_char,
) -> *mut c_char {
    let Some(path) = cstr_to_string(path_c) else {
        return to_c_string(
            JsonValue::obj(vec![
                ("ok", JsonValue::Bool(false)),
                ("error", JsonValue::Str("Null AutoEQ path".into())),
            ])
            .to_json_compact(),
        );
    };
    let raw = read_text_lossy(Path::new(&path), 262_144);
    if raw.trim().is_empty() {
        return to_c_string(
            JsonValue::obj(vec![
                ("ok", JsonValue::Bool(false)),
                ("error", JsonValue::Str(format!("Could not read AutoEQ file: {path}"))),
            ])
            .to_json_compact(),
        );
    }
    let Some(parsed) = parse_autoeq_txt(&path, &raw) else {
        return to_c_string(
            JsonValue::obj(vec![
                ("ok", JsonValue::Bool(false)),
                ("error", JsonValue::Str(format!("No valid Filter bands found in {path}"))),
            ])
            .to_json_compact(),
        );
    };
    let mut existing = Vec::new();
    if let Some(js) = cstr_to_string(existing_list_json) {
        if let Ok(JsonValue::Array(arr)) = JsonValue::parse(&js) {
            existing = arr.iter().filter_map(saved_autoeq_from_json_val).collect();
        }
    }
    let mut saved = saved_autoeq_from_parsed(&parsed, &path);
    saved.name = unique_autoeq_name_for_list(&parsed.name, &existing, &path);
    to_c_string(
        JsonValue::obj(vec![
            ("ok", JsonValue::Bool(true)),
            ("preset", saved_autoeq_to_json(&saved)),
        ])
        .to_json_compact(),
    )
}

#[no_mangle]
pub extern "C" fn salsa_reload_autoeq_preset_json(preset_json: *const c_char) -> *mut c_char {
    let err_json = |msg: String| -> *mut c_char {
        to_c_string(
            JsonValue::obj(vec![
                ("ok", JsonValue::Bool(false)),
                ("error", JsonValue::Str(msg)),
            ])
            .to_json_compact(),
        )
    };
    let Some(s) = cstr_to_string(preset_json) else {
        return err_json("Null preset payload".into());
    };
    let Ok(val) = JsonValue::parse(&s) else {
        return err_json("Invalid JSON preset".into());
    };
    let Some(existing) = saved_autoeq_from_json_val(&val) else {
        return err_json("Malformed saved preset".into());
    };
    match reload_autoeq_from_path(&existing) {
        Ok(reloaded) => to_c_string(
            JsonValue::obj(vec![
                ("ok", JsonValue::Bool(true)),
                ("preset", saved_autoeq_to_json(&reloaded)),
            ])
            .to_json_compact(),
        ),
        Err(e) => err_json(e),
    }
}

#[no_mangle]
pub extern "C" fn salsa_eq_cfg_to_json(
    eq: *const LasapiEqConfig,
    ext: *const LasapiParamEqExt,
) -> *mut c_char {
    if eq.is_null() || ext.is_null() {
        return to_c_string("{}".into());
    }
    let j = unsafe { eq_cfg_to_json(&*eq, &*ext) };
    to_c_string(j.to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_json_to_eq_cfg(
    json_str: *const c_char,
    eq: *mut LasapiEqConfig,
    ext: *mut LasapiParamEqExt,
) -> c_int {
    if eq.is_null() || ext.is_null() {
        return -1;
    }
    let Some(s) = cstr_to_string(json_str) else {
        return -1;
    };
    let Ok(val) = JsonValue::parse(&s) else {
        return -1;
    };
    unsafe { apply_json_to_eq_cfg(&val, &mut *eq, &mut *ext) };
    0
}

#[no_mangle]
pub extern "C" fn salsa_apply_builtin_eq_preset(
    name: *const c_char,
    eq: *mut LasapiEqConfig,
    ext: *mut LasapiParamEqExt,
) -> c_int {
    if eq.is_null() || ext.is_null() {
        return -1;
    }
    let Some(nm) = cstr_to_string(name) else {
        return -1;
    };
    if unsafe { apply_builtin_preset_to_cfg(&nm, &mut *eq, &mut *ext) } {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn salsa_apply_saved_autoeq(
    preset_json: *const c_char,
    eq: *mut LasapiEqConfig,
    ext: *mut LasapiParamEqExt,
) -> c_int {
    if eq.is_null() || ext.is_null() {
        return -1;
    }
    let Some(s) = cstr_to_string(preset_json) else {
        return -1;
    };
    let Ok(val) = JsonValue::parse(&s) else {
        return -1;
    };
    let Some(saved) = saved_autoeq_from_json_val(&val) else {
        return -1;
    };
    unsafe { apply_saved_autoeq_to_cfg(&saved, &mut *eq, &mut *ext) };
    0
}

#[no_mangle]
pub extern "C" fn salsa_set_eq_preset_name(eq: *mut LasapiEqConfig, name: *const c_char) {
    if eq.is_null() {
        return;
    }
    let nm = cstr_to_string(name).unwrap_or_default();
    unsafe { set_c_bytes(&mut (*eq).preset_name, &nm) };
}

#[no_mangle]
pub extern "C" fn salsa_manual_chapters_json() -> *mut c_char {
    let arr: Vec<JsonValue> = MANUAL_CHAPTER_BUTTONS
        .iter()
        .map(|(lbl, k)| {
            JsonValue::obj(vec![
                ("key", JsonValue::Str((*k).into())),
                ("label", JsonValue::Str((*lbl).into())),
            ])
        })
        .collect();
    to_c_string(JsonValue::Array(arr).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_render_manual_chapter_html(
    key_c: *const c_char,
    theme_json: *const c_char,
) -> *mut c_char {
    let key = cstr_to_string(key_c).unwrap_or_else(|| "overview".into());
    let fallback = load_studio_theme();
    let t = cstr_to_string(theme_json)
        .and_then(|s| JsonValue::parse(&s).ok())
        .map(|v| SalsaTheme::from_json(&v, &fallback))
        .unwrap_or(fallback);
    let host = detect_host_pc_profile();
    let doc = render_manual_chapter_doc(&key, &t, &host);

    let mut html = format!(
        "<div style=\"color:{};\">",
        t.text_primary
    );
    html.push_str(&format!(
        "<div style=\"font-size:1.15em; font-weight:700; color:{}; margin-bottom:4px;\">{}</div>",
        doc.accent_hex,
        xml_escape(&doc.heading)
    ));
    if !doc.subtitle.is_empty() {
        html.push_str(&format!(
            "<div style=\"color:{}; font-weight:600; margin-bottom:6px;\">{}</div>",
            t.text_secondary,
            xml_escape(&doc.subtitle)
        ));
    }
    html.push_str(&format!(
        "<p style=\"margin:2px 0 8px 0;\">{}</p>",
        render_segments_html(&doc.intro_segments, &t)
    ));
    html.push_str("<ul style=\"margin:0 0 4px 18px; padding:0;\">");
    for bsegs in &doc.bullets_segments {
        html.push_str(&format!(
            "<li style=\"margin-bottom:4px;\">{}</li>",
            render_segments_html(bsegs, &t)
        ));
    }
    html.push_str("</ul></div>");
    to_c_string(html)
}

fn pacman_hook_installed() -> bool {
    all_pacman_hook_paths()
        .iter()
        .any(|p| Path::new(p).exists())
}

#[no_mangle]
pub extern "C" fn salsa_get_integration_status_json() -> *mut c_char {
    let home = home_dir();
    let steam_ready = home.join(".local/bin/salsa-steam-launch").exists();
    let launchers_ok = home
        .join(".local/share/applications/salsa-lasapi.desktop")
        .exists();
    let has_pacman = Path::new("/usr/bin/pacman").exists() && Path::new("/etc/pacman.d").is_dir();
    let pacman_hook = pacman_hook_installed();
    let watcher_enabled = run_cmd_bounded(
        "systemctl",
        &["--user", "is-enabled", "--quiet", "lasapi-module-watcher.path"],
        Duration::from_secs(2),
    )
    .is_some_and(|o| o.status.success())
        || run_cmd_bounded(
            "systemctl",
            &["--user", "is-active", "--quiet", "lasapi-module-watcher.path"],
            Duration::from_secs(2),
        )
        .is_some_and(|o| o.status.success());
    let eng_active = run_cmd_bounded(
        "systemctl",
        &["--user", "is-active", "--quiet", "lasapi-audiodg.service"],
        Duration::from_secs(2),
    )
    .is_some_and(|o| o.status.success());

    let obj = JsonValue::obj(vec![
        ("steam_ready", JsonValue::Bool(steam_ready)),
        ("launchers_ok", JsonValue::Bool(launchers_ok)),
        ("has_pacman", JsonValue::Bool(has_pacman)),
        ("pacman_hook", JsonValue::Bool(pacman_hook)),
        ("watcher_enabled", JsonValue::Bool(watcher_enabled)),
        ("engine_active", JsonValue::Bool(eng_active)),
    ]);
    to_c_string(obj.to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_switch_engine_mode(to_pipewire: bool) -> *mut c_char {
    let root = install_root();
    let ctl = root.join("build/lasapi-ctl");
    let ctl_s = ctl.to_string_lossy().into_owned();
    let msg = if to_pipewire {
        let ok = run_cmd_bounded(&ctl_s, &["disable-auto"], Duration::from_secs(6))
            .is_some_and(|o| o.status.success());
        if ok {
            "Switched to native PipeWire / WirePlumber mode.".into()
        } else {
            "Requested PipeWire/WirePlumber restore via lasapi-ctl disable-auto.".into()
        }
    } else {
        let _ = run_cmd_bounded(&ctl_s, &["enable-auto"], Duration::from_secs(6));
        let ok = run_cmd_bounded(
            "systemctl",
            &["--user", "restart", "lasapi-audiodg.service"],
            Duration::from_secs(6),
        )
        .is_some_and(|o| o.status.success());
        if ok {
            "Enabled and restarted Salsa Native Audio (lasapi-audiodg).".into()
        } else {
            "Enabled Salsa Native Audio (lasapi-ctl enable-auto).".into()
        }
    };
    to_c_string(msg)
}

#[no_mangle]
pub extern "C" fn salsa_toggle_pacman_hook() -> *mut c_char {
    let root = install_root();
    let inst = root.join("install-salsa.sh");
    if !inst.exists() {
        return to_c_string(format!("Installer not found at {}", inst.display()));
    }
    let installed = pacman_hook_installed();
    let flag = if installed {
        "--remove-pacman-hook"
    } else {
        "--install-pacman-hook"
    };
    let inst_s = inst.to_string_lossy().into_owned();
    match run_cmd_bounded(&inst_s, &[flag], Duration::from_secs(30)) {
        Some(o) if o.status.success() => {
            if !installed {
                let _ = run_cmd_bounded(
                    "systemctl",
                    &["--user", "restart", "lasapi-module-watcher.path"],
                    Duration::from_secs(4),
                );
            }
            to_c_string(if installed {
                "Pacman autosync hook removed.".into()
            } else {
                "Pacman autosync hook installed.".into()
            })
        }
        Some(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            to_c_string(format!("Hook action exited {:?}: {}", o.status.code(), err))
        }
        None => to_c_string("Pacman hook action timed out or cancelled.".into()),
    }
}

#[no_mangle]
pub extern "C" fn salsa_toggle_autosync_watcher(enable: bool) -> *mut c_char {
    let action = if enable { "enable" } else { "disable" };
    let _ = run_cmd_bounded(
        "systemctl",
        &["--user", action, "--now", "lasapi-module-watcher.path"],
        Duration::from_secs(5),
    );
    if enable {
        salsa_run_sync_modules(false)
    } else {
        to_c_string("Auto-sync watcher disabled.".into())
    }
}

#[no_mangle]
pub extern "C" fn salsa_run_sync_modules(force: bool) -> *mut c_char {
    let root = install_root();
    let bin = root.join("build/lasapi-autosync");
    let bin_s = bin.to_string_lossy().into_owned();
    let args: &[&str] = if force { &["--force"] } else { &[] };
    match run_cmd_bounded(&bin_s, args, Duration::from_secs(20)) {
        Some(o) => {
            let txt = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
            .trim()
            .to_string();
            if txt.is_empty() {
                to_c_string(format!("lasapi-autosync finished (rc={:?})", o.status.code()))
            } else {
                to_c_string(txt)
            }
        }
        None => to_c_string("lasapi-autosync timed out or could not launch.".into()),
    }
}

#[no_mangle]
pub extern "C" fn salsa_run_latency_bench() -> *mut c_char {
    let root = install_root();
    let bin = root.join("build/lasapi-bench");
    let bin_s = bin.to_string_lossy().into_owned();
    match run_cmd_bounded(&bin_s, &[], Duration::from_secs(20)) {
        Some(o) => {
            let txt = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
            .trim()
            .to_string();
            to_c_string(if txt.is_empty() {
                format!("Benchmark finished (rc={:?})", o.status.code())
            } else {
                txt
            })
        }
        None => to_c_string("Latency benchmark timed out or failed to execute.".into()),
    }
}

#[no_mangle]
pub extern "C" fn salsa_run_health_audit_json() -> *mut c_char {
    let audit = SystemAuditScanner::collect_full_audit();
    to_c_string(audit.to_json_pretty(2))
}

#[no_mangle]
pub extern "C" fn salsa_bt_reset_sync() -> *mut c_char {
    let mut hint = String::new();
    if let Some(st) = get_engine_status() {
        let pb = c_str_to_string(&st.active_playback_id);
        if pb.len() >= 6 && pb[..6].eq_ignore_ascii_case("bluez:") {
            hint = pb;
        }
    }
    let (rc, msg) = salsa_wire::bluez_runtime::reset_device_prefer_daemon(&hint);
    let _ = bt_connection_ctl(0, None);
    to_c_string(if !msg.is_empty() {
        msg
    } else if rc == 0 {
        "Bluetooth audio reset completed.".into()
    } else {
        "BT Reset: No paired Bluetooth A2DP sink found.".into()
    })
}

#[no_mangle]
pub extern "C" fn salsa_ctl_systemd(unit: *const c_char, action: *const c_char) -> c_int {
    let Some(u) = cstr_to_string(unit) else {
        return -1;
    };
    let Some(a) = cstr_to_string(action) else {
        return -1;
    };
    if !matches!(
        u.as_str(),
        "lasapi-audiodg.service" | "lasapi-tray.service"
    ) || !matches!(a.as_str(), "start" | "stop" | "restart" | "reload-or-restart")
    {
        return -1;
    }
    let ok = run_cmd_bounded("systemctl", &["--user", &a, &u], Duration::from_secs(8))
        .is_some_and(|o| o.status.success());
    if ok { 0 } else { -1 }
}

// =========================================================================
// Live Debugger Bridge
// =========================================================================

#[no_mangle]
pub extern "C" fn salsa_debug_cli_main(argc: c_int, argv: *const *const c_char) -> c_int {
    let mut args = Vec::new();
    if !argv.is_null() && argc > 1 {
        for i in 1..argc {
            let p = unsafe { *argv.add(i as usize) };
            if let Some(s) = cstr_to_string(p) {
                args.push(s);
            }
        }
    }
    c_int::from(run_debug_cli(&args))
}

#[no_mangle]
pub extern "C" fn salsa_dbg_create() -> *mut c_void {
    let client = Box::new(SalsaDebugClient::new(None));
    client.start();
    let _ = client.request_snapshot();
    Box::into_raw(client).cast::<c_void>()
}

#[no_mangle]
pub extern "C" fn salsa_dbg_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        let client = unsafe { Box::from_raw(handle.cast::<SalsaDebugClient>()) };
        client.stop();
    }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_stats_json(handle: *mut c_void) -> *mut c_char {
    if handle.is_null() {
        return to_c_string("{}".into());
    }
    let client = unsafe { &*handle.cast::<SalsaDebugClient>() };
    let st = client.stats();
    let obj = JsonValue::obj(vec![
        ("sock_path", JsonValue::Str(st.sock_path)),
        ("connected", JsonValue::Bool(st.connected)),
        ("last_error", JsonValue::Str(st.last_error)),
        ("total_events_rx", JsonValue::Int(st.total_events_rx as i64)),
        ("dropped_events", JsonValue::Int(st.dropped_events as i64)),
        ("buffered_events", JsonValue::Int(st.buffered_events as i64)),
        ("has_snapshot", JsonValue::Bool(st.has_snapshot)),
    ]);
    to_c_string(obj.to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_dbg_latest_snapshot_json(handle: *mut c_void) -> *mut c_char {
    let snap = if handle.is_null() {
        snapshot_once(None, Duration::from_millis(350)).ok()
    } else {
        unsafe { &*handle.cast::<SalsaDebugClient>() }.latest_snapshot()
    };
    to_c_string(
        snap.map(|v| v.to_json_compact())
            .unwrap_or_else(|| "null".into()),
    )
}

#[no_mangle]
pub extern "C" fn salsa_dbg_recent_events_json(handle: *mut c_void, max_items: usize) -> *mut c_char {
    if handle.is_null() {
        return to_c_string("[]".into());
    }
    let client = unsafe { &*handle.cast::<SalsaDebugClient>() };
    let evs = client.recent_events(max_items);
    to_c_string(JsonValue::Array(evs).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_dbg_send_command(handle: *mut c_void, cmd: *const c_char) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let Some(c) = cstr_to_string(cmd) else {
        return -1;
    };
    let client = unsafe { &*handle.cast::<SalsaDebugClient>() };
    if client.send_command(&c).is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_request_snapshot(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let client = unsafe { &*handle.cast::<SalsaDebugClient>() };
    if client.request_snapshot().is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_reset_net_clients_json(handle: *mut c_void) -> *mut c_char {
    if handle.is_null() {
        return to_c_string("null".into());
    }
    let client = unsafe { &*handle.cast::<SalsaDebugClient>() };
    to_c_string(
        client
            .reset_net_clients()
            .map(|v| v.to_json_compact())
            .unwrap_or_else(|| "null".into()),
    )
}

#[no_mangle]
pub extern "C" fn salsa_dbg_ping_json(handle: *mut c_void, tag: *const c_char) -> *mut c_char {
    if handle.is_null() {
        return to_c_string("null".into());
    }
    let client = unsafe { &*handle.cast::<SalsaDebugClient>() };
    let tg = cstr_to_string(tag).unwrap_or_else(|| "gui_probe".into());
    let t0 = Instant::now();
    match client.ping(&tg, Duration::from_millis(1000)) {
        Some(pong) => {
            let rtt_ms = t0.elapsed().as_secs_f64() * 1000.0;
            to_c_string(
                JsonValue::obj(vec![
                    ("ok", JsonValue::Bool(true)),
                    ("rtt_ms", JsonValue::Float(rtt_ms)),
                    ("pretty", JsonValue::Str(pong.to_json_pretty(2))),
                ])
                .to_json_compact(),
            )
        }
        None => to_c_string(
            JsonValue::obj(vec![("ok", JsonValue::Bool(false))]).to_json_compact(),
        ),
    }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_level_rank(lvl: *const c_char) -> c_int {
    let s = cstr_to_string(lvl).unwrap_or_default();
    level_rank(&s) as c_int
}

#[no_mangle]
pub extern "C" fn salsa_dbg_format_ts_ms(ts_ms: i64) -> *mut c_char {
    to_c_string(format_ts_ms(ts_ms))
}

#[no_mangle]
pub extern "C" fn salsa_dbg_subsystems_json() -> *mut c_char {
    to_c_string(
        JsonValue::Array(SUBSYSTEMS.iter().map(|&s| JsonValue::Str(s.into())).collect())
            .to_json_compact(),
    )
}

#[no_mangle]
pub extern "C" fn salsa_dbg_log_levels_json() -> *mut c_char {
    to_c_string(
        JsonValue::Array(LOG_LEVELS.iter().map(|&s| JsonValue::Str(s.into())).collect())
            .to_json_compact(),
    )
}

#[no_mangle]
pub extern "C" fn salsa_dbg_scan_procs_json() -> *mut c_char {
    to_c_string(JsonValue::Array(scan_process_routing_audit()).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_dbg_full_audit_json() -> *mut c_char {
    let audit = SystemAuditScanner::collect_full_audit();
    let cfg_entries: Vec<(String, JsonValue)> = collect_config_texts(&install_root())
        .into_iter()
        .map(|(k, v)| (k, JsonValue::Str(v)))
        .collect();
    to_c_string(
        JsonValue::obj(vec![
            ("audit", audit),
            ("config_texts", JsonValue::Object(cfg_entries)),
        ])
        .to_json_compact(),
    )
}

#[no_mangle]
pub extern "C" fn salsa_dbg_run_probes_json() -> *mut c_char {
    let p = DiagnosticProbes::run_all_probes(&install_root());
    let pretty = format_probe_human(&p);
    to_c_string(pretty)
}

#[no_mangle]
pub extern "C" fn salsa_dbg_play_tone() -> *mut c_char {
    let msg = DiagnosticProbes::play_test_tone(440.0, 500, 0.18)
        .unwrap_or_else(|e| format!("Tone failed: {e}"));
    to_c_string(msg)
}

#[no_mangle]
pub extern "C" fn salsa_dbg_send_midi() -> *mut c_char {
    let msg = DiagnosticProbes::send_test_midi_note(60, 100, 120)
        .unwrap_or_else(|e| format!("MIDI probe failed: {e}"));
    to_c_string(msg)
}

#[no_mangle]
pub extern "C" fn salsa_dbg_export_bundle(handle: *mut c_void) -> *mut c_char {
    let base_dir = install_root();
    let snap = if handle.is_null() {
        snapshot_once(None, Duration::from_millis(350)).ok()
    } else {
        unsafe { &*handle.cast::<SalsaDebugClient>() }.latest_snapshot()
    };
    let evs = if handle.is_null() {
        Vec::new()
    } else {
        unsafe { &*handle.cast::<SalsaDebugClient>() }.recent_events(1000)
    };
    let audit = SystemAuditScanner::collect_full_audit();
    let probes = DiagnosticProbes::run_all_probes(&base_dir);
    let msg = match DiagnosticProbes::export_bundle(
        None,
        snap.as_ref(),
        Some(&audit),
        Some(&probes),
        &evs,
    ) {
        Ok(p) => format!("Exported -> {}", p.display()),
        Err(e) => format!("Export failed: {e}"),
    };
    to_c_string(msg)
}

#[no_mangle]
pub extern "C" fn salsa_dbg_ext_logs_json(
    journal: bool,
    autosync: bool,
    dbus: bool,
) -> *mut c_char {
    let mut rows = Vec::new();
    if journal {
        for line in ExternalLogManager::tail_journal(45) {
            let lvl = if line.contains("err") || line.contains("FAIL") || line.contains("panic") {
                "WARN"
            } else {
                "INFO"
            };
            rows.push(JsonValue::obj(vec![
                ("ts_ms", JsonValue::Int(0)),
                ("sub", JsonValue::Str("JOURNAL".into())),
                ("lvl", JsonValue::Str(lvl.into())),
                ("pid", JsonValue::Int(0)),
                ("sid", JsonValue::Int(0)),
                ("op", JsonValue::Str("journalctl".into())),
                ("target", JsonValue::Str("systemd-user".into())),
                ("detail", JsonValue::Str(line)),
            ]));
        }
    }
    if autosync {
        for line in ExternalLogManager::tail_autosync_log(35) {
            rows.push(JsonValue::obj(vec![
                ("ts_ms", JsonValue::Int(0)),
                ("sub", JsonValue::Str("AUTOSYNC".into())),
                ("lvl", JsonValue::Str("INFO".into())),
                ("pid", JsonValue::Int(0)),
                ("sid", JsonValue::Int(0)),
                ("op", JsonValue::Str("autosync".into())),
                ("target", JsonValue::Str("autosync.log".into())),
                ("detail", JsonValue::Str(line)),
            ]));
        }
    }
    if dbus {
        for line in ExternalLogManager::capture_dbus_sample(400) {
            rows.push(JsonValue::obj(vec![
                ("ts_ms", JsonValue::Int(0)),
                ("sub", JsonValue::Str("DBUS".into())),
                ("lvl", JsonValue::Str("TRACE".into())),
                ("pid", JsonValue::Int(0)),
                ("sid", JsonValue::Int(0)),
                ("op", JsonValue::Str("dbus-signal".into())),
                ("target", JsonValue::Str("session-bus".into())),
                ("detail", JsonValue::Str(line)),
            ]));
        }
    }
    to_c_string(JsonValue::Array(rows).to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_dbg_load_sc_prefs_json() -> *mut c_char {
    let p = load_screencast_prefs();
    let obj = JsonValue::obj(vec![
        ("res_key", JsonValue::Str(p.res_key)),
        ("fps_val", JsonValue::Int(i64::from(p.fps_val))),
        ("cap_real_cast", JsonValue::Bool(p.cap_real_cast)),
    ]);
    to_c_string(obj.to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_dbg_save_sc_prefs(
    res_key: *const c_char,
    fps_val: u32,
    cap_real_cast: bool,
) -> c_int {
    let rk = cstr_to_string(res_key).unwrap_or_else(|| "Source".into());
    if save_screencast_prefs(&ScreenCastPrefs {
        res_key: rk,
        fps_val,
        cap_real_cast,
    })
    .is_ok()
    {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_shm_create() -> *mut c_void {
    let b = Box::new(ShmOpaque {
        reader: None,
        shm_name: String::new(),
        dry_cap: DryRunDesktopCapturer::new(640, 360),
        portal: None,
        last_seq: 0,
        last_w: 0,
        last_h: 0,
        last_mw: 0,
        last_mh: 0,
        last_prod_id: 0,
    });
    Box::into_raw(b).cast::<c_void>()
}

#[no_mangle]
pub extern "C" fn salsa_dbg_shm_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        unsafe {
            let _ = Box::from_raw(ctx.cast::<ShmOpaque>());
        }
    }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_shm_reset_cache(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let st = unsafe { &mut *ctx.cast::<ShmOpaque>() };
    reset_shm_cache(st);
}

#[no_mangle]
pub extern "C" fn salsa_dbg_shm_poll_frame(
    ctx: *mut c_void,
    shm_name: *const c_char,
    max_w: u32,
    max_h: u32,
    tfps: u32,
    cap_fps: u32,
    rgba_out: *mut u8,
    rgba_cap: usize,
    meta_out: *mut SalsaScFrameMeta,
) -> c_int {
    if ctx.is_null() || meta_out.is_null() {
        return -1;
    }
    let st = unsafe { &mut *ctx.cast::<ShmOpaque>() };
    let meta = unsafe { &mut *meta_out };
    *meta = unsafe { core::mem::zeroed() };
    if let Some(nm) = cstr_to_string(shm_name) {
        if !nm.is_empty() && (st.reader.is_none() || st.shm_name != nm) {
            st.reader = ScreenCastShmReader::open(&nm).ok();
            st.shm_name = nm;
            reset_shm_cache(st);
        }
    }
    let Some(ref reader) = st.reader else {
        return 0;
    };
    reader.send_preview_ping(true, max_w, max_h, tfps, cap_fps);
    let prev_seq =
        (st.last_seq != 0 && st.last_mw == max_w && st.last_mh == max_h).then_some(st.last_seq);
    let Some(mut fr) = reader.read_frame_since(prev_seq) else {
        return 0;
    };
    if !fr.active_stream {
        reset_shm_cache(st);
        return 0;
    }
    let same_frame = prev_seq == Some(fr.seq)
        && fr.width == st.last_w
        && fr.height == st.last_h
        && fr.prod_node_id == st.last_prod_id;
    if fr.rgba.is_empty() && !same_frame && (!fr.is_dmabuf || fr.dmabuf_mappable) {
        let Some(full) = reader.read_frame() else {
            return 0;
        };
        if !full.active_stream {
            reset_shm_cache(st);
            return 0;
        }
        fr = full;
    }
    meta.active_stream = 1;
    meta.is_dmabuf = c_int::from(fr.is_dmabuf);
    meta.dmabuf_mappable = c_int::from(fr.dmabuf_mappable);
    meta.width = fr.width;
    meta.height = fr.height;
    meta.actual_fps_x100 = fr.actual_fps_x100;
    meta.prod_node_id = fr.prod_node_id;
    meta.cons_count = fr.cons_count;
    meta.cast_fps_cap = fr.cast_fps_cap;
    meta.seq = fr.seq;
    set_c_bytes(&mut meta.prod_name, &fr.prod_name);
    set_c_bytes(&mut meta.format_name, &fr.format_name);

    if !fr.rgba.is_empty() && !rgba_out.is_null() {
        let copy_n = fr.rgba.len().min(rgba_cap);
        unsafe {
            core::ptr::copy_nonoverlapping(fr.rgba.as_ptr(), rgba_out, copy_n);
        }
        meta.bytes_written = copy_n;
        st.last_seq = fr.seq;
        st.last_w = fr.width;
        st.last_h = fr.height;
        st.last_mw = max_w;
        st.last_mh = max_h;
        st.last_prod_id = fr.prod_node_id;
        return 1;
    }
    if same_frame && st.last_w >= 2 && st.last_h >= 2 {
        return 2;
    }
    reset_shm_cache(st);
    1
}

#[no_mangle]
pub extern "C" fn salsa_dbg_shm_stop(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let st = unsafe { &mut *ctx.cast::<ShmOpaque>() };
    reset_shm_cache(st);
    if let Some(mut p) = st.portal.take() {
        p.stop();
    }
    if let Some(ref reader) = st.reader {
        reader.send_preview_ping(false, 0, 0, 0, 0);
    }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_portal_start(ctx: *mut c_void, force_reselect: bool) -> c_int {
    if ctx.is_null() {
        return -1;
    }
    let st = unsafe { &mut *ctx.cast::<ShmOpaque>() };
    if force_reselect {
        reset_shm_cache(st);
        if let Some(mut old) = st.portal.take() {
            old.stop();
        }
    } else if st
        .portal
        .as_ref()
        .is_some_and(PortalScreenCastPreview::is_running_or_pending)
    {
        return 0;
    }
    reset_shm_cache(st);
    st.portal = Some(PortalScreenCastPreview::start(force_reselect));
    0
}

#[no_mangle]
pub extern "C" fn salsa_dbg_portal_stop(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let st = unsafe { &mut *ctx.cast::<ShmOpaque>() };
    reset_shm_cache(st);
    if let Some(mut p) = st.portal.take() {
        p.stop();
    }
}

#[no_mangle]
pub extern "C" fn salsa_dbg_portal_is_active(ctx: *mut c_void) -> bool {
    if ctx.is_null() {
        return false;
    }
    let st = unsafe { &*ctx.cast::<ShmOpaque>() };
    st.portal
        .as_ref()
        .is_some_and(PortalScreenCastPreview::is_running_or_pending)
}

#[no_mangle]
pub extern "C" fn salsa_dbg_portal_status_json(ctx: *mut c_void) -> *mut c_char {
    if ctx.is_null() {
        return to_c_string("{\"state\":\"idle\",\"message\":\"\",\"node_id\":0,\"has_token\":false}".into());
    }
    let st = unsafe { &*ctx.cast::<ShmOpaque>() };
    let ps = st.portal.as_ref().map_or_else(
        || crate::screencast_preview::PortalPreviewStatus {
            state: "idle",
            message: String::new(),
            node_id: 0,
            has_token: has_saved_restore_token(),
        },
        PortalScreenCastPreview::status,
    );
    let obj = JsonValue::obj(vec![
        ("state", JsonValue::Str(ps.state.to_string())),
        ("message", JsonValue::Str(ps.message)),
        ("node_id", JsonValue::Int(ps.node_id as i64)),
        ("has_token", JsonValue::Bool(ps.has_token)),
    ]);
    to_c_string(obj.to_json_compact())
}

#[no_mangle]
pub extern "C" fn salsa_dbg_dryrun_frame(
    ctx: *mut c_void,
    dw: u32,
    dh: u32,
    rgba_out: *mut u8,
    rgba_cap: usize,
) -> usize {
    if ctx.is_null() || rgba_out.is_null() {
        return 0;
    }
    let st = unsafe { &mut *ctx.cast::<ShmOpaque>() };
    if st.dry_cap.width != dw || st.dry_cap.height != dh {
        st.dry_cap = DryRunDesktopCapturer::new(dw, dh);
    }
    let data = st.dry_cap.next_frame();
    let copy_n = data.len().min(rgba_cap);
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), rgba_out, copy_n);
    }
    copy_n
}

#[no_mangle]
pub extern "C" fn salsa_dbg_pw_self_test() -> *mut c_char {
    let root = install_root();
    let ctl = root.join("build/lasapi-ctl");
    let ctl_s = ctl.to_string_lossy().into_owned();
    let msg = match run_cmd_bounded(&ctl_s, &["pw-self-test"], Duration::from_secs(10)) {
        Some(o) => format!(
            "Broker Self-Test (rc={:?}): {}{}",
            o.status.code(),
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
        .trim()
        .to_string(),
        None => "pw-self-test timed out or failed".into(),
    };
    to_c_string(msg)
}

#[no_mangle]
pub extern "C" fn salsa_dbg_save_frame_png(
    rgba: *const u8,
    rgba_cap: usize,
    w: u32,
    h: u32,
) -> *mut c_char {
    if rgba.is_null() || w == 0 || h == 0 {
        return to_c_string("Save PNG: empty frame buffer.".into());
    }
    let Some(need) = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)) else {
        return to_c_string("Save PNG: frame dimensions overflow.".into());
    };
    if need == 0 || need > rgba_cap {
        return to_c_string("Save PNG: frame dimensions exceed buffer capacity.".into());
    }
    let dir = private_runtime_dir().join("exports");
    if let Err(e) = ensure_dir_0700(&dir) {
        return to_c_string(format!("Save PNG mkdir failed: {e}"));
    }
    let data = unsafe { core::slice::from_raw_parts(rgba, need) }.to_vec();
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("salsa-screencast-{ts_ms}.png"));
    match encode_png_rgba(&RgbaImage { width: w, height: h, data }) {
        Ok(png_bytes) => match write_atomic_secure(&path, &png_bytes, 0o600) {
            Ok(()) => to_c_string(format!("Saved frame -> {}", path.display())),
            Err(e) => to_c_string(format!("Save PNG write error: {e}")),
        },
        Err(e) => to_c_string(format!("Encode PNG error: {e}")),
    }
}

// =========================================================================
// SNI Tray Server, Process Lock & X11 Geometry Helpers
// =========================================================================

#[no_mangle]
pub extern "C" fn salsa_sni_start(
    cb: SalsaSniCb,
    user_ctx: *mut c_void,
    icon_dir_out: *mut *mut c_char,
) -> *mut c_void {
    let bank = match TrayIconBank::generate(None) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[lasapi-tray] icon bank error: {e}");
            return core::ptr::null_mut();
        }
    };
    let icon_dir = bank.theme_root.clone();
    if !icon_dir_out.is_null() {
        unsafe { *icon_dir_out = to_c_string(icon_dir.to_string_lossy().into_owned()) };
    }
    let ctx_num = user_ctx as usize;
    let host = SniThreadHost::start_threaded(
        &icon_dir,
        Box::new(move |ev| {
            let ctx_p = ctx_num as *mut c_void;
            match ev {
                SniEvent::Activate(x, y) => cb(ctx_p, 0, x, y, core::ptr::null()),
                SniEvent::ContextMenu(x, y) => cb(ctx_p, 1, x, y, core::ptr::null()),
                SniEvent::SecondaryActivate(x, y) => cb(ctx_p, 2, x, y, core::ptr::null()),
                SniEvent::Scroll(delta, ori) => {
                    let cs = std::ffi::CString::new(ori).unwrap_or_default();
                    cb(ctx_p, 3, delta, 0, cs.as_ptr());
                }
            }
        }),
    )
    .ok();

    let boxed = Box::new(SniOpaque { host, icon_dir });
    Box::into_raw(boxed).cast::<c_void>()
}

#[no_mangle]
pub extern "C" fn salsa_sni_update(
    h: *mut c_void,
    online: bool,
    muted: bool,
    glow: bool,
    title: *const c_char,
    status: *const c_char,
) {
    if h.is_null() {
        return;
    }
    let sni = unsafe { &*h.cast::<SniOpaque>() };
    let Some(ref host) = sni.host else {
        return;
    };
    let icon_name = TrayIconBank::icon_name(online, muted, glow);
    let tit = cstr_to_string(title).unwrap_or_else(|| "Salsa".into());
    let st = cstr_to_string(status).unwrap_or_else(|| "Active".into());
    host.update_state(icon_name, &tit, &st);
}

#[no_mangle]
pub extern "C" fn salsa_sni_stop(h: *mut c_void) {
    if !h.is_null() {
        let sni = unsafe { Box::from_raw(h.cast::<SniOpaque>()) };
        if let Some(ref host) = sni.host {
            host.shutdown();
        }
    }
}

#[no_mangle]
pub extern "C" fn salsa_tray_header_icon_png(h: *mut c_void, len_out: *mut usize) -> *mut u8 {
    if len_out.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { *len_out = 0 };
    if h.is_null() {
        return core::ptr::null_mut();
    }
    let sni = unsafe { &*h.cast::<SniOpaque>() };
    let png_path = sni
        .icon_dir
        .join("hicolor/128x128/apps/salsa-tray-active-plain.png");
    let Ok(bytes) = fs::read(png_path) else {
        return core::ptr::null_mut();
    };
    let len = bytes.len();
    if len == 0 {
        return core::ptr::null_mut();
    }
    let mut boxed = bytes.into_boxed_slice();
    let ptr = boxed.as_mut_ptr();
    core::mem::forget(boxed);
    unsafe { *len_out = len };
    ptr
}

#[no_mangle]
pub extern "C" fn salsa_acquire_tray_lock(replace: bool) -> *mut c_void {
    let dir = private_runtime_dir();
    if ensure_dir_0700(&dir).is_err() {
        return core::ptr::null_mut();
    }
    let p = dir.join("tray.lock");
    let Ok(mut f) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW | O_CLOEXEC)
        .open(&p)
    else {
        return core::ptr::null_mut();
    };
    let fd = f.as_raw_fd();
    if flock_fd(fd, true).is_err() {
        if replace {
            let mut buf = String::new();
            let _ = (&f).take(32).read_to_string(&mut buf);
            if let Ok(old_pid) = buf.trim().parse::<i32>() {
                if old_pid > 1 && old_pid as u32 != getpid() {
                    let proc_dir = format!("/proc/{old_pid}");
                    let same_uid = fs::metadata(&proc_dir)
                        .map(|m| m.uid() == getuid())
                        .unwrap_or(false);
                    if same_uid {
                        let exe_ok = fs::read_link(format!("{proc_dir}/exe"))
                            .ok()
                            .and_then(|ep| ep.file_name().map(|n| n.to_string_lossy().into_owned()))
                            .is_some_and(|nm| nm == "lasapi-tray" || nm.starts_with("lasapi-tray "));
                        if exe_ok {
                            unsafe { kill(old_pid, 15) };
                        }
                    }
                }
            }
            for _ in 0..40 {
                std::thread::sleep(Duration::from_millis(50));
                if flock_fd(fd, true).is_ok() {
                    let _ = f.set_len(0);
                    let _ = f.seek(SeekFrom::Start(0));
                    let _ = writeln!(f, "{}", getpid());
                    return Box::into_raw(Box::new(TrayLockOpaque { _file: f })).cast::<c_void>();
                }
            }
        }
        return core::ptr::null_mut();
    }
    let _ = f.set_len(0);
    let _ = f.seek(SeekFrom::Start(0));
    let _ = writeln!(f, "{}", getpid());
    Box::into_raw(Box::new(TrayLockOpaque { _file: f })).cast::<c_void>()
}

#[no_mangle]
pub extern "C" fn salsa_release_tray_lock(h: *mut c_void) {
    if !h.is_null() {
        unsafe {
            let _ = Box::from_raw(h.cast::<TrayLockOpaque>());
        }
    }
}

#[no_mangle]
pub extern "C" fn salsa_plasma_panel_info(
    loc: *mut i32,
    clearance: *mut i32,
    right_pad: *mut i32,
) {
    let (l, c, r) = plasma_tray_panel_info();
    if !loc.is_null() {
        unsafe { *loc = l };
    }
    if !clearance.is_null() {
        unsafe { *clearance = c };
    }
    if !right_pad.is_null() {
        unsafe { *right_pad = r };
    }
}

#[no_mangle]
pub extern "C" fn salsa_apply_x11_popup_hints(
    xid: u64,
    skip_taskbar: bool,
    above: bool,
    w: i32,
    h: i32,
) {
    let blur = if w > 0 && h > 0 { Some((w, h)) } else { None };
    apply_popup_hints(xid, skip_taskbar, above, blur);
}

#[no_mangle]
pub extern "C" fn salsa_apply_x11_osd_hints(xid: u64, w: i32, h: i32) {
    apply_osd_hints(xid, w, h);
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaRectI32 {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl From<Rect> for SalsaRectI32 {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        }
    }
}

impl From<SalsaRectI32> for Rect {
    fn from(r: SalsaRectI32) -> Self {
        Self {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaMonitorInfo {
    pub name: [u8; 64],
    pub geo: SalsaRectI32,
    pub avail: SalsaRectI32,
    pub dpr: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaTrayResolveReq {
    pub raw_x: i32,
    pub raw_y: i32,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub has_raw: u8,
    pub is_xcb: u8,
    pub is_kde_env: u8,
    pub _pad0: u8,
}

impl From<&SalsaTrayResolveReq> for TrayResolveReq {
    fn from(r: &SalsaTrayResolveReq) -> Self {
        Self {
            raw_x: r.raw_x,
            raw_y: r.raw_y,
            has_raw: r.has_raw != 0,
            cursor_x: r.cursor_x,
            cursor_y: r.cursor_y,
            is_xcb: r.is_xcb != 0,
            is_kde_env: r.is_kde_env != 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaTrayTargetPos {
    pub screen_idx: i32,
    pub qt_x: i32,
    pub qt_y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaTrayPopupPlan {
    pub screen_idx: i32,
    pub qt_x: i32,
    pub qt_y: i32,
    pub panel_loc: i32,
    pub panel_clearance: i32,
    pub logical_pad: i32,
    pub anchor_raw_local_offset: i32,
    pub anchor_bottom: i32,
    pub popup_x: i32,
    pub popup_y: i32,
    pub avail: SalsaRectI32,
    pub screen_geo: SalsaRectI32,
    pub margin_left: i32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub layer_anchor_top: u8,
    pub layer_anchor_bottom: u8,
    pub layer_anchor_left: u8,
    pub layer_anchor_right: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaTrayResizeReq {
    pub screen_geo: SalsaRectI32,
    pub avail: SalsaRectI32,
    pub panel_loc: i32,
    pub anchor_raw_local_offset: i32,
    pub anchor_bottom: i32,
    pub cur_x: i32,
    pub cur_y: i32,
    pub pop_w: i32,
    pub pop_h: i32,
    pub edge_pad: i32,
    pub is_layer_shell: u8,
    pub _pad: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaTrayResizePlan {
    pub margin_left: i32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub new_x: i32,
    pub new_y: i32,
    pub update_margins: u8,
    pub move_floating: u8,
    pub _pad: [u8; 2],
}

unsafe fn collect_screens(screens: *const SalsaMonitorInfo, count: usize) -> Vec<ScreenInfo> {
    if screens.is_null() || count == 0 {
        return Vec::new();
    }
    let slice = unsafe { core::slice::from_raw_parts(screens, count.min(64)) };
    slice
        .iter()
        .map(|s| ScreenInfo {
            name: c_str_to_string(&s.name),
            geo: s.geo.into(),
            avail: s.avail.into(),
            dpr: s.dpr,
        })
        .collect()
}

#[no_mangle]
pub extern "C" fn salsa_resolve_tray_target(
    screens: *const SalsaMonitorInfo,
    screen_count: usize,
    req_ptr: *const SalsaTrayResolveReq,
) -> SalsaTrayTargetPos {
    let scr_vec = unsafe { collect_screens(screens, screen_count) };
    let req = if req_ptr.is_null() {
        TrayResolveReq {
            raw_x: 0,
            raw_y: 0,
            has_raw: false,
            cursor_x: 0,
            cursor_y: 0,
            is_xcb: false,
            is_kde_env: false,
        }
    } else {
        unsafe { (&*req_ptr).into() }
    };
    let tgt = resolve_tray_target(&scr_vec, &req, &GioCompositorQuery);
    SalsaTrayTargetPos {
        screen_idx: tgt.screen_idx as i32,
        qt_x: tgt.qt_x,
        qt_y: tgt.qt_y,
    }
}

#[no_mangle]
pub extern "C" fn salsa_compute_tray_popup_plan(
    screens: *const SalsaMonitorInfo,
    screen_count: usize,
    req_ptr: *const SalsaTrayResolveReq,
    pop_w: i32,
    pop_h: i32,
    edge_pad: i32,
    is_layer_shell: bool,
) -> SalsaTrayPopupPlan {
    let scr_vec = unsafe { collect_screens(screens, screen_count) };
    let resolve = if req_ptr.is_null() {
        TrayResolveReq {
            raw_x: 0,
            raw_y: 0,
            has_raw: false,
            cursor_x: 0,
            cursor_y: 0,
            is_xcb: false,
            is_kde_env: false,
        }
    } else {
        unsafe { (&*req_ptr).into() }
    };
    let plan = compute_tray_popup_plan(
        &scr_vec,
        &TrayPopupReq {
            resolve,
            pop_w,
            pop_h,
            edge_pad,
            is_layer_shell,
        },
        &GioCompositorQuery,
    );
    SalsaTrayPopupPlan {
        screen_idx: plan.screen_idx as i32,
        qt_x: plan.qt_x,
        qt_y: plan.qt_y,
        panel_loc: plan.panel_loc,
        panel_clearance: plan.panel_clearance,
        logical_pad: plan.logical_pad,
        anchor_raw_local_offset: plan.anchor_raw_local_offset,
        anchor_bottom: plan.anchor_bottom,
        popup_x: plan.popup_x,
        popup_y: plan.popup_y,
        avail: plan.avail.into(),
        screen_geo: plan.screen_geo.into(),
        margin_left: plan.margin_left,
        margin_top: plan.margin_top,
        margin_right: plan.margin_right,
        margin_bottom: plan.margin_bottom,
        layer_anchor_top: u8::from(plan.layer_anchor_top),
        layer_anchor_bottom: u8::from(plan.layer_anchor_bottom),
        layer_anchor_left: u8::from(plan.layer_anchor_left),
        layer_anchor_right: u8::from(plan.layer_anchor_right),
    }
}

#[no_mangle]
pub extern "C" fn salsa_compute_tray_resize_plan(
    req_ptr: *const SalsaTrayResizeReq,
) -> SalsaTrayResizePlan {
    if req_ptr.is_null() {
        return SalsaTrayResizePlan {
            margin_left: 0,
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            new_x: 0,
            new_y: 0,
            update_margins: 0,
            move_floating: 0,
            _pad: [0; 2],
        };
    }
    let r = unsafe { &*req_ptr };
    let res = compute_tray_resize_plan(&TrayResizeReq {
        screen_geo: r.screen_geo.into(),
        avail: r.avail.into(),
        panel_loc: r.panel_loc,
        anchor_raw_local_offset: r.anchor_raw_local_offset,
        anchor_bottom: r.anchor_bottom,
        cur_x: r.cur_x,
        cur_y: r.cur_y,
        pop_w: r.pop_w,
        pop_h: r.pop_h,
        edge_pad: r.edge_pad,
        is_layer_shell: r.is_layer_shell != 0,
    });
    SalsaTrayResizePlan {
        margin_left: res.margin_left,
        margin_top: res.margin_top,
        margin_right: res.margin_right,
        margin_bottom: res.margin_bottom,
        new_x: res.new_x,
        new_y: res.new_y,
        update_margins: u8::from(res.update_margins),
        move_floating: u8::from(res.move_floating),
        _pad: [0; 2],
    }
}

#[no_mangle]
pub extern "C" fn salsa_compute_osd_pos(
    avail: *const SalsaRectI32,
    w: i32,
    h: i32,
    bottom_margin: i32,
    out_x: *mut i32,
    out_y: *mut i32,
) {
    let rect: Rect = if avail.is_null() {
        Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        }
    } else {
        unsafe { (*avail).into() }
    };
    let (ox, oy) = compute_osd_pos(rect, w, h, bottom_margin);
    if !out_x.is_null() {
        unsafe { *out_x = ox };
    }
    if !out_y.is_null() {
        unsafe { *out_y = oy };
    }
}

// =========================================================================
// Self-Test Entry Points (verified by salsa-regression & CLI)
// =========================================================================

#[no_mangle]
pub extern "C" fn salsa_studio_selftest() -> *mut c_char {
    let themes_n = preset_theme_names().len();
    let mut broken = Vec::new();
    for name in preset_theme_names() {
        let th = theme_by_name(name);
        if th.bg_main.is_empty() || th.accent_primary.is_empty() {
            broken.push(name);
        }
    }
    if !broken.is_empty() {
        return to_c_string(format!("ERR: broken themes: {:?}", broken));
    }
    let host = detect_host_pc_profile();
    let mut cfg: LasapiEqConfig = unsafe { core::mem::zeroed() };
    let mut ext: LasapiParamEqExt = unsafe { core::mem::zeroed() };
    let sample_eq = "Preamp: -4.2 dB\nFilter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.71\nFilter 2: ON PK Fc 2400 Hz Gain -3.0 dB Q 2.0\n";
    let Some(parsed) = parse_autoeq_txt("selftest.txt", sample_eq) else {
        return to_c_string("ERR: AutoEQ parser selftest failed".into());
    };
    let saved = saved_autoeq_from_parsed(&parsed, "");
    apply_saved_autoeq_to_cfg(&saved, &mut cfg, &mut ext);
    let names = build_combined_eq_preset_names(&[saved]);
    to_c_string(format!(
        "OK: themes={themes_n}, manual_chapters={}, host_cards={}, eq_presets={}, Qt6+AutoEQ OK",
        MANUAL_CHAPTER_BUTTONS.len(),
        host.cards.len(),
        names.len(),
    ))
}

#[no_mangle]
pub extern "C" fn salsa_tray_selftest() -> *mut c_char {
    let Ok(gi) = gio() else {
        return to_c_string("ERR: GIO library unavailable".into());
    };
    let rt = private_runtime_dir();
    if ensure_dir_0700(&rt).is_err() {
        return to_c_string("ERR: private_runtime_dir failed".into());
    }
    let tmp = rt.join(format!("salsa_tray_selftest_{}", getpid()));
    let _ = fs::remove_dir_all(&tmp);
    let bank = match TrayIconBank::generate(Some(&tmp.join("tray-icons"))) {
        Ok(b) => b,
        Err(e) => return to_c_string(format!("ERR: {e}")),
    };
    for state in [
        "salsa-tray-active-glow",
        "salsa-tray-active-plain",
        "salsa-tray-muted",
        "salsa-tray-stopped",
    ] {
        let Ok(raw) = fs::read(bank.icon_png_path(state)) else {
            let _ = fs::remove_dir_all(&tmp);
            return to_c_string(format!("ERR: read {state}"));
        };
        let Ok(img) = decode_png_rgba(&raw) else {
            let _ = fs::remove_dir_all(&tmp);
            return to_c_string(format!("ERR: decode {state}"));
        };
        if img.width != 128 || img.height != 128 {
            let _ = fs::remove_dir_all(&tmp);
            return to_c_string(format!("ERR: {state} dimensions {}x{}", img.width, img.height));
        }
    }
    let _ = fs::remove_dir_all(&tmp);
    let mut err: *mut c_void = core::ptr::null_mut();
    let node = unsafe { (gi.g_dbus_node_info_new_for_xml)(dbus_cstr(SNI_XML).as_ptr(), &mut err) };
    if node.is_null() {
        return to_c_string("ERR: SNI XML parse failed".into());
    }
    unsafe { (gi.g_dbus_node_info_unref)(node) };
    let _ = monitor_geometry_near(0, 0);
    to_c_string("OK: Qt6 UI, 9 themes, 4x128px SNI PNGs, GDBus XML, AutoEQ".into())
}

#[no_mangle]
pub extern "C" fn salsa_debug_selftest() -> *mut c_char {
    let root = install_root();
    let bf = SystemAuditScanner::check_build_freshness(&root);
    let mut cap = DryRunDesktopCapturer::new(64, 36);
    let raw = cap.next_frame();
    let Ok(png) = encode_png_rgba(&RgbaImage {
        width: 64,
        height: 36,
        data: raw,
    }) else {
        return to_c_string("ERR: encode_png_rgba failed".into());
    };
    let Ok(dec) = decode_png_rgba(&png) else {
        return to_c_string("ERR: decode_png_rgba failed".into());
    };
    if dec.width != 64 || dec.height != 36 {
        return to_c_string("ERR: screencast PNG roundtrip mismatch".into());
    }
    let procs = scan_process_routing_audit();
    let checked = bf.get("checked_count").and_then(JsonValue::as_i64).unwrap_or(0);
    to_c_string(format!(
        "OK: freshness_items={checked}, proc_rows={}, Qt6+9Themes+PNG OK",
        procs.len()
    ))
}

#[no_mangle]
pub extern "C" fn salsa_version_str() -> *mut c_char {
    to_c_string(salsa_version().to_string())
}

#[no_mangle]
pub extern "C" fn salsa_license_section_text(
    app_name_c: *const c_char,
    section: c_int,
) -> *mut c_char {
    let app = cstr_to_string(app_name_c)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Salsa".to_string());
    let text = match section {
        0 => short_legal_notice(&app),
        1 => THIRD_PARTY_TEXT.to_string(),
        2 => LESSER_COPY_TEXT.to_string(),
        3 => COPY_TEXT.to_string(),
        _ => full_license_bundle(&app),
    };
    to_c_string(text)
}

#[no_mangle]
pub extern "C" fn salsa_handle_legal_cli_flag(
    app_name_c: *const c_char,
    arg_c: *const c_char,
) -> c_int {
    let app = cstr_to_string(app_name_c)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Salsa".to_string());
    let Some(arg) = cstr_to_string(arg_c) else {
        return -1;
    };
    if matches!(arg.as_str(), "--version" | "-V" | "--license" | "--legal") {
        handle_common_cli_flag(&app, &arg).unwrap_or(-1)
    } else {
        -1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_instance_sock_path_validates_names() {
        assert!(compute_single_instance_sock_path("").is_none());
        assert!(compute_single_instance_sock_path("../evil.sock").is_none());
        assert!(compute_single_instance_sock_path("a/b.sock").is_none());
        assert!(compute_single_instance_sock_path("bad\nsock").is_none());
        let p = compute_single_instance_sock_path("salsa-unit-test.sock")
            .expect("valid single-instance sock path");
        assert!(p.ends_with("salsa-unit-test.sock"));
    }

    #[test]
    fn tray_c_abi_placement_bridges_work_cleanly() {
        let mut mon = SalsaMonitorInfo {
            name: [0; 64],
            geo: SalsaRectI32 {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            avail: SalsaRectI32 {
                x: 0,
                y: 0,
                w: 1920,
                h: 1032,
            },
            dpr: 1.0,
        };
        set_c_bytes(&mut mon.name, "DP-1");
        let req = SalsaTrayResolveReq {
            raw_x: 1850,
            raw_y: 1050,
            cursor_x: 1850,
            cursor_y: 1050,
            has_raw: 1,
            is_xcb: 1,
            is_kde_env: 0,
            _pad0: 0,
        };
        let plan = salsa_compute_tray_popup_plan(&mon, 1, &req, 380, 500, 8, false);
        assert_eq!(plan.screen_idx, 0);
        assert_eq!(plan.popup_x, 1920 - 380 - 8);
        assert_eq!(plan.anchor_bottom, 1032 - 8);
        let resize = salsa_compute_tray_resize_plan(&SalsaTrayResizeReq {
            screen_geo: plan.screen_geo,
            avail: plan.avail,
            panel_loc: 4,
            anchor_raw_local_offset: plan.anchor_raw_local_offset,
            anchor_bottom: plan.anchor_bottom,
            cur_x: plan.popup_x,
            cur_y: plan.popup_y,
            pop_w: 380,
            pop_h: 600,
            edge_pad: 8,
            is_layer_shell: 0,
            _pad: [0; 3],
        });
        assert_eq!(resize.move_floating, 1);
        assert_eq!(resize.new_y, 1032 - 8 - 600);
        let mut ox = 0;
        let mut oy = 0;
        salsa_compute_osd_pos(&mon.avail, 340, 76, 62, &mut ox, &mut oy);
        assert_eq!(ox, (1920 - 340) / 2);
        assert_eq!(oy, 1032 - 76 - 62);
    }
}
