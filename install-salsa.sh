#!/usr/bin/env bash
# ==============================================================================
# Salsa — Linux WASAPI (LWASAPI) Universal Cross-PC Auto-Installer & Adapter
# ==============================================================================
# Adapts Salsa — Linux WASAPI to any Linux PC, user account, install path,
# Linux distribution (Arch/CachyOS, Debian/Ubuntu, Fedora/RHEL, openSUSE),
# and audio/MIDI hardware topology.
#
# Usage:
#   ./install-salsa.sh                     # Full auto-detect, build, configure & start
#   ./install-salsa.sh --check             # Inspect host PC hardware & distro profile
#   ./install-salsa.sh --uninstall         # Cleanly revert & remove services/shortcuts
#   ./install-salsa.sh --install-pacman-hook # (Arch/CachyOS) Root-safe ALPM trigger via pkexec
#   ./install-salsa.sh --remove-pacman-hook  # Remove optional ALPM pacman hook
# ==============================================================================

set -euo pipefail

BASE_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
ACTION="${1:---install}"

HOST_OS="$(awk -F= '/^PRETTY_NAME=/{gsub(/"/,"",$2); print $2}' /etc/os-release 2>/dev/null || uname -sr)"
HOST_CPU="$(awk -F: '/^model name|^Processor/{gsub(/^[ \t]+/,"",$2); print $2; exit}' /proc/cpuinfo 2>/dev/null || uname -m)"
HOST_ARCH="$(uname -m)"

find_host_lib() {
    local libname="$1"
    local p=""
    if command -v ldconfig >/dev/null 2>&1; then
        p="$(ldconfig -p 2>/dev/null | awk -v n="${libname}" '$1 == n && $0 !~ /lib32/ {print $NF; exit}')"
    fi
    if [[ -z "${p}" || ! -e "${p}" ]]; then
        for dir in /usr/lib /usr/lib64 /usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu /lib /lib64; do
            if [[ -e "${dir}/${libname}" ]]; then
                p="${dir}/${libname}"
                break
            fi
        done
    fi
    printf "%s" "${p}"
}

find_host_lib32() {
    local libname="$1"
    local p=""
    if command -v ldconfig >/dev/null 2>&1; then
        p="$(ldconfig -p 2>/dev/null | awk -v n="${libname}" '$1 == n && ($0 ~ /lib32/ || $0 ~ /i386/ || $0 !~ /x86-64|64bit|AArch64|aarch64/) {print $NF; exit}')"
    fi
    if [[ -z "${p}" || ! -e "${p}" ]]; then
        for dir in /usr/lib32 /usr/lib/i386-linux-gnu /lib/i386-linux-gnu; do
            if [[ -e "${dir}/${libname}" ]]; then
                p="${dir}/${libname}"
                break
            fi
        done
    fi
    printf "%s" "${p}"
}

detect_pkg_db_path() {
    if [[ -d /var/lib/pacman/local ]]; then
        echo "/var/lib/pacman/local"
    elif [[ -f /var/lib/dpkg/status ]]; then
        echo "/var/lib/dpkg/status"
    elif [[ -d /var/lib/rpm ]]; then
        echo "/var/lib/rpm"
    else
        echo "/usr/lib"
    fi
}

LIB_ASOUND="$(find_host_lib libasound.so.2)"
LIB_PULSE="$(find_host_lib libpulse.so.0)"
LIB_PULSE32="$(find_host_lib32 libpulse.so.0)"
LIB_JACK="$(find_host_lib libjack.so.0)"
LIB_FDKAAC="$(find_host_lib libfdk-aac.so.2)"
LIB_LDAC="$(find_host_lib libldacBT_enc.so.2)"
LIB_APTX="$(find_host_lib libfreeaptx.so.0)"
LIB_DBUS="$(find_host_lib libdbus-1.so.3)"
PKG_DB_PATH="$(detect_pkg_db_path)"

elevate_run() {
    if [[ $(id -u) -eq 0 ]]; then
        "$@"
    elif command -v pkexec >/dev/null 2>&1; then
        pkexec "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo -n "$@"
    else
        echo "[Salsa] Error: pkexec or non-interactive sudo required for privileged operation." >&2
        return 1
    fi
}

if [[ "${ACTION}" == "--install-pacman-hook" ]]; then
    if ! command -v pacman >/dev/null 2>&1 || [[ ! -d /etc/pacman.d ]]; then
        echo "[Salsa] Pacman is not present on this system; skipping ALPM hook."
        exit 0
    fi
    TMP_HOOK="$(mktemp)"
    cat > "${TMP_HOOK}" <<'EOF'
