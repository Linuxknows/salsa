#!/usr/bin/env python3
"""
Salsa — KDE Plasma 6 System Tray, Swipe/Scroll Volume & Mic Control, Floating OSD & Quick Mixer
Direct engine telemetry & control via liblwasapi.so + binary IPC + org.kde.StatusNotifierItem.
"""

import ctypes
import json
import os
import re
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time

from PyQt6 import QtCore, QtDBus, QtGui, QtWidgets

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if SCRIPT_DIR not in sys.path:
    sys.path.insert(0, SCRIPT_DIR)
try:
    from salsa_theme_editor import normalize_custom_theme
except Exception:
    def normalize_custom_theme(data):
        return dict(data) if isinstance(data, dict) else {}

LIB_PATH = os.path.join(SCRIPT_DIR, "build", "liblwasapi.so")
CTL_BIN = os.path.join(SCRIPT_DIR, "build", "lwasapi-ctl")
SALSA_GUI = os.path.join(SCRIPT_DIR, "salsa-gui.py")
SALSA_DEBUG = os.path.join(SCRIPT_DIR, "salsa-debug.py")
ICON_PATH = os.path.join(SCRIPT_DIR, "assets", "salsa-lwasapi.png")
ICON_GLOW_PATH = os.path.join(SCRIPT_DIR, "assets", "salsa-lwasapi-glow.png")

_REAL_DIR = os.path.dirname(os.path.realpath(__file__))
if _REAL_DIR not in sys.path:
    sys.path.insert(0, _REAL_DIR)

from salsa_ipc import (  # noqa: E402
    LWASAPI_CMD_SET_VOLUME,
    LWASAPI_EQ_BANDS,
    LWASAPI_MAX_DEVICES,
    LWASAPI_MAX_SESSIONS,
    MIC_APO_PRESETS,
    PRESETS,
    SUPPORTED_SAMPLE_RATES,
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
)

THEMES = {
    "Salsa Midnight (Default)": {
        "is_light": False, "bg_main": "#090d16", "bg_card": "#111827", "bg_deep": "#0b1120",
        "bg_control": "#1e293b", "bg_hover": "#334155", "border_card": "#1f2937",
        "border_control": "#334155", "text_primary": "#e2e8f0", "text_bright": "#f8fafc",
        "text_secondary": "#cbd5e1", "text_muted": "#94a3b8", "accent_primary": "#00e676",
        "accent_secondary": "#38bdf8", "accent_warn": "#f59e0b", "accent_mic": "#f43f5e",
        "btn_primary_bg": "#059669", "btn_primary_border": "#10b981", "btn_primary_hover": "#10b981",
    },
    "Plasma Breeze Emerald": {
        "is_light": False, "bg_main": "#161b1f", "bg_card": "#1e2429", "bg_deep": "#181d22",
        "bg_control": "#252c33", "bg_hover": "#2f3841", "border_card": "#2c343c",
        "border_control": "#36404a", "text_primary": "#e6edf3", "text_bright": "#f8fafc",
        "text_secondary": "#cbd5e1", "text_muted": "#9ca3af", "accent_primary": "#14b866",
        "accent_secondary": "#38bdf8", "accent_warn": "#f59e0b", "accent_mic": "#f43f5e",
        "btn_primary_bg": "#10b981", "btn_primary_border": "#14b866", "btn_primary_hover": "#14b866",
    },
    "CachyOS Teal Void": {
        "is_light": False, "bg_main": "#0a0e14", "bg_card": "#111722", "bg_deep": "#080b10",
        "bg_control": "#192231", "bg_hover": "#243247", "border_card": "#1c2738",
        "border_control": "#2b3c54", "text_primary": "#e2f1f8", "text_bright": "#f8fafc",
        "text_secondary": "#bfd7e6", "text_muted": "#85a1b8", "accent_primary": "#00d4aa",
        "accent_secondary": "#22d3ee", "accent_warn": "#fbbf24", "accent_mic": "#fb7185",
        "btn_primary_bg": "#0d9488", "btn_primary_border": "#14b8a6", "btn_primary_hover": "#14b8a6",
    },
    "Nordic Frost (Nord)": {
        "is_light": False, "bg_main": "#242933", "bg_card": "#2e3440", "bg_deep": "#1f242d",
        "bg_control": "#3b4252", "bg_hover": "#4c566a", "border_card": "#3b4252",
        "border_control": "#4c566a", "text_primary": "#e5e9f0", "text_bright": "#eceff4",
        "text_secondary": "#d8dee9", "text_muted": "#9aa5ba", "accent_primary": "#a3be8c",
        "accent_secondary": "#88c0d0", "accent_warn": "#ebcb8b", "accent_mic": "#bf616a",
        "btn_primary_bg": "#4b7a62", "btn_primary_border": "#a3be8c", "btn_primary_hover": "#5c9377",
    },
    "Catppuccin Mocha": {
        "is_light": False, "bg_main": "#181825", "bg_card": "#1e1e2e", "bg_deep": "#11111b",
        "bg_control": "#313244", "bg_hover": "#45475a", "border_card": "#313244",
        "border_control": "#45475a", "text_primary": "#cdd6f4", "text_bright": "#f5e0dc",
        "text_secondary": "#bac2de", "text_muted": "#9399b2", "accent_primary": "#a6e3a1",
        "accent_secondary": "#89b4fa", "accent_warn": "#f9e2af", "accent_mic": "#f38ba8",
        "btn_primary_bg": "#2f7d57", "btn_primary_border": "#a6e3a1", "btn_primary_hover": "#3b986b",
    },
    "Dracula Console": {
        "is_light": False, "bg_main": "#21222c", "bg_card": "#282a36", "bg_deep": "#191a21",
        "bg_control": "#383b4d", "bg_hover": "#44475a", "border_card": "#383b4d",
        "border_control": "#6272a4", "text_primary": "#f8f8f2", "text_bright": "#ffffff",
        "text_secondary": "#d6d8e5", "text_muted": "#9ea8cc", "accent_primary": "#50fa7b",
        "accent_secondary": "#8be9fd", "accent_warn": "#f1fa8c", "accent_mic": "#ff5555",
        "btn_primary_bg": "#2ea043", "btn_primary_border": "#50fa7b", "btn_primary_hover": "#3fb950",
    },
    "Synthwave Outrun": {
        "is_light": False, "bg_main": "#0d0618", "bg_card": "#160b29", "bg_deep": "#080311",
        "bg_control": "#241342", "bg_hover": "#361d61", "border_card": "#2e1854",
        "border_control": "#5b21b6", "text_primary": "#f3e8ff", "text_bright": "#ffffff",
        "text_secondary": "#d8b4fe", "text_muted": "#a78bfa", "accent_primary": "#00f5d4",
        "accent_secondary": "#00bbf9", "accent_warn": "#fee440", "accent_mic": "#fb7185",
        "btn_primary_bg": "#009688", "btn_primary_border": "#00f5d4", "btn_primary_hover": "#00bfa5",
    },
    "Analog Studio Rack": {
        "is_light": False, "bg_main": "#131110", "bg_card": "#1d1916", "bg_deep": "#0d0b0a",
        "bg_control": "#2b2521", "bg_hover": "#3c342e", "border_card": "#302924",
        "border_control": "#4a3f37", "text_primary": "#f5ebe0", "text_bright": "#fff8f0",
        "text_secondary": "#d6ccc2", "text_muted": "#a89f96", "accent_primary": "#f59e0b",
        "accent_secondary": "#38bdf8", "accent_warn": "#fbbf24", "accent_mic": "#fb7185",
        "btn_primary_bg": "#b45309", "btn_primary_border": "#f59e0b", "btn_primary_hover": "#d97706",
    },
    "Daylight Mastering (Light)": {
        "is_light": True, "bg_main": "#eef2f6", "bg_card": "#ffffff", "bg_deep": "#f8fafc",
        "bg_control": "#e2e8f0", "bg_hover": "#cbd5e1", "border_card": "#cbd5e1",
        "border_control": "#94a3b8", "text_primary": "#0f172a", "text_bright": "#020617",
        "text_secondary": "#334155", "text_muted": "#475569", "accent_primary": "#047857",
        "accent_secondary": "#0369a1", "accent_warn": "#b45309", "accent_mic": "#be123c",
        "btn_primary_bg": "#059669", "btn_primary_border": "#047857", "btn_primary_hover": "#047857",
    },
}


def _audiodg_sock_candidates() -> list:
    uid = os.getuid()
    xdg = os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{uid}")
    return [
        os.path.join(xdg, "lwasapi", "audiodg.sock"),
        f"/run/user/{uid}/lwasapi/audiodg.sock",
        "/tmp/lwasapi-audiodg.sock",
    ]


def _send_volume_cmd(target_id: int, pct: int, mute_mode: int) -> bool:
    """Send binary LWASAPI_CMD_SET_VOLUME directly over audiodg UNIX socket (0xFFFFFFFF=Master, 0xFFFFFFFE=Mic)."""
    buf = bytearray(400)
    struct.pack_into("<I", buf, 0, LWASAPI_CMD_SET_VOLUME)
    struct.pack_into("<I", buf, 72, max(0, min(3, mute_mode + 1)))
    struct.pack_into("<I", buf, 100, target_id & 0xFFFFFFFF)
    vol_milli = (max(0, min(150, int(pct))) * 10) if pct >= 0 else 0xFFFFFFFF
    struct.pack_into("<I", buf, 104, vol_milli)
    for path in _audiodg_sock_candidates():
        if not os.path.exists(path):
            continue
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(0.25)
        try:
            sock.connect(path)
            sock.sendall(buf)
            resp = sock.recv(24)
            if len(resp) >= 4:
                hr = struct.unpack_from("<I", resp, 0)[0]
                return hr == 0
            return True
        except OSError:
            pass
        finally:
            try:
                sock.close()
            except OSError:
                pass
    return False


class SalsaEngineClient:
    def __init__(self):
        self._lib = None
        if os.path.exists(LIB_PATH):
            try:
                self._lib = bind_liblwasapi(ctypes.CDLL(LIB_PATH))
            except Exception:
                self._lib = None

    def get_status(self):
        if not self._lib:
            return None
        st = LwasapiEngineStatus()
        try:
            if self._lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0:
                return st
        except Exception:
            pass
        return None

    def list_devices(self):
        if not self._lib:
            return []
        dl = LwasapiDeviceList()
        try:
            if self._lib.LWASAPI_ListDevices(ctypes.byref(dl)) == 0:
                n = min(dl.num_devices, LWASAPI_MAX_DEVICES)
                return [dl.devices[i] for i in range(n)]
        except Exception:
            pass
        return []

    def set_devices(self, pb_id: str, cap_id: str) -> bool:
        if not self._lib:
            return False
        try:
            pb = pb_id.encode("utf-8") if pb_id else b""
            cp = cap_id.encode("utf-8") if cap_id else b""
            return self._lib.LWASAPI_SetDevices(pb, cp) == 0
        except Exception:
            return False

    def set_master_volume(self, pct: int, mute_mode: int = -1) -> bool:
        return _send_volume_cmd(0xFFFFFFFF, pct, mute_mode)

    def set_mic_volume(self, pct: int, mute_mode: int = -1) -> bool:
        return _send_volume_cmd(0xFFFFFFFE, pct, mute_mode)

    def set_session_control(self, sid: int, vol_pct: int, mute_mode: int = -1) -> bool:
        if not self._lib:
            return False
        try:
            return self._lib.LWASAPI_SetSessionControl(sid, vol_pct, mute_mode, b"") == 0
        except Exception:
            return False

    def set_eq_config(self, cfg: LwasapiEqConfig) -> bool:
        if not self._lib:
            return False
        try:
            return self._lib.LWASAPI_SetDynamicEQ(ctypes.byref(cfg)) == 0
        except Exception:
            return False

    def set_engine_config(self, rate: int, period_frames: int = 0, format_bits: int = 0) -> bool:
        if not self._lib:
            return False
        try:
            return self._lib.LWASAPI_SetEngineConfig(int(rate), int(period_frames), int(format_bits)) == 0
        except Exception:
            return False

    def set_capture_rate(self, rate: int) -> bool:
        if not self._lib or not hasattr(self._lib, "LWASAPI_SetCaptureSampleRate"):
            return False
        try:
            return self._lib.LWASAPI_SetCaptureSampleRate(int(rate)) == 0
        except Exception:
            return False

    def set_mic_apo(self, cfg: LwasapiMicApoCfg) -> bool:
        if not self._lib or not hasattr(self._lib, "LWASAPI_SetMicAPO"):
            return False
        try:
            return self._lib.LWASAPI_SetMicAPO(ctypes.byref(cfg)) == 0
        except Exception:
            return False

    def set_bt_codec_cfg(self, cfg: LwasapiBtCodecCfg) -> bool:
        if not self._lib or not hasattr(self._lib, "LWASAPI_SetBtCodecConfig"):
            return False
        try:
            return self._lib.LWASAPI_SetBtCodecConfig(ctypes.byref(cfg)) == 0
        except Exception:
            return False

    def set_hw_analog(self, cfg: LwasapiHwAnalogCfg) -> bool:
        if not self._lib or not hasattr(self._lib, "LWASAPI_SetHwAnalog"):
            return False
        try:
            return self._lib.LWASAPI_SetHwAnalog(ctypes.byref(cfg)) == 0
        except Exception:
            return False


def _config_dir() -> str:
    xdg = os.environ.get("XDG_CONFIG_HOME")
    if xdg and os.path.isabs(xdg) and not os.path.islink(xdg):
        return os.path.join(xdg, "lwasapi")
    return os.path.join(os.path.expanduser("~"), ".config", "lwasapi")


def _gui_settings_path() -> str:
    return os.path.join(_config_dir(), "gui_settings.json")


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
                return data if isinstance(data, dict) else {}
        finally:
            if fd >= 0:
                os.close(fd)
    except Exception:
        return {}


def update_gui_settings(updates: dict) -> None:
    d = _config_dir()
    os.makedirs(d, mode=0o700, exist_ok=True)
    try:
        os.chmod(d, 0o700)
    except OSError:
        pass
    cur = load_gui_settings()
    cur.update(updates)
    path = _gui_settings_path()
    try:
        fd, tmp = tempfile.mkstemp(prefix=".tray_settings.", dir=d)
        try:
            os.fchmod(fd, 0o600)
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                fd = -1
                json.dump(cur, f, indent=2)
                f.write("\n")
                f.flush()
                os.fsync(f.fileno())
            os.replace(tmp, path)
        finally:
            if fd >= 0:
                os.close(fd)
    except Exception:
        pass


def resolve_active_theme(settings: dict) -> dict:
    name = settings.get("theme", "Salsa Midnight (Default)")
    if name == "Custom Studio Theme" and isinstance(settings.get("custom_theme"), dict):
        base = dict(THEMES["Salsa Midnight (Default)"])
        base.update(normalize_custom_theme(settings["custom_theme"]))
        return base
    return THEMES.get(name, THEMES["Salsa Midnight (Default)"])


def resolve_tray_theme_name(settings: dict) -> str:
    tray_theme = str(settings.get("tray_theme") or "").strip()
    if tray_theme:
        return tray_theme
    if "tray_breeze_emerald" in settings:
        return "Plasma Breeze Emerald" if bool(settings["tray_breeze_emerald"]) else "Follow Studio Theme"
    return "Follow Studio Theme"


def resolve_tray_theme(settings: dict) -> dict:
    name = resolve_tray_theme_name(settings)
    if name == "Follow Studio Theme":
        return resolve_active_theme(settings)
    if name == "Custom Studio Theme" and isinstance(settings.get("custom_theme"), dict):
        base = dict(THEMES["Salsa Midnight (Default)"])
        base.update(normalize_custom_theme(settings["custom_theme"]))
        return base
    return THEMES.get(name, resolve_active_theme(settings))


def _parse_dbus_pct(raw) -> int | None:
    try:
        if raw is None:
            return None
        if isinstance(raw, (bytes, bytearray)):
            return int(raw[0]) if len(raw) > 0 else None
        return int(round(float(raw)))
    except Exception:
        return None


def read_bluez_battery_pct(target_alias: str = "") -> tuple:
    """Query org.bluez.Battery1 Percentage (decoding D-Bus BYTE) and UPower fallback."""
    target_l = str(target_alias or "").strip().lower()
    try:
        bus = QtDBus.QDBusConnection.systemBus()
        if not bus.isConnected():
            return None, ""
        msg = QtDBus.QDBusMessage.createMethodCall(
            "org.bluez", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects"
        )
        reply = bus.call(msg, QtDBus.QDBus.CallMode.Block, 150)
        if reply.type() != QtDBus.QDBusMessage.MessageType.ErrorMessage and reply.arguments():
            managed = reply.arguments()[0]
            if isinstance(managed, dict):
                candidates: list[tuple[int, int, str]] = []
                for _path, ifaces in managed.items():
                    if not isinstance(ifaces, dict):
                        continue
                    dev = ifaces.get("org.bluez.Device1")
                    bat = ifaces.get("org.bluez.Battery1")
                    if not (isinstance(dev, dict) and dev.get("Connected") and isinstance(bat, dict)):
                        continue
                    pct_val = _parse_dbus_pct(bat.get("Percentage"))
                    if pct_val is None or not (0 <= pct_val <= 100):
                        continue
                    alias_l = str(dev.get("Alias") or "").strip().lower()
                    name_l = str(dev.get("Name") or "").strip().lower()
                    icon_l = str(dev.get("Icon") or "").strip().lower()
                    uuids = [str(u).lower() for u in (dev.get("UUIDs") or [])]
                    is_audio = (
                        icon_l in ("audio-headset", "audio-headphones", "audio-card")
                        or any(u.startswith(("0000110b-", "0000110d-", "0000111e-")) for u in uuids)
                    )
                    score = 0
                    if target_l:
                        if target_l == alias_l or target_l == name_l:
                            score += 100
                        elif target_l in alias_l or alias_l in target_l or target_l in name_l or name_l in target_l:
                            score += 70
                    if is_audio:
                        score += 25
                    if target_l and score == 0:
                        continue
                    candidates.append((score, pct_val, str(dev.get("Address", ""))))
                if candidates:
                    candidates.sort(key=lambda item: item[0], reverse=True)
                    _, best_pct, best_addr = candidates[0]
                    return best_pct, best_addr

        # Fallback: check UPower Bluetooth headset devices over system D-Bus
        u_msg = QtDBus.QDBusMessage.createMethodCall(
            "org.freedesktop.UPower", "/org/freedesktop/UPower",
            "org.freedesktop.UPower", "EnumerateDevices",
        )
        u_rep = bus.call(u_msg, QtDBus.QDBus.CallMode.Block, 150)
        if u_rep.type() != QtDBus.QDBusMessage.MessageType.ErrorMessage and u_rep.arguments():
            dev_paths = u_rep.arguments()[0] or []
            for dpath in dev_paths:
                if "bluez" not in str(dpath).lower() and "headset" not in str(dpath).lower():
                    continue
                p_call = QtDBus.QDBusMessage.createMethodCall(
                    "org.freedesktop.UPower", str(dpath),
                    "org.freedesktop.DBus.Properties", "GetAll",
                )
                p_call.setArguments(["org.freedesktop.UPower.Device"])
                p_rep = bus.call(p_call, QtDBus.QDBus.CallMode.Block, 120)
                if p_rep.type() == QtDBus.QDBusMessage.MessageType.ErrorMessage or not p_rep.arguments():
                    continue
                props = p_rep.arguments()[0]
                if not isinstance(props, dict):
                    continue
                model_l = str(props.get("Model") or "").strip().lower()
                if target_l and model_l and target_l not in model_l and model_l not in target_l:
                    continue
                pct_val = _parse_dbus_pct(props.get("Percentage"))
                if pct_val is not None and 0 < pct_val <= 100:
                    return pct_val, str(props.get("Serial") or "")
    except Exception:
        pass
    return None, ""


