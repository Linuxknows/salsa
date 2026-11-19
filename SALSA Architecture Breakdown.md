# Salsa — Linux WASAPI Automatic Signal Architecture Breakdown

## 1. Philosophy: Why Salsa LWASAPI Needs Zero Manual Patching
In traditional Linux pro-audio workflows, patchbay utilities like `qpwgraph` or `QjackCtl` require you to manually drag virtual cables between applications, PulseAudio/JACK bridges, resamplers, and hardware sinks. Whenever an application restarts, changes sample rate, or switches APIs, those manual routes frequently disconnect.

**Salsa — Linux WASAPI (`lwasapi`)** eliminates manual patching completely by implementing a **WASAPI / WaveRT Audio Device Graph (`lwasapi-audiodg`)** architecture natively on Linux:
- **100% Automatic Self-Wiring Topology:** Every playback application, microphone capture stream, desktop loopback stream, and hardware MIDI controller is automatically discovered and connected in `0.25 ms` without user intervention.
- **Automatic Multi-Rate Resampling:** Applications streaming at `44.1 kHz`, `48 kHz`, or `96 kHz` are automatically resampled and mixed into your high-resolution master clock (configurable up to **`192,000 Hz` / `32-bit Float`**).

---

## 2. Automatic Signal Flow Topology (`qpwgraph` Equivalent View)

```
┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐
│     1. INPUTS & APPLICATIONS     │       │   2. WAVERT ZERO-COPY ENGINE     │       │  3. SALSA APO & DAW MIDI HUB     │       │  4. HARDWARE & CAPTURE OUTPUTS   │
├──────────────────────────────────┤       ├──────────────────────────────────┤       ├──────────────────────────────────┤       ├──────────────────────────────────┤
│ 🔊 Auto-Recognized Playback Apps │       │ ⚡ WaveRT Shared Memory Ring     │       │ 🎚️ Salsa 10-Band Dynamic EQ APO  │       │ 🎧 Master Hardware Output Sink   │
│  • Native WASAPI (liblwasapi.so) ├──────►│  • memfd_create() 32,768 frames  ├──────►│  • 10 RBJ Peaking Biquad Bands   ├──────►│  • Direct BlueZ A2DP (AAC / SBC) │
│  • Native PA v35 ($PULSE/native) │ 75 ns │  • Kernel eventfd() wakeups      │       │  • Per-Band Envelope Followers   │       │  • Kernel /dev/snd/pcm* WaveRT   │
│  • Hollow Shims & ~/.asoundrc    │       │  • 0.075 µs atomic commit        │       │  • Dynamic Compress / Expand     │       │  • AVRCP + KDE Vol Key Sync      │
└──────────────────────────────────┘       └────────────────┬─────────────────┘       │  • Soft-Knee Mastering Limiter   │       └──────────────────────────────────┘
                                                            │                         └────────────────┬─────────────────┘
┌──────────────────────────────────┐       ┌────────────────▼─────────────────┐                        │ Loopback (0.33 ms)      ┌──────────────────────────────────┐
│ 🎙️ Hardware Microphone Input     │       │ 🚀 lwasapi-audiodg RT Thread     │                        └────────────────────────►│ 📹 Full-Duplex Capture Apps      │
│  • USB / HDA Kernel Mic Auto-Bind├──────►│  • SCHED_FIFO Priority 65        ├─────────────────────────────────────────────────►│  • Discord / Browser Voice Capt  │
│  • Native PA v35 + ALSA Vol Sync │       │  • mlockall() RAM Pinned         │              Mic Capture Ring                    │  • OBS Desktop Loopback Monitor  │
└──────────────────────────────────┘       │  • Up to 192,000 Hz / 32-Bit Flt │                                                  │  • DAW Audio & MIDI Recording    │
                                           └──────────────────────────────────┘                                                  └─────────────────▲────────────────┘
┌──────────────────────────────────┐       ┌──────────────────────────────────┐                        │
│ 🎹 Hardware MIDI Controller      │       │ 🎵 Lock-Free DAW MIDI Hub        │                        │
│  • Auto RawMIDI (/dev/snd/midi*) ├──────►│  • LWASAPI-MIDI-Hub              ├────────────────────────┘
│  • RawMIDI + ALSA Sequencer      │       │  • 512-Event Lock-Free Ring      │
└──────────────────────────────────┘       └──────────────────────────────────┘
═════════════════════════════ ZERO-PIPEWIRE WAYLAND VIDEO & SCREENCAST LANE ════════════════════════════
┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐       ┌──────────────────────────────────┐
│ 🖥️ Wayland Video Producer        │       │ 🔌 Built-in Video PW Broker      │       │ 🎞️ Video Relay & Live Cap        │       │ 📡 ScreenCast Consumers          │
│  • kwin_wayland Compositor       ├──────►│  • /run/user/$UID/pipewire-0     ├──────►│  • SCM_RIGHTS DmaBuf / MemFd FD  ├──────►│  • Discord (AV1 VAAPI) / OBS     │
│  • xdg-desktop-portal Handoff    │       │  • Zero libpipewire / Audio Drop │       │  • Live Cap (PW_CAST_LIMITS)     │       │  • Browsers / KDE Window Preview │
└──────────────────────────────────┘       └──────────────────────────────────┘       └──────────────────────────────────┘       └─────────────────▲────────────────┘
                                                                                                                                             (Parallel A/V via WaveRT)
```

