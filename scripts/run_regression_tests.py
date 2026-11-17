#!/usr/bin/env python3
"""Automated regression, ABI parity, and security test suite for Salsa (LWASAPI)."""

import argparse
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time

REPO_DIR = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
BUILD_DIR = os.path.join(REPO_DIR, "build")


def run_cmd(cmd: list[str], env=None, timeout: int = 30) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=REPO_DIR, env=env, capture_output=True, text=True, timeout=timeout)


def find_host_libpulse() -> str:
    try:
        out = subprocess.check_output(["ldconfig", "-p"], text=True)
        for line in out.splitlines():
            if "libpulse.so.0 " in line and "lib32" not in line and "=>" in line:
                cand = line.split("=>", 1)[1].strip()
                if os.path.exists(cand):
                    return cand
    except Exception:
        pass
    for p in (
        "/usr/lib/libpulse.so.0",
        "/usr/lib64/libpulse.so.0",
        "/usr/lib/x86_64-linux-gnu/libpulse.so.0",
        "/usr/lib/aarch64-linux-gnu/libpulse.so.0",
    ):
        if os.path.exists(p):
            return p
    return ""


def export_syms(so_path: str) -> set[str]:
    out = subprocess.check_output(["nm", "-D", "--defined-only", so_path], text=True)
    syms = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 3 and parts[1] in ("T", "W"):
            name = parts[2].split("@")[0]
            if name and not name.startswith("_"):
                syms.add(name)
    return syms


def test_c_unit_suite() -> None:
    if is_audiodg_active():
        ctl_bin = os.path.join(BUILD_DIR, "lwasapi-ctl")
        for _ in range(30):
            if run_cmd([ctl_bin, "status"], timeout=5).returncode == 0:
                break
            time.sleep(0.1)
    bin_path = os.path.join(BUILD_DIR, "lwasapi-test")
    assert os.path.exists(bin_path), f"Missing {bin_path}"
    r = run_cmd([bin_path], timeout=30)
    if r.returncode != 0:
        raise AssertionError(f"lwasapi-test failed (rc={r.returncode}):\n{r.stdout}\n{r.stderr}")
    print("[PASS] C unit suite (build/lwasapi-test)")


def test_python_compile_and_settings_security() -> None:
    for pyfile in (
        "salsa_ipc.py",
        "salsa-gui.py",
        "salsa-debug.py",
        "salsa_debug_core.py",
        "salsa_debug_ui.py",
        "lwasapi-tray.py",
        "scripts/gen_pa_proxy.py",
    ):
        r = run_cmd([sys.executable, "-m", "py_compile", os.path.join(REPO_DIR, pyfile)])
        if r.returncode != 0:
            raise AssertionError(f"py_compile failed for {pyfile}: {r.stderr}")

    sys.path.insert(0, os.path.join(REPO_DIR, "scripts"))
    import gen_pa_proxy  # noqa: E402

    for bad in ("pa_bad;id", "pa_evil\njmp", "../etc/passwd", "_private"):
        try:
            gen_pa_proxy._validate_stubs([bad])
            raise AssertionError(f"gen_pa_proxy accepted unsafe identifier: {bad!r}")
        except ValueError:
            pass

    # Verify salsa-gui.py secure settings atomic 0o600/0o700 write and symlink safety
    with tempfile.TemporaryDirectory() as tmpdir:
        cfg_root = os.path.join(tmpdir, "cfg")
        env = dict(os.environ, XDG_CONFIG_HOME=cfg_root)
        code = """
import importlib.util, os, stat
spec = importlib.util.spec_from_file_location("sg", "salsa-gui.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
mod.save_gui_settings({"eq_per_device": True, "eq_device_profiles": {"dev0": {"enabled": True}}})
p = mod._gui_settings_path()
d = os.path.dirname(p)
assert (os.stat(d).st_mode & 0o777) == 0o700, oct(os.stat(d).st_mode)
assert (os.stat(p).st_mode & 0o777) == 0o600, oct(os.stat(p).st_mode)
loaded = mod.load_gui_settings()
assert loaded.get("eq_per_device") is True
os.unlink(p)
os.symlink("/etc/passwd", p)
assert mod.load_gui_settings() == {}
"""
        r = run_cmd([sys.executable, "-c", code], env=env, timeout=15)
        if r.returncode != 0:
            raise AssertionError(f"Settings security test failed: {r.stderr}\n{r.stdout}")
    print("[PASS] Python compilation, proxy identifier regex guard & 0600/0700 settings security")


def test_elf_hardening_and_proxy_parity() -> None:
    audiodg = os.path.join(BUILD_DIR, "lwasapi-audiodg")
    proxy_so = os.path.join(BUILD_DIR, "pa_reconnect", "libpulse.so.0")
    for binp in (audiodg, proxy_so):
        assert os.path.exists(binp), f"Missing {binp}"
        dyn = subprocess.check_output(["readelf", "-d", binp], text=True)
        if "BIND_NOW" not in dyn:
            raise AssertionError(f"{binp} missing Full RELRO (BIND_NOW)")
    proxy_dyn = subprocess.check_output(["readelf", "-d", proxy_so], text=True)
    if "NODELETE" not in proxy_dyn:
        raise AssertionError(f"{proxy_so} missing DF_1_NODELETE (-Wl,-z,nodelete)")

    host_lp = find_host_libpulse()
    assert host_lp, "Could not locate host libpulse.so.0"
    missing = export_syms(host_lp) - export_syms(proxy_so)
    if missing:
        raise AssertionError(f"Proxy missing {len(missing)} host symbols: {sorted(missing)[:10]}")

    # Non-recursive dlopen/dlsym verification under LD_LIBRARY_PATH=build/pa_reconnect
    probe_c = r"""
#include <stdio.h>
#include <pulse/pulseaudio.h>
int main(void) {
    const char *v = pa_get_library_version();
    if (!v || !v[0]) return 2;
    pa_mainloop *ml = pa_mainloop_new();
    if (!ml) return 3;
    pa_context *ctx = pa_context_new(pa_mainloop_get_api(ml), "SalsaProbe");
    if (!ctx) return 4;
    pa_context_unref(ctx);
    pa_mainloop_free(ml);
    printf("VER=%s\n", v);
    return 0;
}
"""
    with tempfile.TemporaryDirectory() as d:
        src = os.path.join(d, "probe.c")
        exe = os.path.join(d, "probe")
        with open(src, "w", encoding="utf-8") as f:
            f.write(probe_c)
        c_res = run_cmd(["gcc", "-O2", src, "-lpulse", "-o", exe])
        if c_res.returncode != 0:
            raise AssertionError(f"Failed compiling PA proxy probe: {c_res.stderr}")
        env = dict(os.environ, LD_LIBRARY_PATH=os.path.join(BUILD_DIR, "pa_reconnect"))
        r = run_cmd([exe], env=env, timeout=10)
        if r.returncode != 0 or "VER=" not in r.stdout:
            raise AssertionError(f"Proxy runtime probe failed (rc={r.returncode}): {r.stdout} {r.stderr}")
    print("[PASS] ELF hardening (BIND_NOW/NODELETE), 100% host symbol parity & non-recursive proxy probe")


def is_audiodg_active() -> bool:
    r = run_cmd(["systemctl", "--user", "is-active", "lwasapi-audiodg.service"])
    return r.returncode == 0 and r.stdout.strip() == "active"


def test_live_daemon_security_and_pw_broker() -> None:
    r = run_cmd([os.path.join(BUILD_DIR, "lwasapi-ctl"), "status"], timeout=10)
    if r.returncode != 0:
        raise AssertionError(f"lwasapi-ctl status failed: {r.stderr}\n{r.stdout}")

    cfg_dir = os.path.join(os.path.expanduser("~"), ".config", "lwasapi")
    if os.path.isdir(cfg_dir):
        mode = stat.S_IMODE(os.stat(cfg_dir).st_mode)
        if mode != 0o700:
            raise AssertionError(f"{cfg_dir} mode is {oct(mode)}, expected 0o700")
    for fname in ("app_volumes.conf", "gui_settings.json"):
        fpath = os.path.join(cfg_dir, fname)
        if os.path.lexists(fpath):
            assert not os.path.islink(fpath), f"{fpath} must not be a symlink"
            mode = stat.S_IMODE(os.stat(fpath).st_mode)
            if mode != 0o600:
                raise AssertionError(f"{fpath} mode is {oct(mode)}, expected 0o600")

    rt_base = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    rt_lwasapi = os.path.join(rt_base, "lwasapi")
    if os.path.isdir(rt_lwasapi):
        mode = stat.S_IMODE(os.stat(rt_lwasapi).st_mode)
        if mode != 0o700:
            raise AssertionError(f"{rt_lwasapi} mode is {oct(mode)}, expected 0o700")

    pw_sock = os.path.join(rt_base, "pipewire-0")
    if not os.path.exists(pw_sock):
        raise AssertionError(f"ScreenCast video broker socket missing at {pw_sock}")
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(3.0)
    try:
        sock.connect(pw_sock)
    finally:
        sock.close()
    print("[PASS] Live daemon status, 0700/0600 file permissions & pipewire-0 video broker socket health")


