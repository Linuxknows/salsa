// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::{c_char, c_void};
use core::ptr;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use salsa_tools::host::{ensure_dir_0700, getpid, install_root, private_runtime_dir, write_atomic_secure};
use salsa_tools::zlib_png::{decode_png_rgba, encode_png_rgba, resize_rgba, RgbaImage};

use crate::gio_dbus::{
    cstr, gio, glib, gobj, GDBusInterfaceVTable, GDBusNodeInfo, GlibApi, G_BUS_TYPE_SESSION,
};

pub const SNI_XML: &str = r#"<node>
  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="u" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
    <method name="Activate">
      <arg type="i" name="x" direction="in"/>
      <arg type="i" name="y" direction="in"/>
    </method>
    <method name="SecondaryActivate">
      <arg type="i" name="x" direction="in"/>
      <arg type="i" name="y" direction="in"/>
    </method>
    <method name="ContextMenu">
      <arg type="i" name="x" direction="in"/>
      <arg type="i" name="y" direction="in"/>
    </method>
    <method name="Scroll">
      <arg type="i" name="delta" direction="in"/>
      <arg type="s" name="orientation" direction="in"/>
    </method>
    <method name="ProvideXdgActivationToken">
      <arg type="s" name="token" direction="in"/>
    </method>
    <signal name="NewTitle"/>
    <signal name="NewIcon"/>
    <signal name="NewStatus">
      <arg type="s" name="status"/>
    </signal>
    <signal name="NewToolTip"/>
  </interface>
</node>"#;

pub const TRAY_ICON_SIZES: [u32; 6] = [22, 24, 32, 48, 64, 128];

#[derive(Debug, Clone)]
pub struct TrayIconBank {
    pub theme_root: PathBuf,
    pub apps_dir: PathBuf,
}

fn fallback_logo(size: u32) -> RgbaImage {
    let mut data = vec![0u8; (size * size * 4) as usize];
    let c = size as f32 * 0.5;
    let r_out = size as f32 * 0.44;
    let r_in = size as f32 * 0.28;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let d = (dx * dx + dy * dy).sqrt();
            let idx = ((y * size + x) * 4) as usize;
            if d <= r_out && d >= r_in {
                data[idx] = 0x10;
                data[idx + 1] = 0xB9;
                data[idx + 2] = 0x81;
                data[idx + 3] = 255;
            } else if d < r_in {
                data[idx] = 0x11;
                data[idx + 1] = 0x18;
                data[idx + 2] = 0x27;
                data[idx + 3] = 255;
            }
        }
    }
    RgbaImage {
        width: size,
        height: size,
        data,
    }
}

fn crop_tight_square(img: &RgbaImage, alpha_thresh: u8, target_size: u32) -> RgbaImage {
    let w = img.width;
    let h = img.height;
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x: Option<u32> = None;
    let mut max_y: Option<u32> = None;
    for y in 0..h {
        let row_off = (y * w * 4) as usize;
        for x in 0..w {
            let a = img.data[row_off + (x as usize) * 4 + 3];
            if a >= alpha_thresh {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = Some(max_x.map_or(x, |m| m.max(x)));
                max_y = Some(max_y.map_or(y, |m| m.max(y)));
            }
        }
    }
    let cropped = match (max_x, max_y) {
        (Some(mx), Some(my)) if mx >= min_x && my >= min_y => {
            let cw = mx - min_x + 1;
            let ch = my - min_y + 1;
            let sq = cw.max(ch);
            let mut sq_data = vec![0u8; (sq * sq * 4) as usize];
            let ox = (sq - cw) / 2;
            let oy = (sq - ch) / 2;
            for sy in 0..ch {
                let src_row = (((min_y + sy) * w + min_x) * 4) as usize;
                let dst_row = (((oy + sy) * sq + ox) * 4) as usize;
                let row_bytes = (cw * 4) as usize;
                sq_data[dst_row..dst_row + row_bytes]
                    .copy_from_slice(&img.data[src_row..src_row + row_bytes]);
            }
            RgbaImage {
                width: sq,
                height: sq,
                data: sq_data,
            }
        }
        _ => img.clone(),
    };
    if cropped.width == target_size && cropped.height == target_size {
        cropped
    } else {
        resize_rgba(&cropped, target_size, target_size)
    }
}

fn load_cropped_asset(path: &Path, size: u32, alpha_thresh: u8) -> Option<RgbaImage> {
    let bytes = fs::read(path).ok()?;
    let img = decode_png_rgba(&bytes).ok()?;
    Some(crop_tight_square(&img, alpha_thresh, size))
}

