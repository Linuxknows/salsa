// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use salsa_tools::host::{home_dir, read_text_lossy};
use salsa_tools::ipc::{
    gui_settings_path, salsa_state_path, update_gui_settings, LasapiEqConfig, LasapiParamEqExt,
    DEFAULT_BAND_FREQS_HZ, EQ_PRESETS, LASAPI_EQ_BANDS, LASAPI_FILTER_HIGH_SHELF,
    LASAPI_FILTER_LOW_SHELF, LASAPI_FILTER_PEAK,
};
use salsa_tools::json::JsonValue;

use crate::studio_manual::{parse_autoeq_txt, AutoEqProfile};

pub const CUSTOM_PRESET_NAME: &str = "Custom Salsa Curve";

pub const DYN_MODE_ITEMS: &[(&str, &str, u32)] = &[
    ("0: Static Parametric EQ", "Static Parametric", 0),
    ("1: Dynamic Compress (Tame Peaks)", "Dynamic Compress", 1),
    ("2: Dynamic Expand (Upward Punch)", "Dynamic Expand", 2),
];

pub const DEFAULT_SHORTCUTS: &[(&str, &str, &str)] = &[
    ("advanced", "Advanced Options", "Ctrl+,"),
    ("topology", "Topology & Manual", "F1"),
    ("debugger", "Live Debugger", "Ctrl+D"),
    ("mute_spk", "Mute Speaker", "Ctrl+M"),
    ("mute_mic", "Mute Mic", "Ctrl+Shift+M"),
    ("toggle_eq", "Toggle Dynamic EQ", "Ctrl+E"),
    ("vol_up", "Speaker Volume +Step", "Ctrl+Up"),
    ("vol_down", "Speaker Volume -Step", "Ctrl+Down"),
    ("mic_up", "Mic Gain +Step", "Ctrl+Alt+Up"),
    ("mic_down", "Mic Gain -Step", "Ctrl+Alt+Down"),
    ("bt_reset", "Reset Bluetooth", "Ctrl+Shift+B"),
    ("save_config", "Save Settings", "Ctrl+S"),
    ("refresh", "Force Poll / Sync UI", "F5"),
    ("quit", "Quit Salsa", "Ctrl+Q"),
];

pub fn c_bytes_to_str(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

pub fn set_c_bytes<const N: usize>(dst: &mut [u8; N], src: &str) {
    dst.fill(0);
    if N <= 1 {
        return;
    }
    let max_bytes = N - 1;
    let mut end = src.len().min(max_bytes);
    while end > 0 && !src.is_char_boundary(end) {
        end -= 1;
    }
    dst[..end].copy_from_slice(&src.as_bytes()[..end]);
}

pub fn format_band_freq_short(hz: f32, filter_type: u32) -> String {
    let base = if hz >= 1000.0 {
        let khz = hz / 1000.0;
        if (khz - khz.round()).abs() < 0.06 {
            format!("{:.0}k", khz.round())
        } else {
            format!("{khz:.1}k")
        }
    } else {
        format!("{:.0} Hz", hz.round())
    };
    match filter_type {
        LASAPI_FILTER_LOW_SHELF => format!("{base} LS"),
        LASAPI_FILTER_HIGH_SHELF => format!("{base} HS"),
        _ => base,
    }
}

pub fn dyn_mode_short(mode: u32) -> &'static str {
    DYN_MODE_ITEMS
        .iter()
        .find(|(_, _, m)| *m == mode)
        .map(|(_, s, _)| *s)
        .unwrap_or("Dynamic Compress")
}

pub fn dyn_mode_full(mode: u32) -> &'static str {
    DYN_MODE_ITEMS
        .iter()
        .find(|(_, _, m)| *m == mode)
        .map(|(f, _, _)| *f)
        .unwrap_or("1: Dynamic Compress (Tame Peaks)")
}

