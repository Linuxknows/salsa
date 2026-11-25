// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]

use core::cell::RefCell;
use core::ffi::{c_int, c_void};
use core::mem;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audiodg_engine::{
    hw_param_set_mask, hw_param_set_minmax, hw_params_any, SndPcmHwParams, SndPcmSwParams,
    SndXferi, ERRNO_EAGAIN, ERRNO_EINTR, ERRNO_EPIPE, ERRNO_ESTRPIPE, SNDRV_PCM_ACCESS_RW_INTERLEAVED,
    SNDRV_PCM_FORMAT_S16_LE, SNDRV_PCM_FORMAT_S24_3LE, SNDRV_PCM_FORMAT_S32_LE,
    SNDRV_PCM_HW_PARAM_ACCESS, SNDRV_PCM_HW_PARAM_BUFFER_SIZE, SNDRV_PCM_HW_PARAM_CHANNELS,
    SNDRV_PCM_HW_PARAM_FORMAT, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, SNDRV_PCM_HW_PARAM_RATE,
    SNDRV_PCM_IOCTL_HW_PARAMS, SNDRV_PCM_IOCTL_HW_REFINE, SNDRV_PCM_IOCTL_PREPARE,
    SNDRV_PCM_IOCTL_READI_FRAMES, SNDRV_PCM_IOCTL_START, SNDRV_PCM_IOCTL_SW_PARAMS,
    SNDRV_PCM_TSTAMP_ENABLE,
};
use crate::bluez_media::{CAP_RING_FRAMES, CAP_RING_MASK};
use crate::c_abi::{
    c_str_to_string, copy_str_to_c, LasapiDeviceEntry, LasapiDeviceList, LASAPI_DBG_ALSA,
    LASAPI_DBG_INFO, LASAPI_DBG_WARN, LASAPI_MAX_DEVICES,
};
use crate::hw_analog::{
    SndCtlElemId, SndCtlElemInfo, SndCtlElemList, SndCtlElemValue, SNDRV_CTL_ELEM_ID_NAME_MAXLEN,
    SNDRV_CTL_ELEM_IFACE_PCM, SNDRV_CTL_ELEM_TYPE_BOOLEAN, SNDRV_CTL_ELEM_TYPE_BYTES,
    SNDRV_CTL_IOCTL_ELEM_INFO, SNDRV_CTL_IOCTL_ELEM_LIST, SNDRV_CTL_IOCTL_ELEM_READ,
};
use crate::unix_sys::{
    close_raw_fd, dbg_emit, do_poll, enable_realtime, ioctl_raw, last_errno, mono_ns,
    open_raw_flags, PollFd, O_CLOEXEC, O_NONBLOCK, O_RDONLY, O_RDWR, POLLERR, POLLHUP, POLLIN,
};

pub const MAX_ALSA_CARDS_SCAN: usize = 32;
const LASAPI_MAX_CAP_CHANNELS: u32 = 32;

#[derive(Clone)]
pub struct AlsaCardJacks {
    pub checked: bool,
    pub hdmi_jack: [i32; 64],
    pub hdmi_name: [[u8; 32]; 64],
    pub out_jack_count: i32,
    pub out_jack_on: i32,
    pub speaker_jack: i32,
    pub headphone_jack: i32,
    pub lineout_jack: i32,
    pub in_jack_count: i32,
    pub in_jack_on: i32,
    pub in_jack_by_idx: [i32; 16],
}

impl Default for AlsaCardJacks {
    fn default() -> Self {
        Self {
            checked: false,
            hdmi_jack: [-1; 64],
            hdmi_name: [[0u8; 32]; 64],
            out_jack_count: 0,
            out_jack_on: 0,
            speaker_jack: -1,
            headphone_jack: -1,
            lineout_jack: -1,
            in_jack_count: 0,
            in_jack_on: 0,
            in_jack_by_idx: [-1; 16],
        }
    }
}

pub struct AlsaTopology {
    pub jacks: Vec<AlsaCardJacks>,
    pub pb_nonhdmi_cnt: [i32; MAX_ALSA_CARDS_SCAN],
    pub cap_cnt: [i32; MAX_ALSA_CARDS_SCAN],
    pub has_card: [bool; MAX_ALSA_CARDS_SCAN],
    pub pcm_hash: u64,
}

impl Default for AlsaTopology {
    fn default() -> Self {
        Self {
            jacks: vec![AlsaCardJacks::default(); MAX_ALSA_CARDS_SCAN],
            pb_nonhdmi_cnt: [0; MAX_ALSA_CARDS_SCAN],
            cap_cnt: [0; MAX_ALSA_CARDS_SCAN],
            has_card: [false; MAX_ALSA_CARDS_SCAN],
            pcm_hash: 14_695_981_039_346_656_037u64,
        }
    }
}

#[inline]
pub fn fnv1a_step(mut h: u64, data: &[u8]) -> u64 {
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(1_099_511_628_211u64);
    }
    h
}

#[inline]
fn ci_contains(hay: &str, needle: &str) -> bool {
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() {
        return true;
    }
    if hb.len() < nb.len() {
        return false;
    }
    hb.windows(nb.len()).any(|w| w.eq_ignore_ascii_case(nb))
}

pub fn parse_asound_cards(card_names: &mut [String; MAX_ALSA_CARDS_SCAN], card_is_usb: &mut [bool; MAX_ALSA_CARDS_SCAN]) {
    for n in card_names.iter_mut() {
        n.clear();
    }
    card_is_usb.fill(false);

    let Ok(f) = File::open("/proc/asound/cards") else {
        return;
    };
    let reader = BufReader::new(f);
    let mut last_idx: i32 = -1;
    for line_res in reader.lines() {
        let Ok(line) = line_res else {
            break;
        };
        let p = line.trim_start_matches(' ');
        if p.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) {
            let num_end = p
                .bytes()
                .position(|b| !b.is_ascii_digit())
                .unwrap_or(p.len());
            if let Ok(idx) = p[..num_end].parse::<usize>() {
                if idx < MAX_ALSA_CARDS_SCAN {
                    last_idx = idx as i32;
                    if p.contains("USB-Audio") {
                        card_is_usb[idx] = true;
                    }
                    if let Some(pos) = p.find(" - ") {
                        let after = &p[pos + 3..];
                        let trimmed = after.trim_end_matches(['\r', '\n']);
                        card_names[idx] = trimmed.chars().take(63).collect();
                    }
                }
            }
        } else if last_idx >= 0 {
            let idx = last_idx as usize;
            if idx < MAX_ALSA_CARDS_SCAN {
                if let Some(pos) = p.find(" at ") {
                    if pos > 0 {
                        card_names[idx] = p[..pos].chars().take(63).collect();
                    }
                }
            }
            last_idx = -1;
        }
    }
}

fn card_eld_any_present(card: usize) -> (bool, bool) {
    let mut has_eld_files = false;
    let mut any_on = false;
    for k in 0..8 {
        let path = format!("/proc/asound/card{card}/eld#0.{k}");
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        has_eld_files = true;
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("monitor_present") {
                if rest.trim_start_matches([' ', '\t']).starts_with('1') {
                    any_on = true;
                }
                break;
            }
        }
    }
    (any_on, has_eld_files)
}