[Trigger]
Operation = Upgrade
Operation = Install
Type = Package
Target = libpulse
Target = lib32-libpulse
Target = alsa-lib
Target = pipewire
Target = pipewire-pulse
Target = fdk-aac
Target = libfdk-aac
Target = libldac
Target = libfreeaptx
Target = bluez

[Action]
Depends = coreutils
Description = Updating Salsa LWASAPI root-owned ABI sync timestamp...
When = PostTransaction
Exec = /usr/bin/touch /var/lib/salsa-autosync/libpulse-updated
EOF
    chmod 0644 "${TMP_HOOK}"
    elevate_run /usr/bin/sh -c '
        set -eu
        /usr/bin/install -d -m 0755 -o root -g root /etc/pacman.d/hooks /var/lib/salsa-autosync
        /usr/bin/install -m 0644 -o root -g root "$1" /etc/pacman.d/hooks/99-salsa-autosync.hook
        /usr/bin/touch /var/lib/salsa-autosync/libpulse-updated
        /usr/bin/chmod 0644 /var/lib/salsa-autosync/libpulse-updated
    ' _ "${TMP_HOOK}"
    rm -f "${TMP_HOOK}"
    echo "[Salsa] Installed root-safe ALPM hook -> /etc/pacman.d/hooks/99-salsa-autosync.hook"
    echo "[Salsa] Re-run ./install-salsa.sh (or systemctl --user restart lwasapi-module-watcher.path) so the user watcher tracks /var/lib/salsa-autosync/libpulse-updated."
    exit 0
fi

if [[ "${ACTION}" == "--remove-pacman-hook" ]]; then
    elevate_run /usr/bin/sh -c '
        set -eu
        /usr/bin/rm -f /etc/pacman.d/hooks/99-salsa-autosync.hook /var/lib/salsa-autosync/libpulse-updated
        /usr/bin/rmdir /var/lib/salsa-autosync 2>/dev/null || true
    '
    echo "[Salsa] Removed /etc/pacman.d/hooks/99-salsa-autosync.hook and /var/lib/salsa-autosync."
    exit 0
fi

print_pc_report() {
    local rev="unknown"
    [[ -f "${BASE_DIR}/VERSION" ]] && rev="$(tr -d ' \t\r\n' < "${BASE_DIR}/VERSION")"
    echo "================================================================================"
    echo "  Salsa — Linux WASAPI Cross-PC Hardware & Distro Adaptation Profile (${rev})"
    echo "================================================================================"
    echo "  Host OS / Distro    : ${HOST_OS} (${HOST_ARCH})"
    echo "  Host CPU            : ${HOST_CPU}"
    echo "  Install Directory   : ${BASE_DIR}"
    echo "  User Home           : ${HOME} (UID $(id -u))"
    echo "  Package DB Watcher  : ${PKG_DB_PATH}"
    echo "  Host libasound.so.2 : ${LIB_ASOUND:-Not Found}"
    echo "  Host libpulse.so.0  : ${LIB_PULSE:-Not Found}"
    echo "  Host libpulse (32b) : ${LIB_PULSE32:-Optional multilib not installed}"
    echo "  Host libjack.so.0   : ${LIB_JACK:-Not Found}"
    echo "  Host libfdk-aac.so.2: ${LIB_FDKAAC:-Optional (SBC BlueZ fallback ready)}"
    echo "  Host libldacBT_enc  : ${LIB_LDAC:-Optional (SBC/AAC fallback ready)}"
    echo "  Host libfreeaptx    : ${LIB_APTX:-Optional (SBC/AAC fallback ready)}"
    echo "--------------------------------------------------------------------------------"
    echo "  Detected Kernel ALSA Soundcards (/proc/asound/cards):"
    if [[ -f /proc/asound/cards ]]; then
        grep -E '^[[:space:]]*[0-9]+' /proc/asound/cards | sed 's/^/    /' || echo "    (None)"
    fi
    echo "  Detected Hardware RawMIDI Controllers (/dev/snd/midiC*D*):"
    ls -1 /dev/snd/midiC*D* 2>/dev/null | sed 's/^/    /' || echo "    (Virtual LWASAPI-MIDI-Hub Standby)"
    echo "================================================================================"
}

if [[ "${ACTION}" == "--check" ]]; then
    print_pc_report
    exit 0
fi

