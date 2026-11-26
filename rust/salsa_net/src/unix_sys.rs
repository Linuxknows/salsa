// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_int, c_short, c_uint, c_ulong, c_void};
use core::mem::{self, MaybeUninit};
use core::ptr;
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixDatagram, UnixListener};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const POLLIN: c_short = 1;
pub const POLLOUT: c_short = 4;
pub const POLLERR: c_short = 8;
pub const POLLHUP: c_short = 16;
pub const POLLNVAL: c_short = 32;

pub const SOL_SOCKET: c_int = 1;
pub const SO_ERROR: c_int = 4;
pub const SO_SNDBUF: c_int = 7;
pub const SO_PRIORITY: c_int = 12;
pub const SO_PASSCRED: c_int = 16;
pub const SO_PEERCRED: c_int = 17;
pub const SCM_RIGHTS: c_int = 1;
pub const SCM_CREDENTIALS: c_int = 2;
pub const SOCK_SEQPACKET: c_int = 5;
pub const SOCK_NONBLOCK: c_int = 0x800;
pub const SOCK_CLOEXEC: c_int = 0x80000;
pub const SHUT_RDWR: c_int = 2;

pub const MSG_DONTWAIT: c_int = 0x40;
pub const MSG_WAITALL: c_int = 0x100;
pub const MSG_NOSIGNAL: c_int = 0x4000;
pub const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
pub const AF_UNIX: c_int = 1;
pub const SOCK_STREAM: c_int = 1;
pub const SOCK_DGRAM: c_int = 2;

pub const MFD_CLOEXEC: c_uint = 0x1;
pub const MFD_ALLOW_SEALING: c_uint = 0x2;
pub const EFD_NONBLOCK: c_int = 0x800;
pub const EFD_CLOEXEC: c_int = 0x80000;
pub const F_DUPFD_CLOEXEC: c_int = 1030;
pub const F_ADD_SEALS: c_int = 1033;
pub const F_SEAL_SEAL: c_int = 0x0001;
pub const F_SEAL_SHRINK: c_int = 0x0002;
pub const F_SEAL_GROW: c_int = 0x0004;

pub const O_RDONLY: c_int = 0x0;
pub const O_RDWR: c_int = 0x2;
pub const O_CREAT: c_int = 0x40;
pub const O_EXCL: c_int = 0x80;
pub const O_NONBLOCK: c_int = 0x800;
pub const O_NOFOLLOW: c_int = 0x20000;
pub const O_CLOEXEC: c_int = 0x80000;
pub const LOCK_EX: c_int = 2;
pub const LOCK_NB: c_int = 4;
pub const LOCK_UN: c_int = 8;
pub const RTLD_LOCAL: c_int = 0;
pub const RTLD_NOW: c_int = 2;

pub const PROT_READ: c_int = 1;
pub const PROT_WRITE: c_int = 2;
pub const MAP_SHARED: c_int = 1;
pub const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

pub const SCHED_FIFO: c_int = 1;
pub const PRIO_PROCESS: c_int = 0;

pub const DMA_BUF_IOCTL_SYNC: c_ulong = 0x4008_6200;
pub const DMA_BUF_SYNC_READ: u64 = 0x1;
pub const DMA_BUF_SYNC_START: u64 = 0x0;
pub const DMA_BUF_SYNC_END: u64 = 0x4;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PollFd {
    pub fd: c_int,
    pub events: c_short,
    pub revents: c_short,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct UCred {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

#[repr(C)]
struct IoVec {
    iov_base: *mut c_void,
    iov_len: usize,
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
struct MsgHdr {
    msg_name: *mut c_void,
    msg_namelen: u32,
    _pad0: u32,
    msg_iov: *mut IoVec,
    msg_iovlen: usize,
    msg_control: *mut c_void,
    msg_controllen: usize,
    msg_flags: c_int,
    _pad1: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CMsgHdr {
    cmsg_len: usize,
    cmsg_level: c_int,
    cmsg_type: c_int,
}

#[repr(C)]
struct DmaBufSync {
    flags: u64,
}

#[repr(C)]
struct SchedParam {
    sched_priority: c_int,
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
    fn eventfd(initval: c_uint, flags: c_int) -> c_int;
    fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    fn ftruncate(fd: c_int, length: i64) -> c_int;
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
    fn shm_open(name: *const c_char, oflag: c_int, mode: c_uint) -> c_int;
    fn shm_unlink(name: *const c_char) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn socket(domain: c_int, sock_type: c_int, protocol: c_int) -> c_int;
    fn bind(sockfd: c_int, addr: *const c_void, addrlen: u32) -> c_int;
    fn connect(sockfd: c_int, addr: *const c_void, addrlen: u32) -> c_int;
    fn shutdown(sockfd: c_int, how: c_int) -> c_int;
    fn sendmsg(sockfd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
    fn recvmsg(sockfd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn send(sockfd: c_int, buf: *const c_void, len: usize, flags: c_int) -> isize;
    fn recv(sockfd: c_int, buf: *mut c_void, len: usize, flags: c_int) -> isize;
    fn getsockopt(
        sockfd: c_int,
        level: c_int,
        optname: c_int,
        optval: *mut c_void,
        optlen: *mut u32,
    ) -> c_int;
    fn setsockopt(
        sockfd: c_int,
        level: c_int,
        optname: c_int,
        optval: *const c_void,
        optlen: u32,
    ) -> c_int;
    fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn getuid() -> u32;
    fn getpid() -> i32;
    fn gethostname(name: *mut c_char, len: usize) -> c_int;
    fn pthread_self() -> c_ulong;
    fn pthread_setschedparam(thread: c_ulong, policy: c_int, param: *const SchedParam) -> c_int;
    fn setpriority(which: c_int, who: u32, prio: c_int) -> c_int;
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

pub fn ensure_dir_0700_safe(path: &Path) -> io::Result<()> {
    let uid = current_uid();
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
                            ensure_dir_0700_safe(parent)?;
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

pub fn write_atomic_secure_file(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    use core::sync::atomic::{AtomicU64, Ordering};
    use std::io::Write;
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
    let tmp = std::path::PathBuf::from(format!(
        "{}.tmp.{}.{}.{}",
        path.to_string_lossy(),
        current_pid(),
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

/// Reads a bounded UTF-8 regular file safely: rejects symlinks, non-regular files,
/// wrong-UID ownership, and group/world-writable (or optionally readable) permissions.
pub fn read_verified_regular_file_utf8(
    path: &Path,
    max_bytes: usize,
    reject_group_world_read: bool,
) -> io::Result<Option<String>> {
    use std::io::Read;

    let uid = current_uid();
    let lmeta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if lmeta.file_type().is_symlink() || !lmeta.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing non-regular or symlinked file: {}", path.display()),
        ));
    }
    if lmeta.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing unowned file: {}", path.display()),
        ));
    }
    let perm_mask = if reject_group_world_read { 0o077 } else { 0o022 };
    if (lmeta.mode() & perm_mask) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing unsafe permissions on {}", path.display()),
        ));
    }

    let f = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_CLOEXEC)
        .open(path)?;
    let fmeta = f.metadata()?;
    if !fmeta.file_type().is_file() || fmeta.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing changed file metadata on {}", path.display()),
        ));
    }
    if fmeta.len() > max_bytes as u64 {
        return Err(io::Error::other(format!(
            "{} exceeds size limit ({max_bytes} bytes)",
            path.display()
        )));
    }
    let mut buf = Vec::new();
    let n = f.take((max_bytes as u64) + 1).read_to_end(&mut buf)?;
    if n > max_bytes {
        return Err(io::Error::other(format!(
            "{} exceeds size limit ({max_bytes} bytes)",
            path.display()
        )));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid UTF-8: {e}")))
}