fn probe_card_jacks(card: usize, cj: &mut AlsaCardJacks) {
    *cj = AlsaCardJacks::default();
    if card >= MAX_ALSA_CARDS_SCAN {
        return;
    }
    let ctl_path = format!("/dev/snd/controlC{card}");
    let fd = open_raw_flags(&ctl_path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if fd < 0 {
        return;
    }
    cj.checked = true;

    let mut el: SndCtlElemList = unsafe { mem::zeroed() };
    if unsafe { ioctl_raw(fd, SNDRV_CTL_IOCTL_ELEM_LIST, (&mut el as *mut SndCtlElemList).cast()) }
        < 0
        || el.count == 0
    {
        close_raw_fd(fd);
        return;
    }

    let cnt = el.count.min(512) as usize;
    let mut pids = vec![SndCtlElemId::default(); cnt];
    el.offset = 0;
    el.space = cnt as u32;
    el.pids = pids.as_mut_ptr();
    if unsafe { ioctl_raw(fd, SNDRV_CTL_IOCTL_ELEM_LIST, (&mut el as *mut SndCtlElemList).cast()) }
        < 0
    {
        close_raw_fd(fd);
        return;
    }

    let used = (el.used as usize).min(cnt);
    for id in pids.iter().take(used) {
        let nm_end = id
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(SNDRV_CTL_ELEM_ID_NAME_MAXLEN);
        let nm = String::from_utf8_lossy(&id.name[..nm_end]);

        if id.iface == SNDRV_CTL_ELEM_IFACE_PCM && nm == "ELD" && (id.device as usize) < 64 {
            let mut info: SndCtlElemInfo = unsafe { mem::zeroed() };
            info.id = *id;
            if unsafe {
                ioctl_raw(fd, SNDRV_CTL_IOCTL_ELEM_INFO, (&mut info as *mut SndCtlElemInfo).cast())
            } == 0
                && (info.tp as u32) == SNDRV_CTL_ELEM_TYPE_BYTES
                && info.count >= 20
                && info.count <= 512
            {
                let mut ev = SndCtlElemValue::zeroed_for(*id);
                if unsafe {
                    ioctl_raw(
                        fd,
                        SNDRV_CTL_IOCTL_ELEM_READ,
                        (&mut ev as *mut SndCtlElemValue).cast(),
                    )
                } == 0
                {
                    let mnl = (ev.data[4] & 0x1F) as u32;
                    if mnl > 0 && mnl <= 16 && 20 + mnl <= info.count {
                        let mut mname = Vec::with_capacity(mnl as usize);
                        for k in 0..mnl as usize {
                            let ch = ev.data[20 + k];
                            if (0x20..0x7f).contains(&ch) {
                                mname.push(ch);
                            }
                        }
                        while mname.last() == Some(&b' ') {
                            mname.pop();
                        }
                        if let Ok(s) = core::str::from_utf8(&mname) {
                            copy_str_to_c(&mut cj.hdmi_name[id.device as usize], s);
                        }
                    }
                }
            }
            continue;
        }

        if !nm.contains("Jack") {
            continue;
        }

        let mut info: SndCtlElemInfo = unsafe { mem::zeroed() };
        info.id = *id;
        if unsafe {
            ioctl_raw(fd, SNDRV_CTL_IOCTL_ELEM_INFO, (&mut info as *mut SndCtlElemInfo).cast())
        } < 0
            || (info.tp as u32) != SNDRV_CTL_ELEM_TYPE_BOOLEAN
        {
            continue;
        }

        let mut ev = SndCtlElemValue::zeroed_for(*id);
        if unsafe {
            ioctl_raw(fd, SNDRV_CTL_IOCTL_ELEM_READ, (&mut ev as *mut SndCtlElemValue).cast())
        } < 0
        {
            continue;
        }
        let on = ev.get_bool(0);

        if let Some(pos) = nm.find("pcm=") {
            if nm.contains("HDMI") || nm.contains("DP") || nm.contains("DisplayPort") {
                let digits = &nm[pos + 4..];
                let d_end = digits
                    .bytes()
                    .position(|b| !b.is_ascii_digit())
                    .unwrap_or(digits.len());
                if let Ok(pdev) = digits[..d_end].parse::<usize>() {
                    if pdev < 64 {
                        if cj.hdmi_jack[pdev] < 0 {
                            cj.hdmi_jack[pdev] = i32::from(on);
                        } else if on {
                            cj.hdmi_jack[pdev] = 1;
                        }
                    }
                }
                continue;
            }
        }

        let is_in = ci_contains(&nm, "Mic")
            || ci_contains(&nm, "Input")
            || ci_contains(&nm, "Line In")
            || ci_contains(&nm, "Line-In")
            || ci_contains(&nm, "Capture")
            || ci_contains(&nm, "Aux")
            || ci_contains(&nm, "CD");
        if is_in {
            cj.in_jack_count += 1;
            if on {
                cj.in_jack_on += 1;
            }
            let idx = id.index as usize;
            if idx < 16 {
                if cj.in_jack_by_idx[idx] < 0 {
                    cj.in_jack_by_idx[idx] = i32::from(on);
                } else if on {
                    cj.in_jack_by_idx[idx] = 1;
                }
            }
        } else {
            cj.out_jack_count += 1;
            if on {
                cj.out_jack_on += 1;
            }
            if ci_contains(&nm, "Speaker") {
                cj.speaker_jack = i32::from(cj.speaker_jack == 1 || on);
            }
            if ci_contains(&nm, "Headphone") {
                cj.headphone_jack = i32::from(cj.headphone_jack == 1 || on);
            }
            if ci_contains(&nm, "Line Out")
                || ci_contains(&nm, "Line-Out")
                || ci_contains(&nm, "Front")
            {
                cj.lineout_jack = i32::from(cj.lineout_jack == 1 || on);
            }
        }
    }

    close_raw_fd(fd);
}

fn parse_proc_pcm_line(line: &str) -> Option<(usize, usize, String)> {
    let colon = line.find(':')?;
    let prefix = line[..colon].trim();
    let dash = prefix.find('-')?;
    let c: usize = prefix[..dash].trim().parse().ok()?;
    let d: usize = prefix[dash + 1..].trim().parse().ok()?;
    let rest = &line[colon + 1..];
    let after_spc = rest.trim_start_matches(' ');
    let sub_end = after_spc.find(':').unwrap_or(after_spc.len());
    let pcm_name: String = after_spc[..sub_end].trim_end().chars().take(63).collect();
    Some((c, d, pcm_name))
}

pub fn collect_alsa_topology() -> AlsaTopology {
    let mut top = AlsaTopology::default();
    if let Ok(contents) = fs::read("/proc/asound/pcm") {
        top.pcm_hash = fnv1a_step(top.pcm_hash, &contents);
        if let Ok(text) = core::str::from_utf8(&contents) {
            for line in text.lines() {
                let Some((c, _d, pcm_name)) = parse_proc_pcm_line(line) else {
                    continue;
                };
                if c >= MAX_ALSA_CARDS_SCAN {
                    continue;
                }
                top.has_card[c] = true;
                if line.contains("playback")
                    && !ci_contains(&pcm_name, "HDMI")
                    && !ci_contains(&pcm_name, "DisplayPort")
                {
                    top.pb_nonhdmi_cnt[c] += 1;
                }
                if line.contains("capture") {
                    top.cap_cnt[c] += 1;
                }
            }
        }
    }
    for c in 0..MAX_ALSA_CARDS_SCAN {
        if top.has_card[c] {
            probe_card_jacks(c, &mut top.jacks[c]);
        }
    }
    top
}

pub fn is_alsa_pcm_plugged(
    top: &AlsaTopology,
    card_is_usb: &[bool; MAX_ALSA_CARDS_SCAN],
    c: usize,
    d: usize,
    is_capture: bool,
    pcm_name: &str,
) -> bool {
    if c >= MAX_ALSA_CARDS_SCAN {
        return true;
    }
    let cj = &top.jacks[c];
    if !is_capture {
        let is_hdmi = d < 64
            && (cj.hdmi_jack[d] != -1
                || ci_contains(pcm_name, "HDMI")
                || ci_contains(pcm_name, "DisplayPort"));
        if is_hdmi {
            if d < 64 && cj.hdmi_jack[d] != -1 {
                return cj.hdmi_jack[d] != 0;
            }
            let (eld_on, has_eld_files) = card_eld_any_present(c);
            return if has_eld_files { eld_on } else { true };
        }
        if !cj.checked || cj.out_jack_count == 0 {
            return true;
        }
        if cj.out_jack_on == 0 {
            return false;
        }
        if card_is_usb[c]
            && top.pb_nonhdmi_cnt[c] >= 2
            && cj.speaker_jack != -1
            && cj.headphone_jack != -1
        {
            if d == 0 {
                return cj.speaker_jack != 0 || cj.lineout_jack == 1;
            }
            if d == 1 {
                return cj.headphone_jack != 0;
            }
            return false;
        }
        return true;
    }

    if !cj.checked || cj.in_jack_count == 0 {
        return true;
    }
    if cj.in_jack_on == 0 {
        return false;
    }
    if top.cap_cnt[c] > 1 && d < 16 && cj.in_jack_by_idx[d] != -1 {
        return cj.in_jack_by_idx[d] != 0;
    }
    if top.cap_cnt[c] > 1 && cj.in_jack_by_idx[0] != -1 && d >= 2 {
        return false;
    }
    true
}

pub fn alsa_hw_fingerprint_seed() -> u64 {
    let top = collect_alsa_topology();
    let mut h = top.pcm_hash;
    for (c, cj) in top.jacks.iter().enumerate() {
        if !top.has_card[c] {
            continue;
        }
        h = fnv1a_step(h, &(c as u32).to_ne_bytes());
        for &j in &cj.hdmi_jack {
            h = fnv1a_step(h, &j.to_ne_bytes());
        }
        for name in &cj.hdmi_name {
            h = fnv1a_step(h, name);
        }
        for &v in &[
            cj.out_jack_count,
            cj.out_jack_on,
            cj.speaker_jack,
            cj.headphone_jack,
            cj.lineout_jack,
            cj.in_jack_count,
            cj.in_jack_on,
        ] {
            h = fnv1a_step(h, &v.to_ne_bytes());
        }
        for &j in &cj.in_jack_by_idx {
            h = fnv1a_step(h, &j.to_ne_bytes());
        }
    }
    h
}

pub fn append_alsa_devices(
    out_list: &mut LasapiDeviceList,
    active_pb_id: &str,
    active_cap_id: &str,
    card_names: &[String; MAX_ALSA_CARDS_SCAN],
    card_is_usb: &[bool; MAX_ALSA_CARDS_SCAN],
) {
    let top = collect_alsa_topology();
    let Ok(pcm_text) = fs::read_to_string("/proc/asound/pcm") else {
        return;
    };
    for line in pcm_text.lines() {
        if (out_list.num_devices as usize) >= LASAPI_MAX_DEVICES {
            break;
        }
        let Some((c, d, pcm_name)) = parse_proc_pcm_line(line) else {
            continue;
        };
        if c >= MAX_ALSA_CARDS_SCAN {
            continue;
        }
        for is_cap in [false, true] {
            if (out_list.num_devices as usize) >= LASAPI_MAX_DEVICES {
                break;
            }
            let kw = if is_cap { "capture" } else { "playback" };
            if !line.contains(kw) {
                continue;
            }
            let devnode = format!("/dev/snd/pcmC{c}D{d}{}", if is_cap { 'c' } else { 'p' });
            if !Path::new(&devnode).exists() {
                continue;
            }
            if !is_alsa_pcm_plugged(&top, card_is_usb, c, d, is_cap, &pcm_name) {
                continue;
            }
            let id_str = format!("hw:{c},{d}");
            let hdmi_disp = if !is_cap && d < 64 {
                c_str_to_string(&top.jacks[c].hdmi_name[d])
            } else {
                String::new()
            };
            let disp_sub = if hdmi_disp.is_empty() {
                pcm_name.as_str()
            } else {
                hdmi_disp.as_str()
            };
            let disp_name = if !card_names[c].is_empty() {
                let left: String = card_names[c].chars().take(36).collect();
                let right: String = disp_sub.chars().take(23).collect();
                format!("{left} - {right}")
            } else {
                let left: String = disp_sub.chars().take(40).collect();
                let right: String = id_str.chars().take(19).collect();
                format!("{left} ({right})")
            };

            let idx = out_list.num_devices as usize;
            out_list.num_devices += 1;
            let mut e: LasapiDeviceEntry = unsafe { mem::zeroed() };
            copy_str_to_c(&mut e.id, &id_str);
            copy_str_to_c(&mut e.name, &disp_name);
            copy_str_to_c(&mut e.driver, "alsa-kernel");
            e.is_capture = u32::from(is_cap);
            e.is_bluetooth = 0;
            e.card_index = c as u32;
            e.device_index = d as u32;
            e.min_rate = 44_100;
            e.max_rate = 192_000;
            let act_id = if is_cap { active_cap_id } else { active_pb_id };
            e.is_active = u32::from(act_id == id_str);
            out_list.devices[idx] = e;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn try_configure_capture_pcm(
    fd: c_int,
    fmt: i32,
    rate: u32,
    ch_min: u32,
    ch_max: u32,
    per_min: u32,
    per_max: u32,
    buf_min: u32,
    buf_max: u32,
) -> Option<(u32, u32, u32, u32)> {
    let mut params: SndPcmHwParams = unsafe { mem::zeroed() };
    hw_params_any(&mut params);
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_PCM_IOCTL_HW_REFINE,
            (&mut params as *mut SndPcmHwParams).cast(),
        )
    } < 0
    {
        return None;
    }
    hw_param_set_mask(
        &mut params,
        SNDRV_PCM_HW_PARAM_ACCESS,
        SNDRV_PCM_ACCESS_RW_INTERLEAVED,
    );
    hw_param_set_mask(&mut params, SNDRV_PCM_HW_PARAM_FORMAT, fmt as u32);
    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_CHANNELS, ch_min, ch_max);
    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_RATE, rate, rate);
    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, per_min, per_max);
    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_BUFFER_SIZE, buf_min, buf_max);
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_PCM_IOCTL_HW_PARAMS,
            (&mut params as *mut SndPcmHwParams).cast(),
        )
    } < 0
    {
        return None;
    }
    Some((
        params.intervals[SNDRV_PCM_HW_PARAM_RATE - 8].min,
        params.intervals[SNDRV_PCM_HW_PARAM_CHANNELS - 8].min,
        params.intervals[SNDRV_PCM_HW_PARAM_PERIOD_SIZE - 8].min,
        params.intervals[SNDRV_PCM_HW_PARAM_BUFFER_SIZE - 8].min,
    ))
}

