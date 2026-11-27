// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![deny(unsafe_code)]

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, LasapiEqConfig, LasapiMicApoCfg, LasapiParamEqExt,
    LASAPI_EQ_BANDS, LASAPI_FILTER_HIGH_SHELF, LASAPI_FILTER_PEAK,
};
use crate::json::JsonValue;

pub const DEFAULT_BAND_FREQS_HZ: [f32; LASAPI_EQ_BANDS] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

pub const EQ_PREAMP_MIN_DB: f32 = -24.0;
pub const EQ_PREAMP_MAX_DB: f32 = 24.0;
pub const EQ_BAND_GAIN_MIN_DB: f32 = -24.0;
pub const EQ_BAND_GAIN_MAX_DB: f32 = 24.0;
pub const EQ_DYN_THRESH_MIN_DB: f32 = -60.0;
pub const EQ_DYN_THRESH_MAX_DB: f32 = 0.0;
pub const EQ_DYN_THRESH_DEFAULT_DB: f32 = -16.0;
pub const EQ_DYN_RATIO_MIN: f32 = 1.0;
pub const EQ_DYN_RATIO_MAX: f32 = 20.0;
pub const EQ_DYN_RATIO_DEFAULT: f32 = 2.2;
pub const EQ_CROSSFEED_CUTOFF_MIN_HZ: u32 = 300;
pub const EQ_CROSSFEED_CUTOFF_MAX_HZ: u32 = 1500;
pub const EQ_CROSSFEED_CUTOFF_DEFAULT_HZ: u32 = 700;
pub const EQ_CROSSFEED_LEVEL_DEFAULT: f32 = 0.45;

#[derive(Debug, Clone, Copy)]
pub struct EqPreset {
    pub cli_id: &'static str,
    pub name: &'static str,
    pub mode: u32,
    pub preamp: f32,
    pub thresh: f32,
    pub ratio: f32,
    pub gains: [f32; LASAPI_EQ_BANDS],
}

pub const EQ_PRESETS: &[EqPreset] = &[
    EqPreset {
        cli_id: "flat",
        name: "Flat / Studio Reference",
        mode: 0,
        preamp: 0.0,
        thresh: -18.0,
        ratio: 2.0,
        gains: [0.0; LASAPI_EQ_BANDS],
    },
    EqPreset {
        cli_id: "bose",
        name: "Bose QC Ultra 2 Audiophile",
        mode: 0,
        preamp: -1.5,
        thresh: -16.0,
        ratio: 2.0,
        gains: [1.0, 0.5, -1.5, -0.5, 0.5, 0.5, 0.5, 0.0, -1.5, 0.5],
    },
    EqPreset {
        cli_id: "bass",
        name: "Dynamic Bass & Sub Control",
        mode: 1,
        preamp: -3.0,
        thresh: -16.0,
        ratio: 3.5,
        gains: [6.5, 5.5, 2.5, 0.0, -1.0, 0.0, 1.0, 2.0, 2.5, 2.0],
    },
    EqPreset {
        cli_id: "hires",
        name: "192kHz Hi-Res Mastering",
        mode: 2,
        preamp: -1.0,
        thresh: -20.0,
        ratio: 2.2,
        gains: [1.5, 1.0, 0.0, -0.5, 0.0, 0.5, 1.5, 2.5, 3.5, 4.5],
    },
    EqPreset {
        cli_id: "gaming",
        name: "FPS Gaming Footstep Radar",
        mode: 1,
        preamp: -2.0,
        thresh: -22.0,
        ratio: 4.0,
        gains: [-4.0, -2.0, 1.0, 2.5, 3.0, 4.5, 6.0, 5.5, 3.0, 1.0],
    },
    EqPreset {
        cli_id: "vocal",
        name: "Vocal & Podcast Clarity",
        mode: 1,
        preamp: -1.5,
        thresh: -18.0,
        ratio: 3.0,
        gains: [-5.0, -3.0, -1.0, 1.5, 3.5, 4.5, 4.0, 2.5, 1.5, 0.5],
    },
    EqPreset {
        cli_id: "loudness",
        name: "Late-Night Dynamic Loudness",
        mode: 1,
        preamp: -2.5,
        thresh: -24.0,
        ratio: 4.5,
        gains: [4.5, 3.5, 1.0, 0.0, 0.5, 1.5, 2.5, 3.0, 3.5, 3.0],
    },
];

