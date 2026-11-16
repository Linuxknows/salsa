#!/usr/bin/env python3
"""
Salsa Custom Theme Studio & Shared Stylesheet Engine
=====================================================
Provides the single-source QSS/QPalette theme builder shared by the main Salsa
Studio window and the separate Custom Theme Studio popup window, along with an
interactive miniature exact replica of the Salsa GUI that recolors live.
"""

import copy
import math
import os
import tempfile
from PyQt6.QtCore import Qt, QPointF, QRectF, QSize, QTimer, pyqtSignal
from PyQt6.QtGui import (
    QColor,
    QFont,
    QImage,
    QLinearGradient,
    QPainter,
    QPainterPath,
    QPalette,
    QPen,
)
from PyQt6.QtWidgets import (
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDialog,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QSlider,
    QTabWidget,
    QTableWidget,
    QTableWidgetItem,
    QTextBrowser,
    QVBoxLayout,
    QWidget,
)

CUSTOM_THEME_NAME = "Custom"

THEME_COLOR_KEYS = (
    "bg_main",
    "bg_card",
    "bg_deep",
    "bg_canvas",
    "bg_control",
    "bg_hover",
    "border_card",
    "border_control",
    "text_primary",
    "text_bright",
    "text_secondary",
    "text_muted",
    "text_dim",
    "accent_primary",
    "accent_secondary",
    "accent_hot",
    "accent_warn",
    "accent_mic",
    "accent_midi",
    "accent_out",
    "btn_primary_bg",
    "btn_primary_border",
    "btn_primary_hover",
    "btn_primary_text",
    "btn_accent_bg",
    "btn_accent_border",
    "btn_accent_hover",
    "btn_accent_text",
    "combo_popup_bg",
    "combo_popup_sel_bg",
    "combo_popup_sel_text",
    "canvas_grid",
    "spectrum_grid",
    "grid_zero",
    "node_bg_top",
    "node_bg_hover",
    "node_bg_bot",
    "node_border",
    "wire_pulse",
    "handle_border",
    "code_bg",
)

THEME_TOKEN_GROUPS = [
    (
        "Surfaces",
        [
            ("bg_main", "Main Window BG", "Outer window & tab canvas background"),
            ("bg_card", "Card Surface", "Header, EQ card, mixer & session panels"),
            ("bg_deep", "Deep Well / Saved Pill BG", "Tables, saved pills & log console"),
            ("bg_canvas", "Spectrum & Graph Canvas", "EQ spectrum & Auto-Graph backdrop"),
            ("bg_control", "Control / Button / Slider", "Combo boxes, buttons & slider grooves"),
            ("bg_hover", "Control Hover", "Hover highlight on buttons & dropdowns"),
            ("border_card", "Card Border", "Outer panel border"),
            ("border_control", "Control Border", "Inputs, saved pills & control outlines"),
            ("code_bg", "Manual Code Highlight", "Inline code background in Owner's Manual"),
        ],
    ),
    (
        "Accents",
        [
            ("accent_primary", "Primary Accent", "EQ curve, active badges & saved pills"),
            ("accent_secondary", "Secondary Accent", "Sub-banner, sliders & peak meter"),
            ("accent_hot", "Warm / Peak Accent", "Spectrum bar tops & EQ card accent"),
            ("accent_warn", "Dynamic Delta Accent", "Dotted dynamic EQ delta & Δ labels"),
            ("accent_mic", "Mic / Capture Accent", "Capture meters & error alerts"),
            ("accent_midi", "MIDI / Synth Accent", "MIDI hub graph nodes & traces"),
            ("accent_out", "Output / Sink Accent", "Final hardware sink stage accent"),
            ("handle_border", "Handle Rim Color", "Outline around EQ dots & slider knobs"),
            ("wire_pulse", "Signal Pulse Dot", "Moving topology animation dot"),
        ],
    ),
    (
        "Text / Btns",
        [
            ("text_bright", "Bright Headings", "Studio banner & control values"),
            ("text_primary", "Primary Text", "Standard body labels & table entries"),
            ("text_secondary", "Secondary Info", "Device endpoints & manual prose"),
            ("text_muted", "Muted Labels", "Frequency ticks & inactive tab text"),
            ("text_dim", "Dim Labels", "Spectrum dB scale markers"),
            ("btn_primary_bg", "Primary Button BG", "Engine toggle & Benchmark button"),
            ("btn_primary_border", "Primary Button Border", "Outline for primary buttons"),
            ("btn_primary_hover", "Primary Button Hover", "Hover fill for primary buttons"),
            ("btn_primary_text", "Primary Button Text", "Label color on primary buttons"),
            ("btn_accent_bg", "Accent Action BG", "Alert / action accent button fill"),
            ("btn_accent_border", "Accent Action Border", "Outline for accent buttons"),
            ("btn_accent_hover", "Accent Action Hover", "Hover fill for accent buttons"),
            ("btn_accent_text", "Accent Action Text", "Label color on accent buttons"),
        ],
    ),
    (
        "Popups / Grids",
        [
            ("combo_popup_bg", "Dropdown Popup BG", "Background of open combo lists"),
            ("combo_popup_sel_bg", "Dropdown Selected BG", "Highlighted row inside combo popup"),
            ("combo_popup_sel_text", "Dropdown Selected Text", "Text color on selected combo row"),
            ("canvas_grid", "Auto-Graph Grid Lines", "Subtle background grid in topology view"),
            ("spectrum_grid", "Spectrum dB Grid", "EQ spectrum grid lines"),
            ("grid_zero", "Spectrum 0 dB Line", "Center 0 dB reference line"),
            ("node_bg_top", "Topology Node Top", "Top gradient stop on graph boxes"),
            ("node_bg_hover", "Topology Node Hover", "Selected/hover node fill"),
            ("node_bg_bot", "Topology Node Bottom", "Bottom gradient stop on graph boxes"),
            ("node_border", "Topology Node Border", "Outline of Auto-Graph nodes"),
        ],
    ),
]


def _mix_hex(hex_a: str, hex_b: str, t: float) -> str:
    ca = QColor(hex_a)
    cb = QColor(hex_b)
    if not ca.isValid():
        ca = QColor("#111827")
    if not cb.isValid():
        cb = QColor("#00e676")
    t = max(0.0, min(1.0, float(t)))
    r = int(round(ca.red() * (1.0 - t) + cb.red() * t))
    g = int(round(ca.green() * (1.0 - t) + cb.green() * t))
    b = int(round(ca.blue() * (1.0 - t) + cb.blue() * t))
    return f"#{r:02x}{g:02x}{b:02x}"


def _valid_hex(val: object, default_hex: str) -> str:
    if isinstance(val, str):
        s = val.strip()
        if s and QColor.isValidColorName(s):
            return QColor(s).name(QColor.NameFormat.HexRgb).lower()
    return QColor(default_hex).name(QColor.NameFormat.HexRgb).lower()


def normalize_custom_theme(raw: dict | None, fallback: dict) -> dict:
    """Validate and fill all required keys for a custom theme dictionary."""
    out = copy.deepcopy(fallback)
    if isinstance(raw, dict):
        if "is_light" in raw:
            out["is_light"] = bool(raw["is_light"])
        for k in THEME_COLOR_KEYS:
            if k in raw:
                out[k] = _valid_hex(raw[k], fallback[k])
    out["is_light"] = bool(out.get("is_light", False))
    for k in THEME_COLOR_KEYS:
        out[k] = _valid_hex(out.get(k), fallback[k])
    return out


