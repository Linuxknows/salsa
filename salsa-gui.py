#!/usr/bin/env python3
"""
Salsa v6.9-beta — Linux WASAPI Studio Control Center, Dynamic EQ & Auto-Graph
==========================================================================
Fixed-Clock Lossless WaveRT Audio Engine (44.1–192 kHz / 16–32-bit),
10-Band Dynamic EQ + AutoEQ Importer + Bauer Crossfeed, Capture Mic APO
(HPF, Noise Gate, RNNoise, Compressor, Sidetone), Hardware DAC/Amp Analog,
BlueZ Codec Switcher / ABR / HFP SCO, DAW MIDI Hub, and Live Signal Topology.
"""

import copy
import ctypes
import json
import math
import os
import re
import stat
import subprocess
import sys
import threading
import time

_REAL_DIR = os.path.dirname(os.path.realpath(__file__))
if _REAL_DIR not in sys.path:
    sys.path.insert(0, _REAL_DIR)
from salsa_debug_core import snapshot_once
from salsa_theme_editor import (
    CUSTOM_THEME_NAME,
    SalsaCustomThemeDialog,
    build_salsa_palette,
    build_salsa_stylesheet,
    normalize_custom_theme,
)
from PyQt6.QtCore import Qt, QTimer, QPointF, QRectF, QSize, QProcess
from PyQt6.QtGui import (
    QColor,
    QFont,
    QIcon,
    QKeySequence,
    QLinearGradient,
    QPainter,
    QPainterPath,
    QPalette,
    QPen,
    QPixmap,
    QShortcut,
)
from PyQt6.QtWidgets import (
    QApplication,
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFileDialog,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QHeaderView,
    QKeySequenceEdit,
    QLabel,
    QMainWindow,
    QMessageBox,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QSlider,
    QSpinBox,
    QSplitter,
    QTableWidget,
    QTableWidgetItem,
    QTextBrowser,
    QTextEdit,
    QVBoxLayout,
    QWidget,
)

BASE_DIR = os.path.dirname(os.path.realpath(__file__))
LIB_PATH = os.path.join(BASE_DIR, "build", "liblwasapi.so")
CTL_BIN = os.path.join(BASE_DIR, "build", "lwasapi-ctl")
DEBUG_SCRIPT = os.path.join(BASE_DIR, "salsa-debug.py")
ICON_PATH = os.path.join(BASE_DIR, "assets", "salsa-lwasapi.png")
ICON_GLOW_PATH = os.path.join(BASE_DIR, "assets", "salsa-lwasapi-glow.png")

DEFAULT_SHORTCUTS = [
    ("advanced", "Open Advanced Options", "Ctrl+,"),
    ("topology", "Toggle Topology & Manual", "F1"),
    ("debugger", "Open Standalone Debugger", "Ctrl+D"),
    ("mute_spk", "Toggle Speaker Mute", "Ctrl+M"),
    ("mute_mic", "Toggle Mic Capture Mute", "Ctrl+Shift+M"),
    ("toggle_eq", "Toggle 10-Band Dynamic EQ", "Ctrl+E"),
    ("toggle_mic_apo", "Toggle Capture Mic APO", "Ctrl+K"),
    ("vol_up", "Speaker Volume +5%", "Ctrl+Up"),
    ("vol_down", "Speaker Volume -5%", "Ctrl+Down"),
    ("mic_up", "Mic Gain +5%", "Ctrl+Alt+Up"),
    ("mic_down", "Mic Gain -5%", "Ctrl+Alt+Down"),
    ("bt_reset", "Reset Bluetooth A2DP", "Ctrl+Shift+B"),
    ("refresh", "Force Status & Devices Refresh", "F5"),
    ("quit", "Quit Studio", "Ctrl+Q"),
]


def detect_host_pc_profile() -> dict:
    cpu_model = "Generic Linux Processor"
    os_name = "Linux"
    pkg_mgr = "System Package Manager"
    alsa_cards = []

    try:
        with open("/proc/cpuinfo", "r", encoding="utf-8") as f:
            for line in f:
                if line.startswith("model name") or line.startswith("Processor"):
                    cpu_model = line.split(":", 1)[1].strip()
                    break
    except Exception:
        pass

    try:
        with open("/etc/os-release", "r", encoding="utf-8") as f:
            for line in f:
                if line.startswith("PRETTY_NAME="):
                    os_name = line.split("=", 1)[1].strip().strip('"')
                    break
    except Exception:
        pass

    for bin_name, label in [
        ("/usr/bin/pacman", "pacman (/var/lib/pacman/local)"),
        ("/usr/bin/dpkg-query", "apt / dpkg (/var/lib/dpkg/status)"),
        ("/usr/bin/dnf", "dnf / rpm (/var/lib/rpm)"),
        ("/usr/bin/zypper", "zypper / rpm (/var/lib/rpm)"),
    ]:
        if os.path.exists(bin_name):
            pkg_mgr = label
            break

    try:
        with open("/proc/asound/cards", "r", encoding="utf-8") as f:
            for line in f:
                s = line.strip()
                if s and s[0].isdigit() and ":" in s:
                    alsa_cards.append(s)
    except Exception:
        pass

    return {
        "cpu": cpu_model,
        "os": os_name,
        "pkg_mgr": pkg_mgr,
        "cards": alsa_cards if alsa_cards else ["Auto-Discovered Kernel ALSA / BlueZ Endpoints"],
        "base_dir": BASE_DIR,
    }


HOST_PROFILE = detect_host_pc_profile()

BAND_FREQS = ["31 Hz", "62 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"]

from salsa_ipc import (  # noqa: E402
    DEFAULT_BAND_FREQS_HZ,
    DEFAULT_VOL_STEP_PCT,
    LWASAPI_BT_CAP_AAC,
    LWASAPI_BT_CAP_APTX,
    LWASAPI_BT_CAP_APTX_HD,
    LWASAPI_BT_CAP_LDAC,
    LWASAPI_BT_CAP_SBC,
    LWASAPI_BT_PREF_AAC,
    LWASAPI_BT_PREF_APTX,
    LWASAPI_BT_PREF_APTX_HD,
    LWASAPI_BT_PREF_AUTO,
    LWASAPI_BT_PREF_LDAC,
    LWASAPI_BT_PREF_SBC,
    LWASAPI_CFG_FLAG_CAPTURE_RATE,
    LWASAPI_EQ_BANDS,
    LWASAPI_FILTER_HIGH_SHELF,
    LWASAPI_FILTER_LOW_SHELF,
    LWASAPI_FILTER_PEAK,
    LWASAPI_HW_AMP_AUTO,
    LWASAPI_HW_AMP_HIGH,
    LWASAPI_HW_AMP_LOW,
    LWASAPI_HW_AMP_MED,
    LWASAPI_HW_CAP_DAC_VOL,
    LWASAPI_HW_CAP_HID_BOX,
    LWASAPI_HW_CAP_HIGH_RES,
    LWASAPI_HW_CAP_HP_AMP,
    LWASAPI_HW_CAP_LINE_AIR,
    LWASAPI_HW_CAP_MIC_BOOST,
    LWASAPI_HW_CAP_OUT_ROUTE,
    LWASAPI_HW_CAP_STRIX_ARCH,
    LWASAPI_HW_ROUTE_AUTO,
    LWASAPI_HW_ROUTE_HP,
    LWASAPI_HW_ROUTE_SPK,
    LWASAPI_LDAC_MODE_ABR,
    LWASAPI_LDAC_MODE_HQ_990,
    LWASAPI_LDAC_MODE_MQ_330,
    LWASAPI_LDAC_MODE_SQ_660,
    LWASAPI_MAX_DEVICES,
    LWASAPI_MAX_SESSIONS,
    MIC_APO_PRESETS,
    PRESETS,
    SUPPORTED_SAMPLE_RATES,
    SUPPORTED_VOL_STEPS,
    LwasapiBtCodecCfg,
    LwasapiDeviceEntry,
    LwasapiDeviceList,
    LwasapiEngineStatus,
    LwasapiEqConfig,
    LwasapiHwAnalogCfg,
    LwasapiHwAnalogInfo,
    LwasapiMicApoCfg,
    LwasapiParamEqExt,
    LwasapiSessionInfo,
    bind_liblwasapi,
    normalize_vol_step_pct,
)

CUSTOM_PRESET_NAME = "Custom Salsa Curve"

DYN_MODE_LABELS = {
    0: "Static Parametric EQ",
    1: "Dynamic Compress (Tame Peaks)",
    2: "Dynamic Expand (Upward Punch)",
}

DYN_MODE_SHORT_LABELS = {
    0: "Static Parametric",
    1: "Dynamic Compress",
    2: "Dynamic Expand",
}

DEFAULT_THEME = "Salsa Midnight (Default)"

THEMES = {
    "Salsa Midnight (Default)": {
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
        "canvas_grid": "#0f172a",
        "spectrum_grid": "#162032",
        "grid_zero": "#334155",
        "node_bg_top": "#111827",
        "node_bg_hover": "#162033",
        "node_bg_bot": "#0b1120",
        "node_border": "#334155",
        "wire_pulse": "#ffffff",
        "handle_border": "#ffffff",
        "code_bg": "#1e293b",
    },
    "Plasma Breeze Emerald": {
        "is_light": False,
        "bg_main": "#161b1f",
        "bg_card": "#1e2429",
        "bg_deep": "#181d22",
        "bg_canvas": "#14191d",
        "bg_control": "#252c33",
        "bg_hover": "#2f3841",
        "border_card": "#2c343c",
        "border_control": "#36404a",
        "text_primary": "#e6edf3",
        "text_bright": "#f8fafc",
        "text_secondary": "#cbd5e1",
        "text_muted": "#9ca3af",
        "text_dim": "#6b7280",
        "accent_primary": "#14b866",
        "accent_secondary": "#38bdf8",
        "accent_hot": "#ff5e3a",
        "accent_warn": "#f59e0b",
        "accent_mic": "#f43f5e",
        "accent_midi": "#a855f7",
        "accent_out": "#10b981",
        "btn_primary_bg": "#10b981",
        "btn_primary_border": "#14b866",
        "btn_primary_hover": "#14b866",
        "btn_primary_text": "#ffffff",
        "btn_accent_bg": "#e11d48",
        "btn_accent_border": "#f43f5e",
        "btn_accent_hover": "#f43f5e",
        "btn_accent_text": "#ffffff",
        "combo_popup_bg": "#1e2429",
        "combo_popup_sel_bg": "#252c33",
        "combo_popup_sel_text": "#14b866",
        "canvas_grid": "#1b2127",
        "spectrum_grid": "#20272e",
        "grid_zero": "#36404a",
        "node_bg_top": "#1e2429",
        "node_bg_hover": "#252c33",
        "node_bg_bot": "#171c20",
        "node_border": "#36404a",
        "wire_pulse": "#ffffff",
        "handle_border": "#ffffff",
        "code_bg": "#252c33",
    },
    "CachyOS Teal Void": {
        "is_light": False,
        "bg_main": "#0a0e14",
        "bg_card": "#111722",
        "bg_deep": "#080b10",
        "bg_canvas": "#070a0f",
        "bg_control": "#192231",
        "bg_hover": "#243247",
        "border_card": "#1c2738",
        "border_control": "#2b3c54",
        "text_primary": "#e2f1f8",
        "text_bright": "#f8fafc",
        "text_secondary": "#bfd7e6",
        "text_muted": "#85a1b8",
        "text_dim": "#5b7388",
        "accent_primary": "#00d4aa",
        "accent_secondary": "#22d3ee",
        "accent_hot": "#ff6b4a",
        "accent_warn": "#fbbf24",
        "accent_mic": "#fb7185",
        "accent_midi": "#818cf8",
        "accent_out": "#14b8a6",
        "btn_primary_bg": "#0d9488",
        "btn_primary_border": "#14b8a6",
        "btn_primary_hover": "#14b8a6",
        "btn_primary_text": "#ffffff",
        "btn_accent_bg": "#e11d48",
        "btn_accent_border": "#fb7185",
        "btn_accent_hover": "#f43f5e",
        "btn_accent_text": "#ffffff",
        "combo_popup_bg": "#111722",
        "combo_popup_sel_bg": "#192231",
        "combo_popup_sel_text": "#00d4aa",
        "canvas_grid": "#111c29",
        "spectrum_grid": "#142233",
        "grid_zero": "#2b3c54",
        "node_bg_top": "#111722",
        "node_bg_hover": "#172233",
        "node_bg_bot": "#090d14",
        "node_border": "#2b3c54",
        "wire_pulse": "#ffffff",
        "handle_border": "#ffffff",
        "code_bg": "#192231",
    },
    "Nordic Frost (Nord)": {
        "is_light": False,
        "bg_main": "#242933",
        "bg_card": "#2e3440",
        "bg_deep": "#1f242d",
        "bg_canvas": "#1b2028",
        "bg_control": "#3b4252",
        "bg_hover": "#4c566a",
        "border_card": "#3b4252",
        "border_control": "#4c566a",
        "text_primary": "#e5e9f0",
        "text_bright": "#eceff4",
        "text_secondary": "#d8dee9",
        "text_muted": "#9aa5ba",
        "text_dim": "#6f7b91",
        "accent_primary": "#a3be8c",
        "accent_secondary": "#88c0d0",
        "accent_hot": "#d08770",
        "accent_warn": "#ebcb8b",
        "accent_mic": "#bf616a",
        "accent_midi": "#b48ead",
        "accent_out": "#8fbcbb",
        "btn_primary_bg": "#4b7a62",
        "btn_primary_border": "#a3be8c",
        "btn_primary_hover": "#5c9377",
        "btn_primary_text": "#eceff4",
        "btn_accent_bg": "#a5424c",
        "btn_accent_border": "#bf616a",
        "btn_accent_hover": "#bf616a",
        "btn_accent_text": "#eceff4",
        "combo_popup_bg": "#2e3440",
        "combo_popup_sel_bg": "#434c5e",
        "combo_popup_sel_text": "#88c0d0",
        "canvas_grid": "#262c37",
        "spectrum_grid": "#282f3b",
        "grid_zero": "#4c566a",
        "node_bg_top": "#2e3440",
        "node_bg_hover": "#3b4252",
        "node_bg_bot": "#242933",
        "node_border": "#4c566a",
        "wire_pulse": "#eceff4",
        "handle_border": "#eceff4",
        "code_bg": "#3b4252",
    },
    "Catppuccin Mocha": {
        "is_light": False,
        "bg_main": "#181825",
        "bg_card": "#1e1e2e",
        "bg_deep": "#11111b",
        "bg_canvas": "#11111b",
        "bg_control": "#313244",
        "bg_hover": "#45475a",
        "border_card": "#313244",
        "border_control": "#45475a",
        "text_primary": "#cdd6f4",
        "text_bright": "#f5e0dc",
        "text_secondary": "#bac2de",
        "text_muted": "#9399b2",
        "text_dim": "#6c7086",
        "accent_primary": "#a6e3a1",
        "accent_secondary": "#89b4fa",
        "accent_hot": "#fab387",
        "accent_warn": "#f9e2af",
        "accent_mic": "#f38ba8",
        "accent_midi": "#cba6f7",
        "accent_out": "#94e2d5",
        "btn_primary_bg": "#2f7d57",
        "btn_primary_border": "#a6e3a1",
        "btn_primary_hover": "#3b986b",
        "btn_primary_text": "#ffffff",
        "btn_accent_bg": "#be3b5e",
        "btn_accent_border": "#f38ba8",
        "btn_accent_hover": "#d6486e",
        "btn_accent_text": "#ffffff",
        "combo_popup_bg": "#1e1e2e",
        "combo_popup_sel_bg": "#313244",
        "combo_popup_sel_text": "#cba6f7",
        "canvas_grid": "#1c1c2c",
        "spectrum_grid": "#1e1e2e",
        "grid_zero": "#45475a",
        "node_bg_top": "#1e1e2e",
        "node_bg_hover": "#2a2b3d",
        "node_bg_bot": "#141420",
        "node_border": "#45475a",
        "wire_pulse": "#cdd6f4",
        "handle_border": "#cdd6f4",
        "code_bg": "#313244",
    },
    "Dracula Console": {
        "is_light": False,
        "bg_main": "#21222c",
        "bg_card": "#282a36",
        "bg_deep": "#191a21",
        "bg_canvas": "#17181f",
        "bg_control": "#383b4d",
        "bg_hover": "#44475a",
        "border_card": "#383b4d",
        "border_control": "#6272a4",
        "text_primary": "#f8f8f2",
        "text_bright": "#ffffff",
        "text_secondary": "#d6d8e5",
        "text_muted": "#9ea8cc",
        "text_dim": "#6272a4",
        "accent_primary": "#50fa7b",
        "accent_secondary": "#8be9fd",
        "accent_hot": "#ffb86c",
        "accent_warn": "#f1fa8c",
        "accent_mic": "#ff5555",
        "accent_midi": "#bd93f9",
        "accent_out": "#50fa7b",
        "btn_primary_bg": "#2ea043",
        "btn_primary_border": "#50fa7b",
        "btn_primary_hover": "#3fb950",
        "btn_primary_text": "#ffffff",
        "btn_accent_bg": "#d6336c",
        "btn_accent_border": "#ff79c6",
        "btn_accent_hover": "#e64980",
        "btn_accent_text": "#ffffff",
        "combo_popup_bg": "#282a36",
        "combo_popup_sel_bg": "#44475a",
        "combo_popup_sel_text": "#50fa7b",
        "canvas_grid": "#242633",
        "spectrum_grid": "#262835",
        "grid_zero": "#44475a",
        "node_bg_top": "#282a36",
        "node_bg_hover": "#343746",
        "node_bg_bot": "#1e2029",
        "node_border": "#44475a",
        "wire_pulse": "#f8f8f2",
        "handle_border": "#f8f8f2",
        "code_bg": "#383b4d",
    },
    "Synthwave Outrun": {
        "is_light": False,
        "bg_main": "#0d0618",
        "bg_card": "#160b29",
        "bg_deep": "#080311",
        "bg_canvas": "#07020f",
        "bg_control": "#241342",
        "bg_hover": "#361d61",
        "border_card": "#2e1854",
        "border_control": "#5b21b6",
        "text_primary": "#f3e8ff",
        "text_bright": "#ffffff",
        "text_secondary": "#d8b4fe",
        "text_muted": "#a78bfa",
        "text_dim": "#7c3aed",
        "accent_primary": "#00f5d4",
        "accent_secondary": "#00bbf9",
        "accent_hot": "#ff2a85",
        "accent_warn": "#fee440",
        "accent_mic": "#fb7185",
        "accent_midi": "#c084fc",
        "accent_out": "#00f5d4",
        "btn_primary_bg": "#009688",
        "btn_primary_border": "#00f5d4",
        "btn_primary_hover": "#00bfa5",
        "btn_primary_text": "#ffffff",
        "btn_accent_bg": "#db2777",
        "btn_accent_border": "#ff2a85",
        "btn_accent_hover": "#ec4899",
        "btn_accent_text": "#ffffff",
        "combo_popup_bg": "#160b29",
        "combo_popup_sel_bg": "#2e1854",
        "combo_popup_sel_text": "#00f5d4",
        "canvas_grid": "#1a0d30",
        "spectrum_grid": "#20103a",
        "grid_zero": "#4c1d95",
        "node_bg_top": "#180c2c",
        "node_bg_hover": "#251345",
        "node_bg_bot": "#0c0518",
        "node_border": "#4c1d95",
        "wire_pulse": "#ffffff",
        "handle_border": "#ffffff",
        "code_bg": "#241342",
    },
    "Analog Studio Rack": {
        "is_light": False,
        "bg_main": "#131110",
        "bg_card": "#1d1916",
        "bg_deep": "#0d0b0a",
        "bg_canvas": "#0b0908",
        "bg_control": "#2b2521",
        "bg_hover": "#3c342e",
        "border_card": "#302924",
        "border_control": "#4a3f37",
        "text_primary": "#f5ebe0",
        "text_bright": "#fff8f0",
        "text_secondary": "#d6ccc2",
        "text_muted": "#a89f96",
        "text_dim": "#736960",
        "accent_primary": "#f59e0b",
        "accent_secondary": "#38bdf8",
        "accent_hot": "#ef4444",
        "accent_warn": "#fbbf24",
        "accent_mic": "#fb7185",
        "accent_midi": "#c084fc",
        "accent_out": "#22c55e",
        "btn_primary_bg": "#b45309",
        "btn_primary_border": "#f59e0b",
        "btn_primary_hover": "#d97706",
        "btn_primary_text": "#ffffff",
        "btn_accent_bg": "#b91c1c",
        "btn_accent_border": "#ef4444",
        "btn_accent_hover": "#dc2626",
        "btn_accent_text": "#ffffff",
        "combo_popup_bg": "#1d1916",
        "combo_popup_sel_bg": "#3c342e",
        "combo_popup_sel_text": "#f59e0b",
        "canvas_grid": "#1a1612",
        "spectrum_grid": "#201b16",
        "grid_zero": "#4a3f37",
        "node_bg_top": "#1f1a17",
        "node_bg_hover": "#2d2621",
        "node_bg_bot": "#120f0d",
        "node_border": "#4a3f37",
        "wire_pulse": "#fff8f0",
        "handle_border": "#fff8f0",
        "code_bg": "#2b2521",
    },
    "Daylight Mastering (Light)": {
        "is_light": True,
        "bg_main": "#eef2f6",
        "bg_card": "#ffffff",
        "bg_deep": "#f8fafc",
        "bg_canvas": "#f8fafc",
        "bg_control": "#e2e8f0",
        "bg_hover": "#cbd5e1",
        "border_card": "#cbd5e1",
        "border_control": "#94a3b8",
        "text_primary": "#0f172a",
        "text_bright": "#020617",
        "text_secondary": "#334155",
        "text_muted": "#475569",
        "text_dim": "#64748b",
        "accent_primary": "#047857",
        "accent_secondary": "#0369a1",
        "accent_hot": "#ea580c",
        "accent_warn": "#b45309",
        "accent_mic": "#be123c",
        "accent_midi": "#7e22ce",
        "accent_out": "#059669",
        "btn_primary_bg": "#059669",
        "btn_primary_border": "#047857",
        "btn_primary_hover": "#047857",
        "btn_primary_text": "#ffffff",
        "btn_accent_bg": "#e11d48",
        "btn_accent_border": "#be123c",
        "btn_accent_hover": "#be123c",
        "btn_accent_text": "#ffffff",
        "combo_popup_bg": "#ffffff",
        "combo_popup_sel_bg": "#dbeafe",
        "combo_popup_sel_text": "#0369a1",
        "canvas_grid": "#e2e8f0",
        "spectrum_grid": "#e2e8f0",
        "grid_zero": "#94a3b8",
        "node_bg_top": "#ffffff",
        "node_bg_hover": "#f1f5f9",
        "node_bg_bot": "#e2e8f0",
        "node_border": "#94a3b8",
        "wire_pulse": "#0f172a",
        "handle_border": "#0f172a",
        "code_bg": "#e2e8f0",
    },
}


def _gui_settings_path() -> str:
    cfg_root = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(cfg_root, "lwasapi", "gui_settings.json")


def _salsa_state_path() -> str:
    return os.path.join(os.path.expanduser("~"), ".config", "lwasapi", "salsa_state.bin")


def load_gui_settings() -> dict:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    try:
        fd = os.open(_gui_settings_path(), flags)
        try:
            st = os.fstat(fd)
            if not stat.S_ISREG(st.st_mode) or st.st_size > 262144:
                return {}
            with os.fdopen(fd, "r", encoding="utf-8") as f:
                fd = -1
                data = json.load(f)
        finally:
            if fd >= 0:
                os.close(fd)
        if isinstance(data, dict):
            profiles = data.get("eq_device_profiles")
            if isinstance(profiles, dict):
                clean_profiles = {}
                for k, v in profiles.items():
                    if isinstance(k, str) and k.strip() and len(k) <= 64 and isinstance(v, dict):
                        clean_profiles[k.strip()] = v
                data["eq_device_profiles"] = clean_profiles
            elif "eq_device_profiles" in data:
                data["eq_device_profiles"] = {}
            return data
    except Exception:
        pass
    return {}


def save_gui_settings(data: dict) -> None:
    path = _gui_settings_path()
    cfg_dir = os.path.dirname(path)
    tmp = f"{path}.tmp.{os.getpid()}"
    merged = {}
    try:
        if os.path.isfile(path) and not os.path.islink(path):
            with open(path, "r", encoding="utf-8") as rf:
                on_disk = json.load(rf)
            if isinstance(on_disk, dict):
                merged.update(on_disk)
    except Exception:
        pass
    if isinstance(data, dict):
        for k, v in data.items():
            if not str(k).startswith("tray_"):
                merged[k] = v
            elif k not in merged:
                merged[k] = v
        if "eq_per_device" in data:
            val = bool(data["eq_per_device"])
            merged["eq_per_device"] = val
            merged["eq_per_device_enabled"] = val
        elif "eq_per_device_enabled" in data:
            val = bool(data["eq_per_device_enabled"])
            merged["eq_per_device"] = val
            merged["eq_per_device_enabled"] = val
    try:
        os.makedirs(cfg_dir, mode=0o700, exist_ok=True)
        try:
            os.chmod(cfg_dir, 0o700)
        except Exception:
            pass
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
        fd = os.open(tmp, flags, 0o600)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                fd = -1
                json.dump(merged, f, indent=2)
                f.flush()
                os.fsync(f.fileno())
        finally:
            if fd >= 0:
                os.close(fd)
        os.replace(tmp, path)
    except Exception as e:
        try:
            if os.path.lexists(tmp):
                os.unlink(tmp)
        except Exception:
            pass
        print(f"[Salsa] Warning saving GUI settings: {e}")


