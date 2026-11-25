// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::io::Write as _;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use crate::debug_core::{
    format_ts_ms, level_rank, snapshot_once, DiagnosticProbes, ExternalLogManager,
    SalsaDebugClient, SystemAuditScanner,
};
use crate::host::install_root;
use crate::json::JsonValue;
use salsa_wire::legal::handle_common_cli_flag;
use salsa_wire::unix_sys::{install_term_signals, term_signal_requested};

#[derive(Default)]
struct CliOpts {
    stream: bool,
    snapshot: bool,
    audit: bool,
    probe: bool,
    tone: bool,
    midi: bool,
    export: Option<Option<PathBuf>>,
    export_text: Option<Option<PathBuf>>,
    stdout_text: bool,
    sub_filter: String,
    lvl_filter: String,
    pid_filter: Option<i64>,
    duration_secs: Option<f64>,
    freq_hz: f64,
    note: u8,
    telem: bool,
    journal: bool,
    autosync: bool,
    dbus: bool,
    json_out: bool,
    _tab: String,
}

fn print_help() {
    println!(
        "Usage: salsa-debug [OPTIONS]\n\n\
         Modes:\n  \
         --stream, --cli       Stream live telemetry/events over debug.sock\n  \
         --snapshot            Print single live telemetry snapshot\n  \
         --audit               Run full host & build freshness audit\n  \
         --probe               Run IPC, dlopen, and socket health probes\n  \
         --tone                Play a short diagnostic test sine tone\n  \
         --midi                Inject a diagnostic MIDI NoteOn/NoteOff pair\n  \
         --export [DIR]        Write diagnostic bundle zip archive (includes report.txt)\n  \
         --export-text [PATH]  Write detailed plain-text debug report to file/dir ('-' for stdout)\n  \
         --text                Print full detailed plain-text debug report to stdout\n  \
         --journal             Tail Salsa systemd user journal\n  \
         --autosync            Tail autosync.log\n  \
         --dbus                Capture short session D-Bus signal sample\n\n\
         Filters & Formatting:\n  \
         --sub SUB             Subsystem filter (LASAPI, PA, BLUEZ, MIDI, CAPTURE, ALSA, IPC, RT, PW, ALL)\n  \
         --lvl LVL             Min level (TRACE, DEBUG, INFO, WARN, ERROR)\n  \
         --pid PID             Filter events by PID\n  \
         --duration SECS       Duration in seconds (stream or tone)\n  \
         --freq HZ             Test tone frequency (default 440.0)\n  \
         --note N              Test MIDI note 0..127 (default 60)\n  \
         --telem               Include telemetry summaries in --stream output\n  \
         --json                Emit machine-readable JSON\n  \
         -V, --version         Show version and LGPL-3.0 legal notice\n  \
         --license             Show full LGPL-3.0, GPL-3.0, and third-party terms\n  \
         -h, --help            Show help"
    );
}