#[inline]
pub fn current_uid() -> u32 {
    unsafe { getuid() }
}

#[inline]
pub fn current_pid() -> u32 {
    unsafe { getpid().max(0) as u32 }
}

pub fn mono_ns() -> u64 {
    let ts_now = std::time::Instant::now();
    let _ = ts_now;
    let mut ts = libc_timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    extern "C" {
        fn clock_gettime(clk_id: c_int, tp: *mut libc_timespec) -> c_int;
    }
    const CLOCK_MONOTONIC: c_int = 1;
    unsafe {
        if clock_gettime(CLOCK_MONOTONIC, &mut ts) == 0 {
            return (ts.tv_sec as u64)
                .saturating_mul(1_000_000_000)
                .saturating_add(ts.tv_nsec as u64);
        }
    }
    0
}

#[repr(C)]
struct libc_timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

pub fn realtime_sec_usec() -> (u32, u32) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as u32, d.subsec_micros()),
        Err(_) => (0, 0),
    }
}

pub fn realtime_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64)
            .saturating_mul(1000)
            .saturating_add(i64::from(d.subsec_millis())),
        Err(_) => 0,
    }
}

pub fn enable_realtime(prio: i32, fallback_nice: i32) {
    let sp = SchedParam {
        sched_priority: prio,
    };
    unsafe {
        if pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp) != 0 {
            let _ = setpriority(PRIO_PROCESS, 0, fallback_nice);
        }
    }
}

pub fn try_realtime_fifo(prio: i32) -> bool {
    let sp = SchedParam {
        sched_priority: prio,
    };
    unsafe { pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp) == 0 }
}

pub fn sleep_abs_mono_ns(target_ns: u64) {
    extern "C" {
        fn clock_nanosleep(
            clock_id: c_int,
            flags: c_int,
            request: *const libc_timespec,
            remain: *mut libc_timespec,
        ) -> c_int;
    }
    const CLOCK_MONOTONIC: c_int = 1;
    const TIMER_ABSTIME: c_int = 1;
    let req = libc_timespec {
        tv_sec: (target_ns / 1_000_000_000) as i64,
        tv_nsec: (target_ns % 1_000_000_000) as i64,
    };
    unsafe {
        let _ = clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &req, ptr::null_mut());
    }
}

