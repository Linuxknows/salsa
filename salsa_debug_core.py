#!/usr/bin/env python3
"""Core telemetry client, OS audio routing auditor, external log tailers,
interactive probes, and bundle exporter for SALSA (LWASAPI) Live Debugger.
"""

from __future__ import annotations

import collections
import ctypes
import glob
import hashlib
import json
import math
import mmap
import os
import re
import select
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
from typing import Any, Callable, Optional


def resolve_private_runtime_dir() -> str:
    uid = os.getuid()
    candidates: list[str] = []
    xdg = os.environ.get("XDG_RUNTIME_DIR", "")
    if xdg.startswith("/"):
        candidates.append(xdg)
    candidates.append(f"/run/user/{uid}")
    for base in candidates:
        try:
            st = os.lstat(base)
            if os.path.isdir(base) and not os.path.islink(base) and st.st_uid == uid and (st.st_mode & 0o077) == 0:
                priv = os.path.join(base, "lwasapi")
                os.makedirs(priv, mode=0o700, exist_ok=True)
                pst = os.lstat(priv)
                if os.path.isdir(priv) and not os.path.islink(priv) and pst.st_uid == uid:
                    if (pst.st_mode & 0o777) != 0o700:
                        os.chmod(priv, 0o700)
                    return priv
        except OSError:
            continue
    fb = f"/tmp/lwasapi-{uid}"
    os.makedirs(fb, mode=0o700, exist_ok=True)
    os.chmod(fb, 0o700)
    return fb


BASE_DIR = os.path.dirname(os.path.abspath(__file__))
PRIVATE_RUNTIME_DIR = resolve_private_runtime_dir()
DEBUG_SOCK = os.environ.get("LWASAPI_DEBUG_SOCK") or os.path.join(PRIVATE_RUNTIME_DIR, "debug.sock")
DEBUG_IN_SOCK = os.environ.get("LWASAPI_DEBUG_IN_SOCK") or os.path.join(PRIVATE_RUNTIME_DIR, "debug-in.sock")
IPC_SOCK = os.environ.get("LWASAPI_SOCKET") or os.path.join(PRIVATE_RUNTIME_DIR, "audiodg.sock")

BUILD_DIR = os.path.join(BASE_DIR, "build")
SHIMS_DIR = os.path.join(BUILD_DIR, "shims")
LIB_LWASAPI_SO = os.path.join(BUILD_DIR, "liblwasapi.so")
CTL_BIN = (
    os.path.join(BUILD_DIR, "lwasapi-ctl")
    if os.path.exists(os.path.join(BUILD_DIR, "lwasapi-ctl"))
    else "lwasapi-ctl"
)
STATE_DIR = os.path.expanduser("~/.config/lwasapi")
AUTOSYNC_LOG = os.path.join(STATE_DIR, "autosync.log")
GUI_SETTINGS_FILE = os.path.join(STATE_DIR, "gui_settings.json")
DEFAULT_BUNDLE_DIR = os.path.expanduser("~/.cache/lwasapi_debug")

PW_PREVIEW_MAGIC = 0x534C5057
PW_PREVIEW_VERSION = 2
PW_PREVIEW_MAX_W = 3840
PW_PREVIEW_MAX_H = 2160
PW_PREVIEW_MAX_BYTES = PW_PREVIEW_MAX_W * PW_PREVIEW_MAX_H * 4
PW_PREVIEW_PIXELS_OFF = 520
PW_PREVIEW_TOTAL_SIZE = PW_PREVIEW_PIXELS_OFF + PW_PREVIEW_MAX_BYTES
PW_PREVIEW_HDR_STRUCT = struct.Struct("<IIQQ15I4xQQ128s32s5I")

SUBSYSTEMS = ("ENGINE", "WASAPI", "PA", "BLUEZ", "MIDI", "PW_VIDEO", "SHIM", "ALSA", "SYSTEM")
LEVELS = ("TRACE", "INFO", "WARN", "ERR")
LEVEL_RANK = {lvl: idx for idx, lvl in enumerate(LEVELS)}

UNIT_LIST = (
    ("user", "lwasapi-audiodg.service"),
    ("user", "lwasapi-tray.service"),
    ("user", "lwasapi-module-watcher.path"),
    ("user", "lwasapi-module-watcher.service"),
    ("user", "wireplumber.service"),
    ("user", "pipewire.service"),
    ("user", "pipewire.socket"),
    ("user", "pipewire-pulse.service"),
    ("user", "pipewire-pulse.socket"),
    ("system", "bluetooth.service"),
)


def _runtime_dir() -> str:
    return os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"


def _read_text(path: str, max_bytes: int = 131072) -> str:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read(max_bytes)
    except OSError:
        return ""


def _read_cmdline(pid: int) -> str:
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            raw = f.read(4096)
        parts = [p.decode("utf-8", errors="replace") for p in raw.split(b"\x00") if p]
        return " ".join(parts)
    except OSError:
        return ""


def _sha256_file(path: str) -> str:
    try:
        h = hashlib.sha256()
        with open(path, "rb") as f:
            while True:
                chunk = f.read(65536)
                if not chunk:
                    break
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return ""


def _peercred_for_unix_socket(path: str) -> dict[str, Any]:
    """Connect briefly to a stream Unix domain socket and query kernel SO_PEERCRED."""
    info: dict[str, Any] = {
        "path": path,
        "exists": os.path.exists(path),
        "is_socket": False,
        "connectable": False,
        "peer_pid": None,
        "peer_uid": None,
        "peer_comm": "",
        "error": "",
    }
    if not info["exists"]:
        return info
    try:
        st = os.stat(path)
        import stat as stat_mod

        info["is_socket"] = stat_mod.S_ISSOCK(st.st_mode)
        info["mode_oct"] = oct(stat_mod.S_IMODE(st.st_mode))
    except OSError as exc:
        info["error"] = str(exc)
        return info

    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(0.4)
    try:
        sock.connect(path)
        info["connectable"] = True
        ucred = sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
        pid, uid, gid = struct.unpack("3i", ucred)
        info["peer_pid"] = pid
        info["peer_uid"] = uid
        info["peer_gid"] = gid
        if pid > 0:
            info["peer_comm"] = _read_text(f"/proc/{pid}/comm", 256).strip()
    except OSError as exc:
        info["error"] = str(exc)
    finally:
        try:
            sock.close()
        except OSError:
            pass
    return info


def load_saved_theme_name() -> str:
    try:
        with open(GUI_SETTINGS_FILE, "r", encoding="utf-8") as f:
            data = json.load(f)
        if isinstance(data, dict) and isinstance(data.get("theme"), str):
            return data["theme"]
    except (OSError, ValueError):
        pass
    return "Salsa Midnight (Default)"


DEFAULT_SCREENCAST_PREFS: dict[str, Any] = {
    "res_key": "Source",
    "fps_val": 60,
    "cap_real_cast": False,
}
_VALID_SC_RES_KEYS = {"Source", "2160p", "1440p", "1080p", "720p", "480p"}
_VALID_SC_FPS_VALS = {0, 15, 30, 60}


def load_screencast_prefs() -> dict[str, Any]:
    out = dict(DEFAULT_SCREENCAST_PREFS)
    try:
        with open(GUI_SETTINGS_FILE, "r", encoding="utf-8") as f:
            data = json.load(f)
        sc = data.get("screencast_profile") if isinstance(data, dict) else None
        if isinstance(sc, dict):
            rk = sc.get("res_key")
            if isinstance(rk, str) and rk in _VALID_SC_RES_KEYS:
                out["res_key"] = rk
            fv = sc.get("fps_val")
            if isinstance(fv, int) and not isinstance(fv, bool) and fv in _VALID_SC_FPS_VALS:
                out["fps_val"] = fv
            if isinstance(sc.get("cap_real_cast"), bool):
                out["cap_real_cast"] = sc["cap_real_cast"]
    except (OSError, ValueError):
        pass
    return out


