// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::mem;
use std::fs::{self, OpenOptions, Permissions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::c_abi::{
    copy_str_to_c, LwasapiBtCodecCfg, LwasapiEqConfig, LwasapiHwAnalogCfg, LwasapiMicApoCfg,
    LwasapiParamEqExt, SalsaRuntimeState, LWASAPI_BT_PREF_AUTO, LWASAPI_BT_PREF_SBC,
    LWASAPI_EQ_BANDS, LWASAPI_FILTER_HIGH_SHELF, LWASAPI_FILTER_PEAK, LWASAPI_LDAC_MODE_330,
    LWASAPI_LDAC_MODE_ADAPTIVE,
};
use crate::hw_analog::{lwasapi_hw_analog_clamp_cfg, lwasapi_hw_analog_default_cfg};
use crate::unix_sys::{
    current_pid, current_uid, is_safe_owner_dir, user_home_dir, O_CLOEXEC, O_NOFOLLOW,
};

pub const K_EQ_FREQS: [f32; LWASAPI_EQ_BANDS] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

const MAGIC_V1: u32 = 0x5341_4C53; // 'SALS'
const MAGIC_V2: u32 = 0x5341_4C54; // 'SALT'
const MAGIC_V3: u32 = 0x5341_4C55; // 'SALU'
const MAGIC_V4: u32 = 0x5341_4C56; // 'SALV'
const MAGIC_V5: u32 = 0x5341_4C57; // 'SALW'

pub const PERSISTED_V1_SIZE: usize = 152;
pub const PERSISTED_V2_SIZE: usize = 280;
pub const PERSISTED_V3_SIZE: usize = 532;
pub const PERSISTED_V4_SIZE: usize = 556;
pub const PERSISTED_V5_SIZE: usize = 576;

pub fn default_runtime_state() -> SalsaRuntimeState {
    let mut preset_name = [0u8; 32];
    copy_str_to_c(&mut preset_name, "Flat / Studio Reference");

    let eq_cfg = LwasapiEqConfig {
        enabled: 1,
        dynamic_mode: 0,
        preamp_db: 0.0,
        dyn_threshold_db: -16.0,
        dyn_ratio: 2.2,
        limiter_enabled: 1,
        band_gains_db: [0.0; LWASAPI_EQ_BANDS],
        band_q: [1.414; LWASAPI_EQ_BANDS],
        preset_name,
    };

    let param_eq = LwasapiParamEqExt {
        band_freq_hz: K_EQ_FREQS,
        band_type: [LWASAPI_FILTER_PEAK; LWASAPI_EQ_BANDS],
        crossfeed_enabled: 0,
        crossfeed_level: 0.45,
        crossfeed_cutoff_hz: 700,
        reserved: 0,
    };

    let mic_apo = LwasapiMicApoCfg {
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

    let bt_cfg = LwasapiBtCodecCfg {
        preferred_codec: LWASAPI_BT_PREF_AUTO,
        ldac_mode: LWASAPI_LDAC_MODE_ADAPTIVE,
        wireless_abr_enabled: 1,
        effective_kbps: 0,
        supported_codecs_mask: 0,
        hfp_sco_enabled: 0,
        hfp_sco_active: 0,
        bt_sco_codec: [0u8; 8],
        neg_reason: [0u8; 64],
    };

    let mut hw_analog_cfg: LwasapiHwAnalogCfg = unsafe { mem::zeroed() };
    unsafe { lwasapi_hw_analog_default_cfg(&mut hw_analog_cfg) };

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
    }
}

pub fn clamp_eq_cfg(eq: &mut LwasapiEqConfig) {
    eq.enabled = u32::from(eq.enabled != 0);
    if eq.dynamic_mode > 2 {
        eq.dynamic_mode = 1;
    }
    eq.limiter_enabled = u32::from(eq.limiter_enabled != 0);
    eq.preamp_db = if eq.preamp_db.is_finite() {
        eq.preamp_db.clamp(-24.0, 24.0)
    } else {
        0.0
    };
    eq.dyn_threshold_db = if eq.dyn_threshold_db.is_finite() {
        eq.dyn_threshold_db.clamp(-60.0, 0.0)
    } else {
        -16.0
    };
    eq.dyn_ratio = if eq.dyn_ratio.is_finite() {
        eq.dyn_ratio.clamp(1.0, 20.0)
    } else {
        2.2
    };
    for g in &mut eq.band_gains_db {
        *g = if g.is_finite() {
            g.clamp(-24.0, 24.0)
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

pub fn clamp_param_eq_ext(ext: &mut LwasapiParamEqExt) {
    for i in 0..LWASAPI_EQ_BANDS {
        let f = ext.band_freq_hz[i];
        if !(20.0..=20_000.0).contains(&f) {
            ext.band_freq_hz[i] = K_EQ_FREQS[i];
        }
        if ext.band_type[i] > LWASAPI_FILTER_HIGH_SHELF {
            ext.band_type[i] = LWASAPI_FILTER_PEAK;
        }
    }
    ext.crossfeed_enabled = u32::from(ext.crossfeed_enabled != 0);
    if !(0.05..=1.0).contains(&ext.crossfeed_level) {
        ext.crossfeed_level = 0.45;
    }
    if !(300..=1500).contains(&ext.crossfeed_cutoff_hz) {
        ext.crossfeed_cutoff_hz = 700;
    }
}

pub fn clamp_mic_apo_cfg(m: &mut LwasapiMicApoCfg) {
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
    let valid_v5 = bytes.len() == PERSISTED_V5_SIZE && magic == MAGIC_V5;
    let valid_v4 = bytes.len() >= PERSISTED_V4_SIZE && magic == MAGIC_V4;
    let valid_v3 = bytes.len() >= PERSISTED_V3_SIZE && magic == MAGIC_V3;
    let valid_v2 = bytes.len() >= PERSISTED_V2_SIZE && magic == MAGIC_V2;
    let valid_v1 = bytes.len() >= PERSISTED_V1_SIZE && magic == MAGIC_V1;
    if !(valid_v5 || valid_v4 || valid_v3 || valid_v2 || valid_v1) {
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

    // Offset 16..152: LwasapiEqConfig (136 bytes)
    st.eq_cfg.enabled = read_u32(bytes, 16);
    st.eq_cfg.dynamic_mode = read_u32(bytes, 20);
    st.eq_cfg.preamp_db = read_f32(bytes, 24);
    st.eq_cfg.dyn_threshold_db = read_f32(bytes, 28);
    st.eq_cfg.dyn_ratio = read_f32(bytes, 32);
    st.eq_cfg.limiter_enabled = read_u32(bytes, 36);
    for i in 0..LWASAPI_EQ_BANDS {
        st.eq_cfg.band_gains_db[i] = read_f32(bytes, 40 + i * 4);
        let q = read_f32(bytes, 80 + i * 4);
        st.eq_cfg.band_q[i] = if (0.2..=10.0).contains(&q) { q } else { 1.414 };
    }
    st.eq_cfg.preset_name.copy_from_slice(&bytes[120..152]);
    st.eq_cfg.preset_name[31] = 0;
    clamp_eq_cfg(&mut st.eq_cfg);

    if valid_v5 || valid_v4 || valid_v3 || valid_v2 {
        // Offset 152..216: target_playback_id[64], Offset 216..280: target_capture_id[64]
        if bytes[152] != 0 {
            copy_c_field(&mut st.target_playback_id, &bytes[152..216]);
        }
        if bytes[216] != 0 {
            copy_c_field(&mut st.target_capture_id, &bytes[216..280]);
        }
    }

    if valid_v5 || valid_v4 || valid_v3 {
        // Offset 280..284: reserved_0, Offset 284..288: reserved_1
        // Offset 288..384: LwasapiParamEqExt (96 bytes)
        for i in 0..LWASAPI_EQ_BANDS {
            st.param_eq.band_freq_hz[i] = read_f32(bytes, 288 + i * 4);
            st.param_eq.band_type[i] = read_u32(bytes, 328 + i * 4);
        }
        st.param_eq.crossfeed_enabled = read_u32(bytes, 368);
        st.param_eq.crossfeed_level = read_f32(bytes, 372);
        st.param_eq.crossfeed_cutoff_hz = read_u32(bytes, 376);
        st.param_eq.reserved = read_u32(bytes, 380);
        clamp_param_eq_ext(&mut st.param_eq);

        // Offset 384..432: LwasapiMicApoCfg (48 bytes)
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

        // Offset 432..532: LwasapiBtCodecCfg (100 bytes)
        st.bt_cfg.preferred_codec = read_u32(bytes, 432);
        if st.bt_cfg.preferred_codec > LWASAPI_BT_PREF_SBC {
            st.bt_cfg.preferred_codec = LWASAPI_BT_PREF_AUTO;
        }
        st.bt_cfg.ldac_mode = read_u32(bytes, 436);
        if st.bt_cfg.ldac_mode > LWASAPI_LDAC_MODE_330 {
            st.bt_cfg.ldac_mode = LWASAPI_LDAC_MODE_ADAPTIVE;
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

    if valid_v5 || valid_v4 {
        // Offset 532..556: LwasapiHwAnalogCfg (24 bytes)
        st.hw_analog_cfg.hw_dac_unity_vol = read_u32(bytes, 532);
        st.hw_analog_cfg.hp_amp_tier = read_u32(bytes, 536);
        st.hw_analog_cfg.output_route = read_u32(bytes, 540);
        st.hw_analog_cfg.unlock_0db_rails = read_u32(bytes, 544);
        st.hw_analog_cfg.analog_boost_db = read_f32(bytes, 548);
        st.hw_analog_cfg.mic_hw_boost_db = read_f32(bytes, 552);
        unsafe { lwasapi_hw_analog_clamp_cfg(&mut st.hw_analog_cfg) };
    }

    if valid_v5 {
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

    *out = st;
    true
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
    for i in 0..LWASAPI_EQ_BANDS {
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

    for i in 0..LWASAPI_EQ_BANDS {
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

pub fn state_dir_and_file() -> (PathBuf, PathBuf) {
    let home = user_home_dir();
    let dir = PathBuf::from(format!("{home}/.config/lwasapi"));
    let file = dir.join("salsa_state.bin");
    (dir, file)
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
    let max_len = (PERSISTED_V5_SIZE as u64) * 2;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > max_len {
        return false;
    }
    let mut buf = Vec::with_capacity(PERSISTED_V5_SIZE);
    let mut limited = f.take((PERSISTED_V5_SIZE as u64) + 1);
    if limited.read_to_end(&mut buf).is_err() || buf.len() > PERSISTED_V5_SIZE {
        return false;
    }
    decode_persisted_state_bytes(&buf, out)
}

pub fn save_state_to_path(dir: &Path, path: &Path, st: &SalsaRuntimeState) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let _ = fs::set_permissions(dir, Permissions::from_mode(0o700));
    if !is_safe_owner_dir(dir, current_uid(), false) {
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

    let bytes = encode_persisted_state_v5(st);
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

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_clamp_eq_cfg(eq: *mut LwasapiEqConfig) {
    if !eq.is_null() {
        clamp_eq_cfg(unsafe { &mut *eq });
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_clamp_param_eq_ext(ext: *mut LwasapiParamEqExt) {
    if !ext.is_null() {
        clamp_param_eq_ext(unsafe { &mut *ext });
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_clamp_mic_apo_cfg(m: *mut LwasapiMicApoCfg) {
    if !m.is_null() {
        clamp_mic_apo_cfg(unsafe { &mut *m });
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_state_init_defaults(out: *mut SalsaRuntimeState) {
    if !out.is_null() {
        unsafe { *out = default_runtime_state() };
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_state_load(io_st: *mut SalsaRuntimeState) {
    if io_st.is_null() {
        return;
    }
    let (_, file) = state_dir_and_file();
    let _ = load_state_from_path(&file, unsafe { &mut *io_st });
}

#[no_mangle]
pub unsafe extern "C" fn salsa_state_save(st: *const SalsaRuntimeState) {
    if st.is_null() {
        return;
    }
    let (dir, file) = state_dir_and_file();
    let _ = save_state_to_path(&dir, &file, unsafe { &*st });
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use std::os::unix::fs::symlink;

    #[repr(C)]
    struct CPersistedV1 {
        magic: u32,
        sample_rate: u32,
        period_frames: u32,
        format_bits: u32,
        eq_cfg: LwasapiEqConfig,
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
        param_eq: LwasapiParamEqExt,
        mic_apo: LwasapiMicApoCfg,
        bt_cfg: LwasapiBtCodecCfg,
    }

    #[repr(C)]
    struct CPersistedV4 {
        v3: CPersistedV3,
        hw_analog_cfg: LwasapiHwAnalogCfg,
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
        assert_eq!(loaded.bt_cfg.preferred_codec, LWASAPI_BT_PREF_AUTO);
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
        assert_eq!(ext.band_type[2], LWASAPI_FILTER_PEAK);
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
        let base = std::env::temp_dir().join(format!("lwasapi_state_test_{}", current_pid()));
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
}