pub fn eq_cfg_to_json(cfg: &LasapiEqConfig, ext: &LasapiParamEqExt) -> JsonValue {
    let pname = {
        let s = c_bytes_to_str(&cfg.preset_name);
        if s.is_empty() {
            CUSTOM_PRESET_NAME.to_string()
        } else {
            s
        }
    };
    let gains: Vec<JsonValue> = cfg
        .band_gains_db
        .iter()
        .map(|&g| JsonValue::Float((f64::from(g) * 10.0).round() / 10.0))
        .collect();
    let qs: Vec<JsonValue> = cfg
        .band_q
        .iter()
        .map(|&q| {
            let v = if q > 0.05 { f64::from(q) } else { 1.414 };
            JsonValue::Float((v * 1000.0).round() / 1000.0)
        })
        .collect();
    let freqs: Vec<JsonValue> = ext
        .band_freq_hz
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            let v = if f > 1.0 {
                f64::from(f)
            } else {
                f64::from(DEFAULT_BAND_FREQS_HZ[i])
            };
            JsonValue::Float((v * 10.0).round() / 10.0)
        })
        .collect();
    let types: Vec<JsonValue> = ext
        .band_type
        .iter()
        .map(|&t| JsonValue::Int(i64::from(if t <= 2 { t } else { 0 })))
        .collect();
    let xf_cut = if ext.crossfeed_cutoff_hz > 0 {
        ext.crossfeed_cutoff_hz
    } else {
        700
    };

    JsonValue::obj(vec![
        ("enabled", JsonValue::Bool(cfg.enabled != 0)),
        ("dynamic_mode", JsonValue::Int(i64::from(cfg.dynamic_mode))),
        (
            "preamp_db",
            JsonValue::Float((f64::from(cfg.preamp_db) * 10.0).round() / 10.0),
        ),
        (
            "dyn_threshold_db",
            JsonValue::Float((f64::from(cfg.dyn_threshold_db) * 10.0).round() / 10.0),
        ),
        (
            "dyn_ratio",
            JsonValue::Float((f64::from(cfg.dyn_ratio) * 10.0).round() / 10.0),
        ),
        ("limiter_enabled", JsonValue::Bool(cfg.limiter_enabled != 0)),
        ("preset_name", JsonValue::Str(pname)),
        ("band_gains_db", JsonValue::Array(gains)),
        ("band_q", JsonValue::Array(qs)),
        ("band_freq_hz", JsonValue::Array(freqs)),
        ("band_type", JsonValue::Array(types)),
        (
            "crossfeed_enabled",
            JsonValue::Bool(ext.crossfeed_enabled != 0),
        ),
        (
            "crossfeed_level",
            JsonValue::Float((f64::from(ext.crossfeed_level) * 100.0).round() / 100.0),
        ),
        ("crossfeed_cutoff_hz", JsonValue::Int(i64::from(xf_cut))),
    ])
}

