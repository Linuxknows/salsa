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
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const POLLIN: c_short = 1;
pub const POLLOUT: c_short = 4;
pub const POLLERR: c_short = 8;
pub const POLLHUP: c_short = 16;
pub const POLLNVAL: c_short = 32;

pub const SOL_SOCKET: c_int = 1;
pub const SO_PEERCRED: c_int = 17;
pub const SCM_RIGHTS: c_int = 1;

pub const MSG_DONTWAIT: c_int = 0x40;
pub const MSG_NOSIGNAL: c_int = 0x4000;

pub const MFD_CLOEXEC: c_uint = 0x1;
pub const EFD_NONBLOCK: c_int = 0x800;
pub const EFD_CLOEXEC: c_int = 0x80000;
pub const F_DUPFD_CLOEXEC: c_int = 1030;

pub const O_RDONLY: c_int = 0x0;
pub const O_RDWR: c_int = 0x2;
pub const O_CREAT: c_int = 0x40;
pub const O_EXCL: c_int = 0x80;
pub const O_NONBLOCK: c_int = 0x800;
pub const O_NOFOLLOW: c_int = 0x20000;
pub const O_CLOEXEC: c_int = 0x80000;

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
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn getuid() -> u32;
    fn getpid() -> i32;
    fn gethostname(name: *mut c_char, len: usize) -> c_int;
    fn pthread_self() -> c_ulong;
    fn pthread_setschedparam(thread: c_ulong, policy: c_int, param: *const SchedParam) -> c_int;
    fn setpriority(which: c_int, who: u32, prio: c_int) -> c_int;
}

#[cfg(not(test))]
extern "C" {
    fn lwasapi_debug_emit(
        sub: u8,
        lvl: u8,
        pid: u32,
        sid: u32,
        op: *const c_char,
        target: *const c_char,
        fmt: *const c_char,
        ...
    );
    fn lwasapi_debug_emit_vals(
        sub: u8,
        lvl: u8,
        pid: u32,
        sid: u32,
        v1: i32,
        v2: i32,
        hr: i32,
        op: *const c_char,
        target: *const c_char,
        fmt: *const c_char,
        ...
    );
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
    let fd = unsafe { memfd_create(cname.as_ptr(), MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    if size > 0 && unsafe { ftruncate(owned.as_raw_fd(), size as i64) } < 0 {
        return None;
    }
    Some(owned)
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
        let fd = unsafe { shm_open(cname.as_ptr(), O_CREAT | O_EXCL | O_RDWR, 0o600) };
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
        let out = format!("{b}/lwasapi");
        if ensure_owned_dir_0700(&out, uid) {
            return Some(out);
        }
    }
    let tmp = format!("/tmp/lwasapi-{uid}");
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
    let _ = fs::remove_file(path);
    let listener = UnixListener::bind(path).ok()?;
    listener.set_nonblocking(true).ok()?;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
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

pub fn unix_socket_actively_served(path: &str, my_pid: u32) -> bool {
    if fs::symlink_metadata(path).is_err() {
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

#[allow(clippy::result_unit_err)]
pub fn sendmsg_all_with_fds(fd: RawFd, payload: &[u8], fds: &[RawFd]) -> Result<(), ()> {
    if fds.len() > 32 {
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

    let n = unsafe { recvmsg(fd, &mut msg, MSG_DONTWAIT) };
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

pub fn dbg_emit(sub: u8, lvl: u8, pid: u32, sid: u32, op: &str, target: &str, detail: &str) {
    #[cfg(not(test))]
    {
        let Ok(c_op) = CString::new(op) else { return };
        let Ok(c_tgt) = CString::new(target) else { return };
        let Ok(c_det) = CString::new(detail) else { return };
        unsafe {
            lwasapi_debug_emit(
                sub,
                lvl,
                pid,
                sid,
                c_op.as_ptr(),
                c_tgt.as_ptr(),
                c"%s".as_ptr(),
                c_det.as_ptr(),
            );
        }
    }
    #[cfg(test)]
    {
        let _ = (sub, lvl, pid, sid, op, target, detail);
    }
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
        let Ok(c_op) = CString::new(op) else { return };
        let Ok(c_tgt) = CString::new(target) else { return };
        let Ok(c_det) = CString::new(detail) else { return };
        unsafe {
            lwasapi_debug_emit_vals(
                sub,
                lvl,
                pid,
                sid,
                v1,
                v2,
                hr,
                c_op.as_ptr(),
                c_tgt.as_ptr(),
                c"%s".as_ptr(),
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