def test_live_session_control_reconnect_and_app_volumes() -> None:
    app_vol_path = os.path.join(os.path.expanduser("~"), ".config", "lwasapi", "app_volumes.conf")
    had_backup = False
    backup_bytes = b""
    if os.path.exists(app_vol_path) and not os.path.islink(app_vol_path):
        with open(app_vol_path, "rb") as f:
            backup_bytes = f.read()
        had_backup = True

    client_c = r"""
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pulse/pulseaudio.h>

static volatile int g_ready = 0;
static volatile int g_stop = 0;

static void ctx_cb(pa_context *c, void *u) {
    (void)u;
    pa_context_state_t st = pa_context_get_state(c);
    if (st == PA_CONTEXT_READY) g_ready = 1;
}

static void write_cb(pa_stream *s, size_t nbytes, void *u) {
    (void)u;
    static char zeros[4096];
    while (nbytes > 0) {
        size_t chunk = nbytes > sizeof(zeros) ? sizeof(zeros) : nbytes;
        if (pa_stream_write(s, zeros, chunk, NULL, 0, PA_SEEK_RELATIVE) < 0) break;
        nbytes -= chunk;
    }
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    pa_threaded_mainloop *ml = pa_threaded_mainloop_new();
    pa_mainloop_api *api = pa_threaded_mainloop_get_api(ml);
    pa_proplist *props = pa_proplist_new();
    pa_proplist_sets(props, PA_PROP_APPLICATION_NAME, "SalsaRegressionTest");
    pa_proplist_sets(props, PA_PROP_APPLICATION_PROCESS_BINARY, "salsa_regtest_bin");

    pa_context *ctx = pa_context_new_with_proplist(api, "SalsaRegressionTest", props);
    pa_proplist_free(props);
    pa_context_set_state_callback(ctx, ctx_cb, NULL);
    pa_threaded_mainloop_start(ml);

    pa_threaded_mainloop_lock(ml);
    if (pa_context_connect(ctx, NULL, PA_CONTEXT_NOFLAGS, NULL) < 0) return 2;
    for (int i = 0; i < 100 && !g_ready; i++) {
        pa_threaded_mainloop_unlock(ml);
        usleep(50000);
        pa_threaded_mainloop_lock(ml);
    }
    if (!g_ready) {
        pa_threaded_mainloop_unlock(ml);
        return 3;
    }

    pa_sample_spec ss = { .format = PA_SAMPLE_S16LE, .rate = 48000, .channels = 2 };
    pa_stream *st = pa_stream_new(ctx, "SalsaRegTestStream", &ss, NULL);
    pa_stream_set_write_callback(st, write_cb, NULL);
    if (pa_stream_connect_playback(st, NULL, NULL, PA_STREAM_AUTO_TIMING_UPDATE, NULL, NULL) < 0) {
        pa_threaded_mainloop_unlock(ml);
        return 4;
    }
    pa_threaded_mainloop_unlock(ml);

    const char *stop_path = (argc > 1) ? argv[1] : "";
    for (int i = 0; i < 80; i++) {
        usleep(100000);
        pa_usec_t usec = 0;
        pa_threaded_mainloop_lock(ml);
        int rc = pa_stream_get_time(st, &usec);
        pa_stream_state_t sst = pa_stream_get_state(st);
        pa_threaded_mainloop_unlock(ml);
        printf("TICK state=%d rc=%d time=%llu\n", (int)sst, rc, (unsigned long long)usec);
        if (stop_path[0] && access(stop_path, F_OK) == 0) break;
    }

    pa_threaded_mainloop_lock(ml);
    pa_stream_disconnect(st);
    pa_stream_unref(st);
    pa_context_disconnect(ctx);
    pa_context_unref(ctx);
    pa_threaded_mainloop_unlock(ml);
    pa_threaded_mainloop_stop(ml);
    pa_threaded_mainloop_free(ml);
    return 0;
}
"""
    proc = None
    with tempfile.TemporaryDirectory() as d:
        stop_flag = os.path.join(d, "stop")
        try:
            src = os.path.join(d, "client.c")
            exe = os.path.join(d, "client")
            with open(src, "w", encoding="utf-8") as f:
                f.write(client_c)
            cr = run_cmd(["gcc", "-O2", src, "-lpulse", "-o", exe])
            if cr.returncode != 0:
                raise AssertionError(f"Failed compiling PA reconnect client: {cr.stderr}")

            env = dict(os.environ, LD_LIBRARY_PATH=f"{os.path.join(BUILD_DIR, 'pa_reconnect')}:{BUILD_DIR}")
            proc = subprocess.Popen([exe, stop_flag], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)

            # Import LwasapiEngineStatus cleanly from salsa-gui module path
            sys.path.insert(0, REPO_DIR)
            import ctypes
            lib = ctypes.CDLL(os.path.join(BUILD_DIR, "liblwasapi.so"))
            import importlib.util
            spec = importlib.util.spec_from_file_location("salsa_gui_mod", os.path.join(REPO_DIR, "salsa-gui.py"))
            sg = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(sg)

            lib.LWASAPI_GetEngineStatus.argtypes = [ctypes.POINTER(sg.LwasapiEngineStatus)]
            lib.LWASAPI_GetEngineStatus.restype = ctypes.c_int32
            lib.LWASAPI_SetSessionControl.argtypes = [ctypes.c_uint32, ctypes.c_int32, ctypes.c_int32, ctypes.c_char_p]
            lib.LWASAPI_SetSessionControl.restype = ctypes.c_int32

            def find_test_session():
                st = sg.LwasapiEngineStatus()
                if lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) != 0:
                    return None, st
                for i in range(min(st.num_sessions, sg.LWASAPI_MAX_SESSIONS)):
                    s = st.sessions[i]
                    nm = s.client_name.decode("utf-8", "ignore")
                    if "SalsaRegressionTest" in nm or "SalsaRegTestStream" in nm:
                        return s, st
                return None, st

            sess = None
            st = None
            for _ in range(40):
                sess, st = find_test_session()
                if sess is not None:
                    break
                time.sleep(0.1)
            if sess is None:
                raise AssertionError("Test PA stream never appeared in engine status")

            sid = int(sess.session_id)
            active_dev = st.active_playback_id.decode("utf-8", "ignore").strip() or "auto"

            # Exercise SetSessionControl: volume 73%, mute=1
            assert lib.LWASAPI_SetSessionControl(sid, 73, 1, None) == 0
            sess2, _ = find_test_session()
            assert sess2 is not None and sess2.volume_milli == 730 and sess2.muted == 1, (
                f"Expected vol=730 muted=1, got vol={sess2.volume_milli if sess2 else None} muted={sess2.muted if sess2 else None}"
            )

            # Pin to active_dev then reset to auto, and leave volume at 88% unmuted
            if active_dev != "auto":
                assert lib.LWASAPI_SetSessionControl(sid, 88, 0, active_dev.encode("utf-8")) == 0
                sess3, _ = find_test_session()
                assert sess3 is not None and (sess3.routing_flags & 1) == 1
                assert sess3.target_device.decode("utf-8", "ignore") == active_dev
                assert lib.LWASAPI_SetSessionControl(sid, -1, -1, b"auto") == 0
                sess4, _ = find_test_session()
                assert sess4 is not None and (sess4.routing_flags & 1) == 0
            else:
                assert lib.LWASAPI_SetSessionControl(sid, 88, 0, None) == 0

            # Verify app_volumes.conf permissions
            if not os.path.exists(app_vol_path) or stat.S_IMODE(os.stat(app_vol_path).st_mode) != 0o600:
                raise AssertionError(f"Expected {app_vol_path} to exist with 0600 permissions")

            # Restart lwasapi-audiodg.service and verify monotonic stream time + volume persistence
            rr = run_cmd(["systemctl", "--user", "restart", "lwasapi-audiodg.service"], timeout=20)
            if rr.returncode != 0:
                raise AssertionError(f"Failed restarting lwasapi-audiodg.service: {rr.stderr}")

            reconnected = None
            for _ in range(50):
                reconnected, _ = find_test_session()
                if reconnected is not None:
                    break
                time.sleep(0.1)
            if reconnected is None:
                raise AssertionError("PA client did not auto-reconnect after lwasapi-audiodg restart")
            assert reconnected.volume_milli == 880 and reconnected.muted == 0, (
                f"Persisted app_volumes.conf failed across restart: vol={reconnected.volume_milli} muted={reconnected.muted}"
            )

            time.sleep(0.6)
            open(stop_flag, "w").close()
            out, err = proc.communicate(timeout=10)
            assert proc.returncode == 0, f"PA client exited {proc.returncode}: {err}\n{out}"

            times = [int(m.group(1)) for m in re.finditer(r"time=(\d+)", out)]
            assert len(times) >= 5, f"Insufficient PA timing samples: {out}"
            for i in range(1, len(times)):
                if times[i] < times[i - 1]:
                    raise AssertionError(f"Non-monotonic pa_stream_get_time across restart: {times[i-1]} -> {times[i]}")
            assert times[-1] > times[0], f"Stream time failed to advance across session: {times[0]} -> {times[-1]}"
        finally:
            if proc is not None and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=3)
                except Exception:
                    proc.kill()
            if os.path.exists(stop_flag):
                os.unlink(stop_flag)
            # Restore user's original app_volumes.conf cleanly
            if had_backup:
                tmp_restore = f"{app_vol_path}.restore.{os.getpid()}"
                fd = os.open(tmp_restore, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
                with os.fdopen(fd, "wb") as f:
                    f.write(backup_bytes)
                os.replace(tmp_restore, app_vol_path)
                if is_audiodg_active():
                    run_cmd(["systemctl", "--user", "restart", "lwasapi-audiodg.service"], timeout=15)
            elif os.path.exists(app_vol_path) and "SalsaReg" in open(app_vol_path, "r", encoding="utf-8", errors="ignore").read():
                lines = [ln for ln in open(app_vol_path, "r", encoding="utf-8", errors="ignore") if "salsa_regtest_bin" not in ln and "SalsaReg" not in ln]
                tmp_restore = f"{app_vol_path}.restore.{os.getpid()}"
                fd = os.open(tmp_restore, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
                with os.fdopen(fd, "w", encoding="utf-8") as f:
                    f.writelines(lines)
                os.replace(tmp_restore, app_vol_path)

    print("[PASS] Live session control, app_volumes.conf 0600 persistence & monotonic pa_stream_get_time() across restart")


def test_rev6_tray_and_steam_reconnect() -> None:
    # 1. install-salsa.sh syntax check
    sh_chk = run_cmd(["bash", "-n", os.path.join(REPO_DIR, "install-salsa.sh")], timeout=10)
    if sh_chk.returncode != 0:
        raise AssertionError(f"install-salsa.sh syntax error: {sh_chk.stderr}")

    # 2. Isolated offscreen PyQt6 tray, glow icon bank, SwipeFader, OSD, Quick Mixer & 0600 settings checks
    tray_script = os.path.join(REPO_DIR, "lwasapi-tray.py")
    probe = r'''
import ctypes, importlib.util, os, sys, tempfile
os.environ["QT_QPA_PLATFORM"] = "offscreen"
tray_path = sys.argv[1]
spec = importlib.util.spec_from_file_location("lwasapi_tray", tray_path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

assert ctypes.sizeof(mod.LwasapiEqConfig) == 136
assert ctypes.sizeof(mod.LwasapiDeviceEntry) == 188
assert ctypes.sizeof(mod.LwasapiDeviceList) == 6020
assert ctypes.sizeof(mod.LwasapiSessionInfo) == 184
assert ctypes.sizeof(mod.LwasapiParamEqExt) == 96
assert ctypes.sizeof(mod.LwasapiMicApoCfg) == 48
assert ctypes.sizeof(mod.LwasapiBtCodecCfg) == 100
assert ctypes.sizeof(mod.LwasapiHwAnalogCfg) == 24
assert ctypes.sizeof(mod.LwasapiHwAnalogInfo) == 248
assert ctypes.sizeof(mod.LwasapiEngineStatus) == 7136
assert mod.LwasapiEngineStatus.output_peak_l.offset == 480

from PyQt6 import QtCore, QtGui, QtTest, QtWidgets
app = QtWidgets.QApplication.instance() or QtWidgets.QApplication(sys.argv)

with tempfile.TemporaryDirectory(prefix="salsa_rev6_rt_") as tmp_rt, tempfile.TemporaryDirectory(prefix="salsa_rev6_cfg_") as tmp_cfg:
    os.chmod(tmp_rt, 0o700)
    os.chmod(tmp_cfg, 0o700)
    os.environ["XDG_RUNTIME_DIR"] = tmp_rt
    os.environ["XDG_CONFIG_HOME"] = tmp_cfg

    bank = mod.TrayIconBank()
    base_rt = os.path.join(tmp_rt, "lwasapi")
    assert (os.stat(base_rt).st_mode & 0o777) == 0o700
    user_hi = os.path.expanduser("~/.local/share/icons/hicolor")
    for name in ("salsa-tray-active-glow", "salsa-tray-active-plain", "salsa-tray-muted", "salsa-tray-stopped"):
        for sz in (32, 48, 64, 128):
            p = os.path.join(bank.theme_dir, "hicolor", f"{sz}x{sz}", "apps", f"{name}.png")
            assert os.path.isfile(p) and (os.stat(p).st_mode & 0o777) == 0o600
            hp = os.path.join(user_hi, f"{sz}x{sz}", "apps", f"{name}.png")
            assert os.path.isfile(hp) and (os.stat(hp).st_mode & 0o777) == 0o644
    g64 = QtGui.QImage(os.path.join(bank.theme_dir, "hicolor", "64x64", "apps", "salsa-tray-active-glow.png"))
    p64 = QtGui.QImage(os.path.join(bank.theme_dir, "hicolor", "64x64", "apps", "salsa-tray-active-plain.png"))
    diff = sum(1 for y in range(64) for x in range(64) if g64.pixel(x, y) != p64.pixel(x, y))
    assert diff > 200, f"glow halo pixel delta too small: {diff}"

    mod.update_gui_settings({"theme": "Midnight Cyan", "unrelated_key": 77})
    mod.update_gui_settings({"tray_green_glow": False, "tray_left_click_popover": False})
    s = mod.load_gui_settings()
    assert s["theme"] == "Midnight Cyan" and s["unrelated_key"] == 77
    assert s["tray_green_glow"] is False and s["tray_left_click_popover"] is False
    assert (os.stat(mod._config_dir()).st_mode & 0o777) == 0o700
    assert (os.stat(mod._gui_settings_path()).st_mode & 0o777) == 0o600

    fader = mod.SwipeFader("Input Gain", accent="#10B981", max_val=150)
    fader.resize(240, 34)
    fader.show()
    vals, done = [], []
    fader.value_changed.connect(vals.append)
    fader.value_committed.connect(done.append)
    QtTest.QTest.mousePress(fader, QtCore.Qt.MouseButton.LeftButton, pos=QtCore.QPoint(100, 17))
    QtTest.QTest.mouseMove(fader, QtCore.QPoint(170, 17))
    QtTest.QTest.mouseRelease(fader, QtCore.Qt.MouseButton.LeftButton, pos=QtCore.QPoint(170, 17))
    assert len(vals) >= 2 and len(done) == 1
    prev = done[-1]
    fader.setFocus()
    ke = QtGui.QKeyEvent(QtCore.QEvent.Type.KeyPress, QtCore.Qt.Key.Key_Right, QtCore.Qt.KeyboardModifier.NoModifier)
    app.sendEvent(fader, ke)
    assert fader.value() == min(150, prev + 5), f"expected +5 key step, got {fader.value()} from {prev}"
    cur_v = fader.value()
    we = QtGui.QWheelEvent(
        QtCore.QPointF(100, 17), QtCore.QPointF(100, 17),
        QtCore.QPoint(0, 0), QtCore.QPoint(0, -120),
        QtCore.Qt.MouseButton.NoButton, QtCore.Qt.KeyboardModifier.NoModifier,
        QtCore.Qt.ScrollPhase.NoScrollPhase, False,
    )
    app.sendEvent(fader, we)
    assert fader.value() == max(0, cur_v - 5), f"expected -5 wheel step, got {fader.value()} from {cur_v}"

    assert "Plasma Breeze Emerald" in mod.THEMES
    osd = mod.SalsaOsdOverlay()
    assert osd.size() == QtCore.QSize(330, 66)
    osd.show_osd("Bose QC Ultra 2 HP", 79, False, is_mic=False)
    assert osd.isVisible()
    osd.show_osd("Mic Input Gain", 85, False, is_mic=True)
    assert osd.isVisible()
    osd.hide()

    resolver = mod.AppIconResolver()
    for pname in ("Firefox", "Discord", "UnknownCustomGame"):
        pm = resolver.get_pixmap(pname, 0, False, 20)
        assert not pm.isNull() and pm.width() == 20 and pm.height() == 20

    tray = mod.AudioEngineTray(app)
    sni_mo = tray.sni.metaObject()
    ci = {sni_mo.classInfo(i).name(): sni_mo.classInfo(i).value() for i in range(sni_mo.classInfoCount())}
    assert ci.get("D-Bus Interface") == "org.kde.StatusNotifierItem", f"missing D-Bus Interface classInfo: {ci}"

    scr = app.primaryScreen().availableGeometry()
    tray.popover.show_near(0, 0)
    app.processEvents()
    pos_a = tray.popover.pos()
    tray.popover.hide()
    tray.popover.show_near(0, 0)
    app.processEvents()
    pos_b = tray.popover.pos()
    assert pos_a == pos_b, f"repeated show_near(0,0) drifted: {pos_a} vs {pos_b}"
    tscr = tray.popover.screen().geometry() if tray.popover.screen() else scr
    assert tray.popover.y() + tray.popover.height() <= tscr.bottom() - 35, f"popover overlapped tray bottom: {tray.popover.geometry()}"
    b0 = tray.popover.y() + tray.popover.height()
    tray.popover.bt_frame.show()
    tray.popover._keep_bottom_anchored()
    app.processEvents()
    b1 = tray.popover.y() + tray.popover.height()
    assert abs(b1 - b0) <= 1, f"bottom edge drifted on resize: {b0} vs {b1}"
    tray.popover.hide()

    assert os.path.exists(mod.ICON_GLOW_PATH)
    for sz in (22, 24, 128):
        assert os.path.exists(os.path.join(tray.icon_bank.theme_dir, "hicolor", f"{sz}x{sz}", "apps", "salsa-tray-active-glow.png"))

    assert tray.popover.width() == 432, f"unexpected popup width: {tray.popover.width()}"
    click_x = scr.right() - 180
    tray.popover.show_near(click_x, scr.bottom() - 20)
    app.processEvents()
    x_first = tray.popover.x()
    pop_scr = tray.popover.screen() or app.primaryScreen()
    screens = QtWidgets.QApplication.screens()
    pidx = screens.index(pop_scr) if pop_scr in screens else None
    _, p_loc, p_clear, p_rpad = mod._plasma_tray_panel_info(pidx)
    pop_avail = pop_scr.availableGeometry()
    if pop_avail.width() < 200 or pop_avail.height() < 200:
        pop_avail = pop_scr.geometry()
    if p_loc in (3, 4):
        exp_x = max(pop_avail.left() + 8, min(pop_avail.right() + 1 - 432 - p_rpad, pop_avail.right() - 432 - 8))
        assert x_first == exp_x, f"not anchored to KDE systray: {x_first} vs {exp_x}"
    tray.popover.show_near(scr.right() - 40, scr.bottom() - 20)
    assert tray.popover._slide_anim.state() == QtCore.QAbstractAnimation.State.Running, "slide animation did not start on show_near"
    app.processEvents()
    assert tray.popover.x() == x_first, f"systray horizontal anchor shifted between clicks: {tray.popover.x()} vs {x_first}"
    if p_loc == 4:
        assert tray.popover.y() + tray.popover.height() == max(pop_avail.top() + 8 + tray.popover.height(), pop_avail.bottom() + 1 - p_clear), (
            f"popover not flush above panel: {tray.popover.geometry()} clear={p_clear}"
        )
    else:
        assert tray.popover.y() + tray.popover.height() <= scr.bottom() - 35, f"popover overlapped tray bottom: {tray.popover.geometry()}"
    if QtWidgets.QApplication.platformName() == "xcb" and shutil.which("xprop"):
        time.sleep(0.08)
        app.processEvents()
        xp = subprocess.check_output(["xprop", "-id", str(int(tray.popover.winId())), "_NET_WM_STATE"], text=True)
        assert "_NET_WM_STATE_SKIP_TASKBAR" in xp, f"missing _NET_WM_STATE_SKIP_TASKBAR: {xp}"

    assert mod.resolve_tray_theme({"theme": "CachyOS Teal Void"}) == mod.THEMES["CachyOS Teal Void"]
    assert mod.resolve_tray_theme({"theme": "CachyOS Teal Void", "tray_theme": "Plasma Breeze Emerald"}) == mod.THEMES["Plasma Breeze Emerald"]
    assert tray.popover.cmb_tray_theme.findText("Follow Studio Theme") >= 0
    assert tray.popover.cmb_tray_theme.findText("Plasma Breeze Emerald") >= 0
    assert len(getattr(tray, "_theme_actions", [])) >= len(mod.THEMES) + 1

    dsp_card = tray.popover.findChild(QtWidgets.QFrame, "SubCard")
    assert dsp_card is not None and dsp_card.height() >= 95, f"SubCard collapsed: {dsp_card.size() if dsp_card else None}"
    assert tray.popover.cmb_eq.height() >= 24, f"cmb_eq collapsed: {tray.popover.cmb_eq.size()}"
    assert tray.popover.cmb_cap_rate.height() >= 22 and tray.popover.cmb_cap_rate.findData(0) >= 0 and tray.popover.cmb_cap_rate.findData(48000) >= 0
    assert tray.popover.cmb_mic_preset_tray.height() >= 22 and tray.popover.cmb_mic_preset_tray.count() >= 4
    assert hasattr(tray.engine, "set_capture_rate") and (tray.engine._lib is None or hasattr(tray.engine._lib, "LWASAPI_SetCaptureSampleRate"))
    assert tray.popover.cmb_tray_theme.height() >= 24, f"cmb_tray_theme collapsed: {tray.popover.cmb_tray_theme.size()}"
    assert tray.popover.btn_midi_test.height() >= 22, f"btn_midi_test collapsed: {tray.popover.btn_midi_test.size()}"
    assert tray.popover.btn_vol_step.text() == "Step ±5%"
    tray.popover.btn_vol_step.click()
    assert tray.popover.btn_vol_step.text() == "Step ±10%" and tray.popover.fader_master._step_pct == 10
    assert mod.load_gui_settings().get("volume_step_pct") == 10
    for _c in range(3):
        tray.popover.btn_vol_step.click()
    assert tray.popover.btn_vol_step.text() == "Step ±5%" and mod.load_gui_settings().get("volume_step_pct") == 5
    fake_st = mod.LwasapiEngineStatus()
    fake_st.desktop_volume_pct = 50
    fake_st.desktop_mic_volume_pct = 80
    recorded_vols = []
    tray.engine.get_status = lambda: fake_st
    tray.engine.set_master_volume = lambda v, _m=-1: recorded_vols.append(("spk", v))
    tray.engine.set_mic_volume = lambda v, _m=-1: recorded_vols.append(("mic", v))
    tray._on_sni_scroll(120, "vertical")
    assert recorded_vols[-1] == ("spk", 55), f"expected 55 after +5% SNI scroll, got {recorded_vols}"

    assert tray.popover.pages.currentIndex() == 0
    tray.popover.btn_tab_apps.click()
    app.processEvents()
    assert tray.popover.pages.currentIndex() == 1
    tray.popover.btn_tab_devices.click()
    app.processEvents()
    assert tray.popover.pages.currentIndex() == 0

    start_pos = tray.popover.pos()
    grip_pt = tray.popover.drag_bar.mapTo(tray.popover, QtCore.QPoint(20, 10))
    ev_p = QtGui.QMouseEvent(QtCore.QEvent.Type.MouseButtonPress, QtCore.QPointF(grip_pt), QtCore.QPointF(start_pos + grip_pt), QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier)
    ev_m = QtGui.QMouseEvent(QtCore.QEvent.Type.MouseMove, QtCore.QPointF(grip_pt), QtCore.QPointF(start_pos + grip_pt + QtCore.QPoint(60, -40)), QtCore.Qt.MouseButton.NoButton, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier)
    ev_r = QtGui.QMouseEvent(QtCore.QEvent.Type.MouseButtonRelease, QtCore.QPointF(grip_pt), QtCore.QPointF(start_pos + grip_pt + QtCore.QPoint(60, -40)), QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.MouseButton.NoButton, QtCore.Qt.KeyboardModifier.NoModifier)
    tray.popover.mousePressEvent(ev_p)
    tray.popover.mouseMoveEvent(ev_m)
    tray.popover.mouseReleaseEvent(ev_r)
    assert tray.popover.pos() == start_pos, f"fixed popup unexpectedly moved: {tray.popover.pos()} vs {start_pos}"

    assert not tray.popover.btn_pin.isChecked()
    tray.popover.btn_pin.click()
    assert tray.popover.btn_pin.isChecked()
    tray.popover.btn_pin.click()
    assert not tray.popover.btn_pin.isChecked()
    tray.popover.hide()

    launched = []
    tray._launch_script = lambda p, *a: launched.append((p, a))
    tray.popover.btn_graph.click()
    assert launched and launched[-1][0].endswith("salsa-gui.py") and "--graph" in launched[-1][1], f"graph button launch broken: {launched}"

    mo_sni = tray.sni.metaObject()
    sni_sigs = [mo_sni.method(i).methodSignature().data() for i in range(mo_sni.methodCount())]
    assert b"ProvideXdgActivationToken(QString)" in sni_sigs, f"missing ProvideXdgActivationToken slot: {sni_sigs}"

    mo = tray.metaObject()
    sigs = [mo.method(i).methodSignature().data() for i in range(mo.methodCount())]
    assert b"sync_now()" in sigs
    QtCore.QMetaObject.invokeMethod(tray, "sync_now", QtCore.Qt.ConnectionType.QueuedConnection)
    app.processEvents()
    tray._poll_timer.stop()
    tray.popover._peak_timer.stop()
'''
    r_tray = subprocess.run([sys.executable, "-c", probe, tray_script], cwd=REPO_DIR, capture_output=True, text=True, timeout=25)
    if r_tray.returncode != 0:
        raise AssertionError(f"v6.0-beta tray test failed:\n{r_tray.stdout}\n{r_tray.stderr}")

    # 3. Multi-reset pa_threaded_mainloop tee/context/stream safety test under libsalsa_pa_reconnect.so
    so_path = os.path.join(BUILD_DIR, "libsalsa_pa_reconnect.so")
    if shutil.which("gcc") and os.path.exists(so_path):
        c_src = r'''
#include <pulse/pulseaudio.h>
#include <pthread.h>
#include <stdio.h>
#include <unistd.h>
static volatile int g_done = 0;
static void ctx_state_cb(pa_context *c, void *ud) { (void)c; (void)ud; }
static void sink_cb(pa_context *c, const pa_sink_info *i, int eol, void *ud) { (void)c; (void)i; (void)eol; (void)ud; }
static void srv_cb(pa_context *c, const pa_server_info *i, void *ud) { (void)c; (void)i; (void)ud; }
static void sub_cb(pa_context *c, pa_subscription_event_type_t t, uint32_t idx, void *ud) { (void)c; (void)t; (void)idx; (void)ud; }
int main(void) {
    pa_threaded_mainloop *ml = pa_threaded_mainloop_new();
    if (!ml) return 1;
    pa_mainloop_api *api = pa_threaded_mainloop_get_api(ml);
    pa_context *ctx = pa_context_new(api, "SalsaSteamRegTest");
    if (!ctx) return 2;
    pa_context_set_state_callback(ctx, ctx_state_cb, NULL);
    pa_context_set_subscribe_callback(ctx, sub_cb, NULL);
    pa_threaded_mainloop_start(ml);
    pa_threaded_mainloop_lock(ml);
    pa_context_connect(ctx, NULL, PA_CONTEXT_NOFAIL, NULL);
    for (int i = 0; i < 6; i++) {
        pa_operation *o1 = pa_context_get_sink_info_list(ctx, sink_cb, NULL);
        if (o1) pa_operation_unref(o1);
        pa_operation *o2 = pa_context_get_server_info(ctx, srv_cb, NULL);
        if (o2) pa_operation_unref(o2);
        pa_operation *o3 = pa_context_subscribe(ctx, PA_SUBSCRIPTION_MASK_ALL, NULL, NULL);
        if (o3) pa_operation_unref(o3);
        pa_threaded_mainloop_unlock(ml);
        usleep(15000);
        pa_threaded_mainloop_lock(ml);
    }
    pa_context_disconnect(ctx);
    pa_context_unref(ctx);
    pa_threaded_mainloop_unlock(ml);
    pa_threaded_mainloop_stop(ml);
    pa_threaded_mainloop_free(ml);
    return 0;
}
'''
        with tempfile.TemporaryDirectory(prefix="salsa_parec_test_") as td:
            src_f = os.path.join(td, "t.c")
            bin_f = os.path.join(td, "t")
            sock_p = os.path.join(td, "pa.sock")
            with open(src_f, "w", encoding="utf-8") as f:
                f.write(c_src)
            rc = subprocess.run(["gcc", "-O2", "-Wall", "-Werror", src_f, "-o", bin_f, "-lpulse", "-lpthread"],
                                capture_output=True, text=True, timeout=15)
            if rc.returncode == 0:
                env = os.environ.copy()
                env["LD_PRELOAD"] = so_path
                env["PULSE_SERVER"] = f"unix:{sock_p}"
                # Run client cleanly against absent/dropping server to exercise offline/reconnect paths
                rr = subprocess.run([bin_f], env=env, capture_output=True, text=True, timeout=15)
                if rr.returncode != 0:
                    raise AssertionError(f"pa_threaded_mainloop reconnect regression failed rc={rr.returncode}: {rr.stderr}")

    # 4. If salsa-steam-launch wrapper is currently installed, verify permissions & exports
    steam_wrap = os.path.expanduser("~/.local/bin/salsa-steam-launch")
    if os.path.exists(steam_wrap):
        assert not os.path.islink(steam_wrap), f"{steam_wrap} must not be a symlink"
        assert stat.S_IMODE(os.stat(steam_wrap).st_mode) == 0o700, f"{steam_wrap} must have 0700 mode"
        body = open(steam_wrap, "r", encoding="utf-8").read()
        assert "LD_PRELOAD" in body and "libsalsa_pa_reconnect.so" in body

    print("[PASS] v6.0-beta KDE tray (glow icon, SwipeFader, OSD, Quick Mixer, 0600 settings) & Steam reconnect checks")


def test_live_rev61_studio_apis() -> None:
    import ctypes
    import importlib.util

    spec = importlib.util.spec_from_file_location("salsa_gui_rev61", os.path.join(REPO_DIR, "salsa-gui.py"))
    sg = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sg)

    lib = ctypes.CDLL(os.path.join(BUILD_DIR, "liblwasapi.so"))
    lib.LWASAPI_GetEngineStatus.argtypes = [ctypes.POINTER(sg.LwasapiEngineStatus)]
    lib.LWASAPI_GetEngineStatus.restype = ctypes.c_int32
    lib.LWASAPI_SetEngineConfig.argtypes = [ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
    lib.LWASAPI_SetEngineConfig.restype = ctypes.c_int32
    lib.LWASAPI_SetParametricEQ.argtypes = [ctypes.POINTER(sg.LwasapiEqConfig), ctypes.POINTER(sg.LwasapiParamEqExt)]
    lib.LWASAPI_SetParametricEQ.restype = ctypes.c_int32
    lib.LWASAPI_SetMicAPO.argtypes = [ctypes.POINTER(sg.LwasapiMicApoCfg)]
    lib.LWASAPI_SetMicAPO.restype = ctypes.c_int32
    lib.LWASAPI_SetBtCodecConfig.argtypes = [ctypes.POINTER(sg.LwasapiBtCodecCfg)]
    lib.LWASAPI_SetBtCodecConfig.restype = ctypes.c_int32
    lib.LWASAPI_SetHwAnalog.argtypes = [ctypes.POINTER(sg.LwasapiHwAnalogCfg)]
    lib.LWASAPI_SetHwAnalog.restype = ctypes.c_int32
    lib.LWASAPI_SetCaptureSampleRate.argtypes = [ctypes.c_uint32]
    lib.LWASAPI_SetCaptureSampleRate.restype = ctypes.c_int32

    orig = sg.LwasapiEngineStatus()
    got_orig = False
    for _ in range(40):
        if lib.LWASAPI_GetEngineStatus(ctypes.byref(orig)) == 0:
            got_orig = True
            break
        time.sleep(0.1)
    assert got_orig, "Failed fetching initial v6.x-beta engine status"

    # Allow BlueZ A2DP route/rate to finish settling after the prior test's restart
    orig_cap_rate = int(orig.hw_analog_info.reserved)
    orig_cap_pref = orig_cap_rate if (44100 <= orig_cap_rate <= 192000 and orig_cap_rate != int(orig.hw_sample_rate)) else 0
    dbg_r = run_cmd([sys.executable, os.path.join(REPO_DIR, "salsa-debug.py"), "--snapshot", "--json"], timeout=10)
    if dbg_r.returncode == 0 and dbg_r.stdout.strip():
        try:
            snap0 = json.loads(dbg_r.stdout).get("telemetry", {})
            bz = snap0.get("bluez", {})
            if (bz.get("endpoints_registered") or bz.get("device_path")) and not orig.bt_active:
                for _ in range(35):
                    time.sleep(0.1)
                    lib.LWASAPI_GetEngineStatus(ctypes.byref(orig))
                    if orig.bt_active:
                        break
                orig_cap_rate = int(orig.hw_analog_info.reserved)
                orig_cap_pref = orig_cap_rate if (44100 <= orig_cap_rate <= 192000 and orig_cap_rate != int(orig.hw_sample_rate)) else 0
                dbg2 = run_cmd([sys.executable, os.path.join(REPO_DIR, "salsa-debug.py"), "--snapshot", "--json"], timeout=10)
                if dbg2.returncode == 0 and dbg2.stdout.strip():
                    snap0 = json.loads(dbg2.stdout).get("telemetry", {})
            orig_cap_pref = int(snap0.get("capture", {}).get("pref_rate", orig_cap_pref))
        except Exception:
            pass

    try:
        # 1. Fixed Master Clock & Bit Depth Configuration
        assert lib.LWASAPI_SetEngineConfig(48000, int(orig.hw_period_frames), 16) == 0
        st = sg.LwasapiEngineStatus()
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
        assert st.hw_sample_rate >= 44100 and st.hw_format_bits in (16, 24, 32), (
            f"Invalid clock state: rate={st.hw_sample_rate} bits={st.hw_format_bits}"
        )

        # 2. Parametric EQ Low-Shelf + Bauer Crossfeed
        eq = sg.LwasapiEqConfig()
        ctypes.memmove(ctypes.byref(eq), ctypes.byref(orig.eq_cfg), ctypes.sizeof(eq))
        ext = sg.LwasapiParamEqExt()
        ctypes.memmove(ctypes.byref(ext), ctypes.byref(orig.param_eq), ctypes.sizeof(ext))
        ext.band_freq_hz[0] = 105.0
        ext.band_type[0] = 1  # Low-Shelf
        eq.band_q[0] = 0.71
        eq.band_gains_db[0] = 2.5
        eq.enabled = 1
        ext.crossfeed_enabled = 1
        ext.crossfeed_level = 0.35
        assert lib.LWASAPI_SetParametricEQ(ctypes.byref(eq), ctypes.byref(ext)) == 0
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
        assert st.param_eq.band_type[0] == 1 and abs(st.param_eq.band_freq_hz[0] - 105.0) < 0.1
        assert abs(st.eq_cfg.band_q[0] - 0.71) < 0.01
        assert st.param_eq.crossfeed_enabled == 1 and abs(st.param_eq.crossfeed_level - 0.35) < 0.02

        # 3. Capture Mic APO + RNNoise + Sidetone
        mic = sg.LwasapiMicApoCfg()
        ctypes.memmove(ctypes.byref(mic), ctypes.byref(orig.mic_apo), ctypes.sizeof(mic))
        mic.enabled = 1
        mic.rnnoise_enabled = 1
        mic.sidetone_enabled = 1
        mic.sidetone_pct = 25
        mic.hpf_hz = 90.0
        assert lib.LWASAPI_SetMicAPO(ctypes.byref(mic)) == 0
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
        assert st.mic_apo.enabled == 1 and st.mic_apo.rnnoise_enabled == 1
        assert st.mic_apo.sidetone_enabled == 1 and st.mic_apo.sidetone_pct == 25

        # 4. Wireless ABR & BT Codec Config + Pre-flight SEP protection (never drop link on unsupported SEP)
        bt = sg.LwasapiBtCodecCfg()
        ctypes.memmove(ctypes.byref(bt), ctypes.byref(orig.bt_codec_cfg), ctypes.sizeof(bt))
        bt.wireless_abr_enabled = 1
        assert lib.LWASAPI_SetBtCodecConfig(ctypes.byref(bt)) == 0
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
        assert st.bt_codec_cfg.wireless_abr_enabled == 1
        sup_mask = int(orig.bt_codec_cfg.supported_codecs_mask)
        if orig.bt_active and sup_mask != 0 and (sup_mask & 1) == 0:
            bt_rej = sg.LwasapiBtCodecCfg()
            ctypes.memmove(ctypes.byref(bt_rej), ctypes.byref(orig.bt_codec_cfg), ctypes.sizeof(bt_rej))
            bt_rej.preferred_codec = 1  # LWASAPI_BT_PREF_LDAC (unsupported on this peer)
            assert lib.LWASAPI_SetBtCodecConfig(ctypes.byref(bt_rej)) == 0
            assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
            assert st.bt_active == 1, "Unsupported BT codec preflight must never drop A2DP link"
            assert st.bt_codec_cfg.preferred_codec == orig.bt_codec_cfg.preferred_codec
            assert b"LDAC" in bytes(st.bt_codec_cfg.neg_reason)

        # 5. Tray & Studio clock override / topology / Krisp Mic APO parity checks
        with open(os.path.join(REPO_DIR, "salsa-gui.py"), "r", encoding="utf-8") as fh:
            gui_src = fh.read()
        with open(os.path.join(REPO_DIR, "lwasapi-tray.py"), "r", encoding="utf-8") as fh:
            tray_src = fh.read()
        with open(os.path.join(REPO_DIR, "salsa_ipc.py"), "r", encoding="utf-8") as fh:
            ipc_src = fh.read()
        with open(os.path.join(REPO_DIR, "src", "lwasapi_audiodg.c"), "r", encoding="utf-8") as fh:
            audiodg_src = fh.read()
        assert "self.lib.LWASAPI_SetEngineConfig(rate, period, bits)" in gui_src
        assert "self.ctrl.engine.set_engine_config(r_int, self._cur_period, self._cur_bits)" in tray_src
        assert "(44100, 48000, 88200, 96000, 176400, 192000)" in tray_src
        assert "self._rate_hold_until" in tray_src and "self._bt_hold_until" in tray_src
        assert "SalsaAutoGraphCanvas" in gui_src and "self.lbl_auto_badge" in gui_src
        assert "self.cmb_cap_rate" in gui_src and "self.cmb_mic_preset" in gui_src
        assert "self.intro_win" in gui_src and "self.adv_win" in gui_src and "DEFAULT_SHORTCUTS" in gui_src
        assert "StepVolumeSlider" in gui_src and "btn_toggle_pacman_hook" in gui_src and "chk_autosync_now" in gui_src
        assert "SalsaLiveDebuggerWidget" not in gui_src and "snapshot_once" in gui_src
        assert "self.cmb_mic_preset_tray" in tray_src
        assert "PRESETS" in ipc_src and "MIC_APO_PRESETS" in ipc_src and "normalize_vol_step_pct" in ipc_src
        assert "dry_ring" in audiodg_src and "vad_ring" in audiodg_src and "g_mic_demud_flt" in audiodg_src

        # Offscreen PyQt6 check for Intro <-> Studio <-> Advanced window split, Option C & Option 2
        ui_probe = r'''
import importlib.util, os, tempfile
from PyQt6.QtCore import Qt, QPoint, QPointF
from PyQt6.QtGui import QKeyEvent, QKeySequence, QWheelEvent
from PyQt6.QtWidgets import QApplication
with tempfile.TemporaryDirectory() as tmp:
    os.environ["XDG_CONFIG_HOME"] = tmp
    spec = importlib.util.spec_from_file_location("sg", "salsa-gui.py")
    sg = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sg)
    app = QApplication([])
    w = sg.SalsaStudioWindow(start_in_graph=False, start_in_debug=False)
    w.timer.stop()
    assert not hasattr(w, "tabs") and not hasattr(w, "debugger_tab")
    assert w.intro_win is not None and w.adv_win is not None
    assert len(w.shortcut_edits) == len(sg.DEFAULT_SHORTCUTS)
    assert w.cmb_vol_step.currentData() == 5 and w.sld_spk.singleStep() == 5
    w.sld_spk.blockSignals(True)
    w.sld_spk.setValue(50)
    w.sld_spk.blockSignals(False)
    app.sendEvent(w.sld_spk, QWheelEvent(QPointF(20, 10), QPointF(20, 10), QPoint(0, 0), QPoint(0, 120), Qt.MouseButton.NoButton, Qt.KeyboardModifier.NoModifier, Qt.ScrollPhase.NoScrollPhase, False))
    assert w.sld_spk.value() == 55, f"expected 55 after +5% wheel, got {w.sld_spk.value()}"
    app.sendEvent(w.sld_spk, QKeyEvent(QKeyEvent.Type.KeyPress, Qt.Key.Key_Left, Qt.KeyboardModifier.NoModifier))
    assert w.sld_spk.value() == 50, f"expected 50 after -5% key, got {w.sld_spk.value()}"
    idx10 = w.cmb_vol_step.findData(10)
    w.cmb_vol_step.setCurrentIndex(idx10)
    assert w._vol_step_pct == 10 and w.sld_mic.singleStep() == 10
    assert sg.load_gui_settings().get("volume_step_pct") == 10
    assert w._shortcut_row_labels["vol_up"].text() == "Speaker Volume +10%:"
    w.cmb_vol_step.setCurrentIndex(w.cmb_vol_step.findData(5))
    assert w._vol_step_pct == 5
    w._open_topology_intro()
    app.processEvents()
    assert w.intro_win.isVisible()
    w.chk_skip_intro.setChecked(True)
    app.processEvents()
    assert not w.chk_show_intro_startup.isChecked()
    assert sg.load_gui_settings().get("show_topology_intro") is False
    w._enter_studio_from_intro()
    app.processEvents()
    assert w.isVisible() and not w.intro_win.isVisible()
    w._open_advanced_options()
    app.processEvents()
    assert w.adv_win.isVisible()
    assert "● Pacman Hook:" in w.lbl_chip_pacman.text()
    w._is_pacman_hook_installed = lambda: True
    w._refresh_adv_integration_status()
    assert w.lbl_chip_pacman.text() == "● Pacman Hook: ACTIVE" and w.btn_toggle_pacman_hook.isChecked()
    w._is_pacman_hook_installed = lambda: False
    w._refresh_adv_integration_status()
    assert w.lbl_chip_pacman.text() == "● Pacman Hook: OFF" and not w.btn_toggle_pacman_hook.isChecked()
    w._on_shortcut_edited("toggle_eq", QKeySequence("Ctrl+M"))
    assert w._get_shortcut_map()["toggle_eq"] == "Ctrl+M"
    assert w._get_shortcut_map()["mute_spk"] == ""
    w._reset_default_shortcuts()
    assert w._get_shortcut_map()["mute_spk"] == "Ctrl+M"
    assert w._get_shortcut_map()["toggle_eq"] == "Ctrl+E"
    w.close()
'''
        r_ui = subprocess.run(
            [sys.executable, "-c", ui_probe],
            cwd=REPO_DIR,
            env=dict(os.environ, QT_QPA_PLATFORM="offscreen"),
            capture_output=True,
            text=True,
            timeout=20,
        )
        assert r_ui.returncode == 0, f"Multi-window & shortcut Qt probe failed:\n{r_ui.stdout}\n{r_ui.stderr}"

        # 6. Verify 0600 permissions on state & volume files
        for p in (
            os.path.expanduser("~/.config/lwasapi/lwasapi_state.bin"),
            os.path.expanduser("~/.config/lwasapi/app_volumes.conf"),
        ):
            if os.path.exists(p):
                assert not os.path.islink(p), f"{p} must not be a symlink"
                assert stat.S_IMODE(os.stat(p).st_mode) == 0o600, f"{p} must have 0600 mode"

        # 7. v6.2-beta/v6.3-beta Live Hardware Analog Architecture + Safe Defaults Verification
        hw_cfg = sg.LwasapiHwAnalogCfg()
        ctypes.memmove(ctypes.byref(hw_cfg), ctypes.byref(orig.hw_analog_cfg), ctypes.sizeof(hw_cfg))
        hw_cfg.analog_boost_db = 4.0
        hw_cfg.mic_hw_boost_db = 8.0
        assert lib.LWASAPI_SetHwAnalog(ctypes.byref(hw_cfg)) == 0
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
        assert abs(st.hw_analog_cfg.analog_boost_db - 4.0) < 0.05
        assert abs(st.hw_analog_cfg.mic_hw_boost_db - 8.0) < 0.05
        assert ctypes.sizeof(sg.LwasapiEngineStatus) == 7136

        # 8. Split Independent Capture Clock (48 kHz Mic vs High-Rate Master Sink) & Telemetry
        lib.LWASAPI_SetEngineConfig(192000, int(orig.hw_period_frames), int(orig.hw_format_bits))
        assert lib.LWASAPI_SetCaptureSampleRate(48000) == 0
        time.sleep(0.25)
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
        assert int(st.hw_analog_info.reserved) == 48000, f"Unexpected capture rate: {st.hw_analog_info.reserved}"
        if shutil.which("pactl"):
            pa_src = run_cmd(["pactl", "list", "sources", "short"], timeout=5)
            in_lines = [ln for ln in pa_src.stdout.splitlines() if "salsa_in." in ln]
            assert in_lines, f"No salsa_in.* source found: {pa_src.stdout}"
            for ln in in_lines:
                assert "48000Hz" in ln, f"PA capture source not at 48000Hz: {ln}"
            pa_snk = run_cmd(["pactl", "list", "sinks", "short"], timeout=5)
            out_lines = [ln for ln in pa_snk.stdout.splitlines() if "salsa_out." in ln]
            assert out_lines, f"No salsa_out.* sink found: {pa_snk.stdout}"
            for ln in out_lines:
                if int(st.hw_sample_rate) != 48000:
                    assert f"{int(st.hw_sample_rate)}Hz" in ln, f"PA playback sink mismatch: {ln}"
        dbg_cap = run_cmd([sys.executable, os.path.join(REPO_DIR, "salsa-debug.py"), "--snapshot", "--json"], timeout=10)
        assert dbg_cap.returncode == 0, f"salsa-debug --snapshot --json failed: {dbg_cap.stderr}"
        snap_cap = json.loads(dbg_cap.stdout).get("telemetry", {}).get("capture", {})
        assert int(snap_cap.get("pref_rate", -1)) == 48000, f"capture.pref_rate mismatch: {snap_cap}"
        for req_k in ("measured_rate", "kick_count", "is_usb", "duplex_locked"):
            assert req_k in snap_cap, f"Missing capture telemetry field {req_k}: {snap_cap}"

        # 9. MIDI Key-Press-Only Counter (0x90 vel>0 vs vel=0 Note-Off) & Mic APO Preset CLI
        ctl_bin = os.path.join(BUILD_DIR, "lwasapi-ctl")
        st_m0 = sg.LwasapiEngineStatus()
        st_m1 = sg.LwasapiEngineStatus()
        st_m2 = sg.LwasapiEngineStatus()
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st_m0)) == 0
        r_off = run_cmd([ctl_bin, "midi-send", "60", "0"], timeout=5)
        assert r_off.returncode == 0, f"midi-send 60 0 failed: {r_off.stderr}"
        time.sleep(0.08)
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st_m1)) == 0
        assert int(st_m1.midi_events_routed) == int(st_m0.midi_events_routed), (
            f"vel=0 Note-Off incremented key-press counter: {st_m0.midi_events_routed} -> {st_m1.midi_events_routed}"
        )
        r_on = run_cmd([ctl_bin, "midi-send", "60", "100"], timeout=5)
        assert r_on.returncode == 0, f"midi-send 60 100 failed: {r_on.stderr}"
        got_inc = False
        for _ in range(10):
            time.sleep(0.05)
            assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st_m2)) == 0
            if int(st_m2.midi_events_routed) == int(st_m0.midi_events_routed) + 1:
                got_inc = True
                break
        assert got_inc, f"vel=100 key-press did not increment counter: {st_m0.midi_events_routed} -> {st_m2.midi_events_routed}"

        r_mp = run_cmd([ctl_bin, "mic-preset", "discord"], timeout=5)
        assert r_mp.returncode == 0, f"mic-preset discord failed: {r_mp.stderr}"
        time.sleep(0.08)
        assert lib.LWASAPI_GetEngineStatus(ctypes.byref(st)) == 0
        assert st.mic_apo.enabled == 1 and st.mic_apo.rnnoise_enabled == 1

        print("[PASS] v6.x-beta live studio APIs (Fixed/Split Clock, Telemetry, MIDI Key Presses, Mic Presets, EQ/APO, 0600 state)")
    finally:
        lib.LWASAPI_SetHwAnalog(ctypes.byref(orig.hw_analog_cfg))
        lib.LWASAPI_SetParametricEQ(ctypes.byref(orig.eq_cfg), ctypes.byref(orig.param_eq))
        lib.LWASAPI_SetMicAPO(ctypes.byref(orig.mic_apo))
        lib.LWASAPI_SetBtCodecConfig(ctypes.byref(orig.bt_codec_cfg))
        lib.LWASAPI_SetEngineConfig(
            int(orig.hw_sample_rate),
            int(orig.hw_period_frames),
            int(orig.hw_format_bits),
        )
        lib.LWASAPI_SetCaptureSampleRate(orig_cap_pref)


