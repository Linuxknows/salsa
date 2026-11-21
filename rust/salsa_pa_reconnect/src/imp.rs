// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::ptr;
use core::sync::atomic::{AtomicI32, Ordering};
use std::sync::Once;

pub type pa_usec_t = u64;
pub type pa_volume_t = u32;
pub type pa_context_state_t = c_int;
pub type pa_stream_state_t = c_int;
pub type pa_operation_state_t = c_int;
pub type pa_context_flags_t = c_uint;
pub type pa_stream_flags_t = c_uint;
pub type pa_subscription_mask_t = c_uint;
pub type pa_subscription_event_type_t = c_uint;
pub type pa_update_mode_t = c_int;
pub type pa_seek_mode_t = c_int;
pub type pa_device_type_t = c_int;
pub type pa_autoload_type_t = c_int;
pub type pa_sink_flags_t = c_uint;
pub type pa_source_flags_t = c_uint;
pub type pa_sink_state_t = c_int;
pub type pa_source_state_t = c_int;
pub type pa_channel_map_def_t = c_int;

pub const PA_OK: c_int = 0;
pub const PA_ERR_INVALID: c_int = 3;
pub const PA_INVALID_INDEX: u32 = u32::MAX;
pub const PA_PROTOCOL_VERSION: u32 = 35;

pub const PA_CONTEXT_UNCONNECTED: c_int = 0;
pub const PA_CONTEXT_CONNECTING: c_int = 1;
pub const PA_CONTEXT_AUTHORIZING: c_int = 2;
pub const PA_CONTEXT_SETTING_NAME: c_int = 3;
pub const PA_CONTEXT_READY: c_int = 4;
pub const PA_CONTEXT_FAILED: c_int = 5;
pub const PA_CONTEXT_TERMINATED: c_int = 6;

pub const PA_STREAM_UNCONNECTED: c_int = 0;
pub const PA_STREAM_READY: c_int = 2;
pub const PA_STREAM_FAILED: c_int = 3;
pub const PA_STREAM_TERMINATED: c_int = 4;

pub const PA_OPERATION_RUNNING: c_int = 0;
pub const PA_OPERATION_DONE: c_int = 1;
pub const PA_OPERATION_CANCELLED: c_int = 2;

pub const PA_CONTEXT_NOFAIL: c_uint = 0x0002;
pub const PA_STREAM_START_CORKED: c_uint = 0x0001;
pub const PA_SAMPLE_FLOAT32LE: c_int = 5;
pub const PA_CHANNEL_MAP_DEFAULT: c_int = 0;

pub const PA_SUBSCRIPTION_MASK_NULL: c_uint = 0x0000;
pub const PA_SUBSCRIPTION_MASK_SINK: c_uint = 0x0001;
pub const PA_SUBSCRIPTION_MASK_SOURCE: c_uint = 0x0002;
pub const PA_SUBSCRIPTION_MASK_SERVER: c_uint = 0x0080;

pub const PA_SUBSCRIPTION_EVENT_SINK: c_uint = 0x0000;
pub const PA_SUBSCRIPTION_EVENT_SOURCE: c_uint = 0x0001;
pub const PA_SUBSCRIPTION_EVENT_SERVER: c_uint = 0x0007;
pub const PA_SUBSCRIPTION_EVENT_NEW: c_uint = 0x0000;
pub const PA_SUBSCRIPTION_EVENT_CHANGE: c_uint = 0x0010;

const MAX_MAINLOOPS: usize = 64;
const PTHREAD_MUTEX_RECURSIVE: c_int = 1;

