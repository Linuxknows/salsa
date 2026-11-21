// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use core::mem::{self, offset_of};
use core::ptr;
use core::slice;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::fs;
use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, ptr_to_opt_str, LwasapiBtCodecCfg, LwasapiDbgCallbacks,
    LwasapiDbgRtStats, LwasapiDbgSnapshot, LwasapiDeviceList, LwasapiEngineStatus, LwasapiEqConfig,
    LwasapiHwAnalogCfg, LwasapiIpcReq, LwasapiIpcResp, LwasapiMicApoCfg,
    LwasapiMidiEvent, LwasapiMidiHub, LwasapiPaCallbacks, LwasapiParamEqExt,
    LwasapiParamEqPayload, LwasapiPwBrokerOpts, LwasapiShmRing, SalsaBtSnapshot,
    SalsaMicCubicState, SalsaRuntimeState, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_SHAREMODE_EXCLUSIVE,
    AUDCLNT_STREAMFLAGS_CAPTURE, AUDCLNT_STREAMFLAGS_LOOPBACK, E_FAIL, E_INVALIDARG, E_OUTOFMEMORY,
    LWASAPI_BT_CONN_CONNECT, LWASAPI_BT_CONN_DISCONNECT, LWASAPI_BT_CONN_QUERY,
    LWASAPI_BT_CONN_SET_AUTOCONN, LWASAPI_BT_RING_FRAMES, LWASAPI_BT_STAT_CONNECTED,
    LWASAPI_BT_STAT_OPTIONAL, LWASAPI_BT_STAT_TRANSPORT, LWASAPI_BT_STAT_USB_MATCHES_BT,
    LWASAPI_CFG_FLAG_CAPTURE_RATE, LWASAPI_CMD_BT_CONNECTION, LWASAPI_CMD_CREATE_SESSION,
    LWASAPI_CMD_GET_STATUS, LWASAPI_CMD_INJECT_MIDI, LWASAPI_CMD_LIST_DEVICES,
    LWASAPI_CMD_SET_BT_CODEC, LWASAPI_CMD_SET_DEVICES, LWASAPI_CMD_SET_ENGINE_CONFIG,
    LWASAPI_CMD_SET_EQ, LWASAPI_CMD_SET_HW_ANALOG, LWASAPI_CMD_SET_MIC_APO,
    LWASAPI_CMD_SET_PARAM_EQ, LWASAPI_CMD_SET_SESSION_CONTROL, LWASAPI_CMD_SET_VOLUME,
    LWASAPI_DBG_ALSA, LWASAPI_DBG_BLUEZ, LWASAPI_DBG_ENGINE, LWASAPI_DBG_ERR, LWASAPI_DBG_INFO,
    LWASAPI_DBG_MIDI, LWASAPI_DBG_PA, LWASAPI_DBG_WARN, LWASAPI_DBG_WASAPI, LWASAPI_FMT_FLOAT32,
    LWASAPI_MAX_CHANNELS, LWASAPI_MAX_DEVICES, LWASAPI_MAX_PERIOD, LWASAPI_MAX_SESSIONS,
    LWASAPI_RING_FRAMES, LWASAPI_RING_MASK, LWASAPI_RING_MAGIC, LWASAPI_SOCKET_DEFAULT,
    SALSA_PACK_F32_LE, SALSA_PACK_S16_LE, SALSA_PACK_S24_3LE, SALSA_PACK_S24_LE,
    SALSA_PACK_S32_LE, S_OK,
};
use crate::debug_server::{lwasapi_debug_record_rt_period, lwasapi_debug_start, lwasapi_debug_stop};
use crate::dsp::{
    salsa_dsp_consume_session_frames, salsa_dsp_get_telem, salsa_dsp_pack_pcm,
    salsa_dsp_process_capture_mic_apo, salsa_dsp_process_dynamic_eq, salsa_dsp_resample_mic_cubic,
    salsa_dsp_scale_cap_clip, shape_hw_analog_sample,
};
use crate::failsafe::{record_trip, FS_MANUAL_RESET};
use crate::hw_analog::{
    lwasapi_hw_analog_clamp_cfg, lwasapi_hw_analog_create, lwasapi_hw_analog_destroy,
    lwasapi_hw_analog_get_cap_makeup_linear, lwasapi_hw_analog_get_drive_linear,
    lwasapi_hw_analog_get_info, lwasapi_hw_analog_is_dac_unity_active, lwasapi_hw_analog_poll_hid,
    lwasapi_hw_analog_probe, lwasapi_hw_analog_score_card, lwasapi_hw_analog_softknee_active,
    lwasapi_hw_analog_sync, LwasapiHwAnalogCtx,
};
use crate::midi_hub::{
    lwasapi_midi_close, lwasapi_midi_init, lwasapi_midi_poll_events, lwasapi_midi_push_to_ring,
};
use crate::pa_server::{
    lwasapi_pa_server_control_stream, lwasapi_pa_server_fill_debug, lwasapi_pa_server_fill_sessions,
    lwasapi_pa_server_mix_playback, lwasapi_pa_server_notify_change,
    lwasapi_pa_server_notify_volume, lwasapi_pa_server_push_capture,
    lwasapi_pa_server_reset_clients, lwasapi_pa_server_set_stream_volume, lwasapi_pa_server_start,
    lwasapi_pa_server_stop, PaServerHandle,
};
use crate::pw_broker::{
    lwasapi_pw_broker_fill_debug, lwasapi_pw_broker_reset_clients, lwasapi_pw_broker_self_test,
    lwasapi_pw_broker_set_cast_limits, lwasapi_pw_broker_set_preview_ex, lwasapi_pw_broker_start,
    lwasapi_pw_broker_stop, lwasapi_pw_broker_sync_mode, PwBrokerHandle,
};
use crate::state_conf::{
    clamp_eq_cfg, default_runtime_state, salsa_dsp_clamp_mic_apo_cfg,
    salsa_dsp_clamp_param_eq_ext, salsa_state_init_defaults, salsa_state_load, salsa_state_save,
};
use crate::unix_sys::{
    bind_unix_listener_0600, close_raw_fd, create_memfd, current_pid, current_uid, dbg_emit,
    dbg_emit_vals, do_poll, install_term_signals, ioctl_raw, mono_ns, nb_recv, new_eventfd,
    open_chrdev_rw_cloexec, resolve_sock_path_common, seal_memfd_size, send_all_nosignal,
    sendmsg_all_with_fds, sleep_abs_mono_ns, socket_peercred, term_signal_requested,
    try_realtime_fifo, MappedMem, PollFd, POLLERR, POLLHUP, POLLIN,
};

// Linux errno constants (x86_64/generic)
pub(crate) const ERRNO_EINTR: i32 = 4;
pub(crate) const ERRNO_EIO: i32 = 5;
pub(crate) const ERRNO_EBADF: i32 = 9;
pub(crate) const ERRNO_EAGAIN: i32 = 11;
pub(crate) const ERRNO_ENODEV: i32 = 19;
const ERRNO_EINVAL: i32 = 22;
pub(crate) const ERRNO_EPIPE: i32 = 32;
const ERRNO_ENOSYS: i32 = 38;
const ERRNO_EBADFD: i32 = 77;
pub(crate) const ERRNO_ESTRPIPE: i32 = 86;
pub(crate) const ERRNO_ECONNRESET: i32 = 104;
pub(crate) const ERRNO_EINPROGRESS: i32 = 115;

// Linux <sound/asound.h> PCM kernel ABI constants & ioctls
pub(crate) const SNDRV_PCM_IOCTL_HW_REFINE: c_ulong = 0xc260_4110;
pub(crate) const SNDRV_PCM_IOCTL_HW_PARAMS: c_ulong = 0xc260_4111;
pub(crate) const SNDRV_PCM_IOCTL_SW_PARAMS: c_ulong = 0xc088_4113;
pub(crate) const SNDRV_PCM_IOCTL_PREPARE: c_ulong = 0x4140;
pub(crate) const SNDRV_PCM_IOCTL_START: c_ulong = 0x4142;
const SNDRV_PCM_IOCTL_WRITEI_FRAMES: c_ulong = 0x4018_4150;
pub(crate) const SNDRV_PCM_IOCTL_READI_FRAMES: c_ulong = 0x8018_4151;

pub(crate) const SNDRV_PCM_HW_PARAM_ACCESS: usize = 0;
pub(crate) const SNDRV_PCM_HW_PARAM_FORMAT: usize = 1;
pub(crate) const SNDRV_PCM_HW_PARAM_FIRST_MASK: usize = 0;
pub(crate) const SNDRV_PCM_HW_PARAM_LAST_MASK: usize = 2;
pub(crate) const SNDRV_PCM_HW_PARAM_FIRST_INTERVAL: usize = 8;
pub(crate) const SNDRV_PCM_HW_PARAM_LAST_INTERVAL: usize = 19;
pub(crate) const SNDRV_PCM_HW_PARAM_CHANNELS: usize = 10;
pub(crate) const SNDRV_PCM_HW_PARAM_RATE: usize = 11;
pub(crate) const SNDRV_PCM_HW_PARAM_PERIOD_SIZE: usize = 13;
pub(crate) const SNDRV_PCM_HW_PARAM_BUFFER_SIZE: usize = 17;

