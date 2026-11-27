# Salsa — Automatic Signal Architecture Breakdown

## 1. Philosophy: Why Salsa Needs Zero Manual Patching
In traditional Linux pro-audio workflows, patchbay utilities like `qpwgraph` or `QjackCtl` require you to manually drag virtual cables between applications, PulseAudio/JACK bridges, resamplers, and hardware sinks. Whenever an application restarts, changes sample rate, or switches APIs, those manual routes frequently disconnect.

**Salsa** eliminates manual patching completely by implementing a **Linux Audio Session API (`LASAPI`) / WaveRT Audio Device Graph (`lasapi-audiodg`)** architecture natively on Linux:
- **100% Automatic Self-Wiring Topology:** Every playback application, microphone capture stream, desktop loopback stream, Discord/OBS per-app capture tap, and hardware MIDI controller is automatically discovered and connected in `0.25 ms` without user intervention.
- **Automatic Multi-Rate Resampling & Split Clocking:** Applications streaming at `44.1 kHz`, `48 kHz`, `88.2 kHz`, `96 kHz`, `176.4 kHz`, or `192 kHz` are automatically resampled and mixed into your high-resolution master clock (up to **`192,000 Hz` / `32-bit Float`**, with optional independent capture clock pinning).

---

## 2. Automatic Signal Flow Topology (`qpwgraph` Equivalent View)

```
┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐
│     1. INPUTS & APPLICATIONS     │       │   2. WAVERT ZERO-COPY ENGINE     │       │  3. SALSA APO & DAW MIDI HUB     │       │  4. HARDWARE & CAPTURE OUTPUTS   │
├──────────────────────────────────┤       ├──────────────────────────────────┤       ├──────────────────────────────────┤       ├──────────────────────────────────┤
│ 🔊 Audio Sessions & Host Cutover │       │ ⚡ WaveRT Ring (wavert_ring.rs)  │       │ 🎚️ Salsa 10-Band Dynamic EQ APO  │       │ 🎧 Master Hardware Endpoint      │
│  • Native LASAPI (client_seam)   ├──────►│  • Sealed memfd + PodBytes       ├──────►│  • 10 RBJ Parametric EQ Bands    ├──────►│  • RealBluezBackend + A2dpAdapter│
│  • Native PA v35 ($PULSE/native) │ 75 ns │  • Kernel eventfd() wakeups      │       │  • Dynamic Compress / Expand     │       │  • AlsaPlaybackBackend + Analog  │
│  • Hollow Shims & provisioning   │       │  • 0.075 µs atomic commit        │       │  • Bauer Crossfeed & Limiter     │       │  • Hotplug Tracker + Control IPC │
└──────────────────────────────────┘       └────────────────┬─────────────────┘       └────────────────┬─────────────────┘       └──────────────────────────────────┘
                                                            │                                          │ Direct-on-Input Tap     ┌──────────────────────────────────┐
┌──────────────────────────────────┐       ┌────────────────▼─────────────────┐                        └────────────────────────►│ 📹 Full-Duplex Capture Apps      │
│ 🎙️ Capture Clock & Mic APO       │       │ 🚀 Safe lasapi-audiodg Core      │                                                  │  • Discord Voice & App SoundShare│
│  • Independent Capture Clock     ├──────►│  • #![deny(unsafe_code)] Engine  ├─────────────────────────────────────────────────►│  • OBS Desktop Loopback Monitor  │
│  • MsbcScoTranscoder HFP SCO     │       │  • SCHED_FIFO + mlockall()       │              Mic Capture Ring                    │  • DAW Audio & MIDI Recording    │
└──────────────────────────────────┘       │  • Master Clock up to 192kHz/32f │                                                  └─────────────────▲────────────────┘
                                           └──────────────────────────────────┘                                                                    │
┌──────────────────────────────────┐       ┌──────────────────────────────────┐                                                                    │
│ 🎹 Hardware MIDI Controller      │       │ 🎵 Lock-Free DAW MIDI Hub        │                                                                    │
│  • Auto RawMIDI (/dev/snd/midi*) ├──────►│  • LASAPI-MIDI-Hub (midi_hub.rs) ├────────────────────────────────────────────────────────────────────┘
│  • Verified Host ALSA Sequencer  │       │  • 512-Event Lock-Free Ring      │
└──────────────────────────────────┘       └──────────────────────────────────┘
═════════════════════════════ ZERO-PIPEWIRE WAYLAND VIDEO & SCREENCAST LANE ════════════════════════════
┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐
│ 🖥️ Wayland Video Producer        │       │ 🔌 Socket-Activated PW Broker    │       │ 🎞️ Relay & Preview Seqlock       │       │ 📡 ScreenCast Consumers          │
│  • kwin_wayland / gio_dbus Portal├──────►│  • lasapi-pipewire.socket (0ms) ├──────►│  • SCM_RIGHTS + PW_CAST_LIMITS   ├──────►│  • Discord (AV1 VAAPI) / OBS     │
│  • open_verified_optional_lib    │       │  • Zero libpipewire / Audio Guard│       │  • PreviewShmWriter / Reader     │       │  • Browsers / KDE MediaMonitor   │
└──────────────────────────────────┘       └──────────────────────────────────┘       └──────────────────────────────────┘       └─────────────────▲────────────────┘
                                                                                                                                             (Parallel A/V via WaveRT)
```

