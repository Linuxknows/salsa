// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![forbid(unsafe_code)]

pub const BT_WIRE_SBC: i32 = 0;
pub const BT_WIRE_AAC: i32 = 1;
pub const BT_WIRE_APTX: i32 = 2;
pub const BT_WIRE_APTX_HD: i32 = 3;
pub const BT_WIRE_LDAC: i32 = 4;

pub const BT_CAP_LDAC: u32 = 1 << 0;
pub const BT_CAP_APTX_HD: u32 = 1 << 1;
pub const BT_CAP_APTX: u32 = 1 << 2;
pub const BT_CAP_AAC: u32 = 1 << 3;
pub const BT_CAP_SBC: u32 = 1 << 4;

const SBC_FREQ_16000: u8 = 1 << 3;
const SBC_FREQ_32000: u8 = 1 << 2;
const SBC_FREQ_44100: u8 = 1 << 1;
const SBC_FREQ_48000: u8 = 1 << 0;

const SBC_CH_MONO: u8 = 1 << 3;
const SBC_CH_DUAL: u8 = 1 << 2;
const SBC_CH_STEREO: u8 = 1 << 1;
const SBC_CH_JOINT_STEREO: u8 = 1 << 0;

const SBC_BLK_4: u8 = 1 << 3;
const SBC_BLK_8: u8 = 1 << 2;
const SBC_BLK_12: u8 = 1 << 1;
const SBC_BLK_16: u8 = 1 << 0;

const SBC_SUB_4: u8 = 1 << 1;
const SBC_SUB_8: u8 = 1 << 0;
const SBC_ALLOC_SNR: u8 = 1 << 1;
const SBC_ALLOC_LOUD: u8 = 1 << 0;

const AAC_OBJ_MP2_LC: u8 = 0x80;
const AAC_OBJ_MP4_LC: u8 = 0x40;
const AAC_FREQ_44100: u16 = 0x0010;
const AAC_FREQ_48000: u16 = 0x0008;
const AAC_CH_1: u8 = 0x08;
const AAC_CH_2: u8 = 0x04;

const APTX_VENDOR_ID: u32 = 0x0000_004F;
const APTX_CODEC_ID: u16 = 0x0001;
const APTX_HD_VID: u32 = 0x0000_00D7;
const APTX_HD_CID: u16 = 0x0024;
const APTX_FREQ_16000: u8 = 0x08;
const APTX_FREQ_32000: u8 = 0x04;
const APTX_FREQ_44100: u8 = 0x02;
const APTX_FREQ_48000: u8 = 0x01;
const APTX_CH_MONO: u8 = 0x01;
const APTX_CH_STEREO: u8 = 0x02;

const LDAC_VENDOR_ID: u32 = 0x0000_012D;
const LDAC_CODEC_ID: u16 = 0x00AA;
const LDAC_FREQ_44100: u8 = 0x20;
const LDAC_FREQ_48000: u8 = 0x10;
const LDAC_FREQ_88200: u8 = 0x08;
const LDAC_FREQ_96000: u8 = 0x04;
const LDAC_CH_MONO: u8 = 0x04;
const LDAC_CH_DUAL: u8 = 0x02;
const LDAC_CH_STEREO: u8 = 0x01;

