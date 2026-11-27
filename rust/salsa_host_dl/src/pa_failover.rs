// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, CStr};
use core::mem;
use core::ptr;
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

const SALSA_O_NONBLOCK: c_int = 0x800;
const SALSA_O_NOFOLLOW: c_int = 0x20000;
const SALSA_O_CLOEXEC: c_int = 0x80000;
const ERANGE: c_int = 34;

#[repr(C)]
struct Passwd {
    pw_name: *mut c_char,
    pw_passwd: *mut c_char,
    pw_uid: u32,
    pw_gid: u32,
    pw_gecos: *mut c_char,
    pw_dir: *mut c_char,
    pw_shell: *mut c_char,
}

extern "C" {
    fn getuid() -> u32;
    fn getpwuid_r(
        uid: u32,
        pwd: *mut Passwd,
        buf: *mut c_char,
        buflen: usize,
        result: *mut *mut Passwd,
    ) -> c_int;
}

/// Validates that `dir` is an existing non-symlink directory owned by `uid` with safe permissions.
fn valid_owned_dir(dir: &str, uid: u32, group_other_mask: u32) -> bool {
    if !dir.starts_with('/') || dir == "/tmp" {
        return false;
    }
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return false;
    };
    meta.is_dir() && meta.uid() == uid && (meta.mode() & group_other_mask) == 0
}

/// Resolves the home directory for `uid` via thread-safe POSIX `getpwuid_r`.
pub fn pw_home_dir_r(uid: u32) -> Option<String> {
    let mut cap = 1024usize;
    while cap <= 65_536 {
        let mut buf = vec![0u8; cap];
        let mut pwd: Passwd = unsafe { mem::zeroed() };
        let mut res: *mut Passwd = ptr::null_mut();
        let rc = unsafe {
            getpwuid_r(
                uid,
                &raw mut pwd,
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len(),
                &raw mut res,
            )
        };
        if rc == 0 {
            if res.is_null() || pwd.pw_dir.is_null() {
                return None;
            }
            let s = unsafe { CStr::from_ptr(pwd.pw_dir) }
                .to_str()
                .ok()?
                .trim()
                .to_string();
            return (!s.is_empty()).then_some(s);
        }
        if rc != ERANGE {
            return None;
        }
        cap *= 2;
    }
    None
}

/// Resolves the verified home directory for `uid`, preferring a valid `$HOME` env override
/// and falling back to thread-safe `getpwuid_r`.
pub fn resolve_uid_home_dir(uid: u32) -> Option<String> {
    if let Ok(env_home) = std::env::var("HOME") {
        if valid_owned_dir(&env_home, uid, 0o022) {
            return Some(env_home);
        }
    }
    let pw = pw_home_dir_r(uid)?;
    if valid_owned_dir(&pw, uid, 0o022) {
        Some(pw)
    } else {
        None
    }
}

pub fn parse_salsa_tcp_port_str(s: &str) -> Option<u16> {
    let port: u16 = s.trim().parse().ok()?;
    (port > 0).then_some(port)
}

pub fn server_string_allows_tcp_fallback(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.split_whitespace().all(|tok| {
        let entry = match (tok.strip_prefix('{'), tok.find('}')) {
            (Some(rest), Some(_)) => rest.split_once('}').map_or(tok, |(_, after)| after),
            _ => tok,
        };
        entry.starts_with("unix:") || entry.starts_with('/')
    })
}

/// Reads and security-verifies `<home>/.config/pulse/{salsa-tcp-port,cookie}`.
pub fn read_salsa_tcp_target_in_home(home: &str, uid: u32) -> Option<(CString, CString)> {
    if !valid_owned_dir(home, uid, 0o022) {
        return None;
    }
    let pulse_dir = format!("{home}/.config/pulse");
    if !valid_owned_dir(&pulse_dir, uid, 0o077) {
        return None;
    }

    let port_path = format!("{pulse_dir}/salsa-tcp-port");
    let mut pf = OpenOptions::new()
        .read(true)
        .custom_flags(SALSA_O_NOFOLLOW | SALSA_O_NONBLOCK | SALSA_O_CLOEXEC)
        .open(&port_path)
        .ok()?;
    let pmeta = pf.metadata().ok()?;
    if !pmeta.is_file() || pmeta.uid() != uid || (pmeta.mode() & 0o077) != 0 || pmeta.len() > 64 {
        return None;
    }
    let mut pbuf = [0u8; 64];
    let n = pf.read(&mut pbuf).ok()?;
    let raw = core::str::from_utf8(&pbuf[..n]).ok()?;
    let port = parse_salsa_tcp_port_str(raw)?;
    let srv = CString::new(format!("tcp4:127.0.0.1:{port}")).ok()?;

    let cookie_path = format!("{pulse_dir}/cookie");
    let cf = OpenOptions::new()
        .read(true)
        .custom_flags(SALSA_O_NOFOLLOW | SALSA_O_NONBLOCK | SALSA_O_CLOEXEC)
        .open(&cookie_path)
        .ok()?;
    let cmeta = cf.metadata().ok()?;
    if !cmeta.is_file()
        || cmeta.uid() != uid
        || (cmeta.mode() & 0o077) != 0
        || cmeta.len() != 256
    {
        return None;
    }
    let ck = CString::new(cookie_path).ok()?;
    Some((srv, ck))
}

