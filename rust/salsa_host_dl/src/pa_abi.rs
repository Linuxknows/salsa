// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_int, c_long, c_uint, c_void};

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

pub const PA_CHANNELS_MAX: usize = 32;
pub const PA_OK: c_int = 0;
pub const PA_ERR_INVALID: c_int = 3;
pub const PA_INVALID_INDEX: u32 = u32::MAX;
pub const PA_PROTOCOL_VERSION: u32 = 35;
pub const PA_VOLUME_NORM: pa_volume_t = 0x1_0000;
pub const PA_VOLUME_MUTED: pa_volume_t = 0;

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
pub const PA_STREAM_RECORD: c_int = 2;
pub const PA_STREAM_START_CORKED: c_uint = 0x0001;

pub const PA_SAMPLE_S16LE: c_int = 3;
pub const PA_SAMPLE_S16BE: c_int = 4;
pub const PA_SAMPLE_FLOAT32LE: c_int = 5;
pub const PA_SAMPLE_S32LE: c_int = 7;
pub const PA_SAMPLE_S32BE: c_int = 8;

pub const PA_CHANNEL_MAP_DEFAULT: c_int = 0;
pub const PA_CHANNEL_POSITION_FRONT_LEFT: c_int = 1;
pub const PA_CHANNEL_POSITION_FRONT_RIGHT: c_int = 2;

pub const PA_SINK_RUNNING: c_int = 0;
pub const PA_SOURCE_RUNNING: c_int = 0;
pub const PA_SINK_LATENCY: c_uint = 0x0002;
pub const PA_SINK_HARDWARE: c_uint = 0x0004;
pub const PA_SOURCE_LATENCY: c_uint = 0x0002;
pub const PA_SOURCE_HARDWARE: c_uint = 0x0004;

pub const PA_SUBSCRIPTION_MASK_NULL: c_uint = 0x0000;
pub const PA_SUBSCRIPTION_MASK_SINK: c_uint = 0x0001;
pub const PA_SUBSCRIPTION_MASK_SOURCE: c_uint = 0x0002;
pub const PA_SUBSCRIPTION_MASK_SERVER: c_uint = 0x0080;

pub const PA_SUBSCRIPTION_EVENT_SINK: c_uint = 0x0000;
pub const PA_SUBSCRIPTION_EVENT_SOURCE: c_uint = 0x0001;
pub const PA_SUBSCRIPTION_EVENT_SERVER: c_uint = 0x0007;
pub const PA_SUBSCRIPTION_EVENT_NEW: c_uint = 0x0000;
pub const PA_SUBSCRIPTION_EVENT_CHANGE: c_uint = 0x0010;

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
    pub map: [c_int; PA_CHANNELS_MAX],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct pa_cvolume {
    pub channels: u8,
    pub values: [pa_volume_t; PA_CHANNELS_MAX],
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
pub struct pa_sink_input_info {
    pub index: u32,
    pub name: *const c_char,
    pub owner_module: u32,
    pub client: u32,
    pub sink: u32,
    pub sample_spec: pa_sample_spec,
    pub channel_map: pa_channel_map,
    pub volume: pa_cvolume,
    pub buffer_usec: pa_usec_t,
    pub sink_usec: pa_usec_t,
    pub resample_method: *const c_char,
    pub driver: *const c_char,
    pub mute: c_int,
    pub proplist: *mut pa_proplist,
    pub corked: c_int,
    pub has_volume: c_int,
    pub volume_writable: c_int,
    pub format: *mut pa_format_info,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pa_source_output_info {
    pub index: u32,
    pub name: *const c_char,
    pub owner_module: u32,
    pub client: u32,
    pub source: u32,
    pub sample_spec: pa_sample_spec,
    pub channel_map: pa_channel_map,
    pub volume: pa_cvolume,
    pub buffer_usec: pa_usec_t,
    pub source_usec: pa_usec_t,
    pub resample_method: *const c_char,
    pub driver: *const c_char,
    pub mute: c_int,
    pub proplist: *mut pa_proplist,
    pub corked: c_int,
    pub has_volume: c_int,
    pub volume_writable: c_int,
    pub format: *mut pa_format_info,
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn canonical_pa_abi_layouts_sane() {
        assert_eq!(size_of::<pa_sample_spec>(), 12);
        assert_eq!(align_of::<pa_sample_spec>(), 4);
        assert_eq!(size_of::<pa_buffer_attr>(), 20);
        assert_eq!(offset_of!(pa_channel_map, map), 4);
        assert_eq!(offset_of!(pa_cvolume, values), 4);
        assert_eq!(size_of::<pa_spawn_api>(), 3 * size_of::<usize>());
        assert_eq!(size_of::<pa_timing_info>(), size_of::<TimeVal>() + 88);
    }
}
