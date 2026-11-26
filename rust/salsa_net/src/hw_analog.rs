// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, ptr_to_opt_str, LasapiHwAnalogCfg, LasapiHwAnalogInfo,
    LASAPI_DBG_ENGINE, LASAPI_DBG_INFO,
};
use crate::unix_sys::{
    close_raw_fd, dbg_emit, ioctl_raw, nb_read_raw, open_raw_flags, O_CLOEXEC, O_NONBLOCK,
    O_RDONLY, O_RDWR,
};

pub const LASAPI_HW_AMP_AUTO: u32 = 0;
pub const LASAPI_HW_AMP_LOW: u32 = 1;
pub const LASAPI_HW_AMP_MED: u32 = 2;
pub const LASAPI_HW_AMP_HIGH: u32 = 3;

pub const LASAPI_HW_ROUTE_AUTO: u32 = 0;
pub const LASAPI_HW_ROUTE_HP: u32 = 1;
pub const LASAPI_HW_ROUTE_SPK: u32 = 2;

pub const LASAPI_HW_CAP_DAC_VOL: u32 = 1 << 0;
pub const LASAPI_HW_CAP_HP_AMP: u32 = 1 << 1;
pub const LASAPI_HW_CAP_OUT_ROUTE: u32 = 1 << 2;
pub const LASAPI_HW_CAP_MIC_BOOST: u32 = 1 << 3;
pub const LASAPI_HW_CAP_LINE_AIR: u32 = 1 << 4;
pub const LASAPI_HW_CAP_HID_BOX: u32 = 1 << 5;
pub const LASAPI_HW_CAP_STRIX_ARCH: u32 = 1 << 6;
pub const LASAPI_HW_CAP_HIGH_RES: u32 = 1 << 7;

const BUS_USB: u32 = 3;
const HID_MAX_DESCRIPTOR_SIZE: usize = 4096;
const HIDIOCGRAWINFO: c_ulong = 0x8008_4803;
const HIDIOCGRDESCSIZE: c_ulong = 0x8004_4801;
const HIDIOCGRDESC: c_ulong = 0x9004_4802;

pub(crate) const SNDRV_CTL_IOCTL_ELEM_LIST: c_ulong = 0xc050_5510;
pub(crate) const SNDRV_CTL_IOCTL_ELEM_INFO: c_ulong = 0xc110_5511;
pub(crate) const SNDRV_CTL_IOCTL_ELEM_READ: c_ulong = 0xc4c8_5512;
const SNDRV_CTL_IOCTL_ELEM_WRITE: c_ulong = 0xc4c8_5513;

const SNDRV_CTL_ELEM_IFACE_MIXER: i32 = 2;
pub(crate) const SNDRV_CTL_ELEM_IFACE_PCM: i32 = 3;
pub(crate) const SNDRV_CTL_ELEM_TYPE_BOOLEAN: u32 = 1;
const SNDRV_CTL_ELEM_TYPE_INTEGER: u32 = 2;
const SNDRV_CTL_ELEM_TYPE_ENUMERATED: u32 = 3;
pub(crate) const SNDRV_CTL_ELEM_TYPE_BYTES: u32 = 4;
pub(crate) const SNDRV_CTL_ELEM_ID_NAME_MAXLEN: usize = 44;

#[repr(C)]
#[derive(Clone, Copy)]
struct HidrawDevinfo {
    bustype: u32,
    vendor: i16,
    product: i16,
}