pub(crate) const SNDRV_PCM_ACCESS_RW_INTERLEAVED: u32 = 3;
pub(crate) const SNDRV_PCM_FORMAT_S16_LE: i32 = 2;
const SNDRV_PCM_FORMAT_S24_LE: i32 = 6;
pub(crate) const SNDRV_PCM_FORMAT_S32_LE: i32 = 10;
const SNDRV_PCM_FORMAT_FLOAT_LE: i32 = 14;
pub(crate) const SNDRV_PCM_FORMAT_S24_3LE: i32 = 32;
pub(crate) const SNDRV_PCM_TSTAMP_ENABLE: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndMask {
    pub(crate) bits: [u32; 8],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndInterval {
    pub(crate) min: u32,
    pub(crate) max: u32,
    pub(crate) flags: u32,
}

#[repr(C)]
pub(crate) struct SndPcmHwParams {
    pub(crate) flags: u32,
    pub(crate) masks: [SndMask; 8],
    pub(crate) intervals: [SndInterval; 21],
    pub(crate) rmask: u32,
    pub(crate) cmask: u32,
    pub(crate) info: u32,
    pub(crate) msbits: u32,
    pub(crate) rate_num: u32,
    pub(crate) rate_den: u32,
    pub(crate) fifo_size: u64,
    pub(crate) reserved: [u8; 64],
}

#[repr(C)]
pub(crate) struct SndPcmSwParams {
    pub(crate) tstamp_mode: c_int,
    pub(crate) period_step: c_uint,
    pub(crate) sleep_min: c_uint,
    pub(crate) pad0: c_uint,
    pub(crate) avail_min: u64,
    pub(crate) xfer_align: u64,
    pub(crate) start_threshold: u64,
    pub(crate) stop_threshold: u64,
    pub(crate) silence_threshold: u64,
    pub(crate) silence_size: u64,
    pub(crate) boundary: u64,
    pub(crate) proto: u32,
    pub(crate) tstamp_type: u32,
    pub(crate) reserved: [u8; 56],
}

#[repr(C)]
pub(crate) struct SndXferi {
    pub(crate) result: c_long,
    pub(crate) buf: *mut c_void,
    pub(crate) frames: c_ulong,
}

use crate::c_abi::{
    lwasapi_bluez_close, lwasapi_bluez_connection_ctl, lwasapi_bluez_enumerate_devices,
    lwasapi_bluez_get_codec_config, lwasapi_bluez_hw_fingerprint, lwasapi_bluez_init_rates,
    lwasapi_bluez_is_optional_mode, lwasapi_bluez_known_audio_aliases,
    lwasapi_bluez_open_capture, lwasapi_bluez_read_capture_native,
    lwasapi_bluez_reconfigure_rate, lwasapi_bluez_set_codec_config, lwasapi_bluez_set_mic_gain_ppm,
    lwasapi_bluez_set_mic_volume, lwasapi_bluez_set_optional_mode,
    lwasapi_bluez_set_preferred_target, lwasapi_bluez_set_sco_enabled,
    lwasapi_bluez_set_tx_enabled, lwasapi_bluez_set_volume, lwasapi_bluez_write_period,
    salsa_audiodg_bt_ptr, salsa_bt_get_snapshot, salsa_bt_is_initialized, salsa_bt_rt_master_vol,
};

#[derive(Clone, Copy)]
struct KernelWaveRtHw {
    fd: RawFd,
    device_path: [u8; 64],
    is_virtual: bool,
    is_bluetooth: bool,
    sample_rate: u32,
    channels: u32,
    period_frames: u32,
    buffer_frames: u32,
    sndrv_format: i32,
    format_bits: u32,
}

impl Default for KernelWaveRtHw {
    fn default() -> Self {
        let mut device_path = [0u8; 64];
        copy_str_to_c(&mut device_path, "WaveRT-Virtual-Clock");
        Self {
            fd: -1,
            device_path,
            is_virtual: true,
            is_bluetooth: false,
            sample_rate: 48_000,
            channels: 2,
            period_frames: 256,
            buffer_frames: 1024,
            sndrv_format: SNDRV_PCM_FORMAT_FLOAT_LE,
            format_bits: 32,
        }
    }
}

struct HwState {
    hw: KernelWaveRtHw,
    target_playback_id: [u8; 64],
    target_capture_id: [u8; 64],
    active_playback_id: [u8; 64],
    active_capture_id: [u8; 64],
    pref_sample_rate: u32,
    pref_cap_sample_rate: u32,
    pref_period_frames: u32,
    pref_format_bits: u32,
}

impl HwState {
    fn from_runtime(rt: &SalsaRuntimeState) -> Self {
        let mut target_playback_id = [0u8; 64];
        let mut target_capture_id = [0u8; 64];
        let mut active_playback_id = [0u8; 64];
        let mut active_capture_id = [0u8; 64];
        target_playback_id.copy_from_slice(&rt.target_playback_id);
        target_capture_id.copy_from_slice(&rt.target_capture_id);
        copy_str_to_c(&mut active_playback_id, "virtual");
        copy_str_to_c(&mut active_capture_id, "none");
        Self {
            hw: KernelWaveRtHw::default(),
            target_playback_id,
            target_capture_id,
            active_playback_id,
            active_capture_id,
            pref_sample_rate: rt.sample_rate,
            pref_cap_sample_rate: rt.cap_sample_rate,
            pref_period_frames: rt.period_frames,
            pref_format_bits: rt.format_bits,
        }
    }
}

struct EngineSession {
    in_use: bool,
    session_id: u32,
    client_pid: u32,
    client_name: [u8; 64],
    share_mode: u32,
    client_sock: Option<UnixStream>,
    _shm_fd: Option<OwnedFd>,
    event_fd: Option<OwnedFd>,
    ring_map: Option<MappedMem>,
    resample_phase: f64,
    sample_rate: u32,
    channels: u32,
    is_capture: bool,
    is_loopback: bool,
    stream_live: bool,
    underrun_pending: bool,
    last_audio_ns: u64,
}

impl EngineSession {
    const fn empty() -> Self {
        Self {
            in_use: false,
            session_id: 0,
            client_pid: 0,
            client_name: [0; 64],
            share_mode: 0,
            client_sock: None,
            _shm_fd: None,
            event_fd: None,
            ring_map: None,
            resample_phase: 0.0,
            sample_rate: 0,
            channels: 0,
            is_capture: false,
            is_loopback: false,
            stream_live: false,
            underrun_pending: false,
            last_audio_ns: 0,
        }
    }

    fn ring_ptr(&self) -> *mut LwasapiShmRing {
        match &self.ring_map {
            Some(m) => m.as_ptr() as *mut LwasapiShmRing,
            None => ptr::null_mut(),
        }
    }
}

struct SessionsState {
    sessions: [EngineSession; LWASAPI_MAX_SESSIONS],
    next_session_id: u32,
    exclusive_session_id: i32,
    eq_cfg: LwasapiEqConfig,
    param_eq: LwasapiParamEqExt,
    mic_apo: LwasapiMicApoCfg,
    hw_analog_cfg: LwasapiHwAnalogCfg,
}

impl SessionsState {
    fn from_runtime(rt: &SalsaRuntimeState) -> Self {
        Self {
            sessions: [const { EngineSession::empty() }; LWASAPI_MAX_SESSIONS],
            next_session_id: 1,
            exclusive_session_id: -1,
            eq_cfg: rt.eq_cfg,
            param_eq: rt.param_eq,
            mic_apo: rt.mic_apo,
            hw_analog_cfg: rt.hw_analog_cfg,
        }
    }
}

struct SavedBtState {
    saved_bt_cfg: LwasapiBtCodecCfg,
    has_saved_vol_state: bool,
    saved_desktop_vol_pct: u32,
    saved_desktop_muted: u32,
    saved_mic_vol_pct: u32,
    saved_mic_muted: u32,
    saved_mic_gain_ppm: u32,
    bt_optional_mode: u32,
}

impl SavedBtState {
    fn from_runtime(rt: &SalsaRuntimeState) -> Self {
        Self {
            saved_bt_cfg: rt.bt_cfg,
            has_saved_vol_state: rt.has_saved_vol_state,
            saved_desktop_vol_pct: rt.desktop_volume_pct,
            saved_desktop_muted: rt.desktop_muted,
            saved_mic_vol_pct: rt.desktop_mic_volume_pct,
            saved_mic_muted: rt.desktop_mic_muted,
            saved_mic_gain_ppm: rt.desktop_mic_gain_ppm,
            bt_optional_mode: rt.bt_optional_mode,
        }
    }
}

struct MidiHubBox(Box<LwasapiMidiHub>);
unsafe impl Send for MidiHubBox {}

#[derive(Clone, Copy)]
struct MidiMeta {
    initialized: bool,
    rawmidi_open: bool,
    controller_name: [u8; 64],
    dev_path: [u8; 64],
    seq_client_id: i32,
    seq_in_port: i32,
}

impl Default for MidiMeta {
    fn default() -> Self {
        Self {
            initialized: false,
            rawmidi_open: false,
            controller_name: [0; 64],
            dev_path: [0; 64],
            seq_client_id: -1,
            seq_in_port: -1,
        }
    }
}

struct AudiodgEngine {
    reconf_lock: Mutex<()>,
    hw_io_lock: Mutex<()>,
    hw_lock: Mutex<HwState>,
    sessions_lock: Mutex<SessionsState>,
    saved_bt: Mutex<SavedBtState>,
    midi_hub: Mutex<MidiHubBox>,
    midi_meta: Mutex<MidiMeta>,

    bt_ptr: AtomicPtr<c_void>,
    pa_srv: AtomicPtr<PaServerHandle>,
    pw_brk: AtomicPtr<PwBrokerHandle>,
    hw_analog: AtomicPtr<LwasapiHwAnalogCtx>,

    running: AtomicBool,
    rt_priority: AtomicU32,
    total_cycles: AtomicU64,
    hw_xruns: AtomicU64,
    cap_read_cursor: AtomicU64,
    midi_hw_events: AtomicU64,
    midi_injected_events: AtomicU64,
    state_dirty: AtomicBool,
    state_dirty_ns: AtomicU64,
    usb_cap_appear_ns: AtomicU64,
}

impl AudiodgEngine {
    fn new(rt: &SalsaRuntimeState, bt: *mut c_void) -> Self {
        let midi_box: Box<LwasapiMidiHub> = unsafe {
            let mut u = Box::<LwasapiMidiHub>::new_uninit();
            ptr::write_bytes(u.as_mut_ptr(), 0, 1);
            u.assume_init()
        };
        Self {
            reconf_lock: Mutex::new(()),
            hw_io_lock: Mutex::new(()),
            hw_lock: Mutex::new(HwState::from_runtime(rt)),
            sessions_lock: Mutex::new(SessionsState::from_runtime(rt)),
            saved_bt: Mutex::new(SavedBtState::from_runtime(rt)),
            midi_hub: Mutex::new(MidiHubBox(midi_box)),
            midi_meta: Mutex::new(MidiMeta::default()),
            bt_ptr: AtomicPtr::new(bt),
            pa_srv: AtomicPtr::new(ptr::null_mut()),
            pw_brk: AtomicPtr::new(ptr::null_mut()),
            hw_analog: AtomicPtr::new(ptr::null_mut()),
            running: AtomicBool::new(true),
            rt_priority: AtomicU32::new(0),
            total_cycles: AtomicU64::new(0),
            hw_xruns: AtomicU64::new(0),
            cap_read_cursor: AtomicU64::new(0),
            midi_hw_events: AtomicU64::new(0),
            midi_injected_events: AtomicU64::new(0),
            state_dirty: AtomicBool::new(false),
            state_dirty_ns: AtomicU64::new(0),
            usb_cap_appear_ns: AtomicU64::new(0),
        }
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed) && !term_signal_requested()
    }

    fn bt(&self) -> *mut c_void {
        self.bt_ptr.load(Ordering::Acquire)
    }

    fn pa(&self) -> *mut PaServerHandle {
        self.pa_srv.load(Ordering::Acquire)
    }

    fn pw(&self) -> *mut PwBrokerHandle {
        self.pw_brk.load(Ordering::Acquire)
    }

    fn hwa(&self) -> *mut LwasapiHwAnalogCtx {
        self.hw_analog.load(Ordering::Acquire)
    }

    fn mark_state_dirty(&self) {
        self.state_dirty_ns.store(mono_ns(), Ordering::Release);
        self.state_dirty.store(true, Ordering::Release);
    }

    fn save_state(&self) {
        self.state_dirty.store(false, Ordering::Release);
        let mut st = default_runtime_state();
        {
            let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            st.sample_rate = if hws.pref_sample_rate != 0 {
                hws.pref_sample_rate
            } else {
                hws.hw.sample_rate
            };
            st.period_frames = if hws.pref_period_frames != 0 {
                hws.pref_period_frames
            } else {
                hws.hw.period_frames
            };
            st.format_bits = if matches!(hws.pref_format_bits, 16 | 24 | 32) {
                hws.pref_format_bits
            } else {
                hws.hw.format_bits
            };
            st.cap_sample_rate = if (44_100..=192_000).contains(&hws.pref_cap_sample_rate) {
                hws.pref_cap_sample_rate
            } else {
                0
            };
            let pb = c_str_to_string(&hws.target_playback_id);
            let cap = c_str_to_string(&hws.target_capture_id);
            copy_str_to_c(&mut st.target_playback_id, if pb.is_empty() { "auto" } else { &pb });
            copy_str_to_c(
                &mut st.target_capture_id,
                if cap.is_empty() { "auto" } else { &cap },
            );
        }
        {
            let ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
            st.eq_cfg = ss.eq_cfg;
            st.param_eq = ss.param_eq;
            st.mic_apo = ss.mic_apo;
            st.hw_analog_cfg = ss.hw_analog_cfg;
        }
        let bt_snap = snapshot_bt(self.bt());
        {
            let mut sb = self.saved_bt.lock().unwrap_or_else(|e| e.into_inner());
            if bt_snap.initialized {
                unsafe { lwasapi_bluez_get_codec_config(self.bt(), &mut st.bt_cfg) };
                sb.saved_bt_cfg = st.bt_cfg;
                sb.saved_desktop_vol_pct = bt_snap.desktop_volume_pct;
                sb.saved_desktop_muted = u32::from(bt_snap.desktop_muted);
                sb.saved_mic_vol_pct = bt_snap.desktop_mic_volume_pct;
                sb.saved_mic_muted = u32::from(bt_snap.desktop_mic_muted);
                sb.saved_mic_gain_ppm = bt_snap.desktop_mic_gain_ppm;
                let opt = unsafe { lwasapi_bluez_is_optional_mode(self.bt()) };
            if bt_snap.initialized {
                st.bt_optional_mode = u32::from(opt);
                sb.bt_optional_mode = st.bt_optional_mode;
            } else {
                st.bt_optional_mode = sb.bt_optional_mode;
            }
            sb.has_saved_vol_state = true;
            } else {
                st.bt_cfg = sb.saved_bt_cfg;
            }
            st.desktop_volume_pct = sb.saved_desktop_vol_pct;
            st.desktop_muted = sb.saved_desktop_muted;
            st.desktop_mic_volume_pct = sb.saved_mic_vol_pct;
            st.desktop_mic_muted = sb.saved_mic_muted;
            st.desktop_mic_gain_ppm = sb.saved_mic_gain_ppm;
            st.has_saved_vol_state = sb.has_saved_vol_state;
        }
        unsafe { salsa_state_save(&st) };
    }

    fn hw_analog_probe_and_sync(&self, re_probe: bool) {
        let hwa = self.hwa();
        if hwa.is_null() {
            return;
        }
        let (pb_card, cap_card) = {
            let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            let pb = if !hws.hw.is_virtual && !hws.hw.is_bluetooth {
                let mut c = parse_card_idx_from_id(&c_str_to_string(&hws.active_playback_id));
                if c < 0 {
                    c = parse_card_idx_from_id(&c_str_to_string(&hws.hw.device_path));
                }
                c
            } else {
                -1
            };
            let cap = parse_card_idx_from_id(&c_str_to_string(&hws.active_capture_id));
            (pb, cap)
        };
        let cfg = {
            let ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
            ss.hw_analog_cfg
        };
        let bt_snap = snapshot_bt(self.bt());
        if re_probe {
            unsafe { lwasapi_hw_analog_probe(hwa, pb_card, cap_card) };
        }
        unsafe {
            lwasapi_hw_analog_sync(
                hwa,
                &cfg,
                bt_snap.desktop_volume_pct,
                bt_snap.desktop_muted,
                bt_snap.desktop_mic_volume_pct,
                bt_snap.desktop_mic_muted,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_playback_endpoint(
        &self,
        requested_id: &str,
        rate: u32,
        period: u32,
        req_bits: u32,
        force_virtual: bool,
        force_bt: bool,
        pre_dl: Option<&LwasapiDeviceList>,
    ) -> i32 {
        let req = if requested_id.is_empty() {
            "auto"
        } else {
            requested_id
        };
        let auto_mode = req == "auto";
        let want_bt = force_bt || req == "bluetooth" || req.starts_with("bluez:");
        let pref_bits = {
            let hws = self.hw_lock.lock().unwrap();
            hws.pref_format_bits
        };
        let target_bits = if matches!(req_bits, 16 | 24 | 32) {
            req_bits
        } else if matches!(pref_bits, 16 | 24 | 32) {
            pref_bits
        } else {
            32
        };

        // Close old kernel FD under hw_io_lock
        {
            let _io_guard = self.hw_io_lock.lock().unwrap();
            let old_fd = {
                let mut hws = self.hw_lock.lock().unwrap();
                let fd = hws.hw.fd;
                if fd >= 0 {
                    hws.hw.fd = -1;
                    hws.hw.is_virtual = true;
                    copy_str_to_c(&mut hws.hw.device_path, "WaveRT-Virtual-Clock");
                }
                fd
            };
            if old_fd >= 0 {
                close_raw_fd(old_fd);
            }
        }

        let mut next_hw = KernelWaveRtHw {
            fd: -1,
            device_path: [0; 64],
            is_virtual: false,
            is_bluetooth: false,
            sample_rate: rate,
            channels: 2,
            period_frames: period,
            buffer_frames: period.saturating_mul(4),
            sndrv_format: SNDRV_PCM_FORMAT_FLOAT_LE,
            format_bits: target_bits,
        };
        let next_id: String;
        let mut local_dl: LwasapiDeviceList = unsafe { mem::zeroed() };
        let dlp: Option<&LwasapiDeviceList> = if !force_virtual && req != "virtual" {
            Some(match pre_dl {
                Some(d) => d,
                None => {
                    unsafe {
                        lwasapi_bluez_enumerate_devices(
                            self.bt(),
                            ptr::null(),
                            ptr::null(),
                            &mut local_dl,
                        );
                    }
                    &local_dl
                }
            })
        } else {
            None
        };

        let mut bt_snap = snapshot_bt(self.bt());
        let known_bt_aliases = unsafe { lwasapi_bluez_known_audio_aliases(self.bt()) };
        let matching_usb_idx = dlp.and_then(|d| {
            let alias = c_str_to_string(&bt_snap.device_alias);
            find_matching_usb_pb_idx_for_list(d, &alias, &known_bt_aliases)
        });
        let skip_bt_for_matching_usb = auto_mode && !force_bt && matching_usb_idx.is_some();
        if skip_bt_for_matching_usb {
            if let (Some(d), Some(u_idx)) = (dlp, matching_usb_idx) {
                let u_name = c_str_to_string(&d.devices[u_idx].name);
                eprintln!(
                    "[LWASAPI-audiodg] Dual USB+BT device detected ({}) — prioritizing USB endpoint",
                    u_name
                );
                dbg_emit(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "USB_BT_PRIORITY",
                    &u_name,
                    "USB matches BT alias; prioritizing USB",
                );
            }
        }

        let any_bt_in_list = dlp.is_some_and(|d| device_list_has_type(d, "bluetooth", 0));
        let mut auto_want_bt = false;
        if !force_virtual && !skip_bt_for_matching_usb && (want_bt || auto_mode) {
            let bt_opt = unsafe { lwasapi_bluez_is_optional_mode(self.bt()) };
            let mut bt_ready =
                bt_snap.has_bluez_transport && bt_snap.direct_socket_mode && bt_snap.bt_fd >= 0;
            let mut bt_mac = c_str_to_string(&bt_snap.mac_address);
            let mut bt_alias = c_str_to_string(&bt_snap.device_alias);
            let mut bt_codec = c_str_to_string(&bt_snap.codec_name);
            let mut bt_rate = bt_snap.bt_sample_rate;

            let req_mac_ok = |mac_now: &str| -> bool {
                if let Some(req_mac) = req.strip_prefix("bluez:") {
                    mac_now.is_empty() || req_mac.eq_ignore_ascii_case(mac_now)
                } else {
                    true
                }
            };

            auto_want_bt = should_try_bt_on_apply(
                false,
                auto_mode,
                skip_bt_for_matching_usb,
                bt_opt,
                bt_snap.has_bluez_transport,
                any_bt_in_list,
                &bt_mac,
            );

            if bt_ready && !req_mac_ok(&bt_mac) {
                unsafe { lwasapi_bluez_set_tx_enabled(self.bt(), false) };
                bt_ready = false;
            }

            if !bt_ready && (want_bt || auto_want_bt) {
                unsafe { lwasapi_bluez_set_tx_enabled(self.bt(), true) };
                for _ in 0..140 {
                    std::thread::sleep(Duration::from_millis(25));
                    bt_snap = snapshot_bt(self.bt());
                    let mac_now = c_str_to_string(&bt_snap.mac_address);
                    if bt_snap.has_bluez_transport
                        && bt_snap.direct_socket_mode
                        && bt_snap.bt_fd >= 0
                        && req_mac_ok(&mac_now)
                    {
                        break;
                    }
                }
                bt_ready =
                    bt_snap.has_bluez_transport && bt_snap.direct_socket_mode && bt_snap.bt_fd >= 0;
                bt_mac = c_str_to_string(&bt_snap.mac_address);
                bt_alias = c_str_to_string(&bt_snap.device_alias);
                bt_codec = c_str_to_string(&bt_snap.codec_name);
                bt_rate = bt_snap.bt_sample_rate;
            }

            if bt_ready && !req_mac_ok(&bt_mac) {
                bt_ready = false;
            }
            if bt_ready {
                next_hw.is_bluetooth = true;
                let (label, mut pb_label) = format_bt_hw_label(
                    true,
                    &bt_alias,
                    &bt_mac,
                    &bt_codec,
                );
                if pb_label.is_empty() {
                    pb_label = if !bt_mac.is_empty() {
                        bt_mac.clone()
                    } else {
                        "bluetooth".to_string()
                    };
                }
                copy_str_to_c(&mut next_hw.device_path, &label);
                next_id = pb_label;
                unsafe {
                    lwasapi_bluez_reconfigure_rate(self.bt(), rate);
                    lwasapi_bluez_set_tx_enabled(self.bt(), true);
                }
                {
                    let mut hws = self.hw_lock.lock().unwrap();
                    hws.hw = next_hw;
                    copy_str_to_c(&mut hws.active_playback_id, &next_id);
                }
                let codec_show = if bt_codec.is_empty() { "A2DP" } else { &bt_codec };
                eprintln!(
                    "[LWASAPI-audiodg] Active Pure BlueZ A2DP endpoint: {} ({} Hz -> BT {} Hz {})",
                    label, next_hw.sample_rate, bt_rate, codec_show
                );
                dbg_emit(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "ENGINE_ENDPOINT",
                    &next_id,
                    &format!(
                        "driver=bluez-a2dp dev={} rate={} codec={} bt_rate={}",
                        label, next_hw.sample_rate, codec_show, bt_rate
                    ),
                );
                return 0;
            }
            if force_bt {
                eprintln!("[LWASAPI-audiodg] Notice: BlueZ A2DP transport not yet acquired, probing kernel endpoints...");
            }
        }

        // Preserve tx_paused=false when falling back transiently on explicit BT / connected-BT Auto so
        // an in-flight Acquire/ConnectProfile completes and sync_hotplug_topology restores BT automatically.
        if should_disable_bt_tx_on_fallback(want_bt, auto_want_bt) {
            unsafe { lwasapi_bluez_set_tx_enabled(self.bt(), false) };
        }

        if !force_virtual && !auto_mode && !want_bt && req != "virtual" {
            let dev_path = resolve_dev_id_to_path(req);
            if let Some(opened) = open_kernel_pcm_device(&dev_path, rate, period, target_bits) {
                next_hw = opened;
                next_id = if let Some((c, d)) = parse_pcm_card_dev(&dev_path) {
                    format!("hw:{c},{d}")
                } else {
                    req.to_string()
                };
                {
                    let mut hws = self.hw_lock.lock().unwrap();
                    hws.hw = next_hw;
                    copy_str_to_c(&mut hws.active_playback_id, &next_id);
                }
                let ms = (1000.0 * (next_hw.period_frames as f64)) / (next_hw.sample_rate as f64);
                eprintln!(
                    "[LWASAPI-audiodg] Opened direct kernel WaveRT endpoint: {} ({} Hz, {} ch, {}-bit, period={} frames / {:.2} ms)",
                    dev_path, next_hw.sample_rate, next_hw.channels, next_hw.format_bits, next_hw.period_frames, ms
                );
                dbg_emit(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "ENGINE_ENDPOINT",
                    &next_id,
                    &format!(
                        "driver=kernel-pcm req={} dev={} rate={} ch={} bits={} per={}",
                        req,
                        dev_path,
                        next_hw.sample_rate,
                        next_hw.channels,
                        next_hw.format_bits,
                        next_hw.period_frames
                    ),
                );
                return 0;
            }
            let err_str = std::io::Error::last_os_error().to_string();
            eprintln!(
                "[LWASAPI-audiodg] Notice: {} busy or unavailable ({}), probing fallback endpoints...",
                dev_path, err_str
            );
            dbg_emit(
                LWASAPI_DBG_ENGINE,
                LWASAPI_DBG_WARN,
                0,
                0,
                "ENDPOINT_BUSY",
                req,
                &format!("dev={} err={}", dev_path, err_str),
            );
        }

        if let Some(dlp) = dlp {
            let n = (dlp.num_devices as usize).min(LWASAPI_MAX_DEVICES);
            let mut cands: Vec<(usize, i32)> = Vec::new();
            for (i, e) in dlp.devices.iter().enumerate().take(n) {
                if e.is_capture != 0 || e.is_bluetooth != 0 {
                    continue;
                }
                let mut sc = unsafe { lwasapi_hw_analog_score_card(e.card_index, e.name.as_ptr().cast()) };
                if e.device_index == 0 {
                    sc += 10;
                }
                if matching_usb_idx == Some(i) {
                    sc += 50_000;
                }
                cands.push((i, sc));
            }
            cands.sort_by_key(|a| std::cmp::Reverse(a.1));

            for (idx, _) in cands {
                let e = &dlp.devices[idx];
                let dev_path = format!("/dev/snd/pcmC{}D{}p", e.card_index, e.device_index);
                if let Some(opened) = open_kernel_pcm_device(&dev_path, rate, period, target_bits) {
                    next_hw = opened;
                    next_id = c_str_to_string(&e.id);
                    let e_name = c_str_to_string(&e.name);
                    {
                        let mut hws = self.hw_lock.lock().unwrap();
                        hws.hw = next_hw;
                        copy_str_to_c(&mut hws.active_playback_id, &next_id);
                    }
                    let ms =
                        (1000.0 * (next_hw.period_frames as f64)) / (next_hw.sample_rate as f64);
                    eprintln!(
                        "[LWASAPI-audiodg] Auto-bound direct kernel WaveRT endpoint: {} ({} | {} Hz, {} ch, {}-bit, period={} frames / {:.2} ms)",
                        dev_path, e_name, next_hw.sample_rate, next_hw.channels, next_hw.format_bits, next_hw.period_frames, ms
                    );
                    dbg_emit(
                        LWASAPI_DBG_ENGINE,
                        LWASAPI_DBG_INFO,
                        0,
                        0,
                        "ENGINE_ENDPOINT",
                        &next_id,
                        &format!(
                            "driver=auto-kernel name={} dev={} rate={} ch={} bits={} per={}",
                            e_name,
                            dev_path,
                            next_hw.sample_rate,
                            next_hw.channels,
                            next_hw.format_bits,
                            next_hw.period_frames
                        ),
                    );
                    return 0;
                }
            }
        }

        // Virtual clock fallback
        next_hw.fd = -1;
        copy_str_to_c(&mut next_hw.device_path, "WaveRT-Virtual-Clock");
        next_hw.is_virtual = true;
        next_hw.is_bluetooth = false;
        next_hw.sample_rate = rate;
        next_hw.channels = 2;
        next_hw.period_frames = period;
        next_hw.buffer_frames = period.saturating_mul(4);
        next_hw.sndrv_format = SNDRV_PCM_FORMAT_FLOAT_LE;
        next_hw.format_bits = target_bits;
        {
            let mut hws = self.hw_lock.lock().unwrap();
            hws.hw = next_hw;
            copy_str_to_c(&mut hws.active_playback_id, "virtual");
        }
        let ms = (1000.0 * (next_hw.period_frames as f64)) / (next_hw.sample_rate as f64);
        eprintln!(
            "[LWASAPI-audiodg] Initialized deterministic WaveRT clock ({} Hz, period={} frames / {:.2} ms)",
            next_hw.sample_rate, next_hw.period_frames, ms
        );
        dbg_emit(
            LWASAPI_DBG_ENGINE,
            if force_virtual {
                LWASAPI_DBG_INFO
            } else {
                LWASAPI_DBG_WARN
            },
            0,
            0,
            "ENGINE_ENDPOINT",
            "virtual",
            &format!(
                "driver=virtual-clock req={} rate={} per={}",
                req, next_hw.sample_rate, next_hw.period_frames
            ),
        );
        0
    }

    fn init_hardware(
        &self,
        requested_dev: Option<&str>,
        rate: u32,
        period: u32,
        bits: u32,
        force_virtual: bool,
        force_bt: bool,
    ) {
        let _reconf = self.reconf_lock.lock().unwrap();
        let (target_cap, target_pb, pref_cap) = {
            let mut hws = self.hw_lock.lock().unwrap();
            hws.hw.fd = -1;
            if let Some(rd) = requested_dev {
                if !rd.is_empty() && rd != "auto" {
                    copy_str_to_c(&mut hws.target_playback_id, rd);
                }
            }
            (
                c_str_to_string(&hws.target_capture_id),
                c_str_to_string(&hws.target_playback_id),
                hws.pref_cap_sample_rate,
            )
        };

        if !force_virtual {
            let cap_rate = resolve_cap_sample_rate(rate, pref_cap);
            let c_cap = to_cstring_lossy(&target_cap);
            let c_pb = to_cstring_lossy(&target_pb);
            let (bt_cfg_saved, has_vol, d_vol, d_muted, m_vol, m_muted, m_gain, bt_optional) = {
                let sb = self.saved_bt.lock().unwrap();
                (
                    sb.saved_bt_cfg,
                    sb.has_saved_vol_state,
                    sb.saved_desktop_vol_pct,
                    sb.saved_desktop_muted,
                    sb.saved_mic_vol_pct,
                    sb.saved_mic_muted,
                    sb.saved_mic_gain_ppm,
                    sb.bt_optional_mode,
                )
            };
            unsafe {
                lwasapi_bluez_set_optional_mode(self.bt(), bt_optional != 0);
                lwasapi_bluez_init_rates(self.bt(), rate, cap_rate, c_cap.as_ptr(), c_pb.as_ptr());
                lwasapi_bluez_set_sco_enabled(self.bt(), bt_cfg_saved.hfp_sco_enabled != 0);
                lwasapi_bluez_set_codec_config(self.bt(), &bt_cfg_saved);
                if has_vol {
                    lwasapi_bluez_set_volume(
                        self.bt(),
                        d_vol as c_int,
                        if d_muted != 0 { 1 } else { 0 },
                    );
                    dbg_emit_vals(
                        LWASAPI_DBG_ENGINE,
                        LWASAPI_DBG_INFO,
                        0,
                        0,
                        d_vol as i32,
                        i32::from(d_muted != 0),
                        0,
                        "INIT_HW_VOL",
                        "master_out",
                        &format!("restored vol={d_vol}% muted={}", i32::from(d_muted != 0)),
                    );
                    lwasapi_bluez_set_mic_volume(
                        self.bt(),
                        m_vol as c_int,
                        if m_muted != 0 { 1 } else { 0 },
                    );
                    lwasapi_bluez_set_mic_gain_ppm(
                        self.bt(),
                        m_gain,
                        m_vol as c_int,
                        if m_muted != 0 { 1 } else { 0 },
                    );
                }
            }
            let bt_snap = snapshot_bt(self.bt());
            let cap_id = c_str_to_string(&bt_snap.cap_device_id);
            if !cap_id.is_empty() {
                let mut hws = self.hw_lock.lock().unwrap();
                copy_str_to_c(&mut hws.active_capture_id, &cap_id);
            }
        }

        self.apply_playback_endpoint(
            if force_bt { "bluetooth" } else { &target_pb },
            rate,
            period,
            bits,
            force_virtual,
            force_bt,
            None,
        );
        self.hw_analog_probe_and_sync(true);
    }

    fn fill_engine_status_impl(&self, st: &mut LwasapiEngineStatus, include_pa_sessions: bool) {
        *st = unsafe { mem::zeroed() };
        let bt_snap = snapshot_bt(self.bt());
        let bt_has_tr = bt_snap.has_bluez_transport;
        let bt_has_cap = bt_snap.has_capture_hw;
        let bt_cap_rate = bt_snap.cap_hw_rate;
        let bt_delay_tenths = bt_snap.delay_tenths_ms as u32;
        let bt_mac = c_str_to_string(&bt_snap.mac_address);
        let bt_alias = c_str_to_string(&bt_snap.device_alias);
        let bt_codec = c_str_to_string(&bt_snap.codec_name);
        let cap_alias = c_str_to_string(&bt_snap.capture_device_alias);

        let pref_cap = {
            let mut hws = self.hw_lock.lock().unwrap();
            if hws.hw.is_bluetooth && bt_has_tr {
                let (dev_lbl, pb_lbl) = format_bt_hw_label(true, &bt_alias, &bt_mac, &bt_codec);
                copy_str_to_c(&mut hws.hw.device_path, &dev_lbl);
                if !pb_lbl.is_empty() {
                    copy_str_to_c(&mut hws.active_playback_id, &pb_lbl);
                }
            }
            st.hw_device.copy_from_slice(&hws.hw.device_path);
            st.active_playback_id.copy_from_slice(&hws.active_playback_id);
            st.active_capture_id.copy_from_slice(&hws.active_capture_id);
            st.hw_sample_rate = hws.hw.sample_rate;
            st.hw_channels = hws.hw.channels;
            st.hw_period_frames = hws.hw.period_frames;
            st.hw_buffer_frames = hws.hw.buffer_frames;
            st.hw_format_bits = hws.hw.format_bits;
            st.bt_active = u32::from(hws.hw.is_bluetooth && bt_has_tr);
            hws.pref_cap_sample_rate
        };

        st.bt_delay_ms = bt_delay_tenths / 10;
        copy_str_to_c(&mut st.bt_alias, &bt_alias);
        copy_str_to_c(&mut st.bt_codec, &bt_codec);
        st.pa_server_running = u32::from(!self.pa().is_null());

        copy_str_to_c(
            &mut st.hw_capture_device,
            if cap_alias.is_empty() {
                "No Capture Device"
            } else {
                &cap_alias
            },
        );
        {
            let mm = self.midi_meta.lock().unwrap_or_else(|e| e.into_inner());
            let mc = c_str_to_string(&mm.controller_name);
            copy_str_to_c(
                &mut st.midi_device,
                if mc.is_empty() {
                    "LWASAPI-MIDI-Hub"
                } else {
                    &mc
                },
            );
        }

        st.desktop_volume_pct = bt_snap.desktop_volume_pct;
        st.desktop_muted = u32::from(bt_snap.desktop_muted);
        st.desktop_mic_volume_pct = bt_snap.desktop_mic_volume_pct;
        st.desktop_mic_muted = u32::from(bt_snap.desktop_mic_muted);
        st.total_cycles = self.total_cycles.load(Ordering::Relaxed);
        st.hw_xruns = self.hw_xruns.load(Ordering::Relaxed);
        st.midi_events_routed = self
            .midi_hw_events
            .load(Ordering::Relaxed)
            .saturating_add(self.midi_injected_events.load(Ordering::Relaxed));

        {
            let ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
            st.realtime_priority = self.rt_priority.load(Ordering::Relaxed);
            st.exclusive_owner_id = ss.exclusive_session_id;
            st.eq_cfg = ss.eq_cfg;
            st.param_eq = ss.param_eq;
            st.mic_apo = ss.mic_apo;
            unsafe {
                salsa_dsp_get_telem(
                    st.eq_band_levels_db.as_mut_ptr(),
                    st.eq_dyn_delta_db.as_mut_ptr(),
                    &mut st.output_peak_l,
                    &mut st.output_peak_r,
                    &mut st.mic_apo_peak_in,
                    &mut st.mic_apo_peak_out,
                    &mut st.mic_rnnoise_active,
                );
            }

            let mut count = 0usize;
            for slot in &ss.sessions {
                let r = slot.ring_ptr();
                if slot.in_use && !r.is_null() {
                    let info = &mut st.sessions[count];
                    count += 1;
                    unsafe {
                        info.session_id = slot.session_id;
                        info.client_pid = slot.client_pid;
                        info.is_capture = u32::from(slot.is_capture);
                        info.client_name.copy_from_slice(&slot.client_name);
                        info.share_mode = slot.share_mode;
                        info.sample_rate = slot.sample_rate;
                        info.channels = slot.channels;
                        info.active = (*ptr::addr_of!((*r).active)).load(Ordering::Relaxed);
                        info.volume_milli =
                            (*ptr::addr_of!((*r).volume_milli)).load(Ordering::Relaxed);
                        let (w, rd) = if info.is_capture != 0 {
                            (
                                (*ptr::addr_of!((*r).cap_write_frame)).load(Ordering::Relaxed),
                                (*ptr::addr_of!((*r).cap_read_frame)).load(Ordering::Relaxed),
                            )
                        } else {
                            (
                                (*ptr::addr_of!((*r).write_frame)).load(Ordering::Relaxed),
                                (*ptr::addr_of!((*r).read_frame)).load(Ordering::Relaxed),
                            )
                        };
                        info.buffered_frames = w.saturating_sub(rd);
                        info.xruns = (*ptr::addr_of!((*r).xruns)).load(Ordering::Relaxed);
                        info.muted = (*ptr::addr_of!((*r).muted)).load(Ordering::Relaxed);
                        info.routing_flags = 0;
                        if info.is_capture != 0 {
                            info.target_device.copy_from_slice(&st.active_capture_id);
                        } else {
                            info.target_device.copy_from_slice(&st.active_playback_id);
                        }
                    }
                }
            }
            st.num_sessions = count as u32;
            st.hw_analog_cfg = ss.hw_analog_cfg;
        }

        let hwa = self.hwa();
        if !hwa.is_null() {
            unsafe { lwasapi_hw_analog_get_info(hwa, &mut st.hw_analog_info) };
        }
        st.hw_analog_info.reserved = if bt_has_cap && bt_cap_rate >= 8000 {
            bt_cap_rate
        } else {
            resolve_cap_sample_rate(st.hw_sample_rate, pref_cap)
        };

        if bt_snap.initialized {
            unsafe { lwasapi_bluez_get_codec_config(self.bt(), &mut st.bt_codec_cfg) };
        } else {
            let sb = self.saved_bt.lock().unwrap_or_else(|e| e.into_inner());
            st.bt_codec_cfg = sb.saved_bt_cfg;
        }

        let pa = self.pa();
        if include_pa_sessions && !pa.is_null() {
            unsafe { lwasapi_pa_server_fill_sessions(pa, st, &mut st.pa_clients) };
        }
    }

    fn enumerate_devices_safe(&self, out_list: &mut LwasapiDeviceList) {
        let (pb_id, cap_id) = {
            let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            (
                c_str_to_string(&hws.active_playback_id),
                c_str_to_string(&hws.active_capture_id),
            )
        };
        let c_pb = to_cstring_lossy(&pb_id);
        let c_cap = to_cstring_lossy(&cap_id);
        unsafe {
            lwasapi_bluez_enumerate_devices(self.bt(), c_pb.as_ptr(), c_cap.as_ptr(), out_list);
        }
    }

    fn switch_engine_devices_impl(
        &self,
        pb_id: Option<&str>,
        cap_id: Option<&str>,
        notify_pa: bool,
    ) -> i32 {
        let _reconf = self.reconf_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (rate, period, bits, mut hw_rate, pref_cap, target_pb, target_cap) = {
            let mut hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            let rate = if hws.pref_sample_rate != 0 {
                hws.pref_sample_rate
            } else if hws.hw.sample_rate != 0 {
                hws.hw.sample_rate
            } else {
                48_000
            };
            let period = if hws.pref_period_frames != 0 {
                hws.pref_period_frames
            } else if hws.hw.period_frames != 0 {
                hws.hw.period_frames
            } else {
                256
            };
            let bits = if hws.pref_format_bits != 0 {
                hws.pref_format_bits
            } else if hws.hw.format_bits != 0 {
                hws.hw.format_bits
            } else {
                32
            };
            let hw_rate = if hws.hw.sample_rate != 0 {
                hws.hw.sample_rate
            } else {
                rate
            };
            let mut t_pb = String::new();
            let mut t_cap = String::new();
            if let Some(p) = pb_id {
                if !p.is_empty() {
                    copy_str_to_c(&mut hws.target_playback_id, p);
                    t_pb = c_str_to_string(&hws.target_playback_id);
                }
            }
            if let Some(c) = cap_id {
                if !c.is_empty() {
                    copy_str_to_c(&mut hws.target_capture_id, c);
                    t_cap = c_str_to_string(&hws.target_capture_id);
                }
            }
            (rate, period, bits, hw_rate, hws.pref_cap_sample_rate, t_pb, t_cap)
        };

        if !target_pb.is_empty() {
            let bt_snap = snapshot_bt(self.bt());
            if bt_snap.initialized {
                let c_pb = to_cstring_lossy(&target_pb);
                unsafe { lwasapi_bluez_set_preferred_target(self.bt(), c_pb.as_ptr()) };
            }
            self.apply_playback_endpoint(&target_pb, rate, period, bits, false, false, None);
            let hws = self.hw_lock.lock().unwrap();
            if hws.hw.sample_rate != 0 {
                hw_rate = hws.hw.sample_rate;
            }
        }

        if !target_cap.is_empty() {
            let c_cap = to_cstring_lossy(&target_cap);
            let eff_cap = resolve_cap_sample_rate(hw_rate, pref_cap);
            unsafe { lwasapi_bluez_open_capture(self.bt(), c_cap.as_ptr(), eff_cap) };
            let bt_snap = snapshot_bt(self.bt());
            let cap_id_now = c_str_to_string(&bt_snap.cap_device_id);
            let mut hws = self.hw_lock.lock().unwrap();
            copy_str_to_c(
                &mut hws.active_capture_id,
                if cap_id_now.is_empty() {
                    "none"
                } else {
                    &cap_id_now
                },
            );
        }

        self.hw_analog_probe_and_sync(true);
        self.save_state();
        drop(_reconf);

        let pa = self.pa();
        if notify_pa && !pa.is_null() {
            unsafe { lwasapi_pa_server_notify_change(pa) };
        }
        0
    }

    fn reconfigure_engine_rate_impl(
        &self,
        mut new_rate: u32,
        mut new_period: u32,
        mut new_bits: u32,
        update_pref: bool,
    ) -> Result<(LwasapiIpcResp, u32), i32> {
        let _reconf = self.reconf_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (rate_changed, period_changed, bits_changed, is_hw_kernel, was_virtual, target_pb, target_cap, pref_cap) = {
            let mut hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            let cur_rate = hws.hw.sample_rate;
            let cur_period = hws.hw.period_frames;
            let cur_bits = hws.hw.format_bits;
            if new_rate == 0 {
                new_rate = cur_rate;
            }
            if new_period == 0 {
                new_period = cur_period;
            }
            if !matches!(new_bits, 16 | 24 | 32) {
                new_bits = cur_bits;
            }
            if !(44_100..=192_000).contains(&new_rate) || !(64..=LWASAPI_MAX_PERIOD as u32).contains(&new_period) {
                return Err(-1);
            }
            let was_virtual =
                hws.hw.is_virtual && c_str_to_string(&hws.active_playback_id) == "virtual";
            let is_hw_kernel = !hws.hw.is_virtual && !hws.hw.is_bluetooth;
            if update_pref {
                hws.pref_sample_rate = new_rate;
                hws.pref_period_frames = new_period;
                hws.pref_format_bits = new_bits;
            }
            (
                new_rate != cur_rate,
                new_period != cur_period,
                new_bits != cur_bits,
                is_hw_kernel,
                was_virtual,
                c_str_to_string(&hws.target_playback_id),
                c_str_to_string(&hws.target_capture_id),
                hws.pref_cap_sample_rate,
            )
        };

        if rate_changed || period_changed || (bits_changed && is_hw_kernel) {
            self.apply_playback_endpoint(&target_pb, new_rate, new_period, new_bits, was_virtual, false, None);
        }

        let (mut resp, actual_bits) = {
            let mut hws = self.hw_lock.lock().unwrap();
            if (hws.hw.is_virtual || hws.hw.is_bluetooth) && matches!(new_bits, 16 | 24 | 32) {
                hws.hw.format_bits = new_bits;
            }
            let resp = LwasapiIpcResp {
                hr: S_OK,
                session_id: 0,
                actual_sample_rate: hws.hw.sample_rate,
                actual_channels: hws.hw.channels,
                actual_buffer_frames: hws.hw.buffer_frames,
                actual_period_frames: hws.hw.period_frames,
            };
            (resp, hws.hw.format_bits)
        };
        resp.hr = 0;

        let mut bt_snap = snapshot_bt(self.bt());
        if rate_changed && bt_snap.initialized {
            unsafe { lwasapi_bluez_reconfigure_rate(self.bt(), resp.actual_sample_rate) };
            bt_snap = snapshot_bt(self.bt());
            let cap_already_open = bt_snap.has_capture_hw && bt_snap.cap_pcm_fd >= 0;
            if !cap_already_open {
                let c_cap = to_cstring_lossy(&target_cap);
                unsafe {
                    lwasapi_bluez_open_capture(
                        self.bt(),
                        c_cap.as_ptr(),
                        resolve_cap_sample_rate(resp.actual_sample_rate, pref_cap),
                    );
                }
                bt_snap = snapshot_bt(self.bt());
            }
            let cap_id_now = c_str_to_string(&bt_snap.cap_device_id);
            if !cap_id_now.is_empty() {
                let mut hws = self.hw_lock.lock().unwrap();
                copy_str_to_c(&mut hws.active_capture_id, &cap_id_now);
            }
        }

        {
            let ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
            for slot in &ss.sessions {
                let r = slot.ring_ptr();
                if slot.in_use && !r.is_null() {
                    unsafe { ptr::addr_of_mut!((*r).period_frames).write(resp.actual_period_frames) };
                }
            }
        }

        if rate_changed || period_changed || bits_changed {
            self.hw_analog_probe_and_sync(true);
        }
        self.save_state();
        drop(_reconf);

        let pa = self.pa();
        if !pa.is_null() {
            unsafe { lwasapi_pa_server_notify_change(pa) };
        }
        Ok((resp, actual_bits))
    }

    fn reconfigure_capture_rate_impl(&self, new_cap_rate: u32) -> Option<LwasapiIpcResp> {
        if new_cap_rate != 0 && !(44_100..=192_000).contains(&new_cap_rate) {
            return None;
        }
        let _reconf = self.reconf_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (eng_rate, cur_period, cur_buf, target_cap) = {
            let mut hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            hws.pref_cap_sample_rate = new_cap_rate;
            (
                if hws.hw.sample_rate != 0 {
                    hws.hw.sample_rate
                } else {
                    48_000
                },
                hws.hw.period_frames,
                hws.hw.buffer_frames,
                c_str_to_string(&hws.target_capture_id),
            )
        };

        let eff_cap = resolve_cap_sample_rate(eng_rate, new_cap_rate);
        let mut has_cap_now = false;
        let mut cap_rate_now = eff_cap;
        let mut cap_ch_now = 2u32;

        let bt_init = snapshot_bt(self.bt()).initialized;
        if bt_init {
            let c_cap = to_cstring_lossy(if target_cap.is_empty() {
                "auto"
            } else {
                &target_cap
            });
            unsafe { lwasapi_bluez_open_capture(self.bt(), c_cap.as_ptr(), eff_cap) };
            let bt_snap = snapshot_bt(self.bt());
            has_cap_now = bt_snap.has_capture_hw;
            cap_rate_now = bt_snap.cap_hw_rate;
            cap_ch_now = if bt_snap.cap_hw_channels != 0 {
                bt_snap.cap_hw_channels
            } else {
                2
            };
            let cap_id_now = c_str_to_string(&bt_snap.cap_device_id);
            let mut hws = self.hw_lock.lock().unwrap();
            copy_str_to_c(
                &mut hws.active_capture_id,
                if cap_id_now.is_empty() {
                    "none"
                } else {
                    &cap_id_now
                },
            );
        }

        self.hw_analog_probe_and_sync(true);
        self.save_state();
        drop(_reconf);

        let pa = self.pa();
        if !pa.is_null() {
            unsafe { lwasapi_pa_server_notify_change(pa) };
        }

        Some(LwasapiIpcResp {
            hr: S_OK,
            session_id: 0,
            actual_sample_rate: if has_cap_now && cap_rate_now != 0 {
                cap_rate_now
            } else {
                eff_cap
            },
            actual_channels: cap_ch_now,
            actual_buffer_frames: cur_buf,
            actual_period_frames: cur_period,
        })
    }

    fn sync_hotplug_topology(&self, force_virtual: bool, bt_now: bool) -> bool {
        let mut cap_deferred = false;
        let _reconf = self.reconf_lock.lock().unwrap();
        let mut dlist: LwasapiDeviceList = unsafe { mem::zeroed() };
        self.enumerate_devices_safe(&mut dlist);
        let bt_has_cap = snapshot_bt(self.bt()).has_capture_hw;

        let (pref_r, pref_p, pref_b, mut eng_rate, pref_cap, hw_is_virtual, hw_is_bt, target_pb, active_pb, target_cap, active_cap) = {
            let hws = self.hw_lock.lock().unwrap();
            (
                if hws.pref_sample_rate != 0 { hws.pref_sample_rate } else { hws.hw.sample_rate },
                if hws.pref_period_frames != 0 { hws.pref_period_frames } else { hws.hw.period_frames },
                if hws.pref_format_bits != 0 { hws.pref_format_bits } else { hws.hw.format_bits },
                if hws.hw.sample_rate != 0 { hws.hw.sample_rate } else { 48_000 },
                hws.pref_cap_sample_rate,
                hws.hw.is_virtual,
                hws.hw.is_bluetooth,
                c_str_to_string(&hws.target_playback_id),
                c_str_to_string(&hws.active_playback_id),
                c_str_to_string(&hws.target_capture_id),
                c_str_to_string(&hws.active_capture_id),
            )
        };

        if !force_virtual && target_pb != "virtual" {
            let target_auto = target_pb == "auto";
            let target_is_bt = target_pb == "bluetooth" || target_pb.starts_with("bluez");
            let active_kernel_lost =
                !hw_is_virtual && !hw_is_bt && !device_list_has_type(&dlist, &active_pb, 0);
            let active_bt_lost = hw_is_bt && !bt_now;
            let target_returned = !target_auto
                && (!target_is_bt || bt_now)
                && active_pb != target_pb
                && device_list_has_type(&dlist, &target_pb, 0);
            let usb_bt_active = unsafe { any_usb_matches_bt(&dlist, self.bt()) };
            let auto_upgrade_bt = target_auto && !hw_is_bt && bt_now && !usb_bt_active;
            let auto_upgrade_matching_usb = target_auto && hw_is_bt && usb_bt_active;
            let auto_recover_hw =
                target_auto && hw_is_virtual && device_list_has_any_type(&dlist, 0, false);
            let auto_upgrade_usb = target_auto
                && !hw_is_bt
                && !dev_id_is_usb_card(&active_pb)
                && device_list_has_any_type(&dlist, 0, true);

            if target_returned {
                eprintln!(
                    "[LWASAPI-audiodg] Hotplug restored preferred playback endpoint: {}",
                    target_pb
                );
                dbg_emit(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "HOTPLUG_PB",
                    &target_pb,
                    "restored preferred target",
                );
                self.apply_playback_endpoint(&target_pb, pref_r, pref_p, pref_b, false, false, Some(&dlist));
            } else if active_kernel_lost
                || active_bt_lost
                || auto_upgrade_bt
                || auto_upgrade_matching_usb
                || auto_recover_hw
                || auto_upgrade_usb
            {
                eprintln!("[LWASAPI-audiodg] Hotplug playback change detected, re-evaluating auto route...");
                let tgt = if target_auto { "auto" } else { &target_pb };
                dbg_emit(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "HOTPLUG_PB",
                    tgt,
                    &format!(
                        "lost={} btlost={} upg_bt={} upg_usb_bt={} rec={} usb={}",
                        i32::from(active_kernel_lost),
                        i32::from(active_bt_lost),
                        i32::from(auto_upgrade_bt),
                        i32::from(auto_upgrade_matching_usb),
                        i32::from(auto_recover_hw),
                        i32::from(auto_upgrade_usb)
                    ),
                );
                self.apply_playback_endpoint(tgt, pref_r, pref_p, pref_b, false, false, Some(&dlist));
            }
            let hws = self.hw_lock.lock().unwrap();
            if hws.hw.sample_rate != 0 {
                eng_rate = hws.hw.sample_rate;
            }
        }

        if !force_virtual {
            let cap_none = active_cap == "none" || active_cap.is_empty();
            let cap_still_plugged =
                !cap_none && bt_has_cap && device_list_has_type(&dlist, &active_cap, 1);
            let target_cap_auto = target_cap == "auto" || target_cap.is_empty();
            let target_cap_returned = !target_cap_auto
                && active_cap != target_cap
                && device_list_has_type(&dlist, &target_cap, 1);
            let cap_appeared = cap_none && device_list_has_any_type(&dlist, 1, false);
            let cap_upgrade_usb = target_cap_auto
                && cap_still_plugged
                && !dev_id_is_usb_card(&active_cap)
                && device_list_has_any_type(&dlist, 1, true);

            let cap_lost = !cap_none && !cap_still_plugged;
            let cap_new_usb = (cap_appeared || target_cap_returned || cap_upgrade_usb)
                && device_list_has_any_type(&dlist, 1, true);
            if !cap_new_usb {
                self.usb_cap_appear_ns.store(0, Ordering::Relaxed);
            }
            if cap_lost {
                self.usb_cap_appear_ns.store(0, Ordering::Relaxed);
                let mut hws = self.hw_lock.lock().unwrap();
                copy_str_to_c(&mut hws.active_capture_id, "none");
            }
            if cap_new_usb {
                let now_ns = mono_ns();
                let first_ns = self.usb_cap_appear_ns.load(Ordering::Relaxed);
                if first_ns == 0 {
                    self.usb_cap_appear_ns.store(now_ns, Ordering::Relaxed);
                    cap_deferred = true;
                } else if now_ns.saturating_sub(first_ns) < 500_000_000 {
                    cap_deferred = true;
                } else {
                    self.usb_cap_appear_ns.store(0, Ordering::Relaxed);
                }
            }

            if !cap_deferred && (cap_lost || cap_appeared || target_cap_returned || cap_upgrade_usb) {
                let req_cap = if target_cap.is_empty() {
                    "auto"
                } else {
                    &target_cap
                };
                let c_req_cap = to_cstring_lossy(req_cap);
                let cap_rc = unsafe {
                    lwasapi_bluez_open_capture(
                        self.bt(),
                        c_req_cap.as_ptr(),
                        resolve_cap_sample_rate(eng_rate, pref_cap),
                    )
                };
                let bt_snap = snapshot_bt(self.bt());
                let cap_id_now = c_str_to_string(&bt_snap.cap_device_id);
                let new_cap_id = if cap_rc == 0 && bt_snap.has_capture_hw && !cap_id_now.is_empty() {
                    cap_id_now
                } else {
                    "none".to_string()
                };
                {
                    let mut hws = self.hw_lock.lock().unwrap();
                    copy_str_to_c(&mut hws.active_capture_id, &new_cap_id);
                }
                dbg_emit(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "HOTPLUG_CAP",
                    &new_cap_id,
                    &format!("req={} active={}", req_cap, i32::from(bt_snap.has_capture_hw)),
                );
            }
        }

        self.hw_analog_probe_and_sync(true);
        cap_deferred
    }

    fn handle_create_session(&self, client_sock: UnixStream, req: &LwasapiIpcReq) {
        let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
        let req_name = c_str_to_string(&req.client_name);
        let cname_dbg = if req_name.is_empty() {
            "session".to_string()
        } else {
            req_name.clone()
        };

        let Some(shm_fd) = create_memfd("lwasapi_ring", mem::size_of::<LwasapiShmRing>()) else {
            resp.hr = E_OUTOFMEMORY;
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), as_bytes(&resp), &[]);
            return;
        };
        let Some(ev_fd) = new_eventfd() else {
            resp.hr = E_OUTOFMEMORY;
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), as_bytes(&resp), &[]);
            return;
        };
        let Some(ring_map) =
            MappedMem::map_fd(shm_fd.as_raw_fd(), mem::size_of::<LwasapiShmRing>(), true, 0)
        else {
            resp.hr = E_OUTOFMEMORY;
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), as_bytes(&resp), &[]);
            return;
        };

        unsafe {
            ptr::write_bytes(ring_map.as_ptr() as *mut u8, 0, offset_of!(LwasapiShmRing, pcm_data));
        }

        let (hw_rate, hw_ch, hw_per) = {
            let hws = self.hw_lock.lock().unwrap();
            (hws.hw.sample_rate, hws.hw.channels, hws.hw.period_frames)
        };

        let mut ss = self.sessions_lock.lock().unwrap();
        if ss.exclusive_session_id >= 0 {
            resp.hr = AUDCLNT_E_DEVICE_IN_USE;
            let holder = ss.exclusive_session_id;
            drop(ss);
            dbg_emit_vals(
                LWASAPI_DBG_WASAPI,
                LWASAPI_DBG_WARN,
                req.client_pid,
                0,
                holder,
                0,
                resp.hr,
                "EXCL_DENIED",
                &cname_dbg,
                &format!("exclusive lock held by session #{holder}"),
            );
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), as_bytes(&resp), &[]);
            return;
        }

        let Some(slot_idx) = ss.sessions.iter().position(|s| !s.in_use) else {
            resp.hr = E_OUTOFMEMORY;
            drop(ss);
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), as_bytes(&resp), &[]);
            return;
        };

        let ring = ring_map.as_ptr() as *mut LwasapiShmRing;
        let sid = ss.next_session_id;
        ss.next_session_id = ss.next_session_id.wrapping_add(1);

        let req_rate = if (8_000..=768_000).contains(&req.sample_rate) {
            req.sample_rate
        } else {
            hw_rate
        };
        let eff_rate = if req.share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
            hw_rate
        } else if req.sample_rate != 0 {
            req_rate
        } else {
            hw_rate
        };
        let mut eff_ch = if req.channels != 0 && (req.channels as usize) <= LWASAPI_MAX_CHANNELS {
            req.channels
        } else {
            hw_ch
        };
        if eff_ch == 0 || (eff_ch as usize) > LWASAPI_MAX_CHANNELS {
            eff_ch = 2;
        }
        let mut buf_frames = req.requested_buffer_frames;
        if buf_frames < hw_per.saturating_mul(2) {
            buf_frames = hw_per.saturating_mul(4);
        }
        if buf_frames > LWASAPI_RING_FRAMES as u32 {
            buf_frames = LWASAPI_RING_FRAMES as u32;
        }
        let is_cap_u32 = u32::from((req.stream_flags & AUDCLNT_STREAMFLAGS_CAPTURE) != 0);
        let s_name_str = if req_name.is_empty() {
            "AudioSession"
        } else {
            &req_name
        };

        let sock_raw = client_sock.as_raw_fd();
        let auth_pid = socket_peercred(sock_raw)
            .and_then(|uc| (uc.pid > 0).then_some(uc.pid as u32))
            .unwrap_or(req.client_pid);

        unsafe {
            ptr::addr_of_mut!((*ring).magic).write(LWASAPI_RING_MAGIC);
            ptr::addr_of_mut!((*ring).session_id).write(sid);
            ptr::addr_of_mut!((*ring).client_pid).write(auth_pid);
            copy_str_to_c(&mut *ptr::addr_of_mut!((*ring).client_name), s_name_str);
            ptr::addr_of_mut!((*ring).share_mode).write(req.share_mode);
            ptr::addr_of_mut!((*ring).stream_flags).write(req.stream_flags);
            ptr::addr_of_mut!((*ring).is_capture).write(is_cap_u32);
            ptr::addr_of_mut!((*ring).sample_rate).write(eff_rate);
            ptr::addr_of_mut!((*ring).channels).write(eff_ch);
            ptr::addr_of_mut!((*ring).format).write(LWASAPI_FMT_FLOAT32);
            ptr::addr_of_mut!((*ring).period_frames).write(hw_per);
            ptr::addr_of_mut!((*ring).buffer_frames).write(buf_frames);
            (*ptr::addr_of!((*ring).volume_milli)).store(1000, Ordering::SeqCst);
        }

        seal_memfd_size(shm_fd.as_raw_fd());
        let fds_to_send = [shm_fd.as_raw_fd(), ev_fd.as_raw_fd()];

        let mut auth_name = [0u8; 64];
        copy_str_to_c(&mut auth_name, s_name_str);

        ss.sessions[slot_idx] = EngineSession {
            in_use: true,
            session_id: sid,
            client_pid: auth_pid,
            client_name: auth_name,
            share_mode: req.share_mode,
            client_sock: Some(client_sock),
            _shm_fd: Some(shm_fd),
            event_fd: Some(ev_fd),
            ring_map: Some(ring_map),
            resample_phase: 0.0,
            sample_rate: eff_rate,
            channels: eff_ch,
            is_capture: is_cap_u32 != 0,
            is_loopback: (req.stream_flags & AUDCLNT_STREAMFLAGS_LOOPBACK) != 0,
            stream_live: false,
            underrun_pending: false,
            last_audio_ns: 0,
        };
        if req.share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
            ss.exclusive_session_id = sid as i32;
        }

        resp.hr = S_OK;
        resp.session_id = sid;
        resp.actual_sample_rate = eff_rate;
        resp.actual_channels = eff_ch;
        resp.actual_buffer_frames = buf_frames;
        resp.actual_period_frames = hw_per;
        drop(ss);

        if sendmsg_all_with_fds(sock_raw, as_bytes(&resp), &fds_to_send).is_err() {
            let mut ss = self.sessions_lock.lock().unwrap();
            if ss.sessions[slot_idx].in_use && ss.sessions[slot_idx].session_id == sid {
                destroy_session_slot(&mut ss, slot_idx);
            }
            return;
        }

        if req.share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
            eprintln!(
                "[LWASAPI-audiodg] Session #{} ('{}', PID {}) acquired WASAPI EXCLUSIVE hardware lock ({} Hz, period={})",
                sid, s_name_str, auth_pid, eff_rate, hw_per
            );
        } else {
            eprintln!(
                "[LWASAPI-audiodg] Session #{} ('{}', PID {}) initialized in WASAPI {} mode ({} Hz -> {} Hz, {} ch, buf={})",
                sid,
                s_name_str,
                auth_pid,
                if is_cap_u32 != 0 { "CAPTURE" } else { "SHARED" },
                eff_rate,
                hw_rate,
                eff_ch,
                buf_frames
            );
        }

        dbg_emit_vals(
            LWASAPI_DBG_WASAPI,
            LWASAPI_DBG_INFO,
            req.client_pid,
            sid,
            eff_rate as i32,
            eff_ch as i32,
            S_OK,
            "WASAPI_SESSION",
            s_name_str,
            &format!(
                "mode={} cap={} rate={} ch={} buf={} per={}",
                if req.share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
                    "EXCLUSIVE"
                } else {
                    "SHARED"
                },
                is_cap_u32,
                eff_rate,
                eff_ch,
                buf_frames,
                hw_per
            ),
        );
    }

    fn handle_client_cmd(&self, mut stream: UnixStream, req: &LwasapiIpcReq) {
        match req.cmd {
            LWASAPI_CMD_CREATE_SESSION => self.handle_create_session(stream, req),
            LWASAPI_CMD_GET_STATUS => {
                let mut st: LwasapiEngineStatus = unsafe { mem::zeroed() };
                self.fill_engine_status_impl(&mut st, true);
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&st));
            }
            LWASAPI_CMD_LIST_DEVICES => {
                let mut dlist: LwasapiDeviceList = unsafe { mem::zeroed() };
                self.enumerate_devices_safe(&mut dlist);
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&dlist));
            }
            LWASAPI_CMD_SET_DEVICES => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let pb_s = c_str_to_string(&req.target_device);
                let cap_s = c_str_to_string(&req.target_capture_device);
                let pb = (!pb_s.is_empty()).then_some(pb_s.as_str());
                let cap = (!cap_s.is_empty()).then_some(cap_s.as_str());
                if pb.is_none() && cap.is_none() {
                    resp.hr = E_INVALIDARG;
                } else {
                    self.switch_engine_devices_impl(pb, cap, true);
                    let hws = self.hw_lock.lock().unwrap();
                    resp.hr = S_OK;
                    resp.actual_sample_rate = hws.hw.sample_rate;
                    resp.actual_channels = hws.hw.channels;
                    resp.actual_period_frames = hws.hw.period_frames;
                    resp.actual_buffer_frames = hws.hw.buffer_frames;
                }
                let tgt = pb.or(cap).unwrap_or("none");
                dbg_emit_vals(
                    LWASAPI_DBG_ENGINE,
                    if resp.hr == S_OK { LWASAPI_DBG_INFO } else { LWASAPI_DBG_WARN },
                    req.client_pid,
                    0,
                    resp.actual_sample_rate as i32,
                    resp.actual_period_frames as i32,
                    resp.hr,
                    "DEV_SWITCH",
                    tgt,
                    &format!(
                        "pb={} cap={} -> {}Hz/{}",
                        pb.unwrap_or("-"),
                        cap.unwrap_or("-"),
                        resp.actual_sample_rate,
                        resp.actual_period_frames
                    ),
                );
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_VOLUME => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                resp.hr = E_INVALIDARG;
                if req.target_session_id == 0xFFFF_FFFF {
                    let vol_pct = if req.volume_milli <= 1500 {
                        (req.volume_milli / 10) as c_int
                    } else {
                        -1
                    };
                    let mute_flag = (req.share_mode as c_int).wrapping_sub(1);
                    unsafe { lwasapi_bluez_set_volume(self.bt(), vol_pct, mute_flag) };
                    self.hw_analog_probe_and_sync(false);
                    let pa = self.pa();
                    if !pa.is_null() {
                        unsafe { lwasapi_pa_server_notify_volume(pa, false) };
                    }
                    self.mark_state_dirty();
                    resp.hr = S_OK;
                    let cname = c_str_to_string(&req.client_name);
                    let src = if cname.is_empty() { "master_out" } else { &cname };
                    dbg_emit_vals(
                        LWASAPI_DBG_ENGINE,
                        LWASAPI_DBG_INFO,
                        req.client_pid,
                        0,
                        vol_pct,
                        mute_flag,
                        resp.hr,
                        "VOL_CHG",
                        src,
                        &format!("master_out vol={vol_pct}% mute={mute_flag}"),
                    );
                } else if req.target_session_id == 0xFFFF_FFFE {
                    let vol_pct = if req.volume_milli <= 1500 {
                        (req.volume_milli / 10) as c_int
                    } else {
                        -1
                    };
                    let mute_flag = (req.share_mode as c_int).wrapping_sub(1);
                    unsafe { lwasapi_bluez_set_mic_volume(self.bt(), vol_pct, mute_flag) };
                    self.hw_analog_probe_and_sync(false);
                    let pa = self.pa();
                    if !pa.is_null() {
                        unsafe { lwasapi_pa_server_notify_volume(pa, true) };
                    }
                    self.mark_state_dirty();
                    resp.hr = S_OK;
                    dbg_emit_vals(
                        LWASAPI_DBG_ENGINE,
                        LWASAPI_DBG_INFO,
                        req.client_pid,
                        0,
                        vol_pct,
                        mute_flag,
                        resp.hr,
                        "VOL_CHG",
                        "master_mic",
                        &format!("vol={vol_pct}% mute={mute_flag}"),
                    );
                } else {
                    let mut found = false;
                    {
                        let ss = self.sessions_lock.lock().unwrap();
                        for slot in &ss.sessions {
                            let r = slot.ring_ptr();
                            if slot.in_use && slot.session_id == req.target_session_id && !r.is_null() {
                                unsafe {
                                    (*ptr::addr_of!((*r).volume_milli))
                                        .store(req.volume_milli, Ordering::SeqCst);
                                }
                                resp.hr = S_OK;
                                found = true;
                                break;
                            }
                        }
                    }
                    let pa = self.pa();
                    if !found
                        && !pa.is_null()
                        && unsafe {
                            lwasapi_pa_server_set_stream_volume(
                                pa,
                                req.target_session_id,
                                req.volume_milli / 10,
                            ) == 0
                        }
                    {
                        resp.hr = S_OK;
                    }
                    dbg_emit_vals(
                        LWASAPI_DBG_WASAPI,
                        if resp.hr == S_OK { LWASAPI_DBG_INFO } else { LWASAPI_DBG_WARN },
                        req.client_pid,
                        req.target_session_id,
                        req.volume_milli as i32,
                        i32::from(found),
                        resp.hr,
                        "VOL_CHG",
                        "stream",
                        &format!("milli={} wasapi={}", req.volume_milli, i32::from(found)),
                    );
                }
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_SESSION_CONTROL => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                resp.hr = E_INVALIDARG;
                let tgt_dev = c_str_to_string(&req.target_device);
                if req.target_session_id < 10_000 {
                    let ss = self.sessions_lock.lock().unwrap();
                    for slot in &ss.sessions {
                        let r = slot.ring_ptr();
                        if slot.in_use && slot.session_id == req.target_session_id && !r.is_null() {
                            unsafe {
                                if req.volume_milli <= 1500 {
                                    (*ptr::addr_of!((*r).volume_milli))
                                        .store(req.volume_milli, Ordering::SeqCst);
                                }
                                let muted_at = &*ptr::addr_of!((*r).muted);
                                match req.share_mode {
                                    1 => muted_at.store(0, Ordering::SeqCst),
                                    2 => muted_at.store(1, Ordering::SeqCst),
                                    3 => {
                                        let cur = muted_at.load(Ordering::SeqCst);
                                        muted_at.store(u32::from(cur == 0), Ordering::SeqCst);
                                    }
                                    _ => {}
                                }
                            }
                            resp.hr = S_OK;
                            break;
                        }
                    }
                } else {
                    let pa = self.pa();
                    if !pa.is_null() {
                        let vol_pct = if req.volume_milli <= 1500 {
                            (req.volume_milli / 10) as i32
                        } else {
                            -1
                        };
                        let mute_mode = if (1..=3).contains(&req.share_mode) {
                            (req.share_mode - 1) as i32
                        } else {
                            -1
                        };
                        let c_dev = to_cstring_lossy(&tgt_dev);
                        let dev_ptr = if tgt_dev.is_empty() {
                            ptr::null()
                        } else {
                            c_dev.as_ptr()
                        };
                        if unsafe {
                            lwasapi_pa_server_control_stream(
                                pa,
                                req.target_session_id,
                                vol_pct,
                                mute_mode,
                                dev_ptr,
                            ) == 0
                        } {
                            resp.hr = S_OK;
                        }
                    }
                }
                let tshow = if tgt_dev.is_empty() { "-" } else { &tgt_dev };
                dbg_emit_vals(
                    LWASAPI_DBG_WASAPI,
                    if resp.hr == S_OK { LWASAPI_DBG_INFO } else { LWASAPI_DBG_WARN },
                    req.client_pid,
                    req.target_session_id,
                    req.volume_milli as i32,
                    req.share_mode as i32,
                    resp.hr,
                    "SESS_CTL",
                    tshow,
                    &format!(
                        "sid={} vol_m={} mute_op={} dev={}",
                        req.target_session_id, req.volume_milli, req.share_mode, tshow
                    ),
                );
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_INJECT_MIDI => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let mut ev = req.midi_ev;
                if ev.size == 0 || ev.size > 3 {
                    resp.hr = E_INVALIDARG;
                } else {
                    if ev.timestamp_ns == 0 {
                        ev.timestamp_ns = mono_ns();
                    }
                    {
                        let ss = self.sessions_lock.lock().unwrap();
                        for slot in &ss.sessions {
                            let r = slot.ring_ptr();
                            if slot.in_use && !r.is_null() {
                                unsafe { lwasapi_midi_push_to_ring(r, &ev) };
                            }
                        }
                    }
                    if midi_is_key_press(&ev) {
                        self.midi_injected_events.fetch_add(1, Ordering::Relaxed);
                    }
                    resp.hr = S_OK;
                }
                dbg_emit_vals(
                    LWASAPI_DBG_MIDI,
                    if resp.hr == S_OK { LWASAPI_DBG_INFO } else { LWASAPI_DBG_WARN },
                    req.client_pid,
                    0,
                    ev.data[0] as i32,
                    ev.data[1] as i32,
                    resp.hr,
                    "MIDI_INJECT",
                    "ipc",
                    &format!(
                        "status=0x{:02X} d1={} d2={} size={}",
                        ev.data[0], ev.data[1], ev.data[2], ev.size
                    ),
                );
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_ENGINE_CONFIG => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let (cur_rate, cur_period, cur_bits, active_pb_copy) = {
                    let hws = self.hw_lock.lock().unwrap();
                    (
                        hws.hw.sample_rate,
                        hws.hw.period_frames,
                        hws.hw.format_bits,
                        c_str_to_string(&hws.active_playback_id),
                    )
                };
                if req.stream_flags == LWASAPI_CFG_FLAG_CAPTURE_RATE {
                    if let Some(r) = self.reconfigure_capture_rate_impl(req.sample_rate) {
                        resp = r;
                        eprintln!(
                            "[LWASAPI-Salsa] Capture/Mic Clock reconfigured -> {} Hz ({})",
                            resp.actual_sample_rate,
                            if req.sample_rate == 0 {
                                "Auto/Follow Master"
                            } else {
                                "Fixed Independent Mic Clock"
                            }
                        );
                        dbg_emit_vals(
                            LWASAPI_DBG_ENGINE,
                            LWASAPI_DBG_INFO,
                            req.client_pid,
                            0,
                            resp.actual_sample_rate as i32,
                            resp.actual_period_frames as i32,
                            resp.hr,
                            "CFG_CAP_RATE",
                            &active_pb_copy,
                            &format!("CAP {}Hz (pref={})", resp.actual_sample_rate, req.sample_rate),
                        );
                    } else {
                        resp.hr = E_INVALIDARG;
                    }
                    let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
                    return;
                }

                let req_rate = if req.sample_rate != 0 { req.sample_rate } else { cur_rate };
                if req.sample_rate != 0 && !(44_100..=192_000).contains(&req.sample_rate) {
                    resp.hr = E_INVALIDARG;
                    let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
                    return;
                }
                let new_period = if req.requested_period_frames != 0 {
                    req.requested_period_frames
                } else {
                    cur_period
                };
                let new_bits = if req.format != 0 { req.format } else { cur_bits };

                match self.reconfigure_engine_rate_impl(req_rate, new_period, new_bits, true) {
                    Ok((mut r, actual_bits)) => {
                        r.hr = S_OK;
                        resp = r;
                        let ms = (1000.0 * (resp.actual_period_frames as f64))
                            / (resp.actual_sample_rate as f64);
                        eprintln!(
                            "[LWASAPI-Salsa] Engine reconfigured -> {} Hz | {}-bit | Quantum {} frames ({:.2} ms)",
                            resp.actual_sample_rate, actual_bits, resp.actual_period_frames, ms
                        );
                        dbg_emit_vals(
                            LWASAPI_DBG_ENGINE,
                            LWASAPI_DBG_INFO,
                            req.client_pid,
                            0,
                            resp.actual_sample_rate as i32,
                            resp.actual_period_frames as i32,
                            resp.hr,
                            "CFG_RECONF",
                            &active_pb_copy,
                            &format!(
                                "FIXED {}Hz {}bit quantum={}",
                                resp.actual_sample_rate, actual_bits, resp.actual_period_frames
                            ),
                        );
                    }
                    Err(_) => {
                        resp.hr = E_INVALIDARG;
                        dbg_emit_vals(
                            LWASAPI_DBG_ENGINE,
                            LWASAPI_DBG_WARN,
                            req.client_pid,
                            0,
                            req_rate as i32,
                            new_period as i32,
                            resp.hr,
                            "CFG_RECONF",
                            "invalid",
                            &format!("req={req_rate}Hz period={new_period}"),
                        );
                    }
                }
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_EQ => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let eq_copy = {
                    let mut ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                    ss.eq_cfg = req.eq_cfg;
                    clamp_eq_cfg(&mut ss.eq_cfg);
                    ss.eq_cfg
                };
                {
                    let _reconf = self.reconf_lock.lock().unwrap_or_else(|e| e.into_inner());
                    self.save_state();
                }
                let pname = c_str_to_string(&eq_copy.preset_name);
                dbg_emit_vals(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    req.client_pid,
                    0,
                    eq_copy.enabled as i32,
                    eq_copy.dynamic_mode as i32,
                    S_OK,
                    "EQ_SET",
                    if pname.is_empty() { "custom" } else { &pname },
                    &format!(
                        "en={} dyn={} preamp={:.1}dB lim={}",
                        eq_copy.enabled, eq_copy.dynamic_mode, eq_copy.preamp_db, eq_copy.limiter_enabled
                    ),
                );
                resp.hr = S_OK;
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_PARAM_EQ => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let mut pay: LwasapiParamEqPayload = unsafe { mem::zeroed() };
                if req.requested_buffer_frames as usize != mem::size_of::<LwasapiParamEqPayload>()
                    || stream.read_exact(as_bytes_mut(&mut pay)).is_err()
                {
                    resp.hr = E_INVALIDARG;
                    let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
                    return;
                }
                clamp_eq_cfg(&mut pay.eq);
                unsafe { salsa_dsp_clamp_param_eq_ext(&mut pay.ext) };
                {
                    let mut ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                    ss.eq_cfg = pay.eq;
                    ss.param_eq = pay.ext;
                }
                {
                    let _reconf = self.reconf_lock.lock().unwrap_or_else(|e| e.into_inner());
                    self.save_state();
                }
                let pname = c_str_to_string(&pay.eq.preset_name);
                dbg_emit_vals(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    pay.eq.enabled as i32,
                    pay.ext.crossfeed_enabled as i32,
                    S_OK,
                    "PARAM_EQ_SET",
                    if pname.is_empty() { "parametric" } else { &pname },
                    &format!(
                        "en={} dyn={} xfeed={}({:.2})",
                        pay.eq.enabled,
                        pay.eq.dynamic_mode,
                        pay.ext.crossfeed_enabled,
                        pay.ext.crossfeed_level
                    ),
                );
                resp.hr = S_OK;
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_MIC_APO => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let mut cfg: LwasapiMicApoCfg = unsafe { mem::zeroed() };
                if req.requested_buffer_frames as usize != mem::size_of::<LwasapiMicApoCfg>()
                    || stream.read_exact(as_bytes_mut(&mut cfg)).is_err()
                {
                    resp.hr = E_INVALIDARG;
                    let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
                    return;
                }
                unsafe { salsa_dsp_clamp_mic_apo_cfg(&mut cfg) };
                {
                    let mut ss = self.sessions_lock.lock().unwrap();
                    ss.mic_apo = cfg;
                }
                {
                    let _reconf = self.reconf_lock.lock().unwrap();
                    self.save_state();
                }
                dbg_emit_vals(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    cfg.enabled as i32,
                    cfg.rnnoise_enabled as i32,
                    S_OK,
                    "MIC_APO_SET",
                    if cfg.enabled != 0 { "active" } else { "bypass" },
                    &format!(
                        "hpf={:.0} gate={:.0}dB rnn={} comp={:.0}dB sidetone={}%",
                        cfg.hpf_hz,
                        cfg.gate_thresh_db,
                        cfg.rnnoise_enabled,
                        cfg.comp_thresh_db,
                        if cfg.sidetone_enabled != 0 { cfg.sidetone_pct } else { 0 }
                    ),
                );
                resp.hr = S_OK;
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_BT_CODEC => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let mut cfg: LwasapiBtCodecCfg = unsafe { mem::zeroed() };
                if req.requested_buffer_frames as usize != mem::size_of::<LwasapiBtCodecCfg>()
                    || stream.read_exact(as_bytes_mut(&mut cfg)).is_err()
                {
                    resp.hr = E_INVALIDARG;
                    let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
                    return;
                }
                let bt_init = unsafe { salsa_bt_is_initialized(self.bt()) };
                let bt_show = if bt_init {
                    unsafe {
                        lwasapi_bluez_set_sco_enabled(self.bt(), cfg.hfp_sco_enabled != 0);
                        lwasapi_bluez_set_codec_config(self.bt(), &cfg);
                    }
                    let mut out_cfg = cfg;
                    unsafe { lwasapi_bluez_get_codec_config(self.bt(), &mut out_cfg) };
                    {
                        let mut sb = self.saved_bt.lock().unwrap();
                        sb.saved_bt_cfg = out_cfg;
                    }
                    if cfg.hfp_sco_enabled == 0 {
                        let was_sco = {
                            let hws = self.hw_lock.lock().unwrap();
                            c_str_to_string(&hws.active_capture_id).starts_with("bluez_sco:")
                        };
                        if was_sco {
                            self.switch_engine_devices_impl(None, Some("auto"), false);
                        }
                    }
                    out_cfg
                } else {
                    let mut scfg = cfg;
                    scfg.preferred_codec = scfg.preferred_codec.min(4);
                    scfg.ldac_mode = scfg.ldac_mode.min(3);
                    scfg.wireless_abr_enabled = u32::from(scfg.wireless_abr_enabled != 0);
                    scfg.hfp_sco_enabled = u32::from(scfg.hfp_sco_enabled != 0);
                    if let Some(last) = scfg.bt_sco_codec.last_mut() {
                        *last = 0;
                    }
                    if let Some(last) = scfg.neg_reason.last_mut() {
                        *last = 0;
                    }
                    let mut sb = self.saved_bt.lock().unwrap();
                    sb.saved_bt_cfg = scfg;
                    scfg
                };
                {
                    let _reconf = self.reconf_lock.lock().unwrap();
                    self.save_state();
                }
                let pa = self.pa();
                if !pa.is_null() {
                    unsafe { lwasapi_pa_server_notify_change(pa) };
                }
                dbg_emit_vals(
                    LWASAPI_DBG_BLUEZ,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    bt_show.preferred_codec as i32,
                    bt_show.ldac_mode as i32,
                    S_OK,
                    "BT_CODEC_CFG",
                    "a2dp",
                    &format!(
                        "pref={} ldac={} abr={} sco={} kbps={}",
                        bt_show.preferred_codec,
                        bt_show.ldac_mode,
                        bt_show.wireless_abr_enabled,
                        bt_show.hfp_sco_enabled,
                        bt_show.effective_kbps
                    ),
                );
                resp.hr = S_OK;
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_BT_CONNECTION => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let action = req.requested_period_frames;
                let mac = c_str_to_string(&req.target_device);
                let c_mac = to_cstring_lossy(&mac);
                let mut flags: u32 = 0;
                let mut msg_buf = [0u8; 256];
                let rc = unsafe {
                    lwasapi_bluez_connection_ctl(
                        self.bt(),
                        action,
                        c_mac.as_ptr(),
                        msg_buf.as_mut_ptr().cast(),
                        msg_buf.len(),
                        &mut flags,
                    )
                };
                let mut dlist: LwasapiDeviceList = unsafe { mem::zeroed() };
                self.enumerate_devices_safe(&mut dlist);
                let usb_matches_bt = unsafe { any_usb_matches_bt(&dlist, self.bt()) };
                if usb_matches_bt {
                    flags |= LWASAPI_BT_STAT_USB_MATCHES_BT;
                }
                if action != LWASAPI_BT_CONN_QUERY {
                    {
                        let mut sb = self.saved_bt.lock().unwrap();
                        sb.bt_optional_mode = u32::from((flags & LWASAPI_BT_STAT_OPTIONAL) != 0);
                    }
                    let (target_pb, pref_r, pref_p, pref_b, hw_is_bt) = {
                        let hws = self.hw_lock.lock().unwrap();
                        (
                            c_str_to_string(&hws.target_playback_id),
                            if hws.pref_sample_rate != 0 { hws.pref_sample_rate } else { hws.hw.sample_rate },
                            if hws.pref_period_frames != 0 { hws.pref_period_frames } else { hws.hw.period_frames },
                            if hws.pref_format_bits != 0 { hws.pref_format_bits } else { hws.hw.format_bits },
                            hws.hw.is_bluetooth,
                        )
                    };
                    let want_bt_target =
                        target_pb == "bluetooth" || target_pb.starts_with("bluez");
                    let reeval_connect = rc >= 0
                        && ((target_pb == "auto" && !usb_matches_bt) || want_bt_target);
                    let reeval_disconnect = hw_is_bt;
                    let should_reapply = match action {
                        LWASAPI_BT_CONN_CONNECT => reeval_connect,
                        LWASAPI_BT_CONN_SET_AUTOCONN => {
                            reeval_connect && (flags & LWASAPI_BT_STAT_CONNECTED) != 0
                        }
                        LWASAPI_BT_CONN_DISCONNECT => reeval_disconnect,
                        _ => false,
                    };
                    {
                        let _reconf = self.reconf_lock.lock().unwrap();
                        if should_reapply {
                            self.apply_playback_endpoint(
                                &target_pb,
                                pref_r,
                                pref_p,
                                pref_b,
                                false,
                                false,
                                Some(&dlist),
                            );
                        } else if action == LWASAPI_BT_CONN_CONNECT && !hw_is_bt {
                            // Keep A2DP linked in standby without acquiring L2CAP when pinned to TV/wired/USB.
                            unsafe { lwasapi_bluez_set_tx_enabled(self.bt(), false) };
                        }
                        self.save_state();
                    }
                    let pa = self.pa();
                    if !pa.is_null() {
                        unsafe { lwasapi_pa_server_notify_change(pa) };
                    }
                    let snap_after = snapshot_bt(self.bt());
                    flags = 0;
                    if unsafe { lwasapi_bluez_is_optional_mode(self.bt()) } {
                        flags |= LWASAPI_BT_STAT_OPTIONAL;
                    }
                    if snap_after.has_bluez_transport || !c_str_to_string(&snap_after.device_path).is_empty() {
                        flags |= LWASAPI_BT_STAT_CONNECTED;
                    }
                    if snap_after.has_bluez_transport
                        && snap_after.direct_socket_mode
                        && snap_after.bt_fd >= 0
                    {
                        flags |= LWASAPI_BT_STAT_TRANSPORT;
                    }
                    if usb_matches_bt {
                        flags |= LWASAPI_BT_STAT_USB_MATCHES_BT;
                    }
                }
                if action != LWASAPI_BT_CONN_QUERY || rc < 0 {
                    dbg_emit_vals(
                        LWASAPI_DBG_BLUEZ,
                        if rc >= 0 { LWASAPI_DBG_INFO } else { LWASAPI_DBG_WARN },
                        0,
                        0,
                        action as i32,
                        flags as i32,
                        rc,
                        "BT_CONN_CTL",
                        if mac.is_empty() { "bluetooth" } else { &mac },
                        &c_str_to_string(&msg_buf),
                    );
                }
                resp.hr = if rc >= 0 { S_OK } else { E_FAIL };
                resp.session_id = flags;
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            LWASAPI_CMD_SET_HW_ANALOG => {
                let mut resp: LwasapiIpcResp = unsafe { mem::zeroed() };
                let mut cfg: LwasapiHwAnalogCfg = unsafe { mem::zeroed() };
                if req.requested_buffer_frames as usize != mem::size_of::<LwasapiHwAnalogCfg>()
                    || stream.read_exact(as_bytes_mut(&mut cfg)).is_err()
                {
                    resp.hr = E_INVALIDARG;
                    let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
                    return;
                }
                unsafe { lwasapi_hw_analog_clamp_cfg(&mut cfg) };
                {
                    let mut ss = self.sessions_lock.lock().unwrap();
                    ss.hw_analog_cfg = cfg;
                }
                self.hw_analog_probe_and_sync(true);
                {
                    let _reconf = self.reconf_lock.lock().unwrap();
                    self.save_state();
                }
                let pa = self.pa();
                if !pa.is_null() {
                    unsafe { lwasapi_pa_server_notify_change(pa) };
                }
                dbg_emit_vals(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    cfg.hp_amp_tier as i32,
                    cfg.output_route as i32,
                    S_OK,
                    "HW_ANALOG_CFG",
                    "analog",
                    &format!(
                        "unity={} tier={} route={} rails={} boost={:.1}dB mic={:.1}dB",
                        cfg.hw_dac_unity_vol,
                        cfg.hp_amp_tier,
                        cfg.output_route,
                        cfg.unlock_0db_rails,
                        cfg.analog_boost_db,
                        cfg.mic_hw_boost_db
                    ),
                );
                resp.hr = S_OK;
                let _ = send_all_nosignal(stream.as_raw_fd(), as_bytes(&resp));
            }
            _ => {}
        }
    }

    fn run_rt_engine_thread(&self) {
        let mut prio_set = 0u32;
        for p in [65, 50, 20, 10] {
            if try_realtime_fifo(p) {
                prio_set = p as u32;
                eprintln!(
                    "[LWASAPI-MMCSS] Real-time SCHED_FIFO priority {} enabled for audio graph thread.",
                    prio_set
                );
                break;
            }
        }
        if prio_set == 0 {
            eprintln!("[LWASAPI-MMCSS] Running in standard scheduler priority (grant rtprio for SCHED_FIFO).");
        }
        self.rt_priority.store(prio_set, Ordering::Relaxed);

        let mut mix_buf = vec![0.0f32; LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS];
        let mut pa_stereo = vec![0.0f32; LWASAPI_MAX_PERIOD * 2];
        let mut mic_buf = vec![0.0f32; LWASAPI_MAX_PERIOD * 2];
        let mut native_mic_buf = vec![0.0f32; LWASAPI_MAX_PERIOD * 2];
        let mut s32_buf = vec![0i32; LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS];
        let mut s24_buf = vec![0u8; LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS * 3];
        let mut s16_buf = vec![0i16; LWASAPI_MAX_PERIOD * LWASAPI_MAX_CHANNELS];
        let mut midi_ev: [LwasapiMidiEvent; 64] = [LwasapiMidiEvent::default(); 64];

        let mut mic_read_cursor = 0u64;
        let mut mic_up_st = SalsaMicCubicState {
            hist_l: [0.0; 3],
            hist_r: [0.0; 3],
            valid: false,
        };
        let mut last_mic_up_rate = 0u32;
        let mut next_wakeup_ns = mono_ns();
        let mut expected_wake_ns = 0u64;
        let mut have_expected_wake = false;

        while self.is_running() {
            let (mut period, mut ch, rate) = {
                let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
                (
                    hws.hw.period_frames,
                    if hws.hw.channels != 0 { hws.hw.channels } else { 2 },
                    if hws.hw.sample_rate != 0 { hws.hw.sample_rate } else { 48_000 },
                )
            };
            if period < 64 {
                period = 64;
            }
            if (period as usize) > LWASAPI_MAX_PERIOD {
                period = LWASAPI_MAX_PERIOD as u32;
            }
            if (ch as usize) > LWASAPI_MAX_CHANNELS {
                ch = LWASAPI_MAX_CHANNELS as u32;
            }
            let p_us = period as usize;
            let ch_us = ch as usize;
            let period_ns = (1_000_000_000u64 * (period as u64)) / (rate as u64);

            mix_buf[..p_us * ch_us].fill(0.0);
            let now_ns = mono_ns();
            let wake_jitter_ns = if have_expected_wake {
                now_ns.wrapping_sub(expected_wake_ns) as i64
            } else {
                0
            };

            // 1. Native hardware capture read + Mic APO
            let mut mic_frames = period;
            let mut mic_rate = rate;
            unsafe {
                lwasapi_bluez_read_capture_native(
                    self.bt(),
                    native_mic_buf.as_mut_ptr(),
                    LWASAPI_MAX_PERIOD as u32,
                    &mut mic_frames,
                    &mut mic_rate,
                    period,
                    rate,
                    &mut mic_read_cursor,
                );
            }
            if (mic_frames as usize) > LWASAPI_MAX_PERIOD {
                mic_frames = LWASAPI_MAX_PERIOD as u32;
            }
            self.cap_read_cursor.store(mic_read_cursor, Ordering::Relaxed);
            if mic_rate != last_mic_up_rate {
                mic_up_st = SalsaMicCubicState {
                    hist_l: [0.0; 3],
                    hist_r: [0.0; 3],
                    valid: false,
                };
                last_mic_up_rate = mic_rate;
            }
            let hwa = self.hwa();
            if !hwa.is_null() {
                let cap_makeup = unsafe { lwasapi_hw_analog_get_cap_makeup_linear(hwa) };
                unsafe {
                    salsa_dsp_scale_cap_clip(native_mic_buf.as_mut_ptr(), mic_frames * 2, cap_makeup);
                }
            }
            let mic_cfg = {
                let ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                ss.mic_apo
            };
            unsafe {
                salsa_dsp_process_capture_mic_apo(
                    native_mic_buf.as_mut_ptr(),
                    mic_frames,
                    mic_rate,
                    &mic_cfg,
                );
            }

            // 2. MIDI hub poll
            let num_midi = {
                let mut mh = self.midi_hub.lock().unwrap_or_else(|e| e.into_inner());
                let n = unsafe {
                    lwasapi_midi_poll_events(
                        &mut *mh.0,
                        midi_ev.as_mut_ptr(),
                        64,
                        now_ns,
                        rate,
                        period,
                    )
                };
                self.midi_hw_events
                    .store(mh.0.total_events.load(Ordering::Relaxed), Ordering::Relaxed);
                n.max(0) as usize
            };

            let mut pa_mix_ns = 0u64;
            let eq_ns;
            let monitor_is_pa_stereo;
            {
                let mut ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                if num_midi > 0 {
                    for slot in &ss.sessions {
                        let r = slot.ring_ptr();
                        if slot.in_use && !r.is_null() {
                            for ev in &midi_ev[..num_midi] {
                                unsafe { lwasapi_midi_push_to_ring(r, ev) };
                            }
                        }
                    }
                }

                let exclusive_id = ss.exclusive_session_id;
                let pa = self.pa();
                if exclusive_id >= 0 {
                    for slot in &mut ss.sessions {
                        let r = slot.ring_ptr();
                        if slot.in_use && (slot.session_id as i32) == exclusive_id && !r.is_null() {
                            unsafe {
                                (*ptr::addr_of!((*r).hw_clock_ns)).store(now_ns, Ordering::Relaxed);
                                salsa_dsp_consume_session_frames(
                                    r,
                                    &mut slot.resample_phase,
                                    slot.sample_rate,
                                    slot.channels,
                                    &mut slot.stream_live,
                                    &mut slot.underrun_pending,
                                    &mut slot.last_audio_ns,
                                    mix_buf.as_mut_ptr(),
                                    period,
                                    ch,
                                    rate,
                                    true,
                                    now_ns,
                                );
                            }
                            break;
                        }
                    }
                } else {
                    for slot in &mut ss.sessions {
                        let r = slot.ring_ptr();
                        if slot.in_use && !r.is_null() {
                            unsafe {
                                (*ptr::addr_of!((*r).hw_clock_ns)).store(now_ns, Ordering::Relaxed);
                            }
                            if !slot.is_capture && !slot.is_loopback {
                                unsafe {
                                    salsa_dsp_consume_session_frames(
                                        r,
                                        &mut slot.resample_phase,
                                        slot.sample_rate,
                                        slot.channels,
                                        &mut slot.stream_live,
                                        &mut slot.underrun_pending,
                                        &mut slot.last_audio_ns,
                                        mix_buf.as_mut_ptr(),
                                        period,
                                        ch,
                                        rate,
                                        false,
                                        now_ns,
                                    );
                                }
                            }
                        }
                    }
                    if !pa.is_null() {
                        let t_pa0 = mono_ns();
                        if ch == 2 {
                            unsafe {
                                lwasapi_pa_server_mix_playback(pa, mix_buf.as_mut_ptr(), period, rate);
                            }
                        } else {
                            pa_stereo[..p_us * 2].fill(0.0);
                            unsafe {
                                lwasapi_pa_server_mix_playback(pa, pa_stereo.as_mut_ptr(), period, rate);
                            }
                            for f in 0..p_us {
                                if ch_us == 1 {
                                    mix_buf[f] += 0.5 * (pa_stereo[f * 2] + pa_stereo[f * 2 + 1]);
                                } else {
                                    for c in 0..ch_us {
                                        mix_buf[f * ch_us + c] += pa_stereo[f * 2 + (c & 1)];
                                    }
                                }
                            }
                        }
                        pa_mix_ns += mono_ns().saturating_sub(t_pa0);
                    }
                }

                // 3. Dynamic EQ + Spectrum
                let t_eq0 = mono_ns();
                unsafe {
                    salsa_dsp_process_dynamic_eq(
                        mix_buf.as_mut_ptr(),
                        period,
                        ch,
                        rate,
                        &ss.eq_cfg,
                        &ss.param_eq,
                    );
                }
                eq_ns = mono_ns().saturating_sub(t_eq0);

                if ch != 2 {
                    for f in 0..p_us {
                        pa_stereo[f * 2] = mix_buf[f * ch_us];
                        pa_stereo[f * 2 + 1] = if ch_us > 1 {
                            mix_buf[f * ch_us + 1]
                        } else {
                            mix_buf[f * ch_us]
                        };
                    }
                    monitor_is_pa_stereo = true;
                } else {
                    monitor_is_pa_stereo = false;
                }

                // 4. Mic upsample + WASAPI capture push
                let mut need_eng_mic =
                    mic_cfg.enabled != 0 && mic_cfg.sidetone_enabled != 0 && mic_cfg.sidetone_pct > 0;
                if !need_eng_mic {
                    for slot in &ss.sessions {
                        if slot.in_use && slot.is_capture && !slot.ring_ptr().is_null() && !slot.is_loopback {
                            need_eng_mic = true;
                            break;
                        }
                    }
                }
                if need_eng_mic {
                    unsafe {
                        salsa_dsp_resample_mic_cubic(
                            native_mic_buf.as_ptr(),
                            mic_frames,
                            mic_buf.as_mut_ptr(),
                            period,
                            &mut mic_up_st,
                        );
                    }
                }

                for slot in &ss.sessions {
                    let r = slot.ring_ptr();
                    if slot.in_use && !r.is_null() {
                        if slot.is_capture || slot.is_loopback {
                            let cap_src: &[f32] = if slot.is_loopback {
                                if monitor_is_pa_stereo {
                                    &pa_stereo[..p_us * 2]
                                } else {
                                    &mix_buf[..p_us * 2]
                                }
                            } else {
                                &mic_buf[..p_us * 2]
                            };
                            unsafe {
                                let cwf = &*ptr::addr_of!((*r).cap_write_frame);
                                let cw = cwf.load(Ordering::Relaxed);
                                let cap_base = ptr::addr_of_mut!((*r).cap_pcm_data.0) as *mut f32;
                                for f in 0..p_us {
                                    let idx = ((cw.wrapping_add(f as u64) as usize) & LWASAPI_RING_MASK) * 2;
                                    cap_base.add(idx).write(cap_src[f * 2]);
                                    cap_base.add(idx + 1).write(cap_src[f * 2 + 1]);
                                }
                                cwf.store(cw.wrapping_add(period as u64), Ordering::Release);
                            }
                        }
                        if let Some(ref efd) = slot.event_fd {
                            crate::unix_sys::eventfd_signal(efd.as_raw_fd());
                        }
                    }
                }
            }

            let pa = self.pa();
            if !pa.is_null() {
                let t_pa1 = mono_ns();
                let mon_ptr = if monitor_is_pa_stereo {
                    pa_stereo.as_ptr()
                } else {
                    mix_buf.as_ptr()
                };
                unsafe {
                    lwasapi_pa_server_push_capture(
                        pa,
                        native_mic_buf.as_ptr(),
                        mic_frames,
                        mic_rate,
                        mon_ptr,
                        period,
                        rate,
                    );
                }
                pa_mix_ns += mono_ns().saturating_sub(t_pa1);
            }

            if mic_cfg.enabled != 0 && mic_cfg.sidetone_enabled != 0 && mic_cfg.sidetone_pct > 0 {
                let sg = ((mic_cfg.sidetone_pct as f32) / 100.0).min(1.0);
                for f in 0..p_us {
                    let sl = mic_buf[f * 2] * sg;
                    let sr = mic_buf[f * 2 + 1] * sg;
                    if ch_us == 1 {
                        mix_buf[f] += 0.5 * (sl + sr);
                    } else {
                        for c in 0..ch_us {
                            mix_buf[f * ch_us + c] += if (c & 1) != 0 { sr } else { sl };
                        }
                    }
                }
            }

            // 5. Output write to BlueZ A2DP or Kernel ALSA WaveRT PCM
            let mut need_timer_pace = true;
            let t_hw0 = mono_ns();
            {
                let _io_guard = self.hw_io_lock.lock().unwrap_or_else(|e| e.into_inner());
                let hw_snap = {
                    let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
                    hws.hw
                };
                if hw_snap.period_frames == period
                    && hw_snap.channels == ch
                    && hw_snap.sample_rate == rate
                {
                    if hw_snap.is_bluetooth && unsafe { salsa_bt_is_initialized(self.bt()) } {
                        let hwa = self.hwa();
                        let (drive_gain, softknee) = if !hwa.is_null() {
                            unsafe {
                                (
                                    lwasapi_hw_analog_get_drive_linear(hwa),
                                    lwasapi_hw_analog_softknee_active(hwa),
                                )
                            }
                        } else {
                            (1.0, false)
                        };
                        apply_bt_hw_analog_drive(&mut mix_buf[..p_us * ch_us], drive_gain, softknee);
                        unsafe {
                            lwasapi_bluez_write_period(self.bt(), mix_buf.as_ptr(), period, ch);
                        }
                    } else if !hw_snap.is_virtual && hw_snap.fd >= 0 {
                        let mut d_vol = 100u32;
                        let mut d_muted = false;
                        unsafe { salsa_bt_rt_master_vol(self.bt(), &mut d_vol, &mut d_muted) };
                        let mut master_gain = if d_muted {
                            0.0f32
                        } else {
                            (d_vol as f32) / 100.0
                        };
                        let mut softknee = false;
                        if !hwa.is_null() {
                            if unsafe { lwasapi_hw_analog_is_dac_unity_active(hwa) } {
                                master_gain = if d_muted {
                                    0.0
                                } else if d_vol <= 100 {
                                    1.0
                                } else {
                                    (d_vol as f32) / 100.0
                                };
                            }
                            master_gain *= unsafe { lwasapi_hw_analog_get_drive_linear(hwa) };
                            softknee = unsafe { lwasapi_hw_analog_softknee_active(hwa) };
                        }

                        let (pack_fmt, pack_dst): (u32, *mut u8) = match hw_snap.sndrv_format {
                            SNDRV_PCM_FORMAT_S32_LE => (SALSA_PACK_S32_LE, s32_buf.as_mut_ptr().cast()),
                            SNDRV_PCM_FORMAT_S24_LE => (SALSA_PACK_S24_LE, s32_buf.as_mut_ptr().cast()),
                            SNDRV_PCM_FORMAT_S24_3LE => (SALSA_PACK_S24_3LE, s24_buf.as_mut_ptr()),
                            SNDRV_PCM_FORMAT_S16_LE => (SALSA_PACK_S16_LE, s16_buf.as_mut_ptr().cast()),
                            _ => (SALSA_PACK_F32_LE, s32_buf.as_mut_ptr().cast()),
                        };
                        unsafe {
                            salsa_dsp_pack_pcm(
                                mix_buf.as_ptr(),
                                period * ch,
                                master_gain,
                                softknee,
                                pack_fmt,
                                pack_dst,
                            );
                        }

                        let mut xfer = SndXferi {
                            result: 0,
                            buf: pack_dst.cast::<c_void>(),
                            frames: period as c_ulong,
                        };
                        let mut wr = unsafe {
                            ioctl_raw(
                                hw_snap.fd,
                                SNDRV_PCM_IOCTL_WRITEI_FRAMES,
                                (&mut xfer as *mut SndXferi).cast::<c_void>(),
                            )
                        };
                        if wr < 0 {
                            let xerr = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                            if xerr == ERRNO_EPIPE || xerr == ERRNO_ESTRPIPE {
                                let xc = self.hw_xruns.fetch_add(1, Ordering::Relaxed) + 1;
                                let dp = c_str_to_string(&hw_snap.device_path);
                                dbg_emit_vals(
                                    LWASAPI_DBG_ALSA,
                                    LWASAPI_DBG_WARN,
                                    0,
                                    0,
                                    xc as i32,
                                    xerr,
                                    -xerr,
                                    "HW_XRUN",
                                    &dp,
                                    &format!("ALSA PCM XRUN #{xc} (errno={xerr})"),
                                );
                                unsafe {
                                    let _ = ioctl_raw(
                                        hw_snap.fd,
                                        SNDRV_PCM_IOCTL_PREPARE,
                                        ptr::null_mut(),
                                    );
                                }
                                xfer.result = 0;
                                wr = unsafe {
                                    ioctl_raw(
                                        hw_snap.fd,
                                        SNDRV_PCM_IOCTL_WRITEI_FRAMES,
                                        (&mut xfer as *mut SndXferi).cast::<c_void>(),
                                    )
                                };
                            }
                        }

                        if wr < 0 {
                            let dead_err =
                                std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                            if dead_err == ERRNO_ENODEV
                                || dead_err == ERRNO_EBADFD
                                || dead_err == ERRNO_EIO
                            {
                                let dp = c_str_to_string(&hw_snap.device_path);
                                dbg_emit_vals(
                                    LWASAPI_DBG_ALSA,
                                    LWASAPI_DBG_ERR,
                                    0,
                                    0,
                                    dead_err,
                                    0,
                                    -dead_err,
                                    "HW_DEV_LOST",
                                    &dp,
                                    &format!(
                                        "Kernel playback device removed (errno={dead_err}) -> Virtual Clock"
                                    ),
                                );
                                eprintln!(
                                    "[LWASAPI-audiodg] Kernel playback device {} removed (errno={}), dropping to virtual clock until failover.",
                                    dp, dead_err
                                );
                                let mut hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
                                if hws.hw.fd == hw_snap.fd && hws.hw.fd >= 0 {
                                    close_raw_fd(hws.hw.fd);
                                    hws.hw.fd = -1;
                                    hws.hw.is_virtual = true;
                                    copy_str_to_c(&mut hws.hw.device_path, "WaveRT-Virtual-Clock");
                                    copy_str_to_c(&mut hws.active_playback_id, "virtual");
                                }
                            }
                        } else {
                            need_timer_pace = false;
                            next_wakeup_ns = mono_ns();
                        }
                    }
                }
            }

            let t_end = mono_ns();
            let hw_write_ns = t_end.saturating_sub(t_hw0);
            let cycle_ns = if need_timer_pace {
                t_end.saturating_sub(now_ns)
            } else {
                t_hw0.saturating_sub(now_ns)
            };
            lwasapi_debug_record_rt_period(
                cycle_ns,
                wake_jitter_ns,
                period_ns,
                eq_ns,
                pa_mix_ns,
                hw_write_ns,
            );

            if need_timer_pace {
                let mut next_ns = next_wakeup_ns.saturating_add(period_ns);
                let cur_ns = mono_ns();
                if !have_expected_wake || cur_ns >= next_ns.saturating_add(period_ns) {
                    next_ns = cur_ns.saturating_add(period_ns);
                }
                next_wakeup_ns = next_ns;
                expected_wake_ns = next_ns;
                have_expected_wake = true;
                sleep_abs_mono_ns(next_ns);
            } else {
                have_expected_wake = false;
            }

            self.total_cycles.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn destroy_session_slot(ss: &mut SessionsState, slot_idx: usize) {
    if slot_idx >= LWASAPI_MAX_SESSIONS || !ss.sessions[slot_idx].in_use {
        return;
    }
    let sid = ss.sessions[slot_idx].session_id;
    let cpid = ss.sessions[slot_idx].client_pid;
    let cname = c_str_to_string(&ss.sessions[slot_idx].client_name);
    let label = if cname.is_empty() {
        "session".to_string()
    } else {
        cname
    };
    if (sid as i32) == ss.exclusive_session_id {
        ss.exclusive_session_id = -1;
        eprintln!(
            "[LWASAPI-audiodg] Exclusive mode released by session {}; Shared Mode restored.",
            sid
        );
        dbg_emit(
            LWASAPI_DBG_WASAPI,
            LWASAPI_DBG_INFO,
            cpid,
            sid,
            "EXCL_RELEASE",
            &label,
            "exclusive mode released; shared mode restored",
        );
    }
    dbg_emit(
        LWASAPI_DBG_WASAPI,
        LWASAPI_DBG_INFO,
        cpid,
        sid,
        "WASAPI_DESTROY",
        &label,
        &format!("session slot={slot_idx} closed"),
    );
    ss.sessions[slot_idx] = EngineSession::empty();
}

unsafe extern "C" fn pa_cb_get_status(ctx: *mut c_void, out_st: *mut LwasapiEngineStatus) {
    if ctx.is_null() || out_st.is_null() {
        return;
    }
    let eng = unsafe { &*(ctx as *const AudiodgEngine) };
    eng.fill_engine_status_impl(unsafe { &mut *out_st }, false);
}

unsafe extern "C" fn pa_cb_get_devices(ctx: *mut c_void, out_list: *mut LwasapiDeviceList) {
    if ctx.is_null() || out_list.is_null() {
        return;
    }
    let eng = unsafe { &*(ctx as *const AudiodgEngine) };
    eng.enumerate_devices_safe(unsafe { &mut *out_list });
}

unsafe extern "C" fn pa_cb_set_devices(
    ctx: *mut c_void,
    pb_id: *const c_char,
    cap_id: *const c_char,
) -> c_int {
    if ctx.is_null() {
        return -1;
    }
    let eng = unsafe { &*(ctx as *const AudiodgEngine) };
    let pb = unsafe { ptr_to_opt_str(pb_id) };
    let cap = unsafe { ptr_to_opt_str(cap_id) };
    eng.switch_engine_devices_impl(pb.as_deref(), cap.as_deref(), false)
}

unsafe extern "C" fn pa_cb_set_master_volume(
    ctx: *mut c_void,
    is_capture: c_int,
    vol_pct: c_int,
    mute_flag: c_int,
) -> c_int {
    if ctx.is_null() {
        return -1;
    }
    let eng = unsafe { &*(ctx as *const AudiodgEngine) };
    let rc = unsafe {
        if is_capture != 0 {
            lwasapi_bluez_set_mic_volume(eng.bt(), vol_pct, mute_flag)
        } else {
            lwasapi_bluez_set_volume(eng.bt(), vol_pct, mute_flag)
        }
    };
    dbg_emit_vals(
        LWASAPI_DBG_PA,
        LWASAPI_DBG_INFO,
        0,
        is_capture as u32,
        vol_pct,
        mute_flag,
        rc,
        "PA_MASTER_VOL",
        if is_capture != 0 { "master_mic" } else { "master_out" },
        &format!("vol={vol_pct}% mute={mute_flag}"),
    );
    eng.hw_analog_probe_and_sync(false);
    if rc == 0 {
        eng.mark_state_dirty();
    }
    rc
}

unsafe extern "C" fn pa_cb_set_mic_gain_ppm(
    ctx: *mut c_void,
    gain_ppm: u32,
    vol_pct: c_int,
    mute_flag: c_int,
) -> c_int {
    if ctx.is_null() {
        return -1;
    }
    let eng = unsafe { &*(ctx as *const AudiodgEngine) };
    let rc = unsafe { lwasapi_bluez_set_mic_gain_ppm(eng.bt(), gain_ppm, vol_pct, mute_flag) };
    eng.hw_analog_probe_and_sync(false);
    if rc == 0 {
        eng.mark_state_dirty();
    }
    rc
}

unsafe extern "C" fn dbg_fill_snapshot_cb(ctx: *mut c_void, snap: *mut LwasapiDbgSnapshot) {
    if ctx.is_null() || snap.is_null() {
        return;
    }
    let eng = unsafe { &*(ctx as *const AudiodgEngine) };
    let snap_ref = unsafe { &mut *snap };
    let saved_rt: LwasapiDbgRtStats = snap_ref.rt;
    unsafe { ptr::write_bytes(snap_ref, 0, 1) };
    snap_ref.rt = saved_rt;

    eng.fill_engine_status_impl(&mut snap_ref.engine_st, true);

    let (pref_cap, pref_pb_rate, pb_is_hw, active_pb_copy) = {
        let hws = eng.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
        snap_ref.target_playback_id.copy_from_slice(&hws.target_playback_id);
        snap_ref.target_capture_id.copy_from_slice(&hws.target_capture_id);
        (
            hws.pref_cap_sample_rate,
            hws.pref_sample_rate,
            !hws.hw.is_virtual && !hws.hw.is_bluetooth,
            c_str_to_string(&hws.active_playback_id),
        )
    };

    let bt_snap = snapshot_bt(eng.bt());
    snap_ref.bt_initialized = u8::from(bt_snap.initialized);
    snap_ref.bt_endpoints_registered = u8::from(bt_snap.endpoints_registered);
    snap_ref.bt_direct_socket = u8::from(bt_snap.direct_socket_mode);
    snap_ref.bt_has_transport = u8::from(bt_snap.has_bluez_transport);
    snap_ref.bt_tx_paused = u8::from(bt_snap.tx_paused);
    snap_ref.bt_device_path.copy_from_slice(&bt_snap.device_path);
    snap_ref.bt_transport_path.copy_from_slice(&bt_snap.transport_path);
    snap_ref.bt_endpoint_path.copy_from_slice(&bt_snap.endpoint_path);
    snap_ref.bt_sample_rate = bt_snap.bt_sample_rate;
    snap_ref.bt_channels = bt_snap.bt_channels;
    snap_ref.bt_mtu_r = bt_snap.read_mtu;
    snap_ref.bt_mtu_w = bt_snap.write_mtu;
    snap_ref.bt_avrcp_vol = bt_snap.avrcp_volume;
    snap_ref.bt_tx_ring_fill = bt_snap.tx_ring_fill;
    snap_ref.bt_tx_ring_cap = LWASAPI_BT_RING_FRAMES;
    snap_ref.bt_tx_packets = bt_snap.tx_packets_sent;
    snap_ref.bt_tx_underruns = bt_snap.tx_underruns;
    snap_ref.bt_tx_errors = bt_snap.tx_errors;

    snap_ref.cap_active = u8::from(bt_snap.has_capture_hw);
    snap_ref.cap_device_id.copy_from_slice(&bt_snap.cap_device_id);
    snap_ref.cap_pcm_path.copy_from_slice(&bt_snap.cap_pcm_path);
    snap_ref.cap_alias.copy_from_slice(&bt_snap.capture_device_alias);
    snap_ref.cap_hw_rate = bt_snap.cap_hw_rate;
    snap_ref.cap_hw_channels = bt_snap.cap_hw_channels;
    snap_ref.cap_hw_format = bt_snap.cap_hw_format;
    let cap_rd = eng.cap_read_cursor.load(Ordering::Relaxed);
    snap_ref.cap_ring_fill = bt_snap.cap_hw_write_frame.saturating_sub(cap_rd);
    snap_ref.cap_pref_rate = pref_cap;
    snap_ref.cap_measured_rate = bt_snap.cap_measured_rate;
    snap_ref.cap_kick_count = bt_snap.cap_kick_count;
    snap_ref.cap_is_usb = u8::from(bt_snap.cap_is_usb);

    let cap_id_s = c_str_to_string(&bt_snap.cap_device_id);
    let duplex_match = matches!(
        (parse_hw_card_idx(&active_pb_copy), parse_hw_card_idx(&cap_id_s)),
        (Some(pbc), Some(capc)) if pbc == capc
    );
    snap_ref.cap_duplex_locked = u8::from(
        bt_snap.has_capture_hw
            && pb_is_hw
            && pref_pb_rate != 0
            && duplex_match
            && snap_ref.engine_st.hw_sample_rate != pref_pb_rate,
    );

    {
        let mm = eng.midi_meta.lock().unwrap_or_else(|e| e.into_inner());
        snap_ref.midi_initialized = u8::from(mm.initialized);
        snap_ref.midi_rawmidi_open = u8::from(mm.rawmidi_open);
        snap_ref.midi_controller.copy_from_slice(&mm.controller_name);
        snap_ref.midi_dev_path.copy_from_slice(&mm.dev_path);
        snap_ref.midi_seq_client = mm.seq_client_id;
        snap_ref.midi_seq_in_port = mm.seq_in_port;
    }
    snap_ref.midi_events_routed = eng
        .midi_hw_events
        .load(Ordering::Relaxed)
        .saturating_add(eng.midi_injected_events.load(Ordering::Relaxed));

    {
        let ss = eng.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut wcnt = 0usize;
        for slot in &ss.sessions {
            let r = slot.ring_ptr();
            if slot.in_use && !r.is_null() && wcnt < LWASAPI_MAX_SESSIONS {
                let ws = &mut snap_ref.wasapi[wcnt];
                wcnt += 1;
                unsafe {
                    ws.session_id = slot.session_id;
                    ws.client_pid = slot.client_pid;
                    ws.client_name.copy_from_slice(&slot.client_name);
                    ws.share_mode = slot.share_mode;
                    ws.stream_flags = (*r).stream_flags;
                    ws.is_capture = u32::from(slot.is_capture);
                    ws.sample_rate = slot.sample_rate;
                    ws.channels = slot.channels;
                    ws.active = (*ptr::addr_of!((*r).active)).load(Ordering::Relaxed);
                    ws.muted = (*ptr::addr_of!((*r).muted)).load(Ordering::Relaxed);
                    ws.volume_milli = (*ptr::addr_of!((*r).volume_milli)).load(Ordering::Relaxed);
                    ws.buffer_frames = (*r).buffer_frames;
                    ws.period_frames = (*r).period_frames;
                    let (w, rd) = if ws.is_capture != 0 {
                        (
                            (*ptr::addr_of!((*r).cap_write_frame)).load(Ordering::Relaxed),
                            (*ptr::addr_of!((*r).cap_read_frame)).load(Ordering::Relaxed),
                        )
                    } else {
                        (
                            (*ptr::addr_of!((*r).write_frame)).load(Ordering::Relaxed),
                            (*ptr::addr_of!((*r).read_frame)).load(Ordering::Relaxed),
                        )
                    };
                    ws.buffered_frames = w.saturating_sub(rd);
                    ws.xruns = (*ptr::addr_of!((*r).xruns)).load(Ordering::Relaxed);
                }
            }
        }
        snap_ref.num_wasapi = wcnt as u32;
    }

    let pa = eng.pa();
    if !pa.is_null() {
        unsafe { lwasapi_pa_server_fill_debug(pa, &mut snap_ref.pa) };
    }
    let pw = eng.pw();
    if !pw.is_null() {
        unsafe { lwasapi_pw_broker_fill_debug(pw, &mut snap_ref.pw) };
    }
}

unsafe extern "C" fn dbg_handle_cmd_cb(
    ctx: *mut c_void,
    cmd_ptr: *const c_char,
    reply: *mut c_char,
    reply_len: usize,
) -> c_int {
    if ctx.is_null() || cmd_ptr.is_null() {
        return -ERRNO_EINVAL;
    }
    let eng = unsafe { &*(ctx as *const AudiodgEngine) };
    let Some(cmd) = (unsafe { ptr_to_opt_str(cmd_ptr) }) else {
        return -ERRNO_EINVAL;
    };
    let reply_slice = if reply.is_null() || reply_len == 0 {
        None
    } else {
        Some(unsafe { slice::from_raw_parts_mut(reply.cast::<u8>(), reply_len) })
    };

    if cmd == "RESET_NET_CLIENTS" {
        let pa = eng.pa();
        let pw = eng.pw();
        let pa_n = if !pa.is_null() {
            unsafe { lwasapi_pa_server_reset_clients(pa) }
        } else {
            0
        };
        let pw_n = if !pw.is_null() {
            unsafe { lwasapi_pw_broker_reset_clients(pw) }
        } else {
            0
        };
        record_trip(FS_MANUAL_RESET, b"Manual PA/PW network client flush via Debugger");
        let msg = format!(
            "Flushed {pa_n} PA + {pw_n} PW network clients (audio engine uninterrupted)"
        );
        dbg_emit(
            LWASAPI_DBG_ENGINE,
            LWASAPI_DBG_INFO,
            current_pid(),
            0,
            "NET_FAILSAFE_RESET",
            "PA/PW",
            &msg,
        );
        if let Some(out) = reply_slice {
            copy_str_to_c(out, &msg);
        }
        return 0;
    }
    if cmd == "RESET_PA_CLIENTS" {
        let pa = eng.pa();
        let pa_n = if !pa.is_null() {
            unsafe { lwasapi_pa_server_reset_clients(pa) }
        } else {
            0
        };
        record_trip(
            FS_MANUAL_RESET,
            b"Manual PA audio client flush via Debugger (PW preserved)",
        );
        let msg = format!(
            "Flushed {pa_n} PA audio clients (ScreenCast & audio engine preserved)"
        );
        dbg_emit(
            LWASAPI_DBG_ENGINE,
            LWASAPI_DBG_INFO,
            current_pid(),
            0,
            "NET_FAILSAFE_RESET",
            "PA",
            &msg,
        );
        if let Some(out) = reply_slice {
            copy_str_to_c(out, &msg);
        }
        return 0;
    }

    let pw = eng.pw();
    if pw.is_null() {
        if let Some(out) = reply_slice {
            copy_str_to_c(out, "PW video broker not initialized");
        }
        return -19; // -ENODEV
    }

    if let Some(rest) = cmd.strip_prefix("PW_PREVIEW ") {
        let vals: Vec<u32> = rest
            .split_whitespace()
            .filter_map(|t| t.parse::<u32>().ok())
            .collect();
        if !vals.is_empty() {
            let en = vals[0] != 0;
            let ttl = vals.get(1).copied().filter(|&v| v > 0).unwrap_or(15_000);
            let mw = if vals.len() >= 4 { vals[2] } else { 0 };
            let mh = if vals.len() >= 4 { vals[3] } else { 0 };
            let fps = if vals.len() >= 5 { vals[4] } else { 0 };
            unsafe { lwasapi_pw_broker_set_preview_ex(pw, en, ttl, mw, mh, fps) };
            if let Some(out) = reply_slice {
                copy_str_to_c(
                    out,
                    &format!(
                        "preview={} ttl={}ms target={}x{}@{}fps",
                        if en { "on" } else { "off" },
                        ttl,
                        mw,
                        mh,
                        fps
                    ),
                );
            }
            return 0;
        }
        if let Some(out) = reply_slice {
            copy_str_to_c(out, "invalid PW_PREVIEW syntax");
        }
        return -ERRNO_EINVAL;
    }

    if let Some(rest) = cmd.strip_prefix("PW_CAST_LIMITS ") {
        let vals: Vec<u32> = rest
            .split_whitespace()
            .filter_map(|t| t.parse::<u32>().ok())
            .collect();
        if vals.len() >= 3 {
            let (mw, mh, fps) = (vals[0], vals[1], vals[2]);
            unsafe { lwasapi_pw_broker_set_cast_limits(pw, mw, mh, fps) };
            if let Some(out) = reply_slice {
                copy_str_to_c(out, &format!("cast_limits={mw}x{mh}@{fps}fps"));
            }
            return 0;
        }
        if let Some(out) = reply_slice {
            copy_str_to_c(out, "invalid PW_CAST_LIMITS syntax");
        }
        return -ERRNO_EINVAL;
    }

    if cmd == "PW_SELF_TEST" {
        let (ptr, cap) = reply_slice
            .map(|s| (s.as_mut_ptr().cast::<c_char>(), s.len()))
            .unwrap_or((ptr::null_mut(), 0));
        return unsafe { lwasapi_pw_broker_self_test(pw, ptr, cap) };
    }

    if cmd == "PW_SYNC_MODE" {
        let rc = unsafe { lwasapi_pw_broker_sync_mode(pw) };
        if let Some(out) = reply_slice {
            copy_str_to_c(out, &format!("sync_mode rc={rc}"));
        }
        return rc;
    }

    if let Some(out) = reply_slice {
        copy_str_to_c(out, &format!("Unknown debug command: {cmd}"));
    }
    -ERRNO_ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn salsa_audiodg_main(argc: c_int, argv: *const *const c_char) -> c_int {
    std::env::remove_var("LD_LIBRARY_PATH");

    let mut args: Vec<String> = Vec::new();
    if argc > 0 && !argv.is_null() {
        for i in 0..(argc as isize) {
            let p = unsafe { *argv.offset(i) };
            if let Some(s) = unsafe { ptr_to_opt_str(p) } {
                args.push(s);
            } else {
                args.push(String::new());
            }
        }
    }

    for arg in args.iter().skip(1) {
        if matches!(arg.as_str(), "--version" | "-V" | "--license" | "--legal") {
            if let Some(rc) = crate::legal::handle_common_cli_flag("lwasapi-audiodg", arg) {
                return rc;
            }
        }
    }

    let mut sock_path = resolve_sock_path_common("LWASAPI_SOCKET", "audiodg.sock", LWASAPI_SOCKET_DEFAULT);

    let mut rt = default_runtime_state();
    unsafe {
        salsa_state_init_defaults(&mut rt);
        salsa_state_load(&mut rt);
    }
    let mut rate_arg = rt.sample_rate;
    let mut period_arg = rt.period_frames;
    let bits_arg = rt.format_bits;
    let mut device_arg: Option<String> = None;
    let mut force_virtual = false;
    let mut force_bt = false;

    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--device" if i + 1 < args.len() => {
                device_arg = Some(args[i + 1].clone());
                i += 2;
            }
            "--rate" if i + 1 < args.len() => {
                match args[i + 1].trim().parse::<u32>() {
                    Ok(r) if (44_100..=192_000).contains(&r) => rate_arg = r,
                    _ => eprintln!(
                        "[LWASAPI-audiodg] Ignoring invalid --rate '{}', keeping {} Hz",
                        args[i + 1], rate_arg
                    ),
                }
                i += 2;
            }
            "--period" if i + 1 < args.len() => {
                match args[i + 1].trim().parse::<u32>() {
                    Ok(p) if (64..=(LWASAPI_MAX_PERIOD as u32)).contains(&p) => period_arg = p,
                    _ => eprintln!(
                        "[LWASAPI-audiodg] Ignoring invalid --period '{}', keeping {} frames",
                        args[i + 1], period_arg
                    ),
                }
                i += 2;
            }
            "--virtual" => {
                force_virtual = true;
                i += 1;
            }
            "--bluetooth" => {
                force_bt = true;
                i += 1;
            }
            "--socket" if i + 1 < args.len() => {
                sock_path = args[i + 1].clone();
                i += 2;
            }
            _ => i += 1,
        }
    }

    if !(44_100..=192_000).contains(&rate_arg) {
        rate_arg = 48_000;
    }
    if !(64..=(LWASAPI_MAX_PERIOD as u32)).contains(&period_arg) {
        period_arg = if period_arg == 0 {
            512
        } else {
            period_arg.clamp(64, LWASAPI_MAX_PERIOD as u32)
        };
    }

    if !force_virtual && sock_path != LWASAPI_SOCKET_DEFAULT {
        let _ = fs::remove_file(LWASAPI_SOCKET_DEFAULT);
    }

    install_term_signals();
    rt.sample_rate = rate_arg;
    rt.period_frames = period_arg;
    rt.format_bits = bits_arg;

    let bt_ptr = unsafe { salsa_audiodg_bt_ptr() };
    let engine = Arc::new(AudiodgEngine::new(&rt, bt_ptr));
    let hwa_ptr = lwasapi_hw_analog_create();
    engine.hw_analog.store(hwa_ptr, Ordering::Release);

    engine.init_hardware(
        device_arg.as_deref(),
        rate_arg,
        period_arg,
        bits_arg,
        force_virtual,
        force_bt,
    );

    if !(force_virtual && std::env::var("LWASAPI_VIRTUAL_NO_MIDI").as_deref() == Ok("1")) {
        let mut mh = engine.midi_hub.lock().unwrap();
        let _ = unsafe { lwasapi_midi_init(&mut *mh.0) };
        let mut mm = engine.midi_meta.lock().unwrap();
        mm.initialized = mh.0.initialized;
        mm.rawmidi_open = mh.0.rawmidi_fd >= 0;
        mm.controller_name.copy_from_slice(&mh.0.controller_name);
        mm.dev_path.copy_from_slice(&mh.0.dev_path);
        mm.seq_client_id = mh.0.seq_client_id;
        mm.seq_in_port = mh.0.seq_in_port;
    }

    let Some(listener) = bind_unix_listener_0600(&sock_path) else {
        eprintln!("bind failed for {sock_path}");
        return 1;
    };

    let ctx_ptr = Arc::as_ptr(&engine) as *mut c_void;
    let pa_cbs = LwasapiPaCallbacks {
        user_ctx: ctx_ptr,
        get_status: Some(pa_cb_get_status),
        get_devices: Some(pa_cb_get_devices),
        set_devices: Some(pa_cb_set_devices),
        set_master_volume: Some(pa_cb_set_master_volume),
        set_mic_gain_ppm: Some(pa_cb_set_mic_gain_ppm),
    };
    let mut pa_ptr: *mut PaServerHandle = ptr::null_mut();
    if unsafe { lwasapi_pa_server_start(&mut pa_ptr, pa_cbs) == 0 } && !pa_ptr.is_null() {
        engine.pa_srv.store(pa_ptr, Ordering::Release);
        eprintln!("[LWASAPI-audiodg] Built-in PulseAudio native protocol v35 server active.");
    }
    let mut pw_ptr: *mut PwBrokerHandle = ptr::null_mut();
    if unsafe { lwasapi_pw_broker_start(&mut pw_ptr, ptr::null::<LwasapiPwBrokerOpts>()) == 0 }
        && !pw_ptr.is_null()
    {
        engine.pw_brk.store(pw_ptr, Ordering::Release);
        eprintln!("[LWASAPI-audiodg] Built-in Video-Only PipeWire ScreenCast broker initialized.");
    }

    let dbg_cbs = LwasapiDbgCallbacks {
        user_ctx: ctx_ptr,
        fill_snapshot: Some(dbg_fill_snapshot_cb),
        handle_cmd: Some(dbg_handle_cmd_cb),
    };
    lwasapi_debug_start(dbg_cbs);
    {
        let hws = engine.hw_lock.lock().unwrap();
        let pb_id = c_str_to_string(&hws.active_playback_id);
        let devp = c_str_to_string(&hws.hw.device_path);
        let cap_id = c_str_to_string(&hws.active_capture_id);
        dbg_emit(
            LWASAPI_DBG_ENGINE,
            LWASAPI_DBG_INFO,
            current_pid(),
            0,
            "ENGINE_START",
            &pb_id,
            &format!(
                "dev={} cap={} rate={} ch={} bits={} period={}",
                devp,
                cap_id,
                hws.hw.sample_rate,
                hws.hw.channels,
                hws.hw.format_bits,
                hws.hw.period_frames
            ),
        );
    }

    let rt_eng = Arc::clone(&engine);
    let engine_thread = std::thread::Builder::new()
        .name("salsa-rt-engine".to_string())
        .spawn(move || rt_eng.run_rt_engine_thread())
        .ok();

    eprintln!(
        "[LWASAPI-audiodg] Listening on {} (Ready for WASAPI Shared/Exclusive, MIDI Hub & Native PA/ALSA/JACK)",
        sock_path
    );

    let init_bt = snapshot_bt(engine.bt());
    let mut prev_bt_has = init_bt.has_bluez_transport
        && init_bt.direct_socket_mode
        && init_bt.bt_fd >= 0
        && !init_bt.tx_paused;
    let mut prev_bt_avrcp = init_bt.avrcp_volume;
    let mut prev_muted = init_bt.desktop_muted;
    let mut next_watcher_ns = mono_ns().saturating_add(250_000_000);
    let mut prev_hw_fp = 0u64;
    let mut hw_fp_init = false;
    let mut pending_usb_cap_settle = false;

    let my_uid = current_uid();
    while engine.is_running() {
        let mut pfd: Vec<PollFd> = Vec::with_capacity(1 + LWASAPI_MAX_SESSIONS);
        let mut slot_map: Vec<isize> = Vec::with_capacity(1 + LWASAPI_MAX_SESSIONS);
        pfd.push(PollFd {
            fd: listener.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        });
        slot_map.push(-1);

        {
            let ss = engine.sessions_lock.lock().unwrap();
            for (idx, slot) in ss.sessions.iter().enumerate() {
                if slot.in_use {
                    if let Some(ref sock) = slot.client_sock {
                        pfd.push(PollFd {
                            fd: sock.as_raw_fd(),
                            events: POLLIN | POLLHUP | POLLERR,
                            revents: 0,
                        });
                        slot_map.push(idx as isize);
                    }
                }
            }
        }

        let now_p_ns = mono_ns();
        let poll_ms = if now_p_ns < next_watcher_ns {
            let rem_ms = next_watcher_ns.saturating_sub(now_p_ns).saturating_add(999_999) / 1_000_000;
            rem_ms.min(250) as i32
        } else {
            0
        };
        let pr = do_poll(&mut pfd, poll_ms);
        if pr < 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == ERRNO_EINTR {
                continue;
            }
            break;
        }

        if pr > 0 {
            for (idx, p) in pfd.iter().enumerate().skip(1) {
                if (p.revents & (POLLIN | POLLHUP | POLLERR)) != 0 {
                    let mut dummy = [0u8; 16];
                    let closed = match nb_recv(p.fd, &mut dummy) {
                        Ok(0) => true,
                        Ok(_) => false,
                        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
                    };
                    if closed {
                        let slot_idx = slot_map[idx] as usize;
                        let mut ss = engine.sessions_lock.lock().unwrap();
                        if ss.sessions[slot_idx].in_use {
                            let cpid = ss.sessions[slot_idx].client_pid;
                            let cname = c_str_to_string(&ss.sessions[slot_idx].client_name);
                            eprintln!(
                                "[LWASAPI-audiodg] Session #{} ('{}', PID {}) closed.",
                                ss.sessions[slot_idx].session_id, cname, cpid
                            );
                            destroy_session_slot(&mut ss, slot_idx);
                        }
                    }
                }
            }

            if (pfd[0].revents & POLLIN) != 0 {
                while let Ok((stream, _)) = listener.accept() {
                    let _ = stream.set_nonblocking(false);
                    let fd = stream.as_raw_fd();
                    let uc_ok = socket_peercred(fd)
                        .map(|uc| uc.uid == my_uid || uc.uid == 0)
                        .unwrap_or(false);
                    if !uc_ok {
                        continue;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                    let mut req: LwasapiIpcReq = unsafe { mem::zeroed() };
                    let mut stream = stream;
                    if stream.read_exact(as_bytes_mut(&mut req)).is_ok() {
                        engine.handle_client_cmd(stream, &req);
                    }
                }
            }
        }

        let now_w_ns = mono_ns();
        if now_w_ns < next_watcher_ns {
            continue;
        }
        next_watcher_ns = now_w_ns.saturating_add(250_000_000);

        let bt_snap = snapshot_bt(engine.bt());
        let bt_now = bt_snap.has_bluez_transport
            && bt_snap.direct_socket_mode
            && bt_snap.bt_fd >= 0
            && !bt_snap.tx_paused;
        let avrcp_now = bt_snap.avrcp_volume;
        let muted_now = bt_snap.desktop_muted;
        let bt_alias = c_str_to_string(&bt_snap.device_alias);
        let bt_codec = c_str_to_string(&bt_snap.codec_name);
        let bt_rate = bt_snap.bt_sample_rate;

        let hw_fp = unsafe { lwasapi_bluez_hw_fingerprint(engine.bt()) };
        let hw_changed = hw_fp_init && hw_fp != prev_hw_fp;
        if !hw_fp_init {
            prev_hw_fp = hw_fp;
            hw_fp_init = true;
        }

        if hw_changed || bt_now != prev_bt_has || pending_usb_cap_settle {
            if bt_now != prev_bt_has {
                dbg_emit(
                    LWASAPI_DBG_BLUEZ,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "BT_STATE",
                    if bt_alias.is_empty() { "bluetooth" } else { &bt_alias },
                    &format!(
                        "connected={} codec={} rate={}",
                        i32::from(bt_now),
                        if bt_codec.is_empty() { "none" } else { &bt_codec },
                        bt_rate
                    ),
                );
            }
            if hw_changed {
                dbg_emit(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    "HW_TOPOLOGY",
                    "alsa",
                    &format!("fingerprint=0x{hw_fp:x}"),
                );
            }
            pending_usb_cap_settle = engine.sync_hotplug_topology(force_virtual, bt_now);
            prev_hw_fp = unsafe { lwasapi_bluez_hw_fingerprint(engine.bt()) };
            prev_bt_has = bt_now;
            prev_bt_avrcp = avrcp_now;
            prev_muted = muted_now;
            let pa = engine.pa();
            if !pa.is_null() && !pending_usb_cap_settle {
                unsafe { lwasapi_pa_server_notify_change(pa) };
            }
        } else if avrcp_now != prev_bt_avrcp || muted_now != prev_muted {
            dbg_emit(
                LWASAPI_DBG_BLUEZ,
                LWASAPI_DBG_INFO,
                0,
                0,
                "BT_AVRCP_STATE",
                if bt_alias.is_empty() { "bluetooth" } else { &bt_alias },
                &format!("avrcp={avrcp_now} muted={}", i32::from(muted_now)),
            );
            prev_bt_avrcp = avrcp_now;
            prev_muted = muted_now;
            let pa = engine.pa();
            if !pa.is_null() {
                unsafe { lwasapi_pa_server_notify_volume(pa, false) };
            }
            engine.mark_state_dirty();
        }

        let hwa = engine.hwa();
        if !hwa.is_null() {
            let mut hid_delta = 0i32;
            let mut hid_mute = 0i32;
            if unsafe { lwasapi_hw_analog_poll_hid(hwa, &mut hid_delta, &mut hid_mute) > 0 }
                && (hid_delta != 0 || hid_mute != 0)
            {
                let bt_cur = snapshot_bt(engine.bt());
                let new_v = ((bt_cur.desktop_volume_pct as i32) + hid_delta * 2).clamp(0, 150);
                let mute_op = if hid_mute != 0 {
                    i32::from(!bt_cur.desktop_muted)
                } else {
                    -1
                };
                unsafe {
                    lwasapi_bluez_set_volume(
                        engine.bt(),
                        if hid_delta != 0 { new_v } else { -1 },
                        mute_op,
                    );
                }
                dbg_emit_vals(
                    LWASAPI_DBG_ENGINE,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    if hid_delta != 0 { new_v } else { -1 },
                    mute_op,
                    hid_delta,
                    "HID_VOL",
                    "master_out",
                    &format!("hid_delta={hid_delta} vol={new_v}% mute={mute_op}"),
                );
                engine.hw_analog_probe_and_sync(false);
                let pa = engine.pa();
                if !pa.is_null() {
                    unsafe { lwasapi_pa_server_notify_volume(pa, false) };
                }
                engine.mark_state_dirty();
            }
        }

        if engine.state_dirty.load(Ordering::Acquire) {
            let dirty_at = engine.state_dirty_ns.load(Ordering::Acquire);
            if mono_ns() >= dirty_at.saturating_add(400_000_000) {
                engine.state_dirty.store(false, Ordering::Release);
                let _reconf = engine.reconf_lock.lock().unwrap();
                engine.save_state();
            }
        }
    }

    engine.running.store(false, Ordering::SeqCst);
    if let Some(th) = engine_thread {
        let _ = th.join();
    }
    {
        let _reconf = engine.reconf_lock.lock().unwrap();
        engine.save_state();
    }
    lwasapi_debug_stop();

    let pw = engine.pw_brk.swap(ptr::null_mut(), Ordering::AcqRel);
    if !pw.is_null() {
        unsafe { lwasapi_pw_broker_stop(pw) };
    }
    let pa = engine.pa_srv.swap(ptr::null_mut(), Ordering::AcqRel);
    if !pa.is_null() {
        unsafe { lwasapi_pa_server_stop(pa) };
    }

    {
        let mut ss = engine.sessions_lock.lock().unwrap();
        for idx in 0..LWASAPI_MAX_SESSIONS {
            destroy_session_slot(&mut ss, idx);
        }
    }
    {
        let mut mh = engine.midi_hub.lock().unwrap();
        unsafe { lwasapi_midi_close(&mut *mh.0) };
    }
    let hwa_old = engine.hw_analog.swap(ptr::null_mut(), Ordering::AcqRel);
    if !hwa_old.is_null() {
        unsafe { lwasapi_hw_analog_destroy(hwa_old) };
    }
    if snapshot_bt(engine.bt()).initialized {
        unsafe { lwasapi_bluez_close(engine.bt()) };
    }
    {
        let mut hws = engine.hw_lock.lock().unwrap();
        if hws.hw.fd >= 0 {
            close_raw_fd(hws.hw.fd);
            hws.hw.fd = -1;
        }
    }
    drop(listener);
    let _ = fs::remove_file(&sock_path);
    0
}

