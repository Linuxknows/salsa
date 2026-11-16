#!/usr/bin/env python3
"""
Shared Salsa / LWASAPI IPC ctypes definitions, constants, presets & library binding helper.
Used by salsa-gui.py, lwasapi-tray.py, and regression tests so ABI changes live in one place.
"""

import ctypes

LWASAPI_EQ_BANDS = 10
LWASAPI_MAX_SESSIONS = 32
LWASAPI_MAX_DEVICES = 32
LWASAPI_CMD_SET_VOLUME = 4
LWASAPI_CFG_FLAG_CAPTURE_RATE = 0x43415052  # 'CAPR'

LWASAPI_FILTER_PEAK = 0
LWASAPI_FILTER_LOW_SHELF = 1
LWASAPI_FILTER_HIGH_SHELF = 2

LWASAPI_BT_PREF_AUTO = 0
LWASAPI_BT_PREF_LDAC = 1
LWASAPI_BT_PREF_APTX_HD = 2
LWASAPI_BT_PREF_APTX = 3
LWASAPI_BT_PREF_AAC = 4
LWASAPI_BT_PREF_SBC = 5

LWASAPI_LDAC_MODE_ABR = 0
LWASAPI_LDAC_MODE_HQ_990 = 1
LWASAPI_LDAC_MODE_SQ_660 = 2
LWASAPI_LDAC_MODE_MQ_330 = 3

LWASAPI_BT_CAP_LDAC = 1 << 0
LWASAPI_BT_CAP_APTX_HD = 1 << 1
LWASAPI_BT_CAP_APTX = 1 << 2
LWASAPI_BT_CAP_AAC = 1 << 3
LWASAPI_BT_CAP_SBC = 1 << 4

LWASAPI_HW_AMP_AUTO = 0
LWASAPI_HW_AMP_LOW = 1
LWASAPI_HW_AMP_MED = 2
LWASAPI_HW_AMP_HIGH = 3

LWASAPI_HW_ROUTE_AUTO = 0
LWASAPI_HW_ROUTE_HP = 1
LWASAPI_HW_ROUTE_SPK = 2

LWASAPI_HW_CAP_DAC_VOL = 1 << 0
LWASAPI_HW_CAP_HP_AMP = 1 << 1
LWASAPI_HW_CAP_OUT_ROUTE = 1 << 2
LWASAPI_HW_CAP_MIC_BOOST = 1 << 3
LWASAPI_HW_CAP_LINE_AIR = 1 << 4
LWASAPI_HW_CAP_HID_BOX = 1 << 5
LWASAPI_HW_CAP_STRIX_ARCH = 1 << 6
LWASAPI_HW_CAP_HIGH_RES = 1 << 7

DEFAULT_BAND_FREQS_HZ = [31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0]
SUPPORTED_SAMPLE_RATES = (44100, 48000, 88200, 96000, 176400, 192000)


class LwasapiEqConfig(ctypes.Structure):
    _fields_ = [
        ("enabled", ctypes.c_uint32),
        ("dynamic_mode", ctypes.c_uint32),
        ("preamp_db", ctypes.c_float),
        ("dyn_threshold_db", ctypes.c_float),
        ("dyn_ratio", ctypes.c_float),
        ("limiter_enabled", ctypes.c_uint32),
        ("band_gains_db", ctypes.c_float * LWASAPI_EQ_BANDS),
        ("band_q", ctypes.c_float * LWASAPI_EQ_BANDS),
        ("preset_name", ctypes.c_char * 32),
    ]


class LwasapiSessionInfo(ctypes.Structure):
    _fields_ = [
        ("session_id", ctypes.c_uint32),
        ("client_pid", ctypes.c_uint32),
        ("client_name", ctypes.c_char * 64),
        ("share_mode", ctypes.c_uint32),
        ("is_capture", ctypes.c_uint32),
        ("sample_rate", ctypes.c_uint32),
        ("channels", ctypes.c_uint32),
        ("active", ctypes.c_uint32),
        ("volume_milli", ctypes.c_uint32),
        ("buffered_frames", ctypes.c_uint64),
        ("xruns", ctypes.c_uint64),
        ("muted", ctypes.c_uint32),
        ("routing_flags", ctypes.c_uint32),
        ("target_device", ctypes.c_char * 64),
    ]