pub fn format_snapshot_human(snap: &JsonValue) -> String {
    let mut out = String::new();
    let ts = snap.get("ts_ms").and_then(JsonValue::as_i64).unwrap_or(0);
    out.push_str(&format!(
        "=== Salsa Live Telemetry Snapshot ({}) ===\n",
        format_ts_ms(ts)
    ));

    if let Some(eng) = snap.get("engine") {
        let sr = eng.get("sample_rate").and_then(JsonValue::as_i64).unwrap_or(0);
        let ch = eng.get("channels").and_then(JsonValue::as_i64).unwrap_or(0);
        let per = eng
            .get("period_frames")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let bits = eng
            .get("format_bits")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let vol = eng
            .get("desktop_volume_pct")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let muted = eng
            .get("desktop_muted")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let hw = eng
            .get("hw_device")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        let pb = eng
            .get("active_playback_id")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        let eq = eng
            .get("eq_preset")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        out.push_str(&format!(
            "Engine: {sr} Hz {ch}ch {bits}-bit | period={per} | vol={vol}% (muted={muted}) | hw={hw} ({pb}) | eq={eq}\n"
        ));
    }

    if let Some(rt) = snap.get("rt") {
        let cyc = rt
            .get("cycle_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let avg = rt
            .get("avg_cycle_us")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0);
        let max = rt
            .get("max_cycle_us")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0);
        let cpu = rt
            .get("cpu_load_pct")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.0);
        out.push_str(&format!(
            "RT    : cycles={cyc} | avg={avg:.1}us | max={max:.1}us | load={cpu:.2}%\n"
        ));
    }

    if let Some(wire) = snap.get("wire") {
        let backend = wire.get("backend").and_then(JsonValue::as_str).unwrap_or("?");
        let armed = wire.get("armed").and_then(JsonValue::as_i64).unwrap_or(0);
        let trips = wire
            .get("trips_total")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        out.push_str(&format!(
            "Wire  : backend={backend} | failsafe_armed={armed} | trips={trips}\n"
        ));
    }

    if let Some(bt) = snap.get("bluez") {
        let active = bt.get("active").and_then(JsonValue::as_i64).unwrap_or(0);
        let alias = bt.get("alias").and_then(JsonValue::as_str).unwrap_or("");
        let codec = bt.get("codec").and_then(JsonValue::as_str).unwrap_or("");
        out.push_str(&format!(
            "BlueZ : active={active} | alias={alias} | codec={codec}\n"
        ));
    }

    if let Some(pw) = snap.get("pw") {
        let brk = pw
            .get("broker_running")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let cast = pw
            .get("active_casting")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let frames = pw
            .get("frames_relayed_total")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let rej = pw
            .get("audio_rejects")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        out.push_str(&format!(
            "PW    : broker={brk} | casting={cast} | frames={frames} | audio_rejects={rej}\n"
        ));
    }

    let nw = snap
        .get("lasapi_sessions")
        .and_then(JsonValue::as_array)
        .map(<[JsonValue]>::len)
        .unwrap_or(0);
    let nc = snap
        .get("pa_clients")
        .and_then(JsonValue::as_array)
        .map(<[JsonValue]>::len)
        .unwrap_or(0);
    let ns = snap
        .get("pa_streams")
        .and_then(JsonValue::as_array)
        .map(<[JsonValue]>::len)
        .unwrap_or(0);
    out.push_str(&format!(
        "Clients: lasapi_sessions={nw} | pa_clients={nc} | pa_streams={ns}\n"
    ));
    out
}

pub fn format_audit_human(audit: &JsonValue) -> String {
    let mut out = String::new();
    out.push_str("=== Salsa System Audit ===\n");
    if let Some(h) = audit.get("host") {
        let os = h.get("os").and_then(JsonValue::as_str).unwrap_or("?");
        let k = h.get("kernel").and_then(JsonValue::as_str).unwrap_or("?");
        let cpu = h.get("cpu").and_then(JsonValue::as_str).unwrap_or("?");
        let base = h.get("base_dir").and_then(JsonValue::as_str).unwrap_or("?");
        out.push_str(&format!("Host  : {os} | {k}\nCPU   : {cpu}\nRoot  : {base}\n"));
    }
    if let Some(bf) = audit.get("build_freshness") {
        let stale = bf.get("stale").and_then(JsonValue::as_bool).unwrap_or(true);
        let sum = bf
            .get("summary")
            .and_then(JsonValue::as_str)
            .unwrap_or("?");
        out.push_str(&format!("Build : stale={stale} ({sum})\n"));
    }
    if let Some(socks) = audit.get("sockets").and_then(JsonValue::as_array) {
        out.push_str("Sockets:\n");
        for s in socks {
            let name = s.get("name").and_then(JsonValue::as_str).unwrap_or("?");
            let path = s.get("path").and_then(JsonValue::as_str).unwrap_or("?");
            let conn = s
                .get("connectable")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let pid = s
                .get("peer_pid")
                .and_then(JsonValue::as_i64)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());
            let comm = s.get("peer_comm").and_then(JsonValue::as_str).unwrap_or("");
            out.push_str(&format!(
                "  - {name:<18} conn={conn:<5} pid={pid:<7} {comm:<16} {path}\n"
            ));
        }
    }
    if let Some(units) = audit.get("systemd_units").and_then(JsonValue::as_array) {
        out.push_str("Systemd Units:\n");
        for u in units {
            let name = u.get("unit").and_then(JsonValue::as_str).unwrap_or("?");
            let act = u.get("active").and_then(JsonValue::as_str).unwrap_or("?");
            let en = u.get("enabled").and_then(JsonValue::as_str).unwrap_or("?");
            let pid = u.get("main_pid").and_then(JsonValue::as_i64).unwrap_or(0);
            out.push_str(&format!(
                "  - {name:<32} active={act:<10} enabled={en:<10} pid={pid}\n"
            ));
        }
    }
    if let Some(note) = audit.get("pipewire_note").and_then(JsonValue::as_str) {
        out.push_str(&format!("PW Note: {note}\n"));
    }
    out
}

