#![forbid(unsafe_code)]

pub const PA_TAG_STRING: u8 = b't';
pub const PA_TAG_STRING_NULL: u8 = b'N';
pub const PA_TAG_U32: u8 = b'L';
pub const PA_TAG_U8: u8 = b'B';
pub const PA_TAG_SAMPLE_SPEC: u8 = b'a';
pub const PA_TAG_ARBITRARY: u8 = b'x';
pub const PA_TAG_BOOLEAN_TRUE: u8 = b'1';
pub const PA_TAG_BOOLEAN_FALSE: u8 = b'0';
pub const PA_TAG_TIMEVAL: u8 = b'T';
pub const PA_TAG_CHANNEL_MAP: u8 = b'm';
pub const PA_TAG_CVOLUME: u8 = b'v';
pub const PA_TAG_PROPLIST: u8 = b'P';
pub const PA_TAG_FORMAT_INFO: u8 = b'f';

pub const PA_MAX_FRAME_SZ: u32 = 16 * 1024 * 1024;
pub const PA_PROPBAG_CAP: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaFrameHdr {
    pub body_len: u32,
    pub channel: u32,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PaSampleSpec {
    pub format: u8,
    pub channels: u8,
    pub rate: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PaChannelMap {
    pub channels: u8,
    pub map: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PaCVolume {
    pub channels: u8,
    pub values: [u32; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaPropKv {
    pub key: [u8; 64],
    pub val: [u8; 128],
}

impl Default for PaPropKv {
    fn default() -> Self {
        Self {
            key: [0; 64],
            val: [0; 128],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaPropBag {
    pub items: [PaPropKv; PA_PROPBAG_CAP],
    pub count: usize,
}

impl Default for PaPropBag {
    fn default() -> Self {
        Self {
            items: [PaPropKv::default(); PA_PROPBAG_CAP],
            count: 0,
        }
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

impl PaPropBag {
    pub fn set(&mut self, key: &[u8], val: &[u8]) {
        let k_clean = cstr_bytes(key);
        let k_len = k_clean.len().min(63);
        let k_slice = &k_clean[..k_len];
        let cnt = self.count.min(PA_PROPBAG_CAP);
        for item in &mut self.items[..cnt] {
            if cstr_bytes(&item.key) == k_slice {
                copy_cstr(&mut item.val, val);
                return;
            }
        }
        if cnt < PA_PROPBAG_CAP {
            copy_cstr(&mut self.items[cnt].key, k_slice);
            copy_cstr(&mut self.items[cnt].val, val);
            self.count = cnt + 1;
        }
    }
}

pub fn parse_frame_hdr(hdr: &[u8]) -> Option<PaFrameHdr> {
    if hdr.len() < 20 {
        return None;
    }
    let body_len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    if body_len > PA_MAX_FRAME_SZ {
        return None;
    }
    let channel = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    let flags = u32::from_be_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
    Some(PaFrameHdr {
        body_len,
        channel,
        flags,
    })
}

pub struct TagReader<'a> {
    buf: &'a [u8],
    pub pos: usize,
}

impl<'a> TagReader<'a> {
    pub fn new(buf: &'a [u8], pos: usize) -> Option<Self> {
        if pos <= buf.len() {
            Some(Self { buf, pos })
        } else {
            None
        }
    }

    fn rem(&self) -> Option<&'a [u8]> {
        self.buf.get(self.pos..)
    }

    pub fn read_u32(&mut self) -> Option<u32> {
        let r = self.rem()?;
        if r.len() < 5 || r[0] != PA_TAG_U32 {
            return None;
        }
        let val = u32::from_be_bytes([r[1], r[2], r[3], r[4]]);
        self.pos += 5;
        Some(val)
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        let r = self.rem()?;
        if r.len() < 2 || r[0] != PA_TAG_U8 {
            return None;
        }
        let val = r[1];
        self.pos += 2;
        Some(val)
    }

    pub fn read_bool(&mut self) -> Option<bool> {
        let r = self.rem()?;
        let &t = r.first()?;
        match t {
            PA_TAG_BOOLEAN_TRUE => {
                self.pos += 1;
                Some(true)
            }
            PA_TAG_BOOLEAN_FALSE => {
                self.pos += 1;
                Some(false)
            }
            _ => None,
        }
    }

    pub fn read_str_slice(&mut self) -> Option<Option<(usize, usize)>> {
        let r = self.rem()?;
        let &t = r.first()?;
        if t == PA_TAG_STRING_NULL {
            self.pos += 1;
            return Some(None);
        }
        if t != PA_TAG_STRING || r.len() < 2 {
            return None;
        }
        let tail = &r[1..];
        let slen = tail.iter().position(|&b| b == 0)?;
        let off = self.pos + 1;
        self.pos = off + slen + 1;
        Some(Some((off, slen)))
    }

    pub fn read_arbitrary_slice(&mut self) -> Option<(usize, usize)> {
        let r = self.rem()?;
        if r.len() < 5 || r[0] != PA_TAG_ARBITRARY {
            return None;
        }
        let blen = u32::from_be_bytes([r[1], r[2], r[3], r[4]]) as usize;
        let avail = r.len() - 5;
        if blen > avail {
            return None;
        }
        let off = self.pos + 5;
        self.pos = off + blen;
        Some((off, blen))
    }

    pub fn read_ss(&mut self) -> Option<PaSampleSpec> {
        let r = self.rem()?;
        if r.len() < 7 || r[0] != PA_TAG_SAMPLE_SPEC {
            return None;
        }
        let ss = PaSampleSpec {
            format: r[1],
            channels: r[2],
            rate: u32::from_be_bytes([r[3], r[4], r[5], r[6]]),
        };
        self.pos += 7;
        Some(ss)
    }

    pub fn read_map(&mut self) -> Option<PaChannelMap> {
        let r = self.rem()?;
        if r.len() < 2 || r[0] != PA_TAG_CHANNEL_MAP {
            return None;
        }
        let ch = r[1] as usize;
        if ch > r.len() - 2 {
            return None;
        }
        let copy_n = ch.min(32);
        let mut out = PaChannelMap {
            channels: copy_n as u8,
            map: [0; 32],
        };
        out.map[..copy_n].copy_from_slice(&r[2..2 + copy_n]);
        self.pos += 2 + ch;
        Some(out)
    }

    pub fn read_cvol(&mut self) -> Option<PaCVolume> {
        let r = self.rem()?;
        if r.len() < 2 || r[0] != PA_TAG_CVOLUME {
            return None;
        }
        let ch = r[1] as usize;
        let need = ch.checked_mul(4)?;
        if need > r.len() - 2 {
            return None;
        }
        let copy_n = ch.min(32);
        let mut out = PaCVolume {
            channels: copy_n as u8,
            values: [0; 32],
        };
        for (i, dst) in out.values.iter_mut().enumerate().take(copy_n) {
            let base = 2 + i * 4;
            *dst = u32::from_be_bytes([r[base], r[base + 1], r[base + 2], r[base + 3]]);
        }
        self.pos += 2 + need;
        Some(out)
    }

    pub fn read_timeval(&mut self) -> Option<(u32, u32)> {
        let r = self.rem()?;
        if r.len() < 9 || r[0] != PA_TAG_TIMEVAL {
            return None;
        }
        let sec = u32::from_be_bytes([r[1], r[2], r[3], r[4]]);
        let usec = u32::from_be_bytes([r[5], r[6], r[7], r[8]]);
        self.pos += 9;
        Some((sec, usec))
    }

    pub fn read_proplist(&mut self, mut bag: Option<&mut PaPropBag>) -> Option<()> {
        let r = self.rem()?;
        if *r.first()? != PA_TAG_PROPLIST {
            return None;
        }
        self.pos += 1;
        for _ in 0..512 {
            if self.pos >= self.buf.len() {
                return None;
            }
            let Some((k_off, k_len)) = self.read_str_slice()? else {
                return Some(());
            };
            let _vlen = self.read_u32()?;
            let (a_off, a_len) = self.read_arbitrary_slice()?;
            if let Some(pb) = bag.as_deref_mut() {
                if a_len > 0 {
                    let k_raw = &self.buf[k_off..k_off + k_len];
                    let k_trunc = &k_raw[..k_raw.len().min(63)];
                    let a_raw = &self.buf[a_off..a_off + a_len];
                    let a_trunc = &a_raw[..a_raw.len().min(127)];
                    pb.set(k_trunc, a_trunc);
                }
            }
        }
        None
    }

    pub fn skip_format_info(&mut self) -> Option<()> {
        let r = self.rem()?;
        if *r.first()? != PA_TAG_FORMAT_INFO {
            return None;
        }
        self.pos += 1;
        let _enc = self.read_u8()?;
        self.read_proplist(None)
    }
}

pub const PA_TAG_U64: u8 = b'R';
pub const PA_TAG_S64: u8 = b'r';
pub const PA_TAG_USEC: u8 = b'U';
pub const PA_TAG_VOLUME: u8 = b'V';
pub const PA_VOLUME_NORM: u32 = 0x10000;
pub const PA_INVALID_INDEX: u32 = 0xFFFF_FFFF;
pub const PA_CMD_REPLY: u32 = 2;

pub const FMT_U8: u8 = 0;
pub const FMT_ALAW: u8 = 1;
pub const FMT_ULAW: u8 = 2;
pub const FMT_S16LE: u8 = 3;
pub const FMT_S16BE: u8 = 4;
pub const FMT_FLOAT32LE: u8 = 5;
pub const FMT_FLOAT32BE: u8 = 6;
pub const FMT_S32LE: u8 = 7;
pub const FMT_S32BE: u8 = 8;
pub const FMT_S24LE: u8 = 9;
pub const FMT_S24BE: u8 = 10;
pub const FMT_S24_32LE: u8 = 11;
pub const FMT_S24_32BE: u8 = 12;

impl PaPropBag {
    pub fn set_kv(&mut self, key: &str, val: &str) {
        self.set(key.as_bytes(), val.as_bytes());
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        let k = key.as_bytes();
        let cnt = self.count.min(PA_PROPBAG_CAP);
        for item in &self.items[..cnt] {
            if cstr_bytes(&item.key) == k {
                return Some(String::from_utf8_lossy(cstr_bytes(&item.val)).into_owned());
            }
        }
        None
    }
}

pub struct TagBuilder {
    buf: Vec<u8>,
}

impl TagBuilder {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap.max(64)),
        }
    }

    pub fn begin_reply(tag: u32, cap: usize) -> Self {
        let mut tb = Self::new(cap);
        tb.put_u32(PA_TAG_U32, PA_CMD_REPLY);
        tb.put_u32(PA_TAG_U32, tag);
        tb
    }

    pub fn put_u8(&mut self, tag_c: u8, v: u8) {
        self.buf.push(tag_c);
        self.buf.push(v);
    }

    pub fn put_u32(&mut self, tag_c: u8, v: u32) {
        self.buf.push(tag_c);
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_u64_raw(&mut self, tag_c: u8, v: u64) {
        self.buf.push(tag_c);
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_bool(&mut self, v: bool) {
        self.buf
            .push(if v { PA_TAG_BOOLEAN_TRUE } else { PA_TAG_BOOLEAN_FALSE });
    }

    pub fn put_str(&mut self, s: Option<&str>) {
        match s {
            None => self.buf.push(PA_TAG_STRING_NULL),
            Some(txt) => {
                self.buf.push(PA_TAG_STRING);
                let b = txt.as_bytes();
                let clean = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
                self.buf.extend_from_slice(clean);
                self.buf.push(0);
            }
        }
    }

    pub fn put_arbitrary(&mut self, data: &[u8]) {
        self.put_u32(PA_TAG_ARBITRARY, data.len() as u32);
        self.buf.extend_from_slice(data);
    }

    pub fn put_timeval(&mut self, sec: u32, usec: u32) {
        self.buf.push(PA_TAG_TIMEVAL);
        self.buf.extend_from_slice(&sec.to_be_bytes());
        self.buf.extend_from_slice(&usec.to_be_bytes());
    }

    pub fn put_ss(&mut self, ss: &PaSampleSpec) {
        self.buf.push(PA_TAG_SAMPLE_SPEC);
        self.buf.push(ss.format);
        self.buf.push(ss.channels);
        self.buf.extend_from_slice(&ss.rate.to_be_bytes());
    }

    pub fn put_map(&mut self, m: &PaChannelMap) {
        let ch = m.channels.min(32);
        self.buf.push(PA_TAG_CHANNEL_MAP);
        self.buf.push(ch);
        self.buf.extend_from_slice(&m.map[..ch as usize]);
    }

    pub fn put_cvol(&mut self, cv: &PaCVolume) {
        let ch = cv.channels.min(32);
        self.buf.push(PA_TAG_CVOLUME);
        self.buf.push(ch);
        for &v in &cv.values[..ch as usize] {
            self.buf.extend_from_slice(&v.to_be_bytes());
        }
    }

    pub fn put_proplist(&mut self, pb: Option<&PaPropBag>) {
        self.buf.push(PA_TAG_PROPLIST);
        if let Some(bag) = pb {
            let cnt = bag.count.min(PA_PROPBAG_CAP);
            for item in &bag.items[..cnt] {
                let kb = cstr_bytes(&item.key);
                self.buf.push(PA_TAG_STRING);
                self.buf.extend_from_slice(kb);
                self.buf.push(0);
                let vb = cstr_bytes(&item.val);
                let vlen = (vb.len() + 1) as u32;
                self.put_u32(PA_TAG_U32, vlen);
                self.put_u32(PA_TAG_ARBITRARY, vlen);
                self.buf.extend_from_slice(vb);
                self.buf.push(0);
            }
        }
        self.put_str(None);
    }

    pub fn put_pcm_format(&mut self) {
        self.buf.push(PA_TAG_FORMAT_INFO);
        self.put_u8(PA_TAG_U8, 1);
        self.put_proplist(None);
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

pub fn frame_packet(channel: u32, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(20 + payload.len());
    pkt.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    pkt.extend_from_slice(&channel.to_be_bytes());
    pkt.extend_from_slice(&0u32.to_be_bytes());
    pkt.extend_from_slice(&0u32.to_be_bytes());
    pkt.extend_from_slice(&0u32.to_be_bytes());
    pkt.extend_from_slice(payload);
    pkt
}

pub fn format_sample_size(fmt: u8) -> u32 {
    match fmt {
        FMT_U8 | FMT_ALAW | FMT_ULAW => 1,
        FMT_S16LE | FMT_S16BE => 2,
        FMT_S24LE | FMT_S24BE => 3,
        _ => 4,
    }
}

pub fn format_sample_bits(fmt: u8) -> u32 {
    match fmt {
        FMT_U8 | FMT_ALAW | FMT_ULAW | FMT_S16LE | FMT_S16BE => 16,
        FMT_S24LE | FMT_S24BE | FMT_S24_32LE | FMT_S24_32BE => 24,
        FMT_S32LE | FMT_S32BE | FMT_FLOAT32LE | FMT_FLOAT32BE => 32,
        _ => 24,
    }
}

pub fn pct_to_pa_vol(pct: u32) -> u32 {
    let lin = (pct as f64) / 100.0;
    if lin <= 0.0 {
        return 0;
    }
    let sw = (lin.cbrt() * (PA_VOLUME_NORM as f64)).min((PA_VOLUME_NORM as f64) * 1.5);
    (sw + 0.5) as u32
}

pub fn cvol_to_milli(cv: &PaCVolume) -> u32 {
    if cv.channels == 0 {
        return 1000;
    }
    let ch = cv.channels.min(32) as usize;
    let sum: u64 = cv.values[..ch].iter().map(|&v| v as u64).sum();
    let norm = ((sum as f64) / (ch as f64)) / (PA_VOLUME_NORM as f64);
    let lin = norm * norm * norm;
    let m = (lin * 1000.0 + 0.5) as u32;
    m.min(2000)
}

pub fn std_channel_map(ch: u8) -> PaChannelMap {
    let mut m = PaChannelMap {
        channels: if ch == 0 { 2 } else { ch.min(32) },
        map: [0; 32],
    };
    if m.channels == 1 {
        m.map[0] = 0;
    } else {
        m.map[0] = 1;
        m.map[1] = 2;
        for i in 2..(m.channels as usize) {
            m.map[i] = (i as u8) + 1;
        }
    }
    m
}

pub fn decode_sample(p: &[u8], fmt: u8) -> f32 {
    match fmt {
        FMT_FLOAT32LE if p.len() >= 4 => f32::from_le_bytes([p[0], p[1], p[2], p[3]]),
        FMT_FLOAT32BE if p.len() >= 4 => f32::from_be_bytes([p[0], p[1], p[2], p[3]]),
        FMT_S16LE if p.len() >= 2 => (i16::from_le_bytes([p[0], p[1]]) as f32) * (1.0 / 32768.0),
        FMT_S16BE if p.len() >= 2 => (i16::from_be_bytes([p[0], p[1]]) as f32) * (1.0 / 32768.0),
        FMT_S32LE if p.len() >= 4 => {
            (i32::from_le_bytes([p[0], p[1], p[2], p[3]]) as f32) * (1.0 / 2_147_483_648.0)
        }
        FMT_S32BE if p.len() >= 4 => {
            (i32::from_be_bytes([p[0], p[1], p[2], p[3]]) as f32) * (1.0 / 2_147_483_648.0)
        }
        FMT_S24LE if p.len() >= 3 => {
            let mut v = (p[0] as u32) | ((p[1] as u32) << 8) | ((p[2] as u32) << 16);
            if (v & 0x0080_0000) != 0 {
                v |= 0xFF00_0000;
            }
            ((v as i32) as f32) * (1.0 / 8_388_608.0)
        }
        FMT_S24_32LE if p.len() >= 4 => {
            let v = i32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            ((v << 8) >> 8) as f32 * (1.0 / 8_388_608.0)
        }
        FMT_U8 if !p.is_empty() => ((p[0] as f32) - 128.0) * (1.0 / 128.0),
        _ => 0.0,
    }
}

pub fn encode_sample(p: &mut [u8], fmt: u8, mut x: f32) {
    x = x.clamp(-1.0, 1.0);
    match fmt {
        FMT_FLOAT32LE if p.len() >= 4 => p[..4].copy_from_slice(&x.to_le_bytes()),
        FMT_S16LE if p.len() >= 2 => {
            let v = (x * 32767.0) as i16;
            p[..2].copy_from_slice(&v.to_le_bytes());
        }
        FMT_S32LE if p.len() >= 4 => {
            let v = (x * 2_147_483_647.0) as i32;
            p[..4].copy_from_slice(&v.to_le_bytes());
        }
        FMT_S24LE if p.len() >= 3 => {
            let v = (x * 8_388_607.0) as i32;
            p[0] = (v & 0xFF) as u8;
            p[1] = ((v >> 8) & 0xFF) as u8;
            p[2] = ((v >> 16) & 0xFF) as u8;
        }
        FMT_S24_32LE if p.len() >= 4 => {
            let v = (x * 8_388_607.0) as i32;
            p[..4].copy_from_slice(&v.to_le_bytes());
        }
        FMT_U8 if !p.is_empty() => {
            p[0] = ((x * 127.0) + 128.0) as u8;
        }
        _ if p.len() >= 2 => {
            let v = (x * 32767.0) as i16;
            p[..2].copy_from_slice(&v.to_le_bytes());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_oversized_frame() {
        let mut hdr = [0u8; 20];
        hdr[0..4].copy_from_slice(&(PA_MAX_FRAME_SZ + 1).to_be_bytes());
        assert!(parse_frame_hdr(&hdr).is_none());
        hdr[0..4].copy_from_slice(&128u32.to_be_bytes());
        hdr[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            parse_frame_hdr(&hdr),
            Some(PaFrameHdr {
                body_len: 128,
                channel: u32::MAX,
                flags: 0,
            })
        );
    }

    #[test]
    fn reject_truncated_arbitrary() {
        let pkt = [PA_TAG_ARBITRARY, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2];
        let mut tr = TagReader::new(&pkt, 0).unwrap();
        assert!(tr.read_arbitrary_slice().is_none());
    }

    #[test]
    fn tagbuilder_proplist_roundtrip() {
        let mut bag = PaPropBag::default();
        bag.set_kv("application.name", "SalsaRust");
        bag.set_kv("media.role", "music");
        let mut tb = TagBuilder::begin_reply(42, 128);
        tb.put_proplist(Some(&bag));
        let wire = tb.into_vec();
        let mut tr = TagReader::new(&wire, 0).unwrap();
        assert_eq!(tr.read_u32(), Some(PA_CMD_REPLY));
        assert_eq!(tr.read_u32(), Some(42));
        let mut decoded = PaPropBag::default();
        assert!(tr.read_proplist(Some(&mut decoded)).is_some());
        assert_eq!(decoded.get_str("application.name").as_deref(), Some("SalsaRust"));
        assert_eq!(decoded.get_str("media.role").as_deref(), Some("music"));
    }
}
