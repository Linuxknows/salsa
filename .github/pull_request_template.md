## Summary

<!-- Briefly describe what this PR changes and why. Link any related issue (`Closes #...`). -->

## Affected Areas

- [ ] Core Audio / DSP / Wire Protocols (`rust/salsa_net/`, `rust/lasapi_client/`)
- [ ] Shims / Reconnect Proxy (`rust/salsa_pa_reconnect*`, `rust/alsa_plugin_lasapi/`, `rust/shim_*`)
- [ ] Native Qt6 / Wayland UI (`qt6/`, `rust/salsa_ui/`)
- [ ] Installer / Autosync / Diagnostics (`rust/salsa_tools/`, `install-salsa.sh`, `Makefile`)
- [ ] Documentation (`README.md`, `PATCH_NOTES.md`, `SALSA Architecture Breakdown.md`)

## Verification Checklist

- [ ] `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings` passes with **0 warnings**
- [ ] `make -j"$(nproc)" all` builds cleanly with **0 compiler warnings**
- [ ] `cargo test --manifest-path rust/Cargo.toml --workspace` passes
- [ ] `./build/salsa-regression` passes all suites
- [ ] **Multi-PC Portable**: No hardcoded `/home/<user>`, UID `1000`, hostnames, or single-machine device IDs
- [ ] **Architecture & LGPL-3.0 Compliant**: Zero PipeWire audio dependency, native Wayland-first UI (zero X11 in core), `SPDX-License-Identifier: LGPL-3.0-only` headers present, and optional third-party codecs remain `dlopen()`-only