fn as_bytes<T>(v: &T) -> &[u8] {
    unsafe { slice::from_raw_parts((v as *const T).cast::<u8>(), mem::size_of::<T>()) }
}

fn as_bytes_mut<T>(v: &mut T) -> &mut [u8] {
    unsafe { slice::from_raw_parts_mut((v as *mut T).cast::<u8>(), mem::size_of::<T>()) }
}

fn to_cstring_lossy(s: &str) -> std::ffi::CString {
    let clean: Vec<u8> = s.bytes().take_while(|&b| b != 0).collect();
    std::ffi::CString::new(clean).unwrap_or_else(|_| std::ffi::CString::new("").unwrap())
}

fn snapshot_bt(bt: *const c_void) -> SalsaBtSnapshot {
    let mut out: SalsaBtSnapshot = unsafe { mem::zeroed() };
    unsafe { salsa_bt_get_snapshot(bt, &mut out) };
    out
}

#[inline]
pub fn resolve_cap_sample_rate(engine_rate: u32, pref_cap_rate: u32) -> u32 {
    if (44_100..=192_000).contains(&pref_cap_rate) {
        pref_cap_rate
    } else if engine_rate != 0 {
        engine_rate
    } else {
        48_000
    }
}

#[inline]
fn midi_is_key_press(ev: &LwasapiMidiEvent) -> bool {
    ev.size >= 3
        && (ev.data[0] & 0xF0) == 0x90
        && (ev.data[1] & 0x80) == 0
        && (ev.data[2] & 0x80) == 0
        && (ev.data[2] & 0x7F) > 0
}