#[repr(C)]
struct HidrawReportDescriptor {
    size: u32,
    value: [u8; HID_MAX_DESCRIPTOR_SIZE],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndCtlElemId {
    pub(crate) numid: c_uint,
    pub(crate) iface: c_int,
    pub(crate) device: c_uint,
    pub(crate) subdevice: c_uint,
    pub(crate) name: [u8; SNDRV_CTL_ELEM_ID_NAME_MAXLEN],
    pub(crate) index: c_uint,
}

impl Default for SndCtlElemId {
    fn default() -> Self {
        unsafe { mem::zeroed() }
    }
}

#[repr(C)]
pub(crate) struct SndCtlElemList {
    pub(crate) offset: c_uint,
    pub(crate) space: c_uint,
    pub(crate) used: c_uint,
    pub(crate) count: c_uint,
    pub(crate) pids: *mut SndCtlElemId,
    pub(crate) reserved: [u8; 50],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union SndCtlElemInfoValue {
    integer: SndCtlElemInfoInteger,
    enumerated: SndCtlElemInfoEnumerated,
    reserved: [u8; 128],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SndCtlElemInfoInteger {
    min: c_long,
    max: c_long,
    step: c_long,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SndCtlElemInfoEnumerated {
    items: c_uint,
    item: c_uint,
    name: [u8; 64],
    names_ptr: u64,
    names_length: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SndCtlElemInfo {
    pub(crate) id: SndCtlElemId,
    pub(crate) tp: c_int,
    pub(crate) access: c_uint,
    pub(crate) count: c_uint,
    pub(crate) owner: c_int,
    pub(crate) value: SndCtlElemInfoValue,
    pub(crate) dimen: [u16; 4],
    pub(crate) reserved: [u8; 56],
}

#[repr(C, align(8))]
pub(crate) struct SndCtlElemValue {
    pub(crate) id: SndCtlElemId,
    pub(crate) indirect: c_uint,
    pub(crate) _pad: u32,
    pub(crate) data: [u8; 1152],
}

impl SndCtlElemValue {
    pub(crate) fn zeroed_for(id: SndCtlElemId) -> Self {
        let mut s: Self = unsafe { mem::zeroed() };
        s.id = id;
        s
    }

    pub(crate) fn get_bool(&self, idx: usize) -> bool {
        let off = idx * mem::size_of::<c_long>();
        if off + mem::size_of::<c_long>() <= self.data.len() {
            let mut b = [0u8; mem::size_of::<c_long>()];
            b.copy_from_slice(&self.data[off..off + mem::size_of::<c_long>()]);
            c_long::from_ne_bytes(b) != 0
        } else {
            false
        }
    }

    fn set_int(&mut self, idx: usize, val: c_long) {
        let off = idx * mem::size_of::<c_long>();
        if off + mem::size_of::<c_long>() <= self.data.len() {
            let bytes = val.to_ne_bytes();
            self.data[off..off + bytes.len()].copy_from_slice(&bytes);
        }
    }

    fn set_enum(&mut self, idx: usize, val: c_uint) {
        let off = idx * mem::size_of::<c_uint>();
        if off + mem::size_of::<c_uint>() <= self.data.len() {
            let bytes = val.to_ne_bytes();
            self.data[off..off + bytes.len()].copy_from_slice(&bytes);
        }
    }
}

#[derive(Clone, Default)]
struct HwCtlElem {
    valid: bool,
    id: SndCtlElemId,
    tp: u32,
    count: u32,
    imin: c_long,
    imax: c_long,
    items: u32,
    name: String,
}

impl HwCtlElem {
    fn from_info(id: &SndCtlElemId, info: &SndCtlElemInfo, name: &str) -> Self {
        let tp = info.tp as u32;
        let (imin, imax, items) = unsafe {
            if tp == SNDRV_CTL_ELEM_TYPE_INTEGER {
                (info.value.integer.min, info.value.integer.max, 0)
            } else if tp == SNDRV_CTL_ELEM_TYPE_ENUMERATED {
                (0, 0, info.value.enumerated.items)
            } else {
                (0, 0, 0)
            }
        };
        Self {
            valid: true,
            id: *id,
            tp,
            count: info.count,
            imin,
            imax,
            items,
            name: name.to_string(),
        }
    }
}

struct HwAnalogInner {
    pb_card: i32,
    cap_card: i32,
    caps_mask: u32,
    card_arch_name: String,
    diag_note: String,

    pb_dac_vol: HwCtlElem,
    pb_dac_sw: HwCtlElem,
    pb_hp_vol: HwCtlElem,
    pb_hp_sw: HwCtlElem,
    pb_spk_vol: HwCtlElem,
    pb_spk_sw: HwCtlElem,
    pb_amp_ctl: HwCtlElem,
    pb_route_ctl: HwCtlElem,
    cap_boost_ctl: HwCtlElem,
    cap_vol_ctl: HwCtlElem,
    cap_sw_ctl: HwCtlElem,

    hid_fd: i32,
    hid_has_consumer: bool,
    hid_has_report_id: bool,
    hid_consumer_report_id: u8,
    hid_prev_up: bool,
    hid_prev_dn: bool,
    hid_prev_mute: bool,

    sync_cache_valid: bool,
    last_pb_card: i32,
    last_cap_card: i32,
    last_cfg: LasapiHwAnalogCfg,
    last_d_vol: u32,
    last_d_muted: bool,
    last_m_muted: bool,
}

impl HwAnalogInner {
    fn new() -> Self {
        Self {
            pb_card: -1,
            cap_card: -1,
            caps_mask: 0,
            card_arch_name: "Virtual / Standby".to_string(),
            diag_note: "Preserve active hardware defaults".to_string(),
            pb_dac_vol: HwCtlElem::default(),
            pb_dac_sw: HwCtlElem::default(),
            pb_hp_vol: HwCtlElem::default(),
            pb_hp_sw: HwCtlElem::default(),
            pb_spk_vol: HwCtlElem::default(),
            pb_spk_sw: HwCtlElem::default(),
            pb_amp_ctl: HwCtlElem::default(),
            pb_route_ctl: HwCtlElem::default(),
            cap_boost_ctl: HwCtlElem::default(),
            cap_vol_ctl: HwCtlElem::default(),
            cap_sw_ctl: HwCtlElem::default(),
            hid_fd: -1,
            hid_has_consumer: false,
            hid_has_report_id: false,
            hid_consumer_report_id: 0,
            hid_prev_up: false,
            hid_prev_dn: false,
            hid_prev_mute: false,
            sync_cache_valid: false,
            last_pb_card: -1,
            last_cap_card: -1,
            last_cfg: default_cfg_value(),
            last_d_vol: 0,
            last_d_muted: false,
            last_m_muted: false,
        }
    }

    fn close_hid(&mut self) {
        if self.hid_fd >= 0 {
            close_raw_fd(self.hid_fd);
            self.hid_fd = -1;
        }
        self.hid_has_consumer = false;
        self.hid_has_report_id = false;
        self.hid_consumer_report_id = 0;
        self.hid_prev_up = false;
        self.hid_prev_dn = false;
        self.hid_prev_mute = false;
    }
}

impl Drop for HwAnalogInner {
    fn drop(&mut self) {
        self.close_hid();
    }
}

pub struct LasapiHwAnalogCtx {
    inner: Mutex<HwAnalogInner>,
    dac_unity_active: AtomicI32,
    softknee_active: AtomicI32,
    drive_ppm: AtomicU32,
    cap_makeup_ppm: AtomicU32,
}

impl LasapiHwAnalogCtx {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HwAnalogInner::new()),
            dac_unity_active: AtomicI32::new(0),
            softknee_active: AtomicI32::new(0),
            drive_ppm: AtomicU32::new(1_000_000),
            cap_makeup_ppm: AtomicU32::new(1_000_000),
        }
    }
}

#[inline]
pub fn ci_contains(hay: &str, needle: &str) -> bool {
    if hay.is_empty() || needle.is_empty() {
        return false;
    }
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    if nb.len() > hb.len() {
        return false;
    }
    hb.windows(nb.len()).any(|w| w.eq_ignore_ascii_case(nb))
}

#[inline]
pub fn ci_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn read_small_text(path: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let trimmed = text.trim_end_matches(&['\n', '\r', ' '][..]);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn default_cfg_value() -> LasapiHwAnalogCfg {
    LasapiHwAnalogCfg {
        hw_dac_unity_vol: 1,
        hp_amp_tier: LASAPI_HW_AMP_AUTO,
        output_route: LASAPI_HW_ROUTE_AUTO,
        unlock_0db_rails: 0,
        analog_boost_db: 0.0,
        mic_hw_boost_db: 0.0,
    }
}

pub fn clamp_cfg(cfg: &mut LasapiHwAnalogCfg) {
    cfg.hw_dac_unity_vol = u32::from(cfg.hw_dac_unity_vol != 0);
    if cfg.hp_amp_tier > LASAPI_HW_AMP_HIGH {
        cfg.hp_amp_tier = LASAPI_HW_AMP_AUTO;
    }
    if cfg.output_route > LASAPI_HW_ROUTE_SPK {
        cfg.output_route = LASAPI_HW_ROUTE_AUTO;
    }
    cfg.unlock_0db_rails = u32::from(cfg.unlock_0db_rails != 0);
    if !(0.0..=12.0).contains(&cfg.analog_boost_db) {
        cfg.analog_boost_db = if cfg.analog_boost_db > 12.0 { 12.0 } else { 0.0 };
    }
    if !(0.0..=24.0).contains(&cfg.mic_hw_boost_db) {
        cfg.mic_hw_boost_db = if cfg.mic_hw_boost_db > 24.0 { 24.0 } else { 0.0 };
    }
}

fn cfg_eq(a: &LasapiHwAnalogCfg, b: &LasapiHwAnalogCfg) -> bool {
    a.hw_dac_unity_vol == b.hw_dac_unity_vol
        && a.hp_amp_tier == b.hp_amp_tier
        && a.output_route == b.output_route
        && a.unlock_0db_rails == b.unlock_0db_rails
        && (a.analog_boost_db - b.analog_boost_db).abs() < 0.001
        && (a.mic_hw_boost_db - b.mic_hw_boost_db).abs() < 0.001
}

pub fn identify_card_arch_from_meta(
    card: i32,
    cid: &str,
    usbid: &str,
    uevent: &str,
    stream_head: &str,
) -> (String, u32) {
    let mut caps = 0u32;
    let arch = if ci_eq(usbid, "0b05:180c")
        || ci_contains(stream_head, "STRIX RAID DLX")
        || ci_contains(cid, "DLX")
    {
        caps |= LASAPI_HW_CAP_STRIX_ARCH | LASAPI_HW_CAP_HIGH_RES;
        "ASUS STRIX RAID DLX (ES9016 + TPA6120A2)".to_string()
    } else if ci_eq(usbid, "0b05:180d")
        || ci_contains(stream_head, "STRIX")
        || ci_contains(cid, "STRIX")
    {
        caps |= LASAPI_HW_CAP_STRIX_ARCH | LASAPI_HW_CAP_HIGH_RES;
        "ASUS STRIX Sound Card (ES9016 + TPA6120A2)".to_string()
    } else if ci_eq(usbid, "0b05:1808")
        || ci_eq(usbid, "0b05:1809")
        || ci_contains(uevent, "virtuoso")
        || ci_contains(cid, "Xonar")
    {
        caps |= LASAPI_HW_CAP_HIGH_RES;
        "ASUS Xonar Discrete DAC/Amp".to_string()
    } else if usbid.starts_with("1235:")
        || ci_contains(stream_head, "Scarlett")
        || ci_contains(cid, "Gen")
    {
        caps |= LASAPI_HW_CAP_HIGH_RES;
        "Focusrite Scarlett Pro Audio DAC/Preamp".to_string()
    } else if usbid.starts_with("041e:")
        || ci_contains(uevent, "ca0132")
        || ci_contains(cid, "Creative")
    {
        caps |= LASAPI_HW_CAP_HIGH_RES;
        "Creative Sound Blaster DAC / HP Amp".to_string()
    } else if ci_contains(uevent, "snd_hda_intel") || ci_contains(cid, "Generic") {
        let short_cid: String = if cid.is_empty() {
            "Analog".to_string()
        } else {
            cid.chars().take(28).collect()
        };
        format!("HD-Audio Codec ({short_cid})")
    } else if !usbid.is_empty() {
        caps |= LASAPI_HW_CAP_HIGH_RES;
        format!("USB Audio Class DAC ({usbid})")
    } else {
        let nm = if cid.is_empty() { "PCM" } else { cid };
        format!("ALSA Card {card} ({nm})")
    };
    (arch, caps)
}

fn identify_card_architecture(card: i32) -> (String, u32) {
    if card < 0 {
        return (String::new(), 0);
    }
    let cid = read_small_text(&format!("/proc/asound/card{card}/id")).unwrap_or_default();
    let usbid = read_small_text(&format!("/proc/asound/card{card}/usbid")).unwrap_or_default();
    let uevent = read_small_text(&format!("/sys/class/sound/card{card}/device/uevent"))
        .unwrap_or_default();
    let stream_head =
        read_small_text(&format!("/proc/asound/card{card}/stream0")).unwrap_or_default();
    identify_card_arch_from_meta(card, &cid, &usbid, &uevent, &stream_head)
}

pub fn hid_desc_has_consumer_vol(desc: &[u8]) -> (bool, bool, u8) {
    let mut found_consumer = false;
    let mut in_consumer_page = false;
    let mut has_any_rid = false;
    let mut cur_rid = 0u8;
    let mut cons_has_rid = false;
    let mut cons_rid = 0u8;

    let mut i = 0usize;
    while i < desc.len() {
        let b = desc[i];
        if b == 0xFE {
            if i + 1 >= desc.len() {
                break;
            }
            let step = 3 + usize::from(desc[i + 1]);
            if i + step > desc.len() {
                break;
            }
            i += step;
            continue;
        }
        let mut sz = usize::from(b & 0x03);
        if sz == 3 {
            sz = 4;
        }
        let step = 1 + sz;
        if i + step > desc.len() {
            break;
        }
        let tag_type = b & 0xFC;
        if tag_type == 0x04 && sz >= 1 {
            let page = if sz >= 2 {
                u32::from(desc[i + 1]) | (u32::from(desc[i + 2]) << 8)
            } else {
                u32::from(desc[i + 1])
            };
            in_consumer_page = page == 0x0C;
            if in_consumer_page {
                found_consumer = true;
                if has_any_rid && !cons_has_rid {
                    cons_has_rid = true;
                    cons_rid = cur_rid;
                }
            }
        } else if tag_type == 0x84 && sz >= 1 {
            has_any_rid = true;
            cur_rid = desc[i + 1];
            if in_consumer_page {
                cons_has_rid = true;
                cons_rid = cur_rid;
            }
        } else if in_consumer_page && (tag_type == 0x08 || tag_type == 0x80) {
            found_consumer = true;
            if has_any_rid {
                cons_has_rid = true;
                cons_rid = cur_rid;
            }
        }
        i += step;
    }

    if found_consumer && cons_has_rid {
        (true, true, cons_rid)
    } else {
        (found_consumer, false, 0)
    }
}

fn probe_strix_hid_box(inner: &mut HwAnalogInner) {
    inner.close_hid();
    for idx in 0..24 {
        let hpath = format!("/dev/hidraw{idx}");
        let fd = open_raw_flags(&hpath, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
        if fd < 0 {
            continue;
        }
        let mut rinfo: HidrawDevinfo = unsafe { mem::zeroed() };
        let got_info = unsafe {
            ioctl_raw(
                fd,
                HIDIOCGRAWINFO,
                (&mut rinfo as *mut HidrawDevinfo).cast::<c_void>(),
            )
        } == 0;
        let vid = rinfo.vendor as u16;
        let pid = rinfo.product as u16;
        if got_info && rinfo.bustype == BUS_USB && vid == 0x0B05 && (pid == 0x180C || pid == 0x180D)
        {
            let mut dsize: c_int = 0;
            let mut has_cons = false;
            let mut has_rid = false;
            let mut cons_rid = 0u8;
            let got_dsz = unsafe {
                ioctl_raw(
                    fd,
                    HIDIOCGRDESCSIZE,
                    (&mut dsize as *mut c_int).cast::<c_void>(),
                )
            } == 0;
            if got_dsz && dsize > 0 && (dsize as usize) <= HID_MAX_DESCRIPTOR_SIZE {
                let mut rdesc = Box::new(HidrawReportDescriptor {
                    size: dsize as u32,
                    value: [0u8; HID_MAX_DESCRIPTOR_SIZE],
                });
                if unsafe {
                    ioctl_raw(
                        fd,
                        HIDIOCGRDESC,
                        (&mut *rdesc as *mut HidrawReportDescriptor).cast::<c_void>(),
                    )
                } == 0
                {
                    let len = (rdesc.size as usize).min(HID_MAX_DESCRIPTOR_SIZE);
                    (has_cons, has_rid, cons_rid) = hid_desc_has_consumer_vol(&rdesc.value[..len]);
                }
            }
            inner.hid_fd = fd;
            inner.hid_has_consumer = has_cons;
            inner.hid_has_report_id = has_rid;
            inner.hid_consumer_report_id = cons_rid;
            inner.caps_mask |= LASAPI_HW_CAP_HID_BOX;
            let detail = format!(
                "Detected ASUS Strix HID Control Box {vid:04x}:{pid:04x} (consumer={} rid={}:{cons_rid})",
                i32::from(has_cons),
                i32::from(has_rid)
            );
            dbg_emit(
                LASAPI_DBG_ENGINE,
                LASAPI_DBG_INFO,
                0,
                0,
                "HW_STRIX_HID",
                &hpath,
                &detail,
            );
            return;
        }
        close_raw_fd(fd);
    }
}

fn scan_alsa_card_mixers(card: i32, for_capture: bool, inner: &mut HwAnalogInner) {
    if card < 0 {
        return;
    }
    let ctl_path = format!("/dev/snd/controlC{card}");
    let fd = open_raw_flags(&ctl_path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if fd < 0 {
        return;
    }

    let mut el: SndCtlElemList = unsafe { mem::zeroed() };
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_CTL_IOCTL_ELEM_LIST,
            (&mut el as *mut SndCtlElemList).cast::<c_void>(),
        )
    } < 0
        || el.count == 0
    {
        close_raw_fd(fd);
        return;
    }

    let cnt = el.count.min(256) as usize;
    let mut pids = vec![SndCtlElemId::default(); cnt];
    el.offset = 0;
    el.space = cnt as c_uint;
    el.pids = pids.as_mut_ptr();
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_CTL_IOCTL_ELEM_LIST,
            (&mut el as *mut SndCtlElemList).cast::<c_void>(),
        )
    } < 0
    {
        close_raw_fd(fd);
        return;
    }