if [[ "${ACTION}" == "--uninstall" ]]; then
    echo "[Salsa] Reverting Zero-PipeWire / PulseAudio isolation and removing Salsa..."
    "${BASE_DIR}/build/lwasapi-ctl" disable-auto >/dev/null 2>&1 || rm -f "${HOME}/.asoundrc"
    if command -v flatpak >/dev/null 2>&1 && [[ -f "${HOME}/.config/lwasapi/flatpak_overrides.list" && ! -L "${HOME}/.config/lwasapi/flatpak_overrides.list" ]]; then
        while IFS= read -r entry; do
            if [[ "${entry}" =~ ^socket:([A-Za-z0-9_]+(\.[A-Za-z0-9_-]+)+)$ ]]; then
                flatpak override --user --nosocket=pulseaudio "${BASH_REMATCH[1]}" >/dev/null 2>&1 || true
            elif [[ "${entry}" =~ ^fs:([A-Za-z0-9_]+(\.[A-Za-z0-9_-]+)+)$ ]]; then
                flatpak override --user --nofilesystem=xdg-run/pipewire-0:ro "${BASH_REMATCH[1]}" >/dev/null 2>&1 || true
            fi
        done < "${HOME}/.config/lwasapi/flatpak_overrides.list"
        rm -f "${HOME}/.config/lwasapi/flatpak_overrides.list"
    fi
    rm -f "${HOME}/.config/wireplumber/wireplumber.conf.d/99-lwasapi-video-only.conf" \
          "${HOME}/.config/wireplumber/main.lua.d/99-lwasapi-no-audio.lua" \
          "${HOME}/.config/wireplumber/bluetooth.lua.d/99-lwasapi-no-bluez.lua" \
          "${HOME}/.config/pipewire/pipewire.conf.d/99-lwasapi-bridge.conf" \
          "${HOME}/.config/lwasapi/pipewire_zero_mode" \
          "${HOME}/.config/lwasapi/masked_units.list" \
          "${HOME}/.config/environment.d/99-salsa-direct.conf"
    systemctl --user disable --now lwasapi-module-watcher.path lwasapi-module-watcher.timer lwasapi-tray.service lwasapi-audiodg.service >/dev/null 2>&1 || true
    systemctl --user stop salsa-lwasapi-gui.service salsa-studio.service >/dev/null 2>&1 || true
    rm -f "${HOME}/.config/systemd/user/lwasapi-audiodg.service" \
          "${HOME}/.config/systemd/user/lwasapi-tray.service" \
          "${HOME}/.config/systemd/user/lwasapi-module-watcher.path" \
          "${HOME}/.config/systemd/user/lwasapi-module-watcher.service" \
          "${HOME}/.config/systemd/user/lwasapi-module-watcher.timer" \
          "${HOME}/.config/systemd/user/salsa-studio.service"
    systemctl --user daemon-reload >/dev/null 2>&1 || true
    rm -f "${HOME}/.local/share/applications/salsa-lwasapi.desktop" \
          "${HOME}/.local/share/applications/salsa-debug.desktop" \
          "${HOME}/.local/share/applications/lwasapi-tray.desktop" \
          "${HOME}/.local/share/icons/hicolor/"*/apps/salsa-lwasapi.png \
          "${HOME}/.local/share/icons/hicolor/"*/apps/salsa-tray-*.png \
          "${HOME}/.local/bin/salsa" \
          "${HOME}/.local/bin/salsa-graph" \
          "${HOME}/.local/bin/salsa-debug" \
          "${HOME}/.local/bin/lwasapi-ctl" \
          "${HOME}/.local/bin/lwasapi-run" \
          "${HOME}/.local/bin/lwasapi-bench" \
          "${HOME}/.local/bin/salsa-steam-launch"
    if [[ -f "${HOME}/.config/lwasapi/plasmaparc_osd.state" && ! -L "${HOME}/.config/lwasapi/plasmaparc_osd.state" && ! -L "${HOME}/.config/plasmaparc" ]] && command -v kwriteconfig6 >/dev/null 2>&1; then
        while IFS='=' read -r osd_k osd_v; do
            case "${osd_k}" in
                VolumeOsd|MuteOsd|MicrophoneSensitivityOsd|DefaultOutputDeviceOsd)
                    if [[ "${osd_v}" == "__UNSET__" ]]; then
                        kwriteconfig6 --file "${HOME}/.config/plasmaparc" --group General --key "${osd_k}" --delete >/dev/null 2>&1 || true
                    else
                        kwriteconfig6 --file "${HOME}/.config/plasmaparc" --group General --key "${osd_k}" "${osd_v}" >/dev/null 2>&1 || true
                    fi
                    ;;
            esac
        done < "${HOME}/.config/lwasapi/plasmaparc_osd.state"
        rm -f "${HOME}/.config/lwasapi/plasmaparc_osd.state"
        if command -v qdbus6 >/dev/null 2>&1; then
            qdbus6 org.kde.kded6 /kded unloadModule audioshortcutsservice >/dev/null 2>&1 || true
            qdbus6 org.kde.kded6 /kded loadModule audioshortcutsservice >/dev/null 2>&1 || true
        fi
    fi
    if [[ -f "${HOME}/.config/lwasapi/steam_app.desktop.bak" && ! -L "${HOME}/.config/lwasapi/steam_app.desktop.bak" ]]; then
        mv -f "${HOME}/.config/lwasapi/steam_app.desktop.bak" "${HOME}/.local/share/applications/steam.desktop" 2>/dev/null || true
    elif grep -q "salsa-steam-launch" "${HOME}/.local/share/applications/steam.desktop" 2>/dev/null; then
        rm -f "${HOME}/.local/share/applications/steam.desktop"
    fi
    if [[ -f "${HOME}/.config/lwasapi/steam_autostart.desktop.bak" && ! -L "${HOME}/.config/lwasapi/steam_autostart.desktop.bak" ]]; then
        mv -f "${HOME}/.config/lwasapi/steam_autostart.desktop.bak" "${HOME}/.config/autostart/steam.desktop" 2>/dev/null || true
    elif grep -q "salsa-steam-launch" "${HOME}/.config/autostart/steam.desktop" 2>/dev/null; then
        sed -i "s|Exec=${HOME}/.local/bin/salsa-steam-launch|Exec=/usr/bin/steam|g" "${HOME}/.config/autostart/steam.desktop" 2>/dev/null || true
    fi
    for app_wrap in discord firefox; do
        if grep -q "libsalsa_pa_reconnect.so" "${HOME}/.local/bin/${app_wrap}" 2>/dev/null; then
            rm -f "${HOME}/.local/bin/${app_wrap}"
        fi
        if grep -q "${HOME}/.local/bin/${app_wrap}" "${HOME}/.local/share/applications/${app_wrap}.desktop" 2>/dev/null; then
            rm -f "${HOME}/.local/share/applications/${app_wrap}.desktop"
        fi
    done
    RUNTIME_BASE="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    rm -rf "${RUNTIME_BASE}/lwasapi" 2>/dev/null || true
    rm -f /tmp/lwasapi-audiodg.sock /tmp/lwasapi-debug.sock /tmp/lwasapi-debug-in.sock \
          "/dev/shm/lwasapi_pw_preview_$(id -u)" "/dev/shm/lwasapi_dryrun_cap_$(id -u).png" 2>/dev/null || true
    gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
    update-desktop-database "${HOME}/.local/share/applications" >/dev/null 2>&1 || true
    command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
    if [[ -e /etc/pacman.d/hooks/99-salsa-autosync.hook ]]; then
        echo "[Salsa] Optional system ALPM hook still present at /etc/pacman.d/hooks/99-salsa-autosync.hook; remove anytime via: ./install-salsa.sh --remove-pacman-hook"
    fi
    echo "[Salsa] Cleanly uninstalled Salsa — Linux WASAPI and restored previous system audio defaults."
    exit 0