static TERM_SIGNAL_STOP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_term_signal(_sig: c_int) {
    TERM_SIGNAL_STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub fn install_term_signals() {
    extern "C" {
        fn signal(sig: c_int, handler: usize) -> usize;
    }
    const SIGINT: c_int = 2;
    const SIGPIPE: c_int = 13;
    const SIGTERM: c_int = 15;
    const SIG_IGN: usize = 1;
    TERM_SIGNAL_STOP.store(false, std::sync::atomic::Ordering::SeqCst);
    unsafe {
        let h = on_term_signal as *const () as usize;
        let _ = signal(SIGINT, h);
        let _ = signal(SIGTERM, h);
        let _ = signal(SIGPIPE, SIG_IGN);
    }
}

pub fn reset_sigpipe_default() {
    extern "C" {
        fn signal(sig: c_int, handler: usize) -> usize;
    }
    const SIGPIPE: c_int = 13;
    const SIG_DFL: usize = 0;
    unsafe {
        let _ = signal(SIGPIPE, SIG_DFL);
    }
}

pub fn term_signal_requested() -> bool {
    TERM_SIGNAL_STOP.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_thread_nice(nice_val: i32) {
    unsafe {
        let _ = setpriority(PRIO_PROCESS, 0, nice_val);
    }
}

pub fn do_poll(fds: &mut [PollFd], timeout_ms: i32) -> i32 {
    unsafe { poll(fds.as_mut_ptr(), fds.len() as c_ulong, timeout_ms) }
}

pub fn new_eventfd() -> Option<OwnedFd> {
    let fd = unsafe { eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC) };
    if fd >= 0 {
        Some(unsafe { OwnedFd::from_raw_fd(fd) })
    } else {
        None
    }
}

pub fn eventfd_signal(fd: RawFd) {
    if fd < 0 {
        return;
    }
    let val: u64 = 1;
    let _ = unsafe { write(fd, (&val as *const u64).cast::<c_void>(), 8) };
}

pub fn eventfd_drain(fd: RawFd) {
    if fd < 0 {
        return;
    }
    let mut val: u64 = 0;
    let _ = unsafe { read(fd, (&mut val as *mut u64).cast::<c_void>(), 8) };
}

pub fn create_memfd(name: &str, size: usize) -> Option<OwnedFd> {
    let cname = CString::new(name).ok()?;
    let fd = unsafe { memfd_create(cname.as_ptr(), MFD_CLOEXEC | MFD_ALLOW_SEALING) };
    if fd < 0 {
        return None;
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    if size > 0 && unsafe { ftruncate(owned.as_raw_fd(), size as i64) } < 0 {
        return None;
    }
    Some(owned)
}

pub fn seal_memfd_size(fd: RawFd) {
    if fd >= 0 {
        unsafe {
            let _ = fcntl(fd, F_ADD_SEALS, F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL);
        }
    }
}

pub fn dup_fd_cloexec(fd: RawFd) -> Option<OwnedFd> {
    if fd < 0 {
        return None;
    }
    let nfd = unsafe { fcntl(fd, F_DUPFD_CLOEXEC, 3) };
    if nfd >= 0 {
        Some(unsafe { OwnedFd::from_raw_fd(nfd) })
    } else {
        None
    }
}

pub struct MappedMem {
    ptr: *mut u8,
    len: usize,
}

unsafe impl Send for MappedMem {}

impl MappedMem {
    pub fn map_fd(fd: RawFd, len: usize, writable: bool, offset: u64) -> Option<Self> {
        if fd < 0 || len == 0 {
            return None;
        }
        let prot = if writable {
            PROT_READ | PROT_WRITE
        } else {
            PROT_READ
        };
        let p = unsafe { mmap(ptr::null_mut(), len, prot, MAP_SHARED, fd, offset as i64) };
        if p == MAP_FAILED || p.is_null() {
            None
        } else {
            Some(Self {
                ptr: p.cast::<u8>(),
                len,
            })
        }
    }

    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for MappedMem {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.len > 0 {
            unsafe {
                let _ = munmap(self.ptr.cast::<c_void>(), self.len);
            }
        }
    }
}

pub fn dmabuf_sync(fd: RawFd, start: bool) {
    if fd < 0 {
        return;
    }
    let sync = DmaBufSync {
        flags: if start {
            DMA_BUF_SYNC_START | DMA_BUF_SYNC_READ
        } else {
            DMA_BUF_SYNC_END | DMA_BUF_SYNC_READ
        },
    };
    unsafe {
        let _ = ioctl(fd, DMA_BUF_IOCTL_SYNC, &sync as *const DmaBufSync);
    }
}

pub fn open_private_shm(name: &str, size: usize) -> Option<(OwnedFd, bool)> {
    let cname = CString::new(name).ok()?;
    if name.starts_with('/') && !name[1..].contains('/') {
        unsafe { shm_unlink(cname.as_ptr()) };
        let fd = unsafe { shm_open(cname.as_ptr(), O_CREAT | O_EXCL | O_RDWR | O_CLOEXEC, 0o600) };
        if fd < 0 {
            return None;
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { ftruncate(owned.as_raw_fd(), size as i64) } < 0 {
            unsafe { shm_unlink(cname.as_ptr()) };
            return None;
        }
        Some((owned, true))
    } else {
        let _ = fs::remove_file(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(O_CLOEXEC | O_NOFOLLOW)
            .open(name)
            .ok()?;
        let owned: OwnedFd = file.into();
        if unsafe { ftruncate(owned.as_raw_fd(), size as i64) } < 0 {
            let _ = fs::remove_file(name);
            return None;
        }
        Some((owned, false))
    }
}

pub fn unlink_shm_path(name: &str, is_posix_shm: bool) {
    if name.is_empty() {
        return;
    }
    if is_posix_shm {
        if let Ok(c) = CString::new(name) {
            unsafe { shm_unlink(c.as_ptr()) };
        }
    } else {
        let _ = fs::remove_file(name);
    }
}

pub fn memfd_size_valid(fd: RawFd, mapoffset: u64, maxsize: u64) -> bool {
    if fd < 0 {
        return false;
    }
    // Check via duplicated BorrowedFd or fstat from /proc/self/fd or raw metadata
    // Safety: dup then wrap temporarily so OwnedFd drop doesn't close original fd
    let Some(dup) = dup_fd_cloexec(fd) else {
        return false;
    };
    let f = fs::File::from(dup);
    let Ok(meta) = f.metadata() else {
        return false;
    };
    let st_sz = meta.len();
    st_sz > 0 && mapoffset <= st_sz && maxsize <= st_sz - mapoffset
}

pub fn is_safe_owner_dir(path: &Path, uid: u32, require_private: bool) -> bool {
    let s = match path.to_str() {
        Some(v) if v.starts_with('/') => v,
        _ => return false,
    };
    let _ = s;
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() || meta.uid() != uid {
        return false;
    }
    if require_private && (meta.mode() & 0o077) != 0 {
        return false;
    }
    true
}

pub fn ensure_private_dir() -> Option<String> {
    let uid = current_uid();
    let mut base: Option<String> = None;
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let p = Path::new(&xdg);
        if is_safe_owner_dir(p, uid, true) {
            base = Some(xdg);
        }
    }
    if base.is_none() {
        let fb = format!("/run/user/{uid}");
        let p = Path::new(&fb);
        if is_safe_owner_dir(p, uid, true) {
            base = Some(fb);
        }
    }
    if let Some(b) = base {
        let out = format!("{b}/lasapi");
        if ensure_owned_dir_0700(&out, uid) {
            return Some(out);
        }
    }
    let tmp = format!("/tmp/lasapi-{uid}");
    if ensure_owned_dir_0700(&tmp, uid) {
        return Some(tmp);
    }
    None
}

pub fn ensure_owned_dir_0700(path: &str, uid: u32) -> bool {
    let _ = fs::create_dir(path);
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() || meta.uid() != uid {
        return false;
    }
    if (meta.mode() & 0o777) != 0o700 {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    true
}

pub fn bind_unix_listener_0600(path: &str) -> Option<UnixListener> {
    if path.is_empty() || path.len() >= 108 {
        return None;
    }
    let _ = fs::remove_file(path);
    let tmp = format!("{path}.t{}", std::process::id());
    let listener = if tmp.len() < 108 {
        let _ = fs::remove_file(&tmp);
        let l = UnixListener::bind(&tmp).ok()?;
        let _ = l.set_nonblocking(true);
        if fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).is_err()
            || fs::rename(&tmp, path).is_err()
        {
            let _ = fs::remove_file(&tmp);
            return None;
        }
        l
    } else {
        let l = UnixListener::bind(path).ok()?;
        let _ = l.set_nonblocking(true);
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        l
    };
    Some(listener)
}

const SD_LISTEN_FDS_START: RawFd = 3;
const F_GETFD: c_int = 1;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: c_int = 1;
const SO_TYPE: c_int = 3;
const SO_ACCEPTCONN: c_int = 30;

static SYSTEMD_LISTEN_SOCKETS: std::sync::OnceLock<std::sync::Mutex<Vec<(String, OwnedFd)>>> =
    std::sync::OnceLock::new();

/// Claims any `AF_UNIX` `SOCK_STREAM` listening sockets passed via systemd `LISTEN_FDS` /
/// `LISTEN_PID`, marks them `FD_CLOEXEC` + `O_NONBLOCK`, clears the `LISTEN_*` environment
/// variables so child processes never inherit them, and indexes each fd by its `getsockname()`
/// filesystem path for portable multi-PC lookup.
pub fn claim_systemd_unix_listeners() {
    let _ = SYSTEMD_LISTEN_SOCKETS.get_or_init(|| {
        let mut claimed = Vec::new();
        let pid_ok = std::env::var("LISTEN_PID")
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .is_some_and(|p| p == current_pid());
        let nfds = if pid_ok {
            std::env::var("LISTEN_FDS")
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(0)
                .clamp(0, 32)
        } else {
            0
        };
        std::env::remove_var("LISTEN_PID");
        std::env::remove_var("LISTEN_FDS");
        std::env::remove_var("LISTEN_FDNAMES");

        for i in 0..nfds {
            let fd = SD_LISTEN_FDS_START + i;
            let mut stype: c_int = 0;
            let mut slen = mem::size_of::<c_int>() as u32;
            if unsafe {
                getsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_TYPE,
                    (&mut stype as *mut c_int).cast::<c_void>(),
                    &mut slen,
                )
            } != 0
                || stype != SOCK_STREAM
            {
                continue;
            }
            let mut acc: c_int = 0;
            let mut alen = mem::size_of::<c_int>() as u32;
            if unsafe {
                getsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_ACCEPTCONN,
                    (&mut acc as *mut c_int).cast::<c_void>(),
                    &mut alen,
                )
            } != 0
                || acc == 0
            {
                continue;
            }
            unsafe {
                let flags = fcntl(fd, F_GETFD);
                if flags >= 0 {
                    let _ = fcntl(fd, F_SETFD, flags | FD_CLOEXEC);
                }
            }
            let listener = unsafe { UnixListener::from_raw_fd(fd) };
            let _ = listener.set_nonblocking(true);
            let bound_path = listener
                .local_addr()
                .ok()
                .and_then(|a| a.as_pathname().map(|p| p.to_string_lossy().into_owned()));
            let owned: OwnedFd = listener.into();
            if let Some(p) = bound_path {
                claimed.push((p, owned));
            }
        }
        std::sync::Mutex::new(claimed)
    });
}

pub fn take_systemd_unix_listener(target_path: &str) -> Option<OwnedFd> {
    claim_systemd_unix_listeners();
    let mut guard = SYSTEMD_LISTEN_SOCKETS.get()?.lock().ok()?;
    let idx = guard.iter().position(|(p, _)| p == target_path)?;
    Some(guard.swap_remove(idx).1)
}

pub fn dup_unix_listener_nonblock(fd: RawFd) -> Option<UnixListener> {
    let dup = dup_fd_cloexec(fd)?;
    let listener = UnixListener::from(dup);
    let _ = listener.set_nonblocking(true);
    Some(listener)
}

pub fn socket_peercred(fd: RawFd) -> Option<UCred> {
    let mut uc = UCred::default();
    let mut ulen = mem::size_of::<UCred>() as u32;
    let rc = unsafe {
        getsockopt(
            fd,
            SOL_SOCKET,
            SO_PEERCRED,
            (&mut uc as *mut UCred).cast::<c_void>(),
            &mut ulen,
        )
    };
    if rc == 0 {
        Some(uc)
    } else {
        None
    }
}

pub fn resolve_sock_path_common(env_var: &str, sock_name: &str, fallback: &str) -> String {
    if let Ok(val) = std::env::var(env_var) {
        if !val.is_empty() && val.len() < 108 {
            return val;
        }
    }
    if let Some(dir) = ensure_private_dir() {
        let joined = format!("{dir}/{sock_name}");
        if joined.len() < 108 {
            return joined;
        }
    }
    fallback.to_string()
}

pub fn audiodg_sock_path() -> String {
    resolve_sock_path_common("LASAPI_SOCKET", "audiodg.sock", "/tmp/lasapi-audiodg.sock")
}

pub fn debug_in_sock_path() -> String {
    resolve_sock_path_common(
        "LASAPI_DEBUG_IN_SOCK",
        "debug-in.sock",
        "/tmp/lasapi-debug-in.sock",
    )
}

#[repr(C)]
struct SockAddrUn {
    sun_family: u16,
    sun_path: [u8; 108],
}

pub fn connect_unix_stream_blocking(path: &str) -> Option<OwnedFd> {
    if path.is_empty() || path.len() >= 108 {
        return None;
    }
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return None;
    }
    let stream = std::os::unix::net::UnixStream::connect(path).ok()?;
    let timeout = std::time::Duration::from_secs(5);
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let owned: OwnedFd = stream.into();
    let uc = socket_peercred(owned.as_raw_fd())?;
    let my_uid = current_uid();
    if uc.uid != my_uid && uc.uid != 0 {
        return None;
    }
    Some(owned)
}

