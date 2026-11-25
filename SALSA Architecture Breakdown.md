# Salsa — Linux WASAPI Automatic Signal Architecture Breakdown

## 1. Philosophy: Why Salsa LWASAPI Needs Zero Manual Patching
In traditional Linux pro-audio workflows, patchbay utilities like `qpwgraph` or `QjackCtl` require you to manually drag virtual cables between applications, PulseAudio/JACK bridges, resamplers, and hardware sinks. Whenever an application restarts, changes sample rate, or switches APIs, those manual routes frequently disconnect.

**Salsa — Linux WASAPI (`lwasapi`)** eliminates manual patching completely by implementing a **WASAPI / WaveRT Audio Device Graph (`lwasapi-audiodg`)** architecture natively on Linux:
- **100% Automatic Self-Wiring Topology:** Every playback application, microphone capture stream, desktop loopback stream, Discord/OBS per-app capture tap, and hardware MIDI controller is automatically discovered and connected in `0.25 ms` without user intervention.
- **Automatic Multi-Rate Resampling & Split Clocking:** Applications streaming at `44.1 kHz`, `48 kHz`, `88.2 kHz`, `96 kHz`, `176.4 kHz`, or `192 kHz` are automatically resampled and mixed into your high-resolution master clock (up to **`192,000 Hz` / `32-bit Float`**, with optional independent capture clock pinning).

---

## 2. Automatic Signal Flow Topology (`qpwgraph` Equivalent View)

```
┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐
│     1. INPUTS & APPLICATIONS     │       │   2. WAVERT ZERO-COPY ENGINE     │       │  3. SALSA APO & DAW MIDI HUB     │       │  4. HARDWARE & CAPTURE OUTPUTS   │
├──────────────────────────────────┤       ├──────────────────────────────────┤       ├──────────────────────────────────┤       ├──────────────────────────────────┤
│ 🔊 Auto-Recognized Playback Apps │       │ ⚡ WaveRT Shared Memory Ring     │       │ 🎚️ Salsa 10-Band Dynamic EQ APO  │       │ 🎧 Master Hardware Output Sink   │
│  • Native WASAPI (liblwasapi.so) ├──────►│  • memfd_create() 32,768 frames  ├──────►│  • 10 RBJ Parametric EQ Bands    ├──────►│  • BlueZ LDAC/aptX/AAC/SBC/HFP   │
│  • Native PA v35 ($PULSE/native) │ 75 ns │  • Kernel eventfd() wakeups      │       │  • Dynamic Compress / Expand     │       │  • Kernel /dev/snd/pcm* WaveRT   │
│  • Hollow Shims & ~/.asoundrc    │       │  • 0.075 µs atomic commit        │       │  • Bauer Crossfeed & Limiter     │       │  • Smart Hotplug + AVRCP Sync    │
└──────────────────────────────────┘       └────────────────┬─────────────────┘       └────────────────┬─────────────────┘       └──────────────────────────────────┘
                                                            │                                          │ Loopback / PA doi Tap   ┌──────────────────────────────────┐
┌──────────────────────────────────┐       ┌────────────────▼─────────────────┐                        └────────────────────────►│ 📹 Full-Duplex Capture Apps      │
│ 🎙️ Hardware Microphone Input     │       │ 🚀 lwasapi-audiodg RT Thread     │                                                  │  • Discord Voice & App SoundShare│
│  • USB / HDA Kernel Mic Auto-Bind├──────►│  • SCHED_FIFO Priority 65        ├─────────────────────────────────────────────────►│  • OBS Desktop Loopback Monitor  │
│  • Capture Mic APO + RNNoise     │       │  • mlockall() RAM Pinned         │              Mic Capture Ring                    │  • DAW Audio & MIDI Recording    │
└──────────────────────────────────┘       │  • Up to 192,000 Hz / 32-Bit Flt │                                                  └─────────────────▲────────────────┘
                                           └──────────────────────────────────┘                                                                    │
┌──────────────────────────────────┐       ┌──────────────────────────────────┐                                                                    │
│ 🎹 Hardware MIDI Controller      │       │ 🎵 Lock-Free DAW MIDI Hub        │                                                                    │
│  • Auto RawMIDI (/dev/snd/midi*) ├──────►│  • LWASAPI-MIDI-Hub              ├────────────────────────────────────────────────────────────────────┘
│  • RawMIDI + ALSA Sequencer      │       │  • 512-Event Lock-Free Ring      │
└──────────────────────────────────┘       └──────────────────────────────────┘
═════════════════════════════ ZERO-PIPEWIRE WAYLAND VIDEO & SCREENCAST LANE ════════════════════════════
┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐
│ 🖥️ Wayland Video Producer        │       │ 🔌 Socket-Activated PW Broker    │       │ 🎞️ Video Relay & Live Cap        │       │ 📡 ScreenCast Consumers          │
│  • kwin_wayland / wlroots / etc. ├──────►│  • lwasapi-pipewire.socket (0ms) ├──────►│  • SCM_RIGHTS DmaBuf / MemFd FD  ├──────►│  • Discord (AV1 VAAPI) / OBS     │
│  • xdg-desktop-portal Handoff    │       │  • Zero libpipewire / Audio Drop │       │  • Live Cap (PW_CAST_LIMITS)     │       │  • Browsers / KDE MediaMonitor   │
└──────────────────────────────────┘       └──────────────────────────────────┘       └──────────────────────────────────┘       └─────────────────▲────────────────┘
                                                                                                                                             (Parallel A/V via WaveRT)
```