OWNERS_MANUAL_CHAPTER_DEFS = {
    "overview": (
        "accent_primary",
        "📘 Chapter 1: Philosophy — Why Salsa Needs Zero Manual Patching",
        "Click any node in the live Signal Topology Graph above (or use the chapter buttons) to inspect how each stage works.",
        """
        <p>In traditional Linux audio utilities like <code>qpwgraph</code> or <code>QjackCtl</code>, users are forced to manually drag virtual patch cables between applications, PulseAudio/JACK bridges, resamplers, and hardware sinks. Whenever a game restarts, a browser tab changes sample rate, or a USB interface reconnects, manual patchbays frequently drop connections.</p>
        <p><b>Salsa — Linux WASAPI (<code>lwasapi-audiodg</code>)</b> replaces manual patching with a <b>100% self-wiring Windows WASAPI / WaveRT Audio Device Graph architecture</b>:</p>
        <ul>
            <li><b>Fixed Lossless Master Clock & Bit Depth Control:</b> Lock your hardware and BlueZ engine clock cleanly across <code>44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz</code> and <code>16 / 24 / 32-bit</code> precision without unpredictable clock hunting.</li>
            <li><b>Automatic Session Discovery & Per-App Routing:</b> Native WASAPI, ALSA, PulseAudio, SDL2/SDL3, OpenAL, Proton/Wine, and JACK apps register automatically with persistent per-app volume, mute, and hardware endpoint routing.</li>
            <li><b>130x Lower Round-Trip Latency:</b> Full-duplex loopback and capture complete in <b>0.33 ms</b> at <code>192 kHz / 64 frames</code> (compared to <code>42.67 ms</code> on legacy PipeWire-Pulse, which Salsa completely replaces).</li>
        </ul>
        """,
    ),
    "apps_in": (
        "accent_secondary",
        "🔊 Chapter 2: Automatic Application Recognition & Hollow ABI Shims",
        "Zero Wrapper Commands Needed — Automatic Desktop-Wide Interception",
        """
        <p>Every desktop program on your PC connects automatically to Stage 1 of the Salsa graph through three transparent mechanisms:</p>
        <ul>
            <li><b>1. Default ALSA & OpenAL Routing (<code>~/.asoundrc</code> & <code>~/.config/OpenAL/alsoft.conf</code>):</b> Defines <code>pcm.!default</code> as an asymmetrical full-duplex <code>type lwasapi</code> device backed by <code>libasound_module_pcm_lwasapi.so</code>. Any ALSA or OpenAL game connects automatically.</li>
            <li><b>2. Hollow ABI Shims & Native PA v35 Server (<code>build/shims/</code> + <code>$XDG_RUNTIME_DIR/pulse/native</code>):</b> Drop-in shared libraries for <code>libasound.so.2</code>, <code>libpulse.so.0</code>, <code>libpulse-simple.so.0</code>, <code>libpulse-mainloop-glib.so.0</code>, and <code>libjack.so.0</code>—plus a built-in PulseAudio Protocol v35 UNIX socket inside <code>lwasapi-audiodg</code>—ensure browsers, Discord, Steam/Proton, KDE Plasma, and DAWs run without PipeWire-Pulse.</li>
            <li><b>3. Native Windows WASAPI Client API (<code>liblwasapi.so</code>):</b> Exposes <code>IAudioClient</code>, <code>IAudioRenderClient</code>, <code>IAudioCaptureClient</code>, and <code>IAudioClock</code> directly on Linux.</li>
        </ul>
        """,
    ),
    "mic_in": (
        "accent_mic",
        "🎙️ Chapter 3: Full-Duplex Mic Capture, Capture APO & HFP BTPROTO_SCO",
        "Hardware Mic Input -> Optional Capture APO (HPF/Gate/RNNoise/Comp/Sidetone) -> Apps",
        """
        <p>Salsa LWASAPI operates a true <b>full-duplex WaveRT engine</b> that simultaneously captures microphone audio (e.g., Focusrite Scarlett 2i2 USB or optional two-way Bluetooth HFP SCO) while rendering playback:</p>
        <ul>
            <li><b>Bypassable Real-Time Capture Mic APO:</b> Disabled by default so hardware mics like your Scarlett 2i2 pass completely raw. When toggled on, mic input passes through a Voice HPF, hysteresis Noise Gate, neural <code>librnnoise</code> noise suppression, Clarity boost, soft Voice Compressor, and optional low-latency headphone Sidetone (injected post-loopback so OBS never echoes).</li>
            <li><b>Desktop Mic Gain & Mute Synchronization:</b> Adjusting your microphone volume or mute state in KDE Plasma, <code>pactl</code>, or Salsa Studio synchronizes directly through Salsa's native PA v35 server and <code>lwasapi-ctl</code> with zero WirePlumber dependency.</li>
        </ul>
        """,
    ),
    "midi_in": (
        "accent_midi",
        "🎹 Chapter 4: Hardware MIDI Controller & Sample-Accurate DAW MIDI Hub",
        "Hardware RawMIDI + ALSA Sequencer + Lock-Free JACK MIDI Ring",
        """
        <p>Unlike standard audio servers that treat MIDI as an afterthought, Salsa LWASAPI integrates a dedicated Rust real-time MIDI thread (<code>midi_hub.rs</code>) directly alongside the audio engine:</p>
        <ul>
            <li><b>Auto-Discovery of Hardware Controllers:</b> The engine automatically scans <code>/dev/snd/midiC*D*</code> and ALSA Sequencer ports, immediately binding your hardware controller (such as <b>Novation FLkey 37</b>) and exposing the virtual <b><code>LWASAPI-MIDI-Hub</code></b> port.</li>
            <li><b>Sample-Accurate Frame Offsets:</b> Incoming MIDI Note-On, Note-Off, Pitch Bend, and CC messages are timestamped with <code>CLOCK_MONOTONIC_RAW</code> and stamped with the exact <code>frame_offset</code> inside the current audio quantum before being pushed into each DAW session's <code>midi_ring[512]</code> lock-free ring buffer.</li>
        </ul>
        """,
    ),
    "shm_ring": (
        "accent_secondary",
        "⚡ Chapter 5: Zero-Copy WaveRT Shared Memory Ring (memfd + eventfd)",
        "75-Nanosecond Buffer Commits — 14,048x Faster Than Socket IPC",
        """
        <p>The secret behind Salsa LWASAPI's speed is that <b>audio data never travels over Unix sockets or pipes</b>:</p>
        <ul>
            <li><b>Anonymous Shared Memory (<code>memfd_create</code>):</b> When an app opens a session, <code>lwasapi-audiodg</code> allocates a 64-byte-aligned <code>lwasapi_shm_ring_t</code> structure (32,768 stereo float32 frames for 192 kHz headroom) and passes the file descriptor to the client via <code>SCM_RIGHTS</code>.</li>
            <li><b>Zero-Copy Atomic Pointers:</b> The client writes PCM samples directly into L1/L2-cached shared RAM and updates <code>write_frame</code> with a single atomic release instruction (<code>0.075 µs</code> average commit latency).</li>
            <li><b>Kernel <code>eventfd</code> Wakeups:</b> At the end of each hardware period quantum, the engine triggers the client's kernel <code>eventfd</code>, waking event-driven DAWs and games with sub-millisecond precision.</li>
        </ul>
        """,
    ),
    "engine_core": (
        "accent_primary",
        "🚀 Chapter 6: MMCSS SCHED_FIFO Engine & Fixed Lossless Master Clock",
        "Deterministic Sample Rate (44.1–192 kHz) & Bit-Depth Precision (16 / 24 / 32-bit)",
        """
        <p>At the heart of the graph is the <code>lwasapi-audiodg</code> real-time thread, modeled after Windows' Multimedia Class Scheduler Service (MMCSS):</p>
        <ul>
            <li><b>Fixed Lossless Master Clock:</b> Lock your studio master clock to any supported sample rate (<code>44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz</code>) and PCM bit depth (<code>16 / 24 / 32-bit</code>). When a USB DAC hardware descriptor restricts available bit depths, the header reports the hardware-locked precision cleanly.</li>
            <li><b><code>SCHED_FIFO</code> Priority 65 + <code>mlockall</code>:</b> All audio ring buffers and filter states stay locked into physical RAM so page faults never cause dropouts.</li>
        </ul>
        """,
    ),
    "dyn_eq": (
        "accent_hot",
        "🎚️ Chapter 7: 10-Band Parametric Dynamic EQ, AutoEQ Importer & Crossfeed",
        "Per-Band Freq/Q/Shelf Shapes + Oratory1990 .txt Import + Bauer Crossfeed + Limiter",
        """
        <p>Every mixed audio period passes through the built-in <b>Salsa Audio Processing Object (APO)</b> inside the real-time thread before reaching your headphones or speakers:</p>
        <ul>
            <li><b>Full 10-Band Parametric Biquads:</b> Each band supports custom frequency (<code>15 Hz – 22 kHz</code>), adjustable <code>Q</code> (<code>0.20 – 10.00</code>), and selectable filter topology (<b>Peak</b>, <b>Low-Shelf</b>, or <b>High-Shelf</b>), plus instant one-click import of <b>AutoEQ / Oratory1990 <code>.txt</code></b> measurements.</li>
            <li><b>Real-Time Dynamic Processing & Bauer Crossfeed:</b> Dedicated bandpass envelope followers tame harsh peaks or boost transients, while optional Bauer stereo crossfeed reduces headphone in-head fatigue with instant Reset Flat / Bypass.</li>
            <li><b>Soft-Knee Mastering Limiter:</b> Prevents digital clipping above <code>0.95</code> (<code>-0.4 dBFS</code>) using smooth hyperbolic-tangent saturation.</li>
        </ul>
        """,
    ),
    "hw_out": (
        "accent_out",
        "🎧 Chapter 8: Master Output Sink, BlueZ Codec Switcher & Wireless ABR",
        "Active BlueZ Codec Diagnostics + LDAC ABR (990/660/330) + OBS Loopback",
        """
        <p>After Parametric Dynamic EQ, Crossfeed, and Limiter processing, the 32-bit float master bus delivers directly to your active endpoint:</p>
        <ul>
            <li><b>BlueZ Codec Switcher & Adaptive Bitrate (ABR):</b> Inspect remote headset SEPs, view exact negotiation diagnostics (<code>neg_reason</code>), switch codecs on demand (Auto, LDAC, aptX HD, aptX, AAC, SBC), and let Wireless ABR scale LDAC (<code>990 ↔ 660 ↔ 330 kbps</code>) or AAC (<code>256 ↔ 192 ↔ 128 kbps</code>) on link congestion.</li>
            <li><b>Low-Latency Loopback & Optional Sidetone:</b> Copies clean post-EQ desktop audio to OBS (<code>lwasapi_default_sink.monitor</code>) BEFORE injecting Capture Mic APO headphone sidetone so recordings stay echo-free.</li>
        </ul>
        """,
    ),
    "mix_bt": (
        "accent_warn",
        "🎛️ Chapter 12: Capture Mic APO, HW Analog Boost & BlueZ HFP SCO",
        "Neural RNNoise • Discrete Sound Card DAC/Amp • Two-Way Bluetooth HFP Headset Mic",
        """
        <p>Salsa v6.9-beta provides studio voice processing, hardware DAC/amp analog boosting, and wireless controls without PipeWire/WirePlumber dependencies:</p>
        <ul>
            <li><b>Hardware Analog Architecture:</b> Automatically utilizes ALSA DAC unity volume, discrete headphone amp gain tiers, and USB HID volume dials on supported sound cards like the ASUS Strix Raid DLX.</li>
            <li><b>Capture Mic APO & Sidetone:</b> Toggleable per-session strip featuring Voice HPF, Noise Gate, Neural <code>librnnoise</code> denoising, Clarity shelf, Soft Compressor, and zero-latency headphone Sidetone.</li>
            <li><b>Bluetooth HFP <code>BTPROTO_SCO</code> (mSBC/CVSD):</b> Optional toggle (OFF by default so Focusrite Scarlett stays primary) for two-way Bluetooth headset microphone calls.</li>
        </ul>
        """,
    ),
    "autosync": (
        "accent_secondary",
        "🔄 Chapter 9: Automatic Audio Module Update Watcher & ABI Self-Healing",
        "Real-Time Package & Library Watcher (<code>lwasapi-module-watcher.path</code> + <code>lwasapi-autosync.sh</code>)",
        """
        <p>When your Linux distribution updates any system audio or Bluetooth package (<code>alsa-lib</code>, <code>libpulse</code>, <code>libfdk-aac</code>, <code>bluez</code>, <code>sbc</code>, <code>openal</code>, <code>sdl2</code>/<code>sdl3</code>, or compatibility layers), Salsa LWASAPI automatically adapts without manual maintenance:</p>
        <ul>
            <li><b>Systemd Path Watcher (<code>lwasapi-module-watcher.path</code>):</b> Monitors your distro's package database (<code>/var/lib/pacman/local</code>, <code>/var/lib/dpkg/status</code>, or <code>/var/lib/rpm</code>) and shared libraries (<code>libasound.so.2</code>, <code>libpulse.so.0</code>, <code>libjack.so.0</code>). The instant any module upgrades, <code>lwasapi-autosync.sh</code> triggers automatically.</li>
            <li><b>Automatic ELF Symbol-Version Extraction (<code>readelf -V</code>):</b> Dynamically scans the host PC's <code>libasound.so.2</code> and <code>libpulse.so.0</code> to extract all <code>ALSA_*</code> and <code>PULSE_*</code> symbol version nodes, updating <code>src/alsa.version</code> and <code>src/pulse.version</code> and recompiling all shims (<code>make -B</code>) so updated apps never experience symbol version mismatches.</li>
            <li><b>Self-Healing Ownership:</b> Re-applies <code>lwasapi-ctl enable-auto</code> (keeping WirePlumber locked to <code>video-only</code> for Wayland screen sharing and keeping <code>pipewire-pulse</code> masked) and hot-restarts <code>lwasapi-audiodg.service</code>, automatically rebinding BlueZ A2DP / kernel ALSA hardware and restoring saved Dynamic EQ settings.</li>
        </ul>
        """,
    ),
    "crosspc": (
        "accent_out",
        "🌍 Chapter 10: Universal Cross-PC Hardware & Distro Adaptation",
        "Zero Hardcoded Paths • Multi-Distro ABI Discovery • PipeWire/WirePlumber-Free Audio Ownership",
        f"""
        <p>Salsa — Linux WASAPI is engineered to adapt automatically to <b>any Linux PC, laptop, workstation, or handheld</b> out of the box, completely replacing <code>pipewire-pulse</code> and WirePlumber audio routing:</p>
        <ul>
            <li><b>Dynamic Executable & User Path Resolution:</b> Every C binary resolves its location via <code>readlink("/proc/self/exe")</code> and <code>$HOME</code>, Python GUIs resolve via <code>os.path.realpath(__file__)</code>, and shell launchers use <code>readlink -f</code>. Moving the folder to any user account or directory on another PC works immediately.</li>
            <li><b>Direct Kernel & BlueZ Hardware Scoring:</b> At startup and hotplug, <code>lwasapi-audiodg</code> communicates directly with <code>org.bluez</code> D-Bus A2DP/AVRCP (<code>AAC</code> via <code>libfdk-aac</code> + <code>SBC</code>), kernel ALSA PCM nodes (<code>/dev/snd/pcmC*D*</code> with automatic USB DAC / Analog / HDMI priority scoring), and <code>/dev/snd/midiC*D*</code>—with zero <code>wpctl</code> or PipeWire involvement.</li>
            <li><b>Multi-Distro Library & Reversible Isolation:</b> Automatically locates <code>libasound.so.2</code> and <code>libpulse.so.0</code> across Arch/CachyOS, Fedora/RHEL/openSUSE, and Debian/Ubuntu via <code>ldconfig -p</code>, while backups in <code>~/.config/lwasapi/pw_backup/</code> allow instant restoration via <code>lwasapi-ctl disable-auto</code> or <code>./install-salsa.sh --uninstall</code>.</li>
        </ul>
        <p style="margin-top:8px;"><b>Live Detected Host PC Profile:</b>
        <code>OS: {HOST_PROFILE['os']}</code> • <code>CPU: {HOST_PROFILE['cpu']}</code> • <code>Pkg Watcher: {HOST_PROFILE['pkg_mgr']}</code> • <code>Soundcards: {len(HOST_PROFILE['cards'])} detected</code></p>
        """,
    ),
    "vid_src": (
        "accent_secondary",
        "🖥️ Chapter 11A: Wayland Video Producer & Portal Handoff",
        "kwin_wayland ScreenCast Stream Producer + xdg-desktop-portal OpenPipeWireRemote",
        """
        <p>Wayland compositors such as <b>KDE KWin (<code>kwin_wayland</code>)</b> export desktop, monitor, and window captures natively into Salsa's built-in Video PipeWire socket:</p>
        <ul>
            <li><b>Zero-Copy GPU DmaBuf & MemFd Producers:</b> Supports hardware <code>DmaBuf</code> modifiers (RDNA4 / Mesa <code>radv</code> / Vulkan) as well as CPU <code>MemFd</code> fallback with full <code>SPA_PARAM_EnumFormat</code> negotiation.</li>
            <li><b><code>OpenPipeWireRemote</code> Portal Handoff:</b> When <code>xdg-desktop-portal</code> opens a ScreenCast session and passes the detached file descriptor via <code>pw_core_steal_fd()</code> into Discord/Chromium/OBS, Salsa cleanly resets stale portal proxy slots on repeated <code>PW_CORE_M_HELLO</code> so consumer streams bind on the first frame.</li>
        </ul>
        """,
    ),
    "vid_broker": (
        "accent_secondary",
        "🔌 Chapter 11B: Zero-PipeWire Native Rust Video Broker",
        "$XDG_RUNTIME_DIR/pipewire-0 • Zero libpipewire / Zero WirePlumber / Audio Guard Active",
        """
        <p><code>lwasapi-audiodg</code> embeds a standalone <b>Video-Only PipeWire Protocol Broker</b> (<code>pw_broker.rs</code>) listening directly on <code>/run/user/$UID/pipewire-0</code> without linking <code>libpipewire</code> or <code>libspa</code>:</p>
        <ul>
            <li><b>Strict Video-Only Admission:</b> Only <code>Video/Source</code>, <code>Stream/Output/Video</code>, and <code>Stream/Input/Video</code> nodes are routed. Any audio node attempts are actively rejected (counted in <code>audio_rejects</code>) so Salsa keeps 100% exclusive audio ownership via Native PA v35 and WaveRT SHM.</li>
            <li><b>Clean KWin Stability & Format Compatibility:</b> Enforces safe fractional <code>maxFramerate</code> bounds to prevent <code>VsyncSource</code> floating-point faults and handles both <code>DmaBuf</code> and <code>MemFd</code> SPA pods for KDE Spectacle, Plasma previews, browsers, OBS, and Discord.</li>
        </ul>
        """,
    ),
    "vid_relay": (
        "accent_secondary",
        "🎞️ Chapter 11C: Zero-Copy Frame Relay, Live Resolution/FPS Cap & Preview HUD",
        "SCM_RIGHTS FD Relay + Dynamic PW_CAST_LIMITS + Zero-Overhead Live Feed Mirror",
        """
        <p>Stage 3 of the video lane bridges producer buffer pools directly to subscribed consumer nodes:</p>
        <ul>
            <li><b>Direct <code>SCM_RIGHTS</code> Buffer Relay:</b> DmaBuf and MemFd buffer file descriptors are mapped and duplicated directly between producer and consumer with lock-free cycling and <code>eventfd</code> triggers.</li>
            <li><b>Live Resolution & FPS Profile Control (<code>PW_CAST_LIMITS</code>):</b> Inside the Debugger's <b>ScreenCast Broker & Live Feed</b> tab, you can apply live resolution/FPS profiles (<code>480p@15</code>, <code>720p@30</code>, <code>1080p@60</code>, <code>1440p@60</code>, or <code>Source</code>) to cap outgoing ScreenCast delivery cadence and match Discord presets on the fly.</li>
            <li><b>Zero-Overhead Preview HUD (<code>PW_PREVIEW</code>):</b> Frame copying into the debugger SHM preview ring stays dormant (<code>0%</code> CPU/compositor cost) until Live Feed is explicitly toggled on.</li>
        </ul>
        """,
    ),
    "vid_cons": (
        "accent_warn",
        "📡 Chapter 11D: ScreenCast Consumers & Synchronized Audio Capture",
        "Discord (AV1 VAAPI) / OBS Studio / Browsers / KDE Portal + Parallel Salsa Audio Routing",
        """
        <p>Applications on Stage 4 automatically receive Wayland ScreenCast video frames in parallel with ultra-low-latency Salsa audio:</p>
        <ul>
            <li><b>Video Consumers:</b> Discord (<code>capture_linux</code> with hardware AMD VAAPI AV1/H.264 encode), OBS Studio PipeWire Capture, Chromium/Firefox WebRTC share, Spectacle, and KDE taskbar window thumbnails connect to <code>pipewire-0</code> seamlessly.</li>
            <li><b>Parallel Native Audio Route:</b> Voice chat, game capture, and application audio capture travel concurrently through Salsa's <code>0.33 ms</code> WaveRT Capture & Loopback ring (<code>PA v35</code> / <code>IAudioCaptureClient</code>)—keeping audio and video perfectly independent and zero-PipeWire.</li>
        </ul>
        """,
    ),
}


def render_manual_chapter(key: str, theme: dict | None = None) -> str:
    t = theme or THEMES[DEFAULT_THEME]
    accent_key, heading, subtitle, body = OWNERS_MANUAL_CHAPTER_DEFS.get(
        key, OWNERS_MANUAL_CHAPTER_DEFS["overview"]
    )
    heading_color = t.get(accent_key, t["accent_primary"])
    return (
        f'<h2 style="color:{heading_color}; margin-bottom:4px;">{heading}</h2>'
        f'<p style="color:{t["text_muted"]}; font-size:12px; margin-top:0;"><b>{subtitle}</b></p>'
        f"{body}"
    )


OWNERS_MANUAL_CHAPTERS = {
    k: render_manual_chapter(k, THEMES[DEFAULT_THEME]) for k in OWNERS_MANUAL_CHAPTER_DEFS
}


def create_salsa_icon(theme: dict | None = None) -> QIcon:
    for candidate in (
        ICON_GLOW_PATH,
        os.path.expanduser("~/.local/share/icons/hicolor/512x512/apps/salsa-lwasapi.png"),
        ICON_PATH,
    ):
        if os.path.exists(candidate):
            icon = QIcon(candidate)
            if not icon.isNull():
                return icon

    t = theme or THEMES[DEFAULT_THEME]
    pix = QPixmap(64, 64)
    pix.fill(Qt.GlobalColor.transparent)
    p = QPainter(pix)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    grad = QLinearGradient(0, 0, 64, 64)
    grad.setColorAt(0.0, QColor(t["accent_hot"]))
    grad.setColorAt(0.5, QColor(t["accent_mic"]))
    grad.setColorAt(1.0, QColor(t["accent_primary"]))
    p.setBrush(grad)
    p.setPen(QPen(QColor(t["handle_border"]), 2))
    p.drawRoundedRect(4, 4, 56, 56, 14, 14)

    p.setPen(QColor("#ffffff" if t.get("is_light") else t["bg_deep"]))
    f = QFont("Sans Serif", 22, QFont.Weight.Black)
    p.setFont(f)
    p.drawText(pix.rect(), Qt.AlignmentFlag.AlignCenter, "S")
    p.end()
    return QIcon(pix)


