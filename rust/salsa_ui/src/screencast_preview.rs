// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use salsa_tools::host::{state_dir, write_atomic_secure};
use salsa_wire::pw_wire::{
    frame_pw_msg, parse_pkt_hdr_status, Dict, PodBuilder, PodCursor, PktHdrStatus, CHOICE_RANGE,
    DATA_MEM_FD, IO_BUFFERS, MEDIA_VIDEO, OBJ_FORMAT, OBJ_PARAM_BUFS, PARAM_BUFFERS,
    PARAM_ENUM_FORMAT, SPA_FRAC, SPA_ID_INVALID, SPA_OBJECT, SPA_RECT, SUB_RAW,
    VF_ABGR, VF_ARGB, VF_BGRA, VF_BGRX, VF_RGBA, VF_RGBX, VF_XBGR, VF_XRGB,
};
use salsa_wire::unix_sys::{
    dl_open_any, do_poll, eventfd_drain, eventfd_signal, mono_ns, recvmsg_with_fds,
    sendmsg_all_with_fds, MappedMem, PollFd, RecvFdsResult, POLLIN,
};

use crate::gio_dbus::{cstr, glib, gobj, load_sym, G_BUS_TYPE_SESSION};

const PW_CORE_M_HELLO: u8 = 1;
const PW_CORE_M_CREATE_OBJ: u8 = 6;
const PW_CORE_E_ERROR: u8 = 3;
const PW_CORE_E_ADD_MEM: u8 = 6;
const PW_CORE_E_REMOVE_MEM: u8 = 7;

const PW_CN_M_UPDATE: u8 = 2;
const PW_CN_M_PORT_UPDATE: u8 = 3;
const PW_CN_M_SET_ACTIVE: u8 = 4;

const PW_CN_E_TRANSPORT: u8 = 0;
const PW_CN_E_COMMAND: u8 = 4;
const PW_CN_E_PORT_SET_IO: u8 = 9;
const PW_CN_E_SET_ACTIVATION: u8 = 10;

const SPA_NODE_CMD_START: u32 = 2;
const PWB_STATUS_NEED_DATA: u32 = 1 << 0;
const PWB_STATUS_HAVE_DATA: u32 = 1 << 1;

const PORTAL_DEST: &CStr = c"org.freedesktop.portal.Desktop";
const PORTAL_PATH: &CStr = c"/org/freedesktop/portal/desktop";
const PORTAL_SC_IFACE: &CStr = c"org.freedesktop.portal.ScreenCast";
const PORTAL_REQ_IFACE: &CStr = c"org.freedesktop.portal.Request";
const PORTAL_SESS_IFACE: &CStr = c"org.freedesktop.portal.Session";

type SignalCallback = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_void,
    *mut c_void,
);

struct PortalGlibExt {
    g_main_context_iteration: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    g_variant_builder_new: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    g_variant_builder_add_value: unsafe extern "C" fn(*mut c_void, *mut c_void),
    g_variant_builder_end: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    g_variant_builder_unref: unsafe extern "C" fn(*mut c_void),
    g_variant_new_dict_entry: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    g_variant_new_variant: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    g_variant_new_object_path: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    g_variant_lookup_value:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_void) -> *mut c_void,
    g_variant_get_uint32: unsafe extern "C" fn(*mut c_void) -> u32,
    g_variant_get_handle: unsafe extern "C" fn(*mut c_void) -> i32,
}

struct PortalGioExt {
    g_bus_get_sync: unsafe extern "C" fn(c_int, *mut c_void, *mut *mut c_void) -> *mut c_void,
    g_dbus_connection_get_unique_name: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    g_dbus_connection_signal_subscribe: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        c_uint,
        Option<SignalCallback>,
        *mut c_void,
        Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_uint,
    g_dbus_connection_signal_unsubscribe: unsafe extern "C" fn(*mut c_void, c_uint),
    g_dbus_connection_call_sync: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *mut c_void,
        *const c_void,
        c_uint,
        c_int,
        *mut c_void,
        *mut *mut c_void,
    ) -> *mut c_void,
    g_dbus_connection_call_with_unix_fd_list_sync: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *mut c_void,
        *const c_void,
        c_uint,
        c_int,
        *mut c_void,
        *mut *mut c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> *mut c_void,
    g_unix_fd_list_get: unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_void) -> c_int,
}

static PORTAL_GLIB_EXT: OnceLock<Result<PortalGlibExt, String>> = OnceLock::new();
static PORTAL_GIO_EXT: OnceLock<Result<PortalGioExt, String>> = OnceLock::new();

fn portal_glib() -> Result<&'static PortalGlibExt, &'static str> {
    PORTAL_GLIB_EXT
        .get_or_init(|| unsafe {
            let lib = dl_open_any(&["libglib-2.0.so.0", "libglib-2.0.so"]);
            if lib.is_null() {
                return Err("failed dlopen libglib-2.0.so.0".into());
            }
            Ok(PortalGlibExt {
                g_main_context_iteration: load_sym(lib, c"g_main_context_iteration")?,
                g_variant_builder_new: load_sym(lib, c"g_variant_builder_new")?,
                g_variant_builder_add_value: load_sym(lib, c"g_variant_builder_add_value")?,
                g_variant_builder_end: load_sym(lib, c"g_variant_builder_end")?,
                g_variant_builder_unref: load_sym(lib, c"g_variant_builder_unref")?,
                g_variant_new_dict_entry: load_sym(lib, c"g_variant_new_dict_entry")?,
                g_variant_new_variant: load_sym(lib, c"g_variant_new_variant")?,
                g_variant_new_object_path: load_sym(lib, c"g_variant_new_object_path")?,
                g_variant_lookup_value: load_sym(lib, c"g_variant_lookup_value")?,
                g_variant_get_uint32: load_sym(lib, c"g_variant_get_uint32")?,
                g_variant_get_handle: load_sym(lib, c"g_variant_get_handle")?,
            })
        })
        .as_ref()
        .map_err(String::as_str)
}