fn query_capture_min_channels(fd: c_int, fmt: i32, rate: u32) -> Option<u32> {
    let mut params: SndPcmHwParams = unsafe { mem::zeroed() };
    hw_params_any(&mut params);
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_PCM_IOCTL_HW_REFINE,
            (&mut params as *mut SndPcmHwParams).cast(),
        )
    } < 0
    {
        return None;
    }
    hw_param_set_mask(
        &mut params,
        SNDRV_PCM_HW_PARAM_ACCESS,
        SNDRV_PCM_ACCESS_RW_INTERLEAVED,
    );
    hw_param_set_mask(&mut params, SNDRV_PCM_HW_PARAM_FORMAT, fmt as u32);
    hw_param_set_minmax(&mut params, SNDRV_PCM_HW_PARAM_RATE, rate, rate);
    hw_param_set_minmax(
        &mut params,
        SNDRV_PCM_HW_PARAM_CHANNELS,
        1,
        LASAPI_MAX_CAP_CHANNELS,
    );
    if unsafe {
        ioctl_raw(
            fd,
            SNDRV_PCM_IOCTL_HW_REFINE,
            (&mut params as *mut SndPcmHwParams).cast(),
        )
    } < 0
    {
        return None;
    }
    let cmin = params.intervals[SNDRV_PCM_HW_PARAM_CHANNELS - 8].min;
    if (1..=LASAPI_MAX_CAP_CHANNELS).contains(&cmin) {
        Some(cmin)
    } else {
        None
    }
}

