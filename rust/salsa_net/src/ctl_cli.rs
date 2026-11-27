// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, PodBytes, AUDCLNT_SHAREMODE_EXCLUSIVE, E_FAIL, LasapiDeviceList,
    LasapiEngineStatus, LasapiEqConfig, LasapiHwAnalogCfg, LasapiIpcReq, LasapiIpcResp,
    LasapiParamEqPayload, LASAPI_BT_CAP_AAC, LASAPI_BT_CAP_APTX, LASAPI_BT_CAP_APTX_HD,
    LASAPI_BT_CAP_LDAC, LASAPI_BT_CAP_SBC, LASAPI_BT_CONN_CONNECT,
    LASAPI_BT_CONN_DISCONNECT, LASAPI_BT_CONN_QUERY, LASAPI_BT_CONN_SET_AUTOCONN,
    LASAPI_BT_CONN_SET_OPTIONAL, LASAPI_BT_PREF_AAC, LASAPI_BT_PREF_APTX,
    LASAPI_BT_PREF_APTX_HD, LASAPI_BT_PREF_AUTO, LASAPI_BT_PREF_LDAC, LASAPI_BT_PREF_SBC,
    LASAPI_BT_STAT_CONNECTED, LASAPI_BT_STAT_OPTIONAL, LASAPI_BT_STAT_TRANSPORT,
    LASAPI_BT_STAT_USB_MATCHES_BT, LASAPI_CFG_FLAG_CAPTURE_RATE, LASAPI_CMD_BT_CONNECTION,
    LASAPI_CMD_INJECT_MIDI, LASAPI_CMD_RELOAD_CONFIG, LASAPI_CMD_SAVE_CONFIG,
    LASAPI_CMD_SET_BT_CODEC, LASAPI_CMD_SET_ENGINE_CONFIG, LASAPI_CMD_SET_EQ,
    LASAPI_CMD_SET_HW_ANALOG, LASAPI_CMD_SET_MIC_APO, LASAPI_CMD_SET_PARAM_EQ,
    LASAPI_CMD_SET_SESSION_CONTROL, LASAPI_CMD_SET_VOLUME, LASAPI_EQ_BANDS,
    LASAPI_FILTER_HIGH_SHELF, LASAPI_FILTER_LOW_SHELF,
    LASAPI_FILTER_PEAK, LASAPI_LDAC_MODE_330, LASAPI_LDAC_MODE_660, LASAPI_LDAC_MODE_990,
    LASAPI_LDAC_MODE_ADAPTIVE, S_OK,
};
use crate::hw_analog::{
    LASAPI_HW_AMP_AUTO, LASAPI_HW_AMP_HIGH, LASAPI_HW_AMP_LOW, LASAPI_HW_AMP_MED,
    LASAPI_HW_ROUTE_AUTO, LASAPI_HW_ROUTE_HP, LASAPI_HW_ROUTE_SPK,
};
use crate::legal::{handle_common_cli_flag, one_line_legal_notice, salsa_version};
use crate::provisioning::{
    apply_zero_pipewire_cutover, restore_stock_pipewire, RealHostEnvOps, SalsaHostPaths,
};
pub use crate::provisioning::{build_filtered_ld_path, transform_steam_desktop};
use crate::unix_sys::resolve_sock_path_common;

fn get_socket_path() -> String {
    resolve_sock_path_common("LASAPI_SOCKET", "audiodg.sock", "/tmp/lasapi-audiodg.sock")
}

fn get_debug_socket_path() -> String {
    resolve_sock_path_common("LASAPI_DEBUG_SOCK", "debug.sock", "/tmp/lasapi-debug.sock")
}

fn zeroed_struct<T: PodBytes>() -> T {
    T::zeroed()
}

fn get_full_status() -> Option<LasapiEngineStatus> {
    let sock = get_socket_path();
    let resp = crate::control_ipc::ControlClient::send_with_timeout(
        &sock,
        &crate::control_ipc::ControlRequest::GetStatus,
        crate::control_ipc::DEFAULT_CONTROL_TIMEOUT,
    )
    .ok()?;
    match resp.payload {
        crate::control_ipc::ControlPayload::Status { status, .. } if resp.hr == S_OK => Some(*status),
        _ => None,
    }
}

fn send_req_only(req: &LasapiIpcReq) -> Option<LasapiIpcResp> {
    let ctrl_req = crate::control_ipc::legacy_ipc_req_to_control(req, &[])?;
    let sock = get_socket_path();
    let resp = crate::control_ipc::ControlClient::send_with_timeout(
        &sock,
        &ctrl_req,
        crate::control_ipc::DEFAULT_CONTROL_TIMEOUT,
    )
    .ok()?;
    Some(crate::control_ipc::control_resp_to_legacy(&resp))
}

fn send_payload_cmd(cmd: u32, payload: &[u8]) -> (i32, LasapiIpcResp) {
    let mut dummy: LasapiIpcReq = zeroed_struct();
    dummy.cmd = cmd;
    dummy.requested_buffer_frames = payload.len() as u32;
    let Some(ctrl_req) = crate::control_ipc::legacy_ipc_req_to_control(&dummy, payload) else {
        let mut resp: LasapiIpcResp = zeroed_struct();
        resp.hr = E_FAIL;
        return (1, resp);
    };
    let sock = get_socket_path();
    let Ok(resp) = crate::control_ipc::ControlClient::send_with_timeout(
        &sock,
        &ctrl_req,
        crate::control_ipc::DEFAULT_CONTROL_TIMEOUT,
    ) else {
        let mut resp: LasapiIpcResp = zeroed_struct();
        resp.hr = E_FAIL;
        return (1, resp);
    };
    let leg = crate::control_ipc::control_resp_to_legacy(&resp);
    (i32::from(leg.hr != S_OK), leg)
}

fn c_atoi(s: &str) -> i32 {
    let trimmed = s.trim_start();
    let (neg, rest) = if let Some(r) = trimmed.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = trimmed.strip_prefix('+') {
        (false, r)
    } else {
        (false, trimmed)
    };
    let digits_len = rest
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if digits_len == 0 {
        return 0;
    }
    let v = rest[..digits_len].parse::<i64>().unwrap_or(if neg {
        i64::from(i32::MIN)
    } else {
        i64::from(i32::MAX)
    });
    let signed = if neg { -v } else { v };
    signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn c_strtof(s: &str) -> f32 {
    let trimmed = s.trim_start();
    let end = trimmed
        .bytes()
        .take_while(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
        .count();
    if end == 0 {
        return 0.0;
    }
    trimmed[..end].parse::<f32>().unwrap_or(0.0)
}

fn bt_pref_name(pref: u32) -> &'static str {
    match pref {
        LASAPI_BT_PREF_LDAC => "LDAC",
        LASAPI_BT_PREF_APTX_HD => "aptX HD",
        LASAPI_BT_PREF_APTX => "aptX",
        LASAPI_BT_PREF_AAC => "AAC",
        LASAPI_BT_PREF_SBC => "SBC",
        _ => "Auto",
    }
}

fn ldac_tier_name(mode: u32) -> &'static str {
    match mode {
        LASAPI_LDAC_MODE_990 => "990 kbps (HQ)",
        LASAPI_LDAC_MODE_660 => "660 kbps (SQ)",
        LASAPI_LDAC_MODE_330 => "330 kbps (MQ)",
        _ => "Adaptive ABR",
    }
}

fn format_bt_caps(mask: u32) -> String {
    let mut parts = Vec::new();
    if (mask & LASAPI_BT_CAP_LDAC) != 0 {
        parts.push("LDAC");
    }
    if (mask & LASAPI_BT_CAP_APTX_HD) != 0 {
        parts.push("aptX HD");
    }
    if (mask & LASAPI_BT_CAP_APTX) != 0 {
        parts.push("aptX");
    }
    if (mask & LASAPI_BT_CAP_AAC) != 0 {
        parts.push("AAC");
    }
    if (mask & LASAPI_BT_CAP_SBC) != 0 {
        parts.push("SBC");
    }
    if parts.is_empty() {
        "Probing / None".to_string()
    } else {
        parts.join(", ")
    }
}

