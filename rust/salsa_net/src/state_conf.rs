// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::fs::{self, OpenOptions, Permissions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, LasapiBtCodecCfg, LasapiEqConfig,
    LasapiMicApoCfg, LasapiParamEqExt, SalsaRuntimeState, LASAPI_BT_PREF_AUTO,
    LASAPI_BT_PREF_SBC, LASAPI_EQ_BANDS, LASAPI_FILTER_PEAK, LASAPI_LDAC_MODE_330,
    LASAPI_LDAC_MODE_ADAPTIVE,
};
use crate::hw_analog::{clamp_hw_analog_cfg, default_hw_analog_cfg};
use crate::json::JsonValue;
pub use crate::settings::{
    apply_json_to_eq_pair, clamp_eq_cfg, clamp_mic_apo_cfg, clamp_param_eq_ext,
    eq_pair_to_json as eq_cfg_to_json_value, json_bool, round_decimals, sanitize_declarative_str,
    DEFAULT_BAND_FREQS_HZ as K_EQ_FREQS,
};
use crate::unix_sys::{
    current_pid, ensure_dir_0700_safe, flock_fd, funlock_fd, is_safe_owner_dir, posix_owner_id,
    read_verified_regular_file_utf8, user_home_dir, write_atomic_secure_file, O_CLOEXEC,
    O_NOFOLLOW,
};

pub const SALSA_SETTINGS_FILENAME: &str = "salsa_settings.json";
pub const SALSA_SETTINGS_SCHEMA_VERSION: i64 = 1;
const MAX_SETTINGS_JSON_BYTES: usize = 524_288;

const MAGIC_V1: u32 = 0x5341_4C53; // 'SALS'
const MAGIC_V2: u32 = 0x5341_4C54; // 'SALT'
const MAGIC_V3: u32 = 0x5341_4C55; // 'SALU'
const MAGIC_V4: u32 = 0x5341_4C56; // 'SALV'
const MAGIC_V5: u32 = 0x5341_4C57; // 'SALW'
const MAGIC_V6: u32 = 0x5341_4C58; // 'SALX'

pub const PERSISTED_V1_SIZE: usize = 152;
pub const PERSISTED_V2_SIZE: usize = 280;
pub const PERSISTED_V3_SIZE: usize = 532;
pub const PERSISTED_V4_SIZE: usize = 556;
pub const PERSISTED_V5_SIZE: usize = 576;
pub const PERSISTED_V6_SIZE: usize = 580;

pub fn default_runtime_state() -> SalsaRuntimeState {
    let mut preset_name = [0u8; 32];
    copy_str_to_c(&mut preset_name, "Flat / Studio Reference");

    let eq_cfg = LasapiEqConfig {
        enabled: 1,
        dynamic_mode: 0,
        preamp_db: 0.0,
        dyn_threshold_db: -16.0,
        dyn_ratio: 2.2,
        limiter_enabled: 1,
        band_gains_db: [0.0; LASAPI_EQ_BANDS],
        band_q: [1.414; LASAPI_EQ_BANDS],
        preset_name,
    };

    let param_eq = LasapiParamEqExt {
        band_freq_hz: K_EQ_FREQS,
        band_type: [LASAPI_FILTER_PEAK; LASAPI_EQ_BANDS],
        crossfeed_enabled: 0,
        crossfeed_level: 0.45,
        crossfeed_cutoff_hz: 700,
        reserved: 0,
    };

    let mic_apo = LasapiMicApoCfg {
        enabled: 0,
        hpf_enabled: 1,
        hpf_hz: 80.0,
        gate_enabled: 1,
        gate_thresh_db: -50.0,
        rnnoise_enabled: 1,
        comp_enabled: 1,
        comp_thresh_db: -15.0,
        comp_ratio: 2.2,
        clarity_db: 1.5,
        sidetone_enabled: 0,
        sidetone_pct: 0,
    };

    let bt_cfg = LasapiBtCodecCfg {
        preferred_codec: LASAPI_BT_PREF_AUTO,
        ldac_mode: LASAPI_LDAC_MODE_ADAPTIVE,
        wireless_abr_enabled: 1,
        effective_kbps: 0,
        supported_codecs_mask: 0,
        hfp_sco_enabled: 0,
        hfp_sco_active: 0,
        bt_sco_codec: [0u8; 8],
        neg_reason: [0u8; 64],
    };

    let hw_analog_cfg = default_hw_analog_cfg();

    let mut target_playback_id = [0u8; 64];
    let mut target_capture_id = [0u8; 64];
    copy_str_to_c(&mut target_playback_id, "auto");
    copy_str_to_c(&mut target_capture_id, "auto");

    SalsaRuntimeState {
        sample_rate: 48_000,
        period_frames: 256,
        format_bits: 32,
        cap_sample_rate: 0,
        target_playback_id,
        target_capture_id,
        eq_cfg,
        param_eq,
        mic_apo,
        bt_cfg,
        hw_analog_cfg,
        desktop_volume_pct: 100,
        desktop_muted: 0,
        desktop_mic_volume_pct: 100,
        desktop_mic_muted: 0,
        desktop_mic_gain_ppm: 1_000_000,
        has_saved_vol_state: false,
        bt_optional_mode: 1,
    }
}

fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn read_f32(buf: &[u8], off: usize) -> f32 {
    f32::from_bits(read_u32(buf, off))
}

fn write_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_ne_bytes());
}

fn write_f32(buf: &mut [u8], off: usize, v: f32) {
    write_u32(buf, off, v.to_bits());
}

fn copy_c_field(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let nul = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    let n = nul.min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&src[..n]);
}