pub fn parse_card_idx_from_id(id: &str) -> i32 {
    if let Some(rest) = id.strip_prefix("hw:") {
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(c) = num.parse::<i32>() {
            return c;
        }
    }
    if let Some(rest) = id.strip_prefix("/dev/snd/pcmC") {
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(c) = num.parse::<i32>() {
            return c;
        }
    }
    -1
}

fn parse_hw_card_idx(id: &str) -> Option<u32> {
    let rest = id.strip_prefix("hw:")?;
    let mut parts = rest.split(',');
    let c = parts.next()?.parse::<u32>().ok()?;
    let _ = parts.next()?;
    Some(c)
}

pub fn parse_pcm_card_dev(path: &str) -> Option<(u32, u32)> {
    let rest = path.strip_prefix("/dev/snd/pcmC")?;
    let d_pos = rest.find('D')?;
    let c = rest[..d_pos].parse::<u32>().ok()?;
    let after_d = &rest[d_pos + 1..];
    let d_digits: String = after_d.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    let d = d_digits.parse::<u32>().ok()?;
    Some((c, d))
}

pub fn resolve_dev_id_to_path(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("hw:") {
        let mut parts = rest.split(',');
        if let (Some(c_str), Some(d_str)) = (parts.next(), parts.next()) {
            let d_digits: String = d_str.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            if let (Ok(c), Ok(d)) = (c_str.parse::<u32>(), d_digits.parse::<u32>()) {
                return format!("/dev/snd/pcmC{c}D{d}p");
            }
        }
    }
    id.to_string()
}

