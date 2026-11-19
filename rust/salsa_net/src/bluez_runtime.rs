// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_void};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU32, AtomicU64, Ordering};
use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audiodg_engine::{
    ERRNO_EAGAIN, ERRNO_EBADF, ERRNO_ECONNRESET, ERRNO_EINPROGRESS, ERRNO_EINTR, ERRNO_EPIPE,
};
use crate::bluez_cap::{
    alsa_hw_fingerprint_seed, append_alsa_devices, fnv1a_step, open_alsa_capture,
    parse_asound_cards, read_capture_native, stop_capture_thread, CapState, MAX_ALSA_CARDS_SCAN,
};
use crate::bluez_ffi::{
    add_dict_entry_bool, add_dict_entry_byte, add_dict_entry_bytes, add_dict_entry_obj_path,
    add_dict_entry_str, add_dict_entry_u16, cptr_to_opt_str, dbus_error_msg, dbus_get_bool_prop,
    dbus_get_str_prop, dbus_get_u16_prop, dbus_new_method_call, dbus_set_avrcp_volume,
    dbus_set_str_prop, iter_append_str, iter_get_bool, iter_get_byte, iter_get_byte_array,
    iter_get_str, iter_get_u16, iter_get_unix_fd, load_aptx_library, load_dbus_library,
    load_fdk_aac_library, load_ldac_abr_library, load_ldac_library, load_sbc_library, BdAddr,
    BtVoice, DBusApi, DBusConnection, DBusError, DBusHandlerResult, DBusMessage, DBusMessageIter,
    DBusObjectPathVTable, DBusPendingCall, FdkBufDesc, FdkInArgs, FdkInfo, FdkOutArgs, SbcStruct,
    SockaddrSco, AF_BLUETOOTH, BTPROTO_SCO, BT_VOICE, BT_VOICE_CVSD_16BIT, BT_VOICE_TRANSPARENT,
    DBUS_BUS_SYSTEM, DBUS_HANDLER_RESULT_HANDLED, DBUS_HANDLER_RESULT_NOT_YET_HANDLED,
    DBUS_MESSAGE_TYPE_ERROR, DBUS_MESSAGE_TYPE_SIGNAL, DBUS_TYPE_ARRAY, DBUS_TYPE_BYTE,
    DBUS_TYPE_DICT_ENTRY, DBUS_TYPE_OBJECT_PATH, DBUS_TYPE_STRING, FDK_AACENC_OK, FDK_AOT_AAC_LC,
    FDK_AOT_MP2_AAC_LC, FDK_IN_AUDIO_DATA, FDK_OUT_BITSTREAM_DATA, FDK_PARAM_AFTERBURNER,
    FDK_PARAM_AOT, FDK_PARAM_AUDIOMUXVER, FDK_PARAM_BANDWIDTH, FDK_PARAM_BITRATE,
    FDK_PARAM_BITRATEMODE, FDK_PARAM_CHANNELMODE, FDK_PARAM_HEADER_PERIOD, FDK_PARAM_PEAK_BITRATE,
    FDK_PARAM_SAMPLERATE, FDK_PARAM_SIGNALING_MODE, FDK_PARAM_TRANSMUX, FDK_TT_MP4_LATM_MCP1,
    LDACBT_ENC_LSU, LDACBT_EQMID_HQ, LDACBT_EQMID_MQ, LDACBT_EQMID_SQ, LDACBT_SMPL_FMT_S16,
    SBC_LE, SOL_BLUETOOTH,
};
use crate::bluez_media::{
    bt_tx_margin_frames, pop_bt_resampled_s16, sco_push_pcm_to_cap_ring, write_tx_period,
    BT_TX_RING_FRAMES,
};
use crate::bluez_wire::{
    abr_step_aac, abr_step_ldac, extract_mac_from_id, hfp_make_batt_path, ldac_eqmid_to_kbps,
    msbc_h2_byte, msbc_rx_strip_offset, pack_s16_to_s24le, parse_bdaddr, parse_hfp_at_line,
    select_a2dp_config, write_rtp_header, write_rtp_sbc_payload_hdr, BT_WIRE_AAC, BT_WIRE_APTX,
    BT_WIRE_APTX_HD, BT_WIRE_LDAC, BT_WIRE_SBC,
};
use crate::c_abi::{
    copy_str_to_c, LwasapiBtCodecCfg, LwasapiDeviceEntry, LwasapiDeviceList, SalsaBtSnapshot,
    LWASAPI_BT_CAP_AAC, LWASAPI_BT_CAP_APTX, LWASAPI_BT_CAP_APTX_HD, LWASAPI_BT_CAP_LDAC,
    LWASAPI_BT_CAP_SBC, LWASAPI_BT_PREF_AAC, LWASAPI_BT_PREF_APTX, LWASAPI_BT_PREF_APTX_HD,
    LWASAPI_BT_PREF_AUTO, LWASAPI_BT_PREF_LDAC, LWASAPI_BT_PREF_SBC, LWASAPI_DBG_BLUEZ,
    LWASAPI_DBG_ERR, LWASAPI_DBG_INFO, LWASAPI_DBG_TRACE, LWASAPI_DBG_WARN,
    LWASAPI_LDAC_MODE_330, LWASAPI_LDAC_MODE_660, LWASAPI_LDAC_MODE_990,
    LWASAPI_LDAC_MODE_ADAPTIVE, LWASAPI_MAX_DEVICES,
};
use crate::failsafe::{record_trip, FS_BT_FALLBACK};
use crate::unix_sys::{
    bind_raw, close_raw_fd, connect_raw, dbg_emit, dbg_emit_vals, do_poll, enable_realtime,
    getsockopt_raw, last_errno, mono_ns, recv_raw, send_raw, setsockopt_raw, shutdown_rdwr,
    sleep_abs_mono_ns, socket_raw, PollFd, MSG_DONTWAIT, MSG_NOSIGNAL, POLLERR, POLLHUP, POLLIN,
    POLLNVAL, POLLOUT, SOCK_CLOEXEC, SOCK_NONBLOCK, SOCK_SEQPACKET, SOL_SOCKET, SO_ERROR,
    SO_PRIORITY, SO_SNDBUF,
};

const A2DP_SOURCE_UUID: &str = "0000110a-0000-1000-8000-00805f9b34fb";
const A2DP_SINK_UUID: &str = "0000110b-0000-1000-8000-00805f9b34fb";
const HFP_HF_UUID: &str = "0000111e-0000-1000-8000-00805f9b34fb";
const HFP_AG_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";

const A2DP_CODEC_SBC: u8 = 0x00;
const A2DP_CODEC_MPEG24: u8 = 0x02;
const A2DP_CODEC_VENDOR: u8 = 0xFF;

const LWASAPI_BT_CODEC_SBC: u8 = 0;
const LWASAPI_BT_CODEC_AAC: u8 = 1;
const LWASAPI_BT_CODEC_APTX: u8 = 2;
const LWASAPI_BT_CODEC_APTX_HD: u8 = 3;
const LWASAPI_BT_CODEC_LDAC: u8 = 4;

const LWASAPI_EP_LDAC: &str = "/org/lwasapi/A2DP/LDAC/Source";
const LWASAPI_EP_APTXHD: &str = "/org/lwasapi/A2DP/APTXHD/Source";
const LWASAPI_EP_APTX: &str = "/org/lwasapi/A2DP/APTX/Source";
const LWASAPI_EP_AAC: &str = "/org/lwasapi/A2DP/AAC/Source";
const LWASAPI_EP_SBC: &str = "/org/lwasapi/A2DP/SBC/Source";
const LWASAPI_HFP_AG_PATH: &str = "/org/lwasapi/HFP/AG";
const LWASAPI_BATTERY_ROOT: &str = "/org/lwasapi/Battery";

const BT_RTP_HDR_BYTES: usize = 12;
const BT_RTP_SBC_PL_BYTES: usize = 1;

const APTX_VENDOR_ID: u32 = 0x0000_004F;
const APTX_CODEC_ID: u16 = 0x0001;
const APTX_HD_VENDOR_ID: u32 = 0x0000_00D7;
const APTX_HD_CODEC_ID: u16 = 0x0024;
const LDAC_VENDOR_ID: u32 = 0x0000_012D;
const LDAC_CODEC_ID: u16 = 0x00AA;

const SBC_SAMPLING_FREQ_16000: u8 = 1 << 3;
const SBC_SAMPLING_FREQ_32000: u8 = 1 << 2;
const SBC_SAMPLING_FREQ_44100: u8 = 1 << 1;
const SBC_SAMPLING_FREQ_48000: u8 = 1 << 0;
const SBC_CHANNEL_MODE_MONO: u8 = 1 << 3;
const SBC_CHANNEL_MODE_JOINT_STEREO: u8 = 1 << 0;
const SBC_BLOCK_LENGTH_16: u8 = 1 << 0;
const SBC_SUBBANDS_8: u8 = 1 << 0;
const SBC_ALLOCATION_LOUDNESS: u8 = 1 << 0;

const AAC_OBJECT_TYPE_MPEG2_AAC_LC: u8 = 0x80;
const AAC_OBJECT_TYPE_MPEG4_AAC_LC: u8 = 0x40;
const AAC_SAMPLING_FREQ_48000: u16 = 0x0008;
const AAC_CHANNELS_2: u8 = 0x04;

const APTX_SAMPLING_FREQ_16000: u8 = 0x08;
const APTX_SAMPLING_FREQ_32000: u8 = 0x04;
const APTX_SAMPLING_FREQ_44100: u8 = 0x02;
const APTX_SAMPLING_FREQ_48000: u8 = 0x01;
const APTX_CHANNEL_MODE_MONO: u8 = 0x01;
const APTX_CHANNEL_MODE_STEREO: u8 = 0x02;

const LDAC_SAMPLING_FREQ_44100: u8 = 0x20;
const LDAC_SAMPLING_FREQ_48000: u8 = 0x10;
const LDAC_SAMPLING_FREQ_88200: u8 = 0x08;
const LDAC_SAMPLING_FREQ_96000: u8 = 0x04;
const LDAC_CHANNEL_MODE_MONO: u8 = 0x04;
const LDAC_CHANNEL_MODE_DUAL: u8 = 0x02;
const LDAC_CHANNEL_MODE_STEREO: u8 = 0x01;

const MAX_HFP_DEVS: usize = 8;

#[derive(Default)]
struct BtEncoders {
    sbc_enc: Option<Box<SbcStruct>>,
    fdk_enc: *mut c_void,
    fdk_frame_length: u32,
    ldac_enc: *mut c_void,
    ldac_eqmid: i32,
    ldac_abr: *mut c_void,
    aptx_enc: *mut c_void,
}

unsafe impl Send for BtEncoders {}

impl BtEncoders {
    fn close_all(&mut self) {
        if !self.ldac_abr.is_null() {
            if let Some(abr) = load_ldac_abr_library() {
                unsafe { (abr.free_handle)(self.ldac_abr) };
            }
            self.ldac_abr = ptr::null_mut();
        }
        if !self.fdk_enc.is_null() {
            if let Some(fdk) = load_fdk_aac_library() {
                unsafe {
                    let _ = (fdk.close)(&mut self.fdk_enc);
                }
            }
            self.fdk_enc = ptr::null_mut();
        }
        if !self.ldac_enc.is_null() {
            if let Some(ldac) = load_ldac_library() {
                unsafe { (ldac.free_handle)(self.ldac_enc) };
            }
            self.ldac_enc = ptr::null_mut();
        }
        if !self.aptx_enc.is_null() {
            if let Some(aptx) = load_aptx_library() {
                unsafe { (aptx.finish)(self.aptx_enc) };
            }
            self.aptx_enc = ptr::null_mut();
        }
        if let Some(mut sbc) = self.sbc_enc.take() {
            if let Some(api) = load_sbc_library() {
                unsafe { (api.finish)(&mut *sbc) };
            }
        }
    }
}

impl Drop for BtEncoders {
    fn drop(&mut self) {
        self.close_all();
    }
}

struct BtMutable {
    has_bluez_transport: bool,
    direct_socket_mode: bool,
    endpoints_registered: bool,
    tx_stream_active: bool,

    adapter_path: String,
    device_path: String,
    endpoint_path: String,
    transport_path: String,
    device_alias: String,
    mac_address: String,
    codec_name: String,
    preferred_mac: String,
    neg_reason: String,
    bt_sco_codec: String,

    codec_id: u8,
    active_codec_kind: u8,
    a2dp_config: [u8; 16],
    a2dp_config_len: usize,
    bt_sample_rate: u32,
    bt_channels: u32,
    read_mtu: u16,
    write_mtu: u16,
    bt_fd: c_int,
    bt_fd_owner: Option<Arc<OwnedFd>>,

    rtp_seq: u16,
    rtp_timestamp: u32,

    pref_codec: u32,
    ldac_mode: u32,
    wireless_abr_enabled: u32,
    effective_kbps: u32,
    supported_codecs_mask: u32,
    aac_target_bps: u32,
    aac_max_bps: u32,
    abr_last_ms: u64,
    tx_congest_streak: u32,
    tx_clean_streak: u32,
    hfp_sco_enabled: u32,

    enc: BtEncoders,

    // D-Bus state synchronized with bt_dbus_thread_fn
    bt_kick_dev: String,
    bt_startup_autoconnect_dev: String,
    renegotiate_start_ns: u64,
}

impl Default for BtMutable {
    fn default() -> Self {
        Self {
            has_bluez_transport: false,
            direct_socket_mode: false,
            endpoints_registered: false,
            tx_stream_active: false,
            adapter_path: String::new(),
            device_path: String::new(),
            endpoint_path: String::new(),
            transport_path: String::new(),
            device_alias: String::new(),
            mac_address: String::new(),
            codec_name: String::new(),
            preferred_mac: String::new(),
            neg_reason: "Waiting for A2DP endpoint negotiation".to_string(),
            bt_sco_codec: String::new(),
            codec_id: 0,
            active_codec_kind: 0,
            a2dp_config: [0; 16],
            a2dp_config_len: 0,
            bt_sample_rate: 48_000,
            bt_channels: 2,
            read_mtu: 0,
            write_mtu: 0,
            bt_fd: -1,
            bt_fd_owner: None,
            rtp_seq: 0,
            rtp_timestamp: 0,
            pref_codec: LWASAPI_BT_PREF_AUTO,
            ldac_mode: LWASAPI_LDAC_MODE_ADAPTIVE,
            wireless_abr_enabled: 1,
            effective_kbps: 0,
            supported_codecs_mask: 0,
            aac_target_bps: 0,
            aac_max_bps: 0,
            abr_last_ms: 0,
            tx_congest_streak: 0,
            tx_clean_streak: 0,
            hfp_sco_enabled: 0,
            enc: BtEncoders::default(),
            bt_kick_dev: String::new(),
            bt_startup_autoconnect_dev: String::new(),
            renegotiate_start_ns: 0,
        }
    }
}

#[derive(Clone, Default)]
struct HfpBatteryDev {
    in_use: bool,
    rfcomm_active: bool,
    published: bool,
    pending_add: bool,
    pending_chg: bool,
    pending_rem: bool,
    supports_msbc: bool,
    _negotiated_sco_codec: u8,
    fd: c_int,
    fd_owner: Option<Arc<OwnedFd>>,
    pct: i32,
    dev_path: String,
    batt_path: String,
}

impl BtMutable {
    fn replace_bt_fd(&mut self, fd: c_int) {
        self.close_bt_fd();
        if fd >= 0 {
            self.bt_fd = fd;
            self.bt_fd_owner = Some(Arc::new(unsafe { OwnedFd::from_raw_fd(fd) }));
        }
    }

    fn close_bt_fd(&mut self) {
        if self.bt_fd >= 0 {
            shutdown_rdwr(self.bt_fd);
            self.bt_fd_owner = None;
            self.bt_fd = -1;
        }
    }
}

struct DbusConnPtr(*mut DBusConnection);
unsafe impl Send for DbusConnPtr {}

struct HeldDbusConn {
    conn: *mut DBusConnection,
    unref: unsafe extern "C" fn(*mut DBusConnection),
}

impl Drop for HeldDbusConn {
    fn drop(&mut self) {
        if !self.conn.is_null() {
            unsafe { (self.unref)(self.conn) };
        }
    }
}

pub struct BluezRuntime {
    pub initialized: AtomicBool,
    pub engine_sample_rate: AtomicU32,
    pub desktop_volume_pct: AtomicU32,
    pub desktop_muted: AtomicBool,
    pub avrcp_volume: AtomicU16,
    pub avrcp_supported: AtomicBool,
    pub delay_tenths_ms: AtomicU16,

    pub tx_transport_ready: AtomicBool,
    pub tx_paused: AtomicBool,
    pub tx_write_frame: AtomicU64,
    pub tx_read_frame: AtomicU64,
    pub tx_packets_sent: AtomicU64,
    pub tx_underruns: AtomicU64,
    pub tx_errors: AtomicU64,
    pub tx_drop_event_ns: AtomicU64,
    pub tx_ring: Mutex<Box<[f32]>>,

    pub dbus_thread_running: AtomicBool,
    pub tx_thread_running: AtomicBool,
    pub sco_thread_running: AtomicBool,
    pub sco_fd: AtomicI32,
    pub sco_sample_rate: AtomicU32,
    pub hfp_sco_active: AtomicU32,

    pub need_acquire: AtomicBool,
    pub need_reregister: AtomicBool,
    pub need_avrcp_sync: AtomicBool,
    pub pending_avrcp_vol: AtomicI32,
    pub bt_renegotiate_req: AtomicBool,
    pub bt_renegotiate_fallback: AtomicBool,
    pub recovery_active: AtomicBool,
    pub last_recover_ns: AtomicU64,
    pub hfp_battery_dirty: AtomicBool,

    dbus_conn: Mutex<DbusConnPtr>,
    state: Mutex<BtMutable>,
    cap: Arc<CapState>,
    hfp_devs: Mutex<[HfpBatteryDev; MAX_HFP_DEVS]>,
    dbus_thread: Mutex<Option<JoinHandle<()>>>,
    tx_thread: Mutex<Option<JoinHandle<()>>>,
    sco_thread: Mutex<Option<JoinHandle<()>>>,
}

impl BluezRuntime {
    fn new() -> Self {
        Self {
            initialized: AtomicBool::new(false),
            engine_sample_rate: AtomicU32::new(48_000),
            desktop_volume_pct: AtomicU32::new(100),
            desktop_muted: AtomicBool::new(false),
            avrcp_volume: AtomicU16::new(127),
            avrcp_supported: AtomicBool::new(false),
            delay_tenths_ms: AtomicU16::new(0),
            tx_transport_ready: AtomicBool::new(false),
            tx_paused: AtomicBool::new(false),
            tx_write_frame: AtomicU64::new(0),
            tx_read_frame: AtomicU64::new(0),
            tx_packets_sent: AtomicU64::new(0),
            tx_underruns: AtomicU64::new(0),
            tx_errors: AtomicU64::new(0),
            tx_drop_event_ns: AtomicU64::new(0),
            tx_ring: Mutex::new(vec![0.0f32; BT_TX_RING_FRAMES * 2].into_boxed_slice()),
            dbus_thread_running: AtomicBool::new(false),
            tx_thread_running: AtomicBool::new(false),
            sco_thread_running: AtomicBool::new(false),
            sco_fd: AtomicI32::new(-1),
            sco_sample_rate: AtomicU32::new(0),
            hfp_sco_active: AtomicU32::new(0),
            need_acquire: AtomicBool::new(false),
            need_reregister: AtomicBool::new(false),
            need_avrcp_sync: AtomicBool::new(false),
            pending_avrcp_vol: AtomicI32::new(-1),
            bt_renegotiate_req: AtomicBool::new(false),
            bt_renegotiate_fallback: AtomicBool::new(false),
            recovery_active: AtomicBool::new(false),
            last_recover_ns: AtomicU64::new(0),
            hfp_battery_dirty: AtomicBool::new(false),
            dbus_conn: Mutex::new(DbusConnPtr(ptr::null_mut())),
            state: Mutex::new(BtMutable::default()),
            cap: Arc::new(CapState::new()),
            hfp_devs: Mutex::new(std::array::from_fn(|_| HfpBatteryDev::default())),
            dbus_thread: Mutex::new(None),
            tx_thread: Mutex::new(None),
            sco_thread: Mutex::new(None),
        }
    }
}

static GLOBAL_BT: OnceLock<Arc<BluezRuntime>> = OnceLock::new();
static DBUS_ENUM_LOCK: Mutex<()> = Mutex::new(());

fn global_bt_arc() -> &'static Arc<BluezRuntime> {
    GLOBAL_BT.get_or_init(|| Arc::new(BluezRuntime::new()))
}

unsafe fn bt_from_ptr<'a>(bt: *const c_void) -> Option<&'a BluezRuntime> {
    if bt.is_null() {
        None
    } else {
        Some(unsafe { &*bt.cast::<BluezRuntime>() })
    }
}

// ============================================================================
// Encoder Initialization
// ============================================================================