#[derive(Debug, Clone, Copy)]
pub struct MicApoPreset {
    pub cli_id: &'static str,
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
        cli_id: "discord",
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
        cli_id: "broadcast",
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
        cli_id: "clean",
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
        cli_id: "bypass",
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

#[inline]
pub fn round_decimals(v: f32, scale: f64) -> f64 {
    if v.is_finite() {
        (f64::from(v) * scale).round() / scale
    } else {
        0.0
    }
}

pub fn sanitize_declarative_str(raw: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_control() {
            continue;
        }
        if out.len() + ch.len_utf8() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out
}

pub fn json_bool(val: Option<&JsonValue>, default: bool) -> bool {
    match val {
        Some(JsonValue::Bool(b)) => *b,
        Some(JsonValue::Int(i)) => *i != 0,
        _ => default,
    }
}

pub fn default_eq_cfg() -> LasapiEqConfig {
    let mut preset_name = [0u8; 32];
    copy_str_to_c(&mut preset_name, "Flat / Studio Reference");
    LasapiEqConfig {
        enabled: 1,
        dynamic_mode: 0,
        preamp_db: 0.0,
        dyn_threshold_db: EQ_DYN_THRESH_DEFAULT_DB,
        dyn_ratio: EQ_DYN_RATIO_DEFAULT,
        limiter_enabled: 1,
        band_gains_db: [0.0; LASAPI_EQ_BANDS],
        band_q: [1.414; LASAPI_EQ_BANDS],
        preset_name,
    }
}

pub fn default_param_eq_ext() -> LasapiParamEqExt {
    LasapiParamEqExt {
        band_freq_hz: DEFAULT_BAND_FREQS_HZ,
        band_type: [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS],
        crossfeed_enabled: 0,
        crossfeed_level: EQ_CROSSFEED_LEVEL_DEFAULT,
        crossfeed_cutoff_hz: EQ_CROSSFEED_CUTOFF_DEFAULT_HZ,
        reserved: 0,
    }
}

pub fn init_default_param_eq_ext(ext: &mut LasapiParamEqExt) {
    ext.band_freq_hz = DEFAULT_BAND_FREQS_HZ;
    ext.band_type = [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS];
}

pub fn clamp_eq_cfg(eq: &mut LasapiEqConfig) {
    eq.enabled = u32::from(eq.enabled != 0);
    if eq.dynamic_mode > 2 {
        eq.dynamic_mode = 1;
    }
    eq.limiter_enabled = u32::from(eq.limiter_enabled != 0);
    eq.preamp_db = if eq.preamp_db.is_finite() {
        eq.preamp_db.clamp(EQ_PREAMP_MIN_DB, EQ_PREAMP_MAX_DB)
    } else {
        0.0
    };
    eq.dyn_threshold_db = if eq.dyn_threshold_db.is_finite() {
        eq.dyn_threshold_db
            .clamp(EQ_DYN_THRESH_MIN_DB, EQ_DYN_THRESH_MAX_DB)
    } else {
        EQ_DYN_THRESH_DEFAULT_DB
    };
    eq.dyn_ratio = if eq.dyn_ratio.is_finite() {
        eq.dyn_ratio.clamp(EQ_DYN_RATIO_MIN, EQ_DYN_RATIO_MAX)
    } else {
        EQ_DYN_RATIO_DEFAULT
    };
    for g in &mut eq.band_gains_db {
        *g = if g.is_finite() {
            g.clamp(EQ_BAND_GAIN_MIN_DB, EQ_BAND_GAIN_MAX_DB)
        } else {
            0.0
        };
    }
    for q in &mut eq.band_q {
        if !(0.2..=10.0).contains(q) {
            *q = 1.414;
        }
    }
    if let Some(last) = eq.preset_name.last_mut() {
        *last = 0;
    }
}

pub fn clamp_param_eq_ext(ext: &mut LasapiParamEqExt) {
    for i in 0..LASAPI_EQ_BANDS {
        let f = ext.band_freq_hz[i];
        if !(15.0..=22_000.0).contains(&f) {
            ext.band_freq_hz[i] = DEFAULT_BAND_FREQS_HZ[i];
        }
        if ext.band_type[i] > LASAPI_FILTER_HIGH_SHELF {
            ext.band_type[i] = LASAPI_FILTER_PEAK;
        }
    }
    ext.crossfeed_enabled = u32::from(ext.crossfeed_enabled != 0);
    if !(0.0..=1.0).contains(&ext.crossfeed_level) {
        ext.crossfeed_level = EQ_CROSSFEED_LEVEL_DEFAULT;
    }
    if !(EQ_CROSSFEED_CUTOFF_MIN_HZ..=EQ_CROSSFEED_CUTOFF_MAX_HZ)
        .contains(&ext.crossfeed_cutoff_hz)
    {
        ext.crossfeed_cutoff_hz = EQ_CROSSFEED_CUTOFF_DEFAULT_HZ;
    }
}

pub fn clamp_mic_apo_cfg(m: &mut LasapiMicApoCfg) {
    m.enabled = u32::from(m.enabled != 0);
    m.hpf_enabled = u32::from(m.hpf_enabled != 0);
    m.gate_enabled = u32::from(m.gate_enabled != 0);
    m.rnnoise_enabled = u32::from(m.rnnoise_enabled != 0);
    m.comp_enabled = u32::from(m.comp_enabled != 0);
    m.sidetone_enabled = u32::from(m.sidetone_enabled != 0);

    if !(40.0..=250.0).contains(&m.hpf_hz) {
        m.hpf_hz = 80.0;
    }
    if !(-85.0..=-10.0).contains(&m.gate_thresh_db) {
        m.gate_thresh_db = -50.0;
    }
    if !(-45.0..=-1.0).contains(&m.comp_thresh_db) {
        m.comp_thresh_db = -15.0;
    }
    if !(1.0..=12.0).contains(&m.comp_ratio) {
        m.comp_ratio = 2.2;
    }
    if !(-6.0..=12.0).contains(&m.clarity_db) {
        m.clarity_db = 1.5;
    }
    if m.sidetone_pct > 100 {
        m.sidetone_pct = 100;
    }
}

pub fn eq_pair_to_json(cfg: &LasapiEqConfig, ext: &LasapiParamEqExt) -> JsonValue {
    let pname_raw = c_str_to_string(&cfg.preset_name);
    let pname = if pname_raw.trim().is_empty() {
        "Custom".to_string()
    } else {
        sanitize_declarative_str(&pname_raw, 31)
    };
    let gains: Vec<JsonValue> = cfg
        .band_gains_db
        .iter()
        .map(|&g| JsonValue::Float(round_decimals(g, 10.0)))
        .collect();
    let qs: Vec<JsonValue> = cfg
        .band_q
        .iter()
        .map(|&q| {
            let v = if (0.2..=10.0).contains(&q) { q } else { 1.414 };
            JsonValue::Float(round_decimals(v, 1000.0))
        })
        .collect();
    let freqs: Vec<JsonValue> = ext
        .band_freq_hz
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            let v = if (15.0..=22_000.0).contains(&f) {
                f
            } else {
                DEFAULT_BAND_FREQS_HZ[i]
            };
            JsonValue::Float(round_decimals(v, 10.0))
        })
        .collect();
    let types: Vec<JsonValue> = ext
        .band_type
        .iter()
        .map(|&t| {
            JsonValue::Int(i64::from(if t <= LASAPI_FILTER_HIGH_SHELF {
                t
            } else {
                LASAPI_FILTER_PEAK
            }))
        })
        .collect();
    let xf_cut = if (EQ_CROSSFEED_CUTOFF_MIN_HZ..=EQ_CROSSFEED_CUTOFF_MAX_HZ)
        .contains(&ext.crossfeed_cutoff_hz)
    {
        ext.crossfeed_cutoff_hz
    } else {
        EQ_CROSSFEED_CUTOFF_DEFAULT_HZ
    };

    JsonValue::obj(vec![
        ("enabled", JsonValue::Bool(cfg.enabled != 0)),
        ("dynamic_mode", JsonValue::Int(i64::from(cfg.dynamic_mode))),
        (
            "preamp_db",
            JsonValue::Float(round_decimals(cfg.preamp_db, 10.0)),
        ),
        (
            "dyn_threshold_db",
            JsonValue::Float(round_decimals(cfg.dyn_threshold_db, 10.0)),
        ),
        (
            "dyn_ratio",
            JsonValue::Float(round_decimals(cfg.dyn_ratio, 10.0)),
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
            JsonValue::Float(round_decimals(ext.crossfeed_level, 100.0)),
        ),
        ("crossfeed_cutoff_hz", JsonValue::Int(i64::from(xf_cut))),
    ])
}