pub fn sendto_unix_dgram_nonblock(fd: RawFd, path: &str, buf: &[u8]) -> isize {
    extern "C" {
        fn sendto(
            sockfd: c_int,
            buf: *const c_void,
            len: usize,
            flags: c_int,
            dest_addr: *const c_void,
            addrlen: u32,
        ) -> isize;
    }
    if fd < 0 || path.is_empty() || path.len() >= 108 {
        return -1;
    }
    let mut addr = SockAddrUn {
        sun_family: AF_UNIX as u16,
        sun_path: [0u8; 108],
    };
    let bytes = path.as_bytes();
    addr.sun_path[..bytes.len()].copy_from_slice(bytes);
    unsafe {
        sendto(
            fd,
            buf.as_ptr().cast::<c_void>(),
            buf.len(),
            MSG_DONTWAIT | MSG_NOSIGNAL,
            (&addr as *const SockAddrUn).cast::<c_void>(),
            mem::size_of::<SockAddrUn>() as u32,
        )
    }
}

pub fn recv_blocking_exact_with_2fds(fd: RawFd, buf: &mut [u8]) -> Option<(Option<OwnedFd>, Option<OwnedFd>)> {
    #[repr(C, align(8))]
    struct Ctl2 {
        bytes: [u8; 32],
    }
    let mut ctl = MaybeUninit::<Ctl2>::zeroed();
    let mut iov = IoVec {
        iov_base: buf.as_mut_ptr().cast::<c_void>(),
        iov_len: buf.len(),
    };
    let mut msg = MsgHdr {
        msg_name: ptr::null_mut(),
        msg_namelen: 0,
        _pad0: 0,
        msg_iov: &mut iov,
        msg_iovlen: 1,
        msg_control: ctl.as_mut_ptr().cast::<c_void>(),
        msg_controllen: mem::size_of::<Ctl2>(),
        msg_flags: 0,
        _pad1: 0,
    };

    let n = unsafe { recvmsg(fd, &mut msg, MSG_CMSG_CLOEXEC) };
    let ctrunc = (msg.msg_flags & 0x08) != 0;
    let mut f0: Option<OwnedFd> = None;
    let mut f1: Option<OwnedFd> = None;
    let ctl_ref = unsafe { ctl.assume_init() };
    let used = msg.msg_controllen.min(ctl_ref.bytes.len());
    let mut off = 0usize;
    while off + mem::size_of::<CMsgHdr>() <= used {
        let hdr = unsafe { ptr::read(ctl_ref.bytes.as_ptr().add(off).cast::<CMsgHdr>()) };
        if hdr.cmsg_len < mem::size_of::<CMsgHdr>() || off + hdr.cmsg_len > used {
            break;
        }
        if hdr.cmsg_level == SOL_SOCKET && hdr.cmsg_type == SCM_RIGHTS {
            let data_bytes = hdr.cmsg_len - mem::size_of::<CMsgHdr>();
            let nfds = data_bytes / mem::size_of::<c_int>();
            let data_ptr = unsafe { ctl_ref.bytes.as_ptr().add(off + mem::size_of::<CMsgHdr>()) };
            for i in 0..nfds {
                let raw_fd = unsafe { ptr::read_unaligned(data_ptr.cast::<c_int>().add(i)) };
                if raw_fd >= 0 {
                    let owned = unsafe { OwnedFd::from_raw_fd(raw_fd) };
                    if f0.is_none() {
                        f0 = Some(owned);
                    } else if f1.is_none() {
                        f1 = Some(owned);
                    }
                    // Any excess OwnedFds drop immediately and close cleanly.
                }
            }
        }
        let step = (hdr.cmsg_len + 7) & !7;
        if step == 0 {
            break;
        }
        off += step;
    }
    if ctrunc || n != buf.len() as isize {
        drop(f0);
        drop(f1);
        return None;
    }
    Some((f0, f1))
}

