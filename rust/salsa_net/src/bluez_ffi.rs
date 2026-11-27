// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(clippy::missing_transmute_annotations)]

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use core::mem;
use core::ptr;
use std::ffi::{CStr, CString};
use std::sync::OnceLock;

use salsa_host_dl::open_verified_optional_lib;

use crate::unix_sys::dl_sym;

fn open_optional_host_so(sonames: &[&str]) -> *mut c_void {
    unsafe { open_verified_optional_lib(sonames, open_optional_host_so as *const c_void) }
}

// ============================================================================
// Bluetooth Kernel Socket & Codec ABI Constants
// ============================================================================

pub const AF_BLUETOOTH: c_int = 31;
pub const BTPROTO_SCO: c_int = 2;
pub const SOL_BLUETOOTH: c_int = 274;
pub const BT_VOICE: c_int = 11;
pub const BT_VOICE_TRANSPARENT: u16 = 0x0003;
pub const BT_VOICE_CVSD_16BIT: u16 = 0x0060;

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct BdAddr {
    pub b: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SockaddrSco {
    pub sco_family: u16,
    pub sco_bdaddr: BdAddr,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BtVoice {
    pub setting: u16,
}

// ============================================================================
// libsbc.so.1 ABI
// ============================================================================

pub const SBC_LE: u8 = 0x00;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SbcStruct {
    pub flags: c_ulong,
    pub frequency: u8,
    pub blocks: u8,
    pub subbands: u8,
    pub mode: u8,
    pub allocation: u8,
    pub bitpool: u8,
    pub endian: u8,
    pub priv_data: *mut c_void,
    pub priv_alloc_base: *mut c_void,
}

impl Default for SbcStruct {
    fn default() -> Self {
        unsafe { mem::zeroed() }
    }
}

pub struct SbcApi {
    pub init_a2dp: unsafe extern "C" fn(*mut SbcStruct, c_ulong, *const c_void, usize) -> c_int,
    pub init_msbc: unsafe extern "C" fn(*mut SbcStruct, c_ulong) -> c_int,
    pub decode: unsafe extern "C" fn(
        *mut SbcStruct,
        *const c_void,
        usize,
        *mut c_void,
        usize,
        *mut usize,
    ) -> isize,
    pub encode: unsafe extern "C" fn(
        *mut SbcStruct,
        *const c_void,
        usize,
        *mut c_void,
        usize,
        *mut isize,
    ) -> isize,
    pub get_codesize: unsafe extern "C" fn(*mut SbcStruct) -> usize,
    pub finish: unsafe extern "C" fn(*mut SbcStruct),
}

// SAFETY: Resolved ELF function pointers inside libsbc.so.1 are immutable and thread-safe.
unsafe impl Send for SbcApi {}
unsafe impl Sync for SbcApi {}

static SBC_LIB: OnceLock<Option<SbcApi>> = OnceLock::new();

#[allow(clippy::missing_transmute_annotations)]
pub fn load_sbc_library() -> Option<&'static SbcApi> {
    SBC_LIB
        .get_or_init(|| unsafe {
            let h = open_optional_host_so(&["libsbc.so.1"]);
            if h.is_null() {
                return None;
            }
            let init_a2dp = dl_sym(h, "sbc_init_a2dp");
            let init_msbc = dl_sym(h, "sbc_init_msbc");
            let decode = dl_sym(h, "sbc_decode");
            let encode = dl_sym(h, "sbc_encode");
            let get_codesize = dl_sym(h, "sbc_get_codesize");
            let finish = dl_sym(h, "sbc_finish");
            if init_a2dp.is_null()
                || init_msbc.is_null()
                || decode.is_null()
                || encode.is_null()
                || get_codesize.is_null()
                || finish.is_null()
            {
                return None;
            }
            Some(SbcApi {
                init_a2dp: mem::transmute::<*mut c_void, _>(init_a2dp),
                init_msbc: mem::transmute::<*mut c_void, _>(init_msbc),
                decode: mem::transmute::<*mut c_void, _>(decode),
                encode: mem::transmute::<*mut c_void, _>(encode),
                get_codesize: mem::transmute::<*mut c_void, _>(get_codesize),
                finish: mem::transmute::<*mut c_void, _>(finish),
            })
        })
        .as_ref()
}

// ============================================================================
// libfdk-aac.so.2, libldacBT_enc.so.2, libldacBT_abr.so.2, libfreeaptx.so.0 ABI
// ============================================================================

pub const FDK_AACENC_OK: c_int = 0x0000;
pub const FDK_AOT_AAC_LC: c_uint = 2;
pub const FDK_AOT_MP2_AAC_LC: c_uint = 129;
pub const FDK_TT_MP4_LATM_MCP1: c_uint = 6;
pub const FDK_IN_AUDIO_DATA: c_int = 0;
pub const FDK_OUT_BITSTREAM_DATA: c_int = 3;

pub const FDK_PARAM_AOT: c_uint = 0x0100;
pub const FDK_PARAM_BITRATE: c_uint = 0x0101;
pub const FDK_PARAM_BITRATEMODE: c_uint = 0x0102;
pub const FDK_PARAM_SAMPLERATE: c_uint = 0x0103;
pub const FDK_PARAM_CHANNELMODE: c_uint = 0x0106;
pub const FDK_PARAM_AFTERBURNER: c_uint = 0x0200;
pub const FDK_PARAM_BANDWIDTH: c_uint = 0x0203;
pub const FDK_PARAM_PEAK_BITRATE: c_uint = 0x0207;
pub const FDK_PARAM_TRANSMUX: c_uint = 0x0300;
pub const FDK_PARAM_HEADER_PERIOD: c_uint = 0x0301;
pub const FDK_PARAM_SIGNALING_MODE: c_uint = 0x0302;
pub const FDK_PARAM_AUDIOMUXVER: c_uint = 0x0304;

pub const LDACBT_ENC_LSU: usize = 128;
pub const LDACBT_EQMID_HQ: c_int = 0;
pub const LDACBT_EQMID_SQ: c_int = 1;
pub const LDACBT_EQMID_MQ: c_int = 2;
pub const LDACBT_SMPL_FMT_S16: c_int = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FdkInfo {
    pub max_out_buf_bytes: c_uint,
    pub max_anc_bytes: c_uint,
    pub in_buf_fill_level: c_uint,
    pub input_channels: c_uint,
    pub frame_length: c_uint,
    pub n_delay: c_uint,
    pub n_delay_core: c_uint,
    pub conf_buf: [u8; 64],
    pub conf_size: c_uint,
}

impl Default for FdkInfo {
    fn default() -> Self {
        unsafe { mem::zeroed() }
    }
}

#[repr(C)]
pub struct FdkBufDesc {
    pub num_bufs: c_int,
    pub bufs: *mut *mut c_void,
    pub buffer_identifiers: *mut c_int,
    pub buf_sizes: *mut c_int,
    pub buf_el_sizes: *mut c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FdkInArgs {
    pub num_in_samples: c_int,
    pub num_anc_bytes: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FdkOutArgs {
    pub num_out_bytes: c_int,
    pub num_in_samples: c_int,
    pub num_anc_bytes: c_int,
    pub bit_res_state: c_int,
}

pub struct FdkApi {
    pub open: unsafe extern "C" fn(*mut *mut c_void, c_uint, c_uint) -> c_int,
    pub close: unsafe extern "C" fn(*mut *mut c_void) -> c_int,
    pub set_param: unsafe extern "C" fn(*mut c_void, c_uint, c_uint) -> c_int,
    pub info: unsafe extern "C" fn(*mut c_void, *mut FdkInfo) -> c_int,
    pub encode: unsafe extern "C" fn(
        *mut c_void,
        *const FdkBufDesc,
        *const FdkBufDesc,
        *const FdkInArgs,
        *mut FdkOutArgs,
    ) -> c_int,
}

unsafe impl Send for FdkApi {}
unsafe impl Sync for FdkApi {}

static FDK_LIB: OnceLock<Option<FdkApi>> = OnceLock::new();

pub fn load_fdk_aac_library() -> Option<&'static FdkApi> {
    FDK_LIB
        .get_or_init(|| unsafe {
            let h = open_optional_host_so(&["libfdk-aac.so.2"]);
            if h.is_null() {
                return None;
            }
            let open = dl_sym(h, "aacEncOpen");
            let close = dl_sym(h, "aacEncClose");
            let set_param = dl_sym(h, "aacEncoder_SetParam");
            let info = dl_sym(h, "aacEncInfo");
            let encode = dl_sym(h, "aacEncEncode");
            if open.is_null()
                || close.is_null()
                || set_param.is_null()
                || info.is_null()
                || encode.is_null()
            {
                return None;
            }
            Some(FdkApi {
                open: mem::transmute::<*mut c_void, _>(open),
                close: mem::transmute::<*mut c_void, _>(close),
                set_param: mem::transmute::<*mut c_void, _>(set_param),
                info: mem::transmute::<*mut c_void, _>(info),
                encode: mem::transmute::<*mut c_void, _>(encode),
            })
        })
        .as_ref()
}

pub struct LdacApi {
    pub get_handle: unsafe extern "C" fn() -> *mut c_void,
    pub free_handle: unsafe extern "C" fn(*mut c_void),
    pub init_encode: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int, c_int) -> c_int,
    pub encode: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_int,
        *mut u8,
        *mut c_int,
        *mut c_int,
    ) -> c_int,
    pub set_eqmid: Option<unsafe extern "C" fn(*mut c_void, c_int) -> c_int>,
}