fn cmd_status() -> i32 {
    let Some(st) = get_full_status() else {
        eprintln!(
            "Error: Could not read status from lasapi-audiodg on {}",
            get_socket_path()
        );
        return 1;
    };

    let denom = f64::from(if st.hw_sample_rate != 0 {
        st.hw_sample_rate
    } else {
        48_000
    });
    let period_ms = (1000.0 * f64::from(st.hw_period_frames)) / denom;
    let buffer_ms = (1000.0 * f64::from(st.hw_buffer_frames)) / denom;

    const DYN_NAMES: [&str; 3] = [
        "Static Parametric EQ",
        "Dynamic EQ Compress (Downward Peak Control)",
        "Dynamic EQ Expand (Upward Transient Punch)",
    ];
    const SHAPE_TAGS: [&str; 3] = ["PK", "LS", "HS"];
    const BAND_LABELS: [&str; LASAPI_EQ_BANDS] = [
        "31Hz", "62Hz", "125Hz", "250Hz", "500Hz", "1kHz", "2kHz", "4kHz", "8kHz", "16kHz",
    ];

    let hw_device = c_str_to_string(&st.hw_device);
    let act_pb_raw = c_str_to_string(&st.active_playback_id);
    let act_pb = if act_pb_raw.is_empty() {
        "auto"
    } else {
        act_pb_raw.as_str()
    };
    let hw_cap_raw = c_str_to_string(&st.hw_capture_device);
    let hw_cap = if hw_cap_raw.is_empty() {
        "No Capture Device"
    } else {
        hw_cap_raw.as_str()
    };
    let act_cap_raw = c_str_to_string(&st.active_capture_id);
    let act_cap = if act_cap_raw.is_empty() {
        "none"
    } else {
        act_cap_raw.as_str()
    };

    println!("================================================================================");
    println!(
        " Salsa {} — Audio Engine (Fixed/Split Clock + HW Analog)",
        salsa_version()
    );
    println!(" {}", one_line_legal_notice("lasapi-ctl"));
    println!("================================================================================");
    println!(" Render / Sink Endpoint : {hw_device} [{act_pb}]");
    println!(" Capture / Mic Endpoint : {hw_cap} [{act_cap}]");
    println!(
        " Hardware Clock & Depth : {} Hz (Fixed Master) | {} Channels | {}-bit PCM",
        st.hw_sample_rate, st.hw_channels, st.hw_format_bits
    );
    let cap_hz = st.effective_capture_rate_hz();
    if cap_hz != st.hw_sample_rate {
        println!(
            " Capture / Mic Clock    : {} Hz (Independent Mic Clock)",
            cap_hz
        );
    }
    println!(
        " Quantum / Latency      : {} frames ({period_ms:.2} ms period / {buffer_ms:.2} ms buffer)",
        st.hw_period_frames
    );
    let pa_state = if st.pa_server_running != 0 {
        "Active (PipeWire Bypassed)"
    } else {
        "Offline"
    };
    let pa_plural = if st.pa_clients == 1 { "" } else { "s" };
    println!(
        " Native PA Socket       : {pa_state} ($XDG_RUNTIME_DIR/pulse/native | {} connected client{pa_plural})",
        st.pa_clients
    );

    let bt_alias = c_str_to_string(&st.bt_alias);
    if st.bt_active != 0 || !bt_alias.is_empty() {
        let bt_caps = format_bt_caps(st.bt_codec_cfg.supported_codecs_mask);
        let bt_codec_raw = c_str_to_string(&st.bt_codec);
        let bt_sco_codec = c_str_to_string(&st.bt_codec_cfg.bt_sco_codec);
        let neg_reason_raw = c_str_to_string(&st.bt_codec_cfg.neg_reason);
        let alias_disp = if bt_alias.is_empty() {
            "BlueZ Device"
        } else {
            bt_alias.as_str()
        };
        let codec_disp = if bt_codec_raw.is_empty() {
            "A2DP"
        } else {
            bt_codec_raw.as_str()
        };
        let abr_disp = if st.bt_codec_cfg.wireless_abr_enabled != 0 {
            "ON"
        } else {
            "OFF"
        };
        let sink_disp = if st.bt_active != 0 {
            "Active Sink"
        } else {
            "Standby"
        };
        println!(
            " Bluetooth A2DP Native  : {alias_disp} | Codec: {codec_disp} @ {} kbps (ABR:{abr_disp}) | Delay: {} ms ({sink_disp})",
            st.bt_codec_cfg.effective_kbps, st.bt_delay_ms
        );
        let sco_state = if st.bt_codec_cfg.hfp_sco_enabled != 0 {
            if st.bt_codec_cfg.hfp_sco_active != 0 {
                "ACTIVE"
            } else {
                "Enabled/Idle"
            }
        } else {
            "OFF"
        };
        let sco_suffix = if bt_sco_codec.is_empty() {
            String::new()
        } else {
            format!(" ({bt_sco_codec})")
        };
        println!(
            "   BT Policy & HFP SCO  : Pref={} | LDAC={} | HFP Mic={sco_state}{sco_suffix}",
            bt_pref_name(st.bt_codec_cfg.preferred_codec),
            ldac_tier_name(st.bt_codec_cfg.ldac_mode),
        );
        let neg_disp = if neg_reason_raw.is_empty() {
            "Negotiation nominal"
        } else {
            neg_reason_raw.as_str()
        };
        println!("   BT Supported SEPs    : {bt_caps} | {neg_disp}");
    }
    let mut bt_q: LasapiIpcReq = zeroed_struct();
    bt_q.cmd = LASAPI_CMD_BT_CONNECTION;
    bt_q.requested_period_frames = LASAPI_BT_CONN_QUERY;
    if let Some(bt_r) = send_req_only(&bt_q) {
        let f = bt_r.session_id;
        let pol = if (f & LASAPI_BT_STAT_OPTIONAL) != 0 {
            "Optional (Not Mandatory)"
        } else {
            "Auto-Connect"
        };
        let usb_pri = if (f & LASAPI_BT_STAT_USB_MATCHES_BT) != 0 {
            " | USB Priority ACTIVE (Matching USB plugged)"
        } else {
            ""
        };
        println!("   BT Connection Policy : {pol}{usb_pri}");
    }

    println!(
        " Master Speaker Mixer   : Volume {}% ({}) [Pure Native + BlueZ AVRCP]",
        st.desktop_volume_pct,
        if st.desktop_muted != 0 {
            "MUTED"
        } else {
            "Unmuted"
        }
    );
    println!(
        " Master Mic Controls    : Gain   {}% ({}) [Direct Kernel /dev/snd/pcm Capture]",
        st.desktop_mic_volume_pct,
        if st.desktop_mic_muted != 0 {
            "MUTED"
        } else {
            "Unmuted"
        }
    );

    {
        const AMP_NAMES: [&str; 4] = [
            "Auto (Preserve)",
            "Low (IEM)",
            "Med (64-150R)",
            "High (600R TPA6120A2)",
        ];
        const ROUTE_NAMES: [&str; 3] = ["Auto (Preserve)", "Headphones", "Line/Speakers"];
        let at = if st.hw_analog_cfg.hp_amp_tier <= 3 {
            st.hw_analog_cfg.hp_amp_tier as usize
        } else {
            0
        };
        let rt = if st.hw_analog_cfg.output_route <= 2 {
            st.hw_analog_cfg.output_route as usize
        } else {
            0
        };
        let arch_raw = c_str_to_string(&st.hw_analog_info.card_arch_name);
        let arch_disp = if arch_raw.is_empty() {
            "Virtual/Unprobed"
        } else {
            arch_raw.as_str()
        };
        let unity_disp = if st.hw_analog_info.dac_unity_active != 0 {
            "ACTIVE (Bit-Perfect PCM)"
        } else if st.hw_analog_cfg.hw_dac_unity_vol != 0 {
            "Standby/Software Gain"
        } else {
            "Disabled (Software)"
        };
        let dac_ctl_raw = c_str_to_string(&st.hw_analog_info.dac_ctl_name);
        let dac_ctl_disp = if dac_ctl_raw.is_empty() {
            "none"
        } else {
            dac_ctl_raw.as_str()
        };
        println!(" HW Analog Stage        : {arch_disp} | DAC Unity: {unity_disp} ({dac_ctl_disp})");
        println!(
            "   Amp / Route / Rails  : Gain={} | Route={} | 0dB Rails={} | SoftKnee={:+.1}dB | MicBoost={:+.1}dB{}",
            AMP_NAMES[at],
            ROUTE_NAMES[rt],
            if st.hw_analog_cfg.unlock_0db_rails != 0 {
                "UNLOCKED"
            } else {
                "Preserve"
            },
            st.hw_analog_cfg.analog_boost_db,
            st.hw_analog_cfg.mic_hw_boost_db,
            if st.hw_analog_info.hid_box_connected != 0 {
                " | Strix HID: Connected"
            } else {
                ""
            }
        );
        let diag_note = c_str_to_string(&st.hw_analog_info.diag_note);
        if !diag_note.is_empty() {
            println!("   HW Analog Telemetry  : {diag_note}");
        }
    }

    let midi_dev_raw = c_str_to_string(&st.midi_device);
    let midi_dev = if midi_dev_raw.is_empty() {
        "LASAPI-MIDI-Hub"
    } else {
        midi_dev_raw.as_str()
    };
    println!(
        " DAW MIDI Hub & HW      : {midi_dev} (MIDI Key Presses: {})",
        st.midi_events_routed
    );
    println!(
        " MMCSS Real-Time Thread : {} (Priority: {})",
        if st.realtime_priority != 0 {
            "SCHED_FIFO Active"
        } else {
            "Standard CFS"
        },
        st.realtime_priority
    );
    println!(
        " Engine Mode            : {}",
        if st.exclusive_owner_id >= 0 {
            "EXCLUSIVE MODE (Hardware Locked)"
        } else {
            "SHARED MODE (Multi-Stream APO Mixer)"
        }
    );
    println!(
        " Total Period Cycles    : {} (Hardware XRUNs: {})",
        st.total_cycles, st.hw_xruns
    );
    println!("--------------------------------------------------------------------------------");
    println!(" Salsa 10-Band Parametric Dynamic EQ APO:");
    let dyn_idx = if st.eq_cfg.dynamic_mode <= 2 {
        st.eq_cfg.dynamic_mode as usize
    } else {
        0
    };
    println!(
        "   Status / Mode        : {} | {}",
        if st.eq_cfg.enabled != 0 {
            "ACTIVE"
        } else {
            "BYPASSED"
        },
        DYN_NAMES[dyn_idx]
    );
    let preset_raw = c_str_to_string(&st.eq_cfg.preset_name);
    let preset_disp = if preset_raw.is_empty() {
        "Custom"
    } else {
        preset_raw.as_str()
    };
    println!(
        "   Preset & Dynamics    : {preset_disp} | Preamp: {:+.1} dB | Thresh: {:.1} dB | Ratio: {:.1}:1 | Limiter: {}",
        st.eq_cfg.preamp_db,
        st.eq_cfg.dyn_threshold_db,
        st.eq_cfg.dyn_ratio,
        if st.eq_cfg.limiter_enabled != 0 {
            "ON"
        } else {
            "OFF"
        }
    );
    let cf_cut = if st.param_eq.crossfeed_cutoff_hz != 0 {
        st.param_eq.crossfeed_cutoff_hz
    } else {
        700
    };
    println!(
        "   Crossfeed (Bauer)    : {} (Level: {:.0}%, Cutoff: {cf_cut} Hz)",
        if st.param_eq.crossfeed_enabled != 0 {
            "ON"
        } else {
            "OFF"
        },
        st.param_eq.crossfeed_level * 100.0
    );
    print!("   Parametric Bands     :");
    for b in 0..LASAPI_EQ_BANDS {
        let sh = if st.param_eq.band_type[b] <= 2 {
            st.param_eq.band_type[b] as usize
        } else {
            0
        };
        let freq = if st.param_eq.band_freq_hz[b] > 1.0 {
            st.param_eq.band_freq_hz[b]
        } else {
            0.0
        };
        let q = if st.eq_cfg.band_q[b] > 0.05 {
            st.eq_cfg.band_q[b]
        } else {
            1.414
        };
        if freq > 1.0 {
            print!(
                "\n     #{b} [{}|{freq:.0}Hz|Q={q:.2}]: {:+.1}dB",
                SHAPE_TAGS[sh], st.eq_cfg.band_gains_db[b]
            );
        } else {
            print!(" {}:{:+.1}dB", BAND_LABELS[b], st.eq_cfg.band_gains_db[b]);
        }
    }
    println!();
    println!("--------------------------------------------------------------------------------");
    println!(
        " Active Audio & PulseAudio Client Sessions ({}):",
        st.num_sessions
    );
    let n_sess = (st.num_sessions as usize).min(st.sessions.len());
    if n_sess == 0 {
        println!("   (No active client streams)");
    } else {
        for s in &st.sessions[..n_sess] {
            let cname = c_str_to_string(&s.client_name);
            let tdev = c_str_to_string(&s.target_device);
            let mode = if s.share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
                "EXCL"
            } else {
                "SHARED"
            };
            let mute_tag = if s.muted != 0 { " [MUTE]" } else { "" };
            let dir_tag = if s.is_capture != 0 { "CAPT" } else { "PLAY" };
            let act_tag = if s.active != 0 { "ACTIVE" } else { "IDLE" };
            let tgt_sfx = if tdev.is_empty() {
                String::new()
            } else {
                format!(" -> {tdev}")
            };
            println!(
                "   [#{:<5} | PID {:<7}] [{dir_tag}] {cname:<24} | {mode:<6} | {:>6} Hz {}ch | Vol:{:>3}%{mute_tag} | {act_tag:<6}{tgt_sfx}",
                s.session_id,
                s.client_pid,
                s.sample_rate,
                s.channels,
                s.volume_milli / 10,
            );
        }
    }
    println!("================================================================================");
    0
}

