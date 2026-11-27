// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use salsa_host_dl::{
    dlsym, open_real_host_libpulse as host_open_real_libpulse, DlopenFn, RTLD_DEFAULT,
};

#[path = "../../salsa_pa_reconnect/src/imp.rs"]
mod imp;

extern "C" {
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
    let passthrough = dlsym(RTLD_DEFAULT, c"salsa_real_dlopen_passthrough".as_ptr());
    let custom_dlopen = if !passthrough.is_null() {
        Some(core::mem::transmute::<*mut c_void, DlopenFn>(passthrough))
    } else {
        None
    };
    let compiled_host = option_env!("SALSA_HOST_LIBPULSE").unwrap_or("");
    host_open_real_libpulse(
        compiled_host,
        open_real_host_libpulse as *const c_void,
        imp::pa_context_new as *const c_void,
        custom_dlopen,
    )
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
