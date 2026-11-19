#![forbid(unsafe_code)]

pub const PWB_MAX_PROPS: usize = 48;
pub const PWB_MAX_MODIFIERS: usize = 64;
pub const PWB_MAX_PACKET_SZ: u32 = 16 * 1024 * 1024;
pub const PWB_MAX_CLIENT_FDS: u32 = 256;

pub const SPA_NONE: u32 = 1;
pub const SPA_BOOL: u32 = 2;
pub const SPA_ID: u32 = 3;
pub const SPA_INT: u32 = 4;
pub const SPA_LONG: u32 = 5;
pub const SPA_STRING: u32 = 8;
pub const SPA_RECT: u32 = 10;
pub const SPA_FRAC: u32 = 11;
pub const SPA_STRUCT: u32 = 14;
pub const SPA_OBJECT: u32 = 15;
pub const SPA_FD: u32 = 18;
pub const SPA_CHOICE: u32 = 19;

pub const OBJ_FORMAT: u32 = 0x40003;
pub const OBJ_PARAM_BUFS: u32 = 0x40004;
pub const PARAM_ENUM_FORMAT: u32 = 3;
pub const PARAM_FORMAT: u32 = 4;
pub const PARAM_BUFFERS: u32 = 5;
pub const DATA_MEM_FD: u32 = 2;

pub const CHOICE_RANGE: u32 = 1;
pub const CHOICE_ENUM: u32 = 3;
pub const POD_PROP_FLAG_DONT_FIXATE: u32 = 1 << 4;

pub const MEDIA_VIDEO: u32 = 2;
pub const SUB_RAW: u32 = 1;