/// Discovers the verified loopback PulseAudio TCP target (`tcp4:127.0.0.1:<port>`)
/// and 256-byte cookie path for the calling UID.
pub fn read_salsa_tcp_target() -> Option<(CString, CString)> {
    let uid = unsafe { getuid() };
    let home = resolve_uid_home_dir(uid)?;
    read_salsa_tcp_target_in_home(&home, uid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    fn getpwuid_r_resolves_current_uid_home_even_when_home_env_unset() {
        let _g = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let uid = unsafe { getuid() };
        let pw_home = pw_home_dir_r(uid).expect("getpwuid_r returns passwd entry for current uid");
        assert!(pw_home.starts_with('/'));

        let orig = std::env::var_os("HOME");
        std::env::remove_var("HOME");
        let resolved = resolve_uid_home_dir(uid);
        match orig {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        assert_eq!(resolved.as_deref(), Some(pw_home.as_str()));
    }

    #[test]
    fn tcp_target_reads_valid_0700_0600_files_and_rejects_symlinks_and_bad_cookie() {
        let uid = unsafe { getuid() };
        let base = format!("/tmp/salsa_tcp_target_test_{}", std::process::id());
        let _ = fs::remove_dir_all(&base);
        let pulse = format!("{base}/.config/pulse");
        fs::create_dir_all(&pulse).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(format!("{base}/.config"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&pulse, fs::Permissions::from_mode(0o700)).unwrap();

        let port_path = format!("{pulse}/salsa-tcp-port");
        let cookie_path = format!("{pulse}/cookie");
        fs::write(&port_path, b"47199\n").unwrap();
        fs::write(&cookie_path, [0x42u8; 256]).unwrap();
        fs::set_permissions(&port_path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&cookie_path, fs::Permissions::from_mode(0o600)).unwrap();

        let (srv, ck) = read_salsa_tcp_target_in_home(&base, uid).expect("valid tcp target");
        assert_eq!(srv.to_str().unwrap(), "tcp4:127.0.0.1:47199");
        assert_eq!(ck.to_str().unwrap(), cookie_path);

        // Wrong cookie size must be rejected
        fs::write(&cookie_path, [0x42u8; 128]).unwrap();
        assert!(read_salsa_tcp_target_in_home(&base, uid).is_none());

        // Group/world-readable port file must be rejected
        fs::write(&cookie_path, [0x42u8; 256]).unwrap();
        fs::set_permissions(&port_path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_salsa_tcp_target_in_home(&base, uid).is_none());

        // Symlinked salsa-tcp-port must be rejected
        fs::remove_file(&port_path).unwrap();
        let target_real = format!("{base}/real_port");
        fs::write(&target_real, b"47199\n").unwrap();
        symlink(&target_real, &port_path).unwrap();
        assert!(read_salsa_tcp_target_in_home(&base, uid).is_none());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn server_string_policy_allows_only_local_unix() {
        assert!(server_string_allows_tcp_fallback(""));
        assert!(server_string_allows_tcp_fallback(
            "unix:/run/user/1000/pulse/native"
        ));
        assert!(server_string_allows_tcp_fallback(
            "{tag}unix:/run/user/1000/pulse/native /tmp/p"
        ));
        assert!(!server_string_allows_tcp_fallback("tcp:host=evil:4713"));
        assert!(!server_string_allows_tcp_fallback(
            "unix:/tmp/ok tcp4:127.0.0.1:4713"
        ));
    }
}