    let used = (el.used as usize).min(pids.len());
    let mut dac_prio: i32 = -1;
    for id in &pids[..used] {
        if id.iface != SNDRV_CTL_ELEM_IFACE_MIXER {
            continue;
        }
        let mut info: SndCtlElemInfo = unsafe { mem::zeroed() };
        info.id = *id;
        if unsafe {
            ioctl_raw(
                fd,
                SNDRV_CTL_IOCTL_ELEM_INFO,
                (&mut info as *mut SndCtlElemInfo).cast::<c_void>(),
            )
        } < 0
        {
            continue;
        }

        let nm = c_str_to_string(&id.name);
        let tp = info.tp as u32;
        if !for_capture {
            if tp == SNDRV_CTL_ELEM_TYPE_INTEGER
                && unsafe { info.value.integer.max > info.value.integer.min }
            {
                let pr = if ci_eq(&nm, "Master Playback Volume") || ci_eq(&nm, "Master") {
                    100
                } else if ci_eq(&nm, "DAC Playback Volume") || ci_eq(&nm, "DAC") {
                    95
                } else if ci_eq(&nm, "PCM Playback Volume") || ci_eq(&nm, "PCM") {
                    90
                } else if ci_eq(&nm, "Headphone Playback Volume") || ci_eq(&nm, "Headphone") {
                    inner.pb_hp_vol = HwCtlElem::from_info(id, &info, &nm);
                    85
                } else if ci_eq(&nm, "Front Playback Volume")
                    || ci_eq(&nm, "Speaker Playback Volume")
                    || ci_eq(&nm, "Line Out Playback Volume")
                {
                    inner.pb_spk_vol = HwCtlElem::from_info(id, &info, &nm);
                    80
                } else {
                    -1
                };
                if pr > dac_prio {
                    dac_prio = pr;
                    inner.pb_dac_vol = HwCtlElem::from_info(id, &info, &nm);
                    inner.caps_mask |= LASAPI_HW_CAP_DAC_VOL;
                }
            } else if tp == SNDRV_CTL_ELEM_TYPE_BOOLEAN {
                if ci_eq(&nm, "Master Playback Switch")
                    || ci_eq(&nm, "PCM Playback Switch")
                    || ci_eq(&nm, "DAC Playback Switch")
                {
                    inner.pb_dac_sw = HwCtlElem::from_info(id, &info, &nm);
                } else if ci_eq(&nm, "Headphone Playback Switch") || ci_eq(&nm, "Headphone") {
                    inner.pb_hp_sw = HwCtlElem::from_info(id, &info, &nm);
                } else if ci_eq(&nm, "Front Playback Switch")
                    || ci_eq(&nm, "Speaker Playback Switch")
                    || ci_eq(&nm, "Line Out Playback Switch")
                {
                    inner.pb_spk_sw = HwCtlElem::from_info(id, &info, &nm);
                }
            }

            if ci_contains(&nm, "HP Amp")
                || ci_contains(&nm, "Headphone Gain")
                || ci_contains(&nm, "Headphone Amplifier")
                || ci_contains(&nm, "Impedance")
                || ci_contains(&nm, "Output Gain")
            {
                inner.pb_amp_ctl = HwCtlElem::from_info(id, &info, &nm);
                inner.caps_mask |= LASAPI_HW_CAP_HP_AMP;
            }
            if ci_eq(&nm, "Analog Output")
                || ci_eq(&nm, "Output Select")
                || ci_contains(&nm, "Output Select")
                || ci_contains(&nm, "Headphone Jack Mode")
            {
                inner.pb_route_ctl = HwCtlElem::from_info(id, &info, &nm);
                inner.caps_mask |= LASAPI_HW_CAP_OUT_ROUTE;
            }
        } else {
            if ci_contains(&nm, "Line In 1 Air")
                || ci_contains(&nm, "Phantom Power")
                || ci_contains(&nm, "Autogain")
            {
                inner.caps_mask |= LASAPI_HW_CAP_LINE_AIR;
            }
            if ci_contains(&nm, "Mic Boost") && tp == SNDRV_CTL_ELEM_TYPE_INTEGER {
                if !inner.cap_boost_ctl.valid {
                    inner.cap_boost_ctl = HwCtlElem::from_info(id, &info, &nm);
                    inner.caps_mask |= LASAPI_HW_CAP_MIC_BOOST;
                }
            } else if (ci_eq(&nm, "Capture Volume")
                || ci_eq(&nm, "Mic Capture Volume")
                || ci_eq(&nm, "Mic"))
                && tp == SNDRV_CTL_ELEM_TYPE_INTEGER
            {
                if !inner.cap_vol_ctl.valid {
                    inner.cap_vol_ctl = HwCtlElem::from_info(id, &info, &nm);
                }
            } else if (ci_eq(&nm, "Capture Switch") || ci_eq(&nm, "Mic Capture Switch"))
                && tp == SNDRV_CTL_ELEM_TYPE_BOOLEAN
                && !inner.cap_sw_ctl.valid
            {
                inner.cap_sw_ctl = HwCtlElem::from_info(id, &info, &nm);
            }
        }
    }