pub fn decode_persisted_state_bytes(bytes: &[u8], out: &mut SalsaRuntimeState) -> bool {
    if bytes.len() < PERSISTED_V1_SIZE {
        return false;
    }
    let magic = read_u32(bytes, 0);
    let valid_v6 = bytes.len() == PERSISTED_V6_SIZE && magic == MAGIC_V6;
    let valid_v5 = bytes.len() == PERSISTED_V5_SIZE && magic == MAGIC_V5;
    let valid_v4 = bytes.len() >= PERSISTED_V4_SIZE && magic == MAGIC_V4;
    let valid_v3 = bytes.len() >= PERSISTED_V3_SIZE && magic == MAGIC_V3;
    let valid_v2 = bytes.len() >= PERSISTED_V2_SIZE && magic == MAGIC_V2;
    let valid_v1 = bytes.len() >= PERSISTED_V1_SIZE && magic == MAGIC_V1;
    if !(valid_v6 || valid_v5 || valid_v4 || valid_v3 || valid_v2 || valid_v1) {
        return false;
    }

    let mut st = default_runtime_state();

    let sr = read_u32(bytes, 4);
    if (44_100..=192_000).contains(&sr) {
        st.sample_rate = sr;
    }
    let pf = read_u32(bytes, 8);
    if (64..=2048).contains(&pf) {
        st.period_frames = pf;
    }
    let fb = read_u32(bytes, 12);
    if matches!(fb, 16 | 24 | 32) {
        st.format_bits = fb;
    }

    // Offset 16..152: LasapiEqConfig (136 bytes)
    st.eq_cfg.enabled = read_u32(bytes, 16);
    st.eq_cfg.dynamic_mode = read_u32(bytes, 20);
    st.eq_cfg.preamp_db = read_f32(bytes, 24);
    st.eq_cfg.dyn_threshold_db = read_f32(bytes, 28);
    st.eq_cfg.dyn_ratio = read_f32(bytes, 32);
    st.eq_cfg.limiter_enabled = read_u32(bytes, 36);
    for i in 0..LASAPI_EQ_BANDS {
        st.eq_cfg.band_gains_db[i] = read_f32(bytes, 40 + i * 4);
        let q = read_f32(bytes, 80 + i * 4);
        st.eq_cfg.band_q[i] = if (0.2..=10.0).contains(&q) { q } else { 1.414 };
    }
    st.eq_cfg.preset_name.copy_from_slice(&bytes[120..152]);
    st.eq_cfg.preset_name[31] = 0;
    clamp_eq_cfg(&mut st.eq_cfg);

    if valid_v6 || valid_v5 || valid_v4 || valid_v3 || valid_v2 {
        // Offset 152..216: target_playback_id[64], Offset 216..280: target_capture_id[64]
        if bytes[152] != 0 {
            copy_c_field(&mut st.target_playback_id, &bytes[152..216]);
        }
        if bytes[216] != 0 {
            copy_c_field(&mut st.target_capture_id, &bytes[216..280]);
        }
    }

    if valid_v6 || valid_v5 || valid_v4 || valid_v3 {
        // Offset 280..284: reserved_0, Offset 284..288: reserved_1
        // Offset 288..384: LasapiParamEqExt (96 bytes)
        for i in 0..LASAPI_EQ_BANDS {
            st.param_eq.band_freq_hz[i] = read_f32(bytes, 288 + i * 4);
            st.param_eq.band_type[i] = read_u32(bytes, 328 + i * 4);
        }
        st.param_eq.crossfeed_enabled = read_u32(bytes, 368);
        st.param_eq.crossfeed_level = read_f32(bytes, 372);
        st.param_eq.crossfeed_cutoff_hz = read_u32(bytes, 376);
        st.param_eq.reserved = read_u32(bytes, 380);
        clamp_param_eq_ext(&mut st.param_eq);

        // Offset 384..432: LasapiMicApoCfg (48 bytes)
        st.mic_apo.enabled = read_u32(bytes, 384);
        st.mic_apo.hpf_enabled = read_u32(bytes, 388);
        st.mic_apo.hpf_hz = read_f32(bytes, 392);
        st.mic_apo.gate_enabled = read_u32(bytes, 396);
        st.mic_apo.gate_thresh_db = read_f32(bytes, 400);
        st.mic_apo.rnnoise_enabled = read_u32(bytes, 404);
        st.mic_apo.comp_enabled = read_u32(bytes, 408);
        st.mic_apo.comp_thresh_db = read_f32(bytes, 412);
        st.mic_apo.comp_ratio = read_f32(bytes, 416);
        st.mic_apo.clarity_db = read_f32(bytes, 420);
        st.mic_apo.sidetone_enabled = read_u32(bytes, 424);
        st.mic_apo.sidetone_pct = read_u32(bytes, 428);
        clamp_mic_apo_cfg(&mut st.mic_apo);

        // Offset 432..532: LasapiBtCodecCfg (100 bytes)
        st.bt_cfg.preferred_codec = read_u32(bytes, 432);
        if st.bt_cfg.preferred_codec > LASAPI_BT_PREF_SBC {
            st.bt_cfg.preferred_codec = LASAPI_BT_PREF_AUTO;
        }
        st.bt_cfg.ldac_mode = read_u32(bytes, 436);
        if st.bt_cfg.ldac_mode > LASAPI_LDAC_MODE_330 {
            st.bt_cfg.ldac_mode = LASAPI_LDAC_MODE_ADAPTIVE;
        }
        st.bt_cfg.wireless_abr_enabled = read_u32(bytes, 440);
        st.bt_cfg.effective_kbps = read_u32(bytes, 444);
        st.bt_cfg.supported_codecs_mask = read_u32(bytes, 448);
        st.bt_cfg.hfp_sco_enabled = read_u32(bytes, 452);
        st.bt_cfg.hfp_sco_active = read_u32(bytes, 456);
        st.bt_cfg.bt_sco_codec.copy_from_slice(&bytes[460..468]);
        st.bt_cfg.bt_sco_codec[7] = 0;
        st.bt_cfg.neg_reason.copy_from_slice(&bytes[468..532]);
        st.bt_cfg.neg_reason[63] = 0;
    }

    if valid_v6 || valid_v5 || valid_v4 {
        // Offset 532..556: LasapiHwAnalogCfg (24 bytes)
        st.hw_analog_cfg.hw_dac_unity_vol = read_u32(bytes, 532);
        st.hw_analog_cfg.hp_amp_tier = read_u32(bytes, 536);
        st.hw_analog_cfg.output_route = read_u32(bytes, 540);
        st.hw_analog_cfg.unlock_0db_rails = read_u32(bytes, 544);
        st.hw_analog_cfg.analog_boost_db = read_f32(bytes, 548);
        st.hw_analog_cfg.mic_hw_boost_db = read_f32(bytes, 552);
        clamp_hw_analog_cfg(&mut st.hw_analog_cfg);
    }

    if valid_v6 || valid_v5 {
        let reserved_0 = read_u32(bytes, 280);
        let d_vol = read_u32(bytes, 556);
        let d_muted = read_u32(bytes, 560);
        let m_vol = read_u32(bytes, 564);
        let m_muted = read_u32(bytes, 568);
        let m_gain = read_u32(bytes, 572);

        st.desktop_volume_pct = if d_vol <= 150 { d_vol } else { 100 };
        st.desktop_muted = u32::from(d_muted != 0);
        st.desktop_mic_volume_pct = if m_vol <= 150 { m_vol } else { 100 };
        st.desktop_mic_muted = u32::from(m_muted != 0);
        st.desktop_mic_gain_ppm = if (100_000..=4_000_000).contains(&m_gain) {
            m_gain
        } else {
            1_000_000
        };
        st.cap_sample_rate = if (44_100..=192_000).contains(&reserved_0) {
            reserved_0
        } else {
            0
        };
        st.has_saved_vol_state = true;
    }

    if valid_v6 {
        st.bt_optional_mode = u32::from(read_u32(bytes, 576) != 0);
    }

    *out = st;
    true
}

pub fn encode_persisted_state_v6(st: &SalsaRuntimeState) -> [u8; PERSISTED_V6_SIZE] {
    let mut out = [0u8; PERSISTED_V6_SIZE];
    let v5 = encode_persisted_state_v5(st);
    out[..PERSISTED_V5_SIZE].copy_from_slice(&v5);
    write_u32(&mut out, 0, MAGIC_V6);
    write_u32(&mut out, 576, u32::from(st.bt_optional_mode != 0));
    out
}

