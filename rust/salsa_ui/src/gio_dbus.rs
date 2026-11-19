use core::ffi::{c_char, c_int, c_uint, c_void};
use core::mem;
use std::ffi::{CStr, CString};
use std::sync::OnceLock;

use salsa_wire::unix_sys::{dl_open_any, dl_sym};

pub const G_BUS_TYPE_SESSION: c_int = 2;

#[repr(C)]
pub struct GDBusInterfaceVTable {
    pub method_call: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            *const c_char,
            *const c_char,
            *const c_char,
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ),
    >,
    pub get_property: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            *const c_char,
            *const c_char,
            *const c_char,
            *mut *mut c_void,
            *mut c_void,
        ) -> *mut c_void,
    >,
    pub set_property: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            *const c_char,
            *const c_char,
            *const c_char,
            *mut c_void,
            *mut *mut c_void,
            *mut c_void,
        ) -> c_int,
    >,
    pub padding: [*mut c_void; 8],
}

unsafe impl Sync for GDBusInterfaceVTable {}

#[repr(C)]
pub struct GDBusNodeInfo {
    pub ref_count: c_int,
    pub path: *mut c_char,
    pub interfaces: *mut *mut c_void,
    pub nodes: *mut *mut c_void,
    pub annotations: *mut *mut c_void,
}

pub unsafe fn load_sym<T: Copy>(lib: *mut c_void, name: &CStr) -> Result<T, String> {
    let sym = name
        .to_str()
        .map_err(|_| format!("non-utf8 symbol {}", name.to_string_lossy()))?;
    let ptr = unsafe { dl_sym(lib, sym) };
    if ptr.is_null() {
        return Err(format!("missing symbol {sym}"));
    }
    Ok(unsafe { mem::transmute_copy(&ptr) })
}

pub unsafe fn load_opt_sym<T: Copy>(lib: *mut c_void, name: &CStr) -> Option<T> {
    let sym = name.to_str().ok()?;
    let ptr = unsafe { dl_sym(lib, sym) };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { mem::transmute_copy(&ptr) })
    }
}

pub struct GlibApi {
    pub g_free: unsafe extern "C" fn(*mut c_void),
    pub g_main_context_new: unsafe extern "C" fn() -> *mut c_void,
    pub g_main_context_unref: unsafe extern "C" fn(*mut c_void),
    pub g_main_context_push_thread_default: unsafe extern "C" fn(*mut c_void),
    pub g_main_context_pop_thread_default: unsafe extern "C" fn(*mut c_void),
    pub g_main_context_invoke:
        unsafe extern "C" fn(*mut c_void, extern "C" fn(*mut c_void) -> c_int, *mut c_void),
    pub g_main_loop_new: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void,
    pub g_main_loop_run: unsafe extern "C" fn(*mut c_void),
    pub g_main_loop_quit: unsafe extern "C" fn(*mut c_void),
    pub g_main_loop_unref: unsafe extern "C" fn(*mut c_void),
    pub g_variant_unref: unsafe extern "C" fn(*mut c_void),
    pub g_variant_new_string: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    pub g_variant_new_uint32: unsafe extern "C" fn(u32) -> *mut c_void,
    pub g_variant_new_boolean: unsafe extern "C" fn(c_int) -> *mut c_void,
    pub g_variant_new_tuple: unsafe extern "C" fn(*const *mut c_void, usize) -> *mut c_void,
    pub g_variant_new_array:
        unsafe extern "C" fn(*const c_void, *const *mut c_void, usize) -> *mut c_void,
    pub g_variant_get_child_value: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void,
    pub g_variant_n_children: unsafe extern "C" fn(*mut c_void) -> usize,
    pub g_variant_get_string: unsafe extern "C" fn(*mut c_void, *mut usize) -> *const c_char,
    pub g_variant_get_int32: unsafe extern "C" fn(*mut c_void) -> i32,
    pub g_variant_type_checked_: unsafe extern "C" fn(*const c_char) -> *const c_void,
    pub g_error_free: unsafe extern "C" fn(*mut c_void),
}

pub struct GObjApi {
    pub g_object_unref: unsafe extern "C" fn(*mut c_void),
    pub g_object_ref: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
}

pub type GBusNameCallback = Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void)>;
pub type GBusNameAppearedCallback =
    Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut c_void)>;
pub type GDestroyNotifyCallback = Option<unsafe extern "C" fn(*mut c_void)>;

pub struct GioApi {
    pub g_bus_own_name: unsafe extern "C" fn(
        c_int,
        *const c_char,
        c_uint,
        GBusNameCallback,
        GBusNameCallback,
        GBusNameCallback,
        *mut c_void,
        GDestroyNotifyCallback,
    ) -> c_uint,
    pub g_bus_watch_name: unsafe extern "C" fn(
        c_int,
        *const c_char,
        c_uint,
        GBusNameAppearedCallback,
        GBusNameCallback,
        *mut c_void,
        GDestroyNotifyCallback,
    ) -> c_uint,
    pub g_bus_unown_name: unsafe extern "C" fn(c_uint),
    pub g_bus_unwatch_name: unsafe extern "C" fn(c_uint),
    pub g_dbus_node_info_new_for_xml:
        unsafe extern "C" fn(*const c_char, *mut *mut c_void) -> *mut GDBusNodeInfo,
    pub g_dbus_node_info_unref: unsafe extern "C" fn(*mut GDBusNodeInfo),
    pub g_dbus_connection_register_object: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *mut c_void,
        *const GDBusInterfaceVTable,
        *mut c_void,
        Option<unsafe extern "C" fn(*mut c_void)>,
        *mut *mut c_void,
    ) -> c_uint,
    pub g_dbus_connection_unregister_object: unsafe extern "C" fn(*mut c_void, c_uint) -> c_int,
    pub g_dbus_connection_emit_signal: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *mut c_void,
        *mut *mut c_void,
    ) -> c_int,
    pub g_dbus_connection_call: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *mut c_void,
        *const c_void,
        c_int,
        c_int,
        *mut c_void,
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void)>,
        *mut c_void,
    ),
    pub g_dbus_method_invocation_return_value: unsafe extern "C" fn(*mut c_void, *mut c_void),
}