---

## 3. Component-by-Component Owner's Guide

### Stage 1: Audio Sessions, Unified Provisioning & Hollow Shims
- **Unified Host Provisioning ([`provisioning.rs`](rust/salsa_net/src/provisioning.rs)):** Single canonical owner of `~/.asoundrc`, `~/.config/OpenAL/alsoft.conf` (`drivers = pulse,alsa`), `~/.config/environment.d/99-salsa-direct.conf`, systemd unit state, and pacman hook paths, using symlink-safe `O_NOFOLLOW` atomic writes and `HostEnvOps` fake-host testing.
- **Built-in PulseAudio Protocol v35 Server (`$XDG_RUNTIME_DIR/pulse/native`):** `lasapi-audiodg` directly hosts a native PA protocol UNIX socket inside the Rust engine ([`pa_server.rs`](rust/salsa_net/src/pa_server.rs)), serving `pactl`, KDE `plasma-pa`, Chromium/Firefox, Discord, Steam/Proton, and Wine with zero `pipewire-pulse` daemon running, backed by [`libsalsa_pa_reconnect.so`](rust/salsa_pa_reconnect/src/lib.rs) and [`salsa_host_dl`](rust/salsa_host_dl/src/lib.rs) self-`dev/ino` recursion guards.
- **LASAPI-to-PulseAudio Stream Capture Bridge (`Direct-on-Input Tap`):** Active native LASAPI / WaveRT-ASIO / ALSA-shim playback sessions automatically register virtual PA `sink_input` entries with lock-free per-session monitor taps so Discord screen share and OBS application audio capture can tap native WaveRT sessions directly.
- **Hollow ABI Shims (`build/shims/`) & Native C Client Seam ([`client_seam.rs`](rust/salsa_net/src/client_seam.rs)):** Provides drop-in replacements for `libasound.so.2`, `libpulse.so.0`, `libpulse-simple.so.0`, `libpulse-mainloop-glib.so.0`, and `libjack.so.0` (`lasapi-run`) alongside immutable `liblasapi.so` C exports and typed `Control IPC` (`LASAPI_CMD_CONTROL_V2`, [`control_ipc.rs`](rust/salsa_net/src/control_ipc.rs)).

### Stage 2: WaveRT Ring Trust Seam (`memfd_create` + `eventfd` + `PodBytes`)
- Audio samples for LASAPI/WaveRT and shimmed apps never cross UNIX domain sockets. Instead, [`wavert_ring.rs`](rust/salsa_net/src/wavert_ring.rs) (`WaveRtRing` / `WaveRtRingWriter` / `WaveRtRingReader`) wraps a `F_SEAL_SHRINK | F_SEAL_GROW`-sealed `memfd` (`lasapi_shm_ring_t`) passed via `SCM_RIGHTS` with sealed `PodBytes` layout enforcement ([`c_abi.rs`](rust/salsa_net/src/c_abi.rs)).
- Committing a 64-frame stereo `float32` block takes **0.075 µs (75 nanoseconds)**—orders of magnitude faster than legacy `pipewire-pulse` socket writes.

### Stage 3: Safe `#![deny(unsafe_code)]` Engine Core (`audiodg_engine.rs`) & 10-Band Dynamic EQ
- [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) compiles under `#![deny(unsafe_code)]` under Linux `SCHED_FIFO` real-time priority 65 (`mlockall`), delegating kernel hardware playback strictly through the [`AlsaPlaybackBackend`](rust/salsa_net/src/alsa_hw.rs) seam (`KernelAlsaBackend` and deterministic `InMemoryAlsaBackend`).
- Supports live Master Clock switching across `44.1 kHz`, `48 kHz`, `88.2 kHz`, `96 kHz`, `176.4 kHz`, and **`192 kHz`** (`64` to `2048` frame quantums), independent `Capture Clock` pinning, and unified `~/.config/lasapi/settings.json` persistence ([`settings.rs`](rust/salsa_net/src/settings.rs)).
- Processes playback through a 10-band RBJ Dynamic EQ (supporting AutoEQ / Oratory1990 profiles and Per-Device EQ Profiles) with envelope compression/expansion, Bauer stereo crossfeed, Analog Stage drive, and a soft-knee mastering limiter.