pub fn encode_persisted_state_v5(st: &SalsaRuntimeState) -> [u8; PERSISTED_V5_SIZE] {
    let mut out = [0u8; PERSISTED_V5_SIZE];

    write_u32(&mut out, 0, MAGIC_V5);
    write_u32(&mut out, 4, st.sample_rate);
    write_u32(&mut out, 8, st.period_frames);
    let fb = if matches!(st.format_bits, 16 | 24 | 32) {
        st.format_bits
    } else {
        32
    };
    write_u32(&mut out, 12, fb);

    write_u32(&mut out, 16, st.eq_cfg.enabled);
    write_u32(&mut out, 20, st.eq_cfg.dynamic_mode);
    write_f32(&mut out, 24, st.eq_cfg.preamp_db);
    write_f32(&mut out, 28, st.eq_cfg.dyn_threshold_db);
    write_f32(&mut out, 32, st.eq_cfg.dyn_ratio);
    write_u32(&mut out, 36, st.eq_cfg.limiter_enabled);
    for i in 0..LASAPI_EQ_BANDS {
        write_f32(&mut out, 40 + i * 4, st.eq_cfg.band_gains_db[i]);
        write_f32(&mut out, 80 + i * 4, st.eq_cfg.band_q[i]);
    }
    out[120..152].copy_from_slice(&st.eq_cfg.preset_name);
    out[151] = 0;

    if st.target_playback_id[0] != 0 {
        copy_c_field(&mut out[152..216], &st.target_playback_id);
    } else {
        copy_str_to_c(&mut out[152..216], "auto");
    }
    if st.target_capture_id[0] != 0 {
        copy_c_field(&mut out[216..280], &st.target_capture_id);
    } else {
        copy_str_to_c(&mut out[216..280], "auto");
    }

    let r0 = if (44_100..=192_000).contains(&st.cap_sample_rate) {
        st.cap_sample_rate
    } else {
        0
    };
    write_u32(&mut out, 280, r0);
    write_u32(&mut out, 284, 0);

    for i in 0..LASAPI_EQ_BANDS {
        write_f32(&mut out, 288 + i * 4, st.param_eq.band_freq_hz[i]);
        write_u32(&mut out, 328 + i * 4, st.param_eq.band_type[i]);
    }
    write_u32(&mut out, 368, st.param_eq.crossfeed_enabled);
    write_f32(&mut out, 372, st.param_eq.crossfeed_level);
    write_u32(&mut out, 376, st.param_eq.crossfeed_cutoff_hz);
    write_u32(&mut out, 380, st.param_eq.reserved);

    write_u32(&mut out, 384, st.mic_apo.enabled);
    write_u32(&mut out, 388, st.mic_apo.hpf_enabled);
    write_f32(&mut out, 392, st.mic_apo.hpf_hz);
    write_u32(&mut out, 396, st.mic_apo.gate_enabled);
    write_f32(&mut out, 400, st.mic_apo.gate_thresh_db);
    write_u32(&mut out, 404, st.mic_apo.rnnoise_enabled);
    write_u32(&mut out, 408, st.mic_apo.comp_enabled);
    write_f32(&mut out, 412, st.mic_apo.comp_thresh_db);
    write_f32(&mut out, 416, st.mic_apo.comp_ratio);
    write_f32(&mut out, 420, st.mic_apo.clarity_db);
    write_u32(&mut out, 424, st.mic_apo.sidetone_enabled);
    write_u32(&mut out, 428, st.mic_apo.sidetone_pct);

    write_u32(&mut out, 432, st.bt_cfg.preferred_codec);
    write_u32(&mut out, 436, st.bt_cfg.ldac_mode);
    write_u32(&mut out, 440, st.bt_cfg.wireless_abr_enabled);
    write_u32(&mut out, 444, st.bt_cfg.effective_kbps);
    write_u32(&mut out, 448, st.bt_cfg.supported_codecs_mask);
    write_u32(&mut out, 452, st.bt_cfg.hfp_sco_enabled);
    write_u32(&mut out, 456, st.bt_cfg.hfp_sco_active);
    out[460..468].copy_from_slice(&st.bt_cfg.bt_sco_codec);
    out[468..532].copy_from_slice(&st.bt_cfg.neg_reason);

    write_u32(&mut out, 532, st.hw_analog_cfg.hw_dac_unity_vol);
    write_u32(&mut out, 536, st.hw_analog_cfg.hp_amp_tier);
    write_u32(&mut out, 540, st.hw_analog_cfg.output_route);
    write_u32(&mut out, 544, st.hw_analog_cfg.unlock_0db_rails);
    write_f32(&mut out, 548, st.hw_analog_cfg.analog_boost_db);
    write_f32(&mut out, 552, st.hw_analog_cfg.mic_hw_boost_db);

    write_u32(&mut out, 556, st.desktop_volume_pct);
    write_u32(&mut out, 560, st.desktop_muted);
    write_u32(&mut out, 564, st.desktop_mic_volume_pct);
    write_u32(&mut out, 568, st.desktop_mic_muted);
    write_u32(&mut out, 572, st.desktop_mic_gain_ppm);

    out
}

pub fn config_dir_path() -> PathBuf {
    if let Ok(cfg) = std::env::var("XDG_CONFIG_HOME") {
        if cfg.starts_with('/') {
            return PathBuf::from(cfg).join("lasapi");
        }
    }
    PathBuf::from(user_home_dir()).join(".config").join("lasapi")
}

pub fn salsa_settings_file_path() -> PathBuf {
    config_dir_path().join(SALSA_SETTINGS_FILENAME)
}

pub fn state_dir_and_file() -> (PathBuf, PathBuf) {
    let dir = config_dir_path();
    let file = dir.join("salsa_state.bin");
    (dir, file)
}

struct SettingsLockGuard(Option<fs::File>);

impl Drop for SettingsLockGuard {
    fn drop(&mut self) {
        if let Some(ref f) = self.0 {
            funlock_fd(f.as_raw_fd());
        }
    }
}

fn acquire_settings_lock_in(dir: &Path) -> Result<SettingsLockGuard, String> {
    ensure_dir_0700_safe(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let lock_path = dir.join(".salsa_settings.lock");
    if let Ok(meta) = fs::symlink_metadata(&lock_path) {
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(format!("refusing unsafe lock file: {}", lock_path.display()));
        }
    }
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW | O_CLOEXEC)
        .open(&lock_path)
        .map_err(|e| format!("open lock {}: {e}", lock_path.display()))?;
    let _ = f.set_permissions(Permissions::from_mode(0o600));
    flock_fd(f.as_raw_fd(), false).map_err(|e| format!("flock {}: {e}", lock_path.display()))?;
    Ok(SettingsLockGuard(Some(f)))
}

pub fn apply_json_to_eq_cfg_pair(
    eq_obj: &JsonValue,
    cfg: &mut LasapiEqConfig,
    ext: &mut LasapiParamEqExt,
) {
    let _ = apply_json_to_eq_pair(eq_obj, cfg, ext);
}

