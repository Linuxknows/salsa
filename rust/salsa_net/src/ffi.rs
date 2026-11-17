#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::c_char;
use core::ptr;
use core::slice;
use std::ffi::CStr;

use crate::bluez_wire;
use crate::failsafe;
use crate::pa_wire::{self, PaPropBag, TagReader};
use crate::pw_wire::{self, Dict, PodCursor, PWB_MAX_PROPS};

#[repr(C)]
pub struct SalsaPaTagread {
    pub p: *const u8,
    pub len: usize,
    pub pos: usize,
}

#[repr(C)]
pub struct SalsaPaWireSs {
    pub format: u8,
    pub channels: u8,
    pub rate: u32,
}

#[repr(C)]
pub struct SalsaPaWireMap {
    pub channels: u8,
    pub map: [u8; 32],
}

#[repr(C)]
pub struct SalsaPaWireCvol {
    pub channels: u8,
    pub values: [u32; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaPaKv {
    pub key: [u8; 64],
    pub val: [u8; 128],
}

#[repr(C)]
pub struct SalsaPaPropbag {
    pub items: [SalsaPaKv; 24],
    pub count: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SalsaPwbFrac {
    pub num: u32,
    pub denom: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SalsaPwbKv {
    pub key: [u8; 64],
    pub val: [u8; 160],
}

#[repr(C)]
pub struct SalsaPwbDict {
    pub items: [SalsaPwbKv; 48],
    pub count: u32,
}

#[repr(C)]
pub struct SalsaPwbVfmtCaps {
    pub formats: [u32; 32],
    pub n_formats: u32,
    pub modifiers: [u64; 64],
    pub n_modifiers: u32,
    pub has_modifier: bool,
    pub mod_dont_fixate: bool,
    pub w_def: u32,
    pub w_min: u32,
    pub w_max: u32,
    pub h_def: u32,
    pub h_min: u32,
    pub h_max: u32,
    pub fps_def: SalsaPwbFrac,
    pub fps_min: SalsaPwbFrac,
    pub fps_max: SalsaPwbFrac,
    pub max_fps_def: SalsaPwbFrac,
    pub max_fps_min: SalsaPwbFrac,
    pub max_fps_max: SalsaPwbFrac,
    pub has_fps: bool,
    pub has_max_fps: bool,
    pub valid: bool,
}

#[repr(C)]
pub struct SalsaPwbBufCaps {
    pub buf_def: u32,
    pub buf_min: u32,
    pub buf_max: u32,
    pub size_def: u32,
    pub size_min: u32,
    pub size_max: u32,
    pub stride_def: u32,
    pub datatype_mask: u32,
    pub valid: bool,
}

#[repr(C)]
pub struct SalsaPwbParamParsed {
    pub kind: u32,
    pub vfmt: SalsaPwbVfmtCaps,
    pub bcaps: SalsaPwbBufCaps,
}

#[repr(C)]
pub struct SalsaPwbCur {
    pub base: *const u8,
    pub off: usize,
    pub end: usize,
}

#[repr(C)]
pub struct SalsaFailsafeStats {
    pub is_rust: bool,
    pub armed: bool,
    pub trips_total: u64,
    pub pa_quarantined: u64,
    pub pw_quarantined: u64,
    pub bt_fallbacks: u64,
    pub fd_leaks_blocked: u64,
    pub manual_resets: u64,
    pub last_reason: [u8; 96],
}

unsafe fn slice_from_raw<'a>(p: *const u8, len: usize) -> Option<&'a [u8]> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { slice::from_raw_parts(p, len) })
    }
}

unsafe fn tr_to_rust<'a>(tr: *const SalsaPaTagread) -> Option<TagReader<'a>> {
    if tr.is_null() {
        return None;
    }
    let tr_ref = unsafe { &*tr };
    let buf = unsafe { slice_from_raw(tr_ref.p, tr_ref.len) }?;
    TagReader::new(buf, tr_ref.pos)
}

unsafe fn c_propbag_to_rust(pb: &SalsaPaPropbag) -> PaPropBag {
    let mut r = PaPropBag::default();
    let cnt = pb.count.clamp(0, pa_wire::PA_PROPBAG_CAP as i32) as usize;
    r.count = cnt;
    for i in 0..cnt {
        r.items[i].key = pb.items[i].key;
        r.items[i].val = pb.items[i].val;
    }
    r
}

fn rust_propbag_to_c(r: &PaPropBag, pb: &mut SalsaPaPropbag) {
    let cnt = r.count.min(pa_wire::PA_PROPBAG_CAP);
    pb.count = cnt as i32;
    for i in 0..cnt {
        pb.items[i].key = r.items[i].key;
        pb.items[i].val = r.items[i].val;
    }
}

unsafe fn c_dict_to_rust(d: &SalsaPwbDict) -> Dict {
    let mut r = Dict::default();
    let cnt = (d.count as usize).min(PWB_MAX_PROPS);
    r.count = cnt as u32;
    for i in 0..cnt {
        r.items[i].key = d.items[i].key;
        r.items[i].val = d.items[i].val;
    }
    r
}

fn rust_dict_to_c(r: &Dict, d: &mut SalsaPwbDict) {
    let cnt = (r.count as usize).min(PWB_MAX_PROPS);
    d.count = cnt as u32;
    for i in 0..cnt {
        d.items[i].key = r.items[i].key;
        d.items[i].val = r.items[i].val;
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_init(tr: *mut SalsaPaTagread, p: *const u8, len: usize) {
    if tr.is_null() {
        return;
    }
    unsafe {
        (*tr).p = p;
        (*tr).len = if p.is_null() { 0 } else { len };
        (*tr).pos = 0;
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_u32(tr: *mut SalsaPaTagread, out: *mut u32) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some(v) = r.read_u32() else {
        return false;
    };
    if !out.is_null() {
        unsafe { *out = v };
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_u8(tr: *mut SalsaPaTagread, out: *mut u8) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some(v) = r.read_u8() else {
        return false;
    };
    if !out.is_null() {
        unsafe { *out = v };
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_bool(tr: *mut SalsaPaTagread, out: *mut bool) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some(v) = r.read_bool() else {
        return false;
    };
    if !out.is_null() {
        unsafe { *out = v };
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_str_slice(
    tr: *mut SalsaPaTagread,
    out_off: *mut usize,
    out_len: *mut usize,
    out_is_null: *mut bool,
) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some(res) = r.read_str_slice() else {
        return false;
    };
    match res {
        None => {
            if !out_off.is_null() {
                unsafe { *out_off = 0 };
            }
            if !out_len.is_null() {
                unsafe { *out_len = 0 };
            }
            if !out_is_null.is_null() {
                unsafe { *out_is_null = true };
            }
        }
        Some((off, slen)) => {
            if !out_off.is_null() {
                unsafe { *out_off = off };
            }
            if !out_len.is_null() {
                unsafe { *out_len = slen };
            }
            if !out_is_null.is_null() {
                unsafe { *out_is_null = false };
            }
        }
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_arbitrary_slice(
    tr: *mut SalsaPaTagread,
    out_off: *mut usize,
    out_len: *mut usize,
) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some((off, blen)) = r.read_arbitrary_slice() else {
        return false;
    };
    if !out_off.is_null() {
        unsafe { *out_off = off };
    }
    if !out_len.is_null() {
        unsafe { *out_len = blen };
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_ss(tr: *mut SalsaPaTagread, ss: *mut SalsaPaWireSs) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some(res) = r.read_ss() else {
        return false;
    };
    if !ss.is_null() {
        unsafe {
            (*ss).format = res.format;
            (*ss).channels = res.channels;
            (*ss).rate = res.rate;
        }
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_map(tr: *mut SalsaPaTagread, m: *mut SalsaPaWireMap) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some(res) = r.read_map() else {
        return false;
    };
    if !m.is_null() {
        unsafe {
            (*m).channels = res.channels;
            (*m).map = res.map;
        }
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_cvol(tr: *mut SalsaPaTagread, cv: *mut SalsaPaWireCvol) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some(res) = r.read_cvol() else {
        return false;
    };
    if !cv.is_null() {
        unsafe {
            (*cv).channels = res.channels;
            (*cv).values = res.values;
        }
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_timeval(
    tr: *mut SalsaPaTagread,
    sec: *mut u32,
    usec: *mut u32,
) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let Some((s, u)) = r.read_timeval() else {
        return false;
    };
    if !sec.is_null() {
        unsafe { *sec = s };
    }
    if !usec.is_null() {
        unsafe { *usec = u };
    }
    unsafe { (*tr).pos = r.pos };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_propbag_set(
    pb: *mut SalsaPaPropbag,
    k: *const c_char,
    v: *const c_char,
) {
    if pb.is_null() || k.is_null() || v.is_null() {
        return;
    }
    let ks = unsafe { CStr::from_ptr(k) }.to_bytes();
    let vs = unsafe { CStr::from_ptr(v) }.to_bytes();
    let mut r = unsafe { c_propbag_to_rust(&*pb) };
    r.set(ks, vs);
    rust_propbag_to_c(&r, unsafe { &mut *pb });
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_proplist(
    tr: *mut SalsaPaTagread,
    pb: *mut SalsaPaPropbag,
) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let ok = if pb.is_null() {
        r.read_proplist(None).is_some()
    } else {
        let mut rpb = unsafe { c_propbag_to_rust(&*pb) };
        let res = r.read_proplist(Some(&mut rpb)).is_some();
        rust_propbag_to_c(&rpb, unsafe { &mut *pb });
        res
    };
    unsafe { (*tr).pos = r.pos };
    ok
}

#[no_mangle]
pub unsafe extern "C" fn salsa_tr_skip_format_info(tr: *mut SalsaPaTagread) -> bool {
    let Some(mut r) = (unsafe { tr_to_rust(tr) }) else {
        return false;
    };
    let ok = r.skip_format_info().is_some();
    unsafe { (*tr).pos = r.pos };
    ok
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_parse_pkt_hdr(
    buf: *const u8,
    avail: usize,
    out_oid: *mut u32,
    out_op: *mut u8,
    out_psz: *mut u32,
    out_seq: *mut u32,
    out_nfds: *mut u32,
    out_pkt_len: *mut usize,
) -> bool {
    let Some(b) = (unsafe { slice_from_raw(buf, avail) }) else {
        return false;
    };
    let Some(hdr) = pw_wire::parse_pkt_hdr(b) else {
        return false;
    };
    if !out_oid.is_null() {
        unsafe { *out_oid = hdr.oid };
    }
    if !out_op.is_null() {
        unsafe { *out_op = hdr.op };
    }
    if !out_psz.is_null() {
        unsafe { *out_psz = hdr.psz };
    }
    if !out_seq.is_null() {
        unsafe { *out_seq = hdr.seq };
    }
    if !out_nfds.is_null() {
        unsafe { *out_nfds = hdr.nfds };
    }
    if !out_pkt_len.is_null() {
        unsafe { *out_pkt_len = hdr.pkt_len };
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_init(
    c: *mut SalsaPwbCur,
    data: *const core::ffi::c_void,
    len: usize,
) {
    if c.is_null() {
        return;
    }
    unsafe {
        (*c).base = data as *const u8;
        (*c).off = 0;
        (*c).end = if data.is_null() { 0 } else { len };
    }
}

unsafe fn pwb_cur_to_rust<'a>(c: *const SalsaPwbCur) -> Option<PodCursor<'a>> {
    if c.is_null() {
        return None;
    }
    let cr = unsafe { &*c };
    let buf = unsafe { slice_from_raw(cr.base, cr.end) }?;
    PodCursor::new(buf, cr.off, cr.end)
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_next_pod(
    c: *mut SalsaPwbCur,
    out_sz: *mut u32,
    out_tp: *mut u32,
    out_body_off: *mut usize,
    out_pod_total: *mut usize,
) -> bool {
    let Some(mut cur) = (unsafe { pwb_cur_to_rust(c) }) else {
        return false;
    };
    let Some((sz, tp, body_off, total)) = cur.next_pod() else {
        return false;
    };
    if !out_sz.is_null() {
        unsafe { *out_sz = sz };
    }
    if !out_tp.is_null() {
        unsafe { *out_tp = tp };
    }
    if !out_body_off.is_null() {
        unsafe { *out_body_off = body_off };
    }
    if !out_pod_total.is_null() {
        unsafe { *out_pod_total = total };
    }
    unsafe { (*c).off = cur.off };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_enter_struct(
    c: *mut SalsaPwbCur,
    inner: *mut SalsaPwbCur,
) -> bool {
    if inner.is_null() {
        return false;
    }
    let Some(mut cur) = (unsafe { pwb_cur_to_rust(c) }) else {
        return false;
    };
    let Some((body_off, sz)) = cur.enter_struct() else {
        return false;
    };
    let new_base = unsafe { (*c).base.add(body_off) };
    unsafe {
        (*c).off = cur.off;
        (*inner).base = new_base;
        (*inner).off = 0;
        (*inner).end = sz;
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_int(c: *mut SalsaPwbCur, out: *mut i32) -> bool {
    let Some(mut cur) = (unsafe { pwb_cur_to_rust(c) }) else {
        return false;
    };
    let Some(v) = cur.read_int() else {
        return false;
    };
    if !out.is_null() {
        unsafe { *out = v };
    }
    unsafe { (*c).off = cur.off };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_u32(c: *mut SalsaPwbCur, out: *mut u32) -> bool {
    let mut v = 0i32;
    if !(unsafe { salsa_pwb_cur_int(c, &mut v) }) {
        return false;
    }
    if !out.is_null() {
        unsafe { *out = v as u32 };
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_long(c: *mut SalsaPwbCur, out: *mut i64) -> bool {
    let Some(mut cur) = (unsafe { pwb_cur_to_rust(c) }) else {
        return false;
    };
    let Some(v) = cur.read_long() else {
        return false;
    };
    if !out.is_null() {
        unsafe { *out = v };
    }
    unsafe { (*c).off = cur.off };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_str(
    c: *mut SalsaPwbCur,
    out: *mut c_char,
    out_sz: usize,
) -> bool {
    let Some(mut cur) = (unsafe { pwb_cur_to_rust(c) }) else {
        return false;
    };
    let Some(sbytes) = cur.read_str_bytes() else {
        return false;
    };
    if !out.is_null() && out_sz > 0 {
        let copy_n = sbytes.len().min(out_sz - 1);
        let dst = unsafe { slice::from_raw_parts_mut(out as *mut u8, out_sz) };
        dst[..copy_n].copy_from_slice(&sbytes[..copy_n]);
        dst[copy_n] = 0;
    }
    unsafe { (*c).off = cur.off };
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_dict_set(
    d: *mut SalsaPwbDict,
    key: *const c_char,
    val: *const c_char,
) {
    if d.is_null() || key.is_null() {
        return;
    }
    let k = unsafe { CStr::from_ptr(key) }.to_bytes();
    if k.is_empty() {
        return;
    }
    let v = if val.is_null() {
        b"".as_slice()
    } else {
        unsafe { CStr::from_ptr(val) }.to_bytes()
    };
    let mut r = unsafe { c_dict_to_rust(&*d) };
    pw_wire::dict_set(&mut r, k, v);
    rust_dict_to_c(&r, unsafe { &mut *d });
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_cur_dict(
    c: *mut SalsaPwbCur,
    d: *mut SalsaPwbDict,
    wrapped_in_struct: bool,
) -> bool {
    if d.is_null() {
        return false;
    }
    let Some(mut cur) = (unsafe { pwb_cur_to_rust(c) }) else {
        return false;
    };
    let mut rd = unsafe { c_dict_to_rust(&*d) };
    let ok = cur.read_dict(&mut rd, wrapped_in_struct).is_some();
    rust_dict_to_c(&rd, unsafe { &mut *d });
    unsafe { (*c).off = cur.off };
    ok
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_parse_param_pod(
    pod: *const u8,
    pod_sz: usize,
    out: *mut SalsaPwbParamParsed,
) {
    if out.is_null() {
        return;
    }
    unsafe { ptr::write_bytes(out, 0, 1) };
    let Some(buf) = (unsafe { slice_from_raw(pod, pod_sz) }) else {
        return;
    };
    let parsed = pw_wire::parse_param_pod(buf);
    unsafe {
        (*out).kind = parsed.kind;
        (*out).vfmt.formats = parsed.vfmt.formats;
        (*out).vfmt.n_formats = parsed.vfmt.n_formats;
        (*out).vfmt.modifiers = parsed.vfmt.modifiers;
        (*out).vfmt.n_modifiers = parsed.vfmt.n_modifiers;
        (*out).vfmt.has_modifier = parsed.vfmt.has_modifier;
        (*out).vfmt.mod_dont_fixate = parsed.vfmt.mod_dont_fixate;
        (*out).vfmt.w_def = parsed.vfmt.w_def;
        (*out).vfmt.w_min = parsed.vfmt.w_min;
        (*out).vfmt.w_max = parsed.vfmt.w_max;
        (*out).vfmt.h_def = parsed.vfmt.h_def;
        (*out).vfmt.h_min = parsed.vfmt.h_min;
        (*out).vfmt.h_max = parsed.vfmt.h_max;
        (*out).vfmt.fps_def = SalsaPwbFrac {
            num: parsed.vfmt.fps_def.num,
            denom: parsed.vfmt.fps_def.denom,
        };
        (*out).vfmt.fps_min = SalsaPwbFrac {
            num: parsed.vfmt.fps_min.num,
            denom: parsed.vfmt.fps_min.denom,
        };
        (*out).vfmt.fps_max = SalsaPwbFrac {
            num: parsed.vfmt.fps_max.num,
            denom: parsed.vfmt.fps_max.denom,
        };
        (*out).vfmt.max_fps_def = SalsaPwbFrac {
            num: parsed.vfmt.max_fps_def.num,
            denom: parsed.vfmt.max_fps_def.denom,
        };
        (*out).vfmt.max_fps_min = SalsaPwbFrac {
            num: parsed.vfmt.max_fps_min.num,
            denom: parsed.vfmt.max_fps_min.denom,
        };
        (*out).vfmt.max_fps_max = SalsaPwbFrac {
            num: parsed.vfmt.max_fps_max.num,
            denom: parsed.vfmt.max_fps_max.denom,
        };
        (*out).vfmt.has_fps = parsed.vfmt.has_fps;
        (*out).vfmt.has_max_fps = parsed.vfmt.has_max_fps;
        (*out).vfmt.valid = parsed.vfmt.valid;

        (*out).bcaps.buf_def = parsed.bcaps.buf_def;
        (*out).bcaps.buf_min = parsed.bcaps.buf_min;
        (*out).bcaps.buf_max = parsed.bcaps.buf_max;
        (*out).bcaps.size_def = parsed.bcaps.size_def;
        (*out).bcaps.size_min = parsed.bcaps.size_min;
        (*out).bcaps.size_max = parsed.bcaps.size_max;
        (*out).bcaps.stride_def = parsed.bcaps.stride_def;
        (*out).bcaps.datatype_mask = parsed.bcaps.datatype_mask;
        (*out).bcaps.valid = parsed.bcaps.valid;
    }
}

#[no_mangle]
pub extern "C" fn salsa_pwb_vfmt_stride(f: u32, w: u32) -> u32 {
    pw_wire::vfmt_stride(f, w)
}

#[no_mangle]
pub extern "C" fn salsa_pwb_vfmt_size(f: u32, w: u32, h: u32) -> u32 {
    pw_wire::vfmt_size(f, w, h)
}

#[no_mangle]
pub unsafe extern "C" fn salsa_bt_select_a2dp_config(
    codec_kind: i32,
    rcaps: *const u8,
    rlen: usize,
    out_cfg: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
    out_cap_bit: *mut u32,
) -> i32 {
    if rcaps.is_null() || out_cfg.is_null() || out_len.is_null() {
        return -1;
    }
    unsafe { *out_len = 0 };
    if !out_cap_bit.is_null() {
        unsafe { *out_cap_bit = 0 };
    }
    let r_slice = unsafe { slice::from_raw_parts(rcaps, rlen) };
    let w_slice = unsafe { slice::from_raw_parts_mut(out_cfg, out_cap) };
    match bluez_wire::select_a2dp_config(codec_kind, r_slice, w_slice) {
        Some((len, bit)) => {
            unsafe { *out_len = len };
            if !out_cap_bit.is_null() {
                unsafe { *out_cap_bit = bit };
            }
            0
        }
        None => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_bt_parse_bdaddr(
    str_ptr: *const c_char,
    len: usize,
    out_b: *mut u8,
) -> bool {
    if str_ptr.is_null() || out_b.is_null() {
        return false;
    }
    let bytes = unsafe { slice::from_raw_parts(str_ptr as *const u8, len) };
    let Some(parsed) = bluez_wire::parse_bdaddr(bytes) else {
        return false;
    };
    let dst = unsafe { slice::from_raw_parts_mut(out_b, 6) };
    dst.copy_from_slice(&parsed);
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pa_parse_frame_hdr(
    hdr: *const u8,
    out_body_len: *mut u32,
    out_channel: *mut u32,
    out_flags: *mut u32,
) -> bool {
    let Some(h) = (unsafe { slice_from_raw(hdr, 20) }) else {
        return false;
    };
    let Some(parsed) = pa_wire::parse_frame_hdr(h) else {
        return false;
    };
    if !out_body_len.is_null() {
        unsafe { *out_body_len = parsed.body_len };
    }
    if !out_channel.is_null() {
        unsafe { *out_channel = parsed.channel };
    }
    if !out_flags.is_null() {
        unsafe { *out_flags = parsed.flags };
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_pwb_fd_quota_check(
    current_nfds: u32,
    incoming_cnt: u32,
    out_accept: *mut u32,
) -> bool {
    let (ok, accept) = pw_wire::fd_quota_check(current_nfds, incoming_cnt);
    if !out_accept.is_null() {
        unsafe { *out_accept = accept };
    }
    ok
}

#[no_mangle]
pub extern "C" fn salsa_wire_is_rust() -> bool {
    true
}

#[no_mangle]
pub unsafe extern "C" fn salsa_failsafe_record_trip(kind: u32, reason: *const c_char) {
    let r = if reason.is_null() {
        b"".as_slice()
    } else {
        unsafe { CStr::from_ptr(reason) }.to_bytes()
    };
    failsafe::record_trip(kind, r);
}

#[no_mangle]
pub unsafe extern "C" fn salsa_failsafe_get_stats(out: *mut SalsaFailsafeStats) {
    if out.is_null() {
        return;
    }
    let st = failsafe::get_stats();
    unsafe {
        (*out).is_rust = st.is_rust;
        (*out).armed = st.armed;
        (*out).trips_total = st.trips_total;
        (*out).pa_quarantined = st.pa_quarantined;
        (*out).pw_quarantined = st.pw_quarantined;
        (*out).bt_fallbacks = st.bt_fallbacks;
        (*out).fd_leaks_blocked = st.fd_leaks_blocked;
        (*out).manual_resets = st.manual_resets;
        (*out).last_reason = st.last_reason;
    }
}

#[no_mangle]
pub extern "C" fn salsa_failsafe_reset_stats() {
    failsafe::reset_stats();
}