pub fn format_bt_hw_label(
    has_tr: bool,
    alias: &str,
    mac: &str,
    codec: &str,
) -> (String, String) {
    if !has_tr {
        return (String::new(), String::new());
    }
    let name = if !alias.is_empty() {
        alias
    } else if !mac.is_empty() {
        mac
    } else {
        "Bluetooth"
    };
    let name_trunc: String = name.chars().take(32).collect();
    let codec_show = if !codec.is_empty() { codec } else { "A2DP" };
    let dev_label = format!("BlueZ: {name_trunc} ({codec_show})");
    let pb_label = if !mac.is_empty() {
        let mac_trunc: String = mac.chars().take(48).collect();
        format!("bluez:{mac_trunc}")
    } else {
        String::new()
    };
    (dev_label, pb_label)
}

pub fn dev_id_is_usb_card(dev_id: &str) -> bool {
    let Some(c) = parse_hw_card_idx(dev_id) else {
        return false;
    };
    Path::new(&format!("/proc/asound/card{c}/usbid")).exists()
}

pub fn device_list_has_type(dl: &LwasapiDeviceList, dev_id: &str, want_capture: u32) -> bool {
    if dev_id.is_empty() {
        return false;
    }
    let want_any_bt = dev_id == "bluetooth";
    let n = (dl.num_devices as usize).min(LWASAPI_MAX_DEVICES);
    for e in dl.devices.iter().take(n) {
        if e.is_capture != want_capture {
            continue;
        }
        if want_any_bt && e.is_bluetooth != 0 {
            return true;
        }
        if c_str_to_string(&e.id) == dev_id {
            return true;
        }
    }
    false
}

