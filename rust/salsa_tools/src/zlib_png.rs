// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use core::mem;
use core::ptr;
use std::fs::{self, Metadata};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::OnceLock;

use salsa_wire::unix_sys::{dl_open_any, dl_sym};

use crate::host::write_atomic_secure;

const Z_NO_FLUSH: c_int = 0;
const Z_FINISH: c_int = 4;
const Z_OK: c_int = 0;
const Z_STREAM_END: c_int = 1;
const Z_DEFLATED: c_int = 8;
const Z_DEFAULT_STRATEGY: c_int = 0;

#[repr(C)]
struct ZStream {
    next_in: *const u8,
    avail_in: u32,
    total_in: c_ulong,
    next_out: *mut u8,
    avail_out: u32,
    total_out: c_ulong,
    msg: *const c_char,
    state: *mut c_void,
    zalloc: *mut c_void,
    zfree: *mut c_void,
    opaque: *mut c_void,
    data_type: c_int,
    adler: c_ulong,
    reserved: c_ulong,
}

impl Default for ZStream {
    fn default() -> Self {
        unsafe { mem::zeroed() }
    }
}

struct ZlibApi {
    zlib_version: unsafe extern "C" fn() -> *const c_char,
    deflate_init2: unsafe extern "C" fn(
        *mut ZStream,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        *const c_char,
        c_int,
    ) -> c_int,
    deflate: unsafe extern "C" fn(*mut ZStream, c_int) -> c_int,
    deflate_end: unsafe extern "C" fn(*mut ZStream) -> c_int,
    inflate_init2: unsafe extern "C" fn(*mut ZStream, c_int, *const c_char, c_int) -> c_int,
    inflate: unsafe extern "C" fn(*mut ZStream, c_int) -> c_int,
    inflate_end: unsafe extern "C" fn(*mut ZStream) -> c_int,
}

unsafe impl Send for ZlibApi {}
unsafe impl Sync for ZlibApi {}

static ZLIB: OnceLock<Option<ZlibApi>> = OnceLock::new();

fn load_zlib() -> Option<&'static ZlibApi> {
    ZLIB.get_or_init(|| unsafe {
        let h = dl_open_any(&[
            "libz.so.1",
            "/usr/lib/libz.so.1",
            "/usr/lib64/libz.so.1",
            "/usr/lib/x86_64-linux-gnu/libz.so.1",
            "/usr/lib/aarch64-linux-gnu/libz.so.1",
        ]);
        if h.is_null() {
            return None;
        }
        let s_ver = dl_sym(h, "zlibVersion");
        let s_dinit2 = dl_sym(h, "deflateInit2_");
        let s_def = dl_sym(h, "deflate");
        let s_dend = dl_sym(h, "deflateEnd");
        let s_iinit2 = dl_sym(h, "inflateInit2_");
        let s_inf = dl_sym(h, "inflate");
        let s_iend = dl_sym(h, "inflateEnd");
        if s_ver.is_null()
            || s_dinit2.is_null()
            || s_def.is_null()
            || s_dend.is_null()
            || s_iinit2.is_null()
            || s_inf.is_null()
            || s_iend.is_null()
        {
            return None;
        }
        Some(ZlibApi {
            zlib_version: mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const c_char>(
                s_ver,
            ),
            deflate_init2: mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(
                    *mut ZStream,
                    c_int,
                    c_int,
                    c_int,
                    c_int,
                    c_int,
                    *const c_char,
                    c_int,
                ) -> c_int,
            >(s_dinit2),
            deflate: mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut ZStream, c_int) -> c_int>(
                s_def,
            ),
            deflate_end: mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut ZStream) -> c_int>(
                s_dend,
            ),
            inflate_init2: mem::transmute::<
                *mut c_void,
                unsafe extern "C" fn(*mut ZStream, c_int, *const c_char, c_int) -> c_int,
            >(s_iinit2),
            inflate: mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut ZStream, c_int) -> c_int>(
                s_inf,
            ),
            inflate_end: mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut ZStream) -> c_int>(
                s_iend,
            ),
        })
    })
    .as_ref()
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn deflate_with_wbits(data: &[u8], level: i32, wbits: c_int) -> Option<Vec<u8>> {
    let api = load_zlib()?;
    let mut strm = ZStream::default();
    let ver = unsafe { (api.zlib_version)() };
    let rc = unsafe {
        (api.deflate_init2)(
            &raw mut strm,
            level as c_int,
            Z_DEFLATED,
            wbits,
            8,
            Z_DEFAULT_STRATEGY,
            ver,
            mem::size_of::<ZStream>() as c_int,
        )
    };
    if rc != Z_OK {
        return None;
    }

    let mut out = Vec::new();
    let mut chunk = [0u8; 32768];
    let mut in_off = 0usize;
    let total_len = data.len();

    loop {
        if strm.avail_in == 0 && in_off < total_len {
            let step = (total_len - in_off).min(u32::MAX as usize);
            strm.next_in = unsafe { data.as_ptr().add(in_off) };
            strm.avail_in = step as u32;
            in_off += step;
        }
        let flush = if in_off >= total_len {
            Z_FINISH
        } else {
            Z_NO_FLUSH
        };
        strm.next_out = chunk.as_mut_ptr();
        strm.avail_out = chunk.len() as u32;
        let d_rc = unsafe { (api.deflate)(&raw mut strm, flush) };
        let have = chunk.len() - strm.avail_out as usize;
        if have > 0 {
            out.extend_from_slice(&chunk[..have]);
        }
        if d_rc == Z_STREAM_END {
            break;
        }
        if d_rc != Z_OK {
            unsafe { (api.deflate_end)(&raw mut strm) };
            return None;
        }
        if flush == Z_FINISH && have == 0 && strm.avail_out > 0 {
            break;
        }
    }

    unsafe { (api.deflate_end)(&raw mut strm) };
    Some(out)
}

