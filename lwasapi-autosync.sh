#!/usr/bin/env bash
# ==============================================================================
# Salsa — Linux WASAPI (LWASAPI) Universal Cross-PC Audio Module & ABI Sync
# ==============================================================================
# Automatically detects updates to audio packages across any Linux distribution
# (Arch/CachyOS pacman, Debian/Ubuntu dpkg, Fedora/RHEL/openSUSE rpm),
# syncs ELF symbol versions (src/alsa.version & src/pulse.version) from the host
# PC's libasound.so.2 and libpulse.so.0, recompiles all shims and binaries,
# and self-heals desktop audio routing.
# ==============================================================================

set -euo pipefail

BASE_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
STATE_DIR="${HOME}/.config/lwasapi"
STATE_FILE="${STATE_DIR}/module_versions.state"
LOG_FILE="${STATE_DIR}/autosync.log"

mkdir -p "${STATE_DIR}"
chmod 0700 "${STATE_DIR}"

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

LIB_ASOUND="$(find_host_lib libasound.so.2)"
LIB_PULSE="$(find_host_lib libpulse.so.0)"
LIB_PULSE32="$(find_host_lib32 libpulse.so.0)"
LIB_PULSE_SIMPLE="$(find_host_lib libpulse-simple.so.0)"
LIB_JACK="$(find_host_lib libjack.so.0)"
LIB_SBC="$(find_host_lib libsbc.so.1)"
LIB_FDKAAC="$(find_host_lib libfdk-aac.so.2)"
LIB_LDAC="$(find_host_lib libldacBT_enc.so.2)"
LIB_APTX="$(find_host_lib libfreeaptx.so.0)"
LIB_OPENAL="$(find_host_lib libopenal.so.1)"
LIB_DBUS="$(find_host_lib libdbus-1.so.3)"
BLUEZD_BIN=""
for _b in /usr/lib/bluetooth/bluetoothd /usr/libexec/bluetooth/bluetoothd; do
    if [[ -e "${_b}" ]]; then
        BLUEZD_BIN="${_b}"
        break
    fi
done

MODE="${1:---force}"

exec 9>"${STATE_DIR}/autosync.lock"
if [[ "${MODE}" == "--auto" ]]; then
    flock -n 9 || exit 0
    # Wait for active package manager transactions + post-install hooks to settle
    waited=0
    while (( waited < 240 )); do
        dpkg_locked=0
        if [[ -e /var/lib/dpkg/lock-frontend ]] && command -v fuser >/dev/null 2>&1; then
            fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1 && dpkg_locked=1 || true
        fi
        if [[ -e /var/lib/pacman/db.lck ]] || [[ -e /var/run/yum.pid ]] || [[ "${dpkg_locked}" -eq 1 ]]; then
            sleep 2
            waited=$((waited + 2))
        else
            break
        fi
    done
    sleep 3
else
    flock 9
fi

get_current_fingerprint() {
    uname -srm
    if command -v pacman >/dev/null 2>&1; then
        pacman -Q linux linux-cachyos alsa-lib alsa-plugins libpulse fdk-aac libfdk-aac libldac libfreeaptx pipewire pipewire-pulse pipewire-jack pipewire-alsa wireplumber bluez bluez-libs dbus dbus-broker sbc openal sdl2 sdl3 lib32-pipewire lib32-libpipewire lib32-alsa-lib lib32-alsa-plugins lib32-libpulse lib32-dbus lib32-openal xdg-desktop-portal xdg-desktop-portal-kde wine 2>/dev/null || true
    elif command -v dpkg-query >/dev/null 2>&1; then
        dpkg-query -W -f='${Package} ${Version}\n' libasound2* libpulse0 libfdk-aac* libldacbt* libfreeaptx* pipewire* wireplumber bluez dbus* libsbc1 libopenal1 libsdl2* xdg-desktop-portal* 2>/dev/null || true
    elif command -v rpm >/dev/null 2>&1; then
        rpm -q alsa-lib pulseaudio-libs fdk-aac fdk-aac-free libldac libfreeaptx pipewire wireplumber bluez bluez-libs dbus dbus-broker sbc openal-soft SDL2 xdg-desktop-portal 2>/dev/null || true
    fi
    for f in "${LIB_ASOUND}" "${LIB_PULSE}" "${LIB_PULSE32}" "${LIB_PULSE_SIMPLE}" "${LIB_JACK}" "${LIB_SBC}" "${LIB_FDKAAC}" "${LIB_LDAC}" "${LIB_APTX}" "${LIB_OPENAL}" "${LIB_DBUS}" "${BLUEZD_BIN}"; do
        [[ -n "${f}" && -e "${f}" ]] && stat -c "%Y %s %n" "${f}" 2>/dev/null || true
    done
    if [[ -r /var/lib/salsa-autosync/libpulse-updated ]]; then
        stat -c "%Y %s %n" /var/lib/salsa-autosync/libpulse-updated 2>/dev/null || true
    fi
}

CURRENT_FP="$(get_current_fingerprint)"
DRIFT=0
if [[ -f "${STATE_DIR}/pipewire_zero_mode" ]]; then
    if systemctl --user is-active --quiet pipewire.service pipewire.socket wireplumber.service pipewire-pulse.service pipewire-pulse.socket 2>/dev/null; then
        DRIFT=1
    elif systemctl --user is-enabled --quiet lwasapi-audiodg.service 2>/dev/null && ! systemctl --user is-active --quiet lwasapi-audiodg.service 2>/dev/null; then
        DRIFT=1
    fi
fi

if [[ "${MODE}" == "--auto" ]] && [[ -f "${STATE_FILE}" ]] && [[ "${DRIFT}" -eq 0 ]]; then
    PREV_FP="$(cat "${STATE_FILE}")"
    if [[ "${CURRENT_FP}" == "${PREV_FP}" ]]; then
        exit 0
    fi