fn composite_centered(dst: &mut RgbaImage, fg: &RgbaImage, fg_alpha_scale: f32) {
    if fg.width > dst.width || fg.height > dst.height {
        return;
    }
    let ox = (dst.width - fg.width) / 2;
    let oy = (dst.height - fg.height) / 2;
    for y in 0..fg.height {
        for x in 0..fg.width {
            let s_idx = ((y * fg.width + x) * 4) as usize;
            let sa = (f32::from(fg.data[s_idx + 3]) / 255.0) * fg_alpha_scale;
            if sa <= 0.001 {
                continue;
            }
            let d_idx = (((oy + y) * dst.width + (ox + x)) * 4) as usize;
            let da = f32::from(dst.data[d_idx + 3]) / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0001 {
                continue;
            }
            for c in 0..3 {
                let sc = f32::from(fg.data[s_idx + c]) / 255.0;
                let dc = f32::from(dst.data[d_idx + c]) / 255.0;
                let oc = (sc * sa + dc * da * (1.0 - sa)) / out_a;
                dst.data[d_idx + c] = (oc.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            dst.data[d_idx + 3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

fn draw_line_capsule(
    img: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius: f32,
    col: [u8; 4],
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= 1e-4 {
        return;
    }
    let w = img.width as i32;
    let h = img.height as i32;
    let sa = f32::from(col[3]) / 255.0;
    for py in 0..h {
        for px in 0..w {
            let fx = px as f32 + 0.5;
            let fy = py as f32 + 0.5;
            let t = (((fx - x0) * dx + (fy - y0) * dy) / len_sq).clamp(0.0, 1.0);
            let cx = x0 + t * dx;
            let cy = y0 + t * dy;
            let dist = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            if dist <= radius + 0.7 {
                let aa = ((radius + 0.7 - dist) / 1.4).clamp(0.0, 1.0) * sa;
                if aa <= 0.001 {
                    continue;
                }
                let idx = ((py as u32 * img.width + px as u32) * 4) as usize;
                let da = f32::from(img.data[idx + 3]) / 255.0;
                let out_a = aa + da * (1.0 - aa);
                if out_a <= 0.0001 {
                    continue;
                }
                for c in 0..3 {
                    let sc = f32::from(col[c]) / 255.0;
                    let dc = f32::from(img.data[idx + c]) / 255.0;
                    img.data[idx + c] =
                        (((sc * aa + dc * da * (1.0 - aa)) / out_a).clamp(0.0, 1.0) * 255.0)
                            .round() as u8;
                }
                img.data[idx + 3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
}

fn make_glow_variant(base_128: &RgbaImage, glow_asset: Option<RgbaImage>) -> RgbaImage {
    if let Some(g) = glow_asset {
        return g;
    }
    let size = 128u32;
    let mut out = RgbaImage {
        width: size,
        height: size,
        data: vec![0u8; (size * size * 4) as usize],
    };
    let c = size as f32 * 0.5;
    let r = size as f32 * 0.5;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let t = d / r;
            let idx = ((y * size + x) * 4) as usize;
            if t <= 1.0 {
                let (col, alpha) = if t <= 0.45 {
                    ([34u8, 197, 94], 240.0)
                } else if t <= 0.75 {
                    let k = (t - 0.45) / 0.30;
                    ([34, 197, 94], 240.0 + k * (170.0 - 240.0))
                } else {
                    let k = (t - 0.75) / 0.25;
                    ([16, 185, 129], 170.0 * (1.0 - k))
                };
                out.data[idx] = col[0];
                out.data[idx + 1] = col[1];
                out.data[idx + 2] = col[2];
                out.data[idx + 3] = alpha.clamp(0.0, 255.0) as u8;
            }
        }
    }
    let inner = resize_rgba(base_128, 120, 120);
    composite_centered(&mut out, &inner, 1.0);
    out
}

fn make_muted_variant(base_128: &RgbaImage) -> RgbaImage {
    let size = 128u32;
    let mut out = RgbaImage {
        width: size,
        height: size,
        data: vec![0u8; (size * size * 4) as usize],
    };
    let inner = resize_rgba(base_128, 122, 122);
    composite_centered(&mut out, &inner, 0.50);
    let pad = size as f32 * 0.11;
    let sz_f = size as f32;
    draw_line_capsule(
        &mut out,
        pad,
        sz_f - pad,
        sz_f - pad,
        pad,
        (sz_f * 0.125) * 0.5,
        [9, 13, 22, 220],
    );
    draw_line_capsule(
        &mut out,
        pad,
        sz_f - pad,
        sz_f - pad,
        pad,
        (sz_f * 0.08) * 0.5,
        [239, 68, 68, 255],
    );
    out
}

fn make_stopped_variant(base_128: &RgbaImage) -> RgbaImage {
    let size = 128u32;
    let mut out = RgbaImage {
        width: size,
        height: size,
        data: vec![0u8; (size * size * 4) as usize],
    };
    let inner = resize_rgba(base_128, 122, 122);
    composite_centered(&mut out, &inner, 0.45);
    let sz_f = size as f32;
    let dot_d = sz_f * 0.25;
    let rad = dot_d * 0.5;
    let border_w = (sz_f * 0.03).max(2.0);
    let cx = sz_f - dot_d * 0.5 - sz_f * 0.05;
    let cy = sz_f - dot_d * 0.5 - sz_f * 0.05;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let idx = ((y * size + x) * 4) as usize;
            if d <= rad - border_w * 0.5 {
                out.data[idx] = 0xF5;
                out.data[idx + 1] = 0x9E;
                out.data[idx + 2] = 0x0B;
                out.data[idx + 3] = 255;
            } else if d <= rad + border_w * 0.5 {
                out.data[idx] = 0x09;
                out.data[idx + 1] = 0x0D;
                out.data[idx + 2] = 0x16;
                out.data[idx + 3] = 255;
            }
        }
    }
    out
}

impl TrayIconBank {
    pub fn generate(custom_theme_root: Option<&Path>) -> Result<Self, String> {
        let is_custom = custom_theme_root.is_some();
        let base_rt = match custom_theme_root {
            Some(p) => p.to_path_buf(),
            None => private_runtime_dir().join("tray-icons"),
        };
        let hicolor = base_rt.join("hicolor");
        ensure_dir_0700(&base_rt).map_err(|e| format!("mkdir {}: {e}", base_rt.display()))?;
        ensure_dir_0700(&hicolor).map_err(|e| format!("mkdir {}: {e}", hicolor.display()))?;

        // Clean stale salsa-lasapi-* tray icon files from older versions
        for old_name in [
            "salsa-lasapi-active.png",
            "salsa-lasapi-glow.png",
            "salsa-lasapi-muted.png",
            "salsa-lasapi-inactive.png",
        ] {
            let _ = fs::remove_file(base_rt.join(old_name));
        }

        let mut dir_list = Vec::with_capacity(TRAY_ICON_SIZES.len());
        let mut dir_secs = String::new();
        for sz in TRAY_ICON_SIZES {
            let sz_dir = hicolor.join(format!("{sz}x{sz}"));
            let apps_dir = sz_dir.join("apps");
            ensure_dir_0700(&sz_dir).map_err(|e| format!("mkdir {}: {e}", sz_dir.display()))?;
            ensure_dir_0700(&apps_dir).map_err(|e| format!("mkdir {}: {e}", apps_dir.display()))?;
            for old_name in [
                "salsa-lasapi-active.png",
                "salsa-lasapi-glow.png",
                "salsa-lasapi-muted.png",
                "salsa-lasapi-inactive.png",
            ] {
                let _ = fs::remove_file(apps_dir.join(old_name));
            }
            dir_list.push(format!("{sz}x{sz}/apps"));
            dir_secs.push_str(&format!(
                "\n[{sz}x{sz}/apps]\nSize={sz}\nContext=Applications\nType=Fixed\n"
            ));
        }

        let index_theme = format!(
            "[Icon Theme]\nName=hicolor\nDirectories={}\n{}",
            dir_list.join(","),
            dir_secs
        );
        write_atomic_secure(&hicolor.join("index.theme"), index_theme.as_bytes(), 0o600)
            .map_err(|e| format!("write index.theme: {e}"))?;

        let repo = install_root();
        let active_plain_128 = load_cropped_asset(&repo.join("assets/salsa-lasapi.png"), 128, 16)
            .unwrap_or_else(|| fallback_logo(128));
        let glow_path = repo.join("assets/salsa-lasapi-glow.png");
        let glow_asset = glow_path
            .is_file()
            .then(|| load_cropped_asset(&glow_path, 128, 48))
            .flatten();
        let active_glow_128 = make_glow_variant(&active_plain_128, glow_asset);
        let muted_128 = make_muted_variant(&active_plain_128);
        let stopped_128 = make_stopped_variant(&active_plain_128);

        let user_apps_128 = if !is_custom {
            let p = salsa_tools::host::home_dir().join(".local/share/icons/hicolor/128x128/apps");
            let _ = fs::create_dir_all(&p);
            Some(p)
        } else {
            None
        };

        for (icon_name, master) in [
            ("salsa-tray-active-glow", &active_glow_128),
            ("salsa-tray-active-plain", &active_plain_128),
            ("salsa-tray-muted", &muted_128),
            ("salsa-tray-stopped", &stopped_128),
        ] {
            let fname = format!("{icon_name}.png");
            let encoded_128 = encode_png_rgba(master)?;
            write_atomic_secure(&base_rt.join(&fname), &encoded_128, 0o600)
                .map_err(|e| format!("write flat icon {fname}: {e}"))?;
            for sz in TRAY_ICON_SIZES {
                let apps_dir = hicolor.join(format!("{sz}x{sz}/apps"));
                let encoded = if sz == 128 {
                    encoded_128.clone()
                } else {
                    encode_png_rgba(&resize_rgba(master, sz, sz))?
                };
                write_atomic_secure(&apps_dir.join(&fname), &encoded, 0o600)
                    .map_err(|e| format!("write icon {sz}x{sz}/{fname}: {e}"))?;
            }
            if icon_name == "salsa-tray-active-plain" {
                if let Some(ref ua) = user_apps_128 {
                    let _ = write_atomic_secure(&ua.join(&fname), &encoded_128, 0o644);
                }
            }
        }

        Ok(Self {
            theme_root: base_rt,
            apps_dir: hicolor.join("128x128/apps"),
        })
    }

    pub fn icon_name(active: bool, muted: bool, glow: bool) -> &'static str {
        if !active {
            "salsa-tray-stopped"
        } else if muted {
            "salsa-tray-muted"
        } else if glow {
            "salsa-tray-active-glow"
        } else {
            "salsa-tray-active-plain"
        }
    }

    pub fn icon_png_path(&self, icon_name: &str) -> PathBuf {
        self.theme_root.join(format!("{icon_name}.png"))
    }
}

pub trait SniCallbacks {
    fn on_activate(&mut self, x: i32, y: i32);
    fn on_secondary_activate(&mut self, x: i32, y: i32);
    fn on_context_menu(&mut self, x: i32, y: i32);
    fn on_scroll(&mut self, delta: i32, orientation: &str);
}

struct SniInner {
    svc_name: String,
    icon_theme_path: String,
    icon_name: String,
    title: String,
    status: String,
    conn: *mut c_void,
    reg_id: u32,
    own_id: u32,
    watch_id: u32,
    node_info: *mut GDBusNodeInfo,
    callbacks: Option<Box<dyn SniCallbacks>>,
}

#[derive(Clone)]
pub struct StatusNotifierHost {
    inner: Rc<RefCell<SniInner>>,
}

unsafe extern "C" fn sni_method_call(
    _conn: *mut c_void,
    _sender: *const c_char,
    _obj_path: *const c_char,
    _iface: *const c_char,
    method_name: *const c_char,
    parameters: *mut c_void,
    invocation: *mut c_void,
    user_data: *mut c_void,
) {
    let Ok(gi) = gio() else {
        return;
    };
    let Ok(gl) = glib() else {
        return;
    };
    let method = if method_name.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(method_name) }
            .to_str()
            .unwrap_or("")
    };

    let rc = unsafe { Rc::from_raw(user_data as *const RefCell<SniInner>) };
    let mut cbs = {
        let mut st = rc.borrow_mut();
        st.callbacks.take()
    };

    if let Some(ref mut cb) = cbs {
        match method {
            "Activate" => {
                let (x, y) = unsafe { read_two_i32(gl, parameters) };
                cb.on_activate(x, y);
            }
            "SecondaryActivate" => {
                let (x, y) = unsafe { read_two_i32(gl, parameters) };
                cb.on_secondary_activate(x, y);
            }
            "ContextMenu" => {
                let (x, y) = unsafe { read_two_i32(gl, parameters) };
                cb.on_context_menu(x, y);
            }
            "Scroll" => {
                let (delta, orient) = unsafe { read_i32_and_str(gl, parameters) };
                cb.on_scroll(delta, &orient);
            }
            "ProvideXdgActivationToken" => {}
            _ => {}
        }
    }
    rc.borrow_mut().callbacks = cbs;
    let _ = Rc::into_raw(rc);

    unsafe { (gi.g_dbus_method_invocation_return_value)(invocation, ptr::null_mut()) };
}

unsafe fn read_two_i32(gl: &GlibApi, params: *mut c_void) -> (i32, i32) {
    if params.is_null() || unsafe { (gl.g_variant_n_children)(params) } < 2 {
        return (0, 0);
    }
    let c0 = unsafe { (gl.g_variant_get_child_value)(params, 0) };
    let c1 = unsafe { (gl.g_variant_get_child_value)(params, 1) };
    let x = unsafe { (gl.g_variant_get_int32)(c0) };
    let y = unsafe { (gl.g_variant_get_int32)(c1) };
    unsafe {
        (gl.g_variant_unref)(c0);
        (gl.g_variant_unref)(c1);
    }
    (x, y)
}

unsafe fn read_i32_and_str(gl: &GlibApi, params: *mut c_void) -> (i32, String) {
    if params.is_null() || unsafe { (gl.g_variant_n_children)(params) } < 2 {
        return (0, "vertical".to_string());
    }
    let c0 = unsafe { (gl.g_variant_get_child_value)(params, 0) };
    let c1 = unsafe { (gl.g_variant_get_child_value)(params, 1) };
    let delta = unsafe { (gl.g_variant_get_int32)(c0) };
    let mut len: usize = 0;
    let sp = unsafe { (gl.g_variant_get_string)(c1, &mut len) };
    let orient = if sp.is_null() {
        "vertical".to_string()
    } else {
        unsafe { CStr::from_ptr(sp) }.to_string_lossy().into_owned()
    };
    unsafe {
        (gl.g_variant_unref)(c0);
        (gl.g_variant_unref)(c1);
    }
    (delta, orient)
}

unsafe extern "C" fn sni_get_property(
    _conn: *mut c_void,
    _sender: *const c_char,
    _obj_path: *const c_char,
    _iface: *const c_char,
    prop_name: *const c_char,
    _error: *mut *mut c_void,
    user_data: *mut c_void,
) -> *mut c_void {
    let Ok(gl) = glib() else {
        return ptr::null_mut();
    };
    let prop = if prop_name.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(prop_name) }.to_str().unwrap_or("")
    };
    let st_rc = unsafe { &*(user_data as *const RefCell<SniInner>) };
    let st = st_rc.borrow();

    unsafe {
        match prop {
            "Category" => (gl.g_variant_new_string)(c"ApplicationStatus".as_ptr()),
            "Id" => (gl.g_variant_new_string)(c"SalsaAudio".as_ptr()),
            "Title" => {
                let c = cstr(&st.title);
                (gl.g_variant_new_string)(c.as_ptr())
            }
            "Status" => {
                let c = cstr(&st.status);
                (gl.g_variant_new_string)(c.as_ptr())
            }
            "WindowId" => (gl.g_variant_new_uint32)(0),
            "ItemIsMenu" => (gl.g_variant_new_boolean)(0),
            "IconName" => {
                let c = cstr(&st.icon_name);
                (gl.g_variant_new_string)(c.as_ptr())
            }
            "IconThemePath" => {
                let c = cstr(&st.icon_theme_path);
                (gl.g_variant_new_string)(c.as_ptr())
            }
            "ToolTip" => {
                let c_ico = cstr(&st.icon_name);
                let c_title = cstr(&st.title);
                let c_sub = cstr("Salsa — LASAPI Studio");
                let empty_pix_ty = (gl.g_variant_type_checked_)(c"(iiay)".as_ptr());
                let ch0 = (gl.g_variant_new_string)(c_ico.as_ptr());
                let ch1 = (gl.g_variant_new_array)(empty_pix_ty, ptr::null(), 0);
                let ch2 = (gl.g_variant_new_string)(c_title.as_ptr());
                let ch3 = (gl.g_variant_new_string)(c_sub.as_ptr());
                let tuple = [ch0, ch1, ch2, ch3];
                (gl.g_variant_new_tuple)(tuple.as_ptr(), tuple.len())
            }
            _ => ptr::null_mut(),
        }
    }
}

static SNI_VTABLE: GDBusInterfaceVTable = GDBusInterfaceVTable {
    method_call: Some(sni_method_call),
    get_property: Some(sni_get_property),
    set_property: None,
    padding: [ptr::null_mut(); 8],
};

unsafe extern "C" fn on_call_done(_src: *mut c_void, _res: *mut c_void, _data: *mut c_void) {}

unsafe fn register_with_watcher(conn: *mut c_void, svc_name: &str) {
    let Ok(gi) = gio() else {
        return;
    };
    let Ok(gl) = glib() else {
        return;
    };
    let c_svc = cstr(svc_name);
    let arg = unsafe { (gl.g_variant_new_string)(c_svc.as_ptr()) };
    let args = unsafe { (gl.g_variant_new_tuple)(&arg, 1) };
    unsafe {
        (gi.g_dbus_connection_call)(
            conn,
            c"org.kde.StatusNotifierWatcher".as_ptr(),
            c"/StatusNotifierWatcher".as_ptr(),
            c"org.kde.StatusNotifierWatcher".as_ptr(),
            c"RegisterStatusNotifierItem".as_ptr(),
            args,
            ptr::null(),
            0,
            2000,
            ptr::null_mut(),
            Some(on_call_done),
            ptr::null_mut(),
        );
    }
}

unsafe extern "C" fn on_bus_acquired(conn: *mut c_void, _name: *const c_char, user_data: *mut c_void) {
    let Ok(gi) = gio() else {
        return;
    };
    let Ok(go) = gobj() else {
        return;
    };
    let rc = unsafe {
        let raw = user_data as *const RefCell<SniInner>;
        Rc::increment_strong_count(raw);
        Rc::from_raw(raw)
    };
    {
        let mut st = rc.borrow_mut();
        if st.conn != conn {
            let old = st.conn;
            st.conn = if conn.is_null() {
                ptr::null_mut()
            } else {
                unsafe { (go.g_object_ref)(conn) }
            };
            if !old.is_null() {
                unsafe { (go.g_object_unref)(old) };
            }
        }
        if !st.node_info.is_null() {
            let ifaces = unsafe { (*st.node_info).interfaces };
            if !ifaces.is_null() {
                let iface = unsafe { *ifaces };
                if !iface.is_null() {
                    let mut err: *mut c_void = ptr::null_mut();
                    let reg_id = unsafe {
                        (gi.g_dbus_connection_register_object)(
                            conn,
                            c"/StatusNotifierItem".as_ptr(),
                            iface,
                            &SNI_VTABLE,
                            Rc::as_ptr(&rc) as *mut c_void,
                            None,
                            &mut err,
                        )
                    };
                    if !err.is_null() {
                        if let Ok(gl) = glib() {
                            unsafe { (gl.g_error_free)(err) };
                        }
                    }
                    st.reg_id = reg_id;
                }
            }
        }
    }
}

unsafe extern "C" fn on_name_acquired(conn: *mut c_void, _name: *const c_char, user_data: *mut c_void) {
    let rc = unsafe {
        let raw = user_data as *const RefCell<SniInner>;
        Rc::increment_strong_count(raw);
        Rc::from_raw(raw)
    };
    let svc = {
        let mut st = rc.borrow_mut();
        if st.conn.is_null() && !conn.is_null() {
            if let Ok(go) = gobj() {
                st.conn = unsafe { (go.g_object_ref)(conn) };
            }
        }
        st.svc_name.clone()
    };
    unsafe { register_with_watcher(conn, &svc) };
}

unsafe extern "C" fn on_watcher_appeared(
    conn: *mut c_void,
    _name: *const c_char,
    _owner: *const c_char,
    user_data: *mut c_void,
) {
    let rc = unsafe {
        let raw = user_data as *const RefCell<SniInner>;
        Rc::increment_strong_count(raw);
        Rc::from_raw(raw)
    };
    let svc = rc.borrow().svc_name.clone();
    unsafe { register_with_watcher(conn, &svc) };
}

impl StatusNotifierHost {
    pub fn start(icon_theme_dir: &Path, callbacks: Box<dyn SniCallbacks>) -> Result<Self, String> {
        let gi = gio()?;
        let gl = glib()?;
        let mut err: *mut c_void = ptr::null_mut();
        let xml_cs = cstr(SNI_XML);
        let node_info = unsafe { (gi.g_dbus_node_info_new_for_xml)(xml_cs.as_ptr(), &mut err) };
        if node_info.is_null() {
            if !err.is_null() {
                unsafe { (gl.g_error_free)(err) };
            }
            return Err("g_dbus_node_info_new_for_xml failed".into());
        }

        let svc_name = format!("org.kde.StatusNotifierItem-{}-1", getpid());
        let inner = Rc::new(RefCell::new(SniInner {
            svc_name: svc_name.clone(),
            icon_theme_path: icon_theme_dir.to_string_lossy().into_owned(),
            icon_name: "salsa-tray-active-glow".to_string(),
            title: "Salsa — Active".to_string(),
            status: "Active".to_string(),
            conn: ptr::null_mut(),
            reg_id: 0,
            own_id: 0,
            watch_id: 0,
            node_info,
            callbacks: Some(callbacks),
        }));

        let raw_ptr = Rc::into_raw(Rc::clone(&inner)) as *mut c_void;
        let svc_cs = cstr(&svc_name);
        let own_id = unsafe {
            (gi.g_bus_own_name)(
                G_BUS_TYPE_SESSION,
                svc_cs.as_ptr(),
                0,
                Some(on_bus_acquired),
                Some(on_name_acquired),
                None,
                raw_ptr,
                None,
            )
        };
        let watch_id = unsafe {
            (gi.g_bus_watch_name)(
                G_BUS_TYPE_SESSION,
                c"org.kde.StatusNotifierWatcher".as_ptr(),
                0,
                Some(on_watcher_appeared),
                None,
                raw_ptr,
                None,
            )
        };
        {
            let mut st = inner.borrow_mut();
            st.own_id = own_id;
            st.watch_id = watch_id;
        }
        Ok(Self { inner })
    }

    pub fn update_state(&self, icon_name: &str, title: &str, status: &str) {
        let Ok(gi) = gio() else {
            return;
        };
        let Ok(gl) = glib() else {
            return;
        };
        let mut st = self.inner.borrow_mut();
        let ico_chg = st.icon_name != icon_name;
        let tit_chg = st.title != title;
        let sta_chg = st.status != status;
        if !ico_chg && !tit_chg && !sta_chg {
            return;
        }
        st.icon_name = icon_name.to_string();
        st.title = title.to_string();
        st.status = status.to_string();
        let conn = st.conn;
        drop(st);

        if conn.is_null() {
            return;
        }
        unsafe {
            if ico_chg {
                (gi.g_dbus_connection_emit_signal)(
                    conn,
                    ptr::null(),
                    c"/StatusNotifierItem".as_ptr(),
                    c"org.kde.StatusNotifierItem".as_ptr(),
                    c"NewIcon".as_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            }
            if tit_chg || ico_chg {
                if tit_chg {
                    (gi.g_dbus_connection_emit_signal)(
                        conn,
                        ptr::null(),
                        c"/StatusNotifierItem".as_ptr(),
                        c"org.kde.StatusNotifierItem".as_ptr(),
                        c"NewTitle".as_ptr(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                }
                (gi.g_dbus_connection_emit_signal)(
                    conn,
                    ptr::null(),
                    c"/StatusNotifierItem".as_ptr(),
                    c"org.kde.StatusNotifierItem".as_ptr(),
                    c"NewToolTip".as_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            }
            if sta_chg {
                let cs = cstr(status);
                let arg = (gl.g_variant_new_string)(cs.as_ptr());
                let tuple = (gl.g_variant_new_tuple)(&arg, 1);
                (gi.g_dbus_connection_emit_signal)(
                    conn,
                    ptr::null(),
                    c"/StatusNotifierItem".as_ptr(),
                    c"org.kde.StatusNotifierItem".as_ptr(),
                    c"NewStatus".as_ptr(),
                    tuple,
                    ptr::null_mut(),
                );
            }
        }
    }

    pub fn shutdown(&self) {
        let Ok(gi) = gio() else {
            return;
        };
        let mut st = self.inner.borrow_mut();
        st.callbacks = None;
        if !st.conn.is_null() && st.reg_id != 0 {
            unsafe { (gi.g_dbus_connection_unregister_object)(st.conn, st.reg_id) };
            st.reg_id = 0;
        }
        if st.watch_id != 0 {
            unsafe { (gi.g_bus_unwatch_name)(st.watch_id) };
            st.watch_id = 0;
        }
        if st.own_id != 0 {
            unsafe { (gi.g_bus_unown_name)(st.own_id) };
            st.own_id = 0;
        }
        let conn = st.conn;
        st.conn = ptr::null_mut();
        if !conn.is_null() {
            if let Ok(go) = gobj() {
                unsafe { (go.g_object_unref)(conn) };
            }
        }
        if !st.node_info.is_null() {
            unsafe { (gi.g_dbus_node_info_unref)(st.node_info) };
            st.node_info = ptr::null_mut();
        }
    }
}

#[derive(Debug, Clone)]
pub enum SniEvent {
    Activate(i32, i32),
    SecondaryActivate(i32, i32),
    ContextMenu(i32, i32),
    Scroll(i32, String),
}

enum SniCmd {
    Update {
        icon_name: String,
        title: String,
        status: String,
    },
    Shutdown,
}

struct SniDispatchCb(Box<dyn FnMut(SniEvent) + Send>);

impl SniCallbacks for SniDispatchCb {
    fn on_activate(&mut self, x: i32, y: i32) {
        (self.0)(SniEvent::Activate(x, y));
    }

    fn on_secondary_activate(&mut self, x: i32, y: i32) {
        (self.0)(SniEvent::SecondaryActivate(x, y));
    }

    fn on_context_menu(&mut self, x: i32, y: i32) {
        (self.0)(SniEvent::ContextMenu(x, y));
    }

    fn on_scroll(&mut self, delta: i32, orientation: &str) {
        (self.0)(SniEvent::Scroll(delta, orientation.to_string()));
    }
}

struct SniThreadShared {
    queue: Mutex<VecDeque<SniCmd>>,
    ctx_addr: usize,
    shut_down: AtomicBool,
}

thread_local! {
    static SNI_TLS: RefCell<Option<(StatusNotifierHost, *mut c_void)>> = const { RefCell::new(None) };
}

extern "C" fn drain_sni_cmds_on_glib_thread(user_data: *mut c_void) -> core::ffi::c_int {
    if user_data.is_null() {
        return 0;
    }
    let shared = unsafe { Arc::from_raw(user_data as *const SniThreadShared) };
    let cmds: Vec<SniCmd> = {
        let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.drain(..).collect()
    };
    SNI_TLS.with(|slot| {
        let borrow = slot.borrow();
        if let Some((ref host, lp)) = *borrow {
            for cmd in cmds {
                match cmd {
                    SniCmd::Update {
                        icon_name,
                        title,
                        status,
                    } => {
                        host.update_state(&icon_name, &title, &status);
                    }
                    SniCmd::Shutdown => {
                        host.shutdown();
                        if !lp.is_null() {
                            if let Ok(gl) = glib() {
                                unsafe { (gl.g_main_loop_quit)(lp) };
                            }
                        }
                    }
                }
            }
        }
    });
    0
}

pub struct SniThreadHost {
    shared: Arc<SniThreadShared>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
}

impl SniThreadHost {
    pub fn start_threaded(
        icon_theme_dir: &Path,
        on_event: Box<dyn FnMut(SniEvent) + Send>,
    ) -> Result<Self, String> {
        let dir_owned = icon_theme_dir.to_path_buf();
        let (tx_ready, rx_ready) = mpsc::sync_channel::<Result<usize, String>>(1);

        let handle = thread::Builder::new()
            .name("salsa-sni-dbus".to_string())
            .spawn(move || {
                let gl = match glib() {
                    Ok(g) => g,
                    Err(e) => {
                        let _ = tx_ready.send(Err(e.to_string()));
                        return;
                    }
                };
                let ctx = unsafe { (gl.g_main_context_new)() };
                if ctx.is_null() {
                    let _ = tx_ready.send(Err("g_main_context_new failed".to_string()));
                    return;
                }
                unsafe { (gl.g_main_context_push_thread_default)(ctx) };
                let lp = unsafe { (gl.g_main_loop_new)(ctx, 0) };
                if lp.is_null() {
                    unsafe {
                        (gl.g_main_context_pop_thread_default)(ctx);
                        (gl.g_main_context_unref)(ctx);
                    }
                    let _ = tx_ready.send(Err("g_main_loop_new failed".to_string()));
                    return;
                }

                match StatusNotifierHost::start(&dir_owned, Box::new(SniDispatchCb(on_event))) {
                    Ok(host) => {
                        SNI_TLS.with(|slot| {
                            *slot.borrow_mut() = Some((host, lp));
                        });
                        let _ = tx_ready.send(Ok(ctx as usize));
                        unsafe { (gl.g_main_loop_run)(lp) };
                        SNI_TLS.with(|slot| {
                            if let Some((host, _)) = slot.borrow_mut().take() {
                                host.shutdown();
                            }
                        });
                    }
                    Err(e) => {
                        let _ = tx_ready.send(Err(e));
                    }
                }

                unsafe {
                    (gl.g_main_loop_unref)(lp);
                    (gl.g_main_context_pop_thread_default)(ctx);
                    (gl.g_main_context_unref)(ctx);
                }
            })
            .map_err(|e| format!("failed spawning salsa-sni-dbus thread: {e}"))?;

        let ctx_addr = rx_ready
            .recv()
            .map_err(|_| "salsa-sni-dbus init channel closed".to_string())??;

        Ok(Self {
            shared: Arc::new(SniThreadShared {
                queue: Mutex::new(VecDeque::new()),
                ctx_addr,
                shut_down: AtomicBool::new(false),
            }),
            join: Mutex::new(Some(handle)),
        })
    }

    fn post_cmd(&self, cmd: SniCmd) {
        if self.shared.shut_down.load(Ordering::Acquire) {
            return;
        }
        {
            let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            q.push_back(cmd);
        }
        if self.shared.shut_down.load(Ordering::Acquire) {
            return;
        }
        if let Ok(gl) = glib() {
            let raw = Arc::into_raw(Arc::clone(&self.shared)) as *mut c_void;
            unsafe {
                (gl.g_main_context_invoke)(
                    self.shared.ctx_addr as *mut c_void,
                    drain_sni_cmds_on_glib_thread,
                    raw,
                );
            }
        }
    }

    pub fn update_state(&self, icon_name: &str, title: &str, status: &str) {
        self.post_cmd(SniCmd::Update {
            icon_name: icon_name.to_string(),
            title: title.to_string(),
            status: status.to_string(),
        });
    }

    pub fn shutdown(&self) {
        if self.shared.shut_down.swap(true, Ordering::AcqRel) {
            return;
        }
        {
            let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            q.push_back(SniCmd::Shutdown);
        }
        if let Ok(gl) = glib() {
            let raw = Arc::into_raw(Arc::clone(&self.shared)) as *mut c_void;
            unsafe {
                (gl.g_main_context_invoke)(
                    self.shared.ctx_addr as *mut c_void,
                    drain_sni_cmds_on_glib_thread,
                    raw,
                );
            }
        }
        if let Some(h) = self.join.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = h.join();
        }
    }
}

impl Drop for SniThreadHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sni_xml_and_icon_bank_work() {
        let gi = gio().expect("gio loaded");
        let cs = cstr(SNI_XML);
        let mut err: *mut c_void = ptr::null_mut();
        let node = unsafe { (gi.g_dbus_node_info_new_for_xml)(cs.as_ptr(), &mut err) };
        assert!(!node.is_null());
        unsafe { (gi.g_dbus_node_info_unref)(node) };

        let tmp_parent = std::env::temp_dir().join(format!("salsa_icon_test_{}", getpid()));
        let tmp = tmp_parent.join("tray-icons");
        let bank = TrayIconBank::generate(Some(&tmp)).expect("generate TrayIconBank");
        assert!(bank.theme_root.ends_with("tray-icons"));
        let idx_txt = fs::read_to_string(bank.theme_root.join("hicolor/index.theme")).unwrap();
        assert!(idx_txt.contains("Name=hicolor"));
        for name in [
            "salsa-tray-active-glow",
            "salsa-tray-active-plain",
            "salsa-tray-muted",
            "salsa-tray-stopped",
        ] {
            let flat_p = bank.icon_png_path(name);
            let bytes = fs::read(&flat_p).expect("read flat png");
            let img = decode_png_rgba(&bytes).expect("decode png");
            assert_eq!(img.width, 128);
            assert_eq!(img.height, 128);
            for sz in TRAY_ICON_SIZES {
                let sz_p = bank
                    .theme_root
                    .join(format!("hicolor/{sz}x{sz}/apps/{name}.png"));
                let sb = fs::read(&sz_p).expect("read sized png");
                let si = decode_png_rgba(&sb).expect("decode sized png");
                assert_eq!(si.width, sz);
                assert_eq!(si.height, sz);
            }
        }
        let _ = fs::remove_dir_all(&tmp_parent);
    }
}