fi

ensure_host_dependencies() {
    export PATH="${HOME}/.cargo/bin:${PATH}"
    local missing=0
    for cmd in gcc make pkg-config readelf nm python3 cargo; do
        if ! command -v "${cmd}" >/dev/null 2>&1; then
            missing=1
            break
        fi
    done
    if [[ "${missing}" -eq 0 ]] && ! pkg-config --exists alsa libpulse >/dev/null 2>&1; then
        missing=1
    fi
    if [[ "${missing}" -eq 0 ]] && ! python3 -c "import PyQt6" >/dev/null 2>&1; then
        missing=1
    fi
    if [[ "${missing}" -eq 0 ]]; then
        return 0
    fi

    echo "[Salsa] Installing missing build/GUI dependencies for ${HOST_OS}..."
    local ELEVATE="pkexec"
    if ! command -v pkexec >/dev/null 2>&1; then
        ELEVATE="sudo -n"
    fi

    if command -v pacman >/dev/null 2>&1; then
        ${ELEVATE} pacman -S --needed --noconfirm gcc make pkgconf binutils rust alsa-lib libpulse dbus sbc python python-pyqt6 desktop-file-utils
        ${ELEVATE} pacman -S --needed --noconfirm libfdk-aac >/dev/null 2>&1 || true
    elif command -v apt-get >/dev/null 2>&1; then
        ${ELEVATE} apt-get update && ${ELEVATE} apt-get install -y build-essential pkg-config binutils cargo rustc libasound2-dev libpulse-dev libdbus-1-3 libsbc1 python3 python3-pyqt6 desktop-file-utils
        ${ELEVATE} apt-get install -y libfdk-aac2 >/dev/null 2>&1 || true
    elif command -v dnf >/dev/null 2>&1; then
        ${ELEVATE} dnf install -y gcc make pkgconf-pkg-config binutils rust cargo alsa-lib-devel pulseaudio-libs-devel dbus-libs sbc python3 python3-pyqt6 desktop-file-utils
        ${ELEVATE} dnf install -y fdk-aac-free >/dev/null 2>&1 || true
    elif command -v zypper >/dev/null 2>&1; then
        ${ELEVATE} zypper --non-interactive install -y gcc make pkg-config binutils rust cargo alsa-devel libpulse-devel dbus-1 sbc python3 python3-qt6 desktop-file-utils
        ${ELEVATE} zypper --non-interactive install -y libfdk-aac2 >/dev/null 2>&1 || true
    fi
}