/// Merges authoritative runtime audio/hardware sections into a unified settings JSON object,
/// preserving unrelated GUI preference keys (theme, shortcuts, imported AutoEQ presets, etc.).
pub fn merge_runtime_state_into_json(root: &mut JsonValue, st: &SalsaRuntimeState) {
    if !matches!(root, JsonValue::Object(_)) {
        *root = JsonValue::Object(Vec::new());
    }
    root.set("schema_version", JsonValue::Int(SALSA_SETTINGS_SCHEMA_VERSION));

    let sr = if (44_100..=192_000).contains(&st.sample_rate) {
        st.sample_rate
    } else {
        48_000
    };
    let pf = if (64..=2048).contains(&st.period_frames) {
        st.period_frames
    } else {
        256
    };
    let fb = if matches!(st.format_bits, 16 | 24 | 32) {
        st.format_bits
    } else {
        32
    };
    let cap_sr = if (44_100..=192_000).contains(&st.cap_sample_rate) {
        st.cap_sample_rate
    } else {
        0
    };
    let pb_raw = sanitize_declarative_str(&c_str_to_string(&st.target_playback_id), 63);
    let cap_raw = sanitize_declarative_str(&c_str_to_string(&st.target_capture_id), 63);
    let pb_id = if pb_raw.is_empty() { "auto".to_string() } else { pb_raw };
    let cap_id = if cap_raw.is_empty() { "auto".to_string() } else { cap_raw };

    root.set(
        "engine",
        JsonValue::obj(vec![
            ("sample_rate", JsonValue::Int(i64::from(sr))),
            ("period_frames", JsonValue::Int(i64::from(pf))),
            ("format_bits", JsonValue::Int(i64::from(fb))),
            ("capture_sample_rate", JsonValue::Int(i64::from(cap_sr))),
            ("target_playback_id", JsonValue::Str(pb_id)),
            ("target_capture_id", JsonValue::Str(cap_id)),
        ]),
    );

    let d_vol = st.desktop_volume_pct.min(150);
    let m_vol = st.desktop_mic_volume_pct.min(150);
    let m_gain = st.desktop_mic_gain_ppm.clamp(100_000, 4_000_000);
    root.set(
        "volumes",
        JsonValue::obj(vec![
            ("master_pct", JsonValue::Int(i64::from(d_vol))),
            ("master_muted", JsonValue::Bool(st.desktop_muted != 0)),
            ("mic_pct", JsonValue::Int(i64::from(m_vol))),
            ("mic_muted", JsonValue::Bool(st.desktop_mic_muted != 0)),
            ("mic_gain_ppm", JsonValue::Int(i64::from(m_gain))),
        ]),
    );

    let mut hwa = st.hw_analog_cfg;
    clamp_hw_analog_cfg(&mut hwa);
    root.set(
        "hw_analog",
        JsonValue::obj(vec![
            ("hw_dac_unity_vol", JsonValue::Bool(hwa.hw_dac_unity_vol != 0)),
            ("hp_amp_tier", JsonValue::Int(i64::from(hwa.hp_amp_tier))),
            ("output_route", JsonValue::Int(i64::from(hwa.output_route))),
            ("unlock_0db_rails", JsonValue::Bool(hwa.unlock_0db_rails != 0)),
            ("analog_boost_db", JsonValue::Float(round_decimals(hwa.analog_boost_db, 10.0))),
            ("mic_hw_boost_db", JsonValue::Float(round_decimals(hwa.mic_hw_boost_db, 10.0))),
        ]),
    );

    let pref_codec = if st.bt_cfg.preferred_codec <= LASAPI_BT_PREF_SBC {
        st.bt_cfg.preferred_codec
    } else {
        LASAPI_BT_PREF_AUTO
    };
    let ldac_mode = if st.bt_cfg.ldac_mode <= LASAPI_LDAC_MODE_330 {
        st.bt_cfg.ldac_mode
    } else {
        LASAPI_LDAC_MODE_ADAPTIVE
    };
    root.set(
        "bluetooth",
        JsonValue::obj(vec![
            ("preferred_codec", JsonValue::Int(i64::from(pref_codec))),
            ("ldac_mode", JsonValue::Int(i64::from(ldac_mode))),
            ("wireless_abr_enabled", JsonValue::Bool(st.bt_cfg.wireless_abr_enabled != 0)),
            ("hfp_sco_enabled", JsonValue::Bool(st.bt_cfg.hfp_sco_enabled != 0)),
            ("optional_mode", JsonValue::Bool(st.bt_optional_mode != 0)),
        ]),
    );

    let mut mic = st.mic_apo;
    clamp_mic_apo_cfg(&mut mic);
    root.set(
        "mic_apo",
        JsonValue::obj(vec![
            ("enabled", JsonValue::Bool(mic.enabled != 0)),
            ("hpf_enabled", JsonValue::Bool(mic.hpf_enabled != 0)),
            ("hpf_hz", JsonValue::Float(round_decimals(mic.hpf_hz, 10.0))),
            ("gate_enabled", JsonValue::Bool(mic.gate_enabled != 0)),
            ("gate_thresh_db", JsonValue::Float(round_decimals(mic.gate_thresh_db, 10.0))),
            ("rnnoise_enabled", JsonValue::Bool(mic.rnnoise_enabled != 0)),
            ("comp_enabled", JsonValue::Bool(mic.comp_enabled != 0)),
            ("comp_thresh_db", JsonValue::Float(round_decimals(mic.comp_thresh_db, 10.0))),
            ("comp_ratio", JsonValue::Float(round_decimals(mic.comp_ratio, 10.0))),
            ("clarity_db", JsonValue::Float(round_decimals(mic.clarity_db, 10.0))),
            ("sidetone_enabled", JsonValue::Bool(mic.sidetone_enabled != 0)),
            ("sidetone_pct", JsonValue::Int(i64::from(mic.sidetone_pct))),
        ]),
    );

    let mut eq = st.eq_cfg;
    let mut ext = st.param_eq;
    clamp_eq_cfg(&mut eq);
    clamp_param_eq_ext(&mut ext);
    root.set("eq", eq_cfg_to_json_value(&eq, &ext));

    // Drop legacy conflicting duplicate keys once migrated into canonical sections.
    for old_key in [
        "eq_config",
        "eq_preset",
        "eq_dynamic_mode",
        "rate",
        "period",
        "bits",
        "capture_sample_rate",
    ] {
        root.remove(old_key);
    }
}