class AppIconResolver:
    """Resolves desktop/program icons safely via /proc/<pid>, XDG .desktop entries, and QIcon.fromTheme."""

    _ALIASES = {
        "firefox": ["firefox", "firefox-esr", "firefox-developer-edition"],
        "mozilla firefox": ["firefox"],
        "zen": ["zen-browser", "zen"],
        "discord": ["discord", "com.discordapp.Discord", "discord-canary", "discord-ptb"],
        "vesktop": ["vesktop", "dev.vencord.Vesktop", "discord"],
        "chrome": ["google-chrome", "chromium", "chromium-browser"],
        "google chrome": ["google-chrome", "chromium"],
        "chromium": ["chromium", "chromium-browser"],
        "brave": ["brave-browser", "brave"],
        "steam": ["steam", "com.valvesoftware.Steam"],
        "spotify": ["spotify-client", "spotify", "com.spotify.Client"],
        "obs": ["com.obsproject.Studio", "obs"],
        "mpv": ["mpv"],
        "vlc": ["vlc"],
        "elisa": ["elisa"],
        "audacity": ["audacity"],
        "reaper": ["cockos-reaper", "reaper"],
        "lutris": ["lutris", "net.lutris.Lutris"],
        "heroic": ["heroic", "com.heroicgameslauncher.hgl"],
        "bottles": ["com.usebottles.bottles", "bottles"],
    }

    def __init__(self):
        self._desktop_indexed = False
        self._by_exec = {}
        self._by_name = {}
        self._by_wm = {}
        self._cache = {}

    def _safe_read_text(self, path: str, max_bytes: int = 65536) -> str:
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
        try:
            fd = os.open(path, flags)
            try:
                st = os.fstat(fd)
                if not stat.S_ISREG(st.st_mode) or st.st_size > max_bytes * 2:
                    return ""
                with os.fdopen(fd, "r", encoding="utf-8", errors="ignore") as f:
                    fd = -1
                    return f.read(max_bytes)
            finally:
                if fd >= 0:
                    os.close(fd)
        except Exception:
            return ""

    def _safe_read_proc(self, pid: int, leaf: str, max_bytes: int = 4096) -> bytes:
        if pid <= 0 or leaf not in ("comm", "cmdline"):
            return b""
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
        try:
            fd = os.open(f"/proc/{int(pid)}/{leaf}", flags)
            try:
                return os.read(fd, max_bytes)
            finally:
                os.close(fd)
        except Exception:
            return b""

    def _ensure_desktop_index(self) -> None:
        if self._desktop_indexed:
            return
        self._desktop_indexed = True
        home = os.path.expanduser("~")
        dirs = [
            os.path.join(home, ".local", "share", "applications"),
            os.path.join(home, ".local", "share", "flatpak", "exports", "share", "applications"),
            "/var/lib/flatpak/exports/share/applications",
            "/usr/local/share/applications",
            "/usr/share/applications",
        ]
        for d in dirs:
            if not os.path.isdir(d) or os.path.islink(d):
                continue
            try:
                names = sorted(os.listdir(d))
            except OSError:
                continue
            for fn in names:
                if not fn.endswith(".desktop"):
                    continue
                txt = self._safe_read_text(os.path.join(d, fn))
                if not txt:
                    continue
                in_entry = False
                d_name = ""
                d_exec = ""
                d_icon = ""
                d_wm = ""
                for line in txt.splitlines():
                    s = line.strip()
                    if not s or s.startswith("#"):
                        continue
                    if s.startswith("["):
                        in_entry = s == "[Desktop Entry]"
                        continue
                    if not in_entry or "=" not in s:
                        continue
                    k, v = s.split("=", 1)
                    k = k.strip()
                    v = v.strip()
                    if k == "Name" and not d_name:
                        d_name = v.lower()
                    elif k == "Exec" and not d_exec:
                        tok = v.split()[0].strip("\"'") if v else ""
                        d_exec = os.path.basename(tok).lower()
                    elif k == "Icon" and not d_icon:
                        d_icon = v
                    elif k == "StartupWMClass" and not d_wm:
                        d_wm = v.lower()
                if not d_icon:
                    continue
                stem = fn[:-8].lower()
                self._by_name.setdefault(stem, d_icon)
                if d_name:
                    self._by_name.setdefault(d_name, d_icon)
                if d_exec:
                    self._by_exec.setdefault(d_exec, d_icon)
                if d_wm:
                    self._by_wm.setdefault(d_wm, d_icon)

    def _pixmap_from_icon_spec(self, icon_spec: str, size: int) -> QtGui.QPixmap | None:
        if not icon_spec:
            return None
        if "/" in icon_spec:
            if os.path.isfile(icon_spec) and not os.path.islink(icon_spec):
                pm = QtGui.QPixmap(icon_spec)
                if not pm.isNull():
                    return pm.scaled(size, size, QtCore.Qt.AspectRatioMode.KeepAspectRatio, QtCore.Qt.TransformationMode.SmoothTransformation)
            return None
        ic = QtGui.QIcon.fromTheme(icon_spec)
        if not ic.isNull():
            pm = ic.pixmap(size, size)
            if not pm.isNull():
                return pm
        return None

    def get_pixmap(self, client_name: str, client_pid: int, is_capture: bool = False, size: int = 20) -> QtGui.QPixmap:
        clean = (client_name or "").strip()
        key = (int(client_pid), clean.lower(), bool(is_capture), size)
        if key in self._cache:
            return self._cache[key]

        self._ensure_desktop_index()
        candidates = []
        low = clean.lower()
        if low.startswith("[mic] "):
            low = low[6:].strip()
        if low:
            candidates.append(low)
            candidates.extend(self._ALIASES.get(low, []))
            for token in re.split(r"[\s\-_/]+", low):
                if len(token) >= 3:
                    candidates.append(token)
                    candidates.extend(self._ALIASES.get(token, []))

        if client_pid > 0:
            comm = self._safe_read_proc(client_pid, "comm", 256).decode("utf-8", "ignore").strip().lower()
            if comm and not comm.startswith(("wine", "wineserver")):
                candidates.insert(0, comm)
                candidates.extend(self._ALIASES.get(comm, []))
            raw_cmd = self._safe_read_proc(client_pid, "cmdline", 4096)
            if raw_cmd:
                argv0 = raw_cmd.split(b"\x00", 1)[0].decode("utf-8", "ignore").strip()
                base = os.path.basename(argv0).lower()
                if base and not base.startswith(("wine", "python", "env", "sh", "bash")):
                    candidates.insert(0, base)
                    candidates.extend(self._ALIASES.get(base, []))

        # Deduplicate while preserving order
        seen = set()
        ordered = []
        for c in candidates:
            c = c.strip()
            if c and c not in seen:
                seen.add(c)
                ordered.append(c)

        pm = None
        for c in ordered:
            for m in (self._by_exec, self._by_wm, self._by_name):
                icon_spec = m.get(c)
                if icon_spec:
                    pm = self._pixmap_from_icon_spec(icon_spec, size)
                    if pm is not None:
                        break
            if pm is not None:
                break
            pm = self._pixmap_from_icon_spec(c, size)
            if pm is not None:
                break

        if pm is None:
            fb_name = "audio-input-microphone" if is_capture else "applications-multimedia"
            pm = self._pixmap_from_icon_spec(fb_name, size)

        if pm is None:
            pm = QtGui.QPixmap(size, size)
            pm.fill(QtCore.Qt.GlobalColor.transparent)
            p = QtGui.QPainter(pm)
            p.setRenderHint(QtGui.QPainter.RenderHint.Antialiasing)
            p.setBrush(QtGui.QColor("#252c33"))
            p.setPen(QtGui.QPen(QtGui.QColor("#38bdf8" if is_capture else "#14b866"), 1.2))
            p.drawRoundedRect(QtCore.QRectF(1, 1, size - 2, size - 2), 5, 5)
            p.setPen(QtGui.QColor("#f8fafc"))
            f = p.font()
            f.setBold(True)
            f.setPixelSize(max(9, int(size * 0.52)))
            p.setFont(f)
            ch = (clean[:1] or ("M" if is_capture else "A")).upper()
            p.drawText(pm.rect(), QtCore.Qt.AlignmentFlag.AlignCenter, ch)
            p.end()

        if len(self._cache) > 256:
            self._cache.clear()
        self._cache[key] = pm
        return pm