#[repr(C)]
pub struct pa_context {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_stream {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_operation {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_threaded_mainloop {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_proplist {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_format_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_time_event {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_defer_event {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_client_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_sink_input_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_source_output_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_module_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_card_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_sample_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_autoload_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_ext_device_restore_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_ext_device_manager_info {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct pa_ext_stream_restore_info {
    _opaque: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TimeVal {
    pub tv_sec: c_long,
    pub tv_usec: c_long,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pa_sample_spec {
    pub format: c_int,
    pub rate: u32,
    pub channels: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pa_channel_map {
    pub channels: u8,
    pub map: [c_int; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pa_cvolume {
    pub channels: u8,
    pub values: [pa_volume_t; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pa_buffer_attr {
    pub maxlength: u32,
    pub tlength: u32,
    pub prebuf: u32,
    pub minreq: u32,
    pub fragsize: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pa_timing_info {
    pub timestamp: TimeVal,
    pub synchronized_clocks: c_int,
    pub sink_usec: pa_usec_t,
    pub source_usec: pa_usec_t,
    pub transport_usec: pa_usec_t,
    pub playing: c_int,
    pub write_index_corrupt: c_int,
    pub write_index: i64,
    pub read_index_corrupt: c_int,
    pub read_index: i64,
    pub configured_sink_usec: pa_usec_t,
    pub configured_source_usec: pa_usec_t,
    pub since_underrun: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pa_spawn_api {
    pub prefork: Option<unsafe extern "C" fn()>,
    pub postfork: Option<unsafe extern "C" fn()>,
    pub atfork: Option<unsafe extern "C" fn()>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pa_server_info {
    pub user_name: *const c_char,
    pub host_name: *const c_char,
    pub server_version: *const c_char,
    pub server_name: *const c_char,
    pub sample_spec: pa_sample_spec,
    pub default_sink_name: *const c_char,
    pub default_source_name: *const c_char,
    pub cookie: u32,
    pub channel_map: pa_channel_map,
}

impl Default for pa_server_info {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pa_sink_port_info {
    pub name: *const c_char,
    pub description: *const c_char,
    pub priority: u32,
    pub available: c_int,
    pub availability_group: *const c_char,
    pub type_: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pa_source_port_info {
    pub name: *const c_char,
    pub description: *const c_char,
    pub priority: u32,
    pub available: c_int,
    pub availability_group: *const c_char,
    pub type_: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pa_sink_info {
    pub name: *const c_char,
    pub index: u32,
    pub description: *const c_char,
    pub sample_spec: pa_sample_spec,
    pub channel_map: pa_channel_map,
    pub owner_module: u32,
    pub volume: pa_cvolume,
    pub mute: c_int,
    pub monitor_source: u32,
    pub monitor_source_name: *const c_char,
    pub latency: pa_usec_t,
    pub driver: *const c_char,
    pub flags: pa_sink_flags_t,
    pub proplist: *mut pa_proplist,
    pub configured_latency: pa_usec_t,
    pub base_volume: pa_volume_t,
    pub state: pa_sink_state_t,
    pub n_volume_steps: u32,
    pub card: u32,
    pub n_ports: u32,
    pub ports: *mut *mut pa_sink_port_info,
    pub active_port: *mut pa_sink_port_info,
    pub n_formats: u8,
    pub formats: *mut *mut pa_format_info,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pa_source_info {
    pub name: *const c_char,
    pub index: u32,
    pub description: *const c_char,
    pub sample_spec: pa_sample_spec,
    pub channel_map: pa_channel_map,
    pub owner_module: u32,
    pub volume: pa_cvolume,
    pub mute: c_int,
    pub monitor_of_sink: u32,
    pub monitor_of_sink_name: *const c_char,
    pub latency: pa_usec_t,
    pub driver: *const c_char,
    pub flags: pa_source_flags_t,
    pub proplist: *mut pa_proplist,
    pub configured_latency: pa_usec_t,
    pub base_volume: pa_volume_t,
    pub state: pa_source_state_t,
    pub n_volume_steps: u32,
    pub card: u32,
    pub n_ports: u32,
    pub ports: *mut *mut pa_source_port_info,
    pub active_port: *mut pa_source_port_info,
    pub n_formats: u8,
    pub formats: *mut *mut pa_format_info,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pa_stat_info {
    pub memblock_total: u32,
    pub memblock_total_size: u32,
    pub memblock_allocated: u32,
    pub memblock_allocated_size: u32,
    pub scache_size: u32,
}

pub type pa_time_event_cb_t = Option<
    unsafe extern "C" fn(*mut pa_mainloop_api, *mut pa_time_event, *const TimeVal, *mut c_void),
>;
pub type pa_defer_event_cb_t =
    Option<unsafe extern "C" fn(*mut pa_mainloop_api, *mut pa_defer_event, *mut c_void)>;

#[repr(C)]
pub struct pa_mainloop_api {
    pub userdata: *mut c_void,
    pub io_new: *mut c_void,
    pub io_enable: *mut c_void,
    pub io_free: *mut c_void,
    pub io_set_destroy: *mut c_void,
    pub time_new: Option<
        unsafe extern "C" fn(
            *mut pa_mainloop_api,
            *const TimeVal,
            pa_time_event_cb_t,
            *mut c_void,
        ) -> *mut pa_time_event,
    >,
    pub time_restart: Option<unsafe extern "C" fn(*mut pa_time_event, *const TimeVal)>,
    pub time_free: Option<unsafe extern "C" fn(*mut pa_time_event)>,
    pub time_set_destroy: *mut c_void,
    pub defer_new: Option<
        unsafe extern "C" fn(
            *mut pa_mainloop_api,
            pa_defer_event_cb_t,
            *mut c_void,
        ) -> *mut pa_defer_event,
    >,
    pub defer_enable: *mut c_void,
    pub defer_free: Option<unsafe extern "C" fn(*mut pa_defer_event)>,
    pub defer_set_destroy: *mut c_void,
    pub quit: *mut c_void,
}

pub type pa_context_notify_cb_t = Option<unsafe extern "C" fn(*mut pa_context, *mut c_void)>;
pub type pa_context_success_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, c_int, *mut c_void)>;
pub type pa_context_event_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const c_char, *mut pa_proplist, *mut c_void)>;
pub type pa_context_subscribe_cb_t = Option<
    unsafe extern "C" fn(*mut pa_context, pa_subscription_event_type_t, u32, *mut c_void),
>;
pub type pa_context_index_cb_t = Option<unsafe extern "C" fn(*mut pa_context, u32, *mut c_void)>;
pub type pa_context_string_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, c_int, *mut c_char, *mut c_void)>;
pub type pa_server_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_server_info, *mut c_void)>;
pub type pa_sink_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_sink_info, c_int, *mut c_void)>;
pub type pa_source_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_source_info, c_int, *mut c_void)>;
pub type pa_client_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_client_info, c_int, *mut c_void)>;
pub type pa_sink_input_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_sink_input_info, c_int, *mut c_void)>;
pub type pa_source_output_info_cb_t = Option<
    unsafe extern "C" fn(*mut pa_context, *const pa_source_output_info, c_int, *mut c_void),
>;
pub type pa_module_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_module_info, c_int, *mut c_void)>;
pub type pa_card_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_card_info, c_int, *mut c_void)>;
pub type pa_sample_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_sample_info, c_int, *mut c_void)>;
pub type pa_autoload_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_autoload_info, c_int, *mut c_void)>;
pub type pa_stat_info_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *const pa_stat_info, *mut c_void)>;

pub type pa_ext_device_restore_test_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, u32, *mut c_void)>;
pub type pa_ext_device_restore_subscribe_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, pa_device_type_t, u32, *mut c_void)>;
pub type pa_ext_device_restore_read_device_formats_cb_t = Option<
    unsafe extern "C" fn(*mut pa_context, *const pa_ext_device_restore_info, c_int, *mut c_void),
>;
pub type pa_ext_device_manager_test_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, u32, *mut c_void)>;
pub type pa_ext_device_manager_subscribe_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *mut c_void)>;
pub type pa_ext_device_manager_read_cb_t = Option<
    unsafe extern "C" fn(*mut pa_context, *const pa_ext_device_manager_info, c_int, *mut c_void),
>;
pub type pa_ext_stream_restore_test_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, u32, *mut c_void)>;
pub type pa_ext_stream_restore_subscribe_cb_t =
    Option<unsafe extern "C" fn(*mut pa_context, *mut c_void)>;
pub type pa_ext_stream_restore_read_cb_t = Option<
    unsafe extern "C" fn(*mut pa_context, *const pa_ext_stream_restore_info, c_int, *mut c_void),
>;

pub type pa_stream_notify_cb_t = Option<unsafe extern "C" fn(*mut pa_stream, *mut c_void)>;
pub type pa_stream_request_cb_t = Option<unsafe extern "C" fn(*mut pa_stream, usize, *mut c_void)>;
pub type pa_stream_success_cb_t = Option<unsafe extern "C" fn(*mut pa_stream, c_int, *mut c_void)>;
pub type pa_stream_event_cb_t =
    Option<unsafe extern "C" fn(*mut pa_stream, *const c_char, *mut pa_proplist, *mut c_void)>;
pub type pa_operation_notify_cb_t = Option<unsafe extern "C" fn(*mut pa_operation, *mut c_void)>;
pub type pa_free_cb_t = Option<unsafe extern "C" fn(*mut c_void)>;

type PthreadMutex = [u64; 8];
type PthreadMutexAttr = [u64; 2];

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn strdup(s: *const c_char) -> *mut c_char;
    fn strcmp(a: *const c_char, b: *const c_char) -> c_int;
    fn gettimeofday(tv: *mut TimeVal, tz: *mut c_void) -> c_int;
    fn pthread_mutexattr_init(attr: *mut PthreadMutexAttr) -> c_int;
    fn pthread_mutexattr_settype(attr: *mut PthreadMutexAttr, kind: c_int) -> c_int;
    fn pthread_mutexattr_destroy(attr: *mut PthreadMutexAttr) -> c_int;
    fn pthread_mutex_init(mutex: *mut PthreadMutex, attr: *const PthreadMutexAttr) -> c_int;
    fn pthread_mutex_destroy(mutex: *mut PthreadMutex) -> c_int;
    fn pthread_mutex_lock(mutex: *mut PthreadMutex) -> c_int;
    fn pthread_mutex_unlock(mutex: *mut PthreadMutex) -> c_int;
}

unsafe fn xstrdup(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        ptr::null_mut()
    } else {
        strdup(s)
    }
}

struct SyncCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SyncCell<T> {}
impl<T> SyncCell<T> {
    const fn new(v: T) -> Self {
        Self(UnsafeCell::new(v))
    }
    fn get(&self) -> *mut T {
        self.0.get()
    }
}

macro_rules! load_sym {
    ($h:expr, $name:literal, $ty:ty) => {{
        let p = dlsym($h, $name.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(core::mem::transmute::<*mut c_void, $ty>(p))
        }
    }};
}

macro_rules! define_real_syms {
    ($($field:ident : $cstr:literal => $ty:ty),* $(,)?) => {
        pub(crate) struct RealSyms {
            $(pub $field: Option<$ty>,)*
        }
        impl RealSyms {
            const fn empty() -> Self {
                Self {
                    $($field: None,)*
                }
            }
            pub(crate) unsafe fn load_from(&mut self, handle: *mut c_void) {
                $(self.$field = load_sym!(handle, $cstr, $ty);)*
            }
        }
    };
}

define_real_syms! {
    pa_context_new: c"pa_context_new" => unsafe extern "C" fn(*mut pa_mainloop_api, *const c_char) -> *mut pa_context,
    pa_context_new_with_proplist: c"pa_context_new_with_proplist" => unsafe extern "C" fn(*mut pa_mainloop_api, *const c_char, *const pa_proplist) -> *mut pa_context,
    pa_context_ref: c"pa_context_ref" => unsafe extern "C" fn(*mut pa_context) -> *mut pa_context,
    pa_context_unref: c"pa_context_unref" => unsafe extern "C" fn(*mut pa_context),
    pa_context_set_state_callback: c"pa_context_set_state_callback" => unsafe extern "C" fn(*mut pa_context, pa_context_notify_cb_t, *mut c_void),
    pa_context_set_event_callback: c"pa_context_set_event_callback" => unsafe extern "C" fn(*mut pa_context, pa_context_event_cb_t, *mut c_void),
    pa_context_set_subscribe_callback: c"pa_context_set_subscribe_callback" => unsafe extern "C" fn(*mut pa_context, pa_context_subscribe_cb_t, *mut c_void),
    pa_context_errno: c"pa_context_errno" => unsafe extern "C" fn(*const pa_context) -> c_int,
    pa_context_is_pending: c"pa_context_is_pending" => unsafe extern "C" fn(*const pa_context) -> c_int,
    pa_context_get_state: c"pa_context_get_state" => unsafe extern "C" fn(*const pa_context) -> pa_context_state_t,
    pa_context_connect: c"pa_context_connect" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_context_flags_t, *const pa_spawn_api) -> c_int,
    pa_context_disconnect: c"pa_context_disconnect" => unsafe extern "C" fn(*mut pa_context),
    pa_context_drain: c"pa_context_drain" => unsafe extern "C" fn(*mut pa_context, pa_context_notify_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_exit_daemon: c"pa_context_exit_daemon" => unsafe extern "C" fn(*mut pa_context, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_default_sink: c"pa_context_set_default_sink" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_default_source: c"pa_context_set_default_source" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_is_local: c"pa_context_is_local" => unsafe extern "C" fn(*const pa_context) -> c_int,
    pa_context_set_name: c"pa_context_set_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_server: c"pa_context_get_server" => unsafe extern "C" fn(*const pa_context) -> *const c_char,
    pa_context_get_protocol_version: c"pa_context_get_protocol_version" => unsafe extern "C" fn(*const pa_context) -> u32,
    pa_context_get_server_protocol_version: c"pa_context_get_server_protocol_version" => unsafe extern "C" fn(*const pa_context) -> u32,
    pa_context_proplist_update: c"pa_context_proplist_update" => unsafe extern "C" fn(*mut pa_context, pa_update_mode_t, *const pa_proplist, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_proplist_remove: c"pa_context_proplist_remove" => unsafe extern "C" fn(*mut pa_context, *const *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_index: c"pa_context_get_index" => unsafe extern "C" fn(*const pa_context) -> u32,
    pa_context_rttime_new: c"pa_context_rttime_new" => unsafe extern "C" fn(*const pa_context, pa_usec_t, pa_time_event_cb_t, *mut c_void) -> *mut pa_time_event,
    pa_context_rttime_restart: c"pa_context_rttime_restart" => unsafe extern "C" fn(*const pa_context, *mut pa_time_event, pa_usec_t),
    pa_context_get_tile_size: c"pa_context_get_tile_size" => unsafe extern "C" fn(*const pa_context, *const pa_sample_spec) -> usize,
    pa_context_load_cookie_from_file: c"pa_context_load_cookie_from_file" => unsafe extern "C" fn(*mut pa_context, *const c_char) -> c_int,
    pa_context_subscribe: c"pa_context_subscribe" => unsafe extern "C" fn(*mut pa_context, pa_subscription_mask_t, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_server_info: c"pa_context_get_server_info" => unsafe extern "C" fn(*mut pa_context, pa_server_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sink_info_by_name: c"pa_context_get_sink_info_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_sink_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sink_info_by_index: c"pa_context_get_sink_info_by_index" => unsafe extern "C" fn(*mut pa_context, u32, pa_sink_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sink_info_list: c"pa_context_get_sink_info_list" => unsafe extern "C" fn(*mut pa_context, pa_sink_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_volume_by_index: c"pa_context_set_sink_volume_by_index" => unsafe extern "C" fn(*mut pa_context, u32, *const pa_cvolume, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_volume_by_name: c"pa_context_set_sink_volume_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const pa_cvolume, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_mute_by_index: c"pa_context_set_sink_mute_by_index" => unsafe extern "C" fn(*mut pa_context, u32, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_mute_by_name: c"pa_context_set_sink_mute_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_suspend_sink_by_name: c"pa_context_suspend_sink_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_suspend_sink_by_index: c"pa_context_suspend_sink_by_index" => unsafe extern "C" fn(*mut pa_context, u32, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_port_by_index: c"pa_context_set_sink_port_by_index" => unsafe extern "C" fn(*mut pa_context, u32, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_port_by_name: c"pa_context_set_sink_port_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_source_info_by_name: c"pa_context_get_source_info_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_source_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_source_info_by_index: c"pa_context_get_source_info_by_index" => unsafe extern "C" fn(*mut pa_context, u32, pa_source_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_source_info_list: c"pa_context_get_source_info_list" => unsafe extern "C" fn(*mut pa_context, pa_source_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_volume_by_index: c"pa_context_set_source_volume_by_index" => unsafe extern "C" fn(*mut pa_context, u32, *const pa_cvolume, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_volume_by_name: c"pa_context_set_source_volume_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const pa_cvolume, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_mute_by_index: c"pa_context_set_source_mute_by_index" => unsafe extern "C" fn(*mut pa_context, u32, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_mute_by_name: c"pa_context_set_source_mute_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_suspend_source_by_name: c"pa_context_suspend_source_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_suspend_source_by_index: c"pa_context_suspend_source_by_index" => unsafe extern "C" fn(*mut pa_context, u32, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_port_by_index: c"pa_context_set_source_port_by_index" => unsafe extern "C" fn(*mut pa_context, u32, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_port_by_name: c"pa_context_set_source_port_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_module_info: c"pa_context_get_module_info" => unsafe extern "C" fn(*mut pa_context, u32, pa_module_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_module_info_list: c"pa_context_get_module_info_list" => unsafe extern "C" fn(*mut pa_context, pa_module_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_load_module: c"pa_context_load_module" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, pa_context_index_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_unload_module: c"pa_context_unload_module" => unsafe extern "C" fn(*mut pa_context, u32, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_send_message_to_object: c"pa_context_send_message_to_object" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, *const c_char, pa_context_string_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_client_info: c"pa_context_get_client_info" => unsafe extern "C" fn(*mut pa_context, u32, pa_client_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_client_info_list: c"pa_context_get_client_info_list" => unsafe extern "C" fn(*mut pa_context, pa_client_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_kill_client: c"pa_context_kill_client" => unsafe extern "C" fn(*mut pa_context, u32, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_card_info_by_index: c"pa_context_get_card_info_by_index" => unsafe extern "C" fn(*mut pa_context, u32, pa_card_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_card_info_by_name: c"pa_context_get_card_info_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_card_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_card_info_list: c"pa_context_get_card_info_list" => unsafe extern "C" fn(*mut pa_context, pa_card_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_card_profile_by_index: c"pa_context_set_card_profile_by_index" => unsafe extern "C" fn(*mut pa_context, u32, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_card_profile_by_name: c"pa_context_set_card_profile_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_port_latency_offset: c"pa_context_set_port_latency_offset" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, i64, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sink_input_info: c"pa_context_get_sink_input_info" => unsafe extern "C" fn(*mut pa_context, u32, pa_sink_input_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sink_input_info_list: c"pa_context_get_sink_input_info_list" => unsafe extern "C" fn(*mut pa_context, pa_sink_input_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_move_sink_input_by_name: c"pa_context_move_sink_input_by_name" => unsafe extern "C" fn(*mut pa_context, u32, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_move_sink_input_by_index: c"pa_context_move_sink_input_by_index" => unsafe extern "C" fn(*mut pa_context, u32, u32, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_input_volume: c"pa_context_set_sink_input_volume" => unsafe extern "C" fn(*mut pa_context, u32, *const pa_cvolume, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_sink_input_mute: c"pa_context_set_sink_input_mute" => unsafe extern "C" fn(*mut pa_context, u32, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_kill_sink_input: c"pa_context_kill_sink_input" => unsafe extern "C" fn(*mut pa_context, u32, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_source_output_info: c"pa_context_get_source_output_info" => unsafe extern "C" fn(*mut pa_context, u32, pa_source_output_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_source_output_info_list: c"pa_context_get_source_output_info_list" => unsafe extern "C" fn(*mut pa_context, pa_source_output_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_move_source_output_by_name: c"pa_context_move_source_output_by_name" => unsafe extern "C" fn(*mut pa_context, u32, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_move_source_output_by_index: c"pa_context_move_source_output_by_index" => unsafe extern "C" fn(*mut pa_context, u32, u32, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_output_volume: c"pa_context_set_source_output_volume" => unsafe extern "C" fn(*mut pa_context, u32, *const pa_cvolume, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_set_source_output_mute: c"pa_context_set_source_output_mute" => unsafe extern "C" fn(*mut pa_context, u32, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_kill_source_output: c"pa_context_kill_source_output" => unsafe extern "C" fn(*mut pa_context, u32, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_stat: c"pa_context_stat" => unsafe extern "C" fn(*mut pa_context, pa_stat_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sample_info_by_name: c"pa_context_get_sample_info_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_sample_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sample_info_by_index: c"pa_context_get_sample_info_by_index" => unsafe extern "C" fn(*mut pa_context, u32, pa_sample_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_sample_info_list: c"pa_context_get_sample_info_list" => unsafe extern "C" fn(*mut pa_context, pa_sample_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_remove_sample: c"pa_context_remove_sample" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_play_sample: c"pa_context_play_sample" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, pa_volume_t, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_play_sample_with_proplist: c"pa_context_play_sample_with_proplist" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, pa_volume_t, *const pa_proplist, pa_context_index_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_autoload_info_by_name: c"pa_context_get_autoload_info_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_autoload_type_t, pa_autoload_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_autoload_info_by_index: c"pa_context_get_autoload_info_by_index" => unsafe extern "C" fn(*mut pa_context, u32, pa_autoload_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_get_autoload_info_list: c"pa_context_get_autoload_info_list" => unsafe extern "C" fn(*mut pa_context, pa_autoload_info_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_add_autoload: c"pa_context_add_autoload" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_autoload_type_t, *const c_char, *const c_char, pa_context_index_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_remove_autoload_by_name: c"pa_context_remove_autoload_by_name" => unsafe extern "C" fn(*mut pa_context, *const c_char, pa_autoload_type_t, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_context_remove_autoload_by_index: c"pa_context_remove_autoload_by_index" => unsafe extern "C" fn(*mut pa_context, u32, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_new: c"pa_stream_new" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const pa_sample_spec, *const pa_channel_map) -> *mut pa_stream,
    pa_stream_new_with_proplist: c"pa_stream_new_with_proplist" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const pa_sample_spec, *const pa_channel_map, *mut pa_proplist) -> *mut pa_stream,
    pa_stream_new_extended: c"pa_stream_new_extended" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const *mut pa_format_info, c_uint, *mut pa_proplist) -> *mut pa_stream,
    pa_stream_unref: c"pa_stream_unref" => unsafe extern "C" fn(*mut pa_stream),
    pa_stream_ref: c"pa_stream_ref" => unsafe extern "C" fn(*mut pa_stream) -> *mut pa_stream,
    pa_stream_get_state: c"pa_stream_get_state" => unsafe extern "C" fn(*const pa_stream) -> pa_stream_state_t,
    pa_stream_get_context: c"pa_stream_get_context" => unsafe extern "C" fn(*const pa_stream) -> *mut pa_context,
    pa_stream_get_index: c"pa_stream_get_index" => unsafe extern "C" fn(*const pa_stream) -> u32,
    pa_stream_get_device_index: c"pa_stream_get_device_index" => unsafe extern "C" fn(*const pa_stream) -> u32,
    pa_stream_get_device_name: c"pa_stream_get_device_name" => unsafe extern "C" fn(*const pa_stream) -> *const c_char,
    pa_stream_is_suspended: c"pa_stream_is_suspended" => unsafe extern "C" fn(*const pa_stream) -> c_int,
    pa_stream_is_corked: c"pa_stream_is_corked" => unsafe extern "C" fn(*const pa_stream) -> c_int,
    pa_stream_connect_playback: c"pa_stream_connect_playback" => unsafe extern "C" fn(*mut pa_stream, *const c_char, *const pa_buffer_attr, pa_stream_flags_t, *const pa_cvolume, *mut pa_stream) -> c_int,
    pa_stream_connect_record: c"pa_stream_connect_record" => unsafe extern "C" fn(*mut pa_stream, *const c_char, *const pa_buffer_attr, pa_stream_flags_t) -> c_int,
    pa_stream_connect_upload: c"pa_stream_connect_upload" => unsafe extern "C" fn(*mut pa_stream, usize) -> c_int,
    pa_stream_finish_upload: c"pa_stream_finish_upload" => unsafe extern "C" fn(*mut pa_stream) -> c_int,
    pa_stream_disconnect: c"pa_stream_disconnect" => unsafe extern "C" fn(*mut pa_stream) -> c_int,
    pa_stream_begin_write: c"pa_stream_begin_write" => unsafe extern "C" fn(*mut pa_stream, *mut *mut c_void, *mut usize) -> c_int,
    pa_stream_cancel_write: c"pa_stream_cancel_write" => unsafe extern "C" fn(*mut pa_stream) -> c_int,
    pa_stream_write: c"pa_stream_write" => unsafe extern "C" fn(*mut pa_stream, *const c_void, usize, pa_free_cb_t, i64, pa_seek_mode_t) -> c_int,
    pa_stream_write_ext_free: c"pa_stream_write_ext_free" => unsafe extern "C" fn(*mut pa_stream, *const c_void, usize, pa_free_cb_t, *mut c_void, i64, pa_seek_mode_t) -> c_int,
    pa_stream_peek: c"pa_stream_peek" => unsafe extern "C" fn(*mut pa_stream, *mut *const c_void, *mut usize) -> c_int,
    pa_stream_drop: c"pa_stream_drop" => unsafe extern "C" fn(*mut pa_stream) -> c_int,
    pa_stream_writable_size: c"pa_stream_writable_size" => unsafe extern "C" fn(*const pa_stream) -> usize,
    pa_stream_readable_size: c"pa_stream_readable_size" => unsafe extern "C" fn(*const pa_stream) -> usize,
    pa_stream_drain: c"pa_stream_drain" => unsafe extern "C" fn(*mut pa_stream, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_update_timing_info: c"pa_stream_update_timing_info" => unsafe extern "C" fn(*mut pa_stream, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_set_state_callback: c"pa_stream_set_state_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_set_write_callback: c"pa_stream_set_write_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_request_cb_t, *mut c_void),
    pa_stream_set_read_callback: c"pa_stream_set_read_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_request_cb_t, *mut c_void),
    pa_stream_set_overflow_callback: c"pa_stream_set_overflow_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_set_underflow_callback: c"pa_stream_set_underflow_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_set_started_callback: c"pa_stream_set_started_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_set_latency_update_callback: c"pa_stream_set_latency_update_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_set_moved_callback: c"pa_stream_set_moved_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_set_suspended_callback: c"pa_stream_set_suspended_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_set_event_callback: c"pa_stream_set_event_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_event_cb_t, *mut c_void),
    pa_stream_set_buffer_attr_callback: c"pa_stream_set_buffer_attr_callback" => unsafe extern "C" fn(*mut pa_stream, pa_stream_notify_cb_t, *mut c_void),
    pa_stream_cork: c"pa_stream_cork" => unsafe extern "C" fn(*mut pa_stream, c_int, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_flush: c"pa_stream_flush" => unsafe extern "C" fn(*mut pa_stream, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_prebuf: c"pa_stream_prebuf" => unsafe extern "C" fn(*mut pa_stream, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_trigger: c"pa_stream_trigger" => unsafe extern "C" fn(*mut pa_stream, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_set_name: c"pa_stream_set_name" => unsafe extern "C" fn(*mut pa_stream, *const c_char, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_get_time: c"pa_stream_get_time" => unsafe extern "C" fn(*mut pa_stream, *mut pa_usec_t) -> c_int,
    pa_stream_get_latency: c"pa_stream_get_latency" => unsafe extern "C" fn(*mut pa_stream, *mut pa_usec_t, *mut c_int) -> c_int,
    pa_stream_get_timing_info: c"pa_stream_get_timing_info" => unsafe extern "C" fn(*mut pa_stream) -> *const pa_timing_info,
    pa_stream_get_sample_spec: c"pa_stream_get_sample_spec" => unsafe extern "C" fn(*mut pa_stream) -> *const pa_sample_spec,
    pa_stream_get_channel_map: c"pa_stream_get_channel_map" => unsafe extern "C" fn(*mut pa_stream) -> *const pa_channel_map,
    pa_stream_get_format_info: c"pa_stream_get_format_info" => unsafe extern "C" fn(*const pa_stream) -> *const pa_format_info,
    pa_stream_get_buffer_attr: c"pa_stream_get_buffer_attr" => unsafe extern "C" fn(*mut pa_stream) -> *const pa_buffer_attr,
    pa_stream_set_buffer_attr: c"pa_stream_set_buffer_attr" => unsafe extern "C" fn(*mut pa_stream, *const pa_buffer_attr, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_update_sample_rate: c"pa_stream_update_sample_rate" => unsafe extern "C" fn(*mut pa_stream, u32, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_proplist_update: c"pa_stream_proplist_update" => unsafe extern "C" fn(*mut pa_stream, pa_update_mode_t, *mut pa_proplist, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_proplist_remove: c"pa_stream_proplist_remove" => unsafe extern "C" fn(*mut pa_stream, *const *const c_char, pa_stream_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_stream_set_monitor_stream: c"pa_stream_set_monitor_stream" => unsafe extern "C" fn(*mut pa_stream, u32) -> c_int,
    pa_stream_get_monitor_stream: c"pa_stream_get_monitor_stream" => unsafe extern "C" fn(*const pa_stream) -> u32,
    pa_stream_get_underflow_index: c"pa_stream_get_underflow_index" => unsafe extern "C" fn(*const pa_stream) -> i64,
    pa_operation_ref: c"pa_operation_ref" => unsafe extern "C" fn(*mut pa_operation) -> *mut pa_operation,
    pa_operation_unref: c"pa_operation_unref" => unsafe extern "C" fn(*mut pa_operation),
    pa_operation_cancel: c"pa_operation_cancel" => unsafe extern "C" fn(*mut pa_operation),
    pa_operation_get_state: c"pa_operation_get_state" => unsafe extern "C" fn(*const pa_operation) -> pa_operation_state_t,
    pa_operation_set_state_callback: c"pa_operation_set_state_callback" => unsafe extern "C" fn(*mut pa_operation, pa_operation_notify_cb_t, *mut c_void),
    pa_threaded_mainloop_new: c"pa_threaded_mainloop_new" => unsafe extern "C" fn() -> *mut pa_threaded_mainloop,
    pa_threaded_mainloop_free: c"pa_threaded_mainloop_free" => unsafe extern "C" fn(*mut pa_threaded_mainloop),
    pa_threaded_mainloop_get_api: c"pa_threaded_mainloop_get_api" => unsafe extern "C" fn(*mut pa_threaded_mainloop) -> *mut pa_mainloop_api,
    pa_threaded_mainloop_signal: c"pa_threaded_mainloop_signal" => unsafe extern "C" fn(*mut pa_threaded_mainloop, c_int),
    pa_ext_device_restore_test: c"pa_ext_device_restore_test" => unsafe extern "C" fn(*mut pa_context, pa_ext_device_restore_test_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_restore_subscribe: c"pa_ext_device_restore_subscribe" => unsafe extern "C" fn(*mut pa_context, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_restore_set_subscribe_cb: c"pa_ext_device_restore_set_subscribe_cb" => unsafe extern "C" fn(*mut pa_context, pa_ext_device_restore_subscribe_cb_t, *mut c_void),
    pa_ext_device_restore_read_formats_all: c"pa_ext_device_restore_read_formats_all" => unsafe extern "C" fn(*mut pa_context, pa_ext_device_restore_read_device_formats_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_restore_read_formats: c"pa_ext_device_restore_read_formats" => unsafe extern "C" fn(*mut pa_context, pa_device_type_t, u32, pa_ext_device_restore_read_device_formats_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_restore_save_formats: c"pa_ext_device_restore_save_formats" => unsafe extern "C" fn(*mut pa_context, pa_device_type_t, u32, u8, *mut *mut pa_format_info, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_test: c"pa_ext_device_manager_test" => unsafe extern "C" fn(*mut pa_context, pa_ext_device_manager_test_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_read: c"pa_ext_device_manager_read" => unsafe extern "C" fn(*mut pa_context, pa_ext_device_manager_read_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_set_device_description: c"pa_ext_device_manager_set_device_description" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_delete: c"pa_ext_device_manager_delete" => unsafe extern "C" fn(*mut pa_context, *const *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_enable_role_device_priority_routing: c"pa_ext_device_manager_enable_role_device_priority_routing" => unsafe extern "C" fn(*mut pa_context, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_reorder_devices_for_role: c"pa_ext_device_manager_reorder_devices_for_role" => unsafe extern "C" fn(*mut pa_context, *const c_char, *const *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_subscribe: c"pa_ext_device_manager_subscribe" => unsafe extern "C" fn(*mut pa_context, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_device_manager_set_subscribe_cb: c"pa_ext_device_manager_set_subscribe_cb" => unsafe extern "C" fn(*mut pa_context, pa_ext_device_manager_subscribe_cb_t, *mut c_void),
    pa_ext_stream_restore_test: c"pa_ext_stream_restore_test" => unsafe extern "C" fn(*mut pa_context, pa_ext_stream_restore_test_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_stream_restore_read: c"pa_ext_stream_restore_read" => unsafe extern "C" fn(*mut pa_context, pa_ext_stream_restore_read_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_stream_restore_write: c"pa_ext_stream_restore_write" => unsafe extern "C" fn(*mut pa_context, pa_update_mode_t, *const pa_ext_stream_restore_info, c_uint, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_stream_restore_delete: c"pa_ext_stream_restore_delete" => unsafe extern "C" fn(*mut pa_context, *const *const c_char, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_stream_restore_subscribe: c"pa_ext_stream_restore_subscribe" => unsafe extern "C" fn(*mut pa_context, c_int, pa_context_success_cb_t, *mut c_void) -> *mut pa_operation,
    pa_ext_stream_restore_set_subscribe_cb: c"pa_ext_stream_restore_set_subscribe_cb" => unsafe extern "C" fn(*mut pa_context, pa_ext_stream_restore_subscribe_cb_t, *mut c_void),
    // Utility helpers resolved from real libpulse (used internally for deep copies/validation)
    pa_proplist_copy: c"pa_proplist_copy" => unsafe extern "C" fn(*const pa_proplist) -> *mut pa_proplist,
    pa_proplist_free: c"pa_proplist_free" => unsafe extern "C" fn(*mut pa_proplist),
    pa_format_info_copy: c"pa_format_info_copy" => unsafe extern "C" fn(*const pa_format_info) -> *mut pa_format_info,
    pa_format_info_free: c"pa_format_info_free" => unsafe extern "C" fn(*mut pa_format_info),
    pa_format_info_to_sample_spec: c"pa_format_info_to_sample_spec" => unsafe extern "C" fn(*const pa_format_info, *mut pa_sample_spec, *mut pa_channel_map) -> c_int,
    pa_sample_spec_init: c"pa_sample_spec_init" => unsafe extern "C" fn(*mut pa_sample_spec) -> *mut pa_sample_spec,
    pa_sample_spec_valid: c"pa_sample_spec_valid" => unsafe extern "C" fn(*const pa_sample_spec) -> c_int,
    pa_channel_map_init: c"pa_channel_map_init" => unsafe extern "C" fn(*mut pa_channel_map) -> *mut pa_channel_map,
    pa_channel_map_init_auto: c"pa_channel_map_init_auto" => unsafe extern "C" fn(*mut pa_channel_map, c_uint, pa_channel_map_def_t) -> *mut pa_channel_map,
    pa_channel_map_init_extend: c"pa_channel_map_init_extend" => unsafe extern "C" fn(*mut pa_channel_map, c_uint, pa_channel_map_def_t) -> *mut pa_channel_map,
    pa_channel_map_init_stereo: c"pa_channel_map_init_stereo" => unsafe extern "C" fn(*mut pa_channel_map) -> *mut pa_channel_map,
    pa_channel_map_valid: c"pa_channel_map_valid" => unsafe extern "C" fn(*const pa_channel_map) -> c_int,
}

static REAL_SYMS: SyncCell<RealSyms> = SyncCell::new(RealSyms::empty());
static INIT_ONCE: Once = Once::new();

#[inline]
unsafe fn real() -> &'static RealSyms {
    &*REAL_SYMS.get()
}

pub(crate) unsafe fn ensure_init() {
    INIT_ONCE.call_once(|| {
        crate::resolve_real_syms(&mut *REAL_SYMS.get());
    });
}

unsafe extern "C" fn salsa_pa_reconnect_ctor() {
    ensure_init();
}

#[used]
#[link_section = ".init_array"]
static SALSA_PA_RECONNECT_CTOR: unsafe extern "C" fn() = salsa_pa_reconnect_ctor;

#[derive(Clone, Copy)]
struct MlEntry {
    api: *mut pa_mainloop_api,
    ml: *mut pa_threaded_mainloop,
}

static G_ML_LOCK: SyncCell<PthreadMutex> = SyncCell::new([0; 8]);
static G_ML_MAP: SyncCell<[MlEntry; MAX_MAINLOOPS]> = SyncCell::new(
    [MlEntry {
        api: ptr::null_mut(),
        ml: ptr::null_mut(),
    }; MAX_MAINLOOPS],
);

unsafe fn register_ml(api: *mut pa_mainloop_api, ml: *mut pa_threaded_mainloop) {
    if api.is_null() || ml.is_null() {
        return;
    }
    pthread_mutex_lock(G_ML_LOCK.get());
    let map = &mut *G_ML_MAP.get();
    for slot in map.iter_mut() {
        if slot.api == api || slot.ml == ml || slot.api.is_null() {
            slot.api = api;
            slot.ml = ml;
            break;
        }
    }
    pthread_mutex_unlock(G_ML_LOCK.get());
}

unsafe fn unregister_ml(ml: *mut pa_threaded_mainloop) {
    if ml.is_null() {
        return;
    }
    pthread_mutex_lock(G_ML_LOCK.get());
    let map = &mut *G_ML_MAP.get();
    for slot in map.iter_mut() {
        if slot.ml == ml {
            slot.api = ptr::null_mut();
            slot.ml = ptr::null_mut();
        }
    }
    pthread_mutex_unlock(G_ML_LOCK.get());

    pthread_mutex_lock(G_CTX_LOCK.get());
    let mut c = *G_CTX_HEAD.get();
    while !c.is_null() {
        if (*c).tmainloop == ml {
            (*c).tmainloop = ptr::null_mut();
        }
        c = (*c).next;
    }
    pthread_mutex_unlock(G_CTX_LOCK.get());
}

unsafe fn lookup_ml(api: *mut pa_mainloop_api) -> *mut pa_threaded_mainloop {
    if api.is_null() {
        return ptr::null_mut();
    }
    let mut res = ptr::null_mut();
    pthread_mutex_lock(G_ML_LOCK.get());
    let map = &*G_ML_MAP.get();
    for slot in map.iter() {
        if slot.api == api {
            res = slot.ml;
            break;
        }
    }
    pthread_mutex_unlock(G_ML_LOCK.get());
    res
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
enum SynthKind {
    Success = 0,
    StreamSuccess,
    ServerInfo,
    SinkList,
    SinkByName,
    SinkByIndex,
    SourceList,
    SourceByName,
    SourceByIndex,
    EmptyEol,
}

#[repr(C)]
struct Wop {
    r#ref: AtomicI32,
    state: pa_operation_state_t,
    state_cb: pa_operation_notify_cb_t,
    state_userdata: *mut c_void,
    wctx: *mut Wctx,
    stream: *mut Wstream,
    kind: SynthKind,
    user_cb: *mut c_void,
    user_data: *mut c_void,
    target_name: *mut c_char,
    target_idx: u32,
    defer_ev: *mut pa_defer_event,
    next: *mut Wop,
}

static G_WOP_LOCK: SyncCell<PthreadMutex> = SyncCell::new([0; 8]);
static G_WOP_HEAD: SyncCell<*mut Wop> = SyncCell::new(ptr::null_mut());

unsafe fn is_synth_op(o: *const pa_operation) -> bool {
    if o.is_null() {
        return false;
    }
    let mut found = false;
    pthread_mutex_lock(G_WOP_LOCK.get());
    let mut p = *G_WOP_HEAD.get();
    while !p.is_null() {
        if p as *const pa_operation == o {
            found = true;
            break;
        }
        p = (*p).next;
    }
    pthread_mutex_unlock(G_WOP_LOCK.get());
    found
}

#[repr(C)]
struct PaCache {
    has_server: bool,
    server: pa_server_info,
    user_name: *mut c_char,
    host_name: *mut c_char,
    server_version: *mut c_char,
    server_name: *mut c_char,
    default_sink_name: *mut c_char,
    default_source_name: *mut c_char,
    sinks: *mut pa_sink_info,
    n_sinks: usize,
    sources: *mut pa_source_info,
    n_sources: usize,
}

static G_GLOBAL_CACHE_LOCK: SyncCell<PthreadMutex> = SyncCell::new([0; 8]);
static G_GLOBAL_CACHE: SyncCell<PaCache> = SyncCell::new(PaCache {
    has_server: false,
    server: pa_server_info {
        user_name: ptr::null(),
        host_name: ptr::null(),
        server_version: ptr::null(),
        server_name: ptr::null(),
        sample_spec: pa_sample_spec {
            format: 0,
            rate: 0,
            channels: 0,
        },
        default_sink_name: ptr::null(),
        default_source_name: ptr::null(),
        cookie: 0,
        channel_map: pa_channel_map {
            channels: 0,
            map: [0; 32],
        },
    },
    user_name: ptr::null_mut(),
    host_name: ptr::null_mut(),
    server_version: ptr::null_mut(),
    server_name: ptr::null_mut(),
    default_sink_name: ptr::null_mut(),
    default_source_name: ptr::null_mut(),
    sinks: ptr::null_mut(),
    n_sinks: 0,
    sources: ptr::null_mut(),
    n_sources: 0,
});

unsafe fn free_sink_entry(s: *mut pa_sink_info) {
    if s.is_null() {
        return;
    }
    free((*s).name.cast_mut().cast());
    free((*s).description.cast_mut().cast());
    free((*s).monitor_source_name.cast_mut().cast());
    free((*s).driver.cast_mut().cast());
    if !(*s).proplist.is_null() {
        if let Some(f) = real().pa_proplist_free {
            f((*s).proplist);
        }
    }
    if !(*s).ports.is_null() {
        for i in 0..(*s).n_ports as usize {
            let p = *(*s).ports.add(i);
            if !p.is_null() {
                free((*p).name.cast_mut().cast());
                free((*p).description.cast_mut().cast());
                free((*p).availability_group.cast_mut().cast());
                free(p.cast());
            }
        }
        free((*s).ports.cast());
    }
    if !(*s).formats.is_null() {
        for i in 0..(*s).n_formats as usize {
            let f = *(*s).formats.add(i);
            if !f.is_null() {
                if let Some(ff) = real().pa_format_info_free {
                    ff(f);
                }
            }
        }
        free((*s).formats.cast());
    }
}

unsafe fn free_source_entry(s: *mut pa_source_info) {
    if s.is_null() {
        return;
    }
    free((*s).name.cast_mut().cast());
    free((*s).description.cast_mut().cast());
    free((*s).monitor_of_sink_name.cast_mut().cast());
    free((*s).driver.cast_mut().cast());
    if !(*s).proplist.is_null() {
        if let Some(f) = real().pa_proplist_free {
            f((*s).proplist);
        }
    }
    if !(*s).ports.is_null() {
        for i in 0..(*s).n_ports as usize {
            let p = *(*s).ports.add(i);
            if !p.is_null() {
                free((*p).name.cast_mut().cast());
                free((*p).description.cast_mut().cast());
                free((*p).availability_group.cast_mut().cast());
                free(p.cast());
            }
        }
        free((*s).ports.cast());
    }
    if !(*s).formats.is_null() {
        for i in 0..(*s).n_formats as usize {
            let f = *(*s).formats.add(i);
            if !f.is_null() {
                if let Some(ff) = real().pa_format_info_free {
                    ff(f);
                }
            }
        }
        free((*s).formats.cast());
    }
}

unsafe fn copy_sink_entry(dst: *mut pa_sink_info, src: *const pa_sink_info) {
    ptr::write_bytes(dst, 0, 1);
    *dst = *src;
    (*dst).name = xstrdup((*src).name);
    (*dst).description = xstrdup((*src).description);
    (*dst).monitor_source_name = xstrdup((*src).monitor_source_name);
    (*dst).driver = xstrdup((*src).driver);
    (*dst).proplist = if !(*src).proplist.is_null() {
        real().pa_proplist_copy.map_or(ptr::null_mut(), |f| f((*src).proplist))
    } else {
        ptr::null_mut()
    };
    (*dst).ports = ptr::null_mut();
    (*dst).active_port = ptr::null_mut();
    if (*src).n_ports > 0 && !(*src).ports.is_null() {
        let arr = calloc(
            (*src).n_ports as usize + 1,
            core::mem::size_of::<*mut pa_sink_port_info>(),
        )
        .cast::<*mut pa_sink_port_info>();
        (*dst).ports = arr;
        let mut actual_ports = 0usize;
        if !arr.is_null() {
            for i in 0..(*src).n_ports as usize {
                let sp = *(*src).ports.add(i);
                if !sp.is_null() {
                    let p = calloc(1, core::mem::size_of::<pa_sink_port_info>())
                        .cast::<pa_sink_port_info>();
                    if !p.is_null() {
                        *p = *sp;
                        (*p).name = xstrdup((*sp).name);
                        (*p).description = xstrdup((*sp).description);
                        (*p).availability_group = xstrdup((*sp).availability_group);
                        *arr.add(actual_ports) = p;
                        actual_ports += 1;
                        if (*src).active_port == sp {
                            (*dst).active_port = p;
                        }
                    }
                }
            }
        }
        (*dst).n_ports = actual_ports as u32;
    } else {
        (*dst).n_ports = 0;
    }
    (*dst).formats = ptr::null_mut();
    if (*src).n_formats > 0 && !(*src).formats.is_null() {
        let arr = calloc(
            (*src).n_formats as usize,
            core::mem::size_of::<*mut pa_format_info>(),
        )
        .cast::<*mut pa_format_info>();
        (*dst).formats = arr;
        let mut actual_fmts = 0usize;
        if !arr.is_null() {
            for i in 0..(*src).n_formats as usize {
                let sf = *(*src).formats.add(i);
                if !sf.is_null() {
                    if let Some(cf) = real().pa_format_info_copy {
                        let copied = cf(sf);
                        if !copied.is_null() {
                            *arr.add(actual_fmts) = copied;
                            actual_fmts += 1;
                        }
                    }
                }
            }
        }
        (*dst).n_formats = actual_fmts as u8;
    } else {
        (*dst).n_formats = 0;
    }
}

unsafe fn copy_source_entry(dst: *mut pa_source_info, src: *const pa_source_info) {
    ptr::write_bytes(dst, 0, 1);
    *dst = *src;
    (*dst).name = xstrdup((*src).name);
    (*dst).description = xstrdup((*src).description);
    (*dst).monitor_of_sink_name = xstrdup((*src).monitor_of_sink_name);
    (*dst).driver = xstrdup((*src).driver);
    (*dst).proplist = if !(*src).proplist.is_null() {
        real().pa_proplist_copy.map_or(ptr::null_mut(), |f| f((*src).proplist))
    } else {
        ptr::null_mut()
    };
    (*dst).ports = ptr::null_mut();
    (*dst).active_port = ptr::null_mut();
    if (*src).n_ports > 0 && !(*src).ports.is_null() {
        let arr = calloc(
            (*src).n_ports as usize + 1,
            core::mem::size_of::<*mut pa_source_port_info>(),
        )
        .cast::<*mut pa_source_port_info>();
        (*dst).ports = arr;
        let mut actual_ports = 0usize;
        if !arr.is_null() {
            for i in 0..(*src).n_ports as usize {
                let sp = *(*src).ports.add(i);
                if !sp.is_null() {
                    let p = calloc(1, core::mem::size_of::<pa_source_port_info>())
                        .cast::<pa_source_port_info>();
                    if !p.is_null() {
                        *p = *sp;
                        (*p).name = xstrdup((*sp).name);
                        (*p).description = xstrdup((*sp).description);
                        (*p).availability_group = xstrdup((*sp).availability_group);
                        *arr.add(actual_ports) = p;
                        actual_ports += 1;
                        if (*src).active_port == sp {
                            (*dst).active_port = p;
                        }
                    }
                }
            }
        }
        (*dst).n_ports = actual_ports as u32;
    } else {
        (*dst).n_ports = 0;
    }
    (*dst).formats = ptr::null_mut();
    if (*src).n_formats > 0 && !(*src).formats.is_null() {
        let arr = calloc(
            (*src).n_formats as usize,
            core::mem::size_of::<*mut pa_format_info>(),
        )
        .cast::<*mut pa_format_info>();
        (*dst).formats = arr;
        let mut actual_fmts = 0usize;
        if !arr.is_null() {
            for i in 0..(*src).n_formats as usize {
                let sf = *(*src).formats.add(i);
                if !sf.is_null() {
                    if let Some(cf) = real().pa_format_info_copy {
                        let copied = cf(sf);
                        if !copied.is_null() {
                            *arr.add(actual_fmts) = copied;
                            actual_fmts += 1;
                        }
                    }
                }
            }
        }
        (*dst).n_formats = actual_fmts as u8;
    } else {
        (*dst).n_formats = 0;
    }
}

unsafe fn cache_set_server(c: *mut PaCache, info: *const pa_server_info) {
    if c.is_null() || info.is_null() {
        return;
    }
    free((*c).user_name.cast());
    free((*c).host_name.cast());
    free((*c).server_version.cast());
    free((*c).server_name.cast());
    free((*c).default_sink_name.cast());
    free((*c).default_source_name.cast());
    (*c).server = *info;
    (*c).user_name = xstrdup((*info).user_name);
    (*c).host_name = xstrdup((*info).host_name);
    (*c).server_version = xstrdup((*info).server_version);
    (*c).server_name = xstrdup((*info).server_name);
    (*c).default_sink_name = xstrdup((*info).default_sink_name);
    (*c).default_source_name = xstrdup((*info).default_source_name);
    (*c).server.user_name = (*c).user_name;
    (*c).server.host_name = (*c).host_name;
    (*c).server.server_version = (*c).server_version;
    (*c).server.server_name = (*c).server_name;
    (*c).server.default_sink_name = (*c).default_sink_name;
    (*c).server.default_source_name = (*c).default_source_name;
    (*c).has_server = true;
}

unsafe fn cache_replace_sinks(c: *mut PaCache, items: *mut pa_sink_info, count: usize) {
    for i in 0..(*c).n_sinks {
        free_sink_entry((*c).sinks.add(i));
    }
    free((*c).sinks.cast());
    (*c).sinks = items;
    (*c).n_sinks = count;
}

unsafe fn cache_replace_sources(c: *mut PaCache, items: *mut pa_source_info, count: usize) {
    for i in 0..(*c).n_sources {
        free_source_entry((*c).sources.add(i));
    }
    free((*c).sources.cast());
    (*c).sources = items;
    (*c).n_sources = count;
}

unsafe fn cache_free(c: *mut PaCache) {
    free((*c).user_name.cast());
    free((*c).host_name.cast());
    free((*c).server_version.cast());
    free((*c).server_name.cast());
    free((*c).default_sink_name.cast());
    free((*c).default_source_name.cast());
    cache_replace_sinks(c, ptr::null_mut(), 0);
    cache_replace_sources(c, ptr::null_mut(), 0);
    ptr::write_bytes(c, 0, 1);
}

unsafe fn clone_cache_snapshot(wctx: *mut Wctx) -> *mut PaCache {
    let snap = calloc(1, core::mem::size_of::<PaCache>()).cast::<PaCache>();
    if snap.is_null() {
        return ptr::null_mut();
    }
    let has_wctx = is_valid_wctx(wctx);
    if has_wctx {
        pthread_mutex_lock(&raw mut (*wctx).lock);
    }
    pthread_mutex_lock(G_GLOBAL_CACHE_LOCK.get());
    let src = pick_cache_locked(if has_wctx { wctx } else { ptr::null_mut() });
    if (*src).has_server {
        cache_set_server(snap, &raw const (*src).server);
    } else {
        (*snap).default_sink_name = xstrdup((*src).default_sink_name);
        (*snap).default_source_name = xstrdup((*src).default_source_name);
    }
    if (*src).n_sinks > 0 && !(*src).sinks.is_null() {
        let arr = calloc((*src).n_sinks, core::mem::size_of::<pa_sink_info>()).cast::<pa_sink_info>();
        if !arr.is_null() {
            for k in 0..(*src).n_sinks {
                copy_sink_entry(arr.add(k), (*src).sinks.add(k));
            }
            (*snap).sinks = arr;
            (*snap).n_sinks = (*src).n_sinks;
        }
    }
    if (*src).n_sources > 0 && !(*src).sources.is_null() {
        let arr = calloc(
            (*src).n_sources,
            core::mem::size_of::<pa_source_info>(),
        )
        .cast::<pa_source_info>();
        if !arr.is_null() {
            for k in 0..(*src).n_sources {
                copy_source_entry(arr.add(k), (*src).sources.add(k));
            }
            (*snap).sources = arr;
            (*snap).n_sources = (*src).n_sources;
        }
    }
    pthread_mutex_unlock(G_GLOBAL_CACHE_LOCK.get());
    if has_wctx {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
    }
    snap
}

#[repr(C)]
struct Wstream {
    r#ref: AtomicI32,
    wctx: *mut Wctx,
    real_stream: *mut pa_stream,
    reported_state: pa_stream_state_t,
    ever_ready: bool,
    user_connected: bool,
    user_disconnected: bool,
    is_record: bool,
    reconnecting: bool,
    reconnect_retries: c_int,

    name: *mut c_char,
    ss: pa_sample_spec,
    cmap: pa_channel_map,
    has_cmap: bool,
    proplist: *mut pa_proplist,
    formats: *mut *mut pa_format_info,
    n_formats: c_uint,
    created_extended: bool,

    dev: *mut c_char,
    attr: pa_buffer_attr,
    has_attr: bool,
    flags: pa_stream_flags_t,
    volume: pa_cvolume,
    has_volume: bool,
    sync_stream: *mut Wstream,
    corked: c_int,
    has_monitor_stream: bool,
    monitor_stream: u32,

    last_stream_index: u32,
    last_device_index: u32,
    last_timing: pa_timing_info,
    has_last_timing: bool,
    time_offset_us: pa_usec_t,
    last_reported_time_us: pa_usec_t,
    dummy_write_buf: *mut c_void,
    dummy_write_cap: usize,
    using_dummy_write: bool,

    state_cb: pa_stream_notify_cb_t,
    state_ud: *mut c_void,
    write_cb: pa_stream_request_cb_t,
    write_ud: *mut c_void,
    read_cb: pa_stream_request_cb_t,
    read_ud: *mut c_void,
    overflow_cb: pa_stream_notify_cb_t,
    overflow_ud: *mut c_void,
    underflow_cb: pa_stream_notify_cb_t,
    underflow_ud: *mut c_void,
    started_cb: pa_stream_notify_cb_t,
    started_ud: *mut c_void,
    latency_cb: pa_stream_notify_cb_t,
    latency_ud: *mut c_void,
    moved_cb: pa_stream_notify_cb_t,
    moved_ud: *mut c_void,
    suspended_cb: pa_stream_notify_cb_t,
    suspended_ud: *mut c_void,
    event_cb: pa_stream_event_cb_t,
    event_ud: *mut c_void,
    buffer_attr_cb: pa_stream_notify_cb_t,
    buffer_attr_ud: *mut c_void,

    next: *mut Wstream,
    global_next: *mut Wstream,
}

#[repr(C)]
struct DeadHandle {
    ptr: *mut c_void,
    is_ctx: bool,
    next: *mut DeadHandle,
}

#[repr(C)]
struct Wctx {
    r#ref: AtomicI32,
    lock: PthreadMutex,
    api: *mut pa_mainloop_api,
    tmainloop: *mut pa_threaded_mainloop,
    real_ctx: *mut pa_context,

    app_name: *mut c_char,
    props: *mut pa_proplist,
    server: *mut c_char,
    server_set: bool,
    flags: pa_context_flags_t,
    spawn_api: pa_spawn_api,
    has_spawn_api: bool,

    reported_state: pa_context_state_t,
    last_errno: c_int,
    ever_ready: bool,
    reconnecting: bool,
    user_connected: bool,
    user_disconnected: bool,
    reconnect_attempts: c_int,

    state_cb: pa_context_notify_cb_t,
    state_ud: *mut c_void,
    event_cb: pa_context_event_cb_t,
    event_ud: *mut c_void,
    sub_cb: pa_context_subscribe_cb_t,
    sub_ud: *mut c_void,
    has_subscribe: bool,
    sub_mask: pa_subscription_mask_t,

    ext_dev_restore_sub_cb: pa_ext_device_restore_subscribe_cb_t,
    ext_dev_restore_sub_ud: *mut c_void,
    ext_dev_restore_sub_enabled: bool,
    ext_dev_mgr_sub_cb: pa_ext_device_manager_subscribe_cb_t,
    ext_dev_mgr_sub_ud: *mut c_void,
    ext_dev_mgr_sub_enabled: bool,
    ext_stream_restore_sub_cb: pa_ext_stream_restore_subscribe_cb_t,
    ext_stream_restore_sub_ud: *mut c_void,
    ext_stream_restore_sub_enabled: bool,

    reconnect_timer: *mut pa_time_event,
    refresh_defer: *mut pa_defer_event,

    cache: PaCache,
    streams: *mut Wstream,
    dead_handles: *mut DeadHandle,
    next: *mut Wctx,
}

static G_CTX_LOCK: SyncCell<PthreadMutex> = SyncCell::new([0; 8]);
static G_CTX_HEAD: SyncCell<*mut Wctx> = SyncCell::new(ptr::null_mut());
static G_STREAM_LOCK: SyncCell<PthreadMutex> = SyncCell::new([0; 8]);
static G_STREAM_HEAD: SyncCell<*mut Wstream> = SyncCell::new(ptr::null_mut());

unsafe fn is_valid_wctx(c: *const Wctx) -> bool {
    if c.is_null() {
        return false;
    }
    let mut ok = false;
    pthread_mutex_lock(G_CTX_LOCK.get());
    let mut p = *G_CTX_HEAD.get();
    while !p.is_null() {
        if core::ptr::eq(p, c) {
            ok = true;
            break;
        }
        p = (*p).next;
    }
    pthread_mutex_unlock(G_CTX_LOCK.get());
    ok
}

unsafe fn is_valid_wstream(s: *const Wstream) -> bool {
    if s.is_null() {
        return false;
    }
    let mut ok = false;
    pthread_mutex_lock(G_STREAM_LOCK.get());
    let mut ws = *G_STREAM_HEAD.get();
    while !ws.is_null() {
        if core::ptr::eq(ws, s) {
            ok = true;
            break;
        }
        ws = (*ws).global_next;
    }
    pthread_mutex_unlock(G_STREAM_LOCK.get());
    ok
}

unsafe fn snapshot_stream_time_locked(ws: *mut Wstream) {
    if ws.is_null() {
        return;
    }
    if !(*ws).real_stream.is_null() {
        if let Some(f) = real().pa_stream_get_time {
            let mut raw: pa_usec_t = 0;
            if f((*ws).real_stream, &raw mut raw) == 0 {
                let total = (*ws).time_offset_us.saturating_add(raw);
                if total > (*ws).last_reported_time_us {
                    (*ws).last_reported_time_us = total;
                }
            }
        }
    }
    (*ws).time_offset_us = (*ws).last_reported_time_us;
}

unsafe fn wop_free(op: *mut Wop) {
    if op.is_null() {
        return;
    }
    pthread_mutex_lock(G_WOP_LOCK.get());
    let mut pp = G_WOP_HEAD.get();
    while !(*pp).is_null() {
        if *pp == op {
            *pp = (*op).next;
            break;
        }
        pp = &raw mut (**pp).next;
    }
    pthread_mutex_unlock(G_WOP_LOCK.get());
    if !(*op).defer_ev.is_null()
        && !(*op).wctx.is_null()
        && is_valid_wctx((*op).wctx)
        && !(*(*op).wctx).api.is_null()
    {
        if let Some(df) = (*(*(*op).wctx).api).defer_free {
            df((*op).defer_ev);
        }
        (*op).defer_ev = ptr::null_mut();
    }
    if !(*op).stream.is_null() {
        pa_stream_unref((*op).stream.cast());
        (*op).stream = ptr::null_mut();
    }
    free((*op).target_name.cast());
    free(op.cast());
}

unsafe extern "C" fn deliver_synth_op_cb(
    a: *mut pa_mainloop_api,
    e: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    let op = userdata.cast::<Wop>();
    if !is_synth_op(op.cast()) {
        return;
    }
    if (*op).defer_ev == e && !a.is_null() {
        if let Some(df) = (*a).defer_free {
            df(e);
        }
        (*op).defer_ev = ptr::null_mut();
    }

    if (*op).state == PA_OPERATION_RUNNING {
        let wctx = (*op).wctx;
        if is_valid_wctx(wctx) {
            pa_context_ref(wctx.cast());
            let mut cache: *mut PaCache = ptr::null_mut();
            if !matches!(
                (*op).kind,
                SynthKind::Success | SynthKind::StreamSuccess | SynthKind::EmptyEol
            ) {
                cache = clone_cache_snapshot(wctx);
            }

            match (*op).kind {
                SynthKind::Success => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(*mut pa_context, c_int, *mut c_void) =
                            core::mem::transmute((*op).user_cb);
                        cb(wctx.cast(), 1, (*op).user_data);
                    }
                }
                SynthKind::StreamSuccess => {
                    if !(*op).user_cb.is_null()
                        && is_valid_wstream((*op).stream)
                        && !(*(*op).stream).user_disconnected
                    {
                        let cb: unsafe extern "C" fn(*mut pa_stream, c_int, *mut c_void) =
                            core::mem::transmute((*op).user_cb);
                        cb((*op).stream.cast(), 1, (*op).user_data);
                    }
                }
                SynthKind::ServerInfo => {
                    if !(*op).user_cb.is_null() {
                        let si = if !cache.is_null() && (*cache).has_server {
                            &raw const (*cache).server
                        } else {
                            ptr::null()
                        };
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_server_info,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        cb(wctx.cast(), si, (*op).user_data);
                    }
                }
                SynthKind::SinkList => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_sink_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        if !cache.is_null() {
                            for i in 0..(*cache).n_sinks {
                                cb(wctx.cast(), (*cache).sinks.add(i), 0, (*op).user_data);
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*op).user_data);
                    }
                }
                SynthKind::SinkByName => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_sink_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        if !cache.is_null() {
                            let mut t: *const c_char = (*op).target_name;
                            if !t.is_null()
                                && (strcmp(t, c"@DEFAULT_SINK@".as_ptr()) == 0
                                    || strcmp(t, c"default".as_ptr()) == 0)
                            {
                                t = (*cache).default_sink_name;
                            }
                            for i in 0..(*cache).n_sinks {
                                let s = (*cache).sinks.add(i);
                                if (!t.is_null() && !(*s).name.is_null() && strcmp((*s).name, t) == 0)
                                    || (t.is_null() && i == 0)
                                {
                                    cb(wctx.cast(), s, 0, (*op).user_data);
                                    break;
                                }
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*op).user_data);
                    }
                }
                SynthKind::SinkByIndex => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_sink_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        if !cache.is_null() {
                            for i in 0..(*cache).n_sinks {
                                let s = (*cache).sinks.add(i);
                                if (*s).index == (*op).target_idx {
                                    cb(wctx.cast(), s, 0, (*op).user_data);
                                    break;
                                }
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*op).user_data);
                    }
                }
                SynthKind::SourceList => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_source_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        if !cache.is_null() {
                            for i in 0..(*cache).n_sources {
                                cb(wctx.cast(), (*cache).sources.add(i), 0, (*op).user_data);
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*op).user_data);
                    }
                }
                SynthKind::SourceByName => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_source_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        if !cache.is_null() {
                            let mut t: *const c_char = (*op).target_name;
                            if !t.is_null()
                                && (strcmp(t, c"@DEFAULT_SOURCE@".as_ptr()) == 0
                                    || strcmp(t, c"default".as_ptr()) == 0)
                            {
                                t = (*cache).default_source_name;
                            }
                            for i in 0..(*cache).n_sources {
                                let s = (*cache).sources.add(i);
                                if (!t.is_null() && !(*s).name.is_null() && strcmp((*s).name, t) == 0)
                                    || (t.is_null() && i == 0)
                                {
                                    cb(wctx.cast(), s, 0, (*op).user_data);
                                    break;
                                }
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*op).user_data);
                    }
                }
                SynthKind::SourceByIndex => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_source_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        if !cache.is_null() {
                            for i in 0..(*cache).n_sources {
                                let s = (*cache).sources.add(i);
                                if (*s).index == (*op).target_idx {
                                    cb(wctx.cast(), s, 0, (*op).user_data);
                                    break;
                                }
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*op).user_data);
                    }
                }
                SynthKind::EmptyEol => {
                    if !(*op).user_cb.is_null() {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const c_void,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*op).user_cb);
                        cb(wctx.cast(), ptr::null(), 1, (*op).user_data);
                    }
                }
            }
            if !cache.is_null() {
                cache_free(cache);
                free(cache.cast());
            }
            pa_context_unref(wctx.cast());
        }
        (*op).state = PA_OPERATION_DONE;
        if let Some(scb) = (*op).state_cb {
            scb(op.cast(), (*op).state_userdata);
        }
        if is_valid_wctx(wctx) && !(*wctx).tmainloop.is_null() {
            if let Some(sig) = real().pa_threaded_mainloop_signal {
                sig((*wctx).tmainloop, 0);
            }
        }
    }

    if (*op).r#ref.fetch_sub(1, Ordering::AcqRel) == 1 {
        wop_free(op);
    }
}

unsafe fn spawn_synth_op_ex(
    wctx: *mut Wctx,
    stream: *mut Wstream,
    kind: SynthKind,
    cb: *mut c_void,
    ud: *mut c_void,
    target_name: *const c_char,
    target_idx: u32,
) -> *mut pa_operation {
    if wctx.is_null() || (*wctx).api.is_null() {
        return ptr::null_mut();
    }
    let Some(defer_new) = (*(*wctx).api).defer_new else {
        return ptr::null_mut();
    };
    let op = calloc(1, core::mem::size_of::<Wop>()).cast::<Wop>();
    if op.is_null() {
        return ptr::null_mut();
    }
    (*op).r#ref = AtomicI32::new(2);
    (*op).state = PA_OPERATION_RUNNING;
    (*op).wctx = wctx;
    (*op).stream = if !stream.is_null() {
        pa_stream_ref(stream.cast()).cast()
    } else {
        ptr::null_mut()
    };
    (*op).kind = kind;
    (*op).user_cb = cb;
    (*op).user_data = ud;
    (*op).target_name = xstrdup(target_name);
    (*op).target_idx = target_idx;

    pthread_mutex_lock(G_WOP_LOCK.get());
    (*op).next = *G_WOP_HEAD.get();
    *G_WOP_HEAD.get() = op;
    pthread_mutex_unlock(G_WOP_LOCK.get());

    (*op).defer_ev = defer_new((*wctx).api, Some(deliver_synth_op_cb), op.cast());
    if (*op).defer_ev.is_null() {
        wop_free(op);
        return ptr::null_mut();
    }
    op.cast()
}

unsafe fn spawn_synth_op(
    wctx: *mut Wctx,
    kind: SynthKind,
    cb: *mut c_void,
    ud: *mut c_void,
    target_name: *const c_char,
    target_idx: u32,
) -> *mut pa_operation {
    spawn_synth_op_ex(wctx, ptr::null_mut(), kind, cb, ud, target_name, target_idx)
}

unsafe fn spawn_synth_stream_op(
    ws: *mut Wstream,
    cb: *mut c_void,
    ud: *mut c_void,
) -> *mut pa_operation {
    if ws.is_null() || (*ws).wctx.is_null() {
        return ptr::null_mut();
    }
    spawn_synth_op_ex(
        (*ws).wctx,
        ws,
        SynthKind::StreamSuccess,
        cb,
        ud,
        ptr::null(),
        0,
    )
}

unsafe extern "C" fn refresh_defer_cb(
    a: *mut pa_mainloop_api,
    e: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    let wctx = userdata.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return;
    }
    if (*wctx).refresh_defer == e && !a.is_null() {
        if let Some(df) = (*a).defer_free {
            df(e);
        }
        (*wctx).refresh_defer = ptr::null_mut();
    }
    if (*wctx).has_subscribe && !(*wctx).user_disconnected {
        if let Some(sub_cb) = (*wctx).sub_cb {
            if ((*wctx).sub_mask & PA_SUBSCRIPTION_MASK_SERVER) != 0 {
                sub_cb(
                    wctx.cast(),
                    PA_SUBSCRIPTION_EVENT_SERVER | PA_SUBSCRIPTION_EVENT_CHANGE,
                    0,
                    (*wctx).sub_ud,
                );
            }
            if ((*wctx).sub_mask & PA_SUBSCRIPTION_MASK_SINK) != 0 {
                sub_cb(
                    wctx.cast(),
                    PA_SUBSCRIPTION_EVENT_SINK | PA_SUBSCRIPTION_EVENT_NEW,
                    0,
                    (*wctx).sub_ud,
                );
            }
            if ((*wctx).sub_mask & PA_SUBSCRIPTION_MASK_SOURCE) != 0 {
                sub_cb(
                    wctx.cast(),
                    PA_SUBSCRIPTION_EVENT_SOURCE | PA_SUBSCRIPTION_EVENT_NEW,
                    0,
                    (*wctx).sub_ud,
                );
            }
        }
    }
    if !(*wctx).tmainloop.is_null() {
        if let Some(sig) = real().pa_threaded_mainloop_signal {
            sig((*wctx).tmainloop, 0);
        }
    }
}

unsafe fn detach_ctx_callbacks(rc: *mut pa_context) {
    if rc.is_null() {
        return;
    }
    if let Some(f) = real().pa_context_set_state_callback {
        f(rc, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_context_set_event_callback {
        f(rc, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_context_set_subscribe_callback {
        f(rc, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_ext_device_restore_set_subscribe_cb {
        f(rc, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_ext_device_manager_set_subscribe_cb {
        f(rc, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_ext_stream_restore_set_subscribe_cb {
        f(rc, None, ptr::null_mut());
    }
}

unsafe fn detach_stream_callbacks(rs: *mut pa_stream) {
    if rs.is_null() {
        return;
    }
    if let Some(f) = real().pa_stream_set_state_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_write_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_read_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_overflow_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_underflow_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_started_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_latency_update_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_moved_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_suspended_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_event_callback {
        f(rs, None, ptr::null_mut());
    }
    if let Some(f) = real().pa_stream_set_buffer_attr_callback {
        f(rs, None, ptr::null_mut());
    }
}

unsafe fn queue_dead_handle(wctx: *mut Wctx, ptr_h: *mut c_void, is_ctx: bool) {
    if wctx.is_null() || ptr_h.is_null() {
        return;
    }
    let dh = malloc(core::mem::size_of::<DeadHandle>()).cast::<DeadHandle>();
    if dh.is_null() {
        if is_ctx {
            if let Some(f) = real().pa_context_unref {
                f(ptr_h.cast());
            }
        } else if let Some(f) = real().pa_stream_unref {
            f(ptr_h.cast());
        }
        return;
    }
    (*dh).ptr = ptr_h;
    (*dh).is_ctx = is_ctx;
    (*dh).next = (*wctx).dead_handles;
    (*wctx).dead_handles = dh;
}

unsafe fn detach_dead_handles_locked(wctx: *mut Wctx) -> *mut DeadHandle {
    if wctx.is_null() {
        return ptr::null_mut();
    }
    let list = (*wctx).dead_handles;
    (*wctx).dead_handles = ptr::null_mut();
    list
}

unsafe fn flush_dead_handle_list(list: *mut DeadHandle) {
    if list.is_null() {
        return;
    }
    let mut dh = list;
    while !dh.is_null() {
        if !(*dh).is_ctx && !(*dh).ptr.is_null() {
            if let Some(f) = real().pa_stream_unref {
                f((*dh).ptr.cast());
            }
            (*dh).ptr = ptr::null_mut();
        }
        dh = (*dh).next;
    }
    let mut dh = list;
    while !dh.is_null() {
        let nx = (*dh).next;
        if (*dh).is_ctx && !(*dh).ptr.is_null() {
            let old_rc = (*dh).ptr.cast::<pa_context>();
            if let Some(f) = real().pa_context_unref {
                f(old_rc);
            }
            reap_tees_for_rc(old_rc);
        }
        free(dh.cast());
        dh = nx;
    }
}

unsafe extern "C" fn internal_ext_dev_restore_sub_cb(
    rc: *mut pa_context,
    dtype: pa_device_type_t,
    idx: u32,
    ud: *mut c_void,
) {
    let wctx = ud.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx != rc {
        return;
    }
    pa_context_ref(wctx.cast());
    if let Some(cb) = (*wctx).ext_dev_restore_sub_cb {
        cb(wctx.cast(), dtype, idx, (*wctx).ext_dev_restore_sub_ud);
    }
    pa_context_unref(wctx.cast());
}

unsafe extern "C" fn internal_ext_dev_mgr_sub_cb(rc: *mut pa_context, ud: *mut c_void) {
    let wctx = ud.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx != rc {
        return;
    }
    pa_context_ref(wctx.cast());
    if let Some(cb) = (*wctx).ext_dev_mgr_sub_cb {
        cb(wctx.cast(), (*wctx).ext_dev_mgr_sub_ud);
    }
    pa_context_unref(wctx.cast());
}

unsafe extern "C" fn internal_ext_stream_restore_sub_cb(rc: *mut pa_context, ud: *mut c_void) {
    let wctx = ud.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx != rc {
        return;
    }
    pa_context_ref(wctx.cast());
    if let Some(cb) = (*wctx).ext_stream_restore_sub_cb {
        cb(wctx.cast(), (*wctx).ext_stream_restore_sub_ud);
    }
    pa_context_unref(wctx.cast());
}

unsafe fn server_allows_tcp_fallback(server: *const c_char, server_set: bool) -> bool {
    if !server_set || server.is_null() {
        return true;
    }
    let Ok(s) = core::ffi::CStr::from_ptr(server).to_str() else {
        return false;
    };
    let trimmed = s.trim();
    trimmed.is_empty()
        || trimmed.starts_with("unix:")
        || trimmed.starts_with('/')
        || trimmed.contains("/pulse/native")
}

fn parse_salsa_tcp_port_str(s: &str) -> Option<u16> {
    let port: u16 = s.trim().parse().ok()?;
    (port > 0).then_some(port)
}

fn read_salsa_tcp_target() -> Option<(std::ffi::CString, Option<std::ffi::CString>)> {
    let home = std::env::var("HOME").ok()?;
    if !home.starts_with('/') {
        return None;
    }
    let port_path = format!("{home}/.config/pulse/salsa-tcp-port");
    let raw = std::fs::read_to_string(&port_path).ok()?;
    let port = parse_salsa_tcp_port_str(&raw)?;
    let srv = std::ffi::CString::new(format!("tcp4:127.0.0.1:{port}")).ok()?;
    let cookie_path = format!("{home}/.config/pulse/cookie");
    let cookie = std::fs::metadata(&cookie_path)
        .ok()
        .filter(|m| m.is_file() && m.len() == 256)
        .and_then(|_| std::ffi::CString::new(cookie_path).ok());
    Some((srv, cookie))
}

unsafe fn should_try_tcp_fallback(
    reconnect_attempts: c_int,
    server: *const c_char,
    server_set: bool,
) -> bool {
    reconnect_attempts >= 2
        && (reconnect_attempts % 2 == 0)
        && server_allows_tcp_fallback(server, server_set)
}

unsafe extern "C" fn reconnect_timer_cb(
    a: *mut pa_mainloop_api,
    e: *mut pa_time_event,
    _tv: *const TimeVal,
    userdata: *mut c_void,
) {
    let wctx = userdata.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return;
    }
    pa_context_ref(wctx.cast());

    pthread_mutex_lock(&raw mut (*wctx).lock);
    if (*wctx).reconnect_timer == e && !a.is_null() {
        if let Some(tf) = (*a).time_free {
            tf(e);
        }
        (*wctx).reconnect_timer = ptr::null_mut();
    }
    let dead = detach_dead_handles_locked(wctx);
    let mut old_rc: *mut pa_context = ptr::null_mut();
    if !(*wctx).user_disconnected && (*wctx).reconnecting && !(*wctx).real_ctx.is_null() {
        old_rc = (*wctx).real_ctx;
        (*wctx).real_ctx = ptr::null_mut();
        detach_ctx_callbacks(old_rc);
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);

    flush_dead_handle_list(dead);
    if !old_rc.is_null() {
        if let Some(f) = real().pa_context_unref {
            f(old_rc);
        }
        reap_tees_for_rc(old_rc);
    }

    if !is_valid_wctx(wctx) {
        pa_context_unref(wctx.cast());
        return;
    }

    pthread_mutex_lock(&raw mut (*wctx).lock);
    if (*wctx).user_disconnected || !(*wctx).reconnecting {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        pa_context_unref(wctx.cast());
        return;
    }

    let nrc = real()
        .pa_context_new_with_proplist
        .map_or(ptr::null_mut(), |f| {
            f((*wctx).api, (*wctx).app_name, (*wctx).props)
        });
    if nrc.is_null() {
        schedule_reconnect_timer(wctx, 200_000);
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        pa_context_unref(wctx.cast());
        return;
    }

    (*wctx).real_ctx = nrc;
    if let Some(f) = real().pa_context_set_state_callback {
        f(nrc, Some(internal_ctx_state_cb), wctx.cast());
    }
    if let Some(f) = real().pa_context_set_event_callback {
        f(nrc, Some(internal_ctx_event_cb), wctx.cast());
    }
    if let Some(f) = real().pa_context_set_subscribe_callback {
        f(nrc, Some(internal_ctx_sub_cb), wctx.cast());
    }
    if (*wctx).ext_dev_restore_sub_cb.is_some() {
        if let Some(f) = real().pa_ext_device_restore_set_subscribe_cb {
            f(nrc, Some(internal_ext_dev_restore_sub_cb), wctx.cast());
        }
    }
    if (*wctx).ext_dev_mgr_sub_cb.is_some() {
        if let Some(f) = real().pa_ext_device_manager_set_subscribe_cb {
            f(nrc, Some(internal_ext_dev_mgr_sub_cb), wctx.cast());
        }
    }
    if (*wctx).ext_stream_restore_sub_cb.is_some() {
        if let Some(f) = real().pa_ext_stream_restore_set_subscribe_cb {
            f(nrc, Some(internal_ext_stream_restore_sub_cb), wctx.cast());
        }
    }

    let cflags = (*wctx).flags & !PA_CONTEXT_NOFAIL;
    let tcp_fallback = if should_try_tcp_fallback(
        (*wctx).reconnect_attempts,
        (*wctx).server,
        (*wctx).server_set,
    ) {
        read_salsa_tcp_target()
    } else {
        None
    };
    if let Some((_, Some(ref ck))) = tcp_fallback {
        if let Some(load_ck) = real().pa_context_load_cookie_from_file {
            let _ = load_ck(nrc, ck.as_ptr());
        }
    }
    let target_srv = if let Some((ref srv, _)) = tcp_fallback {
        srv.as_ptr()
    } else if (*wctx).server_set {
        (*wctx).server
    } else {
        ptr::null()
    };
    let conn_rc = real().pa_context_connect.map_or(-1, |f| {
        f(
            nrc,
            target_srv,
            cflags,
            if (*wctx).has_spawn_api {
                &raw const (*wctx).spawn_api
            } else {
                ptr::null()
            },
        )
    });
    if conn_rc < 0 && (*wctx).real_ctx == nrc {
        detach_ctx_callbacks(nrc);
        (*wctx).real_ctx = ptr::null_mut();
        (*wctx).reconnect_attempts = (*wctx).reconnect_attempts.saturating_add(1);
        schedule_reconnect_timer(wctx, 200_000);
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        if let Some(f) = real().pa_context_unref {
            f(nrc);
        }
        reap_tees_for_rc(nrc);
        pa_context_unref(wctx.cast());
        return;
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    pa_context_unref(wctx.cast());
}

unsafe fn schedule_reconnect_timer(wctx: *mut Wctx, delay_us: u64) {
    if wctx.is_null() || (*wctx).api.is_null() || (*wctx).user_disconnected {
        return;
    }
    let Some(time_new) = (*(*wctx).api).time_new else {
        return;
    };
    if !(*wctx).reconnect_timer.is_null() {
        if let Some(tf) = (*(*wctx).api).time_free {
            tf((*wctx).reconnect_timer);
        }
        (*wctx).reconnect_timer = ptr::null_mut();
    }
    let mut tv = TimeVal::default();
    gettimeofday(&raw mut tv, ptr::null_mut());
    tv.tv_usec += (delay_us % 1_000_000) as c_long;
    tv.tv_sec += (delay_us / 1_000_000) as c_long;
    if tv.tv_usec >= 1_000_000 {
        tv.tv_sec += 1;
        tv.tv_usec -= 1_000_000;
    }
    (*wctx).reconnect_timer = time_new((*wctx).api, &raw const tv, Some(reconnect_timer_cb), wctx.cast());
}

unsafe extern "C" fn internal_ctx_event_cb(
    rc: *mut pa_context,
    name: *const c_char,
    p: *mut pa_proplist,
    ud: *mut c_void,
) {
    let wctx = ud.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx != rc {
        return;
    }
    if let Some(cb) = (*wctx).event_cb {
        cb(wctx.cast(), name, p, (*wctx).event_ud);
    }
}

unsafe extern "C" fn internal_ctx_sub_cb(
    rc: *mut pa_context,
    t: pa_subscription_event_type_t,
    idx: u32,
    ud: *mut c_void,
) {
    let wctx = ud.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx != rc {
        return;
    }
    if let Some(cb) = (*wctx).sub_cb {
        cb(wctx.cast(), t, idx, (*wctx).sub_ud);
    }
}

unsafe extern "C" fn internal_ctx_state_cb(rc: *mut pa_context, ud: *mut c_void) {
    let wctx = ud.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return;
    }

    pthread_mutex_lock(&raw mut (*wctx).lock);
    if (*wctx).real_ctx != rc {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return;
    }
    pa_context_ref(wctx.cast());

    let st = real()
        .pa_context_get_state
        .map_or(PA_CONTEXT_FAILED, |f| f(rc));
    if st == PA_CONTEXT_READY {
        let was_recovery = (*wctx).ever_ready && (*wctx).reconnecting;
        (*wctx).ever_ready = true;
        (*wctx).reconnecting = false;
        (*wctx).reconnect_attempts = 0;
        (*wctx).reported_state = PA_CONTEXT_READY;
        (*wctx).last_errno = PA_OK;

        if !(*wctx).reconnect_timer.is_null() && !(*wctx).api.is_null() {
            if let Some(tf) = (*(*wctx).api).time_free {
                tf((*wctx).reconnect_timer);
            }
            (*wctx).reconnect_timer = ptr::null_mut();
        }

        if !was_recovery {
            let cb = (*wctx).state_cb;
            let cud = (*wctx).state_ud;
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            if let Some(f) = cb {
                f(wctx.cast(), cud);
            }
            pa_context_unref(wctx.cast());
            return;
        }

        if (*wctx).has_subscribe {
            if let Some(f) = real().pa_context_subscribe {
                let sop = f(rc, (*wctx).sub_mask, None, ptr::null_mut());
                if !sop.is_null() {
                    if let Some(u) = real().pa_operation_unref {
                        u(sop);
                    }
                }
            }
        }
        if (*wctx).ext_dev_restore_sub_enabled {
            if let Some(f) = real().pa_ext_device_restore_subscribe {
                let sop = f(rc, 1, None, ptr::null_mut());
                if !sop.is_null() {
                    if let Some(u) = real().pa_operation_unref {
                        u(sop);
                    }
                }
            }
        }
        if (*wctx).ext_dev_mgr_sub_enabled {
            if let Some(f) = real().pa_ext_device_manager_subscribe {
                let sop = f(rc, 1, None, ptr::null_mut());
                if !sop.is_null() {
                    if let Some(u) = real().pa_operation_unref {
                        u(sop);
                    }
                }
            }
        }
        if (*wctx).ext_stream_restore_sub_enabled {
            if let Some(f) = real().pa_ext_stream_restore_subscribe {
                let sop = f(rc, 1, None, ptr::null_mut());
                if !sop.is_null() {
                    if let Some(u) = real().pa_operation_unref {
                        u(sop);
                    }
                }
            }
        }

        let mut ws = (*wctx).streams;
        while !ws.is_null() {
            if (*ws).user_connected && !(*ws).user_disconnected {
                (*ws).reconnect_retries = 0;
                attach_real_stream(ws);
            }
            ws = (*ws).next;
        }

        if (*wctx).sub_cb.is_some() && !(*wctx).api.is_null() {
            if let Some(defer_new) = (*(*wctx).api).defer_new {
                if !(*wctx).refresh_defer.is_null() {
                    if let Some(df) = (*(*wctx).api).defer_free {
                        df((*wctx).refresh_defer);
                    }
                }
                (*wctx).refresh_defer =
                    defer_new((*wctx).api, Some(refresh_defer_cb), wctx.cast());
            }
        }
        if !(*wctx).tmainloop.is_null() {
            if let Some(sig) = real().pa_threaded_mainloop_signal {
                sig((*wctx).tmainloop, 0);
            }
        }
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        pa_context_unref(wctx.cast());
        return;
    }

    if st == PA_CONTEXT_FAILED && !(*wctx).user_disconnected {
        (*wctx).last_errno = real().pa_context_errno.map_or(PA_ERR_INVALID, |f| f(rc));
        (*wctx).reconnect_attempts += 1;

        let allow_retry = (*wctx).ever_ready
            || ((*wctx).flags & PA_CONTEXT_NOFAIL) != 0
            || (*wctx).reconnect_attempts <= 10;
        if allow_retry {
            (*wctx).reconnecting = true;
            let mut ws = (*wctx).streams;
            while !ws.is_null() {
                if !(*ws).real_stream.is_null() {
                    snapshot_stream_time_locked(ws);
                    let old_s = (*ws).real_stream;
                    (*ws).real_stream = ptr::null_mut();
                    (*ws).reconnecting = (*ws).user_connected && !(*ws).user_disconnected;
                    detach_stream_callbacks(old_s);
                    queue_dead_handle(wctx, old_s.cast(), false);
                }
                ws = (*ws).next;
            }
            (*wctx).real_ctx = ptr::null_mut();
            detach_ctx_callbacks(rc);
            queue_dead_handle(wctx, rc.cast(), true);

            let delay = if (*wctx).reconnect_attempts == 1 {
                50_000
            } else {
                150_000
            };
            schedule_reconnect_timer(wctx, delay);
            let tml = (*wctx).tmainloop;
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            replay_inflight_tees(wctx, rc);
            if !tml.is_null() {
                if let Some(sig) = real().pa_threaded_mainloop_signal {
                    sig(tml, 0);
                }
            }
            pa_context_unref(wctx.cast());
            return;
        }
    }

    (*wctx).reported_state = st;
    if st == PA_CONTEXT_FAILED || st == PA_CONTEXT_TERMINATED {
        (*wctx).last_errno = real().pa_context_errno.map_or(PA_ERR_INVALID, |f| f(rc));
    }
    let cb = (*wctx).state_cb;
    let cud = (*wctx).state_ud;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    if let Some(f) = cb {
        f(wctx.cast(), cud);
    }
    pa_context_unref(wctx.cast());
}

unsafe extern "C" fn internal_stream_state_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if ws.is_null() {
        return;
    }
    let mut wctx: *mut Wctx = ptr::null_mut();
    pthread_mutex_lock(G_STREAM_LOCK.get());
    let mut p = *G_STREAM_HEAD.get();
    while !p.is_null() {
        if p == ws {
            (*ws).r#ref.fetch_add(1, Ordering::AcqRel);
            wctx = (*ws).wctx;
            break;
        }
        p = (*p).global_next;
    }
    pthread_mutex_unlock(G_STREAM_LOCK.get());
    if wctx.is_null() {
        return;
    }

    pthread_mutex_lock(&raw mut (*wctx).lock);
    if (*ws).real_stream != rs {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        pa_stream_unref(ws.cast());
        return;
    }
    let st = real()
        .pa_stream_get_state
        .map_or(PA_STREAM_FAILED, |f| f(rs));
    if st == PA_STREAM_READY {
        let was_reconnecting = (*ws).reconnecting;
        (*ws).ever_ready = true;
        (*ws).reconnecting = false;
        (*ws).reported_state = PA_STREAM_READY;
        (*ws).last_stream_index = real()
            .pa_stream_get_index
            .map_or(PA_INVALID_INDEX, |f| f(rs));
        (*ws).last_device_index = real()
            .pa_stream_get_device_index
            .map_or(PA_INVALID_INDEX, |f| f(rs));
        if let Some(f) = real().pa_stream_get_sample_spec {
            let nss = f(rs);
            if !nss.is_null()
                && real()
                    .pa_sample_spec_valid
                    .is_some_and(|v| v(nss) != 0)
            {
                (*ws).ss = *nss;
            }
        }
        if let Some(f) = real().pa_stream_get_channel_map {
            let ncm = f(rs);
            if !ncm.is_null()
                && real()
                    .pa_channel_map_valid
                    .is_some_and(|v| v(ncm) != 0)
            {
                (*ws).cmap = *ncm;
                (*ws).has_cmap = true;
            }
        }
        if let Some(f) = real().pa_stream_get_buffer_attr {
            let nba = f(rs);
            if !nba.is_null() {
                (*ws).attr = *nba;
                (*ws).has_attr = true;
            }
        }

        let cur_corked = real().pa_stream_is_corked.map_or(-1, |f| f(rs));
        if cur_corked >= 0 && (cur_corked != 0) != ((*ws).corked != 0) {
            if let Some(f) = real().pa_stream_cork {
                let cop = f(rs, if (*ws).corked != 0 { 1 } else { 0 }, None, ptr::null_mut());
                if !cop.is_null() {
                    if let Some(u) = real().pa_operation_unref {
                        u(cop);
                    }
                }
            }
        }
        if was_reconnecting {
            if let Some(f) = real().pa_stream_update_timing_info {
                let top = f(rs, None, ptr::null_mut());
                if !top.is_null() {
                    if let Some(u) = real().pa_operation_unref {
                        u(top);
                    }
                }
            }
        }

        if (*ws).state_cb.is_some() && (*ws).reconnect_retries == 0 {
            let cb = (*ws).state_cb;
            let sud = (*ws).state_ud;
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            if let Some(f) = cb {
                f(ws.cast(), sud);
            }
            pa_stream_unref(ws.cast());
            return;
        }
        if !(*wctx).tmainloop.is_null() {
            if let Some(sig) = real().pa_threaded_mainloop_signal {
                sig((*wctx).tmainloop, 0);
            }
        }
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        pa_stream_unref(ws.cast());
        return;
    }

    if st == PA_STREAM_FAILED
        && (*ws).user_connected
        && !(*ws).user_disconnected
        && !(*ws).reconnecting
    {
        let ctx_not_ready = !(*wctx).real_ctx.is_null()
            && real()
                .pa_context_get_state
                .is_some_and(|f| f((*wctx).real_ctx) != PA_CONTEXT_READY);
        if (*wctx).reconnecting || ctx_not_ready {
            snapshot_stream_time_locked(ws);
            (*ws).reconnecting = true;
            (*ws).real_stream = ptr::null_mut();
            detach_stream_callbacks(rs);
            queue_dead_handle(wctx, rs.cast(), false);
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            pa_stream_unref(ws.cast());
            return;
        }
    }

    if st == PA_STREAM_FAILED
        && (*ws).reconnecting
        && (*ws).user_connected
        && !(*ws).user_disconnected
    {
        (*ws).reconnect_retries += 1;
        if (*ws).reconnect_retries <= 3 {
            snapshot_stream_time_locked(ws);
            (*ws).real_stream = ptr::null_mut();
            detach_stream_callbacks(rs);
            queue_dead_handle(wctx, rs.cast(), false);
            if (*ws).reconnect_retries >= 2 && !(*ws).dev.is_null() {
                free((*ws).dev.cast());
                (*ws).dev = ptr::null_mut();
            }
            attach_real_stream(ws);
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            pa_stream_unref(ws.cast());
            return;
        }
    }

    if !(*ws).reconnecting {
        (*ws).reported_state = st;
        let cb = (*ws).state_cb;
        let sud = (*ws).state_ud;
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        if let Some(f) = cb {
            f(ws.cast(), sud);
        }
        pa_stream_unref(ws.cast());
        return;
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    pa_stream_unref(ws.cast());
}

unsafe extern "C" fn internal_stream_write_cb(rs: *mut pa_stream, nbytes: usize, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).write_cb {
        cb(ws.cast(), nbytes, (*ws).write_ud);
    }
}

unsafe extern "C" fn internal_stream_read_cb(rs: *mut pa_stream, nbytes: usize, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).read_cb {
        cb(ws.cast(), nbytes, (*ws).read_ud);
    }
}

unsafe extern "C" fn internal_stream_overflow_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).overflow_cb {
        cb(ws.cast(), (*ws).overflow_ud);
    }
}

unsafe extern "C" fn internal_stream_underflow_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).underflow_cb {
        cb(ws.cast(), (*ws).underflow_ud);
    }
}

unsafe extern "C" fn internal_stream_started_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).started_cb {
        cb(ws.cast(), (*ws).started_ud);
    }
}

unsafe extern "C" fn internal_stream_latency_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).latency_cb {
        cb(ws.cast(), (*ws).latency_ud);
    }
}

unsafe extern "C" fn internal_stream_moved_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(f) = real().pa_stream_get_device_index {
        (*ws).last_device_index = f(rs);
    }
    if let Some(cb) = (*ws).moved_cb {
        cb(ws.cast(), (*ws).moved_ud);
    }
}

unsafe extern "C" fn internal_stream_suspended_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).suspended_cb {
        cb(ws.cast(), (*ws).suspended_ud);
    }
}

unsafe extern "C" fn internal_stream_event_cb(
    rs: *mut pa_stream,
    name: *const c_char,
    pl: *mut pa_proplist,
    ud: *mut c_void,
) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).event_cb {
        cb(ws.cast(), name, pl, (*ws).event_ud);
    }
}

unsafe extern "C" fn internal_stream_buffer_attr_cb(rs: *mut pa_stream, ud: *mut c_void) {
    let ws = ud.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream != rs {
        return;
    }
    if let Some(cb) = (*ws).buffer_attr_cb {
        cb(ws.cast(), (*ws).buffer_attr_ud);
    }
}

unsafe fn attach_real_stream(ws: *mut Wstream) {
    let wctx = (*ws).wctx;
    if wctx.is_null() || (*wctx).real_ctx.is_null() {
        return;
    }
    if !(*ws).real_stream.is_null() {
        snapshot_stream_time_locked(ws);
        let old = (*ws).real_stream;
        (*ws).real_stream = ptr::null_mut();
        detach_stream_callbacks(old);
        queue_dead_handle(wctx, old.cast(), false);
    }

    let nrs = if (*ws).created_extended {
        real().pa_stream_new_extended.map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                (*ws).name,
                (*ws).formats,
                (*ws).n_formats,
                (*ws).proplist,
            )
        })
    } else {
        real()
            .pa_stream_new_with_proplist
            .map_or(ptr::null_mut(), |f| {
                f(
                    (*wctx).real_ctx,
                    (*ws).name,
                    &raw const (*ws).ss,
                    if (*ws).has_cmap {
                        &raw const (*ws).cmap
                    } else {
                        ptr::null()
                    },
                    (*ws).proplist,
                )
            })
    };
    if nrs.is_null() {
        return;
    }

    (*ws).real_stream = nrs;
    if let Some(f) = real().pa_stream_set_state_callback {
        f(nrs, Some(internal_stream_state_cb), ws.cast());
    }
    if (*ws).write_cb.is_some() {
        if let Some(f) = real().pa_stream_set_write_callback {
            f(nrs, Some(internal_stream_write_cb), ws.cast());
        }
    }
    if (*ws).read_cb.is_some() {
        if let Some(f) = real().pa_stream_set_read_callback {
            f(nrs, Some(internal_stream_read_cb), ws.cast());
        }
    }
    if (*ws).overflow_cb.is_some() {
        if let Some(f) = real().pa_stream_set_overflow_callback {
            f(nrs, Some(internal_stream_overflow_cb), ws.cast());
        }
    }
    if (*ws).underflow_cb.is_some() {
        if let Some(f) = real().pa_stream_set_underflow_callback {
            f(nrs, Some(internal_stream_underflow_cb), ws.cast());
        }
    }
    if (*ws).started_cb.is_some() {
        if let Some(f) = real().pa_stream_set_started_callback {
            f(nrs, Some(internal_stream_started_cb), ws.cast());
        }
    }
    if (*ws).latency_cb.is_some() {
        if let Some(f) = real().pa_stream_set_latency_update_callback {
            f(nrs, Some(internal_stream_latency_cb), ws.cast());
        }
    }
    if (*ws).moved_cb.is_some() {
        if let Some(f) = real().pa_stream_set_moved_callback {
            f(nrs, Some(internal_stream_moved_cb), ws.cast());
        }
    }
    if (*ws).suspended_cb.is_some() {
        if let Some(f) = real().pa_stream_set_suspended_callback {
            f(nrs, Some(internal_stream_suspended_cb), ws.cast());
        }
    }
    if (*ws).event_cb.is_some() {
        if let Some(f) = real().pa_stream_set_event_callback {
            f(nrs, Some(internal_stream_event_cb), ws.cast());
        }
    }
    if (*ws).buffer_attr_cb.is_some() {
        if let Some(f) = real().pa_stream_set_buffer_attr_callback {
            f(nrs, Some(internal_stream_buffer_attr_cb), ws.cast());
        }
    }
    if (*ws).has_monitor_stream {
        if let Some(f) = real().pa_stream_set_monitor_stream {
            f(nrs, (*ws).monitor_stream);
        }
    }

    let mut rflags = (*ws).flags & !PA_STREAM_START_CORKED;
    if (*ws).corked != 0 {
        rflags |= PA_STREAM_START_CORKED;
    }

    let attr_p = if (*ws).has_attr {
        &raw const (*ws).attr
    } else {
        ptr::null()
    };
    let vol_p = if (*ws).has_volume {
        &raw const (*ws).volume
    } else {
        ptr::null()
    };
    let rs_sync = if !(*ws).sync_stream.is_null() && is_valid_wstream((*ws).sync_stream) {
        (*(*ws).sync_stream).real_stream
    } else {
        ptr::null_mut()
    };

    let rc = if (*ws).is_record {
        real()
            .pa_stream_connect_record
            .map_or(-1, |f| f(nrs, (*ws).dev, attr_p, rflags))
    } else {
        real()
            .pa_stream_connect_playback
            .map_or(-1, |f| f(nrs, (*ws).dev, attr_p, rflags, vol_p, rs_sync))
    };

    if rc < 0 && !(*ws).dev.is_null() {
        free((*ws).dev.cast());
        (*ws).dev = ptr::null_mut();
        if (*ws).is_record {
            if let Some(f) = real().pa_stream_connect_record {
                f(nrs, ptr::null(), attr_p, rflags);
            }
        } else if let Some(f) = real().pa_stream_connect_playback {
            f(nrs, ptr::null(), attr_p, rflags, vol_p, ptr::null_mut());
        }
    }
}

// ============================================================================
// Mainloop & Operation Wrappers
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_new() -> *mut pa_threaded_mainloop {
    ensure_init();
    let m = real()
        .pa_threaded_mainloop_new
        .map_or(ptr::null_mut(), |f| f());
    if !m.is_null() {
        if let Some(get_api) = real().pa_threaded_mainloop_get_api {
            register_ml(get_api(m), m);
        }
    }
    m
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_free(m: *mut pa_threaded_mainloop) {
    ensure_init();
    unregister_ml(m);
    if let Some(f) = real().pa_threaded_mainloop_free {
        f(m);
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_threaded_mainloop_get_api(
    m: *mut pa_threaded_mainloop,
) -> *mut pa_mainloop_api {
    ensure_init();
    let api = real()
        .pa_threaded_mainloop_get_api
        .map_or(ptr::null_mut(), |f| f(m));
    register_ml(api, m);
    api
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_ref(o: *mut pa_operation) -> *mut pa_operation {
    ensure_init();
    if is_synth_op(o) {
        (*(o.cast::<Wop>())).r#ref.fetch_add(1, Ordering::AcqRel);
        return o;
    }
    real()
        .pa_operation_ref
        .map_or(ptr::null_mut(), |f| f(o))
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_unref(o: *mut pa_operation) {
    ensure_init();
    if o.is_null() {
        return;
    }
    if is_synth_op(o) {
        let op = o.cast::<Wop>();
        if (*op).r#ref.fetch_sub(1, Ordering::AcqRel) == 1 {
            wop_free(op);
        }
        return;
    }
    if let Some(f) = real().pa_operation_unref {
        f(o);
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_cancel(o: *mut pa_operation) {
    ensure_init();
    if o.is_null() {
        return;
    }
    if is_synth_op(o) {
        let op = o.cast::<Wop>();
        (*op).state = PA_OPERATION_CANCELLED;
        if let Some(cb) = (*op).state_cb {
            cb(o, (*op).state_userdata);
        }
        return;
    }
    if let Some(f) = real().pa_operation_cancel {
        f(o);
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_get_state(o: *const pa_operation) -> pa_operation_state_t {
    ensure_init();
    if o.is_null() {
        return PA_OPERATION_CANCELLED;
    }
    if is_synth_op(o) {
        return (*(o.cast::<Wop>())).state;
    }
    real()
        .pa_operation_get_state
        .map_or(PA_OPERATION_CANCELLED, |f| f(o))
}

#[no_mangle]
pub unsafe extern "C" fn pa_operation_set_state_callback(
    o: *mut pa_operation,
    cb: pa_operation_notify_cb_t,
    userdata: *mut c_void,
) {
    ensure_init();
    if o.is_null() {
        return;
    }
    if is_synth_op(o) {
        let op = o.cast::<Wop>();
        (*op).state_cb = cb;
        (*op).state_userdata = userdata;
        return;
    }
    if let Some(f) = real().pa_operation_set_state_callback {
        f(o, cb, userdata);
    }
}

// ============================================================================
// Context Creation / Lifecycle / Connection
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn pa_context_new_with_proplist(
    mainloop: *mut pa_mainloop_api,
    name: *const c_char,
    proplist: *const pa_proplist,
) -> *mut pa_context {
    ensure_init();
    let rc = real()
        .pa_context_new_with_proplist
        .map_or(ptr::null_mut(), |f| f(mainloop, name, proplist));
    if rc.is_null() {
        return ptr::null_mut();
    }

    let wctx = calloc(1, core::mem::size_of::<Wctx>()).cast::<Wctx>();
    if wctx.is_null() {
        if let Some(f) = real().pa_context_unref {
            f(rc);
        }
        return ptr::null_mut();
    }
    (*wctx).r#ref = AtomicI32::new(1);
    let mut mattr: PthreadMutexAttr = [0; 2];
    pthread_mutexattr_init(&raw mut mattr);
    pthread_mutexattr_settype(&raw mut mattr, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&raw mut (*wctx).lock, &raw const mattr);
    pthread_mutexattr_destroy(&raw mut mattr);

    (*wctx).api = mainloop;
    (*wctx).tmainloop = lookup_ml(mainloop);
    (*wctx).real_ctx = rc;
    (*wctx).app_name = xstrdup(name);
    (*wctx).props = if !proplist.is_null() {
        real().pa_proplist_copy.map_or(ptr::null_mut(), |f| f(proplist))
    } else {
        ptr::null_mut()
    };
    (*wctx).reported_state = PA_CONTEXT_UNCONNECTED;

    if let Some(f) = real().pa_context_set_state_callback {
        f(rc, Some(internal_ctx_state_cb), wctx.cast());
    }
    if let Some(f) = real().pa_context_set_event_callback {
        f(rc, Some(internal_ctx_event_cb), wctx.cast());
    }
    if let Some(f) = real().pa_context_set_subscribe_callback {
        f(rc, Some(internal_ctx_sub_cb), wctx.cast());
    }

    pthread_mutex_lock(G_CTX_LOCK.get());
    (*wctx).next = *G_CTX_HEAD.get();
    *G_CTX_HEAD.get() = wctx;
    pthread_mutex_unlock(G_CTX_LOCK.get());

    wctx.cast()
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_new(
    mainloop: *mut pa_mainloop_api,
    name: *const c_char,
) -> *mut pa_context {
    pa_context_new_with_proplist(mainloop, name, ptr::null())
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_ref(c: *mut pa_context) -> *mut pa_context {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if is_valid_wctx(wctx) {
        (*wctx).r#ref.fetch_add(1, Ordering::AcqRel);
        return c;
    }
    real()
        .pa_context_ref
        .map_or(ptr::null_mut(), |f| f(c))
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_unref(c: *mut pa_context) {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        if !c.is_null() {
            if let Some(f) = real().pa_context_unref {
                f(c);
            }
        }
        return;
    }
    if (*wctx).r#ref.fetch_sub(1, Ordering::AcqRel) > 1 {
        return;
    }

    pthread_mutex_lock(G_CTX_LOCK.get());
    let mut pp = G_CTX_HEAD.get();
    while !(*pp).is_null() {
        if *pp == wctx {
            *pp = (*wctx).next;
            break;
        }
        pp = &raw mut (**pp).next;
    }
    pthread_mutex_unlock(G_CTX_LOCK.get());

    if !(*wctx).reconnect_timer.is_null() && !(*wctx).api.is_null() {
        if let Some(tf) = (*(*wctx).api).time_free {
            tf((*wctx).reconnect_timer);
        }
        (*wctx).reconnect_timer = ptr::null_mut();
    }
    if !(*wctx).refresh_defer.is_null() && !(*wctx).api.is_null() {
        if let Some(df) = (*(*wctx).api).defer_free {
            df((*wctx).refresh_defer);
        }
        (*wctx).refresh_defer = ptr::null_mut();
    }
    detach_tees_for_wctx(wctx);
    let orig_rc = (*wctx).real_ctx;
    (*wctx).real_ctx = ptr::null_mut();
    let dead = detach_dead_handles_locked(wctx);
    if !orig_rc.is_null() {
        detach_ctx_callbacks(orig_rc);
        if let Some(f) = real().pa_context_unref {
            f(orig_rc);
        }
        reap_tees_for_rc(orig_rc);
    }
    flush_dead_handle_list(dead);
    free((*wctx).app_name.cast());
    if !(*wctx).props.is_null() {
        if let Some(f) = real().pa_proplist_free {
            f((*wctx).props);
        }
    }
    free((*wctx).server.cast());
    cache_free(&raw mut (*wctx).cache);
    pthread_mutex_destroy(&raw mut (*wctx).lock);
    free(wctx.cast());
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_set_state_callback(
    c: *mut pa_context,
    cb: pa_context_notify_cb_t,
    userdata: *mut c_void,
) {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return;
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    (*wctx).state_cb = cb;
    (*wctx).state_ud = userdata;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_set_event_callback(
    c: *mut pa_context,
    cb: pa_context_event_cb_t,
    userdata: *mut c_void,
) {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return;
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    (*wctx).event_cb = cb;
    (*wctx).event_ud = userdata;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_set_subscribe_callback(
    c: *mut pa_context,
    cb: pa_context_subscribe_cb_t,
    userdata: *mut c_void,
) {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return;
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    (*wctx).sub_cb = cb;
    (*wctx).sub_ud = userdata;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_state(c: *const pa_context) -> pa_context_state_t {
    ensure_init();
    let wctx = c.cast_mut().cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return PA_CONTEXT_FAILED;
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let st = if (*wctx).ever_ready && !(*wctx).user_disconnected && (*wctx).reconnecting {
        PA_CONTEXT_READY
    } else if !(*wctx).real_ctx.is_null() {
        let mut s = real()
            .pa_context_get_state
            .map_or(PA_CONTEXT_FAILED, |f| f((*wctx).real_ctx));
        if (*wctx).ever_ready
            && !(*wctx).user_disconnected
            && (s == PA_CONTEXT_CONNECTING
                || s == PA_CONTEXT_AUTHORIZING
                || s == PA_CONTEXT_SETTING_NAME)
        {
            s = PA_CONTEXT_READY;
        }
        s
    } else {
        (*wctx).reported_state
    };
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    st
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_errno(c: *const pa_context) -> c_int {
    ensure_init();
    let wctx = c.cast_mut().cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return PA_ERR_INVALID;
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let e = if (*wctx).ever_ready && (*wctx).reconnecting {
        PA_OK
    } else if !(*wctx).real_ctx.is_null() {
        real()
            .pa_context_errno
            .map_or(PA_ERR_INVALID, |f| f((*wctx).real_ctx))
    } else {
        (*wctx).last_errno
    };
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    e
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_is_pending(c: *const pa_context) -> c_int {
    ensure_init();
    let wctx = c.cast_mut().cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return real().pa_context_is_pending.map_or(0, |f| f(c));
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let r = if !(*wctx).real_ctx.is_null() {
        real().pa_context_is_pending.map_or(0, |f| f((*wctx).real_ctx))
    } else {
        0
    };
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    r
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_connect(
    c: *mut pa_context,
    server: *const c_char,
    flags: pa_context_flags_t,
    api: *const pa_spawn_api,
) -> c_int {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return -1;
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    free((*wctx).server.cast());
    (*wctx).server = xstrdup(server);
    (*wctx).server_set = !server.is_null();
    (*wctx).flags = flags;
    if !api.is_null() {
        (*wctx).spawn_api = *api;
        (*wctx).has_spawn_api = true;
    } else {
        (*wctx).has_spawn_api = false;
    }
    (*wctx).user_connected = true;
    (*wctx).user_disconnected = false;
    (*wctx).reported_state = PA_CONTEXT_CONNECTING;

    let mut rc = -1;
    if !(*wctx).real_ctx.is_null() {
        rc = real().pa_context_connect.map_or(-1, |f| {
            f((*wctx).real_ctx, server, flags & !PA_CONTEXT_NOFAIL, api)
        });
        if rc < 0 {
            (*wctx).reconnect_attempts = 1;
            (*wctx).reconnecting = true;
            schedule_reconnect_timer(wctx, 150_000);
            rc = 0;
        }
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    rc
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_disconnect(c: *mut pa_context) {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return;
    }
    pa_context_ref(c);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    (*wctx).user_disconnected = true;
    (*wctx).reconnecting = false;
    (*wctx).reported_state = PA_CONTEXT_TERMINATED;
    if !(*wctx).reconnect_timer.is_null() && !(*wctx).api.is_null() {
        if let Some(tf) = (*(*wctx).api).time_free {
            tf((*wctx).reconnect_timer);
        }
        (*wctx).reconnect_timer = ptr::null_mut();
    }
    if !(*wctx).refresh_defer.is_null() && !(*wctx).api.is_null() {
        if let Some(df) = (*(*wctx).api).defer_free {
            df((*wctx).refresh_defer);
        }
        (*wctx).refresh_defer = ptr::null_mut();
    }
    let dead = detach_dead_handles_locked(wctx);
    let rc = (*wctx).real_ctx;
    let cb = (*wctx).state_cb;
    let ud = (*wctx).state_ud;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    flush_dead_handle_list(dead);
    if !rc.is_null() {
        if let Some(f) = real().pa_context_disconnect {
            f(rc);
        }
    } else if let Some(f) = cb {
        f(c, ud);
    }
    pa_context_unref(c);
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_is_local(c: *const pa_context) -> c_int {
    ensure_init();
    let wctx = c.cast_mut().cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return -1;
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let r = if !(*wctx).real_ctx.is_null() {
        real().pa_context_is_local.map_or(1, |f| f((*wctx).real_ctx))
    } else {
        1
    };
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    r
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_server(c: *const pa_context) -> *const c_char {
    ensure_init();
    let wctx = c.cast_mut().cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null();
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let r = if !(*wctx).real_ctx.is_null() {
        real()
            .pa_context_get_server
            .map_or(ptr::null(), |f| f((*wctx).real_ctx))
    } else {
        (*wctx).server
    };
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    r
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_protocol_version(c: *const pa_context) -> u32 {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx.is_null() {
        return PA_PROTOCOL_VERSION;
    }
    real()
        .pa_context_get_protocol_version
        .map_or(PA_PROTOCOL_VERSION, |f| f((*wctx).real_ctx))
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_server_protocol_version(c: *const pa_context) -> u32 {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx.is_null() {
        return PA_PROTOCOL_VERSION;
    }
    real()
        .pa_context_get_server_protocol_version
        .map_or(PA_PROTOCOL_VERSION, |f| f((*wctx).real_ctx))
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_index(c: *const pa_context) -> u32 {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return PA_INVALID_INDEX;
    }
    if (*wctx).real_ctx.is_null() {
        return 0;
    }
    real().pa_context_get_index.map_or(0, |f| f((*wctx).real_ctx))
}

const PA_USEC_INVALID: pa_usec_t = u64::MAX;
const PA_TIMEVAL_RTCLOCK: c_long = 1 << 30;

unsafe fn rttime_to_timeval(usec: pa_usec_t, tv: *mut TimeVal) -> *const TimeVal {
    if usec == PA_USEC_INVALID {
        return ptr::null();
    }
    // Match libpulse's pa_timeval_rtstore(..., rtclock=true) so mainloop_time_new/restart
    // treat `usec` on the monotonic pa_rtclock timeline rather than wallclock gettimeofday.
    (*tv).tv_sec = (usec / 1_000_000).min(c_long::MAX as u64) as c_long;
    (*tv).tv_usec = ((usec % 1_000_000) as c_long) | PA_TIMEVAL_RTCLOCK;
    tv
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_rttime_new(
    c: *const pa_context,
    usec: pa_usec_t,
    cb: pa_time_event_cb_t,
    userdata: *mut c_void,
) -> *mut pa_time_event {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return real()
            .pa_context_rttime_new
            .map_or(ptr::null_mut(), |f| f(c, usec, cb, userdata));
    }
    let rc = (*wctx).real_ctx;
    if !rc.is_null() {
        if let Some(f) = real().pa_context_rttime_new {
            return f(rc, usec, cb, userdata);
        }
    }
    let api = (*wctx).api;
    if api.is_null() {
        return ptr::null_mut();
    }
    let Some(time_new) = (*api).time_new else {
        return ptr::null_mut();
    };
    let mut tv = TimeVal::default();
    let tv_ptr = rttime_to_timeval(usec, &raw mut tv);
    time_new(api, tv_ptr, cb, userdata)
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_rttime_restart(
    c: *const pa_context,
    e: *mut pa_time_event,
    usec: pa_usec_t,
) {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        if let Some(f) = real().pa_context_rttime_restart {
            f(c, e, usec);
        }
        return;
    }
    let rc = (*wctx).real_ctx;
    if !rc.is_null() {
        if let Some(f) = real().pa_context_rttime_restart {
            f(rc, e, usec);
            return;
        }
    }
    if e.is_null() {
        return;
    }
    let api = (*wctx).api;
    if api.is_null() {
        return;
    }
    let Some(time_restart) = (*api).time_restart else {
        return;
    };
    let mut tv = TimeVal::default();
    let tv_ptr = rttime_to_timeval(usec, &raw mut tv);
    time_restart(e, tv_ptr);
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_tile_size(
    c: *const pa_context,
    ss: *const pa_sample_spec,
) -> usize {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx.is_null() {
        return 65536;
    }
    real()
        .pa_context_get_tile_size
        .map_or(65536, |f| f((*wctx).real_ctx, ss))
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_load_cookie_from_file(
    c: *mut pa_context,
    cookie_file_path: *const c_char,
) -> c_int {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) || (*wctx).real_ctx.is_null() {
        return -1;
    }
    real()
        .pa_context_load_cookie_from_file
        .map_or(-1, |f| f((*wctx).real_ctx, cookie_file_path))
}

// ============================================================================
// Tee'd Introspection
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
enum TeeKind {
    Server = 0,
    SinkList,
    SourceList,
}

#[repr(C)]
struct TeeReq {
    kind: TeeKind,
    wctx: *mut Wctx,
    owner_rc: *mut pa_context,
    user_cb: *mut c_void,
    user_data: *mut c_void,
    sinks: *mut pa_sink_info,
    n_sinks: usize,
    cap_sinks: usize,
    sources: *mut pa_source_info,
    n_sources: usize,
    cap_sources: usize,
    replayed: bool,
    global_next: *mut TeeReq,
}

static G_TEE_LOCK: SyncCell<PthreadMutex> = SyncCell::new([0; 8]);
static G_TEE_HEAD: SyncCell<*mut TeeReq> = SyncCell::new(ptr::null_mut());

unsafe fn pick_cache_locked(wctx: *mut Wctx) -> *mut PaCache {
    if !wctx.is_null()
        && ((*wctx).cache.has_server || (*wctx).cache.n_sinks > 0 || (*wctx).cache.n_sources > 0)
    {
        return &raw mut (*wctx).cache;
    }
    G_GLOBAL_CACHE.get()
}

unsafe fn tee_free(tr: *mut TeeReq) {
    if tr.is_null() {
        return;
    }
    for k in 0..(*tr).n_sinks {
        free_sink_entry((*tr).sinks.add(k));
    }
    free((*tr).sinks.cast());
    for k in 0..(*tr).n_sources {
        free_source_entry((*tr).sources.add(k));
    }
    free((*tr).sources.cast());
    free(tr.cast());
}

unsafe fn tee_register(tr: *mut TeeReq) {
    if tr.is_null() {
        return;
    }
    pthread_mutex_lock(G_TEE_LOCK.get());
    (*tr).global_next = *G_TEE_HEAD.get();
    *G_TEE_HEAD.get() = tr;
    pthread_mutex_unlock(G_TEE_LOCK.get());
}

unsafe fn tee_unregister(tr: *mut TeeReq) {
    if tr.is_null() {
        return;
    }
    pthread_mutex_lock(G_TEE_LOCK.get());
    let mut pp = G_TEE_HEAD.get();
    while !(*pp).is_null() {
        if *pp == tr {
            *pp = (*tr).global_next;
            (*tr).global_next = ptr::null_mut();
            break;
        }
        pp = &raw mut (**pp).global_next;
    }
    pthread_mutex_unlock(G_TEE_LOCK.get());
}

unsafe fn tee_begin_cb(
    tr: *mut TeeReq,
    terminal: bool,
    out_wctx: &mut *mut Wctx,
    out_replayed: &mut bool,
    out_cb: &mut *mut c_void,
    out_ud: &mut *mut c_void,
) -> bool {
    let mut found = false;
    pthread_mutex_lock(G_TEE_LOCK.get());
    let mut pp = G_TEE_HEAD.get();
    while !(*pp).is_null() {
        if *pp == tr {
            found = true;
            *out_wctx = (*tr).wctx;
            *out_replayed = (*tr).replayed;
            *out_cb = (*tr).user_cb;
            *out_ud = (*tr).user_data;
            if terminal {
                *pp = (*tr).global_next;
                (*tr).global_next = ptr::null_mut();
            }
            break;
        }
        pp = &raw mut (**pp).global_next;
    }
    pthread_mutex_unlock(G_TEE_LOCK.get());
    found
}

unsafe fn detach_tees_for_wctx(wctx: *mut Wctx) {
    if wctx.is_null() {
        return;
    }
    pthread_mutex_lock(G_TEE_LOCK.get());
    let mut tr = *G_TEE_HEAD.get();
    while !tr.is_null() {
        if (*tr).wctx == wctx {
            (*tr).wctx = ptr::null_mut();
            (*tr).user_cb = ptr::null_mut();
        }
        tr = (*tr).global_next;
    }
    pthread_mutex_unlock(G_TEE_LOCK.get());
}

unsafe fn reap_tees_for_rc(rc: *mut pa_context) {
    if rc.is_null() {
        return;
    }
    let mut to_free: *mut TeeReq = ptr::null_mut();
    pthread_mutex_lock(G_TEE_LOCK.get());
    let mut pp = G_TEE_HEAD.get();
    while !(*pp).is_null() {
        let tr = *pp;
        if (*tr).owner_rc == rc {
            *pp = (*tr).global_next;
            (*tr).global_next = to_free;
            to_free = tr;
        } else {
            pp = &raw mut (*tr).global_next;
        }
    }
    pthread_mutex_unlock(G_TEE_LOCK.get());
    while !to_free.is_null() {
        let nx = (*to_free).global_next;
        tee_free(to_free);
        to_free = nx;
    }
}

#[repr(C)]
struct ReplayItem {
    kind: TeeKind,
    user_cb: *mut c_void,
    user_data: *mut c_void,
    partial_sent: bool,
    next: *mut ReplayItem,
}

unsafe fn replay_inflight_tees(wctx: *mut Wctx, rc: *mut pa_context) {
    if wctx.is_null() || rc.is_null() {
        return;
    }
    let mut items: *mut ReplayItem = ptr::null_mut();
    let mut tail: *mut *mut ReplayItem = &raw mut items;

    pthread_mutex_lock(G_TEE_LOCK.get());
    let mut tr = *G_TEE_HEAD.get();
    while !tr.is_null() {
        if (*tr).wctx == wctx && (*tr).owner_rc == rc && !(*tr).replayed {
            (*tr).replayed = true;
            if !(*tr).user_cb.is_null() {
                let ri = calloc(1, core::mem::size_of::<ReplayItem>()).cast::<ReplayItem>();
                if !ri.is_null() {
                    (*ri).kind = (*tr).kind;
                    (*ri).user_cb = (*tr).user_cb;
                    (*ri).user_data = (*tr).user_data;
                    (*ri).partial_sent = ((*tr).kind == TeeKind::SinkList && (*tr).n_sinks > 0)
                        || ((*tr).kind == TeeKind::SourceList && (*tr).n_sources > 0);
                    *tail = ri;
                    tail = &raw mut (*ri).next;
                }
            }
        }
        tr = (*tr).global_next;
    }
    pthread_mutex_unlock(G_TEE_LOCK.get());

    let wctx_ref = is_valid_wctx(wctx);
    if wctx_ref {
        pa_context_ref(wctx.cast());
    }
    while !items.is_null() {
        let ri = items;
        items = (*ri).next;
        if !(*ri).user_cb.is_null() && is_valid_wctx(wctx) && !(*wctx).user_disconnected {
            let c = clone_cache_snapshot(wctx);
            if !c.is_null() {
                match (*ri).kind {
                    TeeKind::Server => {
                        let si = if (*c).has_server {
                            &raw const (*c).server
                        } else {
                            ptr::null()
                        };
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_server_info,
                            *mut c_void,
                        ) = core::mem::transmute((*ri).user_cb);
                        cb(wctx.cast(), si, (*ri).user_data);
                    }
                    TeeKind::SinkList => {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_sink_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*ri).user_cb);
                        if !(*ri).partial_sent {
                            for k in 0..(*c).n_sinks {
                                cb(wctx.cast(), (*c).sinks.add(k), 0, (*ri).user_data);
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*ri).user_data);
                    }
                    TeeKind::SourceList => {
                        let cb: unsafe extern "C" fn(
                            *mut pa_context,
                            *const pa_source_info,
                            c_int,
                            *mut c_void,
                        ) = core::mem::transmute((*ri).user_cb);
                        if !(*ri).partial_sent {
                            for k in 0..(*c).n_sources {
                                cb(wctx.cast(), (*c).sources.add(k), 0, (*ri).user_data);
                            }
                        }
                        cb(wctx.cast(), ptr::null(), 1, (*ri).user_data);
                    }
                }
                cache_free(c);
                free(c.cast());
            }
        }
        free(ri.cast());
    }
    if wctx_ref {
        pa_context_unref(wctx.cast());
    }
}

unsafe extern "C" fn tee_server_info_cb(
    _rc: *mut pa_context,
    i: *const pa_server_info,
    ud: *mut c_void,
) {
    let tr = ud.cast::<TeeReq>();
    let mut wctx: *mut Wctx = ptr::null_mut();
    let mut already_replayed = false;
    let mut ucb: *mut c_void = ptr::null_mut();
    let mut uud: *mut c_void = ptr::null_mut();
    if !tee_begin_cb(
        tr,
        true,
        &mut wctx,
        &mut already_replayed,
        &mut ucb,
        &mut uud,
    ) {
        return;
    }
    if already_replayed || !is_valid_wctx(wctx) {
        tee_free(tr);
        return;
    }
    pa_context_ref(wctx.cast());
    if !i.is_null() {
        pthread_mutex_lock(&raw mut (*wctx).lock);
        cache_set_server(&raw mut (*wctx).cache, i);
        pthread_mutex_unlock(&raw mut (*wctx).lock);

        pthread_mutex_lock(G_GLOBAL_CACHE_LOCK.get());
        cache_set_server(G_GLOBAL_CACHE.get(), i);
        pthread_mutex_unlock(G_GLOBAL_CACHE_LOCK.get());
        if !ucb.is_null() {
            let cb: unsafe extern "C" fn(*mut pa_context, *const pa_server_info, *mut c_void) =
                core::mem::transmute(ucb);
            cb(wctx.cast(), i, uud);
        }
    } else if !(*wctx).user_disconnected {
        let c = clone_cache_snapshot(wctx);
        let si = if !c.is_null() && (*c).has_server {
            &raw const (*c).server
        } else {
            ptr::null()
        };
        if !ucb.is_null() {
            let cb: unsafe extern "C" fn(*mut pa_context, *const pa_server_info, *mut c_void) =
                core::mem::transmute(ucb);
            cb(wctx.cast(), si, uud);
        }
        if !c.is_null() {
            cache_free(c);
            free(c.cast());
        }
    } else if !ucb.is_null() {
        let cb: unsafe extern "C" fn(*mut pa_context, *const pa_server_info, *mut c_void) =
            core::mem::transmute(ucb);
        cb(wctx.cast(), i, uud);
    }
    pa_context_unref(wctx.cast());
    tee_free(tr);
}

unsafe extern "C" fn tee_sink_list_cb(
    _rc: *mut pa_context,
    i: *const pa_sink_info,
    eol: c_int,
    ud: *mut c_void,
) {
    let tr = ud.cast::<TeeReq>();
    let mut wctx: *mut Wctx = ptr::null_mut();
    let mut already_replayed = false;
    let mut ucb: *mut c_void = ptr::null_mut();
    let mut uud: *mut c_void = ptr::null_mut();

    if eol == 0 && !i.is_null() {
        if !tee_begin_cb(
            tr,
            false,
            &mut wctx,
            &mut already_replayed,
            &mut ucb,
            &mut uud,
        ) {
            return;
        }
        if already_replayed || !is_valid_wctx(wctx) {
            return;
        }
        if (*tr).n_sinks == (*tr).cap_sinks {
            let nc = if (*tr).cap_sinks != 0 {
                (*tr).cap_sinks * 2
            } else {
                8
            };
            let nb = realloc(
                (*tr).sinks.cast(),
                nc * core::mem::size_of::<pa_sink_info>(),
            )
            .cast::<pa_sink_info>();
            if !nb.is_null() {
                (*tr).sinks = nb;
                (*tr).cap_sinks = nc;
            }
        }
        if (*tr).n_sinks < (*tr).cap_sinks {
            let idx = (*tr).n_sinks;
            (*tr).n_sinks += 1;
            copy_sink_entry((*tr).sinks.add(idx), i);
        }
        if !ucb.is_null() {
            pa_context_ref(wctx.cast());
            let cb: unsafe extern "C" fn(
                *mut pa_context,
                *const pa_sink_info,
                c_int,
                *mut c_void,
            ) = core::mem::transmute(ucb);
            cb(wctx.cast(), i, eol, uud);
            pa_context_unref(wctx.cast());
        }
        return;
    }

    if !tee_begin_cb(
        tr,
        true,
        &mut wctx,
        &mut already_replayed,
        &mut ucb,
        &mut uud,
    ) {
        return;
    }
    if already_replayed || !is_valid_wctx(wctx) {
        tee_free(tr);
        return;
    }

    pa_context_ref(wctx.cast());
    if eol < 0 && (*tr).n_sinks == 0 && !(*wctx).user_disconnected {
        let c = clone_cache_snapshot(wctx);
        let has_cached = !c.is_null() && (*c).n_sinks > 0;
        if has_cached {
            if !ucb.is_null() {
                let cb: unsafe extern "C" fn(
                    *mut pa_context,
                    *const pa_sink_info,
                    c_int,
                    *mut c_void,
                ) = core::mem::transmute(ucb);
                for k in 0..(*c).n_sinks {
                    cb(wctx.cast(), (*c).sinks.add(k), 0, uud);
                }
                cb(wctx.cast(), ptr::null(), 1, uud);
            }
        } else if !ucb.is_null() {
            let out_eol = if (*wctx).reconnecting { 1 } else { eol };
            let cb: unsafe extern "C" fn(
                *mut pa_context,
                *const pa_sink_info,
                c_int,
                *mut c_void,
            ) = core::mem::transmute(ucb);
            cb(wctx.cast(), i, out_eol, uud);
        }
        if !c.is_null() {
            cache_free(c);
            free(c.cast());
        }
    } else if !ucb.is_null() {
        let out_eol = if eol < 0
            && ((*tr).n_sinks > 0 || (*wctx).reconnecting)
            && !(*wctx).user_disconnected
        {
            1
        } else {
            eol
        };
        let cb: unsafe extern "C" fn(*mut pa_context, *const pa_sink_info, c_int, *mut c_void) =
            core::mem::transmute(ucb);
        cb(wctx.cast(), i, out_eol, uud);
    }

    if eol > 0 && (*tr).n_sinks > 0 && is_valid_wctx(wctx) {
        pthread_mutex_lock(&raw mut (*wctx).lock);
        cache_replace_sinks(&raw mut (*wctx).cache, (*tr).sinks, (*tr).n_sinks);
        (*tr).sinks = ptr::null_mut();
        (*tr).n_sinks = 0;
        let n_sinks = (*wctx).cache.n_sinks;
        let gcopy = calloc(n_sinks, core::mem::size_of::<pa_sink_info>()).cast::<pa_sink_info>();
        if !gcopy.is_null() {
            for k in 0..n_sinks {
                copy_sink_entry(gcopy.add(k), (*wctx).cache.sinks.add(k));
            }
            pthread_mutex_lock(G_GLOBAL_CACHE_LOCK.get());
            cache_replace_sinks(G_GLOBAL_CACHE.get(), gcopy, n_sinks);
            pthread_mutex_unlock(G_GLOBAL_CACHE_LOCK.get());
        }
        pthread_mutex_unlock(&raw mut (*wctx).lock);
    }
    pa_context_unref(wctx.cast());
    tee_free(tr);
}

unsafe extern "C" fn tee_source_list_cb(
    _rc: *mut pa_context,
    i: *const pa_source_info,
    eol: c_int,
    ud: *mut c_void,
) {
    let tr = ud.cast::<TeeReq>();
    let mut wctx: *mut Wctx = ptr::null_mut();
    let mut already_replayed = false;
    let mut ucb: *mut c_void = ptr::null_mut();
    let mut uud: *mut c_void = ptr::null_mut();

    if eol == 0 && !i.is_null() {
        if !tee_begin_cb(
            tr,
            false,
            &mut wctx,
            &mut already_replayed,
            &mut ucb,
            &mut uud,
        ) {
            return;
        }
        if already_replayed || !is_valid_wctx(wctx) {
            return;
        }
        if (*tr).n_sources == (*tr).cap_sources {
            let nc = if (*tr).cap_sources != 0 {
                (*tr).cap_sources * 2
            } else {
                8
            };
            let nb = realloc(
                (*tr).sources.cast(),
                nc * core::mem::size_of::<pa_source_info>(),
            )
            .cast::<pa_source_info>();
            if !nb.is_null() {
                (*tr).sources = nb;
                (*tr).cap_sources = nc;
            }
        }
        if (*tr).n_sources < (*tr).cap_sources {
            let idx = (*tr).n_sources;
            (*tr).n_sources += 1;
            copy_source_entry((*tr).sources.add(idx), i);
        }
        if !ucb.is_null() {
            pa_context_ref(wctx.cast());
            let cb: unsafe extern "C" fn(
                *mut pa_context,
                *const pa_source_info,
                c_int,
                *mut c_void,
            ) = core::mem::transmute(ucb);
            cb(wctx.cast(), i, eol, uud);
            pa_context_unref(wctx.cast());
        }
        return;
    }

    if !tee_begin_cb(
        tr,
        true,
        &mut wctx,
        &mut already_replayed,
        &mut ucb,
        &mut uud,
    ) {
        return;
    }
    if already_replayed || !is_valid_wctx(wctx) {
        tee_free(tr);
        return;
    }

    pa_context_ref(wctx.cast());
    if eol < 0 && (*tr).n_sources == 0 && !(*wctx).user_disconnected {
        let c = clone_cache_snapshot(wctx);
        let has_cached = !c.is_null() && (*c).n_sources > 0;
        if has_cached {
            if !ucb.is_null() {
                let cb: unsafe extern "C" fn(
                    *mut pa_context,
                    *const pa_source_info,
                    c_int,
                    *mut c_void,
                ) = core::mem::transmute(ucb);
                for k in 0..(*c).n_sources {
                    cb(wctx.cast(), (*c).sources.add(k), 0, uud);
                }
                cb(wctx.cast(), ptr::null(), 1, uud);
            }
        } else if !ucb.is_null() {
            let out_eol = if (*wctx).reconnecting { 1 } else { eol };
            let cb: unsafe extern "C" fn(
                *mut pa_context,
                *const pa_source_info,
                c_int,
                *mut c_void,
            ) = core::mem::transmute(ucb);
            cb(wctx.cast(), i, out_eol, uud);
        }
        if !c.is_null() {
            cache_free(c);
            free(c.cast());
        }
    } else if !ucb.is_null() {
        let out_eol = if eol < 0
            && ((*tr).n_sources > 0 || (*wctx).reconnecting)
            && !(*wctx).user_disconnected
        {
            1
        } else {
            eol
        };
        let cb: unsafe extern "C" fn(*mut pa_context, *const pa_source_info, c_int, *mut c_void) =
            core::mem::transmute(ucb);
        cb(wctx.cast(), i, out_eol, uud);
    }

    if eol > 0 && (*tr).n_sources > 0 && is_valid_wctx(wctx) {
        pthread_mutex_lock(&raw mut (*wctx).lock);
        cache_replace_sources(&raw mut (*wctx).cache, (*tr).sources, (*tr).n_sources);
        (*tr).sources = ptr::null_mut();
        (*tr).n_sources = 0;
        let n_sources = (*wctx).cache.n_sources;
        let gcopy =
            calloc(n_sources, core::mem::size_of::<pa_source_info>()).cast::<pa_source_info>();
        if !gcopy.is_null() {
            for k in 0..n_sources {
                copy_source_entry(gcopy.add(k), (*wctx).cache.sources.add(k));
            }
            pthread_mutex_lock(G_GLOBAL_CACHE_LOCK.get());
            cache_replace_sources(G_GLOBAL_CACHE.get(), gcopy, n_sources);
            pthread_mutex_unlock(G_GLOBAL_CACHE_LOCK.get());
        }
        pthread_mutex_unlock(&raw mut (*wctx).lock);
    }
    pa_context_unref(wctx.cast());
    tee_free(tr);
}

#[repr(C)]
struct CtxCbWrap {
    wctx: *mut Wctx,
    cb: *mut c_void,
    ud: *mut c_void,
}

unsafe fn mk_ctx_wrap(wctx: *mut Wctx, cb: *mut c_void, ud: *mut c_void) -> *mut CtxCbWrap {
    let w = calloc(1, core::mem::size_of::<CtxCbWrap>()).cast::<CtxCbWrap>();
    if !w.is_null() {
        (*w).wctx = wctx;
        (*w).cb = cb;
        (*w).ud = ud;
    }
    w
}

unsafe extern "C" fn wrap_ctx_success_cb(_rc: *mut pa_context, success: c_int, ud: *mut c_void) {
    let w = ud.cast::<CtxCbWrap>();
    if is_valid_wctx((*w).wctx) {
        pa_context_ref((*w).wctx.cast());
        let out = if success == 0 && (*(*w).wctx).reconnecting && !(*(*w).wctx).user_disconnected {
            1
        } else {
            success
        };
        if !(*w).cb.is_null() {
            let cb: unsafe extern "C" fn(*mut pa_context, c_int, *mut c_void) =
                core::mem::transmute((*w).cb);
            cb((*w).wctx.cast(), out, (*w).ud);
        }
        pa_context_unref((*w).wctx.cast());
    }
    free(w.cast());
}

unsafe extern "C" fn wrap_ctx_notify_cb(_rc: *mut pa_context, ud: *mut c_void) {
    let w = ud.cast::<CtxCbWrap>();
    if is_valid_wctx((*w).wctx) {
        pa_context_ref((*w).wctx.cast());
        if !(*w).cb.is_null() {
            let cb: unsafe extern "C" fn(*mut pa_context, *mut c_void) =
                core::mem::transmute((*w).cb);
            cb((*w).wctx.cast(), (*w).ud);
        }
        pa_context_unref((*w).wctx.cast());
    }
    free(w.cast());
}

unsafe extern "C" fn wrap_ctx_index_cb(_rc: *mut pa_context, idx: u32, ud: *mut c_void) {
    let w = ud.cast::<CtxCbWrap>();
    if is_valid_wctx((*w).wctx) {
        pa_context_ref((*w).wctx.cast());
        if !(*w).cb.is_null() {
            let cb: unsafe extern "C" fn(*mut pa_context, u32, *mut c_void) =
                core::mem::transmute((*w).cb);
            cb((*w).wctx.cast(), idx, (*w).ud);
        }
        pa_context_unref((*w).wctx.cast());
    }
    free(w.cast());
}

unsafe extern "C" fn wrap_ctx_string_cb(
    _rc: *mut pa_context,
    success: c_int,
    response: *mut c_char,
    ud: *mut c_void,
) {
    let w = ud.cast::<CtxCbWrap>();
    if is_valid_wctx((*w).wctx) {
        pa_context_ref((*w).wctx.cast());
        if !(*w).cb.is_null() {
            let cb: unsafe extern "C" fn(*mut pa_context, c_int, *mut c_char, *mut c_void) =
                core::mem::transmute((*w).cb);
            cb((*w).wctx.cast(), success, response, (*w).ud);
        }
        pa_context_unref((*w).wctx.cast());
    }
    free(w.cast());
}

unsafe extern "C" fn wrap_ctx_stat_cb(
    _rc: *mut pa_context,
    i: *const pa_stat_info,
    ud: *mut c_void,
) {
    let w = ud.cast::<CtxCbWrap>();
    if is_valid_wctx((*w).wctx) {
        pa_context_ref((*w).wctx.cast());
        if !(*w).cb.is_null() {
            let cb: unsafe extern "C" fn(*mut pa_context, *const pa_stat_info, *mut c_void) =
                core::mem::transmute((*w).cb);
            cb((*w).wctx.cast(), i, (*w).ud);
        }
        pa_context_unref((*w).wctx.cast());
    }
    free(w.cast());
}

macro_rules! define_wrap_ctx_eol_cb {
    ($fn_name:ident, $info_ty:ty) => {
        unsafe extern "C" fn $fn_name(
            _rc: *mut pa_context,
            i: *const $info_ty,
            eol: c_int,
            ud: *mut c_void,
        ) {
            let w = ud.cast::<CtxCbWrap>();
            if is_valid_wctx((*w).wctx) {
                pa_context_ref((*w).wctx.cast());
                let out_eol =
                    if eol < 0 && (*(*w).wctx).reconnecting && !(*(*w).wctx).user_disconnected {
                        1
                    } else {
                        eol
                    };
                if !(*w).cb.is_null() {
                    let cb: unsafe extern "C" fn(
                        *mut pa_context,
                        *const $info_ty,
                        c_int,
                        *mut c_void,
                    ) = core::mem::transmute((*w).cb);
                    cb((*w).wctx.cast(), i, out_eol, (*w).ud);
                }
                pa_context_unref((*w).wctx.cast());
            }
            if eol != 0 {
                free(w.cast());
            }
        }
    };
}

define_wrap_ctx_eol_cb!(wrap_ctx_client_cb, pa_client_info);
define_wrap_ctx_eol_cb!(wrap_ctx_sink_cb, pa_sink_info);
define_wrap_ctx_eol_cb!(wrap_ctx_source_cb, pa_source_info);
define_wrap_ctx_eol_cb!(wrap_ctx_sink_input_cb, pa_sink_input_info);
define_wrap_ctx_eol_cb!(wrap_ctx_source_output_cb, pa_source_output_info);
define_wrap_ctx_eol_cb!(wrap_ctx_module_cb, pa_module_info);
define_wrap_ctx_eol_cb!(wrap_ctx_card_cb, pa_card_info);
define_wrap_ctx_eol_cb!(wrap_ctx_sample_cb, pa_sample_info);
define_wrap_ctx_eol_cb!(wrap_ctx_autoload_cb, pa_autoload_info);
define_wrap_ctx_eol_cb!(wrap_ctx_ext_dev_restore_cb, pa_ext_device_restore_info);
define_wrap_ctx_eol_cb!(wrap_ctx_ext_dev_mgr_cb, pa_ext_device_manager_info);
define_wrap_ctx_eol_cb!(wrap_ctx_ext_stream_restore_cb, pa_ext_stream_restore_info);

#[inline]
unsafe fn ctx_is_live_ready(wctx: *mut Wctx) -> bool {
    !(*wctx).real_ctx.is_null()
        && real()
            .pa_context_get_state
            .is_some_and(|f| f((*wctx).real_ctx) == PA_CONTEXT_READY)
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_subscribe(
    c: *mut pa_context,
    m: pa_subscription_mask_t,
    cb: pa_context_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    (*wctx).has_subscribe = m != PA_SUBSCRIPTION_MASK_NULL;
    (*wctx).sub_mask = m;
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::Success, cb_ptr, userdata, ptr::null(), 0)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real().pa_context_subscribe.map_or(ptr::null_mut(), |f| {
        f(
            (*wctx).real_ctx,
            m,
            if !w.is_null() {
                Some(wrap_ctx_success_cb)
            } else {
                None
            },
            w.cast(),
        )
    });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_server_info(
    c: *mut pa_context,
    cb: pa_server_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::ServerInfo, cb_ptr, userdata, ptr::null(), 0)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let tr = calloc(1, core::mem::size_of::<TeeReq>()).cast::<TeeReq>();
    if tr.is_null() {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    (*tr).kind = TeeKind::Server;
    (*tr).wctx = wctx;
    (*tr).owner_rc = (*wctx).real_ctx;
    (*tr).user_cb = cb_ptr;
    (*tr).user_data = userdata;
    tee_register(tr);
    let op = real()
        .pa_context_get_server_info
        .map_or(ptr::null_mut(), |f| {
            f((*wctx).real_ctx, Some(tee_server_info_cb), tr.cast())
        });
    if op.is_null() {
        tee_unregister(tr);
        free(tr.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_sink_info_list(
    c: *mut pa_context,
    cb: pa_sink_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::SinkList, cb_ptr, userdata, ptr::null(), 0)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let tr = calloc(1, core::mem::size_of::<TeeReq>()).cast::<TeeReq>();
    if tr.is_null() {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    (*tr).kind = TeeKind::SinkList;
    (*tr).wctx = wctx;
    (*tr).owner_rc = (*wctx).real_ctx;
    (*tr).user_cb = cb_ptr;
    (*tr).user_data = userdata;
    tee_register(tr);
    let op = real()
        .pa_context_get_sink_info_list
        .map_or(ptr::null_mut(), |f| {
            f((*wctx).real_ctx, Some(tee_sink_list_cb), tr.cast())
        });
    if op.is_null() {
        tee_unregister(tr);
        free(tr.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_sink_info_by_name(
    c: *mut pa_context,
    name: *const c_char,
    cb: pa_sink_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::SinkByName, cb_ptr, userdata, name, 0)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real()
        .pa_context_get_sink_info_by_name
        .map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                name,
                if !w.is_null() {
                    Some(wrap_ctx_sink_cb)
                } else {
                    None
                },
                w.cast(),
            )
        });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_sink_info_by_index(
    c: *mut pa_context,
    idx: u32,
    cb: pa_sink_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::SinkByIndex, cb_ptr, userdata, ptr::null(), idx)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real()
        .pa_context_get_sink_info_by_index
        .map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                idx,
                if !w.is_null() {
                    Some(wrap_ctx_sink_cb)
                } else {
                    None
                },
                w.cast(),
            )
        });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_source_info_list(
    c: *mut pa_context,
    cb: pa_source_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::SourceList, cb_ptr, userdata, ptr::null(), 0)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let tr = calloc(1, core::mem::size_of::<TeeReq>()).cast::<TeeReq>();
    if tr.is_null() {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    (*tr).kind = TeeKind::SourceList;
    (*tr).wctx = wctx;
    (*tr).owner_rc = (*wctx).real_ctx;
    (*tr).user_cb = cb_ptr;
    (*tr).user_data = userdata;
    tee_register(tr);
    let op = real()
        .pa_context_get_source_info_list
        .map_or(ptr::null_mut(), |f| {
            f((*wctx).real_ctx, Some(tee_source_list_cb), tr.cast())
        });
    if op.is_null() {
        tee_unregister(tr);
        free(tr.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_source_info_by_name(
    c: *mut pa_context,
    name: *const c_char,
    cb: pa_source_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::SourceByName, cb_ptr, userdata, name, 0)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real()
        .pa_context_get_source_info_by_name
        .map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                name,
                if !w.is_null() {
                    Some(wrap_ctx_source_cb)
                } else {
                    None
                },
                w.cast(),
            )
        });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_get_source_info_by_index(
    c: *mut pa_context,
    idx: u32,
    cb: pa_source_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::SourceByIndex, cb_ptr, userdata, ptr::null(), idx)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real()
        .pa_context_get_source_info_by_index
        .map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                idx,
                if !w.is_null() {
                    Some(wrap_ctx_source_cb)
                } else {
                    None
                },
                w.cast(),
            )
        });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_drain(
    c: *mut pa_context,
    cb: pa_context_notify_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if (*wctx).real_ctx.is_null() {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real().pa_context_drain.map_or(ptr::null_mut(), |f| {
        f(
            (*wctx).real_ctx,
            if !w.is_null() {
                Some(wrap_ctx_notify_cb)
            } else {
                None
            },
            w.cast(),
        )
    });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

macro_rules! ctx_op_success {
    ($fname:ident, ($($pname:ident : $pty:ty),*), ($($fwd:expr),*)) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fname(
            c: *mut pa_context,
            $($pname : $pty,)*
            cb: pa_context_success_cb_t,
            userdata: *mut c_void,
        ) -> *mut pa_operation {
            ensure_init();
            let wctx = c.cast::<Wctx>();
            if !is_valid_wctx(wctx) {
                return ptr::null_mut();
            }
            let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
            pthread_mutex_lock(&raw mut (*wctx).lock);
            if !ctx_is_live_ready(wctx) {
                let op = if (*wctx).reconnecting {
                    spawn_synth_op(wctx, SynthKind::Success, cb_ptr, userdata, ptr::null(), 0)
                } else {
                    ptr::null_mut()
                };
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return op;
            }
            let Some(rf) = real().$fname else {
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return ptr::null_mut();
            };
            let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
            let w_cb: pa_context_success_cb_t = if !w.is_null() { Some(wrap_ctx_success_cb) } else { None };
            let op = rf((*wctx).real_ctx, $($fwd,)* w_cb, w.cast());
            if op.is_null() {
                free(w.cast());
            }
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            op
        }
    };
}

macro_rules! ctx_op_list {
    ($fname:ident, ($($pname:ident : $pty:ty),*), $cb_ty:ty, ($($fwd:expr),*), $wrap_cb:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fname(
            c: *mut pa_context,
            $($pname : $pty,)*
            cb: $cb_ty,
            userdata: *mut c_void,
        ) -> *mut pa_operation {
            ensure_init();
            let wctx = c.cast::<Wctx>();
            if !is_valid_wctx(wctx) {
                return ptr::null_mut();
            }
            let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
            pthread_mutex_lock(&raw mut (*wctx).lock);
            if !ctx_is_live_ready(wctx) {
                let op = if (*wctx).reconnecting {
                    spawn_synth_op(wctx, SynthKind::EmptyEol, cb_ptr, userdata, ptr::null(), 0)
                } else {
                    ptr::null_mut()
                };
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return op;
            }
            let Some(rf) = real().$fname else {
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return ptr::null_mut();
            };
            let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
            let w_cb: $cb_ty = if !w.is_null() { Some($wrap_cb) } else { None };
            let op = rf((*wctx).real_ctx, $($fwd,)* w_cb, w.cast());
            if op.is_null() {
                free(w.cast());
            }
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            op
        }
    };
}

macro_rules! ctx_op_test {
    ($fname:ident, $cb_ty:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fname(
            c: *mut pa_context,
            cb: $cb_ty,
            userdata: *mut c_void,
        ) -> *mut pa_operation {
            ensure_init();
            let wctx = c.cast::<Wctx>();
            if !is_valid_wctx(wctx) {
                return ptr::null_mut();
            }
            pthread_mutex_lock(&raw mut (*wctx).lock);
            if !ctx_is_live_ready(wctx) {
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return ptr::null_mut();
            }
            let Some(rf) = real().$fname else {
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return ptr::null_mut();
            };
            let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
            let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
            let w_cb: $cb_ty = if !w.is_null() {
                Some(wrap_ctx_index_cb)
            } else {
                None
            };
            let op = rf((*wctx).real_ctx, w_cb, w.cast());
            if op.is_null() {
                free(w.cast());
            }
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            op
        }
    };
}

macro_rules! ctx_ext_subscribe {
    ($fname:ident, $field_enabled:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fname(
            c: *mut pa_context,
            enable: c_int,
            cb: pa_context_success_cb_t,
            userdata: *mut c_void,
        ) -> *mut pa_operation {
            ensure_init();
            let wctx = c.cast::<Wctx>();
            if !is_valid_wctx(wctx) {
                return ptr::null_mut();
            }
            let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
            pthread_mutex_lock(&raw mut (*wctx).lock);
            (*wctx).$field_enabled = enable != 0;
            if !ctx_is_live_ready(wctx) {
                let op = if (*wctx).reconnecting {
                    spawn_synth_op(wctx, SynthKind::Success, cb_ptr, userdata, ptr::null(), 0)
                } else {
                    ptr::null_mut()
                };
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return op;
            }
            let Some(rf) = real().$fname else {
                pthread_mutex_unlock(&raw mut (*wctx).lock);
                return ptr::null_mut();
            };
            let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
            let w_cb: pa_context_success_cb_t = if !w.is_null() {
                Some(wrap_ctx_success_cb)
            } else {
                None
            };
            let op = rf((*wctx).real_ctx, enable, w_cb, w.cast());
            if op.is_null() {
                free(w.cast());
            }
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            op
        }
    };
}

macro_rules! ctx_ext_set_sub_cb {
    ($fname:ident, $cb_ty:ty, $field_cb:ident, $field_ud:ident, $internal_cb:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fname(c: *mut pa_context, cb: $cb_ty, userdata: *mut c_void) {
            ensure_init();
            let wctx = c.cast::<Wctx>();
            if !is_valid_wctx(wctx) {
                return;
            }
            pthread_mutex_lock(&raw mut (*wctx).lock);
            (*wctx).$field_cb = cb;
            (*wctx).$field_ud = userdata;
            if !(*wctx).real_ctx.is_null() {
                if let Some(rf) = real().$fname {
                    rf(
                        (*wctx).real_ctx,
                        if cb.is_some() {
                            Some($internal_cb)
                        } else {
                            None
                        },
                        if cb.is_some() {
                            wctx.cast()
                        } else {
                            ptr::null_mut()
                        },
                    );
                }
            }
            pthread_mutex_unlock(&raw mut (*wctx).lock);
        }
    };
}

ctx_op_success!(pa_context_exit_daemon, (), ());
ctx_op_success!(pa_context_set_default_sink, (name: *const c_char), (name));
ctx_op_success!(pa_context_set_default_source, (name: *const c_char), (name));
ctx_op_success!(pa_context_set_name, (name: *const c_char), (name));

#[no_mangle]
pub unsafe extern "C" fn pa_context_proplist_update(
    c: *mut pa_context,
    mode: pa_update_mode_t,
    p: *const pa_proplist,
    cb: pa_context_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        if c.is_null() {
            return ptr::null_mut();
        }
        return real()
            .pa_context_proplist_update
            .map_or(ptr::null_mut(), |f| f(c, mode, p, cb, userdata));
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        let op = if (*wctx).reconnecting {
            spawn_synth_op(wctx, SynthKind::Success, cb_ptr, userdata, ptr::null(), 0)
        } else {
            ptr::null_mut()
        };
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return op;
    }
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real()
        .pa_context_proplist_update
        .map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                mode,
                p,
                if !w.is_null() {
                    Some(wrap_ctx_success_cb)
                } else {
                    None
                },
                w.cast(),
            )
        });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

ctx_op_success!(pa_context_proplist_remove, (keys: *const *const c_char), (keys));
ctx_op_success!(pa_context_set_sink_volume_by_index, (idx: u32, volume: *const pa_cvolume), (idx, volume));
ctx_op_success!(pa_context_set_sink_volume_by_name, (name: *const c_char, volume: *const pa_cvolume), (name, volume));
ctx_op_success!(pa_context_set_sink_mute_by_index, (idx: u32, mute: c_int), (idx, mute));
ctx_op_success!(pa_context_set_sink_mute_by_name, (name: *const c_char, mute: c_int), (name, mute));
ctx_op_success!(pa_context_suspend_sink_by_name, (sink_name: *const c_char, suspend: c_int), (sink_name, suspend));
ctx_op_success!(pa_context_suspend_sink_by_index, (idx: u32, suspend: c_int), (idx, suspend));
ctx_op_success!(pa_context_set_sink_port_by_index, (idx: u32, port: *const c_char), (idx, port));
ctx_op_success!(pa_context_set_sink_port_by_name, (name: *const c_char, port: *const c_char), (name, port));
ctx_op_success!(pa_context_set_source_volume_by_index, (idx: u32, volume: *const pa_cvolume), (idx, volume));
ctx_op_success!(pa_context_set_source_volume_by_name, (name: *const c_char, volume: *const pa_cvolume), (name, volume));
ctx_op_success!(pa_context_set_source_mute_by_index, (idx: u32, mute: c_int), (idx, mute));
ctx_op_success!(pa_context_set_source_mute_by_name, (name: *const c_char, mute: c_int), (name, mute));
ctx_op_success!(pa_context_suspend_source_by_name, (source_name: *const c_char, suspend: c_int), (source_name, suspend));
ctx_op_success!(pa_context_suspend_source_by_index, (idx: u32, suspend: c_int), (idx, suspend));
ctx_op_success!(pa_context_set_source_port_by_index, (idx: u32, port: *const c_char), (idx, port));
ctx_op_success!(pa_context_set_source_port_by_name, (name: *const c_char, port: *const c_char), (name, port));
ctx_op_success!(pa_context_unload_module, (idx: u32), (idx));
ctx_op_success!(pa_context_kill_client, (idx: u32), (idx));
ctx_op_success!(pa_context_set_card_profile_by_index, (idx: u32, profile: *const c_char), (idx, profile));
ctx_op_success!(pa_context_set_card_profile_by_name, (name: *const c_char, profile: *const c_char), (name, profile));
ctx_op_success!(pa_context_set_port_latency_offset, (card_name: *const c_char, port_name: *const c_char, offset: i64), (card_name, port_name, offset));
ctx_op_success!(pa_context_move_sink_input_by_name, (idx: u32, sink_name: *const c_char), (idx, sink_name));
ctx_op_success!(pa_context_move_sink_input_by_index, (idx: u32, sink_idx: u32), (idx, sink_idx));
ctx_op_success!(pa_context_set_sink_input_volume, (idx: u32, volume: *const pa_cvolume), (idx, volume));
ctx_op_success!(pa_context_set_sink_input_mute, (idx: u32, mute: c_int), (idx, mute));
ctx_op_success!(pa_context_kill_sink_input, (idx: u32), (idx));
ctx_op_success!(pa_context_move_source_output_by_name, (idx: u32, source_name: *const c_char), (idx, source_name));
ctx_op_success!(pa_context_move_source_output_by_index, (idx: u32, source_idx: u32), (idx, source_idx));
ctx_op_success!(pa_context_set_source_output_volume, (idx: u32, volume: *const pa_cvolume), (idx, volume));
ctx_op_success!(pa_context_set_source_output_mute, (idx: u32, mute: c_int), (idx, mute));
ctx_op_success!(pa_context_kill_source_output, (idx: u32), (idx));
ctx_op_success!(pa_context_remove_sample, (name: *const c_char), (name));
ctx_op_success!(pa_context_play_sample, (name: *const c_char, dev: *const c_char, volume: pa_volume_t), (name, dev, volume));
ctx_op_success!(pa_context_remove_autoload_by_name, (name: *const c_char, atype: pa_autoload_type_t), (name, atype));
ctx_op_success!(pa_context_remove_autoload_by_index, (idx: u32), (idx));

ctx_op_list!(pa_context_get_module_info, (idx: u32), pa_module_info_cb_t, (idx), wrap_ctx_module_cb);
ctx_op_list!(pa_context_get_module_info_list, (), pa_module_info_cb_t, (), wrap_ctx_module_cb);
ctx_op_list!(pa_context_get_client_info, (idx: u32), pa_client_info_cb_t, (idx), wrap_ctx_client_cb);
ctx_op_list!(pa_context_get_client_info_list, (), pa_client_info_cb_t, (), wrap_ctx_client_cb);
ctx_op_list!(pa_context_get_card_info_by_index, (idx: u32), pa_card_info_cb_t, (idx), wrap_ctx_card_cb);
ctx_op_list!(pa_context_get_card_info_by_name, (name: *const c_char), pa_card_info_cb_t, (name), wrap_ctx_card_cb);
ctx_op_list!(pa_context_get_card_info_list, (), pa_card_info_cb_t, (), wrap_ctx_card_cb);
ctx_op_list!(pa_context_get_sink_input_info, (idx: u32), pa_sink_input_info_cb_t, (idx), wrap_ctx_sink_input_cb);
ctx_op_list!(pa_context_get_sink_input_info_list, (), pa_sink_input_info_cb_t, (), wrap_ctx_sink_input_cb);
ctx_op_list!(pa_context_get_source_output_info, (idx: u32), pa_source_output_info_cb_t, (idx), wrap_ctx_source_output_cb);
ctx_op_list!(pa_context_get_source_output_info_list, (), pa_source_output_info_cb_t, (), wrap_ctx_source_output_cb);
ctx_op_list!(pa_context_get_sample_info_by_name, (name: *const c_char), pa_sample_info_cb_t, (name), wrap_ctx_sample_cb);
ctx_op_list!(pa_context_get_sample_info_by_index, (idx: u32), pa_sample_info_cb_t, (idx), wrap_ctx_sample_cb);
ctx_op_list!(pa_context_get_sample_info_list, (), pa_sample_info_cb_t, (), wrap_ctx_sample_cb);
ctx_op_list!(pa_context_get_autoload_info_by_name, (name: *const c_char, atype: pa_autoload_type_t), pa_autoload_info_cb_t, (name, atype), wrap_ctx_autoload_cb);
ctx_op_list!(pa_context_get_autoload_info_by_index, (idx: u32), pa_autoload_info_cb_t, (idx), wrap_ctx_autoload_cb);
ctx_op_list!(pa_context_get_autoload_info_list, (), pa_autoload_info_cb_t, (), wrap_ctx_autoload_cb);

ctx_op_test!(pa_ext_device_restore_test, pa_ext_device_restore_test_cb_t);
ctx_ext_subscribe!(pa_ext_device_restore_subscribe, ext_dev_restore_sub_enabled);
ctx_ext_set_sub_cb!(
    pa_ext_device_restore_set_subscribe_cb,
    pa_ext_device_restore_subscribe_cb_t,
    ext_dev_restore_sub_cb,
    ext_dev_restore_sub_ud,
    internal_ext_dev_restore_sub_cb
);
ctx_op_list!(
    pa_ext_device_restore_read_formats_all,
    (),
    pa_ext_device_restore_read_device_formats_cb_t,
    (),
    wrap_ctx_ext_dev_restore_cb
);
ctx_op_list!(
    pa_ext_device_restore_read_formats,
    (dtype: pa_device_type_t, idx: u32),
    pa_ext_device_restore_read_device_formats_cb_t,
    (dtype, idx),
    wrap_ctx_ext_dev_restore_cb
);
ctx_op_success!(
    pa_ext_device_restore_save_formats,
    (dtype: pa_device_type_t, idx: u32, n_formats: u8, formats: *mut *mut pa_format_info),
    (dtype, idx, n_formats, formats)
);

ctx_op_test!(pa_ext_device_manager_test, pa_ext_device_manager_test_cb_t);
ctx_op_list!(
    pa_ext_device_manager_read,
    (),
    pa_ext_device_manager_read_cb_t,
    (),
    wrap_ctx_ext_dev_mgr_cb
);
ctx_op_success!(
    pa_ext_device_manager_set_device_description,
    (device: *const c_char, description: *const c_char),
    (device, description)
);
ctx_op_success!(
    pa_ext_device_manager_delete,
    (s: *const *const c_char),
    (s)
);
ctx_op_success!(
    pa_ext_device_manager_enable_role_device_priority_routing,
    (enable: c_int),
    (enable)
);
ctx_op_success!(
    pa_ext_device_manager_reorder_devices_for_role,
    (role: *const c_char, devices: *const *const c_char),
    (role, devices)
);
ctx_ext_subscribe!(pa_ext_device_manager_subscribe, ext_dev_mgr_sub_enabled);
ctx_ext_set_sub_cb!(
    pa_ext_device_manager_set_subscribe_cb,
    pa_ext_device_manager_subscribe_cb_t,
    ext_dev_mgr_sub_cb,
    ext_dev_mgr_sub_ud,
    internal_ext_dev_mgr_sub_cb
);

ctx_op_test!(pa_ext_stream_restore_test, pa_ext_stream_restore_test_cb_t);
ctx_op_list!(
    pa_ext_stream_restore_read,
    (),
    pa_ext_stream_restore_read_cb_t,
    (),
    wrap_ctx_ext_stream_restore_cb
);
ctx_op_success!(
    pa_ext_stream_restore_write,
    (mode: pa_update_mode_t, data: *const pa_ext_stream_restore_info, n: c_uint, apply_immediately: c_int),
    (mode, data, n, apply_immediately)
);
ctx_op_success!(
    pa_ext_stream_restore_delete,
    (s: *const *const c_char),
    (s)
);
ctx_ext_subscribe!(pa_ext_stream_restore_subscribe, ext_stream_restore_sub_enabled);
ctx_ext_set_sub_cb!(
    pa_ext_stream_restore_set_subscribe_cb,
    pa_ext_stream_restore_subscribe_cb_t,
    ext_stream_restore_sub_cb,
    ext_stream_restore_sub_ud,
    internal_ext_stream_restore_sub_cb
);

#[no_mangle]
pub unsafe extern "C" fn pa_context_load_module(
    c: *mut pa_context,
    name: *const c_char,
    argument: *const c_char,
    cb: pa_context_index_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real().pa_context_load_module.map_or(ptr::null_mut(), |f| {
        f(
            (*wctx).real_ctx,
            name,
            argument,
            if !w.is_null() {
                Some(wrap_ctx_index_cb)
            } else {
                None
            },
            w.cast(),
        )
    });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_add_autoload(
    c: *mut pa_context,
    name: *const c_char,
    atype: pa_autoload_type_t,
    module: *const c_char,
    argument: *const c_char,
    cb: pa_context_index_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real().pa_context_add_autoload.map_or(ptr::null_mut(), |f| {
        f(
            (*wctx).real_ctx,
            name,
            atype,
            module,
            argument,
            if !w.is_null() {
                Some(wrap_ctx_index_cb)
            } else {
                None
            },
            w.cast(),
        )
    });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_play_sample_with_proplist(
    c: *mut pa_context,
    name: *const c_char,
    dev: *const c_char,
    volume: pa_volume_t,
    proplist: *const pa_proplist,
    cb: pa_context_index_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real()
        .pa_context_play_sample_with_proplist
        .map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                name,
                dev,
                volume,
                proplist,
                if !w.is_null() {
                    Some(wrap_ctx_index_cb)
                } else {
                    None
                },
                w.cast(),
            )
        });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_send_message_to_object(
    c: *mut pa_context,
    recipient_name: *const c_char,
    message: *const c_char,
    message_parameters: *const c_char,
    cb: pa_context_string_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real()
        .pa_context_send_message_to_object
        .map_or(ptr::null_mut(), |f| {
            f(
                (*wctx).real_ctx,
                recipient_name,
                message,
                message_parameters,
                if !w.is_null() {
                    Some(wrap_ctx_string_cb)
                } else {
                    None
                },
                w.cast(),
            )
        });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

#[no_mangle]
pub unsafe extern "C" fn pa_context_stat(
    c: *mut pa_context,
    cb: pa_stat_info_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !ctx_is_live_ready(wctx) {
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return ptr::null_mut();
    }
    let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
    let w = mk_ctx_wrap(wctx, cb_ptr, userdata);
    let op = real().pa_context_stat.map_or(ptr::null_mut(), |f| {
        f(
            (*wctx).real_ctx,
            if !w.is_null() {
                Some(wrap_ctx_stat_cb)
            } else {
                None
            },
            w.cast(),
        )
    });
    if op.is_null() {
        free(w.cast());
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    op
}

// ============================================================================
// Stream Creation / Lifecycle / I/O
// ============================================================================

unsafe fn alloc_wstream(
    wctx: *mut Wctx,
    rs: *mut pa_stream,
    name: *const c_char,
    ss: *const pa_sample_spec,
    map: *const pa_channel_map,
    p: *mut pa_proplist,
) -> *mut Wstream {
    let ws = calloc(1, core::mem::size_of::<Wstream>()).cast::<Wstream>();
    if ws.is_null() {
        return ptr::null_mut();
    }
    (*ws).r#ref = AtomicI32::new(1);
    (*ws).wctx = wctx;
    (*ws).real_stream = rs;
    (*ws).reported_state = PA_STREAM_UNCONNECTED;
    (*ws).name = xstrdup(name);
    if !ss.is_null() {
        (*ws).ss = *ss;
    }
    if !map.is_null() {
        (*ws).cmap = *map;
        (*ws).has_cmap = true;
    } else if !ss.is_null() {
        if let Some(f) = real().pa_channel_map_init_auto {
            f(&raw mut (*ws).cmap, (*ss).channels as c_uint, PA_CHANNEL_MAP_DEFAULT);
        }
        (*ws).has_cmap = true;
    }
    (*ws).proplist = if !p.is_null() {
        real().pa_proplist_copy.map_or(ptr::null_mut(), |f| f(p))
    } else {
        ptr::null_mut()
    };
    (*ws).last_stream_index = PA_INVALID_INDEX;
    (*ws).last_device_index = PA_INVALID_INDEX;

    if !rs.is_null() {
        if let Some(f) = real().pa_stream_set_state_callback {
            f(rs, Some(internal_stream_state_cb), ws.cast());
        }
    }

    pthread_mutex_lock(&raw mut (*wctx).lock);
    (*ws).next = (*wctx).streams;
    (*wctx).streams = ws;
    pthread_mutex_unlock(&raw mut (*wctx).lock);

    pthread_mutex_lock(G_STREAM_LOCK.get());
    (*ws).global_next = *G_STREAM_HEAD.get();
    *G_STREAM_HEAD.get() = ws;
    pthread_mutex_unlock(G_STREAM_LOCK.get());

    pa_context_ref(wctx.cast());
    ws
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_new_with_proplist(
    c: *mut pa_context,
    name: *const c_char,
    ss: *const pa_sample_spec,
    map: *const pa_channel_map,
    p: *mut pa_proplist,
) -> *mut pa_stream {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    if (*wctx).real_ctx.is_null() {
        if !(*wctx).reconnecting || (*wctx).user_disconnected {
            return ptr::null_mut();
        }
        let ws = alloc_wstream(wctx, ptr::null_mut(), name, ss, map, p);
        if !ws.is_null() {
            (*ws).reconnecting = true;
        }
        return ws.cast();
    }
    let rs = real()
        .pa_stream_new_with_proplist
        .map_or(ptr::null_mut(), |f| f((*wctx).real_ctx, name, ss, map, p));
    if rs.is_null() {
        return ptr::null_mut();
    }
    let ws = alloc_wstream(wctx, rs, name, ss, map, p);
    if ws.is_null() {
        if let Some(f) = real().pa_stream_unref {
            f(rs);
        }
        return ptr::null_mut();
    }
    ws.cast()
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_new(
    c: *mut pa_context,
    name: *const c_char,
    ss: *const pa_sample_spec,
    map: *const pa_channel_map,
) -> *mut pa_stream {
    pa_stream_new_with_proplist(c, name, ss, map, ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_new_extended(
    c: *mut pa_context,
    name: *const c_char,
    formats: *const *mut pa_format_info,
    n_formats: c_uint,
    p: *mut pa_proplist,
) -> *mut pa_stream {
    ensure_init();
    let wctx = c.cast::<Wctx>();
    if !is_valid_wctx(wctx) {
        return ptr::null_mut();
    }
    let mut rs: *mut pa_stream = ptr::null_mut();
    if (*wctx).real_ctx.is_null() {
        if !(*wctx).reconnecting || (*wctx).user_disconnected {
            return ptr::null_mut();
        }
    } else {
        rs = real()
            .pa_stream_new_extended
            .map_or(ptr::null_mut(), |f| f((*wctx).real_ctx, name, formats, n_formats, p));
        if rs.is_null() {
            return ptr::null_mut();
        }
    }

    let mut init_ss = pa_sample_spec {
        format: PA_SAMPLE_FLOAT32LE,
        rate: 48000,
        channels: 2,
    };
    let mut init_cmap = pa_channel_map::default();
    if let Some(f) = real().pa_channel_map_init_stereo {
        f(&raw mut init_cmap);
    }
    if n_formats > 0 && !formats.is_null() {
        for i in 0..n_formats as usize {
            let fmt = *formats.add(i);
            if fmt.is_null() {
                continue;
            }
            let mut cand_ss = pa_sample_spec::default();
            let mut cand_cm = pa_channel_map::default();
            if let Some(f) = real().pa_sample_spec_init {
                f(&raw mut cand_ss);
            }
            if let Some(f) = real().pa_channel_map_init {
                f(&raw mut cand_cm);
            }
            if real()
                .pa_format_info_to_sample_spec
                .is_some_and(|f| f(fmt, &raw mut cand_ss, &raw mut cand_cm) == 0)
                && real()
                    .pa_sample_spec_valid
                    .is_some_and(|f| f(&raw const cand_ss) != 0)
            {
                init_ss = cand_ss;
                if real()
                    .pa_channel_map_valid
                    .is_some_and(|f| f(&raw const cand_cm) != 0)
                    && cand_cm.channels == cand_ss.channels
                {
                    init_cmap = cand_cm;
                } else if let Some(f) = real().pa_channel_map_init_extend {
                    f(
                        &raw mut init_cmap,
                        cand_ss.channels as c_uint,
                        PA_CHANNEL_MAP_DEFAULT,
                    );
                }
                break;
            }
        }
    }

    let ws = alloc_wstream(wctx, rs, name, &raw const init_ss, &raw const init_cmap, p);
    if ws.is_null() {
        if !rs.is_null() {
            if let Some(f) = real().pa_stream_unref {
                f(rs);
            }
        }
        return ptr::null_mut();
    }
    if rs.is_null() {
        (*ws).reconnecting = true;
    }
    (*ws).created_extended = true;
    (*ws).n_formats = 0;
    if n_formats > 0 && !formats.is_null() {
        let arr = calloc(
            n_formats as usize,
            core::mem::size_of::<*mut pa_format_info>(),
        )
        .cast::<*mut pa_format_info>();
        (*ws).formats = arr;
        if !arr.is_null() {
            let mut actual_fmts = 0u32;
            for i in 0..n_formats as usize {
                let fmt = *formats.add(i);
                if !fmt.is_null() {
                    if let Some(cf) = real().pa_format_info_copy {
                        let copied = cf(fmt);
                        if !copied.is_null() {
                            *arr.add(actual_fmts as usize) = copied;
                            actual_fmts += 1;
                        }
                    }
                }
            }
            (*ws).n_formats = actual_fmts;
        }
    }
    ws.cast()
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_ref(s: *mut pa_stream) -> *mut pa_stream {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return if s.is_null() {
            ptr::null_mut()
        } else {
            real().pa_stream_ref.map_or(ptr::null_mut(), |f| f(s))
        };
    }
    (*ws).r#ref.fetch_add(1, Ordering::AcqRel);
    s
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_unref(s: *mut pa_stream) {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        if !s.is_null() {
            if let Some(f) = real().pa_stream_unref {
                f(s);
            }
        }
        return;
    }
    if (*ws).r#ref.fetch_sub(1, Ordering::AcqRel) > 1 {
        return;
    }

    pthread_mutex_lock(G_STREAM_LOCK.get());
    let mut gp = G_STREAM_HEAD.get();
    while !(*gp).is_null() {
        if *gp == ws {
            *gp = (*ws).global_next;
            (*ws).global_next = ptr::null_mut();
            break;
        }
        gp = &raw mut (**gp).global_next;
    }
    pthread_mutex_unlock(G_STREAM_LOCK.get());

    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let mut pp = &raw mut (*wctx).streams;
    while !(*pp).is_null() {
        if *pp == ws {
            *pp = (*ws).next;
            break;
        }
        pp = &raw mut (**pp).next;
    }
    let rs = (*ws).real_stream;
    (*ws).real_stream = ptr::null_mut();
    pthread_mutex_unlock(&raw mut (*wctx).lock);

    if !rs.is_null() {
        detach_stream_callbacks(rs);
        if let Some(f) = real().pa_stream_unref {
            f(rs);
        }
    }

    free((*ws).name.cast());
    free((*ws).dev.cast());
    free((*ws).dummy_write_buf);
    if !(*ws).proplist.is_null() {
        if let Some(f) = real().pa_proplist_free {
            f((*ws).proplist);
        }
    }
    if !(*ws).formats.is_null() {
        for i in 0..(*ws).n_formats as usize {
            let f = *(*ws).formats.add(i);
            if !f.is_null() {
                if let Some(ff) = real().pa_format_info_free {
                    ff(f);
                }
            }
        }
        free((*ws).formats.cast());
    }
    free(ws.cast());
    pa_context_unref(wctx.cast());
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_state(s: *const pa_stream) -> pa_stream_state_t {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return PA_STREAM_FAILED;
    }
    if (*ws).user_connected
        && !(*ws).user_disconnected
        && ((*ws).reconnecting || (*(*ws).wctx).reconnecting)
    {
        return PA_STREAM_READY;
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_get_state
            .map_or((*ws).reported_state, |f| f((*ws).real_stream))
    } else {
        (*ws).reported_state
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_context(s: *const pa_stream) -> *mut pa_context {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if is_valid_wstream(ws) {
        (*ws).wctx.cast()
    } else {
        ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_index(s: *const pa_stream) -> u32 {
    ensure_init();
    let ws = s.cast_mut().cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return PA_INVALID_INDEX;
    }
    if !(*ws).real_stream.is_null() {
        let idx = real()
            .pa_stream_get_index
            .map_or(PA_INVALID_INDEX, |f| f((*ws).real_stream));
        if idx != PA_INVALID_INDEX {
            (*ws).last_stream_index = idx;
        }
        return idx;
    }
    (*ws).last_stream_index
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_device_index(s: *const pa_stream) -> u32 {
    ensure_init();
    let ws = s.cast_mut().cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return PA_INVALID_INDEX;
    }
    if !(*ws).real_stream.is_null() {
        let idx = real()
            .pa_stream_get_device_index
            .map_or(PA_INVALID_INDEX, |f| f((*ws).real_stream));
        if idx != PA_INVALID_INDEX {
            (*ws).last_device_index = idx;
        }
        return idx;
    }
    (*ws).last_device_index
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_device_name(s: *const pa_stream) -> *const c_char {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null();
    }
    if !(*ws).real_stream.is_null() {
        return real()
            .pa_stream_get_device_name
            .map_or(ptr::null(), |f| f((*ws).real_stream));
    }
    if !(*ws).dev.is_null() {
        (*ws).dev
    } else {
        c"default".as_ptr()
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_is_suspended(s: *const pa_stream) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_is_suspended
            .map_or(0, |f| f((*ws).real_stream))
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_is_corked(s: *const pa_stream) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_is_corked
            .map_or((*ws).corked, |f| f((*ws).real_stream))
    } else {
        (*ws).corked
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_connect_playback(
    s: *mut pa_stream,
    dev: *const c_char,
    attr: *const pa_buffer_attr,
    flags: pa_stream_flags_t,
    volume: *const pa_cvolume,
    sync_stream: *mut pa_stream,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    pthread_mutex_lock(&raw mut (*(*ws).wctx).lock);
    (*ws).is_record = false;
    (*ws).user_connected = true;
    (*ws).user_disconnected = false;
    free((*ws).dev.cast());
    (*ws).dev = xstrdup(dev);
    if !attr.is_null() {
        (*ws).attr = *attr;
        (*ws).has_attr = true;
    } else {
        (*ws).has_attr = false;
    }
    (*ws).flags = flags;
    (*ws).corked = if (flags & PA_STREAM_START_CORKED) != 0 {
        1
    } else {
        0
    };
    if !volume.is_null() {
        (*ws).volume = *volume;
        (*ws).has_volume = true;
    } else {
        (*ws).has_volume = false;
    }
    (*ws).sync_stream = sync_stream.cast();

    let mut rc = 0;
    if !(*ws).real_stream.is_null() {
        let rs_sync = if !(*ws).sync_stream.is_null() && is_valid_wstream((*ws).sync_stream) {
            (*(*ws).sync_stream).real_stream
        } else {
            ptr::null_mut()
        };
        rc = real().pa_stream_connect_playback.map_or(-1, |f| {
            f((*ws).real_stream, dev, attr, flags, volume, rs_sync)
        });
    } else if !(*(*ws).wctx).reconnecting {
        rc = -1;
    } else {
        (*ws).reconnecting = true;
    }
    pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
    rc
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_connect_record(
    s: *mut pa_stream,
    dev: *const c_char,
    attr: *const pa_buffer_attr,
    flags: pa_stream_flags_t,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    pthread_mutex_lock(&raw mut (*(*ws).wctx).lock);
    (*ws).is_record = true;
    (*ws).user_connected = true;
    (*ws).user_disconnected = false;
    free((*ws).dev.cast());
    (*ws).dev = xstrdup(dev);
    if !attr.is_null() {
        (*ws).attr = *attr;
        (*ws).has_attr = true;
    } else {
        (*ws).has_attr = false;
    }
    (*ws).flags = flags;
    (*ws).corked = if (flags & PA_STREAM_START_CORKED) != 0 {
        1
    } else {
        0
    };

    let mut rc = 0;
    if !(*ws).real_stream.is_null() {
        rc = real()
            .pa_stream_connect_record
            .map_or(-1, |f| f((*ws).real_stream, dev, attr, flags));
    } else if !(*(*ws).wctx).reconnecting {
        rc = -1;
    } else {
        (*ws).reconnecting = true;
    }
    pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
    rc
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_connect_upload(s: *mut pa_stream, length: usize) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream.is_null() {
        return -1;
    }
    real()
        .pa_stream_connect_upload
        .map_or(-1, |f| f((*ws).real_stream, length))
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_finish_upload(s: *mut pa_stream) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) || (*ws).real_stream.is_null() {
        return -1;
    }
    real()
        .pa_stream_finish_upload
        .map_or(-1, |f| f((*ws).real_stream))
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_disconnect(s: *mut pa_stream) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    pthread_mutex_lock(&raw mut (*(*ws).wctx).lock);
    (*ws).user_connected = false;
    (*ws).user_disconnected = true;
    (*ws).reconnecting = false;
    (*ws).reported_state = PA_STREAM_TERMINATED;
    let rs = (*ws).real_stream;
    let mut rc = 0;
    let mut need_cb = rs.is_null();
    if !rs.is_null() {
        rc = real().pa_stream_disconnect.map_or(-1, |f| f(rs));
        if rc < 0 {
            // If real_stream was UNCONNECTED/FAILED/TERMINATED, libpulse rejects disconnect
            // synchronously and will never emit state_cb(TERMINATED); detach it cleanly so
            // pa_stream_get_state reports TERMINATED and waiters don't deadlock.
            (*ws).real_stream = ptr::null_mut();
            need_cb = true;
        }
    }
    let cb = (*ws).state_cb;
    let ud = (*ws).state_ud;
    pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
    if rc < 0 && !rs.is_null() {
        detach_stream_callbacks(rs);
        if let Some(f) = real().pa_stream_unref {
            f(rs);
        }
    }
    if need_cb {
        if let Some(f) = cb {
            f(s, ud);
        }
        return 0;
    }
    rc
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_begin_write(
    s: *mut pa_stream,
    data: *mut *mut c_void,
    nbytes: *mut usize,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) || data.is_null() || nbytes.is_null() {
        return -1;
    }
    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let rs = (*ws).real_stream;
    if !rs.is_null() {
        (*ws).using_dummy_write = false;
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return real()
            .pa_stream_begin_write
            .map_or(-1, |f| f(rs, data, nbytes));
    }
    let req = if *nbytes == usize::MAX || *nbytes == 0 {
        4096
    } else {
        *nbytes
    };
    if req > (*ws).dummy_write_cap {
        let nb = realloc((*ws).dummy_write_buf, req);
        if nb.is_null() {
            pthread_mutex_unlock(&raw mut (*wctx).lock);
            return -1;
        }
        (*ws).dummy_write_buf = nb;
        (*ws).dummy_write_cap = req;
    }
    (*ws).using_dummy_write = true;
    *data = (*ws).dummy_write_buf;
    *nbytes = req;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    0
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_cancel_write(s: *mut pa_stream) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let rs = (*ws).real_stream;
    if (*ws).using_dummy_write || rs.is_null() {
        (*ws).using_dummy_write = false;
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        return 0;
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    real().pa_stream_cancel_write.map_or(-1, |f| f(rs))
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_write(
    s: *mut pa_stream,
    data: *const c_void,
    nbytes: usize,
    free_cb: pa_free_cb_t,
    offset: i64,
    seek: pa_seek_mode_t,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let rs = (*ws).real_stream;
    let dummy_buf = (*ws).dummy_write_buf;
    let is_dummy = (*ws).using_dummy_write && core::ptr::eq(data, dummy_buf);
    if rs.is_null() || is_dummy {
        (*ws).using_dummy_write = false;
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        if !core::ptr::eq(data, dummy_buf) {
            if let Some(fcb) = free_cb {
                fcb(data.cast_mut());
            }
        }
        return 0;
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    real()
        .pa_stream_write
        .map_or(-1, |f| f(rs, data, nbytes, free_cb, offset, seek))
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_write_ext_free(
    s: *mut pa_stream,
    data: *const c_void,
    nbytes: usize,
    free_cb: pa_free_cb_t,
    free_cb_data: *mut c_void,
    offset: i64,
    seek: pa_seek_mode_t,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        if s.is_null() {
            return -1;
        }
        return real().pa_stream_write_ext_free.map_or(-1, |f| {
            f(s, data, nbytes, free_cb, free_cb_data, offset, seek)
        });
    }
    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let rs = (*ws).real_stream;
    let dummy_buf = (*ws).dummy_write_buf;
    let is_dummy = (*ws).using_dummy_write && core::ptr::eq(data, dummy_buf);
    if rs.is_null() || is_dummy {
        (*ws).using_dummy_write = false;
        pthread_mutex_unlock(&raw mut (*wctx).lock);
        if !core::ptr::eq(data, dummy_buf) {
            if let Some(fcb) = free_cb {
                fcb(free_cb_data);
            }
        }
        return 0;
    }
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    real().pa_stream_write_ext_free.map_or(-1, |f| {
        f(rs, data, nbytes, free_cb, free_cb_data, offset, seek)
    })
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_peek(
    s: *mut pa_stream,
    data: *mut *const c_void,
    nbytes: *mut usize,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) || data.is_null() || nbytes.is_null() {
        return -1;
    }
    if (*ws).real_stream.is_null() {
        *data = ptr::null();
        *nbytes = 0;
        return 0;
    }
    real()
        .pa_stream_peek
        .map_or(-1, |f| f((*ws).real_stream, data, nbytes))
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_drop(s: *mut pa_stream) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    if (*ws).real_stream.is_null() {
        return 0;
    }
    real().pa_stream_drop.map_or(-1, |f| f((*ws).real_stream))
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_writable_size(s: *const pa_stream) -> usize {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return usize::MAX;
    }
    if (*ws).real_stream.is_null() {
        return 0;
    }
    real()
        .pa_stream_writable_size
        .map_or(0, |f| f((*ws).real_stream))
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_readable_size(s: *const pa_stream) -> usize {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return usize::MAX;
    }
    if (*ws).real_stream.is_null() {
        return 0;
    }
    real()
        .pa_stream_readable_size
        .map_or(0, |f| f((*ws).real_stream))
}

#[repr(C)]
struct StreamCbWrap {
    ws: *mut Wstream,
    cb: pa_stream_success_cb_t,
    ud: *mut c_void,
}

unsafe extern "C" fn wrap_stream_success_cb(_rs: *mut pa_stream, success: c_int, ud: *mut c_void) {
    let w = ud.cast::<StreamCbWrap>();
    if is_valid_wstream((*w).ws) {
        let ws = (*w).ws;
        pa_stream_ref(ws.cast());
        let reconn = !(*ws).user_disconnected
            && !(*ws).wctx.is_null()
            && !(*(*ws).wctx).user_disconnected
            && ((*ws).reconnecting || (*(*ws).wctx).reconnecting || (*ws).real_stream.is_null());
        let out = if success == 0 && reconn { 1 } else { success };
        if !(*ws).user_disconnected {
            if let Some(cb) = (*w).cb {
                cb(ws.cast(), out, (*w).ud);
            }
        }
        pa_stream_unref(ws.cast());
    }
    free(w.cast());
}

unsafe fn stream_can_synth_op(ws: *mut Wstream) -> bool {
    !ws.is_null()
        && !(*ws).user_disconnected
        && !(*ws).wctx.is_null()
        && !(*(*ws).wctx).user_disconnected
        && ((*ws).reconnecting || (*(*ws).wctx).reconnecting || (*ws).real_stream.is_null())
}

unsafe fn stream_needs_synth_now(ws: *mut Wstream) -> bool {
    if !stream_can_synth_op(ws) {
        return false;
    }
    if (*ws).real_stream.is_null() {
        return true;
    }
    real()
        .pa_stream_get_state
        .is_some_and(|f| f((*ws).real_stream) != PA_STREAM_READY)
}

unsafe fn alloc_stream_wrap(
    ws: *mut Wstream,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut StreamCbWrap {
    let w = calloc(1, core::mem::size_of::<StreamCbWrap>()).cast::<StreamCbWrap>();
    if !w.is_null() {
        (*w).ws = ws;
        (*w).cb = cb;
        (*w).ud = userdata;
    }
    w
}

unsafe fn finish_stream_op(
    ws: *mut Wstream,
    op: *mut pa_operation,
    w: *mut StreamCbWrap,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    if op.is_null() {
        free(w.cast());
        if stream_can_synth_op(ws) {
            let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
            return spawn_synth_stream_op(ws, cb_ptr, userdata);
        }
    }
    op
}

macro_rules! stream_op_success {
    ($fname:ident, ($($pname:ident : $pty:ty),*), ($($fwd:expr),*)) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fname(
            s: *mut pa_stream,
            $($pname : $pty,)*
            cb: pa_stream_success_cb_t,
            userdata: *mut c_void,
        ) -> *mut pa_operation {
            ensure_init();
            let ws = s.cast::<Wstream>();
            if !is_valid_wstream(ws) {
                return ptr::null_mut();
            }
            if stream_needs_synth_now(ws) {
                let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
                return spawn_synth_stream_op(ws, cb_ptr, userdata);
            }
            if (*ws).real_stream.is_null() {
                return ptr::null_mut();
            }
            let Some(rf) = real().$fname else {
                return ptr::null_mut();
            };
            let w = alloc_stream_wrap(ws, cb, userdata);
            let w_cb: pa_stream_success_cb_t = if !w.is_null() {
                Some(wrap_stream_success_cb)
            } else {
                None
            };
            let op = rf((*ws).real_stream, $($fwd,)* w_cb, w.cast());
            finish_stream_op(ws, op, w, cb, userdata)
        }
    };
}

stream_op_success!(pa_stream_drain, (), ());

#[no_mangle]
pub unsafe extern "C" fn pa_stream_update_timing_info(
    s: *mut pa_stream,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        if s.is_null() {
            return ptr::null_mut();
        }
        return real()
            .pa_stream_update_timing_info
            .map_or(ptr::null_mut(), |f| f(s, cb, userdata));
    }
    if stream_needs_synth_now(ws) {
        let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
        return spawn_synth_stream_op(ws, cb_ptr, userdata);
    }
    if (*ws).real_stream.is_null() {
        return ptr::null_mut();
    }
    let Some(rf) = real().pa_stream_update_timing_info else {
        return ptr::null_mut();
    };
    let w = alloc_stream_wrap(ws, cb, userdata);
    let op = rf(
        (*ws).real_stream,
        if !w.is_null() {
            Some(wrap_stream_success_cb)
        } else {
            None
        },
        w.cast(),
    );
    finish_stream_op(ws, op, w, cb, userdata)
}

stream_op_success!(pa_stream_flush, (), ());
stream_op_success!(pa_stream_prebuf, (), ());
stream_op_success!(pa_stream_trigger, (), ());

#[no_mangle]
pub unsafe extern "C" fn pa_stream_proplist_update(
    s: *mut pa_stream,
    mode: pa_update_mode_t,
    p: *mut pa_proplist,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        if s.is_null() {
            return ptr::null_mut();
        }
        return real()
            .pa_stream_proplist_update
            .map_or(ptr::null_mut(), |f| f(s, mode, p, cb, userdata));
    }
    if stream_needs_synth_now(ws) {
        let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
        return spawn_synth_stream_op(ws, cb_ptr, userdata);
    }
    if (*ws).real_stream.is_null() {
        return ptr::null_mut();
    }
    let Some(rf) = real().pa_stream_proplist_update else {
        return ptr::null_mut();
    };
    let w = alloc_stream_wrap(ws, cb, userdata);
    let op = rf(
        (*ws).real_stream,
        mode,
        p,
        if !w.is_null() {
            Some(wrap_stream_success_cb)
        } else {
            None
        },
        w.cast(),
    );
    finish_stream_op(ws, op, w, cb, userdata)
}

stream_op_success!(pa_stream_proplist_remove, (keys: *const *const c_char), (keys));

#[no_mangle]
pub unsafe extern "C" fn pa_stream_cork(
    s: *mut pa_stream,
    b: c_int,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null_mut();
    }
    (*ws).corked = if b != 0 { 1 } else { 0 };
    if stream_needs_synth_now(ws) {
        let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
        return spawn_synth_stream_op(ws, cb_ptr, userdata);
    }
    if (*ws).real_stream.is_null() {
        return ptr::null_mut();
    }
    let Some(rf) = real().pa_stream_cork else {
        return ptr::null_mut();
    };
    let w = alloc_stream_wrap(ws, cb, userdata);
    let op = rf(
        (*ws).real_stream,
        b,
        if !w.is_null() {
            Some(wrap_stream_success_cb)
        } else {
            None
        },
        w.cast(),
    );
    finish_stream_op(ws, op, w, cb, userdata)
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_name(
    s: *mut pa_stream,
    name: *const c_char,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null_mut();
    }
    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    free((*ws).name.cast());
    (*ws).name = xstrdup(name);
    let rs = (*ws).real_stream;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    if stream_needs_synth_now(ws) {
        let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
        return spawn_synth_stream_op(ws, cb_ptr, userdata);
    }
    if rs.is_null() {
        return ptr::null_mut();
    }
    let Some(rf) = real().pa_stream_set_name else {
        return ptr::null_mut();
    };
    let w = alloc_stream_wrap(ws, cb, userdata);
    let op = rf(
        rs,
        name,
        if !w.is_null() {
            Some(wrap_stream_success_cb)
        } else {
            None
        },
        w.cast(),
    );
    finish_stream_op(ws, op, w, cb, userdata)
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_buffer_attr(
    s: *mut pa_stream,
    attr: *const pa_buffer_attr,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null_mut();
    }
    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    if !attr.is_null() {
        (*ws).attr = *attr;
        (*ws).has_attr = true;
    }
    let rs = (*ws).real_stream;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    if stream_needs_synth_now(ws) {
        let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
        return spawn_synth_stream_op(ws, cb_ptr, userdata);
    }
    if rs.is_null() {
        return ptr::null_mut();
    }
    let Some(rf) = real().pa_stream_set_buffer_attr else {
        return ptr::null_mut();
    };
    let w = alloc_stream_wrap(ws, cb, userdata);
    let op = rf(
        rs,
        attr,
        if !w.is_null() {
            Some(wrap_stream_success_cb)
        } else {
            None
        },
        w.cast(),
    );
    finish_stream_op(ws, op, w, cb, userdata)
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_update_sample_rate(
    s: *mut pa_stream,
    rate: u32,
    cb: pa_stream_success_cb_t,
    userdata: *mut c_void,
) -> *mut pa_operation {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null_mut();
    }
    (*ws).ss.rate = rate;
    if stream_needs_synth_now(ws) {
        let cb_ptr = cb.map_or(ptr::null_mut(), |f| f as *mut c_void);
        return spawn_synth_stream_op(ws, cb_ptr, userdata);
    }
    if (*ws).real_stream.is_null() {
        return ptr::null_mut();
    }
    let Some(rf) = real().pa_stream_update_sample_rate else {
        return ptr::null_mut();
    };
    let w = alloc_stream_wrap(ws, cb, userdata);
    let op = rf(
        (*ws).real_stream,
        rate,
        if !w.is_null() {
            Some(wrap_stream_success_cb)
        } else {
            None
        },
        w.cast(),
    );
    finish_stream_op(ws, op, w, cb, userdata)
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_state_callback(
    s: *mut pa_stream,
    cb: pa_stream_notify_cb_t,
    userdata: *mut c_void,
) {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return;
    }
    (*ws).state_cb = cb;
    (*ws).state_ud = userdata;
}

macro_rules! stream_set_cb {
    ($fname:ident, $cb_ty:ty, $field_cb:ident, $field_ud:ident, $internal_cb:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fname(s: *mut pa_stream, cb: $cb_ty, userdata: *mut c_void) {
            ensure_init();
            let ws = s.cast::<Wstream>();
            if !is_valid_wstream(ws) {
                return;
            }
            (*ws).$field_cb = cb;
            (*ws).$field_ud = userdata;
            if !(*ws).real_stream.is_null() {
                if let Some(rf) = real().$fname {
                    rf(
                        (*ws).real_stream,
                        if cb.is_some() {
                            Some($internal_cb)
                        } else {
                            None
                        },
                        ws.cast(),
                    );
                }
            }
        }
    };
}

stream_set_cb!(
    pa_stream_set_write_callback,
    pa_stream_request_cb_t,
    write_cb,
    write_ud,
    internal_stream_write_cb
);
stream_set_cb!(
    pa_stream_set_read_callback,
    pa_stream_request_cb_t,
    read_cb,
    read_ud,
    internal_stream_read_cb
);
stream_set_cb!(
    pa_stream_set_overflow_callback,
    pa_stream_notify_cb_t,
    overflow_cb,
    overflow_ud,
    internal_stream_overflow_cb
);
stream_set_cb!(
    pa_stream_set_underflow_callback,
    pa_stream_notify_cb_t,
    underflow_cb,
    underflow_ud,
    internal_stream_underflow_cb
);
stream_set_cb!(
    pa_stream_set_started_callback,
    pa_stream_notify_cb_t,
    started_cb,
    started_ud,
    internal_stream_started_cb
);
stream_set_cb!(
    pa_stream_set_latency_update_callback,
    pa_stream_notify_cb_t,
    latency_cb,
    latency_ud,
    internal_stream_latency_cb
);
stream_set_cb!(
    pa_stream_set_moved_callback,
    pa_stream_notify_cb_t,
    moved_cb,
    moved_ud,
    internal_stream_moved_cb
);
stream_set_cb!(
    pa_stream_set_suspended_callback,
    pa_stream_notify_cb_t,
    suspended_cb,
    suspended_ud,
    internal_stream_suspended_cb
);
stream_set_cb!(
    pa_stream_set_event_callback,
    pa_stream_event_cb_t,
    event_cb,
    event_ud,
    internal_stream_event_cb
);
stream_set_cb!(
    pa_stream_set_buffer_attr_callback,
    pa_stream_notify_cb_t,
    buffer_attr_cb,
    buffer_attr_ud,
    internal_stream_buffer_attr_cb
);

#[inline]
unsafe fn stream_is_reconnecting_fallback(ws: *const Wstream) -> bool {
    (*ws).ever_ready
        && !(*ws).user_disconnected
        && !(*ws).wctx.is_null()
        && !(*(*ws).wctx).user_disconnected
        && ((*ws).reconnecting || (*(*ws).wctx).reconnecting)
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_time(s: *mut pa_stream, r_usec: *mut pa_usec_t) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        if s.is_null() {
            return -1;
        }
        return real()
            .pa_stream_get_time
            .map_or(-1, |f| f(s, r_usec));
    }

    pthread_mutex_lock(&raw mut (*(*ws).wctx).lock);
    if (*ws).real_stream.is_null() {
        if stream_is_reconnecting_fallback(ws) {
            if !r_usec.is_null() {
                *r_usec = (*ws).last_reported_time_us;
            }
            pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
            return 0;
        }
        pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
        return -1;
    }
    let mut raw: pa_usec_t = 0;
    let rc = real()
        .pa_stream_get_time
        .map_or(-1, |f| f((*ws).real_stream, &raw mut raw));
    if rc == 0 {
        let mut total = (*ws).time_offset_us.saturating_add(raw);
        if total < (*ws).last_reported_time_us {
            total = (*ws).last_reported_time_us;
        }
        (*ws).last_reported_time_us = total;
        if !r_usec.is_null() {
            *r_usec = total;
        }
        pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
        return 0;
    }
    if stream_is_reconnecting_fallback(ws) {
        if !r_usec.is_null() {
            *r_usec = (*ws).last_reported_time_us;
        }
        pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
        return 0;
    }
    pthread_mutex_unlock(&raw mut (*(*ws).wctx).lock);
    rc
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_latency(
    s: *mut pa_stream,
    r_usec: *mut pa_usec_t,
    negative: *mut c_int,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    let lock = &raw mut (*(*ws).wctx).lock;
    pthread_mutex_lock(lock);
    let rs = (*ws).real_stream;
    let reconnecting = stream_is_reconnecting_fallback(ws);
    if rs.is_null() {
        pthread_mutex_unlock(lock);
        if reconnecting {
            if !r_usec.is_null() {
                *r_usec = 20_000;
            }
            if !negative.is_null() {
                *negative = 0;
            }
            return 0;
        }
        return -1;
    }
    let rc = real()
        .pa_stream_get_latency
        .map_or(-1, |f| f(rs, r_usec, negative));
    if rc < 0 && stream_is_reconnecting_fallback(ws) {
        pthread_mutex_unlock(lock);
        if !r_usec.is_null() {
            *r_usec = 20_000;
        }
        if !negative.is_null() {
            *negative = 0;
        }
        return 0;
    }
    pthread_mutex_unlock(lock);
    rc
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_timing_info(s: *mut pa_stream) -> *const pa_timing_info {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null();
    }
    let lock = &raw mut (*(*ws).wctx).lock;
    pthread_mutex_lock(lock);
    let rs = (*ws).real_stream;
    if !rs.is_null() {
        let ti = real()
            .pa_stream_get_timing_info
            .map_or(ptr::null(), |f| f(rs));
        if !ti.is_null() {
            (*ws).last_timing = *ti;
            (*ws).has_last_timing = true;
            pthread_mutex_unlock(lock);
            return ti;
        }
        if (*ws).has_last_timing && stream_is_reconnecting_fallback(ws) {
            (*ws).last_timing.playing = i32::from((*ws).corked == 0);
            pthread_mutex_unlock(lock);
            return &raw const (*ws).last_timing;
        }
        pthread_mutex_unlock(lock);
        return ptr::null();
    }
    if (*ws).has_last_timing && stream_is_reconnecting_fallback(ws) {
        (*ws).last_timing.playing = i32::from((*ws).corked == 0);
        pthread_mutex_unlock(lock);
        return &raw const (*ws).last_timing;
    }
    pthread_mutex_unlock(lock);
    ptr::null()
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_sample_spec(s: *mut pa_stream) -> *const pa_sample_spec {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null();
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_get_sample_spec
            .map_or(&raw const (*ws).ss, |f| f((*ws).real_stream))
    } else {
        &raw const (*ws).ss
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_channel_map(s: *mut pa_stream) -> *const pa_channel_map {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null();
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_get_channel_map
            .map_or(&raw const (*ws).cmap, |f| f((*ws).real_stream))
    } else {
        &raw const (*ws).cmap
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_format_info(s: *const pa_stream) -> *const pa_format_info {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null();
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_get_format_info
            .map_or(ptr::null(), |f| f((*ws).real_stream))
    } else if !(*ws).formats.is_null() && (*ws).n_formats > 0 {
        *(*ws).formats
    } else {
        ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_buffer_attr(s: *mut pa_stream) -> *const pa_buffer_attr {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return ptr::null();
    }
    let wctx = (*ws).wctx;
    pthread_mutex_lock(&raw mut (*wctx).lock);
    let rs = (*ws).real_stream;
    let has_attr = (*ws).has_attr;
    pthread_mutex_unlock(&raw mut (*wctx).lock);
    if !rs.is_null() {
        real()
            .pa_stream_get_buffer_attr
            .map_or(ptr::null(), |f| f(rs))
    } else if has_attr {
        &raw const (*ws).attr
    } else {
        ptr::null()
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_set_monitor_stream(
    s: *mut pa_stream,
    sink_input_idx: u32,
) -> c_int {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    (*ws).has_monitor_stream = true;
    (*ws).monitor_stream = sink_input_idx;
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_set_monitor_stream
            .map_or(-1, |f| f((*ws).real_stream, sink_input_idx))
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_monitor_stream(s: *const pa_stream) -> u32 {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return PA_INVALID_INDEX;
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_get_monitor_stream
            .map_or(PA_INVALID_INDEX, |f| f((*ws).real_stream))
    } else if (*ws).has_monitor_stream {
        (*ws).monitor_stream
    } else {
        PA_INVALID_INDEX
    }
}

#[no_mangle]
pub unsafe extern "C" fn pa_stream_get_underflow_index(s: *const pa_stream) -> i64 {
    ensure_init();
    let ws = s.cast::<Wstream>();
    if !is_valid_wstream(ws) {
        return -1;
    }
    if !(*ws).real_stream.is_null() {
        real()
            .pa_stream_get_underflow_index
            .map_or(0, |f| f((*ws).real_stream))
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn check_fallback(
        ever_ready: bool,
        stream_reconnecting: bool,
        ctx_reconnecting: bool,
        user_disconnected: bool,
    ) -> bool {
        let mut ctx: Wctx = core::mem::zeroed();
        ctx.reconnecting = ctx_reconnecting;
        ctx.user_disconnected = user_disconnected;
        let mut st: Wstream = core::mem::zeroed();
        st.wctx = &raw mut ctx;
        st.ever_ready = ever_ready;
        st.reconnecting = stream_reconnecting;
        st.user_disconnected = user_disconnected;
        stream_is_reconnecting_fallback(&raw const st)
    }

    #[test]
    fn fallback_requires_active_reconnect() {
        unsafe {
            assert!(!check_fallback(true, false, false, false));
            assert!(check_fallback(true, true, false, false));
            assert!(check_fallback(true, false, true, false));
            assert!(!check_fallback(false, true, true, false));
            assert!(!check_fallback(true, true, true, true));
        }
    }

    #[test]
    fn tcp_fallback_targets_local_unix_on_even_retries() {
        unsafe {
            assert!(server_allows_tcp_fallback(ptr::null(), false));
            assert!(server_allows_tcp_fallback(c"".as_ptr(), true));
            assert!(server_allows_tcp_fallback(c"unix:/run/pressure-vessel/pulse/native".as_ptr(), true));
            assert!(server_allows_tcp_fallback(c"/run/user/1000/pulse/native".as_ptr(), true));
            assert!(!server_allows_tcp_fallback(c"tcp:192.168.1.10".as_ptr(), true));
            assert!(!server_allows_tcp_fallback(c"tcp4:127.0.0.1:4713".as_ptr(), true));

            assert!(!should_try_tcp_fallback(0, ptr::null(), false));
            assert!(!should_try_tcp_fallback(1, ptr::null(), false));
            assert!(should_try_tcp_fallback(2, ptr::null(), false));
            assert!(!should_try_tcp_fallback(3, ptr::null(), false));
            assert!(should_try_tcp_fallback(4, c"unix:/run/pressure-vessel/pulse/native".as_ptr(), true));
            assert!(!should_try_tcp_fallback(4, c"tcp:10.0.0.2".as_ptr(), true));
        }
        assert_eq!(parse_salsa_tcp_port_str("41235\n"), Some(41235));
        assert_eq!(parse_salsa_tcp_port_str("0"), None);
        assert_eq!(parse_salsa_tcp_port_str("abc"), None);
    }

    #[test]
    fn rttime_falls_back_to_mainloop_api_when_real_ctx_null() {
        #[derive(Default)]
        struct FakeTimeState {
            new_calls: usize,
            restart_calls: usize,
            last_null_tv: bool,
            last_sec: c_long,
            last_usec: c_long,
        }

        unsafe extern "C" fn fake_time_new(
            api: *mut pa_mainloop_api,
            tv: *const TimeVal,
            _cb: pa_time_event_cb_t,
            _ud: *mut c_void,
        ) -> *mut pa_time_event {
            let st = &mut *((*api).userdata.cast::<FakeTimeState>());
            st.new_calls += 1;
            if tv.is_null() {
                st.last_null_tv = true;
            } else {
                st.last_null_tv = false;
                st.last_sec = (*tv).tv_sec;
                st.last_usec = (*tv).tv_usec;
            }
            0x1234usize as *mut pa_time_event
        }

        unsafe extern "C" fn fake_time_restart(e: *mut pa_time_event, tv: *const TimeVal) {
            let st = &mut *(e.cast::<FakeTimeState>());
            st.restart_calls += 1;
            if tv.is_null() {
                st.last_null_tv = true;
            } else {
                st.last_null_tv = false;
                st.last_sec = (*tv).tv_sec;
                st.last_usec = (*tv).tv_usec;
            }
        }

        unsafe {
            let mut tv = TimeVal::default();
            assert!(rttime_to_timeval(PA_USEC_INVALID, &raw mut tv).is_null());
            let p = rttime_to_timeval(12_345_678, &raw mut tv);
            assert!(!p.is_null());
            assert_eq!((*p).tv_sec, 12);
            assert_eq!((*p).tv_usec, 345_678 | PA_TIMEVAL_RTCLOCK);

            let mut fake = FakeTimeState::default();
            let mut api: pa_mainloop_api = core::mem::zeroed();
            api.userdata = (&raw mut fake).cast();
            api.time_new = Some(fake_time_new);
            api.time_restart = Some(fake_time_restart);

            let mut ctx: Wctx = core::mem::zeroed();
            ctx.api = &raw mut api;
            ctx.real_ctx = ptr::null_mut();

            pthread_mutex_lock(G_CTX_LOCK.get());
            ctx.next = *G_CTX_HEAD.get();
            *G_CTX_HEAD.get() = &raw mut ctx;
            pthread_mutex_unlock(G_CTX_LOCK.get());

            let ev = pa_context_rttime_new(
                (&raw mut ctx).cast(),
                5_010_000,
                None,
                ptr::null_mut(),
            );
            assert_eq!(ev as usize, 0x1234);
            assert_eq!(fake.new_calls, 1);
            assert!(!fake.last_null_tv);
            assert_eq!(fake.last_sec, 5);
            assert_eq!(fake.last_usec, 10_000 | PA_TIMEVAL_RTCLOCK);

            let fake_ev = (&raw mut fake).cast::<pa_time_event>();
            pa_context_rttime_restart((&raw mut ctx).cast(), fake_ev, 9_020_000);
            assert_eq!(fake.restart_calls, 1);
            assert!(!fake.last_null_tv);
            assert_eq!(fake.last_sec, 9);
            assert_eq!(fake.last_usec, 20_000 | PA_TIMEVAL_RTCLOCK);

            pa_context_rttime_restart((&raw mut ctx).cast(), fake_ev, PA_USEC_INVALID);
            assert_eq!(fake.restart_calls, 2);
            assert!(fake.last_null_tv);

            pthread_mutex_lock(G_CTX_LOCK.get());
            let mut pp = G_CTX_HEAD.get();
            while !(*pp).is_null() {
                if core::ptr::eq(*pp, &raw mut ctx) {
                    *pp = ctx.next;
                    break;
                }
                pp = &raw mut (**pp).next;
            }
            pthread_mutex_unlock(G_CTX_LOCK.get());
        }
    }
}
