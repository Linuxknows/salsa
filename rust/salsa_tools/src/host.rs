// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::cmp::Ordering;
use core::ffi::c_int;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use salsa_wire::unix_sys::{
    current_pid, current_uid, ensure_owned_dir_0700, ensure_private_dir, socket_peercred,
    user_home_dir, O_CLOEXEC, O_NOFOLLOW,
};

pub const LOCK_EX: c_int = 2;
pub const LOCK_NB: c_int = 4;
pub const LOCK_UN: c_int = 8;
pub const S_IFMT: u32 = 0o170000;
pub const S_IFSOCK: u32 = 0o140000;

extern "C" {
    fn flock(fd: c_int, operation: c_int) -> c_int;
}

pub fn flock_fd(fd: RawFd, nonblocking: bool) -> io::Result<()> {
    let op = if nonblocking { LOCK_EX | LOCK_NB } else { LOCK_EX };
    let rc = unsafe { flock(fd, op) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn funlock_fd(fd: RawFd) {
    unsafe {
        let _ = flock(fd, LOCK_UN);
    }
}

pub fn getuid() -> u32 {
    current_uid()
}

pub fn getpid() -> u32 {
    current_pid()
}

pub fn install_root() -> PathBuf {
    if let Ok(over) = env::var("SALSA_BASE_DIR") {
        let p = PathBuf::from(&over);
        if p.join("rust").join("Cargo.toml").is_file() {
            return p;
        }
    }
    if let Ok(exe) = fs::read_link("/proc/self/exe") {
        let mut cur = exe.parent();
        while let Some(dir) = cur {
            if dir.join("rust").join("Cargo.toml").is_file() {
                return dir.to_path_buf();
            }
            cur = dir.parent();
        }
    }
    if let Ok(cwd) = env::current_dir() {
        let mut cur = Some(cwd.as_path());
        while let Some(dir) = cur {
            if dir.join("rust").join("Cargo.toml").is_file() {
                return dir.to_path_buf();
            }
            cur = dir.parent();
        }
        return cwd;
    }
    PathBuf::from(".")
}

pub fn xdg_runtime_dir() -> PathBuf {
    if let Ok(v) = env::var("XDG_RUNTIME_DIR") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    PathBuf::from(format!("/run/user/{}", getuid()))
}

pub fn private_runtime_dir() -> PathBuf {
    if let Some(p) = ensure_private_dir() {
        return PathBuf::from(p);
    }
    let uid = getuid();
    let fb = PathBuf::from(format!("/tmp/lwasapi-{uid}"));
    if ensure_owned_dir_0700(&fb.to_string_lossy(), uid) {
        return fb;
    }
    let pid_fb = PathBuf::from(format!("/tmp/lwasapi-{uid}-{}", getpid()));
    let _ = ensure_owned_dir_0700(&pid_fb.to_string_lossy(), uid);
    pid_fb
}

pub fn state_dir() -> PathBuf {
    if let Ok(cfg) = env::var("XDG_CONFIG_HOME") {
        if cfg.starts_with('/') {
            return PathBuf::from(cfg).join("lwasapi");
        }
    }
    PathBuf::from(user_home_dir()).join(".config").join("lwasapi")
}

pub fn home_dir() -> PathBuf {
    PathBuf::from(user_home_dir())
}

pub fn ensure_dir_0700(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let uid = getuid();
    let s = path.to_string_lossy();
    if ensure_owned_dir_0700(&s, uid) {
        return Ok(());
    }
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing unsafe or unowned directory: {}", path.display()),
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && parent != Path::new("/") {
                    match fs::symlink_metadata(parent) {
                        Ok(pm) => {
                            if pm.file_type().is_symlink() {
                                let target_meta = fs::metadata(parent)?;
                                if !target_meta.is_dir() || (pm.uid() != 0 && pm.uid() != uid) {
                                    return Err(io::Error::new(
                                        io::ErrorKind::PermissionDenied,
                                        format!("refusing symlinked parent: {}", parent.display()),
                                    ));
                                }
                            } else if !pm.is_dir() {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    format!("parent is not a directory: {}", parent.display()),
                                ));
                            }
                        }
                        Err(pe) if pe.kind() == io::ErrorKind::NotFound => {
                            ensure_dir_0700(parent)?;
                        }
                        Err(pe) => return Err(pe),
                    }
                }
            }
            if ensure_owned_dir_0700(&s, uid) {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("failed to create owned 0700 dir: {}", path.display()),
                ))
            }
        }
        Err(e) => Err(e),
    }
}