pub fn open_kernel_capture_pcm(dev_path: &str, req_rate: u32) -> Option<(c_int, u32, u32, i32)> {
    let fd = open_raw_flags(dev_path, O_RDWR | O_CLOEXEC);
    if fd < 0 {
        return None;
    }

    let fmts = [
        SNDRV_PCM_FORMAT_S32_LE,
        SNDRV_PCM_FORMAT_S24_3LE,
        SNDRV_PCM_FORMAT_S16_LE,
    ];
    let chs = [2u32, 1, 4, 6, 8, 10, 12, 16, 18];
    let rates = [req_rate, 48_000, 44_100, 96_000, 192_000, 32_000, 16_000];

    let mut chosen: Option<(i32, u32, u32, u32, u32)> = None;
    'search: for (r_idx, &rate) in rates.iter().enumerate() {
        if rate == 0 || (r_idx > 0 && rate == req_rate) {
            continue;
        }
        for &fmt in &fmts {
            for &ch in &chs {
                if let Some((ar, ac, ap, ab)) =
                    try_configure_capture_pcm(fd, fmt, rate, ch, ch, 64, 256, 1024, 8192)
                        .or_else(|| {
                            try_configure_capture_pcm(fd, fmt, rate, ch, ch, 64, 2048, 512, 8192)
                        })
                {
                    chosen = Some((fmt, ar, ac, ap, ab));
                    break 'search;
                }
            }
            if let Some(min_ch) = query_capture_min_channels(fd, fmt, rate) {
                if let Some((ar, ac, ap, ab)) =
                    try_configure_capture_pcm(fd, fmt, rate, min_ch, min_ch, 64, 256, 1024, 8192)
                        .or_else(|| {
                            try_configure_capture_pcm(
                                fd, fmt, rate, min_ch, min_ch, 64, 2048, 512, 8192,
                            )
                        })
                {
                    chosen = Some((fmt, ar, ac, ap, ab));
                    break 'search;
                }
            }
        }
    }

    let Some((chosen_fmt, actual_rate, actual_ch, actual_per, actual_buf)) = chosen else {
        close_raw_fd(fd);
        return None;
    };

    let mut sw: SndPcmSwParams = unsafe { mem::zeroed() };
    sw.tstamp_mode = SNDRV_PCM_TSTAMP_ENABLE;
    sw.period_step = 1;
    sw.avail_min = if actual_per > 0 { u64::from(actual_per) } else { 256 };
    sw.start_threshold = 1;
    sw.stop_threshold = if actual_buf > 0 { u64::from(actual_buf) } else { 2048 };
    sw.xfer_align = 1;
    unsafe {
        let _ = ioctl_raw(
            fd,
            SNDRV_PCM_IOCTL_SW_PARAMS,
            (&mut sw as *mut SndPcmSwParams).cast(),
        );
        if ioctl_raw(fd, SNDRV_PCM_IOCTL_PREPARE, core::ptr::null_mut()) < 0 {
            close_raw_fd(fd);
            return None;
        }
    }

    let out_rate = if actual_rate > 0 { actual_rate } else { req_rate };
    let out_ch = if (1..=LASAPI_MAX_CAP_CHANNELS).contains(&actual_ch) {
        actual_ch
    } else {
        2
    };
    Some((fd, out_rate, out_ch, chosen_fmt))
}

#[inline]
fn convert_alsa_sample_to_float(raw: &[u8], idx: usize, fmt: i32) -> f32 {
    if fmt == SNDRV_PCM_FORMAT_S32_LE {
        let off = idx * 4;
        if off + 4 <= raw.len() {
            let v = i32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]]);
            return (v as f32) * (1.0 / 2_147_483_648.0);
        }
        0.0
    } else if fmt == SNDRV_PCM_FORMAT_S24_3LE {
        let off = idx * 3;
        if off + 3 <= raw.len() {
            let mut v = (u32::from(raw[off])
                | (u32::from(raw[off + 1]) << 8)
                | (u32::from(raw[off + 2]) << 16)) as i32;
            if (v & 0x0080_0000) != 0 {
                v |= -16_777_216i32;
            }
            return (v as f32) * (1.0 / 8_388_608.0);
        }
        0.0
    } else {
        let off = idx * 2;
        if off + 2 <= raw.len() {
            let v = i16::from_le_bytes([raw[off], raw[off + 1]]);
            return (v as f32) * (1.0 / 32_768.0);
        }
        0.0
    }
}

#[inline]
fn cap_snap_measured_rate(actual_hz: f64) -> u32 {
    const STD: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
    for &sr in &STD {
        let r = sr as f64;
        if (actual_hz - r).abs() <= r * 0.035 {
            return sr;
        }
    }
    if (8_000.0..=384_000.0).contains(&actual_hz) {
        (actual_hz + 0.5) as u32
    } else {
        48_000
    }
}

