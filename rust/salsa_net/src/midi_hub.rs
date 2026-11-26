// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::mem;
use core::ptr;
use core::slice;
use core::sync::atomic::Ordering;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, LasapiMidiEvent, LasapiMidiHub, LasapiShmRing,
    LASAPI_DBG_INFO, LASAPI_DBG_MIDI, LASAPI_DBG_TRACE, LASAPI_MIDI_RING_EVENTS,
    LASAPI_MIDI_RING_MASK,
};
use crate::unix_sys::{
    close_raw_fd, dbg_emit_vals, dl_close, dl_open_any, dl_sym, mono_ns, nb_read_raw,
    open_raw_flags, O_CLOEXEC, O_NONBLOCK, O_RDONLY,
};

type FnSeqOpen = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type FnSeqSetName = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type FnSeqCreatePort = unsafe extern "C" fn(*mut c_void, *const c_char, c_uint, c_uint) -> c_int;
type FnSeqConnectFrom = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int;
type FnSeqNonblock = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type FnSeqClientId = unsafe extern "C" fn(*mut c_void) -> c_int;
type FnSeqClose = unsafe extern "C" fn(*mut c_void) -> c_int;
type FnSeqEventInput = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> c_int;
type FnSeqFreeEvent = unsafe extern "C" fn(*mut c_void) -> c_int;
type FnMidiEventNew = unsafe extern "C" fn(usize, *mut *mut c_void) -> c_int;
type FnMidiEventFree = unsafe extern "C" fn(*mut c_void);
type FnMidiEventNoStatus = unsafe extern "C" fn(*mut c_void, c_int);
type FnMidiEventDecode = unsafe extern "C" fn(*mut c_void, *mut u8, c_long, *const c_void) -> c_long;

#[derive(Default)]
struct AlsaSeqGlobals {
    p_seq_input: Option<FnSeqEventInput>,
    p_seq_free: Option<FnSeqFreeEvent>,
    p_midi_free: Option<FnMidiEventFree>,
    p_midi_dec: Option<FnMidiEventDecode>,
    seq_coder: *mut c_void,
}

unsafe impl Send for AlsaSeqGlobals {}

static ALSA_SEQ_STATE: OnceLock<Mutex<AlsaSeqGlobals>> = OnceLock::new();

fn alsa_state() -> &'static Mutex<AlsaSeqGlobals> {
    ALSA_SEQ_STATE.get_or_init(|| Mutex::new(AlsaSeqGlobals::default()))
}

unsafe fn load_sym<T: Copy>(dl: *mut c_void, name: &str) -> Option<T> {
    let p = unsafe { dl_sym(dl, name) };
    if p.is_null() {
        None
    } else {
        Some(unsafe { mem::transmute_copy::<*mut c_void, T>(&p) })
    }
}

#[inline]
pub fn midi_expected_bytes(status: u8) -> i32 {
    let hi = status & 0xF0;
    if matches!(hi, 0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0) {
        3
    } else if matches!(hi, 0xC0 | 0xD0) {
        2
    } else if status >= 0xF8 {
        1
    } else {
        0
    }
}

#[inline]
pub fn compute_frame_offset(
    ev_ns: u64,
    period_start_ns: u64,
    sample_rate: u32,
    period_frames: u32,
) -> u32 {
    if ev_ns <= period_start_ns {
        return 0;
    }
    let delta_ns = ev_ns - period_start_ns;
    let frame = ((u128::from(delta_ns) * u128::from(sample_rate)) / 1_000_000_000u128) as u64;
    if period_frames == 0 {
        0
    } else if frame >= u64::from(period_frames) {
        period_frames - 1
    } else {
        frame as u32
    }
}

#[inline]
pub fn is_key_press(ev: &LasapiMidiEvent) -> bool {
    ev.size >= 3
        && (ev.data[0] & 0xF0) == 0x90
        && (ev.data[1] & 0x80) == 0
        && (ev.data[2] & 0x80) == 0
        && (ev.data[2] & 0x7F) > 0
}

