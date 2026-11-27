// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

//! Safe seqlock trust seam over the 33 MiB [`LasapiPwPreviewShm`] PipeWire
//! video-preview shared memory region.
//!
//! # Aliasing invariant
//! Shared memory may be accessed concurrently by separate processes (`audiodg`
//! writer and `salsa-debug` / tray readers). This module never constructs
//! `&LasapiPwPreviewShm` or `&mut LasapiPwPreviewShm`; all header reads and
//! writes operate strictly through raw field pointers (`ptr::addr_of!`,
//! `ptr::addr_of_mut!`), [`AtomicU32::from_ptr`], [`AtomicU64::from_ptr`], and
//! bounded seqlock critical sections.

#![allow(unsafe_code)]

use core::mem::{offset_of, size_of};
use core::ptr;
use core::slice;
use core::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};
use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::c_abi::{
    c_str_to_string, LasapiPwPreviewShm, LASAPI_PW_PREVIEW_MAGIC, LASAPI_PW_PREVIEW_MAX_BYTES,
    LASAPI_PW_PREVIEW_MAX_H, LASAPI_PW_PREVIEW_MAX_W, LASAPI_PW_PREVIEW_VERSION,
};
use crate::unix_sys::{create_memfd, mono_ns, MappedMem, O_CLOEXEC, O_NOFOLLOW};

/// Writer metadata published alongside each frame in [`PreviewShmWriter::publish_frame`].
pub struct PreviewFrameHeader<'a> {
    pub is_dmabuf: bool,
    pub dmabuf_mappable: bool,
    pub src_width: u32,
    pub src_height: u32,
    pub src_stride: u32,
    pub src_spa_format: u32,
    pub drm_modifier: u64,
    pub prod_node_id: u32,
    pub cons_count: u32,
    pub pts_ns: u64,
    pub prod_name: &'a str,
    pub format_name: &'a str,
    pub target_max_w: u32,
    pub target_max_h: u32,
    pub target_fps: u32,
    pub actual_fps_x100: u32,
    pub cast_fps_cap: u32,
}

/// Consistent frame and metadata snapshot read from [`PreviewShmReader`].
#[derive(Debug, Clone)]
pub struct PreviewFrameSnapshot {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub seq: u64,
    pub pts_ns: u64,
    pub prod_node_id: u32,
    pub cons_count: u32,
    pub active_stream: bool,
    pub is_dmabuf: bool,
    pub dmabuf_mappable: bool,
    pub actual_fps_x100: u32,
    pub target_max_w: u32,
    pub target_max_h: u32,
    pub target_fps: u32,
    pub cast_fps_cap: u32,
    pub prod_name: String,
    pub format_name: String,
    pub rgba: Vec<u8>,
}

/// Owning producer view over an initialized [`LasapiPwPreviewShm`] mapping.
pub struct PreviewShmWriter {
    map: MappedMem,
}