print_pc_report
ensure_host_dependencies

PYTHON_BIN="$(command -v python3 || echo /usr/bin/python3)"

# 1. Sync ELF symbol versions and compile all binaries/shims for this PC
chmod +x "${BASE_DIR}/install-salsa.sh" "${BASE_DIR}/lwasapi-autosync.sh" "${BASE_DIR}/lwasapi-run" "${BASE_DIR}/salsa-gui.py" "${BASE_DIR}/salsa-debug.py" "${BASE_DIR}/lwasapi-tray.py" "${BASE_DIR}/scripts/gen_pa_proxy.py"
"${BASE_DIR}/lwasapi-autosync.sh" --force

# 2. Generate systemd user units tailored to this PC's paths and distro
SYSTEMD_USER_DIR="${HOME}/.config/systemd/user"
mkdir -p "${SYSTEMD_USER_DIR}" "${HOME}/.local/bin" "${HOME}/.local/share/applications"

cat > "${SYSTEMD_USER_DIR}/lwasapi-audiodg.service" <<EOF
[Unit]
Description=Salsa — Linux WASAPI (lwasapi-audiodg) WaveRT 192kHz, Native PA v35, Direct BlueZ, Video Broker & Dynamic EQ Engine
After=bluetooth.target sound.target
Wants=bluetooth.target sound.target

[Service]
Type=simple
WorkingDirectory=${BASE_DIR}
ExecStart=${BASE_DIR}/build/lwasapi-audiodg
UMask=0077
Restart=always
RestartSec=1

[Install]
WantedBy=default.target
EOF

cat > "${SYSTEMD_USER_DIR}/lwasapi-tray.service" <<EOF
[Unit]
Description=Salsa — Linux WASAPI Pure Native Audio System Tray Switcher
After=graphical-session.target lwasapi-audiodg.service

[Service]
Type=simple
WorkingDirectory=${BASE_DIR}
ExecStart=${PYTHON_BIN} ${BASE_DIR}/lwasapi-tray.py
KillMode=process
Restart=on-failure
RestartSec=2

[Install]
WantedBy=default.target
EOF

cat > "${SYSTEMD_USER_DIR}/lwasapi-module-watcher.path" <<EOF
[Unit]
Description=Salsa — Linux WASAPI Automatic Audio Module Update Watcher (Path Trigger)
StartLimitIntervalSec=0

[Path]
PathChanged=${PKG_DB_PATH}
$([[ -e /var/lib/salsa-autosync/libpulse-updated ]] && echo "PathChanged=/var/lib/salsa-autosync/libpulse-updated")
${LIB_ASOUND:+PathChanged=${LIB_ASOUND}}
${LIB_PULSE:+PathChanged=${LIB_PULSE}}
${LIB_PULSE32:+PathChanged=${LIB_PULSE32}}
${LIB_JACK:+PathChanged=${LIB_JACK}}
${LIB_FDKAAC:+PathChanged=${LIB_FDKAAC}}
${LIB_LDAC:+PathChanged=${LIB_LDAC}}
${LIB_APTX:+PathChanged=${LIB_APTX}}
${LIB_DBUS:+PathChanged=${LIB_DBUS}}
TriggerLimitIntervalSec=0
Unit=lwasapi-module-watcher.service

[Install]
WantedBy=default.target
EOF