pub fn apply_json_to_eq_pair(
    eq_obj: &JsonValue,
    cfg: &mut LasapiEqConfig,
    ext: &mut LasapiParamEqExt,
) -> bool {
    if !matches!(eq_obj, JsonValue::Object(_)) {
        return false;
    }

    let pname = eq_obj
        .get("preset_name")
        .and_then(JsonValue::as_str)
        .map(|s| sanitize_declarative_str(s, 31))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Custom".to_string());
    copy_str_to_c(&mut cfg.preset_name, &pname);

    cfg.enabled = u32::from(json_bool(eq_obj.get("enabled"), true));
    let mode = eq_obj
        .get("dynamic_mode")
        .and_then(JsonValue::as_u64)
        .unwrap_or(1) as u32;
    cfg.dynamic_mode = if mode <= 2 { mode } else { 1 };
    cfg.preamp_db = eq_obj
        .get("preamp_db")
        .and_then(JsonValue::as_f64)
        .map_or(0.0, |v| v as f32);
    cfg.dyn_threshold_db = eq_obj
        .get("dyn_threshold_db")
        .and_then(JsonValue::as_f64)
        .map_or(EQ_DYN_THRESH_DEFAULT_DB, |v| v as f32);
    cfg.dyn_ratio = eq_obj
        .get("dyn_ratio")
        .and_then(JsonValue::as_f64)
        .map_or(EQ_DYN_RATIO_DEFAULT, |v| v as f32);
    cfg.limiter_enabled = u32::from(json_bool(eq_obj.get("limiter_enabled"), true));

    if let Some(arr) = eq_obj.get("band_gains_db").and_then(JsonValue::as_array) {
        if arr.len() == LASAPI_EQ_BANDS {
            for (i, v) in arr.iter().enumerate() {
                cfg.band_gains_db[i] = v.as_f64().map_or(0.0, |x| x as f32);
            }
        }
    }
    if let Some(arr) = eq_obj.get("band_q").and_then(JsonValue::as_array) {
        if arr.len() == LASAPI_EQ_BANDS {
            for (i, v) in arr.iter().enumerate() {
                cfg.band_q[i] = v.as_f64().map_or(1.414, |x| x as f32);
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
                ext.band_freq_hz[i] = v.as_f64().map_or(DEFAULT_BAND_FREQS_HZ[i], |x| x as f32);
            }
        }
    } else {
        ext.band_freq_hz = DEFAULT_BAND_FREQS_HZ;
    }

    if let Some(arr) = eq_obj.get("band_type").and_then(JsonValue::as_array) {
        if arr.len() == LASAPI_EQ_BANDS {
            for (i, v) in arr.iter().enumerate() {
                ext.band_type[i] = v.as_u64().unwrap_or(0) as u32;
            }
        }
    } else {
        ext.band_type = [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS];
    }

    ext.crossfeed_enabled = u32::from(json_bool(eq_obj.get("crossfeed_enabled"), false));
    ext.crossfeed_level = eq_obj
        .get("crossfeed_level")
        .and_then(JsonValue::as_f64)
        .map_or(EQ_CROSSFEED_LEVEL_DEFAULT, |v| v as f32);
    ext.crossfeed_cutoff_hz = eq_obj
        .get("crossfeed_cutoff_hz")
        .and_then(JsonValue::as_u64)
        .unwrap_or(u64::from(EQ_CROSSFEED_CUTOFF_DEFAULT_HZ)) as u32;

    clamp_eq_cfg(cfg);
    clamp_param_eq_ext(ext);
    true
}