impl PreviewShmWriter {
    /// Zeroes the SHM header, writes `magic` and `version`, and returns a writer
    /// if `map` is at least `size_of::<LasapiPwPreviewShm>()` bytes.
    pub fn init_from_map(mut map: MappedMem) -> Option<Self> {
        if map.len() < size_of::<LasapiPwPreviewShm>() {
            return None;
        }
        let hdr_end = offset_of!(LasapiPwPreviewShm, pixels);
        map.as_mut_slice()[..hdr_end].fill(0);
        let shm = map.as_mut_ptr().cast::<LasapiPwPreviewShm>();
        // SAFETY: `map.len() >= size_of::<LasapiPwPreviewShm>()` and `shm` is
        // page-aligned (from `mmap`), so `magic` and `version` writes stay in bounds.
        unsafe {
            ptr::write_volatile(ptr::addr_of_mut!((*shm).magic), LASAPI_PW_PREVIEW_MAGIC);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).version), LASAPI_PW_PREVIEW_VERSION);
        }
        Some(Self { map })
    }

    #[inline]
    fn raw_shm(&self) -> *mut LasapiPwPreviewShm {
        self.map.as_ptr().cast_mut().cast::<LasapiPwPreviewShm>()
    }

    pub fn preview_requested(&self) -> bool {
        let shm = self.raw_shm();
        // SAFETY: `self.map` covers `LasapiPwPreviewShm`; `preview_requested` is a
        // 4-byte-aligned `u32` field accessed atomically across SHM processes.
        let atomic = unsafe { AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).preview_requested)) };
        atomic.load(Ordering::Relaxed) != 0
    }

    pub fn set_preview_requested(&self, enabled: bool) {
        let shm = self.raw_shm();
        // SAFETY: `preview_requested` is a 4-byte-aligned field inside `self.map`.
        let atomic = unsafe { AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).preview_requested)) };
        atomic.store(u32::from(enabled), Ordering::Release);
    }

    pub fn last_gui_ping_ns(&self) -> u64 {
        let shm = self.raw_shm();
        // SAFETY: `last_gui_ping_ns` is an 8-byte-aligned field inside `self.map`.
        let atomic = unsafe { AtomicU64::from_ptr(ptr::addr_of_mut!((*shm).last_gui_ping_ns)) };
        atomic.load(Ordering::Relaxed)
    }

    pub fn read_gui_targets(&self) -> (u32, u32, u32) {
        let shm = self.raw_shm();
        // SAFETY: All three target fields are 4-byte-aligned inside `self.map`.
        unsafe {
            let sw = AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_max_w))
                .load(Ordering::Relaxed);
            let sh = AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_max_h))
                .load(Ordering::Relaxed);
            let sf =
                AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_fps)).load(Ordering::Relaxed);
            (
                sw.min(LASAPI_PW_PREVIEW_MAX_W),
                sh.min(LASAPI_PW_PREVIEW_MAX_H),
                sf.min(240),
            )
        }
    }

    pub fn configure_targets(
        &self,
        max_w: u32,
        max_h: u32,
        target_fps: u32,
        now_ns: u64,
        enabled: bool,
    ) {
        let shm = self.raw_shm();
        // SAFETY: Target/ping/requested fields are naturally aligned within `self.map`.
        unsafe {
            if enabled {
                AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_max_w))
                    .store(max_w.min(LASAPI_PW_PREVIEW_MAX_W), Ordering::Relaxed);
                AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_max_h))
                    .store(max_h.min(LASAPI_PW_PREVIEW_MAX_H), Ordering::Relaxed);
                AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_fps))
                    .store(target_fps.min(240), Ordering::Relaxed);
                AtomicU64::from_ptr(ptr::addr_of_mut!((*shm).last_gui_ping_ns))
                    .store(now_ns, Ordering::Relaxed);
                AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).preview_requested))
                    .store(1, Ordering::Release);
            } else {
                AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).preview_requested))
                    .store(0, Ordering::Release);
            }
        }
    }

    pub fn clear_active_stream(&self) {
        let shm = self.raw_shm();
        // SAFETY: `active_stream` is a 4-byte-aligned field inside `self.map`.
        let atomic = unsafe { AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).active_stream)) };
        atomic.store(0, Ordering::Release);
    }

    /// Publishes one preview frame protected by the SHM `seqlock` (odd while
    /// writing, even once complete).
    ///
    /// `fill_pixels` receives a mutable slice over the SHM `pixels` payload and
    /// returns `Some((dst_width, dst_height, dst_stride, dmabuf_mappable_override))`
    /// on success or `None` when pixels could not be written.
    pub fn publish_frame<F>(&mut self, hdr: &PreviewFrameHeader<'_>, fill_pixels: F)
    where
        F: FnOnce(&mut [u8]) -> Option<(u32, u32, u32, bool)>,
    {
        let shm = self.map.as_mut_ptr().cast::<LasapiPwPreviewShm>();
        // SAFETY: `self.map` has length >= `size_of::<LasapiPwPreviewShm>()`, page
        // alignment from `mmap`, and is owned exclusively on the writer side. All
        // non-atomic header writes happen strictly while `seqlock` is odd.
        unsafe {
            let seqlock = AtomicU64::from_ptr(ptr::addr_of_mut!((*shm).seqlock));
            let seq = seqlock.load(Ordering::Relaxed);
            seqlock.store(seq.wrapping_add(1), Ordering::Release);

            ptr::write_volatile(ptr::addr_of_mut!((*shm).active_stream), 1);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).is_dmabuf), u32::from(hdr.is_dmabuf));
            ptr::write_volatile(
                ptr::addr_of_mut!((*shm).dmabuf_mappable),
                u32::from(hdr.dmabuf_mappable),
            );
            ptr::write_volatile(ptr::addr_of_mut!((*shm).src_width), hdr.src_width);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).src_height), hdr.src_height);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).src_stride), hdr.src_stride);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).src_spa_format), hdr.src_spa_format);
            ptr::write_volatile(
                ptr::addr_of_mut!((*shm).drm_modifier_lo),
                (hdr.drm_modifier & 0xffff_ffff) as u32,
            );
            ptr::write_volatile(
                ptr::addr_of_mut!((*shm).drm_modifier_hi),
                (hdr.drm_modifier >> 32) as u32,
            );
            ptr::write_volatile(ptr::addr_of_mut!((*shm).prod_node_id), hdr.prod_node_id);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).cons_count), hdr.cons_count);

            let prev_frame_seq = ptr::read_volatile(ptr::addr_of!((*shm).frame_seq));
            ptr::write_volatile(
                ptr::addr_of_mut!((*shm).frame_seq),
                prev_frame_seq.wrapping_add(1),
            );
            ptr::write_volatile(ptr::addr_of_mut!((*shm).pts_ns), hdr.pts_ns);

            write_cstr_raw(
                ptr::addr_of_mut!((*shm).prod_name).cast::<u8>(),
                128,
                hdr.prod_name,
            );
            write_cstr_raw(
                ptr::addr_of_mut!((*shm).format_name).cast::<u8>(),
                32,
                hdr.format_name,
            );

            ptr::write_volatile(ptr::addr_of_mut!((*shm).target_max_w), hdr.target_max_w);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).target_max_h), hdr.target_max_h);
            ptr::write_volatile(ptr::addr_of_mut!((*shm).target_fps), hdr.target_fps);
            ptr::write_volatile(
                ptr::addr_of_mut!((*shm).actual_fps_x100),
                hdr.actual_fps_x100,
            );
            ptr::write_volatile(ptr::addr_of_mut!((*shm).cast_fps_cap), hdr.cast_fps_cap);

            let pixels_ptr = ptr::addr_of_mut!((*shm).pixels).cast::<u8>();
            let pixels = slice::from_raw_parts_mut(pixels_ptr, LASAPI_PW_PREVIEW_MAX_BYTES);
            let outcome = fill_pixels(pixels);

            if let Some((dw, dh, dst_stride, dmabuf_mappable_override)) = outcome.filter(
                |&(dw, dh, ds, _)| {
                    dw > 0
                        && dh > 0
                        && dw <= LASAPI_PW_PREVIEW_MAX_W
                        && dh <= LASAPI_PW_PREVIEW_MAX_H
                        && (ds as usize) >= (dw as usize).saturating_mul(4)
                        && (dh as usize).saturating_mul(ds as usize) <= LASAPI_PW_PREVIEW_MAX_BYTES
                },
            ) {
                if dmabuf_mappable_override {
                    ptr::write_volatile(ptr::addr_of_mut!((*shm).dmabuf_mappable), 1);
                }
                ptr::write_volatile(ptr::addr_of_mut!((*shm).dst_width), dw);
                ptr::write_volatile(ptr::addr_of_mut!((*shm).dst_height), dh);
                ptr::write_volatile(ptr::addr_of_mut!((*shm).dst_stride), dst_stride);
            } else {
                ptr::write_volatile(ptr::addr_of_mut!((*shm).dst_width), 0);
                ptr::write_volatile(ptr::addr_of_mut!((*shm).dst_height), 0);
                ptr::write_volatile(ptr::addr_of_mut!((*shm).dst_stride), 0);
            }

            seqlock.store(seq.wrapping_add(2), Ordering::Release);
        }
    }
}