pub fn recv_waitall(fd: RawFd, buf: &mut [u8]) -> bool {
    if fd < 0 || buf.is_empty() {
        return false;
    }
    let mut got = 0usize;
    while got < buf.len() {
        let n = unsafe {
            recv(
                fd,
                buf.as_mut_ptr().add(got).cast::<c_void>(),
                buf.len() - got,
                MSG_WAITALL,
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        if n == 0 {
            return false;
        }
        got += n as usize;
    }
    true
}

pub fn bind_unix_dgram_passcred_0600(path: &str) -> Option<UnixDatagram> {
    if path.is_empty() || path.len() >= 108 {
        return None;
    }
    let _ = fs::remove_file(path);
    let sock = UnixDatagram::bind(path).ok()?;
    sock.set_nonblocking(true).ok()?;
    let one: c_int = 1;
    let _ = unsafe {
        setsockopt(
            sock.as_raw_fd(),
            SOL_SOCKET,
            SO_PASSCRED,
            (&one as *const c_int).cast::<c_void>(),
            mem::size_of::<c_int>() as u32,
        )
    };
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    Some(sock)
}

pub enum RecvDgramCred {
    WouldBlock,
    Err,
    Packet { len: usize, ucred: Option<UCred> },
}

pub fn recv_dgram_with_ucred(fd: RawFd, buf: &mut [u8]) -> RecvDgramCred {
    #[repr(C, align(8))]
    struct CredCtlBuf {
        bytes: [u8; 64],
    }
    let mut ctl = MaybeUninit::<CredCtlBuf>::zeroed();
    let mut iov = IoVec {
        iov_base: buf.as_mut_ptr().cast::<c_void>(),
        iov_len: buf.len(),
    };
    let mut msg = MsgHdr {
        msg_name: ptr::null_mut(),
        msg_namelen: 0,
        _pad0: 0,
        msg_iov: &mut iov,
        msg_iovlen: 1,
        msg_control: ctl.as_mut_ptr().cast::<c_void>(),
        msg_controllen: mem::size_of::<CredCtlBuf>(),
        msg_flags: 0,
        _pad1: 0,
    };

    let n = unsafe { recvmsg(fd, &mut msg, MSG_DONTWAIT | MSG_CMSG_CLOEXEC) };
    if n < 0 {
        let raw = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if raw == 11 || raw == 4 {
            return RecvDgramCred::WouldBlock;
        }
        return RecvDgramCred::Err;
    }

    let mut out_cred = None;
    let ctl_ref = unsafe { ctl.assume_init() };
    let used = msg.msg_controllen.min(ctl_ref.bytes.len());
    let mut off = 0usize;
    while off + mem::size_of::<CMsgHdr>() <= used {
        let hdr = unsafe { ptr::read(ctl_ref.bytes.as_ptr().add(off).cast::<CMsgHdr>()) };
        if hdr.cmsg_len < mem::size_of::<CMsgHdr>() || off + hdr.cmsg_len > used {
            break;
        }
        if hdr.cmsg_level == SOL_SOCKET && hdr.cmsg_type == SCM_RIGHTS {
            let data_bytes = hdr.cmsg_len - mem::size_of::<CMsgHdr>();
            let nfds = data_bytes / mem::size_of::<c_int>();
            let data_ptr = unsafe { ctl_ref.bytes.as_ptr().add(off + mem::size_of::<CMsgHdr>()) };
            for i in 0..nfds {
                let raw_fd = unsafe { ptr::read_unaligned(data_ptr.cast::<c_int>().add(i)) };
                if raw_fd >= 0 {
                    let _ = unsafe { OwnedFd::from_raw_fd(raw_fd) };
                }
            }
        } else if hdr.cmsg_level == SOL_SOCKET
            && hdr.cmsg_type == SCM_CREDENTIALS
            && hdr.cmsg_len >= mem::size_of::<CMsgHdr>() + mem::size_of::<UCred>()
        {
            let data_ptr = unsafe { ctl_ref.bytes.as_ptr().add(off + mem::size_of::<CMsgHdr>()) };
            let uc = unsafe { ptr::read_unaligned(data_ptr.cast::<UCred>()) };
            out_cred = Some(uc);
        }
        let step = (hdr.cmsg_len + 7) & !7;
        if step == 0 {
            break;
        }
        off += step;
    }

    RecvDgramCred::Packet {
        len: n as usize,
        ucred: out_cred,
    }
}

pub fn unix_socket_actively_served(path: &str, my_pid: u32) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    let Ok(stream) = std::os::unix::net::UnixStream::connect(path) else {
        return false;
    };
    if let Some(uc) = socket_peercred(stream.as_raw_fd()) {
        uc.pid > 0 && (uc.pid as u32) != my_pid
    } else {
        false
    }
}

pub fn nb_send(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    let n = unsafe { send(fd, buf.as_ptr().cast::<c_void>(), buf.len(), MSG_DONTWAIT | MSG_NOSIGNAL) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn nb_recv(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    let n = unsafe { recv(fd, buf.as_mut_ptr().cast::<c_void>(), buf.len(), MSG_DONTWAIT) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn send_all_nosignal(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        let n = unsafe { send(fd, buf.as_ptr().cast::<c_void>(), buf.len(), MSG_NOSIGNAL) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "eof"));
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

#[allow(clippy::result_unit_err)]
pub fn sendmsg_all_with_fds(fd: RawFd, payload: &[u8], fds: &[RawFd]) -> Result<(), ()> {
    if fds.len() > 32 || (payload.is_empty() && !fds.is_empty()) {
        return Err(());
    }
    #[repr(C, align(8))]
    struct CtlBuf {
        bytes: [u8; 16 + 32 * 4],
    }
    let mut ctl = CtlBuf {
        bytes: [0u8; 16 + 32 * 4],
    };
    let mut ctl_len = 0usize;
    if !fds.is_empty() {
        let data_len = mem::size_of_val(fds);
        let cmsg_len = mem::size_of::<CMsgHdr>() + data_len;
        ctl_len = (cmsg_len + 7) & !7;
        let hdr = CMsgHdr {
            cmsg_len,
            cmsg_level: SOL_SOCKET,
            cmsg_type: SCM_RIGHTS,
        };
        unsafe {
            ptr::write(ctl.bytes.as_mut_ptr().cast::<CMsgHdr>(), hdr);
            ptr::copy_nonoverlapping(
                fds.as_ptr().cast::<u8>(),
                ctl.bytes.as_mut_ptr().add(mem::size_of::<CMsgHdr>()),
                data_len,
            );
        }
    }

    let mut sent = 0usize;
    let mut tries = 0usize;
    while sent < payload.len() {
        let mut iov = IoVec {
            iov_base: unsafe { payload.as_ptr().add(sent) as *mut c_void },
            iov_len: payload.len() - sent,
        };
        let msg = MsgHdr {
            msg_name: ptr::null_mut(),
            msg_namelen: 0,
            _pad0: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: if sent == 0 && ctl_len > 0 {
                ctl.bytes.as_mut_ptr().cast::<c_void>()
            } else {
                ptr::null_mut()
            },
            msg_controllen: if sent == 0 { ctl_len } else { 0 },
            msg_flags: 0,
            _pad1: 0,
        };
        let n = unsafe { sendmsg(fd, &msg, MSG_NOSIGNAL) };
        if n < 0 {
            let err = io::Error::last_os_error();
            let raw = err.raw_os_error().unwrap_or(0);
            if raw == 4 {
                continue; // EINTR
            }
            if raw == 11 && tries < 40 {
                tries += 1;
                let mut pfd = [PollFd {
                    fd,
                    events: POLLOUT,
                    revents: 0,
                }];
                let _ = do_poll(&mut pfd, 5);
                continue;
            }
            return Err(());
        }
        if n == 0 {
            return Err(());
        }
        sent += n as usize;
    }
    Ok(())
}

pub enum RecvFdsResult {
    Eof,
    WouldBlock,
    Data { bytes: usize, fds: Vec<OwnedFd> },
}

#[allow(clippy::result_unit_err)]
pub fn recvmsg_with_fds(fd: RawFd, buf: &mut [u8]) -> Result<RecvFdsResult, ()> {
    #[repr(C, align(8))]
    struct CtlRx {
        bytes: [u8; 16 + 64 * 4],
    }
    let mut ctl = MaybeUninit::<CtlRx>::zeroed();
    let mut iov = IoVec {
        iov_base: buf.as_mut_ptr().cast::<c_void>(),
        iov_len: buf.len(),
    };
    let mut msg = MsgHdr {
        msg_name: ptr::null_mut(),
        msg_namelen: 0,
        _pad0: 0,
        msg_iov: &mut iov,
        msg_iovlen: 1,
        msg_control: ctl.as_mut_ptr().cast::<c_void>(),
        msg_controllen: mem::size_of::<CtlRx>(),
        msg_flags: 0,
        _pad1: 0,
    };

    let n = unsafe { recvmsg(fd, &mut msg, MSG_DONTWAIT | MSG_CMSG_CLOEXEC) };
    if n == 0 {
        return Ok(RecvFdsResult::Eof);
    }
    if n < 0 {
        let raw = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if raw == 11 || raw == 4 {
            return Ok(RecvFdsResult::WouldBlock);
        }
        return Err(());
    }
    const MSG_CTRUNC: c_int = 0x08;
    if (msg.msg_flags & MSG_CTRUNC) != 0 {
        return Err(());
    }

    let mut out_fds = Vec::new();
    let ctl_ref = unsafe { ctl.assume_init() };
    let used = (msg.msg_controllen).min(ctl_ref.bytes.len());
    let mut off = 0usize;
    while off + mem::size_of::<CMsgHdr>() <= used {
        let hdr = unsafe { ptr::read(ctl_ref.bytes.as_ptr().add(off).cast::<CMsgHdr>()) };
        if hdr.cmsg_len < mem::size_of::<CMsgHdr>() || off + hdr.cmsg_len > used {
            break;
        }
        if hdr.cmsg_level == SOL_SOCKET && hdr.cmsg_type == SCM_RIGHTS {
            let data_bytes = hdr.cmsg_len - mem::size_of::<CMsgHdr>();
            let nfds = data_bytes / mem::size_of::<c_int>();
            let data_ptr = unsafe { ctl_ref.bytes.as_ptr().add(off + mem::size_of::<CMsgHdr>()) };
            for i in 0..nfds {
                let raw_fd = unsafe { ptr::read_unaligned(data_ptr.cast::<c_int>().add(i)) };
                if raw_fd >= 0 {
                    out_fds.push(unsafe { OwnedFd::from_raw_fd(raw_fd) });
                }
            }
        }
        let step = (hdr.cmsg_len + 7) & !7;
        if step == 0 {
            break;
        }
        off += step;
    }

    Ok(RecvFdsResult::Data {
        bytes: n as usize,
        fds: out_fds,
    })
}

pub fn user_home_dir() -> String {
    if let Ok(h) = std::env::var("HOME") {
        if h.starts_with('/') {
            return h;
        }
    }
    let uid = current_uid().to_string();
    if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let mut cols = line.split(':');
            let _name = cols.next();
            let _pw = cols.next();
            let p_uid = cols.next();
            let _gid = cols.next();
            let _gecos = cols.next();
            let home = cols.next();
            if p_uid == Some(uid.as_str()) {
                if let Some(h) = home {
                    if h.starts_with('/') {
                        return h.to_string();
                    }
                }
            }
        }
    }
    "/tmp".to_string()
}

pub fn current_username() -> String {
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return u;
        }
    }
    let uid = current_uid().to_string();
    if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let mut cols = line.split(':');
            let name = cols.next();
            let _pw = cols.next();
            let p_uid = cols.next();
            if p_uid == Some(uid.as_str()) {
                if let Some(n) = name {
                    if !n.is_empty() {
                        return n.to_string();
                    }
                }
            }
        }
    }
    "user".to_string()
}