fn cmd_devices() -> i32 {
    let mut dlist: LasapiDeviceList = zeroed_struct();
    let mut live = false;
    if let Ok(resp) = crate::control_ipc::ControlClient::send_with_timeout(
        &get_socket_path(),
        &crate::control_ipc::ControlRequest::ListDevices,
        crate::control_ipc::DEFAULT_CONTROL_TIMEOUT,
    ) {
        if let crate::control_ipc::ControlPayload::Devices(list) = resp.payload {
            if resp.hr == S_OK {
                dlist = *list;
                live = true;
            }
        }
    }
    if !live {
        use crate::bluez_runtime::BluetoothBackend;
        crate::bluez_runtime::RealBluezBackend::global().enumerate_devices(None, None, &mut dlist);
    }

    let ndev = (dlist.num_devices as usize).min(dlist.devices.len());
    println!("Salsa Native Playback Endpoints (use 'lasapi-ctl set-sink <id>'):");
    let mut pb_cnt = 0u32;
    for e in &dlist.devices[..ndev] {
        if e.is_capture != 0 {
            continue;
        }
        let id = c_str_to_string(&e.id);
        let drv = c_str_to_string(&e.driver);
        let name = c_str_to_string(&e.name);
        let act = if e.is_active != 0 { "  [ACTIVE]" } else { "" };
        println!("  {id:<24} | {drv:<10} | {name}{act}");
        pb_cnt += 1;
    }
    if pb_cnt == 0 {
        println!("  (No playback hardware found - WaveRT Virtual Clock available)");
    }

    println!("\nSalsa Native Capture / Microphone Endpoints (use 'lasapi-ctl set-source <id>'):");
    let mut cap_cnt = 0u32;
    for e in &dlist.devices[..ndev] {
        if e.is_capture == 0 {
            continue;
        }
        let id = c_str_to_string(&e.id);
        let drv = c_str_to_string(&e.driver);
        let name = c_str_to_string(&e.name);
        let act = if e.is_active != 0 { "  [ACTIVE]" } else { "" };
        println!("  {id:<24} | {drv:<10} | {name}{act}");
        cap_cnt += 1;
    }
    if cap_cnt == 0 {
        println!("  (No kernel ALSA capture devices detected)");
    }

    println!("\nAvailable Hardware & Sequencer MIDI Endpoints for DAWs:");
    for c in 0..16 {
        let mpath = format!("/dev/snd/midiC{c}D0");
        if Path::new(&mpath).exists() {
            let id_path = format!("/proc/asound/card{c}/id");
            let card_id = fs::read_to_string(id_path)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.trim_end_matches(&['\r', '\n'][..]).to_string()))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "MIDI".to_string());
            println!("  {mpath:<18} ->  Card {c} Hardware RawMIDI ({card_id})");
        }
    }
    if Path::new("/dev/snd/seq").exists() {
        println!("  /dev/snd/seq       ->  ALSA Sequencer ('LASAPI-MIDI-Hub' + Hardware MIDI Ports)");
    }
    0
}

fn cmd_set_device(is_capture: bool, dev_id: &str) -> i32 {
    let req = crate::control_ipc::ControlRequest::SetDevices {
        playback_id: (!is_capture).then(|| dev_id.to_string()),
        capture_id: is_capture.then(|| dev_id.to_string()),
    };
    let Ok(resp) = crate::control_ipc::ControlClient::send_with_timeout(
        &get_socket_path(),
        &req,
        crate::control_ipc::DEFAULT_CONTROL_TIMEOUT,
    ) else {
        eprintln!(
            "Error: Could not connect to lasapi-audiodg on {}",
            get_socket_path()
        );
        return 1;
    };
    if resp.hr != S_OK {
        eprintln!(
            "Error: Failed to switch {} device to '{dev_id}'",
            if is_capture { "capture" } else { "playback" }
        );
        return 1;
    }
    println!(
        "Switched Salsa active {} endpoint to: {dev_id}",
        if is_capture { "capture" } else { "playback" }
    );
    0
}

fn cmd_bt_reset(mac_or_empty: &str) -> i32 {
    let mut target_hint = String::new();
    if !mac_or_empty.is_empty() {
        target_hint = mac_or_empty.to_string();
    } else if let Some(st) = get_full_status() {
        let pb = c_str_to_string(&st.active_playback_id);
        if pb.len() >= 6 && pb[..6].eq_ignore_ascii_case("bluez:") {
            target_hint = pb;
        }
    }

    let (rc, msg) = crate::bluez_runtime::reset_device_prefer_daemon(&target_hint);
    if rc == 0 {
        println!(
            "{}",
            if msg.is_empty() {
                "Bluetooth audio reset completed."
            } else {
                msg.as_str()
            }
        );
        return 0;
    }
    eprintln!(
        "Error: {}",
        if msg.is_empty() {
            "Bluetooth reset failed."
        } else {
            msg.as_str()
        }
    );
    if rc == -2 { 2 } else { 1 }
}