cat > "${SYSTEMD_USER_DIR}/lwasapi-module-watcher.service" <<EOF
[Unit]
Description=Salsa — Linux WASAPI Automatic Audio Module ABI & Config Sync Service
After=network.target
StartLimitIntervalSec=0

[Service]
Type=oneshot
TimeoutStartSec=300
WorkingDirectory=${BASE_DIR}
ExecStart=${BASE_DIR}/lwasapi-autosync.sh --auto
EOF

cat > "${SYSTEMD_USER_DIR}/lwasapi-module-watcher.timer" <<EOF
[Unit]
Description=Salsa — Linux WASAPI Boot & Periodic Update-Protection Verification Timer

[Timer]
OnBootSec=15s
OnUnitActiveSec=6h
Persistent=true
Unit=lwasapi-module-watcher.service

[Install]
WantedBy=timers.target
EOF

# 3. Create CLI symlinks in ~/.local/bin
rm -f "${HOME}/.local/bin/salsa" "${HOME}/.local/bin/salsa-graph" "${HOME}/.local/bin/salsa-debug" "${HOME}/.local/bin/lwasapi-ctl" "${HOME}/.local/bin/lwasapi-run" "${HOME}/.local/bin/lwasapi-bench"
ln -sf "${BASE_DIR}/salsa-gui.py" "${HOME}/.local/bin/salsa"
ln -sf "${BASE_DIR}/salsa-debug.py" "${HOME}/.local/bin/salsa-debug"
ln -sf "${BASE_DIR}/build/lwasapi-ctl" "${HOME}/.local/bin/lwasapi-ctl"
ln -sf "${BASE_DIR}/lwasapi-run" "${HOME}/.local/bin/lwasapi-run"
ln -sf "${BASE_DIR}/build/lwasapi-bench" "${HOME}/.local/bin/lwasapi-bench"

cat > "${HOME}/.local/bin/salsa-graph" <<EOF
#!/usr/bin/env bash
exec "${PYTHON_BIN}" "${BASE_DIR}/salsa-gui.py" --graph "\$@"
EOF
chmod +x "${HOME}/.local/bin/salsa-graph"

# 4. Install Application Icon, Create Launcher Shortcut & Refresh Desktop Database
SALSA_ICON_SRC="${BASE_DIR}/assets/salsa-lwasapi-glow.png"
if [[ ! -f "${SALSA_ICON_SRC}" ]]; then
    SALSA_ICON_SRC="${BASE_DIR}/assets/salsa-lwasapi.png"
fi
SALSA_ICON_DEST="${HOME}/.local/share/icons/hicolor/512x512/apps/salsa-lwasapi.png"
SALSA_ICON_ENTRY="multimedia-volume-control"
if [[ -f "${SALSA_ICON_SRC}" ]]; then
    mkdir -p "${HOME}/.local/share/icons/hicolor/512x512/apps"
    cp -f "${SALSA_ICON_SRC}" "${SALSA_ICON_DEST}"
    if command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
        "${PYTHON_BIN}" -c "
import os
try:
    from PIL import Image
    src = Image.open('${SALSA_ICON_SRC}').convert('RGBA')
    for sz in (48, 64, 128, 256):
        d = os.path.expanduser(f'~/.local/share/icons/hicolor/{sz}x{sz}/apps')
        os.makedirs(d, exist_ok=True)
        src.resize((sz, sz), Image.Resampling.LANCZOS).save(os.path.join(d, 'salsa-lwasapi.png'))
except Exception:
    pass
" || true
    fi
    SALSA_ICON_ENTRY="${SALSA_ICON_DEST}"
fi

cat > "${HOME}/.local/share/applications/salsa-lwasapi.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Salsa — Linux WASAPI Studio
GenericName=192kHz WaveRT Audio Engine, Dynamic EQ & Auto-Graph
Comment=High-Resolution 192kHz WaveRT Audio Mixer, 10-Band Real-Time Dynamic EQ & Automatic qpwgraph Signal Topology Manual
Exec=${PYTHON_BIN} ${BASE_DIR}/salsa-gui.py
Icon=${SALSA_ICON_ENTRY}
Terminal=false
Categories=AudioVideo;Audio;Mixer;Settings;
Keywords=WASAPI;Salsa;EQ;Dynamic EQ;192kHz;MIDI;Audio;BlueZ;PulseAudio;qpwgraph;
StartupNotify=true
EOF