pub fn apply_json_to_eq_cfg(
    eq_obj: &JsonValue,
    cfg: &mut LasapiEqConfig,
    ext: &mut LasapiParamEqExt,
) {
    let pname = eq_obj
        .get("preset_name")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(CUSTOM_PRESET_NAME);
    set_c_bytes(&mut cfg.preset_name, pname);

    cfg.enabled = u32::from(
        eq_obj
            .get("enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true),
    );
    let mode = eq_obj
        .get("dynamic_mode")
        .and_then(JsonValue::as_u64)
        .unwrap_or(1) as u32;
    cfg.dynamic_mode = if mode <= 2 { mode } else { 1 };
    cfg.preamp_db = (eq_obj
        .get("preamp_db")
        .and_then(JsonValue::as_f64)
        .unwrap_or(0.0) as f32)
        .clamp(-18.0, 12.0);
    cfg.dyn_threshold_db = (eq_obj
        .get("dyn_threshold_db")
        .and_then(JsonValue::as_f64)
        .unwrap_or(-18.0) as f32)
        .clamp(-60.0, 0.0);
    cfg.dyn_ratio = (eq_obj
        .get("dyn_ratio")
        .and_then(JsonValue::as_f64)
        .unwrap_or(2.5) as f32)
        .clamp(1.0, 8.0);
    cfg.limiter_enabled = u32::from(
        eq_obj
            .get("limiter_enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true),
    );

    if let Some(arr) = eq_obj.get("band_gains_db").and_then(JsonValue::as_array) {
        if arr.len() == LASAPI_EQ_BANDS {
            for (i, v) in arr.iter().enumerate() {
                cfg.band_gains_db[i] = (v.as_f64().unwrap_or(0.0) as f32).clamp(-18.0, 18.0);
            }
        }
    }
    if let Some(arr) = eq_obj.get("band_q").and_then(JsonValue::as_array) {
        if arr.len() == LASAPI_EQ_BANDS {
            for (i, v) in arr.iter().enumerate() {
                let q = v.as_f64().unwrap_or(1.414) as f32;
                cfg.band_q[i] = if q >= 0.2 { q.clamp(0.2, 10.0) } else { 1.414 };
            }
        }
    } else {
        for q in &mut cfg.band_q {
            if *q < 0.2 {
                *q = 1.414;
            }
        }
    }

    if let Some(arr) = eq_obj.get("band_freq_hz").and_then(JsonValue::as_array) {
        if arr.len() == LASAPI_EQ_BANDS {
            for (i, v) in arr.iter().enumerate() {
                let f = v.as_f64().unwrap_or(f64::from(DEFAULT_BAND_FREQS_HZ[i])) as f32;
                ext.band_freq_hz[i] = if f > 1.0 {
                    f.clamp(15.0, 22_000.0)
                } else {
                    DEFAULT_BAND_FREQS_HZ[i]
                };
            }
        }
    } else {
        ext.band_freq_hz = DEFAULT_BAND_FREQS_HZ;
    }

    if let Some(arr) = eq_obj.get("band_type").and_then(JsonValue::as_array) {
        if arr.len() == LASAPI_EQ_BANDS {
            for (i, v) in arr.iter().enumerate() {
                let t = v.as_u64().unwrap_or(0) as u32;
                ext.band_type[i] = if t <= 2 { t } else { LASAPI_FILTER_PEAK };
            }
        }
    } else {
        ext.band_type = [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS];
    }

    ext.crossfeed_enabled = u32::from(
        eq_obj
            .get("crossfeed_enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    );
    ext.crossfeed_level = (eq_obj
        .get("crossfeed_level")
        .and_then(JsonValue::as_f64)
        .unwrap_or(0.30) as f32)
        .clamp(0.0, 1.0);
    let cut = eq_obj
        .get("crossfeed_cutoff_hz")
        .and_then(JsonValue::as_u64)
        .unwrap_or(700) as u32;
    ext.crossfeed_cutoff_hz = if [650, 700, 850].contains(&cut) {
        cut
    } else {
        700
    };
}

// ============================================================================
// Persistent Imported AutoEQ Preset Library (gui_settings.json)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct SavedAutoEqBand {
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub filter_type: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedAutoEqPreset {
    pub name: String,
    pub source_path: String,
    pub preamp_db: f32,
    pub bands: [SavedAutoEqBand; LASAPI_EQ_BANDS],
}

fn default_saved_bands() -> [SavedAutoEqBand; LASAPI_EQ_BANDS] {
    core::array::from_fn(|i| SavedAutoEqBand {
        freq_hz: DEFAULT_BAND_FREQS_HZ[i],
        gain_db: 0.0,
        q: 1.414,
        filter_type: LASAPI_FILTER_PEAK,
    })
}

pub fn unique_autoeq_name(raw_name: &str) -> String {
    let trimmed = raw_name.trim();
    let base = if trimmed.is_empty() {
        "Imported AutoEQ"
    } else {
        trimmed
    };
    let collides_builtin = EQ_PRESETS.iter().any(|p| p.name.eq_ignore_ascii_case(base))
        || base.eq_ignore_ascii_case(CUSTOM_PRESET_NAME);
    let mut name = if collides_builtin {
        let suffix = " (AutoEQ)";
        let max_base = 31 - suffix.len();
        let mut end = base.len().min(max_base);
        while !base.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        format!("{}{}", &base[..end], suffix)
    } else {
        base.to_string()
    };
    if name.len() > 31 {
        let mut end = 31;
        while !name.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        name.truncate(end);
    }
    name
}

pub fn unique_autoeq_name_for_list(
    raw_name: &str,
    existing: &[SavedAutoEqPreset],
    source_path: &str,
) -> String {
    let base = unique_autoeq_name(raw_name);
    let src_clean = source_path.trim();
    if existing.iter().any(|p| p.source_path.trim() == src_clean && !src_clean.is_empty()) {
        return existing
            .iter()
            .find(|p| p.source_path.trim() == src_clean)
            .map(|p| p.name.clone())
            .unwrap_or(base);
    }
    if !existing.iter().any(|p| p.name.eq_ignore_ascii_case(&base)) {
        return base;
    }
    for n in 2..=99u32 {
        let suffix = format!(" ({n})");
        let max_b = 31usize.saturating_sub(suffix.len());
        let mut end = base.len().min(max_b);
        while end > 0 && !base.is_char_boundary(end) {
            end -= 1;
        }
        let cand = format!("{}{}", &base[..end], suffix);
        if !existing.iter().any(|p| p.name.eq_ignore_ascii_case(&cand)) {
            return cand;
        }
    }
    base
}

pub fn saved_autoeq_from_parsed(prof: &AutoEqProfile, source_path: &str) -> SavedAutoEqPreset {
    let mut bands = default_saved_bands();
    for (i, b) in prof.bands.iter().take(LASAPI_EQ_BANDS).enumerate() {
        bands[i] = SavedAutoEqBand {
            freq_hz: b.freq_hz.clamp(15.0, 22_000.0),
            gain_db: b.gain_db.clamp(-18.0, 18.0),
            q: if b.q >= 0.2 {
                b.q.clamp(0.2, 10.0)
            } else {
                1.414
            },
            filter_type: if b.filter_type <= 2 {
                b.filter_type
            } else {
                LASAPI_FILTER_PEAK
            },
        };
    }
    SavedAutoEqPreset {
        name: unique_autoeq_name(&prof.name),
        source_path: source_path.trim().to_string(),
        preamp_db: prof.preamp_db.clamp(-18.0, 12.0),
        bands,
    }
}

pub fn saved_autoeq_to_json(p: &SavedAutoEqPreset) -> JsonValue {
    let bands_json: Vec<JsonValue> = p
        .bands
        .iter()
        .map(|b| {
            JsonValue::obj(vec![
                (
                    "freq_hz",
                    JsonValue::Float((f64::from(b.freq_hz) * 10.0).round() / 10.0),
                ),
                (
                    "gain_db",
                    JsonValue::Float((f64::from(b.gain_db) * 10.0).round() / 10.0),
                ),
                (
                    "q",
                    JsonValue::Float((f64::from(b.q) * 1000.0).round() / 1000.0),
                ),
                ("filter_type", JsonValue::Int(i64::from(b.filter_type))),
            ])
        })
        .collect();
    JsonValue::obj(vec![
        ("name", JsonValue::Str(p.name.clone())),
        ("source_path", JsonValue::Str(p.source_path.clone())),
        (
            "preamp_db",
            JsonValue::Float((f64::from(p.preamp_db) * 10.0).round() / 10.0),
        ),
        ("bands", JsonValue::Array(bands_json)),
    ])
}

pub fn saved_autoeq_list_to_json(list: &[SavedAutoEqPreset]) -> JsonValue {
    JsonValue::Array(list.iter().map(saved_autoeq_to_json).collect())
}

pub fn load_saved_autoeq_presets(settings: &JsonValue) -> Vec<SavedAutoEqPreset> {
    let mut out = Vec::new();
    let Some(arr) = settings
        .get("imported_eq_presets")
        .and_then(JsonValue::as_array)
    else {
        return out;
    };
    for item in arr {
        let Some(name_raw) = item.get("name").and_then(JsonValue::as_str) else {
            continue;
        };
        let name = unique_autoeq_name(name_raw);
        if name.is_empty() || out.iter().any(|x: &SavedAutoEqPreset| x.name == name) {
            continue;
        }
        let source_path = item
            .get("source_path")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let preamp_db = (item
            .get("preamp_db")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0) as f32)
            .clamp(-18.0, 12.0);
        let mut bands = default_saved_bands();
        if let Some(barr) = item.get("bands").and_then(JsonValue::as_array) {
            for (i, bv) in barr.iter().take(LASAPI_EQ_BANDS).enumerate() {
                let freq = (bv
                    .get("freq_hz")
                    .and_then(JsonValue::as_f64)
                    .unwrap_or(f64::from(DEFAULT_BAND_FREQS_HZ[i]))
                    as f32)
                    .clamp(15.0, 22_000.0);
                let gain = (bv
                    .get("gain_db")
                    .and_then(JsonValue::as_f64)
                    .unwrap_or(0.0) as f32)
                    .clamp(-18.0, 18.0);
                let q_raw = bv.get("q").and_then(JsonValue::as_f64).unwrap_or(1.414) as f32;
                let q = if q_raw >= 0.2 {
                    q_raw.clamp(0.2, 10.0)
                } else {
                    1.414
                };
                let ft = bv
                    .get("filter_type")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(u64::from(LASAPI_FILTER_PEAK)) as u32;
                bands[i] = SavedAutoEqBand {
                    freq_hz: freq,
                    gain_db: gain,
                    q,
                    filter_type: if ft <= 2 { ft } else { LASAPI_FILTER_PEAK },
                };
            }
        }
        out.push(SavedAutoEqPreset {
            name,
            source_path,
            preamp_db,
            bands,
        });
    }
    out
}

pub fn save_imported_autoeq_presets(list: &[SavedAutoEqPreset]) {
    let _ = update_gui_settings(&[("imported_eq_presets", saved_autoeq_list_to_json(list))]);
}

pub fn upsert_saved_autoeq_preset(
    list: &mut Vec<SavedAutoEqPreset>,
    preset: SavedAutoEqPreset,
) {
    if let Some(slot) = list.iter_mut().find(|p| p.name == preset.name) {
        *slot = preset;
    } else {
        list.push(preset);
    }
}

pub fn remove_saved_autoeq_preset(list: &mut Vec<SavedAutoEqPreset>, name: &str) -> bool {
    let before = list.len();
    list.retain(|p| p.name != name);
    list.len() != before
}

pub fn find_saved_autoeq_preset<'a>(
    list: &'a [SavedAutoEqPreset],
    name: &str,
) -> Option<&'a SavedAutoEqPreset> {
    let trimmed = name.trim();
    list.iter()
        .find(|p| p.name == trimmed)
        .or_else(|| list.iter().find(|p| p.name.eq_ignore_ascii_case(trimmed)))
        .or_else(|| {
            if trimmed.len() >= 31 {
                list.iter().find(|p| {
                    let mut buf = [0u8; 32];
                    set_c_bytes(&mut buf, &p.name);
                    c_bytes_to_str(&buf).eq_ignore_ascii_case(trimmed)
                })
            } else {
                None
            }
        })
}

pub fn apply_saved_autoeq_to_cfg(
    preset: &SavedAutoEqPreset,
    cfg: &mut LasapiEqConfig,
    ext: &mut LasapiParamEqExt,
) {
    cfg.enabled = 1;
    cfg.preamp_db = preset.preamp_db.clamp(-18.0, 12.0);
    set_c_bytes(&mut cfg.preset_name, &preset.name);
    for (i, b) in preset.bands.iter().enumerate() {
        cfg.band_gains_db[i] = b.gain_db.clamp(-18.0, 18.0);
        cfg.band_q[i] = if b.q >= 0.2 {
            b.q.clamp(0.2, 10.0)
        } else {
            1.414
        };
        ext.band_freq_hz[i] = b.freq_hz.clamp(15.0, 22_000.0);
        ext.band_type[i] = if b.filter_type <= 2 {
            b.filter_type
        } else {
            LASAPI_FILTER_PEAK
        };
    }
}

pub fn apply_builtin_preset_to_cfg(
    preset_name: &str,
    cfg: &mut LasapiEqConfig,
    ext: &mut LasapiParamEqExt,
) -> bool {
    let trimmed = preset_name.trim();
    let Some(preset) = EQ_PRESETS
        .iter()
        .find(|p| p.name == trimmed)
        .or_else(|| EQ_PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(trimmed)))
    else {
        return false;
    };
    cfg.enabled = 1;
    cfg.dynamic_mode = preset.mode;
    cfg.preamp_db = preset.preamp;
    cfg.dyn_threshold_db = preset.thresh;
    cfg.dyn_ratio = preset.ratio;
    set_c_bytes(&mut cfg.preset_name, preset.name);
    cfg.band_gains_db = preset.gains;
    cfg.band_q = [1.414; LASAPI_EQ_BANDS];
    ext.band_freq_hz = DEFAULT_BAND_FREQS_HZ;
    ext.band_type = [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS];
    true
}