/// Extracts and validates a `SalsaRuntimeState` from the unified JSON object,
/// clamping every field and falling back safely to defaults on missing or invalid entries.
pub fn apply_json_to_runtime_state(root: &JsonValue, out: &mut SalsaRuntimeState) {
    let mut st = default_runtime_state();

    if let Some(eng) = root.get("engine") {
        if let Some(sr) = eng.get("sample_rate").and_then(JsonValue::as_u64) {
            let sr = sr as u32;
            if (44_100..=192_000).contains(&sr) {
                st.sample_rate = sr;
            }
        }
        if let Some(pf) = eng.get("period_frames").and_then(JsonValue::as_u64) {
            let pf = pf as u32;
            if (64..=2048).contains(&pf) {
                st.period_frames = pf;
            }
        }
        if let Some(fb) = eng.get("format_bits").and_then(JsonValue::as_u64) {
            let fb = fb as u32;
            if matches!(fb, 16 | 24 | 32) {
                st.format_bits = fb;
            }
        }
        if let Some(cs) = eng.get("capture_sample_rate").and_then(JsonValue::as_u64) {
            let cs = cs as u32;
            if cs == 0 || (44_100..=192_000).contains(&cs) {
                st.cap_sample_rate = cs;
            }
        }
        if let Some(pb) = eng.get("target_playback_id").and_then(JsonValue::as_str) {
            let clean = sanitize_declarative_str(pb, 63);
            if !clean.is_empty() {
                copy_str_to_c(&mut st.target_playback_id, &clean);
            }
        }
        if let Some(cap) = eng.get("target_capture_id").and_then(JsonValue::as_str) {
            let clean = sanitize_declarative_str(cap, 63);
            if !clean.is_empty() {
                copy_str_to_c(&mut st.target_capture_id, &clean);
            }
        }
    } else {
        // Legacy gui_settings.json migration fallback
        if let Some(sr) = root.get("rate").and_then(JsonValue::as_u64) {
            let sr = sr as u32;
            if (44_100..=192_000).contains(&sr) {
                st.sample_rate = sr;
            }
        }
        if let Some(pf) = root.get("period").and_then(JsonValue::as_u64) {
            let pf = pf as u32;
            if (64..=2048).contains(&pf) {
                st.period_frames = pf;
            }
        }
        if let Some(fb) = root.get("bits").and_then(JsonValue::as_u64) {
            let fb = fb as u32;
            if matches!(fb, 16 | 24 | 32) {
                st.format_bits = fb;
            }
        }
        if let Some(cs) = root.get("capture_sample_rate").and_then(JsonValue::as_u64) {
            let cs = cs as u32;
            if cs == 0 || (44_100..=192_000).contains(&cs) {
                st.cap_sample_rate = cs;
            }
        }
    }

    if let Some(vol) = root.get("volumes") {
        if let Some(dv) = vol.get("master_pct").and_then(JsonValue::as_u64) {
            if dv <= 150 {
                st.desktop_volume_pct = dv as u32;
            }
        }
        st.desktop_muted = u32::from(json_bool(vol.get("master_muted"), false));
        if let Some(mv) = vol.get("mic_pct").and_then(JsonValue::as_u64) {
            if mv <= 150 {
                st.desktop_mic_volume_pct = mv as u32;
            }
        }
        st.desktop_mic_muted = u32::from(json_bool(vol.get("mic_muted"), false));
        if let Some(mg) = vol.get("mic_gain_ppm").and_then(JsonValue::as_u64) {
            if (100_000..=4_000_000).contains(&mg) {
                st.desktop_mic_gain_ppm = mg as u32;
            }
        }
        st.has_saved_vol_state = true;
    }

    if let Some(hwa) = root.get("hw_analog") {
        st.hw_analog_cfg.hw_dac_unity_vol = u32::from(json_bool(hwa.get("hw_dac_unity_vol"), true));
        if let Some(tier) = hwa.get("hp_amp_tier").and_then(JsonValue::as_u64) {
            st.hw_analog_cfg.hp_amp_tier = tier as u32;
        }
        if let Some(route) = hwa.get("output_route").and_then(JsonValue::as_u64) {
            st.hw_analog_cfg.output_route = route as u32;
        }
        st.hw_analog_cfg.unlock_0db_rails = u32::from(json_bool(hwa.get("unlock_0db_rails"), false));
        if let Some(b) = hwa.get("analog_boost_db").and_then(JsonValue::as_f64) {
            st.hw_analog_cfg.analog_boost_db = b as f32;
        }
        if let Some(mb) = hwa.get("mic_hw_boost_db").and_then(JsonValue::as_f64) {
            st.hw_analog_cfg.mic_hw_boost_db = mb as f32;
        }
        clamp_hw_analog_cfg(&mut st.hw_analog_cfg);
    }

    if let Some(bt) = root.get("bluetooth") {
        if let Some(c) = bt.get("preferred_codec").and_then(JsonValue::as_u64) {
            let c = c as u32;
            st.bt_cfg.preferred_codec = if c <= LASAPI_BT_PREF_SBC {
                c
            } else {
                LASAPI_BT_PREF_AUTO
            };
        }
        if let Some(m) = bt.get("ldac_mode").and_then(JsonValue::as_u64) {
            let m = m as u32;
            st.bt_cfg.ldac_mode = if m <= LASAPI_LDAC_MODE_330 {
                m
            } else {
                LASAPI_LDAC_MODE_ADAPTIVE
            };
        }
        st.bt_cfg.wireless_abr_enabled = u32::from(json_bool(bt.get("wireless_abr_enabled"), true));
        st.bt_cfg.hfp_sco_enabled = u32::from(json_bool(bt.get("hfp_sco_enabled"), false));
        st.bt_optional_mode = u32::from(json_bool(bt.get("optional_mode"), true));
    }

    if let Some(mic) = root.get("mic_apo") {
        st.mic_apo.enabled = u32::from(json_bool(mic.get("enabled"), false));
        st.mic_apo.hpf_enabled = u32::from(json_bool(mic.get("hpf_enabled"), true));
        if let Some(v) = mic.get("hpf_hz").and_then(JsonValue::as_f64) {
            st.mic_apo.hpf_hz = v as f32;
        }
        st.mic_apo.gate_enabled = u32::from(json_bool(mic.get("gate_enabled"), true));
        if let Some(v) = mic.get("gate_thresh_db").and_then(JsonValue::as_f64) {
            st.mic_apo.gate_thresh_db = v as f32;
        }
        st.mic_apo.rnnoise_enabled = u32::from(json_bool(mic.get("rnnoise_enabled"), true));
        st.mic_apo.comp_enabled = u32::from(json_bool(mic.get("comp_enabled"), true));
        if let Some(v) = mic.get("comp_thresh_db").and_then(JsonValue::as_f64) {
            st.mic_apo.comp_thresh_db = v as f32;
        }
        if let Some(v) = mic.get("comp_ratio").and_then(JsonValue::as_f64) {
            st.mic_apo.comp_ratio = v as f32;
        }
        if let Some(v) = mic.get("clarity_db").and_then(JsonValue::as_f64) {
            st.mic_apo.clarity_db = v as f32;
        }
        st.mic_apo.sidetone_enabled = u32::from(json_bool(mic.get("sidetone_enabled"), false));
        if let Some(v) = mic.get("sidetone_pct").and_then(JsonValue::as_u64) {
            st.mic_apo.sidetone_pct = v as u32;
        }
        clamp_mic_apo_cfg(&mut st.mic_apo);
    }

    if let Some(eq_obj) = root.get("eq").or_else(|| root.get("eq_config")) {
        apply_json_to_eq_cfg_pair(eq_obj, &mut st.eq_cfg, &mut st.param_eq);
    }

    *out = st;
}

fn backup_legacy_file_once(src: &Path, bak: &Path) {
    if fs::symlink_metadata(bak).is_ok() {
        return;
    }
    let Ok(meta) = fs::symlink_metadata(src) else {
        return;
    };
    if meta.file_type().is_symlink() || !meta.file_type().is_file() || meta.len() > MAX_SETTINGS_JSON_BYTES as u64 {
        return;
    }
    let Ok(f) = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_CLOEXEC)
        .open(src)
    else {
        return;
    };
    let mut bytes = Vec::new();
    if f.take((MAX_SETTINGS_JSON_BYTES as u64) + 1).read_to_end(&mut bytes).is_ok()
        && bytes.len() <= MAX_SETTINGS_JSON_BYTES
    {
        let _ = write_atomic_secure_file(bak, &bytes, 0o600);
    }
}

