#!/usr/bin/env python3
"""PyQt6 Live Debugger Widget shared by standalone salsa-debug and Salsa Control Center."""

from __future__ import annotations

from collections import deque
import json
import math
import os
import threading
import time
from typing import Any, Callable, Optional

from PyQt6.QtCore import Qt, QTimer
from PyQt6.QtGui import QColor, QFont, QGuiApplication, QImage, QPainter, QPen, QPixmap
from PyQt6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QFileDialog,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QMessageBox,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QSizePolicy,
    QSplitter,
    QTableWidget,
    QTableWidgetItem,
    QTabWidget,
    QVBoxLayout,
    QWidget,
)

from salsa_debug_core import (
    DEBUG_SOCK,
    DEFAULT_BUNDLE_DIR,
    LEVEL_RANK,
    LEVELS,
    SUBSYSTEMS,
    DiagnosticProbes,
    DryRunDesktopCapturer,
    ExternalLogManager,
    SalsaDebugClient,
    ScreenCastShmReader,
    SystemAuditScanner,
    load_screencast_prefs,
    save_screencast_prefs,
)

FALLBACK_THEME = {
    "is_light": False,
    "bg_main": "#090d16",
    "bg_card": "#111827",
    "bg_deep": "#0b1120",
    "bg_canvas": "#070b14",
    "bg_control": "#1e293b",
    "bg_hover": "#334155",
    "border_card": "#1f2937",
    "border_control": "#334155",
    "text_primary": "#e2e8f0",
    "text_bright": "#f8fafc",
    "text_secondary": "#cbd5e1",
    "text_muted": "#94a3b8",
    "text_dim": "#64748b",
    "accent_primary": "#00e676",
    "accent_secondary": "#38bdf8",
    "accent_hot": "#ff5e3a",
    "accent_warn": "#f59e0b",
    "accent_mic": "#f43f5e",
    "accent_midi": "#a855f7",
    "accent_out": "#10b981",
    "btn_primary_bg": "#059669",
    "btn_primary_border": "#10b981",
    "btn_primary_hover": "#10b981",
    "btn_primary_text": "#ffffff",
    "btn_accent_bg": "#e11d48",
    "btn_accent_border": "#f43f5e",
    "btn_accent_hover": "#f43f5e",
    "btn_accent_text": "#ffffff",
    "combo_popup_bg": "#111827",
    "combo_popup_sel_bg": "#1e293b",
    "combo_popup_sel_text": "#00e676",
    "code_bg": "#1e293b",
}

MAX_VISIBLE_EVENTS = 1200


def _make_item(text: str, fg: Optional[str] = None, bg: Optional[str] = None, bold: bool = False, data: Any = None) -> QTableWidgetItem:
    it = QTableWidgetItem(text)
    if fg:
        it.setForeground(QColor(fg))
    if bg:
        it.setBackground(QColor(bg))
    if bold:
        f = it.font()
        f.setBold(True)
        it.setFont(f)
    if data is not None:
        it.setData(Qt.ItemDataRole.UserRole, data)
    return it