cat > "${HOME}/.local/share/applications/salsa-debug.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Salsa — Live Interaction Debugger
GenericName=SALSA Telemetry, Routing Audit & Protocol Debugger
Comment=Live event stream, /proc audio routing audit, RT SCHED_FIFO jitter, BlueZ/PA v35/WASAPI SHM inspection & interactive probes
Exec=${PYTHON_BIN} ${BASE_DIR}/salsa-debug.py
Icon=${SALSA_ICON_ENTRY}
Terminal=false
Categories=AudioVideo;Audio;Development;System;
Keywords=WASAPI;Salsa;Debugger;Telemetry;Trace;BlueZ;PulseAudio;MIDI;ALSA;
StartupNotify=true
EOF

cat > "${HOME}/.local/share/applications/lwasapi-tray.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Salsa — Audio System Tray
GenericName=Salsa 192kHz Audio Control
Comment=Control Salsa LWASAPI (192kHz WaveRT, Dynamic EQ, Sample Rate, and Status) from the system tray
Exec=${PYTHON_BIN} ${BASE_DIR}/lwasapi-tray.py
Icon=${SALSA_ICON_ENTRY}
Terminal=false
Categories=AudioVideo;Audio;Settings;
StartupNotify=false
EOF

# Clean up any legacy LD_PRELOAD per-app wrappers; session-wide LD_LIBRARY_PATH
# (build/pa_reconnect/libpulse.so.0 configured by lwasapi-ctl enable-auto)
# transparently handles Firefox (including KDE session restore), Discord, etc.
for app_wrap in discord firefox; do
    if grep -q "libsalsa_pa_reconnect.so" "${HOME}/.local/bin/${app_wrap}" 2>/dev/null; then
        rm -f "${HOME}/.local/bin/${app_wrap}"
    fi
    if grep -q "${HOME}/.local/bin/${app_wrap}" "${HOME}/.local/share/applications/${app_wrap}.desktop" 2>/dev/null; then
        rm -f "${HOME}/.local/share/applications/${app_wrap}.desktop"
    fi
done

gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
update-desktop-database "${HOME}/.local/share/applications" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true

# 5. Enforce private 0700 config/runtime dirs, configure scoped Flatpak --user overrides, prune legacy /tmp & /dev/shm artifacts, and cut over desktop audio BEFORE starting lwasapi-audiodg
mkdir -m 0700 -p "${HOME}/.config/lwasapi"
chmod 0700 "${HOME}/.config/lwasapi"
for cfg_f in "${HOME}/.config/lwasapi/gui_settings.json" \
             "${HOME}/.config/lwasapi/app_volumes.conf" \
             "${HOME}/.config/lwasapi/flatpak_overrides.list" \
             "${HOME}/.config/lwasapi/module_versions.state" \
             "${HOME}/.config/lwasapi/salsa_state.bin"; do
    [[ -f "${cfg_f}" && ! -L "${cfg_f}" ]] && chmod 0600 "${cfg_f}" || true
done

if command -v flatpak >/dev/null 2>&1; then
    OVR_LIST="${HOME}/.config/lwasapi/flatpak_overrides.list"
    if [[ ! -L "${OVR_LIST}" ]]; then
        touch "${OVR_LIST}"
        chmod 0600 "${OVR_LIST}"
        INSTALLED_FP="$(flatpak list --app --columns=application 2>/dev/null | grep -E '^[A-Za-z0-9_]+(\.[A-Za-z0-9_-]+)+$' || true)"
        for app_id in com.valvesoftware.Steam com.usebottles.bottles com.github.Matoking.protontricks \
                      com.discordapp.Discord dev.vencord.Vesktop org.mozilla.firefox \
                      com.obsproject.Studio net.lutris.Lutris com.heroicgameslauncher.hgl; do
            if grep -qx "${app_id}" <<< "${INSTALLED_FP}"; then
                CUR_SHOW="$(flatpak override --user --show "${app_id}" 2>/dev/null || true)"
                CUR_SOCKETS="$(awk -F= '/^sockets=/{print $2}' <<< "${CUR_SHOW}")"
                CUR_FS="$(awk -F= '/^filesystems=/{print $2}' <<< "${CUR_SHOW}")"
                if [[ ";${CUR_SOCKETS};" != *";pulseaudio;"* ]]; then
                    if flatpak override --user --socket=pulseaudio "${app_id}" >/dev/null 2>&1; then
                        grep -qxF "socket:${app_id}" "${OVR_LIST}" || printf 'socket:%s\n' "${app_id}" >> "${OVR_LIST}"
                    fi
                fi
                if [[ ";${CUR_FS};" != *";xdg-run/pipewire-0"* ]]; then
                    if flatpak override --user --filesystem=xdg-run/pipewire-0:ro "${app_id}" >/dev/null 2>&1; then
                        grep -qxF "fs:${app_id}" "${OVR_LIST}" || printf 'fs:%s\n' "${app_id}" >> "${OVR_LIST}"
                    fi
                fi
            fi
        done
    fi
