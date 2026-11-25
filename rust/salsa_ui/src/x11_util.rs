// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use salsa_tools::host::{home_dir, read_text_lossy};
use salsa_wire::unix_sys::dl_open_any;

use crate::gio_dbus::{load_opt_sym, load_sym};

pub struct X11Api {
    pub x_open_display: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    pub x_close_display: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub x_default_screen: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub x_default_root_window: unsafe extern "C" fn(*mut c_void) -> c_ulong,
    pub x_display_width: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    pub x_display_height: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    pub x_intern_atom: unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_ulong,
    pub x_send_event:
        unsafe extern "C" fn(*mut c_void, c_ulong, c_int, c_long, *mut c_void) -> c_int,
    pub x_change_property: unsafe extern "C" fn(
        *mut c_void,
        c_ulong,
        c_ulong,
        c_ulong,
        c_int,
        c_int,
        *const c_void,
        c_int,
    ) -> c_int,
    pub x_move_window: unsafe extern "C" fn(*mut c_void, c_ulong, c_int, c_int) -> c_int,
    pub x_query_pointer: unsafe extern "C" fn(
        *mut c_void,
        c_ulong,
        *mut c_ulong,
        *mut c_ulong,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_uint,
    ) -> c_int,
    pub x_flush: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub x_set_error_handler: unsafe extern "C" fn(
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    ) -> *mut c_void,
    pub x_resource_manager_string: Option<unsafe extern "C" fn(*mut c_void) -> *const c_char>,
}

#[repr(C)]
struct XRRMonitorInfo {
    name: c_ulong,
    primary: c_int,
    automatic: c_int,
    noutput: c_int,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    mwidth: c_int,
    mheight: c_int,
    outputs: *mut c_ulong,
}

struct XrandrApi {
    xrr_get_monitors: unsafe extern "C" fn(
        *mut c_void,
        c_ulong,
        c_int,
        *mut c_int,
    ) -> *mut XRRMonitorInfo,
    xrr_free_monitors: unsafe extern "C" fn(*mut XRRMonitorInfo),
}

static X11: OnceLock<Option<X11Api>> = OnceLock::new();
static XRANDR: OnceLock<Option<XrandrApi>> = OnceLock::new();
static X11_ERR_HANDLER_SET: AtomicBool = AtomicBool::new(false);

pub fn x11() -> Option<&'static X11Api> {
    X11.get_or_init(|| unsafe {
        let lib = dl_open_any(&["libX11.so.6", "libX11.so"]);
        if lib.is_null() {
            return None;
        }
        Some(X11Api {
            x_open_display: load_sym(lib, c"XOpenDisplay").ok()?,
            x_close_display: load_sym(lib, c"XCloseDisplay").ok()?,
            x_default_screen: load_sym(lib, c"XDefaultScreen").ok()?,
            x_default_root_window: load_sym(lib, c"XDefaultRootWindow").ok()?,
            x_display_width: load_sym(lib, c"XDisplayWidth").ok()?,
            x_display_height: load_sym(lib, c"XDisplayHeight").ok()?,
            x_intern_atom: load_sym(lib, c"XInternAtom").ok()?,
            x_send_event: load_sym(lib, c"XSendEvent").ok()?,
            x_change_property: load_sym(lib, c"XChangeProperty").ok()?,
            x_move_window: load_sym(lib, c"XMoveWindow").ok()?,
            x_query_pointer: load_sym(lib, c"XQueryPointer").ok()?,
            x_flush: load_sym(lib, c"XFlush").ok()?,
            x_set_error_handler: load_sym(lib, c"XSetErrorHandler").ok()?,
            x_resource_manager_string: load_opt_sym(lib, c"XResourceManagerString"),
        })
    })
    .as_ref()
}

fn xrandr() -> Option<&'static XrandrApi> {
    XRANDR
        .get_or_init(|| unsafe {
            let lib = dl_open_any(&["libXrandr.so.2", "libXrandr.so"]);
            if lib.is_null() {
                return None;
            }
            Some(XrandrApi {
                xrr_get_monitors: load_sym(lib, c"XRRGetMonitors").ok()?,
                xrr_free_monitors: load_sym(lib, c"XRRFreeMonitors").ok()?,
            })
        })
        .as_ref()
}

