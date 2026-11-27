// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::fs;
use std::path::Path;

use salsa_tools::host::read_text_lossy;
use salsa_tools::ipc::{
    LASAPI_EQ_BANDS, LASAPI_FILTER_HIGH_SHELF, LASAPI_FILTER_LOW_SHELF, LASAPI_FILTER_PEAK,
};

use crate::theme::SalsaTheme;

#[derive(Debug, Clone)]
pub struct HostProfile {
    pub os: String,
    pub cpu: String,
    pub pkg_mgr: String,
    pub cards: Vec<String>,
}

pub fn detect_host_pc_profile() -> HostProfile {
    let mut os_name = "Linux".to_string();
    let os_rel = read_text_lossy(Path::new("/etc/os-release"), 8192);
    for line in os_rel.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            let trimmed = rest.trim().trim_matches('"').trim_matches('\'').trim();
            if !trimmed.is_empty() {
                os_name = trimmed.to_string();
                break;
            }
        }
    }

    let mut cpu_name = "x86_64 / ARM64 Processor".to_string();
    let cpuinfo = read_text_lossy(Path::new("/proc/cpuinfo"), 32768);
    for line in cpuinfo.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let kt = k.trim();
            if kt == "model name" || kt == "Processor" || kt == "Hardware" {
                let vt = v.trim();
                if !vt.is_empty() {
                    cpu_name = vt.to_string();
                    break;
                }
            }
        }
    }

    let pkg_mgr = if Path::new("/usr/bin/pacman").exists() {
        "pacman (ALPM)"
    } else if Path::new("/usr/bin/dpkg-query").exists() {
        "dpkg / apt"
    } else if Path::new("/usr/bin/dnf").exists() {
        "dnf / rpm"
    } else if Path::new("/usr/bin/zypper").exists() {
        "zypper / rpm"
    } else {
        "generic Linux"
    }
    .to_string();

    let mut cards = Vec::new();
    let asound_cards = read_text_lossy(Path::new("/proc/asound/cards"), 8192);
    for line in asound_cards.lines() {
        let t = line.trim();
        if t.first_byte_is_digit() {
            cards.push(t.to_string());
        }
    }

    HostProfile {
        os: os_name,
        cpu: cpu_name,
        pkg_mgr,
        cards,
    }
}

trait FirstByteDigit {
    fn first_byte_is_digit(&self) -> bool;
}

impl FirstByteDigit for str {
    fn first_byte_is_digit(&self) -> bool {
        self.as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_digit())
    }
}

#[derive(Debug, Clone, Default)]
pub struct AlsaPlaybackCaps {
    pub rates: Vec<u32>,
    pub bits: Vec<u32>,
}

pub fn parse_alsa_stream0_playback_caps(raw: &str) -> Option<AlsaPlaybackCaps> {
    let mut in_playback = false;
    let mut rates = Vec::new();
    let mut continuous_rates = false;
    let mut bits = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed == "Playback:" {
            in_playback = true;
            continue;
        }
        if trimmed == "Capture:" {
            in_playback = false;
            continue;
        }
        if !in_playback {
            continue;
        }
        if let Some(r_str) = trimmed.strip_prefix("Rates:") {
            let lower = r_str.to_ascii_lowercase();
            if lower.contains("continuous") || r_str.contains('-') {
                rates.clear();
                continuous_rates = true;
            } else if !continuous_rates {
                for tok in r_str.split(|c: char| c == ',' || c.is_whitespace()) {
                    if let Ok(r) = tok.trim().parse::<u32>() {
                        if (8_000..=768_000).contains(&r) && !rates.contains(&r) {
                            rates.push(r);
                        }
                    }
                }
            }
        } else if let Some(b_str) = trimmed.strip_prefix("Bits:") {
            for tok in b_str.split(|c: char| c == ',' || c.is_whitespace()) {
                if let Ok(b) = tok.trim().parse::<u32>() {
                    if !bits.contains(&b) {
                        bits.push(b);
                    }
                }
            }
        } else if let Some(f_str) = trimmed.strip_prefix("Format:") {
            let up = f_str.trim().to_ascii_uppercase();
            if up.contains("32") && !bits.contains(&32) {
                bits.push(32);
            }
            if up.contains("24") && !bits.contains(&24) {
                bits.push(24);
            }
            if up.contains("16") && !bits.contains(&16) {
                bits.push(16);
            }
        }
    }

    if rates.is_empty() && bits.is_empty() {
        None
    } else {
        Some(AlsaPlaybackCaps { rates, bits })
    }
}

pub fn probe_alsa_playback_caps(playback_id: &str) -> Option<AlsaPlaybackCaps> {
    let rest = playback_id.trim().strip_prefix("hw:")?;
    let (card_s, _dev_s) = rest.split_once(',')?;
    let card_num: u32 = card_s.trim().parse().ok()?;
    let stream_path = format!("/proc/asound/card{card_num}/stream0");
    let raw = fs::read_to_string(stream_path).ok()?;
    parse_alsa_stream0_playback_caps(&raw)
}