fn write_json_settings_unlocked(path: &Path, data: &JsonValue) -> Result<(), String> {
    let JsonValue::Object(_) = data else {
        return Err("salsa_settings.json root must be a JSON object".into());
    };
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(format!("refusing symlink at {}", path.display()));
        }
    }
    let mut text = data.to_json_pretty(2);
    text.push('\n');
    write_atomic_secure_file(path, text.as_bytes(), 0o600)
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn read_or_migrate_settings_unlocked(dir: &Path) -> Result<Option<JsonValue>, String> {
    let target = dir.join(SALSA_SETTINGS_FILENAME);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            let Some(raw) = read_verified_regular_file_utf8(&target, MAX_SETTINGS_JSON_BYTES, false)
                .map_err(|e| format!("read {}: {e}", target.display()))?
            else {
                return Ok(None);
            };
            let parsed = JsonValue::parse(&raw)
                .map_err(|e| format!("parse {}: {e}", target.display()))?;
            if !matches!(parsed, JsonValue::Object(_)) {
                return Err(format!("{}: expected JSON object", target.display()));
            }
            return Ok(Some(parsed));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("stat {}: {e}", target.display())),
    }

    let legacy_gui = dir.join("gui_settings.json");
    let legacy_bin = dir.join("salsa_state.bin");

    let gui_obj = read_verified_regular_file_utf8(&legacy_gui, MAX_SETTINGS_JSON_BYTES, false)
        .ok()
        .flatten()
        .and_then(|s| JsonValue::parse(&s).ok())
        .filter(|v| matches!(v, JsonValue::Object(_)));

    let mut bin_st = default_runtime_state();
    let has_bin = load_state_from_path(&legacy_bin, &mut bin_st);

    if gui_obj.is_none() && !has_bin {
        return Ok(None);
    }

    backup_legacy_file_once(&legacy_gui, &dir.join("gui_settings.json.bak"));
    backup_legacy_file_once(&legacy_bin, &dir.join("salsa_state.bin.bak"));

    let mut merged = gui_obj.clone().unwrap_or_else(|| JsonValue::Object(Vec::new()));
    let mut st = default_runtime_state();
    if let Some(ref gj) = gui_obj {
        apply_json_to_runtime_state(gj, &mut st);
    }
    if has_bin {
        st = bin_st;
    }
    merge_runtime_state_into_json(&mut merged, &st);
    write_json_settings_unlocked(&target, &merged)?;
    Ok(Some(merged))
}

/// Loads the unified `salsa_settings.json` inside `dir` (migrating legacy files if present).
/// Rejects symlinks, unowned files, or unsafe permissions cleanly by returning an empty object.
pub fn load_unified_settings_json_in(dir: &Path) -> JsonValue {
    let target = dir.join(SALSA_SETTINGS_FILENAME);
    if let Ok(meta) = fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return JsonValue::Object(Vec::new());
        }
        if let Ok(Some(raw)) = read_verified_regular_file_utf8(&target, MAX_SETTINGS_JSON_BYTES, false) {
            if let Ok(obj @ JsonValue::Object(_)) = JsonValue::parse(&raw) {
                return obj;
            }
        }
        return JsonValue::Object(Vec::new());
    }
    let Ok(_lock) = acquire_settings_lock_in(dir) else {
        return JsonValue::Object(Vec::new());
    };
    read_or_migrate_settings_unlocked(dir)
        .ok()
        .flatten()
        .unwrap_or_else(|| JsonValue::Object(Vec::new()))
}

pub fn save_unified_settings_json_in(
    dir: &Path,
    data: &JsonValue,
    preserve_existing: bool,
) -> Result<(), String> {
    let Some(new_entries) = data.as_object() else {
        return Err("settings must be a JSON object".into());
    };
    let _lock = acquire_settings_lock_in(dir)?;
    let target = dir.join(SALSA_SETTINGS_FILENAME);
    let mut out = if preserve_existing {
        read_or_migrate_settings_unlocked(dir)?.unwrap_or_else(|| JsonValue::Object(Vec::new()))
    } else {
        JsonValue::Object(Vec::new())
    };
    if preserve_existing {
        for (k, v) in new_entries {
            if !k.starts_with("tray_") || out.get(k).is_none() {
                out.set(k.as_str(), v.clone());
            }
        }
    } else {
        out = data.clone();
    }
    write_json_settings_unlocked(&target, &out)
}

pub fn update_unified_settings_json_in(
    dir: &Path,
    updates: &[(&str, JsonValue)],
) -> Result<(), String> {
    let _lock = acquire_settings_lock_in(dir)?;
    let target = dir.join(SALSA_SETTINGS_FILENAME);
    let mut cur =
        read_or_migrate_settings_unlocked(dir)?.unwrap_or_else(|| JsonValue::Object(Vec::new()));
    for (k, v) in updates {
        cur.set(*k, v.clone());
    }
    write_json_settings_unlocked(&target, &cur)
}

pub fn load_runtime_state_in_dir(dir: &Path, create_default_if_missing: bool) -> SalsaRuntimeState {
    let mut st = default_runtime_state();
    let Ok(_lock) = acquire_settings_lock_in(dir) else {
        return st;
    };
    match read_or_migrate_settings_unlocked(dir) {
        Ok(Some(json)) => apply_json_to_runtime_state(&json, &mut st),
        Ok(None) if create_default_if_missing => {
            let mut doc = JsonValue::Object(Vec::new());
            merge_runtime_state_into_json(&mut doc, &st);
            let _ = write_json_settings_unlocked(&dir.join(SALSA_SETTINGS_FILENAME), &doc);
        }
        _ => {}
    }
    st
}

pub fn save_runtime_state_in_dir(dir: &Path, st: &SalsaRuntimeState) -> bool {
    let Ok(_lock) = acquire_settings_lock_in(dir) else {
        return false;
    };
    let target = dir.join(SALSA_SETTINGS_FILENAME);
    let mut doc = match read_or_migrate_settings_unlocked(dir) {
        Ok(Some(existing)) => existing,
        Ok(None) => JsonValue::Object(Vec::new()),
        Err(_) => return false,
    };
    merge_runtime_state_into_json(&mut doc, st);
    write_json_settings_unlocked(&target, &doc).is_ok()
}

pub fn load_state_from_path(path: &Path, out: &mut SalsaRuntimeState) -> bool {
    *out = default_runtime_state();
    let Ok(f) = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_CLOEXEC)
        .open(path)
    else {
        return false;
    };
    let Ok(meta) = f.metadata() else {
        return false;
    };
    let max_len = (PERSISTED_V6_SIZE as u64) * 2;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > max_len {
        return false;
    }
    let mut buf = Vec::with_capacity(PERSISTED_V6_SIZE);
    let mut limited = f.take((PERSISTED_V6_SIZE as u64) + 1);
    if limited.read_to_end(&mut buf).is_err() || buf.len() > PERSISTED_V6_SIZE {
        return false;
    }
    decode_persisted_state_bytes(&buf, out)
}

pub fn save_state_to_path(dir: &Path, path: &Path, st: &SalsaRuntimeState) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let _ = fs::set_permissions(dir, Permissions::from_mode(0o700));
    if !is_safe_owner_dir(dir, posix_owner_id(), false) {
        return false;
    }
    let tmp = PathBuf::from(format!("{}.tmp.{}", path.display(), current_pid()));
    let _ = fs::remove_file(&tmp);

    let Ok(mut f) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW | O_CLOEXEC)
        .open(&tmp)
    else {
        return false;
    };

    let bytes = encode_persisted_state_v6(st);
    let ok = f.write_all(&bytes).is_ok() && f.sync_all().is_ok();
    drop(f);
    if !ok {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    true
}