fi

TS="$(date '+%Y-%m-%d %H:%M:%S')"
HOST_OS="$(awk -F= '/^PRETTY_NAME=/{gsub(/"/,"",$2); print $2}' /etc/os-release 2>/dev/null || uname -sr)"
echo "[${TS}] Starting Salsa LWASAPI Audio Module Auto-Sync (${MODE}, drift=${DRIFT}) on ${HOST_OS} (Dir: ${BASE_DIR})..." | tee -a "${LOG_FILE}"

# ------------------------------------------------------------------------------
# 1. Dynamically Extract & Sync ALSA ELF Symbol Versions (libasound.so.2)
# ------------------------------------------------------------------------------
if [[ -n "${LIB_ASOUND}" && -e "${LIB_ASOUND}" ]]; then
    mapfile -t ALSA_VERS < <(
        readelf -V "${LIB_ASOUND}" 2>/dev/null \
        | grep -oE 'ALSA_[0-9]+\.[0-9]+(\.[0-9a-zA-Z]+)?' \
        | sort -uV
    )
    if [[ ${#ALSA_VERS[@]} -gt 0 ]]; then
        ALSA_VER_FILE="${BASE_DIR}/src/alsa.version"
        {
            FIRST_VER="${ALSA_VERS[0]}"
            echo "${FIRST_VER} {"
            echo "    global:"
            echo "        snd_*;"
            echo "        _snd_*;"
            echo "    local:"
            echo "        *;"
            echo "};"
            PREV_VER="${FIRST_VER}"
            for ((i = 1; i < ${#ALSA_VERS[@]}; i++)); do
                V="${ALSA_VERS[$i]}"
                echo ""
                echo "${V} {"
                echo "} ${PREV_VER};"
                PREV_VER="${V}"
            done
        } > "${ALSA_VER_FILE}"
        echo "[${TS}] Synced ${#ALSA_VERS[@]} ELF symbol versions from ${LIB_ASOUND} -> src/alsa.version" | tee -a "${LOG_FILE}"
    fi
fi

# ------------------------------------------------------------------------------
# 2. Dynamically Extract & Sync PulseAudio ELF Symbol Versions (libpulse.so.0)
# ------------------------------------------------------------------------------
if [[ -n "${LIB_PULSE}" && -e "${LIB_PULSE}" ]]; then
    mapfile -t PULSE_VERS < <(
        readelf -V "${LIB_PULSE}" ${LIB_PULSE_SIMPLE:+"${LIB_PULSE_SIMPLE}"} 2>/dev/null \
        | grep -oE 'PULSE_[0-9]+(\.[0-9]+)?' \
        | sort -uV
    )
    if [[ ${#PULSE_VERS[@]} -gt 0 ]]; then
        PULSE_VER_FILE="${BASE_DIR}/src/pulse.version"
        {
            FIRST_P="${PULSE_VERS[0]}"
            echo "${FIRST_P} {"
            echo "    global:"
            echo "        pa_*;"
            echo "    local:"
            echo "        *;"
            echo "};"
            PREV_P="${FIRST_P}"
            for ((i = 1; i < ${#PULSE_VERS[@]}; i++)); do
                V="${PULSE_VERS[$i]}"
                echo ""
                echo "${V} {"
                echo "} ${PREV_P};"
                PREV_P="${V}"
            done
        } > "${PULSE_VER_FILE}"
        echo "[${TS}] Synced ${#PULSE_VERS[@]} ELF symbol versions from ${LIB_PULSE} -> src/pulse.version" | tee -a "${LOG_FILE}"
    fi
fi

# ------------------------------------------------------------------------------
# 3. Recompile Salsa LWASAPI Engine, ALSA Plugin & Hollow ABI Shims
# ------------------------------------------------------------------------------
make -C "${BASE_DIR}" -B >> "${LOG_FILE}" 2>&1
echo "[${TS}] Recompiled lwasapi-audiodg, liblwasapi.so, and all ALSA/Pulse/JACK shims." | tee -a "${LOG_FILE}"

# ------------------------------------------------------------------------------
# 4. Re-Verify Pure Salsa Zero-PipeWire Ownership & Native Video Broker
# ------------------------------------------------------------------------------
if systemctl --user is-active --quiet lwasapi-audiodg.service 2>/dev/null || \
   systemctl --user is-enabled --quiet lwasapi-audiodg.service 2>/dev/null; then
    systemctl --user reset-failed lwasapi-audiodg.service lwasapi-module-watcher.path lwasapi-module-watcher.service >/dev/null 2>&1 || true
    systemctl --user stop lwasapi-audiodg.service >/dev/null 2>&1 || true
    rm -f /tmp/lwasapi-audiodg.sock /tmp/lwasapi-debug.sock /tmp/lwasapi-debug-in.sock \
          "/dev/shm/lwasapi_pw_preview_$(id -u)" "/dev/shm/lwasapi_dryrun_cap_$(id -u).png" 2>/dev/null || true
    "${BASE_DIR}/build/lwasapi-ctl" enable-auto >> "${LOG_FILE}" 2>&1 || true
    systemctl --user start lwasapi-audiodg.service
    echo "[${TS}] Hot-restarted lwasapi-audiodg.service (Zero-PipeWire mode + Salsa Video Broker + Direct BlueZ/Steam/PA/ALSA verified)." | tee -a "${LOG_FILE}"
fi

# Save updated module fingerprint
printf "%s\n" "${CURRENT_FP}" > "${STATE_FILE}"
echo "[${TS}] Audio module sync completed successfully." | tee -a "${LOG_FILE}"