unsafe impl Send for LdacApi {}
unsafe impl Sync for LdacApi {}

static LDAC_LIB: OnceLock<Option<LdacApi>> = OnceLock::new();

pub fn load_ldac_library() -> Option<&'static LdacApi> {
    let api = LDAC_LIB
        .get_or_init(|| unsafe {
            let h = open_optional_host_so(&["libldacBT_enc.so.2"]);
            if h.is_null() {
                return None;
            }
            let get_handle = dl_sym(h, "ldacBT_get_handle");
            let free_handle = dl_sym(h, "ldacBT_free_handle");
            let init_encode = dl_sym(h, "ldacBT_init_handle_encode");
            let encode = dl_sym(h, "ldacBT_encode");
            if get_handle.is_null()
                || free_handle.is_null()
                || init_encode.is_null()
                || encode.is_null()
            {
                return None;
            }
            let set_eqmid_p = dl_sym(h, "ldacBT_set_eqmid");
            Some(LdacApi {
                get_handle: mem::transmute::<*mut c_void, _>(get_handle),
                free_handle: mem::transmute::<*mut c_void, _>(free_handle),
                init_encode: mem::transmute::<*mut c_void, _>(init_encode),
                encode: mem::transmute::<*mut c_void, _>(encode),
                set_eqmid: if set_eqmid_p.is_null() {
                    None
                } else {
                    Some(mem::transmute::<*mut c_void, _>(set_eqmid_p))
                },
            })
        })
        .as_ref();
    if api.is_some() {
        let _ = load_ldac_abr_library();
    }
    api
}

pub struct LdacAbrApi {
    pub get_handle: unsafe extern "C" fn() -> *mut c_void,
    pub free_handle: unsafe extern "C" fn(*mut c_void),
    pub init: unsafe extern "C" fn(*mut c_void, c_uint) -> c_int,
    pub set_thresholds: unsafe extern "C" fn(*mut c_void, c_uint, c_uint, c_uint) -> c_int,
    pub proc_step: unsafe extern "C" fn(*mut c_void, *mut c_void, c_uint, c_uint) -> c_int,
}

unsafe impl Send for LdacAbrApi {}
unsafe impl Sync for LdacAbrApi {}

static LDAC_ABR_LIB: OnceLock<Option<LdacAbrApi>> = OnceLock::new();

pub fn load_ldac_abr_library() -> Option<&'static LdacAbrApi> {
    LDAC_ABR_LIB
        .get_or_init(|| unsafe {
            let h = open_optional_host_so(&["libldacBT_abr.so.2"]);
            if h.is_null() {
                return None;
            }
            let get_handle = dl_sym(h, "ldac_ABR_get_handle");
            let free_handle = dl_sym(h, "ldac_ABR_free_handle");
            let init = dl_sym(h, "ldac_ABR_Init");
            let set_thresholds = dl_sym(h, "ldac_ABR_set_thresholds");
            let proc_step = dl_sym(h, "ldac_ABR_Proc");
            if get_handle.is_null()
                || free_handle.is_null()
                || init.is_null()
                || set_thresholds.is_null()
                || proc_step.is_null()
            {
                return None;
            }
            Some(LdacAbrApi {
                get_handle: mem::transmute::<*mut c_void, _>(get_handle),
                free_handle: mem::transmute::<*mut c_void, _>(free_handle),
                init: mem::transmute::<*mut c_void, _>(init),
                set_thresholds: mem::transmute::<*mut c_void, _>(set_thresholds),
                proc_step: mem::transmute::<*mut c_void, _>(proc_step),
            })
        })
        .as_ref()
}

pub struct AptxApi {
    pub init: unsafe extern "C" fn(c_int) -> *mut c_void,
    pub finish: unsafe extern "C" fn(*mut c_void),
    pub encode:
        unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut u8, usize, *mut usize) -> usize,
}

unsafe impl Send for AptxApi {}
unsafe impl Sync for AptxApi {}

static APTX_LIB: OnceLock<Option<AptxApi>> = OnceLock::new();

pub fn load_aptx_library() -> Option<&'static AptxApi> {
    APTX_LIB
        .get_or_init(|| unsafe {
            let h = open_optional_host_so(&["libfreeaptx.so.0"]);
            if h.is_null() {
                return None;
            }
            let init = dl_sym(h, "aptx_init");
            let finish = dl_sym(h, "aptx_finish");
            let encode = dl_sym(h, "aptx_encode");
            if init.is_null() || finish.is_null() || encode.is_null() {
                return None;
            }
            Some(AptxApi {
                init: mem::transmute::<*mut c_void, _>(init),
                finish: mem::transmute::<*mut c_void, _>(finish),
                encode: mem::transmute::<*mut c_void, _>(encode),
            })
        })
        .as_ref()
}

// ============================================================================
// libdbus-1.so.3 ABI (Strictly Non-Variadic DBusMessageIter Calls)
// ============================================================================

pub const DBUS_BUS_SYSTEM: c_int = 1;
pub const DBUS_MESSAGE_TYPE_ERROR: c_int = 3;
pub const DBUS_MESSAGE_TYPE_SIGNAL: c_int = 4;
pub const DBUS_HANDLER_RESULT_HANDLED: c_int = 0;
pub const DBUS_HANDLER_RESULT_NOT_YET_HANDLED: c_int = 1;

pub const DBUS_TYPE_INVALID: c_int = 0;
pub const DBUS_TYPE_BYTE: c_int = b'y' as c_int;
pub const DBUS_TYPE_BOOLEAN: c_int = b'b' as c_int;
pub const DBUS_TYPE_UINT16: c_int = b'q' as c_int;
pub const DBUS_TYPE_STRING: c_int = b's' as c_int;
pub const DBUS_TYPE_OBJECT_PATH: c_int = b'o' as c_int;
pub const DBUS_TYPE_UNIX_FD: c_int = b'h' as c_int;
pub const DBUS_TYPE_ARRAY: c_int = b'a' as c_int;
pub const DBUS_TYPE_VARIANT: c_int = b'v' as c_int;
pub const DBUS_TYPE_DICT_ENTRY: c_int = b'e' as c_int;

pub type DBusConnection = c_void;
pub type DBusMessage = c_void;
pub type DBusPendingCall = c_void;
pub type DBusHandlerResult = c_int;

#[repr(C)]
pub struct DBusError {
    pub name: *const c_char,
    pub message: *const c_char,
    pub dummy_bits: c_uint,
    pub _pad: c_uint,
    pub padding1: *mut c_void,
}