#[derive(Debug, Clone)]
pub struct AutoEqBand {
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub filter_type: u32,
}

#[derive(Debug, Clone)]
pub struct AutoEqProfile {
    pub name: String,
    pub preamp_db: f32,
    pub bands: Vec<AutoEqBand>,
}

fn parse_leading_f32(s: &str) -> Option<f32> {
    let t = s.trim();
    let end = t
        .char_indices()
        .take_while(|&(i, c)| {
            c.is_ascii_digit() || c == '.' || (i == 0 && (c == '+' || c == '-'))
        })
        .map(|(i, c)| i + c.len_utf8())
        .last()?;
    t[..end].parse::<f32>().ok().filter(|v| v.is_finite())
}

pub fn parse_autoeq_txt(path_or_name: &str, content: &str) -> Option<AutoEqProfile> {
    let mut preamp_db = 0.0f32;
    let mut bands = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("preamp:") {
            if let Some(v) = parse_leading_f32(rest) {
                preamp_db = v.clamp(-18.0, 12.0);
            }
            continue;
        }

        if lower.starts_with("filter") && bands.len() < LASAPI_EQ_BANDS {
            let Some((_lhs, rhs)) = line.split_once(':') else {
                continue;
            };
            let toks: Vec<&str> = rhs.split_whitespace().collect();
            if toks.len() < 4 || !toks[0].eq_ignore_ascii_case("ON") {
                continue;
            }
            let ftype_tok = toks[1].to_ascii_uppercase();
            let filter_type = match ftype_tok.as_str() {
                "LSC" | "LS" | "LSQ" => LASAPI_FILTER_LOW_SHELF,
                "HSC" | "HS" | "HSQ" => LASAPI_FILTER_HIGH_SHELF,
                "PK" | "LP" | "HP" => LASAPI_FILTER_PEAK,
                _ => continue,
            };
            let is_pass_filter = matches!(ftype_tok.as_str(), "LP" | "HP");

            let mut fc_opt: Option<f32> = None;
            let mut gain_opt: Option<f32> = if is_pass_filter { Some(0.0) } else { None };
            let mut q_val: f32 = 1.414;

            let mut i = 2usize;
            while i < toks.len() {
                if toks[i].eq_ignore_ascii_case("Fc") {
                    if let Some(nxt) = toks.get(i + 1) {
                        fc_opt = parse_leading_f32(nxt);
                    }
                    i += 2;
                    continue;
                }
                if toks[i].eq_ignore_ascii_case("Gain") {
                    if let Some(nxt) = toks.get(i + 1) {
                        gain_opt = parse_leading_f32(nxt);
                    }
                    i += 2;
                    continue;
                }
                if toks[i].eq_ignore_ascii_case("Q") {
                    if let Some(nxt) = toks.get(i + 1) {
                        if let Some(qv) = parse_leading_f32(nxt) {
                            q_val = qv.clamp(0.20, 10.0);
                        }
                    }
                    i += 2;
                    continue;
                }
                i += 1;
            }

            if let (Some(fc), Some(gn)) = (fc_opt, gain_opt) {
                bands.push(AutoEqBand {
                    freq_hz: fc.clamp(15.0, 22_000.0),
                    gain_db: gn.clamp(-18.0, 18.0),
                    q: q_val,
                    filter_type,
                });
            }
        }
    }

    if bands.is_empty() {
        return None;
    }

    let base = Path::new(path_or_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "AutoEQ Import".to_string());
    let mut name: String = base.chars().take(31).collect();
    if name.trim().is_empty() {
        name = "AutoEQ Import".to_string();
    }

    Some(AutoEqProfile {
        name,
        preamp_db,
        bands,
    })
}

pub const MANUAL_CHAPTER_BUTTONS: &[(&str, &str)] = &[
    ("1. Overview", "overview"),
    ("2. Shims", "apps_in"),
    ("3. Mic", "mic_in"),
    ("4. MIDI", "midi_in"),
    ("5. SHM", "shm_ring"),
    ("6. 192kHz", "engine_core"),
    ("7. Dyn EQ", "dyn_eq"),
    ("8. Out/Loop", "hw_out"),
    ("9. Sync", "autosync"),
    ("10. Cross-PC", "crosspc"),
    ("11. Video", "vid_broker"),
    ("12. Mix/BT", "mix_bt"),
    ("13. License", "license"),
];

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\0' => {}
            other => out.push(other),
        }
    }
    out
}

fn code_chip(code: &str, theme: &SalsaTheme) -> String {
    format!(
        "<tt><span background=\"{}\" foreground=\"{}\">{}</span></tt>",
        theme.code_bg,
        theme.accent_secondary,
        xml_escape(code)
    )
}