def save_screencast_prefs(prefs: dict[str, Any]) -> None:
    try:
        os.makedirs(STATE_DIR, mode=0o700, exist_ok=True)
        existing: dict[str, Any] = {}
        try:
            with open(GUI_SETTINGS_FILE, "r", encoding="utf-8") as f:
                loaded = json.load(f)
            if isinstance(loaded, dict):
                existing = loaded
        except (OSError, ValueError):
            existing = {}
        clean = {
            "res_key": prefs.get("res_key", "Source") if prefs.get("res_key") in _VALID_SC_RES_KEYS else "Source",
            "fps_val": int(prefs.get("fps_val", 60)) if prefs.get("fps_val") in _VALID_SC_FPS_VALS else 60,
            "cap_real_cast": bool(prefs.get("cap_real_cast", False)),
        }
        existing["screencast_profile"] = clean
        tmp = f"{GUI_SETTINGS_FILE}.tmp.{os.getpid()}"
        try:
            os.unlink(tmp)
        except OSError:
            pass
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(existing, f, indent=2)
            f.write("\n")
        os.chmod(tmp, 0o600)
        os.replace(tmp, GUI_SETTINGS_FILE)
    except OSError:
        pass


def format_ts_ms(ts_ms: int | float) -> str:
    if not ts_ms:
        return "--:--:--.---"
    sec = ts_ms / 1000.0
    lt = time.localtime(sec)
    ms = int(ts_ms) % 1000
    return f"{lt.tm_hour:02d}:{lt.tm_min:02d}:{lt.tm_sec:02d}.{ms:03d}"