fn portal_gio() -> Result<&'static PortalGioExt, &'static str> {
    PORTAL_GIO_EXT
        .get_or_init(|| unsafe {
            let lib = dl_open_any(&["libgio-2.0.so.0", "libgio-2.0.so"]);
            if lib.is_null() {
                return Err("failed dlopen libgio-2.0.so.0".into());
            }
            Ok(PortalGioExt {
                g_bus_get_sync: load_sym(lib, c"g_bus_get_sync")?,
                g_dbus_connection_get_unique_name: load_sym(
                    lib,
                    c"g_dbus_connection_get_unique_name",
                )?,
                g_dbus_connection_signal_subscribe: load_sym(
                    lib,
                    c"g_dbus_connection_signal_subscribe",
                )?,
                g_dbus_connection_signal_unsubscribe: load_sym(
                    lib,
                    c"g_dbus_connection_signal_unsubscribe",
                )?,
                g_dbus_connection_call_sync: load_sym(lib, c"g_dbus_connection_call_sync")?,
                g_dbus_connection_call_with_unix_fd_list_sync: load_sym(
                    lib,
                    c"g_dbus_connection_call_with_unix_fd_list_sync",
                )?,
                g_unix_fd_list_get: load_sym(lib, c"g_unix_fd_list_get")?,
            })
        })
        .as_ref()
        .map_err(String::as_str)
}

#[derive(Clone, Debug)]
pub struct PortalPreviewStatus {
    pub state: &'static str,
    pub message: String,
    pub node_id: u32,
    pub has_token: bool,
}

impl Default for PortalPreviewStatus {
    fn default() -> Self {
        Self {
            state: "idle",
            message: String::new(),
            node_id: 0,
            has_token: has_saved_restore_token(),
        }
    }
}

fn restore_token_path() -> PathBuf {
    state_dir().join("screencast_restore_token")
}

pub fn has_saved_restore_token() -> bool {
    load_restore_token().is_some()
}

fn load_restore_token() -> Option<String> {
    let p = restore_token_path();
    let meta = fs::symlink_metadata(&p).ok()?;
    if meta.file_type().is_symlink() || meta.len() > 1024 {
        return None;
    }
    let txt = fs::read_to_string(&p).ok()?;
    let trimmed = txt.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn save_restore_token(token: &str) {
    let trimmed = token.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return;
    }
    let _ = write_atomic_secure(&restore_token_path(), format!("{trimmed}\n").as_bytes(), 0o600);
}

pub fn clear_restore_token() {
    let _ = fs::remove_file(restore_token_path());
}

struct PortalResponseSlot {
    expected_path: String,
    returned_path: Option<String>,
    done: bool,
    response_code: u32,
    session_handle: Option<String>,
    restore_token: Option<String>,
    node_id: Option<u32>,
}