pub fn sanitize_device_eq_profiles_map(val: &JsonValue) -> JsonValue {
    let Some(entries) = val.as_object() else {
        return JsonValue::Object(Vec::new());
    };
    let mut clean = Vec::new();
    for (k, v) in entries {
        let trimmed = sanitize_declarative_str(k, 64);
        if trimmed.is_empty() || !matches!(v, JsonValue::Object(_)) {
            continue;
        }
        let mut eq = default_eq_cfg();
        let mut ext = default_param_eq_ext();
        apply_json_to_eq_pair(v, &mut eq, &mut ext);
        clean.push((trimmed, eq_pair_to_json(&eq, &ext)));
    }
    JsonValue::Object(clean)
}

pub fn is_per_device_eq_enabled(settings_doc: &JsonValue) -> bool {
    settings_doc
        .get("eq_per_device")
        .and_then(JsonValue::as_bool)
        .or_else(|| {
            settings_doc
                .get("eq_per_device_enabled")
                .and_then(JsonValue::as_bool)
        })
        .unwrap_or(false)
}

pub fn extract_per_device_eq_for_sink(
    settings_doc: &JsonValue,
    sink_id: &str,
) -> Option<(LasapiEqConfig, LasapiParamEqExt)> {
    let trimmed = sink_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    let map = settings_doc
        .get("per_device_eq")
        .or_else(|| settings_doc.get("eq_device_profiles"))?;
    let entry = map
        .get(trimmed)
        .or_else(|| {
            map.as_object().and_then(|pairs| {
                pairs
                    .iter()
                    .find(|(k, _)| k.trim().eq_ignore_ascii_case(trimmed))
                    .map(|(_, v)| v)
            })
        })?;
    if !matches!(entry, JsonValue::Object(_)) {
        return None;
    }
    let mut eq = default_eq_cfg();
    let mut ext = default_param_eq_ext();
    apply_json_to_eq_pair(entry, &mut eq, &mut ext);
    Some((eq, ext))
}

