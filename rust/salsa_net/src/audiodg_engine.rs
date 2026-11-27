// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![deny(unsafe_code)]
#![allow(clippy::too_many_arguments)]

use core::mem;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::fs;
use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::alsa_hw::{
    AlsaPlaybackBackend, AlsaWriteOutcome, KernelAlsaBackend, KernelWaveRtHw,
    SNDRV_PCM_FORMAT_FLOAT_LE,
};
use crate::bluez_runtime::{BluetoothBackend, RealBluezBackend};
use crate::c_abi::{
    c_str_to_string, copy_str_to_c, LasapiBtCodecCfg, LasapiDbgRtStats, LasapiDeviceList,
    LasapiEngineStatus, LasapiEqConfig, LasapiHwAnalogCfg, LasapiIpcReq, LasapiIpcResp,
    LasapiMicApoCfg, LasapiMidiEvent, LasapiParamEqExt, LasapiParamEqPayload, PodBytes,
    SalsaMicCubicState, SalsaRuntimeState, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_SHAREMODE_EXCLUSIVE,
    AUDCLNT_STREAMFLAGS_CAPTURE, AUDCLNT_STREAMFLAGS_LOOPBACK, E_FAIL, E_INVALIDARG, E_OUTOFMEMORY,
    LASAPI_BT_RING_FRAMES, LASAPI_BT_STAT_CONNECTED, LASAPI_BT_STAT_OPTIONAL,
    LASAPI_BT_STAT_TRANSPORT, LASAPI_BT_STAT_USB_MATCHES_BT, LASAPI_CMD_CONTROL_V2,
    LASAPI_CMD_CREATE_SESSION, LASAPI_CMD_SET_BT_CODEC, LASAPI_CMD_SET_HW_ANALOG,
    LASAPI_CMD_SET_MIC_APO, LASAPI_CMD_SET_PARAM_EQ, LASAPI_DBG_ALSA, LASAPI_DBG_BLUEZ,
    LASAPI_DBG_ENGINE, LASAPI_DBG_ERR, LASAPI_DBG_INFO, LASAPI_DBG_LASAPI, LASAPI_DBG_MIDI,
    LASAPI_DBG_PA, LASAPI_DBG_WARN, LASAPI_MAX_CHANNELS, LASAPI_MAX_DEVICES, LASAPI_MAX_PERIOD,
    LASAPI_MAX_SESSIONS, LASAPI_RING_FRAMES, LASAPI_SOCKET_DEFAULT, S_OK,
};
use crate::control_ipc::{
    control_resp_to_legacy, decode_request, legacy_ipc_req_to_control, write_response_frame,
    AudioDir, BtConnOp, ControlPayload, ControlRequest, ControlResponse, MuteOp, SessionTarget,
    MAX_CONTROL_V2_FRAME,
};
use crate::debug_server::{
    debug_start, debug_stop, record_rt_period, rt_stats_to_json, wire_stats_to_json,
    DebugEngineHost,
};
use crate::dsp::{
    dsp_telemetry, process_capture_mic_apo, process_dynamic_eq, resample_mic_cubic_slice,
    scale_cap_clip_slice, shape_hw_analog_sample,
};
use crate::failsafe::{record_trip, FS_MANUAL_RESET};
use crate::hw_analog::{
    card_has_plugged_headphone_jack, clamp_hw_analog_cfg, is_headphone_playback_card,
    score_card_by_index, AnalogStage,
};
use crate::json::JsonValue;
use crate::midi_hub::MidiHub;
use crate::pa_server::{start_pa_server, PaEngineHost, PaServerHandle};
use crate::pw_broker::{empty_debug_json, start_pw_broker, PwBrokerHandle};
use crate::settings::{extract_per_device_eq_for_sink, is_per_device_eq_enabled};
use crate::state_conf::{
    clamp_eq_cfg, clamp_mic_apo_cfg, clamp_param_eq_ext, default_runtime_state, load_runtime_state,
    save_runtime_state,
};
use crate::unix_sys::{
    bind_unix_listener_0600, claim_systemd_unix_listeners, current_pid, dbg_emit, dbg_emit_vals,
    do_poll, install_term_signals, mono_ns, nb_recv, new_eventfd, posix_owner_id,
    resolve_sock_path_common, send_all_nosignal, sendmsg_all_with_fds, sleep_abs_mono_ns,
    socket_peercred, term_signal_requested, try_realtime_fifo, PollFd, ERRNO_EINTR, ERRNO_EINVAL,
    ERRNO_ENOSYS, POLLERR, POLLHUP, POLLIN,
};
use crate::wavert_ring::{
    RingMuteOp, SessionResampleState, SessionRingSpec, WaveRtEngineRing,
};

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
    session_num: u32,
    client_pid: u32,
    client_name: [u8; 64],
    share_mode: u32,
    client_sock: Option<UnixStream>,
    _shm_fd: Option<OwnedFd>,
    event_fd: Option<OwnedFd>,
    ring: Option<WaveRtEngineRing>,
    resample: SessionResampleState,
    sample_rate: u32,
    channels: u32,
    is_capture: bool,
    is_loopback: bool,
    last_pa_ctl_seq: u64,
}

impl EngineSession {
    const fn empty() -> Self {
        Self {
            in_use: false,
            session_num: 0,
            client_pid: 0,
            client_name: [0; 64],
            share_mode: 0,
            client_sock: None,
            _shm_fd: None,
            event_fd: None,
            ring: None,
            resample: SessionResampleState::new(),
            sample_rate: 0,
            channels: 0,
            is_capture: false,
            is_loopback: false,
            last_pa_ctl_seq: 0,
        }
    }
}

struct SessionsState {
    sessions: [EngineSession; LASAPI_MAX_SESSIONS],
    next_session_num: u32,
    exclusive_session_num: i32,
    eq_cfg: LasapiEqConfig,
    param_eq: LasapiParamEqExt,
    mic_apo: LasapiMicApoCfg,
    hw_analog_cfg: LasapiHwAnalogCfg,
}

impl SessionsState {
    fn from_runtime(rt: &SalsaRuntimeState) -> Self {
        Self {
            sessions: [const { EngineSession::empty() }; LASAPI_MAX_SESSIONS],
            next_session_num: 1,
            exclusive_session_num: -1,
            eq_cfg: rt.eq_cfg,
            param_eq: rt.param_eq,
            mic_apo: rt.mic_apo,
            hw_analog_cfg: rt.hw_analog_cfg,
        }
    }
}