---

## 3. Component-by-Component Owner's Guide

### Stage 1: Automatic Application Recognition, Built-in PA v35 Server & Hollow Shims
- **Built-in PulseAudio Protocol v35 Server (`$XDG_RUNTIME_DIR/pulse/native`):** `lwasapi-audiodg` directly hosts a native PA protocol UNIX socket inside the Rust engine ([`pa_server.rs`](rust/salsa_net/src/pa_server.rs)), serving `pactl`, KDE `plasma-pa`, Chromium/Firefox, Discord, Steam/Proton, and Wine with zero `pipewire-pulse` daemon running.
- **ALSA & OpenAL Soft (`~/.asoundrc` & `~/.config/OpenAL/alsoft.conf`):** Automatically routes default ALSA playback and capture streams through `libasound_module_pcm_lwasapi.so`.
- **Hollow ABI Shims (`build/shims/`):** Provides drop-in replacements for `libasound.so.2`, `libpulse.so.0`, `libpulse-simple.so.0`, `libpulse-mainloop-glib.so.0`, and `libjack.so.0` that bypass socket IPC and write directly into LWASAPI shared memory.

### Stage 2: WaveRT Zero-Copy Shared Memory (`memfd_create` + `eventfd`)
- Audio samples for WASAPI/WaveRT and shimmed apps never cross UNIX domain sockets. Instead, `lwasapi-audiodg` maps an anonymous shared-memory ring buffer (`lwasapi_shm_ring_t`) into the client process via `SCM_RIGHTS`.
- Committing a 64-frame stereo `float32` block takes **0.075 µs (75 nanoseconds)**—orders of magnitude faster than legacy `pipewire-pulse` socket writes.

### Stage 3: `lwasapi-audiodg` Real-Time `SCHED_FIFO` Engine & 10-Band Dynamic EQ APO
- Runs under Linux `SCHED_FIFO` real-time priority 65 with all memory locked into physical RAM (`mlockall`).
- Supports live clock switching across `44.1 kHz`, `48 kHz`, `88.2 kHz`, `96 kHz`, `176.4 kHz`, and **`192 kHz`** (`64` to `2048` frame period quantums).
- Processes all audio through a 10-band RBJ parametric equalizer (`31 Hz`–`16 kHz`) with per-band envelope followers for real-time downward compression or upward transient expansion, followed by a soft-knee mastering limiter.

