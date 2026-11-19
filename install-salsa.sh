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

if [[ ! -x "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/salsa_tools/src/bin/salsa_installer.rs" -nt "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/salsa_tools/src/debug_core.rs" -nt "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/salsa_tools/src/host.rs" -nt "${INSTALLER_BIN}" || "${SCRIPT_DIR}/rust/Cargo.toml" -nt "${INSTALLER_BIN}" ]]; then
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

exec "${INSTALLER_BIN}" "$@"