pub fn deflate_raw(data: &[u8], level: i32) -> Option<Vec<u8>> {
    deflate_with_wbits(data, level, -15)
}

pub fn compress_zlib(data: &[u8]) -> Option<Vec<u8>> {
    deflate_with_wbits(data, 6, 15)
}

pub fn uncompress_zlib(data: &[u8], expected_max: usize) -> Option<Vec<u8>> {
    let api = load_zlib()?;
    let mut strm = ZStream::default();
    let ver = unsafe { (api.zlib_version)() };
    let rc = unsafe {
        (api.inflate_init2)(
            &raw mut strm,
            15,
            ver,
            mem::size_of::<ZStream>() as c_int,
        )
    };
    if rc != Z_OK {
        return None;
    }

    let mut out = Vec::new();
    let mut chunk = [0u8; 32768];
    let mut in_off = 0usize;
    let total_len = data.len();

    loop {
        if strm.avail_in == 0 && in_off < total_len {
            let step = (total_len - in_off).min(u32::MAX as usize);
            strm.next_in = unsafe { data.as_ptr().add(in_off) };
            strm.avail_in = step as u32;
            in_off += step;
        }
        strm.next_out = chunk.as_mut_ptr();
        strm.avail_out = chunk.len() as u32;
        let i_rc = unsafe { (api.inflate)(&raw mut strm, Z_NO_FLUSH) };
        let have = chunk.len() - strm.avail_out as usize;
        if have > 0 {
            if out.len().saturating_add(have) > expected_max {
                unsafe { (api.inflate_end)(&raw mut strm) };
                return None;
            }
            out.extend_from_slice(&chunk[..have]);
        }
        if i_rc == Z_STREAM_END {
            break;
        }
        if i_rc != Z_OK || (have == 0 && in_off >= total_len) {
            unsafe { (api.inflate_end)(&raw mut strm) };
            return None;
        }
    }

    unsafe { (api.inflate_end)(&raw mut strm) };
    Some(out)
}