class SalsaLiveDebuggerWidget(QWidget):
    """Comprehensive live debugging interface for all SALSA subsystems and host interactions."""

    def __init__(self, parent: Optional[QWidget] = None, start_client: bool = True) -> None:
        super().__init__(parent)
        self.theme: dict[str, Any] = dict(FALLBACK_THEME)
        self.client = SalsaDebugClient()
        self.ext_logs = ExternalLogManager(self.client)

        self._paused = False
        self._last_ev_seq = 0
        self._last_telem_seq = -1
        self._all_events: list[dict[str, Any]] = []
        self._latest_telem: dict[str, Any] = {}
        self._latest_procs: list[dict[str, Any]] = []
        self._latest_boundary: dict[str, Any] = {}

        # Async worker result queue protected by lock
        self._work_lock = threading.Lock()
        self._pending_procs: Optional[list[dict[str, Any]]] = None
        self._pending_boundary: Optional[dict[str, Any]] = None
        self._pending_probe_msg: Optional[str] = None
        self._pending_sc_status: Optional[str] = None
        self._pending_net_reset_msg: Optional[str] = None
        self._scan_busy = False
        self._boundary_busy = False
        self._net_reset_busy = False

        # ScreenCast Broker & Live Feed state (Off by default for zero overhead)
        self._sc_prefs = load_screencast_prefs()
        self.shm_reader = ScreenCastShmReader()
        self.dryrun_cap = DryRunDesktopCapturer(interval_sec=0.35)
        self.dryrun_cap.set_target_fps(int(self._sc_prefs.get("fps_val", 60)))
        self._sc_mode = "Off (Zero Overhead)"
        self._sc_last_ping_mono = 0.0
        self._sc_sim_phase = 0
        self._sc_sim_anim = 0.0
        self._sc_sim_last_t = time.monotonic()
        self._sc_last_qimg: Optional[QImage] = None
        self._sc_last_mirror_seq = -1
        self._sc_last_shm_seq = -1
        self._sc_mirror_cache_key: Optional[tuple[int, int, int]] = None
        self._sc_mirror_cache_img: Optional[QImage] = None
        self._sc_ui_fps_times: deque[float] = deque(maxlen=120)

        self._build_ui()
        self._update_sc_profile_badge()
        self.set_theme(self.theme)

        if start_client:
            self.client.start()

        self._ui_timer = QTimer(self)
        self._ui_timer.timeout.connect(self._on_tick)
        self._ui_timer.start(120)

        self._audit_timer = QTimer(self)
        self._audit_timer.timeout.connect(self._auto_refresh_audits)
        self._audit_timer.start(3500)

        self._sc_timer = QTimer(self)
        self._sc_timer.timeout.connect(self._on_sc_tick)

        # Initial background scans
        self.refresh_proc_audit()
        self.refresh_boundary_audit()

    def shutdown(self) -> None:
        self._ui_timer.stop()
        self._audit_timer.stop()
        self._sc_timer.stop()
        self.dryrun_cap.stop()
        self.shm_reader.close()
        if self._sc_mode != "Off (Zero Overhead)":
            self.client.send_command("PW_PREVIEW 0")
        self.client.send_command("PW_CAST_LIMITS 0 0 0")
        self.ext_logs.stop_all()
        self.client.stop()

    def set_theme(self, theme: dict[str, Any]) -> None:
        t = dict(FALLBACK_THEME)
        t.update(theme or {})
        self.theme = t

        self.setStyleSheet(
            f"""
            QWidget#DbgRoot {{
                background-color: {t['bg_main']};
                color: {t['text_primary']};
            }}
            QLabel {{
                color: {t['text_primary']};
            }}
            QFrame#DbgCard {{
                background-color: {t['bg_card']};
                border: 1px solid {t['border_card']};
                border-radius: 8px;
            }}
            QLabel#DbgHeader {{
                font-size: 12px;
                font-weight: 700;
                color: {t['accent_primary']};
            }}
            QLabel#DbgStat {{
                font-family: Monospace;
                font-size: 11px;
                color: {t['text_secondary']};
            }}
            QLineEdit, QPlainTextEdit {{
                background-color: {t['bg_deep']};
                color: {t['text_primary']};
                border: 1px solid {t['border_control']};
                border-radius: 6px;
                padding: 4px 8px;
                font-family: Monospace;
                font-size: 11px;
            }}
            QTableWidget {{
                background-color: {t['bg_deep']};
                color: {t['text_primary']};
                gridline-color: {t['border_card']};
                border: 1px solid {t['border_card']};
                border-radius: 6px;
                selection-background-color: {t['combo_popup_sel_bg']};
                selection-color: {t['text_bright']};
            }}
            QHeaderView::section {{
                background-color: {t['bg_card']};
                color: {t['text_secondary']};
                border: 1px solid {t['border_card']};
                padding: 3px 6px;
                font-weight: 700;
            }}
            QTableCornerButton::section {{
                background-color: {t['bg_card']};
                border: 1px solid {t['border_card']};
            }}
            QProgressBar {{
                background-color: {t['bg_deep']};
                color: {t['text_bright']};
                border: 1px solid {t['border_control']};
                border-radius: 5px;
                text-align: center;
                font-size: 10px;
                font-weight: 700;
                min-height: 16px;
            }}
            QProgressBar::chunk {{
                background-color: {t['accent_primary']};
                border-radius: 4px;
            }}
            """
        )
        self._update_health_banner()

    def _build_ui(self) -> None:
        self.setObjectName("DbgRoot")
        root = QVBoxLayout(self)
        root.setContentsMargins(8, 8, 8, 8)
        root.setSpacing(6)

        # ------------------------------------------------------------------
        # 1. Top Live Health Strip + Action Toolbar
        # ------------------------------------------------------------------
        top_card = QFrame()
        top_card.setObjectName("DbgCard")
        top_col = QVBoxLayout(top_card)
        top_col.setContentsMargins(10, 8, 10, 8)
        top_col.setSpacing(6)

        row1 = QHBoxLayout()
        self.lbl_conn = QLabel(f"CONNECTING {DEBUG_SOCK} ...")
        self.lbl_conn.setFont(QFont("Monospace", 10, QFont.Weight.Bold))
        row1.addWidget(self.lbl_conn)

        self.lbl_rt_pill = QLabel("RT: waiting...")
        self.lbl_rt_pill.setObjectName("DbgStat")
        row1.addWidget(self.lbl_rt_pill, 1)

        self.btn_pause = QPushButton("Pause Stream")
        self.btn_pause.setCheckable(True)
        self.btn_pause.toggled.connect(self._on_pause_toggled)
        row1.addWidget(self.btn_pause)

        btn_clear = QPushButton("Clear Events")
        btn_clear.clicked.connect(self._on_clear_events)
        row1.addWidget(btn_clear)

        btn_reset_rt = QPushButton("Reset RT Stats")
        btn_reset_rt.clicked.connect(lambda: self.client.reset_rt_stats())
        row1.addWidget(btn_reset_rt)

        btn_snap = QPushButton("Snapshot Now")
        btn_snap.clicked.connect(lambda: self.client.send_command("SNAPSHOT"))
        row1.addWidget(btn_snap)

        btn_export = QPushButton("Export Bundle...")
        btn_export.setObjectName("PrimaryBtn")
        btn_export.clicked.connect(self._on_export_bundle)
        row1.addWidget(btn_export)

        top_col.addLayout(row1)

        self.lbl_hw_pill = QLabel("Endpoint: -- | BlueZ: -- | PA v35: -- | MIDI: --")
        self.lbl_hw_pill.setObjectName("DbgStat")
        top_col.addWidget(self.lbl_hw_pill)

        fs_row = QHBoxLayout()
        fs_row.setSpacing(10)

        self.lbl_wire_pill = QLabel("● Wire Core: connecting...")
        self.lbl_wire_pill.setObjectName("DbgStat")
        fs_row.addWidget(self.lbl_wire_pill)

        self.lbl_failsafe_pill = QLabel("● Network Failsafe: waiting...")
        self.lbl_failsafe_pill.setObjectName("DbgStat")
        fs_row.addWidget(self.lbl_failsafe_pill, 1)

        self.lbl_net_reset_status = QLabel("")
        self.lbl_net_reset_status.setObjectName("DbgStat")
        fs_row.addWidget(self.lbl_net_reset_status)

        self.btn_reset_net = QPushButton("Failsafe: Reset PA/PW Network Clients")
        self.btn_reset_net.setToolTip(
            "Flushes PulseAudio & PipeWire clients cleanly. Note: resetting PipeWire drops active ScreenCast portal streams (audio clients reconnect automatically)."
        )
        self.btn_reset_net.clicked.connect(self._on_reset_net_clients)
        fs_row.addWidget(self.btn_reset_net)

        top_col.addLayout(fs_row)
        root.addWidget(top_card)

        # ------------------------------------------------------------------
        # 2. Inner 5-Tab Workspace
        # ------------------------------------------------------------------
        self.subtabs = QTabWidget()
        self.subtabs.addTab(self._build_events_tab(), "Live Event Stream")
        self.subtabs.addTab(self._build_proc_tab(), "App & Process Routing Audit")
        self.subtabs.addTab(self._build_telemetry_tab(), "Streams, Rings & RT Timing")
        self.subtabs.addTab(self._build_boundary_tab(), "Boundary, Configs & Probes")
        self.subtabs.addTab(self._build_screencast_tab(), "ScreenCast Broker && Live Feed")
        root.addWidget(self.subtabs, 1)

    def _build_events_tab(self) -> QWidget:
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(6)

        # Filter + External Tailers bar
        fbar = QHBoxLayout()
        fbar.addWidget(QLabel("Subsystem:"))
        self.cmb_sub = QComboBox()
        self.cmb_sub.addItems(["ALL", *SUBSYSTEMS])
        self.cmb_sub.currentTextChanged.connect(lambda _v: self._rebuild_event_table())
        fbar.addWidget(self.cmb_sub)

        fbar.addWidget(QLabel("Min Level:"))
        self.cmb_lvl = QComboBox()
        self.cmb_lvl.addItems(list(LEVELS))
        self.cmb_lvl.setCurrentText("TRACE")
        self.cmb_lvl.currentTextChanged.connect(lambda _v: self._rebuild_event_table())
        fbar.addWidget(self.cmb_lvl)

        self.txt_search = QLineEdit()
        self.txt_search.setPlaceholderText("Filter by PID, op, target, or detail...")
        self.txt_search.textChanged.connect(lambda _v: self._rebuild_event_table())
        fbar.addWidget(self.txt_search, 1)

        self.chk_autoscroll = QCheckBox("Auto-Scroll")
        self.chk_autoscroll.setChecked(True)
        fbar.addWidget(self.chk_autoscroll)

        self.chk_journal = QCheckBox("+ journalctl")
        self.chk_journal.toggled.connect(lambda on: self.ext_logs.set_enabled("journal", on))
        fbar.addWidget(self.chk_journal)

        self.chk_autosync = QCheckBox("+ autosync.log")
        self.chk_autosync.toggled.connect(lambda on: self.ext_logs.set_enabled("autosync", on))
        fbar.addWidget(self.chk_autosync)

        self.chk_dbus = QCheckBox("+ BlueZ D-Bus")
        self.chk_dbus.toggled.connect(lambda on: self.ext_logs.set_enabled("dbus", on))
        fbar.addWidget(self.chk_dbus)

        lay.addLayout(fbar)

        split = QSplitter(Qt.Orientation.Vertical)
        self.tbl_events = QTableWidget(0, 9)
        self.tbl_events.setHorizontalHeaderLabels(
            ["Time", "Sub", "Lvl", "Operation", "Target", "PID", "SID", "Vals / HR", "Detail"]
        )
        hdr = self.tbl_events.horizontalHeader()
        hdr.setSectionResizeMode(8, QHeaderView.ResizeMode.Stretch)
        for col, width in ((0, 95), (1, 68), (2, 55), (3, 130), (4, 150), (5, 62), (6, 48), (7, 115)):
            self.tbl_events.setColumnWidth(col, width)
        self.tbl_events.verticalHeader().setVisible(False)
        self.tbl_events.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.tbl_events.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.tbl_events.itemSelectionChanged.connect(self._on_event_selected)
        split.addWidget(self.tbl_events)

        self.txt_event_detail = QPlainTextEdit()
        self.txt_event_detail.setReadOnly(True)
        self.txt_event_detail.setMaximumHeight(135)
        self.txt_event_detail.setPlaceholderText("Click any event row to inspect its full JSON payload...")
        split.addWidget(self.txt_event_detail)
        split.setStretchFactor(0, 4)
        split.setStretchFactor(1, 1)
        lay.addWidget(split, 1)
        return w

    def _build_proc_tab(self) -> QWidget:
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(6)

        bar = QHBoxLayout()
        bar.addWidget(QLabel("Routing Filter:"))
        self.cmb_proc_filter = QComboBox()
        self.cmb_proc_filter.addItems(["All Audio Processes", "Leaks / Direct ALSA Only", "Salsa Routed Only"])
        self.cmb_proc_filter.currentTextChanged.connect(lambda _v: self._render_proc_table())
        bar.addWidget(self.cmb_proc_filter)

        self.txt_proc_search = QLineEdit()
        self.txt_proc_search.setPlaceholderText("Search process name, PID, or .so path...")
        self.txt_proc_search.textChanged.connect(lambda _v: self._render_proc_table())
        bar.addWidget(self.txt_proc_search, 1)

        self.chk_proc_auto = QCheckBox("Auto-Refresh (3.5s)")
        self.chk_proc_auto.setChecked(True)
        bar.addWidget(self.chk_proc_auto)

        btn_scan = QPushButton("Scan /proc Now")
        btn_scan.setObjectName("PrimaryBtn")
        btn_scan.clicked.connect(self.refresh_proc_audit)
        bar.addWidget(btn_scan)
        lay.addLayout(bar)

        split = QSplitter(Qt.Orientation.Vertical)
        self.tbl_procs = QTableWidget(0, 6)
        self.tbl_procs.setHorizontalHeaderLabels(
            ["PID", "Process", "Classification / Audio Path", "Sockets / SHMs", "ALSA /dev/snd", "Mapped Audio Libraries"]
        )
        self.tbl_procs.horizontalHeader().setSectionResizeMode(2, QHeaderView.ResizeMode.ResizeToContents)
        self.tbl_procs.horizontalHeader().setSectionResizeMode(5, QHeaderView.ResizeMode.Stretch)
        self.tbl_procs.verticalHeader().setVisible(False)
        self.tbl_procs.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.tbl_procs.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.tbl_procs.itemSelectionChanged.connect(self._on_proc_selected)
        split.addWidget(self.tbl_procs)

        self.txt_proc_detail = QPlainTextEdit()
        self.txt_proc_detail.setReadOnly(True)
        self.txt_proc_detail.setMaximumHeight(165)
        self.txt_proc_detail.setPlaceholderText("Select a process to inspect cmdline, sockets, memfd SHM rings, /dev/snd FDs, and /proc/<pid>/maps...")
        split.addWidget(self.txt_proc_detail)
        split.setStretchFactor(0, 3)
        split.setStretchFactor(1, 1)
        lay.addWidget(split, 1)
        return w

    def _build_telemetry_tab(self) -> QWidget:
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(6)

        cards = QHBoxLayout()

        # Card 1: RT Timing
        rt_card = QFrame()
        rt_card.setObjectName("DbgCard")
        rt_lay = QVBoxLayout(rt_card)
        rt_lay.setContentsMargins(10, 8, 10, 8)
        hdr1 = QLabel("REAL-TIME SCHED_FIFO TIMING & STAGE BREAKDOWN")
        hdr1.setObjectName("DbgHeader")
        rt_lay.addWidget(hdr1)
        self.bar_cpu_load = QProgressBar()
        self.bar_cpu_load.setRange(0, 100)
        self.bar_cpu_load.setFormat("RT Budget Load: %p%")
        rt_lay.addWidget(self.bar_cpu_load)
        self.lbl_rt_detail = QLabel("Waiting for telemetry...")
        self.lbl_rt_detail.setObjectName("DbgStat")
        self.lbl_rt_detail.setWordWrap(True)
        rt_lay.addWidget(self.lbl_rt_detail)
        cards.addWidget(rt_card, 1)

        # Card 2: BlueZ & Capture
        bt_card = QFrame()
        bt_card.setObjectName("DbgCard")
        bt_lay = QVBoxLayout(bt_card)
        bt_lay.setContentsMargins(10, 8, 10, 8)
        hdr2 = QLabel("BLUEZ A2DP / AVRCP & KERNEL ALSA CAPTURE")
        hdr2.setObjectName("DbgHeader")
        bt_lay.addWidget(hdr2)
        self.bar_bt_ring = QProgressBar()
        self.bar_bt_ring.setRange(0, 100)
        self.bar_bt_ring.setFormat("BT TX Ring: 0%")
        bt_lay.addWidget(self.bar_bt_ring)
        self.lbl_bt_detail = QLabel("BlueZ: idle")
        self.lbl_bt_detail.setObjectName("DbgStat")
        self.lbl_bt_detail.setWordWrap(True)
        bt_lay.addWidget(self.lbl_bt_detail)
        cards.addWidget(bt_card, 1)

        # Card 3: MIDI + Engine/EQ
        midi_card = QFrame()
        midi_card.setObjectName("DbgCard")
        midi_lay = QVBoxLayout(midi_card)
        midi_lay.setContentsMargins(10, 8, 10, 8)
        hdr3 = QLabel("DAW MIDI HUB & DYNAMIC EQ APO STATE")
        hdr3.setObjectName("DbgHeader")
        midi_lay.addWidget(hdr3)
        self.lbl_midi_detail = QLabel("MIDI / APO: waiting...")
        self.lbl_midi_detail.setObjectName("DbgStat")
        self.lbl_midi_detail.setWordWrap(True)
        midi_lay.addWidget(self.lbl_midi_detail)
        cards.addWidget(midi_card, 1)

        lay.addLayout(cards)

        # Tables for WASAPI SHM + PA v35 Streams + Clients
        split = QSplitter(Qt.Orientation.Vertical)

        self.tbl_wasapi = QTableWidget(0, 9)
        self.tbl_wasapi.setHorizontalHeaderLabels(
            ["WASAPI SID", "PID", "Client Name", "Mode", "Dir", "Rate / Ch", "Vol / Mute", "Ring Fill (frames)", "XRUNs"]
        )
        self.tbl_wasapi.horizontalHeader().setSectionResizeMode(2, QHeaderView.ResizeMode.Stretch)
        self.tbl_wasapi.verticalHeader().setVisible(False)
        self.tbl_wasapi.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        split.addWidget(self.tbl_wasapi)

        self.tbl_pa_streams = QTableWidget(0, 9)
        self.tbl_pa_streams.setHorizontalHeaderLabels(
            ["PA Stream", "Client", "PID", "App / Stream Name", "Dir", "State", "Rate / Ch", "Ring Fill (frames)", "Target Dev"]
        )
        self.tbl_pa_streams.horizontalHeader().setSectionResizeMode(3, QHeaderView.ResizeMode.Stretch)
        self.tbl_pa_streams.verticalHeader().setVisible(False)
        self.tbl_pa_streams.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        split.addWidget(self.tbl_pa_streams)

        self.txt_asound_live = QPlainTextEdit()
        self.txt_asound_live.setReadOnly(True)
        self.txt_asound_live.setMaximumHeight(130)
        split.addWidget(self.txt_asound_live)
        lay.addWidget(split, 1)
        return w

    def _build_boundary_tab(self) -> QWidget:
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(6)

        # Interactive Probes Bar
        probe_card = QFrame()
        probe_card.setObjectName("DbgCard")
        p_col = QVBoxLayout(probe_card)
        p_col.setContentsMargins(10, 8, 10, 8)
        p_col.setSpacing(6)

        prow = QHBoxLayout()
        hdr_p = QLabel("INTERACTIVE SYSTEM & HARDWARE PROBES")
        hdr_p.setObjectName("DbgHeader")
        prow.addWidget(hdr_p)
        prow.addStretch()

        b_ping = QPushButton("IPC + Debug Ping RTT")
        b_ping.clicked.connect(lambda: self._run_probe("ipc", lambda: DiagnosticProbes.probe_ipc_latency(self.client)))
        prow.addWidget(b_ping)

        b_tone = QPushButton("Play 440Hz WASAPI Probe Tone")
        b_tone.clicked.connect(lambda: self._run_probe("tone", DiagnosticProbes.probe_test_tone))
        prow.addWidget(b_tone)

        b_midi = QPushButton("Send MIDI Note Ping (C4)")
        b_midi.clicked.connect(lambda: self._run_probe("midi", DiagnosticProbes.probe_midi_ping))
        prow.addWidget(b_midi)

        b_pa = QPushButton("Verify PA v35 SO_PEERCRED & pactl")
        b_pa.clicked.connect(lambda: self._run_probe("pa", DiagnosticProbes.probe_pa_socket))
        prow.addWidget(b_pa)

        b_ref = QPushButton("Refresh Boundary Audit")
        b_ref.setObjectName("PrimaryBtn")
        b_ref.clicked.connect(self.refresh_boundary_audit)
        prow.addWidget(b_ref)

        p_col.addLayout(prow)

        self.lbl_stale_warn = QLabel("Build & Binary Freshness: checking...")
        self.lbl_stale_warn.setObjectName("DbgStat")
        p_col.addWidget(self.lbl_stale_warn)

        self.txt_probe_out = QPlainTextEdit()
        self.txt_probe_out.setReadOnly(True)
        self.txt_probe_out.setMaximumHeight(82)
        self.txt_probe_out.setPlaceholderText("Click any interactive probe button above to test IPC, WASAPI PCM audio, MIDI injection, or PulseAudio socket ownership...")
        p_col.addWidget(self.txt_probe_out)
        lay.addWidget(probe_card)

        # Split: left = Systemd + Socket ownership; right = Config / Boundary Viewer
        split = QSplitter(Qt.Orientation.Horizontal)

        left_w = QWidget()
        left_lay = QVBoxLayout(left_w)
        left_lay.setContentsMargins(0, 0, 0, 0)

        self.tbl_units = QTableWidget(0, 6)
        self.tbl_units.setHorizontalHeaderLabels(["Scope", "Unit", "Active", "Sub", "Enabled", "MainPID / Restarts"])
        self.tbl_units.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        self.tbl_units.verticalHeader().setVisible(False)
        self.tbl_units.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        left_lay.addWidget(self.tbl_units, 3)

        self.tbl_sockets = QTableWidget(0, 5)
        self.tbl_sockets.setHorizontalHeaderLabels(["Boundary Socket", "Status", "Peer PID", "Owner Comm", "Path"])
        self.tbl_sockets.horizontalHeader().setSectionResizeMode(4, QHeaderView.ResizeMode.Stretch)
        self.tbl_sockets.verticalHeader().setVisible(False)
        self.tbl_sockets.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        left_lay.addWidget(self.tbl_sockets, 2)
        split.addWidget(left_w)

        right_w = QWidget()
        right_lay = QVBoxLayout(right_w)
        right_lay.setContentsMargins(0, 0, 0, 0)
        cfg_bar = QHBoxLayout()
        cfg_bar.addWidget(QLabel("Inspect File / Check:"))
        self.cmb_cfg = QComboBox()
        self.cmb_cfg.addItems(
            [
                "~/.asoundrc",
                "~/.config/OpenAL/alsoft.conf",
                "pipewire_zero_mode",
                "99-salsa-direct.conf",
                "99-lwasapi-video-only.conf",
                "module_versions.state",
                "src/alsa.version",
                "src/pulse.version",
                "PipeWire Nodes (pw-dump)",
                "ALSA Plugin & Build Freshness",
                "/proc/asound/cards + pcm",
            ]
        )
        self.cmb_cfg.currentTextChanged.connect(lambda _v: self._render_config_viewer())
        cfg_bar.addWidget(self.cmb_cfg, 1)
        right_lay.addLayout(cfg_bar)

        self.txt_cfg_view = QPlainTextEdit()
        self.txt_cfg_view.setReadOnly(True)
        right_lay.addWidget(self.txt_cfg_view, 1)
        split.addWidget(right_w)

        split.setStretchFactor(0, 3)
        split.setStretchFactor(1, 2)
        lay.addWidget(split, 1)
        return w

    def _build_screencast_tab(self) -> QWidget:
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setSpacing(8)

        # 1. Top Control & Telemetry Card
        ctrl_card = QFrame()
        ctrl_card.setObjectName("DbgCard")
        ctrl_col = QVBoxLayout(ctrl_card)
        ctrl_col.setSpacing(6)

        row_top = QHBoxLayout()
        self.btn_sc_live = QPushButton("Enable Live Feed")
        self.btn_sc_live.setCheckable(True)
        self.btn_sc_live.setObjectName("PrimaryBtn")
        self.btn_sc_live.toggled.connect(self._on_sc_toggle_clicked)
        row_top.addWidget(self.btn_sc_live)

        row_top.addWidget(QLabel("Feed Mode:"))
        self.cmb_sc_mode = QComboBox()
        self.cmb_sc_mode.addItems(
            [
                "Auto (Real Cast -> Dry-Run Mirror)",
                "Dry-Run: Silent Compositor Mirror",
                "Dry-Run: Simulated Wayland ScreenCast",
                "Real Cast Only (SHM Tap)",
            ]
        )
        self.cmb_sc_mode.currentTextChanged.connect(self._on_sc_mode_changed)
        row_top.addWidget(self.cmb_sc_mode, 1)

        btn_sc_test = QPushButton("Run Broker Self-Test")
        btn_sc_test.clicked.connect(self._on_sc_self_test)
        row_top.addWidget(btn_sc_test)

        btn_sc_sync = QPushButton("Sync Socket Mode")
        btn_sc_sync.clicked.connect(self._on_sc_sync_mode)
        row_top.addWidget(btn_sc_sync)

        btn_sc_save = QPushButton("Save Frame PNG...")
        btn_sc_save.clicked.connect(self._on_sc_save_png)
        row_top.addWidget(btn_sc_save)
        ctrl_col.addLayout(row_top)

        row_profile = QHBoxLayout()
        row_profile.addWidget(QLabel("Stream Quality (Discord):"))

        row_profile.addWidget(QLabel("Resolution:"))
        self.cmb_sc_res = QComboBox()
        sc_res_options = [
            ("Source (Native)", ("Source", 0, 0)),
            ("2160p (4K UHD)", ("2160p", 3840, 2160)),
            ("1440p (QHD)", ("1440p", 2560, 1440)),
            ("1080p (FHD)", ("1080p", 1920, 1080)),
            ("720p (HD)", ("720p", 1280, 720)),
            ("480p (SD)", ("480p", 854, 480)),
        ]
        for lbl, data in sc_res_options:
            self.cmb_sc_res.addItem(lbl, data)
        saved_rk = str(self._sc_prefs.get("res_key", "Source"))
        for idx, (_lbl, data) in enumerate(sc_res_options):
            if data[0] == saved_rk:
                self.cmb_sc_res.setCurrentIndex(idx)
                break
        self.cmb_sc_res.setToolTip("Adjusts live cast preview & dry-run resolution immediately (matches Discord Go Live presets)")
        self.cmb_sc_res.currentIndexChanged.connect(lambda _i: self._on_sc_profile_changed())
        row_profile.addWidget(self.cmb_sc_res)

        row_profile.addWidget(QLabel("FPS:"))
        self.cmb_sc_fps = QComboBox()
        sc_fps_options = [
            ("60 FPS", 60),
            ("30 FPS", 30),
            ("15 FPS", 15),
            ("Source (Native Hz)", 0),
        ]
        for lbl, val in sc_fps_options:
            self.cmb_sc_fps.addItem(lbl, val)
        saved_fps = int(self._sc_prefs.get("fps_val", 60))
        for idx, (_lbl, val) in enumerate(sc_fps_options):
            if val == saved_fps:
                self.cmb_sc_fps.setCurrentIndex(idx)
                break
        self.cmb_sc_fps.setToolTip("Adjusts live cast FPS live without restarting (15 / 30 / 60 / Source)")
        self.cmb_sc_fps.currentIndexChanged.connect(lambda _i: self._on_sc_profile_changed())
        row_profile.addWidget(self.cmb_sc_fps)

        self.chk_sc_cap_broker = QCheckBox("Cap Real Stream FPS")
        self.chk_sc_cap_broker.setChecked(bool(self._sc_prefs.get("cap_real_cast", False)))
        self.chk_sc_cap_broker.setToolTip(
            "When checked, also paces active PipeWire ScreenCast delivery (e.g. Discord) to the selected FPS live while Debugger is open"
        )
        self.chk_sc_cap_broker.toggled.connect(lambda _c: self._on_sc_profile_changed())
        row_profile.addWidget(self.chk_sc_cap_broker)

        self.lbl_sc_profile_badge = QLabel("Profile: Source @ 60 FPS")
        self.lbl_sc_profile_badge.setObjectName("DbgStat")
        row_profile.addWidget(self.lbl_sc_profile_badge, 1)
        ctrl_col.addLayout(row_profile)

        self.lbl_sc_status = QLabel(
            "Broker: waiting for audiodg telemetry... | Live Feed: OFF (zero capture overhead)"
        )
        self.lbl_sc_status.setObjectName("DbgStat")
        ctrl_col.addWidget(self.lbl_sc_status)

        self.lbl_sc_metrics = QLabel(
            "Sockets: -- | Clients: 0 | Nodes: 0 | Links: 0 | Relayed: 0 | Dropped: 0 | Audio Rejects: 0"
        )
        self.lbl_sc_metrics.setObjectName("DbgStat")
        ctrl_col.addWidget(self.lbl_sc_metrics)
        lay.addWidget(ctrl_card)

        # 2. Main Splitter: Left = Live / Dry-Run Viewport; Right = Clients, Nodes & Links
        split = QSplitter(Qt.Orientation.Horizontal)

        left_card = QFrame()
        left_card.setObjectName("DbgCard")
        left_lay = QVBoxLayout(left_card)
        left_lay.setSpacing(6)

        self.lbl_sc_feed_hdr = QLabel("Viewport: LIVE FEED OFF (Toggle 'Enable Live Feed' above to inspect)")
        self.lbl_sc_feed_hdr.setObjectName("DbgHeader")
        self.lbl_sc_feed_hdr.setFont(QFont("Monospace", 10, QFont.Weight.Bold))
        left_lay.addWidget(self.lbl_sc_feed_hdr)

        self.lbl_sc_viewport = QLabel(
            "ScreenCast Live Feed is currently OFF.\n\n"
            "Click 'Enable Live Feed' to mirror any active PipeWire ScreenCast stream locklessly\n"
            "via /dev/shm, or preview a non-broadcasting dry-run composite showing what a\n"
            "ScreenCast sees without starting an active share."
        )
        self.lbl_sc_viewport.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.lbl_sc_viewport.setMinimumSize(420, 260)
        self.lbl_sc_viewport.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Ignored)
        self.lbl_sc_viewport.setStyleSheet(
            "background-color: #070b14; border: 1px solid #1f2937; border-radius: 6px; color: #94a3b8; padding: 12px;"
        )
        left_lay.addWidget(self.lbl_sc_viewport, 1)

        self.lbl_sc_stream_meta = QLabel("SHM: -- | Producer: -- | Transport: -- | Format: -- | Frame: 0")
        self.lbl_sc_stream_meta.setObjectName("DbgStat")
        left_lay.addWidget(self.lbl_sc_stream_meta)
        split.addWidget(left_card)

        right_w = QWidget()
        right_lay = QVBoxLayout(right_w)
        right_lay.setContentsMargins(0, 0, 0, 0)
        right_lay.setSpacing(6)

        self.tbl_pw_clients = QTableWidget(0, 5)
        self.tbl_pw_clients.setHorizontalHeaderLabels(["ID", "PID", "App", "Portal App ID", "Nodes"])
        self.tbl_pw_clients.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.ResizeToContents)
        self.tbl_pw_clients.horizontalHeader().setSectionResizeMode(3, QHeaderView.ResizeMode.Stretch)
        self.tbl_pw_clients.verticalHeader().setVisible(False)
        self.tbl_pw_clients.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.tbl_pw_clients.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        right_lay.addWidget(self.tbl_pw_clients, 2)

        self.tbl_pw_nodes = QTableWidget(0, 6)
        self.tbl_pw_nodes.setHorizontalHeaderLabels(["ID", "Name", "Class", "Dir/State", "Format/Mem", "FPS/Frames"])
        self.tbl_pw_nodes.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.ResizeToContents)
        self.tbl_pw_nodes.horizontalHeader().setStretchLastSection(True)
        self.tbl_pw_nodes.verticalHeader().setVisible(False)
        self.tbl_pw_nodes.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.tbl_pw_nodes.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        right_lay.addWidget(self.tbl_pw_nodes, 3)

        self.tbl_pw_links = QTableWidget(0, 5)
        self.tbl_pw_links.setHorizontalHeaderLabels(["Link", "Producer -> Consumer", "State", "Buf", "Relayed"])
        self.tbl_pw_links.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.ResizeToContents)
        self.tbl_pw_links.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        self.tbl_pw_links.verticalHeader().setVisible(False)
        self.tbl_pw_links.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.tbl_pw_links.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        right_lay.addWidget(self.tbl_pw_links, 2)

        split.addWidget(right_w)
        split.setStretchFactor(0, 5)
        split.setStretchFactor(1, 5)
        lay.addWidget(split, 1)
        return w

    def _current_sc_profile(self) -> tuple[str, int, int, int, bool]:
        if hasattr(self, "cmb_sc_res") and hasattr(self, "cmb_sc_fps"):
            data = self.cmb_sc_res.currentData()
            if isinstance(data, tuple) and len(data) == 3:
                res_key, mw, mh = str(data[0]), int(data[1]), int(data[2])
            else:
                res_key, mw, mh = "Source", 0, 0
            fps_val = int(self.cmb_sc_fps.currentData() or 0)
            cap_real = bool(hasattr(self, "chk_sc_cap_broker") and self.chk_sc_cap_broker.isChecked())
            return res_key, mw, mh, fps_val, cap_real
        prefs = getattr(self, "_sc_prefs", None) or {}
        rk = str(prefs.get("res_key", "Source"))
        res_map = {
            "Source": (0, 0),
            "2160p": (3840, 2160),
            "1440p": (2560, 1440),
            "1080p": (1920, 1080),
            "720p": (1280, 720),
            "480p": (854, 480),
        }
        mw, mh = res_map.get(rk, (0, 0))
        return rk, mw, mh, int(prefs.get("fps_val", 60)), bool(prefs.get("cap_real_cast", False))

    def _get_primary_source_size(self) -> tuple[int, int]:
        try:
            scr = QGuiApplication.primaryScreen()
            if scr is not None:
                gsz = scr.size()
                dpr = float(scr.devicePixelRatio() or 1.0)
                w = int(round(gsz.width() * dpr))
                h = int(round(gsz.height() * dpr))
                if w >= 320 and h >= 240:
                    return w, h
        except Exception:
            pass
        return 1920, 1080

    @staticmethod
    def _compute_target_dims(src_w: int, src_h: int, max_w: int, max_h: int) -> tuple[int, int]:
        sw = max(2, int(src_w))
        sh = max(2, int(src_h))
        cw = int(max_w) if max_w > 0 else 3840
        ch = int(max_h) if max_h > 0 else 2160
        dw, dh = sw, sh
        if dw > cw:
            dh = max(2, int(round((dh * cw) / dw)))
            dw = cw
        if dh > ch:
            dw = max(2, int(round((dw * ch) / dh)))
            dh = ch
        return max(2, dw & ~1), max(2, dh & ~1)

    def _update_sc_timer_interval(self) -> None:
        _rk, _mw, _mh, fps_val, _cap = self._current_sc_profile()
        iv_ms = 16 if (fps_val <= 0 or fps_val >= 60) else max(16, int(round(1000.0 / fps_val)))
        self._sc_timer.setInterval(iv_ms)

    def _send_pw_preview_cmd(self, enabled: bool) -> None:
        if not enabled:
            self.client.send_command("PW_PREVIEW 0")
            return
        _rk, mw, mh, fps_val, _cap = self._current_sc_profile()
        self.client.send_command(f"PW_PREVIEW 1 15000 {mw} {mh} {fps_val}")

    def _send_pw_cast_limits_cmd(self) -> None:
        _rk, mw, mh, fps_val, cap_real = self._current_sc_profile()
        if cap_real:
            self.client.send_command(f"PW_CAST_LIMITS {mw} {mh} {fps_val}")
        else:
            self.client.send_command("PW_CAST_LIMITS 0 0 0")

    def _record_ui_fps(self, now: float) -> float:
        self._sc_ui_fps_times.append(now)
        while len(self._sc_ui_fps_times) > 2 and (now - self._sc_ui_fps_times[0]) > 1.25:
            self._sc_ui_fps_times.popleft()
        if len(self._sc_ui_fps_times) >= 2:
            dt = self._sc_ui_fps_times[-1] - self._sc_ui_fps_times[0]
            if dt > 0.05:
                return (len(self._sc_ui_fps_times) - 1) / dt
        return 0.0

    def _update_sc_profile_badge(self, measured_fps: float = 0.0, active_dims: str = "") -> None:
        if not hasattr(self, "lbl_sc_profile_badge"):
            return
        rk, _mw, _mh, fps_val, cap_real = self._current_sc_profile()
        fps_str = "Source Hz" if fps_val <= 0 else f"{fps_val} FPS"
        live_part = f" | Live: {measured_fps:.1f} FPS" if measured_fps > 0.1 else ""
        dim_part = f" ({active_dims})" if active_dims else ""
        cap_part = " | Broker Cap: ON" if cap_real else ""
        self.lbl_sc_profile_badge.setText(f"Profile: {rk}{dim_part} @ {fps_str}{live_part}{cap_part}")

    def _on_sc_toggle_clicked(self, checked: bool) -> None:
        if checked:
            self._sc_mode = self.cmb_sc_mode.currentText()
            self.btn_sc_live.setText("Disable Live Feed (ON)")
            self._sc_last_ping_mono = 0.0
            self._sc_last_mirror_seq = -1
            self._sc_last_shm_seq = -1
            self._sc_mirror_cache_key = None
            self._sc_mirror_cache_img = None
            self._sc_ui_fps_times.clear()
            self._sc_sim_last_t = time.monotonic()
            _rk, _mw, _mh, fps_val, _cap = self._current_sc_profile()
            self.dryrun_cap.set_target_fps(fps_val)
            if "Compositor Mirror" in self._sc_mode or self._sc_mode.startswith("Auto"):
                self.dryrun_cap.start()
            else:
                self.dryrun_cap.stop()
            self._send_pw_cast_limits_cmd()
            self._send_pw_preview_cmd(True)
            self._sc_last_ping_mono = time.monotonic()
            self._update_sc_timer_interval()
            self._sc_timer.start()
            self._update_sc_profile_badge()
            self._on_sc_tick()
        else:
            self._sc_mode = "Off (Zero Overhead)"
            self.btn_sc_live.setText("Enable Live Feed")
            self._sc_timer.stop()
            self.dryrun_cap.stop()
            self.shm_reader.close()
            self._send_pw_preview_cmd(False)
            self.lbl_sc_feed_hdr.setText("Viewport: LIVE FEED OFF (zero capture overhead)")
            self.lbl_sc_viewport.setPixmap(QPixmap())
            self.lbl_sc_viewport.setText(
                "ScreenCast Live Feed is OFF (Zero CPU / Compositor Overhead).\n\n"
                "Click 'Enable Live Feed' above to view active ScreenCast frames or dry-run preview."
            )
            self.lbl_sc_stream_meta.setText("SHM: idle | Preview stopped")
            self._update_sc_profile_badge()

    def _on_sc_mode_changed(self, _text: str) -> None:
        if self.btn_sc_live.isChecked():
            self._on_sc_toggle_clicked(True)

    def _on_sc_profile_changed(self) -> None:
        rk, _mw, _mh, fps_val, cap_real = self._current_sc_profile()
        save_screencast_prefs({"res_key": rk, "fps_val": fps_val, "cap_real_cast": cap_real})
        self._sc_mirror_cache_key = None
        self._sc_mirror_cache_img = None
        self._sc_ui_fps_times.clear()
        self.dryrun_cap.set_target_fps(fps_val)
        self._send_pw_cast_limits_cmd()
        self._update_sc_profile_badge()
        if self.btn_sc_live.isChecked():
            self._update_sc_timer_interval()
            self._send_pw_preview_cmd(True)
            self._sc_last_ping_mono = time.monotonic()
            self._on_sc_tick()

    def _on_sc_self_test(self) -> None:
        self.lbl_sc_status.setText("Running native PipeWire ScreenCast broker wire self-test...")

        def worker() -> None:
            reply = self.client.send_command_wait("PW_SELF_TEST", timeout=3.0)
            if reply is None:
                msg = "Broker Self-Test FAILED: audiodg debug socket not connected"
            else:
                msg = f"Self-Test (rc={reply.get('rc', -1)}): {reply.get('reply', '')}"
            with self._work_lock:
                self._pending_probe_msg = msg
                self._pending_sc_status = msg

        threading.Thread(target=worker, daemon=True).start()

    def _on_sc_sync_mode(self) -> None:
        def worker() -> None:
            reply = self.client.send_command_wait("PW_SYNC_MODE", timeout=2.0)
            msg = (
                "Socket Sync: daemon not connected"
                if reply is None
                else f"Socket Sync (rc={reply.get('rc', -1)}): {reply.get('reply', '')}"
            )
            with self._work_lock:
                self._pending_sc_status = msg
            self.refresh_boundary_audit()

        threading.Thread(target=worker, daemon=True).start()

    def _on_sc_save_png(self) -> None:
        if self._sc_last_qimg is None or self._sc_last_qimg.isNull():
            QMessageBox.information(self, "Save Preview Frame", "No rendered ScreenCast or dry-run frame yet.")
            return
        os.makedirs(DEFAULT_BUNDLE_DIR, exist_ok=True)
        default_path = os.path.join(DEFAULT_BUNDLE_DIR, f"salsa-screencast-{time.strftime('%Y%m%d_%H%M%S')}.png")
        out_path, _ = QFileDialog.getSaveFileName(self, "Save ScreenCast Frame", default_path, "PNG Images (*.png)")
        if out_path:
            self._sc_last_qimg.save(out_path, "PNG")

    def _display_sc_qimage(self, img: QImage) -> None:
        self._sc_last_qimg = img
        sz = self.lbl_sc_viewport.size()
        tw = max(320, sz.width() - 8)
        th = max(200, sz.height() - 8)
        pix = QPixmap.fromImage(img).scaled(
            tw,
            th,
            Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.SmoothTransformation,
        )
        self.lbl_sc_viewport.setText("")
        self.lbl_sc_viewport.setPixmap(pix)

    def _render_sc_dmabuf_hud_qimage(self, frame: dict[str, Any]) -> QImage:
        rk, mw, mh, fps_val, _cap = self._current_sc_profile()
        src_w = int(frame.get("src_width", 0) or 1920)
        src_h = int(frame.get("src_height", 0) or 1080)
        dw, dh = self._compute_target_dims(src_w, src_h, mw, mh)
        cw = min(dw, 1920)
        ch = min(dh, 1080)
        img = QImage(cw, ch, QImage.Format.Format_RGB32)
        img.fill(QColor("#08101c"))
        p = QPainter(img)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.scale(cw / 640.0, ch / 360.0)
        t = self.theme

        p.setPen(QPen(QColor(t["accent_secondary"]), 2))
        p.drawRoundedRect(14, 14, 612, 332, 10, 10)

        p.setPen(QColor(t["accent_primary"]))
        p.setFont(QFont("Monospace", 13, QFont.Weight.Bold))
        p.drawText(30, 50, "ZERO-COPY GPU DMA-BUF STREAM ACTIVE")

        fps_str = "Source" if fps_val <= 0 else f"{fps_val} FPS"
        act_fps = float(frame.get("actual_fps", 0.0) or 0.0)
        p.setPen(QColor(t["text_primary"]))
        p.setFont(QFont("Monospace", 10))
        mod_hex = f"0x{frame.get('drm_modifier', 0):016x}"
        lines = [
            f"Producer      : {frame.get('prod_name') or '--'} (Node #{frame.get('prod_node_id', 0)})",
            f"Source / Goal : {src_w}x{src_h} -> {rk} ({dw}x{dh}) @ {fps_str} | Live {act_fps:.1f} FPS",
            f"Pixel Format  : {frame.get('format_name') or 'BGRx'}  |  Consumers: {frame.get('cons_count', 0)}",
            f"DRM Modifier  : {mod_hex} (tiled/compressed GPU-only FD)",
            f"Relayed Frame : #{frame.get('frame_seq', 0)} (FD passed locklessly; CPU mmap skipped)",
        ]
        y = 95
        for ln in lines:
            p.drawText(30, y, ln)
            y += 28

        # Pulsing DMA-BUF activity meter
        pulse = int((frame.get("frame_seq", 0) * 19) % 520)
        p.fillRect(30, 260, 580, 22, QColor("#111b2e"))
        p.fillRect(32 + pulse, 262, 56, 18, QColor(t["accent_primary"]))
        p.setPen(QColor(t["text_muted"]))
        p.drawText(30, 315, "GPU DRM Prime FD relayed directly from compositor -> portal consumer.")
        p.end()
        return img

    def _render_sc_simulated_qimage(self, shm_meta: Optional[dict[str, Any]], note: str = "", live_fps: float = 0.0) -> tuple[QImage, int, int]:
        rk, mw, mh, fps_val, _cap = self._current_sc_profile()
        src_w, src_h = self._get_primary_source_size()
        dw, dh = self._compute_target_dims(src_w, src_h, mw, mh)
        img = QImage(dw, dh, QImage.Format.Format_RGB32)
        img.fill(QColor("#0a0f1d"))
        p = QPainter(img)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.scale(dw / 640.0, dh / 360.0)
        t = self.theme
        now = time.monotonic()
        dt = max(0.0, min(0.25, now - self._sc_sim_last_t))
        self._sc_sim_last_t = now
        self._sc_sim_phase = (self._sc_sim_phase + 1) % 1000000
        self._sc_sim_anim = (getattr(self, "_sc_sim_anim", 0.0) + dt * 30.0) % 100000.0
        anim = self._sc_sim_anim

        # Background Wayland grid
        p.setPen(QPen(QColor("#152036"), 1))
        for gx in range(0, 640, 32):
            p.drawLine(gx, 24, gx, 330)
        for gy in range(24, 332, 32):
            p.drawLine(0, gy, 640, gy)

        # Simulated KDE/Wayland top panel
        fps_str = "Source" if fps_val <= 0 else f"{fps_val}"
        live_str = f"{live_fps:.1f}" if live_fps > 0.1 else fps_str
        p.fillRect(0, 0, 640, 24, QColor("#111827"))
        p.setPen(QColor(t["accent_secondary"]))
        p.setFont(QFont("Monospace", 9, QFont.Weight.Bold))
        p.drawText(10, 16, f"DRY-RUN SCREENCAST  |  {rk} ({dw}x{dh}) @ {fps_str} FPS (Live {live_str})")
        p.setPen(QColor(t["text_muted"]))
        p.drawText(540, 16, time.strftime("%H:%M:%S"))

        # Moving simulated shared application window
        wx = 55 + int(95 * (0.5 + 0.5 * math.sin(anim * 0.05)))
        wy = 48 + int(22 * (0.5 + 0.5 * math.cos(anim * 0.04)))
        p.fillRect(wx, wy, 360, 196, QColor("#131d31"))
        p.setPen(QPen(QColor(t["accent_primary"]), 2))
        p.drawRoundedRect(wx, wy, 360, 196, 8, 8)
        p.fillRect(wx + 2, wy + 2, 356, 26, QColor("#1d2a44"))
        p.setPen(QColor(t["text_bright"]))
        p.setFont(QFont("Monospace", 9, QFont.Weight.Bold))
        p.drawText(wx + 10, wy + 19, f"Simulated Wayland Window ({dw}x{dh} BGRx)")

        # Color calibration swatches inside mock window (visual verification)
        swatches = ["#ef4444", "#22c55e", "#3b82f6", "#f59e0b", "#a855f7", "#06b6d4", "#f8fafc"]
        for i, col in enumerate(swatches):
            p.fillRect(wx + 16 + i * 47, wy + 40, 40, 44, QColor(col))

        # Animated SPA 4-slot buffer ring bar
        slot = self._sc_sim_phase % 4
        p.setPen(QColor(t["text_secondary"]))
        p.setFont(QFont("Monospace", 8))
        p.drawText(wx + 16, wy + 108, "SPA_IO_Buffers Ring Relay (Prod -> Broker -> Cons):")
        for b in range(4):
            bx = wx + 16 + b * 82
            p.fillRect(bx, wy + 116, 74, 24, QColor(t["accent_primary"] if b == slot else "#1e293b"))
            p.setPen(QColor("#090d16" if b == slot else t["text_muted"]))
            p.drawText(bx + 8, wy + 132, f"BUF[{b}]")

        pw_d = self._latest_telem.get("pw") or {}
        p.setPen(QColor(t["text_muted"]))
        p.drawText(
            wx + 16,
            wy + 166,
            f"socket: pipewire-0 ({'Salsa' if pw_d.get('broker_running') else 'offline'})  "
            f"audio_rejects: {pw_d.get('audio_rejects', 0)}",
        )
        p.drawText(
            wx + 16,
            wy + 182,
            f"profile={rk} ({dw}x{dh}@{fps_str})  frame={self._sc_sim_phase}  shm={bool(shm_meta and shm_meta.get('active_stream'))}",
        )

        # Moving Wayland pointer arrow
        cx = wx + 40 + (int(anim * 5) % 280)
        cy = wy + 70 + int(30 * math.sin(anim * 0.08))
        p.setPen(QPen(QColor("#ffffff"), 2))
        p.setBrush(QColor(t["accent_hot"]))
        p.drawEllipse(cx - 5, cy - 5, 10, 10)

        # Bottom dry-run HUD footer
        p.fillRect(0, 326, 640, 34, QColor("#070b14"))
        p.setPen(QColor(t["accent_warn"]))
        p.setFont(QFont("Monospace", 9, QFont.Weight.Bold))
        footer = note or f"DRY-RUN SIMULATION: {dw}x{dh} @ {fps_str} FPS (Zero real screen capture)"
        p.drawText(10, 347, footer[:82])
        p.end()
        return img, dw, dh

    def _on_sc_tick(self) -> None:
        if not self.btn_sc_live.isChecked():
            return

        now = time.monotonic()
        if now - self._sc_last_ping_mono >= 3.0:
            self._send_pw_preview_cmd(True)
            self._sc_last_ping_mono = now

        pw_d = self._latest_telem.get("pw") or {}
        if pw_d.get("preview_shm_name"):
            self.shm_reader.set_shm_name(pw_d["preview_shm_name"])

        rk, mw, mh, fps_val, cap_real = self._current_sc_profile()
        fps_str = "Source Hz" if fps_val <= 0 else f"{fps_val} FPS"
        mode = self._sc_mode
        frame = self.shm_reader.read_frame()
        real_active = bool(frame and frame.get("active_stream"))

        # 1. Live active SHM frame path
        if real_active and ("Auto" in mode or "Real Cast" in mode):
            fseq = int(frame.get("frame_seq", 0) or 0)
            if fseq != self._sc_last_shm_seq:
                self._sc_last_shm_seq = fseq
                ui_fps = self._record_ui_fps(now)
            else:
                ui_fps = self._record_ui_fps(now) if not self._sc_ui_fps_times else 0.0
            brk_fps = float(frame.get("actual_fps", 0.0) or 0.0)
            shown_fps = brk_fps if brk_fps > 0.1 else ui_fps
            cap_note = f" | Broker Cap: {frame.get('cast_fps_cap')} FPS" if frame.get("cast_fps_cap") else ""

            if frame and frame.get("pixels") and frame.get("dst_width", 0) > 0:
                w = frame["dst_width"]
                h = frame["dst_height"]
                stride = frame["dst_stride"]
                qimg = QImage(frame["pixels"], w, h, stride, QImage.Format.Format_RGB32).copy()
                self.lbl_sc_feed_hdr.setText(
                    f"Viewport: ● LIVE SCREENCAST  |  {frame.get('prod_name') or 'node'} "
                    f"(Src {frame.get('src_width')}x{frame.get('src_height')} -> {rk} {w}x{h} @ {fps_str}, Live {shown_fps:.1f} FPS)"
                )
                self.lbl_sc_stream_meta.setText(
                    f"SHM: LIVE ({w}x{h} {frame.get('format_name')}, {shown_fps:.1f} FPS{cap_note}) | "
                    f"Producer=#{frame.get('prod_node_id')} | Cons={frame.get('cons_count')} | "
                    f"Transport={'Linear DmaBuf' if frame.get('is_dmabuf') else 'MemFd'} | Frame #{fseq}"
                )
                self._update_sc_profile_badge(shown_fps, f"{w}x{h}")
                self._display_sc_qimage(qimg)
                return
            if frame and frame.get("is_dmabuf"):
                dw, dh = self._compute_target_dims(frame.get("src_width", 1920), frame.get("src_height", 1080), mw, mh)
                self.lbl_sc_feed_hdr.setText(
                    f"Viewport: ● LIVE ZERO-COPY DMA-BUF  |  {frame.get('prod_name') or 'node'} "
                    f"({rk} @ {fps_str}, Live {shown_fps:.1f} FPS)"
                )
                self.lbl_sc_stream_meta.setText(
                    f"SHM: DMA-BUF HUD | Producer=#{frame.get('prod_node_id')} | "
                    f"Src {frame.get('src_width')}x{frame.get('src_height')} {frame.get('format_name')} | "
                    f"Mod=0x{frame.get('drm_modifier', 0):x}{cap_note} | Frame #{fseq}"
                )
                self._update_sc_profile_badge(shown_fps, f"{dw}x{dh}")
                self._display_sc_qimage(self._render_sc_dmabuf_hud_qimage(frame))
                return

        # 2. Real Cast Only mode with no active stream
        if "Real Cast Only" in mode:
            self.lbl_sc_feed_hdr.setText(
                f"Viewport: Waiting for active ScreenCast stream ({rk} @ {fps_str})..."
            )
            self._update_sc_profile_badge()
            if frame and frame.get("frame_seq", 0) > 0 and frame.get("pixels"):
                qimg = QImage(
                    frame["pixels"],
                    frame["dst_width"],
                    frame["dst_height"],
                    frame["dst_stride"],
                    QImage.Format.Format_RGB32,
                ).copy()
                self.lbl_sc_stream_meta.setText(
                    f"SHM: LAST CAPTURED FRAME #{frame.get('frame_seq')} (stream idle) | {frame.get('prod_name')}"
                )
                self._display_sc_qimage(qimg)
            else:
                self.lbl_sc_viewport.setPixmap(QPixmap())
                self.lbl_sc_viewport.setText(
                    f"No active ScreenCast stream relaying right now (Target: {rk} @ {fps_str}).\n"
                    "Start a ScreenCast (Discord / OBS / Spectacle / Browser share) or pick a Dry-Run mode."
                )
                self.lbl_sc_stream_meta.setText("SHM: waiting for first frame...")
            return

        # 3. Dry-Run Silent Compositor Mirror (Spectacle / Grim in /dev/shm)
        if "Compositor Mirror" in mode or mode.startswith("Auto"):
            m_seq, png_bytes, m_status = self.dryrun_cap.get_latest()
            if png_bytes:
                cache_key = (m_seq, mw, mh)
                scaled_base = self._sc_mirror_cache_img if self._sc_mirror_cache_key == cache_key else None
                if scaled_base is None:
                    raw_img = QImage.fromData(png_bytes, "PNG")
                    if not raw_img.isNull():
                        if raw_img.format() != QImage.Format.Format_RGB32:
                            raw_img = raw_img.convertToFormat(QImage.Format.Format_RGB32)
                        dw, dh = self._compute_target_dims(raw_img.width(), raw_img.height(), mw, mh)
                        if dw != raw_img.width() or dh != raw_img.height():
                            raw_img = raw_img.scaled(
                                dw,
                                dh,
                                Qt.AspectRatioMode.IgnoreAspectRatio,
                                Qt.TransformationMode.SmoothTransformation,
                            )
                        scaled_base = raw_img
                        self._sc_mirror_cache_key = cache_key
                        self._sc_mirror_cache_img = scaled_base
                if scaled_base is not None and not scaled_base.isNull():
                    view_fps = self._record_ui_fps(now)
                    snap_fps = self.dryrun_cap.get_capture_fps()
                    img = scaled_base.copy()
                    bar_h = max(24, min(42, img.height() // 18))
                    p = QPainter(img)
                    p.fillRect(0, max(0, img.height() - bar_h), img.width(), bar_h, QColor(7, 11, 20, 210))
                    p.setPen(QColor(self.theme["accent_primary"]))
                    p.setFont(QFont("Monospace", max(8, min(14, bar_h // 2)), QFont.Weight.Bold))
                    p.drawText(
                        12,
                        max(16, img.height() - max(7, bar_h // 3)),
                        f"DRY-RUN MIRROR #{m_seq}  |  {rk} ({img.width()}x{img.height()}) @ {fps_str}  |  View {view_fps:.1f} FPS (Cap {snap_fps:.1f}/s)  |  {time.strftime('%H:%M:%S')}",
                    )
                    p.end()
                    self.lbl_sc_feed_hdr.setText(
                        f"Viewport: DRY-RUN COMPOSITOR MIRROR #{m_seq} ({rk} {img.width()}x{img.height()} @ {fps_str} | View {view_fps:.1f} FPS)"
                    )
                    self.lbl_sc_stream_meta.setText(
                        f"Dry-Run Mirror: {img.width()}x{img.height()} | Snapshot Rate: {snap_fps:.1f} FPS | UI Cadence: {view_fps:.1f} FPS | 0 real casts"
                    )
                    self._update_sc_profile_badge(view_fps, f"{img.width()}x{img.height()}")
                    self._display_sc_qimage(img)
                    return
            # Fallback to simulated test card while compositor snapshot initializes or if headless
            view_fps = self._record_ui_fps(now)
            note = (
                f"Dry-Run Test Card (mirror fallback: {m_status})"
                if m_status and m_status != "stopped"
                else "Dry-Run Test Card (initializing silent compositor mirror...)"
            )
            sim_img, dw, dh = self._render_sc_simulated_qimage(frame, note=note, live_fps=view_fps)
            self.lbl_sc_feed_hdr.setText(
                f"Viewport: DRY-RUN SIMULATED SCREENCAST ({rk} {dw}x{dh} @ {fps_str} | Live {view_fps:.1f} FPS)"
            )
            self.lbl_sc_stream_meta.setText(f"Dry-Run: {note}")
            self._update_sc_profile_badge(view_fps, f"{dw}x{dh}")
            self._display_sc_qimage(sim_img)
            return

        # 4. Dry-Run Simulated Wayland ScreenCast (target-resolution & target-FPS synthetic card)
        view_fps = self._record_ui_fps(now)
        sim_img, dw, dh = self._render_sc_simulated_qimage(frame, live_fps=view_fps)
        self.lbl_sc_feed_hdr.setText(
            f"Viewport: DRY-RUN SIMULATED WAYLAND SCREENCAST ({rk} {dw}x{dh} @ {fps_str} | Live {view_fps:.1f} FPS)"
        )
        self.lbl_sc_stream_meta.setText(
            f"Dry-Run Simulated: {dw}x{dh}@{fps_str} BGRx (Live {view_fps:.1f} FPS) | "
            f"MemFd SPA Ring Frame #{self._sc_sim_phase} | Real Cast Cap: {'ON' if cap_real else 'OFF'}"
        )
        self._update_sc_profile_badge(view_fps, f"{dw}x{dh}")
        self._display_sc_qimage(sim_img)

    def _render_screencast_telemetry(self) -> None:
        pw = self._latest_telem.get("pw") or {}
        if not pw:
            return
        t = self.theme
        listening = bool(pw.get("broker_running"))
        zero_mode = bool(pw.get("zero_pw_mode"))
        casting = bool(pw.get("active_casting"))

        state_txt = "LISTENING" if listening else "STANDBY (Zero-PipeWire off)"
        mode_txt = "Zero-PipeWire Active" if zero_mode else "Legacy/Upstream PW"
        cast_txt = "CASTING" if casting else "Idle"
        with self._work_lock:
            pending_msg = getattr(self, "_pending_sc_status", None)
            if pending_msg is not None:
                self._pending_sc_status = None
        if pending_msg:
            self.lbl_sc_status.setText(pending_msg)
        else:
            self.lbl_sc_status.setText(
                f"Broker: {state_txt} | Policy: {mode_txt} | Stream State: {cast_txt} | "
                f"Socket: {pw.get('socket_path') or '--'}"
            )
        self.lbl_sc_status.setStyleSheet(
            f"color: {t['accent_primary'] if listening else t['accent_warn']}; font-weight: 700;"
        )

        self.lbl_sc_metrics.setText(
            f"Clients: {pw.get('num_clients', 0)} | Nodes: {pw.get('num_nodes', 0)} | "
            f"Links: {pw.get('num_links', 0)} | Relayed: {pw.get('frames_relayed_total', 0)} | "
            f"Dropped: {pw.get('frames_dropped', 0)} | DmaBuf: {pw.get('dmabuf_streams', 0)} | "
            f"MemFd: {pw.get('memfd_streams', 0)} | Audio Rejects: {pw.get('audio_rejects', 0)}"
        )

        clients = pw.get("clients") or []
        self.tbl_pw_clients.setRowCount(len(clients))
        for r, cl in enumerate(clients):
            self.tbl_pw_clients.setItem(r, 0, _make_item(str(cl.get("id", "")), bold=True))
            self.tbl_pw_clients.setItem(r, 1, _make_item(str(cl.get("pid", ""))))
            self.tbl_pw_clients.setItem(r, 2, _make_item(str(cl.get("app_name") or cl.get("binary") or "")))
            self.tbl_pw_clients.setItem(r, 3, _make_item(str(cl.get("portal_app_id") or ("portal" if cl.get("is_portal") else "-"))))
            self.tbl_pw_clients.setItem(r, 4, _make_item(str(cl.get("num_nodes", 0))))

        nodes = pw.get("nodes") or []
        self.tbl_pw_nodes.setRowCount(len(nodes))
        state_names = {0: "error", 1: "creating", 2: "suspended", 3: "idle", 4: "running"}
        for r, nd in enumerate(nodes):
            st_i = int(nd.get("state", 0))
            st_s = state_names.get(st_i, str(st_i))
            dir_s = "out" if nd.get("direction") == 1 else "in"
            w = nd.get("width", 0)
            h = nd.get("height", 0)
            fmt_s = f"{nd.get('format_name') or '--'} {w}x{h} ({nd.get('mem_type') or 'idle'})" if w else (nd.get("mem_type") or "idle")
            fps_d = max(1, int(nd.get("fps_denom", 1) or 1))
            fps_s = f"{int(nd.get('fps_num', 0)) // fps_d}fps / {nd.get('frames', 0)}"
            self.tbl_pw_nodes.setItem(r, 0, _make_item(str(nd.get("id", "")), bold=True))
            self.tbl_pw_nodes.setItem(r, 1, _make_item(str(nd.get("name") or "")))
            self.tbl_pw_nodes.setItem(r, 2, _make_item(str(nd.get("media_class") or "")))
            self.tbl_pw_nodes.setItem(
                r,
                3,
                _make_item(f"{dir_s}/{st_s}", fg=t["accent_primary"] if st_i == 4 else t["text_secondary"]),
            )
            self.tbl_pw_nodes.setItem(r, 4, _make_item(fmt_s))
            self.tbl_pw_nodes.setItem(r, 5, _make_item(fps_s))

        links = pw.get("links") or []
        self.tbl_pw_links.setRowCount(len(links))
        lstate_names = {0: "init", 1: "negotiating", 2: "allocating", 3: "paused", 4: "active"}
        for r, lk in enumerate(links):
            ls = int(lk.get("state", 0))
            self.tbl_pw_links.setItem(r, 0, _make_item(str(lk.get("id", "")), bold=True))
            self.tbl_pw_links.setItem(r, 1, _make_item(f"Node #{lk.get('out_node_id')} -> Node #{lk.get('in_node_id')}"))
            self.tbl_pw_links.setItem(
                r,
                2,
                _make_item(lstate_names.get(ls, str(ls)), fg=t["accent_primary"] if ls == 4 else t["accent_warn"]),
            )
            self.tbl_pw_links.setItem(r, 3, _make_item(str(lk.get("last_buf_id", 0))))
            self.tbl_pw_links.setItem(r, 4, _make_item(str(lk.get("frames_relayed", 0))))

    # ------------------------------------------------------------------
    # Actions & Background Workers
    # ------------------------------------------------------------------
    def _on_pause_toggled(self, checked: bool) -> None:
        self._paused = checked
        self.btn_pause.setText("Resume Stream" if checked else "Pause Stream")

    def _on_clear_events(self) -> None:
        self._all_events.clear()
        self.client.clear_events()
        self.tbl_events.setRowCount(0)
        self.txt_event_detail.clear()

    def _on_reset_net_clients(self) -> None:
        if self._net_reset_busy:
            return
        pw = self._latest_telem.get("pw") or {}
        cmd = "RESET_NET_CLIENTS"
        if bool(pw.get("active_casting")):
            box = QMessageBox(self)
            box.setIcon(QMessageBox.Icon.Warning)
            box.setWindowTitle("Active ScreenCast Detected")
            box.setText(
                "A live PipeWire ScreenCast stream is currently active.\n\n"
                "Portal ScreenCast apps (Discord / OBS) cannot auto-reconnect if their PipeWire "
                "socket is flushed — resetting PipeWire will freeze/end the stream."
            )
            btn_pa_only = box.addButton(
                "Reset Audio Only (Keep Stream)", QMessageBox.ButtonRole.AcceptRole
            )
            btn_full = box.addButton(
                "Reset Audio + PipeWire (Ends Stream)", QMessageBox.ButtonRole.DestructiveRole
            )
            box.addButton(QMessageBox.StandardButton.Cancel)
            box.setDefaultButton(btn_pa_only)
            box.exec()
            clicked = box.clickedButton()
            if clicked == btn_pa_only:
                cmd = "RESET_PA_CLIENTS"
            elif clicked == btn_full:
                cmd = "RESET_NET_CLIENTS"
            else:
                return

        self._net_reset_busy = True
        self.btn_reset_net.setEnabled(False)
        self.lbl_net_reset_status.setText(
            "Flushing PA audio clients..." if cmd == "RESET_PA_CLIENTS" else "Flushing PA/PW clients..."
        )

        def worker() -> None:
            try:
                reply = self.client.send_command_wait(cmd, timeout=2.5)
                msg = (reply or {}).get("msg") or "Timed out resetting network clients"
                self.client.send_command("SNAPSHOT")
                with self._work_lock:
                    self._pending_net_reset_msg = msg
            finally:
                self._net_reset_busy = False

        threading.Thread(target=worker, daemon=True).start()

    def _on_export_bundle(self) -> None:
        os.makedirs(DEFAULT_BUNDLE_DIR, exist_ok=True)
        snap = self.client.get_snapshot()
        try:
            default_path = DiagnosticProbes.export_debug_bundle(
                client_snapshot=snap,
                events=self._all_events[-1500:],
                audit_data=self._latest_boundary or None,
                procs=self._latest_procs or None,
            )
            chosen, _ = QFileDialog.getSaveFileName(
                self, "Save SALSA Debug Bundle", default_path, "JSON Bundles (*.json)"
            )
            if chosen and chosen != default_path:
                os.replace(default_path, chosen)
                final_path = chosen
            else:
                final_path = default_path
            QMessageBox.information(self, "Debug Bundle Exported", f"Saved complete diagnostic snapshot to:\n{final_path}")
        except OSError as exc:
            QMessageBox.warning(self, "Export Failed", str(exc))

    def _run_probe(self, label: str, fn: Callable[[], dict[str, Any]]) -> None:
        self.txt_probe_out.setPlainText(f"Running {label} probe...")

        def worker() -> None:
            res = fn()
            txt = f"[{label.upper()}] {json.dumps(res, indent=2)}"
            with self._work_lock:
                self._pending_probe_msg = txt

        threading.Thread(target=worker, daemon=True).start()

    def _auto_refresh_audits(self) -> None:
        idx = self.subtabs.currentIndex()
        if idx == 1 and self.chk_proc_auto.isChecked():
            self.refresh_proc_audit()
        elif idx in (2, 3):
            self.refresh_boundary_audit()

    def refresh_proc_audit(self) -> None:
        if self._scan_busy:
            return
        self._scan_busy = True
        telem = dict(self._latest_telem) if self._latest_telem else None

        def worker() -> None:
            try:
                rows = SystemAuditScanner.scan_audio_processes(telem)
                with self._work_lock:
                    self._pending_procs = rows
            finally:
                self._scan_busy = False

        threading.Thread(target=worker, daemon=True).start()

    def refresh_boundary_audit(self) -> None:
        if self._boundary_busy:
            return
        self._boundary_busy = True

        def worker() -> None:
            try:
                data = SystemAuditScanner.inspect_boundary()
                with self._work_lock:
                    self._pending_boundary = data
            finally:
                self._boundary_busy = False

        threading.Thread(target=worker, daemon=True).start()

    # ------------------------------------------------------------------
    # Main UI Tick & Renderers
    # ------------------------------------------------------------------
    def _on_tick(self) -> None:
        # 1. Drain new events unless paused
        if not self._paused:
            new_evs, latest_seq = self.client.get_events_since(self._last_ev_seq)
            if latest_seq > self._last_ev_seq:
                self._last_ev_seq = latest_seq
            if new_evs:
                self._all_events.extend(new_evs)
                if len(self._all_events) > 4000:
                    self._all_events = self._all_events[-4000:]
                self._append_events_to_table(new_evs)

        # 2. Check telemetry updates
        snap = self.client.get_snapshot()
        self._update_health_banner(snap)
        if not self._paused and snap.get("telem_seq", -1) != self._last_telem_seq:
            self._last_telem_seq = snap.get("telem_seq", -1)
            self._latest_telem = snap.get("telemetry") or {}
            if self._latest_telem:
                self._render_telemetry()
                self._render_screencast_telemetry()

        # 3. Drain completed background scans/probes
        procs = boundary = probe_msg = sc_status = net_reset_msg = None
        with self._work_lock:
            if self._pending_procs is not None:
                procs = self._pending_procs
                self._pending_procs = None
            if self._pending_boundary is not None:
                boundary = self._pending_boundary
                self._pending_boundary = None
            if self._pending_probe_msg is not None:
                probe_msg = self._pending_probe_msg
                self._pending_probe_msg = None
            if self._pending_sc_status is not None:
                sc_status = self._pending_sc_status
                self._pending_sc_status = None
            if self._pending_net_reset_msg is not None:
                net_reset_msg = self._pending_net_reset_msg
                self._pending_net_reset_msg = None

        if procs is not None:
            self._latest_procs = procs
            self._render_proc_table()
        if boundary is not None:
            self._latest_boundary = boundary
            self._render_boundary()
        if probe_msg is not None:
            self.txt_probe_out.setPlainText(probe_msg)
        if sc_status is not None:
            self.lbl_sc_status.setText(sc_status)
        if net_reset_msg is not None:
            self.lbl_net_reset_status.setText(net_reset_msg)
            self.btn_reset_net.setEnabled(True)

    def _event_matches(self, ev: dict[str, Any]) -> bool:
        want_sub = self.cmb_sub.currentText()
        if want_sub != "ALL" and ev.get("sub") != want_sub:
            return False
        want_lvl = LEVEL_RANK.get(self.cmb_lvl.currentText(), 0)
        if LEVEL_RANK.get(ev.get("lvl", "INFO"), 1) < want_lvl:
            return False
        q = self.txt_search.text().strip().lower()
        if q:
            hay = " ".join(
                (
                    str(ev.get("pid", "")),
                    str(ev.get("sid", "")),
                    str(ev.get("op", "")),
                    str(ev.get("target", "")),
                    str(ev.get("detail", "")),
                    str(ev.get("sub", "")),
                )
            ).lower()
            if q not in hay:
                return False
        return True

    def _sub_color(self, sub: str) -> str:
        t = self.theme
        return {
            "ENGINE": t["accent_primary"],
            "WASAPI": t["accent_out"],
            "PA": t["accent_secondary"],
            "BLUEZ": "#60a5fa",
            "MIDI": t["accent_midi"],
            "SHIM": t["accent_warn"],
            "ALSA": t["accent_mic"],
            "PW_VIDEO": "#22d3ee",
            "SYSTEM": t["text_secondary"],
        }.get(sub, t["text_primary"])

    def _lvl_color(self, lvl: str) -> str:
        t = self.theme
        return {
            "TRACE": t["text_muted"],
            "INFO": t["text_primary"],
            "WARN": t["accent_warn"],
            "ERR": t["accent_hot"],
        }.get(lvl, t["text_primary"])

    def _append_event_row(self, ev: dict[str, Any]) -> None:
        r = self.tbl_events.rowCount()
        if r >= MAX_VISIBLE_EVENTS:
            self.tbl_events.removeRow(0)
            r = MAX_VISIBLE_EVENTS - 1
        self.tbl_events.insertRow(r)
        sub = str(ev.get("sub", ""))
        lvl = str(ev.get("lvl", "INFO"))
        v1, v2, hr = ev.get("v1", 0), ev.get("v2", 0), ev.get("hr", 0)
        val_str = f"{v1}/{v2}" + (f" 0x{hr & 0xffffffff:x}" if hr else "")
        self.tbl_events.setItem(r, 0, _make_item(str(ev.get("time_str", "")), self.theme["text_muted"], data=ev))
        self.tbl_events.setItem(r, 1, _make_item(sub, self._sub_color(sub), bold=True))
        self.tbl_events.setItem(r, 2, _make_item(lvl, self._lvl_color(lvl), bold=(lvl in ("WARN", "ERR"))))
        self.tbl_events.setItem(r, 3, _make_item(str(ev.get("op", "")), self.theme["text_bright"], bold=True))
        self.tbl_events.setItem(r, 4, _make_item(str(ev.get("target", ""))))
        self.tbl_events.setItem(r, 5, _make_item(str(ev.get("pid", 0))))
        self.tbl_events.setItem(r, 6, _make_item(str(ev.get("sid", 0))))
        self.tbl_events.setItem(r, 7, _make_item(val_str, self.theme["text_secondary"]))
        self.tbl_events.setItem(r, 8, _make_item(str(ev.get("detail", ""))))

    def _append_events_to_table(self, evs: list[dict[str, Any]]) -> None:
        matched = [ev for ev in evs if self._event_matches(ev)]
        if not matched:
            return
        self.tbl_events.setUpdatesEnabled(False)
        try:
            for ev in matched[-400:]:
                self._append_event_row(ev)
        finally:
            self.tbl_events.setUpdatesEnabled(True)
        if self.chk_autoscroll.isChecked():
            self.tbl_events.scrollToBottom()

    def _rebuild_event_table(self) -> None:
        matched = [ev for ev in self._all_events if self._event_matches(ev)]
        self.tbl_events.setUpdatesEnabled(False)
        try:
            self.tbl_events.setRowCount(0)
            for ev in matched[-MAX_VISIBLE_EVENTS:]:
                self._append_event_row(ev)
        finally:
            self.tbl_events.setUpdatesEnabled(True)
        if self.chk_autoscroll.isChecked():
            self.tbl_events.scrollToBottom()

    def _on_event_selected(self) -> None:
        row = self.tbl_events.currentRow()
        if row < 0:
            return
        it = self.tbl_events.item(row, 0)
        if it is None:
            return
        ev = it.data(Qt.ItemDataRole.UserRole)
        if ev:
            self.txt_event_detail.setPlainText(json.dumps(ev, indent=2))

    def _update_health_banner(self, snap: Optional[dict[str, Any]] = None) -> None:
        if snap is None:
            snap = self.client.get_snapshot()
        t = self.theme
        connected = bool(snap.get("connected"))
        hello = snap.get("hello") or {}
        telem = snap.get("telemetry") or {}
        if connected:
            dpid = hello.get("pid", "?")
            drop = telem.get("dropped_events", hello.get("dropped_events", 0))
            self.lbl_conn.setText(f"● LIVE CONNECTED (audiodg PID {dpid} | dropped={drop})")
            self.lbl_conn.setStyleSheet(f"color: {t['accent_primary']}; font-weight: 800;")
        else:
            err = snap.get("last_error", "waiting")
            self.lbl_conn.setText(f"● DISCONNECTED ({err})")
            self.lbl_conn.setStyleSheet(f"color: {t['accent_hot']}; font-weight: 800;")

        eng = telem.get("engine") or {}
        rt = telem.get("rt") or {}
        bt = telem.get("bluez") or {}
        if eng:
            prio = eng.get("realtime_priority", 0)
            self.lbl_rt_pill.setText(
                f"SCHED_FIFO={prio} | {eng.get('sample_rate', 0)}Hz/{eng.get('format_bits', 32)}b "
                f"q={eng.get('period_frames', 0)} | load={rt.get('cpu_load_pct', 0):.1f}% "
                f"(avg={rt.get('avg_cycle_us', 0):.1f}us max={rt.get('max_cycle_us', 0):.1f}us "
                f"jitter_max={rt.get('max_jitter_us', 0):.1f}us) | XRUNs={eng.get('hw_xruns', 0)}"
            )
            bt_str = (
                f"{bt.get('alias') or 'BT'} [{bt.get('codec') or 'A2DP'}] err={bt.get('tx_errors', 0)} und={bt.get('tx_underruns', 0)}"
                if bt.get("active") or bt.get("has_transport")
                else ("Ready (Idle)" if bt.get("endpoints_registered") else "Idle")
            )
            pw = telem.get("pw") or {}
            pw_str = (
                f"Video-Only ({len(pw.get('links', []))} link)"
                if pw.get("broker_running")
                else ("Standby" if pw.get("zero_pw_mode") else "Off")
            )
            self.lbl_hw_pill.setText(
                f"Out: {eng.get('hw_device', '--')} [{eng.get('active_playback_id', '--')}] | "
                f"In: {eng.get('hw_capture_device', '--')} | BlueZ: {bt_str} | "
                f"PA v35 Clients: {eng.get('pa_clients', 0)} | MIDI: {eng.get('midi_events_routed', 0)} keys | "
                f"PW Video: {pw_str}"
            )

        wire = telem.get("wire") or {}
        if connected and wire:
            is_rust = bool(wire.get("is_rust"))
            armed = bool(wire.get("armed", True))
            trips = int(wire.get("trips_total", 0))
            if is_rust:
                self.lbl_wire_pill.setText("● Rust Wire Core + Server/Broker: ACTIVE")
                self.lbl_wire_pill.setStyleSheet(f"color: {t['accent_primary']}; font-weight: 700;")
            else:
                self.lbl_wire_pill.setText("● C11 Wire Fallback Core: ACTIVE")
                self.lbl_wire_pill.setStyleSheet(f"color: {t['accent_warn']}; font-weight: 700;")
            self.lbl_failsafe_pill.setText(
                f"● Network Failsafe: {'ARMED' if armed else 'OFF'} "
                f"(Quarantines Bad Sockets · Trips: {trips})"
            )
            fs_color = t["accent_primary"] if (armed and trips == 0) else t["text_secondary"]
            self.lbl_failsafe_pill.setStyleSheet(f"color: {fs_color}; font-weight: 600;")
        elif not connected:
            self.lbl_wire_pill.setText("● Wire Core: waiting...")
            self.lbl_wire_pill.setStyleSheet(f"color: {t['text_muted']};")
            self.lbl_failsafe_pill.setText("● Network Failsafe: waiting...")
            self.lbl_failsafe_pill.setStyleSheet(f"color: {t['text_muted']};")

    def _render_telemetry(self) -> None:
        telem = self._latest_telem
        eng = telem.get("engine") or {}
        rt = telem.get("rt") or {}
        bt = telem.get("bluez") or {}
        cap = telem.get("capture") or {}
        midi = telem.get("midi") or {}

        load_pct = min(100, max(0, int(round(float(rt.get("cpu_load_pct", 0.0))))))
        self.bar_cpu_load.setValue(load_pct)
        self.bar_cpu_load.setFormat(
            f"RT Load: {rt.get('cpu_load_pct', 0.0):.2f}%  (Last {rt.get('last_cycle_us', 0):.1f}us / Budget {rt.get('budget_us', 0):.1f}us)"
        )
        self.lbl_rt_detail.setText(
            f"Cycles: {rt.get('cycle_count', 0)}  |  Avg: {rt.get('avg_cycle_us', 0):.1f} us  "
            f"|  Min/Max: {rt.get('min_cycle_us', 0):.1f} / {rt.get('max_cycle_us', 0):.1f} us\n"
            f"Wake Jitter (Last/Max): {rt.get('last_jitter_us', 0):+.1f} / {rt.get('max_jitter_us', 0):.1f} us  "
            f"|  HW XRUNs: {eng.get('hw_xruns', 0)}\n"
            f"Stages: PA Mix={rt.get('pa_mix_us', 0):.1f}us  |  EQ APO={rt.get('eq_us', 0):.1f}us  "
            f"|  HW Write={rt.get('hw_write_us', 0):.1f}us\n"
            f"Mode: {'EXCLUSIVE SID #' + str(eng.get('exclusive_owner_id')) if eng.get('exclusive_owner_id', -1) >= 0 else 'SHARED APO MIXER'}  "
            f"|  Out Peaks L/R: {eng.get('output_peak_l', 0):.3f} / {eng.get('output_peak_r', 0):.3f}"
        )

        ring_cap = max(1, int(bt.get("tx_ring_cap", 1) or 1))
        ring_fill = int(bt.get("tx_ring_fill", 0) or 0)
        bt_pct = min(100, max(0, int(round(100.0 * ring_fill / ring_cap))))
        self.bar_bt_ring.setValue(bt_pct)
        self.bar_bt_ring.setFormat(f"BlueZ TX Ring: {ring_fill}/{ring_cap} frames ({bt_pct}%)")
        self.lbl_bt_detail.setText(
            f"State: init={bt.get('initialized')} ep_reg={bt.get('endpoints_registered')} "
            f"direct={bt.get('direct_socket')} transport={bt.get('has_transport')} paused={bt.get('tx_paused')}\n"
            f"Codec/Clock: {bt.get('codec') or '-'} @ {bt.get('sample_rate', 0)}Hz {bt.get('channels', 0)}ch  "
            f"|  MTU R/W: {bt.get('mtu_r', 0)}/{bt.get('mtu_w', 0)}  |  Delay: {bt.get('delay_ms', 0)}ms  AVRCP: {bt.get('avrcp_vol', 0)}\n"
            f"TX Stats: pkts={bt.get('tx_packets', 0)}  underruns={bt.get('tx_underruns', 0)}  errors={bt.get('tx_errors', 0)}\n"
            f"ALSA Capture: active={cap.get('active')} dev={cap.get('device_id') or 'none'} "
            f"({cap.get('hw_rate', 0)}Hz meas={cap.get('measured_rate', 0)}Hz "
            f"pref={cap.get('pref_rate', 0) or 'auto'} kicks={cap.get('kick_count', 0)} "
            f"{cap.get('hw_channels', 0)}ch ring={cap.get('ring_fill', 0)}"
            f"{' DUPLEX-LOCK' if cap.get('duplex_locked') else ''})"
        )

        eq_modes = {0: "Static", 1: "Dynamic Compress", 2: "Dynamic Expand"}
        self.lbl_midi_detail.setText(
            f"MIDI Init: {midi.get('initialized')}  |  RawMIDI Open: {midi.get('rawmidi_open')}  "
            f"|  Seq Client:Port = {midi.get('seq_client', -1)}:{midi.get('seq_in_port', -1)}\n"
            f"Controller: {midi.get('controller') or '-'} ({midi.get('dev_path') or '-'})\n"
            f"Total MIDI Key Presses: {midi.get('events_routed', 0)}\n"
            f"EQ APO: en={eng.get('eq_enabled')} mode={eq_modes.get(eng.get('eq_dynamic_mode', 0), '?')} "
            f"preamp={eng.get('eq_preamp_db', 0):.1f}dB lim={eng.get('eq_limiter_enabled')} ({eng.get('eq_preset') or '-'})"
        )

        # WASAPI sessions table
        ws_list = telem.get("wasapi_sessions") or []
        self.tbl_wasapi.setRowCount(len(ws_list))
        for r, ws in enumerate(ws_list):
            fill_pct = 100.0 * ws.get("buffered_frames", 0) / max(1, ws.get("buffer_frames", 1))
            self.tbl_wasapi.setItem(r, 0, _make_item(str(ws.get("session_id"))))
            self.tbl_wasapi.setItem(r, 1, _make_item(str(ws.get("pid"))))
            self.tbl_wasapi.setItem(r, 2, _make_item(str(ws.get("name")), bold=True))
            mode = "EXCLUSIVE" if ws.get("share_mode") == 1 else "SHARED"
            self.tbl_wasapi.setItem(r, 3, _make_item(mode, self.theme["accent_warn"] if mode == "EXCLUSIVE" else None))
            self.tbl_wasapi.setItem(r, 4, _make_item("CAPTURE" if ws.get("is_capture") else "RENDER"))
            self.tbl_wasapi.setItem(r, 5, _make_item(f"{ws.get('rate')}Hz / {ws.get('channels')}ch"))
            vol = f"{ws.get('volume_milli', 1000) / 10.0:.0f}%" + (" MUTED" if ws.get("muted") else "")
            self.tbl_wasapi.setItem(r, 6, _make_item(vol))
            self.tbl_wasapi.setItem(r, 7, _make_item(f"{ws.get('buffered_frames')}/{ws.get('buffer_frames')} ({fill_pct:.0f}%)"))
            self.tbl_wasapi.setItem(r, 8, _make_item(str(ws.get("xruns", 0)), self.theme["accent_hot"] if ws.get("xruns", 0) else None))

        # PA v35 streams table
        ps_list = telem.get("pa_streams") or []
        pc_map = {pc.get("index"): pc for pc in (telem.get("pa_clients") or [])}
        self.tbl_pa_streams.setRowCount(len(ps_list))
        for r, ps in enumerate(ps_list):
            cinfo = pc_map.get(ps.get("client_index"), {})
            cl_str = f"#{ps.get('client_index')} ({cinfo.get('binary') or cinfo.get('name') or '?'})"
            if ps.get("is_monitor"):
                direction = "MONITOR"
            elif ps.get("is_playback"):
                direction = "PLAY"
            else:
                direction = "REC"
            flags = []
            if ps.get("active"):
                flags.append("ACTIVE")
            if ps.get("corked"):
                flags.append("CORKED")
            if ps.get("in_prebuf"):
                flags.append("PREBUF")
            if ps.get("draining"):
                flags.append("DRAIN")
            if ps.get("muted"):
                flags.append("MUTED")
            ring_c = max(1, int(ps.get("ring_cap_frames", 1) or 1))
            buf_f = int(ps.get("buffered_frames", 0) or 0)
            self.tbl_pa_streams.setItem(r, 0, _make_item(str(ps.get("index"))))
            self.tbl_pa_streams.setItem(r, 1, _make_item(cl_str))
            self.tbl_pa_streams.setItem(r, 2, _make_item(str(ps.get("pid"))))
            self.tbl_pa_streams.setItem(r, 3, _make_item(str(ps.get("name")), bold=True))
            self.tbl_pa_streams.setItem(r, 4, _make_item(direction))
            self.tbl_pa_streams.setItem(r, 5, _make_item(" | ".join(flags) or "IDLE", self.theme["accent_primary"] if ps.get("active") else None))
            self.tbl_pa_streams.setItem(r, 6, _make_item(f"{ps.get('rate')}Hz / {ps.get('channels')}ch"))
            self.tbl_pa_streams.setItem(r, 7, _make_item(f"{buf_f}/{ring_c} (tlen={ps.get('tlength_frames')})"))
            self.tbl_pa_streams.setItem(r, 8, _make_item(str(ps.get("device_id") or "-")))

    def _render_proc_table(self) -> None:
        flt = self.cmb_proc_filter.currentText()
        q = self.txt_proc_search.text().strip().lower()
        rows = []
        for r in self._latest_procs:
            sev = r.get("severity", "INFO")
            if flt == "Leaks / Direct ALSA Only" and sev != "WARN":
                continue
            if flt == "Salsa Routed Only" and sev != "OK":
                continue
            if q:
                hay = f"{r['pid']} {r['comm']} {r['route']} {r['cmdline']} {' '.join(r['maps'])} {' '.join(r['sockets'])}".lower()
                if q not in hay:
                    continue
            rows.append(r)

        self.tbl_procs.setRowCount(len(rows))
        for idx, row in enumerate(rows):
            sev = row.get("severity", "INFO")
            fg = self.theme["accent_hot"] if sev == "WARN" else (self.theme["accent_primary"] if sev == "OK" else self.theme["text_secondary"])
            self.tbl_procs.setItem(idx, 0, _make_item(str(row["pid"]), data=row))
            self.tbl_procs.setItem(idx, 1, _make_item(row["comm"], bold=True))
            self.tbl_procs.setItem(idx, 2, _make_item(row["route"], fg=fg, bold=(sev == "WARN")))
            sock_shm = ", ".join(row["sockets"][:2] + [f"shm:{len(row['shm_fds'])}" if row["shm_fds"] else ""])
            self.tbl_procs.setItem(idx, 3, _make_item(sock_shm.strip(", ")))
            self.tbl_procs.setItem(idx, 4, _make_item(", ".join(row["snd_fds"][:2])))
            libs = ", ".join(os.path.basename(m) for m in row["maps"][:4])
            self.tbl_procs.setItem(idx, 5, _make_item(libs))

    def _on_proc_selected(self) -> None:
        row = self.tbl_procs.currentRow()
        if row < 0:
            return
        it = self.tbl_procs.item(row, 0)
        if it is None:
            return
        data = it.data(Qt.ItemDataRole.UserRole)
        if data:
            self.txt_proc_detail.setPlainText(json.dumps(data, indent=2))

    def _render_boundary(self) -> None:
        b = self._latest_boundary
        if not b:
            return

        # Stale build banner
        bf = b.get("build_freshness") or {}
        if bf.get("any_artifact_stale") or bf.get("running_daemon_predates_binary"):
            self.lbl_stale_warn.setText(
                f"WARNING: Stale binary or daemon! src={bf.get('newest_source')} "
                f"stale_art={bf.get('any_artifact_stale')} running_predates_disk={bf.get('running_daemon_predates_binary')}"
            )
            self.lbl_stale_warn.setStyleSheet(f"color: {self.theme['accent_hot']}; font-weight: 700;")
        else:
            self.lbl_stale_warn.setText(
                f"Build Fresh: all artifacts newer than {bf.get('newest_source') or 'sources'} "
                f"| Running audiodg PID(s): {bf.get('running_audiodg_pids')}"
            )
            self.lbl_stale_warn.setStyleSheet(f"color: {self.theme['accent_primary']}; font-weight: 700;")

        # Systemd table
        units = b.get("systemd") or []
        self.tbl_units.setRowCount(len(units))
        for r, u in enumerate(units):
            act = u.get("ActiveState", "")
            uid = u.get("Id", "")
            is_masked = u.get("UnitFileState") == "masked"
            fg = self.theme["accent_primary"] if act == "active" else (self.theme["text_muted"] if is_masked else self.theme["accent_warn"])
            self.tbl_units.setItem(r, 0, _make_item(u.get("Scope", "")))
            self.tbl_units.setItem(r, 1, _make_item(uid, bold=True))
            self.tbl_units.setItem(r, 2, _make_item(act, fg=fg, bold=True))
            self.tbl_units.setItem(r, 3, _make_item(u.get("SubState", "")))
            self.tbl_units.setItem(r, 4, _make_item(u.get("UnitFileState", "")))
            self.tbl_units.setItem(r, 5, _make_item(f"PID={u.get('MainPID', '0')} (R={u.get('NRestarts', '0')})"))

        # Socket table
        socks = b.get("sockets") or {}
        self.tbl_sockets.setRowCount(len(socks))
        for r, (label, info) in enumerate(socks.items()):
            ok = bool(info.get("connectable"))
            fg = self.theme["accent_primary"] if ok else self.theme["accent_warn"]
            self.tbl_sockets.setItem(r, 0, _make_item(label, bold=True))
            self.tbl_sockets.setItem(r, 1, _make_item("LISTENING" if ok else (info.get("error") or "MISSING"), fg=fg))
            self.tbl_sockets.setItem(r, 2, _make_item(str(info.get("peer_pid") or "-")))
            self.tbl_sockets.setItem(r, 3, _make_item(str(info.get("peer_comm") or "-"), fg=fg))
            self.tbl_sockets.setItem(r, 4, _make_item(str(info.get("path") or "")))

        # Active kernel streams into telemetry tab box too
        asound = b.get("asound") or {}
        streams = asound.get("active_streams") or []
        lines = ["=== KERNEL /proc/asound ACTIVE PCM STREAMS ==="]
        if streams:
            for st in streams:
                lines.append(f"[{st['node']}] state={st['state']} {st['owner_pid']}\n{st['hw_params']}")
        else:
            lines.append("(No active kernel PCM substreams — Virtual Clock or BlueZ A2DP active)")
        lines.append("\n=== /proc/asound/cards ===\n" + (asound.get("cards") or "(none)"))
        self.txt_asound_live.setPlainText("\n".join(lines))

        self._render_config_viewer()

    def _render_config_viewer(self) -> None:
        b = self._latest_boundary
        if not b:
            return
        sel = self.cmb_cfg.currentText()
        cfgs = b.get("configs") or {}
        if sel in cfgs:
            entry = cfgs[sel]
            header = f"# Path: {entry['path']} (exists={entry['exists']})\n\n"
            self.txt_cfg_view.setPlainText(header + (entry["content"] or "(file missing or empty)"))
        elif sel == "PipeWire Nodes (pw-dump)":
            pw = b.get("pipewire") or {}
            self.txt_cfg_view.setPlainText(json.dumps(pw, indent=2))
        elif sel == "ALSA Plugin & Build Freshness":
            self.txt_cfg_view.setPlainText(
                json.dumps({"alsa_plugin": b.get("alsa_plugin"), "build_freshness": b.get("build_freshness")}, indent=2)
            )
        elif sel == "/proc/asound/cards + pcm":
            asnd = b.get("asound") or {}
            self.txt_cfg_view.setPlainText(
                f"=== CARDS ===\n{asnd.get('cards', '')}\n\n=== PCM ===\n{asnd.get('pcm', '')}"
            )
