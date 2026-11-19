#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::MetadataExt;

#[path = "../../salsa_pa_reconnect/src/imp.rs"]
mod imp;

const RTLD_LAZY: c_int = 0x00001;
const RTLD_LOCAL: c_int = 0;
const RTLD_DEFAULT: *mut c_void = ptr::null_mut();

#[repr(C)]
struct DlInfo {
    dli_fname: *const c_char,
    dli_fbase: *mut c_void,
    dli_sname: *const c_char,
    dli_saddr: *mut c_void,
}

type DlopenFn = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    fn fprintf(stream: *mut c_void, fmt: *const c_char, ...) -> c_int;
    static stderr: *mut c_void;
}

#[cfg(not(test))]
extern "C" {
    static salsa_pa_stub_names: *const c_char;
    static mut salsa_pa_stub_targets: *mut c_void;
}

#[cfg(test)]
static DUMMY_STUB_NAMES: [usize; 1] = [0];
#[cfg(test)]
static mut DUMMY_STUB_TARGETS: [usize; 1] = [0];

static REAL_PULSE_H: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

unsafe fn open_real_host_libpulse() -> *mut c_void {
    let mut self_dev_ino: Option<(u64, u64)> = None;
    let mut dli = core::mem::zeroed::<DlInfo>();
    if dladdr(open_real_host_libpulse as *const c_void, &raw mut dli) != 0 && !dli.dli_fname.is_null() {
        if let Ok(fname) = CStr::from_ptr(dli.dli_fname).to_str() {
            if let Ok(meta) = fs::metadata(fname) {
                self_dev_ino = Some((meta.dev(), meta.ino()));
            }
        }
    }

    let passthrough = dlsym(RTLD_DEFAULT, c"salsa_real_dlopen_passthrough".as_ptr());
    let real_dlopen: DlopenFn = if !passthrough.is_null() {
        core::mem::transmute::<*mut c_void, DlopenFn>(passthrough)
    } else {
        dlopen
    };

    let compiled_host = option_env!("SALSA_HOST_LIBPULSE").unwrap_or("");
    #[cfg(target_arch = "x86")]
    let defaults: &[&str] = &[
        "/usr/lib32/libpulse.so.0",
        "/usr/lib/i386-linux-gnu/libpulse.so.0",
        "/lib/i386-linux-gnu/libpulse.so.0",
    ];
    #[cfg(not(target_arch = "x86"))]
    let defaults: &[&str] = &[
        "/usr/lib/libpulse.so.0",
        "/usr/lib64/libpulse.so.0",
        "/usr/lib/x86_64-linux-gnu/libpulse.so.0",
        "/lib/x86_64-linux-gnu/libpulse.so.0",
        "/usr/lib/aarch64-linux-gnu/libpulse.so.0",
        "/lib/aarch64-linux-gnu/libpulse.so.0",
    ];

    for cand in core::iter::once(compiled_host).chain(defaults.iter().copied()) {
        if cand.is_empty() {
            continue;
        }
        let Ok(meta) = fs::metadata(cand) else {
            continue;
        };
        if let Some((dev, ino)) = self_dev_ino {
            if meta.dev() == dev && meta.ino() == ino {
                continue;
            }
        }
        let open_str = fs::canonicalize(cand)
            .ok()
            .and_then(|p| p.into_os_string().into_string().ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cand.to_string());
        let Ok(cpath) = CString::new(open_str) else {
            continue;
        };
        let h = real_dlopen(cpath.as_ptr(), RTLD_LAZY | RTLD_LOCAL);
        if h.is_null() {
            continue;
        }
        let probe = dlsym(h, c"pa_context_new".as_ptr());
        if !probe.is_null() && probe != (imp::pa_context_new as *const c_void).cast_mut() {
            return h;
        }
        dlclose(h);
    }
    ptr::null_mut()
}

pub(crate) unsafe fn resolve_real_syms(real: &mut imp::RealSyms) {
    let real_h = open_real_host_libpulse();
    if real_h.is_null() {
        fprintf(
            stderr,
            c"[salsa-pa-proxy] ERROR: could not locate real host libpulse.so.0\n".as_ptr(),
        );
        return;
    }
    REAL_PULSE_H.store(real_h, Ordering::Release);
    real.load_from(real_h);

    #[cfg(not(test))]
    let (names, targets) = (
        (&raw const salsa_pa_stub_names).cast::<*const c_char>(),
        (&raw mut salsa_pa_stub_targets).cast::<*mut c_void>(),
    );
    #[cfg(test)]
    let (names, targets) = (
        (&raw const DUMMY_STUB_NAMES).cast::<*const c_char>(),
        (&raw mut DUMMY_STUB_TARGETS).cast::<*mut c_void>(),
    );

    let mut i = 0usize;
    while !(*names.add(i)).is_null() {
        *targets.add(i) = dlsym(real_h, *names.add(i));
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pa_proxy_ensure_stubs() {
    imp::ensure_init();
}

#[cfg(test)]
mod tests {
    #[test]
    fn proxy_stub_types_sane() {
        assert_eq!(
            core::mem::size_of::<super::imp::pa_timing_info>(),
            core::mem::size_of::<super::imp::TimeVal>() + 88
        );
    }
}