impl Default for DBusError {
    fn default() -> Self {
        Self {
            name: ptr::null(),
            message: ptr::null(),
            dummy_bits: 0,
            _pad: 0,
            padding1: ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DBusMessageIter {
    pub dummy1: *mut c_void,
    pub dummy2: *mut c_void,
    pub dummy3: u32,
    pub dummy4: c_int,
    pub dummy5: c_int,
    pub dummy6: c_int,
    pub dummy7: c_int,
    pub dummy8: c_int,
    pub dummy9: c_int,
    pub dummy10: c_int,
    pub dummy11: c_int,
    pub pad1: c_int,
    pub pad2: *mut c_void,
    pub pad3: *mut c_void,
}

impl Default for DBusMessageIter {
    fn default() -> Self {
        unsafe { mem::zeroed() }
    }
}

pub type DBusObjectPathUnregisterFn = Option<unsafe extern "C" fn(*mut DBusConnection, *mut c_void)>;
pub type DBusMessageHandlerFn = Option<
    unsafe extern "C" fn(*mut DBusConnection, *mut DBusMessage, *mut c_void) -> DBusHandlerResult,
>;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DBusObjectPathVTable {
    pub unregister_function: DBusObjectPathUnregisterFn,
    pub message_function: DBusMessageHandlerFn,
    pub pad1: Option<unsafe extern "C" fn(*mut c_void)>,
    pub pad2: Option<unsafe extern "C" fn(*mut c_void)>,
    pub pad3: Option<unsafe extern "C" fn(*mut c_void)>,
    pub pad4: Option<unsafe extern "C" fn(*mut c_void)>,
}

pub struct DBusApi {
    pub threads_init_default: unsafe extern "C" fn() -> c_uint,
    pub error_init: unsafe extern "C" fn(*mut DBusError),
    pub error_free: unsafe extern "C" fn(*mut DBusError),
    pub bus_get: unsafe extern "C" fn(c_int, *mut DBusError) -> *mut DBusConnection,
    pub bus_get_private: unsafe extern "C" fn(c_int, *mut DBusError) -> *mut DBusConnection,
    pub bus_add_match: unsafe extern "C" fn(*mut DBusConnection, *const c_char, *mut DBusError),
    pub connection_set_exit_on_disconnect: unsafe extern "C" fn(*mut DBusConnection, c_uint),
    pub connection_close: unsafe extern "C" fn(*mut DBusConnection),
    pub connection_ref: unsafe extern "C" fn(*mut DBusConnection) -> *mut DBusConnection,
    pub connection_unref: unsafe extern "C" fn(*mut DBusConnection),
    pub connection_flush: unsafe extern "C" fn(*mut DBusConnection),
    pub connection_read_write_dispatch: unsafe extern "C" fn(*mut DBusConnection, c_int) -> c_uint,
    pub connection_send:
        unsafe extern "C" fn(*mut DBusConnection, *mut DBusMessage, *mut u32) -> c_uint,
    pub connection_send_with_reply: unsafe extern "C" fn(
        *mut DBusConnection,
        *mut DBusMessage,
        *mut *mut DBusPendingCall,
        c_int,
    ) -> c_uint,
    pub connection_send_with_reply_and_block: unsafe extern "C" fn(
        *mut DBusConnection,
        *mut DBusMessage,
        c_int,
        *mut DBusError,
    ) -> *mut DBusMessage,
    pub connection_add_filter: unsafe extern "C" fn(
        *mut DBusConnection,
        DBusMessageHandlerFn,
        *mut c_void,
        *const c_void,
    ) -> c_uint,
    pub connection_remove_filter:
        unsafe extern "C" fn(*mut DBusConnection, DBusMessageHandlerFn, *mut c_void),
    pub connection_register_object_path: unsafe extern "C" fn(
        *mut DBusConnection,
        *const c_char,
        *const DBusObjectPathVTable,
        *mut c_void,
    ) -> c_uint,
    pub connection_register_fallback: unsafe extern "C" fn(
        *mut DBusConnection,
        *const c_char,
        *const DBusObjectPathVTable,
        *mut c_void,
    ) -> c_uint,
    pub connection_unregister_object_path:
        unsafe extern "C" fn(*mut DBusConnection, *const c_char) -> c_uint,
    pub pending_call_get_completed: unsafe extern "C" fn(*mut DBusPendingCall) -> c_uint,
    pub pending_call_steal_reply: unsafe extern "C" fn(*mut DBusPendingCall) -> *mut DBusMessage,
    pub pending_call_cancel: unsafe extern "C" fn(*mut DBusPendingCall),
    pub pending_call_unref: unsafe extern "C" fn(*mut DBusPendingCall),
    pub message_new_method_call: unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> *mut DBusMessage,
    pub message_new_signal:
        unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut DBusMessage,
    pub message_new_method_return: unsafe extern "C" fn(*mut DBusMessage) -> *mut DBusMessage,
    pub message_new_error:
        unsafe extern "C" fn(*mut DBusMessage, *const c_char, *const c_char) -> *mut DBusMessage,
    pub message_unref: unsafe extern "C" fn(*mut DBusMessage),
    pub message_get_type: unsafe extern "C" fn(*mut DBusMessage) -> c_int,
    pub message_get_path: unsafe extern "C" fn(*mut DBusMessage) -> *const c_char,
    pub message_get_interface: unsafe extern "C" fn(*mut DBusMessage) -> *const c_char,
    pub message_get_member: unsafe extern "C" fn(*mut DBusMessage) -> *const c_char,
    pub message_iter_init: unsafe extern "C" fn(*mut DBusMessage, *mut DBusMessageIter) -> c_uint,
    pub message_iter_init_append: unsafe extern "C" fn(*mut DBusMessage, *mut DBusMessageIter),
    pub message_iter_next: unsafe extern "C" fn(*mut DBusMessageIter) -> c_uint,
    pub message_iter_get_arg_type: unsafe extern "C" fn(*mut DBusMessageIter) -> c_int,
    pub message_iter_recurse: unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter),
    pub message_iter_get_basic: unsafe extern "C" fn(*mut DBusMessageIter, *mut c_void),
    pub message_iter_get_fixed_array:
        unsafe extern "C" fn(*mut DBusMessageIter, *mut c_void, *mut c_int),
    pub message_iter_open_container: unsafe extern "C" fn(
        *mut DBusMessageIter,
        c_int,
        *const c_char,
        *mut DBusMessageIter,
    ) -> c_uint,
    pub message_iter_close_container:
        unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter) -> c_uint,
    pub message_iter_append_basic:
        unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_void) -> c_uint,
    pub message_iter_append_fixed_array:
        unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_void, c_int) -> c_uint,
}

unsafe impl Send for DBusApi {}
unsafe impl Sync for DBusApi {}

static DBUS_LIB: OnceLock<Option<DBusApi>> = OnceLock::new();

pub fn load_dbus_library() -> Option<&'static DBusApi> {
    DBUS_LIB
        .get_or_init(|| unsafe {
            let h = open_optional_host_so(&["libdbus-1.so.3"]);
            if h.is_null() {
                return None;
            }
            macro_rules! sym {
                ($name:expr) => {{
                    let p = dl_sym(h, $name);
                    if p.is_null() {
                        return None;
                    }
                    mem::transmute::<*mut c_void, _>(p)
                }};
            }
            Some(DBusApi {
                threads_init_default: sym!("dbus_threads_init_default"),
                error_init: sym!("dbus_error_init"),
                error_free: sym!("dbus_error_free"),
                bus_get: sym!("dbus_bus_get"),
                bus_get_private: sym!("dbus_bus_get_private"),
                bus_add_match: sym!("dbus_bus_add_match"),
                connection_set_exit_on_disconnect: sym!("dbus_connection_set_exit_on_disconnect"),
                connection_close: sym!("dbus_connection_close"),
                connection_ref: sym!("dbus_connection_ref"),
                connection_unref: sym!("dbus_connection_unref"),
                connection_flush: sym!("dbus_connection_flush"),
                connection_read_write_dispatch: sym!("dbus_connection_read_write_dispatch"),
                connection_send: sym!("dbus_connection_send"),
                connection_send_with_reply: sym!("dbus_connection_send_with_reply"),
                connection_send_with_reply_and_block: sym!(
                    "dbus_connection_send_with_reply_and_block"
                ),
                connection_add_filter: sym!("dbus_connection_add_filter"),
                connection_remove_filter: sym!("dbus_connection_remove_filter"),
                connection_register_object_path: sym!("dbus_connection_register_object_path"),
                connection_register_fallback: sym!("dbus_connection_register_fallback"),
                connection_unregister_object_path: sym!("dbus_connection_unregister_object_path"),
                pending_call_get_completed: sym!("dbus_pending_call_get_completed"),
                pending_call_steal_reply: sym!("dbus_pending_call_steal_reply"),
                pending_call_cancel: sym!("dbus_pending_call_cancel"),
                pending_call_unref: sym!("dbus_pending_call_unref"),
                message_new_method_call: sym!("dbus_message_new_method_call"),
                message_new_signal: sym!("dbus_message_new_signal"),
                message_new_method_return: sym!("dbus_message_new_method_return"),
                message_new_error: sym!("dbus_message_new_error"),
                message_unref: sym!("dbus_message_unref"),
                message_get_type: sym!("dbus_message_get_type"),
                message_get_path: sym!("dbus_message_get_path"),
                message_get_interface: sym!("dbus_message_get_interface"),
                message_get_member: sym!("dbus_message_get_member"),
                message_iter_init: sym!("dbus_message_iter_init"),
                message_iter_init_append: sym!("dbus_message_iter_init_append"),
                message_iter_next: sym!("dbus_message_iter_next"),
                message_iter_get_arg_type: sym!("dbus_message_iter_get_arg_type"),
                message_iter_recurse: sym!("dbus_message_iter_recurse"),
                message_iter_get_basic: sym!("dbus_message_iter_get_basic"),
                message_iter_get_fixed_array: sym!("dbus_message_iter_get_fixed_array"),
                message_iter_open_container: sym!("dbus_message_iter_open_container"),
                message_iter_close_container: sym!("dbus_message_iter_close_container"),
                message_iter_append_basic: sym!("dbus_message_iter_append_basic"),
                message_iter_append_fixed_array: sym!("dbus_message_iter_append_fixed_array"),
            })
        })
        .as_ref()
}

// ============================================================================
// Safe Non-Variadic D-Bus Helpers
// ============================================================================

/// # Safety
///
/// If non-null, `p` must point to a valid NUL-terminated C string that remains
/// valid for `'a`.
pub unsafe fn cptr_to_opt_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(p) }.to_str().ok()
    }
}