fn cmd_volume(session_id: u32, mut vol: f32) -> i32 {
    if vol > 2.0 {
        vol /= 100.0;
    }
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_SET_VOLUME;
    req.target_session_id = session_id;
    req.volume_milli = (vol * 1000.0) as u32;
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    println!(
        "Session #{session_id} volume set to {:.0}% (HRESULT=0x{:08x})",
        vol * 100.0,
        resp.hr as u32
    );
    i32::from(resp.hr != S_OK)
}

fn cmd_session_control(session_id: u32, vol_pct: i32, mute_mode: i32, dev_id: Option<&str>) -> i32 {
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_SET_SESSION_CONTROL;
    req.target_session_id = session_id;
    let clamped_pct = vol_pct.clamp(0, 150);
    req.volume_milli = if vol_pct >= 0 {
        (clamped_pct as u32) * 10
    } else {
        u32::MAX
    };
    req.share_mode = if (0..=2).contains(&mute_mode) {
        (mute_mode + 1) as u32
    } else {
        0
    };
    if let Some(d) = dev_id {
        if !d.is_empty() {
            copy_str_to_c(&mut req.target_device, d);
        }
    }
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    let hr = resp.hr as u32;
    println!(
        "Session #{session_id} updated (vol={clamped_pct_disp}% mute={mute_mode} dev={}) -> HRESULT=0x{hr:08x}",
        dev_id.filter(|d| !d.is_empty()).unwrap_or("-"),
        clamped_pct_disp = if vol_pct >= 0 { clamped_pct } else { vol_pct },
    );
    i32::from(resp.hr != S_OK)
}

fn cmd_bt_link(mode: &str, mac: &str) -> i32 {
    let action = match mode {
        "" | "status" | "query" => LASAPI_BT_CONN_QUERY,
        "on" | "connect" => LASAPI_BT_CONN_CONNECT,
        "off" | "disconnect" => LASAPI_BT_CONN_DISCONNECT,
        "optional" => LASAPI_BT_CONN_SET_OPTIONAL,
        "auto" | "mandatory" => LASAPI_BT_CONN_SET_AUTOCONN,
        other => {
            eprintln!("Unknown bt-link mode '{other}'. Use status|on|off|optional|auto.");
            return 2;
        }
    };
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_BT_CONNECTION;
    req.requested_period_frames = action;
    if !mac.is_empty() {
        copy_str_to_c(&mut req.target_device, mac);
    }
    let Some(resp) = send_req_only(&req) else {
        eprintln!("Error: could not contact lasapi-audiodg for bt-link");
        return 1;
    };
    let flags = resp.session_id;
    let opt = (flags & LASAPI_BT_STAT_OPTIONAL) != 0;
    let conn = (flags & LASAPI_BT_STAT_CONNECTED) != 0;
    let tp = (flags & LASAPI_BT_STAT_TRANSPORT) != 0;
    let usb_pri = (flags & LASAPI_BT_STAT_USB_MATCHES_BT) != 0;
    println!(
        "Bluetooth Link Policy={} | Connected={} | A2DP Transport={} | Dual USB+BT Priority={}",
        if opt { "Optional" } else { "Auto-Connect" },
        if conn { "Yes" } else { "No" },
        if tp { "Active" } else { "Idle" },
        if usb_pri { "USB-First" } else { "None" },
    );
    i32::from(resp.hr != S_OK)
}

fn cmd_master_volume(pct: i32, mute_mode: i32) -> i32 {
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_SET_VOLUME;
    req.target_session_id = u32::MAX;
    req.volume_milli = if pct >= 0 { (pct as u32) * 10 } else { u32::MAX };
    req.share_mode = (mute_mode + 1) as u32;
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    println!(
        "Desktop Master Audio Controls updated (HRESULT=0x{:08x})",
        resp.hr as u32
    );
    i32::from(resp.hr != S_OK)
}

fn cmd_mic_volume(pct: i32, mute_mode: i32) -> i32 {
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_SET_VOLUME;
    req.target_session_id = 0xFFFF_FFFE;
    req.volume_milli = if pct >= 0 { (pct as u32) * 10 } else { u32::MAX };
    req.share_mode = (mute_mode + 1) as u32;
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    println!(
        "Desktop Microphone Controls updated (HRESULT=0x{:08x})",
        resp.hr as u32
    );
    i32::from(resp.hr != S_OK)
}

fn cmd_midi_send(note: u8, vel: u8) -> i32 {
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_INJECT_MIDI;
    copy_str_to_c(&mut req.client_name, "lasapi-ctl");
    req.midi_ev.size = 3;
    req.midi_ev.data[0] = 0x90;
    req.midi_ev.data[1] = note & 0x7F;
    req.midi_ev.data[2] = vel & 0x7F;
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    println!(
        "Injected MIDI Note-On (status=0x90, note={note}, velocity={vel}) -> HRESULT=0x{:08x}",
        resp.hr as u32
    );
    i32::from(resp.hr != S_OK)
}

fn cmd_set_rate(rate: u32, period: u32, bits: u32) -> i32 {
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_SET_ENGINE_CONFIG;
    req.sample_rate = rate;
    req.requested_period_frames = period;
    req.format = bits;
    req.stream_flags = 0;
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    let mut actual_bits = bits;
    if resp.hr == S_OK {
        if let Some(st) = get_full_status() {
            actual_bits = st.hw_format_bits;
        }
    }
    let disp_bits = if actual_bits != 0 {
        actual_bits
    } else if bits != 0 {
        bits
    } else {
        32
    };
    let rate_locked = rate != 0 && resp.actual_sample_rate != 0 && resp.actual_sample_rate != rate;
    let bits_locked = bits != 0 && actual_bits != 0 && actual_bits != bits;
    let per_locked =
        period != 0 && resp.actual_period_frames != 0 && resp.actual_period_frames != period;
    if rate_locked || bits_locked || per_locked {
        println!(
            "Salsa Engine Clock -> {} Hz | Period: {} frames | {disp_bits}-bit (HW Locked; requested {} Hz / {} frames / {}-bit) (HRESULT=0x{:08x})",
            resp.actual_sample_rate,
            resp.actual_period_frames,
            if rate != 0 { rate } else { resp.actual_sample_rate },
            if period != 0 { period } else { resp.actual_period_frames },
            if bits != 0 { bits } else { disp_bits },
            resp.hr as u32
        );
    } else {
        println!(
            "Salsa Engine Clock -> {} Hz | Period: {} frames | {disp_bits}-bit (HRESULT=0x{:08x})",
            resp.actual_sample_rate,
            resp.actual_period_frames,
            resp.hr as u32
        );
    }
    i32::from(resp.hr != S_OK)
}

fn cmd_set_cap_rate(rate: u32) -> i32 {
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_SET_ENGINE_CONFIG;
    req.sample_rate = rate;
    req.stream_flags = LASAPI_CFG_FLAG_CAPTURE_RATE;
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    println!(
        "Salsa Capture / Mic Clock -> {} Hz ({}) (HRESULT=0x{:08x})",
        if resp.actual_sample_rate != 0 {
            resp.actual_sample_rate
        } else {
            rate
        },
        if rate == 0 {
            "Auto (Follow Master Clock)"
        } else {
            "Fixed Independent Mic Clock"
        },
        resp.hr as u32
    );
    i32::from(resp.hr != S_OK)
}

fn send_eq(eq: &LasapiEqConfig) -> i32 {
    let mut req: LasapiIpcReq = zeroed_struct();
    req.cmd = LASAPI_CMD_SET_EQ;
    req.eq_cfg = *eq;
    let Some(resp) = send_req_only(&req) else {
        return 1;
    };
    i32::from(resp.hr != S_OK)
}

fn cmd_eq_preset(preset: &str) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut eq = st.eq_cfg;
    let mut ext = st.param_eq;
    if !crate::settings::apply_builtin_eq_preset(preset, &mut eq, &mut ext) {
        eprintln!(
            "Unknown preset '{preset}'. Use: flat, bose, bass, hires, gaming, vocal, loudness"
        );
        return 1;
    }

    let pay = crate::c_abi::LasapiParamEqPayload { eq, ext };
    let (rc, _) = send_payload_cmd(crate::c_abi::LASAPI_CMD_SET_PARAM_EQ, &pay.to_bytes());
    println!(
        "Applied Salsa Dynamic EQ preset: '{}' (rc={rc})",
        c_str_to_string(&eq.preset_name)
    );
    rc
}

fn cmd_eq_band(band: i32, gain_db: f32) -> i32 {
    if !(0..LASAPI_EQ_BANDS as i32).contains(&band) {
        eprintln!("Band index must be 0..9");
        return 1;
    }
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut eq = st.eq_cfg;
    eq.enabled = 1;
    eq.band_gains_db[band as usize] = gain_db;
    copy_str_to_c(&mut eq.preset_name, "Custom Salsa Curve");
    let rc = send_eq(&eq);
    println!("Updated Salsa EQ Band #{band} to {gain_db:+.1} dB");
    rc
}