/// Consumer handle over a [`LasapiPwPreviewShm`] mapping enforcing the seqlock
/// read protocol and coordinate/buffer bounds validation.
pub struct PreviewShmReader {
    _fd: Option<OwnedFd>,
    map: MappedMem,
}

impl PreviewShmReader {
    /// Opens an SHM path (`shm_name` or `/dev/shm/<name>`) securely without
    /// following symlinks, validates file size and header magic/version, and
    /// maps it read/write (for GUI keepalive pings).
    pub fn open_path(shm_name: &str) -> Result<Self, String> {
        let map_size = size_of::<LasapiPwPreviewShm>();
        let trimmed = shm_name.trim_start_matches('/');
        if trimmed.is_empty() {
            return Err("empty preview shm name".to_string());
        }
        let path = if shm_name.starts_with('/') && Path::new(shm_name).exists() {
            PathBuf::from(shm_name)
        } else {
            PathBuf::from(format!("/dev/shm/{trimmed}"))
        };
        let f: File = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_CLOEXEC | O_NOFOLLOW)
            .open(&path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        let meta = f
            .metadata()
            .map_err(|e| format!("stat {}: {e}", path.display()))?;
        if !meta.file_type().is_file() || (meta.len() as usize) < map_size {
            return Err(format!(
                "invalid preview shm {}: size={}",
                path.display(),
                meta.len()
            ));
        }
        let fd: OwnedFd = f.into();
        let map = MappedMem::map_fd(fd.as_raw_fd(), map_size, true, 0)
            .ok_or_else(|| format!("mmap {} failed", path.display()))?;
        Self::from_mapped(Some(fd), map)
            .map_err(|e| format!("invalid preview shm header in {} ({e})", path.display()))
    }