fn init_bt_encoder_for_transport_locked(st: &mut BtMutable) -> i32 {
    st.enc.close_all();
    st.tx_congest_streak = 0;
    st.tx_clean_streak = 0;
    st.abr_last_ms = 0;

    if st.active_codec_kind == LWASAPI_BT_CODEC_LDAC {
        let Some(ldac) = load_ldac_library() else {
            return -1;
        };
        if st.a2dp_config_len < 8 {
            return -1;
        }
        let freq_b = st.a2dp_config[6];
        let cm_b = st.a2dp_config[7];
        let sf = if (freq_b & LDAC_SAMPLING_FREQ_96000) != 0 {
            96_000
        } else if (freq_b & LDAC_SAMPLING_FREQ_88200) != 0 {
            88_200
        } else if (freq_b & LDAC_SAMPLING_FREQ_48000) != 0 {
            48_000
        } else if (freq_b & LDAC_SAMPLING_FREQ_44100) != 0 {
            44_100
        } else {
            48_000
        };
        let (cm, ch) = if (cm_b & LDAC_CHANNEL_MODE_STEREO) != 0 {
            (LDAC_CHANNEL_MODE_STEREO as c_int, 2u32)
        } else if (cm_b & LDAC_CHANNEL_MODE_DUAL) != 0 {
            (LDAC_CHANNEL_MODE_DUAL as c_int, 2u32)
        } else if (cm_b & LDAC_CHANNEL_MODE_MONO) != 0 {
            (LDAC_CHANNEL_MODE_MONO as c_int, 1u32)
        } else {
            (LDAC_CHANNEL_MODE_STEREO as c_int, 2u32)
        };

        let mtu = if st.write_mtu > 0 { st.write_mtu } else { 679 };
        let payload_mtu = if mtu as usize > BT_RTP_HDR_BYTES + BT_RTP_SBC_PL_BYTES + 64 {
            (mtu as usize - BT_RTP_HDR_BYTES - BT_RTP_SBC_PL_BYTES).min(1024) as u16
        } else {
            660
        };
        let eqmid = match st.ldac_mode {
            LWASAPI_LDAC_MODE_990 => LDACBT_EQMID_HQ,
            LWASAPI_LDAC_MODE_660 => LDACBT_EQMID_SQ,
            LWASAPI_LDAC_MODE_330 => LDACBT_EQMID_MQ,
            _ => {
                if payload_mtu >= 660 {
                    LDACBT_EQMID_HQ
                } else {
                    LDACBT_EQMID_SQ
                }
            }
        };

        let lh = unsafe { (ldac.get_handle)() };
        if lh.is_null() {
            return -1;
        }
        if unsafe {
            (ldac.init_encode)(
                lh,
                c_int::from(payload_mtu),
                eqmid,
                cm,
                LDACBT_SMPL_FMT_S16,
                sf as c_int,
            )
        } < 0
        {
            unsafe { (ldac.free_handle)(lh) };
            return -1;
        }
        st.enc.ldac_enc = lh;
        st.enc.ldac_eqmid = eqmid;
        st.bt_sample_rate = sf;
        st.bt_channels = ch;
        st.effective_kbps = ldac_eqmid_to_kbps(sf, eqmid);
        if st.ldac_mode == LWASAPI_LDAC_MODE_ADAPTIVE && st.wireless_abr_enabled != 0 {
            if let Some(abr_api) = load_ldac_abr_library() {
                let abr = unsafe { (abr_api.get_handle)() };
                if !abr.is_null() {
                    unsafe {
                        let _ = (abr_api.init)(abr, 20);
                        let _ = (abr_api.set_thresholds)(abr, 5, 3, 1);
                    }
                    st.enc.ldac_abr = abr;
                }
            }
        }
        st.codec_name = "LDAC".to_string();
        return 0;
    }

    if st.active_codec_kind == LWASAPI_BT_CODEC_APTX_HD
        || st.active_codec_kind == LWASAPI_BT_CODEC_APTX
    {
        let is_hd = st.active_codec_kind == LWASAPI_BT_CODEC_APTX_HD;
        let need_len = if is_hd { 11 } else { 7 };
        let Some(aptx) = load_aptx_library() else {
            return -1;
        };
        if st.a2dp_config_len < need_len {
            return -1;
        }
        let f = (st.a2dp_config[6] >> 4) & 0x0F;
        let sf = if (f & APTX_SAMPLING_FREQ_48000) != 0 {
            48_000
        } else if (f & APTX_SAMPLING_FREQ_44100) != 0 {
            44_100
        } else if (f & APTX_SAMPLING_FREQ_32000) != 0 {
            32_000
        } else if (f & APTX_SAMPLING_FREQ_16000) != 0 {
            16_000
        } else {
            48_000
        };
        let ah = unsafe { (aptx.init)(c_int::from(is_hd)) };
        if ah.is_null() {
            return -1;
        }
        st.enc.aptx_enc = ah;
        st.bt_sample_rate = sf;
        st.bt_channels = 2;
        st.effective_kbps = if is_hd { 576 } else { 352 };
        st.codec_name = if is_hd {
            "aptX HD".to_string()
        } else {
            "aptX".to_string()
        };
        return 0;
    }

    if st.codec_id == A2DP_CODEC_MPEG24 || st.active_codec_kind == LWASAPI_BT_CODEC_AAC {
        let Some(fdk) = load_fdk_aac_library() else {
            return -1;
        };
        let obj = st.a2dp_config[0];
        let freq_mask = (u16::from(st.a2dp_config[1]) << 4) | u16::from((st.a2dp_config[2] >> 4) & 0x0F);
        let ch_bits = st.a2dp_config[2] & 0x0F;
        let vbr = (st.a2dp_config[3] >> 7) & 1;
        let remote_raw = (u32::from(st.a2dp_config[3] & 0x7F) << 16)
            | (u32::from(st.a2dp_config[4]) << 8)
            | u32::from(st.a2dp_config[5]);
        let remote_bps = if remote_raw == 0 { 192_000 } else { remote_raw };

        st.bt_sample_rate = if (freq_mask & AAC_SAMPLING_FREQ_48000) != 0 {
            48_000
        } else {
            44_100
        };
        st.bt_channels = if (ch_bits & AAC_CHANNELS_2) != 0 { 2 } else { 1 };

        let mtu = if st.write_mtu > 0 { st.write_mtu } else { 672 };
        let max_bps = if usize::from(mtu) > BT_RTP_HDR_BYTES {
            (((mtu as u32 - BT_RTP_HDR_BYTES as u32) * 8 * st.bt_sample_rate) / 1024).max(64_000)
        } else {
            192_000
        };
        let target_bps = remote_bps.min(max_bps).clamp(64_000, 256_000);

        let mut enc: *mut c_void = ptr::null_mut();
        if unsafe { (fdk.open)(&mut enc, 0, st.bt_channels) } != FDK_AACENC_OK || enc.is_null() {
            return -1;
        }
        let aot = if (obj & AAC_OBJECT_TYPE_MPEG4_AAC_LC) != 0 {
            FDK_AOT_AAC_LC
        } else if (obj & AAC_OBJECT_TYPE_MPEG2_AAC_LC) != 0 {
            FDK_AOT_MP2_AAC_LC
        } else {
            FDK_AOT_AAC_LC
        };
        unsafe {
            let _ = (fdk.set_param)(enc, FDK_PARAM_AOT, aot);
            let _ = (fdk.set_param)(enc, FDK_PARAM_SAMPLERATE, st.bt_sample_rate);
            let _ = (fdk.set_param)(enc, FDK_PARAM_CHANNELMODE, st.bt_channels);
            if vbr != 0 {
                let _ = (fdk.set_param)(enc, FDK_PARAM_BITRATEMODE, 5);
            }
            let _ = (fdk.set_param)(enc, FDK_PARAM_AUDIOMUXVER, 2);
            let _ = (fdk.set_param)(enc, FDK_PARAM_SIGNALING_MODE, 1);
            let _ = (fdk.set_param)(enc, FDK_PARAM_BITRATE, target_bps);
            let _ = (fdk.set_param)(enc, FDK_PARAM_PEAK_BITRATE, max_bps);
            if vbr == 0 && target_bps > 110_000 {
                let _ = (fdk.set_param)(enc, FDK_PARAM_BANDWIDTH, 19_293);
            }
            let _ = (fdk.set_param)(enc, FDK_PARAM_TRANSMUX, FDK_TT_MP4_LATM_MCP1);
            let _ = (fdk.set_param)(enc, FDK_PARAM_HEADER_PERIOD, 1);
            let _ = (fdk.set_param)(enc, FDK_PARAM_AFTERBURNER, 1);
            if (fdk.encode)(enc, ptr::null(), ptr::null(), ptr::null(), ptr::null_mut())
                != FDK_AACENC_OK
            {
                let _ = (fdk.close)(&mut enc);
                return -1;
            }
            let mut info = FdkInfo::default();
            if (fdk.info)(enc, &mut info) != FDK_AACENC_OK {
                let _ = (fdk.close)(&mut enc);
                return -1;
            }
            st.enc.fdk_enc = enc;
            st.enc.fdk_frame_length = if info.frame_length > 0 {
                info.frame_length
            } else {
                1024
            };
        }
        st.aac_max_bps = target_bps;
        st.aac_target_bps = target_bps;
        st.effective_kbps = target_bps / 1000;
        st.codec_name = "AAC".to_string();
        return 0;
    }

    // Baseline SBC encoder via libsbc.so.1
    let Some(sbc_api) = load_sbc_library() else {
        return -1;
    };
    let mut scfg = if st.a2dp_config_len >= 4 {
        [
            st.a2dp_config[0],
            st.a2dp_config[1],
            st.a2dp_config[2],
            st.a2dp_config[3],
        ]
    } else {
        [
            (SBC_SAMPLING_FREQ_48000 << 4) | SBC_CHANNEL_MODE_JOINT_STEREO,
            (SBC_BLOCK_LENGTH_16 << 4) | (SBC_SUBBANDS_8 << 2) | SBC_ALLOCATION_LOUDNESS,
            2,
            53,
        ]
    };
    let mut sbc = Box::<SbcStruct>::default();
    unsafe {
        let _ = (sbc_api.init_a2dp)(&mut *sbc, 0, scfg.as_mut_ptr().cast::<c_void>(), 4);
    }
    sbc.endian = SBC_LE;
    st.enc.sbc_enc = Some(sbc);

    let freq_nibble = (scfg[0] >> 4) & 0x0F;
    let ch_nibble = scfg[0] & 0x0F;
    st.bt_sample_rate = match freq_nibble {
        SBC_SAMPLING_FREQ_44100 => 44_100,
        SBC_SAMPLING_FREQ_32000 => 32_000,
        SBC_SAMPLING_FREQ_16000 => 16_000,
        _ => 48_000,
    };
    st.bt_channels = if ch_nibble == SBC_CHANNEL_MODE_MONO { 1 } else { 2 };
    st.effective_kbps = if scfg[3] >= 50 { 328 } else { 229 };
    st.codec_name = "SBC".to_string();
    0
}

// ============================================================================
// Paced A2DP Bluetooth TX Thread (RTP + LDAC / aptX / FDK-AAC / SBC)
// ============================================================================

fn pop_s16_for_tx(
    rt: &BluezRuntime,
    bt_sample_rate: u32,
    out_s16: &mut [i16],
    needed_frames: usize,
    bt_ch: u32,
    frac_pos: &mut f64,
    drift_scale: f64,
) -> (usize, u32) {
    let in_rate = match rt.engine_sample_rate.load(Ordering::Relaxed) {
        0 => 48_000,
        r => r,
    };
    let out_rate = if bt_sample_rate > 0 {
        bt_sample_rate
    } else {
        48_000
    };
    let muted = rt.desktop_muted.load(Ordering::Relaxed);
    let vol_pct = rt.desktop_volume_pct.load(Ordering::Relaxed);
    let avrcp_sup = rt.avrcp_supported.load(Ordering::Relaxed);
    let ring = rt.tx_ring.lock().unwrap();
    pop_bt_resampled_s16(
        &ring,
        &rt.tx_write_frame,
        &rt.tx_read_frame,
        in_rate,
        out_rate,
        muted,
        vol_pct,
        avrcp_sup,
        out_s16,
        needed_frames,
        bt_ch,
        frac_pos,
        drift_scale,
    )
}

fn sync_tx_ready(rt: &BluezRuntime, st: &BtMutable) {
    let ready =
        st.has_bluez_transport && st.direct_socket_mode && st.bt_fd >= 0 && st.tx_stream_active;
    rt.tx_transport_ready.store(ready, Ordering::Release);
}

fn configure_bt_l2cap_socket(fd: c_int, write_mtu: u16) {
    if fd < 0 {
        return;
    }
    let sndbuf = ((usize::from(write_mtu.max(672))) * 24).clamp(32_768, 65_536) as c_int;
    let prio: c_int = 6; // TC_PRIO_INTERACTIVE for low-jitter A2DP L2CAP frames
    unsafe {
        let _ = setsockopt_raw(
            fd,
            SOL_SOCKET,
            SO_SNDBUF,
            (&sndbuf as *const c_int).cast::<c_void>(),
            mem::size_of::<c_int>() as u32,
        );
        let _ = setsockopt_raw(
            fd,
            SOL_SOCKET,
            SO_PRIORITY,
            (&prio as *const c_int).cast::<c_void>(),
            mem::size_of::<c_int>() as u32,
        );
    }
}

#[inline]
fn should_reprebuffer_after_partial_pop(
    popped: usize,
    expected_pop: usize,
    partial_streak: u32,
) -> bool {
    if expected_pop == 0 {
        return false;
    }
    popped == 0 || popped < (expected_pop / 2) || partial_streak >= 3
}

fn send_l2cap_packet(rt: &BluezRuntime, tx_fd: c_int, pkt: &[u8]) -> (isize, bool, c_int) {
    let mut sres = send_raw(tx_fd, pkt, MSG_DONTWAIT | MSG_NOSIGNAL);
    if sres >= 0 {
        return (sres, false, 0);
    }
    let mut saved_err = last_errno();
    if saved_err != ERRNO_EAGAIN {
        return (sres, false, saved_err);
    }
    for _ in 0..4 {
        if !rt.tx_thread_running.load(Ordering::Acquire) || rt.tx_paused.load(Ordering::Acquire) {
            break;
        }
        let mut pfd = [PollFd {
            fd: tx_fd,
            events: POLLOUT,
            revents: 0,
        }];
        let pr = do_poll(&mut pfd, 12);
        if pr > 0 && (pfd[0].revents & (POLLERR | POLLHUP | POLLNVAL)) != 0 {
            return (-1, true, ERRNO_EPIPE);
        }
        if pr > 0 && (pfd[0].revents & POLLOUT) != 0 {
            sres = send_raw(tx_fd, pkt, MSG_DONTWAIT | MSG_NOSIGNAL);
            if sres > 0 {
                return (sres, true, 0);
            }
            saved_err = last_errno();
            if saved_err != ERRNO_EAGAIN {
                return (sres, true, saved_err);
            }
        }
    }
    (-1, true, saved_err)
}

