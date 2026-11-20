#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-3.0-only
# Copyright (C) 2026 Linuxknows
#
# Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
# under the terms of the GNU Lesser General Public License as published by
# the Free Software Foundation, version 3 of the License.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
INSTALLER_BIN="${SCRIPT_DIR}/build/salsa-installer"

SKIP_TERM=0
if [[ "${SALSA_INSTALLER_NO_TERMINAL:-0}" == "1" ]]; then
    SKIP_TERM=1
fi
for arg in "$@"; do
    case "${arg}" in
        --no-terminal|--check|--uninstall|-h|--help|-V|--version|--license)
            SKIP_TERM=1
            ;;
    esac
done

launch_in_terminal() {
    local -a candidates=()
    if [[ -n "${TERMINAL:-}" ]]; then
        candidates+=("${TERMINAL}")
    fi
    candidates+=(
        konsole x-terminal-emulator kgx gnome-terminal
        xfce4-terminal kitty alacritty wezterm foot
        mate-terminal tilix xterm
    )
    local self_path="${BASH_SOURCE[0]}"
    local title="Salsa (LWASAPI) Installer"
    for term in "${candidates[@]}"; do
        local bin="${term%% *}"
        command -v "${bin}" >/dev/null 2>&1 || continue
        case "${bin}" in
            konsole)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" --title "${title}" -e bash "${self_path}" "$@"
                return $?
                ;;
            gnome-terminal)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" --wait --title="${title}" -- bash "${self_path}" "$@"
                return $?
                ;;
            kgx)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" --title "${title}" -e bash "${self_path}" "$@"
                return $?
                ;;
            xfce4-terminal)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" --title="${title}" -e "bash $(printf '%q ' "${self_path}" "$@")"
                return $?
                ;;
            kitty)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" --title "${title}" bash "${self_path}" "$@"
                return $?
                ;;
            alacritty|lxterminal|mate-terminal|tilix)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" -e bash "${self_path}" "$@"
                return $?
                ;;
            wezterm)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" start -- bash "${self_path}" "$@"
                return $?
                ;;
            foot)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" --title="${title}" bash "${self_path}" "$@"
                return $?
                ;;
            xterm)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" -T "${title}" -e bash "${self_path}" "$@"
                return $?
                ;;
            *)
                SALSA_INSTALLER_SPAWNED_TERM=1 "${bin}" -e bash "${self_path}" "$@"
                return $?
                ;;
        esac
    done
    return 127
}

if [[ ! -t 0 && ! -t 1 && "${SKIP_TERM}" -eq 0 && "${SALSA_INSTALLER_SPAWNED_TERM:-0}" != "1" ]]; then
    if [[ -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]]; then
        if launch_in_terminal "$@"; then
            exit 0
        fi
    fi
fi

if [[ ! -x "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/salsa_tools/src/bin/salsa_installer.rs" -nt "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/salsa_tools/src/debug_core.rs" -nt "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/salsa_tools/src/host.rs" -nt "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/Cargo.toml" -nt "${INSTALLER_BIN}" ]]; then
    echo "================================================================================"
    echo "  Salsa (LWASAPI) Bootstrap"
    echo "================================================================================"
    echo "[0/6] Compiling Salsa installer helper..."
    if command -v make >/dev/null 2>&1; then
        make -C "${SCRIPT_DIR}" -j"$(nproc 2>/dev/null || echo 4)" build/salsa-installer
    else
        CARGO_BIN="$(command -v cargo 2>/dev/null || echo "${HOME}/.cargo/bin/cargo")"
        HOST_LP="$(ldconfig -p 2>/dev/null | awk '/libpulse\.so\.0 / && $0 !~ /lib32/ {print $NF; exit}')"
        SALSA_HOST_LIBPULSE="${HOST_LP:-/usr/lib/libpulse.so.0}" \
            RUSTFLAGS="-C relocation-model=pic -C link-arg=-Wl,-z,relro,-z,now" \
            "${CARGO_BIN}" build --manifest-path "${SCRIPT_DIR}/rust/Cargo.toml" \
            --release --offline -p salsa_tools --bin salsa-installer \
            --target-dir "${SCRIPT_DIR}/build/rust_target"
        mkdir -p "${SCRIPT_DIR}/build"
        cp "${SCRIPT_DIR}/build/rust_target/release/salsa-installer" "${INSTALLER_BIN}.tmp"
        mv -f "${INSTALLER_BIN}.tmp" "${INSTALLER_BIN}"
    fi
fi

if [[ "${SALSA_INSTALLER_SPAWNED_TERM:-0}" == "1" ]]; then
    rc=0
    "${INSTALLER_BIN}" "$@" || rc=$?
    echo
    if [[ -t 0 ]]; then
        read -r -p "Press Enter to close this installer window... " _ </dev/tty || true
    fi
    exit "${rc}"
fi

exec "${INSTALLER_BIN}" "$@"