unsafe extern "C" fn on_portal_request_response(
    _conn: *mut c_void,
    _sender: *const c_char,
    obj_path: *const c_char,
    _iface: *const c_char,
    _signal: *const c_char,
    parameters: *mut c_void,
    user_data: *mut c_void,
) {
    if obj_path.is_null() || parameters.is_null() || user_data.is_null() {
        return;
    }
    let Ok(gl) = glib() else { return };
    let Ok(pgl) = portal_glib() else { return };

    let slot_arc = unsafe { &*(user_data as *const Mutex<PortalResponseSlot>) };
    let path_str = unsafe { CStr::from_ptr(obj_path) }.to_string_lossy();
    let mut guard = slot_arc.lock().unwrap_or_else(|p| p.into_inner());
    let matches_expected = path_str == guard.expected_path;
    let matches_returned = guard
        .returned_path
        .as_deref()
        .is_some_and(|rp| rp == path_str.as_ref());
    if !matches_expected && !matches_returned {
        return;
    }

    let n = unsafe { (gl.g_variant_n_children)(parameters) };
    if n < 2 {
        return;
    }
    let code_v = unsafe { (gl.g_variant_get_child_value)(parameters, 0) };
    let results_v = unsafe { (gl.g_variant_get_child_value)(parameters, 1) };
    let code = if !code_v.is_null() {
        unsafe { (pgl.g_variant_get_uint32)(code_v) }
    } else {
        2
    };
    if !code_v.is_null() {
        unsafe { (gl.g_variant_unref)(code_v) };
    }

    if !results_v.is_null() {
        let sh_v = unsafe {
            (pgl.g_variant_lookup_value)(
                results_v,
                c"session_handle".as_ptr(),
                (gl.g_variant_type_checked_)(c"s".as_ptr()),
            )
        };
        if !sh_v.is_null() {
            let mut len = 0usize;
            let sp = unsafe { (gl.g_variant_get_string)(sh_v, &mut len) };
            if !sp.is_null() {
                guard.session_handle = Some(
                    unsafe { CStr::from_ptr(sp) }
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            unsafe { (gl.g_variant_unref)(sh_v) };
        }

        let tok_v = unsafe {
            (pgl.g_variant_lookup_value)(
                results_v,
                c"restore_token".as_ptr(),
                (gl.g_variant_type_checked_)(c"s".as_ptr()),
            )
        };
        if !tok_v.is_null() {
            let mut len = 0usize;
            let sp = unsafe { (gl.g_variant_get_string)(tok_v, &mut len) };
            if !sp.is_null() {
                let s = unsafe { CStr::from_ptr(sp) }
                    .to_string_lossy()
                    .into_owned();
                if !s.is_empty() {
                    guard.restore_token = Some(s);
                }
            }
            unsafe { (gl.g_variant_unref)(tok_v) };
        }

        let streams_v = unsafe {
            (pgl.g_variant_lookup_value)(
                results_v,
                c"streams".as_ptr(),
                (gl.g_variant_type_checked_)(c"a(ua{sv})".as_ptr()),
            )
        };
        if !streams_v.is_null() {
            let sn = unsafe { (gl.g_variant_n_children)(streams_v) };
            if sn > 0 {
                let st0 = unsafe { (gl.g_variant_get_child_value)(streams_v, 0) };
                if !st0.is_null() {
                    let nid_v = unsafe { (gl.g_variant_get_child_value)(st0, 0) };
                    if !nid_v.is_null() {
                        guard.node_id = Some(unsafe { (pgl.g_variant_get_uint32)(nid_v) });
                        unsafe { (gl.g_variant_unref)(nid_v) };
                    }
                    unsafe { (gl.g_variant_unref)(st0) };
                }
            }
            unsafe { (gl.g_variant_unref)(streams_v) };
        }
        unsafe { (gl.g_variant_unref)(results_v) };
    }

    guard.response_code = code;
    guard.done = true;
}

unsafe fn build_vardict(entries: &[(&str, *mut c_void)]) -> Option<*mut c_void> {
    let gl = glib().ok()?;
    let pgl = portal_glib().ok()?;
    let tp = unsafe { (gl.g_variant_type_checked_)(c"a{sv}".as_ptr()) };
    let b = unsafe { (pgl.g_variant_builder_new)(tp) };
    if b.is_null() {
        return None;
    }
    for &(k, val) in entries {
        let kc = cstr(k);
        let k_var = unsafe { (gl.g_variant_new_string)(kc.as_ptr()) };
        let wrapped = unsafe { (pgl.g_variant_new_variant)(val) };
        let de = unsafe { (pgl.g_variant_new_dict_entry)(k_var, wrapped) };
        unsafe { (pgl.g_variant_builder_add_value)(b, de) };
    }
    let out = unsafe { (pgl.g_variant_builder_end)(b) };
    unsafe { (pgl.g_variant_builder_unref)(b) };
    Some(out)
}

unsafe fn portal_call_and_wait(
    conn: *mut c_void,
    ctx: *mut c_void,
    sender_sanitized: &str,
    handle_token: &str,
    method: &CStr,
    params: *mut c_void,
    stop: &AtomicBool,
    timeout_ms: u64,
) -> Result<PortalResponseSlot, String> {
    let gl = glib().map_err(str::to_string)?;
    let pgl = portal_glib().map_err(str::to_string)?;
    let pgio = portal_gio().map_err(str::to_string)?;

    let expected_path = format!("/org/freedesktop/portal/desktop/request/{sender_sanitized}/{handle_token}");
    let slot = Box::new(Mutex::new(PortalResponseSlot {
        expected_path,
        returned_path: None,
        done: false,
        response_code: 2,
        session_handle: None,
        restore_token: None,
        node_id: None,
    }));
    let slot_ptr = Box::into_raw(slot);

    let sub_id = unsafe {
        (pgio.g_dbus_connection_signal_subscribe)(
            conn,
            PORTAL_DEST.as_ptr(),
            PORTAL_REQ_IFACE.as_ptr(),
            c"Response".as_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            Some(on_portal_request_response),
            slot_ptr.cast::<c_void>(),
            None,
        )
    };

    let mut gerr: *mut c_void = ptr::null_mut();
    let reply = unsafe {
        (pgio.g_dbus_connection_call_sync)(
            conn,
            PORTAL_DEST.as_ptr(),
            PORTAL_PATH.as_ptr(),
            PORTAL_SC_IFACE.as_ptr(),
            method.as_ptr(),
            params,
            (gl.g_variant_type_checked_)(c"(o)".as_ptr()),
            0,
            15_000,
            ptr::null_mut(),
            &mut gerr,
        )
    };
    if !gerr.is_null() {
        unsafe {
            (gl.g_error_free)(gerr);
            (pgio.g_dbus_connection_signal_unsubscribe)(conn, sub_id);
            drop(Box::from_raw(slot_ptr));
        }
        return Err(format!("D-Bus call {} failed", method.to_string_lossy()));
    }
    if !reply.is_null() {
        let req_v = unsafe { (gl.g_variant_get_child_value)(reply, 0) };
        if !req_v.is_null() {
            let mut len = 0usize;
            let sp = unsafe { (gl.g_variant_get_string)(req_v, &mut len) };
            if !sp.is_null() {
                let rp = unsafe { CStr::from_ptr(sp) }.to_string_lossy().into_owned();
                let mut g = unsafe { &*slot_ptr }.lock().unwrap_or_else(|p| p.into_inner());
                g.returned_path = Some(rp);
            }
            unsafe { (gl.g_variant_unref)(req_v) };
        }
        unsafe { (gl.g_variant_unref)(reply) };
    }

    let deadline_ns = mono_ns().saturating_add(timeout_ms * 1_000_000);
    while !stop.load(Ordering::Acquire) {
        unsafe {
            while (pgl.g_main_context_iteration)(ctx, 0) != 0 {}
        }
        {
            let g = unsafe { &*slot_ptr }.lock().unwrap_or_else(|p| p.into_inner());
            if g.done {
                break;
            }
        }
        if mono_ns() >= deadline_ns {
            unsafe {
                (pgio.g_dbus_connection_signal_unsubscribe)(conn, sub_id);
                drop(Box::from_raw(slot_ptr));
            }
            return Err(format!("{} timed out", method.to_string_lossy()));
        }
        thread::sleep(Duration::from_millis(12));
    }

    unsafe { (pgio.g_dbus_connection_signal_unsubscribe)(conn, sub_id) };
    let final_slot = unsafe { Box::from_raw(slot_ptr) }
        .into_inner()
        .unwrap_or_else(|p| p.into_inner());
    if stop.load(Ordering::Acquire) && !final_slot.done {
        return Err("stopped".to_string());
    }
    Ok(final_slot)
}

fn open_portal_screencast(
    stop: &AtomicBool,
    status: &Arc<Mutex<PortalPreviewStatus>>,
    allow_restore_token: bool,
) -> Result<(OwnedFd, u32, String, *mut c_void), (u32, String)> {
    let gl = glib().map_err(|e| (2, e.to_string()))?;
    let pgl = portal_glib().map_err(|e| (2, e.to_string()))?;
    let pgio = portal_gio().map_err(|e| (2, e.to_string()))?;

    let mut gerr: *mut c_void = ptr::null_mut();
    let conn = unsafe { (pgio.g_bus_get_sync)(G_BUS_TYPE_SESSION, ptr::null_mut(), &mut gerr) };
    if conn.is_null() {
        if !gerr.is_null() {
            unsafe { (gl.g_error_free)(gerr) };
        }
        return Err((2, "failed to connect to session D-Bus".to_string()));
    }
    let go = gobj().map_err(|e| (2, e.to_string()))?;

    let uname_ptr = unsafe { (pgio.g_dbus_connection_get_unique_name)(conn) };
    if uname_ptr.is_null() {
        unsafe { (go.g_object_unref)(conn) };
        return Err((2, "missing D-Bus unique name".to_string()));
    }
    let uname = unsafe { CStr::from_ptr(uname_ptr) }.to_string_lossy();
    let sender_sanitized = uname.trim_start_matches(':').replace('.', "_");
    let ctx = unsafe { (gl.g_main_context_new)() };
    if ctx.is_null() {
        unsafe { (go.g_object_unref)(conn) };
        return Err((2, "g_main_context_new failed".to_string()));
    }
    unsafe { (gl.g_main_context_push_thread_default)(ctx) };

    let seq = (mono_ns() & 0xffff_ffff) as u32;
    let pid = std::process::id();
    let cs_tok = format!("salsa_cs_{pid}_{seq}");
    let sess_tok = format!("salsa_sess_{pid}_{seq}");

    let cs_opts = unsafe {
        let h_cs = cstr(&cs_tok);
        let s_cs = cstr(&sess_tok);
        build_vardict(&[
            ("handle_token", (gl.g_variant_new_string)(h_cs.as_ptr())),
            ("session_handle_token", (gl.g_variant_new_string)(s_cs.as_ptr())),
        ])
    };
    let Some(cs_opts_v) = cs_opts else {
        unsafe {
            (gl.g_main_context_pop_thread_default)(ctx);
            (gl.g_main_context_unref)(ctx);
            (go.g_object_unref)(conn);
        }
        return Err((2, "CreateSession options build failed".to_string()));
    };
    let cs_params = unsafe { (gl.g_variant_new_tuple)(&cs_opts_v, 1) };
    let cs_res = unsafe {
        portal_call_and_wait(
            conn,
            ctx,
            &sender_sanitized,
            &cs_tok,
            c"CreateSession",
            cs_params,
            stop,
            10_000,
        )
    };
    let cs_slot = match cs_res {
        Ok(s) if s.response_code == 0 => s,
        Ok(s) => {
            unsafe {
                (gl.g_main_context_pop_thread_default)(ctx);
                (gl.g_main_context_unref)(ctx);
                (go.g_object_unref)(conn);
            }
            return Err((s.response_code, "ScreenCast CreateSession denied".to_string()));
        }
        Err(e) => {
            unsafe {
                (gl.g_main_context_pop_thread_default)(ctx);
                (gl.g_main_context_unref)(ctx);
                (go.g_object_unref)(conn);
            }
            return Err((2, e));
        }
    };
    let session_handle = cs_slot.session_handle.unwrap_or_else(|| {
        format!("/org/freedesktop/portal/desktop/session/{sender_sanitized}/{sess_tok}")
    });

    let close_session = |c: *mut c_void, sh: &str| {
        let sh_c = cstr(sh);
        let mut e2: *mut c_void = ptr::null_mut();
        let res = unsafe {
            (pgio.g_dbus_connection_call_sync)(
                c,
                PORTAL_DEST.as_ptr(),
                sh_c.as_ptr(),
                PORTAL_SESS_IFACE.as_ptr(),
                c"Close".as_ptr(),
                ptr::null_mut(),
                ptr::null(),
                0,
                2_000,
                ptr::null_mut(),
                &mut e2,
            )
        };
        if !e2.is_null() {
            unsafe { (gl.g_error_free)(e2) };
        }
        if !res.is_null() {
            unsafe { (gl.g_variant_unref)(res) };
        }
    };

    let saved_tok = if allow_restore_token {
        load_restore_token()
    } else {
        None
    };

    {
        let mut st = status.lock().unwrap_or_else(|p| p.into_inner());
        st.state = "authorizing";
        st.has_token = saved_tok.is_some();
        st.message = if saved_tok.is_some() {
            "Restoring Wayland ScreenCast preview session...".to_string()
        } else {
            "Choose a monitor or window in the Wayland ScreenCast dialog...".to_string()
        };
    }

    let sel_tok = format!("salsa_sel_{pid}_{seq}");
    let sel_opts_v = unsafe {
        let h_sel = cstr(&sel_tok);
        let mut items = vec![
            ("handle_token", (gl.g_variant_new_string)(h_sel.as_ptr())),
            ("types", (gl.g_variant_new_uint32)(3)), // Monitor | Window
            ("multiple", (gl.g_variant_new_boolean)(0)),
            ("cursor_mode", (gl.g_variant_new_uint32)(2)), // Embedded
            ("persist_mode", (gl.g_variant_new_uint32)(2)), // Persistent until revoked
        ];
        if let Some(ref rt) = saved_tok {
            let rt_c = cstr(rt);
            items.push(("restore_token", (gl.g_variant_new_string)(rt_c.as_ptr())));
        }
        build_vardict(&items)
    };
    let Some(sel_opts) = sel_opts_v else {
        close_session(conn, &session_handle);
        unsafe {
            (gl.g_main_context_pop_thread_default)(ctx);
            (gl.g_main_context_unref)(ctx);
            (go.g_object_unref)(conn);
        }
        return Err((2, "SelectSources options build failed".to_string()));
    };
    let sh_c = cstr(&session_handle);
    let sel_args = [unsafe { (pgl.g_variant_new_object_path)(sh_c.as_ptr()) }, sel_opts];
    let sel_params = unsafe { (gl.g_variant_new_tuple)(sel_args.as_ptr(), 2) };
    let sel_slot = unsafe {
        portal_call_and_wait(
            conn,
            ctx,
            &sender_sanitized,
            &sel_tok,
            c"SelectSources",
            sel_params,
            stop,
            10_000,
        )
    };
    match sel_slot {
        Ok(s) if s.response_code == 0 => {}
        Ok(s) => {
            close_session(conn, &session_handle);
            unsafe {
                (gl.g_main_context_pop_thread_default)(ctx);
                (gl.g_main_context_unref)(ctx);
                (go.g_object_unref)(conn);
            }
            return Err((s.response_code, "SelectSources rejected".to_string()));
        }
        Err(e) => {
            close_session(conn, &session_handle);
            unsafe {
                (gl.g_main_context_pop_thread_default)(ctx);
                (gl.g_main_context_unref)(ctx);
                (go.g_object_unref)(conn);
            }
            return Err((2, e));
        }
    }

    let start_tok = format!("salsa_start_{pid}_{seq}");
    let start_opts_v = unsafe {
        let h_st = cstr(&start_tok);
        build_vardict(&[("handle_token", (gl.g_variant_new_string)(h_st.as_ptr()))])
    };
    let Some(start_opts) = start_opts_v else {
        close_session(conn, &session_handle);
        unsafe {
            (gl.g_main_context_pop_thread_default)(ctx);
            (gl.g_main_context_unref)(ctx);
            (go.g_object_unref)(conn);
        }
        return Err((2, "Start options build failed".to_string()));
    };
    let start_args = [
        unsafe { (pgl.g_variant_new_object_path)(sh_c.as_ptr()) },
        unsafe { (gl.g_variant_new_string)(c"".as_ptr()) },
        start_opts,
    ];
    let start_params = unsafe { (gl.g_variant_new_tuple)(start_args.as_ptr(), 3) };
    let start_slot = match unsafe {
        portal_call_and_wait(
            conn,
            ctx,
            &sender_sanitized,
            &start_tok,
            c"Start",
            start_params,
            stop,
            120_000,
        )
    } {
        Ok(s) => s,
        Err(e) => {
            close_session(conn, &session_handle);
            unsafe {
                (gl.g_main_context_pop_thread_default)(ctx);
                (gl.g_main_context_unref)(ctx);
                (go.g_object_unref)(conn);
            }
            return Err((2, e));
        }
    };

    if start_slot.response_code != 0 {
        close_session(conn, &session_handle);
        unsafe {
            (gl.g_main_context_pop_thread_default)(ctx);
            (gl.g_main_context_unref)(ctx);
            (go.g_object_unref)(conn);
        }
        let msg = if start_slot.response_code == 1 {
            "Wayland ScreenCast permission cancelled by user".to_string()
        } else {
            format!("Wayland ScreenCast Start failed (code={})", start_slot.response_code)
        };
        return Err((start_slot.response_code, msg));
    }

    if let Some(ref tok) = start_slot.restore_token {
        save_restore_token(tok);
    }
    let Some(node_id) = start_slot.node_id else {
        close_session(conn, &session_handle);
        unsafe {
            (gl.g_main_context_pop_thread_default)(ctx);
            (gl.g_main_context_unref)(ctx);
            (go.g_object_unref)(conn);
        }
        return Err((2, "Portal Start returned no video stream node".to_string()));
    };

    let empty_opts = unsafe { build_vardict(&[]) }.ok_or_else(|| {
        close_session(conn, &session_handle);
        unsafe {
            (gl.g_main_context_pop_thread_default)(ctx);
            (gl.g_main_context_unref)(ctx);
            (go.g_object_unref)(conn);
        }
        (2, "OpenPipeWireRemote options failed".to_string())
    })?;
    let open_args = [
        unsafe { (pgl.g_variant_new_object_path)(sh_c.as_ptr()) },
        empty_opts,
    ];
    let open_params = unsafe { (gl.g_variant_new_tuple)(open_args.as_ptr(), 2) };
    let mut out_fd_list: *mut c_void = ptr::null_mut();
    let mut gerr2: *mut c_void = ptr::null_mut();
    let open_reply = unsafe {
        (pgio.g_dbus_connection_call_with_unix_fd_list_sync)(
            conn,
            PORTAL_DEST.as_ptr(),
            PORTAL_PATH.as_ptr(),
            PORTAL_SC_IFACE.as_ptr(),
            c"OpenPipeWireRemote".as_ptr(),
            open_params,
            (gl.g_variant_type_checked_)(c"(h)".as_ptr()),
            0,
            10_000,
            ptr::null_mut(),
            &mut out_fd_list,
            ptr::null_mut(),
            &mut gerr2,
        )
    };
    if !gerr2.is_null() || open_reply.is_null() || out_fd_list.is_null() {
        if !gerr2.is_null() {
            unsafe { (gl.g_error_free)(gerr2) };
        }
        if !open_reply.is_null() {
            unsafe { (gl.g_variant_unref)(open_reply) };
        }
        if !out_fd_list.is_null() {
            unsafe { (go.g_object_unref)(out_fd_list) };
        }
        close_session(conn, &session_handle);
        unsafe {
            (gl.g_main_context_pop_thread_default)(ctx);
            (gl.g_main_context_unref)(ctx);
            (go.g_object_unref)(conn);
        }
        return Err((2, "OpenPipeWireRemote failed".to_string()));
    }

    let h_v = unsafe { (gl.g_variant_get_child_value)(open_reply, 0) };
    let fd_idx = if !h_v.is_null() {
        let idx = unsafe { (pgl.g_variant_get_handle)(h_v) };
        unsafe { (gl.g_variant_unref)(h_v) };
        idx
    } else {
        -1
    };
    unsafe { (gl.g_variant_unref)(open_reply) };

    let mut fd_err: *mut c_void = ptr::null_mut();
    let raw_fd = if fd_idx >= 0 {
        unsafe { (pgio.g_unix_fd_list_get)(out_fd_list, fd_idx, &mut fd_err) }
    } else {
        -1
    };
    if !fd_err.is_null() {
        unsafe { (gl.g_error_free)(fd_err) };
    }
    unsafe { (go.g_object_unref)(out_fd_list) };
    unsafe {
        (gl.g_main_context_pop_thread_default)(ctx);
        (gl.g_main_context_unref)(ctx);
    }

    if raw_fd < 0 {
        close_session(conn, &session_handle);
        unsafe { (go.g_object_unref)(conn) };
        return Err((2, "OpenPipeWireRemote returned invalid FD".to_string()));
    }

    let owned = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    Ok((owned, node_id, session_handle, conn))
}

fn close_portal_session_dbus(conn: *mut c_void, session_handle: &str) {
    if conn.is_null() || session_handle.is_empty() {
        return;
    }
    let Ok(gl) = glib() else { return };
    let Ok(pgio) = portal_gio() else { return };
    let Ok(go) = gobj() else { return };

    let sh_c = cstr(session_handle);
    let mut gerr: *mut c_void = ptr::null_mut();
    let res = unsafe {
        (pgio.g_dbus_connection_call_sync)(
            conn,
            PORTAL_DEST.as_ptr(),
            sh_c.as_ptr(),
            PORTAL_SESS_IFACE.as_ptr(),
            c"Close".as_ptr(),
            ptr::null_mut(),
            ptr::null(),
            0,
            2_000,
            ptr::null_mut(),
            &mut gerr,
        )
    };
    if !gerr.is_null() {
        unsafe { (gl.g_error_free)(gerr) };
    }
    if !res.is_null() {
        unsafe { (gl.g_variant_unref)(res) };
    }
    unsafe { (go.g_object_unref)(conn) };
}

fn build_consumer_enum_format_pod(vfmt: u32) -> Vec<u8> {
    let mut b = PodBuilder::new(256);
    b.push_object(OBJ_FORMAT, PARAM_ENUM_FORMAT);
    b.prop(1, 0);
    b.id(MEDIA_VIDEO);
    b.prop(2, 0);
    b.id(SUB_RAW);
    b.prop(0x20001, 0);
    b.id(vfmt);

    b.prop(0x20003, 0);
    b.push_choice(CHOICE_RANGE, 0, 8, SPA_RECT);
    b.write_u64_raw((1080u64 << 32) | 1920u64);
    b.write_u64_raw((16u64 << 32) | 16u64);
    b.write_u64_raw((4320u64 << 32) | 7680u64);
    b.pop();

    b.prop(0x20004, 0);
    b.push_choice(CHOICE_RANGE, 0, 8, SPA_FRAC);
    b.write_u64_raw((1u64 << 32) | 60u64);
    b.write_u64_raw((1u64 << 32) | 1u64);
    b.write_u64_raw((1u64 << 32) | 240u64);
    b.pop();

    b.pop();
    b.into_vec()
}

fn build_consumer_buffers_pod(datatype_mask: i32) -> Vec<u8> {
    let mut b = PodBuilder::new(128);
    b.push_object(OBJ_PARAM_BUFS, PARAM_BUFFERS);
    b.prop(1, 0);
    b.int(4);
    b.prop(6, 0);
    b.int(datatype_mask);
    b.pop();
    b.into_vec()
}

fn run_pw_video_consumer(
    pw_fd: OwnedFd,
    target_node_id: u32,
    stop: &AtomicBool,
    status: &Arc<Mutex<PortalPreviewStatus>>,
) -> Result<(), String> {
    const NODE_PROXY_ID: u32 = 3;
    let raw_pw = pw_fd.as_raw_fd();
    let mut seq = 0u32;
    let mut next_seq = || {
        let s = seq;
        seq = seq.wrapping_add(1);
        s
    };

    // 1. Hello
    let mut bh = PodBuilder::new(64);
    bh.push_struct();
    bh.int(4);
    bh.pop();
    sendmsg_all_with_fds(
        raw_pw,
        &frame_pw_msg(0, PW_CORE_M_HELLO, next_seq(), 0, &bh.into_vec()),
        &[],
    )
    .map_err(|_| "PipeWire HELLO send failed".to_string())?;

    // 2. Create input ClientNode targeting portal stream node
    let mut props = Dict::default();
    props.set_str("media.type", "Video");
    props.set_str("media.category", "Capture");
    props.set_str("media.role", "Screen");
    props.set_str("media.class", "Stream/Input/Video");
    props.set_str("node.name", "salsa-live-preview");
    props.set_str("node.autoconnect", "true");
    props.set_str("target.object", &target_node_id.to_string());

    let mut bc = PodBuilder::new(512);
    bc.push_struct();
    bc.str_val(Some("client-node"));
    bc.str_val(Some("PipeWire:Interface:ClientNode"));
    bc.int(6);
    bc.dict(&props);
    bc.int(NODE_PROXY_ID as i32);
    bc.pop();
    sendmsg_all_with_fds(
        raw_pw,
        &frame_pw_msg(0, PW_CORE_M_CREATE_OBJ, next_seq(), 0, &bc.into_vec()),
        &[],
    )
    .map_err(|_| "PipeWire CreateObject ClientNode failed".to_string())?;

    // 3. Node Update (max_in=1, max_out=0)
    let mut bu = PodBuilder::new(128);
    bu.push_struct();
    bu.int(3);
    bu.int(0);
    bu.push_struct();
    bu.int(1);
    bu.int(0);
    bu.long(0);
    bu.long(0);
    bu.pop();
    bu.pop();
    sendmsg_all_with_fds(
        raw_pw,
        &frame_pw_msg(NODE_PROXY_ID, PW_CN_M_UPDATE, next_seq(), 0, &bu.into_vec()),
        &[],
    )
    .map_err(|_| "PipeWire ClientNode Update failed".to_string())?;

    // 4. PortUpdate advertising linear MemFd raw video formats (falls back cleanly through pw_broker)
    let fmts = [
        VF_BGRX, VF_BGRA, VF_RGBX, VF_RGBA, VF_XRGB, VF_XBGR, VF_ARGB, VF_ABGR,
    ];
    let buf_pod = build_consumer_buffers_pod(1 << DATA_MEM_FD);
    let mut pu = PodBuilder::new(2048);
    pu.push_struct();
    pu.int(0); // PWB_DIR_IN
    pu.int(0); // port_id
    pu.int(1); // change_mask (params)
    pu.int((fmts.len() + 1) as i32);
    for fmt in fmts {
        pu.raw_pod(&build_consumer_enum_format_pod(fmt));
    }
    pu.raw_pod(&buf_pod);
    pu.push_struct();
    pu.long(0);
    pu.long(0);
    pu.int(0);
    pu.int(0);
    pu.pop();
    pu.pop();
    sendmsg_all_with_fds(
        raw_pw,
        &frame_pw_msg(NODE_PROXY_ID, PW_CN_M_PORT_UPDATE, next_seq(), 0, &pu.into_vec()),
        &[],
    )
    .map_err(|_| "PipeWire PortUpdate failed".to_string())?;

    // 5. Activate
    let mut ba = PodBuilder::new(64);
    ba.push_struct();
    ba.int(1);
    ba.pop();
    sendmsg_all_with_fds(
        raw_pw,
        &frame_pw_msg(NODE_PROXY_ID, PW_CN_M_SET_ACTIVE, next_seq(), 0, &ba.into_vec()),
        &[],
    )
    .map_err(|_| "PipeWire SetActive failed".to_string())?;

    let mut rx_buf: Vec<u8> = Vec::with_capacity(65536);
    let mut rx_fds: Vec<OwnedFd> = Vec::new();
    let mut mems: HashMap<u32, OwnedFd> = HashMap::new();
    let mut act_readfd: Option<OwnedFd> = None;
    let mut act_writefd: Option<OwnedFd> = None;
    let mut done_efd: Option<OwnedFd> = None;
    let mut io_map: Option<MappedMem> = None;

    while !stop.load(Ordering::Acquire) {
        let mut pfds = [
            PollFd {
                fd: raw_pw,
                events: POLLIN,
                revents: 0,
            },
            PollFd {
                fd: act_readfd.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                events: POLLIN,
                revents: 0,
            },
        ];
        let npfds = if act_readfd.is_some() { 2 } else { 1 };
        let pr = do_poll(&mut pfds[..npfds], 25);
        if pr < 0 {
            break;
        }

        if pfds[0].revents != 0 {
            let mut tmp = [0u8; 65536];
            loop {
                match recvmsg_with_fds(raw_pw, &mut tmp) {
                    Ok(RecvFdsResult::Data { bytes, fds }) => {
                        rx_buf.extend_from_slice(&tmp[..bytes]);
                        rx_fds.extend(fds);
                    }
                    Ok(RecvFdsResult::WouldBlock) => break,
                    Ok(RecvFdsResult::Eof) | Err(()) => return Ok(()),
                }
            }

            let mut consumed = 0usize;
            loop {
                let rem = &rx_buf[consumed..];
                if rem.len() < 16 {
                    break;
                }
                let hdr = match parse_pkt_hdr_status(rem) {
                    PktHdrStatus::Ready(h) => h,
                    PktHdrStatus::Incomplete => break,
                    PktHdrStatus::Malformed => return Err("malformed PipeWire packet".to_string()),
                };
                if rem.len() < hdr.pkt_len || (rx_fds.len() as u32) < hdr.nfds {
                    break;
                }
                let mut local_fds: Vec<Option<OwnedFd>> =
                    rx_fds.drain(..hdr.nfds as usize).map(Some).collect();
                let pay = &rem[16..16 + hdr.psz as usize];
                if let Some(mut cur) = PodCursor::new(pay, 0, pay.len()) {
                    if let Some(mut st) = cur.enter_struct_cursor() {
                        match (hdr.oid, hdr.op) {
                            (0, PW_CORE_E_ADD_MEM) => {
                                if let (Some(mid), Some(_tp), Some(fidx)) =
                                    (st.read_u32(), st.read_u32(), st.read_long())
                                {
                                    if fidx >= 0 {
                                        if let Some(slot) = local_fds.get_mut(fidx as usize) {
                                            if let Some(fd) = slot.take() {
                                                mems.insert(mid, fd);
                                            }
                                        }
                                    }
                                }
                            }
                            (0, PW_CORE_E_REMOVE_MEM) => {
                                if let Some(mid) = st.read_u32() {
                                    mems.remove(&mid);
                                }
                            }
                            (0, PW_CORE_E_ERROR) => {
                                let _obj = st.read_u32();
                                let _seq = st.read_u32();
                                let err_code = st.read_int().unwrap_or(0);
                                let msg = st
                                    .read_str_bytes()
                                    .map(|b| String::from_utf8_lossy(b).into_owned())
                                    .unwrap_or_default();
                                if err_code != 0 {
                                    return Err(format!("PipeWire error {err_code}: {msg}"));
                                }
                            }
                            (NODE_PROXY_ID, PW_CN_E_TRANSPORT) => {
                                if let (Some(ri), Some(wi)) = (st.read_long(), st.read_long()) {
                                    if ri >= 0 {
                                        if let Some(s) = local_fds.get_mut(ri as usize) {
                                            act_readfd = s.take();
                                        }
                                    }
                                    if wi >= 0 {
                                        if let Some(s) = local_fds.get_mut(wi as usize) {
                                            act_writefd = s.take();
                                        }
                                    }
                                }
                            }
                            (NODE_PROXY_ID, PW_CN_E_PORT_SET_IO) => {
                                if let (
                                    Some(_dir),
                                    Some(_pid),
                                    Some(_mix),
                                    Some(io_id),
                                    Some(mid),
                                    Some(off),
                                    Some(sz),
                                ) = (
                                    st.read_u32(),
                                    st.read_u32(),
                                    st.read_u32(),
                                    st.read_u32(),
                                    st.read_u32(),
                                    st.read_u32(),
                                    st.read_u32(),
                                ) {
                                    if io_id == IO_BUFFERS {
                                        if mid == SPA_ID_INVALID || sz == 0 {
                                            io_map = None;
                                        } else if let Some(fd) = mems.get(&mid) {
                                            io_map = MappedMem::map_fd(
                                                fd.as_raw_fd(),
                                                sz as usize,
                                                true,
                                                off as u64,
                                            );
                                        }
                                    }
                                }
                            }
                            (NODE_PROXY_ID, PW_CN_E_SET_ACTIVATION) => {
                                if let (Some(_nid), Some(sidx)) = (st.read_u32(), st.read_long()) {
                                    if sidx >= 0 {
                                        if let Some(s) = local_fds.get_mut(sidx as usize) {
                                            done_efd = s.take();
                                        }
                                    } else {
                                        done_efd = None;
                                    }
                                }
                            }
                            (NODE_PROXY_ID, PW_CN_E_COMMAND) => {
                                if let Some((esz, etp, ebody, _)) = st.next_pod() {
                                    if etp == SPA_OBJECT && esz >= 8 && ebody + 8 <= pay.len() {
                                        let cid = u32::from_le_bytes([
                                            pay[ebody + 4],
                                            pay[ebody + 5],
                                            pay[ebody + 6],
                                            pay[ebody + 7],
                                        ]);
                                        if cid == SPA_NODE_CMD_START {
                                            let mut guard =
                                                status.lock().unwrap_or_else(|p| p.into_inner());
                                            guard.state = "streaming";
                                            guard.message = format!(
                                                "Live Wayland ScreenCast stream active (node {target_node_id})"
                                            );
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                consumed += hdr.pkt_len;
            }
            if consumed > 0 {
                rx_buf.drain(..consumed);
            }
        }

        if npfds == 2 && pfds[1].revents != 0 {
            if let Some(ref ar) = act_readfd {
                eventfd_drain(ar.as_raw_fd());
            }
            if let Some(ref mut map) = io_map {
                let sl = map.as_mut_slice();
                if sl.len() >= 8 {
                    let st = u32::from_le_bytes([sl[0], sl[1], sl[2], sl[3]]);
                    let bid = u32::from_le_bytes([sl[4], sl[5], sl[6], sl[7]]);
                    if st == PWB_STATUS_HAVE_DATA && bid != SPA_ID_INVALID {
                        sl[4..8].copy_from_slice(&bid.to_le_bytes());
                        unsafe {
                            (&*sl.as_ptr().cast::<AtomicU32>())
                                .store(PWB_STATUS_NEED_DATA, Ordering::Release);
                        }
                        if let Some(ref de) = done_efd {
                            eventfd_signal(de.as_raw_fd());
                        } else if let Some(ref aw) = act_writefd {
                            eventfd_signal(aw.as_raw_fd());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub struct PortalScreenCastPreview {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<PortalPreviewStatus>>,
    join: Option<JoinHandle<()>>,
}

impl PortalScreenCastPreview {
    pub fn start(force_reselect: bool) -> Self {
        if force_reselect {
            clear_restore_token();
        }
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(PortalPreviewStatus {
            state: "authorizing",
            message: "Connecting to Wayland ScreenCast portal...".to_string(),
            node_id: 0,
            has_token: has_saved_restore_token(),
        }));
        let stop_t = Arc::clone(&stop);
        let status_t = Arc::clone(&status);

        let join = thread::Builder::new()
            .name("salsa-portal-sc".into())
            .spawn(move || {
                let first = open_portal_screencast(&stop_t, &status_t, !force_reselect);
                let opened = match first {
                    Ok(v) => Ok(v),
                    Err((code, msg)) => {
                        if code != 1 && !force_reselect && has_saved_restore_token() && !stop_t.load(Ordering::Acquire) {
                            clear_restore_token();
                            open_portal_screencast(&stop_t, &status_t, false)
                        } else {
                            Err((code, msg))
                        }
                    }
                };

                match opened {
                    Ok((pw_fd, node_id, sess_handle, conn)) => {
                        {
                            let mut st = status_t.lock().unwrap_or_else(|p| p.into_inner());
                            st.state = "connecting";
                            st.node_id = node_id;
                            st.has_token = has_saved_restore_token();
                            st.message = format!(
                                "Connecting ScreenCast node {node_id} via Salsa pipewire-0 broker..."
                            );
                        }
                        if let Err(e) = run_pw_video_consumer(pw_fd, node_id, &stop_t, &status_t) {
                            let mut st = status_t.lock().unwrap_or_else(|p| p.into_inner());
                            st.state = "error";
                            st.message = e;
                        } else {
                            let mut st = status_t.lock().unwrap_or_else(|p| p.into_inner());
                            if st.state == "streaming" || st.state == "connecting" {
                                st.state = "idle";
                                st.message = "Portal ScreenCast stopped.".to_string();
                            }
                        }
                        close_portal_session_dbus(conn, &sess_handle);
                    }
                    Err((code, msg)) => {
                        let mut st = status_t.lock().unwrap_or_else(|p| p.into_inner());
                        st.state = if code == 1 { "cancelled" } else { "error" };
                        st.has_token = has_saved_restore_token();
                        st.message = msg;
                    }
                }
            })
            .ok();

        Self { stop, status, join }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(jh) = self.join.take() {
            let _ = jh.join();
        }
    }

    pub fn is_running_or_pending(&self) -> bool {
        let st = self.status.lock().unwrap_or_else(|p| p.into_inner());
        matches!(st.state, "authorizing" | "connecting" | "streaming")
    }

    pub fn status(&self) -> PortalPreviewStatus {
        self.status.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for PortalScreenCastPreview {
    fn drop(&mut self) {
        self.stop();
    }
}