def derive_harmony_theme(base: dict) -> dict:
    """Generate cohesive surfaces, borders, grids, buttons, and text from hero colors."""
    out = copy.deepcopy(base)
    is_light = bool(out.get("is_light", False))
    bg = out["bg_main"]
    pri = out["accent_primary"]
    sec = out["accent_secondary"]
    hot = out["accent_hot"]

    if is_light:
        white = "#ffffff"
        ink = "#0f172a"
        out["bg_card"] = _mix_hex(bg, white, 0.72)
        out["bg_deep"] = _mix_hex(bg, white, 0.55)
        out["bg_canvas"] = _mix_hex(bg, white, 0.65)
        out["bg_control"] = _mix_hex(bg, ink, 0.08)
        out["bg_hover"] = _mix_hex(bg, ink, 0.16)
        out["border_card"] = _mix_hex(bg, ink, 0.18)
        out["border_control"] = _mix_hex(bg, pri, 0.35)
        out["text_bright"] = "#020617"
        out["text_primary"] = "#0f172a"
        out["text_secondary"] = "#334155"
        out["text_muted"] = "#475569"
        out["text_dim"] = "#64748b"
        out["combo_popup_bg"] = out["bg_card"]
        out["combo_popup_sel_bg"] = _mix_hex(out["bg_card"], pri, 0.20)
        out["combo_popup_sel_text"] = _mix_hex(pri, ink, 0.30)
        out["canvas_grid"] = _mix_hex(out["bg_canvas"], ink, 0.10)
        out["spectrum_grid"] = _mix_hex(out["bg_canvas"], ink, 0.12)
        out["grid_zero"] = _mix_hex(out["bg_canvas"], ink, 0.35)
        out["node_bg_top"] = out["bg_card"]
        out["node_bg_hover"] = _mix_hex(out["bg_card"], sec, 0.12)
        out["node_bg_bot"] = _mix_hex(bg, ink, 0.05)
        out["node_border"] = out["border_control"]
        out["wire_pulse"] = "#0f172a"
        out["handle_border"] = "#0f172a"
        out["code_bg"] = out["bg_control"]
    else:
        black = "#030408"
        white = "#f8fafc"
        out["bg_card"] = _mix_hex(bg, white, 0.06)
        out["bg_deep"] = _mix_hex(bg, black, 0.32)
        out["bg_canvas"] = _mix_hex(bg, black, 0.42)
        out["bg_control"] = _mix_hex(bg, pri, 0.10)
        out["bg_control"] = _mix_hex(out["bg_control"], white, 0.08)
        out["bg_hover"] = _mix_hex(out["bg_control"], white, 0.12)
        out["border_card"] = _mix_hex(bg, white, 0.14)
        out["border_control"] = _mix_hex(out["bg_control"], pri, 0.32)
        out["text_bright"] = "#f8fafc"
        out["text_primary"] = _mix_hex("#e2e8f0", sec, 0.08)
        out["text_secondary"] = _mix_hex("#cbd5e1", sec, 0.14)
        out["text_muted"] = _mix_hex("#94a3b8", bg, 0.15)
        out["text_dim"] = _mix_hex("#64748b", pri, 0.15)
        out["combo_popup_bg"] = out["bg_card"]
        out["combo_popup_sel_bg"] = _mix_hex(out["bg_control"], pri, 0.25)
        out["combo_popup_sel_text"] = pri
        out["canvas_grid"] = _mix_hex(out["bg_canvas"], white, 0.06)
        out["spectrum_grid"] = _mix_hex(out["bg_canvas"], white, 0.09)
        out["grid_zero"] = _mix_hex(out["bg_canvas"], pri, 0.28)
        out["node_bg_top"] = _mix_hex(out["bg_card"], white, 0.04)
        out["node_bg_hover"] = _mix_hex(out["bg_card"], sec, 0.14)
        out["node_bg_bot"] = out["bg_deep"]
        out["node_border"] = out["border_control"]
        out["wire_pulse"] = "#ffffff"
        out["handle_border"] = "#ffffff"
        out["code_bg"] = out["bg_control"]

    out["btn_primary_bg"] = _mix_hex(pri, "#0f172a", 0.36)
    out["btn_primary_border"] = pri
    out["btn_primary_hover"] = _mix_hex(pri, "#0f172a", 0.16)
    out["btn_primary_text"] = "#ffffff"
    out["btn_accent_bg"] = _mix_hex(hot, "#0f172a", 0.28)
    out["btn_accent_border"] = hot
    out["btn_accent_hover"] = _mix_hex(hot, "#ffffff", 0.12)
    out["btn_accent_text"] = "#ffffff"
    return out


def build_salsa_palette(t: dict) -> QPalette:
    """Build the QPalette matching a Salsa theme dictionary."""
    pal = QPalette()
    pal.setColor(QPalette.ColorRole.Window, QColor(t["bg_main"]))
    pal.setColor(QPalette.ColorRole.WindowText, QColor(t["text_primary"]))
    pal.setColor(QPalette.ColorRole.Base, QColor(t["bg_deep"]))
    pal.setColor(QPalette.ColorRole.AlternateBase, QColor(t["bg_card"]))
    pal.setColor(QPalette.ColorRole.Text, QColor(t["text_primary"]))
    pal.setColor(QPalette.ColorRole.Button, QColor(t["bg_control"]))
    pal.setColor(QPalette.ColorRole.ButtonText, QColor(t["text_bright"]))
    pal.setColor(QPalette.ColorRole.Highlight, QColor(t["combo_popup_sel_bg"]))
    pal.setColor(QPalette.ColorRole.HighlightedText, QColor(t["combo_popup_sel_text"]))
    pal.setColor(QPalette.ColorRole.ToolTipBase, QColor(t["bg_card"]))
    pal.setColor(QPalette.ColorRole.ToolTipText, QColor(t["text_primary"]))
    return pal


_CHECK_ICON_CACHE: dict[bool, str] = {}


def _ensure_check_icon(dark_ink: bool) -> str:
    """Return path to a cached 16x16 anti-aliased checkmark PNG for styled checkboxes."""
    cached = _CHECK_ICON_CACHE.get(dark_ink)
    if cached and os.path.exists(cached):
        return cached
    icon_path = os.path.join(
        tempfile.gettempdir(), f"salsa_theme_check_{'dark' if dark_ink else 'light'}.png"
    )
    if not os.path.exists(icon_path):
        img = QImage(16, 16, QImage.Format.Format_ARGB32)
        img.fill(Qt.GlobalColor.transparent)
        p = QPainter(img)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        pen = QPen(QColor("#071014" if dark_ink else "#ffffff"), 2.2)
        pen.setCapStyle(Qt.PenCapStyle.RoundCap)
        pen.setJoinStyle(Qt.PenJoinStyle.RoundJoin)
        p.setPen(pen)
        path = QPainterPath()
        path.moveTo(3.2, 8.4)
        path.lineTo(6.6, 11.8)
        path.lineTo(12.8, 4.4)
        p.drawPath(path)
        p.end()
        img.save(icon_path, "PNG")
    _CHECK_ICON_CACHE[dark_ink] = icon_path
    return icon_path