pub fn format_probe_human(probes: &JsonValue) -> String {
    let mut out = String::from("=== Salsa Diagnostic Probes ===\n");
    if let Some(obj) = probes.as_object() {
        for (k, v) in obj {
            out.push_str(&format!("  {k:<24}: {}\n", v.to_json_compact()));
        }
    }
    out
}

fn event_matches(ev: &JsonValue, opts: &CliOpts) -> bool {
    if !opts.sub_filter.is_empty() && !opts.sub_filter.eq_ignore_ascii_case("ALL") {
        let sub = ev.get("sub").and_then(JsonValue::as_str).unwrap_or("");
        if !sub.eq_ignore_ascii_case(&opts.sub_filter) {
            return false;
        }
    }
    if !opts.lvl_filter.is_empty() {
        let min_r = level_rank(&opts.lvl_filter);
        let ev_r = level_rank(ev.get("lvl").and_then(JsonValue::as_str).unwrap_or("INFO"));
        if ev_r < min_r {
            return false;
        }
    }
    if let Some(want_pid) = opts.pid_filter {
        let pid = ev.get("pid").and_then(JsonValue::as_i64).unwrap_or(-1);
        if pid != want_pid {
            return false;
        }
    }
    true
}

fn format_event_human(ev: &JsonValue) -> String {
    let ts = format_ts_ms(ev.get("ts_ms").and_then(JsonValue::as_i64).unwrap_or(0));
    let sub = ev.get("sub").and_then(JsonValue::as_str).unwrap_or("?");
    let lvl = ev.get("lvl").and_then(JsonValue::as_str).unwrap_or("INFO");
    let pid = ev.get("pid").and_then(JsonValue::as_i64).unwrap_or(0);
    let op = ev.get("op").and_then(JsonValue::as_str).unwrap_or("");
    let tgt = ev.get("target").and_then(JsonValue::as_str).unwrap_or("");
    let det = ev.get("detail").and_then(JsonValue::as_str).unwrap_or("");
    format!("[{ts}] [{sub}/{lvl}] pid={pid} {op} {tgt}: {det}")
}