pub fn prime_usb_capture_clock(dev_path: &str, target_rate: u32) -> bool {
    if dev_path.is_empty() {
        return false;
    }
    let alt_rate = if target_rate == 96_000 { 48_000 } else { 96_000 };
    let open_res = open_kernel_capture_pcm(dev_path, alt_rate).or_else(|| {
        if alt_rate != 48_000 {
            open_kernel_capture_pcm(dev_path, 48_000)
        } else {
            None
        }
    });
    let Some((alt_fd, _got_rate, got_ch, _got_fmt)) = open_res else {
        return false;
    };
    let ch = if (1..=LASAPI_MAX_CAP_CHANNELS).contains(&got_ch) {
        got_ch
    } else {
        2
    };
    let mut dummy = vec![0u8; 128 * (LASAPI_MAX_CAP_CHANNELS as usize) * 4];
    unsafe {
        let _ = ioctl_raw(alt_fd, SNDRV_PCM_IOCTL_START, core::ptr::null_mut());
    }
    let mut pfd = [PollFd {
        fd: alt_fd,
        events: POLLIN | POLLERR | POLLHUP,
        revents: 0,
    }];
    if do_poll(&mut pfd, 100) > 0 && (pfd[0].revents & POLLIN) != 0 {
        let mut xf = SndXferi {
            result: 0,
            buf: dummy.as_mut_ptr().cast::<c_void>(),
            frames: if ch <= LASAPI_MAX_CAP_CHANNELS { 64 } else { 16 },
        };
        unsafe {
            let _ = ioctl_raw(
                alt_fd,
                SNDRV_PCM_IOCTL_READI_FRAMES,
                (&mut xf as *mut SndXferi).cast(),
            );
        }
    }
    close_raw_fd(alt_fd);
    thread::sleep(Duration::from_millis(25));
    true
}

/// Shared capture state cleanly owned by Rust `BluezRuntime` without exposing C mutex/ring ABI.
pub struct CapState {
    pub has_capture_hw: AtomicBool,
    pub cap_is_usb: AtomicBool,
    pub cap_pcm_fd: AtomicI32,
    pub cap_hw_rate: AtomicU32,
    pub cap_hw_channels: AtomicU32,
    pub cap_hw_format: AtomicI32,
    pub cap_ring_rate: AtomicU32,
    pub cap_measured_rate: AtomicU32,
    pub cap_kick_count: AtomicU32,
    pub desktop_mic_muted: AtomicBool,
    pub desktop_mic_volume_pct: AtomicU32,
    pub desktop_mic_gain_ppm: AtomicU32,
    pub cap_hw_write_frame: AtomicU64,
    pub cap_thread_running: AtomicBool,
    pub cap_ring: Mutex<Box<[f32]>>,
    pub cap_thread: Mutex<Option<JoinHandle<()>>>,
    pub meta: Mutex<CapMeta>,
}

#[derive(Clone, Default)]
pub struct CapMeta {
    pub cap_device_id: String,
    pub cap_pcm_path: String,
    pub capture_device_alias: String,
    pub sco_cap_device_id: String,
}

impl Default for CapState {
    fn default() -> Self {
        Self::new()
    }
}

impl CapState {
    pub fn new() -> Self {
        Self {
            has_capture_hw: AtomicBool::new(false),
            cap_is_usb: AtomicBool::new(false),
            cap_pcm_fd: AtomicI32::new(-1),
            cap_hw_rate: AtomicU32::new(0),
            cap_hw_channels: AtomicU32::new(0),
            cap_hw_format: AtomicI32::new(0),
            cap_ring_rate: AtomicU32::new(0),
            cap_measured_rate: AtomicU32::new(0),
            cap_kick_count: AtomicU32::new(0),
            desktop_mic_muted: AtomicBool::new(false),
            desktop_mic_volume_pct: AtomicU32::new(100),
            desktop_mic_gain_ppm: AtomicU32::new(1_000_000),
            cap_hw_write_frame: AtomicU64::new(0),
            cap_thread_running: AtomicBool::new(false),
            cap_ring: Mutex::new(vec![0.0f32; CAP_RING_FRAMES * 2].into_boxed_slice()),
            cap_thread: Mutex::new(None),
            meta: Mutex::new(CapMeta::default()),
        }
    }

    pub fn write_ring_samples(&self, interleaved_stereo: &[f32], frames: usize) {
        let mut ring = self.cap_ring.lock().unwrap();
        let wr = self.cap_hw_write_frame.load(Ordering::Relaxed);
        for i in 0..frames {
            let idx = ((wr.wrapping_add(i as u64)) as usize) & CAP_RING_MASK;
            ring[idx * 2] = interleaved_stereo[i * 2];
            ring[idx * 2 + 1] = interleaved_stereo[i * 2 + 1];
        }
        self.cap_hw_write_frame
            .store(wr.wrapping_add(frames as u64), Ordering::Release);
    }
}

pub fn stop_capture_thread(cap: &Arc<CapState>) {
    cap.cap_thread_running.store(false, Ordering::Release);
    if let Some(jh) = cap.cap_thread.lock().unwrap().take() {
        let _ = jh.join();
    }
    let old_fd = cap.cap_pcm_fd.swap(-1, Ordering::AcqRel);
    close_raw_fd(old_fd);
    cap.has_capture_hw.store(false, Ordering::Release);
}

fn kick_usb_capture_clock(cap: &Arc<CapState>, pcm_path: &str, target_rate: u32) -> bool {
    if pcm_path.is_empty() {
        return false;
    }
    let old = cap.cap_pcm_fd.swap(-1, Ordering::AcqRel);
    close_raw_fd(old);
    prime_usb_capture_clock(pcm_path, target_rate);
    if !cap.cap_thread_running.load(Ordering::Acquire) {
        return false;
    }
    let Some((fd, new_rate, new_ch, new_fmt)) = open_kernel_capture_pcm(pcm_path, target_rate)
    else {
        return false;
    };
    cap.cap_pcm_fd.store(fd, Ordering::Release);
    cap.cap_hw_rate.store(new_rate, Ordering::Relaxed);
    cap.cap_hw_channels.store(new_ch, Ordering::Relaxed);
    cap.cap_hw_format.store(new_fmt, Ordering::Relaxed);
    true
}