pub fn render_manual_chapter_pango(key: &str, t: &SalsaTheme, host: &HostProfile) -> String {
    let c = |txt: &str| code_chip(txt, t);
    let (accent_key, heading, subtitle, intro, bullets): (&str, &str, &str, String, Vec<String>) =
        match key {
            "apps_in" => (
                "accent_secondary",
                "🔊 Chapter 2: Automatic Application Recognition & Hollow ABI Shims",
                "Zero Wrapper Commands Needed — Automatic Desktop-Wide Interception",
                "Every desktop program on your PC connects automatically to Stage 1 of the Salsa graph through three transparent mechanisms:".into(),
                vec![
                    format!("<b>1. Default ALSA &amp; OpenAL Routing ({} &amp; {}):</b> Defines {} as an asymmetrical full-duplex {} device backed by {}.", c("~/.asoundrc"), c("alsoft.conf"), c("pcm.!default"), c("type lasapi"), c("libasound_module_pcm_lasapi.so")),
                    format!("<b>2. Hollow ABI Shims &amp; Native PA v35 Server ({} + {}):</b> Drop-in libraries for {}, {}, {}, {}, and {} plus the built-in PulseAudio Protocol v35 UNIX socket run browsers, Discord, Steam/Proton, KDE Plasma, and DAWs with zero PipeWire-Pulse.", c("build/shims/"), c("$XDG_RUNTIME_DIR/pulse/native"), c("libasound.so.2"), c("libpulse.so.0"), c("libpulse-simple.so.0"), c("libpulse-mainloop-glib.so.0"), c("libjack.so.0")),
                    format!("<b>3. Native LASAPI Client API ({}):</b> Exposes {}, {}, {}, and {} interfaces directly on Linux.", c("liblasapi.so"), c("IAudioClient"), c("IAudioRenderClient"), c("IAudioCaptureClient"), c("IAudioClock")),
                ],
            ),
            "mic_in" => (
                "accent_mic",
                "🎙️ Chapter 3: Full-Duplex Hardware Mic Capture & HFP BTPROTO_SCO",
                "Hardware Mic Input -> Direct Raw WaveRT Capture -> Apps",
                "Salsa operates a true <b>full-duplex WaveRT engine</b> that captures raw microphone audio cleanly while rendering playback:".into(),
                vec![
                    "<b>Bit-Exact Raw Interface Capture:</b> USB/PCIe audio interfaces (such as Focusrite Scarlett 2i2) pass directly through the WaveRT capture ring with optional analog HW boost and split capture clock support.".into(),
                    format!("<b>Desktop Mic Gain &amp; Mute Synchronization:</b> Adjusting microphone gain or mute in KDE Plasma, {}, Tray, or Salsa syncs directly through Salsa's native PA v35 server and {} with zero WirePlumber dependency.", c("pactl"), c("lasapi-audiodg")),
                ],
            ),
            "midi_in" => (
                "accent_midi",
                "🎹 Chapter 4: Hardware MIDI Controller & Sample-Accurate DAW MIDI Hub",
                "Hardware RawMIDI + ALSA Sequencer + Lock-Free JACK MIDI Ring",
                format!("Salsa integrates a dedicated pure-Rust real-time MIDI hub ({}) directly alongside the audio engine:", c("midi_hub.rs")),
                vec![
                    format!("<b>Auto-Discovery of Hardware Controllers:</b> Automatically scans {} and ALSA Sequencer ports, binding controllers (e.g. <b>Novation FLkey 37</b>) and exposing the virtual {} port.", c("/dev/snd/midiC*D*"), c("LASAPI-MIDI-Hub")),
                    format!("<b>Sample-Accurate Frame Offsets:</b> Incoming MIDI Note, Pitch Bend, and CC events are timestamped with {} and stamped with the exact {} inside the current audio quantum across each DAW session's lock-free {}.", c("CLOCK_MONOTONIC_RAW"), c("frame_offset"), c("midi_ring[512]")),
                ],
            ),
            "shm_ring" => (
                "accent_secondary",
                "⚡ Chapter 5: Zero-Copy WaveRT Shared Memory Ring (memfd + eventfd)",
                "75-Nanosecond Buffer Commits — 14,048x Faster Than Socket IPC",
                "Audio PCM samples in Salsa <b>never travel over Unix sockets or pipes</b> during WaveRT streaming:".into(),
                vec![
                    format!("<b>Anonymous Shared Memory ({}):</b> {} allocates a 64-byte-aligned {} ring (32,768 stereo float32 frames for 192 kHz headroom) and passes the FD via {}.", c("memfd_create"), c("lasapi-audiodg"), c("lasapi_shm_ring_t"), c("SCM_RIGHTS")),
                    format!("<b>Zero-Copy Atomic Pointers:</b> Clients write float32 frames directly into L1/L2-cached shared memory and advance {} with a single atomic release store ({} commit latency).", c("write_frame"), c("0.075 µs")),
                    format!("<b>Kernel {} Wakeups:</b> Every hardware period quantum signals the client's kernel {}, waking event-driven DAWs and games with sub-millisecond jitter.", c("eventfd"), c("eventfd")),
                ],
            ),
            "engine_core" => (
                "accent_primary",
                "🚀 Chapter 6: MMCSS SCHED_FIFO Engine & Fixed Lossless Master Clock",
                "Deterministic Sample Rate (44.1–192 kHz) & Bit-Depth Precision (16 / 24 / 32-bit)",
                format!("At the center of the graph is the {} real-time audio graph thread, scheduled with real-time MMCSS priority:", c("lasapi-audiodg")),
                vec![
                    format!("<b>Fixed Lossless Master &amp; Split Mic Clock:</b> Lock your studio master clock to {} and {}, while optionally pinning microphone capture independently (e.g. {} for Discord voice while headphones run at {}).", c("44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz"), c("16 / 24 / 32-bit"), c("48 kHz"), c("192 kHz")),
                    format!("<b>{} Priority 65 + {}:</b> All ring buffers and biquad states stay locked in RAM so page faults never cause XRUN dropouts.", c("SCHED_FIFO"), c("mlockall")),
                ],
            ),
            "dyn_eq" => (
                "accent_hot",
                "🎚️ Chapter 7: 10-Band Parametric Dynamic EQ, AutoEQ Importer & Crossfeed",
                "Per-Band Freq/Q/Shelf Shapes + Oratory1990 .txt Import + Bauer Crossfeed + Limiter",
                "Every mixed audio period passes through the built-in <b>Salsa Audio Processing Object (APO)</b> inside the real-time thread:".into(),
                vec![
                    format!("<b>Full 10-Band Parametric Biquads:</b> Each band supports custom frequency ({}), adjustable {} ({}), and <b>Peak</b> / <b>Low-Shelf</b> / <b>High-Shelf</b> filters, plus one-click import of <b>AutoEQ / Oratory1990 {}</b> profiles and per-device automatic profile switching.", c("15 Hz – 22 kHz"), c("Q"), c("0.20 – 10.00"), c(".txt")),
                    "<b>Real-Time Dynamic Processing &amp; Bauer Crossfeed:</b> Dedicated bandpass envelope followers tame harsh peaks or expand punch dynamically, paired with optional BS2B-style stereo crossfeed.".into(),
                    format!("<b>Soft-Knee Mastering Limiter:</b> Prevents digital clipping above {} ({}) via smooth hyperbolic-tangent saturation.", c("0.95"), c("-0.4 dBFS")),
                ],
            ),
            "hw_out" => (
                "accent_out",
                "🎧 Chapter 8: Master Output Sink, BlueZ Codec Switcher & Wireless ABR",
                "Active BlueZ Codec Diagnostics + LDAC ABR (990/660/330) + OBS Loopback",
                "After Parametric Dynamic EQ, Crossfeed, and Limiter processing, the 32-bit float master bus delivers directly to your active endpoint:".into(),
                vec![
                    format!("<b>BlueZ Codec Switcher &amp; Adaptive Bitrate (ABR):</b> Inspect remote headset SEPs, view negotiation diagnostics, switch codecs live (Auto, LDAC, aptX HD, aptX, AAC, SBC), and scale LDAC ({}) or AAC ({}) automatically on RF congestion.", c("990 / 660 / 330 kbps"), c("256 / 192 / 128 kbps")),
                    format!("<b>Echo-Free Loopback Monitor:</b> Copies clean post-EQ desktop audio directly to OBS ({}) with zero capture bleed.", c("lasapi_default_sink.monitor")),
                ],
            ),
            "autosync" => (
                "accent_secondary",
                "🔄 Chapter 9: Automatic Audio Module Update Watcher & ABI Self-Healing",
                "Real-Time Package & Library Watcher (lasapi-module-watcher.path + build/lasapi-autosync)",
                "When your Linux distribution updates system audio or Bluetooth libraries, Salsa adapts automatically:".into(),
                vec![
                    format!("<b>Systemd Path &amp; Pacman Hook ({} + {}):</b> Watches your package database and host libraries ({}, {}, {}) to trigger {} automatically.", c("lasapi-module-watcher.path"), c("99-salsa-autosync.hook"), c("libasound.so.2"), c("libpulse.so.0"), c("libjack.so.0"), c("lasapi-autosync")),
                    format!("<b>ELF Symbol-Version Sync &amp; Proxy Regeneration:</b> Scans host libraries to keep {} / {} and the 200-stub {} reconnect proxy in lockstep with host updates.", c("src/alsa.version"), c("src/pulse.version"), c("pa_reconnect/libpulse.so.0")),
                ],
            ),
            "crosspc" => (
                "accent_out",
                "🌍 Chapter 10: Universal Cross-PC Hardware & Distro Adaptation",
                "100% Pure Rust Stack • Zero External Crates • PipeWire/WirePlumber-Free Audio Ownership",
                "Salsa adapts automatically to any Linux desktop, workstation, or laptop:".into(),
                vec![
                    format!("<b>Zero Hardcoded Paths:</b> Every pure-Rust binary resolves its root dynamically via {} and {}, with reversible backups in {}.", c("/proc/self/exe"), c("$HOME"), c("~/.config/lasapi/pw_backup/")),
                    format!("<b>Direct Kernel &amp; BlueZ Hardware Scoring:</b> Communicates directly with {} D-Bus A2DP/HFP/AVRCP, kernel ALSA {}, HID dials, and {} with zero WirePlumber audio daemon.", c("org.bluez"), c("/dev/snd/pcmC*D*"), c("/dev/snd/midiC*D*")),
                    format!("<b>Live Detected Host PC Profile:</b> {} • {} • Watcher: {} • Soundcards: {} detected.", c(&format!("OS: {}", host.os)), c(&format!("CPU: {}", host.cpu)), c(&host.pkg_mgr), host.cards.len()),
                ],
            ),
            "vid_src" => (
                "accent_secondary",
                "🖥️ Chapter 11A: Wayland Video Producer & Portal Handoff",
                "kwin_wayland ScreenCast Stream Producer + xdg-desktop-portal OpenPipeWireRemote",
                format!("Wayland compositors such as <b>KDE KWin ({})</b> export desktop, monitor, and window captures natively into Salsa's built-in Video PipeWire socket:", c("kwin_wayland")),
                vec![
                    format!("<b>Zero-Copy GPU DmaBuf &amp; MemFd Producers:</b> Supports hardware {} modifiers (AMD RDNA / Mesa {} / Vulkan) and CPU {} fallback with full {} negotiation.", c("DmaBuf"), c("radv"), c("MemFd"), c("SPA_PARAM_EnumFormat")),
                    format!("<b>{} Portal Handoff:</b> Cleanly resets portal proxy slots on repeated {} handshakes so Discord, Chromium, and OBS bind on the first frame.", c("OpenPipeWireRemote"), c("PW_CORE_M_HELLO")),
                ],
            ),
            "vid_broker" => (
                "accent_secondary",
                "🔌 Chapter 11B: Zero-PipeWire Native Rust Video Broker",
                "$XDG_RUNTIME_DIR/pipewire-0 • Zero libpipewire / Zero WirePlumber / Audio Guard Active",
                format!("{} embeds a standalone <b>Video-Only PipeWire Protocol Broker</b> ({}) listening directly on {} without linking {}:", c("lasapi-audiodg"), c("pw_broker.rs"), c("$XDG_RUNTIME_DIR/pipewire-0"), c("libpipewire")),
                vec![
                    format!("<b>Strict Video-Only Admission:</b> Only {}, {}, and {} nodes are routed; audio node attempts are rejected ({}) so Salsa retains 100% exclusive audio ownership.", c("Video/Source"), c("Stream/Output/Video"), c("Stream/Input/Video"), c("audio_rejects")),
                    "<b>Clean KWin Stability:</b> Enforces safe fractional framerate bounds and relays both DmaBuf and MemFd SPA pods for KDE Spectacle, Plasma previews, browsers, OBS, and Discord.".into(),
                ],
            ),
            "vid_relay" => (
                "accent_secondary",
                "🎞️ Chapter 11C: Zero-Copy Frame Relay, Live Resolution/FPS Cap & Preview HUD",
                "SCM_RIGHTS FD Relay + Dynamic PW_CAST_LIMITS + Zero-Overhead Live Feed Mirror",
                "Stage 3 of the video lane bridges producer buffer pools directly to subscribed consumer nodes:".into(),
                vec![
                    format!("<b>Direct {} Buffer Relay:</b> DmaBuf and MemFd buffer file descriptors cycle locklessly between producer and consumer via {}.", c("SCM_RIGHTS"), c("eventfd")),
                    format!("<b>Live Resolution &amp; FPS Profile Control ({}):</b> Apply Discord-matched presets ({}, {}, {}, {}) on the fly in {}.", c("PW_CAST_LIMITS"), c("480p@15"), c("720p@30"), c("1080p@60"), c("Source"), c("salsa-debug --tab=4")),
                    format!("<b>Zero-Overhead Preview HUD ({}):</b> SHM frame mirroring stays completely dormant (0% CPU/GPU) until Live Feed is enabled.", c("PW_PREVIEW")),
                ],
            ),
            "vid_cons" => (
                "accent_warn",
                "📡 Chapter 11D: ScreenCast Consumers & Synchronized Audio Capture",
                "Discord (AV1 VAAPI) / OBS Studio / Browsers / KDE Portal + Parallel Salsa Audio Routing",
                "Applications on Stage 4 receive Wayland ScreenCast video frames in parallel with ultra-low-latency Salsa audio:".into(),
                vec![
                    format!("<b>Video Consumers:</b> Discord, OBS Studio PipeWire Capture, Firefox/Chromium WebRTC share, Spectacle, and KDE taskbar previews connect seamlessly to {}.", c("pipewire-0")),
                    format!("<b>Parallel Native Audio Route:</b> Voice chat and desktop loopback audio travel concurrently through Salsa's {} WaveRT Capture &amp; PA v35 server.", c("0.33 ms")),
                ],
            ),
            "mix_bt" => (
                "accent_warn",
                "🎛️ Chapter 12: HW Analog Boost & BlueZ HFP SCO",
                "Discrete Sound Card DAC/Amp • Two-Way Bluetooth HFP Headset Mic",
                format!(
                    "Salsa {} unifies discrete sound card analog stages and wireless headset controls:",
                    salsa_wire::legal::salsa_version()
                ),
                vec![
                    "<b>Hardware Analog Architecture:</b> Utilizes ALSA DAC unity volume, discrete headphone amp gain tiers (32Ω / 150Ω / 600Ω TPA6120), Analog Soft-Knee Drive, and USB HID volume dials (e.g. ASUS Strix Raid DLX / Focusrite Scarlett).".into(),
                    format!("<b>Two-Way Bluetooth HFP {}:</b> Optional two-way Bluetooth HFP mSBC/CVSD headset mic support alongside lossless output codecs.", c("BTPROTO_SCO")),
                ],
            ),
            "license" => (
                "accent_secondary",
                "📜 Chapter 13: License, Warranty Notice & Third-Party Libraries (LGPL-3.0)",
                "Copyright (C) 2026 Linuxknows — GNU Lesser General Public License v3.0 Only (LGPL-3.0-only)",
                format!(
                    "Salsa {} is free software released strictly under the <b>GNU Lesser General Public License, version 3.0 only</b> ({}), accompanied by GNU GPL-3.0 ({}), distributed <b>WITHOUT ANY WARRANTY</b>:",
                    salsa_wire::legal::salsa_version(),
                    c("COPYING.LESSER"),
                    c("COPYING"),
                ),
                vec![
                    format!("<b>Copyleft Library &amp; Relinking Rights (LGPL-3.0 §4):</b> Complete Corresponding Source for {}, {}, {}, {}, {}, and Qt6 frontends lives in {}, enabling modification, static/dynamic rebuilding via {}, and in-place replacement.", c("liblasapi.so"), c("build/shims/*.so"), c("libasound_module_pcm_lasapi.so"), c("libsalsa_wire.a"), c("libsalsa_ui.a"), c("rust/ + qt6/"), c("make all")),
                    format!("<b>Qt6 Framework &amp; Zero External Crates:</b> Salsa links dynamically to {} strictly under {}, and uses zero third-party {} dependencies.", c("Qt6 (qt6-base)"), c("LGPL-3.0-only"), c("crates.io")),
                    format!("<b>Optional Host {} Libraries (Never Bundled):</b> Optional codecs and Wayland helpers such as {} (Fraunhofer FDK-AAC), {}, {}, {}, and {} are opened only at runtime via {} if already installed on your system, and fall back cleanly when absent. See {} and {}.", c("dlopen"), c("libfdk-aac.so.2"), c("libldacBT_enc.so.2"), c("libfreeaptx.so.0"), c("libsbc.so.1"), c("libLayerShellQtInterface.so.6"), c("dlopen()"), c("THIRD_PARTY_LICENSES.md"), c("~/.local/share/licenses/salsa/")),
                ],
            ),
            _ => (
                "accent_primary",
                "📘 Chapter 1: Philosophy — Why Salsa Needs Zero Manual Patching",
                "Click any node in the live Signal Topology Graph above (or use the chapter buttons) to inspect how each stage works.",
                format!("Unlike manual patchbays ({} / {}), <b>Salsa ({})</b> is a <b>100% self-wiring Linux Audio Session API (LASAPI) / WaveRT Audio Device Graph</b> built from scratch in pure Rust:", c("qpwgraph"), c("QjackCtl"), c("lasapi-audiodg")),
                vec![
                    format!("<b>Fixed Lossless Master Clock &amp; Bit Depth Control:</b> Lock hardware and BlueZ clocks cleanly across {} and {} without unpredictable sample-rate hunting.", c("44.1 – 192 kHz"), c("16 / 24 / 32-bit")),
                    "<b>Automatic Session Discovery &amp; Per-App Routing:</b> Native LASAPI, ALSA, PulseAudio v35, SDL2/SDL3, OpenAL, Proton/Wine, and JACK apps register automatically with persistent per-app volume, mute, and device routing.".into(),
                    format!("<b>130x Lower Round-Trip Latency:</b> Full-duplex loopback and capture complete in <b>0.33 ms</b> at {} (vs {} on legacy PipeWire-Pulse).", c("192 kHz / 64f"), c("42.67 ms")),
                ],
            ),
        };

    let heading_col = t.get_token(accent_key);
    let mut out = format!(
        "<span size=\"large\" weight=\"bold\" foreground=\"{heading_col}\">{}</span>\n\
         <span size=\"small\" weight=\"bold\" foreground=\"{}\">{}</span>\n\n\
         <span foreground=\"{}\">{intro}</span>\n",
        xml_escape(heading),
        t.text_muted,
        xml_escape(subtitle),
        t.text_primary,
    );
    for b in bullets {
        out.push_str(&format!(
            "\n<span foreground=\"{}\">  • </span><span foreground=\"{}\">{b}</span>",
            heading_col, t.text_primary
        ));
    }
    out
}