pub fn current_hostname() -> String {
    let mut buf = [0u8; 128];
    if unsafe { gethostname(buf.as_mut_ptr().cast::<c_char>(), buf.len() - 1) } == 0 {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        if end > 0 {
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    "salsa-host".to_string()
}

#[inline]
pub fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[inline]
pub fn socket_raw(domain: c_int, sock_type: c_int, proto: c_int) -> c_int {
    unsafe { socket(domain, sock_type, proto) }
}

/// # Safety
/// `addr` must point to at least `addrlen` readable bytes of a valid `sockaddr`.
#[inline]
pub unsafe fn bind_raw(fd: c_int, addr: *const c_void, addrlen: u32) -> c_int {
    unsafe { bind(fd, addr, addrlen) }
}

/// # Safety
/// `addr` must point to at least `addrlen` readable bytes of a valid `sockaddr`.
#[inline]
pub unsafe fn connect_raw(fd: c_int, addr: *const c_void, addrlen: u32) -> c_int {
    unsafe { connect(fd, addr, addrlen) }
}

/// # Safety
/// `optval` must point to at least `optlen` readable bytes.
#[inline]
pub unsafe fn setsockopt_raw(
    fd: c_int,
    level: c_int,
    optname: c_int,
    optval: *const c_void,
    optlen: u32,
) -> c_int {
    unsafe { setsockopt(fd, level, optname, optval, optlen) }
}

/// # Safety
/// `optval` and `optlen` must be valid writable pointers matching `getsockopt(2)`.
#[inline]
pub unsafe fn getsockopt_raw(
    fd: c_int,
    level: c_int,
    optname: c_int,
    optval: *mut c_void,
    optlen: *mut u32,
) -> c_int {
    unsafe { getsockopt(fd, level, optname, optval, optlen) }
}

#[inline]
pub fn shutdown_rdwr(fd: c_int) {
    if fd >= 0 {
        unsafe {
            let _ = shutdown(fd, SHUT_RDWR);
        }
    }
}

#[inline]
pub fn send_raw(fd: c_int, buf: &[u8], flags: c_int) -> isize {
    if fd < 0 {
        return -1;
    }
    unsafe { send(fd, buf.as_ptr().cast::<c_void>(), buf.len(), flags) }
}

#[inline]
pub fn recv_raw(fd: c_int, buf: &mut [u8], flags: c_int) -> isize {
    if fd < 0 {
        return -1;
    }
    unsafe { recv(fd, buf.as_mut_ptr().cast::<c_void>(), buf.len(), flags) }
}

#[inline]
pub fn read_raw(fd: c_int, buf: &mut [u8]) -> isize {
    if fd < 0 {
        return -1;
    }
    unsafe { read(fd, buf.as_mut_ptr().cast::<c_void>(), buf.len()) }
}

#[inline]
pub fn write_raw(fd: c_int, buf: &[u8]) -> isize {
    if fd < 0 {
        return -1;
    }
    unsafe { write(fd, buf.as_ptr().cast::<c_void>(), buf.len()) }
}

pub fn dbg_emit(sub: u8, lvl: u8, pid: u32, sid: u32, op: &str, target: &str, detail: &str) {
    dbg_emit_vals(sub, lvl, pid, sid, 0, 0, 0, op, target, detail);
}

#[allow(clippy::too_many_arguments)]
pub fn dbg_emit_vals(
    sub: u8,
    lvl: u8,
    pid: u32,
    sid: u32,
    v1: i32,
    v2: i32,
    hr: i32,
    op: &str,
    target: &str,
    detail: &str,
) {
    #[cfg(not(test))]
    {
        if !crate::debug_server::is_dbg_running() {
            return;
        }
        let Ok(c_op) = CString::new(op) else { return };
        let Ok(c_tgt) = CString::new(target) else { return };
        let Ok(c_det) = CString::new(detail) else { return };
        unsafe {
            crate::debug_server::salsa_dbg_emit_raw(
                sub,
                lvl,
                pid,
                sid,
                v1,
                v2,
                hr,
                c_op.as_ptr(),
                c_tgt.as_ptr(),
                c_det.as_ptr(),
            );
        }
    }
    #[cfg(test)]
    {
        let _ = (sub, lvl, pid, sid, v1, v2, hr, op, target, detail);
    }
}

#[inline]
pub fn close_raw_fd(fd: RawFd) {
    if fd >= 0 {
        unsafe {
            let _ = close(fd);
        }
    }
}

pub fn open_raw_flags(path: &str, flags: c_int) -> RawFd {
    let Ok(cpath) = CString::new(path) else {
        return -1;
    };
    unsafe { open(cpath.as_ptr(), flags, 0 as c_uint) }
}

/// Opens a device node with `O_RDWR | O_CLOEXEC | O_NONBLOCK | O_NOFOLLOW`,
/// verifies it is a character device via `fstat`, and clears `O_NONBLOCK`
/// so ALSA PCM kernel ioctls behave in blocking mode safely.
pub fn open_chrdev_rw_cloexec(path: &str) -> RawFd {
    use std::os::fd::{FromRawFd, IntoRawFd};
    use std::os::unix::fs::FileTypeExt;
    const F_GETFL: c_int = 3;
    const F_SETFL: c_int = 4;

    let fd = open_raw_flags(path, O_RDWR | O_CLOEXEC | O_NONBLOCK | O_NOFOLLOW);
    if fd < 0 {
        return -1;
    }
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let is_chr = file
        .metadata()
        .map(|m| m.file_type().is_char_device())
        .unwrap_or(false);
    if !is_chr {
        drop(file);
        return -1;
    }
    let fl = unsafe { fcntl(fd, F_GETFL, 0) };
    if fl >= 0 && (fl & O_NONBLOCK) != 0 {
        let _ = unsafe { fcntl(fd, F_SETFL, fl & !O_NONBLOCK) };
    }
    file.into_raw_fd()
}

pub fn nb_read_raw(fd: RawFd, buf: &mut [u8]) -> isize {
    if fd < 0 || buf.is_empty() {
        return -1;
    }
    unsafe { read(fd, buf.as_mut_ptr().cast::<c_void>(), buf.len()) }
}

/// Executes a raw `ioctl` request on an open file descriptor.
///
/// # Safety
/// `arg` must point to a valid argument structure matching `req` if required by the ioctl.
pub unsafe fn ioctl_raw(fd: RawFd, req: c_ulong, arg: *mut c_void) -> c_int {
    if fd < 0 {
        return -1;
    }
    unsafe { ioctl(fd, req, arg) }
}

/// Opens the first loadable shared library from `cands` with `RTLD_NOW | RTLD_LOCAL`.
///
/// # Safety
/// Loading shared libraries runs their ELF initializers inside the current process.
pub unsafe fn dl_open_any(cands: &[&str]) -> *mut c_void {
    for &cand in cands {
        let Ok(cstr) = CString::new(cand) else {
            continue;
        };
        let h = unsafe { dlopen(cstr.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if !h.is_null() {
            return h;
        }
    }
    ptr::null_mut()
}

/// Resolves a symbol address from an open `dlopen` handle.
///
/// # Safety
/// `handle` must be a valid handle returned by `dlopen`/`dl_open_any` and not yet closed.
pub unsafe fn dl_sym(handle: *mut c_void, symbol: &str) -> *mut c_void {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Ok(csym) = CString::new(symbol) else {
        return ptr::null_mut();
    };
    unsafe { dlsym(handle, csym.as_ptr()) }
}

/// Closes a `dlopen` handle if non-null.
///
/// # Safety
/// `handle` must either be null or a valid open handle with no live references to its symbols.
pub unsafe fn dl_close(handle: *mut c_void) {
    if !handle.is_null() {
        unsafe {
            let _ = dlclose(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    #[test]
    fn scm_rights_fd_roundtrip() {
        let (tx, rx) = UnixStream::pair().unwrap();
        let mem = create_memfd("rust-scm-test", 64).unwrap();
        let mut f = fs::File::from(dup_fd_cloexec(mem.as_raw_fd()).unwrap());
        f.write_all(b"SALSA_RUST").unwrap();

        let payload = b"PING";
        let fds = [mem.as_raw_fd()];
        sendmsg_all_with_fds(tx.as_raw_fd(), payload, &fds).unwrap();

        let mut buf = [0u8; 32];
        match recvmsg_with_fds(rx.as_raw_fd(), &mut buf).unwrap() {
            RecvFdsResult::Data { bytes, fds } => {
                assert_eq!(&buf[..bytes], b"PING");
                assert_eq!(fds.len(), 1);
                let mut rx_file = fs::File::from(fds.into_iter().next().unwrap());
                use std::io::Seek;
                rx_file.seek(io::SeekFrom::Start(0)).unwrap();
                let mut got = [0u8; 10];
                rx_file.read_exact(&mut got).unwrap();
                assert_eq!(&got, b"SALSA_RUST");
            }
            _ => panic!("expected SCM_RIGHTS message"),
        }
    }
}