pub fn device_list_has_any_type(
    dl: &LwasapiDeviceList,
    want_capture: u32,
    usb_only: bool,
) -> bool {
    let n = (dl.num_devices as usize).min(LWASAPI_MAX_DEVICES);
    for e in dl.devices.iter().take(n) {
        if e.is_capture != want_capture {
            continue;
        }
        if usb_only && (e.is_bluetooth != 0 || !dev_id_is_usb_card(&c_str_to_string(&e.id))) {
            continue;
        }
        return true;
    }
    false
}

pub(crate) fn hw_params_any(params: &mut SndPcmHwParams) {
    unsafe { ptr::write_bytes(params, 0, 1) };
    let mask_cnt = SNDRV_PCM_HW_PARAM_LAST_MASK - SNDRV_PCM_HW_PARAM_FIRST_MASK + 1;
    for m in params.masks.iter_mut().take(mask_cnt) {
        m.bits.fill(u32::MAX);
    }
    let int_cnt = SNDRV_PCM_HW_PARAM_LAST_INTERVAL - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL + 1;
    for iv in params.intervals.iter_mut().take(int_cnt) {
        iv.min = 0;
        iv.max = u32::MAX;
    }
    params.rmask = u32::MAX;
}

pub(crate) fn hw_param_set_mask(params: &mut SndPcmHwParams, param: usize, bit: u32) {
    let idx = param - SNDRV_PCM_HW_PARAM_FIRST_MASK;
    params.masks[idx].bits.fill(0);
    params.masks[idx].bits[(bit >> 5) as usize] |= 1u32 << (bit & 31);
    params.cmask |= 1u32 << (param as u32);
}