### Stage 4: Direct Hardware Endpoints, `A2dpCodecAdapter`, DAW MIDI Hub & ScreenCast Broker
- **Render Sink, Hotplug Tracker & `RealBluezBackend`:** Loads optional codecs strictly through [`open_verified_optional_lib`](rust/salsa_host_dl/src/lib.rs) inside [`A2dpCodecAdapter`](rust/salsa_net/src/bluez_ffi.rs) and `MsbcScoTranscoder` (**LDAC** + ABR via `libldacBT_enc.so.2`, **aptX HD** & **aptX** via `libfreeaptx.so.0`, peak-capped CBR **FDK-AAC** at `160 / 128 / 96 kbps` via `libfdk-aac.so.2`, **SBC**, and **HFP** mSBC/CVSD + bidirectional AVRCP volume), paired with kernel `/dev/snd/pcmC*D*`, `Hotplug Tracker`, and daemon-backed `LASAPI_BT_CONN_RESET` ([`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs)).
- **Capture Source & Loopback:** Distributes microphone input and post-EQ desktop loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`) to recording apps in **0.33 ms** round-trip at `192 kHz / 64f`, with live switching via Salsa, the native Wayland `LayerShellQt` System Tray (`lasapi-tray`), or `lasapi-ctl set-sink` / `set-source`.
- **DAW MIDI Hub:** [`midi_hub.rs`](rust/salsa_net/src/midi_hub.rs) loads host ALSA Sequencer via `open_verified_optional_lib`, binds detected hardware MIDI controllers (`/dev/snd/midiC*D*`), and stamps sample-accurate events for JACK MIDI and ALSA Sequencer DAWs.
- **Zero-Downtime Video-Only ScreenCast Broker ([`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs)) & Preview Seqlock ([`preview_shm.rs`](rust/salsa_net/src/preview_shm.rs)):** Adopts systemd socket-activated `$XDG_RUNTIME_DIR/pipewire-0` and `pipewire-0-manager` descriptors from `lasapi-pipewire.socket` before hardware startup without linking `libpipewire`. Wayland Portal ScreenCast sessions run via the consolidated GLib/GObject/GIO loader in [`gio_dbus.rs`](rust/salsa_ui/src/gio_dbus.rs), routing `DmaBuf` (`dmabuf_gl.rs`) and `MemFd` buffers with live `PW_CAST_LIMITS` caps and zero-aliasing [`PreviewShmWriter`](rust/salsa_net/src/preview_shm.rs) / `PreviewShmReader` seqlock thumbnails.

---

## 4. Zero-PipeWire Isolation, Native Wayland UI & Clean Revert Controls
- **Zero-PipeWire Mode + Socket-Activated Video Broker:** `lasapi-ctl enable-auto` masks/stops system `pipewire`, `pipewire-pulse`, and `wireplumber`, enables `lasapi-pipewire.socket` + `lasapi-audiodg.service`, backs up prior configs to `~/.config/lasapi/pw_backup/`, and lets `lasapi-audiodg` host both Native PA v35 (`pulse/native`) and the Video-Only PipeWire ScreenCast socket (`pipewire-0`). Audio admission on `pipewire-0` is strictly blocked so Salsa keeps exclusive audio ownership.
- **100% Native Wayland Qt6 UI Suite:** `salsa-studio`, `salsa-debug`, and `lasapi-tray` run natively on Wayland (`QT_QPA_PLATFORM=wayland;xcb`, zero X11 in core), using runtime `dlopen()` of `libLayerShellQtInterface.so.6` (`zwlr_layer_surface_v1`) so tray popups, menus, and click-through volume OSDs never create panel taskbar icons while falling back cleanly to EWMH hints on X11 or compositors without layer-shell.
- **Instant Revert / Uninstall:** Run `lasapi-ctl disable-auto` at any time to stop `lasapi-audiodg` and `lasapi-pipewire.socket`, restore `~/.config/lasapi/pw_backup/`, and re-enable `pipewire`, `pipewire-pulse`, and `wireplumber`. Run `./install-salsa.sh --uninstall` for complete removal.

---

## 5. Automatic Audio Module Update Watcher & Cross-PC Adaptation (`build/lasapi-autosync`)
- **Systemd Path & Timer Watcher (`lasapi-module-watcher.path` / `.timer`):** Monitors host audio, Bluetooth codec, and D-Bus shared libraries (`libasound.so.2`, `libpulse.so.0`, `libjack.so.0`, `libfdk-aac.so.2`, `libldacBT_enc.so.2`, `libfreeaptx.so.0`, `libdbus-1.so.3`) alongside package-manager hook triggers.
- **SHA-256 Library Fingerprinting & Dynamic ELF Symbol Sync (`readelf -V`):** When triggered, `lasapi-autosync` computes SHA-256 content hashes of the host 64/32-bit audio, codec, and D-Bus `.so` files plus `ALSA_*` / `PULSE_*` ELF symbol version nodes and `lasapi-pipewire.socket` health. Unrelated package updates exit immediately as zero-cost no-ops; only genuine ABI or library changes update `src/alsa.version` / `src/pulse.version`, recompile binaries and shims (`make -B`), and seamlessly restart `lasapi-audiodg.service`.
- **Cross-PC Discovery:** Every binary and script resolves paths dynamically (`readlink("/proc/self/exe")`, `ldconfig -p`, `$XDG_RUNTIME_DIR`), directly probing `org.bluez` and `/dev/snd/{pcmC*D*,midiC*D*}` with zero `wpctl` or hardcoded hardware assumptions.