pub fn spawn_capture_thread(cap: Arc<CapState>) {
    let cap_bg = Arc::clone(&cap);
    let jh = thread::spawn(move || {
        let cap = cap_bg;
        enable_realtime(62, -15);
        const CHUNK: u32 = 128;
        let mut raw_buf = vec![0u8; 256 * (LASAPI_MAX_CAP_CHANNELS as usize) * 4];
        let mut src_float = vec![0.0f32; 256 * 2];
        let mut smooth_gain = 1.0f32;

        let mut eff_in_rate_override = 0u32;
        let mut kick_attempts = 0u32;
        let mut mismatch_streak = 0u32;
        let mut rate_locked_fallback = false;
        let mut warmup_until_ns = mono_ns().saturating_add(150_000_000);
        let mut meas_start_ns = 0u64;
        let mut meas_frames = 0u64;
        let mut meas_ticks = 0u32;

        let pcm_path = cap.meta.lock().unwrap().cap_pcm_path.clone();

        while cap.cap_thread_running.load(Ordering::Acquire) {
            let fd = cap.cap_pcm_fd.load(Ordering::Acquire);
            if fd < 0 {
                meas_start_ns = 0;
                meas_frames = 0;
                mismatch_streak = 0;
                thread::sleep(Duration::from_millis(20));
                continue;
            }

            let mut xf = SndXferi {
                result: 0,
                buf: raw_buf.as_mut_ptr().cast::<c_void>(),
                frames: u64::from(CHUNK),
            };
            let rc = unsafe {
                ioctl_raw(
                    fd,
                    SNDRV_PCM_IOCTL_READI_FRAMES,
                    (&mut xf as *mut SndXferi).cast(),
                )
            };
            if rc < 0 {
                let err = last_errno();
                if err == ERRNO_EPIPE || err == ERRNO_ESTRPIPE {
                    unsafe {
                        let _ = ioctl_raw(fd, SNDRV_PCM_IOCTL_PREPARE, core::ptr::null_mut());
                    }
                    meas_start_ns = 0;
                    meas_frames = 0;
                    mismatch_streak = 0;
                    continue;
                }
                if err == ERRNO_EINTR || err == ERRNO_EAGAIN {
                    meas_start_ns = 0;
                    meas_frames = 0;
                    mismatch_streak = 0;
                    thread::sleep(Duration::from_micros(500));
                    continue;
                }
                let old = cap.cap_pcm_fd.swap(-1, Ordering::AcqRel);
                close_raw_fd(old);
                cap.has_capture_hw.store(false, Ordering::Release);
                {
                    let mut m = cap.meta.lock().unwrap();
                    m.cap_device_id = "none".to_string();
                    m.capture_device_alias = "Capture Disconnected".to_string();
                }
                break;
            }

            if xf.result <= 0 {
                continue;
            }
            let got = (xf.result as u32).min(CHUNK);

            let nominal_rate = match cap.cap_hw_rate.load(Ordering::Relaxed) {
                0 => 48_000,
                r => r,
            };

            meas_frames += u64::from(got);
            meas_ticks = meas_ticks.wrapping_add(1);
            if (meas_ticks & 15) == 0 {
                let now_ns = mono_ns();
                if now_ns >= warmup_until_ns {
                    if meas_start_ns == 0 {
                        meas_start_ns = now_ns;
                        meas_frames = 0;
                    } else if now_ns.saturating_sub(meas_start_ns) >= 350_000_000 {
                        let dt_ns = now_ns - meas_start_ns;
                        let actual_hz = (meas_frames as f64) * 1e9 / (dt_ns as f64);
                        cap.cap_measured_rate
                            .store((actual_hz + 0.5) as u32, Ordering::Relaxed);
                        meas_start_ns = now_ns;
                        meas_frames = 0;
                        if cap.cap_is_usb.load(Ordering::Relaxed) && !rate_locked_fallback {
                            if actual_hz < (nominal_rate as f64) * 0.78
                                || actual_hz > (nominal_rate as f64) * 1.25
                            {
                                mismatch_streak += 1;
                                if mismatch_streak >= 2 {
                                    if kick_attempts < 2 {
                                        kick_attempts += 1;
                                        cap.cap_kick_count.fetch_add(1, Ordering::Relaxed);
                                        eprintln!(
                                            "[LASAPI-Capture] Clock rate mismatch on {pcm_path} (nominal={nominal_rate}Hz actual={actual_hz:.0}Hz) -> cycling UAC2 clock (attempt {kick_attempts}/2)"
                                        );
                                        dbg_emit(
                                            LASAPI_DBG_ALSA,
                                            LASAPI_DBG_WARN,
                                            0,
                                            0,
                                            "ALSA_CAP_CLK_KICK",
                                            &pcm_path,
                                            &format!(
                                                "nominal={nominal_rate}Hz actual={actual_hz:.0}Hz attempt={kick_attempts}"
                                            ),
                                        );
                                        if kick_usb_capture_clock(&cap, &pcm_path, nominal_rate) {
                                            meas_start_ns = 0;
                                            meas_frames = 0;
                                            mismatch_streak = 0;
                                            warmup_until_ns = mono_ns().saturating_add(150_000_000);
                                            continue;
                                        }
                                    }
                                    let snapped = cap_snap_measured_rate(actual_hz);
                                    eff_in_rate_override = snapped;
                                    cap.cap_hw_rate.store(snapped, Ordering::Relaxed);
                                    rate_locked_fallback = true;
                                    mismatch_streak = 0;
                                    eprintln!(
                                        "[LASAPI-Capture] Using measured hardware clock {snapped}Hz (actual={actual_hz:.0}Hz) on {pcm_path} for artifact-free capture"
                                    );
                                    dbg_emit(
                                        LASAPI_DBG_ALSA,
                                        LASAPI_DBG_WARN,
                                        0,
                                        0,
                                        "ALSA_CAP_CLK_FALLBACK",
                                        &pcm_path,
                                        &format!("snapped={snapped}Hz actual={actual_hz:.0}Hz"),
                                    );
                                }
                            } else {
                                mismatch_streak = 0;
                            }
                        }
                    }
                } else {
                    meas_frames = 0;
                }
            }

            let mic_muted = cap.desktop_mic_muted.load(Ordering::Relaxed);
            let ppm = cap.desktop_mic_gain_ppm.load(Ordering::Relaxed);
            let target_gain = if mic_muted {
                0.0
            } else {
                (ppm as f32) * (1.0 / 1_000_000.0)
            };
            let g_step = (target_gain - smooth_gain) / (got as f32);
            let ch = cap
                .cap_hw_channels
                .load(Ordering::Relaxed)
                .clamp(1, LASAPI_MAX_CAP_CHANNELS) as usize;
            let cap_fmt = cap.cap_hw_format.load(Ordering::Relaxed);

            for i in 0..got as usize {
                smooth_gain += g_step;
                let l = (convert_alsa_sample_to_float(&raw_buf, i * ch, cap_fmt) * smooth_gain)
                    .clamp(-1.0, 1.0);
                let r = if ch >= 2 {
                    (convert_alsa_sample_to_float(&raw_buf, i * ch + 1, cap_fmt) * smooth_gain)
                        .clamp(-1.0, 1.0)
                } else {
                    l
                };
                src_float[i * 2] = l;
                src_float[i * 2 + 1] = r;
            }
            smooth_gain = target_gain;

            let in_rate = if eff_in_rate_override > 0 {
                eff_in_rate_override
            } else {
                nominal_rate
            };
            cap.cap_ring_rate.store(in_rate, Ordering::Release);
            cap.write_ring_samples(&src_float, got as usize);
        }
    });
    *cap.cap_thread.lock().unwrap() = Some(jh);
}