pub fn load_runtime_state(io_st: &mut SalsaRuntimeState) {
    *io_st = load_runtime_state_in_dir(&config_dir_path(), true);
}

pub fn save_runtime_state(st: &SalsaRuntimeState) -> bool {
    save_runtime_state_in_dir(&config_dir_path(), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_abi::LasapiHwAnalogCfg;
    use core::mem::size_of;
    use std::os::unix::fs::symlink;

    #[repr(C)]
    struct CPersistedV1 {
        magic: u32,
        sample_rate: u32,
        period_frames: u32,
        format_bits: u32,
        eq_cfg: LasapiEqConfig,
    }

    #[repr(C)]
    struct CPersistedV2 {
        v1: CPersistedV1,
        target_playback_id: [u8; 64],
        target_capture_id: [u8; 64],
    }

    #[repr(C)]
    struct CPersistedV3 {
        v2: CPersistedV2,
        reserved_0: u32,
        reserved_1: u32,
        param_eq: LasapiParamEqExt,
        mic_apo: LasapiMicApoCfg,
        bt_cfg: LasapiBtCodecCfg,
    }

    #[repr(C)]
    struct CPersistedV4 {
        v3: CPersistedV3,
        hw_analog_cfg: LasapiHwAnalogCfg,
    }

    #[repr(C)]
    struct CPersistedV5 {
        v4: CPersistedV4,
        desktop_volume_pct: u32,
        desktop_muted: u32,
        desktop_mic_volume_pct: u32,
        desktop_mic_muted: u32,
        desktop_mic_gain_ppm: u32,
    }

    #[test]
    fn persisted_struct_sizes_match_c() {
        assert_eq!(size_of::<CPersistedV1>(), PERSISTED_V1_SIZE);
        assert_eq!(size_of::<CPersistedV2>(), PERSISTED_V2_SIZE);
        assert_eq!(size_of::<CPersistedV3>(), PERSISTED_V3_SIZE);
        assert_eq!(size_of::<CPersistedV4>(), PERSISTED_V4_SIZE);
        assert_eq!(size_of::<CPersistedV5>(), PERSISTED_V5_SIZE);
    }

    #[test]
    fn v5_encode_decode_roundtrip() {
        let mut st = default_runtime_state();
        st.sample_rate = 192_000;
        st.period_frames = 512;
        st.format_bits = 24;
        st.cap_sample_rate = 96_000;
        copy_str_to_c(&mut st.target_playback_id, "hw:2,0");
        copy_str_to_c(&mut st.target_capture_id, "hw:1,0");
        st.eq_cfg.band_gains_db[2] = -3.5;
        st.eq_cfg.band_q[4] = 2.0;
        st.param_eq.crossfeed_enabled = 1;
        st.param_eq.crossfeed_level = 0.72;
        st.mic_apo.enabled = 1;
        st.mic_apo.clarity_db = 4.25;
        st.bt_cfg.preferred_codec = 1;
        st.hw_analog_cfg.analog_boost_db = 1.5;
        st.desktop_volume_pct = 84;
        st.desktop_muted = 1;
        st.desktop_mic_volume_pct = 115;
        st.desktop_mic_muted = 0;
        st.desktop_mic_gain_ppm = 1_500_000;
        st.has_saved_vol_state = true;

        let raw = encode_persisted_state_v5(&st);
        let mut loaded = default_runtime_state();
        assert!(decode_persisted_state_bytes(&raw, &mut loaded));

        assert_eq!(loaded.sample_rate, 192_000);
        assert_eq!(loaded.period_frames, 512);
        assert_eq!(loaded.format_bits, 24);
        assert_eq!(loaded.cap_sample_rate, 96_000);
        assert_eq!(&loaded.target_playback_id[..6], b"hw:2,0");
        assert_eq!(&loaded.target_capture_id[..6], b"hw:1,0");
        assert!((loaded.eq_cfg.band_gains_db[2] - (-3.5)).abs() < 1e-6);
        assert!((loaded.eq_cfg.band_q[4] - 2.0).abs() < 1e-6);
        assert_eq!(loaded.param_eq.crossfeed_enabled, 1);
        assert!((loaded.mic_apo.clarity_db - 4.25).abs() < 1e-6);
        assert_eq!(loaded.bt_cfg.preferred_codec, 1);
        assert!((loaded.hw_analog_cfg.analog_boost_db - 1.5).abs() < 1e-6);
        assert_eq!(loaded.desktop_volume_pct, 84);
        assert_eq!(loaded.desktop_muted, 1);
        assert_eq!(loaded.desktop_mic_volume_pct, 115);
        assert_eq!(loaded.desktop_mic_gain_ppm, 1_500_000);
        assert!(loaded.has_saved_vol_state);
        assert_eq!(loaded.bt_optional_mode, 1);

        st.bt_optional_mode = 0;
        let raw6 = encode_persisted_state_v6(&st);
        let mut loaded6 = default_runtime_state();
        assert!(decode_persisted_state_bytes(&raw6, &mut loaded6));
        assert_eq!(loaded6.bt_optional_mode, 0);
        st.bt_optional_mode = 1;
        let raw6_opt = encode_persisted_state_v6(&st);
        assert!(decode_persisted_state_bytes(&raw6_opt, &mut loaded6));
        assert_eq!(loaded6.bt_optional_mode, 1);
    }

    #[test]
    fn v1_and_v3_legacy_decode_and_clamp() {
        let mut v5 = encode_persisted_state_v5(&default_runtime_state());
        write_u32(&mut v5, 0, MAGIC_V1);
        write_u32(&mut v5, 4, 96_000);
        write_f32(&mut v5, 80, f32::NAN); // corrupt band_q[0] -> 1.414
        let mut loaded = default_runtime_state();
        assert!(decode_persisted_state_bytes(&v5[..PERSISTED_V1_SIZE], &mut loaded));
        assert_eq!(loaded.sample_rate, 96_000);
        assert!((loaded.eq_cfg.band_q[0] - 1.414).abs() < 1e-6);
        assert!(!loaded.has_saved_vol_state);

        write_u32(&mut v5, 0, MAGIC_V3);
        write_u32(&mut v5, 432, 99); // bad bt preferred_codec
        write_f32(&mut v5, 372, f32::NAN); // bad crossfeed_level
        write_f32(&mut v5, 392, 999.0); // bad hpf_hz
        assert!(decode_persisted_state_bytes(&v5[..PERSISTED_V3_SIZE], &mut loaded));
        assert_eq!(loaded.bt_cfg.preferred_codec, LASAPI_BT_PREF_AUTO);
        assert!((loaded.param_eq.crossfeed_level - 0.45).abs() < 1e-6);
        assert!((loaded.mic_apo.hpf_hz - 80.0).abs() < 1e-6);
        assert!(!loaded.has_saved_vol_state);
    }

    #[test]
    fn clamp_helpers_sanitize_nan_and_bounds() {
        let mut ext = default_runtime_state().param_eq;
        ext.band_freq_hz[0] = f32::NAN;
        ext.band_freq_hz[1] = 25_000.0;
        ext.band_type[2] = 77;
        ext.crossfeed_enabled = 5;
        ext.crossfeed_level = f32::NEG_INFINITY;
        ext.crossfeed_cutoff_hz = 10;
        clamp_param_eq_ext(&mut ext);
        assert!((ext.band_freq_hz[0] - K_EQ_FREQS[0]).abs() < 1e-6);
        assert!((ext.band_freq_hz[1] - K_EQ_FREQS[1]).abs() < 1e-6);
        assert_eq!(ext.band_type[2], LASAPI_FILTER_PEAK);
        assert_eq!(ext.crossfeed_enabled, 1);
        assert!((ext.crossfeed_level - 0.45).abs() < 1e-6);
        assert_eq!(ext.crossfeed_cutoff_hz, 700);

        let mut mic = default_runtime_state().mic_apo;
        mic.enabled = 42;
        mic.hpf_hz = f32::NAN;
        mic.gate_thresh_db = 0.0;
        mic.comp_thresh_db = -100.0;
        mic.comp_ratio = f32::NAN;
        mic.clarity_db = 99.0;
        mic.sidetone_pct = 250;
        clamp_mic_apo_cfg(&mut mic);
        assert_eq!(mic.enabled, 1);
        assert!((mic.hpf_hz - 80.0).abs() < 1e-6);
        assert!((mic.gate_thresh_db - (-50.0)).abs() < 1e-6);
        assert!((mic.comp_thresh_db - (-15.0)).abs() < 1e-6);
        assert!((mic.comp_ratio - 2.2).abs() < 1e-6);
        assert!((mic.clarity_db - 1.5).abs() < 1e-6);
        assert_eq!(mic.sidetone_pct, 100);

        let mut eq = default_runtime_state().eq_cfg;
        eq.dynamic_mode = 2;
        clamp_eq_cfg(&mut eq);
        assert_eq!(eq.dynamic_mode, 2);
        eq.dynamic_mode = 99;
        clamp_eq_cfg(&mut eq);
        assert_eq!(eq.dynamic_mode, 1);
    }

    #[test]
    fn file_roundtrip_and_security_guards() {
        let base = std::env::temp_dir().join(format!("lasapi_state_test_{}", current_pid()));
        let _ = fs::remove_dir_all(&base);
        let file = base.join("salsa_state.bin");

        let mut st = default_runtime_state();
        st.sample_rate = 88_200;
        assert!(save_state_to_path(&base, &file, &st));

        let mut loaded = default_runtime_state();
        assert!(load_state_from_path(&file, &mut loaded));
        assert_eq!(loaded.sample_rate, 88_200);

        // Reject oversized file (> 2 * V5)
        let big = base.join("oversized.bin");
        fs::write(&big, vec![0u8; PERSISTED_V5_SIZE * 2 + 1]).unwrap();
        assert!(!load_state_from_path(&big, &mut loaded));

        // Reject corrupt magic
        let bad = base.join("corrupt.bin");
        fs::write(&bad, vec![0xAAu8; PERSISTED_V5_SIZE]).unwrap();
        assert!(!load_state_from_path(&bad, &mut loaded));

        // Reject symlink via O_NOFOLLOW
        let link = base.join("link.bin");
        symlink(&file, &link).unwrap();
        assert!(!load_state_from_path(&link, &mut loaded));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn unified_settings_json_migration_roundtrip_and_security() {
        let base = std::env::temp_dir().join(format!("lasapi_unified_test_{}", current_pid()));
        let _ = fs::remove_dir_all(&base);
        ensure_dir_0700_safe(&base).unwrap();

        // Seed legacy salsa_state.bin and gui_settings.json
        let mut legacy_st = default_runtime_state();
        legacy_st.sample_rate = 192_000;
        legacy_st.period_frames = 128;
        legacy_st.hw_analog_cfg.hw_dac_unity_vol = 0;
        legacy_st.hw_analog_cfg.hp_amp_tier = 3;
        legacy_st.hw_analog_cfg.output_route = 2;
        legacy_st.hw_analog_cfg.analog_boost_db = 3.5;
        legacy_st.hw_analog_cfg.mic_hw_boost_db = 9.0;
        assert!(save_state_to_path(&base, &base.join("salsa_state.bin"), &legacy_st));

        let legacy_gui = JsonValue::obj(vec![
            ("theme", JsonValue::Str("CachyOS Teal Void".into())),
            ("font_scale_pct", JsonValue::Int(110)),
            ("eq_preset", JsonValue::Str("Stale".into())),
            ("rate", JsonValue::Int(48_000)),
        ]);
        write_json_settings_unlocked(&base.join("gui_settings.json"), &legacy_gui).unwrap();

        // First load migrates both into salsa_settings.json and creates .bak files
        let migrated = load_runtime_state_in_dir(&base, true);
        assert_eq!(migrated.sample_rate, 192_000);
        assert_eq!(migrated.period_frames, 128);
        assert_eq!(migrated.hw_analog_cfg.hw_dac_unity_vol, 0);
        assert_eq!(migrated.hw_analog_cfg.hp_amp_tier, 3);
        assert_eq!(migrated.hw_analog_cfg.output_route, 2);
        assert!((migrated.hw_analog_cfg.analog_boost_db - 3.5).abs() < 0.05);
        assert!((migrated.hw_analog_cfg.mic_hw_boost_db - 9.0).abs() < 0.05);
        assert!(base.join(SALSA_SETTINGS_FILENAME).is_file());
        assert!(base.join("salsa_state.bin.bak").is_file());
        assert!(base.join("gui_settings.json.bak").is_file());

        // GUI keys preserved and stale duplicates stripped
        let doc = load_unified_settings_json_in(&base);
        assert_eq!(doc.get("theme").and_then(JsonValue::as_str), Some("CachyOS Teal Void"));
        assert_eq!(doc.get("font_scale_pct").and_then(JsonValue::as_i64), Some(110));
        assert!(doc.get("eq_preset").is_none());
        assert!(doc.get("rate").is_none());

        // Engine save preserves GUI keys
        let mut updated = migrated;
        updated.hw_analog_cfg.hw_dac_unity_vol = 1;
        updated.hw_analog_cfg.analog_boost_db = 6.0;
        assert!(save_runtime_state_in_dir(&base, &updated));
        let doc2 = load_unified_settings_json_in(&base);
        assert_eq!(doc2.get("theme").and_then(JsonValue::as_str), Some("CachyOS Teal Void"));
        let reloaded = load_runtime_state_in_dir(&base, true);
        assert_eq!(reloaded.hw_analog_cfg.hw_dac_unity_vol, 1);
        assert!((reloaded.hw_analog_cfg.analog_boost_db - 6.0).abs() < 0.05);

        // Security: reject group/world-writable or symlinked salsa_settings.json
        let sfile = base.join(SALSA_SETTINGS_FILENAME);
        fs::set_permissions(&sfile, Permissions::from_mode(0o666)).unwrap();
        assert!(load_unified_settings_json_in(&base).as_object().unwrap().is_empty());

        fs::remove_file(&sfile).unwrap();
        symlink("/etc/passwd", &sfile).unwrap();
        assert!(load_unified_settings_json_in(&base).as_object().unwrap().is_empty());

        let _ = fs::remove_dir_all(&base);
    }
}