pub fn run_debug_cli(raw_args: &[String]) -> u8 {
    install_term_signals();

    let mut opts = CliOpts {
        freq_hz: 440.0,
        note: 60,
        ..CliOpts::default()
    };

    let mut i = 0usize;
    while i < raw_args.len() {
        let arg = &raw_args[i];
        if let Some(rc) = handle_common_cli_flag("salsa-debug", arg) {
            return rc as u8;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return 0;
            }
            "--stream" | "--cli" => opts.stream = true,
            "--snapshot" => opts.snapshot = true,
            "--audit" => opts.audit = true,
            "--probe" | "--probes" => opts.probe = true,
            "--tone" => opts.tone = true,
            "--midi" => opts.midi = true,
            "--telem" => opts.telem = true,
            "--journal" => opts.journal = true,
            "--autosync" => opts.autosync = true,
            "--dbus" => opts.dbus = true,
            "--json" => opts.json_out = true,
            "--export" => {
                if i + 1 < raw_args.len() && !raw_args[i + 1].starts_with('-') {
                    i += 1;
                    opts.export = Some(Some(PathBuf::from(&raw_args[i])));
                } else {
                    opts.export = Some(None);
                }
            }
            "--export-text" => {
                if i + 1 < raw_args.len() && (raw_args[i + 1] == "-" || !raw_args[i + 1].starts_with('-')) {
                    i += 1;
                    if raw_args[i] == "-" {
                        opts.stdout_text = true;
                    } else {
                        opts.export_text = Some(Some(PathBuf::from(&raw_args[i])));
                    }
                } else {
                    opts.export_text = Some(None);
                }
            }
            "--text" => opts.stdout_text = true,
            "--sub" => {
                i += 1;
                if let Some(v) = raw_args.get(i) {
                    opts.sub_filter = v.clone();
                }
            }
            "--lvl" => {
                i += 1;
                if let Some(v) = raw_args.get(i) {
                    opts.lvl_filter = v.clone();
                }
            }
            "--pid" => {
                i += 1;
                if let Some(v) = raw_args.get(i) {
                    opts.pid_filter = v.parse().ok();
                }
            }
            "--duration" => {
                i += 1;
                if let Some(v) = raw_args.get(i) {
                    if let Ok(d) = v.parse::<f64>() {
                        if d.is_finite() && d >= 0.0 {
                            opts.duration_secs = Some(d);
                        }
                    }
                }
            }
            "--freq" => {
                i += 1;
                if let Some(v) = raw_args.get(i) {
                    opts.freq_hz = v.parse().unwrap_or(440.0);
                }
            }
            "--note" => {
                i += 1;
                if let Some(v) = raw_args.get(i) {
                    opts.note = v.parse().unwrap_or(60);
                }
            }
            "--tab" => {
                i += 1;
                if let Some(v) = raw_args.get(i) {
                    opts._tab = v.clone();
                }
            }
            other => {
                eprintln!("Unknown option: {other}");
                return 2;
            }
        }
        i += 1;
    }

    let any_mode = opts.stream
        || opts.snapshot
        || opts.audit
        || opts.probe
        || opts.tone
        || opts.midi
        || opts.export.is_some()
        || opts.export_text.is_some()
        || opts.stdout_text
        || opts.journal
        || opts.autosync
        || opts.dbus;
    if !any_mode {
        opts.snapshot = true;
    }

    let root = install_root();

    if opts.snapshot {
        match snapshot_once(None, Duration::from_millis(1200)) {
            Ok(snap) => {
                if opts.json_out {
                    println!("{}", snap.to_json_pretty(2));
                } else {
                    print!("{}", format_snapshot_human(&snap));
                }
            }
            Err(e) => {
                eprintln!("Snapshot error: {e}");
                return 1;
            }
        }
    }

    if opts.audit {
        let audit = SystemAuditScanner::collect_full_audit();
        if opts.json_out {
            println!("{}", audit.to_json_pretty(2));
        } else {
            print!("{}", format_audit_human(&audit));
        }
    }

    if opts.probe {
        let probes = DiagnosticProbes::run_all_probes(&root);
        if opts.json_out {
            println!("{}", probes.to_json_pretty(2));
        } else {
            print!("{}", format_probe_human(&probes));
        }
    }

    if opts.tone {
        let dur_secs = opts
            .duration_secs
            .filter(|d| d.is_finite() && *d >= 0.0)
            .unwrap_or(0.6);
        let dur_ms = (dur_secs * 1000.0).clamp(100.0, 10000.0) as u32;
        match DiagnosticProbes::play_test_tone(opts.freq_hz, dur_ms, 0.20) {
            Ok(msg) => println!("[OK] {msg}"),
            Err(e) => {
                eprintln!("[ERR] {e}");
                return 1;
            }
        }
    }

    if opts.midi {
        match DiagnosticProbes::send_test_midi_note(opts.note, 96, 120) {
            Ok(msg) => println!("[OK] {msg}"),
            Err(e) => {
                eprintln!("[ERR] {e}");
                return 1;
            }
        }
    }

    if opts.journal {
        for line in ExternalLogManager::tail_journal(80) {
            println!("{line}");
        }
    }

    if opts.autosync {
        for line in ExternalLogManager::tail_autosync_log(80) {
            println!("{line}");
        }
    }

    if opts.dbus {
        for line in ExternalLogManager::capture_dbus_sample(1500) {
            println!("{line}");
        }
    }

    if opts.export.is_some() || opts.export_text.is_some() || opts.stdout_text {
        let snap = snapshot_once(None, Duration::from_millis(800)).ok();
        let audit = SystemAuditScanner::collect_full_audit();
        let probes = DiagnosticProbes::run_all_probes(&root);
        let mut recent_events = Vec::new();
        let ev_client = SalsaDebugClient::new(None);
        ev_client.start();
        if ev_client.wait_connected(Duration::from_millis(250)) {
            thread::sleep(Duration::from_millis(120));
            recent_events = ev_client.drain_events(256);
        }
        ev_client.stop();

        if let Some(dir_opt) = &opts.export {
            match DiagnosticProbes::export_bundle(
                dir_opt.as_deref(),
                snap.as_ref(),
                Some(&audit),
                Some(&probes),
                &recent_events,
            ) {
                Ok(path) => println!("[OK] Exported debug bundle: {}", path.display()),
                Err(e) => {
                    eprintln!("[ERR] Export failed: {e}");
                    return 1;
                }
            }
        }
        if opts.export_text.is_some() || opts.stdout_text {
            let report = DiagnosticProbes::build_detailed_text_report(
                &root,
                snap.as_ref(),
                Some(&audit),
                Some(&probes),
                &recent_events,
            );
            if opts.stdout_text {
                let mut out = std::io::stdout().lock();
                if let Err(e) = out.write_all(report.as_bytes()) {
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        return 0;
                    }
                    eprintln!("[ERR] Failed writing report to stdout: {e}");
                    return 1;
                }
                let _ = out.flush();
            }
            if let Some(txt_opt) = &opts.export_text {
                match DiagnosticProbes::export_text_file(txt_opt.as_deref(), &report) {
                    Ok(path) => println!("[OK] Exported detailed text report: {}", path.display()),
                    Err(e) => {
                        eprintln!("[ERR] Text export failed: {e}");
                        return 1;
                    }
                }
            }
        }
    }

    if opts.stream {
        let client = SalsaDebugClient::new(None);
        client.start();
        if !client.wait_connected(Duration::from_millis(1500)) {
            eprintln!(
                "Unable to connect to Salsa debug socket ({})",
                client.stats().sock_path
            );
            client.stop();
            return 1;
        }
        let deadline = opts.duration_secs.map(|d| {
            let secs = if d.is_finite() {
                d.clamp(0.1, 86_400.0 * 7.0)
            } else {
                0.1
            };
            Instant::now() + Duration::from_secs_f64(secs)
        });
        let mut last_telem = Instant::now();
        while !term_signal_requested() {
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    break;
                }
            }
            for ev in client.drain_events(256) {
                if event_matches(&ev, &opts) {
                    if opts.json_out {
                        println!("{}", ev.to_json_compact());
                    } else {
                        println!("{}", format_event_human(&ev));
                    }
                }
            }
            if opts.telem && last_telem.elapsed() >= Duration::from_secs(1) {
                last_telem = Instant::now();
                if let Some(snap) = client.latest_snapshot() {
                    if opts.json_out {
                        println!("{}", snap.to_json_compact());
                    } else {
                        print!("{}", format_snapshot_human(&snap));
                    }
                }
            }
            thread::sleep(Duration::from_millis(40));
        }
        client.stop();
    }

    0
}
