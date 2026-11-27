// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::MetadataExt;

pub mod pa_abi;
pub mod pa_failover;

pub const RTLD_LAZY: c_int = 0x00001;
pub const RTLD_NOW: c_int = 0x00002;
pub const RTLD_NOLOAD: c_int = 0x00004;
pub const RTLD_LOCAL: c_int = 0;
pub const RTLD_DEFAULT: *mut c_void = ptr::null_mut();
pub const RTLD_NEXT: *mut c_void = (-1isize) as *mut c_void;

pub type DlopenFn = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DlInfo {
    pub dli_fname: *const c_char,
    pub dli_fbase: *mut c_void,
    pub dli_sname: *const c_char,
    pub dli_saddr: *mut c_void,
}

extern "C" {
    pub fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    pub fn dlclose(handle: *mut c_void) -> c_int;
    pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    pub fn dlvsym(
        handle: *mut c_void,
        symbol: *const c_char,
        version: *const c_char,
    ) -> *mut c_void;
    pub fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
}

/// Returns the `(st_dev, st_ino)` identity for the resolved target of `path`.
pub fn path_dev_ino(path: &str) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// Resolves the filesystem path of the shared object containing `addr` via `dladdr`.
pub unsafe fn self_so_path(addr: *const c_void) -> Option<String> {
    if addr.is_null() {
        return None;
    }
    let mut dli: DlInfo = core::mem::zeroed();
    if dladdr(addr, &raw mut dli) == 0 || dli.dli_fname.is_null() {
        return None;
    }
    let s = CStr::from_ptr(dli.dli_fname).to_str().ok()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Resolves `(st_dev, st_ino)` of the shared object containing `addr`.
pub unsafe fn self_so_dev_ino(addr: *const c_void) -> Option<(u64, u64)> {
    let path = self_so_path(addr)?;
    path_dev_ino(&path)
}

/// Filters and canonicalizes host `.so` candidate paths while skipping any path
/// whose resolved `(st_dev, st_ino)` matches `self_dev_ino`.
pub fn select_verified_host_candidates(
    candidates: &[&str],
    self_dev_ino: Option<(u64, u64)>,
) -> Vec<String> {
    let mut out = Vec::new();
    for &cand in candidates {
        let trimmed = cand.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((dev, ino)) = path_dev_ino(trimmed) else {
            continue;
        };
        if let Some((s_dev, s_ino)) = self_dev_ino {
            if dev == s_dev && ino == s_ino {
                continue;
            }
        }
        let resolved = fs::canonicalize(trimmed)
            .ok()
            .and_then(|p| p.into_os_string().into_string().ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| trimmed.to_string());
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    out
}

/// Opens the first verified host `.so` from `candidates` that:
/// 1. Does not match `self_anchor`'s `(st_dev, st_ino)` identity,
/// 2. Contains `verify_symbol`, and
/// 3. Resolves `verify_symbol` to a different address than `self_symbol_addr`.
pub unsafe fn open_verified_host_lib(
    candidates: &[&str],
    self_anchor: *const c_void,
    verify_symbol: &CStr,
    self_symbol_addr: *const c_void,
    custom_dlopen: Option<DlopenFn>,
    flags: c_int,
) -> *mut c_void {
    let self_dev_ino = self_so_dev_ino(self_anchor);
    let open_fn = custom_dlopen.unwrap_or(dlopen);
    for resolved in select_verified_host_candidates(candidates, self_dev_ino) {
        let Ok(cpath) = CString::new(resolved) else {
            continue;
        };
        let h = open_fn(cpath.as_ptr(), flags);
        if h.is_null() {
            continue;
        }
        let sym = dlsym(h, verify_symbol.as_ptr());
        if !sym.is_null() && sym != self_symbol_addr.cast_mut() {
            return h;
        }
        dlclose(h);
    }
    ptr::null_mut()
}

pub const HOST_LIBASOUND_CANDIDATES: &[&str] = &[
    "/usr/lib/libasound.so.2.0.0",
    "/usr/lib/libasound.so.2",
    "/usr/lib64/libasound.so.2.0.0",
    "/usr/lib64/libasound.so.2",
    "/usr/lib/x86_64-linux-gnu/libasound.so.2.0.0",
    "/usr/lib/x86_64-linux-gnu/libasound.so.2",
    "/usr/lib/aarch64-linux-gnu/libasound.so.2.0.0",
    "/usr/lib/aarch64-linux-gnu/libasound.so.2",
    "/lib/x86_64-linux-gnu/libasound.so.2",
    "/lib/aarch64-linux-gnu/libasound.so.2",
    "/lib64/libasound.so.2",
    "/lib/libasound.so.2",
];

#[cfg(target_arch = "x86")]
pub const HOST_LIBPULSE_CANDIDATES: &[&str] = &[
    "/usr/lib32/libpulse.so.0",
    "/usr/lib/i386-linux-gnu/libpulse.so.0",
    "/lib/i386-linux-gnu/libpulse.so.0",
];

#[cfg(not(target_arch = "x86"))]
pub const HOST_LIBPULSE_CANDIDATES: &[&str] = &[
    "/usr/lib/libpulse.so.0",
    "/usr/lib64/libpulse.so.0",
    "/usr/lib/x86_64-linux-gnu/libpulse.so.0",
    "/lib/x86_64-linux-gnu/libpulse.so.0",
    "/usr/lib/aarch64-linux-gnu/libpulse.so.0",
    "/lib/aarch64-linux-gnu/libpulse.so.0",
];

/// Opens the verified host `libasound.so.2` (skipping self-inode collisions).
pub unsafe fn open_real_host_libasound(
    self_anchor: *const c_void,
    self_snd_pcm_open: *const c_void,
) -> *mut c_void {
    open_verified_host_lib(
        HOST_LIBASOUND_CANDIDATES,
        self_anchor,
        c"snd_pcm_open",
        self_snd_pcm_open,
        None,
        RTLD_LAZY | RTLD_LOCAL,
    )
}

/// Opens the verified host `libpulse.so.0` (checking optional compile-time path first,
/// skipping self-inode and self-symbol collisions).
pub unsafe fn open_real_host_libpulse(
    compiled_host: &str,
    self_anchor: *const c_void,
    self_pa_context_new: *const c_void,
    custom_dlopen: Option<DlopenFn>,
) -> *mut c_void {
    let mut cands: Vec<&str> = Vec::with_capacity(HOST_LIBPULSE_CANDIDATES.len() + 1);
    if !compiled_host.trim().is_empty() {
        cands.push(compiled_host.trim());
    }
    cands.extend_from_slice(HOST_LIBPULSE_CANDIDATES);
    open_verified_host_lib(
        &cands,
        self_anchor,
        c"pa_context_new",
        self_pa_context_new,
        custom_dlopen,
        RTLD_LAZY | RTLD_LOCAL,
    )
}

/// Caches and resolves a symbol from a lazily-loaded library handle.
pub unsafe fn resolve_sym_cached(
    slot: &AtomicPtr<c_void>,
    lib_getter: fn() -> *mut c_void,
    name: &CStr,
) -> *mut c_void {
    let cur = slot.load(Ordering::Acquire);
    if !cur.is_null() {
        return cur;
    }
    let dl = lib_getter();
    if dl.is_null() {
        return ptr::null_mut();
    }
    let sym = dlsym(dl, name.as_ptr());
    if !sym.is_null() {
        slot.store(sym, Ordering::Release);
    }
    sym
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn host_candidate_selection_rejects_self_dev_ino_and_follows_distinct() {
        let dir = format!("/tmp/salsa_host_dl_test_{}", std::process::id());
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let self_so = format!("{dir}/self.so");
        let self_link = format!("{dir}/self_link.so");
        let other_so = format!("{dir}/other.so");
        fs::write(&self_so, b"self").unwrap();
        fs::write(&other_so, b"other").unwrap();
        symlink(&self_so, &self_link).unwrap();

        let self_id = path_dev_ino(&self_so).expect("self dev/ino");
        let missing = format!("{dir}/nonexistent.so");
        let picked = select_verified_host_candidates(
            &["", &missing, &self_so, &self_link, &other_so],
            Some(self_id),
        );
        let other_canon = fs::canonicalize(&other_so)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap();
        assert_eq!(picked, vec![other_canon]);

        let _ = fs::remove_dir_all(&dir);
    }
}