    /// Wraps an already-mapped SHM view after checking size and header magic/version.
    pub fn from_mapped(fd: Option<OwnedFd>, map: MappedMem) -> Result<Self, String> {
        if map.len() < size_of::<LasapiPwPreviewShm>() {
            return Err(format!("size={}", map.len()));
        }
        let shm = map.as_ptr().cast::<LasapiPwPreviewShm>();
        // SAFETY: `map.len() >= size_of::<LasapiPwPreviewShm>()` and aligned via `mmap`.
        let (magic, version) = unsafe {
            (
                ptr::read_volatile(ptr::addr_of!((*shm).magic)),
                ptr::read_volatile(ptr::addr_of!((*shm).version)),
            )
        };
        if magic != LASAPI_PW_PREVIEW_MAGIC || version != LASAPI_PW_PREVIEW_VERSION {
            return Err(format!("magic=0x{magic:08x}, version={version}"));
        }
        Ok(Self { _fd: fd, map })
    }

    /// Creates an anonymous initialized `(PreviewShmWriter, PreviewShmReader)` pair
    /// backed by a `memfd` for tests and self-checks.
    pub fn new_memfd_pair() -> Option<(PreviewShmWriter, Self)> {
        let len = size_of::<LasapiPwPreviewShm>();
        let fd = create_memfd("salsa_preview_test", len)?;
        let writer_map = MappedMem::map_fd(fd.as_raw_fd(), len, true, 0)?;
        let writer = PreviewShmWriter::init_from_map(writer_map)?;
        let reader_map = MappedMem::map_fd(fd.as_raw_fd(), len, true, 0)?;
        let reader = Self::from_mapped(Some(fd), reader_map).ok()?;
        Some((writer, reader))
    }