class LwasapiDeviceEntry(ctypes.Structure):
    _fields_ = [
        ("id", ctypes.c_char * 64),
        ("name", ctypes.c_char * 64),
        ("driver", ctypes.c_char * 32),
        ("is_capture", ctypes.c_uint32),
        ("is_bluetooth", ctypes.c_uint32),
        ("is_active", ctypes.c_uint32),
        ("card_index", ctypes.c_uint32),
        ("device_index", ctypes.c_uint32),
        ("min_rate", ctypes.c_uint32),
        ("max_rate", ctypes.c_uint32),
    ]


class LwasapiDeviceList(ctypes.Structure):
    _fields_ = [
        ("num_devices", ctypes.c_uint32),
        ("devices", LwasapiDeviceEntry * LWASAPI_MAX_DEVICES),
    ]


class LwasapiParamEqExt(ctypes.Structure):
    _fields_ = [
        ("band_freq_hz", ctypes.c_float * LWASAPI_EQ_BANDS),
        ("band_type", ctypes.c_uint32 * LWASAPI_EQ_BANDS),
        ("crossfeed_enabled", ctypes.c_uint32),
        ("crossfeed_level", ctypes.c_float),
        ("crossfeed_cutoff_hz", ctypes.c_uint32),
        ("reserved", ctypes.c_uint32),
    ]


class LwasapiMicApoCfg(ctypes.Structure):
    _fields_ = [
        ("enabled", ctypes.c_uint32),
        ("hpf_enabled", ctypes.c_uint32),
        ("hpf_hz", ctypes.c_float),
        ("gate_enabled", ctypes.c_uint32),
        ("gate_thresh_db", ctypes.c_float),
        ("rnnoise_enabled", ctypes.c_uint32),
        ("comp_enabled", ctypes.c_uint32),
        ("comp_thresh_db", ctypes.c_float),
        ("comp_ratio", ctypes.c_float),
        ("clarity_db", ctypes.c_float),
        ("sidetone_enabled", ctypes.c_uint32),
        ("sidetone_pct", ctypes.c_uint32),
    ]


class LwasapiBtCodecCfg(ctypes.Structure):
    _fields_ = [
        ("preferred_codec", ctypes.c_uint32),
        ("ldac_mode", ctypes.c_uint32),
        ("wireless_abr_enabled", ctypes.c_uint32),
        ("effective_kbps", ctypes.c_uint32),
        ("supported_codecs_mask", ctypes.c_uint32),
        ("hfp_sco_enabled", ctypes.c_uint32),
        ("hfp_sco_active", ctypes.c_uint32),
        ("bt_sco_codec", ctypes.c_char * 8),
        ("neg_reason", ctypes.c_char * 64),
    ]


class LwasapiHwAnalogCfg(ctypes.Structure):
    _fields_ = [
        ("hw_dac_unity_vol", ctypes.c_uint32),
        ("hp_amp_tier", ctypes.c_uint32),
        ("output_route", ctypes.c_uint32),
        ("unlock_0db_rails", ctypes.c_uint32),
        ("analog_boost_db", ctypes.c_float),
        ("mic_hw_boost_db", ctypes.c_float),
    ]


class LwasapiHwAnalogInfo(ctypes.Structure):
    _fields_ = [
        ("caps_mask", ctypes.c_uint32),
        ("dac_unity_active", ctypes.c_uint32),
        ("hid_box_connected", ctypes.c_uint32),
        ("active_pb_card", ctypes.c_int32),
        ("active_cap_card", ctypes.c_int32),
        ("reserved", ctypes.c_uint32),
        ("card_arch_name", ctypes.c_char * 64),
        ("dac_ctl_name", ctypes.c_char * 32),
        ("amp_ctl_name", ctypes.c_char * 32),
        ("cap_ctl_name", ctypes.c_char * 32),
        ("diag_note", ctypes.c_char * 64),
    ]