#[repr(C)]
#[derive(Clone, Copy)]
union XClientMessageData {
    l: [c_long; 5],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct XClientMessageEvent {
    ev_type: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    window: c_ulong,
    message_type: c_ulong,
    format: c_int,
    data: XClientMessageData,
}

#[repr(C)]
union XEventPad {
    client: XClientMessageEvent,
    pad: [c_long; 24],
}

unsafe extern "C" fn silent_x11_error_handler(_dpy: *mut c_void, _err: *mut c_void) -> c_int {
    0
}

unsafe fn ensure_x11_error_handler(x: &X11Api) {
    if !X11_ERR_HANDLER_SET.swap(true, Ordering::AcqRel) {
        unsafe { (x.x_set_error_handler)(Some(silent_x11_error_handler)) };
    }
}

unsafe fn open_x11_display(x: &X11Api) -> Option<*mut c_void> {
    unsafe { ensure_x11_error_handler(x) };
    let dpy = unsafe { (x.x_open_display)(ptr::null()) };
    if dpy.is_null() {
        None
    } else {
        Some(dpy)
    }
}

unsafe fn send_net_wm_state_msg(
    x: &X11Api,
    dpy: *mut c_void,
    root: c_ulong,
    xid: c_ulong,
    net_wm_state: c_ulong,
    atom1: c_ulong,
    atom2: c_ulong,
) {
    let mut ev = XEventPad {
        client: XClientMessageEvent {
            ev_type: 33, // ClientMessage
            serial: 0,
            send_event: 0,
            display: dpy,
            window: xid,
            message_type: net_wm_state,
            format: 32,
            data: XClientMessageData {
                l: [1, atom1 as c_long, atom2 as c_long, 1, 0],
            },
        },
    };
    let _ = mem::size_of::<XEventPad>();
    unsafe {
        (x.x_send_event)(
            dpy,
            root,
            0,
            (1 << 20) | (1 << 19),
            (&mut ev as *mut XEventPad).cast::<c_void>(),
        );
    }
}

pub fn apply_popup_hints(xid: u64, skip_taskbar: bool, above: bool, blur_rect: Option<(i32, i32)>) {
    if !window_uses_x11() || xid == 0 {
        return;
    }
    let Some(x) = x11() else {
        return;
    };
    unsafe {
        let Some(dpy) = open_x11_display(x) else {
            return;
        };
        let root = (x.x_default_root_window)(dpy);
        let net_wm_state = (x.x_intern_atom)(dpy, c"_NET_WM_STATE".as_ptr(), 0);
        let skip_tb = (x.x_intern_atom)(dpy, c"_NET_WM_STATE_SKIP_TASKBAR".as_ptr(), 0);
        let skip_pg = (x.x_intern_atom)(dpy, c"_NET_WM_STATE_SKIP_PAGER".as_ptr(), 0);
        let skip_sw = (x.x_intern_atom)(dpy, c"_KDE_NET_WM_STATE_SKIP_SWITCHER".as_ptr(), 0);
        let abv = (x.x_intern_atom)(dpy, c"_NET_WM_STATE_ABOVE".as_ptr(), 0);
        let atom_ty = (x.x_intern_atom)(dpy, c"ATOM".as_ptr(), 0);
        let card_ty = (x.x_intern_atom)(dpy, c"CARDINAL".as_ptr(), 0);

        let mut atoms: Vec<c_ulong> = Vec::new();
        if skip_taskbar {
            atoms.push(skip_tb);
            atoms.push(skip_pg);
            if skip_sw != 0 {
                atoms.push(skip_sw);
            }
        }
        if above {
            atoms.push(abv);
        }
        if !atoms.is_empty() && net_wm_state != 0 && atom_ty != 0 {
            (x.x_change_property)(
                dpy,
                xid as c_ulong,
                net_wm_state,
                atom_ty,
                32,
                0, // PropModeReplace
                atoms.as_ptr().cast::<c_void>(),
                atoms.len() as c_int,
            );
            if skip_taskbar {
                send_net_wm_state_msg(x, dpy, root, xid as c_ulong, net_wm_state, skip_tb, skip_pg);
                if skip_sw != 0 {
                    send_net_wm_state_msg(x, dpy, root, xid as c_ulong, net_wm_state, skip_sw, 0);
                }
            }
            if above {
                send_net_wm_state_msg(x, dpy, root, xid as c_ulong, net_wm_state, abv, 0);
            }
        }

        if let Some((bw, bh)) = blur_rect {
            if bw > 0 && bh > 0 && card_ty != 0 {
                let kde_blur = (x.x_intern_atom)(dpy, c"_KDE_NET_WM_BLUR_BEHIND_REGION".as_ptr(), 0);
                if kde_blur != 0 {
                    let region: [u32; 4] = [0, 0, bw as u32, bh as u32];
                    (x.x_change_property)(
                        dpy,
                        xid as c_ulong,
                        kde_blur,
                        card_ty,
                        32,
                        0,
                        region.as_ptr().cast::<c_void>(),
                        4,
                    );
                }
            }
        }
        (x.x_flush)(dpy);
        (x.x_close_display)(dpy);
    }
}

pub fn apply_osd_hints(xid: u64, w: i32, h: i32) {
    if !window_uses_x11() || xid == 0 {
        return;
    }
    let Some(x) = x11() else {
        return;
    };
    unsafe {
        let Some(dpy) = open_x11_display(x) else {
            return;
        };
        let root = (x.x_default_root_window)(dpy);
        let atom_ty = (x.x_intern_atom)(dpy, c"ATOM".as_ptr(), 0);
        let card_ty = (x.x_intern_atom)(dpy, c"CARDINAL".as_ptr(), 0);
        let win_ty = (x.x_intern_atom)(dpy, c"_NET_WM_WINDOW_TYPE".as_ptr(), 0);
        let osd_ty = (x.x_intern_atom)(dpy, c"_KDE_NET_WM_WINDOW_TYPE_ON_SCREEN_DISPLAY".as_ptr(), 0);
        let notif_ty = (x.x_intern_atom)(dpy, c"_NET_WM_WINDOW_TYPE_NOTIFICATION".as_ptr(), 0);
        if atom_ty != 0 && win_ty != 0 && osd_ty != 0 && notif_ty != 0 {
            let tys = [osd_ty, notif_ty];
            (x.x_change_property)(
                dpy,
                xid as c_ulong,
                win_ty,
                atom_ty,
                32,
                0,
                tys.as_ptr().cast::<c_void>(),
                2,
            );
        }

        let net_wm_state = (x.x_intern_atom)(dpy, c"_NET_WM_STATE".as_ptr(), 0);
        let skip_tb = (x.x_intern_atom)(dpy, c"_NET_WM_STATE_SKIP_TASKBAR".as_ptr(), 0);
        let skip_pg = (x.x_intern_atom)(dpy, c"_NET_WM_STATE_SKIP_PAGER".as_ptr(), 0);
        let abv = (x.x_intern_atom)(dpy, c"_NET_WM_STATE_ABOVE".as_ptr(), 0);
        if atom_ty != 0 && net_wm_state != 0 {
            let sts = [skip_tb, skip_pg, abv];
            (x.x_change_property)(
                dpy,
                xid as c_ulong,
                net_wm_state,
                atom_ty,
                32,
                0,
                sts.as_ptr().cast::<c_void>(),
                3,
            );
            send_net_wm_state_msg(x, dpy, root, xid as c_ulong, net_wm_state, skip_tb, skip_pg);
            send_net_wm_state_msg(x, dpy, root, xid as c_ulong, net_wm_state, abv, 0);
        }

        if card_ty != 0 {
            let user_time = (x.x_intern_atom)(dpy, c"_NET_WM_USER_TIME".as_ptr(), 0);
            if user_time != 0 {
                let zero: [u32; 1] = [0];
                (x.x_change_property)(
                    dpy,
                    xid as c_ulong,
                    user_time,
                    card_ty,
                    32,
                    0,
                    zero.as_ptr().cast::<c_void>(),
                    1,
                );
            }
            if w > 0 && h > 0 {
                let kde_blur = (x.x_intern_atom)(dpy, c"_KDE_NET_WM_BLUR_BEHIND_REGION".as_ptr(), 0);
                if kde_blur != 0 {
                    let region: [u32; 4] = [0, 0, w as u32, h as u32];
                    (x.x_change_property)(
                        dpy,
                        xid as c_ulong,
                        kde_blur,
                        card_ty,
                        32,
                        0,
                        region.as_ptr().cast::<c_void>(),
                        4,
                    );
                }
            }
        }
        (x.x_flush)(dpy);
        (x.x_close_display)(dpy);
    }
}

pub fn move_window_x11(xid: u64, x_pos: i32, y_pos: i32) {
    if !window_uses_x11() || xid == 0 {
        return;
    }
    let Some(x) = x11() else {
        return;
    };
    unsafe {
        if let Some(dpy) = open_x11_display(x) {
            (x.x_move_window)(dpy, xid as c_ulong, x_pos, y_pos);
            (x.x_flush)(dpy);
            (x.x_close_display)(dpy);
        }
    }
}

pub fn query_pointer_x11() -> Option<(i32, i32)> {
    if !window_uses_x11() {
        return None;
    }
    let x = x11()?;
    unsafe {
        let dpy = open_x11_display(x)?;
        let root = (x.x_default_root_window)(dpy);
        let mut r_ret: c_ulong = 0;
        let mut c_ret: c_ulong = 0;
        let mut rx: c_int = 0;
        let mut ry: c_int = 0;
        let mut wx: c_int = 0;
        let mut wy: c_int = 0;
        let mut mask: c_uint = 0;
        let ok = (x.x_query_pointer)(
            dpy,
            root,
            &mut r_ret,
            &mut c_ret,
            &mut rx,
            &mut ry,
            &mut wx,
            &mut wy,
            &mut mask,
        );
        (x.x_close_display)(dpy);
        (ok != 0).then_some((rx, ry))
    }
}

pub fn all_monitor_geometries_x11() -> Vec<(i32, i32, i32, i32, bool)> {
    if !window_uses_x11() {
        return vec![(0, 0, 1920, 1080, true)];
    }
    let Some(x) = x11() else {
        return vec![(0, 0, 1920, 1080, true)];
    };
    unsafe {
        let Some(dpy) = open_x11_display(x) else {
            return vec![(0, 0, 1920, 1080, true)];
        };
        let mut out = Vec::new();
        if let Some(xr) = xrandr() {
            let root = (x.x_default_root_window)(dpy);
            let mut nmon: c_int = 0;
            let mons = (xr.xrr_get_monitors)(dpy, root, 1, &mut nmon);
            if !mons.is_null() && nmon > 0 {
                let slice = core::slice::from_raw_parts(mons, nmon as usize);
                for m in slice {
                    if m.width > 0 && m.height > 0 {
                        out.push((m.x, m.y, m.width, m.height, m.primary != 0));
                    }
                }
                (xr.xrr_free_monitors)(mons);
            }
        }
        if out.is_empty() {
            let scr = (x.x_default_screen)(dpy);
            let w = (x.x_display_width)(dpy, scr);
            let h = (x.x_display_height)(dpy, scr);
            if w > 0 && h > 0 {
                out.push((0, 0, w, h, true));
            } else {
                out.push((0, 0, 1920, 1080, true));
            }
        }
        (x.x_close_display)(dpy);
        out
    }
}

pub fn primary_monitor_geometry() -> (i32, i32, i32, i32) {
    let mons = all_monitor_geometries_x11();
    if let Some(&(x, y, w, h, _)) = mons.iter().find(|m| m.4) {
        return (x, y, w, h);
    }
    let (x, y, w, h, _) = mons[0];
    (x, y, w, h)
}

pub fn monitor_geometry_near(px: i32, py: i32) -> (i32, i32, i32, i32) {
    let mons = all_monitor_geometries_x11();
    for &(x, y, w, h, _) in &mons {
        if px >= x && px < x + w && py >= y && py < y + h {
            return (x, y, w, h);
        }
    }
    if let Some(&(x, y, w, h, _)) = mons.iter().find(|m| m.4) {
        (x, y, w, h)
    } else {
        let (x, y, w, h, _) = mons[0];
        (x, y, w, h)
    }
}

pub fn display_is_x11() -> bool {
    let has_wayland = env::var("WAYLAND_DISPLAY").is_ok_and(|v| !v.is_empty())
        || env::var("WAYLAND_SOCKET").is_ok_and(|v| !v.is_empty());
    let has_x11 = env::var("DISPLAY").is_ok_and(|v| !v.is_empty());
    !has_wayland && has_x11
}

/// Returns true when the current Qt process is using X11/XCB (`QT_QPA_PLATFORM=xcb` on
/// XWayland or a pure X11 session), so EWMH `_NET_WM_STATE_SKIP_TASKBAR` hints apply even
/// on Wayland sessions that fall back to XWayland (e.g. GNOME without `zwlr_layer_shell_v1`).
pub fn window_uses_x11() -> bool {
    let has_x11 = env::var("DISPLAY").is_ok_and(|v| !v.is_empty());
    if !has_x11 {
        return false;
    }
    if let Ok(qpa) = env::var("QT_QPA_PLATFORM") {
        let q = qpa.trim();
        if q.eq_ignore_ascii_case("xcb") || q.starts_with("xcb:") {
            return true;
        }
        if q.to_ascii_lowercase().contains("wayland") || q.eq_ignore_ascii_case("offscreen") {
            return false;
        }
    }
    display_is_x11()
}

fn parse_xft_dpi_from_xrdb(txt: &str) -> Option<f64> {
    for line in txt.lines() {
        let trimmed = line.trim();
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("Xft.dpi") {
                if let Ok(dpi) = v.trim().parse::<f64>() {
                    if (48.0..=576.0).contains(&dpi) {
                        return Some(dpi);
                    }
                }
            }
        }
    }
    None
}