class SalsaAutoGraphCanvas(QWidget):
    """
    qpwgraph-Equivalent Live Automatic Audio, MIDI & Video Signal Topology Canvas
    (Read-Only, Self-Wiring, Click-to-Inspect)
    """

    def __init__(self, on_node_selected):
        super().__init__()
        self.setMinimumHeight(440)
        self.setMouseTracking(True)
        self.on_node_selected = on_node_selected
        self.theme = THEMES[DEFAULT_THEME]

        self.selected_key = "overview"
        self.hover_key = None
        self.anim_phase = 0.0
        self.node_rects = []
        self._anim_timer = QTimer(self)
        self._anim_timer.setInterval(33)
        self._anim_timer.timeout.connect(self._tick_animation)

        # Live audio & MIDI telemetry snapshot
        self.hw_sink = "Auto-Detecting Hardware Sink..."
        self.hw_mic = "Auto-Detecting Microphone..."
        self.hw_midi = "LWASAPI-MIDI-Hub"
        self.sample_rate = 192000
        self.cap_sample_rate = 48000
        self.period_frames = 512
        self.format_bits = 32
        self.eq_enabled = True
        self.eq_preset = "Flat / Studio Reference"
        self.eq_mode_str = "Dynamic Compress"
        self.spk_vol = 80
        self.mic_vol = 100
        self.midi_events = 0
        self.bt_active = False
        self.bt_codec = "NONE"
        self.bt_delay_ms = 0
        self.pa_running = False
        self.pa_clients = 0
        self.crossfeed_enabled = False
        self.mic_apo_enabled = False
        self.mic_rnnoise_active = False
        self.mic_sidetone_enabled = False
        self.mic_sidetone_pct = 0
        self.bt_kbps = 0
        self.bt_abr = True
        self.bt_hfp_sco = False
        self.hw_arch = "Sound Card"
        self.hw_dac_unity = False
        self.hw_amp_tier = 0
        self.hw_analog_boost_db = 0.0
        self.hw_mic_boost_db = 0.0
        self.hw_hid_box = False
        self.play_apps = ["Auto-Standby (All ALSA/Pulse/JACK Apps)"]
        self.capt_apps = ["Loopback / Capture Ready (OBS / Discord)"]

        # Live PipeWire video-only ScreenCast telemetry snapshot
        self.pw_broker_running = False
        self.pw_zero_mode = True
        self.pw_active_casting = False
        self.pw_preview_req = False
        self.pw_socket_path = f"/run/user/{os.getuid()}/pipewire-0"
        self.pw_num_clients = 0
        self.pw_audio_rejects = 0
        self.pw_dmabuf_streams = 0
        self.pw_memfd_streams = 0
        self.pw_relayed_total = 0
        self.pw_dropped_total = 0
        self.pw_actual_fps = 0.0
        self.pw_cast_w = 0
        self.pw_cast_h = 0
        self.pw_cast_fps = 0
        self.pw_prev_w = 0
        self.pw_prev_h = 0
        self.pw_prev_fps = 0
        self.pw_clients = []
        self.pw_nodes = []
        self.pw_links = []

    def set_animation_active(self, active: bool):
        if active:
            if not self._anim_timer.isActive():
                self._anim_timer.start()
        elif self._anim_timer.isActive():
            self._anim_timer.stop()

    def _tick_animation(self):
        self.anim_phase = (self.anim_phase + 0.032) % 1.0
        self.update()

    def set_theme(self, theme: dict):
        self.theme = theme
        self.update()

    @staticmethod
    def _fmt_fps(fps_num: int, fps_denom: int) -> str:
        num = int(fps_num or 0)
        den = max(1, int(fps_denom or 1))
        if num <= 0:
            return "--"
        val = float(num) / float(den)
        if abs(val - round(val)) < 0.08:
            return f"{round(val):.0f}"
        return f"{val:.1f}"

    def _apply_pw_snapshot(self, pw: dict):
        self.pw_broker_running = bool(pw.get("broker_running"))
        self.pw_zero_mode = bool(pw.get("zero_pw_mode", 1))
        self.pw_active_casting = bool(pw.get("active_casting"))
        self.pw_preview_req = bool(pw.get("preview_requested"))
        self.pw_socket_path = pw.get("socket_path") or f"/run/user/{os.getuid()}/pipewire-0"
        self.pw_clients = list(pw.get("clients") or [])
        self.pw_nodes = list(pw.get("nodes") or [])
        self.pw_links = list(pw.get("links") or [])
        self.pw_num_clients = int(pw.get("num_clients", len(self.pw_clients)) or 0)
        self.pw_audio_rejects = int(pw.get("audio_rejects", 0) or 0)
        self.pw_dmabuf_streams = int(pw.get("dmabuf_streams", 0) or 0)
        self.pw_memfd_streams = int(pw.get("memfd_streams", 0) or 0)
        self.pw_relayed_total = int(pw.get("frames_relayed_total", 0) or 0)
        self.pw_dropped_total = int(pw.get("frames_dropped", 0) or 0)
        self.pw_actual_fps = float(pw.get("actual_fps_x100", 0) or 0) / 100.0
        self.pw_cast_w = int(pw.get("cast_limit_w", 0) or 0)
        self.pw_cast_h = int(pw.get("cast_limit_h", 0) or 0)
        self.pw_cast_fps = int(pw.get("cast_limit_fps", 0) or 0)
        self.pw_prev_w = int(pw.get("preview_target_w", 0) or 0)
        self.pw_prev_h = int(pw.get("preview_target_h", 0) or 0)
        self.pw_prev_fps = int(pw.get("preview_target_fps", 0) or 0)

    def update_graph_state(self, st: LwasapiEngineStatus | None, pw: dict | None = None):
        if st is not None:
            self.hw_sink = st.hw_device.decode("utf-8", "ignore") or "Hardware Output Sink"
            self.hw_mic = st.hw_capture_device.decode("utf-8", "ignore") or "Default Microphone"
            self.hw_midi = st.midi_device.decode("utf-8", "ignore") or "LWASAPI-MIDI-Hub"
            self.sample_rate = st.hw_sample_rate if st.hw_sample_rate > 0 else 48000
            cap_r = int(st.hw_analog_info.reserved)
            self.cap_sample_rate = cap_r if 8000 <= cap_r <= 384000 else self.sample_rate
            self.period_frames = st.hw_period_frames if st.hw_period_frames > 0 else 256
            self.format_bits = st.hw_format_bits if st.hw_format_bits > 0 else 32
            self.eq_enabled = bool(st.eq_cfg.enabled)
            self.eq_preset = st.eq_cfg.preset_name.decode("utf-8", "ignore") or "Custom"
            modes = ["Static Parametric", "Dynamic Compress", "Dynamic Expand"]
            self.eq_mode_str = modes[st.eq_cfg.dynamic_mode] if st.eq_cfg.dynamic_mode <= 2 else "Dynamic EQ"
            self.spk_vol = st.desktop_volume_pct
            self.mic_vol = st.desktop_mic_volume_pct
            self.midi_events = st.midi_events_routed
            self.bt_active = bool(st.bt_active)
            self.bt_codec = st.bt_codec.decode("utf-8", "ignore") or ("SBC" if self.bt_active else "PCM")
            self.bt_delay_ms = st.bt_delay_ms
            self.pa_running = bool(st.pa_server_running)
            self.pa_clients = st.pa_clients
            self.crossfeed_enabled = bool(st.param_eq.crossfeed_enabled)
            self.mic_apo_enabled = bool(st.mic_apo.enabled)
            self.mic_rnnoise_active = bool(st.mic_rnnoise_active)
            self.mic_sidetone_enabled = bool(st.mic_apo.sidetone_enabled)
            self.mic_sidetone_pct = int(st.mic_apo.sidetone_pct)
            self.bt_kbps = int(st.bt_codec_cfg.effective_kbps)
            self.bt_abr = bool(st.bt_codec_cfg.wireless_abr_enabled)
            self.bt_hfp_sco = bool(st.bt_codec_cfg.hfp_sco_active or st.bt_codec_cfg.hfp_sco_enabled)
            self.hw_arch = st.hw_analog_info.card_arch_name.decode("utf-8", "ignore").strip("\x00 ") or "Sound Card"
            self.hw_dac_unity = bool(st.hw_analog_info.dac_unity_active)
            self.hw_amp_tier = int(st.hw_analog_cfg.hp_amp_tier)
            self.hw_analog_boost_db = float(st.hw_analog_cfg.analog_boost_db)
            self.hw_mic_boost_db = float(st.hw_analog_cfg.mic_hw_boost_db)
            self.hw_hid_box = bool(st.hw_analog_info.hid_box_connected)

            play_list = []
            capt_list = []
            n_sess = min(st.num_sessions, LWASAPI_MAX_SESSIONS)
            for i in range(n_sess):
                s = st.sessions[i]
                cname = s.client_name.decode("utf-8", "ignore")
                if s.is_capture:
                    capt_list.append(f"{cname} ({s.sample_rate // 1000}kHz)")
                else:
                    play_list.append(f"{cname} ({s.sample_rate // 1000}kHz)")
            self.play_apps = play_list if play_list else ["Auto-Standby (All ALSA/Pulse/JACK Apps)"]
            self.capt_apps = capt_list if capt_list else ["Loopback / Capture Ready (OBS / Discord)"]

        if pw is not None:
            self._apply_pw_snapshot(pw)

        if not self._anim_timer.isActive():
            self.update()

    def mousePressEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            pos = event.position()
            for key, rect in self.node_rects:
                if rect.contains(pos):
                    self.selected_key = key
                    self.on_node_selected(key)
                    self.update()
                    return
            self.selected_key = "overview"
            self.on_node_selected("overview")
            self.update()

    def mouseMoveEvent(self, event):
        pos = event.position()
        hov = None
        for key, rect in self.node_rects:
            if rect.contains(pos):
                hov = key
                break
        if hov != self.hover_key:
            self.hover_key = hov
            self.setCursor(Qt.CursorShape.PointingHandCursor if hov else Qt.CursorShape.ArrowCursor)
            self.update()

    def _draw_wire(self, p: QPainter, start: QPointF, end: QPointF, color: QColor, dashed=False, pulse=True):
        path = QPainterPath()
        path.moveTo(start)
        dx = abs(end.x() - start.x()) * 0.5
        c1 = QPointF(start.x() + dx, start.y())
        c2 = QPointF(end.x() - dx, end.y())
        path.cubicTo(c1, c2, end)

        glow = QColor(color)
        glow.setAlpha(45 if pulse else 24)
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.setPen(QPen(glow, 6.0 if pulse else 4.0))
        p.drawPath(path)

        pen = QPen(color, 2.2 if pulse else 1.7, Qt.PenStyle.DashLine if dashed else Qt.PenStyle.SolidLine)
        p.setPen(pen)
        p.drawPath(path)

        if pulse:
            pulse_col = QColor(self.theme["wire_pulse"])
            for offset in (0.0, 0.5):
                t = (self.anim_phase + offset) % 1.0
                pt = path.pointAtPercent(t)
                p.setBrush(pulse_col)
                p.setPen(QPen(color, 1.5))
                p.drawEllipse(pt, 3.8, 3.8)

    def _draw_node(
        self,
        p: QPainter,
        key: str,
        rect: QRectF,
        title: str,
        badge: str,
        lines: list[str],
        accent: QColor,
        has_in: bool,
        has_out: bool,
    ):
        self.node_rects.append((key, rect))
        is_sel = self.selected_key == key
        is_hov = self.hover_key == key
        t = self.theme

        bg = QLinearGradient(rect.topLeft(), rect.bottomLeft())
        bg.setColorAt(0.0, QColor(t["node_bg_hover"]) if (is_sel or is_hov) else QColor(t["node_bg_top"]))
        bg.setColorAt(1.0, QColor(t["node_bg_bot"]))
        p.setBrush(bg)
        border_w = 2.6 if is_sel else (2.0 if is_hov else 1.3)
        border_col = accent if (is_sel or is_hov) else QColor(t["node_border"])
        p.setPen(QPen(border_col, border_w))
        p.drawRoundedRect(rect, 9.0, 9.0)

        hdr_h = min(23.0, max(19.0, rect.height() * 0.25))
        hdr_rect = QRectF(rect.x() + 1.0, rect.y() + 1.0, rect.width() - 2.0, hdr_h)
        hdr_col = QColor(accent)
        hdr_col.setAlpha(45 if not is_sel else 85)
        p.setBrush(hdr_col)
        p.setPen(Qt.PenStyle.NoPen)
        p.drawRoundedRect(hdr_rect, 8.0, 8.0)

        p.setFont(QFont("Sans Serif", 9, QFont.Weight.Bold))
        p.setPen(QColor(t["text_bright"]))
        p.drawText(
            QRectF(rect.x() + 10.0, rect.y() + 2.0, rect.width() - 75.0, hdr_h),
            Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter,
            title,
        )

        p.setFont(QFont("Monospace", 7, QFont.Weight.Bold))
        p.setPen(accent)
        p.drawText(
            QRectF(rect.right() - 72.0, rect.y() + 2.0, 64.0, hdr_h),
            Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter,
            badge,
        )

        p.setFont(QFont("Monospace", 8))
        p.setPen(QColor(t["text_secondary"]))
        body_top = rect.y() + hdr_h + 4.0
        body_h = max(14.0, rect.bottom() - body_top - 4.0)
        max_lines = min(4, len(lines), max(1, int(body_h // 14.0)))
        step_y = min(15.5, body_h / max(1, max_lines))
        y_cursor = body_top
        for ln in lines[:max_lines]:
            p.drawText(
                QRectF(rect.x() + 10.0, y_cursor, rect.width() - 20.0, 15.0),
                Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter,
                ln[:38],
            )
            y_cursor += step_y

        cy = rect.center().y()
        port_pen = QPen(QColor(t["handle_border"]), 1.3)
        if has_in:
            p.setBrush(accent)
            p.setPen(port_pen)
            p.drawEllipse(QPointF(rect.left(), cy), 5.0, 5.0)
        if has_out:
            p.setBrush(accent)
            p.setPen(port_pen)
            p.drawEllipse(QPointF(rect.right(), cy), 5.0, 5.0)

    def _build_video_node_contents(self):
        sock_base = os.path.basename(self.pw_socket_path) or "pipewire-0"
        prods = sorted(
            [n for n in self.pw_nodes if int(n.get("direction", 0)) == 1],
            key=lambda n: (1 if int(n.get("state", 0)) == 4 else 0, int(n.get("id", 0))),
            reverse=True,
        )
        cons = sorted(
            [n for n in self.pw_nodes if int(n.get("direction", 0)) == 0],
            key=lambda n: (
                1 if int(n.get("state", 0)) == 4 else 0,
                1 if int(n.get("frames", 0)) > 0 else 0,
                int(n.get("id", 0)),
            ),
            reverse=True,
        )
        active_links = [lk for lk in self.pw_links if int(lk.get("state", 0)) == 4]
        all_links = active_links if active_links else list(self.pw_links)
        vid_live = bool(self.pw_active_casting or active_links)
        has_portal = any(bool(cl.get("is_portal")) for cl in self.pw_clients)

        # Col 1: Compositor / Wayland Video Source
        if prods:
            p0 = prods[0]
            pmem = p0.get("mem_type") or "idle"
            if "DmaBuf" in pmem:
                src_badge = "DMABUF"
            elif "MemFd" in pmem:
                src_badge = "MEMFD"
            else:
                src_badge = f"{len(prods)} PROD"
            pw_w, pw_h = int(p0.get("width", 0) or 0), int(p0.get("height", 0) or 0)
            p_fps = self._fmt_fps(p0.get("fps_num", 0), p0.get("fps_denom", 1))
            extra_p = f" (+{len(prods) - 1})" if len(prods) > 1 else ""
            p_res = f"Res: {pw_w}x{pw_h} @ {p_fps} Hz" if pw_w > 0 else "Res: Negotiating SPA EnumFormat"
            p_fmt = f"Fmt: {p0.get('format_name') or '--'} | {pmem} x{p0.get('n_buffers', 0)}"
            src_lines = [
                f"Src: {(p0.get('name') or 'compositor')[:20]} (#{p0.get('id', 0)}){extra_p}",
                p_res,
                p_fmt,
                f"Portal: {'xdg-desktop-portal' if has_portal else 'Wayland ScreenCast'}",
            ]
        else:
            src_badge = "STANDBY" if self.pw_broker_running else "OFF"
            src_lines = [
                "Src: kwin_wayland (Wayland Cast)",
                "Portal: xdg-desktop-portal Ready",
                "Buffers: DmaBuf (GPU) / MemFd",
                "State: Standby (Awaiting Share)",
            ]

        # Col 2: Built-in Video-Only PipeWire Broker
        brk_badge = "ONLINE" if self.pw_broker_running else "OFFLINE"
        mode_lbl = "Zero-PW Video-Only" if self.pw_zero_mode else "Legacy PW/WP Mode"
        brk_lines = [
            f"Sock: .../{sock_base} (No libpw)",
            f"Mode: {mode_lbl}",
            f"Clients: {self.pw_num_clients} | Audio Drop: {self.pw_audio_rejects}",
            f"Pool: {self.pw_dmabuf_streams} DmaBuf / {self.pw_memfd_streams} MemFd",
        ]

        # Col 3: Relay & Live Cap
        if self.pw_cast_w > 0 and self.pw_cast_h > 0:
            cap_res = f"{self.pw_cast_h}p"
        else:
            cap_res = "Source"
        cap_fps = f"{self.pw_cast_fps}fps" if self.pw_cast_fps > 0 else "SrcHz"
        hud_str = (
            f"ON ({self.pw_prev_h or 'Src'}p@{self.pw_prev_fps or 60})"
            if self.pw_preview_req
            else "Standby (0 CPU)"
        )
        relay_badge = "CASTING" if vid_live else ("PREVIEW" if self.pw_preview_req else "IDLE")
        if all_links:
            lk0 = all_links[0]
            mem_tag = "DMA" if lk0.get("is_dmabuf") else "MEM"
            l_str = f"Link #{lk0.get('id', 0)}: #{lk0.get('out_node_id', 0)}->#{lk0.get('in_node_id', 0)} ({mem_tag})"
        else:
            l_str = "Relay: Zero-Copy SCM_RIGHTS FD"
        drop_tag = f" drop:{self.pw_dropped_total}" if self.pw_dropped_total > 0 else ""
        relay_lines = [
            l_str,
            f"Live Cap: {cap_res} @ {cap_fps}",
            f"HUD Mirror: {hud_str}",
            f"Relayed: {self.pw_relayed_total:,}f ({self.pw_actual_fps:.1f} FPS){drop_tag}",
        ]

        # Col 4: Video Consumers
        st_map = {0: "err", 1: "new", 2: "susp", 3: "idle", 4: "live"}
        if cons:
            c_rows = []
            for nd in cons[:2]:
                nm = nd.get("name") or f"node-{nd.get('id', 0)}"
                cw, ch = int(nd.get("width", 0) or 0), int(nd.get("height", 0) or 0)
                if cw > 0:
                    c_fps = self._fmt_fps(nd.get("fps_num", 0), nd.get("fps_denom", 1))
                    tail = f"{cw}x{ch}@{c_fps}"
                else:
                    tail = st_map.get(int(nd.get("state", 0)), "wait")
                c_rows.append(f"• {nm[:16]} (#{nd.get('id', 0)}) {tail}")
            if len(c_rows) == 1:
                nd0 = cons[0]
                cmem = nd0.get("mem_type") or "Stream"
                nb = int(nd0.get("n_buffers", 0) or 0)
                if nb > 0:
                    pool_tag = f"{cmem} x{nb}"
                else:
                    cfmt = nd0.get("format_name") or "Video"
                    pool_tag = f"{cmem} {cfmt}"
                c_rows.append(
                    f"• {pool_tag} | {int(nd0.get('frames', 0) or 0):,}f rx"
                )
            c_rows.append("Handoff: OpenPipeWireRemote OK")
            c_rows.append("Audio Route: Native Salsa PA/WASAPI")
            cons_badge = f"{len(cons)} LIVE"
            cons_lines = c_rows
        else:
            cons_badge = "STANDBY"
            cons_lines = [
                "• Ready: Discord / OBS / Browsers",
                "• KDE Portal: Window Thumbnails",
                "Handoff: OpenPipeWireRemote OK",
                "Audio Route: Native Salsa PA/WASAPI",
            ]

        return (
            vid_live,
            (src_badge, src_lines),
            (brk_badge, brk_lines),
            (relay_badge, relay_lines),
            (cons_badge, cons_lines),
        )

    def paintEvent(self, event):
        self.node_rects.clear()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = float(self.width()), float(self.height())
        t = self.theme

        c_sec = QColor(t["accent_secondary"])
        c_mic = QColor(t["accent_mic"])
        c_pri = QColor(t["accent_primary"])
        c_hot = QColor(t["accent_hot"])
        c_warn = QColor(t["accent_warn"])
        c_midi = QColor(t["accent_midi"])
        c_out = QColor(t["accent_out"])
        c_vid = QColor(t.get("accent_video", "#0891b2" if t.get("is_light") else "#22d3ee"))
        c_vid_dim = QColor(c_vid)
        c_vid_dim.setAlpha(145)

        p.setBrush(QColor(t["bg_canvas"]))
        p.setPen(QPen(QColor(t["border_card"]), 1.5))
        p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 10.0, 10.0)

        p.setPen(QPen(QColor(t["canvas_grid"]), 1.0))
        step = 28
        for gx in range(step, int(w), step):
            p.drawLine(gx, 2, gx, int(h) - 2)
        for gy in range(step, int(h), step):
            p.drawLine(2, gy, int(w) - 2, gy)

        col_w = min(252.0, (w - 80.0) / 4.2)
        gap = (w - 32.0 - col_w * 4.0) / 3.0
        x1 = 16.0
        x2 = x1 + col_w + gap
        x3 = x2 + col_w + gap
        x4 = x3 + col_w + gap

        top_y = 14.0
        bot_y = h - 10.0
        div_gap = 24.0
        usable_h = max(390.0, bot_y - top_y)
        audio_h = max(274.0, (usable_h - div_gap) * 0.72)
        vid_top = top_y + audio_h + div_gap
        vid_h = max(92.0, bot_y - vid_top)

        # Column 1 (Audio + MIDI Sources)
        vg = 8.0
        src_avail = max(240.0, audio_h - 2.0 * vg)
        h_apps = src_avail * (96.0 / 264.0)
        h_mic = src_avail * (84.0 / 264.0)
        h_midi = src_avail * (84.0 / 264.0)
        r_apps_in = QRectF(x1, top_y, col_w, h_apps)
        r_mic_in = QRectF(x1, top_y + h_apps + vg, col_w, h_mic)
        r_midi_in = QRectF(x1, top_y + h_apps + h_mic + 2.0 * vg, col_w, h_midi)

        # Column 2 (WaveRT SHM & Audio Engine)
        r_shm = QRectF(x2, top_y + audio_h * 0.04, col_w, audio_h * 0.42)
        r_engine = QRectF(x2, top_y + audio_h * 0.52, col_w, audio_h * 0.44)

        # Column 3 (Dynamic EQ & MIDI Hub)
        r_dyneq = QRectF(x3, top_y + audio_h * 0.06, col_w, audio_h * 0.46)
        r_midihub = QRectF(x3, top_y + audio_h * 0.58, col_w, audio_h * 0.36)

        # Column 4 (Hardware Sink & Capture/Loopback)
        r_hw_out = QRectF(x4, top_y + audio_h * 0.04, col_w, audio_h * 0.43)
        r_capt_out = QRectF(x4, top_y + audio_h * 0.53, col_w, audio_h * 0.42)

        # Video ScreenCast Lane (Bottom row across Columns 1..4)
        r_vid_src = QRectF(x1, vid_top, col_w, vid_h)
        r_vid_broker = QRectF(x2, vid_top, col_w, vid_h)
        r_vid_relay = QRectF(x3, vid_top, col_w, vid_h)
        r_vid_cons = QRectF(x4, vid_top, col_w, vid_h)

        # Draw Audio & MIDI Wires
        self._draw_wire(
            p,
            QPointF(r_apps_in.right(), r_apps_in.center().y()),
            QPointF(r_shm.left(), r_shm.center().y() - 12.0),
            c_sec,
        )
        self._draw_wire(
            p,
            QPointF(r_mic_in.right(), r_mic_in.center().y()),
            QPointF(r_shm.left(), r_shm.center().y() + 14.0),
            c_mic,
        )
        self._draw_wire(
            p,
            QPointF(r_shm.right(), r_shm.center().y()),
            QPointF(r_engine.left(), r_engine.center().y()),
            c_pri,
        )
        self._draw_wire(
            p,
            QPointF(r_engine.right(), r_engine.center().y() - 10.0),
            QPointF(r_dyneq.left(), r_dyneq.center().y()),
            c_pri,
        )
        self._draw_wire(
            p,
            QPointF(r_dyneq.right(), r_dyneq.center().y()),
            QPointF(r_hw_out.left(), r_hw_out.center().y()),
            c_hot,
        )
        if self.mic_apo_enabled and self.mic_sidetone_enabled:
            self._draw_wire(
                p,
                QPointF(r_engine.right(), r_engine.center().y() + 8.0),
                QPointF(r_hw_out.left(), r_hw_out.center().y() + 18.0),
                c_mic,
                dashed=True,
                pulse=True,
            )
        self._draw_wire(
            p,
            QPointF(r_dyneq.right(), r_dyneq.center().y() + 18.0),
            QPointF(r_capt_out.left(), r_capt_out.center().y() - 12.0),
            c_warn,
            dashed=True,
        )
        self._draw_wire(
            p,
            QPointF(r_engine.right(), r_engine.center().y() + 16.0),
            QPointF(r_capt_out.left(), r_capt_out.center().y() + 12.0),
            c_mic,
        )
        self._draw_wire(
            p,
            QPointF(r_midi_in.right(), r_midi_in.center().y()),
            QPointF(r_midihub.left(), r_midihub.center().y()),
            c_midi,
        )
        self._draw_wire(
            p,
            QPointF(r_midihub.right(), r_midihub.center().y()),
            QPointF(r_capt_out.left(), r_capt_out.center().y() + 26.0),
            c_midi,
            dashed=True,
        )

        # Draw Divider Between Audio/MIDI Plane and Video ScreenCast Plane
        div_y = top_y + audio_h + div_gap * 0.5
        div_pen_col = QColor(c_vid)
        div_pen_col.setAlpha(90)
        p.setPen(QPen(div_pen_col, 1.2, Qt.PenStyle.DashLine))
        p.drawLine(QPointF(16.0, div_y), QPointF(w - 16.0, div_y))

        sock_disp = self.pw_socket_path or "/run/user/pipewire-0"
        div_label = f" ZERO-PIPEWIRE WAYLAND VIDEO & SCREENCAST LANE ({sock_disp}) "
        pill_w = min(560.0, max(280.0, w - 40.0))
        pill_rect = QRectF((w - pill_w) * 0.5, div_y - 9.0, pill_w, 18.0)
        p.setBrush(QColor(t["bg_canvas"]))
        p.setPen(QPen(div_pen_col, 1.0))
        p.drawRoundedRect(pill_rect, 8.0, 8.0)
        p.setFont(QFont("Monospace", 7, QFont.Weight.Bold))
        p.setPen(c_vid)
        p.drawText(pill_rect, Qt.AlignmentFlag.AlignCenter, div_label)

        vid_live, vid_src_d, vid_brk_d, vid_rel_d, vid_con_d = self._build_video_node_contents()
        vw_col = c_vid if vid_live else c_vid_dim

        # Video pipeline wires (Col 1 -> Col 2 -> Col 3 -> Col 4)
        self._draw_wire(
            p,
            QPointF(r_vid_src.right(), r_vid_src.center().y()),
            QPointF(r_vid_broker.left(), r_vid_broker.center().y()),
            vw_col,
            dashed=not vid_live,
            pulse=vid_live,
        )
        self._draw_wire(
            p,
            QPointF(r_vid_broker.right(), r_vid_broker.center().y()),
            QPointF(r_vid_relay.left(), r_vid_relay.center().y()),
            vw_col,
            dashed=not vid_live,
            pulse=vid_live,
        )
        self._draw_wire(
            p,
            QPointF(r_vid_relay.right(), r_vid_relay.center().y()),
            QPointF(r_vid_cons.left(), r_vid_cons.center().y()),
            vw_col,
            dashed=not vid_live,
            pulse=vid_live,
        )
        # Parallel A/V Capture sync indicator between Col 4 Audio Capture and Video Consumers
        av_sync_active = vid_live and any("Loopback / Capture Ready" not in a for a in self.capt_apps)
        self._draw_wire(
            p,
            QPointF(r_capt_out.center().x(), r_capt_out.bottom()),
            QPointF(r_vid_cons.center().x(), r_vid_cons.top()),
            c_warn if av_sync_active else c_vid_dim,
            dashed=True,
            pulse=av_sync_active,
        )

        # Draw Audio & MIDI Nodes
        self._draw_node(
            p,
            "apps_in",
            r_apps_in,
            "🔊 Audio Applications",
            f"{len(self.play_apps)} LIVE",
            [f"• {a}" for a in self.play_apps[:3]] + ["ABI: WASAPI / ALSA / Pulse / JACK"],
            c_sec,
            has_in=False,
            has_out=True,
        )

        mic_badge = f"{self.cap_sample_rate // 1000}k APO" if self.mic_apo_enabled else f"{self.cap_sample_rate // 1000}k Raw"
        mic_apo_line = (
            f"APO: {'Krisp-RNN' if self.mic_rnnoise_active else 'Spectral'} @ {self.cap_sample_rate / 1000.0:g} kHz"
            if self.mic_apo_enabled
            else f"APO: Bypassed ({self.cap_sample_rate / 1000.0:g} kHz Raw)"
        )
        mic_side_line = (
            f"Sidetone: {self.mic_sidetone_pct}% (Post-Loop)"
            if (self.mic_apo_enabled and self.mic_sidetone_enabled)
            else "Sidetone: OFF (OBS Monitor Safe)"
        )
        mic_link_line = f"HFP SCO: {'Armed' if self.bt_hfp_sco else 'OFF (Direct HW Capture)'}"
        self._draw_node(
            p,
            "mic_in",
            r_mic_in,
            "🎙️ Mic & Capture APO",
            mic_badge,
            [
                f"Dev: {self.hw_mic}",
                mic_apo_line,
                mic_side_line,
                mic_link_line,
            ],
            c_mic,
            has_in=False,
            has_out=True,
        )

        self._draw_node(
            p,
            "midi_in",
            r_midi_in,
            "🎹 Hardware MIDI In",
            "RAW+SEQ",
            [
                f"Dev: {self.hw_midi}",
                "Auto: /dev/snd/midiC*D*",
                f"Key Presses: {self.midi_events}",
            ],
            c_midi,
            has_in=False,
            has_out=True,
        )

        self._draw_node(
            p,
            "shm_ring",
            r_shm,
            "⚡ WaveRT Shared Mem",
            "0.075 µs",
            [
                "Transport: memfd_create() Ring",
                "Wakeup: Kernel eventfd()",
                "Capacity: 32,768 Stereo Frames",
                "Zero Socket IPC / Zero Copy",
            ],
            c_sec,
            has_in=True,
            has_out=True,
        )

        period_ms = (1000.0 * self.period_frames) / max(1, self.sample_rate)
        clk_line = f"Master: {self.sample_rate:,} Hz ({self.format_bits}-bit)"
        src_line = (
            f"Mic Clock: {self.cap_sample_rate:,} Hz (Split)"
            if self.cap_sample_rate != self.sample_rate
            else f"MMCSS WaveRT ({self.format_bits}-bit Master)"
        )
        eng_badge = "FIXED MASTER"
        self._draw_node(
            p,
            "engine_core",
            r_engine,
            "🚀 lwasapi-audiodg",
            eng_badge,
            [
                clk_line,
                f"Quantum: {self.period_frames}f ({period_ms:.2f} ms)",
                src_line,
                "Mode: Direct Mixer + Multi-Rate",
            ],
            c_pri,
            has_in=True,
            has_out=True,
        )

        self._draw_node(
            p,
            "dyn_eq",
            r_dyneq,
            "🎚️ Parametric & Dyn EQ",
            "ACTIVE" if self.eq_enabled else "BYPASS",
            [
                f"Preset: {self.eq_preset}",
                f"Mode: {self.eq_mode_str}",
                f"Crossfeed: {'ON (Bauer)' if self.crossfeed_enabled else 'OFF'}",
                "10-Band PK/LS/HS + Limiter",
            ],
            c_hot,
            has_in=True,
            has_out=True,
        )

        self._draw_node(
            p,
            "midihub",
            r_midihub,
            "🎵 DAW MIDI Hub",
            "LOCK-FREE",
            [
                "Seq Port: LWASAPI-MIDI-Hub",
                "Ring: 512 Sample-Accurate Events",
                "Bridges: JACK MIDI + ALSA Seq",
            ],
            c_midi,
            has_in=True,
            has_out=True,
        )

        bt_link_str = (
            f"BT {self.bt_codec}@{self.bt_kbps}k{' ABR' if self.bt_abr else ''} ({self.bt_delay_ms}ms)"
            if self.bt_active
            else f"PA v35 Socket: {self.pa_clients} clients"
        )
        amp_tags = {0: "Auto", 1: "Low", 2: "Med", 3: "600Ω"}
        if (not self.bt_active) and (
            self.hw_dac_unity or self.hw_amp_tier > 0 or self.hw_analog_boost_db > 0.05 or self.hw_hid_box
        ):
            hid_t = " [HID]" if self.hw_hid_box else ""
            drv_t = f" +{self.hw_analog_boost_db:.1f}dB" if self.hw_analog_boost_db > 0.05 else ""
            amp_t = f" Amp:{amp_tags.get(self.hw_amp_tier, 'Auto')}" if self.hw_amp_tier > 0 else ""
            hw_stage_line = f"{self.hw_arch[:22]}{amp_t}{drv_t}{hid_t}"
        else:
            hw_stage_line = "Round-Trip: Down to 0.33 ms"
        fmt_tag = " (HW DAC Unity)" if (self.hw_dac_unity and not self.bt_active) else " Studio Master"
        self._draw_node(
            p,
            "hw_out",
            r_hw_out,
            "🎧 Master Output Sink",
            f"Vol {self.spk_vol}%",
            [
                f"Sink: {self.hw_sink}",
                f"Format: {self.format_bits}-Bit{fmt_tag}",
                hw_stage_line,
                bt_link_str,
            ],
            c_out,
            has_in=True,
            has_out=False,
        )

        self._draw_node(
            p,
            "capt_out",
            r_capt_out,
            "🎙️ Audio Capture & Loop",
            f"{len(self.capt_apps)} REC",
            [f"• {c}" for c in self.capt_apps[:2]]
            + [
                "OBS Monitor: Post-EQ (No Sidetone)",
                "Mic Feed: APO -> Full-Duplex SHM",
            ],
            c_warn,
            has_in=True,
            has_out=False,
        )

        # Draw Video ScreenCast Nodes
        self._draw_node(
            p,
            "vid_src",
            r_vid_src,
            "🖥️ Video Producer",
            vid_src_d[0],
            vid_src_d[1],
            c_vid,
            has_in=False,
            has_out=True,
        )

        self._draw_node(
            p,
            "vid_broker",
            r_vid_broker,
            "🔌 Video PW Broker",
            vid_brk_d[0],
            vid_brk_d[1],
            c_vid if self.pw_broker_running else c_warn,
            has_in=True,
            has_out=True,
        )

        self._draw_node(
            p,
            "vid_relay",
            r_vid_relay,
            "🎞️ Video Relay & Cap",
            vid_rel_d[0],
            vid_rel_d[1],
            c_pri if vid_live else c_vid,
            has_in=True,
            has_out=True,
        )

        self._draw_node(
            p,
            "vid_cons",
            r_vid_cons,
            "📡 Video Consumers",
            vid_con_d[0],
            vid_con_d[1],
            c_warn if vid_live else c_vid,
            has_in=True,
            has_out=False,
        )


class SalsaSpectrumCanvas(QWidget):
    """
    Real-Time 10-Band Spectrum Analyzer + Interactive Parametric & Dynamic EQ Curve Canvas
    """

    def __init__(self, on_band_dragged):
        super().__init__()
        self.setMinimumHeight(165)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setMouseTracking(True)
        self.on_band_dragged = on_band_dragged
        self.theme = THEMES[DEFAULT_THEME]

        self.enabled = True
        self.dynamic_mode = 1
        self.threshold_db = -18.0
        self.static_gains = [0.0] * LWASAPI_EQ_BANDS
        self.dyn_deltas = [0.0] * LWASAPI_EQ_BANDS
        self.band_levels = [-60.0] * LWASAPI_EQ_BANDS
        self.smooth_levels = [-60.0] * LWASAPI_EQ_BANDS
        self.dragging_band = -1
        self.hover_band = -1

    def minimumSizeHint(self):
        return QSize(480, 165)

    def sizeHint(self):
        return QSize(900, 210)

    def set_theme(self, theme: dict):
        self.theme = theme
        self.update()

    def update_telemetry(self, enabled, dynamic_mode, threshold_db, gains, deltas, levels):
        gains_l = list(gains)
        deltas_l = list(deltas)
        changed = (
            self.enabled != enabled
            or self.dynamic_mode != dynamic_mode
            or self.threshold_db != threshold_db
            or self.static_gains != gains_l
            or self.dyn_deltas != deltas_l
        )
        self.enabled = enabled
        self.dynamic_mode = dynamic_mode
        self.threshold_db = threshold_db
        self.static_gains = gains_l
        self.dyn_deltas = deltas_l
        for i in range(LWASAPI_EQ_BANDS):
            target = max(-60.0, min(6.0, levels[i]))
            prev = self.smooth_levels[i]
            nxt = 0.65 * prev + 0.35 * target
            if abs(nxt - target) < 0.02:
                nxt = target
            if nxt != prev:
                changed = True
            self.smooth_levels[i] = nxt
        if changed:
            self.update()

    def _band_x(self, idx: int, w: float) -> float:
        pad = 48.0
        usable = max(100.0, w - pad * 2.0)
        return pad + (usable * idx) / (LWASAPI_EQ_BANDS - 1)

    def _gain_to_y(self, db: float, h: float) -> float:
        pad_top = 18.0
        pad_bot = 28.0
        usable = max(60.0, h - pad_top - pad_bot)
        clamped = max(-18.0, min(18.0, db))
        return pad_top + usable * (1.0 - (clamped + 18.0) / 36.0)

    def _y_to_gain(self, y: float, h: float) -> float:
        pad_top = 18.0
        pad_bot = 28.0
        usable = max(60.0, h - pad_top - pad_bot)
        norm = 1.0 - ((y - pad_top) / usable)
        return max(-18.0, min(18.0, norm * 36.0 - 18.0))

    def mousePressEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            w, h = float(self.width()), float(self.height())
            pos = event.position()
            for i in range(LWASAPI_EQ_BANDS):
                bx = self._band_x(i, w)
                by = self._gain_to_y(self.static_gains[i], h)
                if math.hypot(pos.x() - bx, pos.y() - by) <= 18.0:
                    self.dragging_band = i
                    return

    def mouseMoveEvent(self, event):
        w, h = float(self.width()), float(self.height())
        pos = event.position()
        if self.dragging_band >= 0:
            new_db = round(self._y_to_gain(pos.y(), h) * 2.0) / 2.0
            self.static_gains[self.dragging_band] = new_db
            self.on_band_dragged(self.dragging_band, new_db)
            self.update()
            return

        hov = -1
        for i in range(LWASAPI_EQ_BANDS):
            bx = self._band_x(i, w)
            by = self._gain_to_y(self.static_gains[i], h)
            if math.hypot(pos.x() - bx, pos.y() - by) <= 18.0:
                hov = i
                break
        if hov != self.hover_band:
            self.hover_band = hov
            self.update()

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            self.dragging_band = -1

    def paintEvent(self, event):
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = float(self.width()), float(self.height())
        t = self.theme

        p.setBrush(QColor(t["bg_canvas"]))
        p.setPen(QPen(QColor(t["border_card"]), 1.5))
        p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 10.0, 10.0)

        p.setFont(QFont("Monospace", 8))
        for db in (18, 12, 6, 0, -6, -12, -18):
            y = self._gain_to_y(float(db), h)
            pen = QPen(
                QColor(t["grid_zero"] if db == 0 else t["spectrum_grid"]),
                1.2 if db == 0 else 1.0,
                Qt.PenStyle.DashLine,
            )
            p.setPen(pen)
            p.drawLine(int(40), int(y), int(w - 16), int(y))
            p.setPen(QColor(t["text_dim"]))
            p.drawText(6, int(y + 4), f"{db:+d}dB")

        bar_w = max(14.0, (w - 120.0) / 18.0)
        base_y = h - 26.0
        c_bar_top = QColor(t["accent_hot"])
        c_bar_top.setAlpha(175)
        c_bar_mid = QColor(t["accent_primary"])
        c_bar_mid.setAlpha(135)
        c_bar_bot = QColor(t["accent_secondary"])
        c_bar_bot.setAlpha(65)
        for i in range(LWASAPI_EQ_BANDS):
            bx = self._band_x(i, w)
            lvl_norm = (self.smooth_levels[i] + 60.0) / 66.0
            bar_h = max(3.0, lvl_norm * (h - 48.0))
            bar_grad = QLinearGradient(bx, base_y - bar_h, bx, base_y)
            bar_grad.setColorAt(0.0, c_bar_top)
            bar_grad.setColorAt(0.5, c_bar_mid)
            bar_grad.setColorAt(1.0, c_bar_bot)
            p.setBrush(bar_grad)
            p.setPen(Qt.PenStyle.NoPen)
            p.drawRoundedRect(QRectF(bx - bar_w * 0.5, base_y - bar_h, bar_w, bar_h), 4.0, 4.0)

            p.setPen(QColor(t["text_muted"]))
            p.drawText(QRectF(bx - 28.0, h - 22.0, 56.0, 18.0), Qt.AlignmentFlag.AlignCenter, BAND_FREQS[i])

        eff_pts = []
        stat_pts = []
        for i in range(LWASAPI_EQ_BANDS):
            bx = self._band_x(i, w)
            sy = self._gain_to_y(self.static_gains[i], h)
            ey = self._gain_to_y(self.static_gains[i] + self.dyn_deltas[i], h)
            stat_pts.append(QPointF(bx, sy))
            eff_pts.append(QPointF(bx, ey))

        if len(eff_pts) >= 2:
            dyn_path = QPainterPath()
            dyn_path.moveTo(eff_pts[0])
            for i in range(len(eff_pts) - 1):
                mid_x = 0.5 * (eff_pts[i].x() + eff_pts[i + 1].x())
                dyn_path.cubicTo(
                    QPointF(mid_x, eff_pts[i].y()),
                    QPointF(mid_x, eff_pts[i + 1].y()),
                    eff_pts[i + 1],
                )

            fill_path = QPainterPath(dyn_path)
            zero_y = self._gain_to_y(0.0, h)
            fill_path.lineTo(eff_pts[-1].x(), zero_y)
            fill_path.lineTo(eff_pts[0].x(), zero_y)
            fill_path.closeSubpath()

            c_fill_top = QColor(t["accent_primary"])
            c_fill_top.setAlpha(70)
            c_fill_bot = QColor(t["accent_secondary"])
            c_fill_bot.setAlpha(20)
            fill_grad = QLinearGradient(0.0, 0.0, 0.0, h)
            fill_grad.setColorAt(0.0, c_fill_top)
            fill_grad.setColorAt(1.0, c_fill_bot)
            p.setBrush(fill_grad)
            p.setPen(Qt.PenStyle.NoPen)
            p.drawPath(fill_path)

            p.setBrush(Qt.BrushStyle.NoBrush)
            p.setPen(QPen(QColor(t["accent_warn"]), 2.0, Qt.PenStyle.DotLine))
            p.drawPath(dyn_path)

        if len(stat_pts) >= 2:
            stat_path = QPainterPath()
            stat_path.moveTo(stat_pts[0])
            for i in range(len(stat_pts) - 1):
                mid_x = 0.5 * (stat_pts[i].x() + stat_pts[i + 1].x())
                stat_path.cubicTo(
                    QPointF(mid_x, stat_pts[i].y()),
                    QPointF(mid_x, stat_pts[i + 1].y()),
                    stat_pts[i + 1],
                )
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.setPen(QPen(QColor(t["accent_primary"] if self.enabled else t["text_dim"]), 2.8))
            p.drawPath(stat_path)

        handle_pen = QPen(QColor(t["handle_border"]), 1.8)
        for i, pt in enumerate(stat_pts):
            rad = 7.5 if (i == self.dragging_band or i == self.hover_band) else 5.5
            p.setBrush(QColor(t["accent_hot"] if i == self.dragging_band else t["accent_primary"]))
            p.setPen(handle_pen)
            p.drawEllipse(pt, rad, rad)


class SavedChipLabel(QLabel):
    """Auto-eliding saved-setting badge placed beside Preset and Dynamic Mode."""

    def __init__(self, initial_text: str = "", parent=None):
        super().__init__(parent)
        self._full_text = initial_text
        self.setObjectName("SavedChipLabel")
        self.setAlignment(Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft)
        self.setSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Fixed)
        self.set_saved_text(initial_text)

    def set_saved_text(self, text: str, tooltip: str = "") -> None:
        self._full_text = text or ""
        self.setToolTip(tooltip or self._full_text)
        self.updateGeometry()
        self._update_elided_text()

    def sizeHint(self) -> QSize:
        base = super().sizeHint()
        fm = self.fontMetrics()
        margins = self.contentsMargins()
        w = fm.horizontalAdvance(self._full_text) + margins.left() + margins.right() + 24
        return QSize(max(76, min(w, 270)), max(base.height(), fm.height() + 10))

    def minimumSizeHint(self) -> QSize:
        sh = self.sizeHint()
        return QSize(min(sh.width(), 88), sh.height())

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        self._update_elided_text()

    def _update_elided_text(self) -> None:
        cw = self.contentsRect().width()
        avail = cw if cw > 16 else self.sizeHint().width()
        elided = self.fontMetrics().elidedText(self._full_text, Qt.TextElideMode.ElideRight, max(24, avail))
        if self.text() != elided:
            super().setText(elided)


class SalsaPopWindow(QWidget):
    """Independent Salsa sub-window matching Studio's live palette and QSS."""

    def __init__(self, title: str, on_close=None):
        super().__init__(None, Qt.WindowType.Window)
        self.setWindowTitle(title)
        self._on_close = on_close

    def closeEvent(self, event):
        if callable(self._on_close):
            if self._on_close(event) is False:
                return
        event.accept()


class StepVolumeSlider(QSlider):
    """Horizontal audio volume slider with configurable mouse-wheel & keyboard steps."""

    def __init__(self, step_provider=None, parent=None):
        super().__init__(Qt.Orientation.Horizontal, parent)
        self._step_provider = step_provider
        self.sync_step()

    def _current_step(self) -> int:
        if callable(self._step_provider):
            return normalize_vol_step_pct(self._step_provider())
        return DEFAULT_VOL_STEP_PCT

    def sync_step(self) -> None:
        step = self._current_step()
        self.setSingleStep(step)
        self.setPageStep(step)
        self.setToolTip(f"Drag, mouse wheel, or Left/Right/Up/Down/Page keys adjust by ±{step}%")

    def _apply_step(self, step_units: int) -> None:
        step = self._current_step()
        nv = max(self.minimum(), min(self.maximum(), int(self.value()) + int(step_units) * step))
        if nv != self.value():
            self.setValue(nv)
            self.sliderReleased.emit()

    def wheelEvent(self, ev) -> None:
        dx = ev.angleDelta().x()
        dy = ev.angleDelta().y()
        delta = dy if abs(dy) >= abs(dx) else dx
        if delta != 0:
            notches = max(1, round(abs(delta) / 120.0))
            self._apply_step(notches if delta > 0 else -notches)
        ev.accept()

    def keyPressEvent(self, ev) -> None:
        if ev.key() in (Qt.Key.Key_Right, Qt.Key.Key_Up, Qt.Key.Key_PageUp):
            self._apply_step(1)
            ev.accept()
            return
        if ev.key() in (Qt.Key.Key_Left, Qt.Key.Key_Down, Qt.Key.Key_PageDown):
            self._apply_step(-1)
            ev.accept()
            return
        super().keyPressEvent(ev)