fn cmd_eq_mode(mode_str: &str, thresh: f32, ratio: f32) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut eq = st.eq_cfg;
    if mode_str.eq_ignore_ascii_case("off") || mode_str.eq_ignore_ascii_case("bypass") {
        eq.enabled = 0;
    } else if mode_str.eq_ignore_ascii_case("static") {
        eq.enabled = 1;
        eq.dynamic_mode = 0;
    } else if mode_str.eq_ignore_ascii_case("compress") {
        eq.enabled = 1;
        eq.dynamic_mode = 1;
    } else if mode_str.eq_ignore_ascii_case("expand") {
        eq.enabled = 1;
        eq.dynamic_mode = 2;
    } else {
        eprintln!("Unknown mode '{mode_str}'. Use: off, static, compress, expand");
        return 1;
    }
    if thresh < 0.0 {
        eq.dyn_threshold_db = thresh;
    }
    if ratio >= 1.0 {
        eq.dyn_ratio = ratio;
    }
    let rc = send_eq(&eq);
    println!(
        "Updated Salsa Dynamic EQ Mode -> {mode_str} (Threshold: {:.1} dB, Ratio: {:.1}:1)",
        eq.dyn_threshold_db, eq.dyn_ratio
    );
    rc
}

fn cmd_bt_codecs() -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let caps = format_bt_caps(st.bt_codec_cfg.supported_codecs_mask);
    let bt_alias = c_str_to_string(&st.bt_alias);
    let bt_codec = c_str_to_string(&st.bt_codec);
    let sco_codec = c_str_to_string(&st.bt_codec_cfg.bt_sco_codec);
    let neg_reason = c_str_to_string(&st.bt_codec_cfg.neg_reason);
    println!(
        "Bluetooth Device : {}",
        if bt_alias.is_empty() {
            "(None)"
        } else {
            bt_alias.as_str()
        }
    );
    println!(
        "Active Codec     : {} @ {} kbps (ABR: {})",
        if bt_codec.is_empty() {
            "None"
        } else {
            bt_codec.as_str()
        },
        st.bt_codec_cfg.effective_kbps,
        if st.bt_codec_cfg.wireless_abr_enabled != 0 {
            "ON"
        } else {
            "OFF"
        }
    );
    println!(
        "Preferred Codec  : {}",
        bt_pref_name(st.bt_codec_cfg.preferred_codec)
    );
    println!(
        "LDAC Tier        : {}",
        ldac_tier_name(st.bt_codec_cfg.ldac_mode)
    );
    println!("Supported SEPs   : {caps}");
    println!(
        "HFP SCO Mic      : {}{}",
        if st.bt_codec_cfg.hfp_sco_enabled != 0 {
            if st.bt_codec_cfg.hfp_sco_active != 0 {
                "ACTIVE"
            } else {
                "Enabled"
            }
        } else {
            "OFF"
        },
        if sco_codec.is_empty() {
            String::new()
        } else {
            format!(" ({sco_codec})")
        }
    );
    println!(
        "Diagnostics      : {}",
        if neg_reason.is_empty() {
            "Nominal"
        } else {
            neg_reason.as_str()
        }
    );
    0
}

fn cmd_set_codec(codec_str: &str, tier_str: &str) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut cfg = st.bt_codec_cfg;
    if codec_str.eq_ignore_ascii_case("auto") {
        cfg.preferred_codec = LASAPI_BT_PREF_AUTO;
    } else if codec_str.eq_ignore_ascii_case("ldac") {
        cfg.preferred_codec = LASAPI_BT_PREF_LDAC;
    } else if codec_str.eq_ignore_ascii_case("aptxhd") || codec_str.eq_ignore_ascii_case("aptx_hd") {
        cfg.preferred_codec = LASAPI_BT_PREF_APTX_HD;
    } else if codec_str.eq_ignore_ascii_case("aptx") {
        cfg.preferred_codec = LASAPI_BT_PREF_APTX;
    } else if codec_str.eq_ignore_ascii_case("aac") {
        cfg.preferred_codec = LASAPI_BT_PREF_AAC;
    } else if codec_str.eq_ignore_ascii_case("sbc") {
        cfg.preferred_codec = LASAPI_BT_PREF_SBC;
    } else {
        eprintln!("Unknown BlueZ codec '{codec_str}'. Use: auto|ldac|aptxhd|aptx|aac|sbc");
        return 1;
    }

    if !tier_str.is_empty() {
        if tier_str.eq_ignore_ascii_case("abr") || tier_str.eq_ignore_ascii_case("adaptive") {
            cfg.ldac_mode = LASAPI_LDAC_MODE_ADAPTIVE;
            cfg.wireless_abr_enabled = 1;
        } else if tier_str == "990" {
            cfg.ldac_mode = LASAPI_LDAC_MODE_990;
        } else if tier_str == "660" {
            cfg.ldac_mode = LASAPI_LDAC_MODE_660;
        } else if tier_str == "330" {
            cfg.ldac_mode = LASAPI_LDAC_MODE_330;
        }
    }

    let (rc, resp) = send_payload_cmd(LASAPI_CMD_SET_BT_CODEC, &cfg.to_bytes());
    println!(
        "BlueZ Codec Switcher -> {} (LDAC: {}, ABR: {}, HRESULT=0x{:08x})",
        bt_pref_name(cfg.preferred_codec),
        ldac_tier_name(cfg.ldac_mode),
        if cfg.wireless_abr_enabled != 0 {
            "ON"
        } else {
            "OFF"
        },
        resp.hr as u32
    );
    rc
}

fn cmd_bt_sco(mode_str: &str) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut cfg = st.bt_codec_cfg;
    cfg.hfp_sco_enabled = u32::from(mode_str.eq_ignore_ascii_case("on") || mode_str == "1");
    let (rc, resp) = send_payload_cmd(LASAPI_CMD_SET_BT_CODEC, &cfg.to_bytes());
    println!(
        "Bluetooth HFP BTPROTO_SCO Headset Mic -> {} (HRESULT=0x{:08x})",
        if cfg.hfp_sco_enabled != 0 {
            "ENABLED"
        } else {
            "DISABLED"
        },
        resp.hr as u32
    );
    rc
}