    if !for_capture && inner.pb_hp_sw.valid && inner.pb_spk_sw.valid {
        inner.caps_mask |= LASAPI_HW_CAP_OUT_ROUTE;
    }

    close_raw_fd(fd);
}

fn write_alsa_int(fd: i32, el: &HwCtlElem, target_val: c_long) -> bool {
    if fd < 0 || !el.valid {
        return false;
    }
    if el.tp != SNDRV_CTL_ELEM_TYPE_INTEGER && el.tp != SNDRV_CTL_ELEM_TYPE_BOOLEAN {
        return false;
    }
    let v = if el.tp == SNDRV_CTL_ELEM_TYPE_INTEGER {
        target_val.clamp(el.imin, el.imax)
    } else {
        c_long::from(target_val != 0)
    };
    let mut ev = SndCtlElemValue::zeroed_for(el.id);
    let cnt = if el.count > 0 && el.count <= 16 {
        el.count as usize
    } else {
        1
    };
    for i in 0..cnt {
        ev.set_int(i, v);
    }
    unsafe {
        ioctl_raw(
            fd,
            SNDRV_CTL_IOCTL_ELEM_WRITE,
            (&mut ev as *mut SndCtlElemValue).cast::<c_void>(),
        ) == 0
    }
}

fn read_alsa_bool(fd: i32, el: &HwCtlElem) -> Option<bool> {
    if fd < 0 || !el.valid || el.tp != SNDRV_CTL_ELEM_TYPE_BOOLEAN {
        return None;
    }
    let mut ev = SndCtlElemValue::zeroed_for(el.id);
    let rc = unsafe {
        ioctl_raw(
            fd,
            SNDRV_CTL_IOCTL_ELEM_READ,
            (&mut ev as *mut SndCtlElemValue).cast::<c_void>(),
        )
    };
    if rc != 0 {
        return None;
    }
    let cnt = if el.count > 0 && el.count <= 16 {
        el.count as usize
    } else {
        1
    };
    Some((0..cnt).any(|i| ev.get_bool(i)))
}

#[inline]
fn has_word_token_ci(hay: &str, token: &str) -> bool {
    hay.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| !w.is_empty() && w.eq_ignore_ascii_case(token))
}

/// Returns `true` when metadata/USB descriptors identify dedicated headphones or headsets.
pub fn is_headphone_card_name(dev_name: &str, usbid: &str, stream_head: &str) -> bool {
    let combined_usb = !usbid.is_empty()
        || ci_contains(dev_name, "USB")
        || ci_contains(stream_head, "USB");
    let check_text = |text: &str| -> bool {
        !text.is_empty()
            && (ci_contains(text, "headphone")
                || ci_contains(text, "headset")
                || ci_contains(text, "earphone")
                || ci_contains(text, "earbud")
                || ci_contains(text, "qc ultra")
                || ci_contains(text, "qc45")
                || ci_contains(text, "wh-1000")
                || ci_contains(text, "wf-1000")
                || ci_contains(text, "airpods")
                || ci_contains(text, "soundcore")
                || ci_contains(text, "hyperx")
                || ci_contains(text, "steelseries")
                || ci_contains(text, "arctis")
                || ci_contains(text, "epos")
                || ci_contains(text, "sennheiser")
                || ci_contains(text, "beyerdynamic")
                || ci_contains(text, "audeze")
                || ci_contains(text, "qudelix")
                || ci_contains(text, "jabra")
                || ci_contains(text, "plantronics")
                || ci_contains(text, "apple usb-c")
                || has_word_token_ci(text, "iem")
                || (has_word_token_ci(text, "hp")
                    && (combined_usb || ci_contains(text, "bose") || ci_contains(text, "ultra"))))
    };

    if ci_starts_with(usbid, "05a7:") || ci_starts_with(usbid, "05ac:110") {
        return true;
    }
    check_text(dev_name) || check_text(stream_head)
}