class SalsaDebugClient:
    """Streaming client for lwasapi-audiodg's live debug NDJSON Unix socket."""

    def __init__(self, sock_path: str = DEBUG_SOCK, max_events: int = 4000) -> None:
        self.sock_path = sock_path
        self.max_events = max_events
        self._lock = threading.Lock()
        self._events: collections.deque[dict[str, Any]] = collections.deque(maxlen=max_events)
        self._next_local_seq = 1
        self._telem_seq = 0
        self._hello: dict[str, Any] = {}
        self._telemetry: dict[str, Any] = {}
        self._connected = False
        self._last_error = ""
        self._stop_ev = threading.Event()
        self._thread: Optional[threading.Thread] = None
        self._sock: Optional[socket.socket] = None
        self._send_lock = threading.Lock()
        self._snap_waiters: list[threading.Event] = []
        self._pong_waiters: dict[str, tuple[threading.Event, dict[str, Any]]] = {}
        self._cmd_waiters: list[tuple[threading.Event, dict[str, Any]]] = []
        self._last_cmd_reply: dict[str, Any] = {}

    def start(self) -> None:
        if self._thread and self._thread.is_alive():
            return
        self._stop_ev.clear()
        self._thread = threading.Thread(target=self._run_loop, name="SalsaDebugRX", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop_ev.set()
        with self._send_lock:
            if self._sock is not None:
                try:
                    self._sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                try:
                    self._sock.close()
                except OSError:
                    pass
                self._sock = None
        if self._thread and self._thread.is_alive():
            self._thread.join(timeout=1.5)

    def send_command(self, line: str) -> bool:
        payload = (line.rstrip("\r\n") + "\n").encode("utf-8")
        with self._send_lock:
            if self._sock is None:
                return False
            try:
                self._sock.sendall(payload)
                return True
            except OSError as exc:
                with self._lock:
                    self._last_error = str(exc)
                return False

    def send_command_wait(self, line: str, timeout: float = 2.0) -> Optional[dict[str, Any]]:
        ev = threading.Event()
        box: dict[str, Any] = {}
        with self._lock:
            if not self._connected:
                return None
            self._cmd_waiters.append((ev, box))
        if not self.send_command(line):
            with self._lock:
                self._cmd_waiters = [(e, b) for e, b in self._cmd_waiters if e is not ev]
            return None
        ok = ev.wait(timeout=timeout)
        with self._lock:
            self._cmd_waiters = [(e, b) for e, b in self._cmd_waiters if e is not ev]
        return dict(box) if ok else None

    def request_snapshot(self, timeout: float = 1.5) -> Optional[dict[str, Any]]:
        ev = threading.Event()
        with self._lock:
            if not self._connected:
                return None
            self._snap_waiters.append(ev)
        if not self.send_command("SNAPSHOT"):
            return None
        ev.wait(timeout=timeout)
        with self._lock:
            return dict(self._telemetry) if self._telemetry else None

    def reset_rt_stats(self) -> bool:
        return self.send_command("RESET_STATS")

    def ping(self, timeout: float = 1.5) -> Optional[float]:
        tag = f"p{int(time.monotonic() * 1e6)}"
        ev = threading.Event()
        box: dict[str, Any] = {}
        with self._lock:
            if not self._connected:
                return None
            self._pong_waiters[tag] = (ev, box)
        t0 = time.monotonic_ns()
        if not self.send_command(f"PING {tag}"):
            with self._lock:
                self._pong_waiters.pop(tag, None)
            return None
        ok = ev.wait(timeout=timeout)
        t1 = time.monotonic_ns()
        with self._lock:
            self._pong_waiters.pop(tag, None)
        if not ok:
            return None
        return (t1 - t0) / 1_000_000.0

    def push_external_event(
        self,
        sub: str,
        lvl: str,
        op: str,
        target: str,
        detail: str,
        pid: int = 0,
        sid: int = 0,
        v1: int = 0,
        v2: int = 0,
        hr: int = 0,
        ts_ms: Optional[int] = None,
    ) -> None:
        ev = {
            "type": "event",
            "id": 0,
            "ts_mono_ns": time.monotonic_ns(),
            "ts_ms": ts_ms if ts_ms is not None else int(time.time() * 1000),
            "sub": sub,
            "lvl": lvl,
            "pid": pid,
            "sid": sid,
            "v1": v1,
            "v2": v2,
            "hr": hr,
            "op": op,
            "target": target,
            "detail": detail,
            "external": True,
        }
        self._append_event(ev)

    def clear_events(self) -> None:
        with self._lock:
            self._events.clear()

    def get_snapshot(self) -> dict[str, Any]:
        with self._lock:
            return {
                "connected": self._connected,
                "last_error": self._last_error,
                "hello": dict(self._hello),
                "telemetry": dict(self._telemetry),
                "telem_seq": self._telem_seq,
                "event_count": len(self._events),
                "latest_seq": self._next_local_seq - 1,
            }

    def get_events_since(self, min_seq: int = 0) -> tuple[list[dict[str, Any]], int]:
        with self._lock:
            latest = self._next_local_seq - 1
            if min_seq <= 0:
                return list(self._events), latest
            out = [ev for ev in self._events if ev.get("_seq", 0) > min_seq]
            return out, latest

    def _append_event(self, ev: dict[str, Any]) -> None:
        with self._lock:
            ev["_seq"] = self._next_local_seq
            self._next_local_seq += 1
            if not ev.get("time_str"):
                ev["time_str"] = format_ts_ms(ev.get("ts_ms", 0))
            self._events.append(ev)

    def _handle_line(self, line: str) -> None:
        if not line:
            return
        try:
            msg = json.loads(line)
        except ValueError:
            return
        mtype = msg.get("type")
        if mtype == "event":
            msg["external"] = False
            self._append_event(msg)
        elif mtype == "telemetry":
            with self._lock:
                self._telemetry = msg
                self._telem_seq += 1
                waiters = self._snap_waiters
                self._snap_waiters = []
            for w in waiters:
                w.set()
        elif mtype == "hello":
            with self._lock:
                self._hello = msg
        elif mtype == "pong":
            tag = msg.get("tag", "")
            with self._lock:
                entry = self._pong_waiters.get(tag)
            if entry is not None:
                ev, box = entry
                box.update(msg)
                ev.set()
        elif mtype == "cmd_reply":
            with self._lock:
                self._last_cmd_reply = msg
                cwaiters = list(self._cmd_waiters)
                self._cmd_waiters = []
            for ev, box in cwaiters:
                box.update(msg)
                ev.set()

    def _run_loop(self) -> None:
        backoff = 0.4
        while not self._stop_ev.is_set():
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            sock.settimeout(1.0)
            try:
                sock.connect(self.sock_path)
            except OSError as exc:
                try:
                    sock.close()
                except OSError:
                    pass
                with self._lock:
                    self._connected = False
                    self._last_error = str(exc)
                self._stop_ev.wait(backoff)
                backoff = min(2.0, backoff * 1.4)
                continue

            backoff = 0.4
            sock.settimeout(None)
            with self._send_lock:
                self._sock = sock
            with self._lock:
                self._connected = True
                self._last_error = ""

            buf = bytearray()
            try:
                while not self._stop_ev.is_set():
                    ready, _, _ = select.select([sock], [], [], 0.25)
                    if not ready:
                        continue
                    chunk = sock.recv(65536)
                    if not chunk:
                        break
                    buf.extend(chunk)
                    while True:
                        nl = buf.find(b"\n")
                        if nl < 0:
                            break
                        raw = bytes(buf[:nl]).decode("utf-8", errors="replace").strip()
                        del buf[: nl + 1]
                        if raw:
                            self._handle_line(raw)
            except OSError as exc:
                with self._lock:
                    self._last_error = str(exc)
            finally:
                with self._send_lock:
                    if self._sock is sock:
                        self._sock = None
                try:
                    sock.close()
                except OSError:
                    pass
                with self._lock:
                    self._connected = False


def snapshot_once(sock_path: str = DEBUG_SOCK, timeout: float = 1.5) -> dict[str, Any]:
    """One-shot blocking connect to collect hello, replay events, and latest telemetry."""
    out: dict[str, Any] = {"connected": False, "hello": {}, "events": [], "telemetry": {}, "error": ""}
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect(sock_path)
        out["connected"] = True
    except OSError as exc:
        out["error"] = str(exc)
        try:
            sock.close()
        except OSError:
            pass
        return out

    buf = bytearray()
    deadline = time.monotonic() + timeout
    asked_snap = False
    try:
        while time.monotonic() < deadline:
            rem = max(0.05, deadline - time.monotonic())
            ready, _, _ = select.select([sock], [], [], min(rem, 0.25))
            if ready:
                chunk = sock.recv(65536)
                if not chunk:
                    break
                buf.extend(chunk)
                while True:
                    nl = buf.find(b"\n")
                    if nl < 0:
                        break
                    line = bytes(buf[:nl]).decode("utf-8", errors="replace").strip()
                    del buf[: nl + 1]
                    if not line:
                        continue
                    try:
                        msg = json.loads(line)
                    except ValueError:
                        continue
                    mt = msg.get("type")
                    if mt == "hello":
                        out["hello"] = msg
                    elif mt == "event":
                        msg["time_str"] = format_ts_ms(msg.get("ts_ms", 0))
                        out["events"].append(msg)
                    elif mt == "telemetry":
                        out["telemetry"] = msg
                        return out
            elif not asked_snap:
                try:
                    sock.sendall(b"SNAPSHOT\n")
                except OSError:
                    break
                asked_snap = True
    except OSError as exc:
        out["error"] = str(exc)
    finally:
        try:
            sock.close()
        except OSError:
            pass
    return out


class SystemAuditScanner:
    """Scans /proc, systemd, kernel /proc/asound, and system config boundaries."""

    SALSA_COMMS = {"lwasapi-audiodg", "lwasapi-tray", "lwasapi-ctl", "salsa-gui", "salsa-debug"}
    AUDIO_MAP_TOKENS = (
        "liblwasapi.so",
        "libasound_module_pcm_lwasapi.so",
        "lwasapi/build/shims/",
        "libasound.so",
        "libpulse",
        "libjack",
        "libpipewire",
        "libopenal",
        "libSDL",
    )

    @staticmethod
    def _unix_peer_inode_map(pa_sock: str, pw_sock: str) -> dict[str, str]:
        """Map client socket inodes to listener paths via fast `ss -xnH`."""
        mapping: dict[str, str] = {}
        if not shutil.which("ss"):
            return mapping
        try:
            res = subprocess.run(["ss", "-xnH"], capture_output=True, text=True, timeout=0.8, check=False)
            if res.returncode != 0:
                return mapping
            for ln in res.stdout.splitlines():
                cols = ln.split()
                if len(cols) < 8:
                    continue
                local_addr = cols[4]
                peer_ino = cols[7]
                if not peer_ino.isdigit():
                    continue
                if local_addr == pa_sock or local_addr.endswith("/pulse/native"):
                    mapping[peer_ino] = pa_sock
                elif local_addr == pw_sock or local_addr.endswith("/pipewire-0"):
                    mapping[peer_ino] = pw_sock
                elif local_addr in (IPC_SOCK, DEBUG_SOCK):
                    mapping[peer_ino] = local_addr
        except (OSError, subprocess.SubprocessError):
            pass
        return mapping

    @classmethod
    def scan_audio_processes(cls, telemetry: Optional[dict[str, Any]] = None) -> list[dict[str, Any]]:
        """Scan /proc/<pid> and classify every audio-touching process."""
        rt_dir = _runtime_dir()
        pa_sock = os.path.join(rt_dir, "pulse", "native")
        pw_sock = os.path.join(rt_dir, "pipewire-0")
        peer_map = cls._unix_peer_inode_map(pa_sock, pw_sock)

        pa_pid_streams: dict[int, list[str]] = {}
        pa_pid_clients: set[int] = set()
        wasapi_pid_sessions: dict[int, list[str]] = {}
        if telemetry:
            for pc in telemetry.get("pa_clients") or []:
                if pc.get("pid"):
                    pa_pid_clients.add(int(pc["pid"]))
            for ps in telemetry.get("pa_streams") or []:
                if ps.get("pid"):
                    pa_pid_streams.setdefault(int(ps["pid"]), []).append(str(ps.get("name") or "stream"))
            for ws in telemetry.get("wasapi_sessions") or []:
                if ws.get("pid"):
                    wasapi_pid_sessions.setdefault(int(ws["pid"]), []).append(str(ws.get("name") or "session"))

        rows: list[dict[str, Any]] = []

        for entry in os.listdir("/proc"):
            if not entry.isdigit():
                continue
            pid = int(entry)
            comm = _read_text(f"/proc/{pid}/comm", 256).strip()
            if not comm:
                continue

            fd_dir = f"/proc/{pid}/fd"
            snd_fds: list[str] = []
            shm_fds: list[str] = []
            sock_hits: list[str] = []
            has_pa_sock = pid in pa_pid_clients or pid in pa_pid_streams
            has_pw_sock = False
            has_salsa_ipc = pid in wasapi_pid_sessions
            has_salsa_shm = pid in wasapi_pid_sessions

            try:
                for fd_name in os.listdir(fd_dir):
                    try:
                        target = os.readlink(os.path.join(fd_dir, fd_name))
                    except OSError:
                        continue
                    if target.startswith("socket:[") and target.endswith("]"):
                        ino = target[8:-1]
                        resolved = peer_map.get(ino)
                        if resolved:
                            target = resolved
                    if target.startswith("/dev/snd/"):
                        snd_fds.append(target)
                    elif "lwasapi_ring" in target:
                        shm_fds.append(target)
                        has_salsa_shm = True
                    elif target == IPC_SOCK:
                        sock_hits.append(IPC_SOCK)
                        has_salsa_ipc = True
                    elif target == DEBUG_SOCK or target == DEBUG_IN_SOCK:
                        sock_hits.append(target)
                    elif target == pa_sock or target.endswith("/pulse/native"):
                        sock_hits.append(target)
                        has_pa_sock = True
                    elif target == pw_sock or target.endswith("/pipewire-0"):
                        sock_hits.append(target)
                        has_pw_sock = True
            except OSError:
                pass

            map_hits: list[str] = []
            has_liblwasapi = False
            has_alsa_plugin = False
            has_shim = False
            maps_txt = _read_text(f"/proc/{pid}/maps", 524288)
            if maps_txt:
                seen_libs: set[str] = set()
                for line in maps_txt.splitlines():
                    idx = line.find("/")
                    if idx < 0:
                        continue
                    lib_path = line[idx:].strip()
                    if any(tok in lib_path for tok in cls.AUDIO_MAP_TOKENS):
                        if lib_path not in seen_libs:
                            seen_libs.add(lib_path)
                            map_hits.append(lib_path)
                            if "liblwasapi.so" in lib_path:
                                has_liblwasapi = True
                            if "libasound_module_pcm_lwasapi.so" in lib_path:
                                has_alsa_plugin = True
                            if "/build/shims/" in lib_path:
                                has_shim = True

            is_core = comm in cls.SALSA_COMMS or (
                comm.startswith("python") and ("salsa-gui" in _read_cmdline(pid) or "lwasapi-tray" in _read_cmdline(pid) or "salsa-debug" in _read_cmdline(pid))
            )
            has_audio_footprint = bool(snd_fds or shm_fds or sock_hits or map_hits or is_core or has_pa_sock or has_salsa_shm)
            if not has_audio_footprint:
                continue

            # Determine classification
            route = "Audio Lib Loaded (Idle)"
            severity = "INFO"
            if is_core:
                route = "Salsa Core Component"
                severity = "OK"
            elif has_salsa_shm or has_salsa_ipc or (has_liblwasapi and not has_pa_sock and not has_pw_sock):
                if has_shim or has_alsa_plugin:
                    route = "Salsa Shims / ALSA Plugin -> WaveRT"
                else:
                    snames = wasapi_pid_sessions.get(pid)
                    route = f"Salsa Native WaveRT SHM ({len(snames)} active)" if snames else "Salsa Native WaveRT SHM"
                severity = "OK"
            elif has_pa_sock:
                pnames = pa_pid_streams.get(pid)
                route = f"Salsa PA v35 (/pulse/native | {len(pnames)} stream{'s' if len(pnames) != 1 else ''})" if pnames else "Salsa PA v35 (/pulse/native)"
                severity = "OK"
            elif snd_fds and comm not in ("wireplumber", "pipewire"):
                route = f"Direct Kernel ALSA ({', '.join(sorted(set(snd_fds))[:2])})"
                severity = "WARN"
            elif has_pw_sock:
                if comm in ("wireplumber", "pipewire") or "xdg-desktop-por" in comm or comm in ("kwin_wayland", "spectacle"):
                    route = "PipeWire Video / Screen Portal"
                    severity = "INFO"
                elif any("libpulse" in m or "libasound" in m or "libopenal" in m or "libjack" in m for m in map_hits):
                    route = "PipeWire Socket Attached (/pipewire-0)"
                    severity = "WARN"
                else:
                    route = "PipeWire Client (Portal / Video)"
                    severity = "INFO"
            elif has_shim or has_alsa_plugin:
                route = "Salsa Shims / ALSA Plugin"
                severity = "OK"

            rows.append(
                {
                    "pid": pid,
                    "comm": comm,
                    "cmdline": _read_cmdline(pid)[:220],
                    "route": route,
                    "severity": severity,
                    "snd_fds": sorted(set(snd_fds)),
                    "shm_fds": sorted(set(shm_fds)),
                    "sockets": sorted(set(sock_hits)),
                    "pa_streams": pa_pid_streams.get(pid, []),
                    "wasapi_sessions": wasapi_pid_sessions.get(pid, []),
                    "maps": map_hits,
                }
            )

        order = {"WARN": 0, "OK": 1, "INFO": 2}
        rows.sort(key=lambda r: (order.get(r["severity"], 9), r["comm"], r["pid"]))
        return rows

    @classmethod
    def inspect_systemd_units(cls) -> list[dict[str, str]]:
        props = "Id,LoadState,ActiveState,SubState,UnitFileState,MainPID,NRestarts,ExecMainStartTimestamp"
        results: list[dict[str, str]] = []
        user_units = [u for scope, u in UNIT_LIST if scope == "user"]
        sys_units = [u for scope, u in UNIT_LIST if scope == "system"]

        for scope, units in (("user", user_units), ("system", sys_units)):
            if not units:
                continue
            cmd = ["systemctl"] + (["--user"] if scope == "user" else []) + ["show", "--property=" + props, *units]
            try:
                res = subprocess.run(cmd, capture_output=True, text=True, timeout=2.0, check=False)
                cur: dict[str, str] = {"Scope": scope}
                for raw in res.stdout.splitlines():
                    if not raw.strip():
                        if "Id" in cur:
                            results.append(cur)
                        cur = {"Scope": scope}
                        continue
                    if "=" in raw:
                        k, v = raw.split("=", 1)
                        cur[k] = v
                if "Id" in cur:
                    results.append(cur)
            except (OSError, subprocess.SubprocessError) as exc:
                results.append({"Scope": scope, "Id": "systemctl-error", "ActiveState": str(exc)})
        return results

    @classmethod
    def inspect_kernel_asound(cls) -> dict[str, Any]:
        cards_txt = _read_text("/proc/asound/cards")
        pcm_txt = _read_text("/proc/asound/pcm")
        active_subs: list[dict[str, str]] = []
        for status_path in sorted(glob.glob("/proc/asound/card*/pcm*/sub*/status")):
            st_txt = _read_text(status_path)
            if "state: RUNNING" in st_txt or "state: OPEN" in st_txt or "state: PREPARED" in st_txt or "state: XRUN" in st_txt:
                hw_path = status_path.rsplit("/", 1)[0] + "/hw_params"
                owner = ""
                state = ""
                for line in st_txt.splitlines():
                    if line.startswith("state:"):
                        state = line.split(":", 1)[1].strip()
                    elif "owner_pid" in line:
                        owner = line.split(":", 1)[1].strip()
                active_subs.append(
                    {
                        "node": status_path.replace("/proc/asound/", "").replace("/status", ""),
                        "state": state,
                        "owner_pid": owner,
                        "hw_params": _read_text(hw_path).strip(),
                        "status_raw": st_txt.strip(),
                    }
                )
        return {"cards": cards_txt.strip(), "pcm": pcm_txt.strip(), "active_streams": active_subs}

    @classmethod
    def check_build_freshness(cls) -> dict[str, Any]:
        rust_srcs = sorted(
            glob.glob(os.path.join(BASE_DIR, "rust", "salsa_net", "src", "*.rs"))
            + glob.glob(os.path.join(BASE_DIR, "rust", "salsa_net", "src", "bin", "*.rs"))
            + [os.path.join(BASE_DIR, "rust", "salsa_net", "Cargo.toml")]
        )
        rust_rel_deps = tuple(os.path.relpath(p, BASE_DIR) for p in rust_srcs)
        src_files = sorted(
            glob.glob(os.path.join(BASE_DIR, "src", "*.c"))
            + glob.glob(os.path.join(BASE_DIR, "include", "*.h"))
            + rust_srcs
        )
        newest_src_path = ""
        newest_src_mtime = 0.0
        for p in src_files:
            try:
                mt = os.path.getmtime(p)
                if mt > newest_src_mtime:
                    newest_src_mtime = mt
                    newest_src_path = os.path.relpath(p, BASE_DIR)
            except OSError:
                pass

        lib_deps = ("src/liblwasapi.c", "include/lwasapi.h", "include/lwasapi_debug.h")
        proxy_deps = ("src/salsa_pa_reconnect.c", "src/pulse.version", "scripts/gen_pa_proxy.py")
        artifact_list: list[tuple[str, tuple[str, ...]]] = [
            ("build/lwasapi-audiodg", rust_rel_deps),
            ("build/liblwasapi.so", lib_deps),
            ("build/libasound_module_pcm_lwasapi.so", ("src/alsa_plugin_lwasapi.c", *lib_deps)),
            ("build/libsalsa_pa_reconnect.so", ("src/salsa_pa_reconnect.c",)),
            ("build/pa_reconnect/libpulse.so.0", proxy_deps),
            ("build/lwasapi-ctl", rust_rel_deps),
            ("build/shims/libasound.so.2", ("src/shim_alsa.c", "src/alsa.version", *lib_deps)),
            ("build/shims/libpulse.so.0", ("src/shim_pulse.c", "src/pulse.version", *lib_deps)),
            ("build/shims/libjack.so.0", ("src/shim_jack.c", *lib_deps)),
        ]
        if os.path.exists(os.path.join(BASE_DIR, "build/pa_reconnect32/libpulse.so.0")):
            artifact_list.append(("build/pa_reconnect32/libpulse.so.0", proxy_deps))
        artifact_deps: tuple[tuple[str, tuple[str, ...]], ...] = tuple(artifact_list)
        rows: list[dict[str, Any]] = []
        any_stale = False
        for rel, deps in artifact_deps:
            ap = os.path.join(BASE_DIR, rel)
            if not os.path.exists(ap):
                any_stale = True
                rows.append({"artifact": rel, "exists": False, "stale": True, "mtime": 0.0})
                continue
            mt = os.path.getmtime(ap)
            dep_mtime = 0.0
            for dep in deps:
                dp = os.path.join(BASE_DIR, dep)
                if os.path.exists(dp):
                    dep_mtime = max(dep_mtime, os.path.getmtime(dp))
            stale = dep_mtime > mt + 0.5
            if stale:
                any_stale = True
            rows.append({"artifact": rel, "exists": True, "stale": stale, "mtime": mt})

        # Check running daemon vs disk binary
        daemon_stale = False
        daemon_pids: list[int] = []
        audiodg_bin = os.path.join(BUILD_DIR, "lwasapi-audiodg")
        bin_mtime = os.path.getmtime(audiodg_bin) if os.path.exists(audiodg_bin) else 0.0
        for entry in os.listdir("/proc"):
            if entry.isdigit() and _read_text(f"/proc/{entry}/comm", 64).strip() == "lwasapi-audiodg":
                pid = int(entry)
                daemon_pids.append(pid)
                try:
                    proc_mtime = os.stat(f"/proc/{pid}/stat").st_mtime
                    if bin_mtime > proc_mtime + 1.0:
                        daemon_stale = True
                except OSError:
                    pass

        return {
            "newest_source": newest_src_path,
            "newest_source_mtime": newest_src_mtime,
            "any_artifact_stale": any_stale,
            "running_audiodg_pids": daemon_pids,
            "running_daemon_predates_binary": daemon_stale,
            "artifacts": rows,
        }

    @classmethod
    def inspect_configs_and_links(cls) -> dict[str, Any]:
        cfg_paths = {
            "~/.asoundrc": os.path.expanduser("~/.asoundrc"),
            "~/.config/OpenAL/alsoft.conf": os.path.expanduser("~/.config/OpenAL/alsoft.conf"),
            "pipewire_zero_mode": os.path.join(STATE_DIR, "pipewire_zero_mode"),
            "99-salsa-direct.conf": os.path.expanduser("~/.config/environment.d/99-salsa-direct.conf"),
            "99-lwasapi-video-only.conf": os.path.expanduser("~/.config/wireplumber/wireplumber.conf.d/99-lwasapi-video-only.conf"),
            "module_versions.state": os.path.join(STATE_DIR, "module_versions.state"),
            "src/alsa.version": os.path.join(BASE_DIR, "src", "alsa.version"),
            "src/pulse.version": os.path.join(BASE_DIR, "src", "pulse.version"),
        }
        configs: dict[str, dict[str, Any]] = {}
        for label, path in cfg_paths.items():
            exists = os.path.exists(path)
            configs[label] = {
                "path": path,
                "exists": exists,
                "content": _read_text(path, 32768) if exists else "",
            }

        # Check active ALSA plugin (~/.asoundrc target or system path) vs build artifact
        sys_plugin = "/usr/lib/alsa-lib/libasound_module_pcm_lwasapi.so"
        bld_plugin = os.path.join(BUILD_DIR, "libasound_module_pcm_lwasapi.so")
        asound_txt = configs.get("~/.asoundrc", {}).get("content", "")
        cfg_plugin = ""
        m_plug = re.search(r'"([^"]*libasound_module_pcm_lwasapi\.so)"', asound_txt)
        if m_plug:
            cfg_plugin = os.path.expanduser(m_plug.group(1))
        active_plugin = cfg_plugin if (cfg_plugin and os.path.lexists(cfg_plugin)) else sys_plugin
        plugin_info = {
            "active_path": active_plugin,
            "configured_in_asoundrc": cfg_plugin,
            "system_path": sys_plugin,
            "exists": os.path.lexists(active_plugin),
            "system_exists": os.path.lexists(sys_plugin),
            "is_symlink": os.path.islink(active_plugin),
            "target": os.readlink(active_plugin) if os.path.islink(active_plugin) else "",
            "matches_build_sha": bool(
                os.path.exists(active_plugin) and os.path.exists(bld_plugin) and _sha256_file(active_plugin) == _sha256_file(bld_plugin)
            ),
        }

        rt_dir = _runtime_dir()
        sockets = {
            "audiodg_ipc": _peercred_for_unix_socket(IPC_SOCK),
            "debug_stream": _peercred_for_unix_socket(DEBUG_SOCK),
            "pulse_native": _peercred_for_unix_socket(os.path.join(rt_dir, "pulse", "native")),
            "pipewire_0": _peercred_for_unix_socket(os.path.join(rt_dir, "pipewire-0")),
            "pipewire_0_manager": _peercred_for_unix_socket(os.path.join(rt_dir, "pipewire-0-manager")),
        }
        return {"configs": configs, "alsa_plugin": plugin_info, "sockets": sockets}

    @classmethod
    def inspect_pipewire_boundary(cls) -> dict[str, Any]:
        """Verify PipeWire boundary is either Salsa Pure-Rust Video Broker or video-only WirePlumber."""
        wp_audio_nodes: list[str] = []
        wp_video_nodes: list[str] = []
        rt_dir = _runtime_dir()
        pw0 = _peercred_for_unix_socket(os.path.join(rt_dir, "pipewire-0"))
        zero_txt = _read_text(os.path.join(STATE_DIR, "pipewire_zero_mode"), 64).strip()
        zero_pw_mode = zero_txt.startswith("enabled") or zero_txt.startswith("1")
        salsa_owns_pw0 = pw0.get("peer_comm") == "lwasapi-audiodg"
        err = ""
        if shutil.which("pw-dump"):
            try:
                res = subprocess.run(["pw-dump"], capture_output=True, text=True, timeout=1.5, check=False)
                if res.returncode == 0 and res.stdout.strip().startswith("["):
                    objs = json.loads(res.stdout)
                    for o in objs:
                        if o.get("type") != "PipeWire:Interface:Node":
                            continue
                        props = (o.get("info") or {}).get("props") or {}
                        mc = props.get("media.class", "")
                        node_nm = props.get("node.name", "")
                        if node_nm in ("salsa_lwasapi_pw_sink", "salsa_lwasapi_pw_source"):
                            continue
                        name = props.get("node.description") or node_nm or f"id:{o.get('id')}"
                        if "Audio" in mc:
                            wp_audio_nodes.append(f"{mc}: {name}")
                        elif "Video" in mc or "Stream/" in mc:
                            wp_video_nodes.append(f"{mc}: {name}")
                else:
                    err = res.stderr.strip() or f"pw-dump rc={res.returncode}"
            except (OSError, ValueError, subprocess.SubprocessError) as exc:
                err = str(exc)
        else:
            err = "pw-dump not installed"
        return {
            "broker": "Salsa Pure-Rust Video Broker" if salsa_owns_pw0 else "Upstream PipeWire",
            "zero_pw_mode": zero_pw_mode,
            "salsa_owns_pw0": salsa_owns_pw0,
            "pw0_owner": pw0.get("peer_comm") or "",
            "pw0_pid": pw0.get("peer_pid") or 0,
            "audio_nodes": wp_audio_nodes,
            "video_nodes": wp_video_nodes,
            "error": err,
        }

    @classmethod
    def inspect_boundary(cls) -> dict[str, Any]:
        cfg = cls.inspect_configs_and_links()
        return {
            "timestamp_ms": int(time.time() * 1000),
            "systemd": cls.inspect_systemd_units(),
            "asound": cls.inspect_kernel_asound(),
            "build_freshness": cls.check_build_freshness(),
            "pipewire": cls.inspect_pipewire_boundary(),
            **cfg,
        }


class ExternalLogManager:
    """Manages optional journalctl, autosync.log, and dbus-monitor tailers."""

    def __init__(self, client: SalsaDebugClient) -> None:
        self.client = client
        self._lock = threading.Lock()
        self._stops: dict[str, threading.Event] = {}
        self._procs: dict[str, subprocess.Popen[str]] = {}
        self._threads: dict[str, threading.Thread] = {}

    def set_enabled(self, kind: str, enabled: bool) -> None:
        if enabled:
            self._start_watcher(kind)
        else:
            self._stop_watcher(kind)

    def stop_all(self) -> None:
        for kind in list(self._threads.keys()):
            self._stop_watcher(kind)

    def _stop_watcher(self, kind: str) -> None:
        with self._lock:
            ev = self._stops.pop(kind, None)
            proc = self._procs.pop(kind, None)
            th = self._threads.pop(kind, None)
        if ev:
            ev.set()
        if proc and proc.poll() is None:
            try:
                proc.terminate()
                proc.wait(timeout=0.8)
            except (OSError, subprocess.SubprocessError):
                try:
                    proc.kill()
                except OSError:
                    pass
        if th and th.is_alive():
            th.join(timeout=1.0)

    def _start_watcher(self, kind: str) -> None:
        with self._lock:
            if kind in self._threads and self._threads[kind].is_alive():
                return
            ev = threading.Event()
            self._stops[kind] = ev
            target_fn = {
                "journal": self._watch_journal,
                "autosync": self._watch_autosync,
                "dbus": self._watch_dbus,
            }.get(kind)
            if target_fn is None:
                return
            th = threading.Thread(target=target_fn, args=(ev,), name=f"SalsaExt-{kind}", daemon=True)
            self._threads[kind] = th
            th.start()

    def _watch_journal(self, stop_ev: threading.Event) -> None:
        cmd = [
            "journalctl",
            "--user",
            "-u",
            "lwasapi-audiodg",
            "-u",
            "lwasapi-tray",
            "-u",
            "lwasapi-module-watcher",
            "-u",
            "wireplumber",
            "-n",
            "25",
            "-f",
            "-o",
            "json",
        ]
        try:
            proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        except OSError as exc:
            self.client.push_external_event("SYSTEM", "WARN", "JOURNAL_ERR", "journalctl", str(exc))
            return
        with self._lock:
            self._procs["journal"] = proc
        assert proc.stdout is not None
        for line in proc.stdout:
            if stop_ev.is_set():
                break
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            unit = rec.get("_SYSTEMD_USER_UNIT") or rec.get("SYSLOG_IDENTIFIER") or "journal"
            msg = rec.get("MESSAGE", "")
            prio = int(rec.get("PRIORITY", 6)) if str(rec.get("PRIORITY", "6")).isdigit() else 6
            lvl = "ERR" if prio <= 3 else ("WARN" if prio == 4 else "INFO")
            ts_us = int(rec.get("__REALTIME_TIMESTAMP", 0) or 0)
            self.client.push_external_event(
                "SYSTEM",
                lvl,
                "JOURNAL",
                str(unit).replace(".service", ""),
                str(msg),
                pid=int(rec.get("_PID", 0) or 0),
                ts_ms=ts_us // 1000 if ts_us else None,
            )

    def _watch_autosync(self, stop_ev: threading.Event) -> None:
        pos = 0
        if os.path.exists(AUTOSYNC_LOG):
            txt = _read_text(AUTOSYNC_LOG, 131072)
            lines = [ln for ln in txt.splitlines() if ln.strip()]
            for ln in lines[-20:]:
                self.client.push_external_event("SYSTEM", "INFO", "AUTOSYNC", "autosync.log", ln)
            pos = os.path.getsize(AUTOSYNC_LOG)
        while not stop_ev.is_set():
            if os.path.exists(AUTOSYNC_LOG):
                try:
                    sz = os.path.getsize(AUTOSYNC_LOG)
                    if sz < pos:
                        pos = 0
                    if sz > pos:
                        with open(AUTOSYNC_LOG, "r", encoding="utf-8", errors="replace") as f:
                            f.seek(pos)
                            chunk = f.read()
                            pos = f.tell()
                        for ln in chunk.splitlines():
                            if ln.strip():
                                lvl = "WARN" if "WARN" in ln or "ERROR" in ln else "INFO"
                                self.client.push_external_event("SYSTEM", lvl, "AUTOSYNC", "autosync.log", ln.strip())
                except OSError:
                    pass
            stop_ev.wait(0.8)

    def _watch_dbus(self, stop_ev: threading.Event) -> None:
        if not shutil.which("dbus-monitor"):
            self.client.push_external_event("SYSTEM", "WARN", "DBUS_ERR", "dbus-monitor", "dbus-monitor not installed")
            return
        cmd = [
            "dbus-monitor",
            "--system",
            "sender='org.bluez'",
            "interface='org.bluez.MediaTransport1'",
            "interface='org.bluez.MediaEndpoint1'",
        ]
        try:
            proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        except OSError as exc:
            self.client.push_external_event("BLUEZ", "WARN", "DBUS_ERR", "dbus-monitor", str(exc))
            return
        with self._lock:
            self._procs["dbus"] = proc
        assert proc.stdout is not None
        header_re = re.compile(r"^(signal|method call|method return|error)\s+.*?(member=[^\s]+|interface=[^\s]+)")
        for line in proc.stdout:
            if stop_ev.is_set():
                break
            s = line.strip()
            if not s:
                continue
            if s.startswith(("signal ", "method call ", "error ")) and "org.bluez" in s:
                self.client.push_external_event("BLUEZ", "TRACE", "DBUS", "org.bluez", s[:138])


class DiagnosticProbes:
    """Interactive verification probes (IPC latency, soft sine tone, MIDI ping, PA check, bundle export)."""

    @staticmethod
    def probe_ipc_latency(client: Optional[SalsaDebugClient] = None) -> dict[str, Any]:
        t0 = time.monotonic_ns()
        res = subprocess.run([CTL_BIN, "status"], capture_output=True, text=True, timeout=2.5, check=False)
        ipc_ms = (time.monotonic_ns() - t0) / 1_000_000.0
        dbg_rtt_ms: Optional[float] = None
        if client is not None:
            dbg_rtt_ms = client.ping(timeout=1.5)
        else:
            # One-shot PING over DEBUG_SOCK
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            sock.settimeout(1.2)
            try:
                sock.connect(DEBUG_SOCK)
                p0 = time.monotonic_ns()
                sock.sendall(b"PING probe\n")
                buf = b""
                while time.monotonic_ns() - p0 < 1_000_000_000:
                    chunk = sock.recv(65536)
                    if not chunk:
                        break
                    buf += chunk
                    if b'"type":"pong"' in buf:
                        dbg_rtt_ms = (time.monotonic_ns() - p0) / 1_000_000.0
                        break
            except OSError:
                pass
            finally:
                try:
                    sock.close()
                except OSError:
                    pass
        return {
            "ok": res.returncode == 0,
            "ctl_status_ms": round(ipc_ms, 2),
            "debug_ping_ms": round(dbg_rtt_ms, 3) if dbg_rtt_ms is not None else None,
            "detail": (res.stderr or res.stdout).strip()[:240],
        }

    @staticmethod
    def probe_test_tone(freq_hz: float = 440.0, duration_ms: int = 220, amp: float = 0.14) -> dict[str, Any]:
        """Play a brief gentle stereo sine wave via liblwasapi.so (WASAPI Shared Mode)."""
        if not os.path.exists(LIB_LWASAPI_SO):
            return {"ok": False, "error": f"Missing {LIB_LWASAPI_SO}"}
        try:
            lib = ctypes.CDLL(LIB_LWASAPI_SO)
            lib.LWASAPI_ActivateAudioClient.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
            lib.LWASAPI_ActivateAudioClient.restype = ctypes.c_int32
            lib.IAudioClient_GetMixFormat.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
            lib.IAudioClient_GetMixFormat.restype = ctypes.c_int32
            lib.IAudioClient_Initialize.argtypes = [
                ctypes.c_void_p,
                ctypes.c_int32,
                ctypes.c_uint32,
                ctypes.c_uint32,
                ctypes.c_uint32,
                ctypes.c_void_p,
            ]
            lib.IAudioClient_Initialize.restype = ctypes.c_int32
            lib.ISimpleAudioVolume_SetMasterVolume.argtypes = [ctypes.c_void_p, ctypes.c_float]
            lib.ISimpleAudioVolume_SetMasterVolume.restype = ctypes.c_int32
            lib.IAudioClient_Start.argtypes = [ctypes.c_void_p]
            lib.IAudioClient_Start.restype = ctypes.c_int32
            lib.IAudioRenderClient_WriteFrames.argtypes = [
                ctypes.c_void_p,
                ctypes.c_void_p,
                ctypes.c_uint32,
                ctypes.c_int32,
            ]
            lib.IAudioRenderClient_WriteFrames.restype = ctypes.c_int32
            lib.IAudioClient_Stop.argtypes = [ctypes.c_void_p]
            lib.IAudioClient_Stop.restype = ctypes.c_int32
            lib.IAudioClient_Release.argtypes = [ctypes.c_void_p]
            lib.IAudioClient_Release.restype = ctypes.c_int32

            client_ptr = ctypes.c_void_p()
            hr = lib.LWASAPI_ActivateAudioClient(b"SalsaDebugProbe", ctypes.byref(client_ptr))
            if hr != 0 or not client_ptr.value:
                return {"ok": False, "error": f"ActivateAudioClient hr=0x{hr & 0xffffffff:08x}"}

            wfx = ctypes.create_string_buffer(32)
            hr = lib.IAudioClient_GetMixFormat(client_ptr, wfx)
            if hr != 0:
                lib.IAudioClient_Release(client_ptr)
                return {"ok": False, "error": f"GetMixFormat hr=0x{hr & 0xffffffff:08x}"}

            _fmt_tag, channels, sample_rate = struct.unpack_from("<HHI", wfx.raw, 0)
            channels = max(1, min(8, channels))
            sample_rate = max(44100, min(192000, sample_rate))

            # AUDCLNT_SHAREMODE_SHARED = 0, AUDCLNT_STREAMFLAGS_EVENTCALLBACK = 0x00040000
            hr = lib.IAudioClient_Initialize(client_ptr, 0, 0x00040000, 4096, 256, wfx)
            if hr != 0:
                lib.IAudioClient_Release(client_ptr)
                return {"ok": False, "error": f"Initialize hr=0x{hr & 0xffffffff:08x}"}

            lib.ISimpleAudioVolume_SetMasterVolume(client_ptr, ctypes.c_float(0.6))
            lib.IAudioClient_Start(client_ptr)

            total_frames = max(512, int(sample_rate * (duration_ms / 1000.0)))
            chunk_frames = 512
            written = 0
            while written < total_frames:
                n = min(chunk_frames, total_frames - written)
                arr = (ctypes.c_float * (n * channels))()
                for i in range(n):
                    idx = written + i
                    # Soft 15ms fade-in/out envelope to prevent clicks
                    env = 1.0
                    fade = min(total_frames // 3, int(sample_rate * 0.015))
                    if fade > 0:
                        if idx < fade:
                            env = idx / float(fade)
                        elif total_frames - idx < fade:
                            env = (total_frames - idx) / float(fade)
                    s = float(math.sin(2.0 * math.pi * freq_hz * (idx / float(sample_rate))) * amp * env)
                    for c in range(channels):
                        arr[i * channels + c] = s
                lib.IAudioRenderClient_WriteFrames(client_ptr, arr, n, 1)  # LWASAPI_FMT_FLOAT32 = 1
                written += n

            time.sleep(0.05)
            lib.IAudioClient_Stop(client_ptr)
            lib.IAudioClient_Release(client_ptr)
            return {"ok": True, "sample_rate": sample_rate, "channels": channels, "frames": total_frames}
        except Exception as exc:
            return {"ok": False, "error": str(exc)}

    @staticmethod
    def probe_midi_ping(note: int = 60, vel: int = 96) -> dict[str, Any]:
        r1 = subprocess.run([CTL_BIN, "midi-send", str(note), str(vel)], capture_output=True, text=True, timeout=2.0, check=False)
        time.sleep(0.1)
        r2 = subprocess.run([CTL_BIN, "midi-send", str(note), "0"], capture_output=True, text=True, timeout=2.0, check=False)
        return {
            "ok": r1.returncode == 0 and r2.returncode == 0,
            "note": note,
            "vel": vel,
            "detail": (r1.stdout or r1.stderr or "").strip(),
        }

    @staticmethod
    def probe_pa_socket() -> dict[str, Any]:
        rt_dir = _runtime_dir()
        pa_path = os.path.join(rt_dir, "pulse", "native")
        peer = _peercred_for_unix_socket(pa_path)
        pactl_info = ""
        pactl_ok = None
        if shutil.which("pactl"):
            try:
                res = subprocess.run(["pactl", "info"], capture_output=True, text=True, timeout=1.5, check=False)
                pactl_ok = res.returncode == 0
                pactl_info = (res.stdout if res.returncode == 0 else res.stderr).strip()[:600]
            except (OSError, subprocess.SubprocessError) as exc:
                pactl_ok = False
                pactl_info = str(exc)
        return {"socket": peer, "pactl_ok": pactl_ok, "pactl_info": pactl_info}

    @staticmethod
    def export_debug_bundle(
        client_snapshot: Optional[dict[str, Any]] = None,
        events: Optional[list[dict[str, Any]]] = None,
        audit_data: Optional[dict[str, Any]] = None,
        procs: Optional[list[dict[str, Any]]] = None,
        out_dir: str = DEFAULT_BUNDLE_DIR,
    ) -> str:
        os.makedirs(out_dir, exist_ok=True)
        ts = time.strftime("%Y%m%d_%H%M%S")
        out_path = os.path.join(out_dir, f"salsa-debug-bundle-{ts}.json")

        if client_snapshot is None:
            one = snapshot_once()
            telem = one.get("telemetry", {})
            hello = one.get("hello", {})
            if events is None:
                events = one.get("events", [])
        else:
            telem = client_snapshot.get("telemetry", {})
            hello = client_snapshot.get("hello", {})

        bundle = {
            "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "hello": hello,
            "telemetry": telem,
            "events": events or [],
            "audio_processes": procs if procs is not None else SystemAuditScanner.scan_audio_processes(telem or None),
            "boundary_audit": audit_data if audit_data is not None else SystemAuditScanner.inspect_boundary(),
            "pa_probe": DiagnosticProbes.probe_pa_socket(),
        }
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump(bundle, f, indent=2)
        return out_path


class ScreenCastShmReader:
    """Lockless seqlock-protected reader for PipeWire preview shared memory."""

    def __init__(self, shm_name: str = "") -> None:
        self._uid = os.getuid()
        default_path = os.path.join(PRIVATE_RUNTIME_DIR, "pw_preview")
        self.set_shm_name(shm_name or (default_path if os.path.exists(default_path) else f"/lwasapi_pw_preview_{self._uid}"))
        self._fd: Optional[int] = None
        self._mm: Optional[mmap.mmap] = None
        self._last_frame_seq: int = -1

    def set_shm_name(self, shm_name: str) -> None:
        raw = shm_name or os.path.join(PRIVATE_RUNTIME_DIR, "pw_preview")
        if raw.startswith(("/run/", "/tmp/")) and "/lwasapi" in raw:
            new_path = raw
        else:
            clean = raw.lstrip("/")
            new_path = f"/dev/shm/{clean}"
        if getattr(self, "shm_path", "") != new_path:
            self.close()
            self.shm_path = new_path

    def close(self) -> None:
        if getattr(self, "_mm", None) is not None:
            try:
                self._mm.close()
            except (OSError, ValueError):
                pass
            self._mm = None
        if getattr(self, "_fd", None) is not None:
            try:
                os.close(self._fd)
            except OSError:
                pass
            self._fd = None

    def _ensure_mapped(self) -> bool:
        if self._mm is not None:
            return True
        if not os.path.exists(self.shm_path):
            return False
        try:
            fd = os.open(self.shm_path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        except OSError:
            return False
        try:
            st = os.fstat(fd)
            if st.st_uid != self._uid or st.st_size < PW_PREVIEW_TOTAL_SIZE:
                os.close(fd)
                return False
            mm = mmap.mmap(fd, PW_PREVIEW_TOTAL_SIZE, mmap.MAP_SHARED, mmap.PROT_READ)
            self._fd = fd
            self._mm = mm
            return True
        except OSError:
            try:
                os.close(fd)
            except OSError:
                pass
            return False

    def read_frame(self) -> Optional[dict[str, Any]]:
        if not self._ensure_mapped() or self._mm is None:
            return None
        mm = self._mm
        for _ in range(4):
            try:
                seq1 = struct.unpack_from("<Q", mm, 8)[0]
                if seq1 & 1:
                    time.sleep(0.0005)
                    continue
                vals = PW_PREVIEW_HDR_STRUCT.unpack_from(mm, 0)
                (
                    magic,
                    version,
                    seqlock_v,
                    last_ping_ns,
                    preview_requested,
                    active_stream,
                    is_dmabuf,
                    dmabuf_mappable,
                    src_w,
                    src_h,
                    src_stride,
                    src_spa_fmt,
                    drm_mod_lo,
                    drm_mod_hi,
                    dst_w,
                    dst_h,
                    dst_stride,
                    prod_node_id,
                    cons_count,
                    frame_seq,
                    pts_ns,
                    prod_raw,
                    fmt_raw,
                    target_max_w,
                    target_max_h,
                    target_fps,
                    actual_fps_x100,
                    cast_fps_cap,
                ) = vals
                if magic != PW_PREVIEW_MAGIC or version != PW_PREVIEW_VERSION:
                    return None
                need_bytes = dst_h * dst_stride
                pixels = b""
                if (
                    dst_w > 0
                    and dst_h > 0
                    and dst_stride >= dst_w * 4
                    and need_bytes <= PW_PREVIEW_MAX_BYTES
                ):
                    pixels = bytes(mm[PW_PREVIEW_PIXELS_OFF : PW_PREVIEW_PIXELS_OFF + need_bytes])
                seq2 = struct.unpack_from("<Q", mm, 8)[0]
                if seq1 != seq2 or (seq2 & 1) or seq1 != seqlock_v:
                    continue
                return {
                    "magic": magic,
                    "version": version,
                    "seqlock": seq2,
                    "last_gui_ping_ns": last_ping_ns,
                    "preview_requested": bool(preview_requested),
                    "active_stream": bool(active_stream),
                    "is_dmabuf": bool(is_dmabuf),
                    "dmabuf_mappable": bool(dmabuf_mappable),
                    "src_width": src_w,
                    "src_height": src_h,
                    "src_stride": src_stride,
                    "src_spa_format": src_spa_fmt,
                    "drm_modifier": ((drm_mod_hi << 32) | drm_mod_lo),
                    "dst_width": dst_w,
                    "dst_height": dst_h,
                    "dst_stride": dst_stride,
                    "prod_node_id": prod_node_id,
                    "cons_count": cons_count,
                    "frame_seq": frame_seq,
                    "pts_ns": pts_ns,
                    "prod_name": prod_raw.split(b"\x00", 1)[0].decode("utf-8", errors="replace"),
                    "format_name": fmt_raw.split(b"\x00", 1)[0].decode("utf-8", errors="replace"),
                    "target_max_w": target_max_w,
                    "target_max_h": target_max_h,
                    "target_fps": target_fps,
                    "actual_fps_x100": actual_fps_x100,
                    "actual_fps": actual_fps_x100 / 100.0,
                    "cast_fps_cap": cast_fps_cap,
                    "pixels": pixels,
                }
            except (OSError, ValueError, struct.error):
                self.close()
                return None
        return None


class DryRunDesktopCapturer:
    """Silent desktop snapshot capturer in private runtime dir for dry-run preview mode."""

    def __init__(self, interval_sec: float = 0.35) -> None:
        self.interval_sec = interval_sec
        self._tmp_png: Optional[str] = None
        self._lock = threading.Lock()
        self._stop_ev = threading.Event()
        self._wake_ev = threading.Event()
        self._thread: Optional[threading.Thread] = None
        self._png_bytes: Optional[bytes] = None
        self._seq = 0
        self._status = "stopped"
        self._cap_fps = 0.0
        self._win_t0 = 0.0
        self._win_cnt = 0

    def set_target_fps(self, fps: int) -> None:
        if fps <= 0:
            new_iv = 0.25
        elif fps <= 15:
            new_iv = 0.55
        elif fps <= 30:
            new_iv = 0.35
        else:
            new_iv = 0.25
        with self._lock:
            changed = abs(self.interval_sec - new_iv) > 1e-3
            self.interval_sec = new_iv
        if changed:
            self._wake_ev.set()

    def start(self) -> None:
        if self._thread and self._thread.is_alive():
            return
        self._stop_ev.clear()
        self._wake_ev.clear()
        with self._lock:
            self._win_t0 = 0.0
            self._win_cnt = 0
            self._cap_fps = 0.0
        self._thread = threading.Thread(target=self._run, name="SalsaDryRunCap", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop_ev.set()
        self._wake_ev.set()
        if self._thread and self._thread.is_alive():
            self._thread.join(timeout=1.0)
        self._thread = None
        self._cleanup_tmp()
        with self._lock:
            self._status = "stopped"
            self._cap_fps = 0.0

    def _cleanup_tmp(self) -> None:
        try:
            if self._tmp_png and os.path.exists(self._tmp_png):
                os.unlink(self._tmp_png)
        except OSError:
            pass
        self._tmp_png = None

    def get_latest(self) -> tuple[int, Optional[bytes], str]:
        with self._lock:
            return self._seq, self._png_bytes, self._status

    def get_capture_fps(self) -> float:
        with self._lock:
            return self._cap_fps

    def _run(self) -> None:
        has_spectacle = bool(shutil.which("spectacle"))
        has_grim = bool(shutil.which("grim"))
        if not has_spectacle and not has_grim:
            with self._lock:
                self._status = "No spectacle/grim available for silent desktop mirror"
            return

        while not self._stop_ev.is_set():
            ok = False
            msg = ""
            try:
                self._cleanup_tmp()
                fd, tmp_path = tempfile.mkstemp(prefix="dryrun_cap_", suffix=".png", dir=PRIVATE_RUNTIME_DIR)
                os.close(fd)
                self._tmp_png = tmp_path
                if has_spectacle:
                    cmd = ["spectacle", "-b", "-n", "-m", "-o", self._tmp_png]
                    res = subprocess.run(cmd, capture_output=True, text=True, timeout=3.0, check=False)
                else:
                    res = subprocess.run(["grim", self._tmp_png], capture_output=True, text=True, timeout=3.0, check=False)
                if res.returncode == 0 and self._tmp_png and os.path.exists(self._tmp_png):
                    with open(self._tmp_png, "rb") as f:
                        data = f.read()
                    if data:
                        now = time.monotonic()
                        with self._lock:
                            self._png_bytes = data
                            self._seq += 1
                            self._status = "live_mirror"
                            if self._win_t0 <= 0.0:
                                self._win_t0 = now
                                self._win_cnt = 1
                            else:
                                self._win_cnt += 1
                                dt = now - self._win_t0
                                if dt >= 1.0:
                                    self._cap_fps = (self._win_cnt - 1) / dt
                                    self._win_t0 = now
                                    self._win_cnt = 1
                        ok = True
                if not ok:
                    err_txt = (res.stderr or res.stdout or f"rc={res.returncode}").strip()
                    msg = f"compositor capture unavailable ({err_txt[:90]})"
            except (OSError, subprocess.SubprocessError) as exc:
                msg = f"capture error ({exc})"
            finally:
                self._cleanup_tmp()

            if not ok and msg:
                with self._lock:
                    self._status = msg
            with self._lock:
                iv = self.interval_sec
            self._wake_ev.wait(iv)
            self._wake_ev.clear()