    pub fn send_preview_ping(
        &self,
        enabled: bool,
        max_w: u32,
        max_h: u32,
        target_fps: u32,
        cast_fps_cap: u32,
    ) {
        if self.map.len() < size_of::<LasapiPwPreviewShm>() {
            return;
        }
        let shm = self.map.as_ptr().cast_mut().cast::<LasapiPwPreviewShm>();
        // SAFETY: All target/ping fields are naturally aligned inside `self.map` and
        // updated atomically using the same protocol as `PwBroker`.
        unsafe {
            AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_max_w))
                .store(max_w.min(LASAPI_PW_PREVIEW_MAX_W), Ordering::Relaxed);
            AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_max_h))
                .store(max_h.min(LASAPI_PW_PREVIEW_MAX_H), Ordering::Relaxed);
            AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).target_fps))
                .store(target_fps, Ordering::Relaxed);
            AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).cast_fps_cap))
                .store(cast_fps_cap, Ordering::Relaxed);
            AtomicU32::from_ptr(ptr::addr_of_mut!((*shm).preview_requested))
                .store(u32::from(enabled), Ordering::Release);
            AtomicU64::from_ptr(ptr::addr_of_mut!((*shm).last_gui_ping_ns))
                .store(mono_ns(), Ordering::Release);
        }
    }

    /// Performs one lock-free seqlock probe without waiting if a write is active
    /// (`seqlock` odd) or if the write overlapped the read (`s1 != s2`).
    pub fn try_read_frame_once(&self, prev_seq: Option<u64>) -> Option<PreviewFrameSnapshot> {
        if self.map.len() < size_of::<LasapiPwPreviewShm>() {
            return None;
        }
        let shm = self.map.as_ptr().cast::<LasapiPwPreviewShm>();
        let shm_mut = self.map.as_ptr().cast_mut().cast::<LasapiPwPreviewShm>();
        // SAFETY: `self.map` covers `LasapiPwPreviewShm`; no `&LasapiPwPreviewShm`
        // reference is formed, and all data read under matching even seqlock values
        // is verified with `Acquire` fences before returning.
        unsafe {
            let magic = ptr::read_volatile(ptr::addr_of!((*shm).magic));
            let version = ptr::read_volatile(ptr::addr_of!((*shm).version));
            if magic != LASAPI_PW_PREVIEW_MAGIC || version != LASAPI_PW_PREVIEW_VERSION {
                return None;
            }
            let seqlock = AtomicU64::from_ptr(ptr::addr_of_mut!((*shm_mut).seqlock));
            let s1 = seqlock.load(Ordering::Acquire);
            if (s1 & 1) != 0 {
                return None;
            }

            let w = ptr::read_volatile(ptr::addr_of!((*shm).dst_width));
            let h = ptr::read_volatile(ptr::addr_of!((*shm).dst_height));
            let src_w = ptr::read_volatile(ptr::addr_of!((*shm).src_width));
            let src_h = ptr::read_volatile(ptr::addr_of!((*shm).src_height));
            let stride = ptr::read_volatile(ptr::addr_of!((*shm).dst_stride));
            let seq = ptr::read_volatile(ptr::addr_of!((*shm).frame_seq));
            let pts_ns = ptr::read_volatile(ptr::addr_of!((*shm).pts_ns));
            let prod_node_id = ptr::read_volatile(ptr::addr_of!((*shm).prod_node_id));
            let cons_count = ptr::read_volatile(ptr::addr_of!((*shm).cons_count));
            let active_stream = ptr::read_volatile(ptr::addr_of!((*shm).active_stream)) != 0;
            let is_dmabuf = ptr::read_volatile(ptr::addr_of!((*shm).is_dmabuf)) != 0;
            let dmabuf_mappable = ptr::read_volatile(ptr::addr_of!((*shm).dmabuf_mappable)) != 0;
            let actual_fps_x100 = ptr::read_volatile(ptr::addr_of!((*shm).actual_fps_x100));
            let target_max_w = ptr::read_volatile(ptr::addr_of!((*shm).target_max_w));
            let target_max_h = ptr::read_volatile(ptr::addr_of!((*shm).target_max_h));
            let target_fps = ptr::read_volatile(ptr::addr_of!((*shm).target_fps));
            let cast_fps_cap = ptr::read_volatile(ptr::addr_of!((*shm).cast_fps_cap));
            let prod_raw = ptr::read_volatile(ptr::addr_of!((*shm).prod_name));
            let fmt_raw = ptr::read_volatile(ptr::addr_of!((*shm).format_name));

            let unchanged = active_stream && prev_seq.is_some_and(|p| p != 0 && seq == p);
            let row_bytes = (w as usize).saturating_mul(4);
            let byte_len = (stride as usize).saturating_mul(h as usize);
            let rgba = if !unchanged
                && w > 0
                && h > 0
                && w <= LASAPI_PW_PREVIEW_MAX_W
                && h <= LASAPI_PW_PREVIEW_MAX_H
                && (stride as usize) >= row_bytes
                && byte_len <= LASAPI_PW_PREVIEW_MAX_BYTES
            {
                let total_out = row_bytes.saturating_mul(h as usize);
                let mut out = vec![0u8; total_out];
                let pixels_ptr = ptr::addr_of!((*shm).pixels).cast::<u8>();
                let src = slice::from_raw_parts(pixels_ptr, byte_len);
                let st = stride as usize;
                let wu = w as usize;
                for (y, drow) in out.chunks_exact_mut(row_bytes).enumerate() {
                    let srow = &src[y * st..y * st + row_bytes];
                    let (schunks, _) = srow.as_chunks::<4>();
                    let (dchunks, _) = drow.as_chunks_mut::<4>();
                    for (&[b, g, r, _], dp) in schunks.iter().take(wu).zip(dchunks.iter_mut()) {
                        // PipeWire preview pixels arrive in BGRx/BGRA byte order;
                        // force opaque alpha so BGRx frames render visibly and export cleanly.
                        *dp = [r, g, b, 255];
                    }
                }
                out
            } else {
                Vec::new()
            };

            fence(Ordering::Acquire);
            let s2 = seqlock.load(Ordering::Acquire);
            if s1 != s2 {
                return None;
            }

            let out_stride = if rgba.is_empty() {
                0
            } else {
                w.saturating_mul(4)
            };
            let disp_w = if w > 0 { w } else { src_w };
            let disp_h = if h > 0 { h } else { src_h };
            Some(PreviewFrameSnapshot {
                width: disp_w,
                height: disp_h,
                stride: out_stride,
                seq,
                pts_ns,
                prod_node_id,
                cons_count,
                active_stream,
                is_dmabuf,
                dmabuf_mappable,
                actual_fps_x100,
                target_max_w,
                target_max_h,
                target_fps,
                cast_fps_cap,
                prod_name: c_str_to_string(&prod_raw),
                format_name: c_str_to_string(&fmt_raw),
                rgba,
            })
        }
    }

    pub fn read_frame(&self) -> Option<PreviewFrameSnapshot> {
        self.read_frame_since(None)
    }

    pub fn read_frame_since(&self, prev_seq: Option<u64>) -> Option<PreviewFrameSnapshot> {
        let mut deadline: Option<Instant> = None;
        for attempt in 0..64 {
            if let Some(snap) = self.try_read_frame_once(prev_seq) {
                return Some(snap);
            }
            if attempt < 4 {
                std::hint::spin_loop();
            } else {
                let d =
                    *deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(8));
                if Instant::now() >= d {
                    return None;
                }
                if attempt == 4 {
                    thread::yield_now();
                } else {
                    thread::sleep(Duration::from_micros(250));
                }
            }
        }
        None
    }
}