fn config_home_dir() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| s.starts_with('/'))
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config"))
}

fn kwin_xwayland_scale() -> Option<f64> {
    let cfg = config_home_dir().join("kwinrc");
    if fs::symlink_metadata(&cfg)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true)
    {
        return None;
    }
    let txt = read_text_lossy(&cfg, 131_072);
    let mut in_xwayland = false;
    for raw in txt.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_xwayland = line.eq_ignore_ascii_case("[Xwayland]");
            continue;
        }
        if in_xwayland {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("Scale") {
                    if let Ok(s) = v.trim().parse::<f64>() {
                        if (1.0..=4.0).contains(&s) {
                            return Some(s);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Logical-to-physical UI scale factor on X11/XWayland desktops (e.g. 1.5 for 144 DPI).
pub fn x11_ui_scale() -> f64 {
    if !window_uses_x11() {
        return 1.0;
    }
    let mut desired = None;
    if let Some(x) = x11() {
        if let Some(rm_fn) = x.x_resource_manager_string {
            unsafe {
                if let Some(dpy) = open_x11_display(x) {
                    let rm_ptr = rm_fn(dpy);
                    if !rm_ptr.is_null() {
                        let txt = CStr::from_ptr(rm_ptr).to_string_lossy();
                        desired = parse_xft_dpi_from_xrdb(&txt).map(|dpi| dpi / 96.0);
                    }
                    (x.x_close_display)(dpy);
                }
            }
        }
    }
    if desired.is_none() {
        desired = env::var("GDK_DPI_SCALE")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|&s| (0.5..=4.0).contains(&s));
    }
    let on_wayland_session = env::var_os("WAYLAND_DISPLAY").is_some()
        || env::var("XDG_SESSION_TYPE")
            .ok()
            .is_some_and(|v| v.eq_ignore_ascii_case("wayland"));
    if desired.is_none() && on_wayland_session {
        desired = kwin_xwayland_scale();
    }
    let rel = desired.unwrap_or(1.0).clamp(1.0, 3.0);
    if (rel - 1.0).abs() < 0.05 {
        1.0
    } else {
        rel
    }
}

pub fn map_sni_point_to_screen(req_x: i32, req_y: i32) -> (i32, i32) {
    if req_x == 0 && req_y == 0 {
        return query_pointer_x11().unwrap_or((0, 0));
    }
    let scale = x11_ui_scale();
    if scale <= 1.05 {
        return (req_x, req_y);
    }
    let sx = (f64::from(req_x) * scale).round() as i32;
    let sy = (f64::from(req_y) * scale).round() as i32;
    if let Some((px, py)) = query_pointer_x11() {
        let d_scaled = (sx - px).abs() + (sy - py).abs();
        let d_raw = (req_x - px).abs() + (req_y - py).abs();
        if d_scaled <= 220 || d_scaled < d_raw {
            return if d_scaled <= 120 { (px, py) } else { (sx, sy) };
        }
        return (req_x, req_y);
    }
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        (sx, sy)
    } else {
        (req_x, req_y)
    }
}

fn read_panel_clearance_logical(panel_cid: Option<&str>) -> i32 {
    let rc = config_home_dir().join("plasmashellrc");
    if fs::symlink_metadata(&rc)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true)
    {
        return 60;
    }
    let txt = read_text_lossy(&rc, 131_072);
    let target_exact = panel_cid.map(|cid| format!("[PlasmaViews][Panel {cid}]"));
    let target_sub = panel_cid.map(|cid| format!("[PlasmaViews][Panel {cid}]["));
    let mut best: Option<i32> = None;
    let mut in_panel = false;
    let mut is_target = false;
    for raw in txt.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_panel = line.contains("[PlasmaViews][Panel ");
            is_target = target_exact.as_deref() == Some(line)
                || target_sub.as_ref().is_some_and(|p| line.starts_with(p));
            continue;
        }
        if in_panel && (target_exact.is_none() || is_target) {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == "thickness" {
                    if let Ok(val) = v.trim().parse::<i32>() {
                        if (20..=160).contains(&val) {
                            if is_target {
                                return (val + 6).clamp(40, 120);
                            }
                            best = Some(best.map_or(val, |b| b.max(val)));
                        }
                    }
                }
            }
        }
    }
    best.map(|v| (v + 6).clamp(40, 120)).unwrap_or(60)
}