pub(crate) fn hw_param_set_minmax(params: &mut SndPcmHwParams, param: usize, minv: u32, maxv: u32) {
    let idx = param - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL;
    params.intervals[idx].min = minv;
    params.intervals[idx].max = maxv;
    params.intervals[idx].flags = 0;
    params.cmask |= 1u32 << (param as u32);
}

fn is_valid_alsa_pcm_path(p: &str) -> bool {
    let Some(rest) = p.strip_prefix("/dev/snd/pcmC") else {
        return false;
    };
    let mut chars = rest.chars().peekable();
    let mut card_digits = 0usize;
    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
        chars.next();
        card_digits += 1;
    }
    if card_digits == 0 || chars.next() != Some('D') {
        return false;
    }
    let mut dev_digits = 0usize;
    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
        chars.next();
        dev_digits += 1;
    }
    dev_digits > 0 && matches!(chars.next(), Some('p' | 'c')) && chars.next().is_none()
}

fn open_kernel_pcm_device(
    dev_path: &str,
    req_rate: u32,
    req_period: u32,
    req_bits: u32,
) -> Option<KernelWaveRtHw> {
    if !is_valid_alsa_pcm_path(dev_path) {
        return None;
    }
    let fd = open_chrdev_rw_cloexec(dev_path);
    if fd < 0 {
        return None;
    }

    let (candidate_formats, candidate_bits): ([i32; 5], [u32; 5]) = match req_bits {
        16 => (
            [
                SNDRV_PCM_FORMAT_S16_LE,
                SNDRV_PCM_FORMAT_S24_3LE,
                SNDRV_PCM_FORMAT_S24_LE,
                SNDRV_PCM_FORMAT_S32_LE,
                SNDRV_PCM_FORMAT_FLOAT_LE,
            ],
            [16, 24, 24, 32, 32],
        ),
        24 => (
            [
                SNDRV_PCM_FORMAT_S24_3LE,
                SNDRV_PCM_FORMAT_S24_LE,
                SNDRV_PCM_FORMAT_S32_LE,
                SNDRV_PCM_FORMAT_FLOAT_LE,
                SNDRV_PCM_FORMAT_S16_LE,
            ],
            [24, 24, 32, 32, 16],
        ),
        _ => (
            [
                SNDRV_PCM_FORMAT_S32_LE,
                SNDRV_PCM_FORMAT_FLOAT_LE,
                SNDRV_PCM_FORMAT_S24_3LE,
                SNDRV_PCM_FORMAT_S24_LE,
                SNDRV_PCM_FORMAT_S16_LE,
            ],
            [32, 32, 24, 24, 16],
        ),
    };

    let candidate_chs = [2u32, 1u32];
    let mut candidate_rates = [req_rate, 0u32, 48_000, 44_100];
    if req_rate == 192_000 {
        candidate_rates[1] = 96_000;
    } else if req_rate == 176_400 {
        candidate_rates[1] = 88_200;
        candidate_rates[2] = 44_100;
        candidate_rates[3] = 48_000;
    } else if req_rate == 88_200 || req_rate == 44_100 {
        candidate_rates[2] = 44_100;
        candidate_rates[3] = 48_000;
    }

    let mut params: SndPcmHwParams = unsafe { mem::zeroed() };
    let mut chosen_fmt = -1i32;
    let mut chosen_bits = 0u32;

    'outer: for &bm in &[4u32, 2u32] {
        for (r_idx, &cand_r) in candidate_rates.iter().enumerate() {
            if cand_r < 44_100 {
                continue;
            }
            if r_idx > 0 && cand_r == req_rate {
                continue;
            }
            for &ch in &candidate_chs {
                for (&fmt, &bits) in candidate_formats.iter().zip(candidate_bits.iter()) {
                    hw_params_any(&mut params);
                    if unsafe {
                        ioctl_raw(
                            fd,
                            SNDRV_PCM_IOCTL_HW_REFINE,
                            (&mut params as *mut SndPcmHwParams).cast::<c_void>(),
                        )
                    } < 0
                    {
                        continue;
                    }
                    hw_param_set_mask(
                        &mut params,
                        SNDRV_PCM_HW_PARAM_ACCESS,
                        SNDRV_PCM_ACCESS_RW_INTERLEAVED,
                    );
                    hw_param_set_mask(&mut params, SNDRV_PCM_HW_PARAM_FORMAT, fmt as u32);
                    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_CHANNELS, ch, ch);
                    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_RATE, cand_r, cand_r);
                    let max_p = LWASAPI_MAX_PERIOD as u32;
                    let min_per = req_period.clamp(64, max_p);
                    let max_per = req_period.saturating_mul(2).clamp(min_per, max_p);
                    hw_param_set_minmax(
                        &mut params,
                        SNDRV_PCM_HW_PARAM_PERIOD_SIZE,
                        min_per,
                        max_per,
                    );
                    hw_param_set_minmax(
                        &mut params,
                        SNDRV_PCM_HW_PARAM_BUFFER_SIZE,
                        min_per.saturating_mul(bm),
                        min_per.saturating_mul(8),
                    );
                    if unsafe {
                        ioctl_raw(
                            fd,
                            SNDRV_PCM_IOCTL_HW_PARAMS,
                            (&mut params as *mut SndPcmHwParams).cast::<c_void>(),
                        )
                    } == 0
                    {
                        chosen_fmt = fmt;
                        chosen_bits = bits;
                        break 'outer;
                    }
                }
            }
        }
    }

    if chosen_fmt < 0 {
        close_raw_fd(fd);
        return None;
    }

    let actual_rate = params.intervals[SNDRV_PCM_HW_PARAM_RATE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    let actual_ch = params.intervals[SNDRV_PCM_HW_PARAM_CHANNELS - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    let actual_per =
        params.intervals[SNDRV_PCM_HW_PARAM_PERIOD_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;
    let actual_buf =
        params.intervals[SNDRV_PCM_HW_PARAM_BUFFER_SIZE - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL].min;

    let base_per = if actual_per != 0 { actual_per } else { req_period };
    let mut sw: SndPcmSwParams = unsafe { mem::zeroed() };
    sw.tstamp_mode = SNDRV_PCM_TSTAMP_ENABLE;
    sw.period_step = 1;
    sw.avail_min = base_per as u64;
    sw.start_threshold = if actual_buf >= base_per.saturating_mul(3) {
        (base_per.saturating_mul(2)) as u64
    } else {
        base_per as u64
    };
    sw.stop_threshold = if actual_buf != 0 {
        actual_buf as u64
    } else {
        base_per.saturating_mul(4) as u64
    };
    sw.xfer_align = 1;
    unsafe {
        let _ = ioctl_raw(
            fd,
            SNDRV_PCM_IOCTL_SW_PARAMS,
            (&mut sw as *mut SndPcmSwParams).cast::<c_void>(),
        );
        if ioctl_raw(fd, SNDRV_PCM_IOCTL_PREPARE, ptr::null_mut()) < 0 {
            close_raw_fd(fd);
            return None;
        }
    }

    let mut device_path = [0u8; 64];
    copy_str_to_c(&mut device_path, dev_path);
    Some(KernelWaveRtHw {
        fd,
        device_path,
        is_virtual: false,
        is_bluetooth: false,
        sample_rate: if actual_rate != 0 { actual_rate } else { req_rate },
        channels: if actual_ch != 0 { actual_ch } else { 2 },
        period_frames: base_per,
        buffer_frames: if actual_buf != 0 {
            actual_buf
        } else {
            req_period.saturating_mul(4)
        },
        sndrv_format: chosen_fmt,
        format_bits: chosen_bits,
    })
}

fn is_usb_device_entry(dev: &crate::c_abi::LwasapiDeviceEntry) -> bool {
    if dev.is_bluetooth != 0 {
        return false;
    }
    let drv = c_str_to_string(&dev.driver).to_ascii_lowercase();
    let name = c_str_to_string(&dev.name).to_ascii_lowercase();
    drv.contains("usb") || name.contains("usb")
}

fn normalize_bt_usb_tokens(raw: &str) -> (Vec<String>, String) {
    const FILLER: &[&str] = &[
        "usb",
        "audio",
        "bluetooth",
        "bt",
        "le",
        "a2dp",
        "avrcp",
        "hfp",
        "hsp",
        "corporation",
        "corp",
        "inc",
        "ltd",
        "llc",
        "co",
        "gmbh",
        "analog",
        "digital",
        "hw",
        "stereo",
        "mono",
    ];
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            let tok = core::mem::take(&mut cur);
            if !FILLER.contains(&tok.as_str()) {
                tokens.push(tok);
            }
        }
    }
    if !cur.is_empty() && !FILLER.contains(&cur.as_str()) {
        tokens.push(cur);
    }
    let joined = tokens.concat();
    (tokens, joined)
}