---

## 3. Component-by-Component Owner's Guide

### Stage 1: Automatic Application Recognition, Built-in PA v35 Server & Hollow Shims
- **Built-in PulseAudio Protocol v35 Server (`$XDG_RUNTIME_DIR/pulse/native`):** `lwasapi-audiodg` directly hosts a native PA protocol UNIX socket inside the Rust engine ([`pa_server.rs`](rust/salsa_net/src/pa_server.rs)), serving `pactl`, KDE `plasma-pa`, Chromium/Firefox, Discord, Steam/Proton, and Wine with zero `pipewire-pulse` daemon running, backed by [`libsalsa_pa_reconnect.so`](rust/salsa_pa_reconnect/src/lib.rs) for transparent client reconnects.
- **WASAPI-to-PulseAudio Stream Capture Bridge (`direct_on_input`):** Active native WASAPI / WaveRT-ASIO / ALSA-shim playback sessions automatically register virtual PA `sink_input` entries with lock-free per-session monitor taps so Discord screen share and OBS application audio capture can tap native WaveRT sessions directly.
- **ALSA & OpenAL Soft (`~/.asoundrc` & `~/.config/OpenAL/alsoft.conf`):** Automatically routes default ALSA playback and capture streams through `libasound_module_pcm_lwasapi.so`.
- **Hollow ABI Shims (`build/shims/`):** Provides drop-in replacements for `libasound.so.2`, `libpulse.so.0`, `libpulse-simple.so.0`, `libpulse-mainloop-glib.so.0`, and `libjack.so.0` (`lwasapi-run`) that bypass socket IPC and write directly into LWASAPI shared memory.

### Stage 2: WaveRT Zero-Copy Shared Memory (`memfd_create` + `eventfd`)
- Audio samples for WASAPI/WaveRT and shimmed apps never cross UNIX domain sockets. Instead, `lwasapi-audiodg` maps an anonymous shared-memory ring buffer (`lwasapi_shm_ring_t`) into the client process via `SCM_RIGHTS`.
- Committing a 64-frame stereo `float32` block takes **0.075 µs (75 nanoseconds)**—orders of magnitude faster than legacy `pipewire-pulse` socket writes.

### Stage 3: `lwasapi-audiodg` Real-Time `SCHED_FIFO` Engine, 10-Band Dynamic EQ & Mic APO
- Runs under Linux `SCHED_FIFO` real-time priority 65 with all memory locked into physical RAM (`mlockall`).
- Supports live clock switching across `44.1 kHz`, `48 kHz`, `88.2 kHz`, `96 kHz`, `176.4 kHz`, and **`192 kHz`** (`64` to `2048` frame period quantums), plus independent capture clock pinning.
- Processes playback through a 10-band RBJ parametric equalizer (`31 Hz`–`16 kHz`, supporting AutoEQ / Oratory1990 profiles and per-device EQ memory) with per-band envelope followers for real-time downward compression or upward transient expansion, Bauer stereo headphone crossfeed, hardware analog drive, and a soft-knee mastering limiter, alongside a dedicated Capture Mic APO (noise gate, tonal contour, and optional `librnnoise` suppression).