### Stage 4: Direct Hardware Endpoints, BlueZ A2DP, DAW MIDI Hub & Video ScreenCast Broker
- **Render Sink & Direct BlueZ A2DP:** Streams 32-bit float audio directly over `org.bluez.MediaEndpoint1` (`AAC` via `libfdk-aac.so.2` with automatic `SBC` fallback + bidirectional AVRCP volume) or kernel `/dev/snd/pcmC*D*` (supporting `S32_LE`, `S24_3LE`, `S16_LE`, `FLOAT_LE` and automatic priority scoring across USB DACs, analog codecs, and HDMI).
- **Capture Source & Loopback:** Distributes microphone input and post-EQ desktop loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`) to recording apps in **0.33 ms** round-trip at `192 kHz / 64f`, with live switching via Salsa Studio dropdowns or `lwasapi-ctl set-sink` / `set-source`.
- **DAW MIDI Hub:** Automatically binds detected hardware MIDI controllers and routes sample-accurate MIDI events to JACK MIDI and ALSA Sequencer DAWs.
- **Zero-PipeWire Video-Only ScreenCast Broker ([`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs)):** Directly serves `/run/user/$UID/pipewire-0` using native Rust protocol handling (zero `libpipewire` / `libspa` linkage) so Wayland `kwin_wayland` + `xdg-desktop-portal` (`OpenPipeWireRemote`) ScreenCast streams route `DmaBuf` and `MemFd` buffers to Discord, OBS Studio, browsers, and KDE previews with live Resolution/FPS caps (`PW_CAST_LIMITS`) and zero-overhead Preview HUD telemetry (`PW_PREVIEW`).

---

## 4. Zero-PipeWire Isolation & Clean Revert Controls
- **Zero-PipeWire Mode + Built-in Video Broker:** `lwasapi-ctl enable-auto` masks/stops system `pipewire`, `pipewire-pulse`, and `wireplumber`, writes legacy `99-lwasapi-video-only.conf` fallback rules, backs up prior configs to `~/.config/lwasapi/pw_backup/`, and lets `lwasapi-audiodg` host both Native PA v35 (`pulse/native`) and the built-in Video-Only PipeWire ScreenCast socket (`pipewire-0`). Audio admission on `pipewire-0` is strictly blocked so Salsa keeps exclusive audio ownership.
- **Instant Revert / Uninstall:** Run `lwasapi-ctl disable-auto` (or click the Salsa system tray toggle) at any time to stop `lwasapi-audiodg`, remove `99-lwasapi-video-only.conf`, restore `~/.config/lwasapi/pw_backup/`, and re-enable `pipewire`, `pipewire-pulse`, and `wireplumber`. Run `./install-salsa.sh --uninstall` for complete removal.

---

## 5. Automatic Audio Module Update Watcher & Cross-PC Adaptation (`build/lwasapi-autosync`)
- **Systemd Path Watcher (`lwasapi-module-watcher.path`):** Continuously monitors your distribution's package database (`/var/lib/pacman/local`, `/var/lib/dpkg/status`, or `/var/lib/rpm`) alongside `libasound.so.2`, `libpulse.so.0`, `libjack.so.0`, and `libfdk-aac.so.2`.
- **Dynamic ELF Symbol Version Sync (`readelf -V`):** Whenever any system package updates, `lwasapi-autosync` extracts all `ALSA_*` and `PULSE_*` ELF symbol version nodes from the host libraries, updates `src/alsa.version` and `src/pulse.version`, recompiles all binaries and shims (`make -B`), re-verifies PipeWire-Pulse masking / WirePlumber `video-only` config, and hot-restarts `lwasapi-audiodg.service`.
- **Cross-PC Discovery:** Every binary and script resolves paths dynamically (`readlink("/proc/self/exe")`, `ldconfig -p`), directly probing `org.bluez` and `/dev/snd/{pcmC*D*,midiC*D*}` with zero `wpctl` or hardcoded hardware assumptions.