def build_salsa_stylesheet(t: dict, mini: bool = False) -> str:
    """
    Generate the exact QSS stylesheet for SalsaStudioWindow (mini=False) or for
    MiniSalsaPreview (mini=True) with scaled typography/padding so the live mini
    copy renders every visual control identically.
    """
    base_fs = 9 if mini else 12
    title_fs = 13 if mini else 18
    sub_fs = 8 if mini else 11
    sec_fs = 10 if mini else 13
    eq_chk_fs = 9 if mini else 13
    val_fs = 8 if mini else 11
    dyn_fs = 8 if mini else 10
    tab_fs = 9 if mini else 12
    tab_pad = "4px 10px" if mini else "8px 18px"
    ctl_pad = "2px 6px" if mini else "5px 10px"
    compact_pad = "2px 6px" if mini else "4px 8px"
    saved_pad = "2px 6px" if mini else "4px 8px"
    card_radius = 7 if mini else 10
    ctl_radius = 5 if mini else 6
    slider_h_w = 10 if mini else 14
    slider_v_h = 10 if mini else 14
    slider_groove = 4 if mini else 6
    slider_margin = -3 if mini else -5
    chk_sz = 12 if mini else 15
    chk_spacing = 5 if mini else 7
    use_dark_tick = QColor(t["accent_primary"]).lightness() > 145
    chk_icon_path = _ensure_check_icon(use_dark_tick).replace("\\", "/")

    return f"""
        QMainWindow, QWidget, QDialog {{
            background-color: {t['bg_main']};
            color: {t['text_primary']};
            font-family: 'Inter', 'Noto Sans', sans-serif;
            font-size: {base_fs}px;
        }}
        QFrame#MiniSalsaRoot {{
            background-color: {t['bg_main']};
            border: 2px solid {t['border_control']};
            border-radius: 10px;
        }}
        QLabel {{
            background-color: transparent;
            color: {t['text_primary']};
        }}
        QCheckBox {{
            background-color: transparent;
            color: {t['text_primary']};
            spacing: {chk_spacing}px;
        }}
        QCheckBox::indicator {{
            width: {chk_sz}px;
            height: {chk_sz}px;
            border: 1.5px solid {t['border_control']};
            border-radius: 4px;
            background-color: {t['bg_deep']};
        }}
        QCheckBox::indicator:hover {{
            border-color: {t['accent_secondary']};
        }}
        QCheckBox::indicator:checked {{
            background-color: {t['accent_primary']};
            border-color: {t['accent_primary']};
            image: url({chk_icon_path});
        }}
        QFrame#Card {{
            background-color: {t['bg_card']};
            border: 1px solid {t['border_card']};
            border-radius: {card_radius}px;
        }}
        QLabel#TitleBanner {{
            font-size: {title_fs}px;
            font-weight: 800;
            color: {t['text_bright']};
        }}
        QLabel#SubBanner {{
            font-size: {sub_fs}px;
            color: {t['accent_secondary']};
            font-weight: 600;
        }}
        QLabel#SectionHeader {{
            font-size: {sec_fs}px;
            font-weight: 700;
            color: {t['accent_primary']};
        }}
        QLabel#LatencyLabel {{
            color: {t['accent_primary']};
            font-weight: 700;
        }}
        QCheckBox#EqEnableCheck {{
            font-weight: 800;
            color: {t['accent_primary']};
            font-size: {eq_chk_fs}px;
        }}
        QLabel#PeaksLabel {{
            font-family: Monospace;
            font-weight: 700;
            color: {t['accent_secondary']};
        }}
        QLabel#BandValLabel {{
            font-weight: 700;
            color: {t['text_bright']};
            font-size: {val_fs}px;
        }}
        QLabel#BandDynLabel {{
            color: {t['accent_warn']};
            font-size: {dyn_fs}px;
            font-family: Monospace;
        }}
        QLabel#BandFreqLabel {{
            color: {t['text_muted']};
            font-weight: 700;
            font-size: {val_fs}px;
        }}
        QLabel#EndpointsLabel {{
            color: {t['text_secondary']};
            font-size: {val_fs}px;
        }}
        QLabel#AutoBadgeLabel {{
            color: {t['accent_secondary']};
            font-weight: 700;
            font-size: {val_fs}px;
        }}
        QLabel#SavedChipLabel {{
            background-color: {t['bg_deep']};
            color: {t['accent_primary']};
            border: 1px solid {t['border_control']};
            border-radius: {ctl_radius}px;
            padding: {saved_pad};
            font-weight: 700;
            font-size: {val_fs}px;
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
            padding: {tab_pad};
            margin-right: 4px;
            font-weight: 700;
            font-size: {tab_fs}px;
        }}
        QTabBar::tab:hover {{
            background-color: {t['bg_control']};
            color: {t['text_bright']};
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
            border-radius: {ctl_radius}px;
            padding: {ctl_pad};
            font-weight: 600;
        }}
        QPushButton#CompactBtn {{
            padding: {compact_pad};
            font-size: {val_fs}px;
        }}
        QComboBox:hover, QPushButton:hover {{
            background-color: {t['bg_hover']};
            border-color: {t['accent_secondary']};
        }}
        QComboBox QAbstractItemView {{
            background-color: {t['combo_popup_bg']};
            color: {t['text_bright']};
            border: 1px solid {t['accent_secondary']};
            selection-background-color: {t['combo_popup_sel_bg']};
            selection-color: {t['combo_popup_sel_text']};
            outline: 0;
            padding: 4px;
        }}
        QPushButton#PrimaryBtn {{
            background-color: {t['btn_primary_bg']};
            border-color: {t['btn_primary_border']};
            color: {t['btn_primary_text']};
            font-weight: 700;
        }}
        QPushButton#PrimaryBtn:hover {{
            background-color: {t['btn_primary_hover']};
        }}
        QPushButton#AccentBtn {{
            background-color: {t['btn_accent_bg']};
            border-color: {t['btn_accent_border']};
            color: {t['btn_accent_text']};
            font-weight: 700;
        }}
        QPushButton#AccentBtn:hover {{
            background-color: {t['btn_accent_hover']};
        }}
        QSlider::groove:vertical {{
            background: {t['bg_control']};
            width: {slider_groove}px;
            border-radius: 3px;
        }}
        QSlider::handle:vertical {{
            background: {t['accent_primary']};
            border: 2px solid {t['handle_border']};
            height: {slider_v_h}px;
            margin: 0 {slider_margin}px;
            border-radius: 5px;
        }}
        QSlider::groove:horizontal {{
            background: {t['bg_control']};
            height: {slider_groove}px;
            border-radius: 3px;
        }}
        QSlider::handle:horizontal {{
            background: {t['accent_secondary']};
            border: 2px solid {t['handle_border']};
            width: {slider_h_w}px;
            margin: {slider_margin}px 0;
            border-radius: 5px;
        }}
        QTableWidget, QTextBrowser, QTextEdit {{
            background-color: {t['bg_deep']};
            color: {t['text_primary']};
            border: 1px solid {t['border_control']};
            border-radius: 8px;
            gridline-color: {t['border_card']};
            padding: 4px;
        }}
        QTableWidget::item:selected {{
            background-color: {t['combo_popup_sel_bg']};
            color: {t['combo_popup_sel_text']};
        }}
        QHeaderView::section, QTableCornerButton::section {{
            background-color: {t['bg_control']};
            color: {t['text_muted']};
            padding: 3px;
            border: none;
            font-weight: 700;
        }}
        QTextBrowser#ManualBrowser {{
            font-size: {base_fs + 1}px;
            line-height: 1.4;
            color: {t['text_primary']};
        }}
        QScrollBar:vertical {{
            background: {t['bg_deep']};
            width: 8px;
            border-radius: 4px;
        }}
        QScrollBar::handle:vertical {{
            background: {t['border_control']};
            border-radius: 4px;
            min-height: 18px;
        }}
        QScrollBar::handle:vertical:hover {{
            background: {t['bg_hover']};
        }}
        QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical,
        QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {{
            height: 0px;
            width: 0px;
        }}
        QScrollBar:horizontal {{
            background: {t['bg_deep']};
            height: 8px;
            border-radius: 4px;
        }}
        QScrollBar::handle:horizontal {{
            background: {t['border_control']};
            border-radius: 4px;
            min-width: 18px;
        }}
    """