pub fn build_combined_eq_preset_names(saved: &[SavedAutoEqPreset]) -> Vec<String> {
    let mut out: Vec<String> = EQ_PRESETS.iter().map(|p| p.name.to_string()).collect();
    for s in saved {
        if !out.iter().any(|x| x == &s.name) {
            out.push(s.name.clone());
        }
    }
    out
}

pub fn reload_autoeq_from_path(existing: &SavedAutoEqPreset) -> Result<SavedAutoEqPreset, String> {
    let path_str = existing.source_path.trim();
    if path_str.is_empty() {
        return Err("No source .txt path is recorded for this preset.".to_string());
    }
    let path = Path::new(path_str);
    if !path.is_file() {
        return Err(format!("AutoEQ file not found: {path_str}"));
    }
    let txt = read_text_lossy(path, 262_144);
    let parsed = parse_autoeq_txt(path_str, &txt)
        .ok_or_else(|| format!("No valid Filter / Preamp lines found in {path_str}"))?;
    let mut updated = saved_autoeq_from_parsed(&parsed, path_str);
    updated.name = existing.name.clone();
    Ok(updated)
}

pub fn should_push_saved_eq_on_startup(settings: &JsonValue) -> bool {
    let has_saved = settings.get("eq_config").is_some()
        || settings.get("eq_preset").is_some()
        || settings.get("eq_dynamic_mode").is_some();
    if !has_saved {
        return false;
    }
    let state_bin = salsa_state_path();
    let Ok(bin_meta) = fs::metadata(&state_bin) else {
        return true;
    };
    let Ok(json_meta) = fs::metadata(gui_settings_path()) else {
        return false;
    };
    match (json_meta.modified(), bin_meta.modified()) {
        (Ok(jm), Ok(bm)) => jm > bm,
        _ => false,
    }
}