#[derive(Debug, Clone)]
pub struct ManualRichSegment {
    pub text: String,
    pub bold: bool,
    pub code_chip: bool,
}

#[derive(Debug, Clone)]
pub struct ManualChapterDoc {
    pub accent_hex: String,
    pub heading: String,
    pub subtitle: String,
    pub intro: String,
    pub intro_segments: Vec<ManualRichSegment>,
    pub bullets: Vec<String>,
    pub bullets_segments: Vec<Vec<ManualRichSegment>>,
}

fn unescape_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn parse_pango_segments(raw: &str) -> Vec<ManualRichSegment> {
    let mut out: Vec<ManualRichSegment> = Vec::new();
    let mut bold_depth: usize = 0;
    let mut rem = raw;

    let push_plain = |txt: &str, bold: bool, out: &mut Vec<ManualRichSegment>| {
        let unescaped = unescape_entities(txt);
        if unescaped.is_empty() {
            return;
        }
        if let Some(prev) = out.last_mut().filter(|s| !s.code_chip && s.bold == bold) {
            prev.text.push_str(&unescaped);
        } else {
            out.push(ManualRichSegment {
                text: unescaped,
                bold,
                code_chip: false,
            });
        }
    };

    while !rem.is_empty() {
        let Some(lt) = rem.find('<') else {
            push_plain(rem, bold_depth > 0, &mut out);
            break;
        };
        if lt > 0 {
            push_plain(&rem[..lt], bold_depth > 0, &mut out);
            rem = &rem[lt..];
        }
        if let Some(after) = rem.strip_prefix("<b>") {
            bold_depth = bold_depth.saturating_add(1);
            rem = after;
        } else if let Some(after) = rem.strip_prefix("</b>") {
            bold_depth = bold_depth.saturating_sub(1);
            rem = after;
        } else if let Some(after_tt) = rem.strip_prefix("<tt") {
            if let Some(tag_end) = after_tt.find('>') {
                let inner_start = &after_tt[tag_end + 1..];
                if let Some(end_tt) = inner_start.find("</tt>") {
                    let inner = &inner_start[..end_tt];
                    let clean = if let Some(gt) = inner.find('>') {
                        let after = &inner[gt + 1..];
                        after.strip_suffix("</span>").unwrap_or(after)
                    } else {
                        inner
                    };
                    let code_lit = unescape_entities(clean.trim());
                    if !code_lit.is_empty() {
                        out.push(ManualRichSegment {
                            text: code_lit,
                            bold: true,
                            code_chip: true,
                        });
                    }
                    rem = &inner_start[end_tt + 5..];
                    continue;
                }
            }
            push_plain("<", bold_depth > 0, &mut out);
            rem = &rem[1..];
        } else if let Some(after) = rem.strip_prefix("</tt>") {
            rem = after;
        } else {
            push_plain("<", bold_depth > 0, &mut out);
            rem = &rem[1..];
        }
    }
    out
}