pub fn find_eq_preset(query: &str) -> Option<&'static EqPreset> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    EQ_PRESETS
        .iter()
        .find(|p| p.cli_id.eq_ignore_ascii_case(trimmed) || p.name == trimmed)
        .or_else(|| {
            EQ_PRESETS
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(trimmed))
        })
}

pub fn apply_builtin_eq_preset(
    preset_name_or_id: &str,
    cfg: &mut LasapiEqConfig,
    ext: &mut LasapiParamEqExt,
) -> bool {
    let Some(preset) = find_eq_preset(preset_name_or_id) else {
        return false;
    };
    cfg.enabled = 1;
    cfg.dynamic_mode = preset.mode;
    cfg.preamp_db = preset.preamp;
    cfg.dyn_threshold_db = preset.thresh;
    cfg.dyn_ratio = preset.ratio;
    cfg.limiter_enabled = 1;
    copy_str_to_c(&mut cfg.preset_name, preset.name);
    cfg.band_gains_db = preset.gains;
    cfg.band_q = [1.414; LASAPI_EQ_BANDS];
    init_default_param_eq_ext(ext);
    clamp_eq_cfg(cfg);
    clamp_param_eq_ext(ext);
    true
}

pub fn find_mic_apo_preset(query: &str) -> Option<&'static MicApoPreset> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("natural") {
        return find_mic_apo_preset("clean");
    }
    if trimmed.eq_ignore_ascii_case("raw") || trimmed.eq_ignore_ascii_case("off") {
        return find_mic_apo_preset("bypass");
    }
    MIC_APO_PRESETS
        .iter()
        .find(|p| p.cli_id.eq_ignore_ascii_case(trimmed) || p.name.eq_ignore_ascii_case(trimmed))
}