class MiniSpectrumCanvas(QWidget):
    """Exact miniature version of SalsaSpectrumCanvas reflecting all canvas theme tokens."""

    _BANDS = ("31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k")
    _GAINS = (2.5, 2.0, -1.5, -1.0, 0.5, 1.5, 2.5, 3.0, 3.5, 4.0)
    _DELTAS = (0.8, 0.6, 0.1, 0.0, 0.3, 0.5, 0.9, 1.1, 1.2, 1.0)
    _LEVELS = (-18.0, -14.0, -20.0, -24.0, -21.0, -19.0, -16.0, -17.0, -22.0, -26.0)

    def __init__(self, theme: dict, parent=None):
        super().__init__(parent)
        self.theme = copy.deepcopy(theme)
        self.setMinimumHeight(112)
        self.setMaximumHeight(130)

    def set_theme(self, theme: dict) -> None:
        self.theme = copy.deepcopy(theme)
        self.update()

    def _band_x(self, idx: int, w: float) -> float:
        pad = 36.0
        usable = max(80.0, w - pad * 2.0)
        return pad + (usable * idx) / (len(self._BANDS) - 1)

    def _gain_to_y(self, db: float, h: float) -> float:
        pad_top = 10.0
        pad_bot = 18.0
        usable = max(30.0, h - pad_top - pad_bot)
        clamped = max(-18.0, min(18.0, db))
        return pad_top + usable * (1.0 - (clamped + 18.0) / 36.0)

    def paintEvent(self, event) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = float(self.width()), float(self.height())
        t = self.theme

        p.setBrush(QColor(t["bg_canvas"]))
        p.setPen(QPen(QColor(t["border_card"]), 1.2))
        p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 7.0, 7.0)

        p.setFont(QFont("Monospace", 7))
        for db in (12, 0, -12):
            y = self._gain_to_y(float(db), h)
            p.setPen(
                QPen(
                    QColor(t["grid_zero"] if db == 0 else t["spectrum_grid"]),
                    1.1 if db == 0 else 0.9,
                    Qt.PenStyle.DashLine,
                )
            )
            p.drawLine(30, int(y), int(w - 10), int(y))
            p.setPen(QColor(t["text_dim"]))
            p.drawText(4, int(y + 3), f"{db:+d}")

        bar_w = max(8.0, (w - 84.0) / 20.0)
        base_y = h - 18.0
        c_top = QColor(t["accent_hot"])
        c_top.setAlpha(175)
        c_mid = QColor(t["accent_primary"])
        c_mid.setAlpha(135)
        c_bot = QColor(t["accent_secondary"])
        c_bot.setAlpha(65)

        stat_pts = []
        eff_pts = []
        for i in range(len(self._BANDS)):
            bx = self._band_x(i, w)
            lvl_norm = (self._LEVELS[i] + 60.0) / 66.0
            bar_h = max(3.0, lvl_norm * (h - 32.0))
            grad = QLinearGradient(bx, base_y - bar_h, bx, base_y)
            grad.setColorAt(0.0, c_top)
            grad.setColorAt(0.5, c_mid)
            grad.setColorAt(1.0, c_bot)
            p.setBrush(grad)
            p.setPen(Qt.PenStyle.NoPen)
            p.drawRoundedRect(QRectF(bx - bar_w * 0.5, base_y - bar_h, bar_w, bar_h), 3.0, 3.0)

            p.setPen(QColor(t["text_muted"]))
            p.drawText(QRectF(bx - 18.0, h - 16.0, 36.0, 14.0), Qt.AlignmentFlag.AlignCenter, self._BANDS[i])

            stat_pts.append(QPointF(bx, self._gain_to_y(self._GAINS[i], h)))
            eff_pts.append(QPointF(bx, self._gain_to_y(self._GAINS[i] + self._DELTAS[i], h)))

        dyn_path = QPainterPath()
        dyn_path.moveTo(eff_pts[0])
        for i in range(len(eff_pts) - 1):
            mid_x = 0.5 * (eff_pts[i].x() + eff_pts[i + 1].x())
            dyn_path.cubicTo(QPointF(mid_x, eff_pts[i].y()), QPointF(mid_x, eff_pts[i + 1].y()), eff_pts[i + 1])

        fill_path = QPainterPath(dyn_path)
        zero_y = self._gain_to_y(0.0, h)
        fill_path.lineTo(eff_pts[-1].x(), zero_y)
        fill_path.lineTo(eff_pts[0].x(), zero_y)
        fill_path.closeSubpath()
        f_top = QColor(t["accent_primary"])
        f_top.setAlpha(65)
        f_bot = QColor(t["accent_secondary"])
        f_bot.setAlpha(20)
        f_grad = QLinearGradient(0.0, 0.0, 0.0, h)
        f_grad.setColorAt(0.0, f_top)
        f_grad.setColorAt(1.0, f_bot)
        p.setBrush(f_grad)
        p.setPen(Qt.PenStyle.NoPen)
        p.drawPath(fill_path)

        p.setBrush(Qt.BrushStyle.NoBrush)
        p.setPen(QPen(QColor(t["accent_warn"]), 1.5, Qt.PenStyle.DotLine))
        p.drawPath(dyn_path)

        stat_path = QPainterPath()
        stat_path.moveTo(stat_pts[0])
        for i in range(len(stat_pts) - 1):
            mid_x = 0.5 * (stat_pts[i].x() + stat_pts[i + 1].x())
            stat_path.cubicTo(QPointF(mid_x, stat_pts[i].y()), QPointF(mid_x, stat_pts[i + 1].y()), stat_pts[i + 1])
        p.setPen(QPen(QColor(t["accent_primary"]), 2.2))
        p.drawPath(stat_path)

        h_pen = QPen(QColor(t["handle_border"]), 1.3)
        for i, pt in enumerate(stat_pts):
            p.setBrush(QColor(t["accent_hot"] if i == 6 else t["accent_primary"]))
            p.setPen(h_pen)
            p.drawEllipse(pt, 4.0, 4.0)