### Stage 4: Direct Hardware Endpoints, Multi-Codec BlueZ, DAW MIDI Hub & Video ScreenCast Broker
- **Render Sink, Smart Hotplug & Direct BlueZ Audio:** Streams 32-bit float audio directly over `org.bluez.MediaEndpoint1` (**LDAC** + wireless ABR via `libldacBT_enc.so.2`, **aptX HD** & **aptX** via `libfreeaptx.so.0`, **AAC** + ABR via `libfdk-aac.so.2`, **SBC**, and **HFP** mSBC/CVSD + bidirectional AVRCP volume) or kernel `/dev/snd/pcmC*D*` (`S32_LE`, `S24_3LE`, `S16_LE`, `FLOAT_LE`), with `HotplugTracker` automatically switching to newly connected USB/Bluetooth/3.5 mm headphones and restoring your prior speaker output on unplug.
- **Capture Source & Loopback:** Distributes microphone input and post-EQ desktop loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`) to recording apps in **0.33 ms** round-trip at `192 kHz / 64f`, with live switching via Salsa Studio, the native Wayland `LayerShellQt` System Tray (`lwasapi-tray`), or `lwasapi-ctl set-sink` / `set-source`.
- **DAW MIDI Hub:** Automatically binds detected hardware MIDI controllers (`/dev/snd/midiC*D*`) and routes sample-accurate MIDI events to JACK MIDI and ALSA Sequencer DAWs.
- **Zero-Downtime Video-Only ScreenCast Broker ([`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs)):** Adopts systemd socket-activated `$XDG_RUNTIME_DIR/pipewire-0` and `pipewire-0-manager` descriptors from `lwasapi-pipewire.socket` before hardware initialization (with automatic standalone bind fallback on non-systemd setups) using pure Rust protocol handling (zero `libpipewire` / `libspa` linkage). Wayland `xdg-desktop-portal` ScreenCast streams route `DmaBuf` and `MemFd` buffers to Discord, OBS Studio, browsers, and KDE `libKPipeWire` (`MediaMonitor`) with zero restart disconnects, live Resolution/FPS caps (`PW_CAST_LIMITS`), and zero-overhead Preview HUD telemetry (`PW_PREVIEW`).

---

## 4. Zero-PipeWire Isolation, Native Wayland UI & Clean Revert Controls
- **Zero-PipeWire Mode + Socket-Activated Video Broker:** `lwasapi-ctl enable-auto` masks/stops system `pipewire`, `pipewire-pulse`, and `wireplumber`, enables `lwasapi-pipewire.socket` + `lwasapi-audiodg.service`, backs up prior configs to `~/.config/lwasapi/pw_backup/`, and lets `lwasapi-audiodg` host both Native PA v35 (`pulse/native`) and the Video-Only PipeWire ScreenCast socket (`pipewire-0`). Audio admission on `pipewire-0` is strictly blocked so Salsa keeps exclusive audio ownership.
- **100% Native Wayland Qt6 UI Suite:** `salsa-studio`, `salsa-debug`, and `lwasapi-tray` run natively on Wayland (`QT_QPA_PLATFORM=wayland;xcb`, zero X11 in core), using runtime `dlopen()` of `libLayerShellQtInterface.so.6` (`zwlr_layer_surface_v1`) so tray popups, menus, and click-through volume OSDs never create panel taskbar icons while falling back cleanly to EWMH hints on X11 or compositors without layer-shell.
- **Instant Revert / Uninstall:** Run `lwasapi-ctl disable-auto` at any time to stop `lwasapi-audiodg` and `lwasapi-pipewire.socket`, restore `~/.config/lwasapi/pw_backup/`, and re-enable `pipewire`, `pipewire-pulse`, and `wireplumber`. Run `./install-salsa.sh --uninstall` for complete removal.

---

## 5. Automatic Audio Module Update Watcher & Cross-PC Adaptation (`build/lwasapi-autosync`)
- **Systemd Path & Timer Watcher (`lwasapi-module-watcher.path` / `.timer`):** Monitors host audio, Bluetooth codec, and D-Bus shared libraries (`libasound.so.2`, `libpulse.so.0`, `libjack.so.0`, `libfdk-aac.so.2`, `libldacBT_enc.so.2`, `libfreeaptx.so.0`, `libdbus-1.so.3`) alongside package-manager hook triggers.
- **SHA-256 Library Fingerprinting & Dynamic ELF Symbol Sync (`readelf -V`):** When triggered, `lwasapi-autosync` computes SHA-256 content hashes of the host 64/32-bit audio, codec, and D-Bus `.so` files plus `ALSA_*` / `PULSE_*` ELF symbol version nodes and `lwasapi-pipewire.socket` health. Unrelated package updates exit immediately as zero-cost no-ops; only genuine ABI or library changes update `src/alsa.version` / `src/pulse.version`, recompile binaries and shims (`make -B`), and seamlessly restart `lwasapi-audiodg.service`.
- **Cross-PC Discovery:** Every binary and script resolves paths dynamically (`readlink("/proc/self/exe")`, `ldconfig -p`, `$XDG_RUNTIME_DIR`), directly probing `org.bluez` and `/dev/snd/{pcmC*D*,midiC*D*}` with zero `wpctl` or hardcoded hardware assumptions.