pub fn load_shortcuts_map(settings: &JsonValue) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for &(k, _lbl, def_seq) in DEFAULT_SHORTCUTS {
        out.insert(k.to_string(), def_seq.to_string());
    }
    if let Some(sc_obj) = settings.get("shortcuts").and_then(JsonValue::as_object) {
        for (k, v) in sc_obj {
            if out.contains_key(k) {
                if let Some(s) = v.as_str() {
                    out.insert(k.clone(), s.trim().to_string());
                }
            }
        }
    }
    out
}

pub fn shortcuts_map_to_json(map: &BTreeMap<String, String>) -> JsonValue {
    let mut entries = Vec::with_capacity(map.len());
    for &(k, _lbl, def_seq) in DEFAULT_SHORTCUTS {
        let v = map.get(k).map(String::as_str).unwrap_or(def_seq);
        entries.push((k, JsonValue::Str(v.to_string())));
    }
    JsonValue::obj(entries)
}

pub fn normalize_shortcut_seq(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut super_mod = false;
    let mut key_part = String::new();
    for p in parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "super" | "meta" | "win" => super_mod = true,
            _ => key_part = p.to_string(),
        }
    }
    if key_part.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if super_mod {
        out.push_str("Super+");
    }
    if ctrl {
        out.push_str("Ctrl+");
    }
    if alt {
        out.push_str("Alt+");
    }
    if shift {
        out.push_str("Shift+");
    }
    if key_part.len() == 1 {
        out.push_str(&key_part.to_ascii_uppercase());
    } else {
        out.push_str(&key_part);
    }
    out
}