pub fn parse_proc_asound_cards(contents: &str, card_num: i32) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim_start();
        let Some(sp_or_br) = trimmed.find(|c: char| !c.is_ascii_digit()) else {
            continue;
        };
        let Ok(num) = trimmed[..sp_or_br].parse::<i32>() else {
            continue;
        };
        if num != card_num {
            continue;
        }
        let rest = &trimmed[sp_or_br..];
        let Some(close_br) = rest.find("]: ") else {
            continue;
        };
        let desc = rest[close_br + 3..].trim_end_matches(&['\r', '\n'][..]);
        let clean = desc
            .split_once(" - ")
            .map(|(_, after)| after)
            .unwrap_or(desc);
        return Some(clean.to_string());
    }
    None
}

fn discover_midi_card_name(card_num: i32) -> String {
    if let Ok(text) = fs::read_to_string("/proc/asound/cards") {
        if let Some(name) = parse_proc_asound_cards(&text, card_num) {
            return name;
        }
    }
    format!("Hardware MIDI Card {card_num}")
}

fn feed_raw_byte(
    hub: &mut LasapiMidiHub,
    b: u8,
    ev_ns: u64,
    period_start_ns: u64,
    sample_rate: u32,
    period_frames: u32,
) -> Option<LasapiMidiEvent> {
    if b >= 0xF8 {
        return Some(LasapiMidiEvent {
            frame_offset: compute_frame_offset(ev_ns, period_start_ns, sample_rate, period_frames),
            size: 1,
            port: 0,
            data: [b, 0, 0, 0],
            timestamp_ns: ev_ns,
        });
    }
    if (b & 0x80) != 0 {
        hub.running_status = b;
        hub.msg_expected = midi_expected_bytes(b);
        hub.msg_buf[0] = b;
        hub.msg_idx = 1;
        return None;
    }
    if hub.running_status != 0 && hub.msg_expected > 1 {
        if hub.msg_idx <= 0 {
            hub.msg_buf[0] = hub.running_status;
            hub.msg_idx = 1;
        }
        if (hub.msg_idx as usize) < hub.msg_buf.len() {
            hub.msg_buf[hub.msg_idx as usize] = b;
            hub.msg_idx += 1;
        }
        if hub.msg_idx >= hub.msg_expected {
            let expected = hub.msg_expected.clamp(2, 3) as usize;
            let ev = LasapiMidiEvent {
                frame_offset: compute_frame_offset(
                    ev_ns,
                    period_start_ns,
                    sample_rate,
                    period_frames,
                ),
                size: expected as u8,
                port: 0,
                data: [
                    hub.msg_buf[0],
                    hub.msg_buf[1],
                    if expected > 2 { hub.msg_buf[2] } else { 0 },
                    0,
                ],
                timestamp_ns: ev_ns,
            };
            hub.msg_idx = 0;
            if is_key_press(&ev) {
                hub.total_events.fetch_add(1, Ordering::Relaxed);
            }
            return Some(ev);
        }
    }
    None
}

fn emit_midi_event_log(controller_name: &str, ev: &LasapiMidiEvent) {
    if ev.size == 1 && ev.data[0] >= 0xF8 {
        if ev.data[0] != 0xF8 && ev.data[0] != 0xFE {
            let detail = format!("rt_byte=0x{:02X} off={}", ev.data[0], ev.frame_offset);
            dbg_emit_vals(
                LASAPI_DBG_MIDI,
                LASAPI_DBG_TRACE,
                0,
                0,
                i32::from(ev.data[0]),
                ev.frame_offset as i32,
                0,
                "MIDI_RT",
                controller_name,
                &detail,
            );
        }
    } else {
        let ch = (u32::from(ev.data[0] & 0x0F)) + 1;
        let detail = format!(
            "[{:02X} {:02X} {:02X}] ch={} off={}",
            ev.data[0], ev.data[1], ev.data[2], ch, ev.frame_offset
        );
        dbg_emit_vals(
            LASAPI_DBG_MIDI,
            LASAPI_DBG_INFO,
            0,
            0,
            i32::from(ev.data[0]),
            i32::from(ev.data[1]),
            i32::from(ev.data[2]),
            "MIDI_RX",
            controller_name,
            &detail,
        );
    }
}