#[allow(clippy::too_many_arguments)]
pub fn open_alsa_capture(
    cap: &Arc<CapState>,
    eng_rate: u32,
    preferred_cap_id: &str,
    capture_sample_rate: u32,
    card_names: &[String; MAX_ALSA_CARDS_SCAN],
    card_is_usb: &[bool; MAX_ALSA_CARDS_SCAN],
) -> i32 {
    let req_cap_rate = if capture_sample_rate > 0 {
        capture_sample_rate
    } else {
        eng_rate
    };
    let top = collect_alsa_topology();

    #[derive(Clone)]
    struct CapCand {
        card: usize,
        score: i32,
        id: String,
        path: String,
        alias: String,
    }

    let mut cands: Vec<CapCand> = Vec::new();
    if let Ok(pcm_text) = fs::read_to_string("/proc/asound/pcm") {
        for line in pcm_text.lines() {
            if cands.len() >= 32 || !line.contains("capture") {
                continue;
            }
            let Some((c, d, pcm_name)) = parse_proc_pcm_line(line) else {
                continue;
            };
            if c >= MAX_ALSA_CARDS_SCAN {
                continue;
            }
            let dev_path = format!("/dev/snd/pcmC{c}D{d}c");
            if !Path::new(&dev_path).exists() {
                continue;
            }
            if !is_alsa_pcm_plugged(&top, card_is_usb, c, d, true, &pcm_name) {
                continue;
            }
            let id_tmp = format!("hw:{c},{d}");
            let alias = if !card_names[c].is_empty() {
                card_names[c].clone()
            } else {
                let p_t: String = pcm_name.chars().take(40).collect();
                let id_t: String = id_tmp.chars().take(19).collect();
                format!("{p_t} ({id_t})")
            };

            let mut sc = 10i32;
            if !preferred_cap_id.is_empty()
                && preferred_cap_id != "auto"
                && preferred_cap_id == id_tmp
            {
                sc = 10_000;
            } else {
                if card_is_usb[c] {
                    sc += 200;
                }
                if ci_contains(&alias, "Scarlett")
                    || ci_contains(&alias, "Blue")
                    || ci_contains(&alias, "Shure")
                    || ci_contains(&alias, "Rode")
                    || ci_contains(&alias, "Audient")
                {
                    sc += 300;
                }
                if d == 0 {
                    sc += 20;
                }
                sc -= (c + d) as i32;
            }

            cands.push(CapCand {
                card: c,
                score: sc,
                id: id_tmp,
                path: dev_path,
                alias,
            });
        }
    }

    cands.sort_by_key(|a| core::cmp::Reverse(a.score));

    for cand in &cands {
        let is_usb = card_is_usb[cand.card];
        if is_usb && req_cap_rate > 48_000 {
            prime_usb_capture_clock(&cand.path, req_cap_rate);
        }
        if let Some((fd, actual_rate, actual_ch, actual_fmt)) =
            open_kernel_capture_pcm(&cand.path, req_cap_rate)
        {
            cap.cap_pcm_fd.store(fd, Ordering::Release);
            {
                let mut m = cap.meta.lock().unwrap();
                m.cap_device_id = cand.id.clone();
                m.cap_pcm_path = cand.path.clone();
                m.capture_device_alias = cand.alias.clone();
            }
            cap.cap_hw_rate.store(actual_rate, Ordering::Relaxed);
            cap.cap_hw_channels.store(actual_ch, Ordering::Relaxed);
            cap.cap_hw_format.store(actual_fmt, Ordering::Relaxed);
            cap.cap_is_usb.store(is_usb, Ordering::Relaxed);
            cap.cap_ring_rate.store(actual_rate, Ordering::Release);
            cap.cap_measured_rate.store(0, Ordering::Relaxed);
            cap.cap_kick_count.store(0, Ordering::Relaxed);
            cap.has_capture_hw.store(true, Ordering::Release);
            cap.cap_hw_write_frame.store(0, Ordering::Release);
            cap.cap_thread_running.store(true, Ordering::Release);
            spawn_capture_thread(Arc::clone(cap));
            eprintln!(
                "[LASAPI-Capture] Direct Kernel ALSA Capture active: {} [{} @ {}Hz {}ch]",
                cand.alias, cand.path, actual_rate, actual_ch
            );
            dbg_emit(
                LASAPI_DBG_ALSA,
                LASAPI_DBG_INFO,
                0,
                0,
                "ALSA_CAP_OPEN",
                &cand.path,
                &format!(
                    "{} ({}) @ {}Hz {}ch fmt={}",
                    cand.alias, cand.id, actual_rate, actual_ch, actual_fmt
                ),
            );
            return 0;
        }
    }

    {
        let mut m = cap.meta.lock().unwrap();
        m.cap_device_id = "none".to_string();
        m.capture_device_alias = "No Hardware Mic".to_string();
    }
    dbg_emit(
        LASAPI_DBG_ALSA,
        LASAPI_DBG_WARN,
        0,
        0,
        "ALSA_CAP_NONE",
        if preferred_cap_id.is_empty() {
            "auto"
        } else {
            preferred_cap_id
        },
        &format!(
            "No plugged ALSA capture PCM available ({} candidates tried)",
            cands.len()
        ),
    );
    -1
}

#[inline]
fn cap_catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

#[derive(Clone, Copy, Default)]
struct CapDriftState {
    cursor_key: usize,
    frac: f64,
    avg_avail: f64,
    native_phase: f64,
    last_ring_rate: u32,
    last_l: f32,
    last_r: f32,
    in_underrun: bool,
    primed: bool,
}

thread_local! {
    static TLS_CAP_STATES: RefCell<[CapDriftState; 4]> = const { RefCell::new([CapDriftState {
        cursor_key: 0,
        frac: 0.0,
        avg_avail: 0.0,
        native_phase: 0.0,
        last_ring_rate: 0,
        last_l: 0.0,
        last_r: 0.0,
        in_underrun: false,
        primed: false,
    }; 4]) };
}