/// Returns `(panel_location, scaled_clearance, scaled_right_pad)` for the Plasma panel hosting systemtray.
pub fn plasma_tray_panel_info() -> (i32, i32, i32) {
    let scale = x11_ui_scale();
    let cfg = config_home_dir().join("plasma-org.kde.plasma.desktop-appletsrc");
    let is_bad = fs::symlink_metadata(&cfg)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true);
    let mut location = 4;
    let mut right_pad = 16;
    let mut chosen_cid: Option<String> = None;
    if !is_bad {
        let txt = read_text_lossy(&cfg, 524_288);
        let mut tray_panels: Vec<String> = Vec::new();
        let mut formfactors: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut locations: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
        let mut orders: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut cur_cid: Option<String> = None;
        let mut cur_aid: Option<String> = None;
        let mut cur_sub_general = false;
        for raw in txt.lines() {
            let line = raw.trim();
            if line.starts_with('[') && line.ends_with(']') {
                let inner = &line[1..line.len() - 1];
                let parts: Vec<&str> = inner.split("][").collect();
                cur_cid = None;
                cur_aid = None;
                cur_sub_general = false;
                if parts.len() == 2 && parts[0] == "Containments" {
                    cur_cid = Some(parts[1].to_string());
                } else if parts.len() == 3 && parts[0] == "Containments" && parts[2] == "General" {
                    cur_cid = Some(parts[1].to_string());
                    cur_sub_general = true;
                } else if parts.len() == 4 && parts[0] == "Containments" && parts[2] == "Applets" {
                    cur_cid = Some(parts[1].to_string());
                    cur_aid = Some(parts[3].to_string());
                }
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let Some(ref cid) = cur_cid else {
                continue;
            };
            let k = k.trim();
            let v = v.trim();
            if let Some(ref aid) = cur_aid {
                if k == "plugin" && v == "org.kde.plasma.systemtray" {
                    tray_panels.push(format!("{cid}:{aid}"));
                }
            } else if cur_sub_general {
                if k == "AppletOrder" {
                    orders.insert(cid.clone(), v.to_string());
                }
            } else if k == "formfactor" {
                formfactors.insert(cid.clone(), v.to_string());
            } else if k == "location" {
                if let Ok(loc) = v.parse::<i32>() {
                    locations.insert(cid.clone(), loc);
                }
            }
        }
        for pair in tray_panels {
            if let Some((cid, aid)) = pair.split_once(':') {
                if formfactors.get(cid).map(String::as_str) == Some("2") {
                    chosen_cid = Some(cid.to_string());
                    location = *locations.get(cid).unwrap_or(&4);
                    if let Some(ord) = orders.get(cid) {
                        let items: Vec<&str> =
                            ord.split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
                        if let Some(pos) = items.iter().position(|&id| id == aid) {
                            let after = (items.len() - 1 - pos) as i32;
                            right_pad = (after * 64).clamp(16, 264);
                        }
                    }
                    break;
                }
            }
        }
    }
    let logical_clear = read_panel_clearance_logical(chosen_cid.as_deref());
    let scaled_clear = (f64::from(logical_clear) * scale).round() as i32;
    let scaled_pad = (f64::from(right_pad) * scale).round() as i32;
    (location, scaled_clear, scaled_pad)
}

pub fn plasma_panel_clearance() -> i32 {
    plasma_tray_panel_info().1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xft_dpi_handles_xrdb_formats() {
        assert_eq!(parse_xft_dpi_from_xrdb("Xft.dpi:\t144\n"), Some(144.0));
        assert_eq!(parse_xft_dpi_from_xrdb("Xft.dpi: 96\n"), Some(96.0));
        assert_eq!(parse_xft_dpi_from_xrdb("Xft.dpi: 12\n"), None);
    }
}
