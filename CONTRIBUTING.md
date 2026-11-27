# Contributing to Salsa

Thank you for contributing to **Salsa**! Salsa is a high-frequency (`192,000 Hz / 32-bit Float`) WaveRT zero-copy Linux audio engine, PulseAudio v35 server, Wayland `ScreenCast` video broker, direct BlueZ A2DP/AVRCP engine, 10-band Dynamic EQ APO, and native Wayland Qt6 Studio/Debugger/Tray suite.

By participating in this project, you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

---

## Repository Layout

- `rust/salsa_host_dl/`: Verified host `dlopen` loader (`open_verified_host_lib`, `open_verified_optional_lib`) with `(st_dev, st_ino)` self-recursion guards plus canonical PA ABI / local-Unix failover helpers.
- `rust/salsa_net/` (`salsa_wire`): Core `#![deny(unsafe_code)]` real-time audio engine (`audiodg_engine.rs`), ALSA hardware backend seam (`alsa_hw.rs`), sealed `WaveRtRing` seam (`wavert_ring.rs`), `PodBytes` wire/ABI layouts (`c_abi.rs`), typed `Control IPC` (`control_ipc.rs`, `client_seam.rs`), DSP/Dynamic EQ (`dsp.rs`), unified host provisioning (`provisioning.rs`, `#![deny(unsafe_code)]`), settings schema (`settings.rs`, `state_conf.rs`), PulseAudio v35 server (`pa_wire.rs`, `pa_server.rs`), Wayland ScreenCast video broker & zero-aliasing preview seqlock (`pw_wire.rs`, `pw_broker.rs`, `preview_shm.rs`, `dmabuf_gl.rs`), direct BlueZ D-Bus A2DP/HFP/AVRCP runtime & codec adapters (`bluez_*.rs`), hardware analog & HID controls (`hw_analog.rs`), DAW MIDI hub (`midi_hub.rs`), and `lasapi-ctl` (`ctl_cli.rs`).
- `rust/lasapi_client/`: Client-side WaveRT shared-memory ring library (`liblasapi.so`), unit suite (`lasapi-test`), and benchmark runner (`lasapi-bench`).
- `rust/salsa_pa_reconnect/` & `rust/salsa_pa_reconnect_proxy/`: Drop-in `libpulse.so.0` transparent reconnect proxy.
- `rust/alsa_plugin_lasapi/`, `rust/shim_alsa/`, `rust/shim_pulse/`, `rust/shim_jack/`: Native ALSA PCM plugin and zero-IPC hollow ABI shims (`lasapi-run`).
- `rust/salsa_ui/`: Pure-Rust Studio/Tray/Debugger backend, themes, AutoEQ parser, Owner's Manual (`studio_manual.rs`), tray multi-monitor placement (`tray_placement.rs`), unified GLib/GObject/GIO D-Bus & SNI host (`gio_dbus.rs`, `sni.rs`, `screencast_preview.rs`), and C FFI bridge (`qt_bridge.rs`).
- `rust/salsa_tools/`: Installer (`salsa-installer`), host ELF autosync (`lasapi-autosync`), live debugger CLI (`salsa-debug-cli`), regression harness (`salsa-regression`), and packager (`salsa-dist`).
- `qt6/`: Native C++20 Qt6 user interfaces (`salsa_studio.cpp`, `salsa_debug.cpp`, `salsa_widgets.cpp`, `salsa_debugger_widget.cpp`, `lasapi_tray.cpp`).
- `GLOSSARY.md` & `docs/adr/`: Ubiquitous domain vocabulary and immutable ABI/framing decisions (`ADR-0001`).

---

## Core Design & Portability Invariants

Every pull request must preserve Salsa's foundational engineering guarantees:

1. **Zero PipeWire / WirePlumber Audio Dependency**: Salsa replaces user-space audio daemons by driving kernel `/dev/snd/pcmC*D*` and BlueZ `MediaEndpoint1` directly. `$XDG_RUNTIME_DIR/pipewire-0` (`pw_broker.rs`) exists strictly to relay Wayland Portal `ScreenCast` video frames (`DmaBuf` / `MemFd`) and must never route audio through PipeWire.
2. **Wayland-First Native UI & Zero X11 in Core**: Core crates (`salsa_net`, `lasapi_client`, `salsa_tools`) must have zero X11 dependencies. GUI binaries (`salsa-studio`, `salsa-debug`, `lasapi-tray`) must run natively on Wayland (`QT_QPA_PLATFORM=wayland;xcb`, using runtime `dlopen` of `libLayerShellQtInterface.so.6` where available) and only touch X11/EWMH helpers when Qt resolves to `xcb`.
3. **Strict Multi-PC Portability**: Never hardcode user home paths (`/home/...`), numeric UIDs (`1000`), hostnames, soundcard numbers, or single-vendor assumptions. Always resolve runtime/config paths via `$XDG_RUNTIME_DIR`, `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`, `$HOME`, and portable systemd specifiers (`%h`, `%t`).
4. **Real-Time Audio Safety**: Never allocate heap memory, lock contended mutexes, perform blocking disk/socket I/O, or spawn processes inside the `SCHED_FIFO` real-time mixing callback (`audiodg_engine.rs` / `dsp.rs`). Scrub non-finite (`NaN`/`Inf`) samples at DSP boundaries.
5. **LGPL-3.0-Only & Verified Runtime `dlopen` Policy**:
   - All new source files must include the `SPDX-License-Identifier: LGPL-3.0-only` header.
   - Optional third-party codec or desktop libraries (`libfdk-aac.so.2`, `libldacBT_enc.so.2`, `libfreeaptx.so.0`, `libsbc.so.1`, `libdbus-1.so.3`, `libLayerShellQtInterface.so.6`, `libKF6WindowSystem.so.6`) must **never** be bundled or linked via `DT_NEEDED`; load them exclusively at runtime via `salsa_host_dl::open_verified_optional_lib` (`dlopen()`) with graceful fallback when absent.

---

## Building & Testing Locally

Install standard build prerequisites (`rustc`/`cargo`, `gcc`, `g++`, `make`, `pkg-config`, `alsa-lib`, `libpulse`, and `qt6-base`), then build from the repository root:

```bash
make -j"$(nproc)" all
```

Before submitting a Pull Request, run the full lint, unit test, and integration verification suite:

```bash
# 1. Zero-warning Clippy lint across all workspace crates and targets
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings

# 2. Rust workspace unit tests
cargo test --manifest-path rust/Cargo.toml --workspace

# 3. Full 9-suite C/Rust/UI/ABI/Security regression harness
./build/salsa-regression

# 4. If you touched wire protocol parsers (pa_wire.rs, pw_wire.rs, bluez_wire.rs), run fuzzing:
make test-sanitized
```

---

## Submitting Pull Requests

1. Fork the repository and create a focused topic branch off `main` (e.g., `fix/bt-avrcp-clamp` or `feat/apo-band-solo`).
2. Keep commits atomic and descriptive (`feat(scope): ...`, `fix(scope): ...`, `docs: ...`).
3. Ensure `build/` artifacts, `.zip` archives, and `rust/target/` directories are not staged (`git status --short` should show source/doc files only).
4. Open a Pull Request and complete the checklist in `.github/pull_request_template.md`.