#[derive(Clone, Copy)]
struct ZipEntryMeta {
    crc: u32,
    comp_size: u32,
    uncomp_size: u32,
    method: u16,
    ext_attr: u32,
    local_offset: u32,
}

pub struct ZipWriter {
    buf: Vec<u8>,
    entries: Vec<(Vec<u8>, ZipEntryMeta)>,
}

impl Default for ZipWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ZipWriter {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            entries: Vec::new(),
        }
    }

    pub fn add_file(&mut self, archive_name: &str, data: &[u8], unix_mode: u32) -> Result<(), String> {
        let name_bytes = archive_name.as_bytes();
        let name_len = u16::try_from(name_bytes.len()).map_err(|_| "archive path too long")?;
        let uncomp_len = u32::try_from(data.len()).map_err(|_| "file too large for ZIP32")?;
        let local_off = u32::try_from(self.buf.len()).map_err(|_| "archive exceeds 4GiB")?;
        let crc = crc32(data);

        let compressed = deflate_raw(data, 6);
        let (method, payload): (u16, &[u8]) = match compressed.as_ref() {
            Some(c) if c.len() < data.len() => (8, c.as_slice()),
            _ => (0, data),
        };
        let comp_len = u32::try_from(payload.len()).map_err(|_| "compressed size overflow")?;
        let ext_attr = ((0o100_000 | (unix_mode & 0o777)) << 16) | if (unix_mode & 0o200) == 0 { 1 } else { 0 };

        self.write_local_header(name_bytes, name_len, method, crc, comp_len, uncomp_len);
        self.buf.extend_from_slice(payload);

        self.entries.push((
            name_bytes.to_vec(),
            ZipEntryMeta {
                crc,
                comp_size: comp_len,
                uncomp_size: uncomp_len,
                method,
                ext_attr,
                local_offset: local_off,
            },
        ));
        Ok(())
    }

    pub fn add_symlink(&mut self, archive_name: &str, target: &str) -> Result<(), String> {
        let name_bytes = archive_name.as_bytes();
        let payload = target.as_bytes();
        let name_len = u16::try_from(name_bytes.len()).map_err(|_| "symlink path too long")?;
        let plen = u32::try_from(payload.len()).map_err(|_| "symlink target too long")?;
        let local_off = u32::try_from(self.buf.len()).map_err(|_| "archive exceeds 4GiB")?;
        let crc = crc32(payload);
        let ext_attr = (0o120_777u32) << 16;

        self.write_local_header(name_bytes, name_len, 0, crc, plen, plen);
        self.buf.extend_from_slice(payload);

        self.entries.push((
            name_bytes.to_vec(),
            ZipEntryMeta {
                crc,
                comp_size: plen,
                uncomp_size: plen,
                method: 0,
                ext_attr,
                local_offset: local_off,
            },
        ));
        Ok(())
    }

    pub fn add_fs_entry(
        &mut self,
        archive_name: &str,
        disk_path: &Path,
        meta: &Metadata,
    ) -> Result<(), String> {
        if meta.file_type().is_symlink() {
            let target = fs::read_link(disk_path)
                .map_err(|e| format!("read_link {}: {e}", disk_path.display()))?;
            let target_str = target.to_string_lossy();
            self.add_symlink(archive_name, &target_str)
        } else {
            let data =
                fs::read(disk_path).map_err(|e| format!("read {}: {e}", disk_path.display()))?;
            self.add_file(archive_name, &data, meta.permissions().mode())
        }
    }

    fn write_local_header(
        &mut self,
        name: &[u8],
        name_len: u16,
        method: u16,
        crc: u32,
        comp_size: u32,
        uncomp_size: u32,
    ) {
        self.buf.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        self.buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.buf.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        self.buf.extend_from_slice(&method.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // mod time
        self.buf.extend_from_slice(&0x5421u16.to_le_bytes()); // mod date
        self.buf.extend_from_slice(&crc.to_le_bytes());
        self.buf.extend_from_slice(&comp_size.to_le_bytes());
        self.buf.extend_from_slice(&uncomp_size.to_le_bytes());
        self.buf.extend_from_slice(&name_len.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra len
        self.buf.extend_from_slice(name);
    }

    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        let cd_offset = u32::try_from(self.buf.len()).map_err(|_| "central dir offset overflow")?;
        let count = u16::try_from(self.entries.len()).map_err(|_| "too many entries for ZIP32")?;

        for (name, m) in &self.entries {
            let name_len = name.len() as u16;
            self.buf.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
            self.buf.extend_from_slice(&((3u16 << 8) | 20u16).to_le_bytes()); // UNIX made-by
            self.buf.extend_from_slice(&20u16.to_le_bytes()); // needed
            self.buf.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8
            self.buf.extend_from_slice(&m.method.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0x5421u16.to_le_bytes());
            self.buf.extend_from_slice(&m.crc.to_le_bytes());
            self.buf.extend_from_slice(&m.comp_size.to_le_bytes());
            self.buf.extend_from_slice(&m.uncomp_size.to_le_bytes());
            self.buf.extend_from_slice(&name_len.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // comment
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // disk
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // int attrs
            self.buf.extend_from_slice(&m.ext_attr.to_le_bytes());
            self.buf.extend_from_slice(&m.local_offset.to_le_bytes());
            self.buf.extend_from_slice(name);
        }

        let cd_size =
            u32::try_from(self.buf.len() - cd_offset as usize).map_err(|_| "cd size overflow")?;
        self.buf.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&count.to_le_bytes());
        self.buf.extend_from_slice(&count.to_le_bytes());
        self.buf.extend_from_slice(&cd_size.to_le_bytes());
        self.buf.extend_from_slice(&cd_offset.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        Ok(self.buf)
    }

    pub fn write_to_path_mode(self, out_path: &Path, mode: u32) -> Result<usize, String> {
        let count = self.entries.len();
        let bytes = self.finish()?;
        write_atomic_secure(out_path, &bytes, mode)
            .map_err(|e| format!("write {}: {e}", out_path.display()))?;
        Ok(count)
    }

    pub fn write_to_path(self, out_path: &Path) -> Result<usize, String> {
        self.write_to_path_mode(out_path, 0o644)
    }
}

#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i32::from(a) + i32::from(b) - i32::from(c);
    let pa = (p - i32::from(a)).abs();
    let pb = (p - i32::from(b)).abs();
    let pc = (p - i32::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub fn decode_png_rgba(png: &[u8]) -> Result<RgbaImage, String> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if png.len() < 8 || png[..8] != SIG {
        return Err("invalid PNG signature".into());
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut plte: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat = Vec::new();
    let mut seen_ihdr = false;

    while pos + 12 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        let ctype = &png[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(len).ok_or("PNG chunk overflow")?;
        if data_end + 4 > png.len() {
            return Err("truncated PNG chunk".into());
        }
        let cdata = &png[data_start..data_end];
        let want_crc = u32::from_be_bytes(png[data_end..data_end + 4].try_into().unwrap());
        let got_crc = crc32(&png[pos + 4..data_end]);
        if want_crc != got_crc {
            return Err("PNG CRC mismatch".into());
        }
        match ctype {
            b"IHDR" => {
                if cdata.len() < 13 {
                    return Err("bad IHDR".into());
                }
                width = u32::from_be_bytes(cdata[0..4].try_into().unwrap());
                height = u32::from_be_bytes(cdata[4..8].try_into().unwrap());
                bit_depth = cdata[8];
                color_type = cdata[9];
                interlace = cdata[12];
                seen_ihdr = true;
            }
            b"PLTE" => {
                for rgb in cdata.as_chunks::<3>().0 {
                    plte.push([rgb[0], rgb[1], rgb[2]]);
                }
            }
            b"tRNS" => {
                trns.extend_from_slice(cdata);
            }
            b"IDAT" => idat.extend_from_slice(cdata),
            b"IEND" => break,
            _ => {}
        }
        pos = data_end + 4;
    }

    if !seen_ihdr || width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("unsupported PNG dimensions".into());
    }
    if bit_depth != 8 || interlace != 0 {
        return Err(format!(
            "unsupported PNG bit_depth={bit_depth} interlace={interlace}"
        ));
    }

    let bpp = match color_type {
        0 => 1usize,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(format!("unsupported color_type={color_type}")),
    };
    let row_bytes = (width as usize)
        .checked_mul(bpp)
        .ok_or("PNG row overflow")?;
    let expected = (row_bytes + 1)
        .checked_mul(height as usize)
        .ok_or("PNG image overflow")?;
    let raw = uncompress_zlib(&idat, expected + 4096)
        .ok_or_else(|| "PNG zlib inflate failed".to_string())?;
    if raw.len() < expected {
        return Err("PNG inflated stream too short".into());
    }

    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut prev = vec![0u8; row_bytes];
    let mut cur = vec![0u8; row_bytes];

    for y in 0..height as usize {
        let src_row = &raw[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filt = src_row[0];
        let fdata = &src_row[1..];
        for i in 0..row_bytes {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            cur[i] = match filt {
                0 => fdata[i],
                1 => fdata[i].wrapping_add(a),
                2 => fdata[i].wrapping_add(b),
                3 => fdata[i].wrapping_add((((u16::from(a)) + u16::from(b)) >> 1) as u8),
                4 => fdata[i].wrapping_add(paeth(a, b, c)),
                _ => return Err(format!("unsupported PNG filter {filt}")),
            };
        }
        let dst_row = &mut rgba[y * (width as usize) * 4..(y + 1) * (width as usize) * 4];
        for x in 0..width as usize {
            let d = &mut dst_row[x * 4..x * 4 + 4];
            match color_type {
                0 => {
                    let g = cur[x];
                    d[0] = g;
                    d[1] = g;
                    d[2] = g;
                    d[3] = 255;
                }
                2 => {
                    d[0] = cur[x * 3];
                    d[1] = cur[x * 3 + 1];
                    d[2] = cur[x * 3 + 2];
                    d[3] = 255;
                }
                3 => {
                    let idx = cur[x] as usize;
                    let rgb = plte.get(idx).copied().unwrap_or([0, 0, 0]);
                    d[0] = rgb[0];
                    d[1] = rgb[1];
                    d[2] = rgb[2];
                    d[3] = trns.get(idx).copied().unwrap_or(255);
                }
                4 => {
                    let g = cur[x * 2];
                    d[0] = g;
                    d[1] = g;
                    d[2] = g;
                    d[3] = cur[x * 2 + 1];
                }
                6 => {
                    d.copy_from_slice(&cur[x * 4..x * 4 + 4]);
                }
                _ => unreachable!(),
            }
        }
        prev.copy_from_slice(&cur);
    }

    Ok(RgbaImage {
        width,
        height,
        data: rgba,
    })
}

pub fn resize_rgba(src: &RgbaImage, dst_w: u32, dst_h: u32) -> RgbaImage {
    if src.width == dst_w && src.height == dst_h {
        return src.clone();
    }
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    if dst_w == 0 || dst_h == 0 || src.width == 0 || src.height == 0 {
        return RgbaImage {
            width: dst_w,
            height: dst_h,
            data: out,
        };
    }

    let sw = src.width as f64;
    let sh = src.height as f64;
    let dw = dst_w as f64;
    let dh = dst_h as f64;

    for dy in 0..dst_h {
        let sy0 = (dy as f64 * sh / dh).floor() as u32;
        let sy1 = (((dy + 1) as f64) * sh / dh).ceil().min(sh) as u32;
        for dx in 0..dst_w {
            let sx0 = (dx as f64 * sw / dw).floor() as u32;
            let sx1 = (((dx + 1) as f64) * sw / dw).ceil().min(sw) as u32;
            let mut acc_rgba = [0.0f64; 4];
            let mut cnt = 0.0f64;
            let sy_end = sy1.max(sy0 + 1).min(src.height);
            let sx_end = sx1.max(sx0 + 1).min(src.width);
            for sy in sy0..sy_end {
                for sx in sx0..sx_end {
                    let off = ((sy * src.width + sx) * 4) as usize;
                    let a = f64::from(src.data[off + 3]) / 255.0;
                    acc_rgba[0] += f64::from(src.data[off]) * a;
                    acc_rgba[1] += f64::from(src.data[off + 1]) * a;
                    acc_rgba[2] += f64::from(src.data[off + 2]) * a;
                    acc_rgba[3] += f64::from(src.data[off + 3]);
                    cnt += 1.0;
                }
            }
            let doff = ((dy * dst_w + dx) * 4) as usize;
            if cnt > 0.0 {
                let avg_a = acc_rgba[3] / cnt;
                out[doff + 3] = avg_a.round().clamp(0.0, 255.0) as u8;
                if avg_a > 0.5 {
                    let scale = 255.0 / (avg_a * cnt);
                    out[doff] = (acc_rgba[0] * scale).round().clamp(0.0, 255.0) as u8;
                    out[doff + 1] = (acc_rgba[1] * scale).round().clamp(0.0, 255.0) as u8;
                    out[doff + 2] = (acc_rgba[2] * scale).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    RgbaImage {
        width: dst_w,
        height: dst_h,
        data: out,
    }
}

fn push_png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(tag);
    body.extend_from_slice(data);
    let c = crc32(&body);
    let _ = out.write_all(&(data.len() as u32).to_be_bytes());
    let _ = out.write_all(&body);
    let _ = out.write_all(&c.to_be_bytes());
}

pub fn encode_png_rgba(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let row_bytes = (img.width as usize) * 4;
    if img.data.len() != row_bytes * (img.height as usize) {
        return Err("RGBA buffer size mismatch".into());
    }
    let mut scan = Vec::with_capacity((row_bytes + 1) * (img.height as usize));
    for y in 0..img.height as usize {
        scan.push(0); // Filter: None
        scan.extend_from_slice(&img.data[y * row_bytes..(y + 1) * row_bytes]);
    }
    let comp = compress_zlib(&scan).ok_or_else(|| "libz.so.1 zlib compression unavailable".to_string())?;

    let mut out = Vec::with_capacity(64 + comp.len());
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&img.width.to_be_bytes());
    ihdr.extend_from_slice(&img.height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // no interlace
    push_png_chunk(&mut out, b"IHDR", &ihdr);
    push_png_chunk(&mut out, b"IDAT", &comp);
    push_png_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

#[allow(dead_code)]
fn _keep_c_uint(_: c_uint) -> ptr::NonNull<u8> {
    ptr::NonNull::dangling()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_standard_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn zlib_and_png_roundtrip() {
        let mut raw = vec![0u8; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let off = (y * 16 + x) * 4;
                raw[off] = (x * 16) as u8;
                raw[off + 1] = (y * 16) as u8;
                raw[off + 2] = 0x5A;
                raw[off + 3] = 0xFF;
            }
        }
        let img = RgbaImage {
            width: 16,
            height: 16,
            data: raw,
        };
        let encoded = encode_png_rgba(&img).expect("encode png");
        let decoded = decode_png_rgba(&encoded).expect("decode png");
        assert_eq!(decoded.width, 16);
        assert_eq!(decoded.height, 16);
        assert_eq!(decoded.data, img.data);

        let small = resize_rgba(&decoded, 8, 8);
        assert_eq!(small.width, 8);
        assert_eq!(small.height, 8);
        assert_eq!(small.data.len(), 8 * 8 * 4);
    }

    #[test]
    fn zip_writer_builds_valid_container() {
        let mut zw = ZipWriter::new();
        zw.add_file("salsa/hello.txt", b"hello hello hello hello", 0o644)
            .unwrap();
        zw.add_symlink("salsa/link.txt", "hello.txt").unwrap();
        let bytes = zw.finish().unwrap();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        assert!(bytes.windows(4).any(|w| w == [0x50, 0x4b, 0x05, 0x06]));
    }
}