class MiniGraphCanvas(QWidget):
    """Exact miniature version of SalsaAutoGraphCanvas for Tab 2 live preview."""

    def __init__(self, theme: dict, parent=None):
        super().__init__(parent)
        self.theme = copy.deepcopy(theme)
        self.setMinimumHeight(175)

    def set_theme(self, theme: dict) -> None:
        self.theme = copy.deepcopy(theme)
        self.update()

    def paintEvent(self, event) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = float(self.width()), float(self.height())
        t = self.theme

        p.setBrush(QColor(t["bg_canvas"]))
        p.setPen(QPen(QColor(t["border_card"]), 1.2))
        p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 8.0, 8.0)

        p.setPen(QPen(QColor(t["canvas_grid"]), 1.0))
        step = 24
        for x in range(step, int(w), step):
            p.drawLine(x, 2, x, int(h - 2))
        for y in range(step, int(h), step):
            p.drawLine(2, y, int(w - 2), y)

        nodes = [
            ("Apps & Games", "accent_secondary", 0.03, 0.14, 0.25, 0.30, False),
            ("Mic Capture", "accent_mic", 0.03, 0.56, 0.25, 0.28, False),
            ("WaveRT Clock", "accent_primary", 0.36, 0.16, 0.28, 0.28, True),
            ("Dynamic EQ", "accent_hot", 0.36, 0.54, 0.28, 0.30, False),
            ("MIDI Hub", "accent_midi", 0.71, 0.16, 0.26, 0.28, False),
            ("Hardware Out", "accent_out", 0.71, 0.54, 0.26, 0.30, False),
        ]

        wires = [
            ((0.28, 0.29), (0.36, 0.30), "accent_secondary", 0.45),
            ((0.28, 0.70), (0.36, 0.69), "accent_mic", 0.60),
            ((0.50, 0.44), (0.50, 0.54), "accent_primary", 0.55),
            ((0.64, 0.69), (0.71, 0.69), "accent_hot", 0.50),
            ((0.64, 0.30), (0.71, 0.30), "accent_midi", 0.65),
        ]
        for (x1n, y1n), (x2n, y2n), ckey, pulse_t in wires:
            p1 = QPointF(x1n * w, y1n * h)
            p2 = QPointF(x2n * w, y2n * h)
            p.setPen(QPen(QColor(t[ckey]), 2.0))
            p.drawLine(p1, p2)
            dot = QPointF(p1.x() + (p2.x() - p1.x()) * pulse_t, p1.y() + (p2.y() - p1.y()) * pulse_t)
            p.setBrush(QColor(t["wire_pulse"]))
            p.setPen(QPen(QColor(t[ckey]), 1.2))
            p.drawEllipse(dot, 3.5, 3.5)

        p.setFont(QFont("Inter", 8, QFont.Weight.Bold))
        for title, ckey, rx, ry, rw, rh, hovered in nodes:
            rect = QRectF(rx * w, ry * h, rw * w, rh * h)
            grad = QLinearGradient(rect.topLeft(), rect.bottomLeft())
            grad.setColorAt(0.0, QColor(t["node_bg_hover"] if hovered else t["node_bg_top"]))
            grad.setColorAt(1.0, QColor(t["node_bg_bot"]))
            p.setBrush(grad)
            p.setPen(QPen(QColor(t[ckey] if hovered else t["node_border"]), 1.6 if hovered else 1.1))
            p.drawRoundedRect(rect, 7.0, 7.0)
            p.fillRect(QRectF(rect.left() + 6, rect.top() + 3, rect.width() - 12, 3), QColor(t[ckey]))
            p.setPen(QColor(t["text_bright"]))
            p.drawText(rect.adjusted(6, 8, -6, -4), Qt.AlignmentFlag.AlignCenter, title)