class LwasapiEngineStatus(ctypes.Structure):
    _fields_ = [
        ("hw_device", ctypes.c_char * 64),
        ("hw_capture_device", ctypes.c_char * 64),
        ("midi_device", ctypes.c_char * 64),
        ("hw_sample_rate", ctypes.c_uint32),
        ("hw_channels", ctypes.c_uint32),
        ("hw_period_frames", ctypes.c_uint32),
        ("hw_buffer_frames", ctypes.c_uint32),
        ("hw_format_bits", ctypes.c_uint32),
        ("desktop_volume_pct", ctypes.c_uint32),
        ("desktop_muted", ctypes.c_uint32),
        ("desktop_mic_volume_pct", ctypes.c_uint32),
        ("desktop_mic_muted", ctypes.c_uint32),
        ("realtime_priority", ctypes.c_uint32),
        ("exclusive_owner_id", ctypes.c_int32),
        ("total_cycles", ctypes.c_uint64),
        ("hw_xruns", ctypes.c_uint64),
        ("midi_events_routed", ctypes.c_uint64),
        ("eq_cfg", LwasapiEqConfig),
        ("eq_band_levels_db", ctypes.c_float * LWASAPI_EQ_BANDS),
        ("eq_dyn_delta_db", ctypes.c_float * LWASAPI_EQ_BANDS),
        ("output_peak_l", ctypes.c_float),
        ("output_peak_r", ctypes.c_float),
        ("num_sessions", ctypes.c_uint32),
        ("sessions", LwasapiSessionInfo * LWASAPI_MAX_SESSIONS),
        ("bt_active", ctypes.c_uint32),
        ("bt_delay_ms", ctypes.c_uint32),
        ("bt_alias", ctypes.c_char * 64),
        ("bt_codec", ctypes.c_char * 16),
        ("pa_server_running", ctypes.c_uint32),
        ("pa_clients", ctypes.c_uint32),
        ("active_playback_id", ctypes.c_char * 64),
        ("active_capture_id", ctypes.c_char * 64),
        ("param_eq", LwasapiParamEqExt),
        ("mic_apo", LwasapiMicApoCfg),
        ("bt_codec_cfg", LwasapiBtCodecCfg),
        ("mic_apo_peak_in", ctypes.c_float),
        ("mic_apo_peak_out", ctypes.c_float),
        ("mic_rnnoise_active", ctypes.c_uint32),
        ("hw_analog_cfg", LwasapiHwAnalogCfg),
        ("hw_analog_info", LwasapiHwAnalogInfo),
    ]