unsafe fn write_cstr_raw(dst: *mut u8, cap: usize, s: &str) {
    if cap == 0 {
        return;
    }
    let bytes = s.as_bytes();
    let mut n = bytes.len().min(cap - 1);
    while n > 0 && !s.is_char_boundary(n) {
        n -= 1;
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), dst, n);
        ptr::write_bytes(dst.add(n), 0, cap - n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header<'a>() -> PreviewFrameHeader<'a> {
        PreviewFrameHeader {
            is_dmabuf: false,
            dmabuf_mappable: false,
            src_width: 4,
            src_height: 2,
            src_stride: 16,
            src_spa_format: 8,
            drm_modifier: 0,
            prod_node_id: 77,
            cons_count: 1,
            pts_ns: 123_456,
            prod_name: "wayland-screen",
            format_name: "BGRx",
            target_max_w: 1920,
            target_max_h: 1080,
            target_fps: 60,
            actual_fps_x100: 6000,
            cast_fps_cap: 120,
        }
    }

    #[test]
    fn preview_shm_roundtrip_bgrx_to_rgba_and_unchanged_seq() {
        let (mut writer, reader) = PreviewShmReader::new_memfd_pair().expect("memfd pair");
        let hdr = sample_header();
        writer.publish_frame(&hdr, |pixels| {
            for px in pixels[..32].as_chunks_mut::<4>().0 {
                px.copy_from_slice(&[0x11, 0x55, 0xaa, 0x00]);
            }
            Some((4, 2, 16, false))
        });

        let snap = reader.read_frame().expect("first frame");
        assert!(snap.active_stream);
        assert_eq!(snap.seq, 1);
        assert_eq!(snap.width, 4);
        assert_eq!(snap.height, 2);
        assert_eq!(snap.prod_name, "wayland-screen");
        assert_eq!(snap.format_name, "BGRx");
        assert_eq!(snap.rgba.len(), 32);
        assert_eq!(&snap.rgba[..4], &[0xaa, 0x55, 0x11, 255]);

        let unchanged = reader.read_frame_since(Some(1)).expect("unchanged seq");
        assert!(unchanged.active_stream);
        assert_eq!(unchanged.seq, 1);
        assert!(unchanged.rgba.is_empty());
    }

    #[test]
    fn preview_shm_rejects_odd_seqlock_and_waits_for_concurrent_writer() {
        let len = size_of::<LasapiPwPreviewShm>();
        let fd = create_memfd("salsa_preview_seq_test", len).expect("memfd");
        let wmap = MappedMem::map_fd(fd.as_raw_fd(), len, true, 0).expect("wmap");
        let writer = PreviewShmWriter::init_from_map(wmap).expect("writer");
        let rmap = MappedMem::map_fd(fd.as_raw_fd(), len, true, 0).expect("rmap");
        let reader = PreviewShmReader::from_mapped(None, rmap).expect("reader");

        let shm_addr = writer.raw_shm() as usize;
        unsafe {
            let shm = shm_addr as *mut LasapiPwPreviewShm;
            AtomicU64::from_ptr(ptr::addr_of_mut!((*shm).seqlock)).store(3, Ordering::Release);
        }
        assert!(
            reader.try_read_frame_once(None).is_none(),
            "odd seqlock must be rejected by non-blocking probe"
        );

        let t = thread::spawn(move || {
            let mut w = writer;
            thread::sleep(Duration::from_millis(2));
            let shm = w.raw_shm();
            unsafe {
                AtomicU64::from_ptr(ptr::addr_of_mut!((*shm).seqlock)).store(2, Ordering::Release);
            }
            let hdr = sample_header();
            w.publish_frame(&hdr, |pixels| {
                for px in pixels[..32].as_chunks_mut::<4>().0 {
                    px.copy_from_slice(&[0x22, 0x66, 0xbb, 0x00]);
                }
                Some((4, 2, 16, false))
            });
            drop(w);
        });

        let snap = reader
            .read_frame()
            .expect("blocking reader should wait out 2ms odd seqlock");
        t.join().unwrap();
        assert_eq!(snap.seq, 1);
        assert_eq!(&snap.rgba[..4], &[0xbb, 0x66, 0x22, 255]);
    }

    #[test]
    fn preview_shm_clamps_oversized_dimensions_and_gui_targets() {
        let (mut writer, reader) = PreviewShmReader::new_memfd_pair().expect("memfd pair");
        reader.send_preview_ping(true, 99_999, 88_888, 999, 500);
        assert!(writer.preview_requested());
        assert_eq!(
            writer.read_gui_targets(),
            (LASAPI_PW_PREVIEW_MAX_W, LASAPI_PW_PREVIEW_MAX_H, 240)
        );

        let hdr = sample_header();
        // Out-of-bounds writer metadata must yield dst dimensions = 0 rather than OOB reads.
        writer.publish_frame(&hdr, |_pixels| Some((9000, 9000, 9000 * 4, false)));
        let snap = reader.read_frame().expect("clamped frame");
        assert!(snap.rgba.is_empty());
        assert_eq!(snap.width, 4);
        assert_eq!(snap.height, 2);
    }
}
