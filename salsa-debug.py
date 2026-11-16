#!/usr/bin/env python3
"""SALSA (LWASAPI) Live Interaction Debugger — CLI & Standalone PyQt6 Window."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import signal
import sys
import time
from typing import Any, Optional

BASE_DIR = os.path.dirname(os.path.realpath(__file__))
if BASE_DIR not in sys.path:
    sys.path.insert(0, BASE_DIR)

from salsa_debug_core import (  # noqa: E402
    DEFAULT_BUNDLE_DIR,
    LEVEL_RANK,
    LEVELS,
    SUBSYSTEMS,
    DiagnosticProbes,
    ExternalLogManager,
    SalsaDebugClient,
    SystemAuditScanner,
    load_saved_theme_name,
    snapshot_once,
)


def _load_salsa_gui_helpers() -> tuple[dict[str, dict[str, Any]], Any]:
    gui_path = os.path.join(BASE_DIR, "salsa-gui.py")
    if not os.path.exists(gui_path):
        return {}, None
    try:
        spec = importlib.util.spec_from_file_location("salsa_gui_mod", gui_path)
        if spec and spec.loader:
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            return getattr(mod, "THEMES", {}), getattr(mod, "create_salsa_icon", None)
    except Exception:
        pass
    return {}, None


def _print_event_human(ev: dict[str, Any]) -> None:
    tstr = ev.get("time_str", "")
    sub = ev.get("sub", "")
    lvl = ev.get("lvl", "INFO")
    op = ev.get("op", "")
    target = ev.get("target", "")
    pid = ev.get("pid", 0)
    sid = ev.get("sid", 0)
    v1, v2, hr = ev.get("v1", 0), ev.get("v2", 0), ev.get("hr", 0)
    detail = ev.get("detail", "")
    hr_s = f" hr=0x{hr & 0xffffffff:x}" if hr else ""
    print(f"[{tstr}] {sub:<6} {lvl:<5} pid={pid:<6} sid={sid:<2} {op:<15} {target:<20} ({v1},{v2}{hr_s}) {detail}", flush=True)


def _run_cli_stream(args: argparse.Namespace) -> int:
    client = SalsaDebugClient()
    ext = ExternalLogManager(client)
    stop_flag = False

    def _on_sig(_s: int, _f: Any) -> None:
        nonlocal stop_flag
        stop_flag = True

    signal.signal(signal.SIGINT, _on_sig)
    signal.signal(signal.SIGTERM, _on_sig)

    client.start()
    if args.journal:
        ext.set_enabled("journal", True)
    if args.autosync:
        ext.set_enabled("autosync", True)
    if args.dbus:
        ext.set_enabled("dbus", True)

    min_rank = LEVEL_RANK.get(args.lvl.upper(), 0)
    want_sub = args.sub.upper()
    last_seq = 0
    last_telem_seq = -1
    t_end = (time.monotonic() + args.duration) if args.duration and args.duration > 0 else None

    try:
        while not stop_flag:
            evs, last_seq = client.get_events_since(last_seq)
            for ev in evs:
                if want_sub != "ALL" and ev.get("sub") != want_sub:
                    continue
                if LEVEL_RANK.get(ev.get("lvl", "INFO"), 1) < min_rank:
                    continue
                if args.pid and int(ev.get("pid", 0) or 0) != args.pid:
                    continue
                if args.json:
                    print(json.dumps(ev), flush=True)
                else:
                    _print_event_human(ev)

            if args.telem:
                snap = client.get_snapshot()
                if snap["telem_seq"] != last_telem_seq and snap["telemetry"]:
                    last_telem_seq = snap["telem_seq"]
                    if args.json:
                        print(json.dumps(snap["telemetry"]), flush=True)

            if t_end is not None and time.monotonic() >= t_end:
                break
            time.sleep(0.08)
    except BrokenPipeError:
        try:
            devnull = os.open(os.devnull, os.O_WRONLY)
            os.dup2(devnull, sys.stdout.fileno())
            os.close(devnull)
        except Exception:
            pass
    finally:
        ext.stop_all()
        client.stop()
    return 0


def _run_cli_snapshot(as_json: bool) -> int:
    one = snapshot_once()
    if as_json:
        print(json.dumps(one, indent=2))
        return 0 if one.get("connected") else 1

    if not one.get("connected"):
        print(f"ERROR: Could not connect to {one.get('error')}", file=sys.stderr)
        return 1

    telem = one.get("telemetry") or {}
    eng = telem.get("engine") or {}
    rt = telem.get("rt") or {}
    bt = telem.get("bluez") or {}
    cap = telem.get("capture") or {}
    midi = telem.get("midi") or {}
    print(f"=== SALSA Live Debug Snapshot (audiodg PID {one['hello'].get('pid')}) ===")
    print(f"Playback : {eng.get('hw_device')} [{eng.get('active_playback_id')}] | {eng.get('sample_rate')}Hz {eng.get('format_bits')}b q={eng.get('period_frames')}")
    print(f"Capture  : {eng.get('hw_capture_device')} [{eng.get('active_capture_id')}] (active={cap.get('active')} ring={cap.get('ring_fill')})")
    print(f"RT Timing: FIFO={eng.get('realtime_priority')} | load={rt.get('cpu_load_pct')}% | cycle={rt.get('last_cycle_us')}us avg={rt.get('avg_cycle_us')}us max={rt.get('max_cycle_us')}us budget={rt.get('budget_us')}us | max_jitter={rt.get('max_jitter_us')}us | XRUNs={eng.get('hw_xruns')}")
    print(f"Stages   : PA={rt.get('pa_mix_us')}us EQ={rt.get('eq_us')}us HW={rt.get('hw_write_us')}us | Peak L/R={eng.get('output_peak_l')}/{eng.get('output_peak_r')}")
    print(f"BlueZ    : active={bt.get('active')} alias={bt.get('alias')} codec={bt.get('codec')} rate={bt.get('sample_rate')} ring={bt.get('tx_ring_fill')}/{bt.get('tx_ring_cap')} pkts={bt.get('tx_packets')} underruns={bt.get('tx_underruns')} errs={bt.get('tx_errors')}")
    print(f"MIDI Hub : init={midi.get('initialized')} rawmidi={midi.get('rawmidi_open')} seq={midi.get('seq_client')}:{midi.get('seq_in_port')} '{midi.get('controller')}' events={midi.get('events_routed')}")
    print(f"Sessions : WASAPI={len(telem.get('wasapi_sessions') or [])} | PA Clients={len(telem.get('pa_clients') or [])} | PA Streams={len(telem.get('pa_streams') or [])} | Replay Events={len(one.get('events') or [])}")
    return 0


def _run_cli_audit(as_json: bool) -> int:
    one = snapshot_once(timeout=0.5)
    procs = SystemAuditScanner.scan_audio_processes(one.get("telemetry") or None)
    boundary = SystemAuditScanner.inspect_boundary()
    data = {"processes": procs, "boundary": boundary}
    if as_json:
        print(json.dumps(data, indent=2))
        return 0

    print("=== Audio Process Routing Audit (/proc) ===")
    for p in procs:
        print(f"  PID {p['pid']:<6} {p['comm']:<18} [{p['severity']}] {p['route']}")
    print("\n=== Socket Ownership (SO_PEERCRED) ===")
    for name, s in (boundary.get("sockets") or {}).items():
        st = f"PID={s.get('peer_pid')} ({s.get('peer_comm')})" if s.get("connectable") else (s.get("error") or "MISSING")
        print(f"  {name:<14} {st:<30} {s.get('path')}")
    print("\n=== Systemd Boundary Units ===")
    for u in boundary.get("systemd") or []:
        print(f"  [{u.get('Scope')}] {u.get('Id'):<34} {u.get('ActiveState')}/{u.get('SubState')} ({u.get('UnitFileState')}) PID={u.get('MainPID')}")
    bf = boundary.get("build_freshness") or {}
    print(f"\n=== Build Freshness ===\n  newest_src={bf.get('newest_source')} stale_artifacts={bf.get('any_artifact_stale')} running_predates_disk={bf.get('running_daemon_predates_binary')} pids={bf.get('running_audiodg_pids')}")
    pw = boundary.get("pipewire") or {}
    print(f"=== PipeWire Video-Only Boundary ===\n  audio_nodes={pw.get('audio_nodes')} video_nodes={pw.get('video_nodes')} err={pw.get('error')}")
    return 0


def _run_cli_probe(args: argparse.Namespace) -> int:
    res: dict[str, Any] = {
        "ipc_and_ping": DiagnosticProbes.probe_ipc_latency(None),
        "pa_socket": DiagnosticProbes.probe_pa_socket(),
    }
    if args.tone:
        res["test_tone"] = DiagnosticProbes.probe_test_tone()
    if args.midi:
        res["midi_ping"] = DiagnosticProbes.probe_midi_ping()
    print(json.dumps(res, indent=2))
    return 0 if res["ipc_and_ping"].get("ok") else 1


def _run_gui(start_tab: int) -> int:
    from PyQt6.QtGui import QColor, QPalette
    from PyQt6.QtWidgets import QApplication, QComboBox, QHBoxLayout, QLabel, QMainWindow, QVBoxLayout, QWidget
    from salsa_debug_ui import FALLBACK_THEME, SalsaLiveDebuggerWidget

    themes, icon_fn = _load_salsa_gui_helpers()
    if not themes:
        themes = {"Salsa Midnight (Default)": dict(FALLBACK_THEME)}

    app = QApplication(sys.argv)
    app.setApplicationName("SALSA Live Interaction Debugger")

    class StandaloneDebuggerWindow(QMainWindow):
        def __init__(self) -> None:
            super().__init__()
            self.setWindowTitle("SALSA — Live Interaction & Telemetry Debugger")
            self.resize(1380, 900)
            self.current_theme_name = load_saved_theme_name()
            if self.current_theme_name not in themes:
                self.current_theme_name = next(iter(themes.keys()))
            self.current_theme = themes[self.current_theme_name]

            central = QWidget()
            self.setCentralWidget(central)
            lay = QVBoxLayout(central)
            lay.setContentsMargins(10, 8, 10, 10)
            lay.setSpacing(6)

            hdr = QHBoxLayout()
            title = QLabel("SALSA LIVE INTERACTION DEBUGGER (WASAPI · PA v35 · BlueZ A2DP · MIDI · ALSA · Shims · PipeWire)")
            title.setObjectName("TitleBanner")
            hdr.addWidget(title, 1)
            hdr.addWidget(QLabel("Theme:"))
            self.cmb_theme = QComboBox()
            self.cmb_theme.addItems(list(themes.keys()))
            self.cmb_theme.setCurrentText(self.current_theme_name)
            self.cmb_theme.currentTextChanged.connect(self._on_theme)
            hdr.addWidget(self.cmb_theme)
            lay.addLayout(hdr)

            self.dbg_widget = SalsaLiveDebuggerWidget(self, start_client=True)
            if 0 <= start_tab < self.dbg_widget.subtabs.count():
                self.dbg_widget.subtabs.setCurrentIndex(start_tab)
            lay.addWidget(self.dbg_widget, 1)

            self._apply_theme()

        def _on_theme(self, name: str) -> None:
            if name in themes:
                self.current_theme_name = name
                self.current_theme = themes[name]
                self._apply_theme()

        def _apply_theme(self) -> None:
            t = self.current_theme
            pal = QPalette()
            pal.setColor(QPalette.ColorRole.Window, QColor(t["bg_main"]))
            pal.setColor(QPalette.ColorRole.WindowText, QColor(t["text_primary"]))
            pal.setColor(QPalette.ColorRole.Base, QColor(t["bg_deep"]))
            pal.setColor(QPalette.ColorRole.AlternateBase, QColor(t["bg_card"]))
            pal.setColor(QPalette.ColorRole.Text, QColor(t["text_primary"]))
            pal.setColor(QPalette.ColorRole.Button, QColor(t["bg_control"]))
            pal.setColor(QPalette.ColorRole.ButtonText, QColor(t["text_bright"]))
            app.setPalette(pal)
            if icon_fn is not None:
                try:
                    self.setWindowIcon(icon_fn(t))
                except Exception:
                    pass
            self.setStyleSheet(
                f"""
                QMainWindow, QWidget {{
                    background-color: {t['bg_main']};
                    color: {t['text_primary']};
                    font-family: 'Inter', 'Noto Sans', sans-serif;
                    font-size: 12px;
                }}
                QLabel#TitleBanner {{
                    font-size: 15px;
                    font-weight: 800;
                    color: {t['accent_primary']};
                }}
                QTabWidget::pane {{
                    border: 1px solid {t['border_card']};
                    border-radius: 8px;
                    background-color: {t['bg_main']};
                }}
                QTabBar::tab {{
                    background-color: {t['bg_card']};
                    color: {t['text_muted']};
                    border: 1px solid {t['border_card']};
                    border-bottom: none;
                    border-top-left-radius: 7px;
                    border-top-right-radius: 7px;
                    padding: 7px 14px;
                    margin-right: 4px;
                    font-weight: 700;
                }}
                QTabBar::tab:selected {{
                    background-color: {t['bg_control']};
                    color: {t['accent_primary']};
                    border-color: {t['border_control']};
                }}
                QComboBox, QPushButton {{
                    background-color: {t['bg_control']};
                    color: {t['text_bright']};
                    border: 1px solid {t['border_control']};
                    border-radius: 6px;
                    padding: 5px 10px;
                    font-weight: 600;
                }}
                QComboBox:hover, QPushButton:hover {{
                    background-color: {t['bg_hover']};
                    border-color: {t['accent_secondary']};
                }}
                QComboBox QAbstractItemView {{
                    background-color: {t['combo_popup_bg']};
                    color: {t['text_bright']};
                    selection-background-color: {t['combo_popup_sel_bg']};
                    selection-color: {t['combo_popup_sel_text']};
                }}
                QPushButton#PrimaryBtn {{
                    background-color: {t['btn_primary_bg']};
                    border-color: {t['btn_primary_border']};
                    color: {t['btn_primary_text']};
                    font-weight: 700;
                }}
                QTableWidget {{
                    background-color: {t['bg_deep']};
                    color: {t['text_primary']};
                    border: 1px solid {t['border_control']};
                    border-radius: 7px;
                    gridline-color: {t['border_card']};
                }}
                QHeaderView::section, QTableCornerButton::section {{
                    background-color: {t['bg_control']};
                    color: {t['text_muted']};
                    padding: 4px;
                    border: none;
                    font-weight: 700;
                }}
                """
            )
            self.dbg_widget.set_theme(t)

        def closeEvent(self, event: Any) -> None:
            self.dbg_widget.shutdown()
            event.accept()

    win = StandaloneDebuggerWindow()
    win.show()
    return app.exec()


def main() -> int:
    parser = argparse.ArgumentParser(description="SALSA Live Interaction & Telemetry Debugger")
    parser.add_argument("--stream", "--cli", dest="stream", action="store_true", help="Stream live events from the lwasapi debug socket")
    parser.add_argument("--snapshot", action="store_true", help="Print one-shot telemetry + replay snapshot")
    parser.add_argument("--audit", action="store_true", help="Run /proc audio routing + systemd/socket/config boundary audit")
    parser.add_argument("--probe", action="store_true", help="Run interactive IPC/debug ping & PA v35 SO_PEERCRED probe")
    parser.add_argument("--tone", action="store_true", help="Include 440Hz WASAPI test tone during --probe")
    parser.add_argument("--midi", action="store_true", help="Include MIDI C4 ping during --probe")
    parser.add_argument("--export", nargs="?", const=DEFAULT_BUNDLE_DIR, default=None, help="Export full diagnostic JSON bundle")
    parser.add_argument("--sub", default="ALL", choices=["ALL", *SUBSYSTEMS], help="Filter --stream by subsystem")
    parser.add_argument("--lvl", default="TRACE", choices=list(LEVELS), help="Filter --stream minimum level")
    parser.add_argument("--pid", type=int, default=0, help="Filter --stream by PID")
    parser.add_argument("--duration", type=float, default=0.0, help="Stop --stream automatically after N seconds")
    parser.add_argument("--telem", action="store_true", help="Include periodic telemetry packets in --stream --json")
    parser.add_argument("--journal", action="store_true", help="Include user journalctl units in --stream")
    parser.add_argument("--autosync", action="store_true", help="Include ~/.config/lwasapi/autosync.log in --stream")
    parser.add_argument("--dbus", action="store_true", help="Include BlueZ D-Bus monitor in --stream")
    parser.add_argument("--json", action="store_true", help="Machine-readable JSON output for CLI modes")
    parser.add_argument("--tab", type=int, default=0, help="Initial debugger sub-tab (0..3) when launching GUI")
    args = parser.parse_args()

    if args.export is not None:
        out = DiagnosticProbes.export_debug_bundle(out_dir=args.export)
        print(out)
        return 0
    if args.snapshot:
        return _run_cli_snapshot(args.json)
    if args.audit:
        return _run_cli_audit(args.json)
    if args.probe:
        return _run_cli_probe(args)
    if args.stream:
        return _run_cli_stream(args)
    return _run_gui(args.tab)


if __name__ == "__main__":
    raise SystemExit(main())