/// Returns `true` when a plugged USB endpoint name represents the same physical
/// hardware as a Bluetooth device alias/name (e.g., `"Bose Corporation Bose QC Ultra 2 HP - USB Audio"`
/// vs `"Bose QC Ultra 2 HP"`).
pub(crate) fn usb_device_matches_bt_name(usb_name: &str, bt_name: &str) -> bool {
    let (usb_toks, usb_joined) = normalize_bt_usb_tokens(usb_name);
    let (bt_toks, bt_joined) = normalize_bt_usb_tokens(bt_name);
    if usb_toks.is_empty() || bt_toks.is_empty() || usb_joined.len() < 4 || bt_joined.len() < 4 {
        return false;
    }
    if usb_toks == bt_toks || usb_joined == bt_joined {
        return true;
    }
    if bt_toks.len() >= 2
        && bt_joined.len() >= 6
        && usb_toks.windows(bt_toks.len()).any(|w| w == bt_toks.as_slice())
    {
        return true;
    }
    if usb_toks.len() >= 2
        && usb_joined.len() >= 6
        && bt_toks.windows(usb_toks.len()).any(|w| w == usb_toks.as_slice())
    {
        return true;
    }
    let shorter_joined = usb_joined.len().min(bt_joined.len());
    shorter_joined >= 8 && (usb_joined.contains(&bt_joined) || bt_joined.contains(&usb_joined))
}

/// Returns whether `apply_playback_endpoint` should attempt to activate BlueZ A2DP TX.
fn should_try_bt_on_apply(
    want_bt: bool,
    auto_mode: bool,
    skip_bt_for_matching_usb: bool,
    bt_opt: bool,
    has_transport: bool,
    any_bt_in_list: bool,
    bt_mac: &str,
) -> bool {
    if skip_bt_for_matching_usb {
        return false;
    }
    want_bt || (auto_mode && (has_transport || any_bt_in_list || (!bt_opt && !bt_mac.is_empty())))
}

/// Returns whether falling back to a kernel/virtual sink should pause BlueZ A2DP TX.
fn should_disable_bt_tx_on_fallback(want_bt: bool, auto_want_bt: bool) -> bool {
    !want_bt && !auto_want_bt
}

fn find_matching_usb_pb_idx_for_list(
    dlist: &LwasapiDeviceList,
    bt_alias: &str,
    extra_bt_aliases: &[String],
) -> Option<usize> {
    let n = (dlist.num_devices as usize).min(LWASAPI_MAX_DEVICES);
    let mut bt_names: Vec<String> = Vec::new();
    let trimmed_alias = bt_alias.trim();
    if !trimmed_alias.is_empty() {
        bt_names.push(trimmed_alias.to_string());
    }
    for extra in extra_bt_aliases {
        let t = extra.trim();
        if !t.is_empty() && !bt_names.iter().any(|existing| existing == t) {
            bt_names.push(t.to_string());
        }
    }
    for dev in &dlist.devices[..n] {
        if dev.is_capture == 0 && dev.is_bluetooth != 0 {
            let name = c_str_to_string(&dev.name);
            let t = name.trim();
            if !t.is_empty() && !bt_names.iter().any(|existing| existing == t) {
                bt_names.push(t.to_string());
            }
        }
    }
    if bt_names.is_empty() {
        return None;
    }
    for (i, dev) in dlist.devices[..n].iter().enumerate() {
        if dev.is_capture != 0 || !is_usb_device_entry(dev) {
            continue;
        }
        let usb_name = c_str_to_string(&dev.name);
        if bt_names
            .iter()
            .any(|bt_n| usb_device_matches_bt_name(&usb_name, bt_n))
        {
            return Some(i);
        }
    }
    None
}

unsafe fn any_usb_matches_bt(dlist: &LwasapiDeviceList, bt: *const c_void) -> bool {
    let snap = if bt.is_null() {
        String::new()
    } else {
        let mut s: crate::c_abi::SalsaBtSnapshot = unsafe { mem::zeroed() };
        unsafe { salsa_bt_get_snapshot(bt, &mut s) };
        c_str_to_string(&s.device_alias)
    };
    let extra = unsafe { lwasapi_bluez_known_audio_aliases(bt) };
    find_matching_usb_pb_idx_for_list(dlist, &snap, &extra).is_some()
}

/// Applies the active HW Analog amp drive and SoftKnee shaping to Bluetooth A2DP frames
/// while scrubbing any non-finite samples before encoding.
#[inline]
fn apply_bt_hw_analog_drive(samples: &mut [f32], drive_gain: f32, softknee: bool) {
    let gain = if drive_gain.is_finite() && drive_gain >= 0.0 {
        drive_gain
    } else {
        1.0
    };
    for s in samples {
        *s = if s.is_finite() {
            shape_hw_analog_sample(*s, gain, softknee)
        } else {
            0.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn usb_matches_dual_mode_bt_headset_names() {
        assert!(usb_device_matches_bt_name(
            "Bose Corporation Bose QC Ultra 2 HP - USB Audio",
            "Bose QC Ultra 2 HP"
        ));
        assert!(usb_device_matches_bt_name(
            "Sonova Consumer Hearing MOMENTUM 4 - USB Audio",
            "MOMENTUM 4"
        ));
        assert!(usb_device_matches_bt_name(
            "WH1000XM5 USB Audio",
            "Sony WH-1000XM5"
        ));
        assert!(!usb_device_matches_bt_name(
            "Bose QC Ultra 2 HP - USB Audio",
            "Logitech MX Anywhere 3"
        ));
        assert!(!usb_device_matches_bt_name(
            "Generic USB Audio",
            "Sony BT Audio"
        ));
    }

    #[test]
    fn alsa_pcm_uapi_layouts_match_kernel() {
        assert_eq!(size_of::<SndMask>(), 32);
        assert_eq!(size_of::<SndInterval>(), 12);
        assert_eq!(size_of::<SndPcmHwParams>(), 608);
        assert_eq!(offset_of!(SndPcmHwParams, masks), 4);
        assert_eq!(offset_of!(SndPcmHwParams, intervals), 260);
        assert_eq!(offset_of!(SndPcmHwParams, rmask), 512);
        assert_eq!(offset_of!(SndPcmHwParams, fifo_size), 536);

        assert_eq!(size_of::<SndPcmSwParams>(), 136);
        assert_eq!(offset_of!(SndPcmSwParams, avail_min), 16);
        assert_eq!(offset_of!(SndPcmSwParams, xfer_align), 24);
        assert_eq!(offset_of!(SndPcmSwParams, start_threshold), 32);
        assert_eq!(offset_of!(SndPcmSwParams, stop_threshold), 40);
        assert_eq!(offset_of!(SndPcmSwParams, silence_size), 56);
        assert_eq!(offset_of!(SndPcmSwParams, proto), 72);

        assert_eq!(size_of::<SndXferi>(), 24);
        assert_eq!(offset_of!(SndXferi, buf), 8);
        assert_eq!(offset_of!(SndXferi, frames), 16);
    }

    #[test]
    fn dev_id_bt_label_and_card_parsing_match_c() {
        assert_eq!(parse_card_idx_from_id("hw:3,0"), 3);
        assert_eq!(parse_card_idx_from_id("/dev/snd/pcmC12D0c"), 12);
        assert_eq!(parse_card_idx_from_id("virtual"), -1);

        assert_eq!(resolve_dev_id_to_path("hw:2,1"), "/dev/snd/pcmC2D1p");
        assert_eq!(resolve_dev_id_to_path("/dev/snd/pcmC0D0p"), "/dev/snd/pcmC0D0p");

        let (lbl, pb) = format_bt_hw_label(true, "Bose QC Ultra 2 HP", "00:11:22:33:44:55", "AAC");
        assert_eq!(lbl, "BlueZ: Bose QC Ultra 2 HP (AAC)");
        assert_eq!(pb, "bluez:00:11:22:33:44:55");

        assert_eq!(resolve_cap_sample_rate(192_000, 0), 192_000);
        assert_eq!(resolve_cap_sample_rate(192_000, 48_000), 48_000);
    }

    #[test]
    fn device_list_matching_and_midi_press_work() {
        let mut dl: LwasapiDeviceList = unsafe { mem::zeroed() };
        dl.num_devices = 2;
        copy_str_to_c(&mut dl.devices[0].id, "hw:0,0");
        dl.devices[0].is_capture = 1;
        copy_str_to_c(&mut dl.devices[1].id, "bluez:AA:BB:CC:DD:EE:FF");
        dl.devices[1].is_bluetooth = 1;

        assert!(device_list_has_type(&dl, "hw:0,0", 1));
        assert!(!device_list_has_type(&dl, "hw:0,0", 0));
        assert!(device_list_has_type(&dl, "bluetooth", 0));

        let note_on = LwasapiMidiEvent {
            frame_offset: 0,
            size: 3,
            port: 0,
            data: [0x90, 60, 100, 0],
            timestamp_ns: 0,
        };
        let note_off_vel0 = LwasapiMidiEvent {
            frame_offset: 0,
            size: 3,
            port: 0,
            data: [0x90, 60, 0, 0],
            timestamp_ns: 0,
        };
        assert!(midi_is_key_press(&note_on));
        assert!(!midi_is_key_press(&note_off_vel0));
    }

    #[test]
    fn bt_apply_fallback_and_auto_reconnect_policies_work() {
        // Explicit BT switch or Auto-Priority with a connected headset never sabotages tx_paused
        assert!(!should_disable_bt_tx_on_fallback(true, false));
        assert!(!should_disable_bt_tx_on_fallback(false, true));
        // Switching to TV / wired / matching USB pauses BT TX
        assert!(should_disable_bt_tx_on_fallback(false, false));

        // Optional BT enabled still unpauses connected BT when switching back to Auto-Priority
        assert!(should_try_bt_on_apply(false, true, false, true, false, true, ""));
        // USB cable matching headset overrides BT even when connected
        assert!(!should_try_bt_on_apply(false, true, true, false, true, true, "68:F2:1F:04:F9:F6"));
        // Boot/Auto with Optional BT and no connected BT stays off BT
        assert!(!should_try_bt_on_apply(false, true, false, true, false, false, ""));
    }

    #[test]
    fn bt_hw_analog_drive_amplifies_cleanly_and_scrubs_nonfinite() {
        let mut buf = [0.25f32, -0.5f32, f32::NAN, f32::INFINITY];
        // +7.0 dB HP Amp tier ≈ 2.2387x drive with SoftKnee enabled
        let drive_7db = 10.0f32.powf(7.0 / 20.0);
        apply_bt_hw_analog_drive(&mut buf, drive_7db, true);
        assert!((buf[0] - 0.25 * drive_7db).abs() < 1e-5);
        assert!(buf[1] < -0.72 && buf[1] > -1.0, "softknee should smoothly compress peaks: {}", buf[1]);
        assert_eq!(buf[2], 0.0);
        assert_eq!(buf[3], 0.0);

        let mut unity = [0.4f32, -1.5f32];
        apply_bt_hw_analog_drive(&mut unity, 1.0, false);
        assert!((unity[0] - 0.4).abs() < 1e-6);
        assert_eq!(unity[1], -1.0);
    }
}