fn cmd_hw_analog(args: &[String]) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut cfg: LasapiHwAnalogCfg = st.hw_analog_cfg;

    if args.len() == 2 || (args.len() >= 3 && args[2].eq_ignore_ascii_case("status")) {
        let arch = c_str_to_string(&st.hw_analog_info.card_arch_name);
        let dac_nm = c_str_to_string(&st.hw_analog_info.dac_ctl_name);
        let amp_nm = c_str_to_string(&st.hw_analog_info.amp_ctl_name);
        let cap_nm = c_str_to_string(&st.hw_analog_info.cap_ctl_name);
        let diag = c_str_to_string(&st.hw_analog_info.diag_note);
        println!(
            "Card Architecture : {} (PB Card={}, Cap Card={})",
            if arch.is_empty() {
                "Virtual/None"
            } else {
                arch.as_str()
            },
            st.hw_analog_info.active_pb_card,
            st.hw_analog_info.active_cap_card
        );
        println!(
            "DAC Unity Mode    : {} (Active={}, Elem='{}')",
            if cfg.hw_dac_unity_vol != 0 {
                "ON"
            } else {
                "OFF"
            },
            if st.hw_analog_info.dac_unity_active != 0 {
                "YES"
            } else {
                "NO"
            },
            if dac_nm.is_empty() {
                "none"
            } else {
                dac_nm.as_str()
            }
        );
        println!(
            "HP Amp / Route    : tier={} | route={} | unlock_0db={} (Elem='{}')",
            cfg.hp_amp_tier,
            cfg.output_route,
            cfg.unlock_0db_rails,
            if amp_nm.is_empty() {
                "none"
            } else {
                amp_nm.as_str()
            }
        );
        println!(
            "Analog Boost      : {:+.1} dB | Mic HW Boost: {:+.1} dB (Elem='{}') | Strix HID={}",
            cfg.analog_boost_db,
            cfg.mic_hw_boost_db,
            if cap_nm.is_empty() {
                "none"
            } else {
                cap_nm.as_str()
            },
            st.hw_analog_info.hid_box_connected
        );
        if !diag.is_empty() {
            println!("Telemetry Note    : {diag}");
        }
        return 0;
    }

    if args[2].eq_ignore_ascii_case("reset") {
        cfg.hw_dac_unity_vol = 1;
        cfg.hp_amp_tier = LASAPI_HW_AMP_AUTO;
        cfg.output_route = LASAPI_HW_ROUTE_AUTO;
        cfg.unlock_0db_rails = 0;
        cfg.analog_boost_db = 0.0;
        cfg.mic_hw_boost_db = 0.0;
    } else {
        let mut i = 2usize;
        while i + 1 < args.len() {
            let k = args[i].as_str();
            let v = args[i + 1].as_str();
            if k.eq_ignore_ascii_case("dac") {
                cfg.hw_dac_unity_vol = u32::from(v.eq_ignore_ascii_case("on") || v == "1");
            } else if k.eq_ignore_ascii_case("amp") {
                if v.eq_ignore_ascii_case("auto") || v == "0" {
                    cfg.hp_amp_tier = LASAPI_HW_AMP_AUTO;
                } else if v.eq_ignore_ascii_case("low") || v == "1" {
                    cfg.hp_amp_tier = LASAPI_HW_AMP_LOW;
                } else if v.eq_ignore_ascii_case("med")
                    || v.eq_ignore_ascii_case("medium")
                    || v == "2"
                {
                    cfg.hp_amp_tier = LASAPI_HW_AMP_MED;
                } else if v.eq_ignore_ascii_case("high")
                    || v.eq_ignore_ascii_case("600")
                    || v == "3"
                {
                    cfg.hp_amp_tier = LASAPI_HW_AMP_HIGH;
                } else {
                    eprintln!("Unknown amp tier '{v}'. Use: auto|low|med|high");
                    return 1;
                }
            } else if k.eq_ignore_ascii_case("route") {
                if v.eq_ignore_ascii_case("auto") || v == "0" {
                    cfg.output_route = LASAPI_HW_ROUTE_AUTO;
                } else if v.eq_ignore_ascii_case("hp")
                    || v.eq_ignore_ascii_case("headphones")
                    || v == "1"
                {
                    cfg.output_route = LASAPI_HW_ROUTE_HP;
                } else if v.eq_ignore_ascii_case("spk")
                    || v.eq_ignore_ascii_case("speakers")
                    || v.eq_ignore_ascii_case("line")
                    || v == "2"
                {
                    cfg.output_route = LASAPI_HW_ROUTE_SPK;
                } else {
                    eprintln!("Unknown route '{v}'. Use: auto|hp|spk");
                    return 1;
                }
            } else if k.eq_ignore_ascii_case("rails") {
                cfg.unlock_0db_rails = u32::from(v.eq_ignore_ascii_case("on") || v == "1");
            } else if k.eq_ignore_ascii_case("boost") {
                cfg.analog_boost_db = c_strtof(v).clamp(0.0, 12.0);
            } else if k.eq_ignore_ascii_case("mic-boost") || k.eq_ignore_ascii_case("micboost") {
                cfg.mic_hw_boost_db = c_strtof(v).clamp(0.0, 24.0);
            } else {
                eprintln!(
                    "Unknown hw-analog option '{k}'. Use: dac|amp|route|rails|boost|mic-boost|reset"
                );
                return 1;
            }
            i += 2;
        }
    }

    let (rc, resp) = send_payload_cmd(LASAPI_CMD_SET_HW_ANALOG, &cfg.to_bytes());
    println!(
        "HW Analog Architecture -> DAC={} Amp={} Route={} 0dB-Rails={} SoftKnee={:+.1}dB MicBoost={:+.1}dB (HRESULT=0x{:08x})",
        if cfg.hw_dac_unity_vol != 0 {
            "ON"
        } else {
            "OFF"
        },
        cfg.hp_amp_tier,
        cfg.output_route,
        if cfg.unlock_0db_rails != 0 {
            "UNLOCK"
        } else {
            "Preserve"
        },
        cfg.analog_boost_db,
        cfg.mic_hw_boost_db,
        resp.hr as u32
    );
    rc
}

fn cmd_param_eq(band: i32, freq: f32, q: f32, shape: &str, gain: f32) -> i32 {
    if !(0..LASAPI_EQ_BANDS as i32).contains(&band) {
        eprintln!("Band index must be 0..{}", LASAPI_EQ_BANDS - 1);
        return 1;
    }
    let Some(st) = get_full_status() else {
        return 1;
    };
    let b = band as usize;
    let mut pl = LasapiParamEqPayload {
        eq: st.eq_cfg,
        ext: st.param_eq,
    };
    pl.eq.enabled = 1;
    pl.eq.band_gains_db[b] = gain;
    pl.eq.band_q[b] = if (0.2..=10.0).contains(&q) { q } else { 1.414 };
    pl.ext.band_freq_hz[b] = if (15.0..=22_000.0).contains(&freq) {
        freq
    } else {
        1000.0
    };
    pl.ext.band_type[b] =
        if shape.eq_ignore_ascii_case("lowshelf") || shape.eq_ignore_ascii_case("ls") {
            LASAPI_FILTER_LOW_SHELF
        } else if shape.eq_ignore_ascii_case("highshelf") || shape.eq_ignore_ascii_case("hs") {
            LASAPI_FILTER_HIGH_SHELF
        } else {
            LASAPI_FILTER_PEAK
        };
    copy_str_to_c(&mut pl.eq.preset_name, "Custom Parametric");

    let (rc, resp) = send_payload_cmd(LASAPI_CMD_SET_PARAM_EQ, &pl.to_bytes());
    println!(
        "Parametric Band #{band} -> {:.1} Hz | Q={:.2} | {shape} | {gain:+.1} dB (HRESULT=0x{:08x})",
        pl.ext.band_freq_hz[b],
        pl.eq.band_q[b],
        resp.hr as u32
    );
    rc
}

fn cmd_crossfeed(mode_str: &str, level_pct: f32, cutoff_hz: u32) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut pl = LasapiParamEqPayload {
        eq: st.eq_cfg,
        ext: st.param_eq,
    };
    pl.ext.crossfeed_enabled = u32::from(mode_str.eq_ignore_ascii_case("on") || mode_str == "1");
    if (0.0..=100.0).contains(&level_pct) {
        pl.ext.crossfeed_level = level_pct / 100.0;
    }
    if (200..=2000).contains(&cutoff_hz) {
        pl.ext.crossfeed_cutoff_hz = cutoff_hz;
    }
    let (rc, resp) = send_payload_cmd(LASAPI_CMD_SET_PARAM_EQ, &pl.to_bytes());
    println!(
        "Bauer Headphone Crossfeed -> {} (Level: {:.0}%, Cutoff: {} Hz, HRESULT=0x{:08x})",
        if pl.ext.crossfeed_enabled != 0 {
            "ON"
        } else {
            "OFF"
        },
        pl.ext.crossfeed_level * 100.0,
        pl.ext.crossfeed_cutoff_hz,
        resp.hr as u32
    );
    rc
}

fn cmd_mic_apo(mode_str: &str, rnn_arg: i32, gate_db: f32, side_pct: i32) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut cfg = st.mic_apo;
    cfg.enabled = u32::from(mode_str.eq_ignore_ascii_case("on") || mode_str == "1");
    if rnn_arg >= 0 {
        cfg.rnnoise_enabled = u32::from(rnn_arg != 0);
    }
    if (-80.0..=0.0).contains(&gate_db) {
        cfg.gate_enabled = 1;
        cfg.gate_thresh_db = gate_db;
    }
    if (0..=100).contains(&side_pct) {
        cfg.sidetone_enabled = u32::from(side_pct > 0);
        cfg.sidetone_pct = side_pct as u32;
    }
    let (rc, resp) = send_payload_cmd(LASAPI_CMD_SET_MIC_APO, &cfg.to_bytes());
    println!(
        "Capture Mic APO -> {} | RNNoise: {} | Gate: {:.1} dB | Sidetone: {} ({}%) (HRESULT=0x{:08x})",
        if cfg.enabled != 0 { "ACTIVE" } else { "BYPASSED" },
        if cfg.rnnoise_enabled != 0 { "ON" } else { "OFF" },
        cfg.gate_thresh_db,
        if cfg.sidetone_enabled != 0 { "ON" } else { "OFF" },
        cfg.sidetone_pct,
        resp.hr as u32
    );
    rc
}

fn cmd_mic_preset(preset: &str) -> i32 {
    let Some(st) = get_full_status() else {
        return 1;
    };
    let mut cfg = st.mic_apo;
    if !crate::settings::apply_mic_apo_preset(preset, &mut cfg) {
        eprintln!("Unknown mic preset '{preset}'. Use: discord, broadcast, clean, bypass");
        return 1;
    }

    let (rc, resp) = send_payload_cmd(LASAPI_CMD_SET_MIC_APO, &cfg.to_bytes());
    println!(
        "Mic APO Preset '{preset}' -> {} | HPF {:.0}Hz | Gate {:.0}dB | RNNoise {} | Clarity {:+.1}dB (HRESULT=0x{:08x})",
        if cfg.enabled != 0 { "ACTIVE" } else { "BYPASSED" },
        cfg.hpf_hz,
        cfg.gate_thresh_db,
        if cfg.rnnoise_enabled != 0 { "ON" } else { "OFF" },
        cfg.clarity_db,
        resp.hr as u32
    );
    rc
}

