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
}