pub fn init_hub(hub: &mut LasapiMidiHub) -> c_int {
    unsafe {
        ptr::write_bytes(hub as *mut LasapiMidiHub, 0, 1);
    }
    hub.rawmidi_fd = -1;
    hub.seq_client_id = -1;
    hub.seq_in_port = -1;
    copy_str_to_c(&mut hub.controller_name, "Virtual MIDI Hub");

    for c in 0..16 {
        let path = format!("/dev/snd/midiC{c}D0");
        if Path::new(&path).exists() {
            let cname = discover_midi_card_name(c);
            copy_str_to_c(&mut hub.controller_name, &cname);
            copy_str_to_c(&mut hub.dev_path, &path);
            break;
        }
    }

    const ALSA_CANDS: &[&str] = &[
        "/usr/lib/libasound.so.2.0.0",
        "/usr/lib/libasound.so.2",
        "/usr/lib64/libasound.so.2.0.0",
        "/usr/lib64/libasound.so.2",
        "/usr/lib/x86_64-linux-gnu/libasound.so.2.0.0",
        "/usr/lib/x86_64-linux-gnu/libasound.so.2",
        "/usr/lib/aarch64-linux-gnu/libasound.so.2.0.0",
        "/usr/lib/aarch64-linux-gnu/libasound.so.2",
        "/lib/x86_64-linux-gnu/libasound.so.2",
        "/lib64/libasound.so.2",
        "libasound.so.2",
    ];
    hub.alsa_dl = unsafe { dl_open_any(ALSA_CANDS) };

    let mut has_seq_coder = false;
    let mut has_seq_input = false;
    let mut has_midi_dec = false;

    if !hub.alsa_dl.is_null() {
        unsafe {
            let p_open: Option<FnSeqOpen> = load_sym(hub.alsa_dl, "snd_seq_open");
            let p_name: Option<FnSeqSetName> = load_sym(hub.alsa_dl, "snd_seq_set_client_name");
            let p_port: Option<FnSeqCreatePort> =
                load_sym(hub.alsa_dl, "snd_seq_create_simple_port");
            let p_conn: Option<FnSeqConnectFrom> = load_sym(hub.alsa_dl, "snd_seq_connect_from");
            let p_nb: Option<FnSeqNonblock> = load_sym(hub.alsa_dl, "snd_seq_nonblock");
            let p_cid: Option<FnSeqClientId> = load_sym(hub.alsa_dl, "snd_seq_client_id");
            let p_seq_input: Option<FnSeqEventInput> = load_sym(hub.alsa_dl, "snd_seq_event_input");
            let p_seq_free: Option<FnSeqFreeEvent> = load_sym(hub.alsa_dl, "snd_seq_free_event");
            let p_midi_new: Option<FnMidiEventNew> = load_sym(hub.alsa_dl, "snd_midi_event_new");
            let p_midi_free: Option<FnMidiEventFree> = load_sym(hub.alsa_dl, "snd_midi_event_free");
            let p_midi_nost: Option<FnMidiEventNoStatus> =
                load_sym(hub.alsa_dl, "snd_midi_event_no_status");
            let p_midi_dec: Option<FnMidiEventDecode> =
                load_sym(hub.alsa_dl, "snd_midi_event_decode");

            if let Ok(mut st) = alsa_state().lock() {
                st.p_seq_input = p_seq_input;
                st.p_seq_free = p_seq_free;
                st.p_midi_free = p_midi_free;
                st.p_midi_dec = p_midi_dec;

                if let (Some(f_open), Some(f_name), Some(f_port)) = (p_open, p_name, p_port) {
                    if f_open(&mut hub.seq_handle, c"default".as_ptr(), 3, 1) == 0
                        && !hub.seq_handle.is_null()
                    {
                        if let Some(f_nb) = p_nb {
                            let _ = f_nb(hub.seq_handle, 1);
                        }
                        let _ = f_name(hub.seq_handle, c"LASAPI-MIDI-Hub".as_ptr());
                        hub.seq_client_id = p_cid.map_or(-1, |f| f(hub.seq_handle));
                        hub.seq_in_port = f_port(
                            hub.seq_handle,
                            c"LASAPI DAW MIDI In/Out".as_ptr(),
                            0x20 | 0x40 | 0x01 | 0x02,
                            0x02 | 0x10_0000,
                        );
                        if let (Some(f_new), Some(_)) = (p_midi_new, p_midi_dec) {
                            if !st.seq_coder.is_null() {
                                if let Some(f_free) = st.p_midi_free {
                                    f_free(st.seq_coder);
                                }
                                st.seq_coder = ptr::null_mut();
                            }
                            if f_new(64, &mut st.seq_coder) == 0 && !st.seq_coder.is_null() {
                                if let Some(f_nost) = p_midi_nost {
                                    f_nost(st.seq_coder, 1);
                                }
                            }
                        }
                        if let Some(f_conn) = p_conn {
                            if hub.seq_in_port >= 0 {
                                let _ = f_conn(hub.seq_handle, hub.seq_in_port, 14, 0);
                                for client in 16..=64 {
                                    if client == hub.seq_client_id {
                                        continue;
                                    }
                                    let _ = f_conn(hub.seq_handle, hub.seq_in_port, client, 0);
                                    let _ = f_conn(hub.seq_handle, hub.seq_in_port, client, 1);
                                }
                            }
                        }
                        let cname = c_str_to_string(&hub.controller_name);
                        eprintln!(
                            "[LASAPI-MIDI] Registered kernel ALSA Sequencer hub 'LASAPI-MIDI-Hub' (auto-subscribed to '{cname}')"
                        );
                    }
                }
                has_seq_coder = !st.seq_coder.is_null();
                has_seq_input = st.p_seq_input.is_some();
                has_midi_dec = st.p_midi_dec.is_some();
            }
        }
    }

    let dev_path = c_str_to_string(&hub.dev_path);
    if (hub.seq_handle.is_null() || !has_seq_input || !has_midi_dec || !has_seq_coder)
        && !dev_path.is_empty()
    {
        hub.rawmidi_fd = open_raw_flags(&dev_path, O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    }

    hub.initialized = true;
    let ctrl_name = c_str_to_string(&hub.controller_name);
    let dev_disp = if dev_path.is_empty() {
        "virtual"
    } else {
        dev_path.as_str()
    };
    let detail = format!(
        "dev={dev_disp} seq_client={} port={} rawmidi_fd={}",
        hub.seq_client_id, hub.seq_in_port, hub.rawmidi_fd
    );
    dbg_emit_vals(
        LASAPI_DBG_MIDI,
        LASAPI_DBG_INFO,
        0,
        0,
        hub.seq_client_id,
        hub.seq_in_port,
        hub.rawmidi_fd,
        "MIDI_INIT",
        &ctrl_name,
        &detail,
    );
    0
}

pub fn poll_events(
    hub: &mut LasapiMidiHub,
    out_events: &mut [LasapiMidiEvent],
    period_start_ns: u64,
    sample_rate: u32,
    period_frames: u32,
) -> usize {
    if !hub.initialized || out_events.is_empty() {
        return 0;
    }
    let ctrl_name = c_str_to_string(&hub.controller_name);
    let mut count = 0usize;

    if !hub.seq_handle.is_null() {
        if let Ok(st) = alsa_state().lock() {
            if let (Some(f_input), Some(f_dec)) = (st.p_seq_input, st.p_midi_dec) {
                if !st.seq_coder.is_null() {
                    while count < out_events.len() {
                        let mut seq_ev: *mut c_void = ptr::null_mut();
                        let sr = unsafe { f_input(hub.seq_handle, &mut seq_ev) };
                        if sr < 0 || seq_ev.is_null() {
                            break;
                        }
                        let mut buf = [0u8; 64];
                        let dec_len = unsafe {
                            f_dec(
                                st.seq_coder,
                                buf.as_mut_ptr(),
                                buf.len() as c_long,
                                seq_ev.cast_const(),
                            )
                        };
                        if let Some(f_free) = st.p_seq_free {
                            let _ = unsafe { f_free(seq_ev) };
                        }
                        if (1..=3).contains(&dec_len) {
                            let ev_ns = mono_ns();
                            let ev = LasapiMidiEvent {
                                frame_offset: compute_frame_offset(
                                    ev_ns,
                                    period_start_ns,
                                    sample_rate,
                                    period_frames,
                                ),
                                size: dec_len as u8,
                                port: 0,
                                data: [
                                    buf[0],
                                    if dec_len >= 2 { buf[1] } else { 0 },
                                    if dec_len >= 3 { buf[2] } else { 0 },
                                    0,
                                ],
                                timestamp_ns: ev_ns,
                            };
                            if is_key_press(&ev) {
                                hub.total_events.fetch_add(1, Ordering::Relaxed);
                            }
                            emit_midi_event_log(&ctrl_name, &ev);
                            out_events[count] = ev;
                            count += 1;
                        }
                    }
                }
            }
        }
    }

    if hub.rawmidi_fd >= 0 && count < out_events.len() {
        let mut buf = [0u8; 64];
        let n = nb_read_raw(hub.rawmidi_fd, &mut buf);
        if n > 0 {
            for &b in &buf[..n as usize] {
                if count >= out_events.len() {
                    break;
                }
                let ev_ns = mono_ns();
                if let Some(ev) =
                    feed_raw_byte(hub, b, ev_ns, period_start_ns, sample_rate, period_frames)
                {
                    emit_midi_event_log(&ctrl_name, &ev);
                    out_events[count] = ev;
                    count += 1;
                }
            }
        }
    }

    count
}

pub unsafe fn push_to_ring(ring: *mut LasapiShmRing, ev: &LasapiMidiEvent) {
    if ring.is_null() {
        return;
    }
    let w = unsafe { (*ring).midi_write_idx.load(Ordering::Relaxed) };
    let r = unsafe { (*ring).midi_read_idx.load(Ordering::Acquire) };
    if w.wrapping_sub(r) >= LASAPI_MIDI_RING_EVENTS as u64 {
        return;
    }
    let slot = (w as usize) & LASAPI_MIDI_RING_MASK;
    unsafe {
        ptr::addr_of_mut!((*ring).midi_ring[slot]).write(*ev);
        (*ring).midi_write_idx.store(w.wrapping_add(1), Ordering::Release);
    }
}

pub fn close_hub(hub: &mut LasapiMidiHub) {
    if !hub.initialized {
        return;
    }
    close_raw_fd(hub.rawmidi_fd);
    hub.rawmidi_fd = -1;

    if let Ok(mut st) = alsa_state().lock() {
        if !st.seq_coder.is_null() {
            if let Some(f_free) = st.p_midi_free {
                unsafe { f_free(st.seq_coder) };
            }
            st.seq_coder = ptr::null_mut();
        }
        if !hub.seq_handle.is_null() && !hub.alsa_dl.is_null() {
            let f_close: Option<FnSeqClose> = unsafe { load_sym(hub.alsa_dl, "snd_seq_close") };
            if let Some(close_fn) = f_close {
                let _ = unsafe { close_fn(hub.seq_handle) };
            }
            hub.seq_handle = ptr::null_mut();
        }
        st.p_seq_input = None;
        st.p_seq_free = None;
        st.p_midi_free = None;
        st.p_midi_dec = None;
    }

    if !hub.alsa_dl.is_null() {
        unsafe { dl_close(hub.alsa_dl) };
        hub.alsa_dl = ptr::null_mut();
    }
    hub.initialized = false;
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_midi_init(m: *mut LasapiMidiHub) -> c_int {
    if m.is_null() {
        return -1;
    }
    init_hub(unsafe { &mut *m })
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_midi_poll_events(
    m: *mut LasapiMidiHub,
    out_events: *mut LasapiMidiEvent,
    max_events: c_int,
    period_start_ns: u64,
    sample_rate: u32,
    period_frames: u32,
) -> c_int {
    if m.is_null() || out_events.is_null() || max_events <= 0 {
        return 0;
    }
    let out = unsafe { slice::from_raw_parts_mut(out_events, max_events as usize) };
    poll_events(unsafe { &mut *m }, out, period_start_ns, sample_rate, period_frames) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_midi_push_to_ring(
    ring: *mut LasapiShmRing,
    ev: *const LasapiMidiEvent,
) {
    if ring.is_null() || ev.is_null() {
        return;
    }
    unsafe { push_to_ring(ring, &*ev) };
}

#[no_mangle]
pub unsafe extern "C" fn lasapi_midi_close(m: *mut LasapiMidiHub) {
    if m.is_null() {
        return;
    }
    close_hub(unsafe { &mut *m });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rawmidi_running_status_and_realtime_interleave() {
        let mut hub: LasapiMidiHub = unsafe { mem::zeroed() };
        hub.initialized = true;
        hub.rawmidi_fd = -1;

        assert!(feed_raw_byte(&mut hub, 0x90, 10_000, 0, 48_000, 128).is_none());
        assert!(feed_raw_byte(&mut hub, 60, 10_000, 0, 48_000, 128).is_none());
        let ev1 = feed_raw_byte(&mut hub, 100, 1_000_000, 0, 48_000, 128).unwrap();
        assert_eq!(ev1.size, 3);
        assert_eq!(ev1.data, [0x90, 60, 100, 0]);
        assert_eq!(ev1.frame_offset, 48);
        assert_eq!(hub.total_events.load(Ordering::Relaxed), 1);

        // Real-time clock byte mid-stream must not clobber running status
        let rt = feed_raw_byte(&mut hub, 0xF8, 2_000_000, 0, 48_000, 128).unwrap();
        assert_eq!(rt.size, 1);
        assert_eq!(rt.data, [0xF8, 0, 0, 0]);

        // Running status continuation (note off via vel=0 does not increment key-press count)
        assert!(feed_raw_byte(&mut hub, 60, 2_100_000, 0, 48_000, 128).is_none());
        let ev2 = feed_raw_byte(&mut hub, 0, 2_200_000, 0, 48_000, 128).unwrap();
        assert_eq!(ev2.data, [0x90, 60, 0, 0]);
        assert_eq!(hub.total_events.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn proc_asound_card_parser_and_shm_push() {
        let sample = " 0 [Pro            ]: USB-Audio - Scarlett 2i2 4th Gen\n \
                      5 [FLkey37        ]: USB-Audio - Novation FLkey 37\n";
        assert_eq!(
            parse_proc_asound_cards(sample, 5).as_deref(),
            Some("Novation FLkey 37")
        );
        assert!(parse_proc_asound_cards(sample, 9).is_none());

        let mut boxed: Box<LasapiShmRing> = unsafe {
            let mut u = Box::<LasapiShmRing>::new_uninit();
            ptr::write_bytes(u.as_mut_ptr(), 0, 1);
            u.assume_init()
        };
        let ev = LasapiMidiEvent {
            frame_offset: 12,
            size: 3,
            port: 0,
            data: [0x90, 64, 127, 0],
            timestamp_ns: 999,
        };
        unsafe { push_to_ring(&mut *boxed, &ev) };
        assert_eq!(boxed.midi_write_idx.load(Ordering::Acquire), 1);
        assert_eq!(boxed.midi_ring[0], ev);
    }
}