class TrayIconBank:
    """Renders crisp Salsa tray icons with emerald glow / red slash / amber dot states."""

    def __init__(self):
        uid = os.getuid()
        xdg = os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{uid}")
        base_rt = os.path.join(xdg, "lwasapi")
        os.makedirs(base_rt, mode=0o700, exist_ok=True)
        try:
            os.chmod(base_rt, 0o700)
        except OSError:
            pass
        self.theme_dir = os.path.join(base_rt, "tray-icons")
        os.makedirs(self.theme_dir, mode=0o700, exist_ok=True)
        self._sizes = (22, 24, 32, 48, 64, 128)
        for sz in self._sizes:
            os.makedirs(os.path.join(self.theme_dir, "hicolor", f"{sz}x{sz}", "apps"), mode=0o700, exist_ok=True)
        self._write_index_theme()
        self._icons = {}
        self._render_all()

    def _write_index_theme(self) -> None:
        p = os.path.join(self.theme_dir, "hicolor", "index.theme")
        dirs = ",".join(f"{sz}x{sz}/apps" for sz in self._sizes)
        secs = "\n".join(f"[{sz}x{sz}/apps]\nSize={sz}\nContext=Applications\nType=Fixed" for sz in self._sizes)
        content = f"[Icon Theme]\nName=hicolor\nDirectories={dirs}\n{secs}\n"
        try:
            fd = os.open(p, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0), 0o600)
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(content)
        except OSError:
            pass

    def _load_square_salsa(self, size: int, path_override: str | None = None, alpha_thresh: int = 16) -> QtGui.QPixmap:
        chosen = path_override if (path_override and os.path.exists(path_override)) else ICON_PATH
        cache = getattr(self, "_src_crop_cache", None)
        if cache is None:
            cache = {}
            self._src_crop_cache = cache
        ckey = (chosen, int(alpha_thresh))
        cropped_src = cache.get(ckey)

        if cropped_src is None and os.path.exists(chosen):
            img = QtGui.QImage(chosen)
            if not img.isNull():
                img = img.convertToFormat(QtGui.QImage.Format.Format_ARGB32)
                w, h = img.width(), img.height()
                bpl = img.bytesPerLine()
                buf = bytes(img.constBits().asarray(h * bpl))
                min_x, min_y = w, h
                max_x, max_y = -1, -1
                for y in range(h):
                    row = buf[y * bpl : y * bpl + w * 4]
                    alphas = row[3::4]
                    if max(alphas) < alpha_thresh:
                        continue
                    if y < min_y:
                        min_y = y
                    if y > max_y:
                        max_y = y
                    for x, a in enumerate(alphas):
                        if a >= alpha_thresh:
                            if x < min_x:
                                min_x = x
                            break
                    for x in range(w - 1, -1, -1):
                        if alphas[x] >= alpha_thresh:
                            if x > max_x:
                                max_x = x
                            break
                if max_x >= min_x and max_y >= min_y:
                    cropped_src = QtGui.QPixmap.fromImage(
                        img.copy(min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
                    )
                else:
                    cropped_src = QtGui.QPixmap.fromImage(img)
                cache[ckey] = cropped_src

        if cropped_src is not None and not cropped_src.isNull():
            scaled = cropped_src.scaled(
                size,
                size,
                QtCore.Qt.AspectRatioMode.KeepAspectRatio,
                QtCore.Qt.TransformationMode.SmoothTransformation,
            )
            if scaled.width() == size and scaled.height() == size:
                return scaled
            out = QtGui.QPixmap(size, size)
            out.fill(QtCore.Qt.GlobalColor.transparent)
            p = QtGui.QPainter(out)
            p.drawPixmap((size - scaled.width()) // 2, (size - scaled.height()) // 2, scaled)
            p.end()
            return out

        fallback = QtGui.QPixmap(size, size)
        fallback.fill(QtCore.Qt.GlobalColor.transparent)
        p = QtGui.QPainter(fallback)
        p.setRenderHint(QtGui.QPainter.RenderHint.Antialiasing)
        p.setBrush(QtGui.QColor("#0F172A"))
        p.setPen(QtGui.QPen(QtGui.QColor("#22C55E"), 3))
        p.drawEllipse(2, 2, size - 4, size - 4)
        p.setPen(QtGui.QColor("#F8FAFC"))
        f = p.font()
        f.setBold(True)
        f.setPixelSize(int(size * 0.50))
        p.setFont(f)
        p.drawText(fallback.rect(), QtCore.Qt.AlignmentFlag.AlignCenter, "S")
        p.end()
        return fallback

    def _render_state_pixmap(self, state_key: str, size: int = 128) -> QtGui.QPixmap:
        if state_key == "active_glow" and os.path.exists(ICON_GLOW_PATH):
            glow_pm = self._load_square_salsa(size, ICON_GLOW_PATH, alpha_thresh=48)
            if not glow_pm.isNull():
                return glow_pm

        pm = QtGui.QPixmap(size, size)
        pm.fill(QtCore.Qt.GlobalColor.transparent)
        p = QtGui.QPainter(pm)
        p.setRenderHint(QtGui.QPainter.RenderHint.Antialiasing)
        p.setRenderHint(QtGui.QPainter.RenderHint.SmoothPixmapTransform)
        cx = size / 2.0
        cy = size / 2.0

        if state_key == "active_glow":
            corona = QtGui.QRadialGradient(cx, cy, size * 0.50)
            corona.setColorAt(0.45, QtGui.QColor(34, 197, 94, 240))
            corona.setColorAt(0.75, QtGui.QColor(16, 185, 129, 170))
            corona.setColorAt(1.00, QtGui.QColor(34, 197, 94, 0))
            p.setPen(QtCore.Qt.PenStyle.NoPen)
            p.setBrush(corona)
            p.drawEllipse(QtCore.QRectF(0, 0, size, size))
            inner = int(size * 0.94)
            off = (size - inner) // 2
            p.drawPixmap(off, off, self._load_square_salsa(inner, alpha_thresh=16))

        elif state_key == "active_plain":
            p.drawPixmap(0, 0, self._load_square_salsa(size, alpha_thresh=16))

        elif state_key == "muted":
            inner = int(size * 0.96)
            off = (size - inner) // 2
            bowl = self._load_square_salsa(inner, alpha_thresh=16)
            p.save()
            p.setOpacity(0.50)
            p.drawPixmap(off, off, bowl)
            p.restore()

            pad = size * 0.11
            pen_bg = QtGui.QPen(QtGui.QColor(9, 13, 22, 220), max(4.5, size * 0.125), QtCore.Qt.PenStyle.SolidLine, QtCore.Qt.PenCapStyle.RoundCap)
            p.setPen(pen_bg)
            p.drawLine(QtCore.QPointF(pad, size - pad), QtCore.QPointF(size - pad, pad))
            pen_fg = QtGui.QPen(QtGui.QColor("#EF4444"), max(2.5, size * 0.08), QtCore.Qt.PenStyle.SolidLine, QtCore.Qt.PenCapStyle.RoundCap)
            p.setPen(pen_fg)
            p.drawLine(QtCore.QPointF(pad, size - pad), QtCore.QPointF(size - pad, pad))

        else:  # stopped
            inner = int(size * 0.96)
            off = (size - inner) // 2
            bowl = self._load_square_salsa(inner, alpha_thresh=16)
            p.save()
            p.setOpacity(0.45)
            p.drawPixmap(off, off, bowl)
            p.restore()
            dot = max(8.0, size * 0.25)
            dx = size - dot - size * 0.05
            dy = size - dot - size * 0.05
            p.setBrush(QtGui.QColor("#F59E0B"))
            p.setPen(QtGui.QPen(QtGui.QColor("#090D16"), max(2.0, size * 0.03)))
            p.drawEllipse(QtCore.QRectF(dx, dy, dot, dot))

        p.end()
        return pm

    def _save_png(self, pm: QtGui.QPixmap, out_path: str, mode: int = 0o600) -> None:
        buf = QtCore.QByteArray()
        qbuf = QtCore.QBuffer(buf)
        qbuf.open(QtCore.QIODevice.OpenModeFlag.WriteOnly)
        pm.save(qbuf, "PNG")
        qbuf.close()
        try:
            parent = os.path.dirname(out_path)
            if parent:
                os.makedirs(parent, mode=(0o755 if mode == 0o644 else 0o700), exist_ok=True)
            fd = os.open(out_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0), mode)
            with os.fdopen(fd, "wb") as f:
                os.fchmod(f.fileno(), mode)
                f.write(bytes(buf))
        except OSError:
            pass

    def _render_all(self) -> None:
        self._state_names = {
            "active_glow": "salsa-tray-active-glow",
            "active_plain": "salsa-tray-active-plain",
            "muted": "salsa-tray-muted",
            "stopped": "salsa-tray-stopped",
        }
        user_hicolor = os.path.expanduser("~/.local/share/icons/hicolor")
        for key, icon_name in self._state_names.items():
            pm128 = self._render_state_pixmap(key, 128)
            qicon = QtGui.QIcon()
            for sz in self._sizes:
                scaled = pm128 if sz == 128 else pm128.scaled(
                    sz, sz, QtCore.Qt.AspectRatioMode.KeepAspectRatio, QtCore.Qt.TransformationMode.SmoothTransformation
                )
                qicon.addPixmap(scaled)
                p = os.path.join(self.theme_dir, "hicolor", f"{sz}x{sz}", "apps", f"{icon_name}.png")
                self._save_png(scaled, p)
                hp = os.path.join(user_hicolor, f"{sz}x{sz}", "apps", f"{icon_name}.png")
                self._save_png(scaled, hp, mode=0o644)
            self._save_png(pm128, os.path.join(self.theme_dir, f"{icon_name}.png"))
            self._icons[key] = qicon

        # Update icon cache and tell KDE Plasma's KIconLoader to reload icon names
        try:
            subprocess.Popen(
                ["gtk-update-icon-cache", "-q", "-f", "-t", user_hicolor],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except Exception:
            pass
        try:
            sig = QtDBus.QDBusMessage.createSignal(
                "/KIconLoader", "org.kde.KIconLoader", "iconChanged"
            )
            sig.setArguments([0])
            QtDBus.QDBusConnection.sessionBus().send(sig)
        except Exception:
            pass

    def icon_name(self, active: bool, muted: bool, glow: bool) -> str:
        if not active:
            return self._state_names["stopped"]
        if muted:
            return self._state_names["muted"]
        return self._state_names["active_glow" if glow else "active_plain"]

    def qicon(self, active: bool, muted: bool, glow: bool) -> QtGui.QIcon:
        if not active:
            return self._icons["stopped"]
        if muted:
            return self._icons["muted"]
        return self._icons["active_glow" if glow else "active_plain"]


class SwipeFader(QtWidgets.QWidget):
    """Smooth capsule volume fader supporting horizontal touch/mouse drag & wheel swipe (0..150%)."""

    value_changed = QtCore.pyqtSignal(int)
    value_committed = QtCore.pyqtSignal(int)

    def __init__(self, label: str = "", accent: str = "#22C55E", max_val: int = 150, compact: bool = False, parent=None):
        super().__init__(parent)
        self._label = label
        self._accent = QtGui.QColor(accent)
        self._bg_track = QtGui.QColor("#1e293b")
        self._border_col = QtGui.QColor("#334155")
        self._text_col = QtGui.QColor("#f8fafc")
        self._muted_col = QtGui.QColor("#94a3b8")
        self._max_val = max_val
        self._value = 100
        self._muted = False
        self._compact = compact
        self._dragging = False
        self.setMinimumHeight(28 if compact else 34)
        self.setMaximumHeight(28 if compact else 34)
        self.setCursor(QtCore.Qt.CursorShape.PointingHandCursor)
        self.setFocusPolicy(QtCore.Qt.FocusPolicy.StrongFocus)

    def set_theme_colors(self, accent: str, track: str, border: str, text: str, muted: str) -> None:
        self._accent = QtGui.QColor(accent)
        self._bg_track = QtGui.QColor(track)
        self._border_col = QtGui.QColor(border)
        self._text_col = QtGui.QColor(text)
        self._muted_col = QtGui.QColor(muted)
        self.update()

    def set_label(self, label: str) -> None:
        if self._label != label:
            self._label = label
            self.update()

    def value(self) -> int:
        return self._value

    def set_value(self, val: int, muted: bool = False) -> None:
        val = max(0, min(self._max_val, int(val)))
        if self._dragging:
            if self._muted != muted:
                self._muted = muted
                self.update()
            return
        if self._value != val or self._muted != muted:
            self._value = val
            self._muted = muted
            self.update()

    def _x_to_val(self, x: float) -> int:
        pad = 6.0
        usable = max(10.0, self.width() - pad * 2.0)
        frac = max(0.0, min(1.0, (x - pad) / usable))
        return int(round(frac * self._max_val))

    def mousePressEvent(self, ev: QtGui.QMouseEvent) -> None:
        if ev.button() == QtCore.Qt.MouseButton.LeftButton:
            self._dragging = True
            v = self._x_to_val(ev.position().x())
            if v != self._value:
                self._value = v
                self.update()
            self.value_changed.emit(self._value)
            ev.accept()
            return
        super().mousePressEvent(ev)

    def mouseMoveEvent(self, ev: QtGui.QMouseEvent) -> None:
        if self._dragging:
            v = self._x_to_val(ev.position().x())
            if v != self._value:
                self._value = v
                self.update()
                self.value_changed.emit(self._value)
            ev.accept()
            return
        super().mouseMoveEvent(ev)

    def mouseReleaseEvent(self, ev: QtGui.QMouseEvent) -> None:
        if ev.button() == QtCore.Qt.MouseButton.LeftButton and self._dragging:
            self._dragging = False
            v = self._x_to_val(ev.position().x())
            self._value = v
            self.update()
            self.value_committed.emit(self._value)
            ev.accept()
            return
        super().mouseReleaseEvent(ev)

    def wheelEvent(self, ev: QtGui.QWheelEvent) -> None:
        dx = ev.angleDelta().x()
        dy = ev.angleDelta().y()
        delta = dy if abs(dy) >= abs(dx) else dx
        if delta != 0:
            step = 2 if delta > 0 else -2
            nv = max(0, min(self._max_val, self._value + step))
            if nv != self._value:
                self._value = nv
                self.update()
                self.value_committed.emit(self._value)
        ev.accept()

    def keyPressEvent(self, ev: QtGui.QKeyEvent) -> None:
        if ev.key() in (QtCore.Qt.Key.Key_Right, QtCore.Qt.Key.Key_Up):
            nv = min(self._max_val, self._value + 3)
        elif ev.key() in (QtCore.Qt.Key.Key_Left, QtCore.Qt.Key.Key_Down):
            nv = max(0, self._value - 3)
        else:
            super().keyPressEvent(ev)
            return
        if nv != self._value:
            self._value = nv
            self.update()
            self.value_committed.emit(self._value)
        ev.accept()

    def paintEvent(self, ev: QtGui.QPaintEvent) -> None:
        p = QtGui.QPainter(self)
        p.setRenderHint(QtGui.QPainter.RenderHint.Antialiasing)
        r = QtCore.QRectF(1.0, 1.0, self.width() - 2.0, self.height() - 2.0)
        rad = r.height() / 2.0

        # Capsule track
        p.setBrush(self._bg_track)
        p.setPen(QtGui.QPen(self._border_col, 1.0))
        p.drawRoundedRect(r, rad, rad)

        # Unity 100% tick mark (subtle vertical notch)
        if self._max_val > 100:
            ux = r.left() + (100.0 / self._max_val) * r.width()
            p.setPen(QtGui.QPen(QtGui.QColor(255, 255, 255, 38), 1.0))
            p.drawLine(QtCore.QPointF(ux, r.top() + 5), QtCore.QPointF(ux, r.bottom() - 5))

        # Active fill pill clipped inside groove
        frac = max(0.0, min(1.0, self._value / float(self._max_val)))
        fill_col = QtGui.QColor("#64748b") if self._muted else self._accent
        if frac > 0.005:
            p.save()
            clip = QtGui.QPainterPath()
            clip.addRoundedRect(r, rad, rad)
            p.setClipPath(clip)
            fw = max(r.height() * 0.5, frac * r.width())
            grad = QtGui.QLinearGradient(r.left(), 0, r.left() + fw, 0)
            c0 = QtGui.QColor(fill_col)
            c0.setAlpha(150 if self._compact else 175)
            c1 = QtGui.QColor(fill_col)
            c1.setAlpha(230)
            grad.setColorAt(0.0, c0)
            grad.setColorAt(1.0, c1)
            p.setPen(QtCore.Qt.PenStyle.NoPen)
            p.setBrush(grad)
            p.drawRoundedRect(QtCore.QRectF(r.left(), r.top(), fw, r.height()), rad, rad)
            p.restore()

            # Thumb pill / dot near fill edge
            tx = max(r.left() + r.height() * 0.48, min(r.right() - r.height() * 0.48, r.left() + frac * r.width()))
            tr = (r.height() * 0.30) if self._compact else (r.height() * 0.33)
            p.setBrush(QtGui.QColor("#F8FAFC"))
            p.setPen(QtGui.QPen(fill_col, 2.0))
            p.drawEllipse(QtCore.QPointF(tx, r.center().y()), tr, tr)

        # Label & Percentage text with crisp readability
        font = p.font()
        font.setPixelSize(11 if self._compact else 12)
        font.setBold(True)
        p.setFont(font)
        p.setPen(self._muted_col if self._muted else self._text_col)
        left_rect = r.adjusted(12, 0, -54, 0)
        fm = QtGui.QFontMetrics(font)
        elided = fm.elidedText(self._label, QtCore.Qt.TextElideMode.ElideRight, int(max(20, left_rect.width())))
        p.drawText(left_rect, QtCore.Qt.AlignmentFlag.AlignVCenter | QtCore.Qt.AlignmentFlag.AlignLeft, elided)

        right_rect = r.adjusted(10, 0, -12, 0)
        pct_text = "MUTED" if self._muted else f"{self._value}%"
        p.drawText(right_rect, QtCore.Qt.AlignmentFlag.AlignVCenter | QtCore.Qt.AlignmentFlag.AlignRight, pct_text)
        p.end()


class PeakBarWidget(QtWidgets.QWidget):
    """Compact dual-channel L/R studio peak level meter inside the Quick Mixer."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self._l = 0.0
        self._r = 0.0
        self._accent = QtGui.QColor("#22C55E")
        self._bg = QtGui.QColor("#0f172a")
        self.setFixedHeight(8)

    def set_levels(self, l_val: float, r_val: float, accent: str = "#22C55E") -> None:
        self._l = max(0.0, min(1.0, float(l_val)))
        self._r = max(0.0, min(1.0, float(r_val)))
        self._accent = QtGui.QColor(accent)
        self.update()

    def paintEvent(self, ev: QtGui.QPaintEvent) -> None:
        p = QtGui.QPainter(self)
        p.setRenderHint(QtGui.QPainter.RenderHint.Antialiasing)
        w = self.width()
        for row, val in enumerate((self._l, self._r)):
            y = row * 4
            rect = QtCore.QRectF(0, y, w, 3)
            p.setPen(QtCore.Qt.PenStyle.NoPen)
            p.setBrush(self._bg)
            p.drawRoundedRect(rect, 1.5, 1.5)
            if val > 0.005:
                fw = val * w
                grad = QtGui.QLinearGradient(0, 0, w, 0)
                grad.setColorAt(0.0, self._accent)
                grad.setColorAt(0.75, QtGui.QColor("#F59E0B"))
                grad.setColorAt(1.0, QtGui.QColor("#EF4444"))
                p.setBrush(grad)
                p.drawRoundedRect(QtCore.QRectF(0, y, fw, 3), 1.5, 1.5)
        p.end()


class SalsaOsdOverlay(QtWidgets.QWidget):
    """Floating translucent Plasma-style OSD pill for scroll/swipe/mute/key feedback."""

    def __init__(self):
        super().__init__(
            None,
            QtCore.Qt.WindowType.Tool
            | QtCore.Qt.WindowType.FramelessWindowHint
            | QtCore.Qt.WindowType.WindowStaysOnTopHint
            | QtCore.Qt.WindowType.WindowDoesNotAcceptFocus,
        )
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_TranslucentBackground, True)
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_ShowWithoutActivating, True)
        self.setFixedSize(330, 66)
        self._title = "Master Output"
        self._pct = 75
        self._muted = False
        self._is_mic = False
        self._accent = QtGui.QColor("#22C55E")
        self._hide_timer = QtCore.QTimer(self)
        self._hide_timer.setSingleShot(True)
        self._hide_timer.timeout.connect(self.hide)

    def show_osd(self, title: str, pct: int, muted: bool, is_mic: bool = False, accent: str = "#22C55E") -> None:
        self._title = str(title or ("Mic Input" if is_mic else "Master Output")).strip()
        self._pct = max(0, min(150, int(pct)))
        self._muted = bool(muted)
        self._is_mic = bool(is_mic)
        self._accent = QtGui.QColor(accent)

        screen = QtWidgets.QApplication.screenAt(QtGui.QCursor.pos()) or QtWidgets.QApplication.primaryScreen()
        if screen:
            avail = screen.availableGeometry()
            geom = screen.geometry()
            clearance = _plasma_panel_clearance() if avail.height() >= geom.height() - 4 else 12
            x = avail.center().x() - self.width() // 2
            y = max(avail.top() + 8, avail.bottom() - self.height() - clearance - 34)
            self.setAttribute(QtCore.Qt.WidgetAttribute.WA_Moved, True)
            self.move(x, y)
        self.update()
        self.show()
        _apply_x11_skip_taskbar(self)
        _apply_kde_blur_behind(self, radius=14)
        self._hide_timer.start(1550)

    def _draw_audio_symbol(self, p: QtGui.QPainter, accent: QtGui.QColor) -> None:
        cy = self.height() / 2.0
        muted = self._muted
        stroke_col = QtGui.QColor("#94A3B8") if muted else QtGui.QColor("#CBD5E1")
        wave_col = QtGui.QColor("#EF4444") if muted else accent

        p.save()
        if not self._is_mic:
            # Crisp vector speaker body + flared cone
            cone = QtGui.QPainterPath()
            cone.moveTo(19.0, cy - 4.8)
            cone.lineTo(24.5, cy - 4.8)
            cone.lineTo(31.5, cy - 11.2)
            cone.lineTo(31.5, cy + 11.2)
            cone.lineTo(24.5, cy + 4.8)
            cone.lineTo(19.0, cy + 4.8)
            cone.closeSubpath()

            p.setBrush(QtGui.QColor(stroke_col.red(), stroke_col.green(), stroke_col.blue(), 28))
            p.setPen(QtGui.QPen(stroke_col, 1.9, QtCore.Qt.PenStyle.SolidLine, QtCore.Qt.PenCapStyle.RoundCap, QtCore.Qt.PenJoinStyle.RoundJoin))
            p.drawPath(cone)

            if muted:
                # Diagonal red mute slash / cross mark
                p.setPen(QtGui.QPen(wave_col, 2.1, QtCore.Qt.PenStyle.SolidLine, QtCore.Qt.PenCapStyle.RoundCap))
                p.drawLine(QtCore.QPointF(36.0, cy - 4.5), QtCore.QPointF(44.5, cy + 4.5))
                p.drawLine(QtCore.QPointF(44.5, cy - 4.5), QtCore.QPointF(36.0, cy + 4.5))
            else:
                # Accent curved sound waves scaled by volume level
                w_pen = QtGui.QPen(wave_col, 2.0, QtCore.Qt.PenStyle.SolidLine, QtCore.Qt.PenCapStyle.RoundCap)
                p.setPen(w_pen)
                p.setBrush(QtCore.Qt.BrushStyle.NoBrush)
                if self._pct >= 1:
                    r1 = QtCore.QRectF(29.0, cy - 6.0, 11.0, 12.0)
                    p.drawArc(r1, -46 * 16, 92 * 16)
                if self._pct >= 35:
                    r2 = QtCore.QRectF(31.5, cy - 10.5, 16.0, 21.0)
                    p.drawArc(r2, -48 * 16, 96 * 16)
        else:
            # Crisp vector microphone glyph
            p.setBrush(QtGui.QColor(stroke_col.red(), stroke_col.green(), stroke_col.blue(), 28))
            p.setPen(QtGui.QPen(stroke_col, 1.85, QtCore.Qt.PenStyle.SolidLine, QtCore.Qt.PenCapStyle.RoundCap))
            p.drawRoundedRect(QtCore.QRectF(25.5, cy - 12.5, 8.0, 14.5), 4.0, 4.0)
            p.setBrush(QtCore.Qt.BrushStyle.NoBrush)
            p.setPen(QtGui.QPen(wave_col, 1.9, QtCore.Qt.PenStyle.SolidLine, QtCore.Qt.PenCapStyle.RoundCap))
            p.drawArc(QtCore.QRectF(22.5, cy - 7.5, 14.0, 14.0), -180 * 16, 180 * 16)
            p.drawLine(QtCore.QPointF(29.5, cy + 6.5), QtCore.QPointF(29.5, cy + 11.5))
            p.drawLine(QtCore.QPointF(24.5, cy + 11.5), QtCore.QPointF(34.5, cy + 11.5))
            if muted:
                p.drawLine(QtCore.QPointF(20.0, cy + 10.0), QtCore.QPointF(39.0, cy - 10.0))
        p.restore()

    def paintEvent(self, ev: QtGui.QPaintEvent) -> None:
        p = QtGui.QPainter(self)
        p.setRenderHint(QtGui.QPainter.RenderHint.Antialiasing)

        w, h = float(self.width()), float(self.height())
        r = QtCore.QRectF(1.0, 1.0, w - 2.0, h - 2.0)

        # Frosted translucent dark glass pill
        bg_grad = QtGui.QLinearGradient(0.0, 0.0, 0.0, h)
        bg_grad.setColorAt(0.0, QtGui.QColor(34, 40, 50, 178))
        bg_grad.setColorAt(1.0, QtGui.QColor(20, 24, 31, 188))
        p.setBrush(bg_grad)
        p.setPen(QtGui.QPen(QtGui.QColor(255, 255, 255, 42), 1.0))
        p.drawRoundedRect(r, 14.0, 14.0)

        # Inner subtle glass rim highlight
        p.setBrush(QtCore.Qt.BrushStyle.NoBrush)
        p.setPen(QtGui.QPen(QtGui.QColor(255, 255, 255, 15), 1.0))
        p.drawRoundedRect(QtCore.QRectF(2.0, 2.0, w - 4.0, h - 4.0), 13.0, 13.0)

        accent = QtGui.QColor(self._accent if self._accent.isValid() else "#22C55E")
        self._draw_audio_symbol(p, accent)

        # Label line: "79% — Bose QC Ultra 2 HP" (or "Muted — <Device>")
        text_rect = QtCore.QRectF(58.0, 11.0, w - 76.0, 22.0)
        f = p.font()
        f.setPixelSize(13)
        f.setWeight(QtGui.QFont.Weight.Medium)
        p.setFont(f)
        p.setPen(QtGui.QColor("#FCA5A5") if self._muted else QtGui.QColor(241, 245, 249, 238))
        raw_label = f"Muted — {self._title}" if self._muted else f"{self._pct}% — {self._title}"
        elided = QtGui.QFontMetricsF(f).elidedText(raw_label, QtCore.Qt.TextElideMode.ElideRight, text_rect.width())
        p.drawText(text_rect, QtCore.Qt.AlignmentFlag.AlignVCenter | QtCore.Qt.AlignmentFlag.AlignLeft, elided)

        # Emerald progress bar
        bar = QtCore.QRectF(58.0, 38.0, w - 76.0, 12.0)
        p.setPen(QtCore.Qt.PenStyle.NoPen)
        p.setBrush(QtGui.QColor(255, 255, 255, 28))
        p.drawRoundedRect(bar, 5.0, 5.0)

        frac = 0.0 if self._muted else min(1.0, self._pct / 100.0)
        if frac > 0.0:
            fill_w = max(8.0, frac * bar.width())
            fill_rect = QtCore.QRectF(bar.left(), bar.top(), fill_w, bar.height())
            bar_grad = QtGui.QLinearGradient(bar.left(), 0.0, bar.right(), 0.0)
            if self._is_mic:
                bar_grad.setColorAt(0.0, accent.darker(120))
                bar_grad.setColorAt(1.0, accent.lighter(115))
            else:
                bar_grad.setColorAt(0.0, QtGui.QColor("#0F8B6D"))
                bar_grad.setColorAt(0.65, QtGui.QColor("#1DB97F"))
                bar_grad.setColorAt(1.0, QtGui.QColor("#34D399") if self._pct <= 100 else QtGui.QColor("#F59E0B"))
            p.setBrush(bar_grad)
            p.drawRoundedRect(fill_rect, 5.0, 5.0)
        p.end()


@QtCore.pyqtClassInfo("D-Bus Interface", "org.kde.StatusNotifierItem")
class StatusNotifierItemAdaptor(QtDBus.QDBusAbstractAdaptor):
    """Native KDE org.kde.StatusNotifierItem D-Bus adaptor supporting vertical scroll + horizontal swipe."""

    NewIcon = QtCore.pyqtSignal()
    NewToolTip = QtCore.pyqtSignal()
    NewTitle = QtCore.pyqtSignal()
    NewStatus = QtCore.pyqtSignal(str)

    activated = QtCore.pyqtSignal(int, int)
    secondary_activated = QtCore.pyqtSignal(int, int)
    context_menu_requested = QtCore.pyqtSignal(int, int)
    scrolled = QtCore.pyqtSignal(int, str)

    def __init__(self, parent, icon_theme_dir: str):
        super().__init__(parent)
        self._icon_theme_dir = icon_theme_dir
        self._icon_name = "salsa-tray-active-glow"
        self._title = "Salsa — Linux WASAPI"
        self._status = "Active"

    def update_state(self, icon_name: str, title: str, status: str) -> None:
        icon_changed = self._icon_name != icon_name
        title_changed = self._title != title
        status_changed = self._status != status
        self._icon_name = icon_name
        self._title = title
        self._status = status
        if icon_changed:
            self.NewIcon.emit()
        if title_changed:
            self.NewTitle.emit()
            self.NewToolTip.emit()
        if status_changed:
            self.NewStatus.emit(status)

    @QtCore.pyqtProperty(str)
    def Category(self) -> str:
        return "ApplicationStatus"

    @QtCore.pyqtProperty(str)
    def Id(self) -> str:
        return "salsa-lwasapi-tray"

    @QtCore.pyqtProperty(str)
    def Title(self) -> str:
        return self._title

    @QtCore.pyqtProperty(str)
    def Status(self) -> str:
        return self._status

    @QtCore.pyqtProperty(int)
    def WindowId(self) -> int:
        return 0

    @QtCore.pyqtProperty(bool)
    def ItemIsMenu(self) -> bool:
        return False

    @QtCore.pyqtProperty(str)
    def IconName(self) -> str:
        return self._icon_name

    @QtCore.pyqtProperty(str)
    def IconThemePath(self) -> str:
        return self._icon_theme_dir

    @QtCore.pyqtSlot(int, int)
    def Activate(self, x: int, y: int) -> None:
        self.activated.emit(x, y)

    @QtCore.pyqtSlot(int, int)
    def SecondaryActivate(self, x: int, y: int) -> None:
        self.secondary_activated.emit(x, y)

    @QtCore.pyqtSlot(int, int)
    def ContextMenu(self, x: int, y: int) -> None:
        self.context_menu_requested.emit(x, y)

    @QtCore.pyqtSlot(int, str)
    def Scroll(self, delta: int, orientation: str) -> None:
        self.scrolled.emit(delta, orientation or "vertical")

    @QtCore.pyqtSlot(str)
    def ProvideXdgActivationToken(self, token: str) -> None:
        pass


def _active_kwin_output_name() -> str:
    """Return the active Wayland/X11 output name (e.g. 'HDMI-A-1', 'DP-3') from KWin D-Bus."""
    try:
        bus = QtDBus.QDBusConnection.sessionBus()
        if bus.isConnected():
            msg = QtDBus.QDBusMessage.createMethodCall(
                "org.kde.KWin",
                "/KWin",
                "org.kde.KWin",
                "activeOutputName",
            )
            reply = bus.call(msg, QtDBus.QDBus.CallMode.Block, 50)
            if reply.type() == QtDBus.QDBusMessage.MessageType.ReplyMessage and reply.arguments():
                return str(reply.arguments()[0] or "").strip()
    except Exception:
        pass
    return ""


def _read_plasma_panel_clearance_for(panel_cid: str | None) -> int:
    """Read thickness/floating state for a specific Plasma panel (or safe fallback)."""
    xdg = os.environ.get("XDG_CONFIG_HOME")
    base = xdg if (xdg and os.path.isabs(xdg) and not os.path.islink(xdg)) else os.path.join(os.path.expanduser("~"), ".config")
    rc = os.path.join(base, "plasmashellrc")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    try:
        fd = os.open(rc, flags)
        try:
            st = os.fstat(fd)
            if not stat.S_ISREG(st.st_mode) or st.st_size > 262144:
                return 60
            with os.fdopen(fd, "r", encoding="utf-8", errors="ignore") as f:
                fd = -1
                txt = f.read()
        finally:
            if fd >= 0:
                os.close(fd)
    except Exception:
        return 60

    if panel_cid:
        cur_sec = ""
        thickness: int | None = None
        floating = False
        sec_panel = f"[PlasmaViews][Panel {panel_cid}]"
        sec_defs = f"[PlasmaViews][Panel {panel_cid}][Defaults]"
        for raw in txt.splitlines():
            s = raw.strip()
            if s.startswith("["):
                cur_sec = s
                continue
            if cur_sec == sec_defs and s.startswith("thickness="):
                try:
                    thickness = int(s.split("=", 1)[1].strip())
                except ValueError:
                    pass
            elif cur_sec == sec_panel and s.startswith("floating="):
                floating = s.split("=", 1)[1].strip() in ("1", "true", "True")
        if thickness is not None:
            pad = 8 if floating else 0
            return max(40, min(96, thickness + pad))

    vals = [int(m.group(1)) for m in re.finditer(r"^\s*thickness=(\d+)\s*$", txt, re.MULTILINE)]
    if vals:
        return max(48, min(72, min(vals) + 6))
    return 60


def _plasma_tray_panel_info(preferred_screen_idx: int | None = None) -> tuple[int, int, int, int]:
    """Return (last_screen, location, clearance, right_pad) for the Plasma panel hosting systemtray."""
    last_screen = -1 if preferred_screen_idx is None else preferred_screen_idx
    location = 4
    right_pad = 16
    applets_cfg = os.path.expanduser("~/.config/plasma-org.kde.plasma.desktop-appletsrc")
    try:
        fd = os.open(applets_cfg, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        with os.fdopen(fd, "r", encoding="utf-8", errors="ignore") as fh:
            lines = fh.readlines()
    except OSError:
        return last_screen, location, _read_plasma_panel_clearance_for(None), right_pad

    containments: dict[str, dict[str, str]] = {}
    tray_panels: list[tuple[str, str]] = []  # (cid, tray_applet_id)
    cur_cid: str | None = None
    cur_aid: str | None = None
    cur_sub: str | None = None
    for raw in lines:
        line = raw.strip()
        if line.startswith("[") and line.endswith("]"):
            parts = line[1:-1].split("][")
            cur_cid = None
            cur_aid = None
            cur_sub = None
            if len(parts) == 2 and parts[0] == "Containments":
                cur_cid = parts[1]
                containments.setdefault(cur_cid, {})
            elif len(parts) == 3 and parts[0] == "Containments" and parts[2] == "General":
                cur_cid = parts[1]
                cur_sub = "General"
            elif len(parts) == 4 and parts[0] == "Containments" and parts[2] == "Applets":
                cur_cid = parts[1]
                cur_aid = parts[3]
            continue
        if "=" not in line or cur_cid is None:
            continue
        k, v = line.split("=", 1)
        k = k.strip()
        v = v.strip()
        if cur_aid is not None:
            if k == "plugin" and v == "org.kde.plasma.systemtray":
                tray_panels.append((cur_cid, cur_aid))
        elif cur_sub == "General":
            if k == "AppletOrder":
                containments.setdefault(cur_cid, {})["AppletOrder"] = v
        else:
            containments.setdefault(cur_cid, {})[k] = v

    chosen: tuple[str, str] | None = None
    for cid, aid in tray_panels:
        meta = containments.get(cid, {})
        if meta.get("formfactor") != "2":
            continue
        if preferred_screen_idx is not None:
            try:
                if int(meta.get("lastScreen", "-1")) == preferred_screen_idx:
                    chosen = (cid, aid)
                    break
            except ValueError:
                pass
        elif chosen is None:
            chosen = (cid, aid)
    if chosen is None:
        for cid, aid in tray_panels:
            if containments.get(cid, {}).get("formfactor") == "2":
                chosen = (cid, aid)
                break

    if chosen is not None:
        cid, aid = chosen
        meta = containments[cid]
        try:
            last_screen = int(meta.get("lastScreen", "-1"))
        except ValueError:
            pass
        try:
            location = int(meta.get("location", "4"))
        except ValueError:
            pass
        order_raw = meta.get("AppletOrder", "")
        if order_raw:
            order = [x.strip() for x in order_raw.split(";") if x.strip()]
            if aid in order:
                after_count = len(order) - 1 - order.index(aid)
                right_pad = max(16, min(264, after_count * 64))
        return last_screen, location, _read_plasma_panel_clearance_for(cid), right_pad

    return last_screen, location, _read_plasma_panel_clearance_for(None), right_pad


def _wayland_to_qt_point(gx: int, gy: int) -> QtCore.QPoint:
    """Map Plasma SNI coordinates (Wayland logical or physical device pixels) onto Qt screen space."""
    if QtWidgets.QApplication.platformName() != "xcb":
        return QtCore.QPoint(gx, gy)
    screens = QtWidgets.QApplication.screens()
    for s in screens:
        sg = s.geometry()
        dpr = max(1.0, float(s.devicePixelRatio() or 1.0))
        wx = round(sg.x() / dpr)
        wy = round(sg.y() / dpr)
        if wx <= gx < wx + sg.width() and wy <= gy < wy + sg.height():
            return QtCore.QPoint(sg.x() + (gx - wx), sg.y() + (gy - wy))
    for s in screens:
        sg = s.geometry()
        dpr = max(1.0, float(s.devicePixelRatio() or 1.0))
        px = round(sg.x() / dpr) * dpr
        py = round(sg.y() / dpr) * dpr
        pw = sg.width() * dpr
        ph = sg.height() * dpr
        if px <= gx < px + pw and py <= gy < py + ph:
            return QtCore.QPoint(sg.x() + round((gx - px) / dpr), sg.y() + round((gy - py) / dpr))
    return QtCore.QPoint(gx, gy)


def _plasma_panel_clearance() -> int:
    """Safely read KDE Plasma panel thickness so popups clear the bottom tray cleanly."""
    return _plasma_tray_panel_info()[2]


def _apply_x11_skip_taskbar(widget: QtWidgets.QWidget) -> None:
    """Mark an X11/XWayland tool window SKIP_TASKBAR & SKIP_PAGER so it never shows in the panel taskbar."""
    if QtWidgets.QApplication.platformName() != "xcb":
        return
    try:
        wid = int(widget.winId())
        if wid <= 0:
            return
        x11 = ctypes.CDLL("libX11.so.6")
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        x11.XDefaultRootWindow.restype = ctypes.c_ulong
        x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        x11.XInternAtom.restype = ctypes.c_ulong
        x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
        x11.XSendEvent.restype = ctypes.c_int
        x11.XFlush.argtypes = [ctypes.c_void_p]
        x11.XCloseDisplay.argtypes = [ctypes.c_void_p]

        dpy = x11.XOpenDisplay(None)
        if not dpy:
            return
        try:
            root = x11.XDefaultRootWindow(dpy)
            net_wm_state = x11.XInternAtom(dpy, b"_NET_WM_STATE", False)
            skip_tb = x11.XInternAtom(dpy, b"_NET_WM_STATE_SKIP_TASKBAR", False)
            skip_pg = x11.XInternAtom(dpy, b"_NET_WM_STATE_SKIP_PAGER", False)

            class _XClientMessageData(ctypes.Union):
                _fields_ = [("l", ctypes.c_long * 5)]

            class _XClientMessageEvent(ctypes.Structure):
                _fields_ = [
                    ("type", ctypes.c_int),
                    ("serial", ctypes.c_ulong),
                    ("send_event", ctypes.c_int),
                    ("display", ctypes.c_void_p),
                    ("window", ctypes.c_ulong),
                    ("message_type", ctypes.c_ulong),
                    ("format", ctypes.c_int),
                    ("data", _XClientMessageData),
                ]

            ev = _XClientMessageEvent()
            ev.type = 33  # ClientMessage
            ev.display = dpy
            ev.window = wid
            ev.message_type = net_wm_state
            ev.format = 32
            ev.data.l[0] = 1  # _NET_WM_STATE_ADD
            ev.data.l[1] = skip_tb
            ev.data.l[2] = skip_pg
            ev.data.l[3] = 1
            x11.XSendEvent(dpy, root, False, (1 << 20) | (1 << 19), ctypes.byref(ev))
            x11.XFlush(dpy)
        finally:
            x11.XCloseDisplay(dpy)
    except Exception:
        pass


def _apply_kde_blur_behind(widget: QtWidgets.QWidget, radius: int = 14) -> None:
    """Request KWin Gaussian blur behind a translucent rounded widget on X11/XWayland."""
    if QtWidgets.QApplication.platformName() != "xcb":
        return
    try:
        wid = int(widget.winId())
        if wid <= 0:
            return
        x11 = ctypes.CDLL("libX11.so.6")
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        x11.XInternAtom.restype = ctypes.c_ulong
        x11.XChangeProperty.argtypes = [
            ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong,
            ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int,
        ]
        x11.XFlush.argtypes = [ctypes.c_void_p]
        x11.XCloseDisplay.argtypes = [ctypes.c_void_p]

        dpy = x11.XOpenDisplay(None)
        if not dpy:
            return
        try:
            blur_atom = x11.XInternAtom(dpy, b"_KDE_NET_WM_BLUR_BEHIND_REGION", False)
            cardinal_atom = 6  # XA_CARDINAL
            inset = max(0, min(4, radius // 6))
            rect_data = (ctypes.c_ulong * 4)(
                inset,
                inset,
                max(1, widget.width() - inset * 2),
                max(1, widget.height() - inset * 2),
            )
            x11.XChangeProperty(
                dpy,
                wid,
                blur_atom,
                cardinal_atom,
                32,
                0,  # PropModeReplace
                ctypes.cast(rect_data, ctypes.c_void_p),
                4,
            )
            x11.XFlush(dpy)
        finally:
            x11.XCloseDisplay(dpy)
    except Exception:
        pass


class _SlideViewport(QtWidgets.QWidget):
    """Clip container that lets the popup card slide smoothly out of the panel edge."""

    def __init__(self, card: QtWidgets.QWidget, parent=None) -> None:
        super().__init__(parent)
        self._card = card
        self._card.setParent(self)
        self._offset = QtCore.QPoint(0, 0)

    def set_slide_offset(self, pt: QtCore.QPoint) -> None:
        self._offset = pt
        self._card.move(pt)

    def sizeHint(self) -> QtCore.QSize:
        return self._card.sizeHint()

    def minimumSizeHint(self) -> QtCore.QSize:
        return self._card.minimumSizeHint()

    def resizeEvent(self, ev: QtGui.QResizeEvent) -> None:
        super().resizeEvent(ev)
        self._card.setGeometry(
            self._offset.x(), self._offset.y(),
            self.width(), self.height(),
        )


class SalsaQuickMixerPopup(QtWidgets.QWidget):
    """Clean KDE Plasma-docked Quick Mixer popover with slide-up reveal."""

    def __init__(self, controller):
        super().__init__(
            None,
            QtCore.Qt.WindowType.Tool
            | QtCore.Qt.WindowType.FramelessWindowHint
            | QtCore.Qt.WindowType.WindowStaysOnTopHint,
        )
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_TranslucentBackground, True)
        self.ctrl = controller
        self._last_show_ts = 0.0
        self._syncing = False
        self._rate_hold_until = 0.0
        self._cap_rate_hold_until = 0.0
        self._bt_hold_until = 0.0
        self._dragging_win = False
        self._drag_offset = QtCore.QPoint()
        self._pinned_by_drag = False
        self._user_moved = False
        self._icon_resolver = AppIconResolver()
        self._app_rows = {}
        self._dev_list = []
        self._last_dev_poll = 0.0
        self._bt_pct = None
        self._last_bt_poll = 0.0
        self._bt_addr = ""
        self._bt_resetting = False
        self._cur_eq = None
        self._cur_rate = 48000
        self._cur_period = 256
        self._cur_bits = 32
        self._pending_master = None
        self._pending_mic = None
        self._pending_sess = {}
        self._slide_dir = QtCore.QPoint(0, 30)

        self.setFixedWidth(432)
        self._build_ui()

        self._slide_anim = QtCore.QVariantAnimation(self)
        self._slide_anim.setDuration(170)
        self._slide_anim.setStartValue(0.0)
        self._slide_anim.setEndValue(1.0)
        self._slide_anim.setEasingCurve(QtCore.QEasingCurve.Type.OutCubic)
        self._slide_anim.valueChanged.connect(self._on_slide_step)
        self._slide_anim.finished.connect(self._on_slide_finished)

        self._ipc_flush_timer = QtCore.QTimer(self)
        self._ipc_flush_timer.setInterval(30)
        self._ipc_flush_timer.setSingleShot(True)
        self._ipc_flush_timer.timeout.connect(self._flush_pending_sliders)

        self._peak_timer = QtCore.QTimer(self)
        self._peak_timer.setInterval(85)
        self._peak_timer.timeout.connect(self.refresh_live)

        self.apply_theme()

    def _on_slide_step(self, val: float) -> None:
        t = float(val)
        inv = max(0.0, 1.0 - t)
        self.viewport.set_slide_offset(
            QtCore.QPoint(round(self._slide_dir.x() * inv), round(self._slide_dir.y() * inv))
        )
        self.setWindowOpacity(min(1.0, 0.15 + 0.85 * t))

    def _on_slide_finished(self) -> None:
        self.viewport.set_slide_offset(QtCore.QPoint(0, 0))
        self.setWindowOpacity(1.0)

    def _prepare_slide_in(self, panel_loc: int) -> None:
        self._slide_anim.stop()
        if panel_loc == 3:
            self._slide_dir = QtCore.QPoint(0, -30)
        elif panel_loc == 5:
            self._slide_dir = QtCore.QPoint(-30, 0)
        elif panel_loc == 6:
            self._slide_dir = QtCore.QPoint(30, 0)
        else:
            self._slide_dir = QtCore.QPoint(0, 30)
        self.viewport.set_slide_offset(self._slide_dir)
        self.setWindowOpacity(0.15)

    @staticmethod
    def _make_hline() -> QtWidgets.QFrame:
        line = QtWidgets.QFrame()
        line.setObjectName("HLine")
        line.setFixedHeight(1)
        return line

    @classmethod
    def _make_section_header(cls, title: str) -> QtWidgets.QWidget:
        row = QtWidgets.QWidget()
        lay = QtWidgets.QHBoxLayout(row)
        lay.setContentsMargins(0, 2, 0, 1)
        lay.setSpacing(8)
        lbl = QtWidgets.QLabel(title)
        lbl.setObjectName("SectionTitle")
        lay.addWidget(lbl)
        lay.addWidget(cls._make_hline(), 1)
        return row

    def _on_tab_selected(self, idx: int) -> None:
        self.pages.setCurrentIndex(idx)
        self._keep_bottom_anchored()

    def _build_ui(self) -> None:
        outer = QtWidgets.QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)

        self.card = QtWidgets.QFrame()
        self.card.setObjectName("MixerCard")
        self.viewport = _SlideViewport(self.card, self)
        outer.addWidget(self.viewport)

        root = QtWidgets.QVBoxLayout(self.card)
        root.setContentsMargins(14, 10, 14, 10)
        root.setSpacing(8)

        # Header bar
        self.drag_bar = QtWidgets.QWidget()
        hdr = QtWidgets.QHBoxLayout(self.drag_bar)
        hdr.setContentsMargins(0, 0, 0, 0)
        hdr.setSpacing(7)

        self.hdr_icon = QtWidgets.QLabel()
        self.hdr_icon.setFixedSize(24, 24)
        hdr.addWidget(self.hdr_icon)

        self.lbl_title = QtWidgets.QLabel("Salsa")
        f = self.lbl_title.font()
        f.setBold(True)
        f.setPixelSize(15)
        self.lbl_title.setFont(f)
        hdr.addWidget(self.lbl_title)

        self.lbl_status_pill = QtWidgets.QLabel("ACTIVE")
        self.lbl_status_pill.setObjectName("StatusPill")
        hdr.addWidget(self.lbl_status_pill)
        hdr.addStretch(1)

        self.lbl_hw_badge = QtWidgets.QLabel("48 kHz · 32-bit")
        self.lbl_hw_badge.setObjectName("HwBadge")
        hdr.addWidget(self.lbl_hw_badge)

        self.btn_pin = QtWidgets.QPushButton("Pin")
        self.btn_pin.setObjectName("HdrToggle")
        self.btn_pin.setCheckable(True)
        self.btn_pin.setCursor(QtCore.Qt.CursorShape.PointingHandCursor)
        self.btn_pin.setToolTip("Keep popover open while clicking other windows")
        hdr.addWidget(self.btn_pin)

        self.btn_close = QtWidgets.QPushButton("×")
        self.btn_close.setObjectName("HdrClose")
        self.btn_close.setFixedSize(22, 22)
        self.btn_close.setCursor(QtCore.Qt.CursorShape.PointingHandCursor)
        self.btn_close.setToolTip("Close Quick Mixer")
        self.btn_close.clicked.connect(self.hide)
        hdr.addWidget(self.btn_close)

        root.addWidget(self.drag_bar)

        # KDE Plasma-style underline tab bar (Devices | Applications)
        tab_wrap = QtWidgets.QWidget()
        tab_lay = QtWidgets.QHBoxLayout(tab_wrap)
        tab_lay.setContentsMargins(0, 0, 0, 0)
        tab_lay.setSpacing(4)

        self.btn_tab_devices = QtWidgets.QPushButton("Devices")
        self.btn_tab_devices.setObjectName("TabUnderline")
        self.btn_tab_devices.setCheckable(True)
        self.btn_tab_devices.setChecked(True)
        self.btn_tab_devices.setCursor(QtCore.Qt.CursorShape.PointingHandCursor)

        self.btn_tab_apps = QtWidgets.QPushButton("Applications")
        self.btn_tab_apps.setObjectName("TabUnderline")
        self.btn_tab_apps.setCheckable(True)
        self.btn_tab_apps.setCursor(QtCore.Qt.CursorShape.PointingHandCursor)

        self._tab_group = QtWidgets.QButtonGroup(self)
        self._tab_group.setExclusive(True)
        self._tab_group.addButton(self.btn_tab_devices, 0)
        self._tab_group.addButton(self.btn_tab_apps, 1)
        self._tab_group.idClicked.connect(self._on_tab_selected)

        tab_lay.addWidget(self.btn_tab_devices)
        tab_lay.addWidget(self.btn_tab_apps)
        tab_lay.addStretch(1)
        root.addWidget(tab_wrap)
        root.addWidget(self._make_hline())

        # Stacked content pages (Page 0: Devices, Page 1: Applications)
        self.pages = QtWidgets.QStackedWidget()
        root.addWidget(self.pages)

        # --- Page 0: Devices & Quick DSP ---
        dev_page = QtWidgets.QWidget()
        dev_lay = QtWidgets.QVBoxLayout(dev_page)
        dev_lay.setContentsMargins(0, 2, 0, 2)
        dev_lay.setSpacing(8)

        # Bluetooth live pill (auto-hidden when wired)
        self.bt_frame = QtWidgets.QFrame()
        self.bt_frame.setObjectName("BtPill")
        bt_lay = QtWidgets.QHBoxLayout(self.bt_frame)
        bt_lay.setContentsMargins(10, 6, 8, 6)
        bt_lay.setSpacing(8)
        self.lbl_bt_info = QtWidgets.QLabel("Bluetooth Ready")
        self.lbl_bt_info.setWordWrap(False)
        bt_lay.addWidget(self.lbl_bt_info, 1)
        self.btn_reset_bt = QtWidgets.QPushButton("Reset BT")
        self.btn_reset_bt.setObjectName("SmallBtn")
        self.btn_reset_bt.setCursor(QtCore.Qt.CursorShape.PointingHandCursor)
        self.btn_reset_bt.clicked.connect(self._on_reset_bt)
        bt_lay.addWidget(self.btn_reset_bt)
        self.bt_frame.hide()
        dev_lay.addWidget(self.bt_frame)

        # Output Devices section
        dev_lay.addWidget(self._make_section_header("Playback Devices"))
        out_row = QtWidgets.QHBoxLayout()
        out_row.setSpacing(8)
        lbl_out_d = QtWidgets.QLabel("Output")
        lbl_out_d.setObjectName("DimLabel")
        lbl_out_d.setFixedWidth(44)
        self.cmb_out_dev = QtWidgets.QComboBox()
        self.cmb_out_dev.currentIndexChanged.connect(self._on_device_changed)
        out_row.addWidget(lbl_out_d)
        out_row.addWidget(self.cmb_out_dev, 1)
        dev_lay.addLayout(out_row)

        m_box = QtWidgets.QVBoxLayout()
        m_box.setContentsMargins(0, 0, 0, 0)
        m_box.setSpacing(4)
        m_row = QtWidgets.QHBoxLayout()
        m_row.setSpacing(6)
        self.fader_master = SwipeFader("Master Output", accent="#22C55E")
        self.fader_master.value_changed.connect(self._on_master_drag)
        self.fader_master.value_committed.connect(self._on_master_commit)
        m_row.addWidget(self.fader_master, 1)
        self.btn_mute_master = QtWidgets.QPushButton("Mute")
        self.btn_mute_master.setObjectName("MuteBtn")
        self.btn_mute_master.setFixedSize(56, 34)
        self.btn_mute_master.clicked.connect(self.ctrl.toggle_master_mute)
        m_row.addWidget(self.btn_mute_master)
        m_box.addLayout(m_row)
        self.peak_bar = PeakBarWidget()
        m_box.addWidget(self.peak_bar)
        dev_lay.addLayout(m_box)

        # Input Devices section
        dev_lay.addWidget(self._make_section_header("Recording Devices"))
        mic_sel_row = QtWidgets.QHBoxLayout()
        mic_sel_row.setSpacing(8)
        lbl_mic_d = QtWidgets.QLabel("Input")
        lbl_mic_d.setObjectName("DimLabel")
        lbl_mic_d.setFixedWidth(44)
        self.cmb_mic_dev = QtWidgets.QComboBox()
        self.cmb_mic_dev.currentIndexChanged.connect(self._on_device_changed)
        mic_sel_row.addWidget(lbl_mic_d)
        mic_sel_row.addWidget(self.cmb_mic_dev, 1)
        dev_lay.addLayout(mic_sel_row)

        mic_row = QtWidgets.QHBoxLayout()
        mic_row.setSpacing(6)
        self.fader_mic = SwipeFader("Mic / Input Gain", accent="#38BDF8")
        self.fader_mic.value_changed.connect(self._on_mic_drag)
        self.fader_mic.value_committed.connect(self._on_mic_commit)
        mic_row.addWidget(self.fader_mic, 1)
        self.btn_mute_mic = QtWidgets.QPushButton("Mute")
        self.btn_mute_mic.setObjectName("MuteBtn")
        self.btn_mute_mic.setFixedSize(56, 34)
        self.btn_mute_mic.clicked.connect(self.ctrl.toggle_mic_mute)
        mic_row.addWidget(self.btn_mute_mic)
        dev_lay.addLayout(mic_row)

        # EQ & Engine section
        dev_lay.addWidget(self._make_section_header("EQ & Engine"))
        dsp_frame = QtWidgets.QFrame()
        dsp_frame.setObjectName("SubCard")
        dsp_frame.setSizePolicy(
            QtWidgets.QSizePolicy.Policy.Expanding,
            QtWidgets.QSizePolicy.Policy.Minimum,
        )
        dsp_lay = QtWidgets.QVBoxLayout(dsp_frame)
        dsp_lay.setContentsMargins(9, 7, 9, 7)
        dsp_lay.setSpacing(6)

        eq_row = QtWidgets.QHBoxLayout()
        eq_row.setSpacing(6)
        lbl_eq = QtWidgets.QLabel("EQ Preset")
        lbl_eq.setObjectName("DimLabel")
        eq_row.addWidget(lbl_eq)
        self.cmb_eq = QtWidgets.QComboBox()
        for name in PRESETS:
            self.cmb_eq.addItem(name)
        self.cmb_eq.currentIndexChanged.connect(self._on_eq_preset_selected)
        eq_row.addWidget(self.cmb_eq, 1)
        self.btn_eq_bypass = QtWidgets.QPushButton("Bypass")
        self.btn_eq_bypass.setObjectName("TogglePill")
        self.btn_eq_bypass.setCheckable(True)
        self.btn_eq_bypass.clicked.connect(self._on_eq_toggle)
        eq_row.addWidget(self.btn_eq_bypass)
        dsp_lay.addLayout(eq_row)

        mode_row = QtWidgets.QHBoxLayout()
        mode_row.setSpacing(5)
        lbl_sr = QtWidgets.QLabel("Rate")
        lbl_sr.setObjectName("DimLabel")
        mode_row.addWidget(lbl_sr)
        self.cmb_rate = QtWidgets.QComboBox()
        for r in (44100, 48000, 88200, 96000, 176400, 192000):
            self.cmb_rate.addItem(f"{r/1000:g} kHz", r)
        self.cmb_rate.currentIndexChanged.connect(self._on_rate_changed)
        mode_row.addWidget(self.cmb_rate)

        self.btn_per_dev_eq = QtWidgets.QPushButton("Per-Dev EQ")
        self.btn_per_dev_eq.setObjectName("TogglePill")
        self.btn_per_dev_eq.setCheckable(True)
        self.btn_per_dev_eq.clicked.connect(self._on_per_dev_eq_toggled)
        mode_row.addWidget(self.btn_per_dev_eq)

        self.btn_limiter = QtWidgets.QPushButton("Limiter")
        self.btn_limiter.setObjectName("TogglePill")
        self.btn_limiter.setCheckable(True)
        self.btn_limiter.clicked.connect(self._on_limiter_toggled)
        mode_row.addWidget(self.btn_limiter)

        self.btn_midi_test = QtWidgets.QPushButton("MIDI")
        self.btn_midi_test.setObjectName("SmallBtn")
        self.btn_midi_test.setToolTip("Send Test MIDI Note (C4)")
        self.btn_midi_test.clicked.connect(self._on_test_midi)
        mode_row.addWidget(self.btn_midi_test)
        dsp_lay.addLayout(mode_row)

        apo_tray_row = QtWidgets.QHBoxLayout()
        apo_tray_row.setSpacing(5)
        self.btn_mic_apo = QtWidgets.QPushButton("Mic APO")
        self.btn_mic_apo.setObjectName("TogglePill")
        self.btn_mic_apo.setCheckable(True)
        self.btn_mic_apo.setToolTip("Toggle real-time Capture Mic APO (HPF, Gate, RNNoise, Comp)")
        self.btn_mic_apo.clicked.connect(self._on_tray_mic_apo_toggled)
        apo_tray_row.addWidget(self.btn_mic_apo)

        self.btn_crossfeed = QtWidgets.QPushButton("Crossfeed")
        self.btn_crossfeed.setObjectName("TogglePill")
        self.btn_crossfeed.setCheckable(True)
        self.btn_crossfeed.setToolTip("Toggle Bauer headphone stereo crossfeed")
        self.btn_crossfeed.clicked.connect(self._on_tray_crossfeed_toggled)
        apo_tray_row.addWidget(self.btn_crossfeed)

        self.cmb_bt_pref_tray = QtWidgets.QComboBox()
        for lbl_c, code_c in (("BT: Auto", 0), ("LDAC", 1), ("aptX HD", 2), ("aptX", 3), ("AAC", 4), ("SBC", 5)):
            self.cmb_bt_pref_tray.addItem(lbl_c, code_c)
        self.cmb_bt_pref_tray.currentIndexChanged.connect(self._on_tray_bt_codec_changed)
        apo_tray_row.addWidget(self.cmb_bt_pref_tray, 1)

        self.btn_bt_abr_tray = QtWidgets.QPushButton("BT ABR")
        self.btn_bt_abr_tray.setObjectName("TogglePill")
        self.btn_bt_abr_tray.setCheckable(True)
        self.btn_bt_abr_tray.setToolTip("Toggle Wireless Adaptive Bitrate (LDAC/AAC)")
        self.btn_bt_abr_tray.clicked.connect(self._on_tray_bt_codec_changed)
        apo_tray_row.addWidget(self.btn_bt_abr_tray)
        dsp_lay.addLayout(apo_tray_row)

        mic_clock_row = QtWidgets.QHBoxLayout()
        mic_clock_row.setSpacing(6)
        lbl_mic_sr = QtWidgets.QLabel("Mic Rate")
        lbl_mic_sr.setObjectName("DimLabel")
        mic_clock_row.addWidget(lbl_mic_sr)
        self.cmb_cap_rate = QtWidgets.QComboBox()
        self.cmb_cap_rate.addItem("Auto (Follow Master)", 0)
        for r in SUPPORTED_SAMPLE_RATES:
            self.cmb_cap_rate.addItem(f"{r/1000:g} kHz", r)
        self.cmb_cap_rate.setToolTip("Pin capture/microphone hardware clock independently of Master playback clock")
        self.cmb_cap_rate.currentIndexChanged.connect(self._on_cap_rate_changed)
        mic_clock_row.addWidget(self.cmb_cap_rate, 1)

        lbl_mp = QtWidgets.QLabel("Mic Profile")
        lbl_mp.setObjectName("DimLabel")
        mic_clock_row.addWidget(lbl_mp)
        self.cmb_mic_preset_tray = QtWidgets.QComboBox()
        self.cmb_mic_preset_tray.addItem("Custom Mic APO", "")
        for pname in MIC_APO_PRESETS:
            self.cmb_mic_preset_tray.addItem(pname, pname)
        self.cmb_mic_preset_tray.setToolTip("Apply quick reversible Capture Mic APO voice preset")
        self.cmb_mic_preset_tray.activated.connect(self._on_tray_mic_preset_selected)
        mic_clock_row.addWidget(self.cmb_mic_preset_tray, 1)
        dsp_lay.addLayout(mic_clock_row)

        theme_row = QtWidgets.QHBoxLayout()
        theme_row.setSpacing(6)
        lbl_theme = QtWidgets.QLabel("Theme")
        lbl_theme.setObjectName("DimLabel")
        theme_row.addWidget(lbl_theme)
        self.cmb_tray_theme = QtWidgets.QComboBox()
        self.cmb_tray_theme.addItem("Follow Studio Theme")
        for t_name in THEMES:
            self.cmb_tray_theme.addItem(t_name)
        if "Custom Studio Theme" not in THEMES:
            self.cmb_tray_theme.addItem("Custom Studio Theme")
        self.cmb_tray_theme.currentIndexChanged.connect(self._on_tray_theme_changed)
        theme_row.addWidget(self.cmb_tray_theme, 1)
        dsp_lay.addLayout(theme_row)

        dev_lay.addWidget(dsp_frame)
        dev_lay.addStretch(1)
        self.pages.addWidget(dev_page)

        # --- Page 1: Active Applications ---
        apps_page = QtWidgets.QWidget()
        apps_page_lay = QtWidgets.QVBoxLayout(apps_page)
        apps_page_lay.setContentsMargins(0, 2, 0, 2)
        apps_page_lay.setSpacing(8)

        apps_hdr = QtWidgets.QHBoxLayout()
        lbl_apps = QtWidgets.QLabel("ACTIVE APP STREAMS")
        lbl_apps.setObjectName("SectionCap")
        apps_hdr.addWidget(lbl_apps)
        apps_hdr.addStretch(1)
        self.lbl_app_count = QtWidgets.QLabel("0 active")
        self.lbl_app_count.setObjectName("DimLabel")
        apps_hdr.addWidget(self.lbl_app_count)
        apps_page_lay.addLayout(apps_hdr)

        self.apps_scroll = QtWidgets.QScrollArea()
        self.apps_scroll.setWidgetResizable(True)
        self.apps_scroll.setFrameShape(QtWidgets.QFrame.Shape.NoFrame)
        self.apps_scroll.setMinimumHeight(210)
        self.apps_scroll.setMaximumHeight(320)
        self.apps_container = QtWidgets.QWidget()
        self.apps_layout = QtWidgets.QVBoxLayout(self.apps_container)
        self.apps_layout.setContentsMargins(0, 0, 2, 0)
        self.apps_layout.setSpacing(6)
        self.lbl_no_apps = QtWidgets.QLabel("No active audio streams right now")
        self.lbl_no_apps.setObjectName("DimLabel")
        self.lbl_no_apps.setAlignment(QtCore.Qt.AlignmentFlag.AlignCenter)
        self.apps_layout.addWidget(self.lbl_no_apps)
        self.apps_layout.addStretch(1)
        self.apps_scroll.setWidget(self.apps_container)
        apps_page_lay.addWidget(self.apps_scroll, 1)
        self.pages.addWidget(apps_page)

        # Footer quick-launch buttons
        root.addWidget(self._make_hline())
        ftr = QtWidgets.QHBoxLayout()
        ftr.setSpacing(6)
        self.btn_studio = QtWidgets.QPushButton("Open Studio")
        self.btn_studio.setObjectName("PrimaryBtn")
        self.btn_studio.clicked.connect(lambda: self._run_then_hide(self.ctrl.launch_studio))
        self.btn_graph = QtWidgets.QPushButton("Graph")
        self.btn_graph.setObjectName("FooterBtn")
        self.btn_graph.clicked.connect(lambda: self._run_then_hide(self.ctrl.launch_graph))
        self.btn_debug = QtWidgets.QPushButton("Debug")
        self.btn_debug.setObjectName("FooterBtn")
        self.btn_debug.clicked.connect(lambda: self._run_then_hide(self.ctrl.launch_debugger))
        self.btn_restart = QtWidgets.QPushButton("Restart")
        self.btn_restart.setObjectName("WarnBtn")
        self.btn_restart.clicked.connect(self.ctrl.restart_engine)
        for b in (self.btn_studio, self.btn_graph, self.btn_debug, self.btn_restart):
            b.setCursor(QtCore.Qt.CursorShape.PointingHandCursor)
            ftr.addWidget(b, 1 if b is self.btn_studio else 0)
        root.addLayout(ftr)

    def _on_tray_theme_changed(self) -> None:
        if self._syncing or not hasattr(self, "cmb_tray_theme"):
            return
        choice = self.cmb_tray_theme.currentText().strip()
        if not choice:
            return
        update_gui_settings({
            "tray_theme": choice,
            "tray_breeze_emerald": (choice == "Plasma Breeze Emerald"),
        })
        self.apply_theme()
        self.refresh_live()

    def apply_theme(self) -> None:
        settings = load_gui_settings()
        t = resolve_tray_theme(settings)
        self._theme = t
        if hasattr(self, "cmb_tray_theme"):
            wanted_name = resolve_tray_theme_name(settings)
            idx = self.cmb_tray_theme.findText(wanted_name)
            if idx >= 0 and idx != self.cmb_tray_theme.currentIndex():
                self.cmb_tray_theme.blockSignals(True)
                self.cmb_tray_theme.setCurrentIndex(idx)
                self.cmb_tray_theme.blockSignals(False)
        bg_card = t["bg_card"]
        bg_deep = t["bg_deep"]
        bg_ctrl = t["bg_control"]
        bg_hover = t["bg_hover"]
        border_card = t["border_card"]
        border_ctrl = t["border_control"]
        txt = t["text_primary"]
        bright = t["text_bright"]
        muted = t["text_muted"]
        acc = t["accent_primary"]
        sec = t["accent_secondary"]
        mic = t["accent_mic"]
        warn = t["accent_warn"]

        self.fader_master.set_theme_colors(acc, bg_ctrl, border_ctrl, bright, muted)
        self.fader_mic.set_theme_colors(sec, bg_ctrl, border_ctrl, bright, muted)
        for _, _, fader, _ in self._app_rows.values():
            fader.set_theme_colors(acc, bg_ctrl, border_ctrl, bright, muted)

        self.setStyleSheet(
            f"""
            QFrame#MixerCard {{
                background-color: {bg_card};
                border: 1px solid {border_ctrl};
                border-top-left-radius: 10px;
                border-top-right-radius: 10px;
                border-bottom-left-radius: 0px;
                border-bottom-right-radius: 0px;
            }}
            QFrame#HLine {{
                background-color: {border_card};
                border: none;
                min-height: 1px;
                max-height: 1px;
            }}
            QFrame#SubCard {{
                background-color: {bg_deep};
                border: 1px solid {border_card};
                border-radius: 10px;
            }}
            QFrame#BtPill {{
                background-color: rgba(56, 189, 248, 30);
                border: 1px solid rgba(56, 189, 248, 125);
                border-radius: 11px;
            }}
            QLabel {{ color: {txt}; background: transparent; }}
            QLabel#DimLabel {{ color: {muted}; font-size: 11px; font-weight: 600; }}
            QLabel#SectionTitle {{ color: {muted}; font-size: 11px; font-weight: 700; }}
            QLabel#SectionCap {{ color: {muted}; font-size: 10px; font-weight: 700; letter-spacing: 0.8px; }}
            QLabel#DragGrip {{ color: {muted}; font-size: 13px; padding: 0px 2px 0px 0px; }}
            QLabel#HwBadge {{
                background-color: rgba(20, 184, 102, 34); border: 1px solid rgba(20, 184, 102, 110);
                border-radius: 9px; padding: 3px 8px; font-size: 11px; color: {acc}; font-weight: 700;
            }}
            QLabel#StatusPill {{
                background-color: rgba(20, 184, 102, 38); color: {acc};
                border: 1px solid {acc}; border-radius: 8px; padding: 2px 7px; font-size: 10px; font-weight: 700;
            }}
            QPushButton#TabUnderline {{
                background: transparent;
                color: {muted};
                border: none;
                border-bottom: 2px solid transparent;
                padding: 5px 12px 7px 12px;
                font-size: 12px;
                font-weight: 600;
            }}
            QPushButton#TabUnderline:hover {{ color: {txt}; }}
            QPushButton#TabUnderline:checked {{
                color: {bright};
                border-bottom: 2px solid {acc};
            }}
            QComboBox {{
                background-color: {bg_ctrl}; color: {bright}; border: 1px solid {border_ctrl};
                border-radius: 7px; padding: 3px 8px; min-height: 20px; font-size: 11px; font-weight: 600;
            }}
            QComboBox::drop-down {{ border: none; width: 18px; }}
            QComboBox QAbstractItemView {{
                background-color: {bg_card}; color: {bright}; selection-background-color: {bg_hover};
                selection-color: {acc}; border: 1px solid {border_ctrl};
            }}
            QPushButton#MuteBtn, QPushButton#SmallBtn, QPushButton#FooterBtn, QPushButton#TogglePill {{
                background-color: {bg_ctrl}; color: {txt}; border: 1px solid {border_ctrl};
                border-radius: 8px; padding: 3px 9px; min-height: 18px; font-size: 11px; font-weight: 600;
            }}
            QPushButton#HdrToggle, QPushButton#HdrClose {{
                background-color: {bg_ctrl}; color: {muted}; border: 1px solid {border_ctrl};
                border-radius: 7px; padding: 2px 6px; font-size: 10px; font-weight: 700;
            }}
            QPushButton#HdrToggle:hover, QPushButton#HdrClose:hover {{
                background-color: {bg_hover}; color: {bright}; border-color: {acc};
            }}
            QPushButton#HdrToggle:checked {{
                background-color: {bg_hover}; border-color: {acc}; color: {acc};
            }}
            QPushButton#MuteBtn:hover, QPushButton#SmallBtn:hover, QPushButton#FooterBtn:hover {{
                background-color: {bg_hover};
            }}
            QPushButton#TogglePill:checked {{
                background-color: {bg_hover}; border-color: {acc}; color: {acc};
            }}
            QPushButton#PrimaryBtn {{
                background-color: {t["btn_primary_bg"]}; color: #ffffff;
                border: 1px solid {t["btn_primary_border"]}; border-radius: 8px;
                padding: 5px 10px; min-height: 20px; font-size: 11px; font-weight: 700;
            }}
            QPushButton#PrimaryBtn:hover {{ background-color: {t["btn_primary_hover"]}; }}
            QPushButton#WarnBtn {{
                background-color: {bg_ctrl}; color: {warn}; border: 1px solid {border_ctrl};
                border-radius: 8px; padding: 5px 9px; min-height: 20px; font-size: 11px; font-weight: 600;
            }}
            QPushButton#WarnBtn:hover {{ border-color: {warn}; background-color: {bg_hover}; }}
            QScrollArea {{ background: transparent; }}
            QWidget {{ background: transparent; }}
            """
        )

    def _run_then_hide(self, fn) -> None:
        self.hide()
        fn()

    def showEvent(self, ev: QtGui.QShowEvent) -> None:
        super().showEvent(ev)
        _apply_x11_skip_taskbar(self)

    def resizeEvent(self, ev: QtGui.QResizeEvent) -> None:
        super().resizeEvent(ev)
        if (
            self.isVisible()
            and getattr(self, "_anchored_above_panel", False)
            and not getattr(self, "_positioning", False)
            and getattr(self, "_anchor_panel_loc", 4) not in (3, 5, 6)
        ):
            avail = getattr(self, "_anchor_avail", None)
            top_limit = (avail.top() + 8) if avail is not None else 8
            wanted_y = max(top_limit, self._anchor_bottom - self.height())
            if self.y() != wanted_y:
                self.move(self.x(), wanted_y)

    def show_near(self, gx: int, gy: int) -> None:
        self._positioning = True
        try:
            self.apply_theme()
            self._last_dev_poll = 0.0
            self._anchored_above_panel = False
            self.refresh_live()
            self.ensurePolished()
            for w_ch in self.findChildren(QtWidgets.QWidget):
                w_ch.ensurePolished()
                w_ch.updateGeometry()
            for lay in reversed(self.findChildren(QtWidgets.QLayout)):
                lay.invalidate()
            if self.layout():
                self.layout().activate()
            self.adjustSize()
            sh = self.sizeHint()
            w = min(self.maximumWidth(), max(self.minimumWidth(), sh.width()))
            h = max(self.minimumHeight(), sh.height())
            self.resize(w, h)

            screens = QtWidgets.QApplication.screens()
            if gx > 0 or gy > 0:
                pt = _wayland_to_qt_point(gx, gy)
                screen = QtWidgets.QApplication.screenAt(pt) or QtWidgets.QApplication.primaryScreen()
                pref_idx = screens.index(screen) if (screen in screens) else None
                panel_screen_idx, panel_loc, clearance, right_pad = _plasma_tray_panel_info(pref_idx)
            else:
                pt = None
                pref_idx = None
                active_out = _active_kwin_output_name()
                if active_out:
                    for i, s in enumerate(screens):
                        if s.name() == active_out:
                            pref_idx = i
                            break
                panel_screen_idx, panel_loc, clearance, right_pad = _plasma_tray_panel_info(pref_idx)
                if 0 <= panel_screen_idx < len(screens):
                    screen = screens[panel_screen_idx]
                elif pref_idx is not None and 0 <= pref_idx < len(screens):
                    screen = screens[pref_idx]
                else:
                    screen = QtWidgets.QApplication.primaryScreen()

            if screen:
                avail = screen.availableGeometry()
                if avail.width() < 200 or avail.height() < 200:
                    avail = screen.geometry()
                tray_x = max(avail.left() + 8, min(avail.right() + 1 - w - right_pad, avail.right() - w - 8))
                if panel_loc == 3:  # Top panel — dock flush below panel over systemtray
                    x = tray_x
                    y = min(avail.bottom() - h - 8, avail.top() + clearance)
                elif panel_loc == 5:  # Left panel
                    x = min(avail.right() - w - 8, avail.left() + clearance)
                    raw_y = (pt.y() - h // 2) if pt is not None else (avail.bottom() - h - 16)
                    y = max(avail.top() + 8, min(raw_y, avail.bottom() - h - 8))
                elif panel_loc == 6:  # Right panel
                    x = max(avail.left() + 8, avail.right() + 1 - w - clearance)
                    raw_y = (pt.y() - h // 2) if pt is not None else (avail.bottom() - h - 16)
                    y = max(avail.top() + 8, min(raw_y, avail.bottom() - h - 8))
                else:  # Bottom panel (default) — dock flush above panel over systemtray like KDE
                    x = tray_x
                    y = max(avail.top() + 8, avail.bottom() + 1 - h - clearance)
                self.setAttribute(QtCore.Qt.WidgetAttribute.WA_Moved, True)
                self.move(x, y)
                self._anchor_avail = avail
                self._anchor_panel_loc = panel_loc
                self._anchor_bottom = y + h
                self._anchored_above_panel = True
        finally:
            self._positioning = False

        self._last_show_ts = time.monotonic()
        self._prepare_slide_in(getattr(self, "_anchor_panel_loc", 4))
        self.show()
        _apply_x11_skip_taskbar(self)
        self.raise_()
        self.activateWindow()
        self._slide_anim.start()
        self._peak_timer.start()

    def hideEvent(self, ev: QtGui.QHideEvent) -> None:
        self._slide_anim.stop()
        self.viewport.set_slide_offset(QtCore.QPoint(0, 0))
        self.setWindowOpacity(1.0)
        self._peak_timer.stop()
        super().hideEvent(ev)

    def keyPressEvent(self, ev: QtGui.QKeyEvent) -> None:
        if ev.key() == QtCore.Qt.Key.Key_Escape:
            self.hide()
            return
        super().keyPressEvent(ev)

    def changeEvent(self, ev: QtGui.QEvent) -> None:
        if ev.type() == QtCore.QEvent.Type.ActivationChange and not self.isActiveWindow():
            if (
                not self.btn_pin.isChecked()
                and QtWidgets.QApplication.activePopupWidget() is None
                and (time.monotonic() - self._last_show_ts) > 0.25
            ):
                self.hide()
        super().changeEvent(ev)

    def _on_master_drag(self, v: int) -> None:
        self._pending_master = v
        if not self._ipc_flush_timer.isActive():
            self._ipc_flush_timer.start()

    def _on_master_commit(self, v: int) -> None:
        self._pending_master = None
        self.ctrl.engine.set_master_volume(v, -1)
        self.ctrl.sync_now()

    def _on_mic_drag(self, v: int) -> None:
        self._pending_mic = v
        if not self._ipc_flush_timer.isActive():
            self._ipc_flush_timer.start()

    def _on_mic_commit(self, v: int) -> None:
        self._pending_mic = None
        self.ctrl.engine.set_mic_volume(v, -1)
        self.ctrl.sync_now()

    def _on_sess_drag(self, sid: int, v: int) -> None:
        self._pending_sess[sid] = v
        if not self._ipc_flush_timer.isActive():
            self._ipc_flush_timer.start()

    def _on_sess_commit(self, sid: int, v: int) -> None:
        self._pending_sess.pop(sid, None)
        self.ctrl.engine.set_session_control(sid, v, -1)

    def _flush_pending_sliders(self) -> None:
        if self._pending_master is not None:
            v = self._pending_master
            self._pending_master = None
            self.ctrl.engine.set_master_volume(v, -1)
        if self._pending_mic is not None:
            v = self._pending_mic
            self._pending_mic = None
            self.ctrl.engine.set_mic_volume(v, -1)
        if self._pending_sess:
            items = list(self._pending_sess.items())
            self._pending_sess.clear()
            for sid, v in items:
                self.ctrl.engine.set_session_control(sid, v, -1)

    def _on_device_changed(self) -> None:
        if self._syncing:
            return
        pb_id = self.cmb_out_dev.currentData() or ""
        cap_id = self.cmb_mic_dev.currentData() or ""
        if pb_id or cap_id:
            self.ctrl.engine.set_devices(pb_id, cap_id)
            QtCore.QTimer.singleShot(120, self.refresh_live)

    def _on_eq_preset_selected(self) -> None:
        if self._syncing or not self._cur_eq:
            return
        name = self.cmb_eq.currentText()
        pr = PRESETS.get(name)
        if not pr:
            return
        cfg = LwasapiEqConfig()
        ctypes.memmove(ctypes.byref(cfg), ctypes.byref(self._cur_eq), ctypes.sizeof(LwasapiEqConfig))
        cfg.enabled = 1
        cfg.dynamic_mode = int(pr["mode"])
        cfg.preamp_db = float(pr["preamp"])
        cfg.dyn_threshold_db = float(pr["thresh"])
        cfg.dyn_ratio = float(pr["ratio"])
        for i in range(LWASAPI_EQ_BANDS):
            cfg.band_gains_db[i] = float(pr["gains"][i])
            if cfg.band_q[i] <= 0.1:
                cfg.band_q[i] = 1.41
        cfg.preset_name = name.encode("utf-8")[:31]
        self.ctrl.engine.set_eq_config(cfg)
        self.refresh_live()

    def _on_eq_toggle(self) -> None:
        if self._syncing or not self._cur_eq:
            return
        cfg = LwasapiEqConfig()
        ctypes.memmove(ctypes.byref(cfg), ctypes.byref(self._cur_eq), ctypes.sizeof(LwasapiEqConfig))
        cfg.enabled = 0 if self.btn_eq_bypass.isChecked() else 1
        self.ctrl.engine.set_eq_config(cfg)
        self.refresh_live()

    def _on_limiter_toggled(self) -> None:
        if self._syncing or not self._cur_eq:
            return
        cfg = LwasapiEqConfig()
        ctypes.memmove(ctypes.byref(cfg), ctypes.byref(self._cur_eq), ctypes.sizeof(LwasapiEqConfig))
        cfg.limiter_enabled = 1 if self.btn_limiter.isChecked() else 0
        self.ctrl.engine.set_eq_config(cfg)
        self.refresh_live()

    def _on_per_dev_eq_toggled(self) -> None:
        if self._syncing:
            return
        val = bool(self.btn_per_dev_eq.isChecked())
        update_gui_settings({"eq_per_device_enabled": val, "eq_per_device": val})

    def _on_rate_changed(self) -> None:
        if self._syncing:
            return
        rate = self.cmb_rate.currentData()
        if rate is not None:
            r_int = int(rate)
            self._rate_hold_until = time.monotonic() + 0.8
            self.ctrl.engine.set_engine_config(r_int, self._cur_period, self._cur_bits)
            QtCore.QTimer.singleShot(250, self.refresh_live)

    def _on_tray_mic_apo_toggled(self) -> None:
        if self._syncing or not getattr(self, "_cur_mic_apo", None):
            return
        cfg = LwasapiMicApoCfg()
        ctypes.memmove(ctypes.byref(cfg), ctypes.byref(self._cur_mic_apo), ctypes.sizeof(LwasapiMicApoCfg))
        cfg.enabled = 1 if self.btn_mic_apo.isChecked() else 0
        self.ctrl.engine.set_mic_apo(cfg)
        QtCore.QTimer.singleShot(80, self.refresh_live)

    def _on_tray_crossfeed_toggled(self) -> None:
        if self._syncing or not self._cur_eq or not getattr(self, "_cur_param_eq", None):
            return
        if not self.ctrl.engine._lib or not hasattr(self.ctrl.engine._lib, "LWASAPI_SetParametricEQ"):
            return
        ext = LwasapiParamEqExt()
        ctypes.memmove(ctypes.byref(ext), ctypes.byref(self._cur_param_eq), ctypes.sizeof(LwasapiParamEqExt))
        ext.crossfeed_enabled = 1 if self.btn_crossfeed.isChecked() else 0
        self.ctrl.engine._lib.LWASAPI_SetParametricEQ(ctypes.byref(self._cur_eq), ctypes.byref(ext))
        QtCore.QTimer.singleShot(80, self.refresh_live)

    def _on_tray_bt_codec_changed(self) -> None:
        if self._syncing or not getattr(self, "_cur_bt_cfg", None):
            return
        cfg = LwasapiBtCodecCfg()
        ctypes.memmove(ctypes.byref(cfg), ctypes.byref(self._cur_bt_cfg), ctypes.sizeof(LwasapiBtCodecCfg))
        cfg.preferred_codec = int(self.cmb_bt_pref_tray.currentData() or 0)
        cfg.wireless_abr_enabled = 1 if self.btn_bt_abr_tray.isChecked() else 0
        self._bt_hold_until = time.monotonic() + 1.0
        self.ctrl.engine.set_bt_codec_cfg(cfg)
        QtCore.QTimer.singleShot(300, self.refresh_live)

    def _on_test_midi(self) -> None:
        if os.path.exists(CTL_BIN):
            try:
                subprocess.Popen(
                    [CTL_BIN, "midi-send", "60", "110"],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
                )
            except OSError:
                pass

    def _on_reset_bt(self) -> None:
        if self._bt_resetting:
            return
        addr = self._bt_addr
        self._bt_resetting = True
        self.btn_reset_bt.setEnabled(False)
        self.btn_reset_bt.setText("Resetting...")

        def worker():
            try:
                target = addr
                if not target or not re.fullmatch(r"([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}", target):
                    r = subprocess.run(["bluetoothctl", "devices", "Connected"], capture_output=True, text=True, timeout=2.5)
                    for ln in r.stdout.splitlines():
                        parts = ln.split()
                        if len(parts) >= 2 and re.fullmatch(r"([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}", parts[1]):
                            target = parts[1]
                            break
                if target:
                    subprocess.run(["bluetoothctl", "disconnect", target], capture_output=True, timeout=4)
                    time.sleep(0.5)
                    subprocess.run(["bluetoothctl", "connect", target], capture_output=True, timeout=6)
            except Exception:
                pass
            finally:
                QtCore.QMetaObject.invokeMethod(self, "_finish_bt_reset", QtCore.Qt.ConnectionType.QueuedConnection)

        threading.Thread(target=worker, daemon=True).start()

    @QtCore.pyqtSlot()
    def _finish_bt_reset(self) -> None:
        self._bt_resetting = False
        self.btn_reset_bt.setEnabled(True)
        self.btn_reset_bt.setText("Reset BT")
        self.refresh_live()

    def _toggle_sess_mute(self, sid: int, cur_muted: int) -> None:
        self.ctrl.engine.set_session_control(sid, -1, 0 if cur_muted else 1)
        QtCore.QTimer.singleShot(50, self.refresh_live)

    def refresh_live(self) -> None:
        st = self.ctrl.engine.get_status()
        settings = load_gui_settings()
        glow = bool(settings.get("tray_green_glow", True))
        active = st is not None
        muted = bool(st.desktop_muted) if st else False
        self.hdr_icon.setPixmap(self.ctrl.icon_bank.qicon(active, muted, glow).pixmap(26, 26))

        self._syncing = True
        try:
            self.btn_per_dev_eq.setChecked(
                bool(settings.get("eq_per_device_enabled", settings.get("eq_per_device", False)))
            )
            if st is None:
                self.lbl_status_pill.setText("STOPPED")
                self.lbl_status_pill.setStyleSheet(
                    "background-color: rgba(245,158,11,36); color: #F59E0B; border: 1px solid #F59E0B;"
                    "border-radius: 8px; padding: 2px 7px; font-size: 10px; font-weight: 700;"
                )
                self.lbl_hw_badge.setText("Engine Offline")
                self.bt_frame.hide()
                self.peak_bar.set_levels(0.0, 0.0, self._theme["accent_primary"])
                self._sync_sessions([])
                return

            self.lbl_status_pill.setText("ACTIVE")
            acc = self._theme["accent_primary"]
            self.lbl_status_pill.setStyleSheet(
                f"background-color: rgba(34,197,94,38); color: {acc}; border: 1px solid {acc};"
                "border-radius: 8px; padding: 2px 7px; font-size: 10px; font-weight: 700;"
            )

            self._cur_rate = int(st.hw_sample_rate)
            self._cur_period = int(st.hw_period_frames)
            self._cur_bits = int(st.hw_format_bits)
            fifo = "FIFO" if st.realtime_priority > 0 else "Norm"
            hw_tag = ""
            if not st.bt_active:
                if st.hw_analog_info.dac_unity_active:
                    hw_tag = " · DAC"
                elif float(st.hw_analog_cfg.analog_boost_db) > 0.05:
                    hw_tag = f" · +{float(st.hw_analog_cfg.analog_boost_db):.1f}dB"
            rate_str = f"{st.hw_sample_rate / 1000.0:g} kHz · {st.hw_format_bits}b · {fifo}{hw_tag}"
            self.lbl_hw_badge.setText(rate_str)
            arch_tip = st.hw_analog_info.card_arch_name.decode("utf-8", "replace").strip("\x00 ")
            diag_tip = st.hw_analog_info.diag_note.decode("utf-8", "replace").strip("\x00 ")
            if arch_tip or diag_tip:
                self.lbl_hw_badge.setToolTip(f"{arch_tip}\n{diag_tip}".strip())

            # Bluetooth pill
            if st.bt_active:
                alias = st.bt_alias.decode("utf-8", "replace").strip("\x00 ") or "Bluetooth"
                codec = st.bt_codec.decode("utf-8", "replace").strip("\x00 ") or "SBC"
                now = time.monotonic()
                if alias != getattr(self, "_last_bt_alias", ""):
                    self._last_bt_alias = alias
                    self._last_bt_poll = 0.0
                if now - self._last_bt_poll > 10.0 or self._bt_pct is None:
                    self._last_bt_poll = now
                    self._bt_pct, self._bt_addr = read_bluez_battery_pct(alias)
                    self.ctrl._tray_bt_pct = self._bt_pct
                    self.ctrl._tray_bt_alias = alias
                    self.ctrl._tray_bt_poll = now
                bat_s = f" · {self._bt_pct}%" if self._bt_pct is not None else ""
                kbps_s = f"@{st.bt_codec_cfg.effective_kbps}k" if st.bt_codec_cfg.effective_kbps else ""
                self.lbl_bt_info.setText(f"{alias} · {codec}{kbps_s}{bat_s} · {st.bt_delay_ms} ms")
                self.bt_frame.show()
            else:
                self._bt_pct = None
                self._last_bt_alias = ""
                self.bt_frame.hide()

            # Master & Mic faders + peaks
            self.fader_master.set_value(st.desktop_volume_pct, bool(st.desktop_muted))
            self.btn_mute_master.setText("Unmute" if st.desktop_muted else "Mute")
            self.fader_mic.set_value(st.desktop_mic_volume_pct, bool(st.desktop_mic_muted))
            self.btn_mute_mic.setText("Unmute" if st.desktop_mic_muted else "Mute")
            self.peak_bar.set_levels(st.output_peak_l, st.output_peak_r, acc)

            # Device combos refreshed every ~2.5s
            now = time.monotonic()
            if now - self._last_dev_poll > 2.5:
                self._last_dev_poll = now
                devs = self.ctrl.engine.list_devices()
                pb_id = st.active_playback_id.decode("utf-8", "replace").strip("\x00 ")
                cap_id = st.active_capture_id.decode("utf-8", "replace").strip("\x00 ")
                self._populate_device_combos(devs, pb_id, cap_id)

            # EQ, Parametric, Mic APO, BT Codec & Rate
            self._cur_eq = st.eq_cfg
            self._cur_param_eq = st.param_eq
            self._cur_mic_apo = st.mic_apo
            self._cur_bt_cfg = st.bt_codec_cfg
            preset_name = st.eq_cfg.preset_name.decode("utf-8", "replace").strip("\x00 ")
            idx_eq = self.cmb_eq.findText(preset_name)
            if idx_eq >= 0:
                self.cmb_eq.setCurrentIndex(idx_eq)
            self.btn_eq_bypass.setChecked(st.eq_cfg.enabled == 0)
            self.btn_limiter.setChecked(st.eq_cfg.limiter_enabled != 0)
            self.btn_mic_apo.setChecked(st.mic_apo.enabled != 0)
            self.btn_crossfeed.setChecked(st.param_eq.crossfeed_enabled != 0)
            sup_mask = int(st.bt_codec_cfg.supported_codecs_mask)
            now_mono = time.monotonic()
            if not self.cmb_bt_pref_tray.view().isVisible():
                for idx_i, (base_lbl, code_c) in enumerate(
                    (("BT: Auto", 0), ("LDAC", 1), ("aptX HD", 2), ("aptX", 3), ("AAC", 4), ("SBC", 5))
                ):
                    lbl_i = (
                        f"{base_lbl} (N/A)"
                        if (code_c > 0 and sup_mask != 0 and not (sup_mask & (1 << (code_c - 1))))
                        else base_lbl
                    )
                    if idx_i < self.cmb_bt_pref_tray.count() and self.cmb_bt_pref_tray.itemText(idx_i) != lbl_i:
                        self.cmb_bt_pref_tray.setItemText(idx_i, lbl_i)
                if now_mono >= self._bt_hold_until:
                    idx_bt = self.cmb_bt_pref_tray.findData(int(st.bt_codec_cfg.preferred_codec))
                    if idx_bt >= 0:
                        self.cmb_bt_pref_tray.setCurrentIndex(idx_bt)
            self.btn_bt_abr_tray.setChecked(st.bt_codec_cfg.wireless_abr_enabled != 0)
            pb_id_now = st.active_playback_id.decode("utf-8", "replace").strip("\x00 ")
            if pb_id_now != getattr(self, "_last_cap_pb_id", None):
                self._last_cap_pb_id = pb_id_now
                sup_rates = None
                m_hw = re.match(r"^hw:(\d+),(\d+)$", pb_id_now)
                if m_hw and os.path.exists(f"/proc/asound/card{m_hw.group(1)}/stream0"):
                    parsed = set()
                    in_pb = False
                    try:
                        with open(f"/proc/asound/card{m_hw.group(1)}/stream0", "r", encoding="utf-8", errors="ignore") as fp:
                            for raw in fp:
                                ln = raw.strip()
                                if ln.startswith("Playback:"):
                                    in_pb = True
                                elif ln.startswith("Capture:"):
                                    in_pb = False
                                elif in_pb and ln.startswith("Rates:"):
                                    if "continuous" in ln.lower() or "-" in ln:
                                        parsed = set()
                                        break
                                    for tok in re.findall(r"\d+", ln):
                                        v = int(tok)
                                        if 8000 <= v <= 768000:
                                            parsed.add(v)
                    except OSError:
                        parsed = set()
                    sup_rates = parsed or None
                if not self.cmb_rate.view().isVisible():
                    for idx_r_i in range(self.cmb_rate.count()):
                        r_val = int(self.cmb_rate.itemData(idx_r_i) or 0)
                        lbl_r = f"{r_val/1000:g} kHz (N/A)" if (sup_rates and r_val not in sup_rates) else f"{r_val/1000:g} kHz"
                        if self.cmb_rate.itemText(idx_r_i) != lbl_r:
                            self.cmb_rate.setItemText(idx_r_i, lbl_r)
            want_rate_d = int(st.hw_sample_rate)
            if now_mono >= self._rate_hold_until and not self.cmb_rate.view().isVisible():
                idx_r = self.cmb_rate.findData(want_rate_d)
                if idx_r >= 0:
                    self.cmb_rate.setCurrentIndex(idx_r)

            n_sess = min(st.num_sessions, LWASAPI_MAX_SESSIONS)
            sessions = [st.sessions[i] for i in range(n_sess)]
            self._sync_sessions(sessions)
        finally:
            self._syncing = False
            self._keep_bottom_anchored()

    def _keep_bottom_anchored(self) -> None:
        if not self.isVisible() or not getattr(self, "_anchored_above_panel", False):
            return
        for lay in reversed(self.findChildren(QtWidgets.QLayout)):
            lay.invalidate()
        self.card.updateGeometry()
        self.viewport.updateGeometry()
        if self.layout():
            self.layout().activate()
        sh = self.sizeHint()
        new_w = min(self.maximumWidth(), max(self.minimumWidth(), sh.width()))
        new_h = max(self.minimumHeight(), sh.height())
        if abs(new_h - self.height()) >= 2 or new_w != self.width():
            self.resize(new_w, new_h)
            if getattr(self, "_anchor_panel_loc", 4) != 3:
                avail = getattr(self, "_anchor_avail", None)
                top_limit = (avail.top() + 8) if avail is not None else 8
                new_y = max(top_limit, self._anchor_bottom - new_h)
                self.move(self.x(), new_y)

    def _populate_device_combos(self, devs: list, pb_id: str, cap_id: str) -> None:
        self.cmb_out_dev.clear()
        self.cmb_mic_dev.clear()
        pb_idx = 0
        cap_idx = 0
        p_i = 0
        c_i = 0
        for d in devs:
            did = d.id.decode("utf-8", "replace").strip("\x00 ")
            dname = d.name.decode("utf-8", "replace").strip("\x00 ") or did
            if d.is_capture:
                self.cmb_mic_dev.addItem(dname, did)
                if did == cap_id or d.is_active:
                    cap_idx = c_i
                c_i += 1
            else:
                self.cmb_out_dev.addItem(dname, did)
                if did == pb_id or d.is_active:
                    pb_idx = p_i
                p_i += 1
        if self.cmb_out_dev.count() > 0:
            self.cmb_out_dev.setCurrentIndex(pb_idx)
        if self.cmb_mic_dev.count() > 0:
            self.cmb_mic_dev.setCurrentIndex(cap_idx)

    def _sync_sessions(self, sessions: list) -> None:
        self.lbl_app_count.setText(f"{len(sessions)} active")
        self.btn_tab_apps.setText(f"Applications ({len(sessions)})" if sessions else "Applications")
        self.lbl_no_apps.setVisible(len(sessions) == 0)
        wanted_ids = [int(s.session_id) for s in sessions]

        for sid in list(self._app_rows.keys()):
            if sid not in wanted_ids:
                row_w, _, _, _ = self._app_rows.pop(sid)
                self.apps_layout.removeWidget(row_w)
                row_w.deleteLater()

        acc = self._theme["accent_primary"]
        mic_acc = self._theme["accent_mic"]
        bg_ctrl = self._theme["bg_control"]
        border_ctrl = self._theme["border_control"]
        bright = self._theme["text_bright"]
        muted = self._theme["text_muted"]

        insert_pos = 0
        for s in sessions:
            sid = int(s.session_id)
            raw_name = s.client_name.decode("utf-8", "replace").strip("\x00 ") or f"App #{s.client_pid}"
            label = f"[MIC] {raw_name}" if s.is_capture else raw_name
            pct = max(0, min(150, int(round(s.volume_milli / 10.0))))
            is_muted = bool(s.muted)
            if sid not in self._app_rows:
                row_w = QtWidgets.QWidget()
                rlay = QtWidgets.QHBoxLayout(row_w)
                rlay.setContentsMargins(0, 0, 0, 0)
                rlay.setSpacing(6)
                icon_lbl = QtWidgets.QLabel()
                icon_lbl.setFixedSize(22, 22)
                icon_lbl.setAlignment(QtCore.Qt.AlignmentFlag.AlignCenter)
                fader = SwipeFader(label, accent=(mic_acc if s.is_capture else acc), compact=True)
                fader.set_theme_colors((mic_acc if s.is_capture else acc), bg_ctrl, border_ctrl, bright, muted)
                fader.value_changed.connect(lambda v, _sid=sid: self._on_sess_drag(_sid, v))
                fader.value_committed.connect(lambda v, _sid=sid: self._on_sess_commit(_sid, v))
                btn_m = QtWidgets.QPushButton("Mute")
                btn_m.setObjectName("MuteBtn")
                btn_m.setFixedSize(46, 28)
                rlay.addWidget(icon_lbl)
                rlay.addWidget(fader, 1)
                rlay.addWidget(btn_m)
                self.apps_layout.insertWidget(insert_pos, row_w)
                self._app_rows[sid] = (row_w, icon_lbl, fader, btn_m)
            row_w, icon_lbl, fader, btn_m = self._app_rows[sid]
            icon_lbl.setPixmap(self._icon_resolver.get_pixmap(raw_name, int(s.client_pid), bool(s.is_capture), 20))
            fader.set_label(label)
            fader.set_value(pct, is_muted)
            try:
                btn_m.clicked.disconnect()
            except TypeError:
                pass
            btn_m.clicked.connect(lambda _chk=False, _sid=sid, _cm=int(s.muted): self._toggle_sess_mute(_sid, _cm))
            btn_m.setStyleSheet(
                f"color: #EF4444; border-color: #EF4444; font-weight: 700;" if is_muted else ""
            )
            insert_pos += 1


class AudioEngineTray(QtCore.QObject):
    """Main Plasma 6 tray controller managing SNI D-Bus, QSystemTrayIcon fallback, OSD, and Quick Mixer."""

    def __init__(self, app: QtWidgets.QApplication):
        super().__init__(app)
        self.app = app
        self.engine = SalsaEngineClient()
        self.icon_bank = TrayIconBank()
        self.osd = SalsaOsdOverlay()
        self.popover = SalsaQuickMixerPopup(self)
        self._last_tray_pt = QtCore.QPoint(0, 0)
        self._dev_name_cache: dict[str, tuple[float, str]] = {}
        self._last_osd_out: int | None = None
        self._last_osd_out_mute: bool | None = None
        self._last_osd_mic: int | None = None
        self._last_osd_mic_mute: bool | None = None
        self._osd_suppress_until: float = 0.0
        self.menu = QtWidgets.QMenu()
        self._build_menu()

        self.sni_host = QtCore.QObject(self)
        self.sni = StatusNotifierItemAdaptor(self.sni_host, self.icon_bank.theme_dir)
        self.sni.activated.connect(self._on_left_click)
        self.sni.secondary_activated.connect(self.toggle_master_mute)
        self.sni.context_menu_requested.connect(self._show_context_menu)
        self.sni.scrolled.connect(self._on_sni_scroll)

        self.qtray = None
        self._sni_active = False
        self._setup_sni_or_qtray()

        self._poll_timer = QtCore.QTimer(self)
        self._poll_timer.setInterval(2000)
        self._poll_timer.timeout.connect(self.sync_now)
        self._poll_timer.start()

        self._osd_watch_timer = QtCore.QTimer(self)
        self._osd_watch_timer.setInterval(300)
        self._osd_watch_timer.timeout.connect(self._poll_external_volume)
        self._osd_watch_timer.start()
        self._poll_external_volume()
        self.sync_now()

    def _setup_sni_or_qtray(self) -> None:
        bus = QtDBus.QDBusConnection.sessionBus()
        watcher_present = False
        if bus.isConnected():
            iface = bus.interface()
            if iface and iface.isServiceRegistered("org.kde.StatusNotifierWatcher").value():
                svc_name = f"org.kde.StatusNotifierItem-{os.getpid()}-1"
                if bus.registerService(svc_name) and bus.registerObject(
                    "/StatusNotifierItem",
                    self.sni_host,
                    QtDBus.QDBusConnection.RegisterOption.ExportAdaptors,
                ):
                    msg = QtDBus.QDBusMessage.createMethodCall(
                        "org.kde.StatusNotifierWatcher",
                        "/StatusNotifierWatcher",
                        "org.kde.StatusNotifierWatcher",
                        "RegisterStatusNotifierItem",
                    )
                    msg.setArguments([svc_name])
                    bus.asyncCall(msg)
                    watcher_present = True
        self._sni_active = watcher_present
        if not watcher_present:
            self.qtray = QtWidgets.QSystemTrayIcon(self.app)
            self.qtray.setContextMenu(self.menu)
            self.qtray.activated.connect(self._on_qtray_activated)
            self.qtray.show()

    def _build_menu(self) -> None:
        self.act_status = self.menu.addAction("Salsa — Checking...")
        self.act_status.setEnabled(False)
        self.menu.addSeparator()

        self.act_popover = self.menu.addAction("Open Quick Mixer")
        self.act_popover.triggered.connect(lambda: self._open_popover_at_cursor())
        self.act_studio = self.menu.addAction("Open Salsa Studio")
        self.act_studio.triggered.connect(self.launch_studio)
        self.act_graph = self.menu.addAction("Signal Flow Graph")
        self.act_graph.triggered.connect(self.launch_graph)
        self.act_debug = self.menu.addAction("Live Debugger")
        self.act_debug.triggered.connect(self.launch_debugger)

        self.menu.addSeparator()
        self.act_mute_master = self.menu.addAction("Toggle Master Mute")
        self.act_mute_master.triggered.connect(self.toggle_master_mute)
        self.act_mute_mic = self.menu.addAction("Toggle Mic Mute")
        self.act_mute_mic.triggered.connect(self.toggle_mic_mute)

        self.menu.addSeparator()
        self.act_glow = self.menu.addAction("Green Glow on Active Icon")
        self.act_glow.setCheckable(True)
        self.act_glow.triggered.connect(self._on_toggle_glow)

        self.act_left_pop = self.menu.addAction("Left-Click Opens Quick Mixer")
        self.act_left_pop.setCheckable(True)
        self.act_left_pop.triggered.connect(self._on_toggle_left_pop)

        self.theme_menu = self.menu.addMenu("Quick Mixer Theme")
        self._theme_action_group = QtGui.QActionGroup(self)
        self._theme_action_group.setExclusive(True)
        self._theme_actions = []
        theme_choices = ["Follow Studio Theme", *THEMES.keys()]
        if "Custom Studio Theme" not in THEMES:
            theme_choices.append("Custom Studio Theme")
        for t_name in theme_choices:
            act = self.theme_menu.addAction(t_name)
            act.setCheckable(True)
            act.setData(t_name)
            act.triggered.connect(lambda _checked, tn=t_name: self._on_select_tray_theme(tn))
            self._theme_action_group.addAction(act)
            self._theme_actions.append(act)

        self.menu.addSeparator()
        self.act_start = self.menu.addAction("Start Engine")
        self.act_start.triggered.connect(lambda: self._ctl_systemd("start"))
        self.act_restart = self.menu.addAction("Restart Engine")
        self.act_restart.triggered.connect(self.restart_engine)
        self.act_stop = self.menu.addAction("Stop Engine")
        self.act_stop.triggered.connect(lambda: self._ctl_systemd("stop"))
        self.menu.addSeparator()
        self.act_quit = self.menu.addAction("Quit Tray")
        self.act_quit.triggered.connect(self.app.quit)

    def _on_qtray_activated(self, reason) -> None:
        pos = QtGui.QCursor.pos()
        if reason == QtWidgets.QSystemTrayIcon.ActivationReason.Trigger:
            self._on_left_click(pos.x(), pos.y())
        elif reason == QtWidgets.QSystemTrayIcon.ActivationReason.MiddleClick:
            self.toggle_master_mute()

    def _open_popover_at_cursor(self) -> None:
        self.popover.show_near(self._last_tray_pt.x(), self._last_tray_pt.y())

    def _on_left_click(self, x: int = 0, y: int = 0) -> None:
        self._last_tray_pt = QtCore.QPoint(x, y) if (x > 0 or y > 0) else QtCore.QPoint(0, 0)
        settings = load_gui_settings()
        if bool(settings.get("tray_left_click_popover", True)):
            if self.popover.isVisible():
                self.popover.hide()
            else:
                self.popover.show_near(x, y)
        else:
            self.launch_studio()

    def _show_context_menu(self, x: int, y: int) -> None:
        self.sync_now()
        if x > 0 or y > 0:
            self._last_tray_pt = QtCore.QPoint(x, y)
            pt = _wayland_to_qt_point(x, y)
        else:
            self._last_tray_pt = QtCore.QPoint(0, 0)
            screens = QtWidgets.QApplication.screens()
            pref_idx = None
            active_out = _active_kwin_output_name()
            if active_out:
                for i, s in enumerate(screens):
                    if s.name() == active_out:
                        pref_idx = i
                        break
            panel_screen_idx, panel_loc, clearance, right_pad = _plasma_tray_panel_info(pref_idx)
            screen = screens[panel_screen_idx] if 0 <= panel_screen_idx < len(screens) else QtWidgets.QApplication.primaryScreen()
            if screen:
                avail = screen.availableGeometry()
                if avail.width() < 200 or avail.height() < 200:
                    avail = screen.geometry()
                anchor_x = max(avail.left() + 8, avail.right() - right_pad - 120)
                if panel_loc == 3:
                    pt = QtCore.QPoint(anchor_x, avail.top() + clearance)
                else:
                    pt = QtCore.QPoint(anchor_x, max(avail.top() + 8, avail.bottom() - clearance))
            else:
                pt = QtGui.QCursor.pos()
        self.menu.popup(pt)

    def _active_device_label(self, is_mic: bool = False, st=None) -> str:
        fallback = "Mic Input" if is_mic else "Master Output"
        try:
            st = st or self.engine.get_status()
            target_id = ""
            if st is not None:
                raw_id = st.active_capture_id if is_mic else st.active_playback_id
                target_id = raw_id.decode("utf-8", "replace").strip("\x00 ")
            cache_key = f"{'mic' if is_mic else 'out'}:{target_id}"
            now = time.monotonic()
            cached = self._dev_name_cache.get(cache_key)
            if cached and (now - cached[0]) < 4.0:
                return cached[1]
            chosen = ""
            for d in self.engine.list_devices():
                if bool(d.is_capture) != bool(is_mic):
                    continue
                did = d.id.decode("utf-8", "replace").strip("\x00 ")
                dname = d.name.decode("utf-8", "replace").strip("\x00 ")
                if target_id and did == target_id and dname:
                    chosen = dname
                    break
                if d.is_active and dname and not chosen:
                    chosen = dname
            label = chosen or fallback
            self._dev_name_cache[cache_key] = (now, label)
            return label
        except Exception:
            return fallback

    @QtCore.pyqtSlot()
    def _poll_external_volume(self) -> None:
        st = self.engine.get_status()
        if st is None:
            self._last_osd_out = None
            self._last_osd_out_mute = None
            self._last_osd_mic = None
            self._last_osd_mic_mute = None
            return
        out_v = int(st.desktop_volume_pct)
        out_m = bool(st.desktop_muted)
        mic_v = int(st.desktop_mic_volume_pct)
        mic_m = bool(st.desktop_mic_muted)

        if self._last_osd_out is None:
            self._last_osd_out = out_v
            self._last_osd_out_mute = out_m
            self._last_osd_mic = mic_v
            self._last_osd_mic_mute = mic_m
            return

        suppress = self.popover.isVisible() or (time.monotonic() < self._osd_suppress_until)
        out_changed = (out_v != self._last_osd_out) or (out_m != self._last_osd_out_mute)
        mic_changed = (mic_v != self._last_osd_mic) or (mic_m != self._last_osd_mic_mute)
        self._last_osd_out = out_v
        self._last_osd_out_mute = out_m
        self._last_osd_mic = mic_v
        self._last_osd_mic_mute = mic_m

        if suppress or (not out_changed and not mic_changed):
            return

        t = resolve_tray_theme(load_gui_settings())
        if out_changed:
            self.osd.show_osd(self._active_device_label(False, st), out_v, out_m, is_mic=False, accent=t["accent_primary"])
        elif mic_changed:
            self.osd.show_osd(self._active_device_label(True, st), mic_v, mic_m, is_mic=True, accent=t["accent_secondary"])
        self.sync_now()

    def _on_sni_scroll(self, delta: int, orientation: str) -> None:
        if delta == 0:
            return
        st = self.engine.get_status()
        if st is None:
            return
        settings = load_gui_settings()
        t = resolve_tray_theme(settings)
        notches = max(1, round(abs(delta) / 120.0))
        step = (3 * notches) if delta > 0 else (-3 * notches)

        shift_held = bool(QtWidgets.QApplication.keyboardModifiers() & QtCore.Qt.KeyboardModifier.ShiftModifier)
        is_horizontal = str(orientation).lower().startswith("h")
        self._osd_suppress_until = time.monotonic() + 0.55
        if is_horizontal or shift_held:
            new_mic = max(0, min(150, int(st.desktop_mic_volume_pct) + step))
            self.engine.set_mic_volume(new_mic, -1)
            self._last_osd_mic = new_mic
            self._last_osd_mic_mute = bool(st.desktop_mic_muted)
            self.osd.show_osd(self._active_device_label(True, st), new_mic, bool(st.desktop_mic_muted), is_mic=True, accent=t["accent_secondary"])
        else:
            new_vol = max(0, min(150, int(st.desktop_volume_pct) + step))
            self.engine.set_master_volume(new_vol, -1)
            self._last_osd_out = new_vol
            self._last_osd_out_mute = bool(st.desktop_muted)
            self.osd.show_osd(self._active_device_label(False, st), new_vol, bool(st.desktop_muted), is_mic=False, accent=t["accent_primary"])
        self.sync_now()
        if self.popover.isVisible():
            self.popover.refresh_live()

    def toggle_master_mute(self) -> None:
        st = self.engine.get_status()
        if st is None:
            return
        new_mute = 0 if st.desktop_muted else 1
        self.engine.set_master_volume(-1, new_mute)
        settings = load_gui_settings()
        t = resolve_tray_theme(settings)
        self._osd_suppress_until = time.monotonic() + 0.55
        self._last_osd_out = int(st.desktop_volume_pct)
        self._last_osd_out_mute = bool(new_mute)
        self.osd.show_osd(self._active_device_label(False, st), int(st.desktop_volume_pct), bool(new_mute), is_mic=False, accent=t["accent_primary"])
        self.sync_now()
        if self.popover.isVisible():
            self.popover.refresh_live()

    def toggle_mic_mute(self) -> None:
        st = self.engine.get_status()
        if st is None:
            return
        new_mute = 0 if st.desktop_mic_muted else 1
        self.engine.set_mic_volume(-1, new_mute)
        settings = load_gui_settings()
        t = resolve_tray_theme(settings)
        self._osd_suppress_until = time.monotonic() + 0.55
        self._last_osd_mic = int(st.desktop_mic_volume_pct)
        self._last_osd_mic_mute = bool(new_mute)
        self.osd.show_osd(self._active_device_label(True, st), int(st.desktop_mic_volume_pct), bool(new_mute), is_mic=True, accent=t["accent_secondary"])
        self.sync_now()
        if self.popover.isVisible():
            self.popover.refresh_live()

    def _on_toggle_glow(self, checked: bool) -> None:
        update_gui_settings({"tray_green_glow": bool(checked)})
        self.sync_now()
        if self.popover.isVisible():
            self.popover.refresh_live()

    def _on_toggle_left_pop(self, checked: bool) -> None:
        update_gui_settings({"tray_left_click_popover": bool(checked)})

    def _on_select_tray_theme(self, name: str) -> None:
        update_gui_settings({
            "tray_theme": name,
            "tray_breeze_emerald": (name == "Plasma Breeze Emerald"),
        })
        self.popover.apply_theme()
        if self.popover.isVisible():
            self.popover.refresh_live()

    @QtCore.pyqtSlot()
    def sync_now(self) -> None:
        settings = load_gui_settings()
        glow = bool(settings.get("tray_green_glow", True))
        left_pop = bool(settings.get("tray_left_click_popover", True))
        wanted_theme = resolve_tray_theme_name(settings)
        self.act_glow.setChecked(glow)
        self.act_left_pop.setChecked(left_pop)
        for act in self._theme_actions:
            act.setChecked(act.data() == wanted_theme)

        st = self.engine.get_status()
        active = st is not None
        muted = bool(st.desktop_muted) if st else False
        icon_name = self.icon_bank.icon_name(active, muted, glow)

        if st:
            vol_s = "MUTED" if st.desktop_muted else f"{st.desktop_volume_pct}%"
            mic_s = "MUTED" if st.desktop_mic_muted else f"{st.desktop_mic_volume_pct}%"
            rate_s = f"{st.hw_sample_rate / 1000.0:g} kHz/{st.hw_format_bits}-bit"
            bt_s = ""
            if st.bt_active:
                bt_alias = st.bt_alias.decode("utf-8", "replace").strip("\x00 ") or "Bluetooth"
                bt_codec = st.bt_codec.decode("utf-8", "replace").strip("\x00 ") or "SBC"
                now = time.monotonic()
                if bt_alias != getattr(self, "_tray_bt_alias", ""):
                    self._tray_bt_alias = bt_alias
                    self._tray_bt_poll = 0.0
                if now - getattr(self, "_tray_bt_poll", 0.0) > 10.0 or getattr(self, "_tray_bt_pct", None) is None:
                    self._tray_bt_poll = now
                    self._tray_bt_pct, _ = read_bluez_battery_pct(bt_alias)
                bat_p = getattr(self, "_tray_bt_pct", None)
                bat_frag = f" · {bat_p}%" if bat_p is not None else ""
                bt_s = f" · {bt_alias} ({bt_codec}{bat_frag})"
            else:
                self._tray_bt_pct = None
                self._tray_bt_alias = ""
            title = f"Salsa — Out {vol_s} · Mic {mic_s}{bt_s} · {rate_s}"
            self.act_status.setText(f"Active · Out {vol_s} · Mic {mic_s}{bt_s} · {rate_s}")
            self.act_mute_master.setText("Unmute Master Output" if st.desktop_muted else "Mute Master Output")
            self.act_mute_mic.setText("Unmute Mic Input" if st.desktop_mic_muted else "Mute Mic Input")
            self.act_start.setEnabled(False)
            self.act_stop.setEnabled(True)
            self.act_restart.setEnabled(True)
        else:
            title = "Salsa — Engine Stopped"
            self.act_status.setText("Salsa — Engine Stopped")
            self.act_start.setEnabled(True)
            self.act_stop.setEnabled(False)
            self.act_restart.setEnabled(True)

        self.sni.update_state(icon_name, title, "Active")
        if self.qtray is not None:
            self.qtray.setIcon(self.icon_bank.qicon(active, muted, glow))
            self.qtray.setToolTip(title)

    def _ctl_systemd(self, action: str) -> None:
        def worker():
            try:
                subprocess.run(["systemctl", "--user", action, "lwasapi-audiodg.service"], check=False, timeout=8)
            except Exception:
                pass
            QtCore.QMetaObject.invokeMethod(self, "sync_now", QtCore.Qt.ConnectionType.QueuedConnection)

        threading.Thread(target=worker, daemon=True).start()

    def restart_engine(self) -> None:
        self._ctl_systemd("restart")

    def _launch_script(self, path: str, *args: str) -> None:
        if os.path.exists(path):
            try:
                subprocess.Popen([sys.executable, path, *args], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            except OSError:
                pass

    def launch_studio(self) -> None:
        self._launch_script(SALSA_GUI)

    def launch_graph(self) -> None:
        self._launch_script(SALSA_GUI, "--graph")

    def launch_debugger(self) -> None:
        self._launch_script(SALSA_DEBUG)


def main() -> int:
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    if not os.environ.get("QT_QPA_PLATFORM") and os.environ.get("DISPLAY"):
        os.environ["QT_QPA_PLATFORM"] = "xcb"
    app = QtWidgets.QApplication(sys.argv)
    app.setApplicationName("Salsa Tray")
    app.setDesktopFileName("lwasapi-tray")
    app.setQuitOnLastWindowClosed(False)
    _tray = AudioEngineTray(app)
    app.setWindowIcon(_tray.icon_bank.qicon(True, False, True))
    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