pub fn install_root_from_exe(exe_path: &str) -> String {
    let mut path = Path::new(exe_path).to_path_buf();
    let is_ctl = path.file_name().and_then(|s| s.to_str()) == Some("lasapi-ctl");
    if !path.pop() {
        return ".".to_string();
    }
    if is_ctl && path.file_name().and_then(|s| s.to_str()) == Some("build") {
        let _ = path.pop();
    }
    path.to_string_lossy().into_owned()
}

fn get_install_root() -> String {
    fs::read_link("/proc/self/exe")
        .ok()
        .map(|p| install_root_from_exe(&p.to_string_lossy()))
        .unwrap_or_else(|| ".".to_string())
}

fn cmd_pw_self_test() -> i32 {
    let dsock = get_debug_socket_path();
    let Ok(stream) = UnixStream::connect(&dsock) else {
        eprintln!("Failed to connect to Salsa debug socket: {dsock}");
        return 1;
    };
    let to = Some(Duration::from_millis(5000));
    let _ = stream.set_read_timeout(to);
    let _ = stream.set_write_timeout(to);
    let Ok(mut wr) = stream.try_clone() else {
        return 1;
    };
    if wr.write_all(b"PW_SELF_TEST\n").is_err() {
        eprintln!("Failed to send PW_SELF_TEST to {dsock}");
        return 1;
    }
    let _ = wr.flush();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    for _ in 0..4096 {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.contains("\"type\":\"cmd_reply\"") && trimmed.contains("\"PW_SELF_TEST\"") {
                    println!("{trimmed}");
                    return if trimmed.contains("\"ok\":true") || trimmed.contains("\"rc\":0") {
                        0
                    } else {
                        1
                    };
                }
            }
        }
    }
    eprintln!("Timed out waiting for PW_SELF_TEST reply from {dsock}");
    1
}

fn cmd_enable_auto() -> i32 {
    let root = PathBuf::from(get_install_root());
    let paths = SalsaHostPaths::from_env(root);
    let summary = match apply_zero_pipewire_cutover(&RealHostEnvOps, &paths) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    println!("Enabled Zero-PipeWire Pure Salsa mode (ScreenCast portals preserved by Salsa Video Broker):");
    println!(
        "  - ALSA/OpenAL routing   : {}, {}",
        summary.asoundrc.display(),
        summary.alsoft_conf.display()
    );
    println!(
        "  - Zero-PipeWire flag    : {} (Salsa owns $XDG_RUNTIME_DIR/pipewire-0 for video)",
        summary.zero_flag.display()
    );
    println!(
        "  - Steam/Proton direct   : {} (PROTON_USE_PIPEWIRE=0, direct PA/ALSA)",
        summary.direct_env_file.display()
    );
    println!(
        "  - Steam container proxy : {} (reconnects across engine resets)",
        summary.steam_wrapper.display()
    );
    println!("  - Upstream PW/WP/PA     : stopped & masked cleanly");
    println!("To revert cleanly anytime, run: lasapi-ctl disable-auto");
    0
}