#[inline]
fn ci_starts_with(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Probes ALSA card control elements to check if a playback Headphone jack is actively plugged.
pub fn card_has_plugged_headphone_jack(card: i32) -> bool {
    if card < 0 {
        return false;
    }
    let ctl_path = format!("/dev/snd/controlC{card}");
    let fd = open_raw_flags(&ctl_path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if fd < 0 {
        return false;
    }
    let mut el: SndCtlElemList = unsafe { mem::zeroed() };
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_CTL_IOCTL_ELEM_LIST,
            (&mut el as *mut SndCtlElemList).cast::<c_void>(),
        )
    } < 0
        || el.count == 0
    {
        close_raw_fd(fd);
        return false;
    }
    let cnt = el.count.min(256) as usize;
    let mut pids = vec![SndCtlElemId::default(); cnt];
    el.offset = 0;
    el.space = cnt as c_uint;
    el.pids = pids.as_mut_ptr();
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_CTL_IOCTL_ELEM_LIST,
            (&mut el as *mut SndCtlElemList).cast::<c_void>(),
        )
    } < 0
    {
        close_raw_fd(fd);
        return false;
    }
    let got = (el.used as usize).min(cnt);
    let mut plugged = false;
    for id in pids.iter().take(got) {
        let mut info: SndCtlElemInfo = unsafe { mem::zeroed() };
        info.id = *id;
        if unsafe {
            ioctl_raw(
                fd,
                SNDRV_CTL_IOCTL_ELEM_INFO,
                (&mut info as *mut SndCtlElemInfo).cast::<c_void>(),
            )
        } < 0
        {
            continue;
        }
        if (info.tp as u32) != SNDRV_CTL_ELEM_TYPE_BOOLEAN {
            continue;
        }
        let nm = c_str_to_string(&id.name);
        if !ci_contains(&nm, "Headphone")
            || !ci_contains(&nm, "Jack")
            || ci_contains(&nm, "Mic")
            || ci_contains(&nm, "Phantom")
        {
            continue;
        }
        let mut ev = SndCtlElemValue::zeroed_for(*id);
        if unsafe {
            ioctl_raw(
                fd,
                SNDRV_CTL_IOCTL_ELEM_READ,
                (&mut ev as *mut SndCtlElemValue).cast::<c_void>(),
            )
        } == 0
            && ev.get_bool(0)
        {
            plugged = true;
            break;
        }
    }
    close_raw_fd(fd);
    plugged
}

/// Returns `true` when an ALSA playback card represents USB/BT/headset headphones or an active headphone jack.
pub fn is_headphone_playback_card(card: i32, dev_name: &str) -> bool {
    if card < 0 {
        return is_headphone_card_name(dev_name, "", "");
    }
    let usbid = read_small_text(&format!("/proc/asound/card{card}/usbid")).unwrap_or_default();
    let stream_head =
        read_small_text(&format!("/proc/asound/card{card}/stream0")).unwrap_or_default();
    is_headphone_card_name(dev_name, &usbid, &stream_head) || card_has_plugged_headphone_jack(card)
}

fn write_alsa_enum_match(
    fd: i32,
    el: &HwCtlElem,
    kw1: Option<&str>,
    kw2: Option<&str>,
    fallback_idx: u32,
) -> bool {
    if fd < 0 || !el.valid || el.tp != SNDRV_CTL_ELEM_TYPE_ENUMERATED || el.items == 0 {
        return false;
    }
    let mut chosen = if fallback_idx < el.items {
        fallback_idx
    } else {
        0
    };
    let lim = el.items.min(32);
    for i in 0..lim {
        let mut info: SndCtlElemInfo = unsafe { mem::zeroed() };
        info.id = el.id;
        info.value.enumerated.item = i;
        if unsafe {
            ioctl_raw(
                fd,
                SNDRV_CTL_IOCTL_ELEM_INFO,
                (&mut info as *mut SndCtlElemInfo).cast::<c_void>(),
            )
        } == 0
        {
            let ename = unsafe { c_str_to_string(&info.value.enumerated.name) };
            let m1 = kw1.is_some_and(|k| ci_contains(&ename, k));
            let m2 = kw2.is_some_and(|k| ci_contains(&ename, k));
            if m1 || m2 {
                chosen = i;
                break;
            }
        }
    }
    let mut ev = SndCtlElemValue::zeroed_for(el.id);
    let cnt = if el.count > 0 && el.count <= 16 {
        el.count as usize
    } else {
        1
    };
    for i in 0..cnt {
        ev.set_enum(i, chosen);
    }
    unsafe {
        ioctl_raw(
            fd,
            SNDRV_CTL_IOCTL_ELEM_WRITE,
            (&mut ev as *mut SndCtlElemValue).cast::<c_void>(),
        ) == 0
    }
}

pub fn score_card_with_meta(
    dev_name: &str,
    is_usb: bool,
    has_usb_file: bool,
    usbid: &str,
    uevent: &str,
) -> i32 {
    score_card_with_meta_hp(dev_name, is_usb, has_usb_file, usbid, uevent, "", false)
}

