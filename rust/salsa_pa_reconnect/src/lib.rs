#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

mod imp;

const RTLD_LAZY: c_int = 0x00001;
const RTLD_NOLOAD: c_int = 0x00004;
const RTLD_NEXT: *mut c_void = (-1isize) as *mut c_void;
const MAX_SELF_PATH: usize = 4096;

#[repr(C)]
struct DlInfo {
    dli_fname: *const c_char,
    dli_fbase: *mut c_void,
    dli_sname: *const c_char,
    dli_saddr: *mut c_void,
}

type DlopenFn = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;

extern "C" {
    fn dlvsym(handle: *mut c_void, symbol: *const c_char, version: *const c_char) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
}

static REAL_DLOPEN_PTR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static SELF_PATH_LEN: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static mut SELF_PATH_BUF: [u8; MAX_SELF_PATH] = [0u8; MAX_SELF_PATH];

unsafe fn resolve_real_dlopen() -> Option<DlopenFn> {
    let cur = REAL_DLOPEN_PTR.load(Ordering::Acquire);
    if !cur.is_null() {
        return Some(core::mem::transmute::<*mut c_void, DlopenFn>(cur));
    }
    let mut fn_ptr = dlvsym(RTLD_NEXT, c"dlopen".as_ptr(), c"GLIBC_2.34".as_ptr());
    if fn_ptr.is_null() {
        fn_ptr = dlvsym(RTLD_NEXT, c"dlopen".as_ptr(), c"GLIBC_2.2.5".as_ptr());
    }
    if fn_ptr.is_null() {
        fn_ptr = dlsym(RTLD_NEXT, c"dlopen".as_ptr());
    }
    if !fn_ptr.is_null() {
        REAL_DLOPEN_PTR.store(fn_ptr, Ordering::Release);
        Some(core::mem::transmute::<*mut c_void, DlopenFn>(fn_ptr))
    } else {
        None
    }
}

pub(crate) unsafe fn resolve_real_syms(real: &mut imp::RealSyms) {
    let _ = resolve_real_dlopen();
    let mut dli = core::mem::zeroed::<DlInfo>();
    if dladdr(resolve_real_syms as *const c_void, &raw mut dli) != 0 && !dli.dli_fname.is_null() {
        let bytes = core::ffi::CStr::from_ptr(dli.dli_fname).to_bytes();
        let n = bytes.len().min(MAX_SELF_PATH - 1);
        let buf = (&raw mut SELF_PATH_BUF).cast::<u8>();
        ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n);
        *buf.add(n) = 0;
        SELF_PATH_LEN.store(n, Ordering::Release);
    }
    real.load_from(RTLD_NEXT);
}

#[no_mangle]
pub unsafe extern "C" fn salsa_real_dlopen_passthrough(
    filename: *const c_char,
    flags: c_int,
) -> *mut c_void {
    if let Some(real_dlopen) = resolve_real_dlopen() {
        real_dlopen(filename, flags)
    } else {
        ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void {
    let _ = resolve_real_dlopen();
    imp::ensure_init();
    if !filename.is_null()
        && !strstr(filename, c"libpulse.so".as_ptr()).is_null()
        && strstr(filename, c"libpulse-simple".as_ptr()).is_null()
        && strstr(filename, c"libpulse-mainloop".as_ptr()).is_null()
        && SELF_PATH_LEN.load(Ordering::Acquire) > 0
    {
        if let Some(real_dlopen) = resolve_real_dlopen() {
            let self_ptr = (&raw const SELF_PATH_BUF).cast::<c_char>();
            let self_h = real_dlopen(self_ptr, RTLD_LAZY | RTLD_NOLOAD);
            if !self_h.is_null() {
                return self_h;
            }
            return real_dlopen(self_ptr, flags);
        }
    }
    if let Some(real_dlopen) = resolve_real_dlopen() {
        real_dlopen(filename, flags)
    } else {
        ptr::null_mut()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reconnect_preload_types_sane() {
        assert_eq!(core::mem::size_of::<super::imp::pa_spawn_api>(), 3 * core::mem::size_of::<usize>());
    }
}