def test_rev62_hw_analog_scoring_and_defaults() -> None:
    c_test = r'''
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include "lwasapi_hw_analog.h"
int main(void) {
    lwasapi_hw_analog_cfg_t cfg;
    lwasapi_hw_analog_default_cfg(&cfg);
    assert(cfg.hw_dac_unity_vol == 1);
    assert(cfg.hp_amp_tier == LWASAPI_HW_AMP_AUTO);
    assert(cfg.output_route == LWASAPI_HW_ROUTE_AUTO);
    assert(cfg.unlock_0db_rails == 0);
    assert(fabsf(cfg.analog_boost_db) < 1e-5f);
    assert(fabsf(cfg.mic_hw_boost_db) < 1e-5f);

    int sc_strix = lwasapi_hw_analog_score_card(99, "STRIX RAID DLX USB Audio");
    int sc_soar  = lwasapi_hw_analog_score_card(99, "ASUS STRIX SOAR Audio");
    int sc_scar  = lwasapi_hw_analog_score_card(99, "Scarlett 2i2 4th Gen");
    int sc_hdmi  = lwasapi_hw_analog_score_card(99, "HDA ATI HDMI");
    assert(sc_strix > sc_hdmi + 100);
    assert(sc_soar > sc_hdmi + 100);
    assert(sc_scar > sc_hdmi + 100);
    printf("SCORES strix=%d soar=%d scarlett=%d hdmi=%d\n", sc_strix, sc_soar, sc_scar, sc_hdmi);
    return 0;
}
'''
    with tempfile.TemporaryDirectory(prefix="salsa_hw_score_") as tmp_d:
        src_p = os.path.join(tmp_d, "hw_score_test.c")
        bin_p = os.path.join(tmp_d, "hw_score_test")
        with open(src_p, "w", encoding="utf-8") as fh:
            fh.write(c_test)
        cc = run_cmd([
            "gcc", "-O2", "-std=c11", f"-I{os.path.join(REPO_DIR, 'include')}",
            src_p,
            os.path.join(REPO_DIR, "src", "lwasapi_hw_analog.c"),
            os.path.join(REPO_DIR, "src", "lwasapi_debug.c"),
            os.path.join(REPO_DIR, "build", "libsalsa_wire.a"),
            "-o", bin_p, "-pthread", "-lm", "-ldl",
        ], timeout=20)
        if cc.returncode != 0:
            raise AssertionError(f"v6.2-beta scoring test compile failed: {cc.stderr}")
        run_r = run_cmd([bin_p], timeout=10)
        if run_r.returncode != 0:
            raise AssertionError(f"v6.2-beta scoring test failed: {run_r.stderr}")
    print("[PASS] v6.2-beta Hardware Analog safe defaults & Strix/Soar/Scarlett > HDMI card scoring")