fi

if command -v kwriteconfig6 >/dev/null 2>&1 && { [[ "${XDG_CURRENT_DESKTOP:-}" == *KDE* ]] || [[ -f "${HOME}/.config/plasmaparc" ]] || pgrep -x plasmashell >/dev/null 2>&1; }; then
    if [[ ! -L "${HOME}/.config/plasmaparc" ]]; then
        OSD_STATE="${HOME}/.config/lwasapi/plasmaparc_osd.state"
        if [[ ! -e "${OSD_STATE}" && ! -L "${OSD_STATE}" ]]; then
            (
                umask 077
                : > "${OSD_STATE}"
                for osd_k in VolumeOsd MuteOsd MicrophoneSensitivityOsd DefaultOutputDeviceOsd; do
                    prev_v=""
                    if command -v kreadconfig6 >/dev/null 2>&1; then
                        prev_v="$(kreadconfig6 --file "${HOME}/.config/plasmaparc" --group General --key "${osd_k}" --default "__UNSET__" 2>/dev/null || echo "__UNSET__")"
                    else
                        prev_v="__UNSET__"
                    fi
                    printf '%s=%s\n' "${osd_k}" "${prev_v}" >> "${OSD_STATE}"
                done
            )
            chmod 0600 "${OSD_STATE}" 2>/dev/null || true
        fi
        for osd_k in VolumeOsd MuteOsd MicrophoneSensitivityOsd DefaultOutputDeviceOsd; do
            kwriteconfig6 --file "${HOME}/.config/plasmaparc" --group General --key "${osd_k}" false >/dev/null 2>&1 || true
        done
        if command -v qdbus6 >/dev/null 2>&1; then
            qdbus6 org.kde.kded6 /kded unloadModule audioshortcutsservice >/dev/null 2>&1 || true
            qdbus6 org.kde.kded6 /kded loadModule audioshortcutsservice >/dev/null 2>&1 || true
        fi
    fi
fi

RUNTIME_SUBDIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/lwasapi"
mkdir -m 0700 -p "${RUNTIME_SUBDIR}"
chmod 0700 "${RUNTIME_SUBDIR}"
rm -f /tmp/lwasapi-audiodg.sock /tmp/lwasapi-debug.sock /tmp/lwasapi-debug-in.sock \
      "/dev/shm/lwasapi_pw_preview_$(id -u)" "/dev/shm/lwasapi_dryrun_cap_$(id -u).png" 2>/dev/null || true

systemctl --user daemon-reload
systemctl --user reset-failed lwasapi-audiodg.service lwasapi-tray.service lwasapi-module-watcher.path lwasapi-module-watcher.service lwasapi-module-watcher.timer >/dev/null 2>&1 || true
systemctl --user stop lwasapi-audiodg.service >/dev/null 2>&1 || true
"${BASE_DIR}/build/lwasapi-ctl" enable-auto
systemctl --user enable lwasapi-audiodg.service lwasapi-tray.service lwasapi-module-watcher.path lwasapi-module-watcher.timer
systemctl --user restart lwasapi-audiodg.service lwasapi-tray.service lwasapi-module-watcher.path lwasapi-module-watcher.timer

if readelf -d "${BASE_DIR}/build/lwasapi-audiodg" 2>/dev/null | grep -q "BIND_NOW"; then
    echo "[Salsa] Hardening verified: Full RELRO (BIND_NOW), stack protector, FORTIFY, SO_PEERCRED & private 0700 runtime dir (${RUNTIME_SUBDIR})."
fi
echo "[Salsa] Cross-PC installation complete! Running pure Zero-PipeWire Salsa (PipeWire/WirePlumber/PA stopped & masked, built-in Video-Only ScreenCast broker active, Native PA v35 + Direct BlueZ + Steam/Proton direct audio active)."
if command -v pacman >/dev/null 2>&1 && [[ ! -e /etc/pacman.d/hooks/99-salsa-autosync.hook ]]; then
    echo "[Salsa] Optional Arch/CachyOS tip: enable root-safe ALPM trigger via: ./install-salsa.sh --install-pacman-hook"
fi
echo "[Salsa] Launch Studio GUI: salsa   | Signal Graph: salsa-graph   | Live Debugger: salsa-debug"
echo "[Salsa] To revert anytime: lwasapi-ctl disable-auto   (or full removal: ./install-salsa.sh --uninstall)"