pub fn write_atomic_secure(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    use core::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = PathBuf::from(format!(
        "{}.tmp.{}.{}.{}",
        path.to_string_lossy(),
        getpid(),
        stamp,
        seq
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(O_NOFOLLOW | O_CLOEXEC)
        .open(&tmp)?;
    let write_res = (|| -> io::Result<()> {
        f.write_all(data)?;
        f.sync_all()?;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        Ok(())
    })();
    drop(f);
    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

pub fn which(cmd: &str) -> Option<PathBuf> {
    let p = Path::new(cmd);
    if cmd.contains('/') {
        return is_executable_file(p).then(|| p.to_path_buf());
    }
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let cand = dir.join(cmd);
        if is_executable_file(&cand) {
            return Some(cand);
        }
    }
    None
}

fn is_executable_file(p: &Path) -> bool {
    let Ok(meta) = fs::metadata(p) else {
        return false;
    };
    meta.is_file() && (meta.mode() & 0o111) != 0
}

pub fn find_cargo() -> Option<PathBuf> {
    which("cargo").or_else(|| {
        let p = home_dir().join(".cargo/bin/cargo");
        is_executable_file(&p).then_some(p)
    })
}

pub fn find_host_lib(libname: &str) -> Option<String> {
    if let Ok(out) = Command::new("ldconfig")
        .arg("-p")
        .stderr(Stdio::null())
        .output()
    {
        if out.status.success() {
            let txt = String::from_utf8_lossy(&out.stdout);
            for line in txt.lines() {
                if line.contains("lib32") {
                    continue;
                }
                let first = line.split_whitespace().next().unwrap_or("");
                if first == libname {
                    if let Some((_, rhs)) = line.split_once("=>") {
                        let p = rhs.trim();
                        if Path::new(p).exists() {
                            return Some(p.to_string());
                        }
                    }
                }
            }
        }
    }
    for dir in [
        "/usr/lib",
        "/usr/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib",
        "/lib64",
    ] {
        let p = format!("{dir}/{libname}");
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    None
}

pub fn find_host_lib32(libname: &str) -> Option<String> {
    if let Ok(out) = Command::new("ldconfig")
        .arg("-p")
        .stderr(Stdio::null())
        .output()
    {
        if out.status.success() {
            let txt = String::from_utf8_lossy(&out.stdout);
            for line in txt.lines() {
                let first = line.split_whitespace().next().unwrap_or("");
                if first != libname {
                    continue;
                }
                let is_32 = line.contains("lib32")
                    || line.contains("i386")
                    || (!line.contains("x86-64")
                        && !line.contains("64bit")
                        && !line.contains("AArch64")
                        && !line.contains("aarch64"));
                if !is_32 {
                    continue;
                }
                if let Some((_, rhs)) = line.split_once("=>") {
                    let p = rhs.trim();
                    if Path::new(p).exists() {
                        return Some(p.to_string());
                    }
                }
            }
        }
    }
    for dir in [
        "/usr/lib32",
        "/usr/lib/i386-linux-gnu",
        "/lib/i386-linux-gnu",
    ] {
        let p = format!("{dir}/{libname}");
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    None
}

pub fn detect_pkg_db_path() -> &'static str {
    if Path::new("/var/lib/pacman/local").is_dir() {
        "/var/lib/pacman/local"
    } else if Path::new("/var/lib/dpkg/status").is_file() {
        "/var/lib/dpkg/status"
    } else if Path::new("/var/lib/rpm").is_dir() {
        "/var/lib/rpm"
    } else {
        "/usr/lib"
    }
}

pub fn read_text_lossy(path: &Path, max_bytes: usize) -> String {
    let Ok(mut f) = File::open(path) else {
        return String::new();
    };
    let mut buf = vec![0u8; max_bytes];
    let mut total = 0usize;
    while total < max_bytes {
        match f.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    buf.truncate(total);
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn host_os_pretty_name() -> String {
    let txt = read_text_lossy(Path::new("/etc/os-release"), 8192);
    for line in txt.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return rest.trim_matches('"').to_string();
        }
    }
    Command::new("uname")
        .arg("-sr")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Linux".to_string())
}

pub fn host_cpu_name() -> String {
    let txt = read_text_lossy(Path::new("/proc/cpuinfo"), 16384);
    for line in txt.lines() {
        if line.starts_with("model name") || line.starts_with("Processor") {
            if let Some((_, rhs)) = line.split_once(':') {
                let s = rhs.trim();
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    host_arch()
}

pub fn host_arch() -> String {
    std::env::consts::ARCH.to_string()
}

pub fn uname_srm() -> String {
    Command::new("uname")
        .arg("-srm")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Linux {}", host_arch()))
}

pub fn natural_version_cmp(a: &str, b: &str) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut ia = 0usize;
    let mut ib = 0usize;

    while ia < ab.len() && ib < bb.len() {
        let ca = ab[ia];
        let cb = bb[ib];
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let mut ea = ia;
            while ea < ab.len() && ab[ea].is_ascii_digit() {
                ea += 1;
            }
            let mut eb = ib;
            while eb < bb.len() && bb[eb].is_ascii_digit() {
                eb += 1;
            }
            let mut sa = ia;
            while sa < ea && ab[sa] == b'0' {
                sa += 1;
            }
            let mut sb = ib;
            while sb < eb && bb[sb] == b'0' {
                sb += 1;
            }
            let da = &ab[sa..ea];
            let db = &bb[sb..eb];
            match da.len().cmp(&db.len()).then_with(|| da.cmp(db)) {
                Ordering::Equal => {}
                ord => return ord,
            }
            // Equal numeric values: more leading zeros sorts slightly later in Python/GNU fallback unless equal length
            match (ea - ia).cmp(&(eb - ib)) {
                Ordering::Equal => {}
                ord => return ord,
            }
            ia = ea;
            ib = eb;
        } else {
            let ra = if ca.is_ascii_digit() { 0u16 } else { u16::from(ca) + 1 };
            let rb = if cb.is_ascii_digit() { 0u16 } else { u16::from(cb) + 1 };
            match ra.cmp(&rb) {
                Ordering::Equal => {
                    ia += 1;
                    ib += 1;
                }
                ord => return ord,
            }
        }
    }
    ab.len().cmp(&bb.len())
}

pub fn sha256_bytes(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        let vals = [a, b, c, d, e, f, g, hh];
        for (dst, v) in h.iter_mut().zip(vals) {
            *dst = dst.wrapping_add(v);
        }
    }

    let mut out = String::with_capacity(64);
    for word in h {
        use std::fmt::Write as _;
        let _ = write!(out, "{word:08x}");
    }
    out
}

pub fn sha256_file(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(sha256_bytes(&bytes))
}

#[derive(Debug, Clone)]
pub struct PeerCredInfo {
    pub path: String,
    pub exists: bool,
    pub is_socket: bool,
    pub mode_oct: Option<String>,
    pub connectable: bool,
    pub peer_pid: Option<i32>,
    pub peer_uid: Option<u32>,
    pub peer_gid: Option<u32>,
    pub peer_comm: String,
    pub error: String,
}

pub fn peercred_for_unix_socket(path: &Path) -> PeerCredInfo {
    let p_str = path.to_string_lossy().into_owned();
    let mut info = PeerCredInfo {
        path: p_str,
        exists: path.exists(),
        is_socket: false,
        mode_oct: None,
        connectable: false,
        peer_pid: None,
        peer_uid: None,
        peer_gid: None,
        peer_comm: String::new(),
        error: String::new(),
    };
    if !info.exists {
        return info;
    }
    match fs::metadata(path) {
        Ok(st) => {
            let mode = st.mode();
            info.is_socket = (mode & S_IFMT) == S_IFSOCK;
            info.mode_oct = Some(format!("0o{:o}", mode & 0o777));
        }
        Err(e) => {
            info.error = e.to_string();
            return info;
        }
    }

    match UnixStream::connect(path) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
            info.connectable = true;
            if let Some(uc) = socket_peercred(stream.as_raw_fd()) {
                info.peer_pid = Some(uc.pid);
                info.peer_uid = Some(uc.uid);
                info.peer_gid = Some(uc.gid);
                if uc.pid > 0 {
                    info.peer_comm =
                        read_text_lossy(Path::new(&format!("/proc/{}/comm", uc.pid)), 256)
                            .trim()
                            .to_string();
                }
            }
            let _ = stream.shutdown(Shutdown::Both);
        }
        Err(e) if e.raw_os_error() == Some(91) => {
            // SOCK_DGRAM socket (e.g. debug-in.sock): verify reachability via UnixDatagram::connect.
            match UnixDatagram::unbound().and_then(|dgram| dgram.connect(path).map(|()| dgram)) {
                Ok(dgram) => {
                    info.connectable = true;
                    if let Some(uc) = socket_peercred(dgram.as_raw_fd()) {
                        info.peer_pid = Some(uc.pid);
                        info.peer_uid = Some(uc.uid);
                        info.peer_gid = Some(uc.gid);
                        if uc.pid > 0 {
                            info.peer_comm =
                                read_text_lossy(Path::new(&format!("/proc/{}/comm", uc.pid)), 256)
                                    .trim()
                                    .to_string();
                        }
                    }
                }
                Err(de) => {
                    info.error = de.to_string();
                }
            }
        }
        Err(e) => {
            info.error = e.to_string();
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_nist_abc() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn natural_sort_alsa_versions() {
        let mut vers = vec![
            "ALSA_1.2.10",
            "ALSA_0.9.0rc8",
            "ALSA_0.9.3",
            "ALSA_0.9",
            "ALSA_1.2.9",
            "ALSA_0.9.0rc4",
            "ALSA_0.9.0",
        ];
        vers.sort_by(|a, b| natural_version_cmp(a, b));
        assert_eq!(
            vers,
            vec![
                "ALSA_0.9",
                "ALSA_0.9.0",
                "ALSA_0.9.0rc4",
                "ALSA_0.9.0rc8",
                "ALSA_0.9.3",
                "ALSA_1.2.9",
                "ALSA_1.2.10",
            ]
        );
    }
}