PRESETS = {
    "Flat / Studio Reference": {
        "mode": 0,
        "preamp": 0.0,
        "thresh": -18.0,
        "ratio": 2.0,
        "gains": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    "Bose QC Ultra 2 Audiophile": {
        "mode": 0,
        "preamp": -1.5,
        "thresh": -16.0,
        "ratio": 2.0,
        "gains": [1.0, 0.5, -1.5, -0.5, 0.5, 0.5, 0.5, 0.0, -1.5, 0.5],
    },
    "Dynamic Bass & Sub Control": {
        "mode": 1,
        "preamp": -3.0,
        "thresh": -16.0,
        "ratio": 3.5,
        "gains": [6.5, 5.5, 2.5, 0.0, -1.0, 0.0, 1.0, 2.0, 2.5, 2.0],
    },
    "192kHz Hi-Res Mastering": {
        "mode": 2,
        "preamp": -1.0,
        "thresh": -20.0,
        "ratio": 2.2,
        "gains": [1.5, 1.0, 0.0, -0.5, 0.0, 0.5, 1.5, 2.5, 3.5, 4.5],
    },
    "FPS Gaming Footstep Radar": {
        "mode": 1,
        "preamp": -2.0,
        "thresh": -22.0,
        "ratio": 4.0,
        "gains": [-4.0, -2.0, 1.0, 2.5, 3.0, 4.5, 6.0, 5.5, 3.0, 1.0],
    },
    "Vocal & Podcast Clarity": {
        "mode": 1,
        "preamp": -1.5,
        "thresh": -18.0,
        "ratio": 3.0,
        "gains": [-5.0, -3.0, -1.0, 1.5, 3.5, 4.5, 4.0, 2.5, 1.5, 0.5],
    },
    "Late-Night Dynamic Loudness": {
        "mode": 1,
        "preamp": -2.5,
        "thresh": -24.0,
        "ratio": 4.5,
        "gains": [4.5, 3.5, 1.0, 0.0, 0.5, 1.5, 2.5, 3.0, 3.5, 3.0],
    },
}

MIC_APO_PRESETS = {
    "Discord Voice (48k + RNNoise)": {
        "enabled": 1,
        "hpf_enabled": 1,
        "hpf_hz": 65.0,
        "gate_enabled": 1,
        "gate_thresh_db": -50.0,
        "rnnoise_enabled": 1,
        "comp_enabled": 1,
        "comp_thresh_db": -14.0,
        "comp_ratio": 2.0,
        "clarity_db": 1.5,
    },
    "Studio Broadcast (Warm AI)": {
        "enabled": 1,
        "hpf_enabled": 1,
        "hpf_hz": 75.0,
        "gate_enabled": 1,
        "gate_thresh_db": -45.0,
        "rnnoise_enabled": 1,
        "comp_enabled": 1,
        "comp_thresh_db": -15.0,
        "comp_ratio": 2.2,
        "clarity_db": 2.0,
    },
    "Clean Vocal (No RNNoise)": {
        "enabled": 1,
        "hpf_enabled": 1,
        "hpf_hz": 65.0,
        "gate_enabled": 1,
        "gate_thresh_db": -52.0,
        "rnnoise_enabled": 0,
        "comp_enabled": 1,
        "comp_thresh_db": -16.0,
        "comp_ratio": 1.8,
        "clarity_db": 1.0,
    },
    "Bypass (Raw Mic)": {
        "enabled": 0,
    },
}


def bind_liblwasapi(lib: ctypes.CDLL) -> ctypes.CDLL:
    """Attach standard ctypes argtypes and restype bindings to loaded liblwasapi.so."""
    lib.LWASAPI_GetEngineStatus.argtypes = [ctypes.POINTER(LwasapiEngineStatus)]
    lib.LWASAPI_GetEngineStatus.restype = ctypes.c_int32
    lib.LWASAPI_SetEngineConfig.argtypes = [ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
    lib.LWASAPI_SetEngineConfig.restype = ctypes.c_int32
    if hasattr(lib, "LWASAPI_SetCaptureSampleRate"):
        lib.LWASAPI_SetCaptureSampleRate.argtypes = [ctypes.c_uint32]
        lib.LWASAPI_SetCaptureSampleRate.restype = ctypes.c_int32
    lib.LWASAPI_SetDynamicEQ.argtypes = [ctypes.POINTER(LwasapiEqConfig)]
    lib.LWASAPI_SetDynamicEQ.restype = ctypes.c_int32
    lib.LWASAPI_ListDevices.argtypes = [ctypes.POINTER(LwasapiDeviceList)]
    lib.LWASAPI_ListDevices.restype = ctypes.c_int32
    lib.LWASAPI_SetDevices.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.LWASAPI_SetDevices.restype = ctypes.c_int32
    if hasattr(lib, "LWASAPI_SetSessionControl"):
        lib.LWASAPI_SetSessionControl.argtypes = [
            ctypes.c_uint32, ctypes.c_int32, ctypes.c_int32, ctypes.c_char_p,
        ]
        lib.LWASAPI_SetSessionControl.restype = ctypes.c_int32
    if hasattr(lib, "LWASAPI_SetParametricEQ"):
        lib.LWASAPI_SetParametricEQ.argtypes = [
            ctypes.POINTER(LwasapiEqConfig), ctypes.POINTER(LwasapiParamEqExt),
        ]
        lib.LWASAPI_SetParametricEQ.restype = ctypes.c_int32
    if hasattr(lib, "LWASAPI_SetMicAPO"):
        lib.LWASAPI_SetMicAPO.argtypes = [ctypes.POINTER(LwasapiMicApoCfg)]
        lib.LWASAPI_SetMicAPO.restype = ctypes.c_int32
    if hasattr(lib, "LWASAPI_SetBtCodecConfig"):
        lib.LWASAPI_SetBtCodecConfig.argtypes = [ctypes.POINTER(LwasapiBtCodecCfg)]
        lib.LWASAPI_SetBtCodecConfig.restype = ctypes.c_int32
    if hasattr(lib, "LWASAPI_SetHwAnalog"):
        lib.LWASAPI_SetHwAnalog.argtypes = [ctypes.POINTER(LwasapiHwAnalogCfg)]
        lib.LWASAPI_SetHwAnalog.restype = ctypes.c_int32
    return lib