pub fn apply_mic_apo_preset(query: &str, cfg: &mut LasapiMicApoCfg) -> bool {
    let Some(preset) = find_mic_apo_preset(query) else {
        return false;
    };
    cfg.enabled = preset.enabled;
    cfg.hpf_enabled = preset.hpf_enabled;
    cfg.hpf_hz = preset.hpf_hz;
    cfg.gate_enabled = preset.gate_enabled;
    cfg.gate_thresh_db = preset.gate_thresh_db;
    cfg.rnnoise_enabled = preset.rnnoise_enabled;
    cfg.comp_enabled = preset.comp_enabled;
    cfg.comp_thresh_db = preset.comp_thresh_db;
    cfg.comp_ratio = preset.comp_ratio;
    cfg.clarity_db = preset.clarity_db;
    clamp_mic_apo_cfg(cfg);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_dsp_bounds_and_custom_crossfeed_roundtrip() {
        let mut eq = default_eq_cfg();
        let mut ext = default_param_eq_ext();
        eq.preamp_db = 21.5;
        eq.dyn_ratio = 16.5;
        eq.band_gains_db[0] = -22.0;
        eq.band_gains_db[9] = 22.0;
        ext.crossfeed_enabled = 1;
        ext.crossfeed_cutoff_hz = 1120;
        ext.crossfeed_level = 0.62;

        let encoded = eq_pair_to_json(&eq, &ext);
        let mut dec_eq = default_eq_cfg();
        let mut dec_ext = default_param_eq_ext();
        assert!(apply_json_to_eq_pair(&encoded, &mut dec_eq, &mut dec_ext));
        assert!((dec_eq.preamp_db - 21.5).abs() < 0.05);
        assert!((dec_eq.dyn_ratio - 16.5).abs() < 0.05);
        assert!((dec_eq.band_gains_db[0] - (-22.0)).abs() < 0.05);
        assert!((dec_eq.band_gains_db[9] - 22.0).abs() < 0.05);
        assert_eq!(dec_ext.crossfeed_cutoff_hz, 1120);
        assert!((dec_ext.crossfeed_level - 0.62).abs() < 0.02);
    }

    #[test]
    fn preset_resets_parametric_freqs_while_keeping_crossfeed() {
        let mut eq = default_eq_cfg();
        let mut ext = default_param_eq_ext();
        ext.band_freq_hz[0] = 105.0;
        ext.band_type[0] = LASAPI_FILTER_HIGH_SHELF;
        ext.crossfeed_enabled = 1;
        ext.crossfeed_cutoff_hz = 950;

        assert!(apply_builtin_eq_preset("gaming", &mut eq, &mut ext));
        assert_eq!(c_str_to_string(&eq.preset_name), "FPS Gaming Footstep Radar");
        assert_eq!(ext.band_freq_hz, DEFAULT_BAND_FREQS_HZ);
        assert_eq!(ext.band_type, [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS]);
        assert_eq!(ext.crossfeed_enabled, 1);
        assert_eq!(ext.crossfeed_cutoff_hz, 950);
    }

    #[test]
    fn per_device_eq_reads_canonical_and_legacy_keys() {
        let mut bass_eq = default_eq_cfg();
        let mut bass_ext = default_param_eq_ext();
        assert!(apply_builtin_eq_preset("bass", &mut bass_eq, &mut bass_ext));

        let doc_primary = JsonValue::obj(vec![
            ("eq_per_device", JsonValue::Bool(true)),
            (
                "per_device_eq",
                sanitize_device_eq_profiles_map(&JsonValue::obj(vec![(
                    " hw:0,0 ",
                    eq_pair_to_json(&bass_eq, &bass_ext),
                )])),
            ),
        ]);
        assert!(is_per_device_eq_enabled(&doc_primary));
        let (loaded_eq, _) =
            extract_per_device_eq_for_sink(&doc_primary, "hw:0,0").expect("per_device_eq hit");
        assert_eq!(
            c_str_to_string(&loaded_eq.preset_name),
            "Dynamic Bass & Sub Control"
        );

        let doc_legacy = JsonValue::obj(vec![
            ("eq_per_device_enabled", JsonValue::Bool(true)),
            (
                "eq_device_profiles",
                JsonValue::obj(vec![("bluez:AA:BB:CC:DD:EE:FF", eq_pair_to_json(&bass_eq, &bass_ext))]),
            ),
        ]);
        assert!(is_per_device_eq_enabled(&doc_legacy));
        let (legacy_eq, _) =
            extract_per_device_eq_for_sink(&doc_legacy, "bluez:aa:bb:cc:dd:ee:ff")
                .expect("legacy fallback hit");
        assert_eq!(
            c_str_to_string(&legacy_eq.preset_name),
            "Dynamic Bass & Sub Control"
        );
    }

    #[test]
    fn mic_apo_presets_lookup_and_clamp() {
        let mut mic = LasapiMicApoCfg {
            enabled: 0,
            hpf_enabled: 0,
            hpf_hz: 80.0,
            gate_enabled: 0,
            gate_thresh_db: -50.0,
            rnnoise_enabled: 0,
            comp_enabled: 0,
            comp_thresh_db: -15.0,
            comp_ratio: 2.2,
            clarity_db: 1.5,
            sidetone_enabled: 1,
            sidetone_pct: 35,
        };
        assert!(apply_mic_apo_preset("natural", &mut mic));
        assert_eq!(mic.enabled, 1);
        assert_eq!(mic.rnnoise_enabled, 0);
        assert_eq!(mic.sidetone_pct, 35);
    }
}