class SalsaStudioWindow(QMainWindow):
    def __init__(self, start_in_graph: bool = False, start_in_debug: bool = False):
        super().__init__()
        self.gui_settings = load_gui_settings()
        self._vol_step_pct = normalize_vol_step_pct(self.gui_settings.get("volume_step_pct"))
        self._hook_proc = None
        self._watcher_proc = None
        self._shortcut_row_labels = {}
        saved_theme = self.gui_settings.get("theme", DEFAULT_THEME)
        base_seed = THEMES[saved_theme] if saved_theme in THEMES else THEMES[DEFAULT_THEME]
        self.custom_theme = normalize_custom_theme(self.gui_settings.get("custom_theme"), base_seed)
        if saved_theme == CUSTOM_THEME_NAME:
            self.current_theme_name = CUSTOM_THEME_NAME
            self.current_theme = copy.deepcopy(self.custom_theme)
        else:
            self.current_theme_name = saved_theme if saved_theme in THEMES else DEFAULT_THEME
            self.current_theme = THEMES[self.current_theme_name]
        self._custom_theme_dlg = None
        self.current_manual_key = "overview"
        self._closing_all = False
        self._start_in_graph_flag = bool(start_in_graph)
        self._studio_entered = False

        self.setWindowTitle(
            "Salsa — Linux WASAPI Studio (192 kHz WaveRT Engine & Dynamic EQ)"
        )
        self.setWindowIcon(create_salsa_icon(self.current_theme))
        try:
            win_w = max(640, min(7680, int(self.gui_settings.get("window_width", 1580))))
        except (TypeError, ValueError):
            win_w = 1580
        try:
            win_h = max(480, min(4320, int(self.gui_settings.get("window_height", 840))))
        except (TypeError, ValueError):
            win_h = 840
        self.resize(win_w, win_h)
        self._gui_settings_mtime = self._read_gui_settings_mtime()

        self.lib = None
        self._lib_mtime = 0.0
        if os.path.exists(LIB_PATH):
            try:
                self._lib_mtime = os.path.getmtime(LIB_PATH)
                self.lib = bind_liblwasapi(ctypes.CDLL(LIB_PATH))
            except Exception as e:
                print(f"[Salsa] Warning loading liblwasapi.so: {e}")

        self._dev_poll_ticks = 0
        self._last_dev_sig = None
        self._active_playback_id = ""
        self._cached_sinks = [("auto", "Auto (Global Output)")]
        self._cached_sources = [("auto", "Auto (Global Mic)")]
        self._cached_pw_telem = None
        self._pw_telem_ts = 0.0
        self._pw_telem_inflight = False
        self._svc_restart_proc = None
        self._sess_widgets = {}
        self._sess_last_send_ts = {}
        self._last_sess_struct_sig = None
        self._last_sess_rows = None
        self._last_applied_eq_sig = None
        self._last_mic_apo_sig = None
        self._last_bt_cfg_sig = None
        self._last_hw_analog_sig = None
        self._last_clock_policy_sig = None
        self._last_cap_rate_sig = None
        self._last_cap_rate_pref = None
        self._cap_rate_hold_until = 0.0
        self._mic_apo_hold_until = 0.0
        self._bt_cfg_hold_until = 0.0
        self._hw_analog_hold_until = 0.0

        self.current_eq = LwasapiEqConfig()
        self.current_eq.enabled = 1
        self.current_eq.dynamic_mode = 1
        self.current_eq.preamp_db = 0.0
        self.current_eq.dyn_threshold_db = -18.0
        self.current_eq.dyn_ratio = 2.5
        self.current_eq.limiter_enabled = 1
        self.current_param_eq = LwasapiParamEqExt()
        self.current_param_eq.crossfeed_enabled = 0
        self.current_param_eq.crossfeed_level = 0.30
        self.current_param_eq.crossfeed_cutoff_hz = 700
        for i in range(LWASAPI_EQ_BANDS):
            self.current_eq.band_gains_db[i] = 0.0
            self.current_eq.band_q[i] = 1.414
            self.current_param_eq.band_freq_hz[i] = DEFAULT_BAND_FREQS_HZ[i]
            self.current_param_eq.band_type[i] = LWASAPI_FILTER_PEAK
        self.current_eq.preset_name = b"Flat / Studio Reference"

        self.current_mic_apo = LwasapiMicApoCfg()
        self.current_mic_apo.enabled = 0
        self.current_mic_apo.hpf_enabled = 1
        self.current_mic_apo.hpf_hz = 65.0
        self.current_mic_apo.gate_enabled = 1
        self.current_mic_apo.gate_thresh_db = -50.0
        self.current_mic_apo.rnnoise_enabled = 1
        self.current_mic_apo.comp_enabled = 1
        self.current_mic_apo.comp_thresh_db = -14.0
        self.current_mic_apo.comp_ratio = 2.0
        self.current_mic_apo.clarity_db = 1.0
        self.current_mic_apo.sidetone_enabled = 0
        self.current_mic_apo.sidetone_pct = 25

        self.current_bt_cfg = LwasapiBtCodecCfg()
        self.current_bt_cfg.preferred_codec = LWASAPI_BT_PREF_AUTO
        self.current_bt_cfg.ldac_mode = LWASAPI_LDAC_MODE_ABR
        self.current_bt_cfg.wireless_abr_enabled = 1
        self.current_bt_cfg.hfp_sco_enabled = 0

        self.current_hw_analog = LwasapiHwAnalogCfg()
        self.current_hw_analog.hw_dac_unity_vol = 1
        self.current_hw_analog.hp_amp_tier = LWASAPI_HW_AMP_AUTO
        self.current_hw_analog.output_route = LWASAPI_HW_ROUTE_AUTO
        self.current_hw_analog.unlock_0db_rails = 0
        self.current_hw_analog.analog_boost_db = 0.0
        self.current_hw_analog.mic_hw_boost_db = 0.0

        self.updating_ui = False
        self.intro_win = None
        self.adv_win = None
        self._build_ui()
        self._build_topology_intro_window()
        self._build_advanced_options_window()
        self._setup_shortcuts()
        self._apply_theme()
        self._restore_saved_eq_on_build()

        self._poll_engine(initial=True)

        self.timer = QTimer(self)
        self.timer.timeout.connect(lambda: self._poll_engine(initial=False))
        self.timer.start(80)

    def _read_gui_settings_mtime(self) -> float:
        try:
            path = _gui_settings_path()
            if os.path.isfile(path) and not os.path.islink(path):
                return os.path.getmtime(path)
        except Exception:
            pass
        return 0.0

    def _apply_palette(self, t: dict):
        pal = build_salsa_palette(t)
        app = QApplication.instance()
        if app is not None:
            app.setPalette(pal)
        self.setPalette(pal)
        if getattr(self, "intro_win", None) is not None:
            self.intro_win.setPalette(pal)
        if getattr(self, "adv_win", None) is not None:
            self.adv_win.setPalette(pal)

    def _apply_theme(self):
        t = self.current_theme
        ico = create_salsa_icon(t)
        self._apply_palette(t)
        self._apply_stylesheet()
        self.setWindowIcon(ico)
        self.canvas.set_theme(t)
        self.graph_canvas.set_theme(t)
        if getattr(self, "intro_win", None) is not None:
            self.intro_win.setWindowIcon(ico)
        if getattr(self, "adv_win", None) is not None:
            self.adv_win.setWindowIcon(ico)
        self.manual_browser.document().setDefaultStyleSheet(
            f"body, p, li {{ color: {t['text_primary']}; }} "
            f"code {{ background-color: {t['code_bg']}; color: {t['accent_secondary']}; "
            f"padding: 1px 4px; border-radius: 3px; }}"
        )
        self.manual_browser.setHtml(render_manual_chapter(self.current_manual_key, t))
        if hasattr(self, "lbl_ver_pill"):
            self.lbl_ver_pill.setStyleSheet(
                f"background-color: {t['bg_deep']}; color: {t['accent_warn']}; "
                f"border: 1px solid {t['accent_warn']}; border-radius: 5px; "
                "padding: 2px 8px; font-weight: 800; font-size: 11px;"
            )
        if hasattr(self, "btn_toggle_pacman_hook"):
            self._refresh_adv_integration_status()

    def _on_theme_activated(self, idx: int):
        theme_name = self.cmb_theme.itemText(idx)
        if theme_name == CUSTOM_THEME_NAME:
            self._open_custom_theme_editor()
        else:
            self._on_theme_changed(theme_name)

    def _on_theme_changed(self, theme_name: str):
        if theme_name == CUSTOM_THEME_NAME:
            self.current_theme_name = CUSTOM_THEME_NAME
            self.current_theme = copy.deepcopy(self.custom_theme)
        elif theme_name in THEMES:
            if theme_name == self.current_theme_name:
                return
            self.current_theme_name = theme_name
            self.current_theme = THEMES[theme_name]
        else:
            return
        self.gui_settings["theme"] = self.current_theme_name
        save_gui_settings(self.gui_settings)
        self._apply_theme()

    def _open_custom_theme_editor(self):
        self._theme_edit_prev = (self.current_theme_name, copy.deepcopy(self.current_theme))
        has_saved_custom = isinstance(self.gui_settings.get("custom_theme"), dict)
        seed_theme = self.custom_theme if has_saved_custom else self.current_theme
        if self._custom_theme_dlg is None:
            dlg = SalsaCustomThemeDialog(seed_theme, THEMES, parent=self)
            dlg.live_theme_changed.connect(self._on_custom_theme_live)
            dlg.theme_applied.connect(self._on_custom_theme_applied)
            dlg.mirror_stopped.connect(self._on_custom_mirror_stopped)
            dlg.rejected.connect(self._on_custom_theme_rejected)
            self._custom_theme_dlg = dlg
        else:
            self._custom_theme_dlg.reset_for_open(seed_theme)
        if self.current_theme_name in THEMES:
            self._custom_theme_dlg.cmb_clone.setCurrentText(self.current_theme_name)
        self._custom_theme_dlg.show()
        self._custom_theme_dlg.raise_()
        self._custom_theme_dlg.activateWindow()

    def _on_custom_theme_live(self, theme_dict: dict):
        self.current_theme = normalize_custom_theme(theme_dict, THEMES[DEFAULT_THEME])
        self._apply_theme()

    def _on_custom_mirror_stopped(self):
        prev_name, prev_theme = getattr(self, "_theme_edit_prev", (self.current_theme_name, self.current_theme))
        self.current_theme_name = prev_name
        self.current_theme = copy.deepcopy(prev_theme)
        self._apply_theme()

    def _on_custom_theme_applied(self, theme_dict: dict):
        self.custom_theme = normalize_custom_theme(theme_dict, THEMES[DEFAULT_THEME])
        self.current_theme_name = CUSTOM_THEME_NAME
        self.current_theme = copy.deepcopy(self.custom_theme)
        self._theme_edit_prev = (CUSTOM_THEME_NAME, copy.deepcopy(self.custom_theme))
        self.gui_settings["theme"] = CUSTOM_THEME_NAME
        self.gui_settings["custom_theme"] = self.custom_theme
        save_gui_settings(self.gui_settings)
        self.cmb_theme.blockSignals(True)
        self.cmb_theme.setCurrentText(CUSTOM_THEME_NAME)
        self.cmb_theme.blockSignals(False)
        self._apply_theme()

    def _on_custom_theme_rejected(self):
        prev_name, prev_theme = getattr(self, "_theme_edit_prev", (self.current_theme_name, self.current_theme))
        self.current_theme_name = prev_name
        self.current_theme = copy.deepcopy(prev_theme)
        self.cmb_theme.blockSignals(True)
        self.cmb_theme.setCurrentText(prev_name)
        self.cmb_theme.blockSignals(False)
        self._apply_theme()

    def _apply_stylesheet(self):
        t = self.current_theme
        qss = build_salsa_stylesheet(t, mini=False) + f"""
        QKeySequenceEdit, QKeySequenceEdit QLineEdit {{
            background-color: {t['bg_control']};
            color: {t['text_bright']};
            border: 1px solid {t['border_control']};
            border-radius: 6px;
            padding: 3px 8px;
            font-weight: 700;
        }}
        QKeySequenceEdit:focus, QKeySequenceEdit QLineEdit:focus {{
            border-color: {t['accent_primary']};
            background-color: {t['bg_hover']};
        }}
        """
        self.setStyleSheet(qss)
        if getattr(self, "intro_win", None) is not None:
            self.intro_win.setStyleSheet(qss)
        if getattr(self, "adv_win", None) is not None:
            self.adv_win.setStyleSheet(qss)

    def _build_ui(self):
        central = QWidget()
        self.setCentralWidget(central)
        root = QVBoxLayout(central)
        root.setContentsMargins(10, 8, 10, 8)
        root.setSpacing(6)
        self._root_layout = root

        # =====================================================================
        # TOP BAR: Branding + Clocks + Topology / Advanced Window Buttons
        # =====================================================================
        top_card = QFrame()
        top_card.setObjectName("Card")
        top_layout = QHBoxLayout(top_card)
        top_layout.setContentsMargins(12, 8, 12, 8)
        top_layout.setSpacing(10)

        title_col = QVBoxLayout()
        title_row = QHBoxLayout()
        title_row.setContentsMargins(0, 0, 0, 0)
        title_row.setSpacing(8)
        self.lbl_title = QLabel("SALSA — LINUX WASAPI STUDIO")
        self.lbl_title.setObjectName("TitleBanner")
        title_row.addWidget(self.lbl_title)
        self.lbl_ver_pill = QLabel("v6.9-beta")
        self.lbl_ver_pill.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
        t_init = self.current_theme
        self.lbl_ver_pill.setStyleSheet(
            f"background-color: {t_init['bg_deep']}; color: {t_init['accent_warn']}; "
            f"border: 1px solid {t_init['accent_warn']}; border-radius: 5px; "
            "padding: 2px 8px; font-weight: 800; font-size: 11px;"
        )
        title_row.addWidget(self.lbl_ver_pill)
        title_row.addStretch(1)
        title_col.addLayout(title_row)
        self.lbl_subtitle = QLabel(
            f"Fixed-Clock Lossless WaveRT • Hardware DAC/Amp Analog • Mic APO & Studio EQ • Host: {HOST_PROFILE['os']} ({HOST_PROFILE['cpu']})"
        )
        self.lbl_subtitle.setObjectName("SubBanner")
        self.lbl_subtitle.setWordWrap(True)
        title_col.addWidget(self.lbl_subtitle)
        top_layout.addLayout(title_col, 2)

        rate_col = QVBoxLayout()
        rate_col.addWidget(QLabel("Master Sample Rate:"))
        self.cmb_rate = QComboBox()
        self.rates = [44100, 48000, 88200, 96000, 176400, 192000]
        for r in self.rates:
            tag = " (Ultra-HD Master)" if r >= 176400 else (" (Hi-Res)" if r >= 88200 else "")
            self.cmb_rate.addItem(f"{r:,} Hz{tag}", r)
        self.cmb_rate.setCurrentIndex(1)
        self.cmb_rate.currentIndexChanged.connect(self._on_clock_changed)
        rate_col.addWidget(self.cmb_rate)
        self.lbl_clock_source = QLabel("Master Clock: 48kHz / 32-bit PCM")
        self.lbl_clock_source.setObjectName("BandDynLabel")
        rate_col.addWidget(self.lbl_clock_source)
        top_layout.addLayout(rate_col)

        period_col = QVBoxLayout()
        period_col.addWidget(QLabel("WaveRT Quantum (Period):"))
        self.cmb_period = QComboBox()
        self.periods = [64, 128, 256, 512, 1024, 2048]
        for p in self.periods:
            self.cmb_period.addItem(f"{p} frames", p)
        self.cmb_period.setCurrentIndex(2)
        self.cmb_period.currentIndexChanged.connect(self._on_clock_changed)
        period_col.addWidget(self.cmb_period)
        self.cmb_cap_rate = QComboBox()
        self.cmb_cap_rate.addItem("Mic Clock: Auto (Follow Master)", 0)
        for r in self.rates:
            self.cmb_cap_rate.addItem(f"Mic Clock: {r:,} Hz", r)
        self.cmb_cap_rate.setToolTip("Pin microphone/capture clock independently (e.g. 48,000 Hz for Discord while keeping headphones at 192 kHz)")
        self.cmb_cap_rate.currentIndexChanged.connect(self._on_cap_rate_changed)
        period_col.addWidget(self.cmb_cap_rate)
        top_layout.addLayout(period_col)

        bits_col = QVBoxLayout()
        bits_col.addWidget(QLabel("Bit Depth / Precision:"))
        self.cmb_bits = QComboBox()
        self.bit_depths = [32, 24, 16]
        self.cmb_bits.addItem("32-Bit Float (Studio)", 32)
        self.cmb_bits.addItem("24-Bit Studio PCM", 24)
        self.cmb_bits.addItem("16-Bit Standard PCM", 16)
        self.cmb_bits.currentIndexChanged.connect(self._on_clock_changed)
        bits_col.addWidget(self.cmb_bits)
        top_layout.addLayout(bits_col)

        theme_col = QVBoxLayout()
        theme_col.addWidget(QLabel("GUI Theme:"))
        self.cmb_theme = QComboBox()
        for theme_name in THEMES:
            self.cmb_theme.addItem(theme_name)
        self.cmb_theme.addItem(CUSTOM_THEME_NAME)
        self.cmb_theme.setCurrentText(self.current_theme_name)
        self.cmb_theme.activated.connect(self._on_theme_activated)
        theme_col.addWidget(self.cmb_theme)
        top_layout.addLayout(theme_col)

        eng_col = QVBoxLayout()
        self.lbl_latency = QLabel("Quantum: 5.33 ms | SCHED_FIFO")
        self.lbl_latency.setObjectName("LatencyLabel")
        eng_col.addWidget(self.lbl_latency)
        hdr_btns = QHBoxLayout()
        hdr_btns.setContentsMargins(0, 0, 0, 0)
        hdr_btns.setSpacing(6)
        self.btn_open_topology = QPushButton("🕸️ Topology (F1)")
        self.btn_open_topology.setToolTip("Open Auto-Graph Signal Topology & Owner's Manual window (F1)")
        self.btn_open_topology.clicked.connect(self._open_topology_intro)
        hdr_btns.addWidget(self.btn_open_topology)
        self.btn_open_advanced = QPushButton("⚙️ Advanced Options (Ctrl+,)")
        self.btn_open_advanced.setObjectName("PrimaryBtn")
        self.btn_open_advanced.setToolTip("Open Advanced Options, Standalone Live Debugger, DSP, Hardware Analog & Shortcuts (Ctrl+,)")
        self.btn_open_advanced.clicked.connect(self._open_advanced_options)
        hdr_btns.addWidget(self.btn_open_advanced)
        eng_col.addLayout(hdr_btns)
        top_layout.addLayout(eng_col)

        root.addWidget(top_card)

        # ---------------------------------------------------------------------
        # MAIN STUDIO VIEW: 192 kHz Clock, Dynamic EQ, Mic APO & Full-Duplex Mixer
        # ---------------------------------------------------------------------
        tab_studio = QWidget()
        studio_layout = QVBoxLayout(tab_studio)
        studio_layout.setContentsMargins(0, 2, 0, 0)
        studio_layout.setSpacing(6)

        eq_card = QFrame()
        eq_card.setObjectName("Card")
        eq_layout = QVBoxLayout(eq_card)
        eq_layout.setContentsMargins(12, 7, 12, 7)
        eq_layout.setSpacing(5)

        eq_bar = QHBoxLayout()
        eq_bar.setSpacing(6)
        self.chk_eq_enable = QCheckBox("Enable Salsa 10-Band Dynamic EQ APO")
        self.chk_eq_enable.setChecked(True)
        self.chk_eq_enable.setObjectName("EqEnableCheck")
        self.chk_eq_enable.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
        self.chk_eq_enable.toggled.connect(self._send_eq_update)
        eq_bar.addWidget(self.chk_eq_enable)

        eq_bar.addSpacing(4)
        lbl_preset_hdr = QLabel("Preset:")
        lbl_preset_hdr.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Preferred)
        eq_bar.addWidget(lbl_preset_hdr)
        self.lbl_saved_preset = SavedChipLabel("Saved: Flat / Studio Reference")
        eq_bar.addWidget(self.lbl_saved_preset)
        self.cmb_preset = QComboBox()
        self.cmb_preset.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
        for name in PRESETS:
            self.cmb_preset.addItem(name)
        self.cmb_preset.currentTextChanged.connect(self._on_preset_selected)
        eq_bar.addWidget(self.cmb_preset)

        eq_bar.addSpacing(4)
        lbl_dyn_hdr = QLabel("Dynamic Mode:")
        lbl_dyn_hdr.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Preferred)
        eq_bar.addWidget(lbl_dyn_hdr)
        self.lbl_saved_dyn_mode = SavedChipLabel("Saved: Dynamic Compress")
        eq_bar.addWidget(self.lbl_saved_dyn_mode)
        self.cmb_dyn_mode = QComboBox()
        self.cmb_dyn_mode.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
        self.cmb_dyn_mode.addItem("Static Parametric EQ", 0)
        self.cmb_dyn_mode.addItem("Dynamic Compress (Tame Peaks)", 1)
        self.cmb_dyn_mode.addItem("Dynamic Expand (Upward Punch)", 2)
        self.cmb_dyn_mode.setCurrentIndex(1)
        self.cmb_dyn_mode.currentIndexChanged.connect(self._send_eq_update)
        eq_bar.addWidget(self.cmb_dyn_mode)

        btn_flat = QPushButton("Reset Flat")
        btn_flat.setObjectName("CompactBtn")
        btn_flat.setToolTip("Reset all EQ bands flat (0 dB), restore octave centers, and bypass Crossfeed")
        btn_flat.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
        btn_flat.clicked.connect(self._on_reset_flat_clicked)
        eq_bar.addWidget(btn_flat)
        eq_bar.addStretch(1)
        eq_layout.addLayout(eq_bar)

        self.canvas = SalsaSpectrumCanvas(self._on_canvas_band_dragged)
        eq_layout.addWidget(self.canvas)

        dyn_row = QHBoxLayout()
        self.lbl_preamp = QLabel("Preamp: +0.0 dB")
        dyn_row.addWidget(self.lbl_preamp)
        self.sld_preamp = QSlider(Qt.Orientation.Horizontal)
        self.sld_preamp.setRange(-120, 120)
        self.sld_preamp.setValue(0)
        self.sld_preamp.valueChanged.connect(self._on_dynamics_slider_changed)
        dyn_row.addWidget(self.sld_preamp)

        dyn_row.addSpacing(12)
        self.lbl_thresh = QLabel("Dyn Threshold: -18.0 dB")
        dyn_row.addWidget(self.lbl_thresh)
        self.sld_thresh = QSlider(Qt.Orientation.Horizontal)
        self.sld_thresh.setRange(-480, 0)
        self.sld_thresh.setValue(-180)
        self.sld_thresh.valueChanged.connect(self._on_dynamics_slider_changed)
        dyn_row.addWidget(self.sld_thresh)

        dyn_row.addSpacing(12)
        self.lbl_ratio = QLabel("Dyn Ratio: 2.5:1")
        dyn_row.addWidget(self.lbl_ratio)
        self.sld_ratio = QSlider(Qt.Orientation.Horizontal)
        self.sld_ratio.setRange(10, 80)
        self.sld_ratio.setValue(25)
        self.sld_ratio.valueChanged.connect(self._on_dynamics_slider_changed)
        dyn_row.addWidget(self.sld_ratio)

        dyn_row.addSpacing(16)
        self.lbl_peaks = QLabel("Output Peak L/R: -60.0 / -60.0 dBFS")
        self.lbl_peaks.setObjectName("PeaksLabel")
        dyn_row.addWidget(self.lbl_peaks)
        eq_layout.addLayout(dyn_row)

        faders_grid = QGridLayout()
        faders_grid.setSpacing(6)
        self.band_sliders = []
        self.band_val_labels = []
        self.band_dyn_labels = []
        self.band_freq_labels = []

        for i in range(LWASAPI_EQ_BANDS):
            col_box = QVBoxLayout()
            col_box.setSpacing(2)
            col_box.setAlignment(Qt.AlignmentFlag.AlignHCenter)

            lbl_val = QLabel("+0.0 dB")
            lbl_val.setAlignment(Qt.AlignmentFlag.AlignCenter)
            lbl_val.setObjectName("BandValLabel")
            col_box.addWidget(lbl_val)

            sld = QSlider(Qt.Orientation.Vertical)
            sld.setRange(-180, 180)
            sld.setValue(0)
            sld.setMinimumHeight(56)
            sld.valueChanged.connect(lambda val, idx=i: self._on_band_slider_changed(idx, val))
            col_box.addWidget(sld, alignment=Qt.AlignmentFlag.AlignHCenter)

            lbl_dyn = QLabel("Δ 0.0dB")
            lbl_dyn.setAlignment(Qt.AlignmentFlag.AlignCenter)
            lbl_dyn.setObjectName("BandDynLabel")
            col_box.addWidget(lbl_dyn)

            lbl_freq = QLabel(BAND_FREQS[i])
            lbl_freq.setAlignment(Qt.AlignmentFlag.AlignCenter)
            lbl_freq.setObjectName("BandFreqLabel")
            col_box.addWidget(lbl_freq)

            faders_grid.addLayout(col_box, 0, i)
            self.band_sliders.append(sld)
            self.band_val_labels.append(lbl_val)
            self.band_dyn_labels.append(lbl_dyn)
            self.band_freq_labels.append(lbl_freq)

        eq_layout.addLayout(faders_grid)

        studio_layout.addWidget(eq_card, 3)

        apo_mix_row = QHBoxLayout()
        apo_mix_row.setSpacing(10)

        mic_apo_card = QFrame()
        mic_apo_card.setObjectName("Card")
        mic_apo_col = QVBoxLayout(mic_apo_card)
        mic_apo_col.setContentsMargins(12, 8, 12, 8)
        mic_apo_col.setSpacing(6)

        mic_apo_top = QHBoxLayout()
        self.chk_mic_apo = QCheckBox("KRISP-STYLE CAPTURE MIC APO (Scarlett/HW Passes Raw When OFF)")
        self.chk_mic_apo.setObjectName("EqEnableCheck")
        self.chk_mic_apo.toggled.connect(self._on_mic_apo_changed)
        mic_apo_top.addWidget(self.chk_mic_apo)
        mic_apo_top.addSpacing(8)
        mic_apo_top.addWidget(QLabel("Mic Preset:"))
        self.cmb_mic_preset = QComboBox()
        self.cmb_mic_preset.addItem("Custom Mic APO", "")
        for pname in MIC_APO_PRESETS:
            self.cmb_mic_preset.addItem(pname, pname)
        self.cmb_mic_preset.setToolTip("Load a quick reversible Mic APO profile (e.g. Discord Voice with RNNoise + -50dB gate)")
        self.cmb_mic_preset.activated.connect(self._on_mic_preset_selected)
        mic_apo_top.addWidget(self.cmb_mic_preset)
        mic_apo_top.addStretch(1)
        self.lbl_mic_apo_meter = QLabel("Bypassed (Raw Mic)")
        self.lbl_mic_apo_meter.setObjectName("BandDynLabel")
        mic_apo_top.addWidget(self.lbl_mic_apo_meter)
        mic_apo_col.addLayout(mic_apo_top)

        mic_apo_ctrl = QHBoxLayout()
        mic_apo_ctrl.setSpacing(6)
        self.chk_mic_hpf = QCheckBox("HPF 80Hz")
        self.chk_mic_hpf.setChecked(True)
        self.chk_mic_hpf.toggled.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.chk_mic_hpf)

        self.chk_mic_gate = QCheckBox("Gate")
        self.chk_mic_gate.setChecked(True)
        self.chk_mic_gate.toggled.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.chk_mic_gate)
        self.spn_mic_gate = QSpinBox()
        self.spn_mic_gate.setRange(-80, -10)
        self.spn_mic_gate.setValue(-50)
        self.spn_mic_gate.setSuffix(" dB")
        self.spn_mic_gate.valueChanged.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.spn_mic_gate)

        self.chk_mic_rnnoise = QCheckBox("RNNoise AI")
        self.chk_mic_rnnoise.setChecked(True)
        self.chk_mic_rnnoise.toggled.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.chk_mic_rnnoise)

        self.chk_mic_comp = QCheckBox("Comp")
        self.chk_mic_comp.setChecked(True)
        self.chk_mic_comp.toggled.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.chk_mic_comp)

        mic_apo_ctrl.addWidget(QLabel("Clarity:"))
        self.spn_mic_clarity = QDoubleSpinBox()
        self.spn_mic_clarity.setRange(-6.0, 9.0)
        self.spn_mic_clarity.setSingleStep(0.5)
        self.spn_mic_clarity.setValue(1.5)
        self.spn_mic_clarity.setSuffix(" dB")
        self.spn_mic_clarity.valueChanged.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.spn_mic_clarity)

        self.chk_mic_sidetone = QCheckBox("Sidetone")
        self.chk_mic_sidetone.toggled.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.chk_mic_sidetone)
        self.spn_mic_sidetone = QSpinBox()
        self.spn_mic_sidetone.setRange(0, 100)
        self.spn_mic_sidetone.setValue(25)
        self.spn_mic_sidetone.setSuffix("%")
        self.spn_mic_sidetone.valueChanged.connect(self._on_mic_apo_changed)
        mic_apo_ctrl.addWidget(self.spn_mic_sidetone)
        mic_apo_col.addLayout(mic_apo_ctrl)
        apo_mix_row.addWidget(mic_apo_card, 1)

        studio_layout.addLayout(apo_mix_row)

        bot_layout = QHBoxLayout()
        bot_layout.setSpacing(10)

        hw_card = QFrame()
        hw_card.setObjectName("Card")
        hw_col = QVBoxLayout(hw_card)
        hw_col.setContentsMargins(12, 6, 12, 6)
        hw_col.setSpacing(3)

        hw_hdr_row = QHBoxLayout()
        hw_hdr_row.setSpacing(8)
        lbl_hw_hdr = QLabel("FULL-DUPLEX MASTER MIXER & DAW MIDI HUB")
        lbl_hw_hdr.setObjectName("SectionHeader")
        hw_hdr_row.addWidget(lbl_hw_hdr)
        hw_hdr_row.addStretch()
        hw_hdr_row.addWidget(QLabel("Wheel/Key Step:"))
        self.cmb_vol_step = QComboBox()
        for step_opt in SUPPORTED_VOL_STEPS:
            self.cmb_vol_step.addItem(f"{step_opt}%", step_opt)
        s_idx = self.cmb_vol_step.findData(self._vol_step_pct)
        if s_idx >= 0:
            self.cmb_vol_step.setCurrentIndex(s_idx)
        self.cmb_vol_step.setMinimumWidth(68)
        self.cmb_vol_step.setToolTip("Mouse wheel and keyboard arrow/shortcut volume step for Speaker, Mic & App sliders")
        self.cmb_vol_step.currentIndexChanged.connect(self._on_vol_step_changed)
        hw_hdr_row.addWidget(self.cmb_vol_step)
        hw_col.addLayout(hw_hdr_row)

        self.lbl_endpoints = QLabel("Sink: Detecting... | Mic: Detecting... | PA v35: Initializing...")
        self.lbl_endpoints.setObjectName("EndpointsLabel")
        hw_col.addWidget(self.lbl_endpoints)

        dev_row = QHBoxLayout()
        dev_row.addWidget(QLabel("Output Sink:"))
        self.cmb_sink = QComboBox()
        self.cmb_sink.addItem("Auto-Priority (BT A2DP > USB > Analog > HDMI)", "auto")
        self.cmb_sink.activated.connect(self._on_sink_selected)
        dev_row.addWidget(self.cmb_sink, 1)
        self.btn_bt_reset = QPushButton("Reset BT")
        self.btn_bt_reset.setToolTip("Cleanly reset & reconnect Bluetooth A2DP headphones over BR/EDR")
        self.btn_bt_reset.clicked.connect(self._on_bt_reset_clicked)
        self._bt_reset_proc = None
        dev_row.addWidget(self.btn_bt_reset)
        dev_row.addWidget(QLabel("Mic Input:"))
        self.cmb_source = QComboBox()
        self.cmb_source.addItem("Auto-Priority (USB Mic > HDA > Virtual)", "auto")
        self.cmb_source.activated.connect(self._on_source_selected)
        dev_row.addWidget(self.cmb_source, 1)
        hw_col.addLayout(dev_row)

        bt_row = QHBoxLayout()
        bt_row.setSpacing(6)
        bt_row.addWidget(QLabel("BT Codec:"))
        self.cmb_bt_codec = QComboBox()
        self.cmb_bt_codec.addItem("Auto (Best)", LWASAPI_BT_PREF_AUTO)
        self.cmb_bt_codec.addItem("LDAC", LWASAPI_BT_PREF_LDAC)
        self.cmb_bt_codec.addItem("aptX HD", LWASAPI_BT_PREF_APTX_HD)
        self.cmb_bt_codec.addItem("aptX", LWASAPI_BT_PREF_APTX)
        self.cmb_bt_codec.addItem("AAC", LWASAPI_BT_PREF_AAC)
        self.cmb_bt_codec.addItem("SBC", LWASAPI_BT_PREF_SBC)
        self.cmb_bt_codec.currentIndexChanged.connect(self._on_bt_cfg_changed)
        bt_row.addWidget(self.cmb_bt_codec)

        self.cmb_ldac_mode = QComboBox()
        self.cmb_ldac_mode.addItem("ABR Adaptive", LWASAPI_LDAC_MODE_ABR)
        self.cmb_ldac_mode.addItem("990 kbps (HQ)", LWASAPI_LDAC_MODE_HQ_990)
        self.cmb_ldac_mode.addItem("660 kbps (SQ)", LWASAPI_LDAC_MODE_SQ_660)
        self.cmb_ldac_mode.addItem("330 kbps (MQ)", LWASAPI_LDAC_MODE_MQ_330)
        self.cmb_ldac_mode.currentIndexChanged.connect(self._on_bt_cfg_changed)
        bt_row.addWidget(self.cmb_ldac_mode)

        self.chk_bt_abr = QCheckBox("Wireless ABR")
        self.chk_bt_abr.setChecked(True)
        self.chk_bt_abr.setToolTip("Automatically step LDAC/AAC bitrate down on link congestion and recover cleanly")
        self.chk_bt_abr.toggled.connect(self._on_bt_cfg_changed)
        bt_row.addWidget(self.chk_bt_abr)

        self.chk_bt_sco = QCheckBox("HFP Mic (SCO)")
        self.chk_bt_sco.setChecked(False)
        self.chk_bt_sco.setToolTip("Optional two-way Bluetooth HFP BTPROTO_SCO (mSBC/CVSD) headset mic; OFF keeps Scarlett/HW mic")
        self.chk_bt_sco.toggled.connect(self._on_bt_cfg_changed)
        bt_row.addWidget(self.chk_bt_sco)

        self.lbl_bt_diag = QLabel("BT: Standby")
        self.lbl_bt_diag.setObjectName("BandDynLabel")
        bt_row.addWidget(self.lbl_bt_diag, 1)
        hw_col.addLayout(bt_row)

        spk_row = QHBoxLayout()
        self.lbl_spk = QLabel("Speaker Vol (80%):")
        self.lbl_spk.setMinimumWidth(135)
        spk_row.addWidget(self.lbl_spk)
        self.sld_spk = StepVolumeSlider(lambda: self._vol_step_pct)
        self.sld_spk.setRange(0, 150)
        self.sld_spk.setValue(80)
        self._spk_proc = None
        self._spk_send_timer = QTimer(self)
        self._spk_send_timer.setSingleShot(True)
        self._spk_send_timer.setInterval(40)
        self._spk_send_timer.timeout.connect(self._flush_spk_vol)
        self._spk_hold_timer = QTimer(self)
        self._spk_hold_timer.setSingleShot(True)
        self._spk_hold_timer.setInterval(350)
        self.sld_spk.valueChanged.connect(self._on_spk_vol_moving)
        self.sld_spk.sliderReleased.connect(self._flush_spk_vol)
        spk_row.addWidget(self.sld_spk)
        self.btn_spk_mute = QPushButton("Mute Spk")
        self.btn_spk_mute.clicked.connect(lambda: subprocess.Popen([CTL_BIN, "mute", "toggle"]))
        spk_row.addWidget(self.btn_spk_mute)
        hw_col.addLayout(spk_row)

        mic_row = QHBoxLayout()
        self.lbl_mic = QLabel("Mic Capture (100%):")
        self.lbl_mic.setMinimumWidth(135)
        mic_row.addWidget(self.lbl_mic)
        self.sld_mic = StepVolumeSlider(lambda: self._vol_step_pct)
        self.sld_mic.setRange(0, 150)
        self.sld_mic.setValue(100)
        self._mic_proc = None
        self._mic_send_timer = QTimer(self)
        self._mic_send_timer.setSingleShot(True)
        self._mic_send_timer.setInterval(40)
        self._mic_send_timer.timeout.connect(self._flush_mic_vol)
        self._mic_hold_timer = QTimer(self)
        self._mic_hold_timer.setSingleShot(True)
        self._mic_hold_timer.setInterval(350)
        self.sld_mic.valueChanged.connect(self._on_mic_vol_moving)
        self.sld_mic.sliderReleased.connect(self._flush_mic_vol)
        mic_row.addWidget(self.sld_mic)
        self.btn_mic_mute = QPushButton("Mute Mic")
        self.btn_mic_mute.clicked.connect(lambda: subprocess.Popen([CTL_BIN, "mic-mute", "toggle"]))
        mic_row.addWidget(self.btn_mic_mute)
        hw_col.addLayout(mic_row)

        self.lbl_midi_status = QLabel("DAW MIDI Hub: Auto-Detecting (0 key presses)")
        hw_col.addWidget(self.lbl_midi_status)

        bot_layout.addWidget(hw_card, 2)

        sess_card = QFrame()
        sess_card.setObjectName("Card")
        sess_col = QVBoxLayout(sess_card)
        sess_col.setContentsMargins(14, 10, 14, 10)
        sess_col.setSpacing(6)

        self.lbl_sess_hdr = QLabel("AUTO-RECOGNIZED FULL-DUPLEX SESSIONS (0)")
        self.lbl_sess_hdr.setObjectName("SectionHeader")
        sess_col.addWidget(self.lbl_sess_hdr)

        self.tbl_sessions = QTableWidget(0, 8)
        self.tbl_sessions.setHorizontalHeaderLabels(
            ["ID", "Dir", "Application / Stream", "Mode", "Format", "Volume", "Mute", "Route Target"]
        )
        hdr = self.tbl_sessions.horizontalHeader()
        hdr.setSectionResizeMode(2, QHeaderView.ResizeMode.Stretch)
        hdr.setSectionResizeMode(5, QHeaderView.ResizeMode.Fixed)
        hdr.setSectionResizeMode(6, QHeaderView.ResizeMode.Fixed)
        hdr.setSectionResizeMode(7, QHeaderView.ResizeMode.Fixed)
        self.tbl_sessions.setColumnWidth(0, 56)
        self.tbl_sessions.setColumnWidth(1, 68)
        self.tbl_sessions.setColumnWidth(3, 66)
        self.tbl_sessions.setColumnWidth(4, 128)
        self.tbl_sessions.setColumnWidth(5, 148)
        self.tbl_sessions.setColumnWidth(6, 72)
        self.tbl_sessions.setColumnWidth(7, 170)
        self.tbl_sessions.verticalHeader().setVisible(False)
        self.tbl_sessions.verticalHeader().setDefaultSectionSize(32)
        self.tbl_sessions.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        self.tbl_sessions.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        self.tbl_sessions.setSelectionMode(QTableWidget.SelectionMode.NoSelection)
        sess_col.addWidget(self.tbl_sessions)

        bot_layout.addWidget(sess_card, 3)
        studio_layout.addLayout(bot_layout, 2)

        studio_scroll = QScrollArea()
        studio_scroll.setWidgetResizable(True)
        studio_scroll.setFrameShape(QFrame.Shape.NoFrame)
        studio_scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        studio_scroll.setWidget(tab_studio)
        self._root_layout.addWidget(studio_scroll, 1)

    def _build_topology_intro_window(self):
        self.intro_win = SalsaPopWindow(
            "Salsa — Auto-Graph Topology & Owner's Manual",
            on_close=self._on_intro_win_close,
        )
        self.intro_win.resize(1340, 840)
        graph_layout = QVBoxLayout(self.intro_win)
        graph_layout.setContentsMargins(10, 8, 10, 8)
        graph_layout.setSpacing(8)

        graph_card = QFrame()
        graph_card.setObjectName("Card")
        graph_box = QVBoxLayout(graph_card)
        graph_box.setContentsMargins(14, 10, 14, 10)
        graph_box.setSpacing(6)

        graph_hdr_row = QHBoxLayout()
        lbl_g_title = QLabel(
            "AUTOMATIC AUDIO, MIDI & VIDEO SIGNAL TOPOLOGY (100% SELF-WIRING — CLICK ANY NODE TO INSPECT)"
        )
        lbl_g_title.setObjectName("SectionHeader")
        graph_hdr_row.addWidget(lbl_g_title)
        graph_hdr_row.addStretch()

        self.lbl_auto_badge = QLabel("✔ v6.9-beta LIVE | MASTER CLOCK | HW ANALOG DAC | MIC APO | BT ABR")
        self.lbl_auto_badge.setObjectName("AutoBadgeLabel")
        graph_hdr_row.addWidget(self.lbl_auto_badge)
        graph_box.addLayout(graph_hdr_row)

        self.graph_canvas = SalsaAutoGraphCanvas(self._on_graph_node_clicked)
        graph_box.addWidget(self.graph_canvas)

        manual_card = QFrame()
        manual_card.setObjectName("Card")
        manual_box = QVBoxLayout(manual_card)
        manual_box.setContentsMargins(14, 10, 14, 10)
        manual_box.setSpacing(8)

        chap_bar = QHBoxLayout()
        lbl_man = QLabel("📘 OWNER'S MANUAL:")
        lbl_man.setObjectName("SectionHeader")
        chap_bar.addWidget(lbl_man)

        chapter_buttons = [
            ("overview", "1. Overview"),
            ("apps_in", "2. Shims"),
            ("mic_in", "3. Mic"),
            ("midi_in", "4. MIDI"),
            ("shm_ring", "5. SHM"),
            ("engine_core", "6. 192kHz"),
            ("dyn_eq", "7. Dyn EQ"),
            ("hw_out", "8. Out/Loop"),
            ("autosync", "9. Sync"),
            ("crosspc", "10. Cross-PC"),
            ("vid_broker", "11. Video"),
            ("mix_bt", "12. Mix/BT"),
        ]
        for c_key, c_label in chapter_buttons:
            btn = QPushButton(c_label)
            btn.clicked.connect(lambda _checked=False, k=c_key: self._select_manual_chapter(k))
            chap_bar.addWidget(btn)
        btn_sync_now = QPushButton("Sync Now")
        btn_sync_now.setObjectName("PrimaryBtn")
        btn_sync_now.clicked.connect(self._run_module_sync)
        chap_bar.addWidget(btn_sync_now)
        chap_bar.addStretch()
        manual_box.addLayout(chap_bar)

        self.manual_browser = QTextBrowser()
        self.manual_browser.setOpenExternalLinks(False)
        self.manual_browser.setObjectName("ManualBrowser")
        manual_box.addWidget(self.manual_browser)

        graph_splitter = QSplitter(Qt.Orientation.Vertical)
        graph_splitter.setChildrenCollapsible(False)
        graph_splitter.addWidget(graph_card)
        graph_splitter.addWidget(manual_card)
        graph_splitter.setStretchFactor(0, 13)
        graph_splitter.setStretchFactor(1, 7)
        graph_splitter.setSizes([500, 240])
        graph_layout.addWidget(graph_splitter, 1)

        intro_footer = QFrame()
        intro_footer.setObjectName("Card")
        foot_row = QHBoxLayout(intro_footer)
        foot_row.setContentsMargins(14, 8, 14, 8)
        foot_row.setSpacing(12)
        show_intro = bool(self.gui_settings.get("show_topology_intro", True))
        self.chk_skip_intro = QCheckBox("Don't show Topology Intro on startup")
        self.chk_skip_intro.setObjectName("EqEnableCheck")
        self.chk_skip_intro.setChecked(not show_intro)
        self.chk_skip_intro.toggled.connect(lambda ch: self._set_show_topology_intro(not ch))
        foot_row.addWidget(self.chk_skip_intro)

        self.lbl_intro_shortcuts = QLabel("")
        self.lbl_intro_shortcuts.setObjectName("BandDynLabel")
        foot_row.addWidget(self.lbl_intro_shortcuts, 1)

        self.btn_enter_studio = QPushButton("Enter Studio Mixer  ->")
        self.btn_enter_studio.setObjectName("PrimaryBtn")
        self.btn_enter_studio.setMinimumHeight(32)
        self.btn_enter_studio.clicked.connect(self._enter_studio_from_intro)
        foot_row.addWidget(self.btn_enter_studio)
        graph_layout.addWidget(intro_footer)

    def _build_advanced_options_window(self):
        self.adv_win = SalsaPopWindow(
            "Salsa — Advanced Options, DSP, Hardware Analog & Diagnostics",
            on_close=self._on_adv_win_close,
        )
        self.adv_win.resize(1120, 645)
        adv_root = QVBoxLayout(self.adv_win)
        adv_root.setContentsMargins(12, 10, 12, 10)
        adv_root.setSpacing(8)

        hdr_card = QFrame()
        hdr_card.setObjectName("Card")
        hc_lay = QHBoxLayout(hdr_card)
        hc_lay.setContentsMargins(12, 8, 12, 8)
        lbl_ah = QLabel("SALSA — ADVANCED OPTIONS, DSP, HARDWARE ANALOG & DIAGNOSTICS")
        lbl_ah.setObjectName("TitleBanner")
        hc_lay.addWidget(lbl_ah, 1)
        btn_close_adv = QPushButton("Close (Esc)")
        btn_close_adv.clicked.connect(self.adv_win.hide)
        hc_lay.addWidget(btn_close_adv)
        adv_root.addWidget(hdr_card)

        cols_row = QHBoxLayout()
        cols_row.setSpacing(10)
        left_col = QVBoxLayout()
        left_col.setSpacing(8)
        right_col = QVBoxLayout()
        right_col.setSpacing(8)

        # Card L1: Diagnostics, Standalone Debugger & Audio Services
        diag_card = QFrame()
        diag_card.setObjectName("Card")
        dc_lay = QVBoxLayout(diag_card)
        dc_lay.setContentsMargins(12, 8, 12, 8)
        dc_lay.setSpacing(6)
        lbl_diag_hdr = QLabel("DIAGNOSTICS, STANDALONE DEBUGGER & AUDIO SERVICES")
        lbl_diag_hdr.setObjectName("SectionHeader")
        dc_lay.addWidget(lbl_diag_hdr)

        g_diag = QGridLayout()
        g_diag.setSpacing(6)
        self.btn_open_debugger = QPushButton("Open Live Debugger Window (Ctrl+D)")
        self.btn_open_debugger.setObjectName("PrimaryBtn")
        self.btn_open_debugger.clicked.connect(self._open_standalone_debugger)
        g_diag.addWidget(self.btn_open_debugger, 0, 0, 1, 2)

        btn_bench = QPushButton("Run Latency Benchmark")
        btn_bench.clicked.connect(self._run_latency_benchmark)
        g_diag.addWidget(btn_bench, 1, 0)

        btn_audit = QPushButton("Run System Health Audit")
        btn_audit.clicked.connect(self._run_health_audit)
        g_diag.addWidget(btn_audit, 1, 1)

        btn_sync_adv = QPushButton("Sync Audio Modules Now")
        btn_sync_adv.clicked.connect(self._run_module_sync)
        g_diag.addWidget(btn_sync_adv, 2, 0)

        self.btn_midi_test = QPushButton("Send Test MIDI (C4)")
        self.btn_midi_test.clicked.connect(lambda: subprocess.Popen([CTL_BIN, "midi-send", "60", "115"]))
        g_diag.addWidget(self.btn_midi_test, 2, 1)

        self.btn_rst_engine = QPushButton("Restart Audio Engine")
        self.btn_rst_engine.clicked.connect(lambda: self._restart_systemd_service("lwasapi-audiodg.service"))
        g_diag.addWidget(self.btn_rst_engine, 3, 0)

        self.btn_rst_tray = QPushButton("Restart Tray Service")
        self.btn_rst_tray.clicked.connect(lambda: self._restart_systemd_service("lwasapi-tray.service"))
        g_diag.addWidget(self.btn_rst_tray, 3, 1)
        dc_lay.addLayout(g_diag)

        r_chips = QHBoxLayout()
        r_chips.setSpacing(6)
        self.lbl_chip_steam = QLabel("● Steam: ...")
        self.lbl_chip_launchers = QLabel("● Launchers: ...")
        self.lbl_chip_pacman = QLabel("● Pacman Hook: ...")
        for chip in (self.lbl_chip_steam, self.lbl_chip_launchers, self.lbl_chip_pacman):
            r_chips.addWidget(chip)
        r_chips.addStretch()
        dc_lay.addLayout(r_chips)

        r_hook_ctrl = QHBoxLayout()
        r_hook_ctrl.setSpacing(8)
        self.btn_toggle_pacman_hook = QPushButton("Toggle Pacman Hook (pkexec)")
        self.btn_toggle_pacman_hook.setObjectName("CompactBtn")
        self.btn_toggle_pacman_hook.setCheckable(True)
        self.btn_toggle_pacman_hook.clicked.connect(self._toggle_pacman_hook)
        r_hook_ctrl.addWidget(self.btn_toggle_pacman_hook, 1)

        self.chk_autosync_now = QCheckBox("Auto-Sync Watcher + Sync Now")
        self.chk_autosync_now.setToolTip(
            "Enable/disable lwasapi-module-watcher.path; switching ON runs Sync Audio Modules Now immediately"
        )
        self.chk_autosync_now.clicked.connect(self._on_autosync_watcher_clicked)
        r_hook_ctrl.addWidget(self.chk_autosync_now)
        dc_lay.addLayout(r_hook_ctrl)
        left_col.addWidget(diag_card)

        # Card L2: Engine Ownership, Startup & Appearance
        eng_card = QFrame()
        eng_card.setObjectName("Card")
        ec_lay = QVBoxLayout(eng_card)
        ec_lay.setContentsMargins(12, 8, 12, 8)
        ec_lay.setSpacing(6)
        lbl_eng_hdr = QLabel("ENGINE OWNERSHIP, STARTUP & APPEARANCE")
        lbl_eng_hdr.setObjectName("SectionHeader")
        ec_lay.addWidget(lbl_eng_hdr)

        self.btn_engine_toggle = QPushButton("Engine: SALSA NATIVE AUDIO (Revert to PW/WP)")
        self.btn_engine_toggle.setObjectName("PrimaryBtn")
        self.btn_engine_toggle.clicked.connect(self._toggle_engine_mode)
        ec_lay.addWidget(self.btn_engine_toggle)

        self.chk_show_intro_startup = QCheckBox("Show Auto-Graph Topology Intro window on startup")
        self.chk_show_intro_startup.setChecked(bool(self.gui_settings.get("show_topology_intro", True)))
        self.chk_show_intro_startup.toggled.connect(self._set_show_topology_intro)
        ec_lay.addWidget(self.chk_show_intro_startup)

        row_app = QHBoxLayout()
        self.btn_custom_theme = QPushButton("Open Custom Theme Studio...")
        self.btn_custom_theme.clicked.connect(self._open_custom_theme_editor)
        row_app.addWidget(self.btn_custom_theme, 1)
        btn_reset_all = QPushButton("Reset Settings to Safe Defaults")
        btn_reset_all.setObjectName("CompactBtn")
        btn_reset_all.clicked.connect(self._reset_all_safe_defaults)
        row_app.addWidget(btn_reset_all)
        ec_lay.addLayout(row_app)
        left_col.addWidget(eng_card)

        # Card R1: Advanced EQ & Spatial DSP
        eqadv_card = QFrame()
        eqadv_card.setObjectName("Card")
        qa_lay = QVBoxLayout(eqadv_card)
        qa_lay.setContentsMargins(12, 8, 12, 8)
        qa_lay.setSpacing(6)
        lbl_qa = QLabel("ADVANCED EQ & SPATIAL DSP")
        lbl_qa.setObjectName("SectionHeader")
        qa_lay.addWidget(lbl_qa)

        row_qa1 = QHBoxLayout()
        self.chk_limiter = QCheckBox("Soft-Knee Mastering Limiter")
        self.chk_limiter.setChecked(True)
        self.chk_limiter.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
        self.chk_limiter.toggled.connect(self._send_eq_update)
        row_qa1.addWidget(self.chk_limiter)

        self.chk_eq_per_dev = QCheckBox("Per-Device EQ")
        self.chk_eq_per_dev.setChecked(bool(self.gui_settings.get("eq_per_device", self.gui_settings.get("eq_per_device_enabled", False))))
        self.chk_eq_per_dev.setToolTip("Automatically save and restore a separate EQ curve for each output device")
        self.chk_eq_per_dev.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
        self.chk_eq_per_dev.toggled.connect(self._on_per_device_eq_toggled)
        row_qa1.addWidget(self.chk_eq_per_dev)

        btn_import_autoeq = QPushButton("Import AutoEQ .txt")
        btn_import_autoeq.setObjectName("CompactBtn")
        btn_import_autoeq.setToolTip("Import EqualizerAPO / AutoEQ / Oratory1990 ParametricEQ .txt profile")
        btn_import_autoeq.clicked.connect(self._import_autoeq_txt)
        row_qa1.addWidget(btn_import_autoeq)
        row_qa1.addStretch(1)
        qa_lay.addLayout(row_qa1)

        row_xf = QHBoxLayout()
        self.chk_crossfeed = QCheckBox("Bauer Crossfeed")
        self.chk_crossfeed.setToolTip("Optional BS2B-style stereo crossfeed for natural headphone staging")
        self.chk_crossfeed.toggled.connect(self._on_param_control_changed)
        row_xf.addWidget(self.chk_crossfeed)

        self.sld_crossfeed = QSlider(Qt.Orientation.Horizontal)
        self.sld_crossfeed.setRange(0, 100)
        self.sld_crossfeed.setValue(30)
        self.sld_crossfeed.setMaximumWidth(220)
        self.sld_crossfeed.valueChanged.connect(self._on_param_control_changed)
        row_xf.addWidget(self.sld_crossfeed, 1)

        self.lbl_crossfeed = QLabel("30%")
        row_xf.addWidget(self.lbl_crossfeed)

        self.cmb_crossfeed_cut = QComboBox()
        self.cmb_crossfeed_cut.addItem("650 Hz", 650)
        self.cmb_crossfeed_cut.addItem("700 Hz", 700)
        self.cmb_crossfeed_cut.addItem("850 Hz", 850)
        self.cmb_crossfeed_cut.setCurrentIndex(1)
        self.cmb_crossfeed_cut.setMaximumWidth(78)
        self.cmb_crossfeed_cut.currentIndexChanged.connect(self._on_param_control_changed)
        row_xf.addWidget(self.cmb_crossfeed_cut)
        qa_lay.addLayout(row_xf)
        right_col.addWidget(eqadv_card)

        # Card R2: Hardware Analog Stage
        hwa_card = QFrame()
        hwa_card.setObjectName("Card")
        ha_lay = QVBoxLayout(hwa_card)
        ha_lay.setContentsMargins(12, 8, 12, 8)
        ha_lay.setSpacing(6)
        lbl_ha = QLabel("HARDWARE ANALOG STAGE (STRIX / DISCRETE SOUND CARDS)")
        lbl_ha.setObjectName("SectionHeader")
        ha_lay.addWidget(lbl_ha)

        hw_analog_row1 = QHBoxLayout()
        hw_analog_row1.setSpacing(6)
        self.chk_hw_dac_unity = QCheckBox("HW DAC Unity")
        self.chk_hw_dac_unity.setChecked(True)
        self.chk_hw_dac_unity.setToolTip(
            "Drive ALSA hardware DAC mixer for lossless 0..100% volume (leaves Scarlett/BlueZ untouched)"
        )
        self.chk_hw_dac_unity.toggled.connect(self._on_hw_analog_cfg_changed)
        hw_analog_row1.addWidget(self.chk_hw_dac_unity)

        self.cmb_hw_amp = QComboBox()
        self.cmb_hw_amp.addItem("Amp: Auto (Preserve)", LWASAPI_HW_AMP_AUTO)
        self.cmb_hw_amp.addItem("Amp: Low (<=32Ω IEM)", LWASAPI_HW_AMP_LOW)
        self.cmb_hw_amp.addItem("Amp: Med (64-150Ω)", LWASAPI_HW_AMP_MED)
        self.cmb_hw_amp.addItem("Amp: High (600Ω TPA6120)", LWASAPI_HW_AMP_HIGH)
        self.cmb_hw_amp.currentIndexChanged.connect(self._on_hw_analog_cfg_changed)
        hw_analog_row1.addWidget(self.cmb_hw_amp)

        self.cmb_hw_route = QComboBox()
        self.cmb_hw_route.addItem("Route: Auto", LWASAPI_HW_ROUTE_AUTO)
        self.cmb_hw_route.addItem("Route: Headphones", LWASAPI_HW_ROUTE_HP)
        self.cmb_hw_route.addItem("Route: Line / Spk", LWASAPI_HW_ROUTE_SPK)
        self.cmb_hw_route.currentIndexChanged.connect(self._on_hw_analog_cfg_changed)
        hw_analog_row1.addWidget(self.cmb_hw_route)

        self.chk_hw_rails = QCheckBox("0dB Rails")
        self.chk_hw_rails.setChecked(False)
        self.chk_hw_rails.setToolTip("Unmute & normalize ALSA DAC playback switches to 0 dB")
        self.chk_hw_rails.toggled.connect(self._on_hw_analog_cfg_changed)
        hw_analog_row1.addWidget(self.chk_hw_rails)
        ha_lay.addLayout(hw_analog_row1)

        hw_analog_row2 = QHBoxLayout()
        hw_analog_row2.setSpacing(6)
        hw_analog_row2.addWidget(QLabel("Analog Drive:"))
        self.sld_hw_boost = QSlider(Qt.Orientation.Horizontal)
        self.sld_hw_boost.setRange(0, 24)
        self.sld_hw_boost.setValue(0)
        self.sld_hw_boost.setMaximumWidth(130)
        self.sld_hw_boost.setToolTip("Strix/Soundcard clean soft-knee analog drive boost (0.0 .. +12.0 dB)")
        self.sld_hw_boost.valueChanged.connect(self._on_hw_boost_slider_changed)
        self.sld_hw_boost.sliderReleased.connect(self._on_hw_analog_cfg_changed)
        hw_analog_row2.addWidget(self.sld_hw_boost)
        self.lbl_hw_boost = QLabel("+0.0 dB")
        self.lbl_hw_boost.setMinimumWidth(52)
        hw_analog_row2.addWidget(self.lbl_hw_boost)

        hw_analog_row2.addWidget(QLabel("Mic HW Boost:"))
        self.sld_hw_mic_boost = QSlider(Qt.Orientation.Horizontal)
        self.sld_hw_mic_boost.setRange(0, 24)
        self.sld_hw_mic_boost.setValue(0)
        self.sld_hw_mic_boost.setMaximumWidth(130)
        self.sld_hw_mic_boost.setToolTip("ALSA Mic Boost + clean capture preamp makeup (0 .. +24 dB)")
        self.sld_hw_mic_boost.valueChanged.connect(self._on_hw_mic_boost_slider_changed)
        self.sld_hw_mic_boost.sliderReleased.connect(self._on_hw_analog_cfg_changed)
        hw_analog_row2.addWidget(self.sld_hw_mic_boost)
        self.lbl_hw_mic_boost = QLabel("+0.0 dB")
        self.lbl_hw_mic_boost.setMinimumWidth(52)
        hw_analog_row2.addWidget(self.lbl_hw_mic_boost)

        self.btn_hw_analog_reset = QPushButton("Safe Defaults")
        self.btn_hw_analog_reset.setToolTip("Revert Hardware Analog Stage to non-invasive safe defaults")
        self.btn_hw_analog_reset.clicked.connect(self._on_hw_analog_reset)
        hw_analog_row2.addWidget(self.btn_hw_analog_reset)
        ha_lay.addLayout(hw_analog_row2)

        self.lbl_hw_analog_diag = QLabel("HW Stage: Probing...")
        self.lbl_hw_analog_diag.setObjectName("BandDynLabel")
        ha_lay.addWidget(self.lbl_hw_analog_diag)
        right_col.addWidget(hwa_card)

        right_col.addStretch(1)
        left_col.addStretch(1)
        cols_row.addLayout(left_col, 1)
        cols_row.addLayout(right_col, 1)
        adv_root.addLayout(cols_row)

        # Card Bottom: Customizable Keyboard Shortcuts
        sc_card = QFrame()
        sc_card.setObjectName("Card")
        sc_lay = QVBoxLayout(sc_card)
        sc_lay.setContentsMargins(12, 8, 12, 8)
        sc_lay.setSpacing(6)
        sc_hdr_row = QHBoxLayout()
        lbl_sc_h = QLabel("CUSTOMIZABLE KEYBOARD SHORTCUTS")
        lbl_sc_h.setObjectName("SectionHeader")
        sc_hdr_row.addWidget(lbl_sc_h)
        self.lbl_shortcut_status = QLabel("Click any box and press a key combo to customize (saved automatically)")
        self.lbl_shortcut_status.setObjectName("BandDynLabel")
        sc_hdr_row.addWidget(self.lbl_shortcut_status, 1)
        btn_sc_reset = QPushButton("Restore Defaults")
        btn_sc_reset.setObjectName("CompactBtn")
        btn_sc_reset.clicked.connect(self._reset_default_shortcuts)
        sc_hdr_row.addWidget(btn_sc_reset)
        sc_lay.addLayout(sc_hdr_row)

        sc_grid = QGridLayout()
        sc_grid.setHorizontalSpacing(14)
        sc_grid.setVerticalSpacing(4)
        self.shortcut_edits = {}
        self._shortcut_row_labels = {}
        sc_map = self._get_shortcut_map()
        half_n = (len(DEFAULT_SHORTCUTS) + 1) // 2
        for idx_s, (act_id, act_label, _def_seq) in enumerate(DEFAULT_SHORTCUTS):
            col_blk = 0 if idx_s < half_n else 2
            row_blk = idx_s if idx_s < half_n else idx_s - half_n
            lbl_act = QLabel(f"{act_label}:")
            self._shortcut_row_labels[act_id] = lbl_act
            kedit = QKeySequenceEdit(QKeySequence(sc_map.get(act_id, "")))
            if hasattr(kedit, "setMaximumSequenceLength"):
                kedit.setMaximumSequenceLength(1)
            if hasattr(kedit, "setClearButtonEnabled"):
                kedit.setClearButtonEnabled(True)
            kedit.setFixedWidth(155)
            kedit.keySequenceChanged.connect(lambda seq, k=act_id: self._on_shortcut_edited(k, seq))
            self.shortcut_edits[act_id] = kedit
            sc_grid.addWidget(lbl_act, row_blk, col_blk)
            sc_grid.addWidget(kedit, row_blk, col_blk + 1)
        sc_grid.setColumnStretch(0, 1)
        sc_grid.setColumnStretch(2, 1)
        sc_lay.addLayout(sc_grid)
        adv_root.addWidget(sc_card)
        self._refresh_shortcut_row_labels()
        self._refresh_adv_integration_status()

    def _get_shortcut_map(self) -> dict:
        raw = self.gui_settings.get("shortcuts")
        out = {k: d for k, _lbl, d in DEFAULT_SHORTCUTS}
        if isinstance(raw, dict):
            for k, _lbl, _d in DEFAULT_SHORTCUTS:
                if k in raw and isinstance(raw[k], str):
                    out[k] = raw[k].strip()
        return out

    def _setup_shortcuts(self):
        self._shortcut_objs = []
        sc_map = self._get_shortcut_map()
        actions = {
            "advanced": self._open_advanced_options,
            "topology": self._toggle_topology_intro,
            "debugger": self._open_standalone_debugger,
            "mute_spk": lambda: subprocess.Popen([CTL_BIN, "mute", "toggle"]),
            "mute_mic": lambda: subprocess.Popen([CTL_BIN, "mic-mute", "toggle"]),
            "toggle_eq": self.chk_eq_enable.toggle,
            "toggle_mic_apo": self.chk_mic_apo.toggle,
            "vol_up": lambda: self._adjust_vol_step("spk", self._vol_step_pct),
            "vol_down": lambda: self._adjust_vol_step("spk", -self._vol_step_pct),
            "mic_up": lambda: self._adjust_vol_step("mic", self._vol_step_pct),
            "mic_down": lambda: self._adjust_vol_step("mic", -self._vol_step_pct),
            "bt_reset": self._on_bt_reset_clicked,
            "refresh": self._refresh_all_now,
            "quit": self.close,
        }
        for act_id, _lbl, _def in DEFAULT_SHORTCUTS:
            seq_txt = sc_map.get(act_id, "")
            fn = actions.get(act_id)
            if seq_txt and fn is not None:
                sc = QShortcut(QKeySequence(seq_txt), self)
                sc.setContext(Qt.ShortcutContext.WindowShortcut)
                sc.activated.connect(fn)
                self._shortcut_objs.append(sc)
        if getattr(self, "intro_win", None) is not None:
            for k_enter in ("Return", "Enter", "Escape"):
                sc_i = QShortcut(QKeySequence(k_enter), self.intro_win)
                sc_i.setContext(Qt.ShortcutContext.WindowShortcut)
                sc_i.activated.connect(self._enter_studio_from_intro)
                self._shortcut_objs.append(sc_i)
            for act_id, fn in (("topology", self._toggle_topology_intro), ("advanced", self._open_advanced_options), ("debugger", self._open_standalone_debugger), ("quit", self.close)):
                seq_txt = sc_map.get(act_id, "")
                if seq_txt:
                    sc_iw = QShortcut(QKeySequence(seq_txt), self.intro_win)
                    sc_iw.setContext(Qt.ShortcutContext.WindowShortcut)
                    sc_iw.activated.connect(fn)
                    self._shortcut_objs.append(sc_iw)
        if getattr(self, "adv_win", None) is not None:
            sc_esc = QShortcut(QKeySequence("Escape"), self.adv_win)
            sc_esc.setContext(Qt.ShortcutContext.WindowShortcut)
            sc_esc.activated.connect(self.adv_win.hide)
            self._shortcut_objs.append(sc_esc)
            for act_id, fn in (("advanced", self.adv_win.hide), ("topology", self._open_topology_intro), ("debugger", self._open_standalone_debugger), ("refresh", self._refresh_all_now), ("quit", self.close)):
                seq_txt = sc_map.get(act_id, "")
                if seq_txt:
                    sc_aw = QShortcut(QKeySequence(seq_txt), self.adv_win)
                    sc_aw.setContext(Qt.ShortcutContext.WindowShortcut)
                    sc_aw.activated.connect(fn)
                    self._shortcut_objs.append(sc_aw)
        self._update_shortcut_hints(sc_map)

    def _rebuild_shortcuts(self):
        for sc in getattr(self, "_shortcut_objs", []):
            try:
                sc.setEnabled(False)
                sc.deleteLater()
            except Exception:
                pass
        self._shortcut_objs = []
        self._setup_shortcuts()

    def _update_shortcut_hints(self, sc_map: dict):
        top_k = sc_map.get("topology") or "Unbound"
        adv_k = sc_map.get("advanced") or "Unbound"
        dbg_k = sc_map.get("debugger") or "Unbound"
        if hasattr(self, "btn_open_topology"):
            self.btn_open_topology.setText(f"🕸️ Topology ({top_k})" if sc_map.get("topology") else "🕸️ Topology")
            self.btn_open_topology.setToolTip(f"Open Auto-Graph Signal Topology & Owner's Manual window ({top_k})")
        if hasattr(self, "btn_open_advanced"):
            self.btn_open_advanced.setText(f"⚙️ Advanced Options ({adv_k})" if sc_map.get("advanced") else "⚙️ Advanced Options")
            self.btn_open_advanced.setToolTip(f"Open Advanced Options, Standalone Live Debugger & Hardware Controls ({adv_k})")
        if hasattr(self, "btn_open_debugger"):
            self.btn_open_debugger.setText(f"Open Live Debugger Window ({dbg_k})" if sc_map.get("debugger") else "Open Live Debugger Window")
        if hasattr(self, "chk_eq_enable"):
            self.chk_eq_enable.setToolTip(f"Toggle 10-Band Dynamic EQ APO ({sc_map.get('toggle_eq') or 'Unbound'})")
        if hasattr(self, "chk_mic_apo"):
            self.chk_mic_apo.setToolTip(f"Toggle Capture Mic APO ({sc_map.get('toggle_mic_apo') or 'Unbound'})")
        if hasattr(self, "lbl_intro_shortcuts"):
            self.lbl_intro_shortcuts.setText(
                f"Shortcuts: Enter / Esc -> Studio   •   {top_k} -> Topology   •   {adv_k} -> Advanced Options"
            )

    def _on_shortcut_edited(self, act_id: str, seq: QKeySequence):
        new_txt = seq.toString(QKeySequence.SequenceFormat.PortableText).strip()
        sc_map = self._get_shortcut_map()
        label_map = {k: lbl for k, lbl, _d in DEFAULT_SHORTCUTS}
        conflict_lbl = None
        if new_txt:
            for other_k, other_seq in list(sc_map.items()):
                if other_k != act_id and other_seq.lower() == new_txt.lower():
                    sc_map[other_k] = ""
                    conflict_lbl = label_map.get(other_k, other_k)
                    ed_other = self.shortcut_edits.get(other_k)
                    if ed_other is not None:
                        ed_other.blockSignals(True)
                        ed_other.clear()
                        ed_other.blockSignals(False)
        sc_map[act_id] = new_txt
        self.gui_settings["shortcuts"] = sc_map
        save_gui_settings(self.gui_settings)
        self._gui_settings_mtime = self._read_gui_settings_mtime()
        self._rebuild_shortcuts()
        if hasattr(self, "lbl_shortcut_status"):
            lbl_name = label_map.get(act_id, act_id)
            if not new_txt:
                self.lbl_shortcut_status.setText(f"Cleared shortcut for '{lbl_name}'.")
            elif conflict_lbl:
                self.lbl_shortcut_status.setText(f"Bound '{lbl_name}' -> {new_txt} (cleared duplicate on '{conflict_lbl}').")
            else:
                self.lbl_shortcut_status.setText(f"Saved '{lbl_name}' -> {new_txt}.")

    def _reset_default_shortcuts(self):
        defaults = {k: d for k, _lbl, d in DEFAULT_SHORTCUTS}
        self.gui_settings["shortcuts"] = dict(defaults)
        save_gui_settings(self.gui_settings)
        self._gui_settings_mtime = self._read_gui_settings_mtime()
        for act_id, seq_txt in defaults.items():
            ed = self.shortcut_edits.get(act_id)
            if ed is not None:
                ed.blockSignals(True)
                ed.setKeySequence(QKeySequence(seq_txt))
                ed.blockSignals(False)
        self._rebuild_shortcuts()
        if hasattr(self, "lbl_shortcut_status"):
            self.lbl_shortcut_status.setText("Restored all keyboard shortcuts to defaults.")

    def _refresh_shortcut_row_labels(self):
        step = getattr(self, "_vol_step_pct", DEFAULT_VOL_STEP_PCT)
        dyn_labels = {
            "vol_up": f"Speaker Volume +{step}%:",
            "vol_down": f"Speaker Volume -{step}%:",
            "mic_up": f"Mic Gain +{step}%:",
            "mic_down": f"Mic Gain -{step}%:",
        }
        for act_id, txt in dyn_labels.items():
            lbl = self._shortcut_row_labels.get(act_id)
            if lbl is not None:
                lbl.setText(txt)

    def _on_vol_step_changed(self, _idx: int = 0):
        if not hasattr(self, "cmb_vol_step"):
            return
        step = self.cmb_vol_step.currentData()
        self._apply_vol_step(step, save=True)

    def _apply_vol_step(self, step_pct, save: bool = True):
        step = normalize_vol_step_pct(step_pct)
        self._vol_step_pct = step
        if hasattr(self, "cmb_vol_step"):
            idx = self.cmb_vol_step.findData(step)
            if idx >= 0 and self.cmb_vol_step.currentIndex() != idx:
                self.cmb_vol_step.blockSignals(True)
                self.cmb_vol_step.setCurrentIndex(idx)
                self.cmb_vol_step.blockSignals(False)
        for sld in (getattr(self, "sld_spk", None), getattr(self, "sld_mic", None)):
            if sld is not None and hasattr(sld, "sync_step"):
                sld.sync_step()
        for meta in getattr(self, "_sess_widgets", {}).values():
            w_sld = meta.get("sld")
            if w_sld is not None and hasattr(w_sld, "sync_step"):
                w_sld.sync_step()
        self._refresh_shortcut_row_labels()
        if save:
            self.gui_settings["volume_step_pct"] = step
            save_gui_settings(self.gui_settings)
            self._gui_settings_mtime = self._read_gui_settings_mtime()

    def _adjust_vol_step(self, kind: str, delta: int):
        sld = self.sld_mic if kind == "mic" else self.sld_spk
        new_v = max(0, min(150, int(sld.value()) + int(delta)))
        sld.setValue(new_v)
        self._flush_vol(kind)

    def _refresh_all_now(self):
        self._last_dev_sig = None
        self._poll_engine(initial=True)
        if getattr(self, "adv_win", None) is not None and self.adv_win.isVisible():
            self._refresh_adv_integration_status()

    def _set_show_topology_intro(self, enabled: bool):
        enabled = bool(enabled)
        self.gui_settings["show_topology_intro"] = enabled
        save_gui_settings(self.gui_settings)
        self._gui_settings_mtime = self._read_gui_settings_mtime()
        if hasattr(self, "chk_skip_intro"):
            self.chk_skip_intro.blockSignals(True)
            self.chk_skip_intro.setChecked(not enabled)
            self.chk_skip_intro.blockSignals(False)
        if hasattr(self, "chk_show_intro_startup"):
            self.chk_show_intro_startup.blockSignals(True)
            self.chk_show_intro_startup.setChecked(enabled)
            self.chk_show_intro_startup.blockSignals(False)

    def _open_topology_intro(self):
        if getattr(self, "intro_win", None) is None:
            return
        if hasattr(self, "btn_enter_studio"):
            self.btn_enter_studio.setText(
                "Return to Studio Mixer  ->" if self._studio_entered else "Enter Studio Mixer  ->"
            )
        self.intro_win.show()
        self.intro_win.raise_()
        self.intro_win.activateWindow()
        self._sync_graph_anim_timer()
        self._poll_engine(initial=False)
        self.graph_canvas.update()

    def _toggle_topology_intro(self):
        if getattr(self, "intro_win", None) is None:
            return
        if self.intro_win.isVisible() and self._studio_entered:
            self.intro_win.hide()
            self._sync_graph_anim_timer()
            self.raise_()
            self.activateWindow()
        else:
            self._open_topology_intro()

    def _enter_studio_from_intro(self):
        self._studio_entered = True
        self.show()
        self.raise_()
        self.activateWindow()
        if getattr(self, "intro_win", None) is not None:
            self.intro_win.hide()
        self._sync_graph_anim_timer()
        self._poll_engine(initial=False)
        self.canvas.update()

    def _on_intro_win_close(self, event):
        if self._closing_all:
            return True
        if self._studio_entered or self.isVisible():
            self.intro_win.hide()
            self._sync_graph_anim_timer()
            event.ignore()
            return False
        return True

    def _open_advanced_options(self):
        if getattr(self, "adv_win", None) is None:
            return
        self._refresh_adv_integration_status()
        self.adv_win.show()
        self.adv_win.raise_()
        self.adv_win.activateWindow()

    def _on_adv_win_close(self, event):
        if self._closing_all:
            return True
        self.adv_win.hide()
        event.ignore()
        return False

    def _open_standalone_debugger(self):
        if not os.path.exists(DEBUG_SCRIPT):
            QMessageBox.warning(self.adv_win or self, "Debugger Missing", f"Could not find {DEBUG_SCRIPT}")
            return
        try:
            subprocess.Popen(
                [sys.executable, DEBUG_SCRIPT],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        except Exception as e:
            QMessageBox.warning(self.adv_win or self, "Debugger Launch Error", f"Failed to launch standalone debugger:\n{e}")

    def _run_health_audit(self):
        parent_w = self.adv_win if (self.adv_win and self.adv_win.isVisible()) else self
        res = subprocess.run([sys.executable, DEBUG_SCRIPT, "--audit"], capture_output=True, text=True, timeout=15)
        t = self.current_theme
        dlg = QDialog(parent_w)
        dlg.setWindowTitle("Salsa LWASAPI System Health & Process Audit")
        dlg.resize(840, 520)
        lay = QVBoxLayout(dlg)
        txt = QTextEdit(dlg)
        txt.setReadOnly(True)
        txt.setStyleSheet(
            f"background-color: {t['bg_deep']}; color: {t['accent_primary']}; "
            "font-family: Monospace; font-size: 11px;"
        )
        txt.setPlainText((res.stdout or "") + ("\n" + res.stderr if res.stderr else ""))
        lay.addWidget(txt)
        btn_close = QPushButton("Close Audit Report")
        btn_close.clicked.connect(dlg.accept)
        lay.addWidget(btn_close)
        dlg.exec()

    def _restart_systemd_service(self, unit_name: str):
        if self._svc_restart_proc is not None and self._svc_restart_proc.state() != QProcess.ProcessState.NotRunning:
            return
        btn = self.btn_rst_engine if "audiodg" in unit_name else self.btn_rst_tray
        orig_txt = btn.text()
        btn.setEnabled(False)
        btn.setText(f"Restarting {unit_name.split('.')[0]}...")
        proc = QProcess(self)
        proc.finished.connect(lambda _code, _st, b=btn, ot=orig_txt, p=proc: self._on_service_restarted(b, ot, p))
        self._svc_restart_proc = proc
        proc.start("systemctl", ["--user", "restart", unit_name])

    def _on_service_restarted(self, btn: QPushButton, orig_txt: str, proc: QProcess):
        try:
            proc.deleteLater()
        except Exception:
            pass
        self._svc_restart_proc = None
        btn.setEnabled(True)
        btn.setText(orig_txt)
        self._last_dev_sig = None
        QTimer.singleShot(300, lambda: self._poll_engine(initial=True))

    def _is_pacman_hook_installed(self) -> bool:
        return (
            os.path.exists("/etc/pacman.d/hooks/99-salsa-autosync.hook")
            or os.path.exists("/var/lib/salsa-autosync/libpulse-updated")
        )

    def _is_watcher_enabled(self) -> bool:
        try:
            res = subprocess.run(
                ["systemctl", "--user", "is-active", "lwasapi-module-watcher.path"],
                capture_output=True,
                text=True,
                timeout=2,
            )
            return res.returncode == 0 and res.stdout.strip() == "active"
        except Exception:
            return False

    def _set_adv_chip(self, lbl: QLabel, text: str, active: bool | None):
        t = self.current_theme
        if active is True:
            fg, bd = t["accent_primary"], t["border_control"]
        elif active is False:
            fg, bd = t["accent_warn"], t["accent_warn"]
        else:
            fg, bd = t["text_muted"], t["border_control"]
        lbl.setText(text)
        lbl.setStyleSheet(
            f"background-color: {t['bg_deep']}; color: {fg}; border: 1px solid {bd}; "
            "border-radius: 5px; padding: 2px 7px; font-weight: 700; font-size: 11px;"
        )

    def _refresh_adv_integration_status(self):
        if not hasattr(self, "lbl_chip_pacman"):
            return
        steam_bin = os.path.exists(os.path.expanduser("~/.local/bin/salsa-steam-launch"))
        desktop_ok = os.path.exists(os.path.expanduser("~/.local/share/applications/salsa-lwasapi.desktop"))
        has_pacman = os.path.exists("/usr/bin/pacman") and os.path.isdir("/etc/pacman.d")
        hook_ok = self._is_pacman_hook_installed()
        watcher_ok = self._is_watcher_enabled()

        self._set_adv_chip(
            self.lbl_chip_steam,
            f"● Steam: {'READY' if steam_bin else 'Off'}",
            True if steam_bin else None,
        )
        self._set_adv_chip(
            self.lbl_chip_launchers,
            f"● Launchers: {'OK' if desktop_ok else 'Missing'}",
            True if desktop_ok else False,
        )
        if not has_pacman:
            self._set_adv_chip(self.lbl_chip_pacman, "● Pacman Hook: N/A", None)
        else:
            self._set_adv_chip(
                self.lbl_chip_pacman,
                f"● Pacman Hook: {'ACTIVE' if hook_ok else 'OFF'}",
                True if hook_ok else False,
            )

        if hasattr(self, "btn_toggle_pacman_hook"):
            busy = self._hook_proc is not None and self._hook_proc.state() != QProcess.ProcessState.NotRunning
            if not busy:
                self.btn_toggle_pacman_hook.blockSignals(True)
                self.btn_toggle_pacman_hook.setChecked(bool(hook_ok))
                self.btn_toggle_pacman_hook.setEnabled(has_pacman)
                self.btn_toggle_pacman_hook.setText(
                    "Pacman Hook: ON  (Click to Disable)" if hook_ok else "Toggle Pacman Hook (pkexec)"
                )
                self.btn_toggle_pacman_hook.setToolTip(
                    "Install or remove /etc/pacman.d/hooks/99-salsa-autosync.hook via graphical pkexec"
                    if has_pacman
                    else "Pacman ALPM hook applies to Arch/CachyOS systems; Path Watcher handles updates here"
                )
                self.btn_toggle_pacman_hook.blockSignals(False)

        if hasattr(self, "chk_autosync_now"):
            self.chk_autosync_now.blockSignals(True)
            self.chk_autosync_now.setChecked(watcher_ok)
            self.chk_autosync_now.setToolTip(
                f"Path Watcher: {'ACTIVE' if watcher_ok else 'OFF'} (lwasapi-module-watcher.path)\n"
                "Toggling ON enables the watcher and immediately runs Sync Audio Modules Now."
            )
            self.chk_autosync_now.blockSignals(False)

    def _toggle_pacman_hook(self):
        if self._hook_proc is not None and self._hook_proc.state() != QProcess.ProcessState.NotRunning:
            return
        install_sh = os.path.join(BASE_DIR, "install-salsa.sh")
        if not os.path.exists(install_sh):
            QMessageBox.warning(self.adv_win or self, "Installer Missing", f"Could not find {install_sh}")
            self._refresh_adv_integration_status()
            return
        installed = self._is_pacman_hook_installed()
        flag = "--remove-pacman-hook" if installed else "--install-pacman-hook"
        self.btn_toggle_pacman_hook.setEnabled(False)
        self.btn_toggle_pacman_hook.setText("Removing Hook (pkexec)..." if installed else "Installing Hook (pkexec)...")
        proc = QProcess(self)
        proc.finished.connect(lambda code, _st, fl=flag, p=proc: self._on_pacman_hook_finished(code, fl, p))
        self._hook_proc = proc
        proc.start(install_sh, [flag])

    def _on_pacman_hook_finished(self, exit_code: int, flag: str, proc: QProcess):
        out = ""
        err = ""
        try:
            out = bytes(proc.readAllStandardOutput()).decode("utf-8", "replace").strip()
            err = bytes(proc.readAllStandardError()).decode("utf-8", "replace").strip()
            proc.deleteLater()
        except Exception:
            pass
        self._hook_proc = None
        if exit_code == 0 and flag == "--install-pacman-hook":
            try:
                subprocess.run(["systemctl", "--user", "restart", "lwasapi-module-watcher.path"], check=False, timeout=4)
            except Exception:
                pass
            if hasattr(self, "chk_autosync_now") and self.chk_autosync_now.isChecked():
                self._refresh_adv_integration_status()
                self._run_module_sync()
                return
        elif exit_code != 0:
            msg = err or out or f"exit code {exit_code}"
            QMessageBox.warning(
                self.adv_win or self,
                "Pacman Hook Update Cancelled / Failed",
                f"Action ({flag}) did not complete:\n{msg}",
            )
        self._refresh_adv_integration_status()

    def _on_autosync_watcher_clicked(self, checked: bool):
        action = "enable" if checked else "disable"
        try:
            subprocess.run(
                ["systemctl", "--user", action, "--now", "lwasapi-module-watcher.path"],
                capture_output=True,
                text=True,
                timeout=4,
            )
        except Exception:
            pass
        self._refresh_adv_integration_status()
        if checked:
            self._run_module_sync()

    def _reset_all_safe_defaults(self):
        parent_w = self.adv_win if (self.adv_win and self.adv_win.isVisible()) else self
        reply = QMessageBox.question(
            parent_w,
            "Reset Settings to Safe Defaults",
            "Reset Dynamic EQ flat (0 dB), bypass Crossfeed, reset Hardware Analog Stage to non-invasive safe defaults, and re-enable the Topology Intro window on startup?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No,
        )
        if reply != QMessageBox.StandardButton.Yes:
            return
        self._on_reset_flat_clicked()
        self._on_hw_analog_reset()
        self._set_show_topology_intro(True)

    # =========================================================================
    # Event Handlers & Real-Time IPC Sync
    # =========================================================================

    def _on_graph_node_clicked(self, key: str):
        chapter_map = {
            "midihub": "midi_in",
            "capt_out": "hw_out",
        }
        target_key = chapter_map.get(key, key)
        if target_key in OWNERS_MANUAL_CHAPTER_DEFS:
            self.current_manual_key = target_key
            self.manual_browser.setHtml(render_manual_chapter(target_key, self.current_theme))

    def _select_manual_chapter(self, key: str):
        self.graph_canvas.selected_key = key
        self.graph_canvas.update()
        if key in OWNERS_MANUAL_CHAPTER_DEFS:
            self.current_manual_key = key
            self.manual_browser.setHtml(render_manual_chapter(key, self.current_theme))

    def _on_clock_changed(self):
        if self.updating_ui or not self.lib:
            return
        rate = int(self.cmb_rate.currentData())
        period = int(self.cmb_period.currentData())
        bits = int(self.cmb_bits.currentData())
        self._last_clock_req = (rate, period, bits, time.monotonic() + 4.0)
        self._last_clock_policy_sig = None
        self.lib.LWASAPI_SetEngineConfig(rate, period, bits)
        QTimer.singleShot(60, self._poll_engine)

    def _on_cap_rate_changed(self):
        if self.updating_ui or not self.lib or not hasattr(self.lib, "LWASAPI_SetCaptureSampleRate"):
            return
        cap_rate = int(self.cmb_cap_rate.currentData() or 0)
        self._last_cap_rate_pref = cap_rate
        self._cap_rate_hold_until = time.monotonic() + 2.0
        self._last_cap_rate_sig = None
        self.lib.LWASAPI_SetCaptureSampleRate(cap_rate)
        QTimer.singleShot(60, self._poll_engine)

    def _any_eq_slider_down(self) -> bool:
        return (
            self.sld_preamp.isSliderDown()
            or self.sld_thresh.isSliderDown()
            or self.sld_ratio.isSliderDown()
            or self.sld_crossfeed.isSliderDown()
            or any(s.isSliderDown() for s in self.band_sliders)
        )

    def _has_saved_eq_settings(self) -> bool:
        return any(k in self.gui_settings for k in ("eq_preset", "eq_dynamic_mode", "eq_config"))

    def _eq_cfg_signature(self, cfg, param_ext=None) -> tuple:
        pname = cfg.preset_name.decode("utf-8", "ignore").strip() or CUSTOM_PRESET_NAME
        ext = param_ext if param_ext is not None else self.current_param_eq
        return (
            int(cfg.enabled),
            int(cfg.dynamic_mode),
            round(float(cfg.preamp_db), 1),
            round(float(cfg.dyn_threshold_db), 1),
            round(float(cfg.dyn_ratio), 1),
            int(cfg.limiter_enabled),
            pname,
            tuple(round(float(cfg.band_gains_db[i]), 1) for i in range(LWASAPI_EQ_BANDS)),
            tuple(round(float(cfg.band_q[i]), 2) for i in range(LWASAPI_EQ_BANDS)),
            tuple(round(float(ext.band_freq_hz[i]), 1) for i in range(LWASAPI_EQ_BANDS)),
            tuple(int(ext.band_type[i]) for i in range(LWASAPI_EQ_BANDS)),
            int(ext.crossfeed_enabled),
            round(float(ext.crossfeed_level), 2),
            int(ext.crossfeed_cutoff_hz),
        )

    def _extract_eq_dict_from_cfg(self, cfg, param_ext=None) -> dict:
        pname = cfg.preset_name.decode("utf-8", "ignore").strip() or CUSTOM_PRESET_NAME
        ext = param_ext if param_ext is not None else self.current_param_eq
        return {
            "enabled": bool(cfg.enabled),
            "dynamic_mode": int(cfg.dynamic_mode),
            "preamp_db": round(float(cfg.preamp_db), 1),
            "dyn_threshold_db": round(float(cfg.dyn_threshold_db), 1),
            "dyn_ratio": round(float(cfg.dyn_ratio), 1),
            "limiter_enabled": bool(cfg.limiter_enabled),
            "preset_name": pname,
            "band_gains_db": [round(float(cfg.band_gains_db[i]), 1) for i in range(LWASAPI_EQ_BANDS)],
            "band_q": [round(float(cfg.band_q[i]) if cfg.band_q[i] > 0.05 else 1.414, 3) for i in range(LWASAPI_EQ_BANDS)],
            "band_freq_hz": [round(float(ext.band_freq_hz[i]) if ext.band_freq_hz[i] > 1.0 else DEFAULT_BAND_FREQS_HZ[i], 1) for i in range(LWASAPI_EQ_BANDS)],
            "band_type": [int(ext.band_type[i]) if ext.band_type[i] <= 2 else 0 for i in range(LWASAPI_EQ_BANDS)],
            "crossfeed_enabled": bool(ext.crossfeed_enabled),
            "crossfeed_level": round(float(ext.crossfeed_level), 2),
            "crossfeed_cutoff_hz": int(ext.crossfeed_cutoff_hz or 700),
        }

    def _set_preset_combo_value(self, preset_name: str) -> None:
        target = (preset_name or "").strip() or CUSTOM_PRESET_NAME
        self.cmb_preset.blockSignals(True)
        custom_idx = self.cmb_preset.findText(CUSTOM_PRESET_NAME)
        if target in PRESETS:
            if custom_idx >= 0:
                self.cmb_preset.removeItem(custom_idx)
            self.cmb_preset.setCurrentText(target)
        else:
            existing_idx = self.cmb_preset.findText(target)
            if existing_idx < 0:
                if custom_idx >= 0 and target != CUSTOM_PRESET_NAME:
                    self.cmb_preset.setItemText(custom_idx, target)
                    existing_idx = custom_idx
                else:
                    self.cmb_preset.addItem(target)
                    existing_idx = self.cmb_preset.count() - 1
            self.cmb_preset.setCurrentIndex(existing_idx)
        self.cmb_preset.blockSignals(False)

    def _update_saved_eq_badges(self, preset_name: str, dynamic_mode: int) -> None:
        pname = (preset_name or "").strip() or CUSTOM_PRESET_NAME
        mode = int(dynamic_mode) if int(dynamic_mode) in DYN_MODE_LABELS else 1
        short_mode = DYN_MODE_SHORT_LABELS.get(mode, DYN_MODE_SHORT_LABELS[1])
        full_mode = DYN_MODE_LABELS.get(mode, DYN_MODE_LABELS[1])
        if hasattr(self, "lbl_saved_preset"):
            self.lbl_saved_preset.set_saved_text(f"Saved: {pname}", tooltip=f"Saved Preset: {pname}")
        if hasattr(self, "lbl_saved_dyn_mode"):
            self.lbl_saved_dyn_mode.set_saved_text(
                f"Saved: {short_mode}",
                tooltip=f"Saved Dynamic Mode: {full_mode}",
            )

    def _persist_eq_settings(self, eq_dict: dict) -> None:
        pname = (eq_dict.get("preset_name") or "").strip() or CUSTOM_PRESET_NAME
        mode = int(eq_dict.get("dynamic_mode", 1))
        self._update_saved_eq_badges(pname, mode)
        per_dev_on = hasattr(self, "chk_eq_per_dev") and self.chk_eq_per_dev.isChecked()
        dev_id = (getattr(self, "_active_playback_id", "") or "").strip()[:64]
        profiles = self.gui_settings.get("eq_device_profiles")
        if not isinstance(profiles, dict):
            profiles = {}
        dev_dirty = bool(per_dev_on and dev_id and profiles.get(dev_id) != eq_dict)
        if (
            self.gui_settings.get("eq_preset") == pname
            and self.gui_settings.get("eq_dynamic_mode") == mode
            and self.gui_settings.get("eq_config") == eq_dict
            and not dev_dirty
        ):
            return
        self.gui_settings["eq_preset"] = pname
        self.gui_settings["eq_dynamic_mode"] = mode
        self.gui_settings["eq_config"] = copy.deepcopy(eq_dict)
        if per_dev_on and dev_id:
            profiles[dev_id] = copy.deepcopy(eq_dict)
            self.gui_settings["eq_device_profiles"] = profiles
        save_gui_settings(self.gui_settings)

    def _on_per_device_eq_toggled(self, checked: bool) -> None:
        self.gui_settings["eq_per_device"] = bool(checked)
        if checked:
            dev_id = (getattr(self, "_active_playback_id", "") or "").strip()[:64]
            if dev_id:
                profiles = self.gui_settings.setdefault("eq_device_profiles", {})
                if isinstance(profiles, dict):
                    profiles[dev_id] = self._extract_eq_dict_from_cfg(self.current_eq)
        save_gui_settings(self.gui_settings)

    def _maybe_switch_per_device_eq(self, new_dev_id: str, initial: bool = False) -> None:
        new_id = (new_dev_id or "").strip()[:64]
        if not new_id or new_id == "auto":
            return
        prev_id = (getattr(self, "_active_playback_id", "") or "").strip()[:64]
        per_dev_on = hasattr(self, "chk_eq_per_dev") and self.chk_eq_per_dev.isChecked()
        if initial:
            self._active_playback_id = new_id
            if per_dev_on:
                profiles = self.gui_settings.get("eq_device_profiles")
                if isinstance(profiles, dict) and isinstance(profiles.get(new_id), dict):
                    self._apply_eq_dict_to_ui(profiles[new_id])
                    self._push_eq_cfg_to_engine(self.current_eq)
                    self._persist_eq_settings(self._extract_eq_dict_from_cfg(self.current_eq))
            return
        if new_id == prev_id:
            return
        if per_dev_on:
            profiles = self.gui_settings.setdefault("eq_device_profiles", {})
            if isinstance(profiles, dict):
                if prev_id:
                    profiles[prev_id] = self._extract_eq_dict_from_cfg(self.current_eq)
                self._active_playback_id = new_id
                saved_dev_eq = profiles.get(new_id)
                if isinstance(saved_dev_eq, dict):
                    self._apply_eq_dict_to_ui(saved_dev_eq)
                    self._push_eq_cfg_to_engine(self.current_eq)
                self._persist_eq_settings(self._extract_eq_dict_from_cfg(self.current_eq))
                return
        self._active_playback_id = new_id

    def _apply_eq_dict_to_ui(self, eq_dict: dict) -> None:
        pname = (eq_dict.get("preset_name") or "").strip() or CUSTOM_PRESET_NAME
        mode = int(eq_dict.get("dynamic_mode", 1))
        if mode not in DYN_MODE_LABELS:
            mode = 1
        pre = float(eq_dict.get("preamp_db", 0.0))
        thr = float(eq_dict.get("dyn_threshold_db", -18.0))
        rat = float(eq_dict.get("dyn_ratio", 2.5))
        gains = eq_dict.get("band_gains_db") or [0.0] * LWASAPI_EQ_BANDS
        if len(gains) != LWASAPI_EQ_BANDS:
            gains = [0.0] * LWASAPI_EQ_BANDS
        qs = eq_dict.get("band_q") or [1.414] * LWASAPI_EQ_BANDS
        if len(qs) != LWASAPI_EQ_BANDS:
            qs = [1.414] * LWASAPI_EQ_BANDS
        freqs = eq_dict.get("band_freq_hz") or list(DEFAULT_BAND_FREQS_HZ)
        if len(freqs) != LWASAPI_EQ_BANDS:
            freqs = list(DEFAULT_BAND_FREQS_HZ)
        shapes = eq_dict.get("band_type") or [LWASAPI_FILTER_PEAK] * LWASAPI_EQ_BANDS
        if len(shapes) != LWASAPI_EQ_BANDS:
            shapes = [LWASAPI_FILTER_PEAK] * LWASAPI_EQ_BANDS
        xf_en = bool(eq_dict.get("crossfeed_enabled", False))
        xf_lvl = max(0.0, min(1.0, float(eq_dict.get("crossfeed_level", 0.30))))
        xf_cut = int(eq_dict.get("crossfeed_cutoff_hz", 700) or 700)

        self.updating_ui = True
        self.chk_eq_enable.setChecked(bool(eq_dict.get("enabled", True)))
        self._set_preset_combo_value(pname)
        mode_idx = self.cmb_dyn_mode.findData(mode)
        if mode_idx >= 0:
            self.cmb_dyn_mode.setCurrentIndex(mode_idx)
        self.sld_preamp.setValue(int(round(pre * 10)))
        self.sld_thresh.setValue(int(round(thr * 10)))
        self.sld_ratio.setValue(int(round(rat * 10)))
        self.chk_limiter.setChecked(bool(eq_dict.get("limiter_enabled", True)))
        self.chk_crossfeed.setChecked(xf_en)
        self.sld_crossfeed.setValue(int(round(xf_lvl * 100)))
        self.lbl_crossfeed.setText(f"{int(round(xf_lvl * 100))}%")
        cut_idx = self.cmb_crossfeed_cut.findData(xf_cut)
        if cut_idx >= 0:
            self.cmb_crossfeed_cut.setCurrentIndex(cut_idx)

        for i in range(LWASAPI_EQ_BANDS):
            g = float(gains[i])
            self.band_sliders[i].setValue(int(round(g * 10)))
            self.band_val_labels[i].setText(f"{g:+.1f} dB")

        self.current_eq.enabled = 1 if self.chk_eq_enable.isChecked() else 0
        self.current_eq.dynamic_mode = mode
        self.current_eq.preamp_db = self.sld_preamp.value() / 10.0
        self.current_eq.dyn_threshold_db = self.sld_thresh.value() / 10.0
        self.current_eq.dyn_ratio = self.sld_ratio.value() / 10.0
        self.current_eq.limiter_enabled = 1 if self.chk_limiter.isChecked() else 0
        self.current_param_eq.crossfeed_enabled = 1 if xf_en else 0
        self.current_param_eq.crossfeed_level = xf_lvl
        self.current_param_eq.crossfeed_cutoff_hz = xf_cut
        for i in range(LWASAPI_EQ_BANDS):
            self.current_eq.band_gains_db[i] = self.band_sliders[i].value() / 10.0
            self.current_eq.band_q[i] = float(qs[i]) if float(qs[i]) >= 0.2 else 1.414
            self.current_param_eq.band_freq_hz[i] = float(freqs[i]) if float(freqs[i]) > 1.0 else DEFAULT_BAND_FREQS_HZ[i]
            self.current_param_eq.band_type[i] = int(shapes[i]) if int(shapes[i]) <= 2 else LWASAPI_FILTER_PEAK
        self.current_eq.preset_name = pname.encode("utf-8")[:31]
        self._refresh_band_freq_labels()
        self.updating_ui = False
        self._update_saved_eq_badges(pname, mode)

    def _restore_saved_eq_on_build(self) -> None:
        if not self._has_saved_eq_settings():
            self._update_saved_eq_badges(
                self.cmb_preset.currentText(),
                int(self.cmb_dyn_mode.currentData()),
            )
            return

        saved_cfg = self.gui_settings.get("eq_config")
        saved_preset = self.gui_settings.get("eq_preset")
        saved_mode = self.gui_settings.get("eq_dynamic_mode")

        if isinstance(saved_cfg, dict):
            eq_dict = dict(saved_cfg)
        elif isinstance(saved_preset, str) and saved_preset in PRESETS:
            p = PRESETS[saved_preset]
            eq_dict = {
                "enabled": True,
                "dynamic_mode": p["mode"],
                "preamp_db": p["preamp"],
                "dyn_threshold_db": p["thresh"],
                "dyn_ratio": p["ratio"],
                "limiter_enabled": True,
                "preset_name": saved_preset,
                "band_gains_db": list(p["gains"]),
            }
        else:
            eq_dict = self._extract_eq_dict_from_cfg(self.current_eq)

        if isinstance(saved_preset, str) and saved_preset.strip():
            eq_dict["preset_name"] = saved_preset.strip()
        if isinstance(saved_mode, int) and saved_mode in DYN_MODE_LABELS:
            eq_dict["dynamic_mode"] = saved_mode

        self._apply_eq_dict_to_ui(eq_dict)

    def _push_eq_cfg_to_engine(self, cfg: LwasapiEqConfig, param_ext: LwasapiParamEqExt | None = None) -> None:
        if not self.lib:
            self._pending_eq_push = True
            return
        ext = param_ext if param_ext is not None else self.current_param_eq
        if hasattr(self.lib, "LWASAPI_SetParametricEQ"):
            rc = self.lib.LWASAPI_SetParametricEQ(ctypes.byref(cfg), ctypes.byref(ext))
        else:
            rc = self.lib.LWASAPI_SetDynamicEQ(ctypes.byref(cfg))
        if rc == 0:
            self._pending_eq_push = False
            self._last_applied_eq_sig = self._eq_cfg_signature(cfg, ext)
        else:
            self._pending_eq_push = True

    def _refresh_band_freq_labels(self):
        if not hasattr(self, "band_freq_labels") or len(self.band_freq_labels) != LWASAPI_EQ_BANDS:
            return
        tags = ("", " LS", " HS")
        for i in range(LWASAPI_EQ_BANDS):
            hz = float(self.current_param_eq.band_freq_hz[i] or DEFAULT_BAND_FREQS_HZ[i])
            sh = int(self.current_param_eq.band_type[i]) if int(self.current_param_eq.band_type[i]) <= 2 else 0
            f_txt = f"{hz / 1000.0:.1f}k" if hz >= 1000.0 else f"{hz:.0f}"
            self.band_freq_labels[i].setText(f"{f_txt}{tags[sh]}")

    def _on_param_control_changed(self, *_args):
        if self.updating_ui:
            return
        self.current_param_eq.crossfeed_enabled = 1 if self.chk_crossfeed.isChecked() else 0
        self.current_param_eq.crossfeed_level = self.sld_crossfeed.value() / 100.0
        self.current_param_eq.crossfeed_cutoff_hz = int(self.cmb_crossfeed_cut.currentData() or 700)
        self.lbl_crossfeed.setText(f"{self.sld_crossfeed.value()}%")
        self._send_eq_update(preset_override=CUSTOM_PRESET_NAME)

    def _on_reset_flat_clicked(self):
        self.updating_ui = True
        for i in range(LWASAPI_EQ_BANDS):
            self.current_param_eq.band_freq_hz[i] = DEFAULT_BAND_FREQS_HZ[i]
            self.current_param_eq.band_type[i] = LWASAPI_FILTER_PEAK
            self.current_eq.band_q[i] = 1.414
        self.current_param_eq.crossfeed_enabled = 0
        self.chk_crossfeed.setChecked(False)
        self._refresh_band_freq_labels()
        self.updating_ui = False
        self._on_preset_selected("Flat / Studio Reference")

    def _import_autoeq_txt(self):
        path, _ = QFileDialog.getOpenFileName(
            self,
            "Import AutoEQ / Oratory1990 Parametric EQ (.txt)",
            os.path.expanduser("~"),
            "Parametric EQ Files (*.txt);;All Files (*)",
        )
        if not path:
            return
        try:
            with open(path, "r", encoding="utf-8", errors="ignore") as f:
                lines = f.readlines()
        except Exception as e:
            QMessageBox.warning(self, "AutoEQ Import Error", f"Could not read file:\n{e}")
            return

        preamp = 0.0
        bands = []
        rx_pre = re.compile(r"Preamp:\s*([+-]?\d+(?:\.\d+)?)\s*dB", re.IGNORECASE)
        rx_flt = re.compile(
            r"Filter\s*\d*:\s*ON\s+(PK|LSC|HSC|LS|HS|LP|HP)\s+Fc\s+(\d+(?:\.\d+)?)\s*Hz\s+Gain\s*([+-]?\d+(?:\.\d+)?)\s*dB(?:\s+Q\s+(\d+(?:\.\d+)?))?",
            re.IGNORECASE,
        )
        for ln in lines:
            m_pre = rx_pre.search(ln)
            if m_pre:
                preamp = max(-12.0, min(12.0, float(m_pre.group(1))))
                continue
            m_f = rx_flt.search(ln)
            if m_f and len(bands) < LWASAPI_EQ_BANDS:
                ft = m_f.group(1).upper()
                fc = max(15.0, min(22000.0, float(m_f.group(2))))
                gn = max(-18.0, min(18.0, float(m_f.group(3))))
                qv = float(m_f.group(4)) if m_f.group(4) else 1.414
                qv = max(0.20, min(10.0, qv))
                shape = LWASAPI_FILTER_PEAK
                if ft in ("LSC", "LS"):
                    shape = LWASAPI_FILTER_LOW_SHELF
                elif ft in ("HSC", "HS"):
                    shape = LWASAPI_FILTER_HIGH_SHELF
                bands.append((fc, gn, qv, shape))

        if not bands:
            QMessageBox.information(
                self,
                "AutoEQ Import",
                "No valid EqualizerAPO / Oratory1990 'Filter: ON ... Fc ... Gain ... Q ...' lines found.",
            )
            return

        pname = os.path.splitext(os.path.basename(path))[0][:31] or "AutoEQ Import"
        self.updating_ui = True
        self.chk_eq_enable.setChecked(True)
        self.chk_limiter.setChecked(True)
        self.sld_preamp.setValue(int(round(preamp * 10)))
        self.lbl_preamp.setText(f"Preamp: {preamp:+.1f} dB")
        mode_idx = self.cmb_dyn_mode.findData(0)
        if mode_idx >= 0:
            self.cmb_dyn_mode.setCurrentIndex(mode_idx)
        for i in range(LWASAPI_EQ_BANDS):
            if i < len(bands):
                fc, gn, qv, sh = bands[i]
            else:
                fc, gn, qv, sh = DEFAULT_BAND_FREQS_HZ[i], 0.0, 1.414, LWASAPI_FILTER_PEAK
            self.current_param_eq.band_freq_hz[i] = fc
            self.current_param_eq.band_type[i] = sh
            self.current_eq.band_q[i] = qv
            self.band_sliders[i].setValue(int(round(gn * 10)))
            self.band_val_labels[i].setText(f"{gn:+.1f} dB")
        self._refresh_band_freq_labels()
        self.updating_ui = False
        self._send_eq_update(preset_override=pname)

    def _on_preset_selected(self, preset_name: str):
        if self.updating_ui or preset_name not in PRESETS:
            return
        p = PRESETS[preset_name]
        self.updating_ui = True
        self._set_preset_combo_value(preset_name)
        self.chk_eq_enable.setChecked(True)
        mode_idx = self.cmb_dyn_mode.findData(p["mode"])
        if mode_idx >= 0:
            self.cmb_dyn_mode.setCurrentIndex(mode_idx)
        self.sld_preamp.setValue(int(round(p["preamp"] * 10)))
        self.sld_thresh.setValue(int(round(p["thresh"] * 10)))
        self.sld_ratio.setValue(int(round(p["ratio"] * 10)))
        for i in range(LWASAPI_EQ_BANDS):
            self.band_sliders[i].setValue(int(round(p["gains"][i] * 10)))
            self.band_val_labels[i].setText(f"{p['gains'][i]:+.1f} dB")
            self.current_param_eq.band_freq_hz[i] = DEFAULT_BAND_FREQS_HZ[i]
            self.current_param_eq.band_type[i] = LWASAPI_FILTER_PEAK
            self.current_eq.band_q[i] = 1.414
        self._refresh_band_freq_labels()
        self.updating_ui = False
        self._send_eq_update(preset_override=preset_name)

    def _on_band_slider_changed(self, idx: int, val: int):
        gain_db = val / 10.0
        self.band_val_labels[idx].setText(f"{gain_db:+.1f} dB")
        if not self.updating_ui:
            self._send_eq_update(preset_override=CUSTOM_PRESET_NAME)

    def _on_canvas_band_dragged(self, idx: int, gain_db: float):
        self.updating_ui = True
        self.band_sliders[idx].setValue(int(round(gain_db * 10)))
        self.band_val_labels[idx].setText(f"{gain_db:+.1f} dB")
        self.updating_ui = False
        self._send_eq_update(preset_override=CUSTOM_PRESET_NAME)

    def _on_dynamics_slider_changed(self):
        pre = self.sld_preamp.value() / 10.0
        thr = self.sld_thresh.value() / 10.0
        rat = self.sld_ratio.value() / 10.0
        self.lbl_preamp.setText(f"Preamp: {pre:+.1f} dB")
        self.lbl_thresh.setText(f"Dyn Threshold: {thr:.1f} dB")
        self.lbl_ratio.setText(f"Dyn Ratio: {rat:.1f}:1")
        if not self.updating_ui:
            self._send_eq_update()

    def _send_eq_update(self, preset_override=None):
        if self.updating_ui:
            return
        pname = (
            preset_override.strip()
            if isinstance(preset_override, str) and preset_override.strip()
            else self.cmb_preset.currentText()
        )
        if isinstance(preset_override, str):
            self._set_preset_combo_value(pname)

        self.current_eq.enabled = 1 if self.chk_eq_enable.isChecked() else 0
        self.current_eq.dynamic_mode = int(self.cmb_dyn_mode.currentData())
        self.current_eq.preamp_db = self.sld_preamp.value() / 10.0
        self.current_eq.dyn_threshold_db = self.sld_thresh.value() / 10.0
        self.current_eq.dyn_ratio = self.sld_ratio.value() / 10.0
        self.current_eq.limiter_enabled = 1 if self.chk_limiter.isChecked() else 0
        for i in range(LWASAPI_EQ_BANDS):
            self.current_eq.band_gains_db[i] = self.band_sliders[i].value() / 10.0
            if self.current_eq.band_q[i] < 0.2:
                self.current_eq.band_q[i] = 1.414
            if self.current_param_eq.band_freq_hz[i] < 15.0:
                self.current_param_eq.band_freq_hz[i] = DEFAULT_BAND_FREQS_HZ[i]
        self.current_param_eq.crossfeed_enabled = 1 if self.chk_crossfeed.isChecked() else 0
        self.current_param_eq.crossfeed_level = self.sld_crossfeed.value() / 100.0
        self.current_param_eq.crossfeed_cutoff_hz = int(self.cmb_crossfeed_cut.currentData() or 700)
        self.current_eq.preset_name = pname.encode("utf-8")[:31]

        self._push_eq_cfg_to_engine(self.current_eq, self.current_param_eq)
        self._persist_eq_settings(self._extract_eq_dict_from_cfg(self.current_eq, self.current_param_eq))

    def _on_mic_apo_changed(self, *_args):
        if self.updating_ui or not self.lib or not hasattr(self.lib, "LWASAPI_SetMicAPO"):
            return
        self._mic_apo_hold_until = time.monotonic() + 0.5
        self.current_mic_apo.enabled = 1 if self.chk_mic_apo.isChecked() else 0
        self.current_mic_apo.hpf_enabled = 1 if self.chk_mic_hpf.isChecked() else 0
        self.current_mic_apo.hpf_hz = 65.0
        self.current_mic_apo.gate_enabled = 1 if self.chk_mic_gate.isChecked() else 0
        self.current_mic_apo.gate_thresh_db = float(self.spn_mic_gate.value())
        self.current_mic_apo.rnnoise_enabled = 1 if self.chk_mic_rnnoise.isChecked() else 0
        self.current_mic_apo.comp_enabled = 1 if self.chk_mic_comp.isChecked() else 0
        self.current_mic_apo.comp_thresh_db = -14.0
        self.current_mic_apo.comp_ratio = 2.0
        self.current_mic_apo.clarity_db = float(self.spn_mic_clarity.value())
        self.current_mic_apo.sidetone_enabled = 1 if self.chk_mic_sidetone.isChecked() else 0
        self.current_mic_apo.sidetone_pct = int(self.spn_mic_sidetone.value())
        self.lib.LWASAPI_SetMicAPO(ctypes.byref(self.current_mic_apo))

    def _on_mic_preset_selected(self, *_args):
        if self.updating_ui or not self.lib or not hasattr(self.lib, "LWASAPI_SetMicAPO"):
            return
        pname = self.cmb_mic_preset.currentData() or self.cmb_mic_preset.currentText()
        p = MIC_APO_PRESETS.get(pname)
        if not p:
            return
        self._mic_apo_hold_until = time.monotonic() + 0.5
        self.updating_ui = True
        try:
            self.chk_mic_apo.setChecked(bool(p["enabled"]))
            self.chk_mic_hpf.setChecked(bool(p["hpf_enabled"]))
            self.chk_mic_gate.setChecked(bool(p["gate_enabled"]))
            self.spn_mic_gate.setValue(int(round(p["gate_thresh_db"])))
            self.chk_mic_rnnoise.setChecked(bool(p["rnnoise_enabled"]))
            self.chk_mic_comp.setChecked(bool(p["comp_enabled"]))
            self.spn_mic_clarity.setValue(float(p["clarity_db"]))
        finally:
            self.updating_ui = False
        self.current_mic_apo.enabled = int(p["enabled"])
        self.current_mic_apo.hpf_enabled = int(p["hpf_enabled"])
        self.current_mic_apo.hpf_hz = float(p["hpf_hz"])
        self.current_mic_apo.gate_enabled = int(p["gate_enabled"])
        self.current_mic_apo.gate_thresh_db = float(p["gate_thresh_db"])
        self.current_mic_apo.rnnoise_enabled = int(p["rnnoise_enabled"])
        self.current_mic_apo.comp_enabled = int(p["comp_enabled"])
        self.current_mic_apo.comp_thresh_db = float(p["comp_thresh_db"])
        self.current_mic_apo.comp_ratio = float(p["comp_ratio"])
        self.current_mic_apo.clarity_db = float(p["clarity_db"])
        self.current_mic_apo.sidetone_enabled = 1 if self.chk_mic_sidetone.isChecked() else 0
        self.current_mic_apo.sidetone_pct = int(self.spn_mic_sidetone.value())
        self.lib.LWASAPI_SetMicAPO(ctypes.byref(self.current_mic_apo))

    def _on_bt_cfg_changed(self, *_args):
        if self.updating_ui or not self.lib or not hasattr(self.lib, "LWASAPI_SetBtCodecConfig"):
            return
        self._bt_cfg_hold_until = time.monotonic() + 0.6
        self.current_bt_cfg.preferred_codec = int(self.cmb_bt_codec.currentData() or 0)
        self.current_bt_cfg.ldac_mode = int(self.cmb_ldac_mode.currentData() or 0)
        self.current_bt_cfg.wireless_abr_enabled = 1 if self.chk_bt_abr.isChecked() else 0
        self.current_bt_cfg.hfp_sco_enabled = 1 if self.chk_bt_sco.isChecked() else 0
        self.lib.LWASAPI_SetBtCodecConfig(ctypes.byref(self.current_bt_cfg))

    def _on_hw_boost_slider_changed(self, val: int):
        db = float(val) * 0.5
        self.lbl_hw_boost.setText(f"+{db:.1f} dB")
        if not self.updating_ui and not self.sld_hw_boost.isSliderDown():
            self._on_hw_analog_cfg_changed()

    def _on_hw_mic_boost_slider_changed(self, val: int):
        db = float(val)
        self.lbl_hw_mic_boost.setText(f"+{db:.1f} dB")
        if not self.updating_ui and not self.sld_hw_mic_boost.isSliderDown():
            self._on_hw_analog_cfg_changed()

    def _on_hw_analog_reset(self):
        self.updating_ui = True
        self.chk_hw_dac_unity.setChecked(True)
        self.cmb_hw_amp.setCurrentIndex(0)
        self.cmb_hw_route.setCurrentIndex(0)
        self.chk_hw_rails.setChecked(False)
        self.sld_hw_boost.setValue(0)
        self.lbl_hw_boost.setText("+0.0 dB")
        self.sld_hw_mic_boost.setValue(0)
        self.lbl_hw_mic_boost.setText("+0.0 dB")
        self.updating_ui = False
        self._on_hw_analog_cfg_changed()

    def _on_hw_analog_cfg_changed(self, *_args):
        if self.updating_ui or not self.lib or not hasattr(self.lib, "LWASAPI_SetHwAnalog"):
            return
        self._hw_analog_hold_until = time.monotonic() + 0.6
        self.current_hw_analog.hw_dac_unity_vol = 1 if self.chk_hw_dac_unity.isChecked() else 0
        self.current_hw_analog.hp_amp_tier = int(self.cmb_hw_amp.currentData() or 0)
        self.current_hw_analog.output_route = int(self.cmb_hw_route.currentData() or 0)
        self.current_hw_analog.unlock_0db_rails = 1 if self.chk_hw_rails.isChecked() else 0
        self.current_hw_analog.analog_boost_db = float(self.sld_hw_boost.value()) * 0.5
        self.current_hw_analog.mic_hw_boost_db = float(self.sld_hw_mic_boost.value())
        self.lib.LWASAPI_SetHwAnalog(ctypes.byref(self.current_hw_analog))

    def _on_vol_moving(self, kind: str, val: int):
        is_mic = (kind == "mic")
        lbl = self.lbl_mic if is_mic else self.lbl_spk
        prefix = "Mic Gain" if is_mic else "Speaker"
        lbl.setText(f"{prefix} ({val}%):")
        hold_tm = self._mic_hold_timer if is_mic else self._spk_hold_timer
        send_tm = self._mic_send_timer if is_mic else self._spk_send_timer
        hold_tm.start(350)
        if not send_tm.isActive():
            send_tm.start(40)

    def _flush_vol(self, kind: str):
        is_mic = (kind == "mic")
        send_tm = self._mic_send_timer if is_mic else self._spk_send_timer
        hold_tm = self._mic_hold_timer if is_mic else self._spk_hold_timer
        sld = self.sld_mic if is_mic else self.sld_spk
        proc = self._mic_proc if is_mic else self._spk_proc
        cmd = "mic-volume" if is_mic else "master-volume"
        send_tm.stop()
        hold_tm.start(350)
        if proc is not None and proc.poll() is None:
            send_tm.start(40)
            return
        new_proc = subprocess.Popen([CTL_BIN, cmd, str(sld.value())])
        if is_mic:
            self._mic_proc = new_proc
        else:
            self._spk_proc = new_proc

    def _on_spk_vol_moving(self, val: int):
        self._on_vol_moving("spk", val)

    def _on_mic_vol_moving(self, val: int):
        self._on_vol_moving("mic", val)

    def _flush_spk_vol(self):
        self._flush_vol("spk")

    def _flush_mic_vol(self):
        self._flush_vol("mic")

    def _run_latency_benchmark(self):
        bench_bin = os.path.join(BASE_DIR, "build", "lwasapi-bench")
        res = subprocess.run([bench_bin], capture_output=True, text=True)
        t = self.current_theme
        parent_w = self.adv_win if (self.adv_win and self.adv_win.isVisible()) else self
        dlg = QDialog(parent_w)
        dlg.setWindowTitle("Salsa — Linux WASAPI vs. Linux Audio Modules Latency Benchmark")
        dlg.resize(820, 500)
        lay = QVBoxLayout(dlg)
        txt = QTextEdit(dlg)
        txt.setReadOnly(True)
        txt.setStyleSheet(
            f"background-color: {t['bg_deep']}; color: {t['accent_primary']}; "
            "font-family: Monospace; font-size: 11px;"
        )
        txt.setPlainText(res.stdout if res.returncode == 0 else res.stderr)
        lay.addWidget(txt)
        btn_close = QPushButton("Close Benchmark Report")
        btn_close.clicked.connect(dlg.accept)
        lay.addWidget(btn_close)
        dlg.exec()

    def _run_module_sync(self):
        sync_script = os.path.join(BASE_DIR, "lwasapi-autosync.sh")
        res = subprocess.run([sync_script, "--force"], capture_output=True, text=True)
        self._select_manual_chapter("autosync")
        t = self.current_theme
        parent_w = (
            self.adv_win if (self.adv_win and self.adv_win.isVisible())
            else (self.intro_win if (self.intro_win and self.intro_win.isVisible()) else self)
        )
        dlg = QDialog(parent_w)
        dlg.setWindowTitle("Salsa LWASAPI Audio Module Auto-Sync Report")
        dlg.resize(760, 360)
        lay = QVBoxLayout(dlg)
        txt = QTextEdit(dlg)
        txt.setReadOnly(True)
        txt.setStyleSheet(
            f"background-color: {t['bg_deep']}; color: {t['accent_primary']}; "
            "font-family: Monospace; font-size: 11px;"
        )
        txt.setPlainText(res.stdout + ("\n" + res.stderr if res.stderr else ""))
        lay.addWidget(txt)
        btn_close = QPushButton("Close Sync Report")
        btn_close.clicked.connect(dlg.accept)
        lay.addWidget(btn_close)
        dlg.exec()

    def _on_sink_selected(self, _idx=0):
        if not self.lib:
            return
        dev_id = self.cmb_sink.currentData() or "auto"
        self.lib.LWASAPI_SetDevices(dev_id.encode("utf-8"), None)
        self._last_dev_sig = None

    def _on_source_selected(self, _idx=0):
        if not self.lib:
            return
        dev_id = self.cmb_source.currentData() or "auto"
        self.lib.LWASAPI_SetDevices(None, dev_id.encode("utf-8"))
        self._last_dev_sig = None

    def _on_bt_reset_clicked(self):
        if self._bt_reset_proc is not None and self._bt_reset_proc.state() != QProcess.ProcessState.NotRunning:
            return
        self.btn_bt_reset.setEnabled(False)
        self.btn_bt_reset.setText("Resetting BT...")
        self.lbl_endpoints.setText("Resetting & reconnecting Bluetooth A2DP (BR/EDR)...")
        proc = QProcess(self)
        proc.finished.connect(self._on_bt_reset_finished)
        self._bt_reset_proc = proc
        proc.start(CTL_BIN, ["bt-reset"])

    def _on_bt_reset_finished(self, exit_code: int, _exit_status):
        proc = self._bt_reset_proc
        out = ""
        err = ""
        if proc is not None:
            out = bytes(proc.readAllStandardOutput()).decode("utf-8", "ignore").strip()
            err = bytes(proc.readAllStandardError()).decode("utf-8", "ignore").strip()
            proc.deleteLater()
            self._bt_reset_proc = None
        self.btn_bt_reset.setEnabled(True)
        self.btn_bt_reset.setText("Reset BT")
        self._last_dev_sig = None
        msg = out or err or ("Bluetooth audio reconnected." if exit_code == 0 else "Bluetooth reset failed.")
        if exit_code != 0:
            self.lbl_endpoints.setText(msg)
            QMessageBox.warning(self, "Salsa Bluetooth Reset", msg)

    def _refresh_devices(self, active_sink_id: str, active_cap_id: str, force: bool = False):
        if not self.lib:
            return
        dlist = LwasapiDeviceList()
        if self.lib.LWASAPI_ListDevices(ctypes.byref(dlist)) != 0:
            return
        n = min(dlist.num_devices, LWASAPI_MAX_DEVICES)
        sinks = [("auto", "Auto-Priority (BT A2DP > USB > Analog > HDMI)")]
        sources = [("auto", "Auto-Priority (USB Mic > HDA > Virtual)")]
        sig_parts = [active_sink_id, active_cap_id]
        for i in range(n):
            d = dlist.devices[i]
            did = d.id.decode("utf-8", "ignore")
            dname = d.name.decode("utf-8", "ignore") or did
            drv = d.driver.decode("utf-8", "ignore")
            label = f"{dname} [{drv}] ({did})"
            sig_parts.append(f"{did}:{d.is_capture}:{d.is_active}")
            if d.is_capture:
                sources.append((did, label))
            else:
                sinks.append((did, label))

        self._cached_sinks = [("auto", "Auto (Global Output)")] + sinks[1:]
        self._cached_sources = [("auto", "Auto (Global Mic)")] + sources[1:]

        sig = tuple(sig_parts)
        if not force and sig == self._last_dev_sig:
            return
        self._last_dev_sig = sig

        for cmb, items, active_id in (
            (self.cmb_sink, sinks, active_sink_id),
            (self.cmb_source, sources, active_cap_id),
        ):
            if cmb.view().isVisible():
                continue
            cmb.blockSignals(True)
            cmb.clear()
            sel_idx = 0
            for idx, (did, label) in enumerate(items):
                cmb.addItem(label, did)
                if did == active_id:
                    sel_idx = idx
            cmb.setCurrentIndex(sel_idx)
            cmb.blockSignals(False)

    def _send_session_control(self, sid: int, vol_pct: int = -1, mute_mode: int = -1, target_device: str | None = None):
        if not self.lib or not hasattr(self.lib, "LWASAPI_SetSessionControl"):
            return
        dev_arg = target_device.encode("utf-8")[:63] if isinstance(target_device, str) and target_device else None
        self.lib.LWASAPI_SetSessionControl(int(sid), int(vol_pct), int(mute_mode), dev_arg)
        meta = self._sess_widgets.get(int(sid))
        if meta is not None:
            meta["hold_until"] = time.monotonic() + 0.45

    def _toggle_engine_mode(self):
        res = subprocess.run(
            ["systemctl", "--user", "is-active", "lwasapi-audiodg.service"],
            capture_output=True,
            text=True,
        )
        is_active = res.stdout.strip() == "active"
        ui_connected = "Revert to PW/WP" in self.btn_engine_toggle.text()
        if is_active and ui_connected:
            subprocess.run([CTL_BIN, "disable-auto"])
        else:
            subprocess.run([CTL_BIN, "enable-auto"])
            subprocess.run(["systemctl", "--user", "restart", "lwasapi-audiodg.service"])
            if os.path.exists(LIB_PATH) and os.path.getmtime(LIB_PATH) != self._lib_mtime:
                os.execv(sys.executable, [sys.executable, *sys.argv])
        self._last_dev_sig = None

    def _poll_pw_telem_async(self):
        if self._pw_telem_inflight:
            return
        now_m = time.monotonic()
        if (now_m - self._pw_telem_ts) < 1.2:
            return
        self._pw_telem_inflight = True
        self._pw_telem_ts = now_m

        def _worker():
            pw_d = None
            try:
                snap = snapshot_once(timeout=0.35)
                telem = snap.get("telemetry") if isinstance(snap, dict) else None
                if isinstance(telem, dict) and isinstance(telem.get("pw"), dict):
                    pw_d = telem["pw"]
            except Exception:
                pass
            if pw_d is not None:
                self._cached_pw_telem = pw_d
            self._pw_telem_inflight = False

        threading.Thread(target=_worker, daemon=True).start()

    def _get_pw_telem(self) -> dict | None:
        if getattr(self, "intro_win", None) is not None and self.intro_win.isVisible():
            self._poll_pw_telem_async()
        return self._cached_pw_telem

    @staticmethod
    def _probe_alsa_playback_caps(pb_id: str):
        m = re.match(r"^hw:(\d+),(\d+)$", (pb_id or "").strip())
        if not m:
            return None, None
        path = f"/proc/asound/card{m.group(1)}/stream0"
        if not os.path.exists(path):
            return None, None
        sup_rates = set()
        sup_bits = set()
        in_playback = False
        try:
            with open(path, "r", encoding="utf-8", errors="ignore") as f:
                for raw in f:
                    ln = raw.strip()
                    if ln.startswith("Playback:"):
                        in_playback = True
                        continue
                    if ln.startswith("Capture:"):
                        in_playback = False
                        continue
                    if not in_playback:
                        continue
                    if ln.startswith("Rates:"):
                        if "continuous" in ln.lower() or "-" in ln:
                            return None, None
                        for tok in re.findall(r"\d+", ln):
                            v = int(tok)
                            if 8000 <= v <= 768000:
                                sup_rates.add(v)
                    elif ln.startswith("Bits:"):
                        for tok in re.findall(r"\d+", ln):
                            b = int(tok)
                            if b in (16, 24, 32):
                                sup_bits.add(b)
                    elif ln.startswith("Format:"):
                        if "S16" in ln:
                            sup_bits.add(16)
                        if "S24" in ln:
                            sup_bits.add(24)
                        if "S32" in ln or "FLOAT" in ln:
                            sup_bits.add(32)
        except OSError:
            return None, None
        return (sup_rates or None), (sup_bits or None)

    def _sync_clock_cap_labels(self, pb_id: str):
        if pb_id == getattr(self, "_last_cap_pb_id", None):
            return
        self._last_cap_pb_id = pb_id
        sup_rates, sup_bits = self._probe_alsa_playback_caps(pb_id)
        self._hw_sup_rates = sup_rates
        self._hw_sup_bits = sup_bits
        if not self.cmb_rate.view().isVisible():
            for idx, r in enumerate(self.rates):
                if sup_rates and r not in sup_rates:
                    lbl = f"{r:,} Hz (DAC N/A)"
                else:
                    tag = " (Ultra-HD Master)" if r >= 176400 else (" (Hi-Res)" if r >= 88200 else "")
                    lbl = f"{r:,} Hz{tag}"
                if self.cmb_rate.itemText(idx) != lbl:
                    self.cmb_rate.setItemText(idx, lbl)
        if not self.cmb_bits.view().isVisible():
            base_bits = {32: "32-Bit Float (Studio)", 24: "24-Bit Studio PCM", 16: "16-Bit Standard PCM"}
            for idx, b in enumerate(self.bit_depths):
                lbl = f"{b}-Bit (DAC N/A)" if (sup_bits and b not in sup_bits) else base_bits[b]
                if self.cmb_bits.itemText(idx) != lbl:
                    self.cmb_bits.setItemText(idx, lbl)

    def _poll_engine(self, initial=False):
        intro_visible = bool(
            getattr(self, "intro_win", None) is not None
            and self.intro_win.isVisible()
            and not self.intro_win.isMinimized()
        )
        adv_visible = bool(
            getattr(self, "adv_win", None) is not None
            and self.adv_win.isVisible()
            and not self.adv_win.isMinimized()
        )
        ui_visible = (self.isVisible() and not self.isMinimized()) or adv_visible
        hidden_or_min = (not initial) and (not ui_visible) and (not intro_visible)
        want_ms = 500 if hidden_or_min else 80
        if hasattr(self, "timer") and self.timer.interval() != want_ms:
            self.timer.setInterval(want_ms)
        if not initial:
            self._sync_graph_anim_timer()

        if not self.lib:
            return
        st = LwasapiEngineStatus()
        hr = self.lib.LWASAPI_GetEngineStatus(ctypes.byref(st))
        if hr != 0:
            self.btn_engine_toggle.setText("Engine: STOCK PW/WP (Click for Pure Salsa)")
            if self.btn_engine_toggle.objectName() != "AccentBtn":
                self.btn_engine_toggle.setObjectName("AccentBtn")
                self.btn_engine_toggle.style().unpolish(self.btn_engine_toggle)
                self.btn_engine_toggle.style().polish(self.btn_engine_toggle)
            self.lbl_latency.setText("LWASAPI Daemon Stopped (Stock PW/WP Revert Mode)")
            if initial or intro_visible:
                self.graph_canvas.update_graph_state(None, self._get_pw_telem() or {"broker_running": 0})
            return

        btn_txt = "Engine: SALSA NATIVE AUDIO (Revert to PW/WP)"
        if self.btn_engine_toggle.text() != btn_txt:
            self.btn_engine_toggle.setText(btn_txt)
        if self.btn_engine_toggle.objectName() != "PrimaryBtn":
            self.btn_engine_toggle.setObjectName("PrimaryBtn")
            self.btn_engine_toggle.style().unpolish(self.btn_engine_toggle)
            self.btn_engine_toggle.style().polish(self.btn_engine_toggle)

        pb_id_now = st.active_playback_id.decode("utf-8", "ignore").strip("\x00 ")
        self._sync_clock_cap_labels(pb_id_now)

        rate = st.hw_sample_rate if st.hw_sample_rate > 0 else 48000
        period = st.hw_period_frames if st.hw_period_frames > 0 else 256
        bits = st.hw_format_bits if st.hw_format_bits > 0 else 32
        period_ms = (1000.0 * period) / rate
        rt_str = f"SCHED_FIFO Prio {st.realtime_priority}" if st.realtime_priority else "Standard CFS"
        lat_txt = f"Quantum: {period_ms:.2f} ms ({rate / 1000.0:g}kHz/{bits}b) | {rt_str}"
        if self.lbl_latency.text() != lat_txt:
            self.lbl_latency.setText(lat_txt)

        sup_rates = getattr(self, "_hw_sup_rates", None)
        sup_bits = getattr(self, "_hw_sup_bits", None)
        sup_r_note = (
            f" ({pb_id_now} DAC supports {', '.join(f'{r / 1000:g}k' for r in sorted(sup_rates))} Hz)"
            if sup_rates else ""
        )
        sup_b_note = (
            f" ({pb_id_now} DAC supports {'/'.join(str(b) for b in sorted(sup_bits))}-bit)"
            if sup_bits else ""
        )

        req_info = getattr(self, "_last_clock_req", None)
        locks = []
        if req_info and time.monotonic() <= req_info[3]:
            req_r, req_p, req_b = int(req_info[0]), int(req_info[1]), int(req_info[2])
            if req_r > 0 and req_r != rate:
                locks.append(f"{req_r / 1000.0:g}kHz N/A")
            if req_b > 0 and req_b != bits:
                locks.append(f"{req_b}b N/A")
            if req_p > 0 and req_p != period:
                locks.append(f"{req_p}f->{period}f")

        if locks:
            clk_src_txt = f"HW Locked: {rate / 1000.0:g}kHz/{bits}b ({', '.join(locks)})"
            lock_tip = f"Hardware endpoint {pb_id_now or 'active'} clamped clock to {rate:,} Hz / {bits}-bit{sup_r_note}{sup_b_note}."
            self.cmb_rate.setToolTip(lock_tip)
            self.cmb_bits.setToolTip(lock_tip)
        else:
            clk_src_txt = f"Master Clock: {rate / 1000.0:g}kHz / {bits}-bit PCM"
            self.cmb_rate.setToolTip(f"Select engine Master Sample Rate{sup_r_note or ' (44.1–192 kHz)'}.")
            self.cmb_bits.setToolTip(f"Select engine PCM bit depth{sup_b_note or ' (16, 24, or 32-bit)'}.")
        cap_rate = int(st.hw_analog_info.reserved) if 44100 <= int(st.hw_analog_info.reserved) <= 192000 else rate
        if cap_rate != rate and "Mic:" not in clk_src_txt:
            clk_src_txt += f"  •  Mic: {cap_rate / 1000.0:g}kHz"
        if hasattr(self, "lbl_clock_source") and self.lbl_clock_source.text() != clk_src_txt:
            self.lbl_clock_source.setText(clk_src_txt)

        cap_sig = (cap_rate, rate)
        if hasattr(self, "cmb_cap_rate") and (initial or cap_sig != self._last_cap_rate_sig) and (
            time.monotonic() >= self._cap_rate_hold_until and not self.cmb_cap_rate.view().isVisible()
        ):
            self._last_cap_rate_sig = cap_sig
            if cap_rate != rate:
                want_cap_sel = cap_rate
            elif self._last_cap_rate_pref == rate:
                want_cap_sel = rate
            else:
                want_cap_sel = 0
            idx_cap = self.cmb_cap_rate.findData(want_cap_sel)
            if idx_cap >= 0 and self.cmb_cap_rate.currentIndex() != idx_cap:
                self.cmb_cap_rate.blockSignals(True)
                self.cmb_cap_rate.setCurrentIndex(idx_cap)
                self.cmb_cap_rate.blockSignals(False)

        clk_policy_sig = (rate, period, bits)
        if (initial or clk_policy_sig != self._last_clock_policy_sig) and not (
            self.cmb_rate.view().isVisible()
            or self.cmb_period.view().isVisible()
            or self.cmb_bits.view().isVisible()
        ):
            self._last_clock_policy_sig = clk_policy_sig
            self.updating_ui = True
            if rate in self.rates:
                self.cmb_rate.setCurrentIndex(self.rates.index(rate))
            if period in self.periods:
                self.cmb_period.setCurrentIndex(self.periods.index(period))
            if bits in self.bit_depths:
                self.cmb_bits.setCurrentIndex(self.bit_depths.index(bits))
            self.updating_ui = False

        if initial:
            if not os.path.exists(_salsa_state_path()) and self._has_saved_eq_settings():
                self._push_eq_cfg_to_engine(self.current_eq, self.current_param_eq)
            else:
                eq_dict = self._extract_eq_dict_from_cfg(st.eq_cfg, st.param_eq)
                self._apply_eq_dict_to_ui(eq_dict)
                self._last_applied_eq_sig = self._eq_cfg_signature(st.eq_cfg, st.param_eq)
                self._persist_eq_settings(eq_dict)
        elif getattr(self, "_pending_eq_push", False):
            self._push_eq_cfg_to_engine(self.current_eq, self.current_param_eq)
        else:
            eq_sig = self._eq_cfg_signature(st.eq_cfg, st.param_eq)
            if (
                eq_sig != self._last_applied_eq_sig
                and not self.updating_ui
                and not self.cmb_preset.view().isVisible()
                and not self.cmb_dyn_mode.view().isVisible()
                and not self._any_eq_slider_down()
            ):
                eq_dict = self._extract_eq_dict_from_cfg(st.eq_cfg, st.param_eq)
                self._apply_eq_dict_to_ui(eq_dict)
                self._last_applied_eq_sig = eq_sig
                self._persist_eq_settings(eq_dict)

        cur_cfg_mtime = self._read_gui_settings_mtime()
        if cur_cfg_mtime and cur_cfg_mtime != self._gui_settings_mtime:
            self._gui_settings_mtime = cur_cfg_mtime
            disk_cfg = load_gui_settings()
            if isinstance(disk_cfg, dict) and disk_cfg:
                self.gui_settings.update(disk_cfg)
                if hasattr(self, "chk_eq_per_dev"):
                    per_dev = bool(disk_cfg.get("eq_per_device", disk_cfg.get("eq_per_device_enabled", False)))
                    if self.chk_eq_per_dev.isChecked() != per_dev:
                        self.chk_eq_per_dev.blockSignals(True)
                        self.chk_eq_per_dev.setChecked(per_dev)
                        self.chk_eq_per_dev.blockSignals(False)
                disk_step = normalize_vol_step_pct(disk_cfg.get("volume_step_pct"))
                if disk_step != self._vol_step_pct:
                    self._apply_vol_step(disk_step, save=False)

        gains = [self.band_sliders[i].value() / 10.0 for i in range(LWASAPI_EQ_BANDS)]
        deltas = [st.eq_dyn_delta_db[i] for i in range(LWASAPI_EQ_BANDS)]
        levels = [st.eq_band_levels_db[i] for i in range(LWASAPI_EQ_BANDS)]
        for i in range(LWASAPI_EQ_BANDS):
            d_txt = f"Δ {deltas[i]:+.1f}dB"
            if self.band_dyn_labels[i].text() != d_txt:
                self.band_dyn_labels[i].setText(d_txt)

        if initial or (self.isVisible() and not self.isMinimized()):
            self.canvas.update_telemetry(
                bool(self.chk_eq_enable.isChecked()),
                int(self.cmb_dyn_mode.currentData()),
                self.sld_thresh.value() / 10.0,
                gains,
                deltas,
                levels,
            )

        if hasattr(self, "lbl_auto_badge"):
            bt_cod_s = st.bt_codec.decode("utf-8", "ignore") or "PCM"
            clk_b = f"{rate // 1000}k/{cap_rate // 1000}k/{bits}b" if cap_rate != rate else f"{rate // 1000}kHz/{bits}b"
            eq_b = f"EQ:{'ON' if st.eq_cfg.enabled else 'OFF'}{'+CF' if st.param_eq.crossfeed_enabled else ''}"
            mic_b = f"Mic:{'Krisp-APO' if st.mic_apo.enabled else 'Raw'}"
            bt_b = f"BT:{bt_cod_s}{' ABR' if st.bt_codec_cfg.wireless_abr_enabled else ''}" if st.bt_active else ("DAC:Unity" if st.hw_analog_info.dac_unity_active else "Out:ALSA")
            badge_txt = f"✔ v6.9-beta LIVE | {clk_b} | {eq_b} | {mic_b} | {bt_b}"
            if self.lbl_auto_badge.text() != badge_txt:
                self.lbl_auto_badge.setText(badge_txt)

        if initial or intro_visible:
            self.graph_canvas.update_graph_state(st, self._get_pw_telem())

        pk_l = 20.0 * math.log10(st.output_peak_l) if st.output_peak_l > 0.0001 else -60.0
        pk_r = 20.0 * math.log10(st.output_peak_r) if st.output_peak_r > 0.0001 else -60.0
        pk_txt = f"Output Peak L/R: {pk_l:5.1f} / {pk_r:5.1f} dBFS"
        if self.lbl_peaks.text() != pk_txt:
            self.lbl_peaks.setText(pk_txt)

        now_mono = time.monotonic()
        if hasattr(self, "chk_mic_apo"):
            if st.mic_apo.enabled:
                rnn_lbl = "Neural" if st.mic_rnnoise_active else ("Spec" if st.mic_apo.rnnoise_enabled else "Off")
                apo_m_txt = f"ACTIVE ({rnn_lbl}) In:{st.mic_apo_peak_in:.2f} -> Out:{st.mic_apo_peak_out:.2f}"
            else:
                apo_m_txt = f"Bypassed (Raw Mic) In:{st.mic_apo_peak_in:.2f}"
            if self.lbl_mic_apo_meter.text() != apo_m_txt:
                self.lbl_mic_apo_meter.setText(apo_m_txt)

            mic_sig = (
                int(st.mic_apo.enabled),
                int(st.mic_apo.hpf_enabled),
                int(st.mic_apo.gate_enabled),
                round(float(st.mic_apo.gate_thresh_db), 0),
                int(st.mic_apo.rnnoise_enabled),
                int(st.mic_apo.comp_enabled),
                round(float(st.mic_apo.clarity_db), 1),
                int(st.mic_apo.sidetone_enabled),
                int(st.mic_apo.sidetone_pct),
            )
            if (initial or mic_sig != self._last_mic_apo_sig) and now_mono >= self._mic_apo_hold_until:
                self._last_mic_apo_sig = mic_sig
                self.updating_ui = True
                self.chk_mic_apo.setChecked(bool(st.mic_apo.enabled))
                self.chk_mic_hpf.setChecked(bool(st.mic_apo.hpf_enabled))
                self.chk_mic_gate.setChecked(bool(st.mic_apo.gate_enabled))
                self.spn_mic_gate.setValue(int(round(st.mic_apo.gate_thresh_db)))
                self.chk_mic_rnnoise.setChecked(bool(st.mic_apo.rnnoise_enabled))
                self.chk_mic_comp.setChecked(bool(st.mic_apo.comp_enabled))
                self.spn_mic_clarity.setValue(float(st.mic_apo.clarity_db))
                self.chk_mic_sidetone.setChecked(bool(st.mic_apo.sidetone_enabled))
                self.spn_mic_sidetone.setValue(int(st.mic_apo.sidetone_pct))
                self.updating_ui = False

        sink_name = st.hw_device.decode("utf-8", "ignore")
        mic_name = st.hw_capture_device.decode("utf-8", "ignore") or "Default Mic"
        midi_name = st.midi_device.decode("utf-8", "ignore") or "LWASAPI-MIDI-Hub"
        active_sink_id = st.active_playback_id.decode("utf-8", "ignore") or "auto"
        active_cap_id = st.active_capture_id.decode("utf-8", "ignore") or "auto"
        pa_tag = f"PA v35 ({st.pa_clients} clients)" if st.pa_server_running else "PA Socket Off"
        bt_codec = st.bt_codec.decode("utf-8", "ignore") or "SBC"
        bt_kbps = int(st.bt_codec_cfg.effective_kbps)
        bt_tag = f" • BT {bt_codec}@{bt_kbps}k ({st.bt_delay_ms}ms)" if st.bt_active else ""
        ep_txt = f"Sink: {sink_name} • Mic: {mic_name} • {pa_tag}{bt_tag}"
        if self.lbl_endpoints.text() != ep_txt:
            self.lbl_endpoints.setText(ep_txt)

        if hasattr(self, "lbl_bt_diag"):
            bt_reason = st.bt_codec_cfg.neg_reason.decode("utf-8", "ignore").strip()
            bt_diag_txt = (
                f"{bt_codec} @ {bt_kbps}k | {bt_reason}"
                if (st.bt_active or bt_reason)
                else "BT: Standby (No active BlueZ headset)"
            )
            if self.lbl_bt_diag.text() != bt_diag_txt:
                self.lbl_bt_diag.setText(bt_diag_txt)
            sup_mask = int(st.bt_codec_cfg.supported_codecs_mask)
            bt_sig = (
                int(st.bt_codec_cfg.preferred_codec),
                int(st.bt_codec_cfg.ldac_mode),
                int(st.bt_codec_cfg.wireless_abr_enabled),
                int(st.bt_codec_cfg.hfp_sco_enabled),
                sup_mask,
            )
            if (
                (initial or bt_sig != self._last_bt_cfg_sig)
                and now_mono >= self._bt_cfg_hold_until
                and not self.cmb_bt_codec.view().isVisible()
                and not self.cmb_ldac_mode.view().isVisible()
            ):
                self._last_bt_cfg_sig = bt_sig
                self.updating_ui = True
                for idx_i, (base_lbl, code_c) in enumerate(
                    (
                        ("Auto (Best)", LWASAPI_BT_PREF_AUTO),
                        ("LDAC", LWASAPI_BT_PREF_LDAC),
                        ("aptX HD", LWASAPI_BT_PREF_APTX_HD),
                        ("aptX", LWASAPI_BT_PREF_APTX),
                        ("AAC", LWASAPI_BT_PREF_AAC),
                        ("SBC", LWASAPI_BT_PREF_SBC),
                    )
                ):
                    lbl_i = (
                        f"{base_lbl} (N/A on Headset)"
                        if (code_c > 0 and sup_mask != 0 and not (sup_mask & (1 << (code_c - 1))))
                        else base_lbl
                    )
                    if idx_i < self.cmb_bt_codec.count() and self.cmb_bt_codec.itemText(idx_i) != lbl_i:
                        self.cmb_bt_codec.setItemText(idx_i, lbl_i)
                p_idx = self.cmb_bt_codec.findData(int(st.bt_codec_cfg.preferred_codec))
                if p_idx >= 0:
                    self.cmb_bt_codec.setCurrentIndex(p_idx)
                l_idx = self.cmb_ldac_mode.findData(int(st.bt_codec_cfg.ldac_mode))
                if l_idx >= 0:
                    self.cmb_ldac_mode.setCurrentIndex(l_idx)
                self.chk_bt_abr.setChecked(bool(st.bt_codec_cfg.wireless_abr_enabled))
                self.chk_bt_sco.setChecked(bool(st.bt_codec_cfg.hfp_sco_enabled))
                self.updating_ui = False

        if hasattr(self, "lbl_hw_analog_diag"):
            arch_s = st.hw_analog_info.card_arch_name.decode("utf-8", "ignore").strip("\x00 ") or "Standby"
            dac_s = st.hw_analog_info.dac_ctl_name.decode("utf-8", "ignore").strip("\x00 ")
            hid_s = " • Strix HID" if st.hw_analog_info.hid_box_connected else ""
            unity_s = f"DAC Unity ({dac_s})" if st.hw_analog_info.dac_unity_active else "SW/AVRCP Gain"
            hw_diag_txt = f"{arch_s[:28]} | {unity_s}{hid_s}"
            if self.lbl_hw_analog_diag.text() != hw_diag_txt:
                self.lbl_hw_analog_diag.setText(hw_diag_txt)
            hw_sig = (
                int(st.hw_analog_cfg.hw_dac_unity_vol),
                int(st.hw_analog_cfg.hp_amp_tier),
                int(st.hw_analog_cfg.output_route),
                int(st.hw_analog_cfg.unlock_0db_rails),
                round(float(st.hw_analog_cfg.analog_boost_db), 2),
                round(float(st.hw_analog_cfg.mic_hw_boost_db), 2),
            )
            if (
                (initial or hw_sig != self._last_hw_analog_sig)
                and now_mono >= self._hw_analog_hold_until
                and not self.cmb_hw_amp.view().isVisible()
                and not self.cmb_hw_route.view().isVisible()
                and not self.sld_hw_boost.isSliderDown()
                and not self.sld_hw_mic_boost.isSliderDown()
            ):
                self._last_hw_analog_sig = hw_sig
                self.updating_ui = True
                self.chk_hw_dac_unity.setChecked(bool(st.hw_analog_cfg.hw_dac_unity_vol))
                a_idx = self.cmb_hw_amp.findData(int(st.hw_analog_cfg.hp_amp_tier))
                if a_idx >= 0:
                    self.cmb_hw_amp.setCurrentIndex(a_idx)
                r_idx = self.cmb_hw_route.findData(int(st.hw_analog_cfg.output_route))
                if r_idx >= 0:
                    self.cmb_hw_route.setCurrentIndex(r_idx)
                self.chk_hw_rails.setChecked(bool(st.hw_analog_cfg.unlock_0db_rails))
                bst_val = max(0, min(24, int(round(float(st.hw_analog_cfg.analog_boost_db) * 2.0))))
                self.sld_hw_boost.setValue(bst_val)
                self.lbl_hw_boost.setText(f"+{float(st.hw_analog_cfg.analog_boost_db):.1f} dB")
                mic_bst_val = max(0, min(24, int(round(float(st.hw_analog_cfg.mic_hw_boost_db)))))
                self.sld_hw_mic_boost.setValue(mic_bst_val)
                self.lbl_hw_mic_boost.setText(f"+{float(st.hw_analog_cfg.mic_hw_boost_db):.1f} dB")
                self.updating_ui = False

        midi_txt = f"DAW MIDI Hub: {midi_name} ({st.midi_events_routed} key presses)"
        if self.lbl_midi_status.text() != midi_txt:
            self.lbl_midi_status.setText(midi_txt)

        self._dev_poll_ticks += 1
        if initial or self._last_dev_sig is None or (self._dev_poll_ticks % 20) == 0:
            self._refresh_devices(active_sink_id, active_cap_id, force=initial)
        self._maybe_switch_per_device_eq(active_sink_id, initial=initial)

        spk_mute_str = " [MUTED]" if st.desktop_muted else ""
        spk_txt = f"Speaker ({st.desktop_volume_pct}%{spk_mute_str}):"
        if not self.sld_spk.isSliderDown() and not self._spk_hold_timer.isActive():
            if self.lbl_spk.text() != spk_txt:
                self.lbl_spk.setText(spk_txt)
            if self.sld_spk.value() != st.desktop_volume_pct:
                self.sld_spk.blockSignals(True)
                self.sld_spk.setValue(st.desktop_volume_pct)
                self.sld_spk.blockSignals(False)

        mic_mute_str = " [MUTED]" if st.desktop_mic_muted else ""
        mic_txt = f"Mic Gain ({st.desktop_mic_volume_pct}%{mic_mute_str}):"
        if not self.sld_mic.isSliderDown() and not self._mic_hold_timer.isActive():
            if self.lbl_mic.text() != mic_txt:
                self.lbl_mic.setText(mic_txt)
            if self.sld_mic.value() != st.desktop_mic_volume_pct:
                self.sld_mic.blockSignals(True)
                self.sld_mic.setValue(st.desktop_mic_volume_pct)
                self.sld_mic.blockSignals(False)

        n_sess = min(st.num_sessions, LWASAPI_MAX_SESSIONS)
        sess_hdr = f"AUTO-RECOGNIZED FULL-DUPLEX SESSIONS ({n_sess})"
        if self.lbl_sess_hdr.text() != sess_hdr:
            self.lbl_sess_hdr.setText(sess_hdr)

        sess_entries = []
        for r in range(n_sess):
            s = st.sessions[r]
            sid = int(s.session_id)
            is_cap = bool(s.is_capture)
            pinned = bool(s.routing_flags & 1)
            tgt_dev = s.target_device.decode("utf-8", "ignore").strip() if pinned else "auto"
            if pinned and not tgt_dev:
                tgt_dev = "auto"
            sess_entries.append({
                "sid": sid,
                "is_cap": is_cap,
                "cols": (
                    f"#{sid}",
                    "🎙️ CAPT" if is_cap else "🔊 PLAY",
                    s.client_name.decode("utf-8", "ignore"),
                    "EXCL" if s.share_mode == 1 else "SHARED",
                    f"{s.sample_rate} Hz ({s.channels}ch)",
                ),
                "vol_pct": max(0, min(150, int(s.volume_milli // 10))),
                "muted": bool(s.muted),
                "tgt_dev": tgt_dev,
            })

        text_rows = [e["cols"] for e in sess_entries]
        dev_opts_sig = (
            tuple(d[0] for d in self._cached_sinks),
            tuple(d[0] for d in self._cached_sources),
        )
        struct_sig = (tuple((e["sid"], e["is_cap"]) for e in sess_entries), dev_opts_sig)
        interacting = any(
            w["sld"].isSliderDown() or w["cmb"].view().isVisible()
            for w in self._sess_widgets.values()
        )

        if struct_sig != self._last_sess_struct_sig and not interacting:
            self._last_sess_struct_sig = struct_sig
            self._last_sess_rows = text_rows
            self._sess_widgets = {}
            self.tbl_sessions.setRowCount(0)
            self.tbl_sessions.setRowCount(n_sess)
            for r, e in enumerate(sess_entries):
                sid = e["sid"]
                for c, val in enumerate(e["cols"]):
                    self.tbl_sessions.setItem(r, c, QTableWidgetItem(val))

                vol_box = QWidget()
                vol_lay = QHBoxLayout(vol_box)
                vol_lay.setContentsMargins(4, 2, 4, 2)
                vol_lay.setSpacing(4)
                sld = StepVolumeSlider(lambda: self._vol_step_pct)
                sld.setRange(0, 150)
                sld.setValue(e["vol_pct"])
                lbl = QLabel(f"{e['vol_pct']}%")
                lbl.setFixedWidth(36)
                lbl.setAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
                vol_lay.addWidget(sld, 1)
                vol_lay.addWidget(lbl)
                sld.valueChanged.connect(lambda v, s_id=sid, w_lbl=lbl: self._on_sess_vol_moved(s_id, v, w_lbl, False))
                sld.sliderReleased.connect(lambda s_id=sid, w_sld=sld, w_lbl=lbl: self._on_sess_vol_moved(s_id, w_sld.value(), w_lbl, True))
                self.tbl_sessions.setCellWidget(r, 5, vol_box)

                btn = QPushButton("MUTED" if e["muted"] else "Mute")
                btn.setCheckable(True)
                btn.setChecked(e["muted"])
                btn.setObjectName("CompactBtn")
                btn.setFixedHeight(24)
                btn.clicked.connect(lambda chk, s_id=sid, w_btn=btn: self._on_sess_mute_clicked(s_id, chk, w_btn))
                self.tbl_sessions.setCellWidget(r, 6, btn)

                cmb = QComboBox()
                items = list(self._cached_sources if e["is_cap"] else self._cached_sinks)
                if e["tgt_dev"] != "auto" and not any(did == e["tgt_dev"] for did, _ in items):
                    items.append((e["tgt_dev"], e["tgt_dev"]))
                sel_idx = 0
                for idx, (did, label) in enumerate(items):
                    cmb.addItem(label, did)
                    cmb.setItemData(idx, label, Qt.ItemDataRole.ToolTipRole)
                    if did == e["tgt_dev"]:
                        sel_idx = idx
                cmb.setCurrentIndex(sel_idx)
                cmb.setToolTip(cmb.currentText())
                cmb.activated.connect(lambda _idx, s_id=sid, w_cmb=cmb: self._on_sess_route_activated(s_id, w_cmb))
                self.tbl_sessions.setCellWidget(r, 7, cmb)

                self._sess_widgets[sid] = {
                    "sld": sld,
                    "lbl": lbl,
                    "btn": btn,
                    "cmb": cmb,
                    "hold_until": 0.0,
                }
        else:
            if self._last_sess_rows != text_rows and self.tbl_sessions.rowCount() == n_sess:
                self._last_sess_rows = text_rows
                for r, e in enumerate(sess_entries):
                    for c, val in enumerate(e["cols"]):
                        it = self.tbl_sessions.item(r, c)
                        if it is None or it.text() != val:
                            self.tbl_sessions.setItem(r, c, QTableWidgetItem(val))
            now = time.monotonic()
            for e in sess_entries:
                w = self._sess_widgets.get(e["sid"])
                if not w or now < w["hold_until"]:
                    continue
                if not w["sld"].isSliderDown() and w["sld"].value() != e["vol_pct"]:
                    w["sld"].blockSignals(True)
                    w["sld"].setValue(e["vol_pct"])
                    w["sld"].blockSignals(False)
                    w["lbl"].setText(f"{e['vol_pct']}%")
                if w["btn"].isChecked() != e["muted"]:
                    w["btn"].blockSignals(True)
                    w["btn"].setChecked(e["muted"])
                    w["btn"].setText("MUTED" if e["muted"] else "Mute")
                    w["btn"].blockSignals(False)
                if not w["cmb"].view().isVisible():
                    cur_did = w["cmb"].currentData()
                    if cur_did != e["tgt_dev"]:
                        idx = w["cmb"].findData(e["tgt_dev"])
                        if idx < 0 and e["tgt_dev"]:
                            w["cmb"].addItem(e["tgt_dev"], e["tgt_dev"])
                            idx = w["cmb"].count() - 1
                        if idx >= 0:
                            w["cmb"].blockSignals(True)
                            w["cmb"].setCurrentIndex(idx)
                            w["cmb"].setToolTip(w["cmb"].currentText())
                            w["cmb"].blockSignals(False)

    def _on_sess_vol_moved(self, sid: int, val: int, lbl: QLabel, force: bool = False):
        val = max(0, min(150, int(val)))
        lbl.setText(f"{val}%")
        now = time.monotonic()
        meta = self._sess_widgets.get(sid)
        if meta is not None:
            meta["hold_until"] = now + 0.45
        last_ts = self._sess_last_send_ts.get(sid, 0.0)
        if force or (now - last_ts) >= 0.04:
            self._sess_last_send_ts[sid] = now
            self._send_session_control(sid, vol_pct=val)

    def _on_sess_mute_clicked(self, sid: int, checked: bool, btn: QPushButton):
        btn.setText("MUTED" if checked else "Mute")
        self._send_session_control(sid, mute_mode=(1 if checked else 0))

    def _on_sess_route_activated(self, sid: int, cmb: QComboBox):
        tgt = cmb.currentData() or "auto"
        cmb.setToolTip(cmb.currentText())
        self._send_session_control(sid, target_device=str(tgt))

    def _sync_graph_anim_timer(self):
        active = bool(
            getattr(self, "intro_win", None) is not None
            and self.intro_win.isVisible()
            and not self.intro_win.isMinimized()
        )
        if hasattr(self, "graph_canvas"):
            self.graph_canvas.set_animation_active(active)

    def showEvent(self, event):
        super().showEvent(event)
        self._sync_graph_anim_timer()
        if hasattr(self, "timer") and not self.isMinimized() and self.timer.interval() != 80:
            self.timer.setInterval(80)
            self._poll_engine(initial=False)

    def hideEvent(self, event):
        super().hideEvent(event)
        self._sync_graph_anim_timer()

    def changeEvent(self, event):
        super().changeEvent(event)
        self._sync_graph_anim_timer()
        if hasattr(self, "timer") and self.isVisible() and not self.isMinimized() and self.timer.interval() != 80:
            self.timer.setInterval(80)
            self._poll_engine(initial=False)

    def closeEvent(self, event):
        self._closing_all = True
        if getattr(self, "_custom_theme_dlg", None) is not None:
            self._custom_theme_dlg.close()
        if not self.isMaximized() and not self.isFullScreen():
            self.gui_settings["window_width"] = max(640, int(self.width()))
            self.gui_settings["window_height"] = max(480, int(self.height()))
        save_gui_settings(self.gui_settings)
        if hasattr(self, "graph_canvas"):
            self.graph_canvas.set_animation_active(False)
        if getattr(self, "intro_win", None) is not None:
            self.intro_win.close()
        if getattr(self, "adv_win", None) is not None:
            self.adv_win.close()
        event.accept()


def main():
    start_debug = "--debug" in sys.argv or "--debugger" in sys.argv
    if start_debug and os.path.exists(DEBUG_SCRIPT):
        os.execv(sys.executable, [sys.executable, DEBUG_SCRIPT])
    app = QApplication(sys.argv)
    app.setApplicationName("Salsa — Linux WASAPI")
    app.setDesktopFileName("salsa-lwasapi")
    app.setWindowIcon(create_salsa_icon())
    start_graph = "--graph" in sys.argv or "--manual" in sys.argv or os.path.basename(sys.argv[0]) == "salsa-graph"
    win = SalsaStudioWindow(start_in_graph=start_graph, start_in_debug=False)
    show_intro = bool(win.gui_settings.get("show_topology_intro", True))
    if start_graph or (show_intro and "--no-intro" not in sys.argv):
        win._open_topology_intro()
    else:
        win._studio_entered = True
        win.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