#[derive(Debug, Clone)]
pub struct AutoEqDirEntry {
    pub label: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub fn list_autoeq_dir_entries(target: &Path) -> (PathBuf, Vec<AutoEqDirEntry>) {
    let dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target
            .parent()
            .filter(|p| p.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(home_dir)
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                dirs.push((name.to_string(), p));
            } else {
                let low = name.to_ascii_lowercase();
                if low.ends_with(".txt") || low.ends_with(".apo") || low.ends_with(".csv") {
                    files.push((name.to_string(), p));
                }
            }
        }
    }
    dirs.sort_by_key(|(n, _)| n.to_ascii_lowercase());
    files.sort_by_key(|(n, _)| n.to_ascii_lowercase());

    let mut out = Vec::with_capacity(dirs.len() + files.len());
    for (name, p) in dirs {
        out.push(AutoEqDirEntry {
            label: format!("📁  {name}/"),
            path: p,
            is_dir: true,
        });
    }
    for (name, p) in files {
        out.push(AutoEqDirEntry {
            label: format!("📄  {name}"),
            path: p,
            is_dir: false,
        });
    }
    (dir, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem;

    #[test]
    fn shortcut_normalization_works() {
        assert_eq!(normalize_shortcut_seq("ctrl + ,"), "Ctrl+,");
        assert_eq!(normalize_shortcut_seq("shift+ctrl+m"), "Ctrl+Shift+M");
        assert_eq!(normalize_shortcut_seq("F1"), "F1");
        assert_eq!(normalize_shortcut_seq("ctrl+alt+Up"), "Ctrl+Alt+Up");
        assert_eq!(normalize_shortcut_seq("  "), "");
        assert_eq!(normalize_shortcut_seq("Ctrl+Shift+"), "");
    }

    #[test]
    fn eq_json_roundtrip_preserves_parametric_and_crossfeed() {
        let mut cfg: LasapiEqConfig = unsafe { mem::zeroed() };
        let mut ext: LasapiParamEqExt = unsafe { mem::zeroed() };
        cfg.enabled = 1;
        cfg.dynamic_mode = 2;
        cfg.preamp_db = -2.5;
        cfg.dyn_threshold_db = -20.0;
        cfg.dyn_ratio = 3.2;
        cfg.limiter_enabled = 1;
        cfg.band_gains_db = [1.5, -2.0, 0.0, 3.0, -1.0, 0.5, 2.0, 4.0, -0.5, 1.0];
        cfg.band_q = [0.71, 1.414, 2.1, 1.414, 1.414, 1.414, 1.414, 1.414, 1.414, 0.85];
        set_c_bytes(&mut cfg.preset_name, "Bose Oratory");
        ext.band_freq_hz = [
            105.0, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 10000.0,
        ];
        ext.band_type[0] = LASAPI_FILTER_LOW_SHELF;
        ext.band_type[9] = LASAPI_FILTER_HIGH_SHELF;
        ext.crossfeed_enabled = 1;
        ext.crossfeed_level = 0.45;
        ext.crossfeed_cutoff_hz = 850;

        let j = eq_cfg_to_json(&cfg, &ext);
        let mut cfg2: LasapiEqConfig = unsafe { mem::zeroed() };
        let mut ext2: LasapiParamEqExt = unsafe { mem::zeroed() };
        apply_json_to_eq_cfg(&j, &mut cfg2, &mut ext2);

        assert_eq!(c_bytes_to_str(&cfg2.preset_name), "Bose Oratory");
        assert_eq!(cfg2.dynamic_mode, 2);
        assert!((cfg2.preamp_db - (-2.5)).abs() < 0.05);
        assert_eq!(ext2.band_type[0], LASAPI_FILTER_LOW_SHELF);
        assert_eq!(ext2.band_type[9], LASAPI_FILTER_HIGH_SHELF);
        assert_eq!(ext2.crossfeed_cutoff_hz, 850);
        assert_eq!(
            format_band_freq_short(105.0, LASAPI_FILTER_LOW_SHELF),
            "105 Hz LS"
        );
        assert_eq!(
            format_band_freq_short(10_000.0, LASAPI_FILTER_HIGH_SHELF),
            "10k HS"
        );
    }

    #[test]
    fn autoeq_persistence_switch_and_reload_preserve_profiles() {
        let tmp_path = std::env::temp_dir().join(format!(
            "salsa_autoeq_reload_test_{}.txt",
            salsa_tools::host::getpid()
        ));
        let txt_v1 = "\
Preamp: -4.5 dB
Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.71
Filter 2: ON PK Fc 1450 Hz Gain -3.2 dB Q 2.10
Filter 3: ON HSC Fc 9500 Hz Gain 2.0 dB Q 0.85
";
        fs::write(&tmp_path, txt_v1).unwrap();

        let parsed =
            parse_autoeq_txt(&tmp_path.to_string_lossy(), txt_v1).expect("parsed v1 autoeq");
        let saved = saved_autoeq_from_parsed(&parsed, &tmp_path.to_string_lossy());
        let mut library = Vec::new();
        upsert_saved_autoeq_preset(&mut library, saved.clone());

        // Round-trip via JSON as stored in gui_settings.json
        let root_json = JsonValue::obj(vec![(
            "imported_eq_presets",
            saved_autoeq_list_to_json(&library),
        )]);
        let loaded = load_saved_autoeq_presets(&root_json);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, saved.name);
        assert_eq!(loaded[0].source_path, tmp_path.to_string_lossy());
        assert!((loaded[0].bands[1].freq_hz - 1450.0).abs() < 0.5);

        let mut cfg: LasapiEqConfig = unsafe { mem::zeroed() };
        let mut ext: LasapiParamEqExt = unsafe { mem::zeroed() };
        apply_saved_autoeq_to_cfg(&loaded[0], &mut cfg, &mut ext);
        assert_eq!(c_bytes_to_str(&cfg.preset_name), saved.name);
        assert_eq!(ext.band_type[0], LASAPI_FILTER_LOW_SHELF);
        assert_eq!(ext.band_type[2], LASAPI_FILTER_HIGH_SHELF);

        // Selecting a built-in preset must reset parametric freqs/types without erasing `loaded`
        let builtin = &EQ_PRESETS[1];
        assert!(apply_builtin_preset_to_cfg(builtin.name, &mut cfg, &mut ext));
        assert_eq!(c_bytes_to_str(&cfg.preset_name), builtin.name);
        assert_eq!(ext.band_freq_hz, DEFAULT_BAND_FREQS_HZ);
        assert_eq!(ext.band_type, [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS]);
        assert!((cfg.preamp_db - builtin.preamp).abs() < 0.01);
        assert_eq!(
            find_saved_autoeq_preset(&loaded, &saved.name).map(|p| &p.name),
            Some(&saved.name)
        );

        // Switching back to the saved AutoEQ preset restores custom freqs/types/preamp
        apply_saved_autoeq_to_cfg(&loaded[0], &mut cfg, &mut ext);
        assert!((cfg.preamp_db - (-4.5)).abs() < 0.05);
        assert_eq!(ext.band_type[0], LASAPI_FILTER_LOW_SHELF);

        // Modify file on disk and verify 1-click reload updates parameters from source_path
        let txt_v2 = "\
Preamp: -6.0 dB
Filter 1: ON PK Fc 220 Hz Gain -4.0 dB Q 1.80
";
        fs::write(&tmp_path, txt_v2).unwrap();
        let reloaded = reload_autoeq_from_path(&loaded[0]).expect("reload from .txt");
        assert_eq!(reloaded.name, saved.name);
        assert!((reloaded.preamp_db - (-6.0)).abs() < 0.01);
        assert!((reloaded.bands[0].freq_hz - 220.0).abs() < 0.1);
        assert_eq!(reloaded.bands[0].filter_type, LASAPI_FILTER_PEAK);

        let _ = fs::remove_file(&tmp_path);
    }
}