/// # Safety
///
/// `out_stereo_float` must point to writable storage for at least
/// `max_frames * 2` `f32` samples, and any non-null `out_frames`, `out_rate`, or
/// `io_read_cursor` pointers must be valid for aligned scalar writes.
#[allow(clippy::too_many_arguments)]
pub unsafe fn read_capture_native(
    cap: Option<&CapState>,
    bt_initialized: bool,
    out_stereo_float: *mut f32,
    max_frames: u32,
    out_frames: *mut u32,
    out_rate: *mut u32,
    eng_frames: u32,
    eng_rate: u32,
    io_read_cursor: *mut u64,
) -> i32 {
    if out_stereo_float.is_null() || max_frames == 0 || eng_frames == 0 || eng_rate == 0 {
        return -1;
    }

    let has_cap = cap
        .map(|c| bt_initialized && c.has_capture_hw.load(Ordering::Acquire))
        .unwrap_or(false);
    let mut ring_rate = if let Some(c) = cap {
        if has_cap {
            let rr = c.cap_ring_rate.load(Ordering::Acquire);
            if rr < 8_000 {
                c.cap_hw_rate.load(Ordering::Relaxed)
            } else {
                rr
            }
        } else {
            0
        }
    } else {
        0
    };
    if ring_rate < 8_000 {
        ring_rate = eng_rate;
    }

    let key = io_read_cursor as usize;
    let out_slice =
        unsafe { core::slice::from_raw_parts_mut(out_stereo_float, (max_frames as usize) * 2) };

    TLS_CAP_STATES.with(|tls| {
        let mut states = tls.borrow_mut();
        let slot = if key != 0 {
            if let Some(i) = states.iter().position(|s| s.cursor_key == key) {
                i
            } else {
                states.iter().position(|s| s.cursor_key == 0).unwrap_or(0)
            }
        } else {
            0
        };
        let st = &mut states[slot];
        if key != 0 && st.cursor_key != key {
            *st = CapDriftState {
                cursor_key: key,
                ..CapDriftState::default()
            };
        }

        if st.last_ring_rate != ring_rate {
            st.last_ring_rate = ring_rate;
            st.native_phase = 0.0;
            st.frac = 0.0;
            st.in_underrun = false;
            st.primed = false;
            if !io_read_cursor.is_null() {
                unsafe { *io_read_cursor = 0 };
            }
        }

        let exact_n =
            (f64::from(eng_frames) * f64::from(ring_rate)) / f64::from(eng_rate) + st.native_phase;
        let frames = (exact_n + 1e-9).floor().clamp(1.0, f64::from(max_frames)) as u32;
        st.native_phase = (exact_n - f64::from(frames)).max(0.0);

        if !out_frames.is_null() {
            unsafe { *out_frames = frames };
        }
        if !out_rate.is_null() {
            unsafe { *out_rate = ring_rate };
        }

        let Some(c) = cap else {
            out_slice[..(frames as usize) * 2].fill(0.0);
            return -1;
        };
        if !has_cap {
            out_slice[..(frames as usize) * 2].fill(0.0);
            return -1;
        }

        let min_target = if ring_rate <= 64_000 { 192u32 } else { 512u32 };
        let target_fill = frames.saturating_mul(4).max(min_target);
        let max_fill = target_fill
            .saturating_mul(6)
            .min((CAP_RING_FRAMES as u32) - 1024);

        let wr = c.cap_hw_write_frame.load(Ordering::Acquire);
        let mut rd = if !io_read_cursor.is_null() {
            unsafe { *io_read_cursor }
        } else {
            0
        };

        if rd == 0 {
            if wr < u64::from(target_fill) {
                out_slice[..(frames as usize) * 2].fill(0.0);
                return 0;
            }
            rd = wr - u64::from(target_fill);
            st.frac = 0.0;
            st.avg_avail = f64::from(target_fill);
            st.last_l = 0.0;
            st.last_r = 0.0;
            st.in_underrun = false;
            st.primed = true;
        } else if wr < rd || (wr - rd) > u64::from(max_fill) {
            rd = wr.saturating_sub(u64::from(target_fill));
            st.frac = 0.0;
            st.avg_avail = f64::from(target_fill);
            st.in_underrun = false;
            st.primed = true;
        }

        let avail = wr.saturating_sub(rd);
        if st.in_underrun {
            if avail < u64::from(target_fill) {
                out_slice[..(frames as usize) * 2].fill(0.0);
                if !io_read_cursor.is_null() {
                    unsafe { *io_read_cursor = rd };
                }
                return 0;
            }
            st.in_underrun = false;
            st.frac = 0.0;
            st.avg_avail = avail as f64;
        }

        let ring = c.cap_ring.lock().unwrap();
        if avail < u64::from(frames) {
            let to_copy = avail as u32;
            let mut last_l = st.last_l;
            let mut last_r = st.last_r;
            for i in 0..to_copy as usize {
                let idx = ((rd + i as u64) as usize) & CAP_RING_MASK;
                last_l = ring[idx * 2];
                last_r = ring[idx * 2 + 1];
                out_slice[i * 2] = last_l;
                out_slice[i * 2 + 1] = last_r;
            }
            for j in 0..(frames - to_copy) as usize {
                if j < 16 {
                    let g = ((15 - j) as f32) / 16.0;
                    out_slice[(to_copy as usize + j) * 2] = last_l * g;
                    out_slice[(to_copy as usize + j) * 2 + 1] = last_r * g;
                } else {
                    out_slice[(to_copy as usize + j) * 2] = 0.0;
                    out_slice[(to_copy as usize + j) * 2 + 1] = 0.0;
                }
            }
            st.last_l = 0.0;
            st.last_r = 0.0;
            st.frac = 0.0;
            st.in_underrun = true;
            if !io_read_cursor.is_null() {
                unsafe { *io_read_cursor = rd + u64::from(to_copy) };
            }
            return 0;
        }

        if !st.primed {
            st.avg_avail = avail as f64;
            st.primed = true;
        } else {
            st.avg_avail = st.avg_avail * 0.96 + (avail as f64) * 0.04;
        }

        let err = st.avg_avail - f64::from(target_fill);
        let dead = f64::from(target_fill) * 0.08;
        let step = if err > dead {
            let adj = ((err - dead) / (f64::from(target_fill) * 1.5)).min(0.0025);
            1.0 + adj
        } else if err < -dead {
            let adj = ((-err - dead) / (f64::from(target_fill) * 1.5)).min(0.0025);
            1.0 - adj
        } else {
            1.0
        };

        if step == 1.0 && st.frac.abs() < 1e-9 {
            for i in 0..frames as usize {
                let idx = ((rd + i as u64) as usize) & CAP_RING_MASK;
                st.last_l = ring[idx * 2];
                st.last_r = ring[idx * 2 + 1];
                out_slice[i * 2] = st.last_l;
                out_slice[i * 2 + 1] = st.last_r;
            }
            st.frac = 0.0;
            if !io_read_cursor.is_null() {
                unsafe { *io_read_cursor = rd + u64::from(frames) };
            }
            return 0;
        }

        let mut pos = st.frac;
        for i in 0..frames as usize {
            let i0 = pos as u64;
            if i0 + 1 < avail {
                let alpha = (pos - (i0 as f64)) as f32;
                let im1 = i0.saturating_sub(1);
                let ip1 = i0 + 1;
                let ip2 = if i0 + 2 < avail { i0 + 2 } else { ip1 };
                let km1 = ((rd + im1) as usize) & CAP_RING_MASK;
                let k0 = ((rd + i0) as usize) & CAP_RING_MASK;
                let kp1 = ((rd + ip1) as usize) & CAP_RING_MASK;
                let kp2 = ((rd + ip2) as usize) & CAP_RING_MASK;
                st.last_l = cap_catmull_rom(
                    ring[km1 * 2],
                    ring[k0 * 2],
                    ring[kp1 * 2],
                    ring[kp2 * 2],
                    alpha,
                )
                .clamp(-1.0, 1.0);
                st.last_r = cap_catmull_rom(
                    ring[km1 * 2 + 1],
                    ring[k0 * 2 + 1],
                    ring[kp1 * 2 + 1],
                    ring[kp2 * 2 + 1],
                    alpha,
                )
                .clamp(-1.0, 1.0);
            } else {
                let c0 = avail.saturating_sub(1);
                let idx0 = ((rd + c0) as usize) & CAP_RING_MASK;
                st.last_l = ring[idx0 * 2];
                st.last_r = ring[idx0 * 2 + 1];
            }
            out_slice[i * 2] = st.last_l;
            out_slice[i * 2 + 1] = st.last_r;
            pos += step;
        }

        let mut whole = pos as u64;
        if whole > avail {
            whole = avail;
            pos = whole as f64;
        }
        st.frac = pos - (whole as f64);
        if !io_read_cursor.is_null() {
            unsafe { *io_read_cursor = rd + whole };
        }
        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_topology_and_fnv_seed_work() {
        let h1 = fnv1a_step(14_695_981_039_346_656_037u64, b"hw:0,0");
        assert_ne!(h1, 0);
        let cap = Arc::new(CapState::new());
        let samples = [0.25f32, -0.25f32].repeat(600);
        cap.has_capture_hw.store(true, Ordering::Release);
        cap.cap_hw_rate.store(48_000, Ordering::Relaxed);
        cap.cap_ring_rate.store(48_000, Ordering::Release);
        cap.write_ring_samples(&samples, 600);

        let mut out = [0.0f32; 128 * 2];
        let mut out_f = 0u32;
        let mut out_r = 0u32;
        let mut cursor = 0u64;
        let rc = unsafe {
            read_capture_native(
                Some(&cap),
                true,
                out.as_mut_ptr(),
                128,
                &mut out_f,
                &mut out_r,
                128,
                48_000,
                &mut cursor,
            )
        };
        assert_eq!(rc, 0);
        assert_eq!(out_f, 128);
        assert_eq!(out_r, 48_000);
        assert!((out[0] - 0.25).abs() < 1e-4);
        assert!((out[1] + 0.25).abs() < 1e-4);
    }
}