pub fn is_valid_dbus_object_path(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    if !path.starts_with('/') || path.ends_with('/') {
        return false;
    }
    path[1..].split('/').all(|seg| {
        !seg.is_empty()
            && seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

/// # Safety
///
/// `it` must point to a valid initialized `DBusMessageIter`.
pub unsafe fn iter_get_str(db: &DBusApi, it: *mut DBusMessageIter) -> Option<String> {
    let t = unsafe { (db.message_iter_get_arg_type)(it) };
    if t != DBUS_TYPE_STRING && t != DBUS_TYPE_OBJECT_PATH {
        return None;
    }
    let mut p: *const c_char = ptr::null();
    unsafe { (db.message_iter_get_basic)(it, (&mut p as *mut *const c_char).cast::<c_void>()) };
    if p.is_null() {
        None
    } else {
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        if t == DBUS_TYPE_OBJECT_PATH && !is_valid_dbus_object_path(&s) {
            return None;
        }
        Some(s)
    }
}

/// # Safety
///
/// `it` must point to a valid initialized `DBusMessageIter`.
pub unsafe fn iter_get_bool(db: &DBusApi, it: *mut DBusMessageIter) -> Option<bool> {
    if unsafe { (db.message_iter_get_arg_type)(it) } != DBUS_TYPE_BOOLEAN {
        return None;
    }
    let mut b: u32 = 0;
    unsafe { (db.message_iter_get_basic)(it, (&mut b as *mut u32).cast::<c_void>()) };
    Some(b != 0)
}

/// # Safety
///
/// `it` must point to a valid initialized `DBusMessageIter`.
pub unsafe fn iter_get_u16(db: &DBusApi, it: *mut DBusMessageIter) -> Option<u16> {
    if unsafe { (db.message_iter_get_arg_type)(it) } != DBUS_TYPE_UINT16 {
        return None;
    }
    let mut v: u16 = 0;
    unsafe { (db.message_iter_get_basic)(it, (&mut v as *mut u16).cast::<c_void>()) };
    Some(v)
}

/// # Safety
///
/// `it` must point to a valid initialized `DBusMessageIter`.
pub unsafe fn iter_get_byte(db: &DBusApi, it: *mut DBusMessageIter) -> Option<u8> {
    if unsafe { (db.message_iter_get_arg_type)(it) } != DBUS_TYPE_BYTE {
        return None;
    }
    let mut v: u8 = 0;
    unsafe { (db.message_iter_get_basic)(it, (&mut v as *mut u8).cast::<c_void>()) };
    Some(v)
}

/// # Safety
///
/// `it` must point to a valid initialized `DBusMessageIter`.
pub unsafe fn iter_get_unix_fd(db: &DBusApi, it: *mut DBusMessageIter) -> Option<c_int> {
    if unsafe { (db.message_iter_get_arg_type)(it) } != DBUS_TYPE_UNIX_FD {
        return None;
    }
    let mut fd: c_int = -1;
    unsafe { (db.message_iter_get_basic)(it, (&mut fd as *mut c_int).cast::<c_void>()) };
    Some(fd)
}

/// # Safety
///
/// `it` must point to a valid initialized `DBusMessageIter`.
pub unsafe fn iter_get_byte_array(db: &DBusApi, it: *mut DBusMessageIter) -> Option<Vec<u8>> {
    if unsafe { (db.message_iter_get_arg_type)(it) } != DBUS_TYPE_ARRAY {
        return None;
    }
    let mut arr = DBusMessageIter::default();
    let mut raw: *const u8 = ptr::null();
    let mut n: c_int = 0;
    unsafe {
        (db.message_iter_recurse)(it, &mut arr);
        (db.message_iter_get_fixed_array)(
            &mut arr,
            (&mut raw as *mut *const u8).cast::<c_void>(),
            &mut n,
        );
        if raw.is_null() || n <= 0 {
            return Some(Vec::new());
        }
        Some(core::slice::from_raw_parts(raw, n as usize).to_vec())
    }
}

/// # Safety
///
/// `it` must point to a valid writable append `DBusMessageIter`.
pub unsafe fn iter_append_str(db: &DBusApi, it: *mut DBusMessageIter, kind: c_int, s: &str) -> bool {
    let Ok(cs) = CString::new(s) else {
        return false;
    };
    let p: *const c_char = cs.as_ptr();
    unsafe {
        (db.message_iter_append_basic)(it, kind, (&p as *const *const c_char).cast::<c_void>()) != 0
    }
}

/// # Safety
///
/// `dict` must point to a valid writable array container `DBusMessageIter`.
pub unsafe fn add_dict_entry_str(db: &DBusApi, dict: *mut DBusMessageIter, k: &str, v: &str) {
    let (Ok(ck), Ok(cv)) = (CString::new(k), CString::new(v)) else {
        return;
    };
    let mut entry = DBusMessageIter::default();
    let mut var = DBusMessageIter::default();
    let pk = ck.as_ptr();
    let pv = cv.as_ptr();
    unsafe {
        if (db.message_iter_open_container)(dict, DBUS_TYPE_DICT_ENTRY, ptr::null(), &mut entry)
            == 0
        {
            return;
        }
        let _ = (db.message_iter_append_basic)(
            &mut entry,
            DBUS_TYPE_STRING,
            (&pk as *const *const c_char).cast::<c_void>(),
        );
        if (db.message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"s".as_ptr(), &mut var)
            != 0
        {
            let _ = (db.message_iter_append_basic)(
                &mut var,
                DBUS_TYPE_STRING,
                (&pv as *const *const c_char).cast::<c_void>(),
            );
            let _ = (db.message_iter_close_container)(&mut entry, &mut var);
        }
        let _ = (db.message_iter_close_container)(dict, &mut entry);
    }
}

/// # Safety
///
/// `dict` must point to a valid writable array container `DBusMessageIter`.
pub unsafe fn add_dict_entry_obj_path(db: &DBusApi, dict: *mut DBusMessageIter, k: &str, v: &str) {
    if !is_valid_dbus_object_path(v) {
        return;
    }
    let (Ok(ck), Ok(cv)) = (CString::new(k), CString::new(v)) else {
        return;
    };
    let mut entry = DBusMessageIter::default();
    let mut var = DBusMessageIter::default();
    let pk = ck.as_ptr();
    let pv = cv.as_ptr();
    unsafe {
        if (db.message_iter_open_container)(dict, DBUS_TYPE_DICT_ENTRY, ptr::null(), &mut entry)
            == 0
        {
            return;
        }
        let _ = (db.message_iter_append_basic)(
            &mut entry,
            DBUS_TYPE_STRING,
            (&pk as *const *const c_char).cast::<c_void>(),
        );
        if (db.message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"o".as_ptr(), &mut var)
            != 0
        {
            let _ = (db.message_iter_append_basic)(
                &mut var,
                DBUS_TYPE_OBJECT_PATH,
                (&pv as *const *const c_char).cast::<c_void>(),
            );
            let _ = (db.message_iter_close_container)(&mut entry, &mut var);
        }
        let _ = (db.message_iter_close_container)(dict, &mut entry);
    }
}

/// # Safety
///
/// `dict` must point to a valid writable array container `DBusMessageIter`.
pub unsafe fn add_dict_entry_byte(db: &DBusApi, dict: *mut DBusMessageIter, k: &str, v: u8) {
    let Ok(ck) = CString::new(k) else {
        return;
    };
    let mut entry = DBusMessageIter::default();
    let mut var = DBusMessageIter::default();
    let pk = ck.as_ptr();
    unsafe {
        if (db.message_iter_open_container)(dict, DBUS_TYPE_DICT_ENTRY, ptr::null(), &mut entry)
            == 0
        {
            return;
        }
        let _ = (db.message_iter_append_basic)(
            &mut entry,
            DBUS_TYPE_STRING,
            (&pk as *const *const c_char).cast::<c_void>(),
        );
        if (db.message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"y".as_ptr(), &mut var)
            != 0
        {
            let _ = (db.message_iter_append_basic)(
                &mut var,
                DBUS_TYPE_BYTE,
                (&v as *const u8).cast::<c_void>(),
            );
            let _ = (db.message_iter_close_container)(&mut entry, &mut var);
        }
        let _ = (db.message_iter_close_container)(dict, &mut entry);
    }
}

/// # Safety
///
/// `dict` must point to a valid writable array container `DBusMessageIter`.
pub unsafe fn add_dict_entry_u16(db: &DBusApi, dict: *mut DBusMessageIter, k: &str, v: u16) {
    let Ok(ck) = CString::new(k) else {
        return;
    };
    let mut entry = DBusMessageIter::default();
    let mut var = DBusMessageIter::default();
    let pk = ck.as_ptr();
    unsafe {
        if (db.message_iter_open_container)(dict, DBUS_TYPE_DICT_ENTRY, ptr::null(), &mut entry)
            == 0
        {
            return;
        }
        let _ = (db.message_iter_append_basic)(
            &mut entry,
            DBUS_TYPE_STRING,
            (&pk as *const *const c_char).cast::<c_void>(),
        );
        if (db.message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"q".as_ptr(), &mut var)
            != 0
        {
            let _ = (db.message_iter_append_basic)(
                &mut var,
                DBUS_TYPE_UINT16,
                (&v as *const u16).cast::<c_void>(),
            );
            let _ = (db.message_iter_close_container)(&mut entry, &mut var);
        }
        let _ = (db.message_iter_close_container)(dict, &mut entry);
    }
}

/// # Safety
///
/// `dict` must point to a valid writable array container `DBusMessageIter`.
pub unsafe fn add_dict_entry_bool(db: &DBusApi, dict: *mut DBusMessageIter, k: &str, v: bool) {
    let Ok(ck) = CString::new(k) else {
        return;
    };
    let mut entry = DBusMessageIter::default();
    let mut var = DBusMessageIter::default();
    let pk = ck.as_ptr();
    let bv: u32 = u32::from(v);
    unsafe {
        if (db.message_iter_open_container)(dict, DBUS_TYPE_DICT_ENTRY, ptr::null(), &mut entry)
            == 0
        {
            return;
        }
        let _ = (db.message_iter_append_basic)(
            &mut entry,
            DBUS_TYPE_STRING,
            (&pk as *const *const c_char).cast::<c_void>(),
        );
        if (db.message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"b".as_ptr(), &mut var)
            != 0
        {
            let _ = (db.message_iter_append_basic)(
                &mut var,
                DBUS_TYPE_BOOLEAN,
                (&bv as *const u32).cast::<c_void>(),
            );
            let _ = (db.message_iter_close_container)(&mut entry, &mut var);
        }
        let _ = (db.message_iter_close_container)(dict, &mut entry);
    }
}

/// # Safety
///
/// `dict` must point to a valid writable array container `DBusMessageIter`.
pub unsafe fn add_dict_entry_bytes(db: &DBusApi, dict: *mut DBusMessageIter, k: &str, buf: &[u8]) {
    let Ok(ck) = CString::new(k) else {
        return;
    };
    let mut entry = DBusMessageIter::default();
    let mut var = DBusMessageIter::default();
    let mut arr = DBusMessageIter::default();
    let pk = ck.as_ptr();
    let pbuf = buf.as_ptr();
    unsafe {
        if (db.message_iter_open_container)(dict, DBUS_TYPE_DICT_ENTRY, ptr::null(), &mut entry)
            == 0
        {
            return;
        }
        let _ = (db.message_iter_append_basic)(
            &mut entry,
            DBUS_TYPE_STRING,
            (&pk as *const *const c_char).cast::<c_void>(),
        );
        if (db.message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"ay".as_ptr(), &mut var)
            != 0
        {
            if (db.message_iter_open_container)(
                &mut var,
                DBUS_TYPE_ARRAY,
                c"y".as_ptr(),
                &mut arr,
            ) != 0
            {
                let _ = (db.message_iter_append_fixed_array)(
                    &mut arr,
                    DBUS_TYPE_BYTE,
                    (&pbuf as *const *const u8).cast::<c_void>(),
                    buf.len() as c_int,
                );
                let _ = (db.message_iter_close_container)(&mut var, &mut arr);
            }
            let _ = (db.message_iter_close_container)(&mut entry, &mut var);
        }
        let _ = (db.message_iter_close_container)(dict, &mut entry);
    }
}

pub fn dbus_new_method_call(
    db: &DBusApi,
    dest: &str,
    path: &str,
    iface: &str,
    method: &str,
) -> *mut DBusMessage {
    if !is_valid_dbus_object_path(path) {
        return ptr::null_mut();
    }
    let (Ok(cd), Ok(cp), Ok(ci), Ok(cm)) = (
        CString::new(dest),
        CString::new(path),
        CString::new(iface),
        CString::new(method),
    ) else {
        return ptr::null_mut();
    };
    unsafe { (db.message_new_method_call)(cd.as_ptr(), cp.as_ptr(), ci.as_ptr(), cm.as_ptr()) }
}

/// # Safety
///
/// If non-null, `conn` must be a valid open `DBusConnection` pointer.
pub unsafe fn dbus_get_prop_msg(
    db: &DBusApi,
    conn: *mut DBusConnection,
    path: &str,
    iface: &str,
    prop: &str,
    timeout_ms: c_int,
) -> *mut DBusMessage {
    if conn.is_null() || path.is_empty() {
        return ptr::null_mut();
    }
    let msg = dbus_new_method_call(
        db,
        "org.bluez",
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
    );
    if msg.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let mut it = DBusMessageIter::default();
        (db.message_iter_init_append)(msg, &mut it);
        if !iter_append_str(db, &mut it, DBUS_TYPE_STRING, iface)
            || !iter_append_str(db, &mut it, DBUS_TYPE_STRING, prop)
        {
            (db.message_unref)(msg);
            return ptr::null_mut();
        }
        let rep = (db.connection_send_with_reply_and_block)(conn, msg, timeout_ms, ptr::null_mut());
        (db.message_unref)(msg);
        rep
    }
}

/// # Safety
///
/// If non-null, `conn` must be a valid open `DBusConnection` pointer.
pub unsafe fn dbus_get_str_prop(
    db: &DBusApi,
    conn: *mut DBusConnection,
    path: &str,
    iface: &str,
    prop: &str,
) -> Option<String> {
    let rep = unsafe { dbus_get_prop_msg(db, conn, path, iface, prop, 1500) };
    if rep.is_null() {
        return None;
    }
    unsafe {
        let mut it = DBusMessageIter::default();
        let mut var = DBusMessageIter::default();
        let mut res = None;
        if (db.message_iter_init)(rep, &mut it) != 0
            && (db.message_iter_get_arg_type)(&mut it) == DBUS_TYPE_VARIANT
        {
            (db.message_iter_recurse)(&mut it, &mut var);
            res = iter_get_str(db, &mut var);
        }
        (db.message_unref)(rep);
        res
    }
}

/// # Safety
///
/// If non-null, `conn` must be a valid open `DBusConnection` pointer.
pub unsafe fn dbus_get_u16_prop(
    db: &DBusApi,
    conn: *mut DBusConnection,
    path: &str,
    iface: &str,
    prop: &str,
) -> Option<u16> {
    let rep = unsafe { dbus_get_prop_msg(db, conn, path, iface, prop, 1000) };
    if rep.is_null() {
        return None;
    }
    unsafe {
        let mut it = DBusMessageIter::default();
        let mut var = DBusMessageIter::default();
        let mut res = None;
        if (db.message_iter_init)(rep, &mut it) != 0
            && (db.message_iter_get_arg_type)(&mut it) == DBUS_TYPE_VARIANT
        {
            (db.message_iter_recurse)(&mut it, &mut var);
            res = iter_get_u16(db, &mut var);
        }
        (db.message_unref)(rep);
        res
    }
}

/// # Safety
///
/// If non-null, `conn` must be a valid open `DBusConnection` pointer.
pub unsafe fn dbus_get_bool_prop(
    db: &DBusApi,
    conn: *mut DBusConnection,
    path: &str,
    iface: &str,
    prop: &str,
) -> Option<bool> {
    let rep = unsafe { dbus_get_prop_msg(db, conn, path, iface, prop, 1500) };
    if rep.is_null() {
        return None;
    }
    unsafe {
        let mut it = DBusMessageIter::default();
        let mut var = DBusMessageIter::default();
        let mut res = None;
        if (db.message_iter_init)(rep, &mut it) != 0
            && (db.message_iter_get_arg_type)(&mut it) == DBUS_TYPE_VARIANT
        {
            (db.message_iter_recurse)(&mut it, &mut var);
            res = iter_get_bool(db, &mut var);
        }
        (db.message_unref)(rep);
        res
    }
}

/// # Safety
///
/// If non-null, `conn` must be a valid open `DBusConnection` pointer.
pub unsafe fn dbus_set_avrcp_volume(
    db: &DBusApi,
    conn: *mut DBusConnection,
    transport_path: &str,
    vol_u16: u16,
) {
    if conn.is_null() || transport_path.is_empty() {
        return;
    }
    let msg = dbus_new_method_call(
        db,
        "org.bluez",
        transport_path,
        "org.freedesktop.DBus.Properties",
        "Set",
    );
    if msg.is_null() {
        return;
    }
    unsafe {
        let mut it = DBusMessageIter::default();
        let mut var = DBusMessageIter::default();
        (db.message_iter_init_append)(msg, &mut it);
        let _ = iter_append_str(
            db,
            &mut it,
            DBUS_TYPE_STRING,
            "org.bluez.MediaTransport1",
        );
        let _ = iter_append_str(db, &mut it, DBUS_TYPE_STRING, "Volume");
        if (db.message_iter_open_container)(&mut it, DBUS_TYPE_VARIANT, c"q".as_ptr(), &mut var)
            != 0
        {
            let _ = (db.message_iter_append_basic)(
                &mut var,
                DBUS_TYPE_UINT16,
                (&vol_u16 as *const u16).cast::<c_void>(),
            );
            let _ = (db.message_iter_close_container)(&mut it, &mut var);
        }
        let rep = (db.connection_send_with_reply_and_block)(conn, msg, 1000, ptr::null_mut());
        (db.message_unref)(msg);
        if !rep.is_null() {
            (db.message_unref)(rep);
        }
    }
}

/// # Safety
///
/// If non-null, `conn` must be a valid open `DBusConnection` pointer.
pub unsafe fn dbus_set_str_prop(
    db: &DBusApi,
    conn: *mut DBusConnection,
    path: &str,
    iface: &str,
    prop: &str,
    val: &str,
) {
    if conn.is_null() || path.is_empty() {
        return;
    }
    let msg = dbus_new_method_call(
        db,
        "org.bluez",
        path,
        "org.freedesktop.DBus.Properties",
        "Set",
    );
    if msg.is_null() {
        return;
    }
    unsafe {
        let mut it = DBusMessageIter::default();
        let mut var = DBusMessageIter::default();
        (db.message_iter_init_append)(msg, &mut it);
        let _ = iter_append_str(db, &mut it, DBUS_TYPE_STRING, iface);
        let _ = iter_append_str(db, &mut it, DBUS_TYPE_STRING, prop);
        if (db.message_iter_open_container)(&mut it, DBUS_TYPE_VARIANT, c"s".as_ptr(), &mut var)
            != 0
        {
            let _ = iter_append_str(db, &mut var, DBUS_TYPE_STRING, val);
            let _ = (db.message_iter_close_container)(&mut it, &mut var);
        }
        let rep = (db.connection_send_with_reply_and_block)(conn, msg, 1500, ptr::null_mut());
        (db.message_unref)(msg);
        if !rep.is_null() {
            (db.message_unref)(rep);
        }
    }
}

/// # Safety
///
/// If non-null, `err.message` must point to a valid NUL-terminated C string.
pub unsafe fn dbus_error_msg(err: &DBusError) -> String {
    if err.message.is_null() {
        "unknown".to_string()
    } else {
        unsafe { CStr::from_ptr(err.message) }
            .to_string_lossy()
            .into_owned()
    }
}

// ============================================================================
// Encapsulated A2DP & SCO Codec Adapters (Host FFI + Deterministic Test Double)
// ============================================================================

pub fn is_sbc_available() -> bool {
    load_sbc_library().is_some()
}

pub fn is_fdk_aac_available() -> bool {
    load_fdk_aac_library().is_some()
}

pub fn is_ldac_available() -> bool {
    load_ldac_library().is_some()
}

pub fn is_aptx_available() -> bool {
    load_aptx_library().is_some()
}

#[derive(Default)]
struct HostCodecHandles {
    sbc_enc: Option<Box<SbcStruct>>,
    fdk_enc: *mut c_void,
    fdk_frame_length: u32,
    ldac_enc: *mut c_void,
    ldac_eqmid: i32,
    ldac_abr: *mut c_void,
    aptx_enc: *mut c_void,
}

#[derive(Default)]
struct SyntheticCodecState {
    ldac_active: bool,
    ldac_abr_active: bool,
    ldac_eqmid: i32,
    aptx_active: bool,
    aptx_hd: bool,
    fdk_active: bool,
    fdk_frame_length: u32,
    fdk_bitrate: u32,
    sbc_active: bool,
}

enum CodecAdapterMode {
    Host(HostCodecHandles),
    Synthetic(SyntheticCodecState),
}

impl Default for CodecAdapterMode {
    fn default() -> Self {
        Self::Host(HostCodecHandles::default())
    }
}

/// Safe owning adapter around dynamically loaded A2DP encoder handles and ABR
/// state (`LDAC`, `aptX`/`aptX HD`, `FDK-AAC`, `SBC`).
#[derive(Default)]
pub struct A2dpCodecAdapter {
    mode: CodecAdapterMode,
}

// SAFETY: Host encoder pointers owned uniquely inside `A2dpCodecAdapter` are only accessed under
// `BluezRuntime`'s state mutex and freed exclusively by `close_all` / `Drop`.
unsafe impl Send for A2dpCodecAdapter {}

impl A2dpCodecAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_synthetic() -> Self {
        Self {
            mode: CodecAdapterMode::Synthetic(SyntheticCodecState::default()),
        }
    }

    pub fn close_all(&mut self) {
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if !h.ldac_abr.is_null() {
                    if let Some(abr) = load_ldac_abr_library() {
                        unsafe { (abr.free_handle)(h.ldac_abr) };
                    }
                    h.ldac_abr = ptr::null_mut();
                }
                if !h.fdk_enc.is_null() {
                    if let Some(fdk) = load_fdk_aac_library() {
                        unsafe {
                            let _ = (fdk.close)(&mut h.fdk_enc);
                        }
                    }
                    h.fdk_enc = ptr::null_mut();
                }
                if !h.ldac_enc.is_null() {
                    if let Some(ldac) = load_ldac_library() {
                        unsafe { (ldac.free_handle)(h.ldac_enc) };
                    }
                    h.ldac_enc = ptr::null_mut();
                }
                if !h.aptx_enc.is_null() {
                    if let Some(aptx) = load_aptx_library() {
                        unsafe { (aptx.finish)(h.aptx_enc) };
                    }
                    h.aptx_enc = ptr::null_mut();
                }
                if let Some(mut sbc) = h.sbc_enc.take() {
                    if let Some(api) = load_sbc_library() {
                        unsafe { (api.finish)(&mut *sbc) };
                    }
                }
                h.fdk_frame_length = 0;
                h.ldac_eqmid = 0;
            }
            CodecAdapterMode::Synthetic(s) => {
                *s = SyntheticCodecState::default();
            }
        }
    }

    pub fn has_ldac(&self) -> bool {
        match &self.mode {
            CodecAdapterMode::Host(h) => !h.ldac_enc.is_null(),
            CodecAdapterMode::Synthetic(s) => s.ldac_active,
        }
    }

    pub fn has_ldac_abr(&self) -> bool {
        match &self.mode {
            CodecAdapterMode::Host(h) => !h.ldac_abr.is_null(),
            CodecAdapterMode::Synthetic(s) => s.ldac_abr_active,
        }
    }

    pub fn ldac_eqmid(&self) -> i32 {
        match &self.mode {
            CodecAdapterMode::Host(h) => h.ldac_eqmid,
            CodecAdapterMode::Synthetic(s) => s.ldac_eqmid,
        }
    }

    pub fn init_ldac(
        &mut self,
        payload_mtu: u16,
        eqmid: i32,
        channel_mode: i32,
        sample_rate: u32,
        try_abr: bool,
    ) -> bool {
        self.close_all();
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                let Some(ldac) = load_ldac_library() else {
                    return false;
                };
                let lh = unsafe { (ldac.get_handle)() };
                if lh.is_null() {
                    return false;
                }
                if unsafe {
                    (ldac.init_encode)(
                        lh,
                        c_int::from(payload_mtu),
                        eqmid,
                        channel_mode,
                        LDACBT_SMPL_FMT_S16,
                        sample_rate as c_int,
                    )
                } < 0
                {
                    unsafe { (ldac.free_handle)(lh) };
                    return false;
                }
                h.ldac_enc = lh;
                h.ldac_eqmid = eqmid;
                if try_abr {
                    let _ = self.enable_ldac_abr();
                }
                true
            }
            CodecAdapterMode::Synthetic(s) => {
                s.ldac_active = true;
                s.ldac_eqmid = eqmid;
                s.ldac_abr_active = try_abr;
                true
            }
        }
    }

    pub fn enable_ldac_abr(&mut self) -> bool {
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if !h.ldac_abr.is_null() {
                    return true;
                }
                let Some(abr_api) = load_ldac_abr_library() else {
                    return false;
                };
                let abr = unsafe { (abr_api.get_handle)() };
                if abr.is_null() {
                    return false;
                }
                unsafe {
                    let _ = (abr_api.init)(abr, 20);
                    let _ = (abr_api.set_thresholds)(abr, 5, 3, 1);
                }
                h.ldac_abr = abr;
                true
            }
            CodecAdapterMode::Synthetic(s) => {
                s.ldac_abr_active = s.ldac_active;
                s.ldac_abr_active
            }
        }
    }

    pub fn step_ldac_abr(&mut self, qdepth: u32) -> Option<i32> {
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if h.ldac_enc.is_null() || h.ldac_abr.is_null() {
                    return None;
                }
                let abr_api = load_ldac_abr_library()?;
                let ret = unsafe { (abr_api.proc_step)(h.ldac_enc, h.ldac_abr, qdepth, 1) };
                if (LDACBT_EQMID_HQ..=LDACBT_EQMID_MQ).contains(&ret) {
                    h.ldac_eqmid = ret;
                    Some(ret)
                } else {
                    None
                }
            }
            CodecAdapterMode::Synthetic(s) => {
                if !s.ldac_active || !s.ldac_abr_active {
                    return None;
                }
                s.ldac_eqmid = if qdepth >= 4 {
                    LDACBT_EQMID_MQ
                } else {
                    LDACBT_EQMID_HQ
                };
                Some(s.ldac_eqmid)
            }
        }
    }

    pub fn set_ldac_eqmid(&mut self, eqmid: i32) -> bool {
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if h.ldac_enc.is_null() {
                    return false;
                }
                let Some(set_eqmid) = load_ldac_library().and_then(|l| l.set_eqmid) else {
                    return false;
                };
                if unsafe { set_eqmid(h.ldac_enc, eqmid) } == 0 {
                    h.ldac_eqmid = eqmid;
                    true
                } else {
                    false
                }
            }
            CodecAdapterMode::Synthetic(s) => {
                if s.ldac_active {
                    s.ldac_eqmid = eqmid;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn encode_ldac(
        &mut self,
        pcm_interleaved: &mut [i16],
        flen: usize,
        ch: u32,
        out_payload: &mut [u8],
    ) -> Option<(usize, u8)> {
        let ch_us = ch as usize;
        let need_samples = flen.checked_mul(ch_us)?;
        if pcm_interleaved.len() < need_samples || out_payload.is_empty() {
            return None;
        }
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if h.ldac_enc.is_null() {
                    return None;
                }
                let ldac = load_ldac_library()?;
                let mut pcm_used = (need_samples * mem::size_of::<i16>()) as c_int;
                let mut stream_sz = out_payload.len().min(c_int::MAX as usize) as c_int;
                let mut frame_num = 0 as c_int;
                let enc_rc = unsafe {
                    (ldac.encode)(
                        h.ldac_enc,
                        pcm_interleaved.as_mut_ptr().cast::<c_void>(),
                        &mut pcm_used,
                        out_payload.as_mut_ptr(),
                        &mut stream_sz,
                        &mut frame_num,
                    )
                };
                if enc_rc == 0
                    && frame_num > 0
                    && stream_sz > 0
                    && (stream_sz as usize) <= out_payload.len()
                {
                    Some((stream_sz as usize, frame_num as u8))
                } else {
                    None
                }
            }
            CodecAdapterMode::Synthetic(s) => {
                if !s.ldac_active {
                    return None;
                }
                let sz = out_payload.len().min(128);
                out_payload[..sz].fill(0xAA);
                Some((sz, 1))
            }
        }
    }

    pub fn has_aptx(&self) -> bool {
        match &self.mode {
            CodecAdapterMode::Host(h) => !h.aptx_enc.is_null(),
            CodecAdapterMode::Synthetic(s) => s.aptx_active,
        }
    }

    pub fn init_aptx(&mut self, is_hd: bool) -> bool {
        self.close_all();
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                let Some(aptx) = load_aptx_library() else {
                    return false;
                };
                let ah = unsafe { (aptx.init)(c_int::from(is_hd)) };
                if ah.is_null() {
                    return false;
                }
                h.aptx_enc = ah;
                true
            }
            CodecAdapterMode::Synthetic(s) => {
                s.aptx_active = true;
                s.aptx_hd = is_hd;
                true
            }
        }
    }

    pub fn encode_aptx(&mut self, s24_bytes: &[u8], out_payload: &mut [u8]) -> Option<usize> {
        if s24_bytes.is_empty() || out_payload.is_empty() {
            return None;
        }
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if h.aptx_enc.is_null() {
                    return None;
                }
                let aptx = load_aptx_library()?;
                let mut written = 0usize;
                unsafe {
                    let _ = (aptx.encode)(
                        h.aptx_enc,
                        s24_bytes.as_ptr(),
                        s24_bytes.len(),
                        out_payload.as_mut_ptr(),
                        out_payload.len(),
                        &mut written,
                    );
                }
                (written > 0 && written <= out_payload.len()).then_some(written)
            }
            CodecAdapterMode::Synthetic(s) => {
                if !s.aptx_active {
                    return None;
                }
                let blk = if s.aptx_hd { 6 } else { 4 };
                let out_len = (s24_bytes.len() / 24 * blk).min(out_payload.len());
                if out_len == 0 {
                    return None;
                }
                out_payload[..out_len].fill(0x5A);
                Some(out_len)
            }
        }
    }

    pub fn has_fdk_aac(&self) -> bool {
        match &self.mode {
            CodecAdapterMode::Host(h) => !h.fdk_enc.is_null(),
            CodecAdapterMode::Synthetic(s) => s.fdk_active,
        }
    }

    pub fn fdk_frame_length(&self) -> u32 {
        match &self.mode {
            CodecAdapterMode::Host(h) => h.fdk_frame_length,
            CodecAdapterMode::Synthetic(s) => s.fdk_frame_length,
        }
    }

    pub fn init_fdk_aac(
        &mut self,
        channels: u32,
        aot: u32,
        sample_rate: u32,
        target_bps: u32,
        peak_bps: u32,
    ) -> bool {
        self.close_all();
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                let Some(fdk) = load_fdk_aac_library() else {
                    return false;
                };
                let mut enc: *mut c_void = ptr::null_mut();
                if unsafe { (fdk.open)(&mut enc, 0, channels) } != FDK_AACENC_OK || enc.is_null() {
                    return false;
                }
                unsafe {
                    let _ = (fdk.set_param)(enc, FDK_PARAM_AOT, aot);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_SAMPLERATE, sample_rate);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_CHANNELMODE, channels);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_BITRATEMODE, 0);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_AUDIOMUXVER, 2);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_SIGNALING_MODE, 1);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_BITRATE, target_bps);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_PEAK_BITRATE, peak_bps);
                    if target_bps > 110_000 {
                        let _ = (fdk.set_param)(enc, FDK_PARAM_BANDWIDTH, 19_293);
                    }
                    let _ = (fdk.set_param)(enc, FDK_PARAM_TRANSMUX, FDK_TT_MP4_LATM_MCP1);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_HEADER_PERIOD, 1);
                    let _ = (fdk.set_param)(enc, FDK_PARAM_AFTERBURNER, 1);
                    if (fdk.encode)(enc, ptr::null(), ptr::null(), ptr::null(), ptr::null_mut())
                        != FDK_AACENC_OK
                    {
                        let _ = (fdk.close)(&mut enc);
                        return false;
                    }
                    let mut info = FdkInfo::default();
                    if (fdk.info)(enc, &mut info) != FDK_AACENC_OK {
                        let _ = (fdk.close)(&mut enc);
                        return false;
                    }
                    h.fdk_enc = enc;
                    h.fdk_frame_length = if info.frame_length > 0 {
                        info.frame_length
                    } else {
                        1024
                    };
                }
                true
            }
            CodecAdapterMode::Synthetic(s) => {
                s.fdk_active = true;
                s.fdk_frame_length = 1024;
                s.fdk_bitrate = target_bps;
                true
            }
        }
    }

    pub fn set_fdk_aac_bitrate(&mut self, next_bps: u32, next_peak: u32) -> bool {
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if h.fdk_enc.is_null() {
                    return false;
                }
                let Some(fdk) = load_fdk_aac_library() else {
                    return false;
                };
                if unsafe { (fdk.set_param)(h.fdk_enc, FDK_PARAM_BITRATE, next_bps) }
                    == FDK_AACENC_OK
                {
                    unsafe {
                        let _ = (fdk.set_param)(h.fdk_enc, FDK_PARAM_PEAK_BITRATE, next_peak);
                    }
                    true
                } else {
                    false
                }
            }
            CodecAdapterMode::Synthetic(s) => {
                if s.fdk_active {
                    s.fdk_bitrate = next_bps;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn encode_fdk_aac(
        &mut self,
        pcm_interleaved: &mut [i16],
        flen: usize,
        ch: u32,
        out_payload: &mut [u8],
    ) -> Result<usize, (i32, i32)> {
        let ch_us = ch as usize;
        let Some(need_samples) = flen.checked_mul(ch_us) else {
            return Err((-1, 0));
        };
        if pcm_interleaved.len() < need_samples || out_payload.is_empty() {
            return Err((-1, 0));
        }
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                if h.fdk_enc.is_null() {
                    return Err((-1, 0));
                }
                let Some(fdk) = load_fdk_aac_library() else {
                    return Err((-1, 0));
                };
                let mut in_ptr = pcm_interleaved.as_mut_ptr().cast::<c_void>();
                let mut out_ptr = out_payload.as_mut_ptr().cast::<c_void>();
                let mut in_id = FDK_IN_AUDIO_DATA;
                let mut out_id = FDK_OUT_BITSTREAM_DATA;
                let mut in_sz = (need_samples * 2) as c_int;
                let mut out_sz = out_payload.len().min(c_int::MAX as usize) as c_int;
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
                    num_in_samples: need_samples as c_int,
                    num_anc_bytes: 0,
                };
                let mut out_args = FdkOutArgs::default();
                let enc_rc = unsafe {
                    (fdk.encode)(
                        h.fdk_enc,
                        &in_desc,
                        &out_desc,
                        &in_args,
                        &mut out_args,
                    )
                };
                if enc_rc == FDK_AACENC_OK
                    && out_args.num_out_bytes > 0
                    && (out_args.num_out_bytes as usize) <= out_payload.len()
                {
                    Ok(out_args.num_out_bytes as usize)
                } else {
                    Err((enc_rc, out_args.num_out_bytes))
                }
            }
            CodecAdapterMode::Synthetic(s) => {
                if !s.fdk_active {
                    return Err((-1, 0));
                }
                let out_len = out_payload.len().min(192);
                out_payload[..out_len].fill(0x47);
                Ok(out_len)
            }
        }
    }

    pub fn has_sbc(&self) -> bool {
        match &self.mode {
            CodecAdapterMode::Host(h) => h.sbc_enc.is_some(),
            CodecAdapterMode::Synthetic(s) => s.sbc_active,
        }
    }

    pub fn init_sbc_a2dp(&mut self, cfg4: &mut [u8; 4]) -> bool {
        self.close_all();
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                let Some(sbc_api) = load_sbc_library() else {
                    return false;
                };
                let mut sbc = Box::<SbcStruct>::default();
                unsafe {
                    let _ = (sbc_api.init_a2dp)(&mut *sbc, 0, cfg4.as_mut_ptr().cast::<c_void>(), 4);
                }
                sbc.endian = SBC_LE;
                h.sbc_enc = Some(sbc);
                true
            }
            CodecAdapterMode::Synthetic(s) => {
                s.sbc_active = true;
                true
            }
        }
    }

    pub fn sbc_samples_per_frame(&mut self, ch: u32) -> Option<(usize, usize)> {
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                let sbc = h.sbc_enc.as_mut()?;
                let sbc_api = load_sbc_library()?;
                let codesize = unsafe { (sbc_api.get_codesize)(&mut **sbc) };
                let spf = (codesize / ((ch as usize).max(1) * mem::size_of::<i16>())).max(1);
                let eff_spf = if codesize / ((ch as usize).max(1) * 2) == 0 {
                    128
                } else {
                    spf
                };
                Some((codesize, eff_spf))
            }
            CodecAdapterMode::Synthetic(s) => s.sbc_active.then(|| {
                let codesize = 128 * (ch as usize).max(1) * mem::size_of::<i16>();
                (codesize, 128)
            }),
        }
    }

    pub fn encode_sbc_frame(
        &mut self,
        pcm_in: &[i16],
        codesize: usize,
        out_frame: &mut [u8],
    ) -> Option<usize> {
        if codesize == 0 || out_frame.is_empty() || mem::size_of_val(pcm_in) < codesize {
            return None;
        }
        match &mut self.mode {
            CodecAdapterMode::Host(h) => {
                let sbc = h.sbc_enc.as_mut()?;
                let sbc_api = load_sbc_library()?;
                let mut enc_written = 0isize;
                let in_consumed = unsafe {
                    (sbc_api.encode)(
                        &mut **sbc,
                        pcm_in.as_ptr().cast::<c_void>(),
                        codesize,
                        out_frame.as_mut_ptr().cast::<c_void>(),
                        out_frame.len(),
                        &mut enc_written,
                    )
                };
                if in_consumed > 0 && enc_written > 0 && (enc_written as usize) <= out_frame.len() {
                    Some(enc_written as usize)
                } else {
                    None
                }
            }
            CodecAdapterMode::Synthetic(s) => {
                if !s.sbc_active || out_frame.len() < 119 {
                    return None;
                }
                out_frame[..119].fill(0x9C);
                Some(119)
            }
        }
    }
}