pub const VF_I420: u32 = 2;
pub const VF_YUY2: u32 = 4;
pub const VF_RGBX: u32 = 7;
pub const VF_BGRX: u32 = 8;
pub const VF_XRGB: u32 = 9;
pub const VF_XBGR: u32 = 10;
pub const VF_RGBA: u32 = 11;
pub const VF_BGRA: u32 = 12;
pub const VF_ARGB: u32 = 13;
pub const VF_ABGR: u32 = 14;
pub const VF_RGB: u32 = 15;
pub const VF_BGR: u32 = 16;
pub const VF_NV12: u32 = 23;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PktHdr {
    pub oid: u32,
    pub op: u8,
    pub psz: u32,
    pub seq: u32,
    pub nfds: u32,
    pub pkt_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Frac {
    pub num: u32,
    pub denom: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DictKv {
    pub key: [u8; 64],
    pub val: [u8; 160],
}

impl Default for DictKv {
    fn default() -> Self {
        Self {
            key: [0; 64],
            val: [0; 160],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dict {
    pub items: [DictKv; PWB_MAX_PROPS],
    pub count: u32,
}

impl Default for Dict {
    fn default() -> Self {
        Self {
            items: [DictKv::default(); PWB_MAX_PROPS],
            count: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VfmtCaps {
    pub formats: [u32; 32],
    pub n_formats: u32,
    pub modifiers: [u64; PWB_MAX_MODIFIERS],
    pub n_modifiers: u32,
    pub has_modifier: bool,
    pub mod_dont_fixate: bool,
    pub w_def: u32,
    pub w_min: u32,
    pub w_max: u32,
    pub h_def: u32,
    pub h_min: u32,
    pub h_max: u32,
    pub fps_def: Frac,
    pub fps_min: Frac,
    pub fps_max: Frac,
    pub max_fps_def: Frac,
    pub max_fps_min: Frac,
    pub max_fps_max: Frac,
    pub has_fps: bool,
    pub has_max_fps: bool,
    pub valid: bool,
}

impl Default for VfmtCaps {
    fn default() -> Self {
        Self {
            formats: [0; 32],
            n_formats: 0,
            modifiers: [0; PWB_MAX_MODIFIERS],
            n_modifiers: 0,
            has_modifier: false,
            mod_dont_fixate: false,
            w_def: 0,
            w_min: 0,
            w_max: 0,
            h_def: 0,
            h_min: 0,
            h_max: 0,
            fps_def: Frac::default(),
            fps_min: Frac::default(),
            fps_max: Frac::default(),
            max_fps_def: Frac::default(),
            max_fps_min: Frac::default(),
            max_fps_max: Frac::default(),
            has_fps: false,
            has_max_fps: false,
            valid: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BufCaps {
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

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ParamParsed {
    pub kind: u32,
    pub vfmt: VfmtCaps,
    pub bcaps: BufCaps,
}

fn le32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn le64(b: &[u8], off: usize) -> Option<u64> {
    let s = b.get(off..off.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn pad8(n: usize) -> usize {
    match n.checked_add(7) {
        Some(v) => v & !7usize,
        None => n,
    }
}

fn cstr_bytes(buf: &[u8]) -> &[u8] {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    &buf[..end]
}

fn copy_cstr<const N: usize>(dst: &mut [u8; N], src: &[u8]) {
    dst.fill(0);
    if N == 0 {
        return;
    }
    let clean = cstr_bytes(src);
    let n = clean.len().min(N - 1);
    dst[..n].copy_from_slice(&clean[..n]);
}

pub fn fd_quota_check(current_nfds: u32, incoming_cnt: u32) -> (bool, u32) {
    if current_nfds >= PWB_MAX_CLIENT_FDS {
        return (false, 0);
    }
    let avail = PWB_MAX_CLIENT_FDS - current_nfds;
    if incoming_cnt > avail {
        (false, avail)
    } else {
        (true, incoming_cnt)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PktHdrStatus {
    Ready(PktHdr),
    Incomplete,
    Malformed,
}

pub fn parse_pkt_hdr_status(buf: &[u8]) -> PktHdrStatus {
    if buf.len() < 16 {
        return PktHdrStatus::Incomplete;
    }
    let Some(oid) = le32(buf, 0) else {
        return PktHdrStatus::Incomplete;
    };
    let Some(w1) = le32(buf, 4) else {
        return PktHdrStatus::Incomplete;
    };
    let Some(seq) = le32(buf, 8) else {
        return PktHdrStatus::Incomplete;
    };
    let Some(nfds) = le32(buf, 12) else {
        return PktHdrStatus::Incomplete;
    };
    let op = ((w1 >> 24) & 0xff) as u8;
    let psz = w1 & 0x00ffffff;
    if nfds > PWB_MAX_CLIENT_FDS || psz > PWB_MAX_PACKET_SZ {
        return PktHdrStatus::Malformed;
    }
    let Some(pkt_len) = 16usize.checked_add(pad8(psz as usize)) else {
        return PktHdrStatus::Malformed;
    };
    if buf.len() < pkt_len {
        return PktHdrStatus::Incomplete;
    }
    PktHdrStatus::Ready(PktHdr {
        oid,
        op,
        psz,
        seq,
        nfds,
        pkt_len,
    })
}

pub fn parse_pkt_hdr(buf: &[u8]) -> Option<PktHdr> {
    match parse_pkt_hdr_status(buf) {
        PktHdrStatus::Ready(h) => Some(h),
        PktHdrStatus::Incomplete | PktHdrStatus::Malformed => None,
    }
}

pub fn dict_set(d: &mut Dict, key: &[u8], val: &[u8]) {
    let k_clean = cstr_bytes(key);
    if k_clean.is_empty() {
        return;
    }
    let cnt = (d.count as usize).min(PWB_MAX_PROPS);
    let k_cmp = &k_clean[..k_clean.len().min(63)];
    for item in &mut d.items[..cnt] {
        if cstr_bytes(&item.key) == k_cmp {
            copy_cstr(&mut item.val, val);
            return;
        }
    }
    if cnt < PWB_MAX_PROPS {
        copy_cstr(&mut d.items[cnt].key, k_cmp);
        copy_cstr(&mut d.items[cnt].val, val);
        d.count = (cnt + 1) as u32;
    }
}

pub struct PodCursor<'a> {
    pub base: &'a [u8],
    pub off: usize,
    pub end: usize,
}

impl<'a> PodCursor<'a> {
    pub fn new(base: &'a [u8], off: usize, end: usize) -> Option<Self> {
        if off <= end && end <= base.len() {
            Some(Self { base, off, end })
        } else {
            None
        }
    }

    pub fn next_pod(&mut self) -> Option<(u32, u32, usize, usize)> {
        if self.off > self.end || self.end - self.off < 8 {
            return None;
        }
        let sz = le32(self.base, self.off)?;
        let tp = le32(self.base, self.off + 4)?;
        let rem_after = self.end - self.off - 8;
        if (sz as usize) > rem_after {
            return None;
        }
        let unpadded = 8usize + (sz as usize);
        let mut total = pad8(unpadded);
        if total < unpadded || total > self.end - self.off {
            total = unpadded;
        }
        let body_off = self.off + 8;
        self.off += total;
        Some((sz, tp, body_off, unpadded))
    }

    pub fn enter_struct(&mut self) -> Option<(usize, usize)> {
        let (sz, tp, body_off, _) = self.next_pod()?;
        if tp != SPA_STRUCT {
            return None;
        }
        Some((body_off, sz as usize))
    }

    pub fn enter_struct_cursor(&mut self) -> Option<Self> {
        let (body_off, sz) = self.enter_struct()?;
        Self::new(self.base, body_off, body_off.checked_add(sz)?)
    }

    pub fn read_int(&mut self) -> Option<i32> {
        let (sz, tp, body_off, _) = self.next_pod()?;
        if (tp != SPA_INT && tp != SPA_ID && tp != SPA_BOOL) || sz < 4 {
            return None;
        }
        Some(le32(self.base, body_off)? as i32)
    }

    pub fn read_u32(&mut self) -> Option<u32> {
        self.read_int().map(|v| v as u32)
    }

    pub fn read_long(&mut self) -> Option<i64> {
        let (sz, tp, body_off, _) = self.next_pod()?;
        if (tp != SPA_LONG && tp != SPA_FD) || sz < 8 {
            return None;
        }
        Some(le64(self.base, body_off)? as i64)
    }

    pub fn read_str_bytes(&mut self) -> Option<&'a [u8]> {
        let (sz, tp, body_off, _) = self.next_pod()?;
        if tp == SPA_NONE {
            return Some(&[]);
        }
        if tp != SPA_STRING {
            return None;
        }
        let body = self.base.get(body_off..body_off + sz as usize)?;
        Some(cstr_bytes(body))
    }

    pub fn read_dict(&mut self, d: &mut Dict, wrapped_in_struct: bool) -> Option<()> {
        if wrapped_in_struct {
            let (inner_off, inner_len) = self.enter_struct()?;
            let mut inner = PodCursor::new(self.base, inner_off, inner_off + inner_len)?;
            return inner.read_dict(d, false);
        }
        let n = self.read_int()?;
        if !(0..=1024).contains(&n) {
            return None;
        }
        for _ in 0..n {
            let k = self.read_str_bytes()?;
            let mut kbuf = [0u8; 64];
            copy_cstr(&mut kbuf, k);
            let v = self.read_str_bytes()?;
            let mut vbuf = [0u8; 160];
            copy_cstr(&mut vbuf, v);
            dict_set(d, &kbuf, &vbuf);
        }
        Some(())
    }
}

fn parse_choice_u32(
    tp: u32,
    body: &[u8],
    arr: Option<&mut [u32]>,
    out_cnt: Option<&mut u32>,
    out_def: Option<&mut u32>,
    out_min: Option<&mut u32>,
    out_max: Option<&mut u32>,
) {
    if tp == SPA_ID || tp == SPA_INT {
        if let Some(v) = le32(body, 0) {
            if let Some(d) = out_def {
                *d = v;
            }
            if let Some(m) = out_min {
                *m = v;
            }
            if let Some(m) = out_max {
                *m = v;
            }
            if let Some(a) = arr {
                if !a.is_empty() {
                    a[0] = v;
                    if let Some(c) = out_cnt {
                        *c = 1;
                    }
                }
            }
        }
        return;
    }
    if tp == SPA_CHOICE && body.len() >= 16 {
        let Some(ctype) = le32(body, 0) else { return };
        let Some(esz) = le32(body, 8) else { return };
        let Some(etp) = le32(body, 12) else { return };
        if (etp == SPA_ID || etp == SPA_INT) && esz == 4 {
            let n = (body.len() - 16) / 4;
            if n > 0 {
                let Some(v0) = le32(body, 16) else { return };
                if let Some(d) = out_def {
                    *d = v0;
                }
                if ctype == CHOICE_RANGE && n >= 3 {
                    if let (Some(vmin), Some(vmax)) = (le32(body, 20), le32(body, 24)) {
                        if let Some(m) = out_min {
                            *m = vmin;
                        }
                        if let Some(m) = out_max {
                            *m = vmax;
                        }
                    }
                } else {
                    if let Some(m) = out_min {
                        *m = v0;
                    }
                    if let Some(m) = out_max {
                        *m = v0;
                    }
                    if let Some(a) = arr {
                        if !a.is_empty() {
                            let mut c = 0usize;
                            let start = if n > 1 { 1 } else { 0 };
                            a[c] = v0;
                            c += 1;
                            for i in start..n {
                                if c >= a.len() {
                                    break;
                                }
                                let Some(vi) = le32(body, 16 + i * 4) else { break };
                                if !a[..c].contains(&vi) {
                                    a[c] = vi;
                                    c += 1;
                                }
                            }
                            if let Some(oc) = out_cnt {
                                *oc = c as u32;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn parse_choice_long(tp: u32, body: &[u8], arr: &mut [u64], out_cnt: &mut u32) {
    if tp == SPA_LONG {
        if let Some(v) = le64(body, 0) {
            if !arr.is_empty() {
                arr[0] = v;
                *out_cnt = 1;
            }
        }
        return;
    }
    if tp == SPA_CHOICE && body.len() >= 16 {
        let Some(esz) = le32(body, 8) else { return };
        let Some(etp) = le32(body, 12) else { return };
        if etp == SPA_LONG && esz == 8 {
            let n = (body.len() - 16) / 8;
            let mut c = 0usize;
            for i in 0..n {
                if c >= arr.len() {
                    break;
                }
                let Some(vi) = le64(body, 16 + i * 8) else { break };
                if !arr[..c].contains(&vi) {
                    arr[c] = vi;
                    c += 1;
                }
            }
            *out_cnt = c as u32;
        }
    }
}

fn parse_choice_rect(tp: u32, body: &[u8], c: &mut VfmtCaps) {
    if tp == SPA_RECT && body.len() >= 8 {
        if let (Some(w), Some(h)) = (le32(body, 0), le32(body, 4)) {
            c.w_def = w;
            c.w_min = w;
            c.w_max = w;
            c.h_def = h;
            c.h_min = h;
            c.h_max = h;
        }
        return;
    }
    if tp == SPA_CHOICE && body.len() >= 16 {
        let Some(ctype) = le32(body, 0) else { return };
        let Some(esz) = le32(body, 8) else { return };
        let Some(etp) = le32(body, 12) else { return };
        if etp == SPA_RECT && esz == 8 {
            let n = (body.len() - 16) / 8;
            if n >= 1 {
                let (Some(w0), Some(h0)) = (le32(body, 16), le32(body, 20)) else {
                    return;
                };
                c.w_def = w0;
                c.w_min = w0;
                c.w_max = w0;
                c.h_def = h0;
                c.h_min = h0;
                c.h_max = h0;
                if ctype == CHOICE_RANGE && n >= 3 {
                    if let (Some(wmin), Some(hmin), Some(wmax), Some(hmax)) = (
                        le32(body, 24),
                        le32(body, 28),
                        le32(body, 32),
                        le32(body, 36),
                    ) {
                        c.w_min = wmin;
                        c.h_min = hmin;
                        c.w_max = wmax;
                        c.h_max = hmax;
                    }
                } else if ctype == CHOICE_ENUM && n > 1 {
                    for i in 1..n {
                        let (Some(wi), Some(hi)) =
                            (le32(body, 16 + i * 8), le32(body, 20 + i * 8))
                        else {
                            break;
                        };
                        c.w_min = c.w_min.min(wi);
                        c.w_max = c.w_max.max(wi);
                        c.h_min = c.h_min.min(hi);
                        c.h_max = c.h_max.max(hi);
                    }
                }
            }
        }
    }
}

fn parse_choice_frac(
    tp: u32,
    body: &[u8],
    fdef: &mut Frac,
    fmin: &mut Frac,
    fmax: &mut Frac,
) {
    if tp == SPA_FRAC && body.len() >= 8 {
        if let (Some(num), Some(mut denom)) = (le32(body, 0), le32(body, 4)) {
            if denom == 0 {
                denom = 1;
            }
            let f = Frac { num, denom };
            *fdef = f;
            *fmin = f;
            *fmax = f;
        }
        return;
    }
    if tp == SPA_CHOICE && body.len() >= 16 {
        let Some(ctype) = le32(body, 0) else { return };
        let Some(esz) = le32(body, 8) else { return };
        let Some(etp) = le32(body, 12) else { return };
        if etp == SPA_FRAC && esz == 8 {
            let n = (body.len() - 16) / 8;
            if n >= 1 {
                let (Some(num0), Some(mut den0)) = (le32(body, 16), le32(body, 20)) else {
                    return;
                };
                if den0 == 0 {
                    den0 = 1;
                }
                let f0 = Frac {
                    num: num0,
                    denom: den0,
                };
                *fdef = f0;
                *fmin = f0;
                *fmax = f0;
                if ctype == CHOICE_RANGE && n >= 3 {
                    if let (Some(n1), Some(mut d1), Some(n2), Some(mut d2)) = (
                        le32(body, 24),
                        le32(body, 28),
                        le32(body, 32),
                        le32(body, 36),
                    ) {
                        if d1 == 0 {
                            d1 = 1;
                        }
                        if d2 == 0 {
                            d2 = 1;
                        }
                        let mut a = Frac { num: n1, denom: d1 };
                        let mut b = Frac { num: n2, denom: d2 };
                        if (a.num as u64) * (b.denom as u64) > (b.num as u64) * (a.denom as u64) {
                            core::mem::swap(&mut a, &mut b);
                        }
                        *fmin = a;
                        *fmax = b;
                    }
                } else if ctype == CHOICE_ENUM && n > 1 {
                    for i in 1..n {
                        let (Some(num), Some(mut denom)) =
                            (le32(body, 16 + i * 8), le32(body, 20 + i * 8))
                        else {
                            break;
                        };
                        if denom == 0 {
                            denom = 1;
                        }
                        let fi = Frac { num, denom };
                        if (fi.num as u64) * (fmin.denom as u64)
                            < (fmin.num as u64) * (fi.denom as u64)
                        {
                            *fmin = fi;
                        }
                        if (fi.num as u64) * (fmax.denom as u64)
                            > (fmax.num as u64) * (fi.denom as u64)
                        {
                            *fmax = fi;
                        }
                    }
                }
            }
        }
    }
}

pub fn parse_param_pod(pod: &[u8]) -> ParamParsed {
    let mut out = ParamParsed::default();
    if pod.len() < 16 {
        return out;
    }
    let Some(sz) = le32(pod, 0) else { return out };
    let Some(tp) = le32(pod, 4) else { return out };
    if tp != SPA_OBJECT || sz < 8 || (sz as usize) > pod.len() - 8 {
        return out;
    }
    let Some(otype) = le32(pod, 8) else { return out };
    let Some(oid) = le32(pod, 12) else { return out };
    let Some(mut cur) = PodCursor::new(pod, 16, 8 + sz as usize) else {
        return out;
    };

    if otype == OBJ_FORMAT && (oid == PARAM_ENUM_FORMAT || oid == PARAM_FORMAT) {
        let mut c = VfmtCaps {
            w_def: 1920,
            w_min: 1,
            w_max: 7680,
            h_def: 1080,
            h_min: 1,
            h_max: 4320,
            fps_def: Frac { num: 0, denom: 1 },
            fps_min: Frac { num: 0, denom: 1 },
            fps_max: Frac { num: 0, denom: 1 },
            max_fps_def: Frac { num: 60, denom: 1 },
            max_fps_min: Frac { num: 1, denom: 1 },
            max_fps_max: Frac { num: 240, denom: 1 },
            ..VfmtCaps::default()
        };
        let mut mtype = 0u32;
        let mut msub = 0u32;

        while cur.off <= cur.end && cur.end - cur.off >= 8 {
            let Some(k) = le32(cur.base, cur.off) else { break };
            let Some(fl) = le32(cur.base, cur.off + 4) else { break };
            cur.off += 8;
            let Some((vsz, vtp, body_off, _)) = cur.next_pod() else { break };
            let vbody = &cur.base[body_off..body_off + vsz as usize];
            match k {
                1 => parse_choice_u32(vtp, vbody, None, None, Some(&mut mtype), None, None),
                2 => parse_choice_u32(vtp, vbody, None, None, Some(&mut msub), None, None),
                0x20001 => parse_choice_u32(
                    vtp,
                    vbody,
                    Some(&mut c.formats),
                    Some(&mut c.n_formats),
                    None,
                    None,
                    None,
                ),
                0x20002 => {
                    parse_choice_long(vtp, vbody, &mut c.modifiers, &mut c.n_modifiers);
                    c.has_modifier = c.n_modifiers > 0;
                    c.mod_dont_fixate =
                        c.has_modifier && ((fl & POD_PROP_FLAG_DONT_FIXATE) != 0);
                }
                0x20003 => parse_choice_rect(vtp, vbody, &mut c),
                0x20004 => {
                    parse_choice_frac(vtp, vbody, &mut c.fps_def, &mut c.fps_min, &mut c.fps_max);
                    c.has_fps = true;
                }
                0x20005 => {
                    parse_choice_frac(
                        vtp,
                        vbody,
                        &mut c.max_fps_def,
                        &mut c.max_fps_min,
                        &mut c.max_fps_max,
                    );
                    c.has_max_fps = true;
                }
                _ => {}
            }
        }
        if (mtype == 0 || mtype == MEDIA_VIDEO) && (msub == 0 || msub == SUB_RAW) {
            if c.n_formats == 0 {
                c.formats[0] = VF_BGRX;
                c.formats[1] = VF_BGRA;
                c.n_formats = 2;
            }
            c.valid = true;
            out.kind = 1;
            out.vfmt = c;
        }
    } else if otype == OBJ_PARAM_BUFS && oid == PARAM_BUFFERS {
        let mut bc = BufCaps {
            buf_def: 3,
            buf_min: 2,
            buf_max: 8,
            datatype_mask: 1 << DATA_MEM_FD,
            ..BufCaps::default()
        };
        while cur.off <= cur.end && cur.end - cur.off >= 8 {
            let Some(k) = le32(cur.base, cur.off) else { break };
            cur.off += 8;
            let Some((vsz, vtp, body_off, _)) = cur.next_pod() else { break };
            let vbody = &cur.base[body_off..body_off + vsz as usize];
            match k {
                1 => parse_choice_u32(
                    vtp,
                    vbody,
                    None,
                    None,
                    Some(&mut bc.buf_def),
                    Some(&mut bc.buf_min),
                    Some(&mut bc.buf_max),
                ),
                3 => parse_choice_u32(
                    vtp,
                    vbody,
                    None,
                    None,
                    Some(&mut bc.size_def),
                    Some(&mut bc.size_min),
                    Some(&mut bc.size_max),
                ),
                4 => parse_choice_u32(vtp, vbody, None, None, Some(&mut bc.stride_def), None, None),
                6 => parse_choice_u32(
                    vtp,
                    vbody,
                    None,
                    None,
                    Some(&mut bc.datatype_mask),
                    None,
                    None,
                ),
                _ => {}
            }
        }
        bc.valid = true;
        out.kind = 2;
        out.bcaps = bc;
    }

    out
}

fn vfmt_bpp_num(f: u32) -> u32 {
    match f {
        VF_BGRX | VF_BGRA | VF_RGBX | VF_RGBA | VF_XRGB | VF_XBGR | VF_ARGB | VF_ABGR => 4,
        VF_RGB | VF_BGR | VF_NV12 | VF_I420 => 3,
        VF_YUY2 => 2,
        _ => 4,
    }
}

pub fn vfmt_stride(f: u32, w: u32) -> u32 {
    if f == VF_NV12 || f == VF_I420 {
        return w;
    }
    let s = (w as u64) * (vfmt_bpp_num(f) as u64);
    if s > u32::MAX as u64 {
        u32::MAX
    } else {
        s as u32
    }
}

pub fn vfmt_size(f: u32, w: u32, h: u32) -> u32 {
    let total = if f == VF_NV12 || f == VF_I420 {
        ((w as u64) * (h as u64) * 3) / 2
    } else {
        (vfmt_stride(f, w) as u64) * (h as u64)
    };
    if total > u32::MAX as u64 {
        u32::MAX
    } else {
        total as u32
    }
}

pub const DATA_DMA_BUF: u32 = 3;
pub const DATA_MEM_ID: u32 = 4;
pub const SPA_ID_INVALID: u32 = 0xffff_ffff;
pub const DRM_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
pub const SPA_ASYNC_BIT: u32 = 0x4000_0000;
pub const META_HEADER: u32 = 1;
pub const META_VIDEO_CROP: u32 = 2;
pub const META_BUSY: u32 = 7;
pub const IO_BUFFERS: u32 = 1;
pub const IO_CLOCK: u32 = 3;
pub const IO_POSITION: u32 = 7;
pub const EVT_NODE: u32 = 0x20002;
pub const CMD_NODE: u32 = 0x30002;
pub const CHOICE_NONE: u32 = 0;

pub fn vfmt_name(f: u32) -> &'static str {
    match f {
        VF_BGRX => "BGRx",
        VF_BGRA => "BGRA",
        VF_RGBX => "RGBx",
        VF_RGBA => "RGBA",
        VF_XRGB => "xRGB",
        VF_XBGR => "xBGR",
        VF_ARGB => "ARGB",
        VF_ABGR => "ABGR",
        VF_RGB => "RGB",
        VF_BGR => "BGR",
        VF_NV12 => "NV12",
        VF_I420 => "I420",
        VF_YUY2 => "YUY2",
        _ => "Unknown",
    }
}

impl Dict {
    pub fn set_str(&mut self, key: &str, val: &str) {
        dict_set(self, key.as_bytes(), val.as_bytes());
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        let k = key.as_bytes();
        let cnt = (self.count as usize).min(PWB_MAX_PROPS);
        for item in &self.items[..cnt] {
            if cstr_bytes(&item.key) == k {
                return Some(String::from_utf8_lossy(cstr_bytes(&item.val)).into_owned());
            }
        }
        None
    }
}

struct PodFrame {
    off: usize,
    hdr_sz: usize,
}

pub struct PodBuilder {
    buf: Vec<u8>,
    frames: Vec<PodFrame>,
}

impl PodBuilder {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap.max(64)),
            frames: Vec::with_capacity(8),
        }
    }

    fn pad(&mut self) {
        let p = pad8(self.buf.len());
        if p > self.buf.len() {
            self.buf.resize(p, 0);
        }
    }

    fn hdr(&mut self, sz: u32, tp: u32) {
        self.buf.extend_from_slice(&sz.to_le_bytes());
        self.buf.extend_from_slice(&tp.to_le_bytes());
    }

    pub fn none(&mut self) {
        self.hdr(0, SPA_NONE);
    }

    pub fn id(&mut self, v: u32) {
        self.hdr(4, SPA_ID);
        self.buf.extend_from_slice(&v.to_le_bytes());
        self.pad();
    }

    pub fn int(&mut self, v: i32) {
        self.hdr(4, SPA_INT);
        self.buf.extend_from_slice(&v.to_le_bytes());
        self.pad();
    }

    pub fn long(&mut self, v: i64) {
        self.hdr(8, SPA_LONG);
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn fd(&mut self, idx: i64) {
        self.hdr(8, SPA_FD);
        self.buf.extend_from_slice(&idx.to_le_bytes());
    }

    pub fn rect(&mut self, w: u32, h: u32) {
        self.hdr(8, SPA_RECT);
        self.buf.extend_from_slice(&w.to_le_bytes());
        self.buf.extend_from_slice(&h.to_le_bytes());
    }

    pub fn frac(&mut self, num: u32, den: u32) {
        self.hdr(8, SPA_FRAC);
        self.buf.extend_from_slice(&num.to_le_bytes());
        self.buf.extend_from_slice(&(if den == 0 { 1 } else { den }).to_le_bytes());
    }

    pub fn str_val(&mut self, s: Option<&str>) {
        let Some(txt) = s else {
            self.none();
            return;
        };
        let b = txt.as_bytes();
        let clean = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
        self.hdr((clean.len() + 1) as u32, SPA_STRING);
        self.buf.extend_from_slice(clean);
        self.buf.push(0);
        self.pad();
    }

    pub fn raw_pod(&mut self, pod: &[u8]) {
        if pod.len() < 8 {
            self.none();
            return;
        }
        self.buf.extend_from_slice(pod);
        self.pad();
    }

    pub fn push_struct(&mut self) {
        let off = self.buf.len();
        self.frames.push(PodFrame { off, hdr_sz: 8 });
        self.hdr(0, SPA_STRUCT);
    }

    pub fn push_object(&mut self, obj_type: u32, obj_id: u32) {
        let off = self.buf.len();
        self.frames.push(PodFrame { off, hdr_sz: 8 });
        self.hdr(8, SPA_OBJECT);
        self.buf.extend_from_slice(&obj_type.to_le_bytes());
        self.buf.extend_from_slice(&obj_id.to_le_bytes());
    }

    pub fn push_choice(&mut self, ctype: u32, cflags: u32, esz: u32, etp: u32) {
        let off = self.buf.len();
        self.frames.push(PodFrame { off, hdr_sz: 8 });
        self.hdr(16, SPA_CHOICE);
        self.buf.extend_from_slice(&ctype.to_le_bytes());
        self.buf.extend_from_slice(&cflags.to_le_bytes());
        self.buf.extend_from_slice(&esz.to_le_bytes());
        self.buf.extend_from_slice(&etp.to_le_bytes());
    }

    pub fn write_u64_raw(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn prop(&mut self, key: u32, flags: u32) {
        self.buf.extend_from_slice(&key.to_le_bytes());
        self.buf.extend_from_slice(&flags.to_le_bytes());
    }

    pub fn pop(&mut self) {
        if let Some(frame) = self.frames.pop() {
            if self.buf.len() >= frame.off + frame.hdr_sz {
                let sz = (self.buf.len() - (frame.off + frame.hdr_sz)) as u32;
                self.buf[frame.off..frame.off + 4].copy_from_slice(&sz.to_le_bytes());
            }
            self.pad();
        }
    }

    pub fn dict(&mut self, d: &Dict) {
        self.push_struct();
        let cnt = (d.count as usize).min(PWB_MAX_PROPS);
        self.int(cnt as i32);
        for item in &d.items[..cnt] {
            let ks = String::from_utf8_lossy(cstr_bytes(&item.key));
            let vs = String::from_utf8_lossy(cstr_bytes(&item.val));
            self.str_val(Some(&ks));
            if vs.starts_with("pointer:") {
                self.str_val(Some(""));
            } else {
                self.str_val(Some(&vs));
            }
        }
        self.pop();
    }

    pub fn param_info(&mut self, pis: &[(u32, i32)]) {
        self.push_struct();
        self.int(pis.len() as i32);
        for &(id, flags) in pis {
            self.id(id);
            self.int(flags);
        }
        self.pop();
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

pub fn frame_pw_msg(obj_id: u32, op: u8, seq: u32, nfds: u32, payload: &[u8]) -> Vec<u8> {
    let pad_len = pad8(payload.len());
    let mut pkt = vec![0u8; 16 + pad_len];
    let w1 = (((op as u32) & 0xff) << 24) | ((payload.len() as u32) & 0x00ff_ffff);
    pkt[0..4].copy_from_slice(&obj_id.to_le_bytes());
    pkt[4..8].copy_from_slice(&w1.to_le_bytes());
    pkt[8..12].copy_from_slice(&seq.to_le_bytes());
    pkt[12..16].copy_from_slice(&nfds.to_le_bytes());
    if !payload.is_empty() {
        pkt[16..16 + payload.len()].copy_from_slice(payload);
    }
    pkt
}

#[inline]
pub fn frac_mhz(f: Frac) -> u64 {
    if f.denom == 0 {
        0
    } else {
        ((f.num as u64) * 1000) / (f.denom as u64)
    }
}

pub fn sanitize_non_zero_fps(f: Frac) -> Frac {
    if f.num == 0 || f.denom == 0 {
        return Frac { num: 60, denom: 1 };
    }
    let mhz = frac_mhz(f);
    if mhz == 0 {
        return Frac { num: 1, denom: 1 };
    }
    if mhz > 1_000_000 {
        return Frac {
            num: 1000,
            denom: 1,
        };
    }
    if (f.num as u64) * 1000 > u32::MAX as u64 {
        return Frac {
            num: mhz as u32,
            denom: 1000,
        };
    }
    f
}

struct FpsRange {
    min_mhz: u64,
    max_mhz: u64,
    min_frac: Frac,
    max_frac: Frac,
    flex: bool,
    pref_frac: Frac,
}

fn extract_vfmt_fps_caps(v: &VfmtCaps) -> FpsRange {
    let mut min_mhz = 1000u64;
    let mut max_mhz = 1_000_000u64;
    let mut min_f = Frac { num: 1, denom: 1 };
    let mut max_f = Frac {
        num: 1000,
        denom: 1,
    };
    let mut pref_f = Frac { num: 0, denom: 1 };
    let mut flex = true;

    if v.has_fps {
        let def_m = frac_mhz(v.fps_def);
        let fmin_m = frac_mhz(v.fps_min);
        let fmax_m = frac_mhz(v.fps_max);
        if v.fps_def.num == 0 && fmax_m < 1000 {
            flex = true;
        } else if fmin_m < fmax_m {
            flex = true;
            if fmin_m >= 1000 {
                min_mhz = fmin_m;
                min_f = v.fps_min;
            }
            if fmax_m >= 1000 && fmax_m < max_mhz {
                max_mhz = fmax_m;
                max_f = v.fps_max;
            }
            if def_m >= 1000 {
                pref_f = v.fps_def;
            }
        } else if def_m >= 1000 {
            flex = false;
            min_mhz = def_m;
            max_mhz = def_m;
            min_f = v.fps_def;
            max_f = v.fps_def;
            pref_f = v.fps_def;
        }
    }

    if v.has_max_fps {
        let mdef_m = frac_mhz(v.max_fps_def);
        let mmin_m = frac_mhz(v.max_fps_min);
        let mmax_m = frac_mhz(v.max_fps_max);
        if mmax_m >= 1000 && mmax_m < max_mhz {
            max_mhz = mmax_m;
            max_f = v.max_fps_max;
        }
        if mmin_m >= 1000 && mmin_m > min_mhz && (!v.has_fps || v.fps_def.num == 0) {
            min_mhz = mmin_m;
            min_f = v.max_fps_min;
        }
        if mmin_m < mmax_m {
            flex = true;
        } else if !v.has_fps && mdef_m >= 1000 {
            flex = false;
            min_mhz = mdef_m;
            max_mhz = mdef_m;
            min_f = v.max_fps_def;
            max_f = v.max_fps_def;
        }
        if pref_f.num == 0 && mdef_m >= 1000 {
            pref_f = v.max_fps_def;
        }
    }

    if min_mhz > max_mhz {
        min_mhz = max_mhz;
        min_f = max_f;
    }

    FpsRange {
        min_mhz,
        max_mhz,
        min_frac: min_f,
        max_frac: max_f,
        flex,
        pref_frac: pref_f,
    }
}

pub struct IntersectedVfmt {
    pub fmt: u32,
    pub w: u32,
    pub h: u32,
    pub fps: Frac,
    pub max_fps: Frac,
    pub mods: Vec<u64>,
}

pub fn intersect_vfmt(
    a: &VfmtCaps,
    b: &VfmtCaps,
    want_mod: bool,
    lim_w: u32,
    lim_h: u32,
    lim_fps: u32,
) -> Option<IntersectedVfmt> {
    if !a.valid || !b.valid {
        return None;
    }
    if want_mod && (!a.has_modifier || !b.has_modifier) {
        return None;
    }
    if !want_mod && (a.has_modifier && b.has_modifier) {
        return None;
    }
    let na = (a.n_formats as usize).min(32);
    let nb = (b.n_formats as usize).min(32);
    let mut chosen_fmt = 0u32;
    'outer: for &fa in &a.formats[..na] {
        for &fb in &b.formats[..nb] {
            if fa == fb {
                chosen_fmt = fa;
                break 'outer;
            }
        }
    }
    if chosen_fmt == 0 {
        return None;
    }

    let minw = a.w_min.max(b.w_min);
    let maxw = a.w_max.min(b.w_max).min(7680);
    let minh = a.h_min.max(b.h_min);
    let maxh = a.h_max.min(b.h_max).min(4320);
    if minw > maxw || minh > maxh {
        return None;
    }

    let mut w = a.w_def.clamp(minw, maxw);
    let mut h = a.h_def.clamp(minh, maxh);
    if lim_w > 0 && lim_h > 0 && (w > lim_w || h > lim_h) && (minw < maxw || minh < maxh) {
        let mut nw = w;
        let mut nh = h;
        if nw > lim_w && nw > 0 {
            nh = (((nh as u64) * (lim_w as u64)) / (nw as u64)) as u32;
            nw = lim_w;
        }
        if nh > lim_h && nh > 0 {
            nw = (((nw as u64) * (lim_h as u64)) / (nh as u64)) as u32;
            nh = lim_h;
        }
        if nw >= minw && nw <= maxw && nh >= minh && nh <= maxh && nw > 0 && nh > 0 {
            w = nw;
            h = nh;
        }
    }

    let mut mods = Vec::new();
    if want_mod {
        let ma = (a.n_modifiers as usize).min(PWB_MAX_MODIFIERS);
        let mb = (b.n_modifiers as usize).min(PWB_MAX_MODIFIERS);
        for tier in 0..3 {
            for &m in &a.modifiers[..ma] {
                if mods.len() >= PWB_MAX_MODIFIERS {
                    break;
                }
                if tier == 0 && (m == 0 || m == DRM_MOD_INVALID) {
                    continue;
                }
                if tier == 1 && m != 0 {
                    continue;
                }
                if tier == 2 && m != DRM_MOD_INVALID {
                    continue;
                }
                if b.modifiers[..mb].contains(&m) && !mods.contains(&m) {
                    mods.push(m);
                }
            }
        }
        if mods.is_empty() {
            return None;
        }
    }

    let ar = extract_vfmt_fps_caps(a);
    let br = extract_vfmt_fps_caps(b);
    let mut lo_mhz = ar.min_mhz.max(br.min_mhz);
    let mut hi_mhz = ar.max_mhz.min(br.max_mhz);
    let mut lo_f = if ar.min_mhz >= br.min_mhz {
        ar.min_frac
    } else {
        br.min_frac
    };
    let mut hi_f = if ar.max_mhz <= br.max_mhz {
        ar.max_frac
    } else {
        br.max_frac
    };
    if lo_mhz > hi_mhz {
        lo_mhz = ar.min_mhz;
        lo_f = ar.min_frac;
        hi_mhz = ar.max_mhz;
        hi_f = ar.max_frac;
    }

    let hi_cap_mhz = hi_mhz.min(240_000);
    let hi_cap_f = if hi_mhz > 240_000 {
        Frac {
            num: 240,
            denom: 1,
        }
    } else {
        hi_f
    };

    let mut chosen_fps = Frac { num: 60, denom: 1 };
    if !ar.flex && frac_mhz(ar.pref_frac) >= 1000 {
        chosen_fps = ar.pref_frac;
    } else if !br.flex && frac_mhz(br.pref_frac) >= 1000 {
        chosen_fps = br.pref_frac;
    } else if lim_fps == 0 && ar.flex && br.flex && hi_cap_mhz >= 1000 {
        chosen_fps = hi_cap_f;
    } else if frac_mhz(ar.pref_frac) >= 1000 {
        chosen_fps = ar.pref_frac;
    }

    let ch_mhz = frac_mhz(chosen_fps);
    if ch_mhz < lo_mhz {
        chosen_fps = lo_f;
    } else if ch_mhz > hi_mhz {
        chosen_fps = hi_f;
    }

    if ar.flex && lim_fps > 0 {
        let lim_mhz = (lim_fps as u64) * 1000;
        if lim_mhz >= lo_mhz && lim_mhz <= hi_cap_mhz {
            chosen_fps = Frac {
                num: lim_fps,
                denom: 1,
            };
        } else if lim_mhz > hi_cap_mhz && hi_cap_mhz >= 1000 {
            chosen_fps = hi_cap_f;
        }
    }

    let fps_out = sanitize_non_zero_fps(chosen_fps);
    let max_fps_out = if lim_fps == 0 && (ar.flex || br.flex) && hi_cap_mhz > frac_mhz(fps_out) {
        sanitize_non_zero_fps(hi_cap_f)
    } else {
        fps_out
    };

    Some(IntersectedVfmt {
        fmt: chosen_fmt,
        w,
        h,
        fps: fps_out,
        max_fps: max_fps_out,
        mods,
    })
}

pub fn build_negotiated_format_pod_iv(
    iv: &IntersectedVfmt,
    has_mod: bool,
    dont_fixate: bool,
) -> Vec<u8> {
    let f_out = sanitize_non_zero_fps(iv.fps);
    let maxf = sanitize_non_zero_fps(iv.max_fps);
    let mut b = PodBuilder::new(512);
    b.push_object(OBJ_FORMAT, PARAM_FORMAT);
    b.prop(1, 0);
    b.id(MEDIA_VIDEO);
    b.prop(2, 0);
    b.id(SUB_RAW);
    b.prop(0x20001, 0);
    b.id(iv.fmt);
    if has_mod && !iv.mods.is_empty() {
        b.prop(0x20002, if dont_fixate { POD_PROP_FLAG_DONT_FIXATE } else { 0 });
        if dont_fixate {
            b.push_choice(CHOICE_ENUM, 0, 8, SPA_LONG);
            b.write_u64_raw(iv.mods[0]);
            for &m in &iv.mods {
                b.write_u64_raw(m);
            }
            b.pop();
        } else {
            b.push_choice(CHOICE_NONE, 0, 8, SPA_LONG);
            b.write_u64_raw(iv.mods[0]);
            b.pop();
        }
    }
    b.prop(0x20003, 0);
    b.rect(iv.w, iv.h);
    b.prop(0x20004, 0);
    b.frac(f_out.num, f_out.denom);
    b.prop(0x20005, 0);
    b.frac(maxf.num, maxf.denom);
    b.pop();
    b.into_vec()
}

pub fn build_negotiated_format_pod(
    fmt: u32,
    w: u32,
    h: u32,
    fps: Frac,
    has_mod: bool,
    dont_fixate: bool,
    mods: &[u64],
) -> Vec<u8> {
    let iv = IntersectedVfmt {
        fmt,
        w,
        h,
        fps,
        max_fps: fps,
        mods: mods.to_vec(),
    };
    build_negotiated_format_pod_iv(&iv, has_mod, dont_fixate)
}

#[inline]
fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

#[allow(clippy::too_many_arguments)]
pub fn convert_preview_row(
    fmt: u32,
    src_base: &[u8],
    sy: u32,
    sh: u32,
    sw: u32,
    sstride: usize,
    dst_row: &mut [u8],
    dw: u32,
) {
    if sw == 0 || dw == 0 || dst_row.len() < (dw as usize) * 4 {
        return;
    }
    let row_off = (sy as usize).saturating_mul(sstride);
    let Some(srow) = src_base.get(row_off..) else {
        return;
    };
    let x_step = ((sw as u64) << 16) / (dw as u64);

    if fmt == VF_BGRX || fmt == VF_BGRA {
        if dw == sw && srow.len() >= (dw as usize) * 4 {
            let n = (dw as usize) * 4;
            dst_row[..n].copy_from_slice(&srow[..n]);
            return;
        }
        let mut xp = 0u64;
        for x in 0..(dw as usize) {
            let sx = ((xp >> 16) as usize).min((sw - 1) as usize);
            xp += x_step;
            let off = sx * 4;
            if let Some(px) = srow.get(off..off + 4) {
                dst_row[x * 4] = px[0];
                dst_row[x * 4 + 1] = px[1];
                dst_row[x * 4 + 2] = px[2];
                dst_row[x * 4 + 3] = 255;
            }
        }
    } else if fmt == VF_RGBX || fmt == VF_RGBA {
        let mut xp = 0u64;
        for x in 0..(dw as usize) {
            let sx = ((xp >> 16) as usize).min((sw - 1) as usize);
            xp += x_step;
            let off = sx * 4;
            if let Some(px) = srow.get(off..off + 4) {
                dst_row[x * 4] = px[2];
                dst_row[x * 4 + 1] = px[1];
                dst_row[x * 4 + 2] = px[0];
                dst_row[x * 4 + 3] = 255;
            }
        }
    } else if fmt == VF_YUY2 {
        let mut xp = 0u64;
        for x in 0..(dw as usize) {
            let sx = ((xp >> 16) as usize).min((sw - 1) as usize);
            xp += x_step;
            let pair = (sx & !1) * 2;
            if srow.len() >= pair + 4 && srow.len() > sx * 2 {
                let yv = srow[sx * 2] as i32;
                let u = srow[pair + 1] as i32 - 128;
                let v = srow[pair + 3] as i32 - 128;
                dst_row[x * 4 + 2] = clamp_u8(yv + ((359 * v) >> 8));
                dst_row[x * 4 + 1] = clamp_u8(yv - ((88 * u + 183 * v) >> 8));
                dst_row[x * 4] = clamp_u8(yv + ((454 * u) >> 8));
                dst_row[x * 4 + 3] = 255;
            }
        }
    } else if fmt == VF_NV12 {
        let uv_off = (sh as usize)
            .saturating_mul(sstride)
            .saturating_add(((sy >> 1) as usize).saturating_mul(sstride));
        let Some(uvrow) = src_base.get(uv_off..) else {
            return;
        };
        let mut xp = 0u64;
        for x in 0..(dw as usize) {
            let sx = ((xp >> 16) as usize).min((sw - 1) as usize);
            xp += x_step;
            let p_uv = sx & !1;
            if sx < srow.len() && uvrow.len() >= p_uv + 2 {
                let yv = srow[sx] as i32;
                let u = uvrow[p_uv] as i32 - 128;
                let v = uvrow[p_uv + 1] as i32 - 128;
                dst_row[x * 4 + 2] = clamp_u8(yv + ((359 * v) >> 8));
                dst_row[x * 4 + 1] = clamp_u8(yv - ((88 * u + 183 * v) >> 8));
                dst_row[x * 4] = clamp_u8(yv + ((454 * u) >> 8));
                dst_row[x * 4 + 3] = 255;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_pod_blocks_wrap_and_underflow() {
        let mut pod = [0u8; 16];
        pod[0..4].copy_from_slice(&4u32.to_le_bytes());
        pod[4..8].copy_from_slice(&SPA_OBJECT.to_le_bytes());
        pod[8..12].copy_from_slice(&OBJ_FORMAT.to_le_bytes());
        pod[12..16].copy_from_slice(&PARAM_ENUM_FORMAT.to_le_bytes());
        assert_eq!(parse_param_pod(&pod).kind, 0);

        pod[0..4].copy_from_slice(&0xFFFF_FFFCu32.to_le_bytes());
        assert_eq!(parse_param_pod(&pod).kind, 0);
    }

    #[test]
    fn fd_quota_enforced() {
        assert_eq!(fd_quota_check(200, 10), (true, 10));
        assert_eq!(fd_quota_check(250, 10), (false, 6));
        assert_eq!(fd_quota_check(256, 1), (false, 0));
    }

    #[test]
    fn pod_builder_and_parse_roundtrip() {
        let pod = build_negotiated_format_pod(
            VF_BGRX,
            1920,
            1080,
            Frac { num: 60, denom: 1 },
            false,
            false,
            &[],
        );
        let parsed = parse_param_pod(&pod);
        assert_eq!(parsed.kind, 1);
        assert!(parsed.vfmt.valid);
        assert_eq!(parsed.vfmt.w_def, 1920);
        assert_eq!(parsed.vfmt.h_def, 1080);
        assert_eq!(parsed.vfmt.formats[0], VF_BGRX);
    }

    #[test]
    fn high_refresh_uncapped_negotiation_uses_output_ceiling() {
        let mut formats = [0u32; 32];
        formats[0] = VF_BGRA;
        let prod = VfmtCaps {
            valid: true,
            n_formats: 1,
            formats,
            w_def: 2560,
            w_min: 640,
            w_max: 3840,
            h_def: 1440,
            h_min: 480,
            h_max: 2160,
            has_fps: true,
            fps_def: Frac { num: 0, denom: 1 },
            has_max_fps: true,
            max_fps_def: Frac { num: 60, denom: 1 },
            max_fps_min: Frac { num: 1, denom: 1 },
            max_fps_max: Frac { num: 240, denom: 1 },
            ..VfmtCaps::default()
        };

        let mut cons_flex = prod.clone();
        cons_flex.fps_def = Frac { num: 60, denom: 1 };
        cons_flex.fps_min = Frac { num: 1, denom: 1 };
        cons_flex.fps_max = Frac { num: 240, denom: 1 };

        let iv_uncapped = intersect_vfmt(&prod, &cons_flex, false, 0, 0, 0).expect("intersect uncapped");
        assert_eq!(iv_uncapped.fps.num, 240);
        assert_eq!(iv_uncapped.max_fps.num, 240);

        let iv_120 = intersect_vfmt(&prod, &cons_flex, false, 2560, 1440, 120).expect("intersect 120");
        assert_eq!(iv_120.fps.num, 120);
        assert_eq!(iv_120.max_fps.num, 120);

        let mut cons_fixed60 = prod.clone();
        cons_fixed60.fps_def = Frac { num: 60, denom: 1 };
        cons_fixed60.fps_min = Frac { num: 60, denom: 1 };
        cons_fixed60.fps_max = Frac { num: 60, denom: 1 };
        let iv_fixed = intersect_vfmt(&prod, &cons_fixed60, false, 0, 0, 0).expect("intersect fixed60");
        assert_eq!(iv_fixed.fps.num, 60);
    }
}