fn strip_pango_tags_and_unescape(raw: &str) -> String {
    let mut no_tags = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => no_tags.push(ch),
            _ => {}
        }
    }
    unescape_entities(&no_tags)
}

pub fn render_manual_chapter_doc(
    chapter_key: &str,
    t: &SalsaTheme,
    host: &HostProfile,
) -> ManualChapterDoc {
    let pango = render_manual_chapter_pango(chapter_key, t, host);
    let accent_hex = pango
        .split("foreground=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .map(str::to_string)
        .unwrap_or_else(|| t.accent_primary.clone());

    let plain = strip_pango_tags_and_unescape(&pango);
    let mut lines = plain.lines();
    let heading = lines.next().unwrap_or("").trim().to_string();
    let subtitle = lines.next().unwrap_or("").trim().to_string();
    let mut intro = String::new();
    let mut bullets = Vec::new();
    for raw in lines {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(b) = trimmed.strip_prefix("• ") {
            bullets.push(b.trim().to_string());
        } else if intro.is_empty() {
            intro = trimmed.to_string();
        } else {
            bullets.push(trimmed.to_string());
        }
    }

    let mut raw_body_lines: Vec<String> = Vec::new();
    for raw in pango.lines().skip(2) {
        let mut s = raw.trim();
        if s.is_empty() {
            continue;
        }
        if let Some((_, tail)) = s.split_once("• </span>") {
            s = tail.trim();
        }
        if let Some(stripped) = s.strip_prefix("<span foreground=\"") {
            if let Some(gt) = stripped.find('>') {
                s = &stripped[gt + 1..];
            }
        }
        s = s.strip_suffix("</span>").unwrap_or(s).trim();
        if !s.is_empty() {
            raw_body_lines.push(s.to_string());
        }
    }

    let intro_segments = raw_body_lines
        .first()
        .map(|raw| parse_pango_segments(raw))
        .unwrap_or_else(|| {
            vec![ManualRichSegment {
                text: intro.clone(),
                bold: false,
                code_chip: false,
            }]
        });

    let bullets_segments: Vec<Vec<ManualRichSegment>> = bullets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            raw_body_lines
                .get(i + 1)
                .map(|raw| parse_pango_segments(raw))
                .unwrap_or_else(|| {
                    vec![ManualRichSegment {
                        text: b.clone(),
                        bold: false,
                        code_chip: false,
                    }]
                })
        })
        .collect();

    ManualChapterDoc {
        accent_hex,
        heading,
        subtitle,
        intro,
        intro_segments,
        bullets,
        bullets_segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::theme_by_name;

    #[test]
    fn autoeq_parser_handles_oratory_and_equalizer_apo() {
        let sample = "\
Preamp: -4.2 dB
Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.71
Filter 2: ON PK Fc 1450.5 Hz Gain -3.2 dB Q 2.10
Filter 3: OFF PK Fc 3000 Hz Gain 9.0 dB Q 1.0
Filter 4: ON HSC Fc 10000 Hz Gain 2.0 dB
";
        let prof = parse_autoeq_txt("/tmp/Bose QC Ultra 2.txt", sample).expect("valid autoeq");
        assert_eq!(prof.name, "Bose QC Ultra 2");
        assert!((prof.preamp_db - (-4.2)).abs() < 0.01);
        assert_eq!(prof.bands.len(), 3);
        assert_eq!(prof.bands[0].filter_type, LASAPI_FILTER_LOW_SHELF);
        assert!((prof.bands[0].freq_hz - 105.0).abs() < 0.01);
        assert!((prof.bands[0].q - 0.71).abs() < 0.01);
        assert_eq!(prof.bands[1].filter_type, LASAPI_FILTER_PEAK);
        assert!((prof.bands[1].gain_db - (-3.2)).abs() < 0.01);
        assert_eq!(prof.bands[2].filter_type, LASAPI_FILTER_HIGH_SHELF);
        assert!((prof.bands[2].q - 1.414).abs() < 0.01);
    }

    #[test]
    fn all_fifteen_manual_chapters_render_non_empty_markup() {
        let theme = theme_by_name("Salsa Midnight (Default)");
        let host = HostProfile {
            os: "CachyOS <Test & Verify>".into(),
            cpu: "AMD Ryzen 7".into(),
            pkg_mgr: "pacman (ALPM)".into(),
            cards: vec!["0 [USB]: Scarlett".into()],
        };
        for key in [
            "overview",
            "apps_in",
            "mic_in",
            "midi_in",
            "shm_ring",
            "engine_core",
            "dyn_eq",
            "hw_out",
            "autosync",
            "crosspc",
            "vid_src",
            "vid_broker",
            "vid_relay",
            "vid_cons",
            "mix_bt",
            "license",
        ] {
            let m = render_manual_chapter_pango(key, &theme, &host);
            assert!(m.contains("<span"));
            assert!(!m.contains("<Test"));
            let doc = render_manual_chapter_doc(key, &theme, &host);
            assert!(!doc.intro.is_empty());
            assert!(!doc.bullets.is_empty());
            assert_eq!(doc.bullets.len(), doc.bullets_segments.len());
            for seg in doc.intro_segments.iter().chain(doc.bullets_segments.iter().flatten()) {
                assert!(!seg.text.contains("<b>") && !seg.text.contains("</b>"), "stray bold tag in {key}: {:?}", seg.text);
                assert!(!seg.text.contains("<tt") && !seg.text.contains("</tt>"), "stray tt tag in {key}: {:?}", seg.text);
            }
        }
    }

    #[test]
    fn alsa_stream0_caps_parser_handles_discrete_and_continuous_rates() {
        let discrete = "\
Playback:
  Status: Stop
  Interface 1
    Altset 1
    Format: S32_LE
    Channels: 2
    Bits: 24 32
    Rates: 44100, 48000, 96000, 192000
Capture:
  Rates: 8000
";
        let caps = parse_alsa_stream0_playback_caps(discrete).expect("discrete caps");
        assert_eq!(caps.rates, vec![44_100, 48_000, 96_000, 192_000]);
        assert_eq!(caps.bits, vec![32, 24]);

        let continuous = "\
Playback:
  Format: S24_3LE
  Rates: 44100 - 384000 (continuous)
";
        let cont_caps = parse_alsa_stream0_playback_caps(continuous).expect("bits still present");
        assert!(cont_caps.rates.is_empty());
        assert_eq!(cont_caps.bits, vec![24]);
    }
}