impl Drop for A2dpCodecAdapter {
    fn drop(&mut self) {
        self.close_all();
    }
}

pub struct MsbcScoTranscoder {
    api: &'static SbcApi,
    dec: SbcStruct,
    enc: SbcStruct,
}

unsafe impl Send for MsbcScoTranscoder {}

impl MsbcScoTranscoder {
    pub fn new_if_available() -> Option<Self> {
        let api = load_sbc_library()?;
        let mut dec = SbcStruct::default();
        let mut enc = SbcStruct::default();
        unsafe {
            let _ = (api.init_msbc)(&mut dec, 0);
            let _ = (api.init_msbc)(&mut enc, 0);
        }
        Some(Self { api, dec, enc })
    }

    pub fn decode_frame(&mut self, src: &[u8], dst: &mut [i16]) -> Option<usize> {
        if src.is_empty() || dst.is_empty() {
            return None;
        }
        let mut written = 0usize;
        let rc = unsafe {
            (self.api.decode)(
                &mut self.dec,
                src.as_ptr().cast::<c_void>(),
                src.len(),
                dst.as_mut_ptr().cast::<c_void>(),
                mem::size_of_val(dst),
                &mut written,
            )
        };
        (rc > 0).then_some(written / 2)
    }

    pub fn encode_frame(&mut self, src_bytes: &[u8], dst: &mut [u8]) -> Option<usize> {
        if src_bytes.is_empty() || dst.is_empty() {
            return None;
        }
        let mut enc_written = 0isize;
        let rc = unsafe {
            (self.api.encode)(
                &mut self.enc,
                src_bytes.as_ptr().cast::<c_void>(),
                src_bytes.len(),
                dst.as_mut_ptr().cast::<c_void>(),
                dst.len(),
                &mut enc_written,
            )
        };
        if rc > 0 && enc_written > 0 && (enc_written as usize) <= dst.len() {
            Some(enc_written as usize)
        } else {
            None
        }
    }
}