pub fn score_card_with_meta_hp(
    dev_name: &str,
    is_usb: bool,
    has_usb_file: bool,
    usbid: &str,
    uevent: &str,
    stream_head: &str,
    has_plugged_hp_jack: bool,
) -> i32 {
    let mut sc = 0i32;
    if is_usb || has_usb_file {
        sc += 100;
    }
    if !dev_name.is_empty() && !ci_contains(dev_name, "HDMI") && !ci_contains(dev_name, "DisplayPort") {
        sc += 50;
    }
    if ci_contains(dev_name, "Scarlett") || usbid.starts_with("1235:") {
        sc += 140;
    }
    if ci_contains(dev_name, "STRIX")
        || ci_eq(usbid, "0b05:180c")
        || ci_eq(usbid, "0b05:180d")
    {
        sc += 130;
    }
    if ci_contains(dev_name, "Xonar")
        || ci_contains(uevent, "virtuoso")
        || ci_contains(dev_name, "Sound Blaster")
        || ci_contains(uevent, "ca0132")
        || ci_contains(dev_name, "Essence")
        || ci_contains(dev_name, "Sabre")
    {
        sc += 120;
    }
    if is_headphone_card_name(dev_name, usbid, stream_head) {
        sc += 200;
    }
    if has_plugged_hp_jack {
        sc += 220;
    }
    if ci_contains(dev_name, "Generic USB Audio") || ci_eq(usbid, "26ce:0a08") {
        sc -= 20;
    }
    sc
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_default_cfg(cfg: *mut LasapiHwAnalogCfg) {
    if !cfg.is_null() {
        unsafe { *cfg = default_cfg_value() };
    }
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_clamp_cfg(cfg: *mut LasapiHwAnalogCfg) {
    if !cfg.is_null() {
        clamp_cfg(unsafe { &mut *cfg });
    }
}

#[no_mangle]
pub extern "C" fn lasapi_hw_analog_create() -> *mut LasapiHwAnalogCtx {
    Box::into_raw(Box::new(LasapiHwAnalogCtx::new()))
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_destroy(ctx: *mut LasapiHwAnalogCtx) {
    if !ctx.is_null() {
        drop(unsafe { Box::from_raw(ctx) });
    }
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_probe(
    ctx: *mut LasapiHwAnalogCtx,
    pb_card: c_int,
    cap_card: c_int,
) {
    if ctx.is_null() {
        return;
    }
    let ctx_ref = unsafe { &*ctx };
    let Ok(mut inner) = ctx_ref.inner.lock() else {
        return;
    };
    inner.pb_card = pb_card;
    inner.cap_card = cap_card;
    inner.caps_mask = 0;
    inner.pb_dac_vol = HwCtlElem::default();
    inner.pb_dac_sw = HwCtlElem::default();
    inner.pb_hp_vol = HwCtlElem::default();
    inner.pb_hp_sw = HwCtlElem::default();
    inner.pb_spk_vol = HwCtlElem::default();
    inner.pb_spk_sw = HwCtlElem::default();
    inner.pb_amp_ctl = HwCtlElem::default();
    inner.pb_route_ctl = HwCtlElem::default();
    inner.cap_boost_ctl = HwCtlElem::default();
    inner.cap_vol_ctl = HwCtlElem::default();
    inner.cap_sw_ctl = HwCtlElem::default();

    if pb_card >= 0 {
        let (arch, caps) = identify_card_architecture(pb_card);
        inner.card_arch_name = arch;
        inner.caps_mask |= caps;
        scan_alsa_card_mixers(pb_card, false, &mut inner);
    } else {
        inner.card_arch_name = "BlueZ / Virtual Stream".to_string();
    }

    if cap_card >= 0 {
        let (arch, caps) = identify_card_architecture(cap_card);
        if pb_card < 0 {
            inner.card_arch_name = arch;
            inner.caps_mask |= caps;
        } else {
            inner.caps_mask |= caps & (LASAPI_HW_CAP_STRIX_ARCH | LASAPI_HW_CAP_HIGH_RES);
        }
        scan_alsa_card_mixers(cap_card, true, &mut inner);
    }

    if (inner.caps_mask & LASAPI_HW_CAP_STRIX_ARCH) != 0 {
        probe_strix_hid_box(&mut inner);
    } else {
        inner.close_hid();
    }

    if (inner.caps_mask & LASAPI_HW_CAP_STRIX_ARCH) != 0
        && (inner.caps_mask & LASAPI_HW_CAP_HP_AMP) == 0
    {
        inner.diag_note = "Strix CM6632AE UAC: DAC/Analog Soft-Amp + Preamp Ready".to_string();
    } else if (inner.caps_mask & LASAPI_HW_CAP_DAC_VOL) != 0 {
        let short_name: String = inner.pb_dac_vol.name.chars().take(24).collect();
        inner.diag_note = format!("ALSA Hardware DAC Unity Stage Active ({short_name})");
    } else {
        inner.diag_note = "Preserving Hardware Stage (Digital/AVRCP Nominal)".to_string();
    }

    inner.sync_cache_valid = false;
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_sync(
    ctx: *mut LasapiHwAnalogCtx,
    cfg_in: *const LasapiHwAnalogCfg,
    desktop_vol_pct: u32,
    desktop_muted: bool,
    _mic_vol_pct: u32,
    mic_muted: bool,
) {
    if ctx.is_null() || cfg_in.is_null() {
        return;
    }
    let mut cfg = unsafe { *cfg_in };
    clamp_cfg(&mut cfg);

    let ctx_ref = unsafe { &*ctx };
    let Ok(mut inner) = ctx_ref.inner.lock() else {
        return;
    };

    if inner.sync_cache_valid
        && inner.last_pb_card == inner.pb_card
        && inner.last_cap_card == inner.cap_card
        && cfg_eq(&inner.last_cfg, &cfg)
        && inner.last_d_vol == desktop_vol_pct
        && inner.last_d_muted == desktop_muted
        && inner.last_m_muted == mic_muted
    {
        return;
    }

    let mut dac_unity_ok = false;
    if inner.pb_card >= 0 {
        let pfd = open_raw_flags(
            &format!("/dev/snd/controlC{}", inner.pb_card),
            O_RDWR | O_NONBLOCK | O_CLOEXEC,
        );
        if pfd >= 0 {
            if cfg.output_route != LASAPI_HW_ROUTE_AUTO {
                if inner.pb_route_ctl.valid {
                    if cfg.output_route == LASAPI_HW_ROUTE_HP {
                        write_alsa_enum_match(
                            pfd,
                            &inner.pb_route_ctl,
                            Some("Headphone"),
                            Some("FP"),
                            1,
                        );
                    } else {
                        write_alsa_enum_match(
                            pfd,
                            &inner.pb_route_ctl,
                            Some("Speaker"),
                            Some("Line"),
                            0,
                        );
                    }
                }
                if inner.pb_hp_sw.valid && inner.pb_spk_sw.valid {
                    write_alsa_int(
                        pfd,
                        &inner.pb_hp_sw,
                        c_long::from(cfg.output_route == LASAPI_HW_ROUTE_HP),
                    );
                    write_alsa_int(
                        pfd,
                        &inner.pb_spk_sw,
                        c_long::from(cfg.output_route == LASAPI_HW_ROUTE_SPK),
                    );
                }
            }

            if cfg.hp_amp_tier != LASAPI_HW_AMP_AUTO && inner.pb_amp_ctl.valid {
                if inner.pb_amp_ctl.tp == SNDRV_CTL_ELEM_TYPE_ENUMERATED {
                    if cfg.hp_amp_tier == LASAPI_HW_AMP_LOW {
                        write_alsa_enum_match(pfd, &inner.pb_amp_ctl, Some("Low"), Some("32"), 0);
                    } else if cfg.hp_amp_tier == LASAPI_HW_AMP_MED {
                        write_alsa_enum_match(pfd, &inner.pb_amp_ctl, Some("Med"), Some("150"), 1);
                    } else {
                        write_alsa_enum_match(pfd, &inner.pb_amp_ctl, Some("High"), Some("600"), 2);
                    }
                } else if inner.pb_amp_ctl.tp == SNDRV_CTL_ELEM_TYPE_INTEGER {
                    let span = inner.pb_amp_ctl.imax - inner.pb_amp_ctl.imin;
                    let target = match cfg.hp_amp_tier {
                        LASAPI_HW_AMP_LOW => inner.pb_amp_ctl.imin,
                        LASAPI_HW_AMP_MED => inner.pb_amp_ctl.imin + span / 2,
                        _ => inner.pb_amp_ctl.imax,
                    };
                    write_alsa_int(pfd, &inner.pb_amp_ctl, target);
                }
            }

            if cfg.unlock_0db_rails != 0 {
                if inner.pb_hp_vol.valid
                    && (!inner.pb_dac_vol.valid
                        || !ci_eq(&inner.pb_hp_vol.name, &inner.pb_dac_vol.name))
                {
                    write_alsa_int(pfd, &inner.pb_hp_vol, inner.pb_hp_vol.imax);
                }
                if inner.pb_spk_vol.valid
                    && (!inner.pb_dac_vol.valid
                        || !ci_eq(&inner.pb_spk_vol.name, &inner.pb_dac_vol.name))
                {
                    write_alsa_int(pfd, &inner.pb_spk_vol, inner.pb_spk_vol.imax);
                }
                if cfg.output_route == LASAPI_HW_ROUTE_AUTO {
                    if inner.pb_hp_sw.valid {
                        write_alsa_int(pfd, &inner.pb_hp_sw, 1);
                    }
                    if inner.pb_spk_sw.valid {
                        write_alsa_int(pfd, &inner.pb_spk_sw, 1);
                    }
                }
            }

            if cfg.hw_dac_unity_vol != 0 && inner.pb_dac_vol.valid {
                let hw_pct = if desktop_muted {
                    0
                } else {
                    desktop_vol_pct.min(100)
                };
                let span = inner.pb_dac_vol.imax - inner.pb_dac_vol.imin;
                let val = inner.pb_dac_vol.imin
                    + (((f64::from(hw_pct) * (span as f64)) / 100.0) + 0.5) as c_long;
                if write_alsa_int(pfd, &inner.pb_dac_vol, val) {
                    dac_unity_ok = true;
                }
                if inner.pb_dac_sw.valid {
                    write_alsa_int(pfd, &inner.pb_dac_sw, c_long::from(!desktop_muted));
                }
            } else if !desktop_muted
                && (!inner.sync_cache_valid
                    || inner.pb_card != inner.last_pb_card
                    || inner.last_d_muted)
            {
                if inner.pb_dac_sw.valid && read_alsa_bool(pfd, &inner.pb_dac_sw) == Some(false) {
                    write_alsa_int(pfd, &inner.pb_dac_sw, 1);
                }
                if inner.pb_hp_sw.valid
                    && cfg.output_route != LASAPI_HW_ROUTE_SPK
                    && card_has_plugged_headphone_jack(inner.pb_card)
                    && read_alsa_bool(pfd, &inner.pb_hp_sw) == Some(false)
                {
                    write_alsa_int(pfd, &inner.pb_hp_sw, 1);
                }
            }
            close_raw_fd(pfd);
        }
    }

    let mut amp_stage_db = cfg.analog_boost_db;
    if cfg.hp_amp_tier != LASAPI_HW_AMP_AUTO && !inner.pb_amp_ctl.valid {
        if cfg.hp_amp_tier == LASAPI_HW_AMP_MED {
            amp_stage_db += 3.5;
        } else if cfg.hp_amp_tier == LASAPI_HW_AMP_HIGH {
            amp_stage_db += 7.0;
        }
    }
    if amp_stage_db > 15.0 {
        amp_stage_db = 15.0;
    }
    let drive_lin = if amp_stage_db > 0.01 {
        10.0f32.powf(amp_stage_db / 20.0)
    } else {
        1.0
    };
    let drive_ppm = ((drive_lin * 1_000_000.0) + 0.5).max(1_000_000.0) as u32;

    let mut cap_makeup_db = cfg.mic_hw_boost_db;
    if inner.cap_card >= 0 && cfg.mic_hw_boost_db > 0.1 {
        let cfd = open_raw_flags(
            &format!("/dev/snd/controlC{}", inner.cap_card),
            O_RDWR | O_NONBLOCK | O_CLOEXEC,
        );
        if cfd >= 0 {
            if inner.cap_sw_ctl.valid {
                write_alsa_int(cfd, &inner.cap_sw_ctl, c_long::from(!mic_muted));
            }
            if inner.cap_vol_ctl.valid {
                write_alsa_int(cfd, &inner.cap_vol_ctl, inner.cap_vol_ctl.imax);
            }
            if inner.cap_boost_ctl.valid && inner.cap_boost_ctl.imax > inner.cap_boost_ctl.imin {
                let span = inner.cap_boost_ctl.imax - inner.cap_boost_ctl.imin;
                let step = if cfg.mic_hw_boost_db >= 16.0 {
                    inner.cap_boost_ctl.imax
                } else {
                    inner.cap_boost_ctl.imin
                        + (((cfg.mic_hw_boost_db / 24.0) * (span as f32)) + 0.5) as c_long
                };
                if write_alsa_int(cfd, &inner.cap_boost_ctl, step) {
                    let absorbed = if step >= inner.cap_boost_ctl.imax {
                        18.0
                    } else {
                        10.0
                    };
                    cap_makeup_db = (cap_makeup_db - absorbed).max(0.0);
                }
            }
            close_raw_fd(cfd);
        }
    }
    let cap_lin = if cap_makeup_db > 0.05 {
        10.0f32.powf(cap_makeup_db / 20.0)
    } else {
        1.0
    };
    let cap_ppm = ((cap_lin * 1_000_000.0) + 0.5).max(1_000_000.0) as u32;

    ctx_ref
        .dac_unity_active
        .store(i32::from(dac_unity_ok), Ordering::Release);
    ctx_ref
        .softknee_active
        .store(i32::from(amp_stage_db > 0.05), Ordering::Release);
    ctx_ref.drive_ppm.store(drive_ppm, Ordering::Release);
    ctx_ref.cap_makeup_ppm.store(cap_ppm, Ordering::Release);

    inner.diag_note = if dac_unity_ok && amp_stage_db > 0.05 {
        format!(
            "DAC Unity ({}) + {amp_stage_db:.1}dB Analog Amp Drive",
            inner.pb_dac_vol.name
        )
    } else if dac_unity_ok {
        format!("Bit-Perfect DAC Unity -> {}", inner.pb_dac_vol.name)
    } else if amp_stage_db > 0.05 || cfg.mic_hw_boost_db > 0.1 {
        format!(
            "Amp +{amp_stage_db:.1}dB | HW Mic Preamp +{:.1}dB",
            cfg.mic_hw_boost_db
        )
    } else {
        "Hardware Defaults Preserved (Zero Alteration)".to_string()
    };

    inner.last_pb_card = inner.pb_card;
    inner.last_cap_card = inner.cap_card;
    inner.last_cfg = cfg;
    inner.last_d_vol = desktop_vol_pct;
    inner.last_d_muted = desktop_muted;
    inner.last_m_muted = mic_muted;
    inner.sync_cache_valid = true;
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_poll_hid(
    ctx: *mut LasapiHwAnalogCtx,
    out_vol_delta: *mut c_int,
    out_mute_toggle: *mut c_int,
) -> c_int {
    if !out_vol_delta.is_null() {
        unsafe { *out_vol_delta = 0 };
    }
    if !out_mute_toggle.is_null() {
        unsafe { *out_mute_toggle = 0 };
    }
    if ctx.is_null() {
        return 0;
    }
    let ctx_ref = unsafe { &*ctx };
    let Ok(mut inner) = ctx_ref.inner.lock() else {
        return 0;
    };
    let fd = inner.hid_fd;
    if fd < 0 || !inner.hid_has_consumer {
        return 0;
    }

    let mut buf = [0u8; 64];
    let mut events = 0;
    for _ in 0..16 {
        let n = nb_read_raw(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let len = n as usize;
        let bits = if inner.hid_has_report_id {
            if !(2..=8).contains(&len) || buf[0] != inner.hid_consumer_report_id {
                continue;
            }
            buf[1]
        } else {
            if !(1..=8).contains(&len) {
                continue;
            }
            if len >= 2 { buf[1] } else { buf[0] }
        };
        let up = (bits & 0x01) != 0;
        let dn = (bits & 0x02) != 0;
        let mute = (bits & 0x04) != 0;
        if up && !inner.hid_prev_up {
            if !out_vol_delta.is_null() {
                unsafe { *out_vol_delta += 2 };
            }
            events += 1;
        }
        if dn && !inner.hid_prev_dn {
            if !out_vol_delta.is_null() {
                unsafe { *out_vol_delta -= 2 };
            }
            events += 1;
        }
        if mute && !inner.hid_prev_mute {
            if !out_mute_toggle.is_null() {
                unsafe { *out_mute_toggle = 1 };
            }
            events += 1;
        }
        inner.hid_prev_up = up;
        inner.hid_prev_dn = dn;
        inner.hid_prev_mute = mute;
    }
    events
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_get_info(
    ctx: *mut LasapiHwAnalogCtx,
    out_info: *mut LasapiHwAnalogInfo,
) {
    if out_info.is_null() {
        return;
    }
    unsafe { ptr::write_bytes(out_info, 0, 1) };
    let out = unsafe { &mut *out_info };
    if ctx.is_null() {
        out.active_pb_card = -1;
        out.active_cap_card = -1;
        return;
    }
    let ctx_ref = unsafe { &*ctx };
    let Ok(inner) = ctx_ref.inner.lock() else {
        out.active_pb_card = -1;
        out.active_cap_card = -1;
        return;
    };

    out.caps_mask = inner.caps_mask;
    out.dac_unity_active = ctx_ref.dac_unity_active.load(Ordering::Relaxed) as u32;
    out.hid_box_connected = u32::from(inner.hid_fd >= 0);
    out.active_pb_card = inner.pb_card;
    out.active_cap_card = inner.cap_card;
    copy_str_to_c(&mut out.card_arch_name, &inner.card_arch_name);

    let dac_name = if inner.pb_dac_vol.valid {
        inner.pb_dac_vol.name.as_str()
    } else {
        "Software/AVRCP"
    };
    copy_str_to_c(&mut out.dac_ctl_name, dac_name);

    let amp_name = if inner.pb_amp_ctl.valid {
        inner.pb_amp_ctl.name.as_str()
    } else if inner.pb_route_ctl.valid {
        inner.pb_route_ctl.name.as_str()
    } else if (inner.caps_mask & LASAPI_HW_CAP_STRIX_ARCH) != 0 {
        "TPA6120A2 Analog Stage"
    } else {
        "Default Output Stage"
    };
    copy_str_to_c(&mut out.amp_ctl_name, amp_name);

    let cap_name = if inner.cap_boost_ctl.valid {
        inner.cap_boost_ctl.name.as_str()
    } else if inner.cap_vol_ctl.valid {
        inner.cap_vol_ctl.name.as_str()
    } else {
        "Direct PCM Preamp"
    };
    copy_str_to_c(&mut out.cap_ctl_name, cap_name);
    copy_str_to_c(&mut out.diag_note, &inner.diag_note);
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_is_dac_unity_active(
    ctx: *const LasapiHwAnalogCtx,
) -> bool {
    !ctx.is_null() && unsafe { &*ctx }.dac_unity_active.load(Ordering::Relaxed) != 0
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_get_drive_linear(
    ctx: *const LasapiHwAnalogCtx,
) -> f32 {
    if ctx.is_null() {
        return 1.0;
    }
    let ppm = unsafe { &*ctx }.drive_ppm.load(Ordering::Relaxed);
    (ppm as f32) * (1.0 / 1_000_000.0)
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_softknee_active(
    ctx: *const LasapiHwAnalogCtx,
) -> bool {
    !ctx.is_null() && unsafe { &*ctx }.softknee_active.load(Ordering::Relaxed) != 0
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_get_cap_makeup_linear(
    ctx: *const LasapiHwAnalogCtx,
) -> f32 {
    if ctx.is_null() {
        return 1.0;
    }
    let ppm = unsafe { &*ctx }.cap_makeup_ppm.load(Ordering::Relaxed);
    (ppm as f32) * (1.0 / 1_000_000.0)
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_hw_analog_score_card(
    card_index: u32,
    dev_name: *const c_char,
) -> c_int {
    let p_usb = format!("/proc/asound/card{card_index}/usbid");
    let p_drv = format!("/sys/class/sound/card{card_index}/device/uevent");
    let p_stream = format!("/proc/asound/card{card_index}/stream0");
    let usbid_opt = read_small_text(&p_usb);
    let is_usb = usbid_opt.is_some();
    let has_usb_file = Path::new(&p_usb).exists();
    let usbid = usbid_opt.unwrap_or_default();
    let uevent = read_small_text(&p_drv).unwrap_or_default();
    let stream_head = read_small_text(&p_stream).unwrap_or_default();
    let has_hp_jack = card_has_plugged_headphone_jack(card_index as i32);
    let dname = unsafe { ptr_to_opt_str(dev_name) }.unwrap_or_default();
    score_card_with_meta_hp(
        &dname,
        is_usb,
        has_usb_file,
        &usbid,
        &uevent,
        &stream_head,
        has_hp_jack,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn alsa_and_hidraw_uapi_layouts_match_kernel() {
        assert_eq!(size_of::<HidrawDevinfo>(), 8);
        assert_eq!(size_of::<HidrawReportDescriptor>(), 4100);
        assert_eq!(size_of::<SndCtlElemId>(), 64);
        assert_eq!(offset_of!(SndCtlElemId, iface), 4);
        assert_eq!(offset_of!(SndCtlElemId, name), 16);
        assert_eq!(size_of::<SndCtlElemList>(), 80);
        assert_eq!(offset_of!(SndCtlElemList, used), 8);
        assert_eq!(offset_of!(SndCtlElemList, count), 12);
        assert_eq!(offset_of!(SndCtlElemList, pids), 16);
        assert_eq!(size_of::<SndCtlElemInfo>(), 272);
        assert_eq!(offset_of!(SndCtlElemInfo, tp), 64);
        assert_eq!(offset_of!(SndCtlElemInfo, count), 72);
        assert_eq!(offset_of!(SndCtlElemInfo, value), 80);
        assert_eq!(size_of::<SndCtlElemValue>(), 1224);
        assert_eq!(offset_of!(SndCtlElemValue, data), 72);
    }

    #[test]
    fn hid_consumer_page_and_card_scoring_work() {
        let desc = [0x05u8, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x02, 0x09, 0xE9];
        assert_eq!(hid_desc_has_consumer_vol(&desc), (true, true, 2));
        assert_eq!(hid_desc_has_consumer_vol(&[0x05, 0x01, 0x09, 0x06]), (false, false, 0));

        let sc_strix = score_card_with_meta("STRIX RAID DLX USB Audio", false, false, "", "");
        let sc_soar = score_card_with_meta("ASUS STRIX SOAR Audio", false, false, "", "");
        let sc_scar = score_card_with_meta("Scarlett 2i2 4th Gen", false, false, "", "");
        let sc_hdmi = score_card_with_meta("HDA ATI HDMI", false, false, "", "");
        assert!(sc_strix > sc_hdmi + 100);
        assert!(sc_soar > sc_hdmi + 100);
        assert!(sc_scar > sc_hdmi + 100);

        let sc_scar_usb = score_card_with_meta("Focusrite Scarlett 2i2 4th Gen", true, true, "1235:8219", "");
        let sc_bose_usb = score_card_with_meta(
            "Bose Corporation Bose QC Ultra 2 HP - USB Audio",
            true,
            true,
            "05a7:4082",
            "",
        );
        assert!(sc_bose_usb > sc_scar_usb);
        assert!(is_headphone_card_name("Bose Corporation Bose QC Ultra 2 HP (USB)", "05a7:4082", ""));
        assert!(is_headphone_card_name("HyperX Cloud III Wireless", "", ""));
        assert!(!is_headphone_card_name("Focusrite Scarlett 2i2 4th Gen", "1235:8219", ""));
        assert!(!is_headphone_card_name("HDA ATI HDMI", "", ""));

        let sc_jack_hp = score_card_with_meta_hp("Generic USB Audio", true, true, "26ce:0a08", "", "", true);
        assert!(sc_jack_hp > sc_scar_usb);

        let (arch, caps) = identify_card_arch_from_meta(2, "DLX", "0b05:180c", "", "");
        assert!(arch.contains("STRIX"));
        assert_ne!(caps & LASAPI_HW_CAP_STRIX_ARCH, 0);
    }

    #[test]
    fn hw_analog_cfg_clamp_and_ctx_getters_safe() {
        let mut cfg = default_cfg_value();
        cfg.hp_amp_tier = 99;
        cfg.output_route = 77;
        cfg.analog_boost_db = f32::NAN;
        cfg.mic_hw_boost_db = 42.0;
        clamp_cfg(&mut cfg);
        assert_eq!(cfg.hp_amp_tier, LASAPI_HW_AMP_AUTO);
        assert_eq!(cfg.output_route, LASAPI_HW_ROUTE_AUTO);
        assert_eq!(cfg.analog_boost_db, 0.0);
        assert_eq!(cfg.mic_hw_boost_db, 24.0);

        let ctx = lasapi_hw_analog_create();
        unsafe {
            assert!(!lasapi_hw_analog_is_dac_unity_active(ctx));
            assert!((lasapi_hw_analog_get_drive_linear(ctx) - 1.0).abs() < 1e-6);
            lasapi_hw_analog_destroy(ctx);
        }
    }
}