class MiniSalsaPreview(QFrame):
    """
    Miniature interactive copy of the real SalsaStudioWindow styled by the exact
    same build_salsa_stylesheet() function so color edits reflect identically.
    """

    _BAND_GAINS = (2.5, 2.0, -1.5, -1.0, 0.5, 1.5, 2.5, 3.0, 3.5, 4.0)
    _BAND_DELTAS = ("+0.8", "+0.6", "+0.1", "0.0", "+0.3", "+0.5", "+0.9", "+1.1", "+1.2", "+1.0")
    _BAND_FREQS = ("31Hz", "62Hz", "125Hz", "250Hz", "500Hz", "1kHz", "2kHz", "4kHz", "8kHz", "16kHz")

    def __init__(self, theme: dict, parent=None):
        super().__init__(parent)
        self.setObjectName("MiniSalsaRoot")
        self.theme = copy.deepcopy(theme)
        self._build_mini_ui()
        self.set_theme(self.theme)

    def _build_mini_ui(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(8, 8, 8, 8)
        root.setSpacing(6)

        # Top Bar Copy
        top_card = QFrame()
        top_card.setObjectName("Card")
        top_lay = QHBoxLayout(top_card)
        top_lay.setContentsMargins(8, 6, 8, 6)
        top_lay.setSpacing(6)

        t_col = QVBoxLayout()
        t_col.setSpacing(1)
        lbl_t = QLabel("SALSA — LINUX WASAPI STUDIO")
        lbl_t.setObjectName("TitleBanner")
        lbl_sub = QLabel("WaveRT Zero-Copy • 192 kHz / 32-Bit • 10-Band Dynamic EQ")
        lbl_sub.setObjectName("SubBanner")
        t_col.addWidget(lbl_t)
        t_col.addWidget(lbl_sub)
        top_lay.addLayout(t_col, 2)

        for hdr, items in (
            ("Rate:", ("48,000 Hz", "192,000 Hz")),
            ("Quantum:", ("256 frames", "512 frames")),
            ("Theme:", ("Custom", "Synthwave Outrun")),
        ):
            col = QVBoxLayout()
            col.setSpacing(1)
            col.addWidget(QLabel(hdr))
            cmb = QComboBox()
            cmb.addItems(items)
            col.addWidget(cmb)
            top_lay.addLayout(col)

        eng_col = QVBoxLayout()
        eng_col.setSpacing(2)
        lbl_lat = QLabel("5.33 ms | SCHED_FIFO")
        lbl_lat.setObjectName("LatencyLabel")
        btn_eng = QPushButton("SALSA NATIVE")
        btn_eng.setObjectName("PrimaryBtn")
        eng_col.addWidget(lbl_lat)
        eng_col.addWidget(btn_eng)
        top_lay.addLayout(eng_col)
        root.addWidget(top_card)

        # Mini Tabs (exact structure of Tab 1, Tab 2, Tab 3)
        self.tabs = QTabWidget()
        root.addWidget(self.tabs, 1)

        # Mini Tab 1: Studio Mixer & Dynamic EQ
        tab_eq = QWidget()
        eq_outer = QVBoxLayout(tab_eq)
        eq_outer.setContentsMargins(5, 5, 5, 5)
        eq_outer.setSpacing(5)

        eq_card = QFrame()
        eq_card.setObjectName("Card")
        eq_lay = QVBoxLayout(eq_card)
        eq_lay.setContentsMargins(7, 6, 7, 6)
        eq_lay.setSpacing(4)

        eq_bar = QHBoxLayout()
        eq_bar.setSpacing(4)
        chk_eq = QCheckBox("Dynamic EQ APO")
        chk_eq.setChecked(True)
        chk_eq.setObjectName("EqEnableCheck")
        eq_bar.addWidget(chk_eq)

        eq_bar.addWidget(QLabel("Preset:"))
        chip_p = QLabel("Saved: Bose QC Ultra 2")
        chip_p.setObjectName("SavedChipLabel")
        eq_bar.addWidget(chip_p)
        cmb_p = QComboBox()
        cmb_p.addItems(["Bose QC Ultra 2 Audiophile", "Flat / Studio Reference"])
        eq_bar.addWidget(cmb_p)

        eq_bar.addWidget(QLabel("Mode:"))
        chip_d = QLabel("Saved: Dynamic Expand")
        chip_d.setObjectName("SavedChipLabel")
        eq_bar.addWidget(chip_d)
        cmb_d = QComboBox()
        cmb_d.addItems(["Dynamic Expand", "Dynamic Compress"])
        eq_bar.addWidget(cmb_d)

        btn_flat = QPushButton("Reset Flat")
        btn_flat.setObjectName("CompactBtn")
        eq_bar.addWidget(btn_flat)
        eq_lay.addLayout(eq_bar)

        self.spectrum = MiniSpectrumCanvas(self.theme)
        eq_lay.addWidget(self.spectrum)

        dyn_row = QHBoxLayout()
        dyn_row.setSpacing(6)
        dyn_row.addWidget(QLabel("Preamp: -1.5 dB"))
        s_pre = QSlider(Qt.Orientation.Horizontal)
        s_pre.setRange(-120, 120)
        s_pre.setValue(-15)
        dyn_row.addWidget(s_pre)
        dyn_row.addWidget(QLabel("Thresh: -18 dB"))
        s_thr = QSlider(Qt.Orientation.Horizontal)
        s_thr.setRange(-480, 0)
        s_thr.setValue(-180)
        dyn_row.addWidget(s_thr)
        lbl_pk = QLabel("Peak L/R: -14.2 / -14.8 dBFS")
        lbl_pk.setObjectName("PeaksLabel")
        dyn_row.addWidget(lbl_pk)
        eq_lay.addLayout(dyn_row)

        f_grid = QGridLayout()
        f_grid.setSpacing(4)
        for i in range(10):
            col = QVBoxLayout()
            col.setSpacing(1)
            col.setAlignment(Qt.AlignmentFlag.AlignHCenter)
            lv = QLabel(f"{self._BAND_GAINS[i]:+.1f}")
            lv.setObjectName("BandValLabel")
            lv.setAlignment(Qt.AlignmentFlag.AlignCenter)
            sld = QSlider(Qt.Orientation.Vertical)
            sld.setRange(-180, 180)
            sld.setValue(int(self._BAND_GAINS[i] * 10))
            sld.setMinimumHeight(44)
            sld.setMaximumHeight(52)
            ld = QLabel(f"Δ{self._BAND_DELTAS[i]}")
            ld.setObjectName("BandDynLabel")
            ld.setAlignment(Qt.AlignmentFlag.AlignCenter)
            lf = QLabel(self._BAND_FREQS[i])
            lf.setObjectName("BandFreqLabel")
            lf.setAlignment(Qt.AlignmentFlag.AlignCenter)
            col.addWidget(lv)
            col.addWidget(sld, alignment=Qt.AlignmentFlag.AlignHCenter)
            col.addWidget(ld)
            col.addWidget(lf)
            f_grid.addLayout(col, 0, i)
        eq_lay.addLayout(f_grid)
        eq_outer.addWidget(eq_card)

        bot = QHBoxLayout()
        bot.setSpacing(6)

        hw_card = QFrame()
        hw_card.setObjectName("Card")
        hw_col = QVBoxLayout(hw_card)
        hw_col.setContentsMargins(7, 6, 7, 6)
        hw_col.setSpacing(4)
        lbl_hw = QLabel("FULL-DUPLEX MASTER MIXER & MIDI HUB")
        lbl_hw.setObjectName("SectionHeader")
        lbl_ep = QLabel("Sink: Bose QC Ultra 2 (AAC) • PA v35 Active")
        lbl_ep.setObjectName("EndpointsLabel")
        hw_col.addWidget(lbl_hw)
        hw_col.addWidget(lbl_ep)

        dev_row = QHBoxLayout()
        dev_row.setSpacing(4)
        dev_row.addWidget(QLabel("Sink:"))
        cmb_s = QComboBox()
        cmb_s.addItem("Auto (Bose QC Ultra 2)")
        dev_row.addWidget(cmb_s, 1)
        dev_row.addWidget(QLabel("Mic:"))
        cmb_m = QComboBox()
        cmb_m.addItem("Focusrite 2i2")
        dev_row.addWidget(cmb_m, 1)
        hw_col.addLayout(dev_row)

        spk_row = QHBoxLayout()
        spk_row.addWidget(QLabel("Spk (80%):"))
        s_spk = QSlider(Qt.Orientation.Horizontal)
        s_spk.setRange(0, 150)
        s_spk.setValue(80)
        spk_row.addWidget(s_spk)
        btn_mute = QPushButton("Mute")
        btn_mute.setObjectName("CompactBtn")
        spk_row.addWidget(btn_mute)
        hw_col.addLayout(spk_row)

        mic_row = QHBoxLayout()
        mic_row.addWidget(QLabel("Mic (100%):"))
        s_mic = QSlider(Qt.Orientation.Horizontal)
        s_mic.setRange(0, 150)
        s_mic.setValue(100)
        mic_row.addWidget(s_mic)
        btn_rev = QPushButton("Revert")
        btn_rev.setObjectName("AccentBtn")
        btn_rev.setFixedHeight(20)
        mic_row.addWidget(btn_rev)
        hw_col.addLayout(mic_row)
        hw_col.addStretch(1)
        bot.addWidget(hw_card, 11)

        sess_card = QFrame()
        sess_card.setObjectName("Card")
        sess_col = QVBoxLayout(sess_card)
        sess_col.setContentsMargins(7, 6, 7, 6)
        sess_col.setSpacing(4)
        lbl_sh = QLabel("ACTIVE SESSIONS (2)")
        lbl_sh.setObjectName("SectionHeader")
        sess_col.addWidget(lbl_sh)
        tbl = QTableWidget(2, 4)
        tbl.setHorizontalHeaderLabels(["Dir", "Application", "Mode", "Vol"])
        hdr = tbl.horizontalHeader()
        hdr.setMinimumSectionSize(28)
        hdr.setSectionResizeMode(0, QHeaderView.ResizeMode.ResizeToContents)
        hdr.setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        hdr.setSectionResizeMode(2, QHeaderView.ResizeMode.ResizeToContents)
        hdr.setSectionResizeMode(3, QHeaderView.ResizeMode.ResizeToContents)
        tbl.verticalHeader().setVisible(False)
        tbl.verticalHeader().setDefaultSectionSize(22)
        tbl.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        tbl.setWordWrap(False)
        tbl.setFixedHeight(74)
        for r, vals in enumerate((("PLAY", "Steam / Proton", "SHARED", "100%"), ("CAPT", "Focusrite 2i2", "EXCL", "100%"))):
            for c, txt in enumerate(vals):
                tbl.setItem(r, c, QTableWidgetItem(txt))
        tbl.selectRow(0)
        sess_col.addWidget(tbl)
        sess_col.addStretch(1)
        bot.addWidget(sess_card, 10)

        eq_outer.addLayout(bot)
        self.tabs.addTab(tab_eq, "🎛️ Studio Mixer && EQ")

        # Mini Tab 2: Auto-Graph Topology & Owner's Manual
        tab_graph = QWidget()
        g_lay = QVBoxLayout(tab_graph)
        g_lay.setContentsMargins(6, 6, 6, 6)
        g_lay.setSpacing(5)
        g_card = QFrame()
        g_card.setObjectName("Card")
        g_box = QVBoxLayout(g_card)
        g_box.setContentsMargins(7, 6, 7, 6)
        g_hdr = QHBoxLayout()
        lbl_gt = QLabel("AUTOMATIC SIGNAL FLOW TOPOLOGY")
        lbl_gt.setObjectName("SectionHeader")
        lbl_ab = QLabel("✔ AUTO-WIRING ACTIVE")
        lbl_ab.setObjectName("AutoBadgeLabel")
        g_hdr.addWidget(lbl_gt)
        g_hdr.addStretch()
        g_hdr.addWidget(lbl_ab)
        g_box.addLayout(g_hdr)
        self.graph = MiniGraphCanvas(self.theme)
        g_box.addWidget(self.graph)
        g_lay.addWidget(g_card, 2)

        self.mini_manual = QTextBrowser()
        self.mini_manual.setObjectName("ManualBrowser")
        self.mini_manual.setMaximumHeight(70)
        g_lay.addWidget(self.mini_manual, 1)
        self.tabs.addTab(tab_graph, "🕸️ Auto-Graph && Manual")

        # Mini Tab 3: Live Debugger
        tab_dbg = QWidget()
        d_lay = QVBoxLayout(tab_dbg)
        d_lay.setContentsMargins(6, 6, 6, 6)
        d_card = QFrame()
        d_card.setObjectName("Card")
        d_box = QVBoxLayout(d_card)
        d_box.setContentsMargins(7, 6, 7, 6)
        d_hdr = QLabel("LIVE INTERCEPTION DEBUGGER PREVIEW")
        d_hdr.setObjectName("SectionHeader")
        d_box.addWidget(d_hdr)
        self.mini_dbg_log = QTextBrowser()
        self.mini_dbg_log.setMaximumHeight(165)
        d_box.addWidget(self.mini_dbg_log)
        d_lay.addWidget(d_card)
        self.tabs.addTab(tab_dbg, "🔬 Debugger")

    def set_theme(self, theme: dict) -> None:
        self.theme = copy.deepcopy(theme)
        t = self.theme
        self.setPalette(build_salsa_palette(t))
        self.setStyleSheet(build_salsa_stylesheet(t, mini=True))
        self.spectrum.set_theme(t)
        self.graph.set_theme(t)
        self.mini_manual.document().setDefaultStyleSheet(
            f"body, p {{ color: {t['text_primary']}; font-size: 9px; }} "
            f"code {{ background-color: {t['code_bg']}; color: {t['accent_secondary']}; padding: 1px 3px; border-radius: 3px; }}"
        )
        self.mini_manual.setHtml(
            f"<p><b style='color:{t['accent_primary']}'>Zero Manual Patching:</b> "
            f"<code>lwasapi-audiodg</code> routes WaveRT Audio + <code>Dynamic EQ</code> + <code>MIDI Hub</code> automatically.</p>"
        )
        self.mini_dbg_log.setHtml(
            f"<div style='font-family:Monospace; font-size:9px;'>"
            f"<div><span style='color:{t['text_muted']}'>[05:18:10]</span> "
            f"<span style='color:{t['accent_primary']}; font-weight:700;'>ENGINE INFO</span> "
            f"<span style='color:{t['text_primary']}'>Quantum 256f (48kHz/32b) XRUNs=0</span></div>"
            f"<div><span style='color:{t['text_muted']}'>[05:18:11]</span> "
            f"<span style='color:{t['accent_midi']}; font-weight:700;'>MIDI TRACE</span> "
            f"<span style='color:{t['text_secondary']}'>FLkey 37 routed on 128:0</span></div>"
            f"<div><span style='color:{t['text_muted']}'>[05:18:12]</span> "
            f"<span style='color:{t['accent_warn']}; font-weight:700;'>BT WARN</span> "
            f"<span style='color:{t['text_primary']}'>Bose QC Ultra 2 AAC delay=220ms</span></div>"
            f"<div><span style='color:{t['text_muted']}'>[05:18:13]</span> "
            f"<span style='color:{t['accent_hot']}; font-weight:700;'>EQ SET</span> "
            f"<span style='color:{t['text_bright']}'>Bose QC Ultra 2 Audiophile (Expand)</span></div>"
            f"<div><span style='color:{t['text_muted']}'>[05:18:14]</span> "
            f"<span style='color:{t['accent_mic']}; font-weight:700;'>CAPTURE</span> "
            f"<span style='color:{t['text_secondary']}'>Focusrite Scarlett 2i2 ready</span></div>"
            f"</div>"
        )


class ColorSwatchRow(QWidget):
    """Single theme token editor row with live color swatch button + hex input."""

    color_changed = pyqtSignal(str, str)

    def __init__(self, key: str, label: str, hint: str, initial_hex: str, parent=None):
        super().__init__(parent)
        self.key = key
        self._hex = initial_hex
        self._syncing = False

        lay = QHBoxLayout(self)
        lay.setContentsMargins(2, 2, 2, 2)
        lay.setSpacing(6)

        self.btn_swatch = QPushButton()
        self.btn_swatch.setFixedSize(28, 22)
        self.btn_swatch.setToolTip(f"{hint} — click to open live color wheel")
        self.btn_swatch.clicked.connect(self._open_picker)
        lay.addWidget(self.btn_swatch)

        lbl_col = QVBoxLayout()
        lbl_col.setContentsMargins(0, 0, 0, 0)
        lbl_col.setSpacing(0)
        lbl_title = QLabel(label)
        lbl_title.setStyleSheet("font-weight: 600;")
        lbl_key = QLabel(hint)
        lbl_key.setStyleSheet("font-size: 10px; opacity: 0.72;")
        lbl_col.addWidget(lbl_title)
        lbl_col.addWidget(lbl_key)
        lay.addLayout(lbl_col, 1)

        self.txt_hex = QLineEdit(self._hex)
        self.txt_hex.setFixedWidth(74)
        self.txt_hex.setMaxLength(9)
        self.txt_hex.setToolTip(f"Hex color for {key}")
        self.txt_hex.textEdited.connect(self._on_hex_edited)
        lay.addWidget(self.txt_hex)

        self._update_swatch_visual()

    def set_hex(self, hex_color: str) -> None:
        clean = _valid_hex(hex_color, self._hex)
        self._hex = clean
        self._syncing = True
        if self.txt_hex.text().lower() != clean:
            self.txt_hex.setText(clean)
        self._syncing = False
        self._update_swatch_visual()

    def _update_swatch_visual(self) -> None:
        c = QColor(self._hex)
        border = "#ffffff" if c.lightness() < 128 else "#0f172a"
        self.btn_swatch.setStyleSheet(
            f"background-color: {self._hex}; border: 1px solid {border}; border-radius: 4px; padding: 0px;"
        )

    def _on_hex_edited(self, txt: str) -> None:
        if self._syncing:
            return
        s = txt.strip()
        if len(s) == 7 and s.startswith("#") and QColor.isValidColorName(s):
            clean = QColor(s).name(QColor.NameFormat.HexRgb).lower()
            self._hex = clean
            self._update_swatch_visual()
            self.color_changed.emit(self.key, clean)

    def _open_picker(self) -> None:
        orig_hex = self._hex
        dlg = QColorDialog(QColor(orig_hex), self)
        dlg.setWindowTitle(f"Pick Color — {self.key}")
        dlg.setOption(QColorDialog.ColorDialogOption.DontUseNativeDialog, True)

        def _on_live_color(col: QColor) -> None:
            if col.isValid():
                clean = col.name(QColor.NameFormat.HexRgb).lower()
                self.set_hex(clean)
                self.color_changed.emit(self.key, clean)

        dlg.currentColorChanged.connect(_on_live_color)
        if dlg.exec() == QDialog.DialogCode.Accepted:
            chosen = dlg.selectedColor()
            if chosen.isValid():
                clean = chosen.name(QColor.NameFormat.HexRgb).lower()
                self.set_hex(clean)
                self.color_changed.emit(self.key, clean)
        else:
            self.set_hex(orig_hex)
            self.color_changed.emit(self.key, orig_hex)


class SalsaCustomThemeDialog(QDialog):
    """
    Separate popup Custom Theme Studio displaying color controls on the left
    and an interactive mini exact copy of the Salsa GUI on the right that
    live updates colors as choices change.
    """

    live_theme_changed = pyqtSignal(dict)
    theme_applied = pyqtSignal(dict)
    mirror_stopped = pyqtSignal()

    def __init__(self, initial_theme: dict, preset_themes: dict, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Salsa Custom Theme Studio — Live Mini GUI Preview")
        self.resize(1180, 720)
        self.preset_themes = preset_themes
        default_fallback = next(iter(preset_themes.values()))
        self._working_theme = normalize_custom_theme(initial_theme, default_fallback)
        self._initial_snapshot = copy.deepcopy(self._working_theme)
        self._swatch_rows: dict[str, ColorSwatchRow] = {}

        self._preview_timer = QTimer(self)
        self._preview_timer.setSingleShot(True)
        self._preview_timer.setInterval(16)
        self._preview_timer.timeout.connect(self._flush_preview_update)

        self._build_dialog_ui()
        self._refresh_all_controls_from_theme()
        self._flush_preview_update()

    def result_theme(self) -> dict:
        return copy.deepcopy(self._working_theme)

    def reset_for_open(self, initial_theme: dict) -> None:
        default_fallback = next(iter(self.preset_themes.values()))
        self._working_theme = normalize_custom_theme(initial_theme, default_fallback)
        self._initial_snapshot = copy.deepcopy(self._working_theme)
        self._refresh_all_controls_from_theme()
        self._flush_preview_update()

    def _build_dialog_ui(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(12, 10, 12, 10)
        root.setSpacing(10)

        # Quick Seed Bar
        top_row = QHBoxLayout()
        top_row.setSpacing(8)
        top_row.addWidget(QLabel("Clone Base Palette:"))
        self.cmb_clone = QComboBox()
        for name in self.preset_themes:
            self.cmb_clone.addItem(name)
        top_row.addWidget(self.cmb_clone)

        btn_clone = QPushButton("Load Base")
        btn_clone.clicked.connect(self._clone_selected_base)
        top_row.addWidget(btn_clone)

        btn_autoshade = QPushButton("Auto-Shade Supporting Colors")
        btn_autoshade.setToolTip(
            "Derives cards, borders, grids, node gradients, and buttons harmoniously from your Window BG and Primary/Secondary/Hot accents"
        )
        btn_autoshade.clicked.connect(self._apply_auto_shade)
        top_row.addWidget(btn_autoshade)

        self.chk_light = QCheckBox("Light Mode Polarity")
        self.chk_light.toggled.connect(self._on_light_toggled)
        top_row.addWidget(self.chk_light)

        top_row.addStretch(1)
        self.chk_mirror_main = QCheckBox("Mirror Live on Main Salsa Window")
        self.chk_mirror_main.setChecked(False)
        self.chk_mirror_main.toggled.connect(self._on_mirror_toggled)
        top_row.addWidget(self.chk_mirror_main)
        root.addLayout(top_row)

        # Split Content: Left = Color Pickers, Right = Mini Exact GUI Copy
        body = QHBoxLayout()
        body.setSpacing(12)

        left_box = QVBoxLayout()
        left_box.setSpacing(6)
        self.token_tabs = QTabWidget()
        self.token_tabs.setUsesScrollButtons(False)
        self.token_tabs.setMinimumWidth(420)
        self.token_tabs.setMaximumWidth(475)
        self.token_tabs.setStyleSheet("QTabBar::tab { padding: 5px 11px; font-size: 11px; }")
        for group_name, entries in THEME_TOKEN_GROUPS:
            scroll = QScrollArea()
            scroll.setWidgetResizable(True)
            scroll.setFrameShape(QFrame.Shape.NoFrame)
            page = QWidget()
            page_lay = QVBoxLayout(page)
            page_lay.setContentsMargins(6, 6, 6, 6)
            page_lay.setSpacing(4)
            for key, label, hint in entries:
                row = ColorSwatchRow(key, label, hint, self._working_theme[key])
                row.color_changed.connect(self._on_token_color_changed)
                self._swatch_rows[key] = row
                page_lay.addWidget(row)
            page_lay.addStretch(1)
            scroll.setWidget(page)
            self.token_tabs.addTab(scroll, group_name)
        left_box.addWidget(self.token_tabs, 1)
        body.addLayout(left_box)

        right_card = QFrame()
        right_card.setObjectName("Card")
        right_lay = QVBoxLayout(right_card)
        right_lay.setContentsMargins(10, 8, 10, 10)
        right_lay.setSpacing(6)

        prev_hdr = QHBoxLayout()
        lbl_prev = QLabel("LIVE MINI EXACT COPY OF SALSA GUI (INTERACTIVE)")
        lbl_prev.setObjectName("SectionHeader")
        prev_hdr.addWidget(lbl_prev)
        prev_hdr.addStretch(1)
        self.lbl_swatch_strip = QLabel("● ● ● ● ●")
        self.lbl_swatch_strip.setStyleSheet("font-size: 13px; letter-spacing: 2px;")
        prev_hdr.addWidget(self.lbl_swatch_strip)
        right_lay.addLayout(prev_hdr)

        self.mini_preview = MiniSalsaPreview(self._working_theme)
        right_lay.addWidget(self.mini_preview, 1)
        body.addWidget(right_card, 1)

        root.addLayout(body, 1)

        # Bottom Actions
        act_row = QHBoxLayout()
        act_row.setSpacing(8)
        btn_reset = QPushButton("Reset Changes")
        btn_reset.clicked.connect(self._reset_to_snapshot)
        act_row.addWidget(btn_reset)
        act_row.addStretch(1)

        btn_cancel = QPushButton("Cancel")
        btn_cancel.clicked.connect(self.reject)
        act_row.addWidget(btn_cancel)

        btn_apply = QPushButton("Apply")
        btn_apply.clicked.connect(self._on_apply_clicked)
        act_row.addWidget(btn_apply)

        btn_save = QPushButton("Save && Apply Custom Theme")
        btn_save.setObjectName("PrimaryBtn")
        btn_save.clicked.connect(self._on_save_and_close)
        act_row.addWidget(btn_save)
        root.addLayout(act_row)

    def _refresh_all_controls_from_theme(self) -> None:
        self.chk_light.blockSignals(True)
        self.chk_light.setChecked(bool(self._working_theme.get("is_light", False)))
        self.chk_light.blockSignals(False)
        for key, row in self._swatch_rows.items():
            row.set_hex(self._working_theme[key])

    def _on_light_toggled(self, checked: bool) -> None:
        self._working_theme["is_light"] = bool(checked)
        self._schedule_preview_update()

    def _on_token_color_changed(self, key: str, hex_color: str) -> None:
        self._working_theme[key] = hex_color
        self._schedule_preview_update()

    def _clone_selected_base(self) -> None:
        name = self.cmb_clone.currentText()
        if name in self.preset_themes:
            self._working_theme = copy.deepcopy(self.preset_themes[name])
            self._refresh_all_controls_from_theme()
            self._flush_preview_update()

    def _apply_auto_shade(self) -> None:
        self._working_theme = derive_harmony_theme(self._working_theme)
        self._refresh_all_controls_from_theme()
        self._flush_preview_update()

    def _reset_to_snapshot(self) -> None:
        self._working_theme = copy.deepcopy(self._initial_snapshot)
        self._refresh_all_controls_from_theme()
        self._flush_preview_update()

    def _on_mirror_toggled(self, checked: bool) -> None:
        if checked:
            self.live_theme_changed.emit(self.result_theme())
        else:
            self.mirror_stopped.emit()

    def _schedule_preview_update(self) -> None:
        if not self._preview_timer.isActive():
            self._preview_timer.start()

    def _flush_preview_update(self) -> None:
        t = self._working_theme
        self.mini_preview.set_theme(t)
        self.lbl_swatch_strip.setText(
            f"<span style='color:{t['bg_card']}'>■</span> "
            f"<span style='color:{t['accent_primary']}'>●</span> "
            f"<span style='color:{t['accent_secondary']}'>●</span> "
            f"<span style='color:{t['accent_hot']}'>●</span> "
            f"<span style='color:{t['accent_warn']}'>●</span> "
            f"<span style='color:{t['accent_midi']}'>●</span>"
        )
        if self.chk_mirror_main.isChecked():
            self.live_theme_changed.emit(self.result_theme())

    def _on_apply_clicked(self) -> None:
        self._initial_snapshot = copy.deepcopy(self._working_theme)
        self.theme_applied.emit(self.result_theme())

    def _on_save_and_close(self) -> None:
        self._initial_snapshot = copy.deepcopy(self._working_theme)
        self.theme_applied.emit(self.result_theme())
        self.accept()