fn cmd_disable_auto() -> i32 {
    let root = PathBuf::from(get_install_root());
    let paths = SalsaHostPaths::from_env(root);
    match restore_stock_pipewire(&RealHostEnvOps, &paths) {
        Ok(summary) => {
            if summary.restored_asoundrc_from_backup {
                println!("Restored previous {} from backup.", summary.asoundrc.display());
            } else {
                println!(
                    "Removed {} and restored stock ALSA configuration.",
                    summary.asoundrc.display()
                );
            }
            println!(
                "Restored upstream desktop sound server (PipeWire, PipeWire-Pulse & WirePlumber) cleanly."
            );
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn print_usage(prog: &str) {
    let ver = salsa_version();
    eprint!(
        "Usage: {prog} <command>\n\
  status                                    Show Salsa {ver} engine, BlueZ, PA socket & HW analog status\n\
  bench                                     Run live latency benchmark vs. Native PA / ALSA\n\
  sync-modules                              Sync ELF symbol versions, rebuild shims & self-heal configs\n\
  devices                                   List active WaveRT ALSA, BlueZ A2DP & MIDI endpoints\n\
  set-sink <id|auto|virtual>                Switch playback output endpoint (e.g. hw:3,0, bluez:MAC)\n\
  set-source <id|auto>                      Switch kernel ALSA capture endpoint (e.g. hw:3,0)\n\
  bt-reset [mac]                            Cleanly reset & reconnect Bluetooth A2DP over BR/EDR\n\
  bt-link <status|on|off|optional|auto> [mac]\n\
                                            Control optional BT connection & check USB-over-BT priority\n\
  bt-codecs                                 Inspect remote BT supported SEPs & negotiation diagnostics\n\
  set-codec <auto|ldac|aptxhd|aptx|aac|sbc> [abr|990|660|330]\n\
                                            Switch BlueZ A2DP preferred codec & LDAC bitrate tier\n\
  bt-sco <on|off>                           Toggle two-way Bluetooth HFP BTPROTO_SCO headset mic\n\
  set-rate <44100..192000|keep> [period] [bits]\n\
                                            Configure fixed Master Sample Rate Hz, quantum period & bit depth (16/24/32)\n\
  set-cap-rate <44100..192000|auto>\n\
                                            Pin capture/mic sample rate independently of playback clock (auto = follow master)\n\
  hw-analog [status|reset | <dac|amp|route|rails|boost|mic-boost> <val> ...]\n\
                                            Configure HW DAC Unity, Discrete Amp Tier (auto|low|med|high), Route & Boost\n\
  eq-preset <flat|bose|bass|hires|gaming|vocal|loudness>\n\
                                            Load a Salsa 10-band Dynamic EQ studio preset\n\
  eq-band <0-9> <gain_db>                   Adjust a specific EQ band (-18.0 to +18.0 dB)\n\
  param-eq <0-9> <freq> <q> <peak|lowshelf|highshelf> <gain_db>\n\
                                            Configure Parametric EQ frequency, Q & filter shape\n\
  crossfeed <on|off> [level_pct] [cutoff]   Toggle Bauer headphone stereo crossfeed\n\
  eq-mode <off|static|compress|expand> [thresh_db] [ratio]\n\
                                            Configure Dynamic EQ mode, threshold & ratio\n\
  mic-apo <on|off> [rnnoise 0|1] [gate_db] [sidetone_pct]\n\
                                            Configure Capture Mic APO (HPF, Gate, RNNoise, Comp, Sidetone)\n\
  mic-preset <discord|broadcast|clean|bypass>\n\
                                            Load a quick reversible Mic APO profile (e.g. discord = RNNoise + -50dB gate)\n\
  volume <id> <0.0-1.0|0-100>               Set volume for a specific session or PA client stream\n\
  app-volume <id> <0-150>                   Set & persist per-app session volume (0-150%)\n\
  app-mute <id> <0|1|toggle>                Mute, unmute, or toggle a specific session/app\n\
  move-app <id> <dev_id|auto>               Route a PA client stream to a specific sink/source\n\
  master-volume <0-150>                     Set Salsa master speaker volume percentage (+ AVRCP)\n\
  mute <0|1|toggle>                         Set or toggle Salsa master speaker mute\n\
  mic-volume <0-150>                        Set Salsa microphone input gain percentage\n\
  mic-mute <0|1|toggle>                     Set or toggle Salsa microphone mute\n\
  midi-send <note> [velocity]               Inject a MIDI Note-On into the LASAPI DAW MIDI Hub\n\
  pw-self-test                              Run PipeWire ScreenCast broker live self-test via debug socket\n\
  save-config                               Overwrite ~/.config/lasapi/salsa_settings.json with live engine state\n\
  reload-config                             Reload ~/.config/lasapi/salsa_settings.json into live engine\n\
  enable-auto                               Enable Zero-PipeWire Salsa mode (Video Broker, Steam/ALSA/PA direct)\n\
  disable-auto                              Restore upstream PipeWire, PipeWire-Pulse & WirePlumber cleanly\n\
  license                                   Show Salsa Copyright, No-Warranty Notice & full LGPL-3.0 / GPL-3.0 terms\n\
  --version                                 Show Salsa version & LGPL-3.0 legal notice\n\n\
{}\n"
    , one_line_legal_notice("lasapi-ctl"));
}

pub fn run_ctl(args: &[String]) -> i32 {
    let prog = args.first().map_or("lasapi-ctl", String::as_str);
    if args.len() < 2 || args[1] == "status" {
        return cmd_status();
    }
    let sub = args[1].as_str();
    if let Some(rc) = handle_common_cli_flag("lasapi-ctl", sub) {
        return rc;
    }
    if sub == "-h" || sub == "--help" {
        print_usage(prog);
        return 0;
    }
    match sub {
        "devices" => return cmd_devices(),
        "set-sink" | "set-output" if args.len() >= 3 => return cmd_set_device(false, &args[2]),
        "set-source" | "set-input" if args.len() >= 3 => return cmd_set_device(true, &args[2]),
        "bt-reset" | "reset-bt" | "bt-reconnect" => {
            return cmd_bt_reset(if args.len() >= 3 { &args[2] } else { "" });
        }
        "bt-link" | "bt-connect" | "bt-policy" => {
            let mode = if args.len() >= 3 { &args[2] } else { "status" };
            let mac = if args.len() >= 4 { &args[3] } else { "" };
            return cmd_bt_link(mode, mac);
        }
        "volume" | "set-volume" | "app-volume" if args.len() >= 4 => {
            if sub == "app-volume" {
                let pct = c_atoi(&args[3]).clamp(0, 150);
                return cmd_session_control(c_atoi(&args[2]) as u32, pct, -1, None);
            }
            return cmd_volume(c_atoi(&args[2]) as u32, c_strtof(&args[3]));
        }
        "app-mute" if args.len() >= 4 => {
            let m = if args[3] == "toggle" {
                2
            } else {
                i32::from(c_atoi(&args[3]) != 0)
            };
            return cmd_session_control(c_atoi(&args[2]) as u32, -1, m, None);
        }
        "move-app" if args.len() >= 4 => {
            return cmd_session_control(c_atoi(&args[2]) as u32, -1, -1, Some(&args[3]));
        }
        "master-volume" if args.len() >= 3 => return cmd_master_volume(c_atoi(&args[2]), -1),
        "mute" if args.len() >= 3 => {
            let m = if args[2] == "toggle" {
                2
            } else {
                c_atoi(&args[2])
            };
            return cmd_master_volume(-1, m);
        }
        "mic-volume" if args.len() >= 3 => return cmd_mic_volume(c_atoi(&args[2]), -1),
        "mic-mute" if args.len() >= 3 => {
            let m = if args[2] == "toggle" {
                2
            } else {
                c_atoi(&args[2])
            };
            return cmd_mic_volume(-1, m);
        }
        "midi-send" if args.len() >= 3 => {
            let note = c_atoi(&args[2]) as u8;
            let vel = if args.len() >= 4 {
                c_atoi(&args[3]) as u8
            } else {
                100
            };
            return cmd_midi_send(note, vel);
        }
        "set-rate" if args.len() >= 3 => {
            let period = if args.len() >= 4 {
                c_atoi(&args[3]) as u32
            } else {
                0
            };
            let bits = if args.len() >= 5 {
                c_atoi(&args[4]) as u32
            } else {
                0
            };
            let rate = if args[2].eq_ignore_ascii_case("keep") {
                0
            } else {
                c_atoi(&args[2]) as u32
            };
            return cmd_set_rate(rate, period, bits);
        }
        "set-cap-rate" if args.len() >= 3 => {
            let rate =
                if args[2].eq_ignore_ascii_case("auto") || args[2].eq_ignore_ascii_case("follow") {
                    0
                } else {
                    c_atoi(&args[2]) as u32
                };
            if rate != 0 && !(44_100..=192_000).contains(&rate) {
                eprintln!("Usage: {prog} set-cap-rate <44100..192000|auto>");
                return 1;
            }
            return cmd_set_cap_rate(rate);
        }
        "bt-codecs" => return cmd_bt_codecs(),
        "set-codec" if args.len() >= 3 => {
            let tier = if args.len() >= 4 {
                args[3].as_str()
            } else {
                ""
            };
            return cmd_set_codec(&args[2], tier);
        }
        "bt-sco" if args.len() >= 3 => return cmd_bt_sco(&args[2]),
        "param-eq" if args.len() >= 7 => {
            return cmd_param_eq(
                c_atoi(&args[2]),
                c_strtof(&args[3]),
                c_strtof(&args[4]),
                &args[5],
                c_strtof(&args[6]),
            );
        }
        "crossfeed" if args.len() >= 3 => {
            let lvl = if args.len() >= 4 {
                c_strtof(&args[3])
            } else {
                -1.0
            };
            let cut = if args.len() >= 5 {
                c_atoi(&args[4]) as u32
            } else {
                0
            };
            return cmd_crossfeed(&args[2], lvl, cut);
        }
        "mic-apo" if args.len() >= 3 => {
            let rnn = if args.len() >= 4 {
                c_atoi(&args[3])
            } else {
                -1
            };
            let gate = if args.len() >= 5 {
                c_strtof(&args[4])
            } else {
                1.0
            };
            let side = if args.len() >= 6 {
                c_atoi(&args[5])
            } else {
                -1
            };
            return cmd_mic_apo(&args[2], rnn, gate, side);
        }
        "mic-preset" if args.len() >= 3 => return cmd_mic_preset(&args[2]),
        "hw-analog" => return cmd_hw_analog(args),
        "eq-preset" if args.len() >= 3 => return cmd_eq_preset(&args[2]),
        "eq-band" if args.len() >= 4 => return cmd_eq_band(c_atoi(&args[2]), c_strtof(&args[3])),
        "eq-mode" if args.len() >= 3 => {
            let thresh = if args.len() >= 4 {
                c_strtof(&args[3])
            } else {
                1.0
            };
            let ratio = if args.len() >= 5 {
                c_strtof(&args[4])
            } else {
                0.0
            };
            return cmd_eq_mode(&args[2], thresh, ratio);
        }
        "pw-self-test" => return cmd_pw_self_test(),
        "save-config" => {
            let (rc, resp) = send_payload_cmd(LASAPI_CMD_SAVE_CONFIG, &[]);
            println!(
                "Saved & overwrote {} (HRESULT=0x{:08x})",
                crate::state_conf::salsa_settings_file_path().display(),
                resp.hr as u32
            );
            return rc;
        }
        "reload-config" => {
            let (rc, resp) = send_payload_cmd(LASAPI_CMD_RELOAD_CONFIG, &[]);
            println!(
                "Reloaded {} into live engine (HRESULT=0x{:08x})",
                crate::state_conf::salsa_settings_file_path().display(),
                resp.hr as u32
            );
            return rc;
        }
        "enable-auto" => return cmd_enable_auto(),
        "disable" | "disable-auto" => return cmd_disable_auto(),
        "bench" | "benchmark" => {
            let bpath = format!("{}/build/lasapi-bench", get_install_root());
            let err = Command::new(&bpath)
                .arg0("lasapi-bench")
                .args(&args[2..])
                .exec();
            eprintln!("execl lasapi-bench: {err}");
            return 1;
        }
        "sync-modules" | "sync" => {
            let spath = format!("{}/build/lasapi-autosync", get_install_root());
            let err = Command::new(&spath)
                .arg0("lasapi-autosync")
                .arg("--force")
                .exec();
            eprintln!("execl lasapi-autosync: {err}");
            return 1;
        }
        _ => {}
    }

    print_usage(prog);
    1
}

pub fn run_cli_from_env() -> i32 {
    let args: Vec<String> = env::args().collect();
    run_ctl(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtered_ld_path_strips_stale_salsa_and_dedups() {
        let cur = "/usr/lib:/opt/lasapi/build/pa_reconnect:/foo:/opt/lasapi/build/pa_reconnect32:/bar";
        let res = build_filtered_ld_path(
            Some("/new/build/pa_reconnect:/new/build/pa_reconnect32"),
            Some(cur),
        );
        assert_eq!(
            res,
            "/new/build/pa_reconnect:/new/build/pa_reconnect32:/usr/lib:/foo:/bar"
        );
        assert_eq!(build_filtered_ld_path(None, Some(cur)), "/usr/lib:/foo:/bar");
    }

    #[test]
    fn steam_desktop_rewrite_enables_and_reverts_cleanly() {
        let orig = "[Desktop Entry]\nName=Steam\nExec=/usr/bin/steam %U\n";
        let wrap = "/home/user/.local/bin/salsa-steam-launch";
        let wrapped = transform_steam_desktop(orig, wrap, true);
        assert!(wrapped.contains(&format!("Exec={wrap} %U\n")));
        let restored = transform_steam_desktop(&wrapped, wrap, false);
        assert_eq!(restored, orig);
    }

    #[test]
    fn install_root_resolves_from_build_subdir_or_root() {
        assert_eq!(
            install_root_from_exe("/opt/lasapi/build/lasapi-ctl"),
            "/opt/lasapi"
        );
        assert_eq!(
            install_root_from_exe("/usr/local/bin/lasapi-ctl"),
            "/usr/local/bin"
        );
    }

    #[test]
    fn filtered_ld_path_truncates_on_colon_boundary() {
        let mut long_segs = Vec::new();
        for i in 0..80 {
            long_segs.push(format!("/opt/custom/lib/segment_{i:03}/extra_padding_dir"));
        }
        let joined = long_segs.join(":");
        assert!(joined.len() > 2048);
        let out = build_filtered_ld_path(None, Some(&joined));
        assert!(out.len() <= 2047);
        assert!(!out.ends_with(':'));
        assert!(out.split(':').all(|s| s.ends_with("/extra_padding_dir")));
    }
}