struct SavedBtState {
    saved_bt_cfg: LasapiBtCodecCfg,
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

#[derive(Clone, Debug, Default)]
struct HotplugTracker {
    initialized: bool,
    seen_pb_ids: Vec<String>,
    hp_jack_cards: [bool; 32],
    restore_target_pb: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PlaybackHotplugAction {
    None,
    Switch {
        apply_req: String,
        new_target_pb: Option<String>,
        reason: &'static str,
    },
}

fn collect_playback_ids(dl: &LasapiDeviceList) -> Vec<String> {
    let n = (dl.num_devices as usize).min(LASAPI_MAX_DEVICES);
    let mut ids = Vec::with_capacity(n);
    for e in dl.devices.iter().take(n) {
        if e.is_capture == 0 {
            let id = c_str_to_string(&e.id);
            if !id.is_empty() && !ids.iter().any(|existing| existing == &id) {
                ids.push(id);
            }
        }
    }
    ids
}

#[inline]
fn is_hdmi_playback_name(name: &str) -> bool {
    let low = name.to_ascii_lowercase();
    low.contains("hdmi") || low.contains("displayport")
}

fn any_usb_in_list_matches_bt_name<FUsb>(
    dl: &LasapiDeviceList,
    bt_name: &str,
    is_usb_id: &FUsb,
) -> bool
where
    FUsb: Fn(&str) -> bool,
{
    let n = (dl.num_devices as usize).min(LASAPI_MAX_DEVICES);
    dl.devices.iter().take(n).any(|e| {
        if e.is_capture != 0 || e.is_bluetooth != 0 {
            return false;
        }
        let id = c_str_to_string(&e.id);
        if !is_usb_id(&id) {
            return false;
        }
        let nm = c_str_to_string(&e.name);
        usb_device_matches_bt_name(&nm, bt_name)
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_playback_hotplug_with<FUsb, FHp>(
    force_virtual: bool,
    target_pb: &str,
    active_pb: &str,
    hw_is_virtual: bool,
    hw_is_bt: bool,
    bt_now: bool,
    usb_bt_active: bool,
    dlist: &LasapiDeviceList,
    card_hp_jacks: &[bool; 32],
    tracker: &mut HotplugTracker,
    is_usb_id: FUsb,
    is_hp_dev: FHp,
) -> PlaybackHotplugAction
where
    FUsb: Fn(&str) -> bool,
    FHp: Fn(&str, &str) -> bool,
{
    let curr_ids = collect_playback_ids(dlist);
    if !tracker.initialized {
        tracker.seen_pb_ids = curr_ids;
        tracker.hp_jack_cards = *card_hp_jacks;
        tracker.initialized = true;
        return PlaybackHotplugAction::None;
    }

    let n = (dlist.num_devices as usize).min(LASAPI_MAX_DEVICES);
    let mut newly_added_usb_hp: Option<String> = None;
    let mut newly_added_usb: Option<String> = None;
    let mut newly_added_bt: Option<String> = None;
    let mut newly_plugged_hp: Option<String> = None;
    let mut active_name = String::new();
    let mut has_usb_hp_in_list = false;

    for e in dlist.devices.iter().take(n) {
        if e.is_capture != 0 {
            continue;
        }
        let id = c_str_to_string(&e.id);
        let nm = c_str_to_string(&e.name);
        if id == active_pb {
            active_name = nm.clone();
        }
        let is_bt_entry = e.is_bluetooth != 0;
        let is_usb_entry = !is_bt_entry && is_usb_id(&id);
        let is_hp_entry = !is_bt_entry && is_hp_dev(&id, &nm);
        if is_usb_entry && is_hp_entry && !is_hdmi_playback_name(&nm) {
            has_usb_hp_in_list = true;
        }
        if !is_bt_entry {
            let cidx = parse_card_idx_from_id(&id);
            if cidx >= 0 && (cidx as usize) < 32 {
                let ci = cidx as usize;
                if !tracker.hp_jack_cards[ci] && card_hp_jacks[ci] && newly_plugged_hp.is_none() {
                    newly_plugged_hp = Some(id.clone());
                }
            }
        }
        if !id.is_empty() && !tracker.seen_pb_ids.iter().any(|prev| prev == &id) {
            if is_usb_entry && !is_hdmi_playback_name(&nm) {
                if is_hp_entry && newly_added_usb_hp.is_none() {
                    newly_added_usb_hp = Some(id.clone());
                } else if newly_added_usb.is_none() {
                    newly_added_usb = Some(id.clone());
                }
            } else if is_bt_entry
                && !any_usb_in_list_matches_bt_name(dlist, &nm, &is_usb_id)
                && newly_added_bt.is_none()
            {
                newly_added_bt = Some(id.clone());
            }
        }
    }

    tracker.seen_pb_ids = curr_ids;
    tracker.hp_jack_cards = *card_hp_jacks;

    if force_virtual || target_pb == "virtual" {
        tracker.restore_target_pb = None;
        return PlaybackHotplugAction::None;
    }

    let target_auto = target_pb == "auto" || target_pb.is_empty();
    let target_is_bt = target_pb == "bluetooth" || target_pb.starts_with("bluez");
    let bt_in_list = device_list_has_type(dlist, "bluetooth", 0);

    // 1. Newly plugged headphones (USB headphones > analog headphone jack > new USB DAC > new BT headset)
    let hotplug_cand = newly_added_usb_hp
        .or(newly_plugged_hp)
        .or(newly_added_usb)
        .or(newly_added_bt);
    if let Some(new_id) = hotplug_cand {
        if new_id != active_pb {
            let new_target = if target_auto {
                None
            } else {
                if tracker.restore_target_pb.is_none()
                    && target_pb != new_id
                    && device_list_has_type(dlist, target_pb, 0)
                {
                    tracker.restore_target_pb = Some(target_pb.to_string());
                }
                Some(new_id.clone())
            };
            return PlaybackHotplugAction::Switch {
                apply_req: new_id,
                new_target_pb: new_target,
                reason: "hotplug_headphone_or_usb",
            };
        }
    }

    // 2. Dual USB+BT priority: upgrade to USB cable if matching USB is present while BT plays
    if hw_is_bt && usb_bt_active {
        return PlaybackHotplugAction::Switch {
            apply_req: "auto".to_string(),
            new_target_pb: None,
            reason: "dual_usb_over_bt",
        };
    }

    // 3. Preferred explicit target returned (after temporary loss/failover)
    let same_target = active_pb == target_pb || (target_pb == "bluetooth" && hw_is_bt);
    let target_available = device_list_has_type(dlist, target_pb, 0) && !(target_is_bt && usb_bt_active);
    if !target_auto && !same_target && target_available {
        return PlaybackHotplugAction::Switch {
            apply_req: target_pb.to_string(),
            new_target_pb: None,
            reason: "restored_target",
        };
    }

    // 4. Active endpoint lost OR stranded on WaveRT-Virtual-Clock when real hardware is available
    let active_kernel_lost =
        !hw_is_virtual && !hw_is_bt && !device_list_has_type(dlist, active_pb, 0);
    let active_bt_lost = hw_is_bt && !bt_now && !device_list_has_type(dlist, active_pb, 0);
    let any_hw_avail = device_list_has_any_type(dlist, 0, false) || bt_in_list;
    if (active_kernel_lost || active_bt_lost || hw_is_virtual) && any_hw_avail {
        if let Some(prev_pb) = tracker.restore_target_pb.clone() {
            if bt_in_list && !usb_bt_active && !prev_pb.starts_with("bluez") && prev_pb != "bluetooth" {
                return PlaybackHotplugAction::Switch {
                    apply_req: "auto".to_string(),
                    new_target_pb: None,
                    reason: "unplug_failover_bt",
                };
            }
            if device_list_has_type(dlist, &prev_pb, 0) {
                tracker.restore_target_pb = None;
                return PlaybackHotplugAction::Switch {
                    apply_req: prev_pb.clone(),
                    new_target_pb: Some(prev_pb),
                    reason: "unplug_restore_prev_target",
                };
            }
            tracker.restore_target_pb = None;
        }
        return PlaybackHotplugAction::Switch {
            apply_req: "auto".to_string(),
            new_target_pb: None,
            reason: if hw_is_virtual {
                "recover_from_virtual"
            } else {
                "active_lost_failover"
            },
        };
    }

    // 5. Auto-Priority steady-state upgrades
    if target_auto {
        let auto_upgrade_bt = !hw_is_bt && bt_in_list && !usb_bt_active;
        let auto_upgrade_hp_usb =
            !hw_is_bt && !is_hp_dev(active_pb, &active_name) && has_usb_hp_in_list;
        let auto_upgrade_nonusb_to_usb =
            !hw_is_bt && !is_usb_id(active_pb) && device_list_has_any_type(dlist, 0, true);
        if auto_upgrade_bt || auto_upgrade_hp_usb || auto_upgrade_nonusb_to_usb {
            return PlaybackHotplugAction::Switch {
                apply_req: "auto".to_string(),
                new_target_pb: None,
                reason: "auto_priority_upgrade",
            };
        }
    }

    PlaybackHotplugAction::None
}

struct AudiodgEngine {
    reconf_lock: Mutex<()>,
    hw_io_lock: Mutex<()>,
    hw_lock: Mutex<HwState>,
    sessions_lock: Mutex<SessionsState>,
    saved_bt: Mutex<SavedBtState>,
    midi_hub: Mutex<MidiHub>,
    midi_meta: Mutex<MidiMeta>,
    hotplug_tracker: Mutex<HotplugTracker>,

    bt: Arc<dyn BluetoothBackend>,
    alsa_hw: Arc<dyn AlsaPlaybackBackend>,
    pa_srv: OnceLock<PaServerHandle>,
    pw_brk: OnceLock<PwBrokerHandle>,
    hw_analog: AnalogStage,

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
    fn new(rt: &SalsaRuntimeState, bt: Arc<dyn BluetoothBackend>) -> Self {
        Self::with_backends(rt, bt, KernelAlsaBackend::global())
    }

    fn with_backends(
        rt: &SalsaRuntimeState,
        bt: Arc<dyn BluetoothBackend>,
        alsa_hw: Arc<dyn AlsaPlaybackBackend>,
    ) -> Self {
        Self {
            reconf_lock: Mutex::new(()),
            hw_io_lock: Mutex::new(()),
            hw_lock: Mutex::new(HwState::from_runtime(rt)),
            sessions_lock: Mutex::new(SessionsState::from_runtime(rt)),
            saved_bt: Mutex::new(SavedBtState::from_runtime(rt)),
            midi_hub: Mutex::new(MidiHub::new()),
            midi_meta: Mutex::new(MidiMeta::default()),
            hotplug_tracker: Mutex::new(HotplugTracker::default()),
            bt,
            alsa_hw,
            pa_srv: OnceLock::new(),
            pw_brk: OnceLock::new(),
            hw_analog: AnalogStage::new(),
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

    fn bt(&self) -> &dyn BluetoothBackend {
        &*self.bt
    }

    fn pa(&self) -> Option<&PaServerHandle> {
        self.pa_srv.get()
    }

    fn pw(&self) -> Option<&PwBrokerHandle> {
        self.pw_brk.get()
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
        let bt_snap = self.bt().snapshot();
        {
            let mut sb = self.saved_bt.lock().unwrap_or_else(|e| e.into_inner());
            if bt_snap.initialized {
                st.bt_cfg = self.bt().get_codec_config();
                sb.saved_bt_cfg = st.bt_cfg;
                sb.saved_desktop_vol_pct = bt_snap.desktop_volume_pct;
                sb.saved_desktop_muted = u32::from(bt_snap.desktop_muted);
                sb.saved_mic_vol_pct = bt_snap.desktop_mic_volume_pct;
                sb.saved_mic_muted = u32::from(bt_snap.desktop_mic_muted);
                sb.saved_mic_gain_ppm = bt_snap.desktop_mic_gain_ppm;
                let opt = self.bt().is_optional_mode();
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
        let _ = save_runtime_state(&st);
    }

    fn hw_analog_probe_and_sync(&self, re_probe: bool) {
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
        let bt_snap = self.bt().snapshot();
        if re_probe {
            self.hw_analog.probe(pb_card, cap_card);
        }
        self.hw_analog.sync(
            &cfg,
            bt_snap.desktop_volume_pct,
            bt_snap.desktop_muted,
            bt_snap.desktop_mic_volume_pct,
            bt_snap.desktop_mic_muted,
        );
    }

    fn maybe_apply_per_device_eq_for_sink(&self, sink_id: &str) {
        let trimmed = sink_id.trim();
        if trimmed.is_empty() || trimmed == "virtual" {
            return;
        }
        let doc = crate::state_conf::load_unified_settings_json_in(&crate::state_conf::config_dir_path());
        if !is_per_device_eq_enabled(&doc) {
            return;
        }
        let Some((eq, ext)) = extract_per_device_eq_for_sink(&doc, trimmed) else {
            return;
        };
        {
            let mut ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
            ss.eq_cfg = eq;
            ss.param_eq = ext;
        }
        let pname = c_str_to_string(&eq.preset_name);
        dbg_emit(
            LASAPI_DBG_ENGINE,
            LASAPI_DBG_INFO,
            0,
            0,
            "EQ_PER_DEV",
            trimmed,
            if pname.is_empty() { "custom" } else { &pname },
        );
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
        pre_dl: Option<&LasapiDeviceList>,
    ) -> i32 {
        let prev_pb = {
            let hws = self.hw_lock.lock().unwrap();
            c_str_to_string(&hws.active_playback_id)
        };
        let rc = self.apply_playback_endpoint_inner(
            requested_id,
            rate,
            period,
            req_bits,
            force_virtual,
            force_bt,
            pre_dl,
        );
        if !force_virtual {
            let new_pb = {
                let hws = self.hw_lock.lock().unwrap();
                c_str_to_string(&hws.active_playback_id)
            };
            if !new_pb.is_empty() && new_pb != prev_pb {
                self.maybe_apply_per_device_eq_for_sink(&new_pb);
            }
        }
        rc
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_playback_endpoint_inner(
        &self,
        requested_id: &str,
        rate: u32,
        period: u32,
        req_bits: u32,
        force_virtual: bool,
        force_bt: bool,
        pre_dl: Option<&LasapiDeviceList>,
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
                self.alsa_hw.close_pcm(old_fd);
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
        let mut local_dl = LasapiDeviceList::zeroed();
        let dlp: Option<&LasapiDeviceList> = if !force_virtual && req != "virtual" {
            Some(match pre_dl {
                Some(d) => d,
                None => {
                    self.bt().enumerate_devices(None, None, &mut local_dl);
                    &local_dl
                }
            })
        } else {
            None
        };

        let mut bt_snap = self.bt().snapshot();
        let known_bt_aliases = self.bt().known_audio_aliases();
        let matching_usb_idx = dlp.and_then(|d| {
            let alias = c_str_to_string(&bt_snap.device_alias);
            find_matching_usb_pb_idx_for_list(d, &alias, &known_bt_aliases)
        });
        let skip_bt_for_matching_usb = auto_mode && !force_bt && matching_usb_idx.is_some();
        if skip_bt_for_matching_usb {
            if let (Some(d), Some(u_idx)) = (dlp, matching_usb_idx) {
                let u_name = c_str_to_string(&d.devices[u_idx].name);
                eprintln!(
                    "[LASAPI-audiodg] Dual USB+BT device detected ({}) — prioritizing USB endpoint",
                    u_name
                );
                dbg_emit(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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
            let bt_opt = self.bt().is_optional_mode();
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
                self.bt().set_tx_enabled(false);
                bt_ready = false;
            }

            if !bt_ready && (want_bt || auto_want_bt) {
                self.bt().set_tx_enabled(true);
                for _ in 0..140 {
                    std::thread::sleep(Duration::from_millis(25));
                    bt_snap = self.bt().snapshot();
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
                self.bt().reconfigure_rate(rate);
                self.bt().set_tx_enabled(true);
                {
                    let mut hws = self.hw_lock.lock().unwrap();
                    hws.hw = next_hw;
                    copy_str_to_c(&mut hws.active_playback_id, &next_id);
                }
                let codec_show = if bt_codec.is_empty() { "A2DP" } else { &bt_codec };
                eprintln!(
                    "[LASAPI-audiodg] Active Pure BlueZ A2DP endpoint: {} ({} Hz -> BT {} Hz {})",
                    label, next_hw.sample_rate, bt_rate, codec_show
                );
                dbg_emit(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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
                eprintln!("[LASAPI-audiodg] Notice: BlueZ A2DP transport not yet acquired, probing kernel endpoints...");
            }
        }

        // Preserve tx_paused=false when falling back transiently on explicit BT / connected-BT Auto so
        // an in-flight Acquire/ConnectProfile completes and sync_hotplug_topology restores BT automatically.
        if should_disable_bt_tx_on_fallback(want_bt, auto_want_bt) {
            self.bt().set_tx_enabled(false);
        }

        if !force_virtual && !auto_mode && !want_bt && req != "virtual" {
            let dev_path = resolve_dev_id_to_path(req);
            if let Some(opened) = self.alsa_hw.open_pcm(&dev_path, rate, period, target_bits) {
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
                    "[LASAPI-audiodg] Opened direct kernel WaveRT endpoint: {} ({} Hz, {} ch, {}-bit, period={} frames / {:.2} ms)",
                    dev_path, next_hw.sample_rate, next_hw.channels, next_hw.format_bits, next_hw.period_frames, ms
                );
                dbg_emit(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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
                "[LASAPI-audiodg] Notice: {} busy or unavailable ({}), probing fallback endpoints...",
                dev_path, err_str
            );
            dbg_emit(
                LASAPI_DBG_ENGINE,
                LASAPI_DBG_WARN,
                0,
                0,
                "ENDPOINT_BUSY",
                req,
                &format!("dev={} err={}", dev_path, err_str),
            );
        }

        if let Some(dlp) = dlp {
            let n = (dlp.num_devices as usize).min(LASAPI_MAX_DEVICES);
            let mut cands: Vec<(usize, i32)> = Vec::new();
            for (i, e) in dlp.devices.iter().enumerate().take(n) {
                if e.is_capture != 0 || e.is_bluetooth != 0 {
                    continue;
                }
                let mut sc = score_card_by_index(e.card_index, &c_str_to_string(&e.name));
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
                if let Some(opened) = self.alsa_hw.open_pcm(&dev_path, rate, period, target_bits) {
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
                        "[LASAPI-audiodg] Auto-bound direct kernel WaveRT endpoint: {} ({} | {} Hz, {} ch, {}-bit, period={} frames / {:.2} ms)",
                        dev_path, e_name, next_hw.sample_rate, next_hw.channels, next_hw.format_bits, next_hw.period_frames, ms
                    );
                    dbg_emit(
                        LASAPI_DBG_ENGINE,
                        LASAPI_DBG_INFO,
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
            "[LASAPI-audiodg] Initialized deterministic WaveRT clock ({} Hz, period={} frames / {:.2} ms)",
            next_hw.sample_rate, next_hw.period_frames, ms
        );
        dbg_emit(
            LASAPI_DBG_ENGINE,
            if force_virtual {
                LASAPI_DBG_INFO
            } else {
                LASAPI_DBG_WARN
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
            self.bt().set_optional_mode(bt_optional != 0);
            self.bt().init_rates(rate, cap_rate, &target_cap, &target_pb);
            self.bt().set_sco_enabled(bt_cfg_saved.hfp_sco_enabled != 0);
            self.bt().set_codec_config(&bt_cfg_saved);
            if has_vol {
                self.bt().set_volume(d_vol as i32, i32::from(d_muted != 0));
                dbg_emit_vals(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
                    0,
                    0,
                    d_vol as i32,
                    i32::from(d_muted != 0),
                    0,
                    "INIT_HW_VOL",
                    "master_out",
                    &format!("restored vol={d_vol}% muted={}", i32::from(d_muted != 0)),
                );
                self.bt().set_mic_volume(m_vol as i32, i32::from(m_muted != 0));
                self.bt().set_mic_gain_ppm(
                    m_gain,
                    m_vol as i32,
                    i32::from(m_muted != 0),
                );
            }
            let bt_snap = self.bt().snapshot();
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

    fn fill_engine_status_impl(&self, st: &mut LasapiEngineStatus, include_pa_sessions: bool) {
        *st = LasapiEngineStatus::zeroed();
        let bt_snap = self.bt().snapshot();
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
        st.pa_server_running = u32::from(self.pa().is_some());

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
                    "LASAPI-MIDI-Hub"
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
            st.exclusive_owner_id = ss.exclusive_session_num;
            st.eq_cfg = ss.eq_cfg;
            st.param_eq = ss.param_eq;
            st.mic_apo = ss.mic_apo;
            let dt = dsp_telemetry();
            st.eq_band_levels_db = dt.band_levels_db;
            st.eq_dyn_delta_db = dt.dyn_delta_db;
            st.output_peak_l = dt.output_peak_l;
            st.output_peak_r = dt.output_peak_r;
            st.mic_apo_peak_in = dt.mic_apo_peak_in;
            st.mic_apo_peak_out = dt.mic_apo_peak_out;
            st.mic_rnnoise_active = dt.mic_rnnoise_active;

            let mut count = 0usize;
            for slot in &ss.sessions {
                if let Some(ring) = slot.ring.as_ref().filter(|_| slot.in_use) {
                    let tel = ring.telemetry();
                    let info = &mut st.sessions[count];
                    count += 1;
                    info.session_num = slot.session_num;
                    info.client_pid = slot.client_pid;
                    info.is_capture = u32::from(slot.is_capture);
                    info.client_name.copy_from_slice(&slot.client_name);
                    info.share_mode = slot.share_mode;
                    info.sample_rate = slot.sample_rate;
                    info.channels = slot.channels;
                    info.active = tel.active;
                    info.volume_milli = tel.volume_milli;
                    info.buffered_frames = tel.buffered_frames;
                    info.xruns = tel.xruns;
                    info.muted = tel.muted;
                    info.routing_flags = 0;
                    if info.is_capture != 0 {
                        info.target_device.copy_from_slice(&st.active_capture_id);
                    } else {
                        info.target_device.copy_from_slice(&st.active_playback_id);
                    }
                }
            }
            st.num_sessions = count as u32;
            st.hw_analog_cfg = ss.hw_analog_cfg;
        }

        st.hw_analog_info = self.hw_analog.info();
        st.hw_analog_info.reserved = if bt_has_cap && bt_cap_rate >= 8000 {
            bt_cap_rate
        } else {
            resolve_cap_sample_rate(st.hw_sample_rate, pref_cap)
        };

        if bt_snap.initialized {
            st.bt_codec_cfg = self.bt().get_codec_config();
        } else {
            let sb = self.saved_bt.lock().unwrap_or_else(|e| e.into_inner());
            st.bt_codec_cfg = sb.saved_bt_cfg;
        }

        if include_pa_sessions {
            if let Some(pa) = self.pa() {
                pa.fill_sessions(st);
            }
        }
    }

    fn enumerate_devices_safe(&self, out_list: &mut LasapiDeviceList) {
        let (pb_id, cap_id) = {
            let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            (
                c_str_to_string(&hws.active_playback_id),
                c_str_to_string(&hws.active_capture_id),
            )
        };
        self.bt()
            .enumerate_devices(Some(&pb_id), Some(&cap_id), out_list);
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
            let bt_snap = self.bt().snapshot();
            if bt_snap.initialized {
                self.bt().set_preferred_target(&target_pb);
            }
            self.apply_playback_endpoint(&target_pb, rate, period, bits, false, false, None);
            let hws = self.hw_lock.lock().unwrap();
            if hws.hw.sample_rate != 0 {
                hw_rate = hws.hw.sample_rate;
            }
        }

        if !target_cap.is_empty() {
            let eff_cap = resolve_cap_sample_rate(hw_rate, pref_cap);
            self.bt().open_capture(&target_cap, eff_cap);
            let bt_snap = self.bt().snapshot();
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
        if pb_id.is_some_and(|p| !p.is_empty()) {
            self.seed_hotplug_baseline(true);
        }
        self.save_state();
        drop(_reconf);

        if notify_pa {
            if let Some(pa) = self.pa() {
                pa.notify_change();
            }
        }
        0
    }

    fn reconfigure_engine_rate_impl(
        &self,
        mut new_rate: u32,
        mut new_period: u32,
        mut new_bits: u32,
        update_pref: bool,
    ) -> Result<(LasapiIpcResp, u32), i32> {
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
            if !(44_100..=192_000).contains(&new_rate) || !(64..=LASAPI_MAX_PERIOD as u32).contains(&new_period) {
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
            let resp = LasapiIpcResp {
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

        let mut bt_snap = self.bt().snapshot();
        if rate_changed && bt_snap.initialized {
            self.bt().reconfigure_rate(resp.actual_sample_rate);
            bt_snap = self.bt().snapshot();
            let cap_already_open = bt_snap.has_capture_hw && bt_snap.cap_pcm_fd >= 0;
            if !cap_already_open {
                self.bt().open_capture(
                    &target_cap,
                    resolve_cap_sample_rate(resp.actual_sample_rate, pref_cap),
                );
                bt_snap = self.bt().snapshot();
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
                if let Some(ring) = slot.ring.as_ref().filter(|_| slot.in_use) {
                    ring.set_period_frames(resp.actual_period_frames);
                }
            }
        }

        if rate_changed || period_changed || bits_changed {
            self.hw_analog_probe_and_sync(true);
        }
        self.save_state();
        drop(_reconf);

        if let Some(pa) = self.pa() {
            pa.notify_change();
        }
        Ok((resp, actual_bits))
    }

    fn reconfigure_capture_rate_impl(&self, new_cap_rate: u32) -> Option<LasapiIpcResp> {
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

        let bt_init = self.bt().snapshot().initialized;
        if bt_init {
            let req_cap = if target_cap.is_empty() {
                "auto"
            } else {
                &target_cap
            };
            self.bt().open_capture(req_cap, eff_cap);
            let bt_snap = self.bt().snapshot();
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

        if let Some(pa) = self.pa() {
            pa.notify_change();
        }

        Some(LasapiIpcResp {
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

    fn probe_card_hp_jacks() -> [bool; 32] {
        let mut out = [false; 32];
        for (c, slot) in out.iter_mut().enumerate() {
            let p = format!("/dev/snd/controlC{c}");
            if Path::new(&p).exists() {
                *slot = card_has_plugged_headphone_jack(c as i32);
            }
        }
        out
    }

    fn seed_hotplug_baseline(&self, clear_restore: bool) {
        let mut dlist = LasapiDeviceList::zeroed();
        self.enumerate_devices_safe(&mut dlist);
        let hp_jacks = Self::probe_card_hp_jacks();
        let mut ht = self.hotplug_tracker.lock().unwrap_or_else(|e| e.into_inner());
        ht.seen_pb_ids = collect_playback_ids(&dlist);
        ht.hp_jack_cards = hp_jacks;
        ht.initialized = true;
        if clear_restore {
            ht.restore_target_pb = None;
        }
    }

    fn sync_hotplug_topology(&self, force_virtual: bool, bt_now: bool) -> bool {
        let mut cap_deferred = false;
        let _reconf = self.reconf_lock.lock().unwrap();
        let mut dlist = LasapiDeviceList::zeroed();
        self.enumerate_devices_safe(&mut dlist);
        let hp_jacks = Self::probe_card_hp_jacks();
        let bt_has_cap = self.bt().snapshot().has_capture_hw;

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

        let usb_bt_active = any_usb_matches_bt(&dlist, self.bt());
        let pb_action = {
            let mut ht = self.hotplug_tracker.lock().unwrap_or_else(|e| e.into_inner());
            evaluate_playback_hotplug_with(
                force_virtual,
                &target_pb,
                &active_pb,
                hw_is_virtual,
                hw_is_bt,
                bt_now,
                usb_bt_active,
                &dlist,
                &hp_jacks,
                &mut ht,
                dev_id_is_usb_card,
                |id, nm| is_headphone_playback_card(parse_card_idx_from_id(id), nm),
            )
        };

        if let PlaybackHotplugAction::Switch {
            apply_req,
            new_target_pb,
            reason,
        } = pb_action
        {
            if let Some(ref updated_tgt) = new_target_pb {
                {
                    let mut hws = self.hw_lock.lock().unwrap();
                    copy_str_to_c(&mut hws.target_playback_id, updated_tgt);
                }
                if self.bt().snapshot().initialized {
                    self.bt().set_preferred_target(updated_tgt);
                }
            }
            eprintln!(
                "[LASAPI-audiodg] Hotplug playback switch ({reason}): req={apply_req}"
            );
            dbg_emit(
                LASAPI_DBG_ENGINE,
                LASAPI_DBG_INFO,
                0,
                0,
                "HOTPLUG_PB",
                &apply_req,
                reason,
            );
            self.apply_playback_endpoint(
                &apply_req,
                pref_r,
                pref_p,
                pref_b,
                false,
                false,
                Some(&dlist),
            );
            if new_target_pb.is_some() {
                self.save_state();
            }
        }
        {
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
                let cap_rc = self.bt().open_capture(
                    req_cap,
                    resolve_cap_sample_rate(eng_rate, pref_cap),
                );
                let bt_snap = self.bt().snapshot();
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
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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

    fn handle_create_session(&self, client_sock: UnixStream, req: &LasapiIpcReq) {
        let mut resp = LasapiIpcResp::zeroed();
        let req_name = c_str_to_string(&req.client_name);
        let cname_dbg = if req_name.is_empty() {
            "session".to_string()
        } else {
            req_name.clone()
        };

        let Some(ev_fd) = new_eventfd() else {
            resp.hr = E_OUTOFMEMORY;
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), &resp.to_bytes(), &[]);
            return;
        };

        let (hw_rate, hw_ch, hw_per) = {
            let hws = self.hw_lock.lock().unwrap();
            (hws.hw.sample_rate, hws.hw.channels, hws.hw.period_frames)
        };

        let mut ss = self.sessions_lock.lock().unwrap();
        if ss.exclusive_session_num >= 0 {
            resp.hr = AUDCLNT_E_DEVICE_IN_USE;
            let holder = ss.exclusive_session_num;
            drop(ss);
            dbg_emit_vals(
                LASAPI_DBG_LASAPI,
                LASAPI_DBG_WARN,
                req.client_pid,
                0,
                holder,
                0,
                resp.hr,
                "EXCL_DENIED",
                &cname_dbg,
                &format!("exclusive lock held by session #{holder}"),
            );
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), &resp.to_bytes(), &[]);
            return;
        }

        let Some(slot_idx) = ss.sessions.iter().position(|s| !s.in_use) else {
            resp.hr = E_OUTOFMEMORY;
            drop(ss);
            let _ = sendmsg_all_with_fds(client_sock.as_raw_fd(), &resp.to_bytes(), &[]);
            return;
        };

        let sid = ss.next_session_num;
        ss.next_session_num = ss.next_session_num.wrapping_add(1);

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
        let mut eff_ch = if req.channels != 0 && (req.channels as usize) <= LASAPI_MAX_CHANNELS {
            req.channels
        } else {
            hw_ch
        };
        if eff_ch == 0 || (eff_ch as usize) > LASAPI_MAX_CHANNELS {
            eff_ch = 2;
        }
        let mut buf_frames = req.requested_buffer_frames;
        if buf_frames < hw_per.saturating_mul(2) {
            buf_frames = hw_per.saturating_mul(4);
        }
        if buf_frames > LASAPI_RING_FRAMES as u32 {
            buf_frames = LASAPI_RING_FRAMES as u32;
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

        let mut auth_name = [0u8; 64];
        copy_str_to_c(&mut auth_name, s_name_str);

        let Some((ring, shm_fd)) = WaveRtEngineRing::create_sealed(SessionRingSpec {
            session_id: sid,
            client_pid: auth_pid,
            client_name: auth_name,
            share_mode: req.share_mode,
            stream_flags: req.stream_flags,
            is_capture: is_cap_u32 != 0,
            sample_rate: eff_rate,
            channels: eff_ch,
            format: crate::c_abi::LASAPI_FMT_FLOAT32,
            buffer_frames: buf_frames,
            period_frames: hw_per,
        }) else {
            resp.hr = E_OUTOFMEMORY;
            drop(ss);
            let _ = sendmsg_all_with_fds(sock_raw, &resp.to_bytes(), &[]);
            return;
        };

        let fds_to_send = [shm_fd.as_raw_fd(), ev_fd.as_raw_fd()];

        let is_loopback = (req.stream_flags & AUDCLNT_STREAMFLAGS_LOOPBACK) != 0;
        ss.sessions[slot_idx] = EngineSession {
            in_use: true,
            session_num: sid,
            client_pid: auth_pid,
            client_name: auth_name,
            share_mode: req.share_mode,
            client_sock: Some(client_sock),
            _shm_fd: Some(shm_fd),
            event_fd: Some(ev_fd),
            ring: Some(ring),
            resample: SessionResampleState::new(),
            sample_rate: eff_rate,
            channels: eff_ch,
            is_capture: is_cap_u32 != 0,
            is_loopback,
            last_pa_ctl_seq: 0,
        };
        if req.share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
            ss.exclusive_session_num = sid as i32;
        }

        resp.hr = S_OK;
        resp.session_id = sid;
        resp.actual_sample_rate = eff_rate;
        resp.actual_channels = eff_ch;
        resp.actual_buffer_frames = buf_frames;
        resp.actual_period_frames = hw_per;
        drop(ss);

        if sendmsg_all_with_fds(sock_raw, &resp.to_bytes(), &fds_to_send).is_err() {
            let mut ss = self.sessions_lock.lock().unwrap();
            if ss.sessions[slot_idx].in_use && ss.sessions[slot_idx].session_num == sid {
                destroy_session_slot(&mut ss, slot_idx);
            }
            return;
        }

        if is_cap_u32 == 0 && !is_loopback {
            if let Some(pa) = self.pa() {
                pa.register_lasapi_session(
                    slot_idx as u32,
                    sid,
                    auth_pid,
                    s_name_str,
                    eff_rate,
                    eff_ch,
                );
            }
        }

        if req.share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
            eprintln!(
                "[LASAPI-audiodg] Session #{} ('{}', PID {}) acquired LASAPI EXCLUSIVE hardware lock ({} Hz, period={})",
                sid, s_name_str, auth_pid, eff_rate, hw_per
            );
        } else {
            eprintln!(
                "[LASAPI-audiodg] Session #{} ('{}', PID {}) initialized in LASAPI {} mode ({} Hz -> {} Hz, {} ch, buf={})",
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
            LASAPI_DBG_LASAPI,
            LASAPI_DBG_INFO,
            req.client_pid,
            sid,
            eff_rate as i32,
            eff_ch as i32,
            S_OK,
            "LASAPI_SESSION",
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

    fn dispatch_control_req(&self, peer_pid: u32, req: ControlRequest) -> ControlResponse {
        match req {
            ControlRequest::GetStatus => {
                let mut st = LasapiEngineStatus::zeroed();
                self.fill_engine_status_impl(&mut st, true);
                let active_capture_rate_hz = st.effective_capture_rate_hz();
                ControlResponse {
                    hr: S_OK,
                    payload: ControlPayload::Status {
                        status: Box::new(st),
                        active_capture_rate_hz,
                    },
                }
            }
            ControlRequest::ListDevices => {
                let mut dlist = LasapiDeviceList::zeroed();
                self.enumerate_devices_safe(&mut dlist);
                ControlResponse {
                    hr: S_OK,
                    payload: ControlPayload::Devices(Box::new(dlist)),
                }
            }
            ControlRequest::SetDevices {
                playback_id,
                capture_id,
            } => {
                let pb = playback_id.as_deref().filter(|s| !s.is_empty());
                let cap = capture_id.as_deref().filter(|s| !s.is_empty());
                if pb.is_none() && cap.is_none() {
                    return ControlResponse::invalid();
                }
                self.switch_engine_devices_impl(pb, cap, true);
                let (sr, ch, pf, bf) = {
                    let hws = self.hw_lock.lock().unwrap();
                    (
                        hws.hw.sample_rate,
                        hws.hw.channels,
                        hws.hw.period_frames,
                        hws.hw.buffer_frames,
                    )
                };
                let tgt = pb.or(cap).unwrap_or("none");
                dbg_emit_vals(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
                    peer_pid,
                    0,
                    sr as i32,
                    pf as i32,
                    S_OK,
                    "DEV_SWITCH",
                    tgt,
                    &format!(
                        "pb={} cap={} -> {}Hz/{}",
                        pb.unwrap_or("-"),
                        cap.unwrap_or("-"),
                        sr,
                        pf
                    ),
                );
                ControlResponse {
                    hr: S_OK,
                    payload: ControlPayload::ClockAck {
                        sample_rate: sr,
                        channels: ch,
                        period_frames: pf,
                        buffer_frames: bf,
                    },
                }
            }
            ControlRequest::SetMasterVolume {
                dir,
                volume_pct,
                mute,
                client_tag,
            } => {
                let vol_pct = volume_pct.map_or(-1, |v| v.min(150) as i32);
                let mute_flag = mute.to_legacy_flag();
                let is_cap = matches!(dir, AudioDir::Capture);
                if is_cap {
                    self.bt().set_mic_volume(vol_pct, mute_flag);
                } else {
                    self.bt().set_volume(vol_pct, mute_flag);
                }
                self.hw_analog_probe_and_sync(false);
                if let Some(pa) = self.pa() {
                    pa.notify_volume(is_cap);
                }
                self.mark_state_dirty();
                let src = if is_cap {
                    "master_mic"
                } else if client_tag.is_empty() {
                    "master_out"
                } else {
                    client_tag.as_str()
                };
                dbg_emit_vals(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
                    peer_pid,
                    0,
                    vol_pct,
                    mute_flag,
                    S_OK,
                    "VOL_CHG",
                    src,
                    &format!(
                        "{} vol={vol_pct}% mute={mute_flag}",
                        if is_cap { "master_mic" } else { "master_out" }
                    ),
                );
                ControlResponse::ok()
            }
            ControlRequest::SetSessionVolume {
                session_id,
                volume_milli,
            } => {
                let mut hr = E_INVALIDARG;
                let mut found = false;
                let mut upd_pa: Option<(u32, u32, u32, bool)> = None;
                {
                    let ss = self.sessions_lock.lock().unwrap();
                    for (slot_idx, slot) in ss.sessions.iter().enumerate() {
                        if let Some(ring) = slot
                            .ring
                            .as_ref()
                            .filter(|_| slot.in_use && slot.session_num == session_id)
                        {
                            let muted_now = ring.set_volume_milli_query_muted(volume_milli);
                            if !slot.is_capture && !slot.is_loopback {
                                upd_pa =
                                    Some((slot_idx as u32, slot.session_num, volume_milli, muted_now));
                            }
                            hr = S_OK;
                            found = true;
                            break;
                        }
                    }
                }
                if let Some(pa) = self.pa() {
                    if let Some((w_slot, w_sid, vm, mu)) = upd_pa {
                        pa.update_lasapi_session_vol(w_slot, w_sid, vm, mu);
                    }
                    if !found && pa.set_stream_volume(session_id, volume_milli / 10) == 0 {
                        hr = S_OK;
                    }
                }
                dbg_emit_vals(
                    LASAPI_DBG_LASAPI,
                    if hr == S_OK { LASAPI_DBG_INFO } else { LASAPI_DBG_WARN },
                    peer_pid,
                    session_id,
                    volume_milli as i32,
                    i32::from(found),
                    hr,
                    "VOL_CHG",
                    "stream",
                    &format!("milli={volume_milli} lasapi={}", i32::from(found)),
                );
                ControlResponse::err(hr)
            }
            ControlRequest::SetSessionControl {
                target,
                volume_milli,
                mute,
                route_device,
            } => {
                let mut hr = E_INVALIDARG;
                let tgt_dev = route_device.unwrap_or_default();
                let vm_raw = volume_milli.map_or(u32::MAX, |v| v.min(1500));
                match target {
                    SessionTarget::NativeShm(sid) => {
                        let mut upd_pa: Option<(u32, u32, u32, bool)> = None;
                        {
                            let ss = self.sessions_lock.lock().unwrap();
                            for (slot_idx, slot) in ss.sessions.iter().enumerate() {
                                if let Some(ring) = slot
                                    .ring
                                    .as_ref()
                                    .filter(|_| slot.in_use && slot.session_num == sid)
                                {
                                    let op = match mute {
                                        MuteOp::Unmute => RingMuteOp::Unmute,
                                        MuteOp::Mute => RingMuteOp::Mute,
                                        MuteOp::Toggle => RingMuteOp::Toggle,
                                        MuteOp::Keep => RingMuteOp::Keep,
                                    };
                                    let (vm, mu) = ring.apply_control(volume_milli, op);
                                    if !slot.is_capture && !slot.is_loopback {
                                        upd_pa = Some((slot_idx as u32, slot.session_num, vm, mu));
                                    }
                                    hr = S_OK;
                                    break;
                                }
                            }
                        }
                        if let Some((w_slot, w_sid, vm, mu)) = upd_pa {
                            if let Some(pa) = self.pa() {
                                pa.update_lasapi_session_vol(w_slot, w_sid, vm, mu);
                            }
                        }
                    }
                    SessionTarget::PulseStream(sid) => {
                        if let Some(pa) = self.pa() {
                            let vol_pct = volume_milli.map_or(-1, |v| (v.min(1500) / 10) as i32);
                            let mute_mode = mute.to_legacy_flag();
                            let dev_opt = (!tgt_dev.is_empty()).then_some(tgt_dev.as_str());
                            if pa.control_stream(sid, vol_pct, mute_mode, dev_opt) == 0 {
                                hr = S_OK;
                            }
                        }
                    }
                }
                let sid = target.raw_id();
                let tshow = if tgt_dev.is_empty() { "-" } else { &tgt_dev };
                dbg_emit_vals(
                    LASAPI_DBG_LASAPI,
                    if hr == S_OK { LASAPI_DBG_INFO } else { LASAPI_DBG_WARN },
                    peer_pid,
                    sid,
                    vm_raw as i32,
                    mute.to_legacy_flag(),
                    hr,
                    "SESS_CTL",
                    tshow,
                    &format!(
                        "sid={sid} vol_m={vm_raw} mute_op={} dev={tshow}",
                        mute.to_legacy_flag()
                    ),
                );
                ControlResponse::err(hr)
            }
            ControlRequest::InjectMidi(mut ev) => {
                if ev.size == 0 || ev.size > 3 {
                    dbg_emit_vals(
                        LASAPI_DBG_MIDI,
                        LASAPI_DBG_WARN,
                        peer_pid,
                        0,
                        ev.data[0] as i32,
                        ev.data[1] as i32,
                        E_INVALIDARG,
                        "MIDI_INJECT",
                        "ipc",
                        &format!(
                            "status=0x{:02X} d1={} d2={} size={}",
                            ev.data[0], ev.data[1], ev.data[2], ev.size
                        ),
                    );
                    return ControlResponse::invalid();
                }
                if ev.timestamp_ns == 0 {
                    ev.timestamp_ns = mono_ns();
                }
                {
                    let ss = self.sessions_lock.lock().unwrap();
                    for slot in &ss.sessions {
                        if let Some(ring) = slot.ring.as_ref().filter(|_| slot.in_use) {
                            let _ = ring.push_midi(&ev);
                        }
                    }
                }
                if midi_is_key_press(&ev) {
                    self.midi_injected_events.fetch_add(1, Ordering::Relaxed);
                }
                dbg_emit_vals(
                    LASAPI_DBG_MIDI,
                    LASAPI_DBG_INFO,
                    peer_pid,
                    0,
                    ev.data[0] as i32,
                    ev.data[1] as i32,
                    S_OK,
                    "MIDI_INJECT",
                    "ipc",
                    &format!(
                        "status=0x{:02X} d1={} d2={} size={}",
                        ev.data[0], ev.data[1], ev.data[2], ev.size
                    ),
                );
                ControlResponse::ok()
            }
            ControlRequest::SetCaptureClock { sample_rate } => {
                let active_pb_copy = {
                    let hws = self.hw_lock.lock().unwrap();
                    c_str_to_string(&hws.active_playback_id)
                };
                if let Some(r) = self.reconfigure_capture_rate_impl(sample_rate) {
                    eprintln!(
                        "[LASAPI-Salsa] Capture/Mic Clock reconfigured -> {} Hz ({})",
                        r.actual_sample_rate,
                        if sample_rate == 0 {
                            "Auto/Follow Master"
                        } else {
                            "Fixed Independent Mic Clock"
                        }
                    );
                    dbg_emit_vals(
                        LASAPI_DBG_ENGINE,
                        LASAPI_DBG_INFO,
                        peer_pid,
                        0,
                        r.actual_sample_rate as i32,
                        r.actual_period_frames as i32,
                        r.hr,
                        "CFG_CAP_RATE",
                        &active_pb_copy,
                        &format!("CAP {}Hz (pref={sample_rate})", r.actual_sample_rate),
                    );
                    ControlResponse {
                        hr: r.hr,
                        payload: ControlPayload::ClockAck {
                            sample_rate: r.actual_sample_rate,
                            channels: r.actual_channels,
                            period_frames: r.actual_period_frames,
                            buffer_frames: r.actual_buffer_frames,
                        },
                    }
                } else {
                    ControlResponse::invalid()
                }
            }
            ControlRequest::SetPlaybackClock {
                sample_rate,
                period_frames,
                format_bits,
            } => {
                let (cur_rate, cur_period, cur_bits, active_pb_copy) = {
                    let hws = self.hw_lock.lock().unwrap();
                    (
                        hws.hw.sample_rate,
                        hws.hw.period_frames,
                        hws.hw.format_bits,
                        c_str_to_string(&hws.active_playback_id),
                    )
                };
                let req_rate = if sample_rate != 0 {
                    sample_rate
                } else {
                    cur_rate
                };
                if sample_rate != 0 && !(44_100..=192_000).contains(&sample_rate) {
                    return ControlResponse::invalid();
                }
                let new_period = if period_frames != 0 {
                    period_frames
                } else {
                    cur_period
                };
                let new_bits = if format_bits != 0 {
                    format_bits
                } else {
                    cur_bits
                };
                match self.reconfigure_engine_rate_impl(req_rate, new_period, new_bits, true) {
                    Ok((r, actual_bits)) => {
                        let ms = (1000.0 * (r.actual_period_frames as f64))
                            / (r.actual_sample_rate as f64);
                        eprintln!(
                            "[LASAPI-Salsa] Engine reconfigured -> {} Hz | {}-bit | Quantum {} frames ({:.2} ms)",
                            r.actual_sample_rate, actual_bits, r.actual_period_frames, ms
                        );
                        dbg_emit_vals(
                            LASAPI_DBG_ENGINE,
                            LASAPI_DBG_INFO,
                            peer_pid,
                            0,
                            r.actual_sample_rate as i32,
                            r.actual_period_frames as i32,
                            S_OK,
                            "CFG_RECONF",
                            &active_pb_copy,
                            &format!(
                                "FIXED {}Hz {}bit quantum={}",
                                r.actual_sample_rate, actual_bits, r.actual_period_frames
                            ),
                        );
                        ControlResponse {
                            hr: S_OK,
                            payload: ControlPayload::ClockAck {
                                sample_rate: r.actual_sample_rate,
                                channels: r.actual_channels,
                                period_frames: r.actual_period_frames,
                                buffer_frames: r.actual_buffer_frames,
                            },
                        }
                    }
                    Err(_) => {
                        dbg_emit_vals(
                            LASAPI_DBG_ENGINE,
                            LASAPI_DBG_WARN,
                            peer_pid,
                            0,
                            req_rate as i32,
                            new_period as i32,
                            E_INVALIDARG,
                            "CFG_RECONF",
                            "invalid",
                            &format!("req={req_rate}Hz period={new_period}"),
                        );
                        ControlResponse::invalid()
                    }
                }
            }
            ControlRequest::SetDynamicEq(mut eq) => {
                clamp_eq_cfg(&mut eq);
                let eq_copy = {
                    let mut ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                    ss.eq_cfg = *eq;
                    ss.eq_cfg
                };
                {
                    let _reconf = self.reconf_lock.lock().unwrap_or_else(|e| e.into_inner());
                    self.save_state();
                }
                let pname = c_str_to_string(&eq_copy.preset_name);
                dbg_emit_vals(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
                    peer_pid,
                    0,
                    eq_copy.enabled as i32,
                    eq_copy.dynamic_mode as i32,
                    S_OK,
                    "EQ_SET",
                    if pname.is_empty() { "custom" } else { &pname },
                    &format!(
                        "en={} dyn={} preamp={:.1}dB lim={}",
                        eq_copy.enabled,
                        eq_copy.dynamic_mode,
                        eq_copy.preamp_db,
                        eq_copy.limiter_enabled
                    ),
                );
                ControlResponse::ok()
            }
            ControlRequest::SetParametricEq(mut pay) => {
                clamp_eq_cfg(&mut pay.eq);
                clamp_param_eq_ext(&mut pay.ext);
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
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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
                ControlResponse::ok()
            }
            ControlRequest::SetMicApo(mut cfg) => {
                clamp_mic_apo_cfg(&mut cfg);
                {
                    let mut ss = self.sessions_lock.lock().unwrap();
                    ss.mic_apo = cfg;
                }
                {
                    let _reconf = self.reconf_lock.lock().unwrap();
                    self.save_state();
                }
                dbg_emit_vals(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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
                        if cfg.sidetone_enabled != 0 {
                            cfg.sidetone_pct
                        } else {
                            0
                        }
                    ),
                );
                ControlResponse::ok()
            }
            ControlRequest::SetBtCodec(cfg) => {
                let bt_init = self.bt().is_initialized();
                let bt_show = if bt_init {
                    self.bt().set_sco_enabled(cfg.hfp_sco_enabled != 0);
                    self.bt().set_codec_config(&cfg);
                    let out_cfg = self.bt().get_codec_config();
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
                if let Some(pa) = self.pa() {
                    pa.notify_change();
                }
                dbg_emit_vals(
                    LASAPI_DBG_BLUEZ,
                    LASAPI_DBG_INFO,
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
                ControlResponse::ok()
            }
            ControlRequest::BtConnection { op, mac } => {
                let action = op.to_u32();
                let (rc, msg_str, mut flags) = self.bt().connection_ctl(action, &mac);
                let mut dlist = LasapiDeviceList::zeroed();
                self.enumerate_devices_safe(&mut dlist);
                let usb_matches_bt = any_usb_matches_bt(&dlist, self.bt());
                if usb_matches_bt {
                    flags |= LASAPI_BT_STAT_USB_MATCHES_BT;
                }
                if op != BtConnOp::Query {
                    {
                        let mut sb = self.saved_bt.lock().unwrap();
                        sb.bt_optional_mode = u32::from((flags & LASAPI_BT_STAT_OPTIONAL) != 0);
                    }
                    let (target_pb, pref_r, pref_p, pref_b, hw_is_bt) = {
                        let hws = self.hw_lock.lock().unwrap();
                        (
                            c_str_to_string(&hws.target_playback_id),
                            if hws.pref_sample_rate != 0 {
                                hws.pref_sample_rate
                            } else {
                                hws.hw.sample_rate
                            },
                            if hws.pref_period_frames != 0 {
                                hws.pref_period_frames
                            } else {
                                hws.hw.period_frames
                            },
                            if hws.pref_format_bits != 0 {
                                hws.pref_format_bits
                            } else {
                                hws.hw.format_bits
                            },
                            hws.hw.is_bluetooth,
                        )
                    };
                    let want_bt_target =
                        target_pb == "bluetooth" || target_pb.starts_with("bluez");
                    let reeval_connect =
                        rc >= 0 && ((target_pb == "auto" && !usb_matches_bt) || want_bt_target);
                    let should_reapply = match op {
                        BtConnOp::Connect => reeval_connect,
                        BtConnOp::SetAutoConn | BtConnOp::Reset => {
                            reeval_connect && (flags & LASAPI_BT_STAT_CONNECTED) != 0
                        }
                        BtConnOp::Disconnect => hw_is_bt,
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
                        } else if op == BtConnOp::Connect && !hw_is_bt {
                            self.bt().set_tx_enabled(false);
                        }
                        self.save_state();
                    }
                    if let Some(pa) = self.pa() {
                        pa.notify_change();
                    }
                    let snap_after = self.bt().snapshot();
                    flags = 0;
                    if self.bt().is_optional_mode() {
                        flags |= LASAPI_BT_STAT_OPTIONAL;
                    }
                    if snap_after.has_bluez_transport
                        || !c_str_to_string(&snap_after.device_path).is_empty()
                    {
                        flags |= LASAPI_BT_STAT_CONNECTED;
                    }
                    if snap_after.has_bluez_transport
                        && snap_after.direct_socket_mode
                        && snap_after.bt_fd >= 0
                    {
                        flags |= LASAPI_BT_STAT_TRANSPORT;
                    }
                    if usb_matches_bt {
                        flags |= LASAPI_BT_STAT_USB_MATCHES_BT;
                    }
                }
                if op != BtConnOp::Query || rc < 0 {
                    dbg_emit_vals(
                        LASAPI_DBG_BLUEZ,
                        if rc >= 0 { LASAPI_DBG_INFO } else { LASAPI_DBG_WARN },
                        0,
                        0,
                        action as i32,
                        flags as i32,
                        rc,
                        "BT_CONN_CTL",
                        if mac.is_empty() { "bluetooth" } else { &mac },
                        &msg_str,
                    );
                }
                let payload = if op == BtConnOp::Reset {
                    ControlPayload::BtFlagsMsg {
                        flags,
                        message: msg_str,
                    }
                } else {
                    ControlPayload::BtFlags(flags)
                };
                ControlResponse {
                    hr: if rc >= 0 { S_OK } else { E_FAIL },
                    payload,
                }
            }
            ControlRequest::SetHwAnalog(mut cfg) => {
                clamp_hw_analog_cfg(&mut cfg);
                {
                    let mut ss = self.sessions_lock.lock().unwrap();
                    ss.hw_analog_cfg = cfg;
                }
                self.hw_analog_probe_and_sync(true);
                {
                    let _reconf = self.reconf_lock.lock().unwrap();
                    self.save_state();
                }
                if let Some(pa) = self.pa() {
                    pa.notify_change();
                }
                dbg_emit_vals(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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
                ControlResponse::ok()
            }
            ControlRequest::SaveConfig => {
                {
                    let _reconf = self.reconf_lock.lock().unwrap_or_else(|e| e.into_inner());
                    self.save_state();
                }
                dbg_emit(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
                    peer_pid,
                    0,
                    "CFG_SAVE",
                    "salsa_settings.json",
                    "overwrote unified settings config",
                );
                ControlResponse::ok()
            }
            ControlRequest::ReloadConfig => {
                let mut rt = default_runtime_state();
                load_runtime_state(&mut rt);
                clamp_eq_cfg(&mut rt.eq_cfg);
                clamp_param_eq_ext(&mut rt.param_eq);
                clamp_mic_apo_cfg(&mut rt.mic_apo);
                clamp_hw_analog_cfg(&mut rt.hw_analog_cfg);

                {
                    let mut ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                    ss.eq_cfg = rt.eq_cfg;
                    ss.param_eq = rt.param_eq;
                    ss.mic_apo = rt.mic_apo;
                    ss.hw_analog_cfg = rt.hw_analog_cfg;
                }

                let bt_init = self.bt().is_initialized();
                {
                    let mut sb = self.saved_bt.lock().unwrap_or_else(|e| e.into_inner());
                    sb.saved_bt_cfg = rt.bt_cfg;
                    sb.bt_optional_mode = rt.bt_optional_mode;
                    if rt.has_saved_vol_state {
                        sb.saved_desktop_vol_pct = rt.desktop_volume_pct;
                        sb.saved_desktop_muted = rt.desktop_muted;
                        sb.saved_mic_vol_pct = rt.desktop_mic_volume_pct;
                        sb.saved_mic_muted = rt.desktop_mic_muted;
                        sb.saved_mic_gain_ppm = rt.desktop_mic_gain_ppm;
                        sb.has_saved_vol_state = true;
                    }
                }
                if bt_init {
                    self.bt().set_optional_mode(rt.bt_optional_mode != 0);
                    self.bt().set_sco_enabled(rt.bt_cfg.hfp_sco_enabled != 0);
                    self.bt().set_codec_config(&rt.bt_cfg);
                    if rt.has_saved_vol_state {
                        self.bt().set_volume(
                            rt.desktop_volume_pct as i32,
                            i32::from(rt.desktop_muted != 0),
                        );
                        self.bt().set_mic_gain_ppm(
                            rt.desktop_mic_gain_ppm,
                            rt.desktop_mic_volume_pct as i32,
                            i32::from(rt.desktop_mic_muted != 0),
                        );
                    }
                }

                let (cur_r, cur_p, cur_b, cur_cap_r, cur_pb, cur_cap) = {
                    let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
                    (
                        if hws.pref_sample_rate != 0 {
                            hws.pref_sample_rate
                        } else {
                            hws.hw.sample_rate
                        },
                        if hws.pref_period_frames != 0 {
                            hws.pref_period_frames
                        } else {
                            hws.hw.period_frames
                        },
                        if hws.pref_format_bits != 0 {
                            hws.pref_format_bits
                        } else {
                            hws.hw.format_bits
                        },
                        hws.pref_cap_sample_rate,
                        c_str_to_string(&hws.target_playback_id),
                        c_str_to_string(&hws.target_capture_id),
                    )
                };
                let new_pb = c_str_to_string(&rt.target_playback_id);
                let new_cap = c_str_to_string(&rt.target_capture_id);
                if new_pb != cur_pb || new_cap != cur_cap {
                    self.switch_engine_devices_impl(
                        (!new_pb.is_empty()).then_some(new_pb.as_str()),
                        (!new_cap.is_empty()).then_some(new_cap.as_str()),
                        false,
                    );
                }
                if rt.sample_rate != cur_r
                    || rt.period_frames != cur_p
                    || rt.format_bits != cur_b
                {
                    let _ = self.reconfigure_engine_rate_impl(
                        rt.sample_rate,
                        rt.period_frames,
                        rt.format_bits,
                        true,
                    );
                }
                if rt.cap_sample_rate != cur_cap_r {
                    let _ = self.reconfigure_capture_rate_impl(rt.cap_sample_rate);
                }
                self.hw_analog_probe_and_sync(true);
                if let Some(pa) = self.pa() {
                    pa.notify_change();
                    pa.notify_volume(false);
                }
                dbg_emit(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
                    peer_pid,
                    0,
                    "CFG_RELOAD",
                    "salsa_settings.json",
                    "reloaded unified settings config",
                );
                ControlResponse::ok()
            }
        }
    }

    fn handle_client_cmd(&self, mut stream: UnixStream, req: &LasapiIpcReq) {
        if req.cmd == LASAPI_CMD_CREATE_SESSION {
            self.handle_create_session(stream, req);
            return;
        }
        if req.cmd == LASAPI_CMD_CONTROL_V2 {
            let len = req.requested_buffer_frames as usize;
            if len == 0 || len > MAX_CONTROL_V2_FRAME {
                let _ = write_response_frame(&mut stream, &ControlResponse::invalid());
                return;
            }
            let mut buf = vec![0u8; len];
            if stream.read_exact(&mut buf).is_err() {
                return;
            }
            let Some(ctrl_req) = decode_request(&buf) else {
                let _ = write_response_frame(&mut stream, &ControlResponse::invalid());
                return;
            };
            let resp = self.dispatch_control_req(req.client_pid, ctrl_req);
            let _ = write_response_frame(&mut stream, &resp);
            return;
        }

        let mut legacy_payload = Vec::new();
        let expected_payload = match req.cmd {
            LASAPI_CMD_SET_PARAM_EQ => Some(mem::size_of::<LasapiParamEqPayload>()),
            LASAPI_CMD_SET_MIC_APO => Some(mem::size_of::<LasapiMicApoCfg>()),
            LASAPI_CMD_SET_BT_CODEC => Some(mem::size_of::<LasapiBtCodecCfg>()),
            LASAPI_CMD_SET_HW_ANALOG => Some(mem::size_of::<LasapiHwAnalogCfg>()),
            _ => None,
        };
        if let Some(exp) = expected_payload {
            if req.requested_buffer_frames as usize != exp {
                let mut err_resp = LasapiIpcResp::zeroed();
                err_resp.hr = E_INVALIDARG;
                let _ = send_all_nosignal(stream.as_raw_fd(), &err_resp.to_bytes());
                return;
            }
            legacy_payload.resize(exp, 0);
            if stream.read_exact(&mut legacy_payload).is_err() {
                let mut err_resp = LasapiIpcResp::zeroed();
                err_resp.hr = E_INVALIDARG;
                let _ = send_all_nosignal(stream.as_raw_fd(), &err_resp.to_bytes());
                return;
            }
        }

        let Some(ctrl_req) = legacy_ipc_req_to_control(req, &legacy_payload) else {
            return;
        };
        let resp = self.dispatch_control_req(req.client_pid, ctrl_req);
        match resp.payload {
            ControlPayload::Status { status, .. } => {
                let _ = send_all_nosignal(stream.as_raw_fd(), &status.to_bytes());
            }
            ControlPayload::Devices(dlist) => {
                let _ = send_all_nosignal(stream.as_raw_fd(), &dlist.to_bytes());
            }
            _ => {
                let leg = control_resp_to_legacy(&resp);
                let _ = send_all_nosignal(stream.as_raw_fd(), &leg.to_bytes());
            }
        }
    }

    fn record_hw_xrun(&self, hw_snap: &KernelWaveRtHw, xerr: i32) {
        let xc = self.hw_xruns.fetch_add(1, Ordering::Relaxed) + 1;
        if xc <= 16 || xc.is_power_of_two() || xc.is_multiple_of(256) {
            let dp = c_str_to_string(&hw_snap.device_path);
            dbg_emit_vals(
                LASAPI_DBG_ALSA,
                LASAPI_DBG_WARN,
                0,
                0,
                xc as i32,
                xerr,
                -xerr,
                "HW_XRUN",
                &dp,
                &format!("ALSA PCM XRUN #{xc} (errno={xerr})"),
            );
        }
    }

    fn write_alsa_period_and_handle_fault(
        &self,
        hw_snap: &KernelWaveRtHw,
        mix_buf: &[f32],
        period: u32,
        ch: u32,
        master_gain: f32,
        softknee: bool,
        pcm_pack_bytes: &mut [u8],
    ) -> (bool, bool) {
        match self.alsa_hw.write_packed_period(
            hw_snap,
            mix_buf,
            period,
            ch,
            master_gain,
            softknee,
            pcm_pack_bytes,
        ) {
            AlsaWriteOutcome::Ok => (true, true),
            AlsaWriteOutcome::RecoveredXrun { xerr } => {
                self.record_hw_xrun(hw_snap, xerr);
                (false, true)
            }
            AlsaWriteOutcome::DeviceLost { dead_err, xerr } => {
                if let Some(xe) = xerr {
                    self.record_hw_xrun(hw_snap, xe);
                }
                let dp = c_str_to_string(&hw_snap.device_path);
                dbg_emit_vals(
                    LASAPI_DBG_ALSA,
                    LASAPI_DBG_ERR,
                    0,
                    0,
                    dead_err,
                    0,
                    -dead_err,
                    "HW_DEV_LOST",
                    &dp,
                    &format!("Kernel playback device removed (errno={dead_err}) -> Virtual Clock"),
                );
                eprintln!(
                    "[LASAPI-audiodg] Kernel playback device {} removed (errno={}), dropping to virtual clock until failover.",
                    dp, dead_err
                );
                let mut hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
                if hws.hw.fd == hw_snap.fd && hws.hw.fd >= 0 {
                    self.alsa_hw.close_pcm(hws.hw.fd);
                    hws.hw.fd = -1;
                    hws.hw.is_virtual = true;
                    copy_str_to_c(&mut hws.hw.device_path, "WaveRT-Virtual-Clock");
                    copy_str_to_c(&mut hws.active_playback_id, "virtual");
                }
                (false, false)
            }
            AlsaWriteOutcome::TransientErr { had_xrun, xerr } => {
                if had_xrun {
                    self.record_hw_xrun(hw_snap, xerr);
                }
                (false, false)
            }
        }
    }

    fn run_rt_engine_thread(&self) {
        let mut prio_set = 0u32;
        for p in [65, 50, 20, 10] {
            if try_realtime_fifo(p) {
                prio_set = p as u32;
                eprintln!(
                    "[LASAPI-MMCSS] Real-time SCHED_FIFO priority {} enabled for audio graph thread.",
                    prio_set
                );
                break;
            }
        }
        if prio_set == 0 {
            eprintln!("[LASAPI-MMCSS] Running in standard scheduler priority (grant rtprio for SCHED_FIFO).");
        }
        self.rt_priority.store(prio_set, Ordering::Relaxed);

        let mut mix_buf = vec![0.0f32; LASAPI_MAX_PERIOD * LASAPI_MAX_CHANNELS];
        let mut lasapi_doi_buf = vec![0.0f32; LASAPI_MAX_PERIOD * LASAPI_MAX_CHANNELS];
        let mut pa_stereo = vec![0.0f32; LASAPI_MAX_PERIOD * 2];
        let mut mic_buf = vec![0.0f32; LASAPI_MAX_PERIOD * 2];
        let mut native_mic_buf = vec![0.0f32; LASAPI_MAX_PERIOD * 2];
        let mut pcm_pack_bytes = vec![0u8; LASAPI_MAX_PERIOD * LASAPI_MAX_CHANNELS * 4];
        let mut midi_ev: [LasapiMidiEvent; 64] = [LasapiMidiEvent::default(); 64];

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
            if (period as usize) > LASAPI_MAX_PERIOD {
                period = LASAPI_MAX_PERIOD as u32;
            }
            if (ch as usize) > LASAPI_MAX_CHANNELS {
                ch = LASAPI_MAX_CHANNELS as u32;
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
            self.bt().read_capture_native(
                &mut native_mic_buf,
                LASAPI_MAX_PERIOD as u32,
                &mut mic_frames,
                &mut mic_rate,
                period,
                rate,
                &mut mic_read_cursor,
            );
            if (mic_frames as usize) > LASAPI_MAX_PERIOD {
                mic_frames = LASAPI_MAX_PERIOD as u32;
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
            let cap_makeup = self.hw_analog.cap_makeup_linear();
            scale_cap_clip_slice(&mut native_mic_buf[..(mic_frames as usize) * 2], cap_makeup);
            let mic_cfg = {
                let ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                ss.mic_apo
            };
            process_capture_mic_apo(&mut native_mic_buf, mic_frames as usize, mic_rate, &mic_cfg);

            // 2. MIDI hub poll
            let num_midi = {
                let mut mh = self.midi_hub.lock().unwrap_or_else(|e| e.into_inner());
                let n = mh.poll_events(&mut midi_ev, now_ns, rate, period);
                self.midi_hw_events.store(mh.total_events(), Ordering::Relaxed);
                n
            };

            let mut pa_mix_ns = 0u64;
            let eq_ns;
            let monitor_is_pa_stereo;
            {
                let mut ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
                if num_midi > 0 {
                    for slot in &ss.sessions {
                        if let Some(ring) = slot.ring.as_ref().filter(|_| slot.in_use) {
                            for ev in &midi_ev[..num_midi] {
                                let _ = ring.push_midi(ev);
                            }
                        }
                    }
                }

                let exclusive_id = ss.exclusive_session_num;
                let pa = self.pa();
                let doi_mask = pa.map_or(0, PaServerHandle::lasapi_doi_mask);
                if exclusive_id >= 0 {
                    for (slot_idx, slot) in ss.sessions.iter_mut().enumerate() {
                        if slot.in_use && (slot.session_num as i32) == exclusive_id && slot.ring.is_some() {
                            if let Some(ref ring) = slot.ring {
                                ring.set_hw_clock_ns(now_ns);
                            }
                            consume_lasapi_playback_slot(
                                slot_idx,
                                slot,
                                pa,
                                doi_mask,
                                &mut lasapi_doi_buf,
                                &mut mix_buf,
                                (period, ch, rate, now_ns),
                                true,
                            );
                            break;
                        }
                    }
                } else {
                    for (slot_idx, slot) in ss.sessions.iter_mut().enumerate() {
                        if slot.in_use && slot.ring.is_some() {
                            if let Some(ref ring) = slot.ring {
                                ring.set_hw_clock_ns(now_ns);
                            }
                            if !slot.is_capture && !slot.is_loopback {
                                consume_lasapi_playback_slot(
                                    slot_idx,
                                    slot,
                                    pa,
                                    doi_mask,
                                    &mut lasapi_doi_buf,
                                    &mut mix_buf,
                                    (period, ch, rate, now_ns),
                                    false,
                                );
                            }
                        }
                    }
                    if let Some(pa) = pa {
                        let t_pa0 = mono_ns();
                        if ch == 2 {
                            pa.mix_playback(&mut mix_buf[..p_us * 2], period, rate);
                        } else {
                            pa_stereo[..p_us * 2].fill(0.0);
                            pa.mix_playback(&mut pa_stereo[..p_us * 2], period, rate);
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
                process_dynamic_eq(&mut mix_buf, p_us, ch_us, rate, &ss.eq_cfg, &ss.param_eq);
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

                // 4. Mic upsample + LASAPI capture push
                let mut need_eng_mic =
                    mic_cfg.enabled != 0 && mic_cfg.sidetone_enabled != 0 && mic_cfg.sidetone_pct > 0;
                if !need_eng_mic {
                    for slot in &ss.sessions {
                        if slot.in_use && slot.is_capture && slot.ring.is_some() && !slot.is_loopback {
                            need_eng_mic = true;
                            break;
                        }
                    }
                }
                if need_eng_mic {
                    resample_mic_cubic_slice(
                        &native_mic_buf,
                        mic_frames as usize,
                        &mut mic_buf,
                        p_us,
                        &mut mic_up_st,
                    );
                }

                for slot in &ss.sessions {
                    if let Some(ring) = slot.ring.as_ref().filter(|_| slot.in_use) {
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
                            ring.push_capture_stereo(cap_src, p_us);
                        }
                        if let Some(ref efd) = slot.event_fd {
                            crate::unix_sys::eventfd_signal(efd.as_raw_fd());
                        }
                    }
                }
            }

            if let Some(pa) = self.pa() {
                let t_pa1 = mono_ns();
                let mon_slice = if monitor_is_pa_stereo {
                    &pa_stereo[..p_us * 2]
                } else {
                    &mix_buf[..p_us * 2]
                };
                let mic_slice = (mic_frames > 0)
                    .then_some(&native_mic_buf[..(mic_frames as usize) * 2]);
                pa.push_capture(mic_slice, mic_rate, Some(mon_slice), rate);
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
                    if hw_snap.is_bluetooth && self.bt().is_initialized() {
                        let drive_gain = self.hw_analog.drive_linear();
                        let softknee = self.hw_analog.softknee_active();
                        apply_bt_hw_analog_drive(&mut mix_buf[..p_us * ch_us], drive_gain, softknee);
                        self.bt().write_period(&mix_buf[..p_us * ch_us], period, ch);
                    } else if !hw_snap.is_virtual && hw_snap.fd >= 0 {
                        let (d_vol, d_muted) = self.bt().rt_master_vol();
                        let mut master_gain = if d_muted {
                            0.0f32
                        } else if self.hw_analog.is_dac_unity_active() && d_vol <= 100 {
                            1.0
                        } else {
                            (d_vol as f32) / 100.0
                        };
                        master_gain *= self.hw_analog.drive_linear();
                        let softknee = self.hw_analog.softknee_active();

                        let (clear_pace, update_wake) = self.write_alsa_period_and_handle_fault(
                            &hw_snap,
                            &mix_buf,
                            period,
                            ch,
                            master_gain,
                            softknee,
                            &mut pcm_pack_bytes,
                        );
                        if clear_pace {
                            need_timer_pace = false;
                        }
                        if update_wake {
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
            record_rt_period(
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
    if slot_idx >= LASAPI_MAX_SESSIONS || !ss.sessions[slot_idx].in_use {
        return;
    }
    let sid = ss.sessions[slot_idx].session_num;
    let cpid = ss.sessions[slot_idx].client_pid;
    let cname = c_str_to_string(&ss.sessions[slot_idx].client_name);
    let label = if cname.is_empty() {
        "session".to_string()
    } else {
        cname
    };
    if (sid as i32) == ss.exclusive_session_num {
        ss.exclusive_session_num = -1;
        eprintln!(
            "[LASAPI-audiodg] Exclusive mode released by session {}; Shared Mode restored.",
            sid
        );
        dbg_emit(
            LASAPI_DBG_LASAPI,
            LASAPI_DBG_INFO,
            cpid,
            sid,
            "EXCL_RELEASE",
            &label,
            "exclusive mode released; shared mode restored",
        );
    }
    dbg_emit(
        LASAPI_DBG_LASAPI,
        LASAPI_DBG_INFO,
        cpid,
        sid,
        "LASAPI_DESTROY",
        &label,
        &format!("session slot={slot_idx} closed"),
    );
    ss.sessions[slot_idx] = EngineSession::empty();
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn consume_lasapi_playback_slot(
    slot_idx: usize,
    slot: &mut EngineSession,
    pa: Option<&PaServerHandle>,
    doi_mask: u32,
    lasapi_doi_buf: &mut [f32],
    mix_buf: &mut [f32],
    spec: (u32, u32, u32, u64),
    exclusive: bool,
) {
    let Some(ref ring) = slot.ring else {
        return;
    };
    let (period, ch, rate, now_ns) = spec;
    if let Some(pa_srv) = pa {
        if let Some((new_vol, new_mut)) =
            pa_srv.poll_lasapi_ctl(slot_idx as u32, &mut slot.last_pa_ctl_seq)
        {
            ring.set_volume_and_mute(new_vol, new_mut);
        }
    }
    let watch_doi = pa.is_some() && slot_idx < 32 && ((doi_mask >> slot_idx) & 1) != 0;
    let total_samples = (period as usize) * (ch as usize);
    let rep = if watch_doi && total_samples <= lasapi_doi_buf.len() && total_samples <= mix_buf.len()
    {
        let doi_slice = &mut lasapi_doi_buf[..total_samples];
        doi_slice.fill(0.0);
        let r = ring.consume_playback(&mut slot.resample, doi_slice, period, ch, rate, true, now_ns);
        if let Some(pa_srv) = pa {
            pa_srv.push_lasapi_period(slot_idx as u32, doi_slice, period, ch);
        }
        if exclusive {
            mix_buf[..total_samples].copy_from_slice(doi_slice);
        } else {
            for (dst, src) in mix_buf[..total_samples].iter_mut().zip(doi_slice.iter()) {
                *dst += *src;
            }
        }
        r
    } else {
        ring.consume_playback(
            &mut slot.resample,
            mix_buf,
            period,
            ch,
            rate,
            exclusive,
            now_ns,
        )
    };
    if rep.pre_active != 0 && rep.pre_live {
        static LAST_LASAPI_XRUN_LOG_NS: AtomicU64 = AtomicU64::new(0);
        let avail = rep.pre_avail;
        let empty_trip = !rep.pre_pending && rep.post_pending;
        let partial_trip = avail > 0 && avail < u64::from(period);
        let recover_trip = rep.post_xruns > rep.pre_xruns;
        if empty_trip || partial_trip || recover_trip {
            let prev_t = LAST_LASAPI_XRUN_LOG_NS.load(Ordering::Relaxed);
            if now_ns.saturating_sub(prev_t) >= 200_000_000 {
                LAST_LASAPI_XRUN_LOG_NS.store(now_ns, Ordering::Relaxed);
                let cname = c_str_to_string(&slot.client_name);
                let tag = if partial_trip {
                    "LASAPI_PARTIAL"
                } else if recover_trip {
                    "LASAPI_XRUN"
                } else {
                    "LASAPI_UNDERRUN"
                };
                dbg_emit_vals(
                    LASAPI_DBG_LASAPI,
                    LASAPI_DBG_WARN,
                    slot.client_pid,
                    slot.session_num,
                    avail.min(i32::MAX as u64) as i32,
                    period as i32,
                    rep.post_xruns as i32,
                    tag,
                    if cname.is_empty() { "session" } else { &cname },
                    &format!(
                        "sid={} avail={avail}/{period} buf={} xruns={}",
                        slot.session_num, rep.pre_buf, rep.post_xruns
                    ),
                );
            }
        }
    }
}

impl PaEngineHost for AudiodgEngine {
    fn fill_status(&self, out: &mut LasapiEngineStatus) {
        self.fill_engine_status_impl(out, false);
    }

    fn enumerate_devices(&self, out: &mut LasapiDeviceList) {
        self.enumerate_devices_safe(out);
    }

    fn switch_devices(&self, pb_id: Option<&str>, cap_id: Option<&str>) -> i32 {
        self.switch_engine_devices_impl(pb_id, cap_id, false)
    }

    fn set_master_volume(&self, is_capture: bool, vol_pct: i32, mute_flag: i32) -> i32 {
        let rc = if is_capture {
            self.bt().set_mic_volume(vol_pct, mute_flag)
        } else {
            self.bt().set_volume(vol_pct, mute_flag)
        };
        dbg_emit_vals(
            LASAPI_DBG_PA,
            LASAPI_DBG_INFO,
            0,
            u32::from(is_capture),
            vol_pct,
            mute_flag,
            rc,
            "PA_MASTER_VOL",
            if is_capture { "master_mic" } else { "master_out" },
            &format!("vol={vol_pct}% mute={mute_flag}"),
        );
        self.hw_analog_probe_and_sync(false);
        if rc == 0 {
            self.mark_state_dirty();
        }
        rc
    }

    fn set_mic_gain_ppm(&self, gain_ppm: u32, vol_pct: i32, mute_flag: i32) -> i32 {
        let rc = self.bt().set_mic_gain_ppm(gain_ppm, vol_pct, mute_flag);
        self.hw_analog_probe_and_sync(false);
        if rc == 0 {
            self.mark_state_dirty();
        }
        rc
    }
}

impl DebugEngineHost for AudiodgEngine {
    fn build_snapshot_json(&self, rt: &LasapiDbgRtStats) -> JsonValue {
        let mut st = LasapiEngineStatus::zeroed();
        self.fill_engine_status_impl(&mut st, true);

        let (tgt_pb_id, tgt_cap_id, pref_cap, pref_pb_rate, pb_is_hw, active_pb_copy) = {
            let hws = self.hw_lock.lock().unwrap_or_else(|e| e.into_inner());
            (
                c_str_to_string(&hws.target_playback_id),
                c_str_to_string(&hws.target_capture_id),
                hws.pref_cap_sample_rate,
                hws.pref_sample_rate,
                !hws.hw.is_virtual && !hws.hw.is_bluetooth,
                c_str_to_string(&hws.active_playback_id),
            )
        };

        let bt_snap = self.bt().snapshot();
        let cap_rd = self.cap_read_cursor.load(Ordering::Relaxed);
        let cap_ring_fill = bt_snap.cap_hw_write_frame.saturating_sub(cap_rd);
        let cap_id_s = c_str_to_string(&bt_snap.cap_device_id);
        let duplex_match = matches!(
            (parse_hw_card_idx(&active_pb_copy), parse_hw_card_idx(&cap_id_s)),
            (Some(pbc), Some(capc)) if pbc == capc
        );
        let cap_duplex_locked = u8::from(
            bt_snap.has_capture_hw
                && pb_is_hw
                && pref_pb_rate != 0
                && duplex_match
                && st.hw_sample_rate != pref_pb_rate,
        );

        let (midi_init, midi_raw, midi_ctrl, midi_dev, midi_seq_cli, midi_seq_port) = {
            let mm = self.midi_meta.lock().unwrap_or_else(|e| e.into_inner());
            (
                u8::from(mm.initialized),
                u8::from(mm.rawmidi_open),
                c_str_to_string(&mm.controller_name),
                c_str_to_string(&mm.dev_path),
                mm.seq_client_id,
                mm.seq_in_port,
            )
        };
        let midi_events_routed = self
            .midi_hw_events
            .load(Ordering::Relaxed)
            .saturating_add(self.midi_injected_events.load(Ordering::Relaxed));

        let mut lasapi_sessions = Vec::new();
        {
            let ss = self.sessions_lock.lock().unwrap_or_else(|e| e.into_inner());
            for slot in &ss.sessions {
                if let Some(ring) = slot.ring.as_ref().filter(|_| slot.in_use) {
                    let tel = ring.telemetry();
                    let is_cap = u32::from(slot.is_capture);
                    lasapi_sessions.push(JsonValue::obj(vec![
                        ("session_id", JsonValue::Int(i64::from(slot.session_num))),
                        ("pid", JsonValue::Int(i64::from(slot.client_pid))),
                        ("name", JsonValue::Str(c_str_to_string(&slot.client_name))),
                        ("share_mode", JsonValue::Int(i64::from(slot.share_mode))),
                        ("stream_flags", JsonValue::Int(i64::from(tel.stream_flags))),
                        ("is_capture", JsonValue::Int(i64::from(is_cap))),
                        ("rate", JsonValue::Int(i64::from(slot.sample_rate))),
                        ("channels", JsonValue::Int(i64::from(slot.channels))),
                        ("active", JsonValue::Int(i64::from(tel.active))),
                        ("muted", JsonValue::Int(i64::from(tel.muted))),
                        ("volume_milli", JsonValue::Int(i64::from(tel.volume_milli))),
                        ("buffer_frames", JsonValue::Int(i64::from(tel.buffer_frames))),
                        ("period_frames", JsonValue::Int(i64::from(tel.period_frames))),
                        ("buffered_frames", JsonValue::from_u64(tel.buffered_frames)),
                        ("xruns", JsonValue::from_u64(tel.xruns)),
                    ]));
                }
            }
        }

        let (pa_clients, pa_streams) = self.pa().map_or_else(
            || (JsonValue::Array(Vec::new()), JsonValue::Array(Vec::new())),
            PaServerHandle::debug_snapshot_json,
        );
        let pw_json = self
            .pw()
            .map_or_else(empty_debug_json, PwBrokerHandle::debug_snapshot_json);

        let st_pb_id = c_str_to_string(&st.active_playback_id);
        let eff_pb_id = if st_pb_id.is_empty() { tgt_pb_id } else { st_pb_id };
        let st_cap_id = c_str_to_string(&st.active_capture_id);
        let eff_cap_id = if st_cap_id.is_empty() { tgt_cap_id } else { st_cap_id };

        JsonValue::obj(vec![
            (
                "engine",
                JsonValue::obj(vec![
                    ("hw_device", JsonValue::Str(c_str_to_string(&st.hw_device))),
                    (
                        "hw_capture_device",
                        JsonValue::Str(c_str_to_string(&st.hw_capture_device)),
                    ),
                    ("midi_device", JsonValue::Str(c_str_to_string(&st.midi_device))),
                    ("active_playback_id", JsonValue::Str(eff_pb_id)),
                    ("active_capture_id", JsonValue::Str(eff_cap_id)),
                    ("sample_rate", JsonValue::Int(i64::from(st.hw_sample_rate))),
                    ("channels", JsonValue::Int(i64::from(st.hw_channels))),
                    ("period_frames", JsonValue::Int(i64::from(st.hw_period_frames))),
                    ("buffer_frames", JsonValue::Int(i64::from(st.hw_buffer_frames))),
                    ("format_bits", JsonValue::Int(i64::from(st.hw_format_bits))),
                    (
                        "desktop_volume_pct",
                        JsonValue::Int(i64::from(st.desktop_volume_pct)),
                    ),
                    ("desktop_muted", JsonValue::Int(i64::from(st.desktop_muted))),
                    (
                        "desktop_mic_volume_pct",
                        JsonValue::Int(i64::from(st.desktop_mic_volume_pct)),
                    ),
                    (
                        "desktop_mic_muted",
                        JsonValue::Int(i64::from(st.desktop_mic_muted)),
                    ),
                    (
                        "realtime_priority",
                        JsonValue::Int(i64::from(st.realtime_priority)),
                    ),
                    (
                        "exclusive_owner_id",
                        JsonValue::Int(i64::from(st.exclusive_owner_id)),
                    ),
                    ("total_cycles", JsonValue::from_u64(st.total_cycles)),
                    ("hw_xruns", JsonValue::from_u64(st.hw_xruns)),
                    (
                        "midi_events_routed",
                        JsonValue::from_u64(st.midi_events_routed),
                    ),
                    ("eq_enabled", JsonValue::Int(i64::from(st.eq_cfg.enabled))),
                    (
                        "eq_dynamic_mode",
                        JsonValue::Int(i64::from(st.eq_cfg.dynamic_mode)),
                    ),
                    (
                        "eq_preamp_db",
                        JsonValue::rounded_float(f64::from(st.eq_cfg.preamp_db), 100.0),
                    ),
                    (
                        "eq_dyn_threshold_db",
                        JsonValue::rounded_float(f64::from(st.eq_cfg.dyn_threshold_db), 100.0),
                    ),
                    (
                        "eq_dyn_ratio",
                        JsonValue::rounded_float(f64::from(st.eq_cfg.dyn_ratio), 100.0),
                    ),
                    (
                        "eq_limiter_enabled",
                        JsonValue::Int(i64::from(st.eq_cfg.limiter_enabled)),
                    ),
                    (
                        "output_peak_l",
                        JsonValue::rounded_float(f64::from(st.output_peak_l), 10_000.0),
                    ),
                    (
                        "output_peak_r",
                        JsonValue::rounded_float(f64::from(st.output_peak_r), 10_000.0),
                    ),
                    (
                        "pa_server_running",
                        JsonValue::Int(i64::from(st.pa_server_running)),
                    ),
                    ("pa_clients", JsonValue::Int(i64::from(st.pa_clients))),
                    (
                        "eq_preset",
                        JsonValue::Str(c_str_to_string(&st.eq_cfg.preset_name)),
                    ),
                ]),
            ),
            ("rt", rt_stats_to_json(rt)),
            (
                "bluez",
                JsonValue::obj(vec![
                    ("active", JsonValue::Int(i64::from(st.bt_active))),
                    ("initialized", JsonValue::Int(i64::from(bt_snap.initialized))),
                    (
                        "endpoints_registered",
                        JsonValue::Int(i64::from(bt_snap.endpoints_registered)),
                    ),
                    (
                        "direct_socket",
                        JsonValue::Int(i64::from(bt_snap.direct_socket_mode)),
                    ),
                    (
                        "has_transport",
                        JsonValue::Int(i64::from(bt_snap.has_bluez_transport)),
                    ),
                    ("tx_paused", JsonValue::Int(i64::from(bt_snap.tx_paused))),
                    ("sample_rate", JsonValue::Int(i64::from(bt_snap.bt_sample_rate))),
                    ("channels", JsonValue::Int(i64::from(bt_snap.bt_channels))),
                    ("delay_ms", JsonValue::Int(i64::from(st.bt_delay_ms))),
                    ("mtu_r", JsonValue::Int(i64::from(bt_snap.read_mtu))),
                    ("mtu_w", JsonValue::Int(i64::from(bt_snap.write_mtu))),
                    ("avrcp_vol", JsonValue::Int(i64::from(bt_snap.avrcp_volume))),
                    ("tx_ring_fill", JsonValue::from_u64(bt_snap.tx_ring_fill)),
                    ("tx_ring_cap", JsonValue::Int(i64::from(LASAPI_BT_RING_FRAMES))),
                    ("tx_packets", JsonValue::from_u64(bt_snap.tx_packets_sent)),
                    ("tx_underruns", JsonValue::from_u64(bt_snap.tx_underruns)),
                    ("tx_errors", JsonValue::from_u64(bt_snap.tx_errors)),
                    ("alias", JsonValue::Str(c_str_to_string(&st.bt_alias))),
                    ("codec", JsonValue::Str(c_str_to_string(&st.bt_codec))),
                    (
                        "device_path",
                        JsonValue::Str(c_str_to_string(&bt_snap.device_path)),
                    ),
                    (
                        "transport_path",
                        JsonValue::Str(c_str_to_string(&bt_snap.transport_path)),
                    ),
                    (
                        "endpoint_path",
                        JsonValue::Str(c_str_to_string(&bt_snap.endpoint_path)),
                    ),
                ]),
            ),
            (
                "capture",
                JsonValue::obj(vec![
                    ("active", JsonValue::Int(i64::from(bt_snap.has_capture_hw))),
                    ("hw_rate", JsonValue::Int(i64::from(bt_snap.cap_hw_rate))),
                    ("hw_channels", JsonValue::Int(i64::from(bt_snap.cap_hw_channels))),
                    ("hw_format", JsonValue::Int(i64::from(bt_snap.cap_hw_format))),
                    ("ring_fill", JsonValue::from_u64(cap_ring_fill)),
                    ("pref_rate", JsonValue::Int(i64::from(pref_cap))),
                    ("measured_rate", JsonValue::Int(i64::from(bt_snap.cap_measured_rate))),
                    ("kick_count", JsonValue::Int(i64::from(bt_snap.cap_kick_count))),
                    ("is_usb", JsonValue::Int(i64::from(bt_snap.cap_is_usb))),
                    ("duplex_locked", JsonValue::Int(i64::from(cap_duplex_locked))),
                    ("device_id", JsonValue::Str(cap_id_s)),
                    (
                        "pcm_path",
                        JsonValue::Str(c_str_to_string(&bt_snap.cap_pcm_path)),
                    ),
                    (
                        "alias",
                        JsonValue::Str(c_str_to_string(&bt_snap.capture_device_alias)),
                    ),
                ]),
            ),
            (
                "midi",
                JsonValue::obj(vec![
                    ("initialized", JsonValue::Int(i64::from(midi_init))),
                    ("rawmidi_open", JsonValue::Int(i64::from(midi_raw))),
                    ("seq_client", JsonValue::Int(i64::from(midi_seq_cli))),
                    ("seq_in_port", JsonValue::Int(i64::from(midi_seq_port))),
                    ("events_routed", JsonValue::from_u64(midi_events_routed)),
                    ("controller", JsonValue::Str(midi_ctrl)),
                    ("dev_path", JsonValue::Str(midi_dev)),
                ]),
            ),
            ("lasapi_sessions", JsonValue::Array(lasapi_sessions)),
            ("pa_clients", pa_clients),
            ("pa_streams", pa_streams),
            ("pw", pw_json),
            ("wire", wire_stats_to_json()),
        ])
    }

    fn handle_debug_cmd(&self, cmd: &str) -> Result<String, (i32, String)> {
        if cmd == "RESET_NET_CLIENTS" {
            let pa_n = self.pa().map_or(0, PaServerHandle::reset_clients);
            let pw_n = self.pw().map_or(0, PwBrokerHandle::reset_clients);
            record_trip(FS_MANUAL_RESET, b"Manual PA/PW network client flush via Debugger");
            let msg = format!(
                "Flushed {pa_n} PA + {pw_n} PW network clients (audio engine uninterrupted)"
            );
            dbg_emit(
                LASAPI_DBG_ENGINE,
                LASAPI_DBG_INFO,
                current_pid(),
                0,
                "NET_FAILSAFE_RESET",
                "PA/PW",
                &msg,
            );
            return Ok(msg);
        }
        if cmd == "RESET_PA_CLIENTS" {
            let pa_n = self.pa().map_or(0, PaServerHandle::reset_clients);
            record_trip(
                FS_MANUAL_RESET,
                b"Manual PA audio client flush via Debugger (PW preserved)",
            );
            let msg = format!(
                "Flushed {pa_n} PA audio clients (ScreenCast & audio engine preserved)"
            );
            dbg_emit(
                LASAPI_DBG_ENGINE,
                LASAPI_DBG_INFO,
                current_pid(),
                0,
                "NET_FAILSAFE_RESET",
                "PA",
                &msg,
            );
            return Ok(msg);
        }

        let Some(pw) = self.pw() else {
            return Err((-19, "PW video broker not initialized".into()));
        };

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
                pw.set_preview_ex(en, ttl, mw, mh, fps);
                return Ok(format!(
                    "preview={} ttl={}ms target={}x{}@{}fps",
                    if en { "on" } else { "off" },
                    ttl,
                    mw,
                    mh,
                    fps
                ));
            }
            return Err((-ERRNO_EINVAL, "invalid PW_PREVIEW syntax".into()));
        }

        if let Some(rest) = cmd.strip_prefix("PW_CAST_LIMITS ") {
            let vals: Vec<u32> = rest
                .split_whitespace()
                .filter_map(|t| t.parse::<u32>().ok())
                .collect();
            if vals.len() >= 3 {
                let (mw, mh, fps) = (vals[0], vals[1], vals[2]);
                pw.set_cast_limits(mw, mh, fps);
                return Ok(format!("cast_limits={mw}x{mh}@{fps}fps"));
            }
            return Err((-ERRNO_EINVAL, "invalid PW_CAST_LIMITS syntax".into()));
        }

        if cmd == "PW_SELF_TEST" {
            return pw.self_test().map_err(|m| (-1, m));
        }

        if cmd == "PW_SYNC_MODE" {
            let rc = pw.sync_mode();
            let msg = format!("sync_mode rc={rc}");
            return if rc == 0 { Ok(msg) } else { Err((rc, msg)) };
        }

        Err((-ERRNO_ENOSYS, format!("Unknown debug command: {cmd}")))
    }
}

pub fn run_audiodg_from_env() -> i32 {
    std::env::remove_var("LD_LIBRARY_PATH");
    claim_systemd_unix_listeners();

    let args: Vec<String> = std::env::args().collect();

    for arg in args.iter().skip(1) {
        if matches!(arg.as_str(), "--version" | "-V" | "--license" | "--legal") {
            if let Some(rc) = crate::legal::handle_common_cli_flag("lasapi-audiodg", arg) {
                return rc;
            }
        }
    }

    let mut sock_path = resolve_sock_path_common("LASAPI_SOCKET", "audiodg.sock", LASAPI_SOCKET_DEFAULT);

    let mut rt = default_runtime_state();
    load_runtime_state(&mut rt);
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
                        "[LASAPI-audiodg] Ignoring invalid --rate '{}', keeping {} Hz",
                        args[i + 1], rate_arg
                    ),
                }
                i += 2;
            }
            "--period" if i + 1 < args.len() => {
                match args[i + 1].trim().parse::<u32>() {
                    Ok(p) if (64..=(LASAPI_MAX_PERIOD as u32)).contains(&p) => period_arg = p,
                    _ => eprintln!(
                        "[LASAPI-audiodg] Ignoring invalid --period '{}', keeping {} frames",
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
    if !(64..=(LASAPI_MAX_PERIOD as u32)).contains(&period_arg) {
        period_arg = if period_arg == 0 {
            512
        } else {
            period_arg.clamp(64, LASAPI_MAX_PERIOD as u32)
        };
    }

    if !force_virtual && sock_path != LASAPI_SOCKET_DEFAULT {
        let _ = fs::remove_file(LASAPI_SOCKET_DEFAULT);
    }

    install_term_signals();
    rt.sample_rate = rate_arg;
    rt.period_frames = period_arg;
    rt.format_bits = bits_arg;

    let engine = Arc::new(AudiodgEngine::new(&rt, RealBluezBackend::global()));

    // Bind `audiodg.sock` BEFORE `init_hardware()` so active games/WASAPI clients
    // opening streams across a daemon restart connect immediately into the kernel
    // listen backlog instead of hitting ENOENT/ECONNREFUSED during BlueZ negotiation.
    let Some(listener) = bind_unix_listener_0600(&sock_path) else {
        eprintln!("bind failed for {sock_path}");
        return 1;
    };

    // Start the Video-Only PipeWire-0 ScreenCast broker BEFORE `init_hardware()` so
    // KDE `KPipeWire`/`MediaMonitor` and `xdg-desktop-portal` clients reconnecting
    // across daemon restarts complete their PipeWire handshake immediately without
    // waiting on ALSA/BlueZ RFCOMM/L2CAP negotiation.
    if let Some(pw) = start_pw_broker(None) {
        let _ = engine.pw_brk.set(pw);
        eprintln!("[LASAPI-audiodg] Built-in Video-Only PipeWire ScreenCast broker initialized.");
    }

    engine.init_hardware(
        device_arg.as_deref(),
        rate_arg,
        period_arg,
        bits_arg,
        force_virtual,
        force_bt,
    );
    engine.seed_hotplug_baseline(true);

    if !(force_virtual && std::env::var("LASAPI_VIRTUAL_NO_MIDI").as_deref() == Ok("1")) {
        let mut mh = engine.midi_hub.lock().unwrap();
        let _ = mh.init();
        let snap = mh.meta_snapshot();
        let mut mm = engine.midi_meta.lock().unwrap();
        mm.initialized = snap.initialized;
        mm.rawmidi_open = snap.rawmidi_open;
        mm.controller_name.copy_from_slice(&snap.controller_name);
        mm.dev_path.copy_from_slice(&snap.dev_path);
        mm.seq_client_id = snap.seq_client_id;
        mm.seq_in_port = snap.seq_in_port;
    }

    if let Some(pa) = start_pa_server(Arc::clone(&engine) as Arc<dyn PaEngineHost>) {
        let _ = engine.pa_srv.set(pa);
        eprintln!("[LASAPI-audiodg] Built-in PulseAudio native protocol v35 server active.");
    }

    debug_start(Arc::clone(&engine) as Arc<dyn DebugEngineHost>);
    {
        let hws = engine.hw_lock.lock().unwrap();
        let pb_id = c_str_to_string(&hws.active_playback_id);
        let devp = c_str_to_string(&hws.hw.device_path);
        let cap_id = c_str_to_string(&hws.active_capture_id);
        dbg_emit(
            LASAPI_DBG_ENGINE,
            LASAPI_DBG_INFO,
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
        "[LASAPI-audiodg] Listening on {} (Ready for LASAPI Shared/Exclusive, MIDI Hub & Native PA/ALSA/JACK)",
        sock_path
    );

    let init_bt = engine.bt().snapshot();
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
    let mut last_virtual_failover_fp: Option<u64> = None;

    let owner_id = posix_owner_id();
    while engine.is_running() {
        let mut pfd: Vec<PollFd> = Vec::with_capacity(1 + LASAPI_MAX_SESSIONS);
        let mut slot_map: Vec<isize> = Vec::with_capacity(1 + LASAPI_MAX_SESSIONS);
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
                        let mut unreg_sid = 0u32;
                        {
                            let mut ss = engine.sessions_lock.lock().unwrap();
                            if ss.sessions[slot_idx].in_use {
                                let sid = ss.sessions[slot_idx].session_num;
                                let is_pb_lasapi =
                                    !ss.sessions[slot_idx].is_capture && !ss.sessions[slot_idx].is_loopback;
                                let cpid = ss.sessions[slot_idx].client_pid;
                                let cname = c_str_to_string(&ss.sessions[slot_idx].client_name);
                                eprintln!(
                                    "[LASAPI-audiodg] Session #{} ('{}', PID {}) closed.",
                                    sid, cname, cpid
                                );
                                destroy_session_slot(&mut ss, slot_idx);
                                if is_pb_lasapi {
                                    unreg_sid = sid;
                                }
                            }
                        }
                        if unreg_sid != 0 {
                            if let Some(pa) = engine.pa() {
                                pa.unregister_lasapi_session(slot_idx as u32, unreg_sid);
                            }
                        }
                    }
                }
            }

            if (pfd[0].revents & POLLIN) != 0 {
                while let Ok((stream, _)) = listener.accept() {
                    let _ = stream.set_nonblocking(false);
                    let fd = stream.as_raw_fd();
                    let uc_ok = socket_peercred(fd)
                        .map(|uc| uc.uid == owner_id || uc.uid == 0)
                        .unwrap_or(false);
                    if !uc_ok {
                        continue;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                    let mut req_buf = [0u8; mem::size_of::<LasapiIpcReq>()];
                    let mut stream = stream;
                    if stream.read_exact(&mut req_buf).is_ok() {
                        if let Some(req) = LasapiIpcReq::from_bytes(&req_buf) {
                            engine.handle_client_cmd(stream, &req);
                        }
                    }
                }
            }
        }

        let now_w_ns = mono_ns();
        if now_w_ns < next_watcher_ns {
            continue;
        }
        next_watcher_ns = now_w_ns.saturating_add(250_000_000);

        let bt_snap = engine.bt().snapshot();
        let bt_now = bt_snap.has_bluez_transport
            && bt_snap.direct_socket_mode
            && bt_snap.bt_fd >= 0
            && !bt_snap.tx_paused;
        let avrcp_now = bt_snap.avrcp_volume;
        let muted_now = bt_snap.desktop_muted;
        let bt_alias = c_str_to_string(&bt_snap.device_alias);
        let bt_codec = c_str_to_string(&bt_snap.codec_name);
        let bt_rate = bt_snap.bt_sample_rate;

        let hw_fp = engine.bt().hw_fingerprint();
        let hw_changed = hw_fp_init && hw_fp != prev_hw_fp;
        if !hw_fp_init {
            prev_hw_fp = hw_fp;
            hw_fp_init = true;
        }
        let stranded_virtual = !force_virtual && {
            let hws = engine.hw_lock.lock().unwrap();
            hws.hw.is_virtual && c_str_to_string(&hws.target_playback_id) != "virtual"
        };
        if !stranded_virtual || hw_changed {
            last_virtual_failover_fp = None;
        }
        let need_virtual_failover = stranded_virtual && last_virtual_failover_fp != Some(hw_fp);

        if hw_changed || bt_now != prev_bt_has || pending_usb_cap_settle || need_virtual_failover {
            if bt_now != prev_bt_has {
                dbg_emit(
                    LASAPI_DBG_BLUEZ,
                    LASAPI_DBG_INFO,
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
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
                    0,
                    0,
                    "HW_TOPOLOGY",
                    "alsa",
                    &format!("fingerprint=0x{hw_fp:x}"),
                );
            }
            pending_usb_cap_settle = engine.sync_hotplug_topology(force_virtual, bt_now);
            prev_hw_fp = engine.bt().hw_fingerprint();
            if stranded_virtual {
                last_virtual_failover_fp = Some(prev_hw_fp);
            }
            prev_bt_has = bt_now;
            prev_bt_avrcp = avrcp_now;
            prev_muted = muted_now;
            if !pending_usb_cap_settle {
                if let Some(pa) = engine.pa() {
                    pa.notify_change();
                }
            }
        } else if avrcp_now != prev_bt_avrcp || muted_now != prev_muted {
            dbg_emit(
                LASAPI_DBG_BLUEZ,
                LASAPI_DBG_INFO,
                0,
                0,
                "BT_AVRCP_STATE",
                if bt_alias.is_empty() { "bluetooth" } else { &bt_alias },
                &format!("avrcp={avrcp_now} muted={}", i32::from(muted_now)),
            );
            prev_bt_avrcp = avrcp_now;
            prev_muted = muted_now;
            if let Some(pa) = engine.pa() {
                pa.notify_volume(false);
            }
            engine.mark_state_dirty();
        }

        if let Some((hid_delta, hid_mute)) = engine.hw_analog.poll_hid() {
            if hid_delta != 0 || hid_mute != 0 {
                let bt_cur = engine.bt().snapshot();
                let new_v = ((bt_cur.desktop_volume_pct as i32) + hid_delta * 2).clamp(0, 150);
                let mute_op = if hid_mute != 0 {
                    i32::from(!bt_cur.desktop_muted)
                } else {
                    -1
                };
                engine.bt().set_volume(
                    if hid_delta != 0 { new_v } else { -1 },
                    mute_op,
                );
                dbg_emit_vals(
                    LASAPI_DBG_ENGINE,
                    LASAPI_DBG_INFO,
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
                if let Some(pa) = engine.pa() {
                    pa.notify_volume(false);
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
    debug_stop();

    if let Some(pw) = engine.pw() {
        pw.stop();
    }
    if let Some(pa) = engine.pa() {
        pa.stop();
    }

    {
        let mut ss = engine.sessions_lock.lock().unwrap();
        for idx in 0..LASAPI_MAX_SESSIONS {
            destroy_session_slot(&mut ss, idx);
        }
    }
    {
        let mut mh = engine.midi_hub.lock().unwrap();
        mh.close();
    }
    if engine.bt().snapshot().initialized {
        engine.bt().close();
    }
    {
        let mut hws = engine.hw_lock.lock().unwrap();
        if hws.hw.fd >= 0 {
            engine.alsa_hw.close_pcm(hws.hw.fd);
            hws.hw.fd = -1;
        }
    }
    drop(listener);
    let _ = fs::remove_file(&sock_path);
    0
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
fn midi_is_key_press(ev: &LasapiMidiEvent) -> bool {
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

pub fn device_list_has_type(dl: &LasapiDeviceList, dev_id: &str, want_capture: u32) -> bool {
    if dev_id.is_empty() {
        return false;
    }
    let want_any_bt = dev_id == "bluetooth";
    let n = (dl.num_devices as usize).min(LASAPI_MAX_DEVICES);
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
    dl: &LasapiDeviceList,
    want_capture: u32,
    usb_only: bool,
) -> bool {
    let n = (dl.num_devices as usize).min(LASAPI_MAX_DEVICES);
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

fn is_usb_device_entry(dev: &crate::c_abi::LasapiDeviceEntry) -> bool {
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
    dlist: &LasapiDeviceList,
    bt_alias: &str,
    extra_bt_aliases: &[String],
) -> Option<usize> {
    let n = (dlist.num_devices as usize).min(LASAPI_MAX_DEVICES);
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

fn any_usb_matches_bt(dlist: &LasapiDeviceList, bt: &dyn BluetoothBackend) -> bool {
    let alias = c_str_to_string(&bt.snapshot().device_alias);
    let extra = bt.known_audio_aliases();
    find_matching_usb_pb_idx_for_list(dlist, &alias, &extra).is_some()
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
        let mut dl = LasapiDeviceList::zeroed();
        dl.num_devices = 2;
        copy_str_to_c(&mut dl.devices[0].id, "hw:0,0");
        dl.devices[0].is_capture = 1;
        copy_str_to_c(&mut dl.devices[1].id, "bluez:AA:BB:CC:DD:EE:FF");
        dl.devices[1].is_bluetooth = 1;

        assert!(device_list_has_type(&dl, "hw:0,0", 1));
        assert!(!device_list_has_type(&dl, "hw:0,0", 0));
        assert!(device_list_has_type(&dl, "bluetooth", 0));

        let note_on = LasapiMidiEvent {
            frame_offset: 0,
            size: 3,
            port: 0,
            data: [0x90, 60, 100, 0],
            timestamp_ns: 0,
        };
        let note_off_vel0 = LasapiMidiEvent {
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

    fn make_pb_list(devs: &[(&str, &str, bool)]) -> LasapiDeviceList {
        let mut dl = LasapiDeviceList::zeroed();
        for &(id, name, is_bt) in devs {
            let i = dl.num_devices as usize;
            dl.num_devices += 1;
            copy_str_to_c(&mut dl.devices[i].id, id);
            copy_str_to_c(&mut dl.devices[i].name, name);
            dl.devices[i].is_bluetooth = u32::from(is_bt);
        }
        dl
    }

    #[test]
    fn hotplug_auto_switches_to_plugged_usb_or_bt_headphones_and_restores_on_unplug() {
        use crate::hw_analog::is_headphone_card_name;
        let jacks_none = [false; 32];
        let mut tracker = HotplugTracker::default();
        let scar_only = make_pb_list(&[
            ("hw:0,0", "Focusrite Scarlett 2i2 4th Gen - USB Audio", false),
            ("hw:1,8", "HDA ATI HDMI - Q32G11ZNE", false),
        ]);
        let is_usb = |id: &str| id == "hw:0,0" || id == "hw:6,0";
        let is_hp = |id: &str, nm: &str| id == "hw:6,0" || is_headphone_card_name(nm, "", "");

        // Seed baseline on Scarlett (pinned hw:0,0) -> no switch
        let act0 = evaluate_playback_hotplug_with(
            false,
            "hw:0,0",
            "hw:0,0",
            false,
            false,
            false,
            false,
            &scar_only,
            &jacks_none,
            &mut tracker,
            is_usb,
            is_hp,
        );
        assert_eq!(act0, PlaybackHotplugAction::None);

        // Plug in Bose QC Ultra 2 HP (USB) as hw:6,0 while hw:0,0 (also USB) is active -> auto-switches!
        let with_bose = make_pb_list(&[
            ("hw:0,0", "Focusrite Scarlett 2i2 4th Gen - USB Audio", false),
            ("hw:1,8", "HDA ATI HDMI - Q32G11ZNE", false),
            ("hw:6,0", "Bose Corporation Bose QC Ultra 2 HP - USB Audio", false),
        ]);
        let act1 = evaluate_playback_hotplug_with(
            false,
            "hw:0,0",
            "hw:0,0",
            false,
            false,
            false,
            false,
            &with_bose,
            &jacks_none,
            &mut tracker,
            is_usb,
            is_hp,
        );
        assert_eq!(
            act1,
            PlaybackHotplugAction::Switch {
                apply_req: "hw:6,0".to_string(),
                new_target_pb: Some("hw:6,0".to_string()),
                reason: "hotplug_headphone_or_usb",
            }
        );

        // Unplug hw:6,0 (dropped to virtual) -> seamlessly restores saved hw:0,0 instead of staying virtual!
        let act2 = evaluate_playback_hotplug_with(
            false,
            "hw:6,0",
            "virtual",
            true,
            false,
            false,
            false,
            &scar_only,
            &jacks_none,
            &mut tracker,
            is_usb,
            is_hp,
        );
        assert_eq!(
            act2,
            PlaybackHotplugAction::Switch {
                apply_req: "hw:0,0".to_string(),
                new_target_pb: Some("hw:0,0".to_string()),
                reason: "unplug_restore_prev_target",
            }
        );

        // Connect Bluetooth headphones while on Scarlett -> auto-switches to Bluetooth
        let with_bt = make_pb_list(&[
            ("hw:0,0", "Focusrite Scarlett 2i2 4th Gen - USB Audio", false),
            ("bluez:68:F2:1F:04:F9:F6", "Bose QC Ultra 2 HP (AAC)", true),
        ]);
        let act3 = evaluate_playback_hotplug_with(
            false,
            "hw:0,0",
            "hw:0,0",
            false,
            false,
            false,
            false,
            &with_bt,
            &jacks_none,
            &mut tracker,
            is_usb,
            is_hp,
        );
        assert_eq!(
            act3,
            PlaybackHotplugAction::Switch {
                apply_req: "bluez:68:F2:1F:04:F9:F6".to_string(),
                new_target_pb: Some("bluez:68:F2:1F:04:F9:F6".to_string()),
                reason: "hotplug_headphone_or_usb",
            }
        );

        // Plugging in an HDMI monitor wakeup must NOT hijack playback away from Scarlett
        let with_new_hdmi = make_pb_list(&[
            ("hw:0,0", "Focusrite Scarlett 2i2 4th Gen - USB Audio", false),
            ("bluez:68:F2:1F:04:F9:F6", "Bose QC Ultra 2 HP (AAC)", true),
            ("hw:1,9", "HDA ATI HDMI - Beyond TV", false),
        ]);
        let act4 = evaluate_playback_hotplug_with(
            false,
            "hw:0,0",
            "hw:0,0",
            false,
            false,
            false,
            false,
            &with_new_hdmi,
            &jacks_none,
            &mut tracker,
            is_usb,
            is_hp,
        );
        assert_eq!(act4, PlaybackHotplugAction::None);

        // Plugging 3.5mm analog headphone jack into card 4 switches playback automatically
        let mut tracker_jack = HotplugTracker::default();
        let with_hda = make_pb_list(&[
            ("hw:0,0", "Focusrite Scarlett 2i2 4th Gen - USB Audio", false),
            ("hw:4,0", "Generic USB Audio", false),
        ]);
        let _ = evaluate_playback_hotplug_with(
            false,
            "auto",
            "hw:0,0",
            false,
            false,
            false,
            false,
            &with_hda,
            &jacks_none,
            &mut tracker_jack,
            |id| id == "hw:0,0" || id == "hw:4,0",
            |_, nm| is_headphone_card_name(nm, "", ""),
        );
        let mut jacks_c4 = [false; 32];
        jacks_c4[4] = true;
        let act5 = evaluate_playback_hotplug_with(
            false,
            "auto",
            "hw:0,0",
            false,
            false,
            false,
            false,
            &with_hda,
            &jacks_c4,
            &mut tracker_jack,
            |id| id == "hw:0,0" || id == "hw:4,0",
            |_, nm| is_headphone_card_name(nm, "", ""),
        );
        assert_eq!(
            act5,
            PlaybackHotplugAction::Switch {
                apply_req: "hw:4,0".to_string(),
                new_target_pb: None,
                reason: "hotplug_headphone_or_usb",
            }
        );

        // Pinned hw:6,0 unplugged with no restore target -> fails over cleanly to auto instead of silent virtual
        let mut tracker_pin = HotplugTracker::default();
        let _ = evaluate_playback_hotplug_with(
            false,
            "hw:6,0",
            "hw:6,0",
            false,
            false,
            false,
            false,
            &with_bose,
            &jacks_none,
            &mut tracker_pin,
            is_usb,
            is_hp,
        );
        let act6 = evaluate_playback_hotplug_with(
            false,
            "hw:6,0",
            "virtual",
            true,
            false,
            false,
            false,
            &scar_only,
            &jacks_none,
            &mut tracker_pin,
            is_usb,
            is_hp,
        );
        assert_eq!(
            act6,
            PlaybackHotplugAction::Switch {
                apply_req: "auto".to_string(),
                new_target_pb: None,
                reason: "recover_from_virtual",
            }
        );
    }

    #[test]
    fn control_ipc_unix_pair_dispatches_into_engine() {
        use std::os::unix::net::UnixStream;

        let rt = default_runtime_state();
        let eng = Arc::new(AudiodgEngine::new(
            &rt,
            Arc::new(crate::bluez_runtime::NullBluetoothBackend),
        ));
        let (mut cli, mut srv) = UnixStream::pair().unwrap();

        let eng_t = Arc::clone(&eng);
        let h = std::thread::spawn(move || {
            for _ in 0..3 {
                let mut hdr_buf = [0u8; mem::size_of::<LasapiIpcReq>()];
                srv.read_exact(&mut hdr_buf).unwrap();
                let req = crate::c_abi::PodBytes::from_bytes(&hdr_buf).unwrap();
                let s = srv.try_clone().unwrap();
                eng_t.handle_client_cmd(s, &req);
            }
        });

        let st_resp = crate::control_ipc::ControlClient::send_over_stream(
            &mut cli,
            &ControlRequest::GetStatus,
        )
        .unwrap();
        assert_eq!(st_resp.hr, S_OK);
        assert!(matches!(
            st_resp.payload,
            ControlPayload::Status { .. }
        ));

        let eq = crate::settings::default_eq_cfg();
        let eq_resp = crate::control_ipc::ControlClient::send_over_stream(
            &mut cli,
            &ControlRequest::SetDynamicEq(Box::new(eq)),
        )
        .unwrap();
        assert_eq!(eq_resp.hr, S_OK);

        let vol_resp = crate::control_ipc::ControlClient::send_over_stream(
            &mut cli,
            &ControlRequest::SetMasterVolume {
                dir: AudioDir::Playback,
                volume_pct: Some(77),
                mute: MuteOp::Unmute,
                client_tag: "test".into(),
            },
        )
        .unwrap();
        assert_eq!(vol_resp.hr, S_OK);

        h.join().unwrap();
    }

    #[test]
    fn engine_alsa_backend_seam_handles_writes_xrun_and_device_loss() {
        use crate::alsa_hw::InMemoryAlsaBackend;
        use crate::unix_sys::{ERRNO_ENODEV, ERRNO_EPIPE};

        let mut rt = default_runtime_state();
        copy_str_to_c(&mut rt.target_playback_id, "hw:2,1");
        let alsa = Arc::new(InMemoryAlsaBackend::new());
        let eng = AudiodgEngine::with_backends(
            &rt,
            Arc::new(crate::bluez_runtime::NullBluetoothBackend),
            Arc::clone(&alsa) as Arc<dyn AlsaPlaybackBackend>,
        );

        let opened = alsa.open_pcm("/dev/snd/pcmC2D1p", 48_000, 256, 32).unwrap();
        let opened_fd = opened.fd;
        {
            let mut hws = eng.hw_lock.lock().unwrap();
            hws.hw = opened;
            copy_str_to_c(&mut hws.active_playback_id, "hw:2,1");
        }
        assert_eq!(alsa.opened_paths(), vec!["/dev/snd/pcmC2D1p".to_string()]);

        let mix = vec![0.25f32; 256 * 2];
        let mut scratch = vec![0u8; 256 * 2 * 4];

        let (clear1, wake1) = eng.write_alsa_period_and_handle_fault(
            &opened, &mix, 256, 2, 1.0, false, &mut scratch,
        );
        assert_eq!((clear1, wake1), (true, true));
        assert_eq!(alsa.written_packet_count(), 1);
        assert_eq!(eng.hw_xruns.load(Ordering::Relaxed), 0);

        alsa.inject_fault(AlsaWriteOutcome::RecoveredXrun { xerr: ERRNO_EPIPE });
        let (clear2, wake2) = eng.write_alsa_period_and_handle_fault(
            &opened, &mix, 256, 2, 1.0, false, &mut scratch,
        );
        assert_eq!((clear2, wake2), (false, true));
        assert_eq!(eng.hw_xruns.load(Ordering::Relaxed), 1);

        alsa.inject_fault(AlsaWriteOutcome::DeviceLost {
            dead_err: ERRNO_ENODEV,
            xerr: None,
        });
        let (clear3, wake3) = eng.write_alsa_period_and_handle_fault(
            &opened, &mix, 256, 2, 1.0, false, &mut scratch,
        );
        assert_eq!((clear3, wake3), (false, false));
        assert_eq!(alsa.closed_fds(), vec![opened_fd]);

        let hws = eng.hw_lock.lock().unwrap();
        assert!(hws.hw.is_virtual);
        assert_eq!(hws.hw.fd, -1);
        assert_eq!(c_str_to_string(&hws.active_playback_id), "virtual");
    }
}