fn spawn_tx_thread(rt: Arc<BluezRuntime>) {
    let rt_bg = Arc::clone(&rt);
    let jh = thread::spawn(move || {
        let rt = rt_bg;
        enable_realtime(58, -14);

        let mut pkt = [0u8; 2048];
        let mut pcm = [0i16; 4096];
        let mut s24_buf = [0u8; 96 * 24];
        let mut frac_pos = 0.0f64;
        let mut next_tick_ns = mono_ns();
        let mut prebuffering = true;
        let mut avg_avail = 2160.0f64;
        let mut partial_streak = 0u32;
        let mut local_clean_streak = 0u32;
        let mut last_congest_log_ns = 0u64;

        while rt.tx_thread_running.load(Ordering::Acquire) {
            let (active, rate, ch, raw_mtu, codec_id, codec_kind, fdk_flen) = {
                let st = rt.state.lock().unwrap();
                (
                    st.tx_stream_active && st.bt_fd >= 0 && st.has_bluez_transport,
                    if st.bt_sample_rate > 0 { st.bt_sample_rate } else { 48_000 },
                    if st.bt_channels > 0 { st.bt_channels } else { 2 },
                    if st.write_mtu > 0 { usize::from(st.write_mtu) } else { 672 },
                    st.codec_id,
                    st.active_codec_kind,
                    if st.enc.fdk_frame_length > 0 {
                        st.enc.fdk_frame_length
                    } else {
                        1024
                    },
                )
            };
            let in_rate = match rt.engine_sample_rate.load(Ordering::Relaxed) {
                0 => 48_000,
                r => r,
            };

            if rt.tx_paused.load(Ordering::Acquire) || !active {
                prebuffering = true;
                frac_pos = 0.0;
                avg_avail = bt_tx_margin_frames(in_rate, 1024) as f64;
                partial_streak = 0;
                local_clean_streak = 0;
                next_tick_ns = mono_ns();
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            let mtu = raw_mtu.min(pkt.len());
            let est_need_out = if codec_kind == LWASAPI_BT_CODEC_LDAC {
                LDACBT_ENC_LSU
            } else if codec_kind == LWASAPI_BT_CODEC_APTX_HD
                || codec_kind == LWASAPI_BT_CODEC_APTX
            {
                256
            } else if codec_id == A2DP_CODEC_MPEG24 || codec_kind == LWASAPI_BT_CODEC_AAC {
                fdk_flen as usize
            } else {
                512
            };
            let est_need_in =
                ((est_need_out as f64) * (f64::from(in_rate) / f64::from(rate))).ceil() as u64 + 4;
            let margin_in = bt_tx_margin_frames(in_rate, est_need_in);

            {
                let wr = rt.tx_write_frame.load(Ordering::Acquire);
                let rd = rt.tx_read_frame.load(Ordering::Relaxed);
                let mut avail = wr.saturating_sub(rd);
                if prebuffering {
                    let goal = est_need_in
                        .saturating_add(margin_in)
                        .min((BT_TX_RING_FRAMES as u64) - 1024);
                    if avail < goal {
                        thread::sleep(Duration::from_micros(1500));
                        continue;
                    }
                    prebuffering = false;
                    partial_streak = 0;
                    avg_avail = if avail > est_need_in {
                        (avail - est_need_in) as f64
                    } else {
                        margin_in as f64
                    };
                    next_tick_ns = mono_ns();
                } else if avail < est_need_in {
                    for _ in 0..12 {
                        if !rt.tx_thread_running.load(Ordering::Acquire)
                            || rt.tx_paused.load(Ordering::Acquire)
                        {
                            break;
                        }
                        thread::sleep(Duration::from_micros(1000));
                        let wr2 = rt.tx_write_frame.load(Ordering::Acquire);
                        let rd2 = rt.tx_read_frame.load(Ordering::Relaxed);
                        avail = wr2.saturating_sub(rd2);
                        if avail >= est_need_in {
                            break;
                        }
                    }
                }
            }

            let margin_f = margin_in.max(1) as f64;
            let drift_scale = if avg_avail > margin_f * 1.65 {
                1.0006
            } else if avg_avail < margin_f * 0.70 {
                0.9994
            } else {
                1.0
            };

            let mut pkt_len = 0usize;
            let mut ts_step = 0u32;
            let mut expected_pop = 0usize;
            let mut popped = 0usize;
            let mut rem_avail = 0u32;
            let tx_fd;
            let tx_fd_owner;
            let mut dev_alias = String::new();

            {
                let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
                if st.bt_fd < 0 || !st.tx_stream_active {
                    continue;
                }
                tx_fd_owner = st.bt_fd_owner.clone();
                if tx_fd_owner.is_none() {
                    continue;
                }
                tx_fd = st.bt_fd;
                dev_alias.clone_from(&st.device_alias);

                if st.active_codec_kind == LWASAPI_BT_CODEC_LDAC && !st.enc.ldac_enc.is_null() {
                    let flen = LDACBT_ENC_LSU;
                    let hdr_off = BT_RTP_HDR_BYTES + BT_RTP_SBC_PL_BYTES;
                    expected_pop = flen;
                    (popped, rem_avail) = pop_s16_for_tx(
                        &rt,
                        st.bt_sample_rate,
                        &mut pcm,
                        flen,
                        ch,
                        &mut frac_pos,
                        drift_scale,
                    );
                    if mtu > hdr_off {
                        if let Some(ldac) = load_ldac_library() {
                            let mut pcm_used = (flen * (ch as usize) * 2) as c_int;
                            let mut stream_sz = (mtu - hdr_off) as c_int;
                            let mut frame_num = 0 as c_int;
                            let enc_rc = unsafe {
                                (ldac.encode)(
                                    st.enc.ldac_enc,
                                    pcm.as_mut_ptr().cast::<c_void>(),
                                    &mut pcm_used,
                                    pkt[hdr_off..].as_mut_ptr(),
                                    &mut stream_sz,
                                    &mut frame_num,
                                )
                            };
                            if enc_rc == 0
                                && frame_num > 0
                                && stream_sz > 0
                                && hdr_off + (stream_sz as usize) <= mtu
                            {
                                let seq = st.rtp_seq;
                                st.rtp_seq = st.rtp_seq.wrapping_add(1);
                                let ts = st.rtp_timestamp;
                                if write_rtp_header(&mut pkt[..mtu], 96, false, seq, ts, 1)
                                    == Some(BT_RTP_HDR_BYTES)
                                    && write_rtp_sbc_payload_hdr(
                                        &mut pkt[BT_RTP_HDR_BYTES..mtu],
                                        frame_num as u8,
                                    ) == Some(BT_RTP_SBC_PL_BYTES)
                                {
                                    pkt_len = hdr_off + (stream_sz as usize);
                                }
                            }
                        }
                    }
                    ts_step = flen as u32;
                } else if (st.active_codec_kind == LWASAPI_BT_CODEC_APTX_HD
                    || st.active_codec_kind == LWASAPI_BT_CODEC_APTX)
                    && !st.enc.aptx_enc.is_null()
                {
                    let is_hd = st.active_codec_kind == LWASAPI_BT_CODEC_APTX_HD;
                    let hdr_off = if is_hd { BT_RTP_HDR_BYTES } else { 0 };
                    let enc_blk_bytes = if is_hd { 6usize } else { 4usize };
                    let avail_out = mtu.saturating_sub(hdr_off);
                    let max_blocks = (avail_out / enc_blk_bytes).min(96);
                    let flen = (max_blocks * 4).max(4);
                    expected_pop = flen;
                    (popped, rem_avail) = pop_s16_for_tx(
                        &rt,
                        st.bt_sample_rate,
                        &mut pcm,
                        flen,
                        2,
                        &mut frac_pos,
                        drift_scale,
                    );
                    let nsamp = flen * 2;
                    if let (Some(aptx), Some(s24_len)) = (
                        load_aptx_library(),
                        pack_s16_to_s24le(&pcm[..nsamp], &mut s24_buf),
                    ) {
                        if s24_len > 0 && avail_out >= enc_blk_bytes {
                            let mut written = 0usize;
                            unsafe {
                                let _ = (aptx.encode)(
                                    st.enc.aptx_enc,
                                    s24_buf.as_ptr(),
                                    s24_len,
                                    pkt[hdr_off..].as_mut_ptr(),
                                    avail_out,
                                    &mut written,
                                );
                            }
                            if written > 0 && hdr_off + written <= mtu {
                                if !is_hd {
                                    pkt_len = hdr_off + written;
                                } else {
                                    let seq = st.rtp_seq;
                                    st.rtp_seq = st.rtp_seq.wrapping_add(1);
                                    let ts = st.rtp_timestamp;
                                    if write_rtp_header(&mut pkt[..mtu], 96, false, seq, ts, 1)
                                        == Some(BT_RTP_HDR_BYTES)
                                    {
                                        pkt_len = hdr_off + written;
                                    }
                                }
                            }
                        }
                    }
                    ts_step = flen as u32;
                } else if (st.codec_id == A2DP_CODEC_MPEG24
                    || st.active_codec_kind == LWASAPI_BT_CODEC_AAC)
                    && !st.enc.fdk_enc.is_null()
                {
                    let flen = if st.enc.fdk_frame_length > 0 {
                        st.enc.fdk_frame_length as usize
                    } else {
                        1024
                    };
                    expected_pop = flen;
                    (popped, rem_avail) = pop_s16_for_tx(
                        &rt,
                        st.bt_sample_rate,
                        &mut pcm,
                        flen,
                        ch,
                        &mut frac_pos,
                        drift_scale,
                    );
                    if mtu > BT_RTP_HDR_BYTES {
                        if let Some(fdk) = load_fdk_aac_library() {
                            let mut in_ptr = pcm.as_mut_ptr().cast::<c_void>();
                            let mut out_ptr = pkt[BT_RTP_HDR_BYTES..].as_mut_ptr().cast::<c_void>();
                            let mut in_id = FDK_IN_AUDIO_DATA;
                            let mut out_id = FDK_OUT_BITSTREAM_DATA;
                            let mut in_sz = (flen * (ch as usize) * 2) as c_int;
                            let mut out_sz = (mtu - BT_RTP_HDR_BYTES) as c_int;
                            let mut in_el = 2 as c_int;
                            let mut out_el = 1 as c_int;
                            let in_desc = FdkBufDesc {
                                num_bufs: 1,
                                bufs: &mut in_ptr,
                                buffer_identifiers: &mut in_id,
                                buf_sizes: &mut in_sz,
                                buf_el_sizes: &mut in_el,
                            };
                            let out_desc = FdkBufDesc {
                                num_bufs: 1,
                                bufs: &mut out_ptr,
                                buffer_identifiers: &mut out_id,
                                buf_sizes: &mut out_sz,
                                buf_el_sizes: &mut out_el,
                            };
                            let in_args = FdkInArgs {
                                num_in_samples: (flen * (ch as usize)) as c_int,
                                num_anc_bytes: 0,
                            };
                            let mut out_args = FdkOutArgs::default();
                            if unsafe {
                                (fdk.encode)(
                                    st.enc.fdk_enc,
                                    &in_desc,
                                    &out_desc,
                                    &in_args,
                                    &mut out_args,
                                )
                            } == FDK_AACENC_OK
                                && out_args.num_out_bytes > 0
                                && BT_RTP_HDR_BYTES + (out_args.num_out_bytes as usize) <= mtu
                            {
                                let seq = st.rtp_seq;
                                st.rtp_seq = st.rtp_seq.wrapping_add(1);
                                let ts = st.rtp_timestamp;
                                if write_rtp_header(&mut pkt[..mtu], 96, true, seq, ts, 1)
                                    == Some(BT_RTP_HDR_BYTES)
                                {
                                    pkt_len =
                                        BT_RTP_HDR_BYTES + (out_args.num_out_bytes as usize);
                                }
                            }
                        }
                    }
                    ts_step = flen as u32;
                } else if st.enc.sbc_enc.is_some() {
                    if let Some(sbc_api) = load_sbc_library() {
                        let sbc_ptr = (&mut **st.enc.sbc_enc.as_mut().unwrap()) as *mut SbcStruct;
                        let codesize = unsafe { (sbc_api.get_codesize)(sbc_ptr) };
                        let mut samples_per_frame =
                            codesize / ((ch as usize).max(1) * mem::size_of::<i16>());
                        if samples_per_frame == 0 {
                            samples_per_frame = 128;
                        }
                        let max_frames = if 4 * samples_per_frame * (ch as usize) > 4096 {
                            2usize
                        } else {
                            4usize
                        };
                        expected_pop = max_frames * samples_per_frame;
                        (popped, rem_avail) = pop_s16_for_tx(
                            &rt,
                            st.bt_sample_rate,
                            &mut pcm,
                            expected_pop,
                            ch,
                            &mut frac_pos,
                            drift_scale,
                        );
                        let header_len = BT_RTP_HDR_BYTES + BT_RTP_SBC_PL_BYTES;
                        let mut off = header_len;
                        let mut encoded_cnt = 0u32;
                        if mtu > header_len {
                            for f in 0..max_frames {
                                let mut enc_written = 0isize;
                                let pcm_off = f * samples_per_frame * (ch as usize);
                                let in_consumed = unsafe {
                                    (sbc_api.encode)(
                                        sbc_ptr,
                                        pcm[pcm_off..].as_ptr().cast::<c_void>(),
                                        codesize,
                                        pkt[off..].as_mut_ptr().cast::<c_void>(),
                                        mtu - off,
                                        &mut enc_written,
                                    )
                                };
                                if in_consumed <= 0 || enc_written <= 0 {
                                    break;
                                }
                                off += enc_written as usize;
                                encoded_cnt += 1;
                                if off + (enc_written as usize) > mtu {
                                    break;
                                }
                            }
                        }
                        if encoded_cnt > 0 {
                            let seq = st.rtp_seq;
                            st.rtp_seq = st.rtp_seq.wrapping_add(1);
                            let ts = st.rtp_timestamp;
                            if write_rtp_header(&mut pkt[..mtu], 96, false, seq, ts, 1)
                                == Some(BT_RTP_HDR_BYTES)
                                && write_rtp_sbc_payload_hdr(
                                    &mut pkt[BT_RTP_HDR_BYTES..mtu],
                                    encoded_cnt as u8,
                                ) == Some(BT_RTP_SBC_PL_BYTES)
                            {
                                pkt_len = off;
                            }
                        }
                        ts_step = encoded_cnt * (samples_per_frame as u32);
                    }
                }

                st.rtp_timestamp = st.rtp_timestamp.wrapping_add(ts_step);
            }

            if popped == expected_pop {
                partial_streak = 0;
                avg_avail = avg_avail * 0.90 + f64::from(rem_avail) * 0.10;
            } else if expected_pop > 0 {
                partial_streak = partial_streak.saturating_add(1);
                rt.tx_underruns.fetch_add(1, Ordering::Relaxed);
                if should_reprebuffer_after_partial_pop(popped, expected_pop, partial_streak) {
                    prebuffering = true;
                    partial_streak = 0;
                    dbg_emit_vals(
                        LWASAPI_DBG_BLUEZ,
                        LWASAPI_DBG_WARN,
                        0,
                        0,
                        popped as i32,
                        expected_pop as i32,
                        0,
                        "BT_TX_UNDERRUN",
                        &dev_alias,
                        &format!("TX ring underrun ({popped}/{expected_pop} frames) -> re-prebuffering"),
                    );
                } else {
                    avg_avail = avg_avail * 0.70 + f64::from(rem_avail) * 0.30;
                    dbg_emit_vals(
                        LWASAPI_DBG_BLUEZ,
                        LWASAPI_DBG_WARN,
                        0,
                        0,
                        popped as i32,
                        expected_pop as i32,
                        0,
                        "BT_TX_UNDERRUN",
                        &dev_alias,
                        &format!("TX ring minor slip ({popped}/{expected_pop} frames) -> keeping link live"),
                    );
                }
            }

            if pkt_len > 0 && tx_fd >= 0 {
                let (sres, had_eagain, saved_err) =
                    send_l2cap_packet(&rt, tx_fd, &pkt[..pkt_len]);
                if sres > 0 {
                    rt.tx_packets_sent.fetch_add(1, Ordering::Relaxed);
                    if !had_eagain {
                        local_clean_streak = local_clean_streak.saturating_add(1);
                    }
                    let check_abr = had_eagain
                        || local_clean_streak >= 8
                        || (codec_kind == LWASAPI_BT_CODEC_LDAC && local_clean_streak >= 4);
                    if check_abr {
                        let now_ms = mono_ns() / 1_000_000;
                        let mut st = rt.state.lock().unwrap();
                        if had_eagain {
                            st.tx_congest_streak = st.tx_congest_streak.saturating_add(1);
                            st.tx_clean_streak = 0;
                        } else {
                            st.tx_congest_streak =
                                st.tx_congest_streak.saturating_sub(local_clean_streak);
                            st.tx_clean_streak =
                                st.tx_clean_streak.saturating_add(local_clean_streak);
                        }
                        local_clean_streak = 0;
                        if st.active_codec_kind == LWASAPI_BT_CODEC_LDAC
                            && !st.enc.ldac_enc.is_null()
                            && st.ldac_mode == LWASAPI_LDAC_MODE_ADAPTIVE
                            && st.wireless_abr_enabled != 0
                            && now_ms.saturating_sub(st.abr_last_ms) >= 20
                        {
                            st.abr_last_ms = now_ms;
                            let qdepth = if had_eagain {
                                4 + st.tx_congest_streak.min(6)
                            } else {
                                0
                            };
                            let prev_eq = st.enc.ldac_eqmid;
                            let mut new_eq = prev_eq;
                            if !st.enc.ldac_abr.is_null() {
                                if let Some(abr_api) = load_ldac_abr_library() {
                                    let ret = unsafe {
                                        (abr_api.proc_step)(
                                            st.enc.ldac_enc,
                                            st.enc.ldac_abr,
                                            qdepth,
                                            1,
                                        )
                                    };
                                    if (LDACBT_EQMID_HQ..=LDACBT_EQMID_MQ).contains(&ret) {
                                        new_eq = ret;
                                    }
                                }
                            } else if let Some(set_eqmid) =
                                load_ldac_library().and_then(|l| l.set_eqmid)
                            {
                                let (cand_eq, clr_cong, clr_clean) = abr_step_ldac(
                                    prev_eq,
                                    st.tx_congest_streak,
                                    st.tx_clean_streak,
                                );
                                if cand_eq != prev_eq
                                    && unsafe { set_eqmid(st.enc.ldac_enc, cand_eq) } == 0
                                {
                                    new_eq = cand_eq;
                                    if clr_cong {
                                        st.tx_congest_streak = 0;
                                    }
                                    if clr_clean {
                                        st.tx_clean_streak = 0;
                                    }
                                }
                            }
                            if new_eq != prev_eq
                                && (LDACBT_EQMID_HQ..=LDACBT_EQMID_MQ).contains(&new_eq)
                            {
                                st.enc.ldac_eqmid = new_eq;
                                st.effective_kbps = ldac_eqmid_to_kbps(st.bt_sample_rate, new_eq);
                                st.neg_reason =
                                    format!("LDAC ABR adapted -> {}kbps", st.effective_kbps);
                                dbg_emit_vals(
                                    LWASAPI_DBG_BLUEZ,
                                    LWASAPI_DBG_INFO,
                                    0,
                                    0,
                                    prev_eq,
                                    new_eq,
                                    st.effective_kbps as i32,
                                    "BT_LDAC_ABR",
                                    &st.device_alias,
                                    &format!(
                                        "eqmid {} -> {} ({}kbps)",
                                        prev_eq, new_eq, st.effective_kbps
                                    ),
                                );
                            }
                        } else if (st.codec_id == A2DP_CODEC_MPEG24
                            || st.active_codec_kind == LWASAPI_BT_CODEC_AAC)
                            && !st.enc.fdk_enc.is_null()
                            && st.wireless_abr_enabled != 0
                        {
                            let cur_bps = if st.aac_target_bps > 0 {
                                st.aac_target_bps
                            } else {
                                192_000
                            };
                            let max_bps = if st.aac_max_bps > 0 {
                                st.aac_max_bps
                            } else {
                                256_000
                            };
                            let (next_bps, clr_cong, clr_clean) = abr_step_aac(
                                cur_bps,
                                max_bps,
                                st.tx_congest_streak,
                                st.tx_clean_streak,
                            );
                            if clr_cong {
                                st.tx_congest_streak = 0;
                            }
                            if clr_clean {
                                st.tx_clean_streak = 0;
                            }
                            if next_bps != cur_bps {
                                if let Some(fdk) = load_fdk_aac_library() {
                                    if unsafe {
                                        (fdk.set_param)(st.enc.fdk_enc, FDK_PARAM_BITRATE, next_bps)
                                    } == FDK_AACENC_OK
                                    {
                                        st.aac_target_bps = next_bps;
                                        st.effective_kbps = next_bps / 1000;
                                        st.neg_reason =
                                            format!("AAC ABR adapted -> {}kbps", st.effective_kbps);
                                    }
                                }
                            }
                        }
                    }
                } else if sres < 0 {
                    if saved_err == ERRNO_EPIPE
                        || saved_err == ERRNO_ECONNRESET
                        || saved_err == ERRNO_EBADF
                    {
                        rt.tx_errors.fetch_add(1, Ordering::Relaxed);
                        dbg_emit_vals(
                            LWASAPI_DBG_BLUEZ,
                            LWASAPI_DBG_ERR,
                            0,
                            0,
                            tx_fd,
                            saved_err,
                            -saved_err,
                            "BT_TX_ERR",
                            &dev_alias,
                            &format!(
                                "L2CAP send failed on fd={tx_fd}: {}",
                                std::io::Error::from_raw_os_error(saved_err)
                            ),
                        );
                        {
                            let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
                            if st.bt_fd == tx_fd {
                                st.close_bt_fd();
                                st.direct_socket_mode = false;
                                st.tx_stream_active = false;
                                st.has_bluez_transport = false;
                                sync_tx_ready(&rt, &st);
                            }
                        }
                        rt.tx_drop_event_ns.store(mono_ns(), Ordering::Relaxed);
                        continue;
                    } else if saved_err == ERRNO_EAGAIN {
                        rt.tx_errors.fetch_add(1, Ordering::Relaxed);
                        local_clean_streak = 0;
                        {
                            let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
                            st.tx_congest_streak = st.tx_congest_streak.saturating_add(1);
                            st.tx_clean_streak = 0;
                        }
                        let now_c = mono_ns();
                        if now_c.saturating_sub(last_congest_log_ns) >= 2_000_000_000 {
                            last_congest_log_ns = now_c;
                            dbg_emit_vals(
                                LWASAPI_DBG_BLUEZ,
                                LWASAPI_DBG_WARN,
                                0,
                                0,
                                tx_fd,
                                pkt_len as i32,
                                0,
                                "BT_TX_CONGEST",
                                &dev_alias,
                                "L2CAP socket backpressure persisted after POLLOUT retry window; packet dropped",
                            );
                        }
                    } else {
                        rt.tx_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            if prebuffering {
                next_tick_ns = mono_ns();
                continue;
            }
            let step_frames = if ts_step == 0 { 1024 } else { ts_step };
            let period_ns = (u64::from(step_frames) * 1_000_000_000) / u64::from(rate);
            next_tick_ns = next_tick_ns.saturating_add(period_ns);
            let now_ns = mono_ns();
            if now_ns > next_tick_ns.saturating_add(period_ns * 2) {
                next_tick_ns = now_ns.saturating_add(period_ns);
            }
            sleep_abs_mono_ns(next_tick_ns);
        }
    });
    *rt.tx_thread.lock().unwrap() = Some(jh);
}

// ============================================================================
// Shared BlueZ ObjectManager (GetManagedObjects) Scanner
// ============================================================================

struct ManagedDevice {
    obj_path: String,
    address: String,
    alias: String,
    connected: bool,
    paired: bool,
    trusted: bool,
    has_a2dp_sink: bool,
    has_hfp_hf: bool,
}

struct ManagedTransport {
    device_path: String,
    state: String,
}

struct ManagedSep {
    obj_path: String,
    is_sink_sep: bool,
    codec: u8,
    caps: Vec<u8>,
}

#[derive(Default)]
struct ManagedObjects {
    adapters: Vec<String>,
    devices: Vec<ManagedDevice>,
    transports: Vec<ManagedTransport>,
    seps: Vec<ManagedSep>,
}

fn fetch_managed_objects(
    db: &DBusApi,
    conn: *mut DBusConnection,
    timeout_ms: c_int,
) -> Option<ManagedObjects> {
    if conn.is_null() {
        return None;
    }
    let msg = dbus_new_method_call(
        db,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
    );
    if msg.is_null() {
        return None;
    }
    let rep = unsafe {
        let r = (db.connection_send_with_reply_and_block)(conn, msg, timeout_ms, ptr::null_mut());
        (db.message_unref)(msg);
        r
    };
    if rep.is_null() {
        return None;
    }

    let mut out = ManagedObjects::default();
    unsafe {
        let mut root = DBusMessageIter::default();
        let mut obj_arr = DBusMessageIter::default();
        if (db.message_iter_init)(rep, &mut root) != 0
            && (db.message_iter_get_arg_type)(&mut root) == DBUS_TYPE_ARRAY
        {
            (db.message_iter_recurse)(&mut root, &mut obj_arr);
            while (db.message_iter_get_arg_type)(&mut obj_arr) == DBUS_TYPE_DICT_ENTRY {
                let mut obj_entry = DBusMessageIter::default();
                let mut if_arr = DBusMessageIter::default();
                (db.message_iter_recurse)(&mut obj_arr, &mut obj_entry);
                let obj_path = iter_get_str(db, &mut obj_entry).unwrap_or_default();
                if (db.message_iter_next)(&mut obj_entry) != 0 {
                    (db.message_iter_recurse)(&mut obj_entry, &mut if_arr);
                    while (db.message_iter_get_arg_type)(&mut if_arr) == DBUS_TYPE_DICT_ENTRY {
                        let mut if_entry = DBusMessageIter::default();
                        let mut prop_arr = DBusMessageIter::default();
                        (db.message_iter_recurse)(&mut if_arr, &mut if_entry);
                        let iface = iter_get_str(db, &mut if_entry).unwrap_or_default();
                        if (db.message_iter_next)(&mut if_entry) != 0 {
                            (db.message_iter_recurse)(&mut if_entry, &mut prop_arr);
                            if iface == "org.bluez.Media1" && !obj_path.is_empty() {
                                out.adapters.push(obj_path.clone());
                            } else if iface == "org.bluez.Device1" && !obj_path.is_empty() {
                                let mut dev = ManagedDevice {
                                    obj_path: obj_path.clone(),
                                    address: String::new(),
                                    alias: String::new(),
                                    connected: false,
                                    paired: false,
                                    trusted: false,
                                    has_a2dp_sink: false,
                                    has_hfp_hf: false,
                                };
                                while (db.message_iter_get_arg_type)(&mut prop_arr)
                                    == DBUS_TYPE_DICT_ENTRY
                                {
                                    let mut pe = DBusMessageIter::default();
                                    let mut var = DBusMessageIter::default();
                                    (db.message_iter_recurse)(&mut prop_arr, &mut pe);
                                    let pk = iter_get_str(db, &mut pe).unwrap_or_default();
                                    if (db.message_iter_next)(&mut pe) != 0 {
                                        (db.message_iter_recurse)(&mut pe, &mut var);
                                        match pk.as_str() {
                                            "Connected" => {
                                                dev.connected =
                                                    iter_get_bool(db, &mut var).unwrap_or(false)
                                            }
                                            "Paired" => {
                                                dev.paired =
                                                    iter_get_bool(db, &mut var).unwrap_or(false)
                                            }
                                            "Trusted" => {
                                                dev.trusted =
                                                    iter_get_bool(db, &mut var).unwrap_or(false)
                                            }
                                            "Address" => {
                                                dev.address =
                                                    iter_get_str(db, &mut var).unwrap_or_default()
                                            }
                                            "Alias" => {
                                                dev.alias =
                                                    iter_get_str(db, &mut var).unwrap_or_default()
                                            }
                                            "UUIDs" => {
                                                let arg_type =
                                                    (db.message_iter_get_arg_type)(&mut var);
                                                if arg_type == DBUS_TYPE_ARRAY {
                                                    let mut uarr = DBusMessageIter::default();
                                                    (db.message_iter_recurse)(&mut var, &mut uarr);
                                                    while (db.message_iter_get_arg_type)(&mut uarr)
                                                        == DBUS_TYPE_STRING
                                                    {
                                                        if let Some(u) = iter_get_str(db, &mut uarr)
                                                        {
                                                            if u.len() >= 8
                                                                && u[..8].eq_ignore_ascii_case(
                                                                    "0000110b",
                                                                )
                                                            {
                                                                dev.has_a2dp_sink = true;
                                                            }
                                                            if u.len() >= 8
                                                                && u[..8].eq_ignore_ascii_case(
                                                                    "0000111e",
                                                                )
                                                            {
                                                                dev.has_hfp_hf = true;
                                                            }
                                                        }
                                                        if (db.message_iter_next)(&mut uarr) == 0 {
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    if (db.message_iter_next)(&mut prop_arr) == 0 {
                                        break;
                                    }
                                }
                                out.devices.push(dev);
                            } else if iface == "org.bluez.MediaTransport1" && !obj_path.is_empty() {
                                let mut tdev = String::new();
                                let mut tstate = String::new();
                                while (db.message_iter_get_arg_type)(&mut prop_arr)
                                    == DBUS_TYPE_DICT_ENTRY
                                {
                                    let mut pe = DBusMessageIter::default();
                                    let mut var = DBusMessageIter::default();
                                    (db.message_iter_recurse)(&mut prop_arr, &mut pe);
                                    let pk = iter_get_str(db, &mut pe).unwrap_or_default();
                                    if (db.message_iter_next)(&mut pe) != 0 {
                                        (db.message_iter_recurse)(&mut pe, &mut var);
                                        if pk == "Device" {
                                            tdev = iter_get_str(db, &mut var).unwrap_or_default();
                                        } else if pk == "State" {
                                            tstate = iter_get_str(db, &mut var).unwrap_or_default();
                                        }
                                    }
                                    if (db.message_iter_next)(&mut prop_arr) == 0 {
                                        break;
                                    }
                                }
                                if tdev.is_empty() {
                                    tdev = match obj_path.find("/sep") {
                                        Some(pos) => obj_path[..pos].to_string(),
                                        None => obj_path.clone(),
                                    };
                                }
                                out.transports.push(ManagedTransport {
                                    device_path: tdev,
                                    state: tstate,
                                });
                            } else if iface == "org.bluez.MediaEndpoint1" && !obj_path.is_empty() {
                                let mut sep = ManagedSep {
                                    obj_path: obj_path.clone(),
                                    is_sink_sep: false,
                                    codec: 0xFF,
                                    caps: Vec::new(),
                                };
                                while (db.message_iter_get_arg_type)(&mut prop_arr)
                                    == DBUS_TYPE_DICT_ENTRY
                                {
                                    let mut pe = DBusMessageIter::default();
                                    let mut var = DBusMessageIter::default();
                                    (db.message_iter_recurse)(&mut prop_arr, &mut pe);
                                    let pk = iter_get_str(db, &mut pe).unwrap_or_default();
                                    if (db.message_iter_next)(&mut pe) != 0 {
                                        (db.message_iter_recurse)(&mut pe, &mut var);
                                        if pk == "UUID" {
                                            if let Some(u) = iter_get_str(db, &mut var) {
                                                if u.len() >= 8
                                                    && u[..8].eq_ignore_ascii_case("0000110b")
                                                {
                                                    sep.is_sink_sep = true;
                                                }
                                            }
                                        } else if pk == "Codec" {
                                            sep.codec = iter_get_byte(db, &mut var).unwrap_or(0xFF);
                                        } else if pk == "Capabilities" {
                                            if let Some(bytes) = iter_get_byte_array(db, &mut var) {
                                                sep.caps = bytes.into_iter().take(16).collect();
                                            }
                                        }
                                    }
                                    if (db.message_iter_next)(&mut prop_arr) == 0 {
                                        break;
                                    }
                                }
                                out.seps.push(sep);
                            }
                        }
                        if (db.message_iter_next)(&mut if_arr) == 0 {
                            break;
                        }
                    }
                }
                if (db.message_iter_next)(&mut obj_arr) == 0 {
                    break;
                }
            }
        }
        (db.message_unref)(rep);
    }
    Some(out)
}

// ============================================================================
// D-Bus Endpoint Negotiation, Signal Filtering & HFP Battery/Profile Handling
// ============================================================================

fn extract_mac_str(id: &str) -> Option<String> {
    extract_mac_from_id(id.as_bytes()).map(|m| String::from_utf8_lossy(&m).into_owned())
}

fn send_connect_profile(db: &DBusApi, conn: *mut DBusConnection, dev_path: &str, uuid: &str) {
    if conn.is_null() || dev_path.is_empty() {
        return;
    }
    let msg = dbus_new_method_call(
        db,
        "org.bluez",
        dev_path,
        "org.bluez.Device1",
        "ConnectProfile",
    );
    if msg.is_null() {
        return;
    }
    unsafe {
        let mut it = DBusMessageIter::default();
        (db.message_iter_init_append)(msg, &mut it);
        if iter_append_str(db, &mut it, DBUS_TYPE_STRING, uuid) {
            let _ = (db.connection_send)(conn, msg, ptr::null_mut());
        }
        (db.message_unref)(msg);
    }
}

fn select_a2dp_config_for_endpoint(path: &str, rcaps: &[u8]) -> Option<(Vec<u8>, u32)> {
    let kind = match path {
        LWASAPI_EP_LDAC => BT_WIRE_LDAC,
        LWASAPI_EP_APTXHD => BT_WIRE_APTX_HD,
        LWASAPI_EP_APTX => BT_WIRE_APTX,
        LWASAPI_EP_AAC => BT_WIRE_AAC,
        LWASAPI_EP_SBC => BT_WIRE_SBC,
        _ => return None,
    };
    if kind == BT_WIRE_SBC && rcaps.len() >= 4 && (rcaps[2] < 2 || rcaps[3] > 53 || rcaps[2] > rcaps[3])
    {
        record_trip(FS_BT_FALLBACK, b"Clamped malformed A2DP SBC bitpool bounds");
    }
    let mut out = [0u8; 16];
    if let Some((len, cap)) = select_a2dp_config(kind, rcaps, &mut out) {
        Some((out[..len].to_vec(), cap))
    } else {
        record_trip(
            FS_BT_FALLBACK,
            b"Rejected malformed BlueZ A2DP endpoint capability payload",
        );
        None
    }
}

unsafe fn msg_headers<'a>(
    db: &DBusApi,
    msg: *mut DBusMessage,
) -> (Option<&'a str>, Option<&'a str>, Option<&'a str>) {
    unsafe {
        (
            cptr_to_opt_str((db.message_get_path)(msg)),
            cptr_to_opt_str((db.message_get_interface)(msg)),
            cptr_to_opt_str((db.message_get_member)(msg)),
        )
    }
}

fn acquire_bluez_transport(db: &DBusApi, conn: *mut DBusConnection, rt: &BluezRuntime) {
    let tp = rt.state.lock().unwrap().transport_path.clone();
    if conn.is_null() || tp.is_empty() {
        return;
    }
    let mut err = DBusError::default();
    unsafe { (db.error_init)(&mut err) };

    let mut msg = dbus_new_method_call(db, "org.bluez", &tp, "org.bluez.MediaTransport1", "Acquire");
    if msg.is_null() {
        return;
    }
    let mut rep = unsafe {
        let r = (db.connection_send_with_reply_and_block)(conn, msg, 3000, &mut err);
        (db.message_unref)(msg);
        r
    };
    if rep.is_null() {
        unsafe {
            (db.error_free)(&mut err);
            (db.error_init)(&mut err);
        }
        msg = dbus_new_method_call(
            db,
            "org.bluez",
            &tp,
            "org.bluez.MediaTransport1",
            "TryAcquire",
        );
        if msg.is_null() {
            return;
        }
        rep = unsafe {
            let r = (db.connection_send_with_reply_and_block)(conn, msg, 2000, &mut err);
            (db.message_unref)(msg);
            r
        };
    }

    if rep.is_null() {
        let emsg = unsafe { dbus_error_msg(&err) };
        println!("[LWASAPI-BlueZ] Transport Acquire deferred ({emsg})");
        dbg_emit(
            LWASAPI_DBG_BLUEZ,
            LWASAPI_DBG_WARN,
            0,
            0,
            "BT_ACQ_DEFER",
            &tp,
            &format!("MediaTransport1.Acquire deferred: {emsg}"),
        );
        unsafe { (db.error_free)(&mut err) };
        return;
    }

    unsafe {
        let mut it = DBusMessageIter::default();
        if (db.message_iter_init)(rep, &mut it) != 0 {
            let fd = iter_get_unix_fd(db, &mut it).unwrap_or(-1);
            let mut rmtu = 0u16;
            let mut wmtu = 0u16;
            if (db.message_iter_next)(&mut it) != 0 {
                rmtu = iter_get_u16(db, &mut it).unwrap_or(0);
            }
            if (db.message_iter_next)(&mut it) != 0 {
                wmtu = iter_get_u16(db, &mut it).unwrap_or(0);
            }
            if fd >= 0 {
                let (cname, alias, srate) = {
                    let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
                    st.replace_bt_fd(fd);
                    configure_bt_l2cap_socket(fd, wmtu);
                    st.read_mtu = rmtu;
                    st.write_mtu = wmtu;
                    st.direct_socket_mode = true;
                    st.has_bluez_transport = true;
                    init_bt_encoder_for_transport_locked(&mut st);
                    st.tx_stream_active = true;
                    sync_tx_ready(rt, &st);
                    (st.codec_name.clone(), st.device_alias.clone(), st.bt_sample_rate)
                };
                println!(
                    "[LWASAPI-BlueZ] Acquired L2CAP fd={fd} MTU={rmtu}/{wmtu} Codec={cname} ({alias} @ {srate}Hz)"
                );
                dbg_emit_vals(
                    LWASAPI_DBG_BLUEZ,
                    LWASAPI_DBG_INFO,
                    0,
                    0,
                    i32::from(rmtu),
                    i32::from(wmtu),
                    fd,
                    "BT_L2CAP_ACQ",
                    &alias,
                    &format!("L2CAP fd={fd} mtu={rmtu}/{wmtu} codec={cname} rate={srate}"),
                );
            }
        }
        (db.error_free)(&mut err);
        (db.message_unref)(rep);
    }
}

unsafe extern "C" fn endpoint_message_handler(
    conn: *mut DBusConnection,
    msg: *mut DBusMessage,
    user_data: *mut c_void,
) -> DBusHandlerResult {
    let Some(db) = load_dbus_library() else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    let Some(rt) = (unsafe { bt_from_ptr(user_data) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    let (Some(path), Some(iface), Some(member)) = (unsafe { msg_headers(db, msg) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    if iface != "org.bluez.MediaEndpoint1" {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    }

    if member == "SelectConfiguration" {
        let mut it = DBusMessageIter::default();
        if unsafe { (db.message_iter_init)(msg, &mut it) } == 0
            || unsafe { (db.message_iter_get_arg_type)(&mut it) } != DBUS_TYPE_ARRAY
        {
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        }
        let rcaps = unsafe { iter_get_byte_array(db, &mut it) }.unwrap_or_default();
        let selected = select_a2dp_config_for_endpoint(path, &rcaps);
        if let Some((ref _cfg, cap_bit)) = selected {
            if cap_bit != 0 {
                rt.state.lock().unwrap().supported_codecs_mask |= cap_bit;
            }
        }
        unsafe {
            let rep = if selected.is_some() {
                (db.message_new_method_return)(msg)
            } else {
                (db.message_new_error)(
                    msg,
                    c"org.bluez.Error.InvalidArguments".as_ptr(),
                    c"Unsupported codec capabilities".as_ptr(),
                )
            };
            if !rep.is_null() {
                if let Some((cfg, _)) = selected {
                    let mut rit = DBusMessageIter::default();
                    let mut rarr = DBusMessageIter::default();
                    (db.message_iter_init_append)(rep, &mut rit);
                    if (db.message_iter_open_container)(
                        &mut rit,
                        DBUS_TYPE_ARRAY,
                        c"y".as_ptr(),
                        &mut rarr,
                    ) != 0
                    {
                        let p = cfg.as_ptr();
                        let _ = (db.message_iter_append_fixed_array)(
                            &mut rarr,
                            DBUS_TYPE_BYTE,
                            (&p as *const *const u8).cast::<c_void>(),
                            cfg.len() as c_int,
                        );
                        let _ = (db.message_iter_close_container)(&mut rit, &mut rarr);
                    }
                }
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if member == "SetConfiguration" {
        let mut args = DBusMessageIter::default();
        if unsafe { (db.message_iter_init)(msg, &mut args) } == 0
            || unsafe { (db.message_iter_get_arg_type)(&mut args) } != DBUS_TYPE_OBJECT_PATH
        {
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        }
        let tp = unsafe { iter_get_str(db, &mut args) }.unwrap_or_default();
        let mut dev_path = String::new();
        let mut cfg = [0u8; 16];
        let mut cfg_len = 0usize;
        let mut delay = 1500u16;
        let mut vol = 127u16;
        let mut has_vol = false;

        if unsafe { (db.message_iter_next)(&mut args) } != 0
            && unsafe { (db.message_iter_get_arg_type)(&mut args) } == DBUS_TYPE_ARRAY
        {
            let mut dict = DBusMessageIter::default();
            unsafe { (db.message_iter_recurse)(&mut args, &mut dict) };
            while unsafe { (db.message_iter_get_arg_type)(&mut dict) } == DBUS_TYPE_DICT_ENTRY {
                let mut entry = DBusMessageIter::default();
                let mut var = DBusMessageIter::default();
                unsafe { (db.message_iter_recurse)(&mut dict, &mut entry) };
                let key = unsafe { iter_get_str(db, &mut entry) }.unwrap_or_default();
                if unsafe { (db.message_iter_next)(&mut entry) } != 0 {
                    unsafe { (db.message_iter_recurse)(&mut entry, &mut var) };
                    match key.as_str() {
                        "Device" => {
                            if let Some(dp) = unsafe { iter_get_str(db, &mut var) } {
                                dev_path = dp;
                            }
                        }
                        "Configuration" => {
                            if let Some(bytes) = unsafe { iter_get_byte_array(db, &mut var) } {
                                cfg_len = bytes.len().min(16);
                                cfg[..cfg_len].copy_from_slice(&bytes[..cfg_len]);
                            }
                        }
                        "Delay" => {
                            if let Some(d) = unsafe { iter_get_u16(db, &mut var) } {
                                delay = d;
                            }
                        }
                        "Volume" => {
                            if let Some(v) = unsafe { iter_get_u16(db, &mut var) } {
                                vol = v;
                                has_vol = true;
                            }
                        }
                        _ => {}
                    }
                }
                if unsafe { (db.message_iter_next)(&mut dict) } == 0 {
                    break;
                }
            }
        }

        let alias = if !dev_path.is_empty() {
            unsafe { dbus_get_str_prop(db, conn, &dev_path, "org.bluez.Device1", "Alias") }
                .unwrap_or_else(|| "Bluetooth A2DP Sink".to_string())
        } else {
            "Bluetooth A2DP Sink".to_string()
        };
        let addr = if !dev_path.is_empty() {
            unsafe { dbus_get_str_prop(db, conn, &dev_path, "org.bluez.Device1", "Address") }
                .unwrap_or_else(|| "00:00:00:00:00:00".to_string())
        } else {
            "00:00:00:00:00:00".to_string()
        };

        let (codec_id, codec_name_log, pref_codec, cur_vol) = {
            let mut st = rt.state.lock().unwrap();
            st.transport_path = tp;
            st.device_path = dev_path;
            st.endpoint_path = path.to_string();
            st.device_alias = alias.clone();
            st.mac_address = addr.clone();
            match path {
                LWASAPI_EP_LDAC => {
                    st.codec_id = A2DP_CODEC_VENDOR;
                    st.active_codec_kind = LWASAPI_BT_CODEC_LDAC;
                    st.supported_codecs_mask |= LWASAPI_BT_CAP_LDAC;
                    st.codec_name = "LDAC".to_string();
                }
                LWASAPI_EP_APTXHD => {
                    st.codec_id = A2DP_CODEC_VENDOR;
                    st.active_codec_kind = LWASAPI_BT_CODEC_APTX_HD;
                    st.supported_codecs_mask |= LWASAPI_BT_CAP_APTX_HD;
                    st.codec_name = "aptX HD".to_string();
                }
                LWASAPI_EP_APTX => {
                    st.codec_id = A2DP_CODEC_VENDOR;
                    st.active_codec_kind = LWASAPI_BT_CODEC_APTX;
                    st.supported_codecs_mask |= LWASAPI_BT_CAP_APTX;
                    st.codec_name = "aptX".to_string();
                }
                LWASAPI_EP_AAC => {
                    st.codec_id = A2DP_CODEC_MPEG24;
                    st.active_codec_kind = LWASAPI_BT_CODEC_AAC;
                    st.supported_codecs_mask |= LWASAPI_BT_CAP_AAC;
                    st.codec_name = "AAC".to_string();
                }
                _ => {
                    st.codec_id = A2DP_CODEC_SBC;
                    st.active_codec_kind = LWASAPI_BT_CODEC_SBC;
                    st.supported_codecs_mask |= LWASAPI_BT_CAP_SBC;
                    st.codec_name = "SBC".to_string();
                }
            }
            st.supported_codecs_mask |= LWASAPI_BT_CAP_SBC;
            let clabel: String = st.codec_name.chars().take(15).collect();
            st.neg_reason = if st.pref_codec != LWASAPI_BT_PREF_AUTO {
                format!("Switched to {clabel} per user codec preference")
            } else if st.active_codec_kind == LWASAPI_BT_CODEC_LDAC {
                "Negotiated LDAC (peer + host SEP match)".to_string()
            } else if st.active_codec_kind == LWASAPI_BT_CODEC_APTX_HD
                || st.active_codec_kind == LWASAPI_BT_CODEC_APTX
            {
                format!("Negotiated {clabel} (headset lacks LDAC SEP)")
            } else if st.active_codec_kind == LWASAPI_BT_CODEC_AAC {
                "AAC active: peer lacked LDAC/aptX or chose AAC".to_string()
            } else {
                "SBC fallback: peer offered baseline A2DP only".to_string()
            };
            rt.bt_renegotiate_fallback.store(false, Ordering::Release);
            st.a2dp_config = cfg;
            st.a2dp_config_len = cfg_len;
            st.has_bluez_transport = true;
            rt.delay_tenths_ms.store(delay, Ordering::Relaxed);
            if has_vol {
                rt.avrcp_volume.store(vol, Ordering::Relaxed);
                rt.desktop_volume_pct
                    .store((u32::from(vol) * 100 + 63) / 127, Ordering::Relaxed);
                rt.avrcp_supported.store(true, Ordering::Relaxed);
            }
            (
                st.codec_id,
                st.codec_name.clone(),
                st.pref_codec,
                if has_vol {
                    vol
                } else {
                    rt.avrcp_volume.load(Ordering::Relaxed)
                },
            )
        };

        dbg_emit_vals(
            LWASAPI_DBG_BLUEZ,
            LWASAPI_DBG_INFO,
            0,
            0,
            i32::from(delay),
            i32::from(if has_vol { vol } else { 0 }),
            i32::from(codec_id),
            "BT_SET_CFG",
            &alias,
            &format!(
                "addr={addr} codec={codec_name_log} ep={path} pref={pref_codec} delay={:.1}ms vol={cur_vol}",
                f64::from(delay) / 10.0
            ),
        );

        unsafe {
            let rep = (db.message_new_method_return)(msg);
            if !rep.is_null() {
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        rt.need_acquire.store(true, Ordering::Release);
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if member == "ClearConfiguration" || member == "Release" {
        let mut it = DBusMessageIter::default();
        let tp = if unsafe { (db.message_iter_init)(msg, &mut it) } != 0 {
            unsafe { iter_get_str(db, &mut it) }
        } else {
            None
        };
        {
            let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
            if member == "Release"
                || tp.as_deref().is_none()
                || tp.as_deref() == Some(st.transport_path.as_str())
            {
                st.close_bt_fd();
                st.direct_socket_mode = false;
                st.has_bluez_transport = false;
                st.tx_stream_active = false;
                st.transport_path.clear();
                st.enc.close_all();
                sync_tx_ready(rt, &st);
            }
        }
        dbg_emit(
            LWASAPI_DBG_BLUEZ,
            LWASAPI_DBG_INFO,
            0,
            0,
            "BT_CLEAR_CFG",
            tp.as_deref().unwrap_or(path),
            &format!("Endpoint {member} by BlueZ"),
        );
        unsafe {
            let rep = (db.message_new_method_return)(msg);
            if !rep.is_null() {
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    DBUS_HANDLER_RESULT_NOT_YET_HANDLED
}

unsafe extern "C" fn bluez_signal_filter(
    _conn: *mut DBusConnection,
    msg: *mut DBusMessage,
    user_data: *mut c_void,
) -> DBusHandlerResult {
    let Some(db) = load_dbus_library() else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    let Some(rt) = (unsafe { bt_from_ptr(user_data) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    if unsafe { (db.message_get_type)(msg) } != DBUS_MESSAGE_TYPE_SIGNAL {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    }
    let (path, Some(iface), Some(member)) = (unsafe { msg_headers(db, msg) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };

    if iface == "org.freedesktop.DBus.Properties" && member == "PropertiesChanged" {
        let matches_tp = {
            let st = rt.state.lock().unwrap();
            !st.transport_path.is_empty() && path == Some(st.transport_path.as_str())
        };
        if matches_tp {
            let mut it = DBusMessageIter::default();
            if unsafe { (db.message_iter_init)(msg, &mut it) } != 0 {
                let prop_iface = unsafe { iter_get_str(db, &mut it) }.unwrap_or_default();
                if prop_iface == "org.bluez.MediaTransport1"
                    && unsafe { (db.message_iter_next)(&mut it) } != 0
                {
                    let mut dict = DBusMessageIter::default();
                    unsafe { (db.message_iter_recurse)(&mut it, &mut dict) };
                    while unsafe { (db.message_iter_get_arg_type)(&mut dict) }
                        == DBUS_TYPE_DICT_ENTRY
                    {
                        let mut entry = DBusMessageIter::default();
                        let mut var = DBusMessageIter::default();
                        unsafe { (db.message_iter_recurse)(&mut dict, &mut entry) };
                        let k = unsafe { iter_get_str(db, &mut entry) }.unwrap_or_default();
                        if unsafe { (db.message_iter_next)(&mut entry) } != 0 {
                            unsafe { (db.message_iter_recurse)(&mut entry, &mut var) };
                            if k == "Volume" {
                                if let Some(v) = unsafe { iter_get_u16(db, &mut var) } {
                                    let pct = (u32::from(v) * 100 + 63) / 127;
                                    rt.avrcp_volume.store(v, Ordering::Relaxed);
                                    rt.desktop_volume_pct.store(pct, Ordering::Relaxed);
                                    rt.avrcp_supported.store(true, Ordering::Relaxed);
                                    let alias = rt.state.lock().unwrap().device_alias.clone();
                                    dbg_emit_vals(
                                        LWASAPI_DBG_BLUEZ,
                                        LWASAPI_DBG_INFO,
                                        0,
                                        0,
                                        i32::from(v),
                                        pct as i32,
                                        0,
                                        "BT_AVRCP_VOL",
                                        &alias,
                                        &format!("AVRCP Volume={v} ({pct}%)"),
                                    );
                                }
                            } else if k == "Delay" {
                                if let Some(d) = unsafe { iter_get_u16(db, &mut var) } {
                                    rt.delay_tenths_ms.store(d, Ordering::Relaxed);
                                    let alias = rt.state.lock().unwrap().device_alias.clone();
                                    dbg_emit_vals(
                                        LWASAPI_DBG_BLUEZ,
                                        LWASAPI_DBG_TRACE,
                                        0,
                                        0,
                                        i32::from(d),
                                        0,
                                        0,
                                        "BT_DELAY_CHG",
                                        &alias,
                                        &format!("Delay={:.1}ms", f64::from(d) / 10.0),
                                    );
                                }
                            }
                        }
                        if unsafe { (db.message_iter_next)(&mut dict) } == 0 {
                            break;
                        }
                    }
                }
            }
        }
    } else if iface == "org.freedesktop.DBus" && member == "NameOwnerChanged" {
        let mut it = DBusMessageIter::default();
        if unsafe { (db.message_iter_init)(msg, &mut it) } != 0 {
            let name = unsafe { iter_get_str(db, &mut it) }.unwrap_or_default();
            let _old = if unsafe { (db.message_iter_next)(&mut it) } != 0 {
                unsafe { iter_get_str(db, &mut it) }.unwrap_or_default()
            } else {
                String::new()
            };
            let new_owner = if unsafe { (db.message_iter_next)(&mut it) } != 0 {
                unsafe { iter_get_str(db, &mut it) }.unwrap_or_default()
            } else {
                String::new()
            };
            if name == "org.bluez" {
                {
                    let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
                    st.close_bt_fd();
                    st.direct_socket_mode = false;
                    st.has_bluez_transport = false;
                    st.tx_stream_active = false;
                    st.transport_path.clear();
                    st.adapter_path.clear();
                    st.endpoints_registered = false;
                    st.enc.close_all();
                    sync_tx_ready(rt, &st);
                }
                if !new_owner.is_empty() {
                    rt.need_reregister.store(true, Ordering::Release);
                    dbg_emit(
                        LWASAPI_DBG_BLUEZ,
                        LWASAPI_DBG_INFO,
                        0,
                        0,
                        "BT_BLUEZ_UP",
                        &new_owner,
                        "org.bluez acquired/restarted; scheduling MediaEndpoint1 re-registration",
                    );
                } else {
                    rt.need_reregister.store(false, Ordering::Release);
                    dbg_emit(
                        LWASAPI_DBG_BLUEZ,
                        LWASAPI_DBG_WARN,
                        0,
                        0,
                        "BT_BLUEZ_DOWN",
                        "org.bluez",
                        "org.bluez exited; waiting for bluetoothd restart",
                    );
                }
            }
        }
    } else if iface == "org.freedesktop.DBus.ObjectManager" && member == "InterfacesAdded" {
        let mut it = DBusMessageIter::default();
        if unsafe { (db.message_iter_init)(msg, &mut it) } != 0 {
            let obj = unsafe { iter_get_str(db, &mut it) }.unwrap_or_default();
            if unsafe { (db.message_iter_next)(&mut it) } != 0
                && unsafe { (db.message_iter_get_arg_type)(&mut it) } == DBUS_TYPE_ARRAY
            {
                let mut if_arr = DBusMessageIter::default();
                unsafe { (db.message_iter_recurse)(&mut it, &mut if_arr) };
                while unsafe { (db.message_iter_get_arg_type)(&mut if_arr) } == DBUS_TYPE_DICT_ENTRY
                {
                    let mut if_entry = DBusMessageIter::default();
                    unsafe { (db.message_iter_recurse)(&mut if_arr, &mut if_entry) };
                    let added_iface = unsafe { iter_get_str(db, &mut if_entry) }.unwrap_or_default();
                    if added_iface == "org.bluez.Media1" {
                        let need = {
                            let st = rt.state.lock().unwrap();
                            !st.endpoints_registered || (!obj.is_empty() && obj != st.adapter_path)
                        };
                        if need {
                            rt.need_reregister.store(true, Ordering::Release);
                            dbg_emit(
                                LWASAPI_DBG_BLUEZ,
                                LWASAPI_DBG_INFO,
                                0,
                                0,
                                "BT_MEDIA_ADD",
                                if obj.is_empty() { "/org/bluez" } else { &obj },
                                "org.bluez.Media1 appeared; scheduling endpoint registration",
                            );
                        }
                        break;
                    }
                    if unsafe { (db.message_iter_next)(&mut if_arr) } == 0 {
                        break;
                    }
                }
            }
        }
    } else if iface == "org.freedesktop.DBus.ObjectManager" && member == "InterfacesRemoved" {
        let mut it = DBusMessageIter::default();
        if unsafe { (db.message_iter_init)(msg, &mut it) } != 0 {
            let obj = unsafe { iter_get_str(db, &mut it) }.unwrap_or_default();
            let mut removed = false;
            {
                let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
                if !obj.is_empty() && (obj == st.transport_path || obj == st.device_path) {
                    st.close_bt_fd();
                    st.direct_socket_mode = false;
                    st.has_bluez_transport = false;
                    st.tx_stream_active = false;
                    st.transport_path.clear();
                    st.enc.close_all();
                    sync_tx_ready(rt, &st);
                    removed = true;
                }
            }
            if removed {
                dbg_emit(
                    LWASAPI_DBG_BLUEZ,
                    LWASAPI_DBG_WARN,
                    0,
                    0,
                    "BT_IFACE_REM",
                    &obj,
                    "BlueZ ObjectManager removed device/transport",
                );
            }
        }
    }

    DBUS_HANDLER_RESULT_NOT_YET_HANDLED
}

// ============================================================================
// HFP AG (/org/lwasapi/HFP/AG) + BlueZ BatteryProvider1 (/org/lwasapi/Battery)
// ============================================================================

fn make_batt_path(dev_path: &str) -> String {
    let mut out = [0u8; 160];
    if let Some(len) = hfp_make_batt_path(dev_path.as_bytes(), &mut out) {
        String::from_utf8_lossy(&out[..len]).into_owned()
    } else {
        "/org/lwasapi/Battery/dev".to_string()
    }
}

fn hfp_find_or_alloc_locked<'a>(
    devs: &'a mut [HfpBatteryDev; MAX_HFP_DEVS],
    dev_path: &str,
) -> Option<&'a mut HfpBatteryDev> {
    if let Some(pos) = devs.iter().position(|d| d.in_use && d.dev_path == dev_path) {
        return Some(&mut devs[pos]);
    }
    let free = devs.iter().position(|d| !d.in_use)?;
    let d = &mut devs[free];
    *d = HfpBatteryDev {
        in_use: true,
        fd: -1,
        pct: -1,
        dev_path: dev_path.to_string(),
        batt_path: make_batt_path(dev_path),
        ..HfpBatteryDev::default()
    };
    Some(d)
}

fn hfp_set_battery_pct(rt: &BluezRuntime, dev_path: &str, raw_pct: i32) {
    let pct = raw_pct.clamp(0, 100);
    {
        let mut devs = rt.hfp_devs.lock().unwrap();
        if let Some(d) = hfp_find_or_alloc_locked(&mut devs, dev_path) {
            if !d.published {
                d.pct = pct;
                d.pending_add = true;
                d.pending_rem = false;
                rt.hfp_battery_dirty.store(true, Ordering::Release);
            } else if d.pct != pct {
                d.pct = pct;
                d.pending_chg = true;
                rt.hfp_battery_dirty.store(true, Ordering::Release);
            }
        }
    }
    eprintln!("[LWASAPI-HFP] Battery update for {dev_path}: {pct}%");
}

fn hfp_mark_disconnected(rt: &BluezRuntime, dev_path: &str, expected_fd: Option<c_int>) {
    let mut devs = rt.hfp_devs.lock().unwrap_or_else(|e| e.into_inner());
    for d in devs.iter_mut() {
        if d.in_use && d.dev_path == dev_path {
            if let Some(efd) = expected_fd {
                if d.fd != efd {
                    continue;
                }
            }
            if d.fd >= 0 {
                shutdown_rdwr(d.fd);
                d.fd_owner = None;
                d.fd = -1;
            }
            d.rfcomm_active = false;
            if d.published {
                d.pending_rem = true;
                d.pending_add = false;
                d.pending_chg = false;
                rt.hfp_battery_dirty.store(true, Ordering::Release);
            } else {
                d.in_use = false;
            }
        }
    }
}

fn hfp_is_connected(rt: &BluezRuntime, dev_path: &str) -> bool {
    let devs = rt.hfp_devs.lock().unwrap();
    devs.iter()
        .any(|d| d.in_use && d.dev_path == dev_path && d.rfcomm_active && d.fd >= 0)
}

unsafe fn append_battery_props_dict(
    db: &DBusApi,
    dict: *mut DBusMessageIter,
    dev_path: &str,
    pct: u8,
) {
    unsafe {
        add_dict_entry_obj_path(db, dict, "Device", dev_path);
        add_dict_entry_byte(db, dict, "Percentage", pct);
        add_dict_entry_str(db, dict, "Source", "HFP");
    }
}

fn hfp_flush_battery_dbus(db: &DBusApi, conn: *mut DBusConnection, rt: &BluezRuntime) {
    if conn.is_null() || !rt.hfp_battery_dirty.swap(false, Ordering::AcqRel) {
        return;
    }
    let mut devs = rt.hfp_devs.lock().unwrap();
    for d in devs.iter_mut() {
        if !d.in_use {
            continue;
        }
        if d.pending_add && d.pct >= 0 {
            d.pending_add = false;
            d.pending_chg = false;
            d.published = true;
            unsafe {
                let sig = (db.message_new_signal)(
                    c"/org/lwasapi/Battery".as_ptr(),
                    c"org.freedesktop.DBus.ObjectManager".as_ptr(),
                    c"InterfacesAdded".as_ptr(),
                );
                if !sig.is_null() {
                    let mut it = DBusMessageIter::default();
                    let mut if_arr = DBusMessageIter::default();
                    let mut if_ent = DBusMessageIter::default();
                    let mut props = DBusMessageIter::default();
                    (db.message_iter_init_append)(sig, &mut it);
                    let _ = iter_append_str(db, &mut it, DBUS_TYPE_OBJECT_PATH, &d.batt_path);
                    if (db.message_iter_open_container)(
                        &mut it,
                        DBUS_TYPE_ARRAY,
                        c"{sa{sv}}".as_ptr(),
                        &mut if_arr,
                    ) != 0
                    {
                        if (db.message_iter_open_container)(
                            &mut if_arr,
                            DBUS_TYPE_DICT_ENTRY,
                            ptr::null(),
                            &mut if_ent,
                        ) != 0
                        {
                            let _ = iter_append_str(
                                db,
                                &mut if_ent,
                                DBUS_TYPE_STRING,
                                "org.bluez.BatteryProvider1",
                            );
                            if (db.message_iter_open_container)(
                                &mut if_ent,
                                DBUS_TYPE_ARRAY,
                                c"{sv}".as_ptr(),
                                &mut props,
                            ) != 0
                            {
                                append_battery_props_dict(db, &mut props, &d.dev_path, d.pct as u8);
                                let _ = (db.message_iter_close_container)(&mut if_ent, &mut props);
                            }
                            let _ = (db.message_iter_close_container)(&mut if_arr, &mut if_ent);
                        }
                        let _ = (db.message_iter_close_container)(&mut it, &mut if_arr);
                    }
                    let _ = (db.connection_send)(conn, sig, ptr::null_mut());
                    (db.message_unref)(sig);
                }
            }
        } else if d.pending_chg && d.published && d.pct >= 0 {
            d.pending_chg = false;
            let Ok(cbpath) = CString::new(d.batt_path.as_str()) else {
                continue;
            };
            unsafe {
                let sig = (db.message_new_signal)(
                    cbpath.as_ptr(),
                    c"org.freedesktop.DBus.Properties".as_ptr(),
                    c"PropertiesChanged".as_ptr(),
                );
                if !sig.is_null() {
                    let mut it = DBusMessageIter::default();
                    let mut props = DBusMessageIter::default();
                    let mut inv = DBusMessageIter::default();
                    (db.message_iter_init_append)(sig, &mut it);
                    let _ = iter_append_str(
                        db,
                        &mut it,
                        DBUS_TYPE_STRING,
                        "org.bluez.BatteryProvider1",
                    );
                    if (db.message_iter_open_container)(
                        &mut it,
                        DBUS_TYPE_ARRAY,
                        c"{sv}".as_ptr(),
                        &mut props,
                    ) != 0
                    {
                        add_dict_entry_byte(db, &mut props, "Percentage", d.pct as u8);
                        let _ = (db.message_iter_close_container)(&mut it, &mut props);
                    }
                    if (db.message_iter_open_container)(
                        &mut it,
                        DBUS_TYPE_ARRAY,
                        c"s".as_ptr(),
                        &mut inv,
                    ) != 0
                    {
                        let _ = (db.message_iter_close_container)(&mut it, &mut inv);
                    }
                    let _ = (db.connection_send)(conn, sig, ptr::null_mut());
                    (db.message_unref)(sig);
                }
            }
        } else if d.pending_rem {
            d.pending_rem = false;
            if d.published {
                d.published = false;
                unsafe {
                    let sig = (db.message_new_signal)(
                        c"/org/lwasapi/Battery".as_ptr(),
                        c"org.freedesktop.DBus.ObjectManager".as_ptr(),
                        c"InterfacesRemoved".as_ptr(),
                    );
                    if !sig.is_null() {
                        let mut it = DBusMessageIter::default();
                        let mut arr = DBusMessageIter::default();
                        (db.message_iter_init_append)(sig, &mut it);
                        let _ = iter_append_str(db, &mut it, DBUS_TYPE_OBJECT_PATH, &d.batt_path);
                        if (db.message_iter_open_container)(
                            &mut it,
                            DBUS_TYPE_ARRAY,
                            c"s".as_ptr(),
                            &mut arr,
                        ) != 0
                        {
                            let _ = iter_append_str(
                                db,
                                &mut arr,
                                DBUS_TYPE_STRING,
                                "org.bluez.BatteryProvider1",
                            );
                            let _ = (db.message_iter_close_container)(&mut it, &mut arr);
                        }
                        let _ = (db.connection_send)(conn, sig, ptr::null_mut());
                        (db.message_unref)(sig);
                    }
                }
            }
            if !d.rfcomm_active {
                d.in_use = false;
            }
        }
    }
}

unsafe extern "C" fn hfp_battery_message_handler(
    conn: *mut DBusConnection,
    msg: *mut DBusMessage,
    user_data: *mut c_void,
) -> DBusHandlerResult {
    let Some(db) = load_dbus_library() else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    let Some(rt) = (unsafe { bt_from_ptr(user_data) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    let (Some(path), iface, Some(member)) = (unsafe { msg_headers(db, msg) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };

    if path == LWASAPI_BATTERY_ROOT
        && iface == Some("org.freedesktop.DBus.ObjectManager")
        && member == "GetManagedObjects"
    {
        unsafe {
            let rep = (db.message_new_method_return)(msg);
            if !rep.is_null() {
                let mut it = DBusMessageIter::default();
                let mut obj_arr = DBusMessageIter::default();
                (db.message_iter_init_append)(rep, &mut it);
                if (db.message_iter_open_container)(
                    &mut it,
                    DBUS_TYPE_ARRAY,
                    c"{oa{sa{sv}}}".as_ptr(),
                    &mut obj_arr,
                ) != 0
                {
                    let mut devs = rt.hfp_devs.lock().unwrap();
                    for d in devs.iter_mut() {
                        if !d.in_use || d.pct < 0 || d.pending_rem {
                            continue;
                        }
                        d.published = true;
                        d.pending_add = false;
                        let mut obj_ent = DBusMessageIter::default();
                        let mut if_arr = DBusMessageIter::default();
                        let mut if_ent = DBusMessageIter::default();
                        let mut props = DBusMessageIter::default();
                        if (db.message_iter_open_container)(
                            &mut obj_arr,
                            DBUS_TYPE_DICT_ENTRY,
                            ptr::null(),
                            &mut obj_ent,
                        ) != 0
                        {
                            let _ = iter_append_str(
                                db,
                                &mut obj_ent,
                                DBUS_TYPE_OBJECT_PATH,
                                &d.batt_path,
                            );
                            if (db.message_iter_open_container)(
                                &mut obj_ent,
                                DBUS_TYPE_ARRAY,
                                c"{sa{sv}}".as_ptr(),
                                &mut if_arr,
                            ) != 0
                            {
                                if (db.message_iter_open_container)(
                                    &mut if_arr,
                                    DBUS_TYPE_DICT_ENTRY,
                                    ptr::null(),
                                    &mut if_ent,
                                ) != 0
                                {
                                    let _ = iter_append_str(
                                        db,
                                        &mut if_ent,
                                        DBUS_TYPE_STRING,
                                        "org.bluez.BatteryProvider1",
                                    );
                                    if (db.message_iter_open_container)(
                                        &mut if_ent,
                                        DBUS_TYPE_ARRAY,
                                        c"{sv}".as_ptr(),
                                        &mut props,
                                    ) != 0
                                    {
                                        append_battery_props_dict(
                                            db,
                                            &mut props,
                                            &d.dev_path,
                                            d.pct as u8,
                                        );
                                        let _ = (db.message_iter_close_container)(
                                            &mut if_ent,
                                            &mut props,
                                        );
                                    }
                                    let _ = (db.message_iter_close_container)(
                                        &mut if_arr,
                                        &mut if_ent,
                                    );
                                }
                                let _ =
                                    (db.message_iter_close_container)(&mut obj_ent, &mut if_arr);
                            }
                            let _ = (db.message_iter_close_container)(&mut obj_arr, &mut obj_ent);
                        }
                    }
                    let _ = (db.message_iter_close_container)(&mut it, &mut obj_arr);
                }
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if iface == Some("org.freedesktop.DBus.Properties") && member == "GetAll" {
        let mut req_it = DBusMessageIter::default();
        let req_iface = if unsafe { (db.message_iter_init)(msg, &mut req_it) } != 0 {
            unsafe { iter_get_str(db, &mut req_it) }
        } else {
            None
        };
        unsafe {
            let rep = (db.message_new_method_return)(msg);
            if !rep.is_null() {
                let mut it = DBusMessageIter::default();
                let mut props = DBusMessageIter::default();
                (db.message_iter_init_append)(rep, &mut it);
                if (db.message_iter_open_container)(
                    &mut it,
                    DBUS_TYPE_ARRAY,
                    c"{sv}".as_ptr(),
                    &mut props,
                ) != 0
                {
                    if req_iface.as_deref() == Some("org.bluez.BatteryProvider1") {
                        let devs = rt.hfp_devs.lock().unwrap();
                        if let Some(d) = devs
                            .iter()
                            .find(|d| d.in_use && d.batt_path == path && d.pct >= 0)
                        {
                            append_battery_props_dict(db, &mut props, &d.dev_path, d.pct as u8);
                        }
                    }
                    let _ = (db.message_iter_close_container)(&mut it, &mut props);
                }
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    DBUS_HANDLER_RESULT_NOT_YET_HANDLED
}

fn hfp_write_bytes(fd: c_int, bytes: &[u8]) {
    let mut off = 0usize;
    while off < bytes.len() {
        let w = crate::unix_sys::write_raw(fd, &bytes[off..]);
        if w <= 0 {
            break;
        }
        off += w as usize;
    }
}

fn hfp_handle_at_line(rt: &BluezRuntime, fd: c_int, dev_path: &str, raw_line: &[u8]) {
    let s = String::from_utf8_lossy(raw_line);
    let trimmed = s.trim_start_matches([' ', '\t']);
    if trimmed.is_empty() {
        return;
    }
    eprintln!("[LWASAPI-HFP] AT << {trimmed}");
    let act = parse_hfp_at_line(trimmed.as_bytes());
    if !act.handled {
        return;
    }
    if act.has_msbc_update || act.sco_codec != 0 {
        let mut devs = rt.hfp_devs.lock().unwrap();
        if let Some(d) = hfp_find_or_alloc_locked(&mut devs, dev_path) {
            if act.has_msbc_update {
                d.supports_msbc = act.supports_msbc;
            }
            if act.sco_codec != 0 {
                d._negotiated_sco_codec = act.sco_codec;
            }
        }
    }
    if act.battery_pct >= 0 {
        hfp_set_battery_pct(rt, dev_path, act.battery_pct);
    }
    if act.reply_len > 0 {
        hfp_write_bytes(fd, &act.reply[..act.reply_len]);
    }
}

unsafe extern "C" fn hfp_profile_message_handler(
    conn: *mut DBusConnection,
    msg: *mut DBusMessage,
    user_data: *mut c_void,
) -> DBusHandlerResult {
    let Some(db) = load_dbus_library() else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    let Some(rt) = (unsafe { bt_from_ptr(user_data) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    let (_path, Some(iface), Some(member)) = (unsafe { msg_headers(db, msg) }) else {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    };
    if iface != "org.bluez.Profile1" {
        return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
    }

    if member == "Release" {
        unsafe {
            let rep = (db.message_new_method_return)(msg);
            if !rep.is_null() {
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if member == "RequestDisconnection" {
        let mut it = DBusMessageIter::default();
        if unsafe { (db.message_iter_init)(msg, &mut it) } != 0 {
            if let Some(dp) = unsafe { iter_get_str(db, &mut it) } {
                hfp_mark_disconnected(rt, &dp, None);
            }
        }
        unsafe {
            let rep = (db.message_new_method_return)(msg);
            if !rep.is_null() {
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    if member == "NewConnection" {
        let mut it = DBusMessageIter::default();
        if unsafe { (db.message_iter_init)(msg, &mut it) } != 0 {
            let dev_path = unsafe { iter_get_str(db, &mut it) }.unwrap_or_default();
            let fd = if unsafe { (db.message_iter_next)(&mut it) } != 0 {
                unsafe { iter_get_unix_fd(db, &mut it) }.unwrap_or(-1)
            } else {
                -1
            };
            if !dev_path.is_empty() && fd >= 0 {
                hfp_mark_disconnected(rt, &dev_path, None);
                let fd_owner = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
                let mut allocated = false;
                {
                    let mut devs = rt.hfp_devs.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(d) = hfp_find_or_alloc_locked(&mut devs, &dev_path) {
                        d.fd = fd;
                        d.fd_owner = Some(Arc::clone(&fd_owner));
                        d.rfcomm_active = true;
                        allocated = true;
                    }
                }
                if !allocated {
                    drop(fd_owner);
                } else {
                    let rt_arc = Arc::clone(global_bt_arc());
                    let dp = dev_path;
                    thread::spawn(move || {
                        let _fd_keepalive = fd_owner;
                        let fd = _fd_keepalive.as_raw_fd();
                        eprintln!("[LWASAPI-HFP] RFCOMM connected for {dp} (fd={fd})");
                        let mut line = Vec::<u8>::with_capacity(512);
                        let mut buf = [0u8; 256];
                        loop {
                            let mut pfd = [PollFd {
                                fd,
                                events: POLLIN,
                                revents: 0,
                            }];
                            let pr = do_poll(&mut pfd, 1000);
                            if pr < 0 {
                                if last_errno() == ERRNO_EINTR {
                                    continue;
                                }
                                break;
                            }
                            if pr == 0 {
                                continue;
                            }
                            if (pfd[0].revents & (POLLERR | POLLHUP | POLLNVAL)) != 0
                                && (pfd[0].revents & POLLIN) == 0
                            {
                                break;
                            }
                            let n = crate::unix_sys::read_raw(fd, &mut buf);
                            if n < 0 {
                                let e = last_errno();
                                if e == ERRNO_EAGAIN || e == ERRNO_EINTR {
                                    continue;
                                }
                                eprintln!(
                                    "[LWASAPI-HFP] RFCOMM read error on {dp}: {}",
                                    std::io::Error::from_raw_os_error(e)
                                );
                                break;
                            }
                            if n == 0 {
                                break;
                            }
                            for &b in &buf[..n as usize] {
                                if b == b'\r' || b == b'\n' {
                                    if !line.is_empty() {
                                        hfp_handle_at_line(&rt_arc, fd, &dp, &line);
                                        line.clear();
                                    }
                                } else if line.len() + 1 < 512 {
                                    line.push(b);
                                }
                            }
                        }
                        eprintln!("[LWASAPI-HFP] RFCOMM disconnected for {dp}");
                        hfp_mark_disconnected(&rt_arc, &dp, Some(fd));
                    });
                }
            } else if fd >= 0 {
                close_raw_fd(fd);
            }
        }
        unsafe {
            let rep = (db.message_new_method_return)(msg);
            if !rep.is_null() {
                let _ = (db.connection_send)(conn, rep, ptr::null_mut());
                (db.message_unref)(rep);
            }
        }
        return DBUS_HANDLER_RESULT_HANDLED;
    }

    DBUS_HANDLER_RESULT_NOT_YET_HANDLED
}

fn register_hfp_and_battery_provider(
    db: &DBusApi,
    conn: *mut DBusConnection,
    adapters: &[String],
    rt_ptr: *mut c_void,
    hfp_reg: &mut bool,
    batt_reg: &mut bool,
) {
    let hfp_vtbl = DBusObjectPathVTable {
        message_function: Some(hfp_profile_message_handler),
        ..DBusObjectPathVTable::default()
    };
    let batt_vtbl = DBusObjectPathVTable {
        message_function: Some(hfp_battery_message_handler),
        ..DBusObjectPathVTable::default()
    };
    unsafe {
        if !*hfp_reg
            && (db.connection_register_object_path)(
                conn,
                c"/org/lwasapi/HFP/AG".as_ptr(),
                &hfp_vtbl,
                rt_ptr,
            ) != 0
        {
            *hfp_reg = true;
        }
        if !*batt_reg
            && (db.connection_register_fallback)(
                conn,
                c"/org/lwasapi/Battery".as_ptr(),
                &batt_vtbl,
                rt_ptr,
            ) != 0
        {
            *batt_reg = true;
        }
    }

    let pmsg = dbus_new_method_call(
        db,
        "org.bluez",
        "/org/bluez",
        "org.bluez.ProfileManager1",
        "RegisterProfile",
    );
    if !pmsg.is_null() {
        unsafe {
            let mut it = DBusMessageIter::default();
            let mut dict = DBusMessageIter::default();
            (db.message_iter_init_append)(pmsg, &mut it);
            let _ = iter_append_str(db, &mut it, DBUS_TYPE_OBJECT_PATH, LWASAPI_HFP_AG_PATH);
            let _ = iter_append_str(db, &mut it, DBUS_TYPE_STRING, HFP_AG_UUID);
            if (db.message_iter_open_container)(&mut it, DBUS_TYPE_ARRAY, c"{sv}".as_ptr(), &mut dict)
                != 0
            {
                add_dict_entry_str(db, &mut dict, "Name", "LWASAPI HFP Battery");
                add_dict_entry_u16(db, &mut dict, "Version", 0x0107);
                add_dict_entry_u16(db, &mut dict, "Features", 0x0009);
                add_dict_entry_bool(db, &mut dict, "AutoConnect", true);
                let _ = (db.message_iter_close_container)(&mut it, &mut dict);
            }
            let mut err = DBusError::default();
            (db.error_init)(&mut err);
            let rep =
                (db.connection_send_with_reply_and_block)(conn, pmsg, 2000, &mut err);
            (db.message_unref)(pmsg);
            if !rep.is_null() {
                (db.message_unref)(rep);
                println!("[LWASAPI-HFP] Registered HFP AG profile {LWASAPI_HFP_AG_PATH}");
            } else {
                let emsg = dbus_error_msg(&err);
                eprintln!("[LWASAPI-HFP] RegisterProfile failed: {emsg}");
                (db.error_free)(&mut err);
            }
        }
    }

    for adapter in adapters {
        let bmsg = dbus_new_method_call(
            db,
            "org.bluez",
            adapter,
            "org.bluez.BatteryProviderManager1",
            "RegisterBatteryProvider",
        );
        if bmsg.is_null() {
            continue;
        }
        unsafe {
            let mut it = DBusMessageIter::default();
            (db.message_iter_init_append)(bmsg, &mut it);
            let _ = iter_append_str(db, &mut it, DBUS_TYPE_OBJECT_PATH, LWASAPI_BATTERY_ROOT);
            let mut err = DBusError::default();
            (db.error_init)(&mut err);
            let rep = (db.connection_send_with_reply_and_block)(conn, bmsg, 2000, &mut err);
            (db.message_unref)(bmsg);
            if !rep.is_null() {
                (db.message_unref)(rep);
                println!("[LWASAPI-HFP] Registered BatteryProvider {LWASAPI_BATTERY_ROOT} on {adapter}");
            } else {
                let emsg = dbus_error_msg(&err);
                eprintln!("[LWASAPI-HFP] RegisterBatteryProvider on {adapter} failed: {emsg}");
                (db.error_free)(&mut err);
            }
        }
    }
}

fn register_endpoint_on_adapter(
    db: &DBusApi,
    conn: *mut DBusConnection,
    adapter: &str,
    ep_path: &str,
    codec: u8,
    caps: &[u8],
) {
    let msg = dbus_new_method_call(db, "org.bluez", adapter, "org.bluez.Media1", "RegisterEndpoint");
    if msg.is_null() {
        return;
    }
    let cname = match ep_path {
        LWASAPI_EP_LDAC => "LDAC",
        LWASAPI_EP_APTXHD => "aptX HD",
        LWASAPI_EP_APTX => "aptX",
        _ if codec == A2DP_CODEC_MPEG24 => "AAC",
        _ => "SBC",
    };
    unsafe {
        let mut it = DBusMessageIter::default();
        let mut dict = DBusMessageIter::default();
        (db.message_iter_init_append)(msg, &mut it);
        let _ = iter_append_str(db, &mut it, DBUS_TYPE_OBJECT_PATH, ep_path);
        if (db.message_iter_open_container)(&mut it, DBUS_TYPE_ARRAY, c"{sv}".as_ptr(), &mut dict)
            != 0
        {
            add_dict_entry_str(db, &mut dict, "UUID", A2DP_SOURCE_UUID);
            add_dict_entry_byte(db, &mut dict, "Codec", codec);
            add_dict_entry_bool(db, &mut dict, "DelayReporting", true);
            add_dict_entry_bytes(db, &mut dict, "Capabilities", caps);
            let _ = (db.message_iter_close_container)(&mut it, &mut dict);
        }
        let mut err = DBusError::default();
        (db.error_init)(&mut err);
        let rep = (db.connection_send_with_reply_and_block)(conn, msg, 2000, &mut err);
        (db.message_unref)(msg);
        if !rep.is_null() {
            (db.message_unref)(rep);
            println!("[LWASAPI-BlueZ] Registered A2DP {cname} endpoint {ep_path} on {adapter}");
            dbg_emit(
                LWASAPI_DBG_BLUEZ,
                LWASAPI_DBG_INFO,
                0,
                0,
                "BT_EP_REG",
                adapter,
                &format!("Registered {ep_path} ({cname})"),
            );
        } else {
            let emsg = dbus_error_msg(&err);
            dbg_emit(
                LWASAPI_DBG_BLUEZ,
                LWASAPI_DBG_WARN,
                0,
                0,
                "BT_EP_ERR",
                adapter,
                &format!("RegisterEndpoint {ep_path} failed: {emsg}"),
            );
            (db.error_free)(&mut err);
        }
    }
}

fn write_vendor_hdr(out: &mut [u8], vid: u32, cid: u16) {
    out[0..4].copy_from_slice(&vid.to_le_bytes());
    out[4..6].copy_from_slice(&cid.to_le_bytes());
}

fn register_all_adapters_and_kick_connected(
    db: &DBusApi,
    conn: *mut DBusConnection,
    rt: &BluezRuntime,
    ep_reg: &mut [bool; 5],
    hfp_reg: &mut bool,
    batt_reg: &mut bool,
) {
    let pref_mac = {
        let mut st = rt.state.lock().unwrap();
        st.bt_kick_dev.clear();
        st.bt_startup_autoconnect_dev.clear();
        st.preferred_mac.clone()
    };
    let Some(objs) = fetch_managed_objects(db, conn, 2000) else {
        return;
    };

    let adapters: Vec<String> = objs.adapters.into_iter().take(4).collect();
    let mut kick_devices = Vec::<String>::new();
    let mut hfp_devices = Vec::<String>::new();
    let mut best_kick = String::new();
    let mut kick_matches_pref = false;
    let mut startup_auto = String::new();

    for d in &objs.devices {
        let matches_pref = !pref_mac.is_empty() && d.address.eq_ignore_ascii_case(&pref_mac);
        if d.connected && d.has_hfp_hf && hfp_devices.len() < 4 {
            hfp_devices.push(d.obj_path.clone());
        }
        if d.connected && d.has_a2dp_sink && kick_devices.len() < 4 {
            kick_devices.push(d.obj_path.clone());
            if best_kick.is_empty() || (matches_pref && !kick_matches_pref) {
                best_kick.clone_from(&d.obj_path);
                kick_matches_pref = matches_pref;
            }
        } else if !d.connected
            && d.has_a2dp_sink
            && d.paired
            && d.trusted
            && matches_pref
            && startup_auto.is_empty()
        {
            startup_auto.clone_from(&d.obj_path);
        }
    }

    {
        let mut st = rt.state.lock().unwrap();
        st.bt_kick_dev = best_kick;
        st.bt_startup_autoconnect_dev = startup_auto;
    }

    let ldac_ok = load_ldac_library().is_some();
    let aptx_ok = load_aptx_library().is_some();
    let aac_ok = load_fdk_aac_library().is_some();

    let mut ldac_caps = [0u8; 8];
    write_vendor_hdr(&mut ldac_caps, LDAC_VENDOR_ID, LDAC_CODEC_ID);
    ldac_caps[6] = LDAC_SAMPLING_FREQ_44100
        | LDAC_SAMPLING_FREQ_48000
        | LDAC_SAMPLING_FREQ_88200
        | LDAC_SAMPLING_FREQ_96000;
    ldac_caps[7] = LDAC_CHANNEL_MODE_MONO | LDAC_CHANNEL_MODE_DUAL | LDAC_CHANNEL_MODE_STEREO;

    let mut aptx_hd_caps = [0u8; 11];
    write_vendor_hdr(&mut aptx_hd_caps, APTX_HD_VENDOR_ID, APTX_HD_CODEC_ID);
    aptx_hd_caps[6] = ((APTX_SAMPLING_FREQ_16000
        | APTX_SAMPLING_FREQ_32000
        | APTX_SAMPLING_FREQ_44100
        | APTX_SAMPLING_FREQ_48000)
        << 4)
        | (APTX_CHANNEL_MODE_MONO | APTX_CHANNEL_MODE_STEREO);

    let mut aptx_caps = [0u8; 7];
    write_vendor_hdr(&mut aptx_caps, APTX_VENDOR_ID, APTX_CODEC_ID);
    aptx_caps[6] = aptx_hd_caps[6];

    let aac_caps = [0xC0u8, 0x01, 0x8C, 0x83, 0xE8, 0x00];
    let sbc_caps = [0x3Fu8, 0xFF, 0x02, 0x53];

    let rt_ptr = (rt as *const BluezRuntime as *mut BluezRuntime).cast::<c_void>();
    let vtbl = DBusObjectPathVTable {
        message_function: Some(endpoint_message_handler),
        ..DBusObjectPathVTable::default()
    };

    let ep_defs: [(&str, bool, &CStr); 5] = [
        (LWASAPI_EP_LDAC, ldac_ok, c"/org/lwasapi/A2DP/LDAC/Source"),
        (
            LWASAPI_EP_APTXHD,
            aptx_ok,
            c"/org/lwasapi/A2DP/APTXHD/Source",
        ),
        (LWASAPI_EP_APTX, aptx_ok, c"/org/lwasapi/A2DP/APTX/Source"),
        (LWASAPI_EP_AAC, aac_ok, c"/org/lwasapi/A2DP/AAC/Source"),
        (LWASAPI_EP_SBC, true, c"/org/lwasapi/A2DP/SBC/Source"),
    ];
    for (idx, &(_, enabled, cpath)) in ep_defs.iter().enumerate() {
        if enabled
            && !ep_reg[idx]
            && unsafe { (db.connection_register_object_path)(conn, cpath.as_ptr(), &vtbl, rt_ptr) }
                != 0
        {
            ep_reg[idx] = true;
        }
    }

    for (i, adapter) in adapters.iter().enumerate() {
        if i == 0 {
            rt.state.lock().unwrap().adapter_path = adapter.chars().take(63).collect();
        }
        if ldac_ok {
            register_endpoint_on_adapter(
                db,
                conn,
                adapter,
                LWASAPI_EP_LDAC,
                A2DP_CODEC_VENDOR,
                &ldac_caps,
            );
        }
        if aptx_ok {
            register_endpoint_on_adapter(
                db,
                conn,
                adapter,
                LWASAPI_EP_APTXHD,
                A2DP_CODEC_VENDOR,
                &aptx_hd_caps,
            );
            register_endpoint_on_adapter(
                db,
                conn,
                adapter,
                LWASAPI_EP_APTX,
                A2DP_CODEC_VENDOR,
                &aptx_caps,
            );
        }
        if aac_ok {
            register_endpoint_on_adapter(
                db,
                conn,
                adapter,
                LWASAPI_EP_AAC,
                A2DP_CODEC_MPEG24,
                &aac_caps,
            );
        }
        register_endpoint_on_adapter(db, conn, adapter, LWASAPI_EP_SBC, A2DP_CODEC_SBC, &sbc_caps);
    }

    if !adapters.is_empty() {
        register_hfp_and_battery_provider(db, conn, &adapters, rt_ptr, hfp_reg, batt_reg);
    }
    rt.state.lock().unwrap().endpoints_registered = !adapters.is_empty();

    for _ in 0..10 {
        unsafe { (db.connection_read_write_dispatch)(conn, 20) };
    }
    for hdev in &hfp_devices {
        if !hfp_is_connected(rt, hdev) {
            send_connect_profile(db, conn, hdev, HFP_HF_UUID);
        }
    }
    let has_tp = rt.state.lock().unwrap().has_bluez_transport;
    if !has_tp {
        for kdev in &kick_devices {
            unsafe {
                dbus_set_str_prop(db, conn, kdev, "org.bluez.Device1", "PreferredBearer", "bredr");
            }
            send_connect_profile(db, conn, kdev, A2DP_SINK_UUID);
        }
    }
}

fn probe_remote_seps(
    db: &DBusApi,
    conn: *mut DBusConnection,
    rt: &BluezRuntime,
    want_cap: u32,
) -> Option<(String, Vec<u8>)> {
    let objs = fetch_managed_objects(db, conn, 1500)?;
    let target_dev = rt.state.lock().unwrap().device_path.clone();

    let mut mask = 0u32;
    let mut matched: Option<(String, Vec<u8>)> = None;

    for sep in &objs.seps {
        if !target_dev.is_empty() && !sep.obj_path.starts_with(&target_dev) {
            continue;
        }
        if !sep.is_sink_sep {
            continue;
        }
        let sep_cap = if sep.codec == A2DP_CODEC_SBC {
            LWASAPI_BT_CAP_SBC
        } else if sep.codec == A2DP_CODEC_MPEG24 {
            LWASAPI_BT_CAP_AAC
        } else if sep.codec == A2DP_CODEC_VENDOR && sep.caps.len() >= 6 {
            let vid = u32::from_le_bytes([sep.caps[0], sep.caps[1], sep.caps[2], sep.caps[3]]);
            let cid = u16::from_le_bytes([sep.caps[4], sep.caps[5]]);
            if vid == LDAC_VENDOR_ID && cid == LDAC_CODEC_ID {
                LWASAPI_BT_CAP_LDAC
            } else if vid == APTX_HD_VENDOR_ID && cid == APTX_HD_CODEC_ID {
                LWASAPI_BT_CAP_APTX_HD
            } else if vid == APTX_VENDOR_ID && cid == APTX_CODEC_ID {
                LWASAPI_BT_CAP_APTX
            } else {
                0
            }
        } else {
            0
        };
        mask |= sep_cap;
        if want_cap != 0 && sep_cap == want_cap && matched.is_none() && !sep.caps.is_empty() {
            matched = Some((sep.obj_path.clone(), sep.caps.clone()));
        }
    }
    if mask != 0 {
        rt.state.lock().unwrap().supported_codecs_mask = mask;
    }
    matched
}

fn send_remote_sep_set_config(
    db: &DBusApi,
    conn: *mut DBusConnection,
    sep_path: &str,
    local_ep: &str,
    cfg_bytes: &[u8],
) -> *mut DBusPendingCall {
    if conn.is_null() || sep_path.is_empty() || local_ep.is_empty() || cfg_bytes.is_empty() {
        return ptr::null_mut();
    }
    let msg = dbus_new_method_call(
        db,
        "org.bluez",
        sep_path,
        "org.bluez.MediaEndpoint1",
        "SetConfiguration",
    );
    if msg.is_null() {
        return ptr::null_mut();
    }
    let mut pending: *mut DBusPendingCall = ptr::null_mut();
    unsafe {
        let mut args = DBusMessageIter::default();
        let mut dict = DBusMessageIter::default();
        (db.message_iter_init_append)(msg, &mut args);
        let _ = iter_append_str(db, &mut args, DBUS_TYPE_OBJECT_PATH, local_ep);
        if (db.message_iter_open_container)(&mut args, DBUS_TYPE_ARRAY, c"{sv}".as_ptr(), &mut dict)
            != 0
        {
            add_dict_entry_bytes(db, &mut dict, "Capabilities", cfg_bytes);
            let _ = (db.message_iter_close_container)(&mut args, &mut dict);
        }
        let ok = (db.connection_send_with_reply)(conn, msg, &mut pending, 4000);
        (db.message_unref)(msg);
        (db.connection_flush)(conn);
        if ok != 0 {
            pending
        } else {
            ptr::null_mut()
        }
    }
}

fn spawn_dbus_thread(rt: Arc<BluezRuntime>) {
    let rt_bg = Arc::clone(&rt);
    let jh = thread::spawn(move || {
        let rt = rt_bg;
        let Some(db) = load_dbus_library() else {
            return;
        };
        unsafe { (db.threads_init_default)() };
        rt.need_reregister.store(false, Ordering::Release);

        let mut err = DBusError::default();
        unsafe { (db.error_init)(&mut err) };
        let conn = unsafe { (db.bus_get_private)(DBUS_BUS_SYSTEM, &mut err) };
        if conn.is_null() {
            unsafe { (db.error_free)(&mut err) };
            return;
        }
        unsafe { (db.connection_set_exit_on_disconnect)(conn, 0) };
        *rt.dbus_conn.lock().unwrap_or_else(|e| e.into_inner()) = DbusConnPtr(conn);

        unsafe {
            (db.bus_add_match)(
                conn,
                c"type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.bluez'".as_ptr(),
                ptr::null_mut(),
            );
            (db.bus_add_match)(
                conn,
                c"type='signal',sender='org.bluez',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'".as_ptr(),
                ptr::null_mut(),
            );
            (db.bus_add_match)(
                conn,
                c"type='signal',sender='org.bluez',interface='org.freedesktop.DBus.ObjectManager',member='InterfacesAdded'".as_ptr(),
                ptr::null_mut(),
            );
            (db.bus_add_match)(
                conn,
                c"type='signal',sender='org.bluez',interface='org.freedesktop.DBus.ObjectManager',member='InterfacesRemoved'".as_ptr(),
                ptr::null_mut(),
            );
            let rt_ptr = (&*rt as *const BluezRuntime as *mut BluezRuntime).cast::<c_void>();
            (db.connection_add_filter)(conn, Some(bluez_signal_filter), rt_ptr, ptr::null());
        }

        let mut ep_reg = [false; 5];
        let mut hfp_reg = false;
        let mut batt_reg = false;
        register_all_adapters_and_kick_connected(
            db,
            conn,
            &rt,
            &mut ep_reg,
            &mut hfp_reg,
            &mut batt_reg,
        );
        let _ = probe_remote_seps(db, conn, &rt, 0);

        let thread_start_ns = mono_ns();
        let mut no_transport_since_ns = thread_start_ns;
        let mut last_rereg_attempt_ns = thread_start_ns;
        let mut autoconnect_done = false;
        let mut kick_retried = false;
        let mut kick_cycled = false;
        let mut last_acquire_retry_ns = 0u64;
        let mut reneg_pending: *mut DBusPendingCall = ptr::null_mut();

        while rt.dbus_thread_running.load(Ordering::Acquire) {
            unsafe { (db.connection_read_write_dispatch)(conn, 50) };
            hfp_flush_battery_dbus(db, conn, &rt);

            let loop_now_ns = mono_ns();

            if !reneg_pending.is_null()
                && unsafe { (db.pending_call_get_completed)(reneg_pending) } != 0
            {
                let rrep = unsafe { (db.pending_call_steal_reply)(reneg_pending) };
                unsafe { (db.pending_call_unref)(reneg_pending) };
                reneg_pending = ptr::null_mut();
                if !rrep.is_null() {
                    if unsafe { (db.message_get_type)(rrep) } == DBUS_MESSAGE_TYPE_ERROR {
                        let mut rit = DBusMessageIter::default();
                        let emsg = if unsafe { (db.message_iter_init)(rrep, &mut rit) } != 0 {
                            unsafe { iter_get_str(db, &mut rit) }
                        } else {
                            None
                        };
                        let err_txt = emsg.as_deref().unwrap_or("BlueZ error");
                        let (dpath, has_tp) = {
                            let mut st = rt.state.lock().unwrap();
                            let err_short: String = err_txt.chars().take(28).collect();
                            st.neg_reason = format!("Codec rejected ({err_short}); restoring");
                            st.pref_codec = LWASAPI_BT_PREF_AUTO;
                            (st.device_path.clone(), st.has_bluez_transport)
                        };
                        dbg_emit(
                            LWASAPI_DBG_BLUEZ,
                            LWASAPI_DBG_WARN,
                            0,
                            0,
                            "BT_RENEG_ERR",
                            &dpath,
                            &format!("Remote SetConfiguration failed: {err_txt}"),
                        );
                        if !dpath.is_empty() && !has_tp {
                            send_connect_profile(db, conn, &dpath, A2DP_SINK_UUID);
                        }
                    }
                    unsafe { (db.message_unref)(rrep) };
                }
                rt.bt_renegotiate_fallback.store(false, Ordering::Release);
            }

            if rt.bt_renegotiate_req.swap(false, Ordering::AcqRel) {
                if !reneg_pending.is_null() {
                    unsafe {
                        (db.pending_call_cancel)(reneg_pending);
                        (db.pending_call_unref)(reneg_pending);
                    }
                    reneg_pending = ptr::null_mut();
                }
                let _ = probe_remote_seps(db, conn, &rt, 0);

                let (dpath, old_tp, want_pref, sup_mask, cur_kind, had_tp) = {
                    let st = rt.state.lock().unwrap();
                    (
                        st.device_path.clone(),
                        st.transport_path.clone(),
                        st.pref_codec,
                        st.supported_codecs_mask,
                        st.active_codec_kind,
                        st.has_bluez_transport,
                    )
                };

                let (target_cap, local_ep) = match want_pref {
                    LWASAPI_BT_PREF_AUTO => {
                        if (sup_mask & LWASAPI_BT_CAP_LDAC) != 0 && load_ldac_library().is_some() {
                            (LWASAPI_BT_CAP_LDAC, LWASAPI_EP_LDAC)
                        } else if (sup_mask & LWASAPI_BT_CAP_APTX_HD) != 0
                            && load_aptx_library().is_some()
                        {
                            (LWASAPI_BT_CAP_APTX_HD, LWASAPI_EP_APTXHD)
                        } else if (sup_mask & LWASAPI_BT_CAP_APTX) != 0
                            && load_aptx_library().is_some()
                        {
                            (LWASAPI_BT_CAP_APTX, LWASAPI_EP_APTX)
                        } else if (sup_mask & LWASAPI_BT_CAP_AAC) != 0
                            && load_fdk_aac_library().is_some()
                        {
                            (LWASAPI_BT_CAP_AAC, LWASAPI_EP_AAC)
                        } else {
                            (LWASAPI_BT_CAP_SBC, LWASAPI_EP_SBC)
                        }
                    }
                    LWASAPI_BT_PREF_LDAC => (LWASAPI_BT_CAP_LDAC, LWASAPI_EP_LDAC),
                    LWASAPI_BT_PREF_APTX_HD => (LWASAPI_BT_CAP_APTX_HD, LWASAPI_EP_APTXHD),
                    LWASAPI_BT_PREF_APTX => (LWASAPI_BT_CAP_APTX, LWASAPI_EP_APTX),
                    LWASAPI_BT_PREF_AAC => (LWASAPI_BT_CAP_AAC, LWASAPI_EP_AAC),
                    _ => (LWASAPI_BT_CAP_SBC, LWASAPI_EP_SBC),
                };

                let cur_cap = match cur_kind {
                    LWASAPI_BT_CODEC_LDAC => LWASAPI_BT_CAP_LDAC,
                    LWASAPI_BT_CODEC_APTX_HD => LWASAPI_BT_CAP_APTX_HD,
                    LWASAPI_BT_CODEC_APTX => LWASAPI_BT_CAP_APTX,
                    LWASAPI_BT_CODEC_AAC => LWASAPI_BT_CAP_AAC,
                    _ => LWASAPI_BT_CAP_SBC,
                };

                if target_cap == cur_cap && had_tp {
                    let mut st = rt.state.lock().unwrap();
                    let cn: String = st.codec_name.chars().take(15).collect();
                    st.neg_reason = format!("Active codec {cn} already optimal for Auto");
                } else {
                    let remote_match = if (target_cap != cur_cap || !had_tp) && !dpath.is_empty() {
                        probe_remote_seps(db, conn, &rt, target_cap).and_then(|(sp, rc)| {
                            select_a2dp_config_for_endpoint(local_ep, &rc)
                                .map(|(sc, _)| (sp, sc))
                        })
                    } else {
                        None
                    };
                    if let Some((sep_path, sel_cfg)) = remote_match {
                        dbg_emit(
                            LWASAPI_DBG_BLUEZ,
                            LWASAPI_DBG_INFO,
                            0,
                            0,
                            "BT_RENEG_SEND",
                            &sep_path,
                            &format!("Switching SEP -> {local_ep} (pref={want_pref})"),
                        );
                        set_tx_enabled_impl(&rt, false);
                        {
                            let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
                            st.close_bt_fd();
                            st.direct_socket_mode = false;
                            st.has_bluez_transport = false;
                            st.tx_stream_active = false;
                            st.enc.close_all();
                            sync_tx_ready(&rt, &st);
                        }
                        if !old_tp.is_empty() {
                            let rel = dbus_new_method_call(
                                db,
                                "org.bluez",
                                &old_tp,
                                "org.bluez.MediaTransport1",
                                "Release",
                            );
                            if !rel.is_null() {
                                unsafe {
                                    let _ = (db.connection_send)(conn, rel, ptr::null_mut());
                                    (db.message_unref)(rel);
                                    (db.connection_flush)(conn);
                                    for _ in 0..4 {
                                        let _ = (db.connection_read_write_dispatch)(conn, 20);
                                    }
                                }
                            }
                        }
                        reneg_pending =
                            send_remote_sep_set_config(db, conn, &sep_path, local_ep, &sel_cfg);
                        if !reneg_pending.is_null() {
                            let now_r = mono_ns();
                            rt.state.lock().unwrap().renegotiate_start_ns = now_r;
                            rt.bt_renegotiate_fallback.store(true, Ordering::Release);
                            no_transport_since_ns = now_r;
                        } else {
                            send_connect_profile(db, conn, &dpath, A2DP_SINK_UUID);
                        }
                    } else {
                        dbg_emit(
                            LWASAPI_DBG_BLUEZ,
                            LWASAPI_DBG_WARN,
                            0,
                            0,
                            "BT_RENEG_SKIP",
                            local_ep,
                            &format!("can_switch=0 target_cap=0x{target_cap:x} cur_cap=0x{cur_cap:x}"),
                        );
                        let mut st = rt.state.lock().unwrap();
                        st.pref_codec = LWASAPI_BT_PREF_AUTO;
                        let kept = if st.codec_name.is_empty() {
                            "active".to_string()
                        } else {
                            st.codec_name.chars().take(15).collect()
                        };
                        st.neg_reason = format!("Headset lacks requested SEP; kept {kept}");
                    }
                }
            }

            let reneg_now_ns = mono_ns();
            if rt.bt_renegotiate_fallback.load(Ordering::Acquire) {
                let (has_tp, start_ns, dpath) = {
                    let mut st = rt.state.lock().unwrap();
                    let expired = !st.has_bluez_transport
                        && st.renegotiate_start_ns > 0
                        && reneg_now_ns.saturating_sub(st.renegotiate_start_ns) >= 4_500_000_000;
                    if expired {
                        st.pref_codec = LWASAPI_BT_PREF_AUTO;
                        st.neg_reason =
                            "Peer timed out codec switch; restored Auto".to_string();
                    }
                    (st.has_bluez_transport, st.renegotiate_start_ns, st.device_path.clone())
                };
                if !has_tp
                    && start_ns > 0
                    && reneg_now_ns.saturating_sub(start_ns) >= 4_500_000_000
                {
                    rt.bt_renegotiate_fallback.store(false, Ordering::Release);
                    dbg_emit(
                        LWASAPI_DBG_BLUEZ,
                        LWASAPI_DBG_WARN,
                        0,
                        0,
                        "BT_RENEG_TMO",
                        &dpath,
                        "Peer timed out codec switch; restoring Auto",
                    );
                    if !dpath.is_empty() {
                        send_connect_profile(db, conn, &dpath, A2DP_SINK_UUID);
                    }
                }
            }

            let ep_registered = rt.state.lock().unwrap().endpoints_registered;
            if rt.need_reregister.swap(false, Ordering::AcqRel)
                || (!ep_registered
                    && loop_now_ns.saturating_sub(last_rereg_attempt_ns) >= 1_500_000_000)
            {
                last_rereg_attempt_ns = loop_now_ns;
                register_all_adapters_and_kick_connected(
                    db,
                    conn,
                    &rt,
                    &mut ep_reg,
                    &mut hfp_reg,
                    &mut batt_reg,
                );
                let _ = probe_remote_seps(db, conn, &rt, 0);
                no_transport_since_ns = mono_ns();
                kick_retried = false;
                kick_cycled = false;
            }

            if rt.need_acquire.swap(false, Ordering::AcqRel) {
                last_acquire_retry_ns = loop_now_ns;
                acquire_bluez_transport(db, conn, &rt);
                let _ = probe_remote_seps(db, conn, &rt, 0);
                let tp_now = rt.state.lock().unwrap().transport_path.clone();
                if !tp_now.is_empty() {
                    if let Some(cur_v) = unsafe {
                        dbus_get_u16_prop(db, conn, &tp_now, "org.bluez.MediaTransport1", "Volume")
                    } {
                        rt.avrcp_volume.store(cur_v, Ordering::Relaxed);
                        rt.desktop_volume_pct
                            .store((u32::from(cur_v) * 100 + 63) / 127, Ordering::Relaxed);
                        rt.avrcp_supported.store(true, Ordering::Relaxed);
                    }
                }
            }

            if rt.need_avrcp_sync.swap(false, Ordering::AcqRel) {
                let pct = rt.pending_avrcp_vol.load(Ordering::Relaxed);
                let tp_now = rt.state.lock().unwrap().transport_path.clone();
                if pct >= 0 && !tp_now.is_empty() {
                    let clamped = (pct as u32).min(100);
                    let v127 = ((clamped * 127 + 50) / 100) as u16;
                    rt.avrcp_volume.store(v127, Ordering::Relaxed);
                    unsafe { dbus_set_avrcp_volume(db, conn, &tp_now, v127) };
                }
            }

            let now_ns = mono_ns();
            let (bt_live, bt_cfg, bt_has_tp, ep_reg_now, cand_kick, startup_auto) = {
                let st = rt.state.lock().unwrap();
                let live = st.has_bluez_transport
                    && st.direct_socket_mode
                    && st.bt_fd >= 0
                    && st.tx_stream_active;
                let cand = if !st.bt_kick_dev.is_empty() {
                    st.bt_kick_dev.clone()
                } else {
                    st.device_path.clone()
                };
                (
                    live,
                    st.has_bluez_transport,
                    !st.transport_path.is_empty(),
                    st.endpoints_registered,
                    cand,
                    st.bt_startup_autoconnect_dev.clone(),
                )
            };

            if bt_live {
                no_transport_since_ns = now_ns;
                last_acquire_retry_ns = 0;
                kick_retried = false;
                kick_cycled = false;
                rt.state.lock().unwrap().bt_kick_dev.clear();
                continue;
            }

            if bt_cfg && bt_has_tp && now_ns.saturating_sub(last_acquire_retry_ns) >= 300_000_000 {
                last_acquire_retry_ns = now_ns;
                acquire_bluez_transport(db, conn, &rt);
                let now_live = {
                    let st = rt.state.lock().unwrap();
                    st.has_bluez_transport
                        && st.direct_socket_mode
                        && st.bt_fd >= 0
                        && st.tx_stream_active
                };
                if now_live {
                    no_transport_since_ns = mono_ns();
                    last_acquire_retry_ns = 0;
                    kick_retried = false;
                    kick_cycled = false;
                    rt.state.lock().unwrap().bt_kick_dev.clear();
                    continue;
                }
            }

            if !autoconnect_done
                && !startup_auto.is_empty()
                && now_ns.saturating_sub(thread_start_ns) >= 1_200_000_000
            {
                autoconnect_done = true;
                let already = unsafe {
                    dbus_get_bool_prop(db, conn, &startup_auto, "org.bluez.Device1", "Connected")
                }
                .unwrap_or(false);
                if !already {
                    unsafe {
                        dbus_set_str_prop(
                            db,
                            conn,
                            &startup_auto,
                            "org.bluez.Device1",
                            "PreferredBearer",
                            "bredr",
                        );
                    }
                    let cmsg = dbus_new_method_call(
                        db,
                        "org.bluez",
                        &startup_auto,
                        "org.bluez.Device1",
                        "Connect",
                    );
                    if !cmsg.is_null() {
                        unsafe {
                            let _ = (db.connection_send)(conn, cmsg, ptr::null_mut());
                            (db.message_unref)(cmsg);
                        }
                        dbg_emit(
                            LWASAPI_DBG_BLUEZ,
                            LWASAPI_DBG_INFO,
                            0,
                            0,
                            "BT_AUTOCONN",
                            &startup_auto,
                            "Startup BR/EDR auto-connect triggered for preferred device",
                        );
                    }
                }
            }

            let drop_ns = rt.tx_drop_event_ns.load(Ordering::Relaxed);
            if drop_ns > no_transport_since_ns {
                no_transport_since_ns = drop_ns;
                kick_retried = false;
                kick_cycled = false;
            }
            if ep_reg_now
                && !cand_kick.is_empty()
                && reneg_pending.is_null()
                && !rt.bt_renegotiate_fallback.load(Ordering::Acquire)
                && !rt.recovery_active.load(Ordering::Acquire)
            {
                let elapsed_ns = now_ns.saturating_sub(no_transport_since_ns);
                let last_rec = rt.last_recover_ns.load(Ordering::Relaxed);
                let cooldown_ok =
                    last_rec == 0 || now_ns.saturating_sub(last_rec) >= 30_000_000_000;
                if cooldown_ok && elapsed_ns >= 1_800_000_000 && !kick_retried {
                    kick_retried = true;
                    let is_conn = unsafe {
                        dbus_get_bool_prop(db, conn, &cand_kick, "org.bluez.Device1", "Connected")
                    }
                    .unwrap_or(false);
                    if is_conn {
                        unsafe {
                            dbus_set_str_prop(
                                db,
                                conn,
                                &cand_kick,
                                "org.bluez.Device1",
                                "PreferredBearer",
                                "bredr",
                            );
                        }
                        send_connect_profile(db, conn, &cand_kick, A2DP_SINK_UUID);
                    } else {
                        kick_cycled = true;
                    }
                } else if cooldown_ok && elapsed_ns >= 6_000_000_000 && !kick_cycled {
                    kick_cycled = true;
                    rt.last_recover_ns.store(now_ns, Ordering::Relaxed);
                    let is_conn = unsafe {
                        dbus_get_bool_prop(db, conn, &cand_kick, "org.bluez.Device1", "Connected")
                    }
                    .unwrap_or(false);
                    if is_conn {
                        send_connect_profile(db, conn, &cand_kick, A2DP_SINK_UUID);
                    }
                }
            }
        }

        if !reneg_pending.is_null() {
            unsafe {
                (db.pending_call_cancel)(reneg_pending);
                (db.pending_call_unref)(reneg_pending);
            }
        }
        unsafe {
            let rt_ptr = (&*rt as *const BluezRuntime as *mut BluezRuntime).cast::<c_void>();
            (db.connection_remove_filter)(conn, Some(bluez_signal_filter), rt_ptr);
            let paths: [&CStr; 7] = [
                c"/org/lwasapi/A2DP/LDAC/Source",
                c"/org/lwasapi/A2DP/APTXHD/Source",
                c"/org/lwasapi/A2DP/APTX/Source",
                c"/org/lwasapi/A2DP/AAC/Source",
                c"/org/lwasapi/A2DP/SBC/Source",
                c"/org/lwasapi/HFP/AG",
                c"/org/lwasapi/Battery",
            ];
            for (idx, p) in paths.iter().enumerate() {
                let was_reg = match idx {
                    0..=4 => ep_reg[idx],
                    5 => hfp_reg,
                    _ => batt_reg,
                };
                if was_reg {
                    let _ = (db.connection_unregister_object_path)(conn, p.as_ptr());
                }
            }
            {
                let mut g = rt.dbus_conn.lock().unwrap_or_else(|e| e.into_inner());
                g.0 = ptr::null_mut();
            }
            (db.connection_close)(conn);
            (db.connection_unref)(conn);
        }
    });
    *rt.dbus_thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(jh);
}

// ============================================================================
// Two-Way HFP BTPROTO_SCO Capture
// ============================================================================

fn stop_sco_capture(rt: &BluezRuntime) {
    rt.sco_thread_running.store(false, Ordering::Release);
    let fd = rt.sco_fd.load(Ordering::Acquire);
    if fd >= 0 {
        shutdown_rdwr(fd);
    }
    if let Some(jh) = rt.sco_thread.lock().unwrap().take() {
        let _ = jh.join();
    }
    let old = rt.sco_fd.swap(-1, Ordering::AcqRel);
    close_raw_fd(old);
    rt.hfp_sco_active.store(0, Ordering::Release);
    rt.state.lock().unwrap().bt_sco_codec.clear();
}

fn open_sco_capture(rt: &BluezRuntime, mac_or_empty: &str) -> i32 {
    let Some(db) = load_dbus_library() else {
        return -1;
    };
    let _held_conn = {
        let g = rt.dbus_conn.lock().unwrap_or_else(|e| e.into_inner());
        if g.0.is_null() {
            return -1;
        }
        HeldDbusConn {
            conn: unsafe { (db.connection_ref)(g.0) },
            unref: db.connection_unref,
        }
    };
    let conn = _held_conn.conn;
    if conn.is_null() {
        return -1;
    }
    let target_mac = if let Some(m) = extract_mac_str(mac_or_empty) {
        m
    } else {
        let cur_mac = rt
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mac_address
            .clone();
        if cur_mac.is_empty() {
            return -1;
        }
        cur_mac
    };

    let adapter_path = rt
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .adapter_path
        .clone();
    let adapter_mac = if !adapter_path.is_empty() {
        unsafe { dbus_get_str_prop(db, conn, &adapter_path, "org.bluez.Adapter1", "Address") }
            .unwrap_or_default()
    } else {
        String::new()
    };

    let Some(remote_bytes) = parse_bdaddr(target_mac.as_bytes()) else {
        return -1;
    };
    let remote_ba = BdAddr { b: remote_bytes };

    let (want_msbc, rfcomm_owner) = {
        let devs = rt.hfp_devs.lock().unwrap_or_else(|e| e.into_inner());
        devs.iter()
            .find(|d| d.in_use && d.rfcomm_active)
            .map(|d| (d.supports_msbc, d.fd_owner.clone()))
            .unwrap_or((false, None))
    };

    if let Some(rfcomm) = rfcomm_owner {
        if want_msbc {
            hfp_write_bytes(rfcomm.as_raw_fd(), b"\r\n+BCS: 2\r\n");
            thread::sleep(Duration::from_millis(120));
        }
    }

    let sfd = socket_raw(
        AF_BLUETOOTH,
        SOCK_SEQPACKET | SOCK_CLOEXEC | SOCK_NONBLOCK,
        BTPROTO_SCO,
    );
    if sfd < 0 {
        return -1;
    }

    if !adapter_mac.is_empty() {
        if let Some(lbytes) = parse_bdaddr(adapter_mac.as_bytes()) {
            let laddr = SockaddrSco {
                sco_family: AF_BLUETOOTH as u16,
                sco_bdaddr: BdAddr { b: lbytes },
            };
            unsafe {
                let _ = bind_raw(
                    sfd,
                    (&laddr as *const SockaddrSco).cast::<c_void>(),
                    mem::size_of::<SockaddrSco>() as u32,
                );
            }
        }
    }

    let voice = BtVoice {
        setting: if want_msbc {
            BT_VOICE_TRANSPARENT
        } else {
            BT_VOICE_CVSD_16BIT
        },
    };
    unsafe {
        let _ = setsockopt_raw(
            sfd,
            SOL_BLUETOOTH,
            BT_VOICE,
            (&voice as *const BtVoice).cast::<c_void>(),
            mem::size_of::<BtVoice>() as u32,
        );
    }

    let raddr = SockaddrSco {
        sco_family: AF_BLUETOOTH as u16,
        sco_bdaddr: remote_ba,
    };
    if unsafe {
        connect_raw(
            sfd,
            (&raddr as *const SockaddrSco).cast::<c_void>(),
            mem::size_of::<SockaddrSco>() as u32,
        )
    } < 0
    {
        let e = last_errno();
        if e != ERRNO_EINPROGRESS && e != ERRNO_EAGAIN {
            close_raw_fd(sfd);
            return -1;
        }
        let mut pfd = [PollFd {
            fd: sfd,
            events: POLLOUT,
            revents: 0,
        }];
        let mut soerr: c_int = 0;
        let mut slen = mem::size_of::<c_int>() as u32;
        if do_poll(&mut pfd, 1500) <= 0
            || unsafe {
                getsockopt_raw(
                    sfd,
                    SOL_SOCKET,
                    SO_ERROR,
                    (&mut soerr as *mut c_int).cast::<c_void>(),
                    &mut slen,
                )
            } != 0
            || soerr != 0
        {
            close_raw_fd(sfd);
            return -1;
        }
    }

    let sco_codec = if want_msbc { "mSBC" } else { "CVSD" };
    let sco_rate = if want_msbc { 16_000u32 } else { 8_000u32 };
    let eng_rate = rt.engine_sample_rate.load(Ordering::Relaxed).max(8_000);
    let sco_dev_id = format!("bluez_sco:{target_mac}");

    rt.sco_fd.store(sfd, Ordering::Release);
    rt.sco_sample_rate.store(sco_rate, Ordering::Relaxed);
    rt.state.lock().unwrap().bt_sco_codec = sco_codec.to_string();
    {
        let mut m = rt.cap.meta.lock().unwrap();
        m.sco_cap_device_id = sco_dev_id.clone();
        m.cap_device_id = sco_dev_id.clone();
        m.capture_device_alias = format!("BT HFP Headset ({sco_codec})");
    }
    rt.cap.cap_hw_rate.store(eng_rate, Ordering::Relaxed);
    rt.cap.cap_hw_channels.store(2, Ordering::Relaxed);
    rt.cap.cap_is_usb.store(false, Ordering::Relaxed);
    rt.cap.cap_ring_rate.store(eng_rate, Ordering::Release);
    rt.cap.cap_measured_rate.store(0, Ordering::Relaxed);
    rt.cap.cap_kick_count.store(0, Ordering::Relaxed);
    rt.cap.has_capture_hw.store(true, Ordering::Release);
    rt.cap.cap_hw_write_frame.store(0, Ordering::Release);
    rt.hfp_sco_active.store(1, Ordering::Release);
    rt.sco_thread_running.store(true, Ordering::Release);

    let rt_arc = Arc::clone(global_bt_arc());
    let use_msbc = want_msbc;
    let jh = thread::spawn(move || {
        let sbc_api = if use_msbc { load_sbc_library() } else { None };
        let mut sbc_dec = SbcStruct::default();
        let mut sbc_enc = SbcStruct::default();
        if let Some(api) = sbc_api {
            unsafe {
                let _ = (api.init_msbc)(&mut sbc_dec, 0);
                let _ = (api.init_msbc)(&mut sbc_enc, 0);
            }
        }
        let mut rx_pkt = [0u8; 256];
        let mut tx_pkt = [0u8; 64];
        let mut dec_pcm = [0i16; 256];
        let zero_bytes = [0u8; 256];
        let mut seq = 0u32;
        let mut resamp_pos = 0.0f64;
        let mut prev_s = 0.0f32;

        while rt_arc.sco_thread_running.load(Ordering::Acquire) {
            let cur_fd = rt_arc.sco_fd.load(Ordering::Acquire);
            if cur_fd < 0 {
                break;
            }
            let mut pfd = [PollFd {
                fd: cur_fd,
                events: POLLIN,
                revents: 0,
            }];
            let pr = do_poll(&mut pfd, 200);
            if pr < 0 {
                if last_errno() == ERRNO_EINTR {
                    continue;
                }
                break;
            }
            if pr == 0 {
                continue;
            }
            if (pfd[0].revents & (POLLERR | POLLHUP | POLLNVAL)) != 0 {
                break;
            }
            let r = recv_raw(cur_fd, &mut rx_pkt, 0);
            if r <= 0 {
                break;
            }
            let rlen = r as usize;
            let mut nsamp = 0usize;
            if use_msbc && rlen >= 59 {
                if let Some(api) = sbc_api {
                    let off = msbc_rx_strip_offset(&rx_pkt[..rlen]);
                    let mut written = 0usize;
                    if rlen > off
                        && unsafe {
                            (api.decode)(
                                &mut sbc_dec,
                                rx_pkt[off..rlen].as_ptr().cast::<c_void>(),
                                rlen - off,
                                dec_pcm.as_mut_ptr().cast::<c_void>(),
                                mem::size_of_val(&dec_pcm),
                                &mut written,
                            )
                        } > 0
                    {
                        nsamp = written / 2;
                    }
                    let mut enc_written = 0isize;
                    tx_pkt[0] = 0x01;
                    tx_pkt[1] = msbc_h2_byte(seq);
                    seq = seq.wrapping_add(1);
                    if unsafe {
                        (api.encode)(
                            &mut sbc_enc,
                            zero_bytes.as_ptr().cast::<c_void>(),
                            240,
                            tx_pkt[2..].as_mut_ptr().cast::<c_void>(),
                            tx_pkt.len() - 3,
                            &mut enc_written,
                        )
                    } > 0
                        && enc_written == 57
                    {
                        tx_pkt[59] = 0x00;
                        let _ = send_raw(cur_fd, &tx_pkt[..60], MSG_DONTWAIT | MSG_NOSIGNAL);
                    }
                }
            } else if !use_msbc && rlen >= 2 {
                nsamp = (rlen / 2).min(128);
                for (i, ch) in rx_pkt[..nsamp * 2].as_chunks::<2>().0.iter().enumerate() {
                    dec_pcm[i] = i16::from_le_bytes(*ch);
                }
                let _ = send_raw(
                    cur_fd,
                    &zero_bytes[..rlen.min(128)],
                    MSG_DONTWAIT | MSG_NOSIGNAL,
                );
            }

            if nsamp > 0 {
                let er = match rt_arc.engine_sample_rate.load(Ordering::Relaxed) {
                    0 => 48_000,
                    e => e,
                };
                rt_arc.cap.cap_ring_rate.store(er, Ordering::Release);
                let mut ring = rt_arc.cap.cap_ring.lock().unwrap();
                sco_push_pcm_to_cap_ring(
                    &mut ring,
                    &rt_arc.cap.cap_hw_write_frame,
                    &dec_pcm[..nsamp],
                    sco_rate,
                    er,
                    &mut resamp_pos,
                    &mut prev_s,
                );
            }
        }
        if let Some(api) = sbc_api {
            unsafe {
                (api.finish)(&mut sbc_dec);
                (api.finish)(&mut sbc_enc);
            }
        }
        rt_arc.hfp_sco_active.store(0, Ordering::Release);
    });
    *rt.sco_thread.lock().unwrap() = Some(jh);

    dbg_emit(
        LWASAPI_DBG_BLUEZ,
        LWASAPI_DBG_INFO,
        0,
        0,
        "BT_SCO_OPEN",
        &sco_dev_id,
        &format!("codec={sco_codec} rate={sco_rate}Hz"),
    );
    0
}

// ============================================================================
// Public Entrypoints Backing c_abi.rs / SalsaBtOps
// ============================================================================

pub fn audiodg_bt_ptr_impl() -> *mut c_void {
    let rt = global_bt_arc();
    ((&**rt) as *const BluezRuntime as *mut BluezRuntime).cast::<c_void>()
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`, and any
/// non-null `out_vol_pct` or `out_muted` pointer must be valid for scalar writes.
pub unsafe fn bt_rt_master_vol_impl(
    bt: *const c_void,
    out_vol_pct: *mut u32,
    out_muted: *mut bool,
) {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        if !out_vol_pct.is_null() {
            unsafe { *out_vol_pct = 100 };
        }
        if !out_muted.is_null() {
            unsafe { *out_muted = false };
        }
        return;
    };
    if !out_vol_pct.is_null() {
        unsafe { *out_vol_pct = rt.desktop_volume_pct.load(Ordering::Relaxed) };
    }
    if !out_muted.is_null() {
        unsafe { *out_muted = rt.desktop_muted.load(Ordering::Relaxed) };
    }
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl` and
/// `out` must point to writable storage for `SalsaBtSnapshot`.
pub unsafe fn bt_get_snapshot_impl(bt: *const c_void, out: *mut SalsaBtSnapshot) {
    if out.is_null() {
        return;
    }
    let out_ref = unsafe { &mut *out };
    *out_ref = unsafe { mem::zeroed() };
    out_ref.bt_fd = -1;
    out_ref.cap_pcm_fd = -1;
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return;
    };

    out_ref.initialized = rt.initialized.load(Ordering::Acquire);
    out_ref.has_capture_hw = rt.cap.has_capture_hw.load(Ordering::Acquire);
    out_ref.tx_paused = rt.tx_paused.load(Ordering::Relaxed);
    out_ref.cap_is_usb = rt.cap.cap_is_usb.load(Ordering::Relaxed);
    out_ref.cap_pcm_fd = rt.cap.cap_pcm_fd.load(Ordering::Relaxed);
    out_ref.delay_tenths_ms = rt.delay_tenths_ms.load(Ordering::Relaxed);
    out_ref.avrcp_volume = rt.avrcp_volume.load(Ordering::Relaxed);
    out_ref.cap_hw_rate = rt.cap.cap_hw_rate.load(Ordering::Relaxed);
    out_ref.cap_hw_channels = rt.cap.cap_hw_channels.load(Ordering::Relaxed);
    out_ref.cap_hw_format = rt.cap.cap_hw_format.load(Ordering::Relaxed);
    out_ref.cap_measured_rate = rt.cap.cap_measured_rate.load(Ordering::Relaxed);
    out_ref.cap_kick_count = rt.cap.cap_kick_count.load(Ordering::Relaxed);
    out_ref.desktop_volume_pct = rt.desktop_volume_pct.load(Ordering::Relaxed);
    out_ref.desktop_muted = rt.desktop_muted.load(Ordering::Relaxed);
    out_ref.desktop_mic_volume_pct = rt.cap.desktop_mic_volume_pct.load(Ordering::Relaxed);
    out_ref.desktop_mic_muted = rt.cap.desktop_mic_muted.load(Ordering::Relaxed);
    out_ref.desktop_mic_gain_ppm = rt.cap.desktop_mic_gain_ppm.load(Ordering::Relaxed);

    let tx_wr = rt.tx_write_frame.load(Ordering::Relaxed);
    let tx_rd = rt.tx_read_frame.load(Ordering::Relaxed);
    out_ref.tx_ring_fill = tx_wr.saturating_sub(tx_rd);
    out_ref.tx_packets_sent = rt.tx_packets_sent.load(Ordering::Relaxed);
    out_ref.tx_underruns = rt.tx_underruns.load(Ordering::Relaxed);
    out_ref.tx_errors = rt.tx_errors.load(Ordering::Relaxed);
    out_ref.cap_hw_write_frame = rt.cap.cap_hw_write_frame.load(Ordering::Relaxed);

    {
        let m = rt.cap.meta.lock().unwrap();
        copy_str_to_c(&mut out_ref.cap_device_id, &m.cap_device_id);
        copy_str_to_c(&mut out_ref.cap_pcm_path, &m.cap_pcm_path);
        copy_str_to_c(&mut out_ref.capture_device_alias, &m.capture_device_alias);
    }

    let st = rt.state.lock().unwrap();
    out_ref.has_bluez_transport = st.has_bluez_transport;
    out_ref.direct_socket_mode = st.direct_socket_mode;
    out_ref.endpoints_registered = st.endpoints_registered;
    out_ref.bt_fd = st.bt_fd;
    out_ref.read_mtu = st.read_mtu;
    out_ref.write_mtu = st.write_mtu;
    out_ref.bt_sample_rate = st.bt_sample_rate;
    out_ref.bt_channels = st.bt_channels;
    copy_str_to_c(&mut out_ref.mac_address, &st.mac_address);
    copy_str_to_c(&mut out_ref.device_alias, &st.device_alias);
    copy_str_to_c(&mut out_ref.codec_name, &st.codec_name);
    copy_str_to_c(&mut out_ref.device_path, &st.device_path);
    copy_str_to_c(&mut out_ref.transport_path, &st.transport_path);
    copy_str_to_c(&mut out_ref.endpoint_path, &st.endpoint_path);
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl` and
/// `preferred_pb_id` must be null or point to a valid NUL-terminated C string.
pub unsafe fn bluez_set_preferred_target_impl(bt: *mut c_void, preferred_pb_id: *const c_char) {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return;
    };
    let pb = crate::c_abi::ptr_to_opt_str(preferred_pb_id).unwrap_or_default();
    let mac = if pb.len() >= 6 && pb[..6].eq_ignore_ascii_case("bluez:") {
        extract_mac_str(&pb).unwrap_or_default()
    } else {
        String::new()
    };
    rt.state.lock().unwrap().preferred_mac = mac;
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`, and
/// `preferred_cap_id` / `preferred_pb_id` must be null or valid NUL-terminated
/// C strings.
pub unsafe fn bluez_init_rates_impl(
    bt: *mut c_void,
    engine_sample_rate: u32,
    capture_sample_rate: u32,
    preferred_cap_id: *const c_char,
    preferred_pb_id: *const c_char,
) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    bluez_close_impl(bt);

    let pb = crate::c_abi::ptr_to_opt_str(preferred_pb_id).unwrap_or_default();
    let cap_id = crate::c_abi::ptr_to_opt_str(preferred_cap_id).unwrap_or_default();
    let eff_eng = if engine_sample_rate > 0 {
        engine_sample_rate
    } else {
        48_000
    };

    rt.engine_sample_rate.store(eff_eng, Ordering::Release);
    rt.desktop_volume_pct.store(100, Ordering::Relaxed);
    rt.desktop_muted.store(false, Ordering::Relaxed);
    rt.cap.desktop_mic_volume_pct.store(100, Ordering::Relaxed);
    rt.cap.desktop_mic_muted.store(false, Ordering::Relaxed);
    rt.cap
        .desktop_mic_gain_ppm
        .store(1_000_000, Ordering::Relaxed);
    rt.avrcp_volume.store(127, Ordering::Relaxed);
    rt.tx_drop_event_ns.store(0, Ordering::Relaxed);
    rt.tx_write_frame.store(0, Ordering::Release);
    rt.tx_read_frame.store(0, Ordering::Release);
    rt.tx_transport_ready.store(false, Ordering::Release);
    rt.tx_paused.store(false, Ordering::Release);

    let has_pref_mac = {
        let mut st = rt.state.lock().unwrap();
        *st = BtMutable::default();
        if pb.len() >= 6 && pb[..6].eq_ignore_ascii_case("bluez:") {
            st.preferred_mac = extract_mac_str(&pb).unwrap_or_default();
        }
        !st.preferred_mac.is_empty()
    };
    rt.initialized.store(true, Ordering::Release);

    rt.dbus_thread_running.store(true, Ordering::Release);
    spawn_dbus_thread(Arc::clone(global_bt_arc()));

    rt.tx_thread_running.store(true, Ordering::Release);
    spawn_tx_thread(Arc::clone(global_bt_arc()));

    let want_bt_pref = has_pref_mac
        || pb.eq_ignore_ascii_case("bluetooth")
        || pb.eq_ignore_ascii_case("bluez");
    let max_wait_ticks = if want_bt_pref { 125 } else { 20 };
    let mut bt_ready_now = false;
    for _ in 0..max_wait_ticks {
        {
            let st = rt.state.lock().unwrap();
            bt_ready_now = st.has_bluez_transport && st.direct_socket_mode && st.bt_fd >= 0;
        }
        if bt_ready_now {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let req_cap = if capture_sample_rate > 0 {
        capture_sample_rate
    } else {
        eff_eng
    };
    unsafe { bluez_open_capture_impl(bt, preferred_cap_id, req_cap) };
    let _ = cap_id;
    if bt_ready_now {
        0
    } else {
        -1
    }
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_reconfigure_rate_impl(bt: *mut c_void, new_engine_sample_rate: u32) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    if !rt.initialized.load(Ordering::Acquire) {
        return -1;
    }
    if (44_100..=192_000).contains(&new_engine_sample_rate) {
        let _g = rt.state.lock().unwrap();
        rt.engine_sample_rate
            .store(new_engine_sample_rate, Ordering::Release);
    }
    0
}

fn set_tx_enabled_impl(rt: &BluezRuntime, enabled: bool) {
    if !enabled {
        rt.tx_paused.store(true, Ordering::Release);
    } else if rt.tx_paused.swap(false, Ordering::AcqRel) {
        let _ring = rt.tx_ring.lock().unwrap_or_else(|e| e.into_inner());
        let wr = rt.tx_write_frame.load(Ordering::Acquire);
        rt.tx_read_frame.store(wr, Ordering::Release);
    }
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_set_tx_enabled_impl(bt: *mut c_void, enabled: bool) {
    if let Some(rt) = unsafe { bt_from_ptr(bt) } {
        set_tx_enabled_impl(rt, enabled);
    }
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`, and
/// `interleaved_float` must point to at least `frames * channels` readable
/// `f32` samples.
pub unsafe fn bluez_write_period_impl(
    bt: *mut c_void,
    interleaved_float: *const f32,
    frames: u32,
    channels: u32,
) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    if !rt.initialized.load(Ordering::Acquire)
        || !rt.tx_transport_ready.load(Ordering::Acquire)
        || interleaved_float.is_null()
        || frames == 0
        || channels == 0
    {
        return -1;
    }
    let Some(total) = (frames as usize).checked_mul(channels as usize) else {
        return -1;
    };
    let in_slice = unsafe { core::slice::from_raw_parts(interleaved_float, total) };
    let paused = rt.tx_paused.load(Ordering::Relaxed);
    let mut ring = rt.tx_ring.lock().unwrap();
    write_tx_period(
        &mut ring,
        &rt.tx_write_frame,
        paused,
        in_slice,
        frames as usize,
        channels as usize,
    )
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`, and
/// `preferred_cap_id` must be null or point to a valid NUL-terminated C string.
pub unsafe fn bluez_open_capture_impl(
    bt: *mut c_void,
    preferred_cap_id: *const c_char,
    capture_sample_rate: u32,
) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    stop_capture_thread(&rt.cap);
    stop_sco_capture(rt);

    let mut eng_rate = rt.engine_sample_rate.load(Ordering::Relaxed);
    if eng_rate == 0 {
        eng_rate = if capture_sample_rate > 0 {
            capture_sample_rate
        } else {
            48_000
        };
        rt.engine_sample_rate.store(eng_rate, Ordering::Release);
    }
    let cap_id = crate::c_abi::ptr_to_opt_str(preferred_cap_id).unwrap_or_default();
    let sco_enabled = rt.state.lock().unwrap().hfp_sco_enabled != 0;
    if let Some(mac) = cap_id.strip_prefix("bluez_sco:") {
        if sco_enabled && open_sco_capture(rt, mac) == 0 {
            return 0;
        }
    }

    let mut card_names: [String; MAX_ALSA_CARDS_SCAN] = std::array::from_fn(|_| String::new());
    let mut card_is_usb = [false; MAX_ALSA_CARDS_SCAN];
    parse_asound_cards(&mut card_names, &mut card_is_usb);
    open_alsa_capture(
        &rt.cap,
        eng_rate,
        &cap_id,
        capture_sample_rate,
        &card_names,
        &card_is_usb,
    )
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`,
/// `out_stereo_float` must point to writable storage for `max_frames * 2`
/// `f32` samples, and any non-null scalar output pointers must be valid.
#[allow(clippy::too_many_arguments)]
pub unsafe fn bluez_read_capture_native_impl(
    bt: *mut c_void,
    out_stereo_float: *mut f32,
    max_frames: u32,
    out_frames: *mut u32,
    out_rate: *mut u32,
    eng_frames: u32,
    eng_rate: u32,
    io_read_cursor: *mut u64,
) -> c_int {
    let rt = unsafe { bt_from_ptr(bt) };
    let (cap_ref, init) = match rt {
        Some(r) => (Some(&*r.cap), r.initialized.load(Ordering::Acquire)),
        None => (None, false),
    };
    unsafe {
        read_capture_native(
            cap_ref,
            init,
            out_stereo_float,
            max_frames,
            out_frames,
            out_rate,
            eng_frames,
            eng_rate,
            io_read_cursor,
        )
    }
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`,
/// `active_pb_id` and `active_cap_id` must be valid NUL-terminated C strings,
/// and `out_list` must point to writable storage for `LwasapiDeviceList`.
pub unsafe fn bluez_enumerate_devices_impl(
    bt: *const c_void,
    active_pb_id: *const c_char,
    active_cap_id: *const c_char,
    out_list: *mut LwasapiDeviceList,
) -> c_int {
    if out_list.is_null() {
        return -1;
    }
    let out = unsafe { &mut *out_list };
    *out = unsafe { mem::zeroed() };

    let pb_id = crate::c_abi::ptr_to_opt_str(active_pb_id).unwrap_or_default();
    let cap_id = crate::c_abi::ptr_to_opt_str(active_cap_id).unwrap_or_default();
    let rt = unsafe { bt_from_ptr(bt) };

    let mut card_names: [String; MAX_ALSA_CARDS_SCAN] = std::array::from_fn(|_| String::new());
    let mut card_is_usb = [false; MAX_ALSA_CARDS_SCAN];
    parse_asound_cards(&mut card_names, &mut card_is_usb);

    {
        let _guard = DBUS_ENUM_LOCK.lock().unwrap();
        if let Some(db) = load_dbus_library() {
            unsafe { (db.threads_init_default)() };
            let conn = unsafe { (db.bus_get)(DBUS_BUS_SYSTEM, ptr::null_mut()) };
            if !conn.is_null() {
                if let Some(objs) = fetch_managed_objects(db, conn, 1200) {
                    let (bt_live, bt_mac, bt_codec, sco_en) = if let Some(r) = rt {
                        let st = r.state.lock().unwrap();
                        (
                            st.has_bluez_transport && st.direct_socket_mode && st.bt_fd >= 0,
                            st.mac_address.clone(),
                            st.codec_name.clone(),
                            st.hfp_sco_enabled != 0,
                        )
                    } else {
                        (false, String::new(), String::new(), false)
                    };

                    for d in &objs.devices {
                        if (out.num_devices as usize) >= LWASAPI_MAX_DEVICES {
                            break;
                        }
                        let has_mt = objs.transports.iter().any(|t| t.device_path == d.obj_path);
                        let matches_live =
                            bt_live && !bt_mac.is_empty() && bt_mac.eq_ignore_ascii_case(&d.address);
                        if d.connected
                            && d.has_a2dp_sink
                            && !d.address.is_empty()
                            && (matches_live || (rt.is_none() && has_mt))
                        {
                            let idx = out.num_devices as usize;
                            out.num_devices += 1;
                            let mut e: LwasapiDeviceEntry = unsafe { mem::zeroed() };
                            let eid = format!("bluez:{}", d.address);
                            let cn = if (matches_live || (bt_live && bt_mac.is_empty()))
                                && !bt_codec.is_empty()
                            {
                                bt_codec.as_str()
                            } else {
                                "A2DP"
                            };
                            let label = if !d.alias.is_empty() {
                                d.alias.as_str()
                            } else {
                                d.address.as_str()
                            };
                            let ename = format!("{label} ({cn})");
                            copy_str_to_c(&mut e.id, &eid);
                            copy_str_to_c(&mut e.name, &ename);
                            copy_str_to_c(&mut e.driver, "bluez-a2dp");
                            e.is_capture = 0;
                            e.is_bluetooth = 1;
                            e.is_active = u32::from(
                                pb_id == eid
                                    || pb_id == "bluetooth"
                                    || pb_id == "bluez",
                            );
                            e.min_rate = 44_100;
                            e.max_rate = if cn == "LDAC" { 96_000 } else { 48_000 };
                            out.devices[idx] = e;
                        }

                        if d.connected
                            && d.has_hfp_hf
                            && !d.address.is_empty()
                            && sco_en
                            && (out.num_devices as usize) < LWASAPI_MAX_DEVICES
                        {
                            let idx = out.num_devices as usize;
                            out.num_devices += 1;
                            let mut sc: LwasapiDeviceEntry = unsafe { mem::zeroed() };
                            let sid = format!("bluez_sco:{}", d.address);
                            let label: String = if !d.alias.is_empty() {
                                d.alias.chars().take(42).collect()
                            } else {
                                d.address.chars().take(42).collect()
                            };
                            let sname = format!("{label} (HFP SCO Mic)");
                            copy_str_to_c(&mut sc.id, &sid);
                            copy_str_to_c(&mut sc.name, &sname);
                            copy_str_to_c(&mut sc.driver, "bluez-hfp-sco");
                            sc.is_capture = 1;
                            sc.is_bluetooth = 1;
                            sc.is_active = u32::from(cap_id == sid);
                            sc.min_rate = 8_000;
                            sc.max_rate = 16_000;
                            out.devices[idx] = sc;
                        }
                    }
                }
            }
        }
    }

    append_alsa_devices(out, &pb_id, &cap_id, &card_names, &card_is_usb);
    0
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_hw_fingerprint_impl(bt: *const c_void) -> u64 {
    let mut h = alsa_hw_fingerprint_seed();
    if let Some(rt) = unsafe { bt_from_ptr(bt) } {
        let (bt_live, tx_act, cid, mac) = {
            let st = rt.state.lock().unwrap();
            (
                st.has_bluez_transport && st.direct_socket_mode && st.bt_fd >= 0,
                st.tx_stream_active,
                st.codec_id,
                st.mac_address.clone(),
            )
        };
        let has_cap = rt.cap.has_capture_hw.load(Ordering::Acquire);
        let cap_id = rt.cap.meta.lock().unwrap().cap_device_id.clone();
        let flags = [
            u8::from(bt_live),
            u8::from(tx_act),
            u8::from(has_cap),
            cid,
        ];
        h = fnv1a_step(h, &flags);
        if bt_live {
            h = fnv1a_step(h, mac.as_bytes());
        }
        h = fnv1a_step(h, cap_id.as_bytes());
    }
    h
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_set_volume_impl(bt: *mut c_void, vol_pct: c_int, mute_flag: c_int) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    if vol_pct >= 0 {
        let clamped = vol_pct.clamp(0, 150);
        rt.desktop_volume_pct
            .store(clamped as u32, Ordering::Relaxed);
        rt.pending_avrcp_vol.store(clamped, Ordering::Relaxed);
        rt.need_avrcp_sync.store(true, Ordering::Release);
    }
    if mute_flag >= 0 {
        rt.desktop_muted.store(mute_flag != 0, Ordering::Relaxed);
    }
    0
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_set_mic_volume_impl(
    bt: *mut c_void,
    vol_pct: c_int,
    mute_flag: c_int,
) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    if vol_pct >= 0 {
        let clamped = vol_pct.clamp(0, 150) as u32;
        rt.cap.desktop_mic_volume_pct.store(clamped, Ordering::Relaxed);
        rt.cap
            .desktop_mic_gain_ppm
            .store(clamped * 10_000, Ordering::Relaxed);
    }
    if mute_flag >= 0 {
        rt.cap
            .desktop_mic_muted
            .store(mute_flag != 0, Ordering::Relaxed);
    }
    0
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_set_mic_gain_ppm_impl(
    bt: *mut c_void,
    gain_ppm: u32,
    vol_pct: c_int,
    mute_flag: c_int,
) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    let clamped_ppm = gain_ppm.min(2_000_000);
    rt.cap
        .desktop_mic_gain_ppm
        .store(clamped_ppm, Ordering::Relaxed);
    if vol_pct >= 0 {
        rt.cap
            .desktop_mic_volume_pct
            .store(vol_pct.clamp(0, 150) as u32, Ordering::Relaxed);
    }
    if mute_flag >= 0 {
        rt.cap
            .desktop_mic_muted
            .store(mute_flag != 0, Ordering::Relaxed);
    }
    0
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl` and
/// `cfg_ptr` must point to a valid `LwasapiBtCodecCfg`.
pub unsafe fn bluez_set_codec_config_impl(
    bt: *mut c_void,
    cfg_ptr: *const LwasapiBtCodecCfg,
) -> c_int {
    if bt.is_null() || cfg_ptr.is_null() {
        return -1;
    }
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    let cfg = unsafe { &*cfg_ptr };

    let need_reneg;
    let new_sco = cfg.hfp_sco_enabled != 0;
    {
        let mut st = rt.state.lock().unwrap();
        let old_pref = st.pref_codec;
        let new_pref = if cfg.preferred_codec <= LWASAPI_BT_PREF_SBC {
            cfg.preferred_codec
        } else {
            LWASAPI_BT_PREF_AUTO
        };
        let new_ldac = if cfg.ldac_mode <= LWASAPI_LDAC_MODE_330 {
            cfg.ldac_mode
        } else {
            LWASAPI_LDAC_MODE_ADAPTIVE
        };
        let new_abr = u32::from(cfg.wireless_abr_enabled != 0);
        let sup_mask = st.supported_codecs_mask;
        let has_link = st.has_bluez_transport;
        let mut preflight_ok = true;

        if new_pref != LWASAPI_BT_PREF_AUTO {
            let (want_cap, lib_ok, codec_label) = match new_pref {
                LWASAPI_BT_PREF_LDAC => (LWASAPI_BT_CAP_LDAC, load_ldac_library().is_some(), "LDAC"),
                LWASAPI_BT_PREF_APTX_HD => {
                    (LWASAPI_BT_CAP_APTX_HD, load_aptx_library().is_some(), "aptX HD")
                }
                LWASAPI_BT_PREF_APTX => (LWASAPI_BT_CAP_APTX, load_aptx_library().is_some(), "aptX"),
                LWASAPI_BT_PREF_AAC => (
                    LWASAPI_BT_CAP_AAC,
                    load_fdk_aac_library().is_some(),
                    "AAC",
                ),
                _ => (LWASAPI_BT_CAP_SBC, true, "SBC"),
            };
            let cur_name = match st.active_codec_kind {
                LWASAPI_BT_CODEC_LDAC => "LDAC",
                LWASAPI_BT_CODEC_APTX_HD => "aptX HD",
                LWASAPI_BT_CODEC_APTX => "aptX",
                LWASAPI_BT_CODEC_AAC => "AAC",
                _ => "SBC",
            };
            if !lib_ok {
                preflight_ok = false;
                st.neg_reason =
                    format!("{codec_label} host encoder library missing — keeping {cur_name}");
                dbg_emit(
                    LWASAPI_DBG_BLUEZ,
                    LWASAPI_DBG_WARN,
                    0,
                    0,
                    "BT_CODEC_PREFLIGHT",
                    codec_label,
                    "Rejected host-missing codec without disturbing BlueZ",
                );
            } else if has_link && sup_mask != 0 && (sup_mask & want_cap) == 0 {
                preflight_ok = false;
                st.neg_reason =
                    format!("Headset does not advertise {codec_label} SEP — keeping {cur_name}");
                dbg_emit(
                    LWASAPI_DBG_BLUEZ,
                    LWASAPI_DBG_WARN,
                    0,
                    0,
                    "BT_CODEC_PREFLIGHT",
                    codec_label,
                    "Headset lacks peer SEP for requested codec — active link preserved",
                );
            }
        }

        st.pref_codec = if preflight_ok { new_pref } else { old_pref };
        st.ldac_mode = new_ldac;
        st.wireless_abr_enabled = new_abr;
        st.hfp_sco_enabled = u32::from(new_sco);

        if preflight_ok
            && st.active_codec_kind == LWASAPI_BT_CODEC_LDAC
            && !st.enc.ldac_enc.is_null()
            && new_pref == old_pref
        {
            if new_ldac != LWASAPI_LDAC_MODE_ADAPTIVE {
                let eq = match new_ldac {
                    LWASAPI_LDAC_MODE_990 => LDACBT_EQMID_HQ,
                    LWASAPI_LDAC_MODE_660 => LDACBT_EQMID_SQ,
                    _ => LDACBT_EQMID_MQ,
                };
                if let Some(set_eqmid) = load_ldac_library().and_then(|l| l.set_eqmid) {
                    if unsafe { set_eqmid(st.enc.ldac_enc, eq) } == 0 {
                        st.enc.ldac_eqmid = eq;
                        st.effective_kbps = ldac_eqmid_to_kbps(st.bt_sample_rate, eq);
                        st.neg_reason =
                            format!("LDAC locked manually -> {}kbps", st.effective_kbps);
                    }
                }
            } else if new_abr != 0 {
                if st.enc.ldac_abr.is_null() {
                    if let Some(abr_api) = load_ldac_abr_library() {
                        let abr = unsafe { (abr_api.get_handle)() };
                        if !abr.is_null() {
                            unsafe {
                                let _ = (abr_api.init)(abr, 20);
                                let _ = (abr_api.set_thresholds)(abr, 5, 3, 1);
                            }
                            st.enc.ldac_abr = abr;
                        }
                    }
                }
                st.neg_reason = format!("LDAC Adaptive Bitrate active ({}kbps)", st.effective_kbps);
            }
        }

        let cur_pref_kind = match st.active_codec_kind {
            LWASAPI_BT_CODEC_LDAC => LWASAPI_BT_PREF_LDAC,
            LWASAPI_BT_CODEC_APTX_HD => LWASAPI_BT_PREF_APTX_HD,
            LWASAPI_BT_CODEC_APTX => LWASAPI_BT_PREF_APTX,
            LWASAPI_BT_CODEC_AAC => LWASAPI_BT_PREF_AAC,
            _ => LWASAPI_BT_PREF_SBC,
        };
        let mut target_pref = new_pref;
        if target_pref == LWASAPI_BT_PREF_AUTO && sup_mask != 0 {
            target_pref =
                if (sup_mask & LWASAPI_BT_CAP_LDAC) != 0 && load_ldac_library().is_some() {
                    LWASAPI_BT_PREF_LDAC
                } else if (sup_mask & LWASAPI_BT_CAP_APTX_HD) != 0 && load_aptx_library().is_some()
                {
                    LWASAPI_BT_PREF_APTX_HD
                } else if (sup_mask & LWASAPI_BT_CAP_APTX) != 0 && load_aptx_library().is_some() {
                    LWASAPI_BT_PREF_APTX
                } else if (sup_mask & LWASAPI_BT_CAP_AAC) != 0 && load_fdk_aac_library().is_some()
                {
                    LWASAPI_BT_PREF_AAC
                } else {
                    LWASAPI_BT_PREF_SBC
                };
        }
        need_reneg = preflight_ok && (new_pref != old_pref) && (target_pref != cur_pref_kind);
    }

    if !new_sco
        && rt
            .cap
            .meta
            .lock()
            .unwrap()
            .cap_device_id
            .starts_with("bluez_sco:")
    {
        stop_sco_capture(rt);
    }
    if need_reneg {
        rt.bt_renegotiate_req.store(true, Ordering::Release);
    }
    0
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl` and
/// `out_cfg` must point to writable storage for `LwasapiBtCodecCfg`.
pub unsafe fn bluez_get_codec_config_impl(
    bt: *const c_void,
    out_cfg: *mut LwasapiBtCodecCfg,
) {
    if out_cfg.is_null() {
        return;
    }
    let out = unsafe { &mut *out_cfg };
    *out = unsafe { mem::zeroed() };
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return;
    };
    let st = rt.state.lock().unwrap();
    out.preferred_codec = st.pref_codec;
    out.ldac_mode = st.ldac_mode;
    out.wireless_abr_enabled = st.wireless_abr_enabled;
    out.effective_kbps = st.effective_kbps;
    out.supported_codecs_mask = st.supported_codecs_mask;
    out.hfp_sco_enabled = st.hfp_sco_enabled;
    out.hfp_sco_active = rt.hfp_sco_active.load(Ordering::Acquire);
    copy_str_to_c(&mut out.bt_sco_codec, &st.bt_sco_codec);
    copy_str_to_c(&mut out.neg_reason, &st.neg_reason);
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_set_sco_enabled_impl(bt: *mut c_void, enabled: bool) -> c_int {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return -1;
    };
    rt.state.lock().unwrap().hfp_sco_enabled = u32::from(enabled);
    if !enabled {
        stop_sco_capture(rt);
    }
    0
}

/// # Safety
///
/// If non-null, `bt` must be a handle returned by `audiodg_bt_ptr_impl`.
pub unsafe fn bluez_close_impl(bt: *mut c_void) {
    let Some(rt) = (unsafe { bt_from_ptr(bt) }) else {
        return;
    };
    stop_capture_thread(&rt.cap);
    stop_sco_capture(rt);

    rt.tx_thread_running.store(false, Ordering::Release);
    if let Some(jh) = rt.tx_thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = jh.join();
    }
    rt.dbus_thread_running.store(false, Ordering::Release);
    if let Some(jh) = rt.dbus_thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = jh.join();
    }

    {
        let mut devs = rt.hfp_devs.lock().unwrap_or_else(|e| e.into_inner());
        for d in devs.iter_mut() {
            if d.fd >= 0 {
                shutdown_rdwr(d.fd);
                d.fd_owner = None;
                d.fd = -1;
            }
            d.rfcomm_active = false;
            d.in_use = false;
        }
    }

    {
        let mut st = rt.state.lock().unwrap_or_else(|e| e.into_inner());
        st.close_bt_fd();
        st.enc.close_all();
        st.direct_socket_mode = false;
        st.has_bluez_transport = false;
        st.tx_stream_active = false;
        sync_tx_ready(rt, &st);
    }
    rt.initialized.store(false, Ordering::Release);
}

/// # Safety
///
/// If non-null, `mac_or_empty` must point to a valid NUL-terminated C string,
/// and `out_msg` must point to writable storage of at least `out_len` bytes.
pub unsafe fn bluez_reset_device_impl(
    mac_or_empty: *const c_char,
    out_msg: *mut c_char,
    out_len: usize,
) -> c_int {
    let write_msg = |msg: &str| {
        if !out_msg.is_null() && out_len > 0 {
            let dst = unsafe { core::slice::from_raw_parts_mut(out_msg.cast::<u8>(), out_len) };
            copy_str_to_c(dst, msg);
        }
    };
    let Some(db) = load_dbus_library() else {
        write_msg("Failed to load libdbus-1.so.3");
        return -1;
    };
    let mut err = DBusError::default();
    unsafe { (db.error_init)(&mut err) };
    let conn = unsafe { (db.bus_get_private)(DBUS_BUS_SYSTEM, &mut err) };
    if conn.is_null() {
        let emsg = unsafe { dbus_error_msg(&err) };
        write_msg(&format!("Failed to connect to system D-Bus: {emsg}"));
        unsafe { (db.error_free)(&mut err) };
        return -1;
    }
    unsafe { (db.connection_set_exit_on_disconnect)(conn, 0) };

    let mac_filter = crate::c_abi::ptr_to_opt_str(mac_or_empty).unwrap_or_default();
    let want_mac = extract_mac_str(&mac_filter);

    let mut rc = -1i32;
    if let Some(objs) = fetch_managed_objects(db, conn, 2000) {
        let best = objs
            .devices
            .iter()
            .filter(|d| d.has_a2dp_sink && d.paired)
            .filter_map(|d| {
                let exact = want_mac
                    .as_deref()
                    .is_some_and(|w| d.address.eq_ignore_ascii_case(w));
                if want_mac.is_some() && !exact {
                    None
                } else {
                    let score = (if exact { 100 } else { 0 })
                        + (if d.connected { 10 } else { 0 })
                        + (if d.trusted { 2 } else { 0 });
                    Some((score, d))
                }
            })
            .max_by_key(|(s, _)| *s)
            .map(|(_, d)| d);

        if let Some(dev) = best {
            let label = if dev.alias.is_empty() {
                dev.address.as_str()
            } else {
                dev.alias.as_str()
            };
            if unsafe {
                dbus_get_bool_prop(db, conn, &dev.obj_path, "org.bluez.Device1", "Connected")
            }
            .unwrap_or(false)
            {
                let dmsg = dbus_new_method_call(
                    db,
                    "org.bluez",
                    &dev.obj_path,
                    "org.bluez.Device1",
                    "Disconnect",
                );
                if !dmsg.is_null() {
                    unsafe {
                        let drep = (db.connection_send_with_reply_and_block)(
                            conn,
                            dmsg,
                            5000,
                            ptr::null_mut(),
                        );
                        (db.message_unref)(dmsg);
                        if !drep.is_null() {
                            (db.message_unref)(drep);
                        }
                    }
                }
                for _ in 0..20 {
                    if !unsafe {
                        dbus_get_bool_prop(
                            db,
                            conn,
                            &dev.obj_path,
                            "org.bluez.Device1",
                            "Connected",
                        )
                    }
                    .unwrap_or(true)
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                thread::sleep(Duration::from_millis(400));
            }
            unsafe {
                dbus_set_str_prop(
                    db,
                    conn,
                    &dev.obj_path,
                    "org.bluez.Device1",
                    "PreferredBearer",
                    "bredr",
                );
            }
            let mut cerr = DBusError::default();
            unsafe { (db.error_init)(&mut cerr) };
            let cmsg = dbus_new_method_call(
                db,
                "org.bluez",
                &dev.obj_path,
                "org.bluez.Device1",
                "Connect",
            );
            let mut key_missing = false;
            let mut conn_failed: Option<String> = None;
            if !cmsg.is_null() {
                unsafe {
                    let crep =
                        (db.connection_send_with_reply_and_block)(conn, cmsg, 12_000, &mut cerr);
                    (db.message_unref)(cmsg);
                    if crep.is_null() {
                        let em = dbus_error_msg(&cerr);
                        if em.contains("br-connection-key-missing") {
                            key_missing = true;
                            rc = -2;
                        } else if !em.contains("AlreadyConnected") && !em.contains("InProgress") {
                            conn_failed = Some(em);
                        }
                    } else {
                        (db.message_unref)(crep);
                    }
                    (db.error_free)(&mut cerr);
                }
            }
            if key_missing {
                write_msg(&format!(
                    "Bluetooth link key mismatch (br-connection-key-missing) for {label}. Please re-pair headphones."
                ));
            } else if let Some(em) = conn_failed {
                write_msg(&format!("Failed to connect {label}: {em}"));
            } else {
                let mut conn_ok = false;
                for _ in 0..24 {
                    if unsafe {
                        dbus_get_bool_prop(db, conn, &dev.obj_path, "org.bluez.Device1", "Connected")
                    }
                    .unwrap_or(false)
                    {
                        conn_ok = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                if !conn_ok {
                    write_msg(&format!(
                        "Failed to connect {label}. Ensure headphones are powered on."
                    ));
                } else {
                    let mut tp_ok = false;
                    for _ in 0..30 {
                        if let Some(m2) = fetch_managed_objects(db, conn, 1500) {
                            if m2.transports.iter().any(|t| {
                                t.device_path == dev.obj_path
                                    && (t.state == "active" || t.state == "idle")
                            }) {
                                tp_ok = true;
                                break;
                            }
                        }
                        thread::sleep(Duration::from_millis(200));
                    }
                    if tp_ok {
                        rc = 0;
                        write_msg(&format!(
                            "{} ({}) reconnected on BR/EDR A2DP.",
                            if dev.alias.is_empty() {
                                "Bluetooth device"
                            } else {
                                dev.alias.as_str()
                            },
                            dev.address
                        ));
                    } else {
                        write_msg(&format!(
                            "{label} connected on BR/EDR, but A2DP audio transport is still initializing."
                        ));
                    }
                }
            }
        } else if !mac_filter.is_empty() {
            write_msg(&format!(
                "No paired Bluetooth A2DP sink found matching '{mac_filter}'."
            ));
        } else {
            write_msg("No paired Bluetooth A2DP sink device found in BlueZ.");
        }
    } else if !mac_filter.is_empty() {
        write_msg(&format!(
            "No paired Bluetooth A2DP sink found matching '{mac_filter}'."
        ));
    } else {
        write_msg("No paired Bluetooth A2DP sink device found in BlueZ.");
    }

    unsafe {
        (db.connection_close)(conn);
        (db.connection_unref)(conn);
    }
    rc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_tx_and_snapshot_smoke() {
        let rt = BluezRuntime::new();
        let p = ((&rt) as *const BluezRuntime as *mut BluezRuntime).cast::<c_void>();
        unsafe {
            assert_eq!(bluez_set_volume_impl(p, 80, 0), 0);
            assert_eq!(bluez_set_mic_gain_ppm_impl(p, 1_500_000, 120, 0), 0);
            let mut snap: SalsaBtSnapshot = mem::zeroed();
            bt_get_snapshot_impl(p, &mut snap);
            assert_eq!(snap.desktop_volume_pct, 80);
            assert_eq!(snap.desktop_mic_volume_pct, 120);
            assert_eq!(snap.desktop_mic_gain_ppm, 1_500_000);
            let mut vol = 0u32;
            let mut muted = true;
            bt_rt_master_vol_impl(p, &mut vol, &mut muted);
            assert_eq!(vol, 80);
            assert!(!muted);
        }
    }

    #[test]
    fn partial_pop_recovery_and_enable_preserve_queue() {
        assert!(!should_reprebuffer_after_partial_pop(767, 1024, 1));
        assert!(!should_reprebuffer_after_partial_pop(767, 1024, 2));
        assert!(should_reprebuffer_after_partial_pop(767, 1024, 3));
        assert!(should_reprebuffer_after_partial_pop(400, 1024, 1));
        assert!(should_reprebuffer_after_partial_pop(0, 1024, 1));

        let rt = BluezRuntime::new();
        rt.initialized.store(true, Ordering::Release);
        let p = (&rt as *const BluezRuntime as *mut BluezRuntime).cast::<c_void>();
        let pcm = [0.25f32; 128 * 2];

        // Lock-free tx_transport_ready gates period writes without locking rt.state
        assert_eq!(
            unsafe { bluez_write_period_impl(p, pcm.as_ptr(), 128, 2) },
            -1
        );
        rt.tx_transport_ready.store(true, Ordering::Release);
        assert_eq!(
            unsafe { bluez_write_period_impl(p, pcm.as_ptr(), 128, 2) },
            0
        );
        assert_eq!(rt.tx_write_frame.load(Ordering::Acquire), 128);

        // Re-enabling TX when already unpaused must not flush queued audio
        set_tx_enabled_impl(&rt, true);
        assert_eq!(rt.tx_read_frame.load(Ordering::Acquire), 0);

        // Resuming from a real pause advances read cursor cleanly past stale frames
        set_tx_enabled_impl(&rt, false);
        set_tx_enabled_impl(&rt, true);
        assert_eq!(rt.tx_read_frame.load(Ordering::Acquire), 128);
    }
}