static GLIB: OnceLock<Result<GlibApi, String>> = OnceLock::new();
static GOBJ: OnceLock<Result<GObjApi, String>> = OnceLock::new();
static GIO: OnceLock<Result<GioApi, String>> = OnceLock::new();

pub fn glib() -> Result<&'static GlibApi, &'static str> {
    GLIB.get_or_init(|| unsafe {
        let lib = dl_open_any(&["libglib-2.0.so.0", "libglib-2.0.so"]);
        if lib.is_null() {
            return Err("failed dlopen libglib-2.0.so.0".into());
        }
        Ok(GlibApi {
            g_free: load_sym(lib, c"g_free")?,
            g_main_context_new: load_sym(lib, c"g_main_context_new")?,
            g_main_context_unref: load_sym(lib, c"g_main_context_unref")?,
            g_main_context_push_thread_default: load_sym(
                lib,
                c"g_main_context_push_thread_default",
            )?,
            g_main_context_pop_thread_default: load_sym(
                lib,
                c"g_main_context_pop_thread_default",
            )?,
            g_main_context_invoke: load_sym(lib, c"g_main_context_invoke")?,
            g_main_loop_new: load_sym(lib, c"g_main_loop_new")?,
            g_main_loop_run: load_sym(lib, c"g_main_loop_run")?,
            g_main_loop_quit: load_sym(lib, c"g_main_loop_quit")?,
            g_main_loop_unref: load_sym(lib, c"g_main_loop_unref")?,
            g_variant_unref: load_sym(lib, c"g_variant_unref")?,
            g_variant_new_string: load_sym(lib, c"g_variant_new_string")?,
            g_variant_new_uint32: load_sym(lib, c"g_variant_new_uint32")?,
            g_variant_new_boolean: load_sym(lib, c"g_variant_new_boolean")?,
            g_variant_new_tuple: load_sym(lib, c"g_variant_new_tuple")?,
            g_variant_new_array: load_sym(lib, c"g_variant_new_array")?,
            g_variant_get_child_value: load_sym(lib, c"g_variant_get_child_value")?,
            g_variant_n_children: load_sym(lib, c"g_variant_n_children")?,
            g_variant_get_string: load_sym(lib, c"g_variant_get_string")?,
            g_variant_get_int32: load_sym(lib, c"g_variant_get_int32")?,
            g_variant_type_checked_: load_sym(lib, c"g_variant_type_checked_")?,
            g_error_free: load_sym(lib, c"g_error_free")?,
        })
    })
    .as_ref()
    .map_err(String::as_str)
}

pub fn gobj() -> Result<&'static GObjApi, &'static str> {
    GOBJ.get_or_init(|| unsafe {
        let lib = dl_open_any(&["libgobject-2.0.so.0", "libgobject-2.0.so"]);
        if lib.is_null() {
            return Err("failed dlopen libgobject-2.0.so.0".into());
        }
        Ok(GObjApi {
            g_object_unref: load_sym(lib, c"g_object_unref")?,
            g_object_ref: load_sym(lib, c"g_object_ref")?,
        })
    })
    .as_ref()
    .map_err(String::as_str)
}

pub fn gio() -> Result<&'static GioApi, &'static str> {
    GIO.get_or_init(|| unsafe {
        let lib = dl_open_any(&["libgio-2.0.so.0", "libgio-2.0.so"]);
        if lib.is_null() {
            return Err("failed dlopen libgio-2.0.so.0".into());
        }
        Ok(GioApi {
            g_bus_own_name: load_sym(lib, c"g_bus_own_name")?,
            g_bus_watch_name: load_sym(lib, c"g_bus_watch_name")?,
            g_bus_unown_name: load_sym(lib, c"g_bus_unown_name")?,
            g_bus_unwatch_name: load_sym(lib, c"g_bus_unwatch_name")?,
            g_dbus_node_info_new_for_xml: load_sym(lib, c"g_dbus_node_info_new_for_xml")?,
            g_dbus_node_info_unref: load_sym(lib, c"g_dbus_node_info_unref")?,
            g_dbus_connection_register_object: load_sym(
                lib,
                c"g_dbus_connection_register_object",
            )?,
            g_dbus_connection_unregister_object: load_sym(
                lib,
                c"g_dbus_connection_unregister_object",
            )?,
            g_dbus_connection_emit_signal: load_sym(lib, c"g_dbus_connection_emit_signal")?,
            g_dbus_connection_call: load_sym(lib, c"g_dbus_connection_call")?,
            g_dbus_method_invocation_return_value: load_sym(
                lib,
                c"g_dbus_method_invocation_return_value",
            )?,
        })
    })
    .as_ref()
    .map_err(String::as_str)
}

pub fn cstr(s: &str) -> CString {
    if let Ok(cs) = CString::new(s) {
        return cs;
    }
    let clean: String = s.chars().map(|c| if c == '\0' { ' ' } else { c }).collect();
    CString::new(clean).unwrap_or_else(|_| CString::new("").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dlopen_glib_gobj_gio_succeeds() {
        assert!(glib().is_ok());
        assert!(gobj().is_ok());
        assert!(gio().is_ok());
    }
}