fn a2dp_vid(hdr: &[u8]) -> Option<u32> {
    let s = hdr.get(0..4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn a2dp_cid(hdr: &[u8]) -> Option<u16> {
    let s = hdr.get(4..6)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn write_vendor_hdr(out: &mut [u8], vid: u32, cid: u16) {
    out[0..4].copy_from_slice(&vid.to_le_bytes());
    out[4..6].copy_from_slice(&cid.to_le_bytes());
}

pub fn select_a2dp_config(
    codec_kind: i32,
    rcaps: &[u8],
    out_cfg: &mut [u8],
) -> Option<(usize, u32)> {
    if rcaps.is_empty() {
        return None;
    }
    match codec_kind {
        BT_WIRE_LDAC if rcaps.len() >= 8 && out_cfg.len() >= 8 => {
            if a2dp_vid(rcaps)? == LDAC_VENDOR_ID && a2dp_cid(rcaps)? == LDAC_CODEC_ID {
                let in_f = rcaps[6];
                let in_c = rcaps[7];
                let freq = if (in_f & LDAC_FREQ_96000) != 0 {
                    LDAC_FREQ_96000
                } else if (in_f & LDAC_FREQ_88200) != 0 {
                    LDAC_FREQ_88200
                } else if (in_f & LDAC_FREQ_48000) != 0 {
                    LDAC_FREQ_48000
                } else if (in_f & LDAC_FREQ_44100) != 0 {
                    LDAC_FREQ_44100
                } else {
                    0
                };
                let ch = if (in_c & LDAC_CH_STEREO) != 0 {
                    LDAC_CH_STEREO
                } else if (in_c & LDAC_CH_DUAL) != 0 {
                    LDAC_CH_DUAL
                } else if (in_c & LDAC_CH_MONO) != 0 {
                    LDAC_CH_MONO
                } else {
                    0
                };
                if freq != 0 && ch != 0 {
                    out_cfg[..8].fill(0);
                    write_vendor_hdr(out_cfg, LDAC_VENDOR_ID, LDAC_CODEC_ID);
                    out_cfg[6] = freq;
                    out_cfg[7] = ch;
                    return Some((8, BT_CAP_LDAC));
                }
            }
            None
        }
        BT_WIRE_APTX_HD if rcaps.len() >= 11 && out_cfg.len() >= 11 => {
            if a2dp_vid(rcaps)? == APTX_HD_VID && a2dp_cid(rcaps)? == APTX_HD_CID {
                let in_b = rcaps[6];
                let in_c = in_b & 0x0F;
                let in_f = (in_b >> 4) & 0x0F;
                let freq = pick_aptx_freq(in_f);
                let ch = pick_aptx_ch(in_c);
                if freq != 0 && ch != 0 {
                    out_cfg[..11].fill(0);
                    write_vendor_hdr(out_cfg, APTX_HD_VID, APTX_HD_CID);
                    out_cfg[6] = ((freq & 0x0F) << 4) | (ch & 0x0F);
                    return Some((11, BT_CAP_APTX_HD));
                }
            }
            None
        }
        BT_WIRE_APTX if rcaps.len() >= 7 && out_cfg.len() >= 7 => {
            if a2dp_vid(rcaps)? == APTX_VENDOR_ID && a2dp_cid(rcaps)? == APTX_CODEC_ID {
                let in_b = rcaps[6];
                let in_c = in_b & 0x0F;
                let in_f = (in_b >> 4) & 0x0F;
                let freq = pick_aptx_freq(in_f);
                let ch = pick_aptx_ch(in_c);
                if freq != 0 && ch != 0 {
                    out_cfg[..7].fill(0);
                    write_vendor_hdr(out_cfg, APTX_VENDOR_ID, APTX_CODEC_ID);
                    out_cfg[6] = ((freq & 0x0F) << 4) | (ch & 0x0F);
                    return Some((7, BT_CAP_APTX));
                }
            }
            None
        }
        BT_WIRE_AAC if rcaps.len() >= 6 && out_cfg.len() >= 6 => {
            let obj_in = rcaps[0];
            let obj = if (obj_in & AAC_OBJ_MP2_LC) != 0 {
                AAC_OBJ_MP2_LC
            } else if (obj_in & AAC_OBJ_MP4_LC) != 0 {
                AAC_OBJ_MP4_LC
            } else {
                obj_in
            };
            let infreq = ((rcaps[1] as u16) << 4) | (((rcaps[2] >> 4) & 0x0F) as u16);
            let sfreq = if (infreq & AAC_FREQ_48000) != 0 {
                AAC_FREQ_48000
            } else if (infreq & AAC_FREQ_44100) != 0 {
                AAC_FREQ_44100
            } else {
                AAC_FREQ_48000
            };
            let ch_in = rcaps[2] & 0x0F;
            let ch = if (ch_in & AAC_CH_2) != 0 { AAC_CH_2 } else { AAC_CH_1 };
            let br1_in = (rcaps[3] & 0x7F) as u32;
            let vbr = (rcaps[3] >> 7) & 0x01;
            let mut max_bps = (br1_in << 16) | ((rcaps[4] as u32) << 8) | (rcaps[5] as u32);
            if max_bps == 0 || max_bps > 256_000 {
                max_bps = 256_000;
            }
            out_cfg[0] = obj;
            out_cfg[1] = ((sfreq >> 4) & 0xFF) as u8;
            out_cfg[2] = (((sfreq & 0x0F) as u8) << 4) | (ch & 0x0F);
            out_cfg[3] = ((vbr & 0x01) << 7) | (((max_bps >> 16) & 0x7F) as u8);
            out_cfg[4] = ((max_bps >> 8) & 0xFF) as u8;
            out_cfg[5] = (max_bps & 0xFF) as u8;
            Some((6, BT_CAP_AAC))
        }
        BT_WIRE_SBC if rcaps.len() >= 4 && out_cfg.len() >= 4 => {
            let b0 = rcaps[0];
            let b1 = rcaps[1];
            let in_ch = b0 & 0x0F;
            let in_freq = (b0 >> 4) & 0x0F;
            let in_alloc = b1 & 0x03;
            let in_sub = (b1 >> 2) & 0x03;
            let in_blk = (b1 >> 4) & 0x0F;

            let freq = if (in_freq & SBC_FREQ_48000) != 0 {
                SBC_FREQ_48000
            } else if (in_freq & SBC_FREQ_44100) != 0 {
                SBC_FREQ_44100
            } else if (in_freq & SBC_FREQ_32000) != 0 {
                SBC_FREQ_32000
            } else {
                SBC_FREQ_16000
            };
            let ch = if (in_ch & SBC_CH_JOINT_STEREO) != 0 {
                SBC_CH_JOINT_STEREO
            } else if (in_ch & SBC_CH_STEREO) != 0 {
                SBC_CH_STEREO
            } else if (in_ch & SBC_CH_DUAL) != 0 {
                SBC_CH_DUAL
            } else {
                SBC_CH_MONO
            };
            let blk = if (in_blk & SBC_BLK_16) != 0 {
                SBC_BLK_16
            } else if (in_blk & SBC_BLK_12) != 0 {
                SBC_BLK_12
            } else if (in_blk & SBC_BLK_8) != 0 {
                SBC_BLK_8
            } else {
                SBC_BLK_4
            };
            let sub = if (in_sub & SBC_SUB_8) != 0 { SBC_SUB_8 } else { SBC_SUB_4 };
            let alloc = if (in_alloc & SBC_ALLOC_LOUD) != 0 {
                SBC_ALLOC_LOUD
            } else {
                SBC_ALLOC_SNR
            };
            let min_bp = rcaps[2].clamp(2, 53);
            let max_bp = rcaps[3].clamp(2, 53).max(min_bp);
            out_cfg[0] = ((freq & 0x0F) << 4) | (ch & 0x0F);
            out_cfg[1] = ((blk & 0x0F) << 4) | ((sub & 0x03) << 2) | (alloc & 0x03);
            out_cfg[2] = min_bp;
            out_cfg[3] = max_bp;
            Some((4, BT_CAP_SBC))
        }
        _ => None,
    }
}

fn pick_aptx_freq(in_f: u8) -> u8 {
    if (in_f & APTX_FREQ_48000) != 0 {
        APTX_FREQ_48000
    } else if (in_f & APTX_FREQ_44100) != 0 {
        APTX_FREQ_44100
    } else if (in_f & APTX_FREQ_32000) != 0 {
        APTX_FREQ_32000
    } else if (in_f & APTX_FREQ_16000) != 0 {
        APTX_FREQ_16000
    } else {
        0
    }
}

fn pick_aptx_ch(in_c: u8) -> u8 {
    if (in_c & APTX_CH_STEREO) != 0 {
        APTX_CH_STEREO
    } else if (in_c & APTX_CH_MONO) != 0 {
        APTX_CH_MONO
    } else {
        0
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn parse_bdaddr(bytes: &[u8]) -> Option<[u8; 6]> {
    if bytes.len() != 17 {
        return None;
    }
    let mut out = [0u8; 6];
    for idx in 0..6 {
        let pos = idx * 3;
        let hi = hex_val(bytes[pos])?;
        let lo = hex_val(bytes[pos + 1])?;
        if idx < 5 && bytes[pos + 2] != b':' {
            return None;
        }
        out[5 - idx] = (hi << 4) | lo;
    }
    Some(out)
}

pub fn extract_mac_from_id(id: &[u8]) -> Option<[u8; 17]> {
    let clean_end = id.iter().position(|&b| b == 0).unwrap_or(id.len());
    let mut s = &id[..clean_end];
    if s.len() >= 6 && s[..6].eq_ignore_ascii_case(b"bluez:") {
        s = &s[6..];
    }
    if s.len() < 17 {
        return None;
    }
    let mut out = [0u8; 17];
    let mut colons = 0usize;
    for (i, &ch) in s[..17].iter().enumerate() {
        if (i % 3) == 2 {
            if ch != b':' && ch != b'_' && ch != b'-' {
                return None;
            }
            out[i] = b':';
            colons += 1;
        } else {
            if !ch.is_ascii_hexdigit() {
                return None;
            }
            out[i] = ch.to_ascii_uppercase();
        }
    }
    if colons != 5 {
        return None;
    }
    Some(out)
}

pub fn hfp_make_batt_path(dev_path: &[u8], out: &mut [u8]) -> Option<usize> {
    const ROOT: &[u8] = b"/org/lasapi/Battery/";
    if out.len() <= ROOT.len() {
        return None;
    }
    let clean_end = dev_path.iter().position(|&b| b == 0).unwrap_or(dev_path.len());
    let clean = &dev_path[..clean_end];
    let tail = match clean.iter().rposition(|&b| b == b'/') {
        Some(pos) if pos + 1 < clean.len() => &clean[pos + 1..],
        _ => b"dev",
    };

    out.fill(0);
    out[..ROOT.len()].copy_from_slice(ROOT);
    let mut pos = ROOT.len();
    let max_pos = out.len() - 1;
    for &b in tail {
        if pos >= max_pos {
            break;
        }
        out[pos] = if b.is_ascii_alphanumeric() || b == b'_' {
            b
        } else {
            b'_'
        };
        pos += 1;
    }
    out[pos] = 0;
    Some(pos)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HfpAtAction {
    pub handled: bool,
    pub reply: [u8; 192],
    pub reply_len: usize,
    pub has_msbc_update: bool,
    pub supports_msbc: bool,
    pub sco_codec: u8,     // 0=unchanged, 1=CVSD, 2=mSBC
    pub battery_pct: i32,  // -1=unchanged, 0..=100=new pct
}

impl Default for HfpAtAction {
    fn default() -> Self {
        Self {
            handled: false,
            reply: [0u8; 192],
            reply_len: 0,
            has_msbc_update: false,
            supports_msbc: false,
            sco_codec: 0,
            battery_pct: -1,
        }
    }
}

impl HfpAtAction {
    fn set_reply(&mut self, msg: &[u8]) {
        let n = msg.len().min(self.reply.len() - 1);
        self.reply[..n].copy_from_slice(&msg[..n]);
        self.reply[n] = 0;
        self.reply_len = n;
    }
}

fn parse_dec_prefix_i64(s: &[u8]) -> Option<(i64, usize)> {
    let mut i = 0usize;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    if i >= s.len() {
        return None;
    }
    let mut neg = false;
    if s[i] == b'-' {
        neg = true;
        i += 1;
    } else if s[i] == b'+' {
        i += 1;
    }
    if i >= s.len() || !s[i].is_ascii_digit() {
        return None;
    }
    let mut acc: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = (s[i] - b'0') as i64;
        acc = acc.saturating_mul(10).saturating_add(d);
        i += 1;
    }
    Some((if neg { -acc } else { acc }, i))
}

fn parse_bac_supports_msbc(list: &[u8]) -> bool {
    for (i, &b) in list.iter().enumerate() {
        if b == b'2' {
            let left_ok = i == 0 || list[i - 1] == b',' || list[i - 1] == b' ';
            let right_ok = i + 1 == list.len() || list[i + 1] == b',' || list[i + 1] == b' ';
            if left_ok && right_ok {
                return true;
            }
        }
    }
    false
}

pub fn parse_hfp_at_line(raw: &[u8]) -> HfpAtAction {
    let mut act = HfpAtAction::default();
    let nul_end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let mut s = &raw[..nul_end];
    while !s.is_empty() && (s[0] == b' ' || s[0] == b'\t') {
        s = &s[1..];
    }
    if s.is_empty() {
        return act;
    }

    if s.starts_with(b"AT+BRSF=") {
        act.handled = true;
        act.set_reply(b"\r\n+BRSF: 1536\r\n\r\nOK\r\n");
    } else if let Some(rest) = s.strip_prefix(b"AT+BAC=") {
        act.handled = true;
        act.has_msbc_update = true;
        act.supports_msbc = parse_bac_supports_msbc(rest);
        act.set_reply(b"\r\nOK\r\n");
    } else if let Some(rest) = s.strip_prefix(b"AT+BCS=") {
        act.handled = true;
        let c = parse_dec_prefix_i64(rest).map(|(v, _)| v).unwrap_or(0);
        act.sco_codec = if c == 2 { 2 } else { 1 };
        act.set_reply(b"\r\nOK\r\n");
    } else if s == b"AT+CIND=?" {
        act.handled = true;
        act.set_reply(
            b"\r\n+CIND: (\"service\",(0,1)),(\"call\",(0,1)),(\"callsetup\",(0,3)),(\"callheld\",(0,2)),(\"signal\",(0,5)),(\"roam\",(0,1)),(\"battchg\",(0,5))\r\n\r\nOK\r\n",
        );
    } else if s == b"AT+CIND?" {
        act.handled = true;
        act.set_reply(b"\r\n+CIND: 1,0,0,0,5,0,5\r\n\r\nOK\r\n");
    } else if s.starts_with(b"AT+CMER=") {
        act.handled = true;
        act.set_reply(b"\r\nOK\r\n");
    } else if s == b"AT+CHLD=?" {
        act.handled = true;
        act.set_reply(b"\r\n+CHLD: (0,1,2,3)\r\n\r\nOK\r\n");
    } else if s == b"AT+BIND=?" {
        act.handled = true;
        act.set_reply(b"\r\n+BIND: (1,2)\r\n\r\nOK\r\n");
    } else if s == b"AT+BIND?" {
        act.handled = true;
        act.set_reply(b"\r\n+BIND: 2,1\r\n\r\nOK\r\n");
    } else if s.starts_with(b"AT+BIND=") {
        act.handled = true;
        act.set_reply(b"\r\nOK\r\n");
    } else if let Some(rest) = s.strip_prefix(b"AT+BIEV=") {
        act.handled = true;
        if let Some((ind, used1)) = parse_dec_prefix_i64(rest) {
            let mut rem = &rest[used1..];
            while !rem.is_empty() && matches!(rem[0], b' ' | b'\t') {
                rem = &rem[1..];
            }
            if let Some(after_comma) = rem.strip_prefix(b",") {
                if let Some((val, _)) = parse_dec_prefix_i64(after_comma) {
                    if ind == 2 && (0..=100).contains(&val) {
                        act.battery_pct = val as i32;
                    }
                }
            }
        }
        act.set_reply(b"\r\nOK\r\n");
    } else if s.starts_with(b"AT+XAPL=") {
        act.handled = true;
        act.set_reply(b"\r\n+XAPL=iPhone,7\r\n\r\nOK\r\n");
    } else if let Some(rest) = s.strip_prefix(b"AT+IPHONEACCEV=") {
        act.handled = true;
        if let Some((count, used0)) = parse_dec_prefix_i64(rest) {
            if (1..=16).contains(&count) {
                let mut pos = used0;
                for _ in 0..count {
                    while pos < rest.len() && (rest[pos] == b',' || rest[pos] == b' ') {
                        pos += 1;
                    }
                    let Some((key, uk)) = parse_dec_prefix_i64(&rest[pos..]) else {
                        break;
                    };
                    pos += uk;
                    while pos < rest.len() && (rest[pos] == b',' || rest[pos] == b' ') {
                        pos += 1;
                    }
                    let Some((val, uv)) = parse_dec_prefix_i64(&rest[pos..]) else {
                        break;
                    };
                    pos += uv;
                    if key == 1 && (0..=9).contains(&val) {
                        act.battery_pct = ((val + 1) * 10) as i32;
                    }
                }
            }
        }
        act.set_reply(b"\r\nOK\r\n");
    } else if s.starts_with(b"AT") {
        act.handled = true;
        act.set_reply(b"\r\nOK\r\n");
    }
    act
}

pub fn write_rtp_header(
    out: &mut [u8],
    payload_type: u8,
    marker: bool,
    seq: u16,
    timestamp: u32,
    ssrc: u32,
) -> Option<usize> {
    if out.len() < 12 {
        return None;
    }
    out[0] = 0x80; // v=2, p=0, x=0, cc=0
    out[1] = (if marker { 0x80 } else { 0 }) | (payload_type & 0x7F);
    out[2..4].copy_from_slice(&seq.to_be_bytes());
    out[4..8].copy_from_slice(&timestamp.to_be_bytes());
    out[8..12].copy_from_slice(&ssrc.to_be_bytes());
    Some(12)
}

pub fn write_rtp_sbc_payload_hdr(out: &mut [u8], frame_count: u8) -> Option<usize> {
    if out.is_empty() {
        return None;
    }
    out[0] = frame_count & 0x0F;
    Some(1)
}

pub fn pack_s16_to_s24le(pcm_s16: &[i16], out_s24: &mut [u8]) -> Option<usize> {
    let need = pcm_s16.len().checked_mul(3)?;
    if out_s24.len() < need {
        return None;
    }
    for (i, &sample) in pcm_s16.iter().enumerate() {
        let v = (sample as i32) << 8;
        let off = i * 3;
        out_s24[off] = (v & 0xFF) as u8;
        out_s24[off + 1] = ((v >> 8) & 0xFF) as u8;
        out_s24[off + 2] = ((v >> 16) & 0xFF) as u8;
    }
    Some(need)
}

#[inline]
pub fn ldac_eqmid_to_kbps(sf: u32, eqmid: i32) -> u32 {
    let is_44k = sf == 44100 || sf == 88200;
    match eqmid {
        0 => if is_44k { 909 } else { 990 }, // LDACBT_EQMID_HQ
        1 => if is_44k { 606 } else { 660 }, // LDACBT_EQMID_SQ
        _ => if is_44k { 303 } else { 330 }, // LDACBT_EQMID_MQ
    }
}

#[inline]
pub fn msbc_h2_byte(seq: u32) -> u8 {
    const H2: [u8; 4] = [0x08, 0x38, 0xc8, 0xf8];
    H2[(seq & 3) as usize]
}

#[inline]
pub fn msbc_rx_strip_offset(rx: &[u8]) -> usize {
    if rx.len() >= 2 && rx[0] == 0x01 && (rx[1] & 0x0F) == 0x08 {
        2
    } else {
        0
    }
}

#[inline]
pub fn abr_step_ldac(cur_eqmid: i32, congest_streak: u32, clean_streak: u32) -> (i32, bool, bool) {
    if congest_streak >= 4 && cur_eqmid < 2 {
        (cur_eqmid + 1, true, false)
    } else if clean_streak >= 250 && cur_eqmid > 0 {
        (cur_eqmid - 1, false, true)
    } else {
        (cur_eqmid, false, false)
    }
}

#[inline]
pub fn abr_step_aac(
    cur_bps: u32,
    max_bps: u32,
    congest_streak: u32,
    clean_streak: u32,
) -> (u32, bool, bool) {
    let cur = if cur_bps == 0 { 192_000 } else { cur_bps };
    let cap = if max_bps == 0 { 256_000 } else { max_bps };
    if congest_streak >= 4 && cur > 128_000 {
        let next = if cur > 192_000 { 192_000 } else { 128_000 };
        (next, true, false)
    } else if clean_streak >= 300 && cur < cap {
        let next = cur.saturating_add(32_000).min(cap);
        (next, false, true)
    } else {
        (cur, false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sbc_clamps_malicious_bitpools() {
        let rcaps = [0x11u8, 0x15u8, 250u8, 10u8];
        let mut out = [0u8; 16];
        let (len, bit) = select_a2dp_config(BT_WIRE_SBC, &rcaps, &mut out).unwrap();
        assert_eq!(len, 4);
        assert_eq!(bit, BT_CAP_SBC);
        assert_eq!(out[2], 53);
        assert_eq!(out[3], 53);
    }

    #[test]
    fn bdaddr_rejects_trailing_garbage() {
        assert!(parse_bdaddr(b"00:11:22:33:44:55").is_some());
        assert!(parse_bdaddr(b"00:11:22:33:44:55 ").is_none());
        assert!(parse_bdaddr(b"00-11-22-33-44-55").is_none());
    }

    #[test]
    fn hfp_at_commands_and_battery_parsing() {
        let brsf = parse_hfp_at_line(b"  \tAT+BRSF=4031");
        assert!(brsf.handled);
        assert_eq!(&brsf.reply[..brsf.reply_len], b"\r\n+BRSF: 1536\r\n\r\nOK\r\n");

        let bac_yes = parse_hfp_at_line(b"AT+BAC=1,2");
        assert!(bac_yes.has_msbc_update && bac_yes.supports_msbc);
        let bac_no = parse_hfp_at_line(b"AT+BAC=1,21");
        assert!(bac_no.has_msbc_update && !bac_no.supports_msbc);

        let bcs = parse_hfp_at_line(b"AT+BCS=2");
        assert_eq!(bcs.sco_codec, 2);

        let biev = parse_hfp_at_line(b"AT+BIEV=2,75");
        assert_eq!(biev.battery_pct, 75);
        let biev_bad = parse_hfp_at_line(b"AT+BIEV=2,150");
        assert_eq!(biev_bad.battery_pct, -1);

        let apple = parse_hfp_at_line(b"AT+IPHONEACCEV=2,2,0,1,4");
        assert_eq!(apple.battery_pct, 50);

        let clcc = parse_hfp_at_line(b"AT+CLCC");
        assert!(clcc.handled);
        assert_eq!(&clcc.reply[..clcc.reply_len], b"\r\nOK\r\n");
    }

    #[test]
    fn rtp_pack_and_mac_path_sanitization() {
        let mut pkt = [0u8; 16];
        assert_eq!(write_rtp_header(&mut pkt, 96, true, 0x1234, 0xabcd_ef01, 1), Some(12));
        assert_eq!(
            &pkt[..12],
            &[0x80, 0xE0, 0x12, 0x34, 0xAB, 0xCD, 0xEF, 0x01, 0, 0, 0, 1]
        );
        assert_eq!(write_rtp_sbc_payload_hdr(&mut pkt[12..], 0x27), Some(1));
        assert_eq!(pkt[12], 0x07);

        let mut s24 = [0u8; 6];
        assert_eq!(pack_s16_to_s24le(&[0x1234, -1], &mut s24), Some(6));
        assert_eq!(&s24, &[0x00, 0x34, 0x12, 0x00, 0xFF, 0xFF]);

        let mac = extract_mac_from_id(b"bluez:00_11_22_33_44_55").unwrap();
        assert_eq!(&mac, b"00:11:22:33:44:55");

        let mut bpath = [0u8; 160];
        let blen = hfp_make_batt_path(b"/org/bluez/hci0/dev_AA-BB..evil", &mut bpath).unwrap();
        assert_eq!(
            &bpath[..blen],
            b"/org/lasapi/Battery/dev_AA_BB__evil"
        );
        assert_eq!(msbc_rx_strip_offset(&[0x01, 0x38, 0xAD]), 2);
        assert_eq!(msbc_rx_strip_offset(&[0xAD, 0x00]), 0);
    }
}