def test_rust_wire_core_and_failsafe_reset() -> None:
    cargo_bin = shutil.which("cargo") or os.path.expanduser("~/.cargo/bin/cargo")
    if cargo_bin and os.path.exists(cargo_bin):
        env = dict(os.environ)
        env["PATH"] = f"{os.path.dirname(cargo_bin)}:{env.get('PATH', '')}"
        manifest = os.path.join(REPO_DIR, "rust", "salsa_net", "Cargo.toml")
        r_clippy = run_cmd(
            [cargo_bin, "clippy", "--manifest-path", manifest, "--offline", "--all-targets", "--", "-D", "warnings"],
            env=env,
            timeout=60,
        )
        assert r_clippy.returncode == 0, f"cargo clippy failed:\n{r_clippy.stdout}\n{r_clippy.stderr}"
        r_test = run_cmd(
            [cargo_bin, "test", "--manifest-path", manifest, "--offline"],
            env=env,
            timeout=60,
        )
        assert r_test.returncode == 0, f"cargo test failed:\n{r_test.stdout}\n{r_test.stderr}"
        print("[PASS] Rust salsa_net wire/PA-server/PW-broker clippy + offline tests passed")


def test_live_rust_wire_failsafe() -> None:
    sys.path.insert(0, REPO_DIR)
    from salsa_debug_core import SalsaDebugClient

    cli = SalsaDebugClient()
    cli.start()
    try:
        telem = None
        t0 = time.monotonic()
        while time.monotonic() - t0 < 3.0:
            snap = cli.get_snapshot()
            t = snap.get("telemetry") or {}
            if snap.get("connected") and isinstance(t.get("wire"), dict):
                telem = t
                break
            time.sleep(0.08)
        assert telem is not None, "Did not receive live 'wire' telemetry object over lwasapi_debug"
        wire0 = telem["wire"]
        assert wire0.get("armed") == 1, f"Network failsafe not armed: {wire0}"
        assert wire0.get("is_rust") == 1 and wire0.get("backend") == "rust", f"Expected active Rust wire core: {wire0}"

        trips_before = int(wire0.get("trips_total", 0))
        resets_before = int(wire0.get("manual_resets", 0))
        rep = cli.send_command_wait("RESET_NET_CLIENTS", timeout=2.5)
        assert rep and rep.get("rc", -1) == 0, f"RESET_NET_CLIENTS failed: {rep}"
        assert "audio engine uninterrupted" in rep.get("msg", ""), f"Unexpected reply: {rep}"

        cli.send_command("SNAPSHOT")
        wire1 = None
        t1 = time.monotonic()
        while time.monotonic() - t1 < 2.0:
            snap = cli.get_snapshot()
            w = (snap.get("telemetry") or {}).get("wire") or {}
            if int(w.get("manual_resets", 0)) >= resets_before + 1:
                wire1 = w
                break
            time.sleep(0.08)
        assert wire1 is not None, "manual_resets did not increment in live wire telemetry"
        assert int(wire1.get("trips_total", 0)) >= trips_before + 1
        print("[PASS] Live Rust Wire Core + Silent Failsafe armed + RESET_NET_CLIENTS verified")
    finally:
        cli.stop()


def main() -> int:
    parser = argparse.ArgumentParser(description="Run Salsa LWASAPI regression & security suite")
    parser.add_argument("--require-live", action="store_true", help="Fail if lwasapi-audiodg.service is not active")
    args = parser.parse_args()

    test_c_unit_suite()
    test_python_compile_and_settings_security()
    test_elf_hardening_and_proxy_parity()
    test_rev6_tray_and_steam_reconnect()
    test_rev62_hw_analog_scoring_and_defaults()
    test_rust_wire_core_and_failsafe_reset()

    if is_audiodg_active():
        test_live_daemon_security_and_pw_broker()
        test_live_session_control_reconnect_and_app_volumes()
        test_live_rev61_studio_apis()
        test_live_rust_wire_failsafe()
    elif args.require_live:
        raise AssertionError("lwasapi-audiodg.service is not active (--require-live specified)")
    else:
        print("[SKIP] Live daemon checks skipped because lwasapi-audiodg.service is not active")

    print("ALL SALSA REGRESSION TESTS PASSED.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