impl Drop for MsbcScoTranscoder {
    fn drop(&mut self) {
        unsafe {
            (self.api.finish)(&mut self.dec);
            (self.api.finish)(&mut self.enc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn ffi_layouts_match_system_headers() {
        assert_eq!(size_of::<SbcStruct>(), 32);
        assert_eq!(offset_of!(SbcStruct, frequency), 8);
        assert_eq!(offset_of!(SbcStruct, endian), 14);
        assert_eq!(offset_of!(SbcStruct, priv_data), 16);
        assert_eq!(size_of::<DBusError>(), 32);
        assert_eq!(offset_of!(DBusError, message), 8);
        assert_eq!(size_of::<DBusMessageIter>(), 72);
        assert_eq!(size_of::<DBusObjectPathVTable>(), 48);
        assert_eq!(offset_of!(DBusObjectPathVTable, message_function), 8);
        assert_eq!(size_of::<SockaddrSco>(), 8);
        assert_eq!(size_of::<BtVoice>(), 2);
        assert!(load_dbus_library().is_some());
        assert!(load_sbc_library().is_some());
    }

    #[test]
    fn dbus_object_path_validation_rejects_malformed() {
        assert!(is_valid_dbus_object_path("/"));
        assert!(is_valid_dbus_object_path("/org/bluez/hci0/dev_AA_BB"));
        assert!(!is_valid_dbus_object_path(""));
        assert!(!is_valid_dbus_object_path("org/bluez"));
        assert!(!is_valid_dbus_object_path("/org/bluez/"));
        assert!(!is_valid_dbus_object_path("/org//bluez"));
        assert!(!is_valid_dbus_object_path("/org/bad-dash"));
        assert!(!is_valid_dbus_object_path("/org/\0null"));
    }

    #[test]
    fn a2dp_codec_adapter_synthetic_lifecycle_and_encoding_work() {
        let mut ad = A2dpCodecAdapter::new_synthetic();
        assert!(!ad.has_ldac());

        assert!(ad.init_ldac(660, LDACBT_EQMID_SQ, 1, 96_000, true));
        assert!(ad.has_ldac());
        assert!(ad.has_ldac_abr());
        assert_eq!(ad.step_ldac_abr(5), Some(LDACBT_EQMID_MQ));
        assert!(ad.set_ldac_eqmid(LDACBT_EQMID_HQ));
        assert_eq!(ad.ldac_eqmid(), LDACBT_EQMID_HQ);
        let mut pcm = [1024i16; 256];
        let mut out = [0u8; 512];
        assert_eq!(ad.encode_ldac(&mut pcm, 128, 2, &mut out), Some((128, 1)));

        assert!(ad.init_aptx(true));
        assert!(!ad.has_ldac());
        assert!(ad.has_aptx());
        let s24 = [0u8; 48];
        assert_eq!(ad.encode_aptx(&s24, &mut out), Some(12));

        assert!(ad.init_fdk_aac(2, FDK_AOT_AAC_LC, 48_000, 192_000, 240_000));
        assert!(ad.has_fdk_aac());
        assert_eq!(ad.fdk_frame_length(), 1024);
        assert!(ad.set_fdk_aac_bitrate(160_000, 192_000));
        let mut aac_pcm = [0i16; 2048];
        assert_eq!(ad.encode_fdk_aac(&mut aac_pcm, 1024, 2, &mut out), Ok(192));

        let mut cfg = [0u8; 4];
        assert!(ad.init_sbc_a2dp(&mut cfg));
        assert!(ad.has_sbc());
        let (codesize, spf) = ad.sbc_samples_per_frame(2).unwrap();
        assert_eq!(spf, 128);
        assert_eq!(ad.encode_sbc_frame(&pcm, codesize, &mut out), Some(119));

        ad.close_all();
        assert!(!ad.has_sbc());
        assert!(!ad.has_fdk_aac());
    }
}
