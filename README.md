# Salsa — Linux WASAPI Portable Installer Package

"**"You can't spell Salsa without ALSA — because even the cleanest audio chips sound flat until they have a zero-latency dip to taste, with zero chunks dropped and plenty of Dynamic EQ heat!"**

High-Resolution (Up to **192,000 Hz / 32-Bit Float**) Windows WASAPI / WaveRT Zero-Copy Audio Engine, **100% Pure Rust Backend Daemon & CLI (`lwasapi-audiodg` / `lwasapi-ctl`), PA v35 Server & PipeWire-0 Video Broker**, **Direct BlueZ D-Bus A2DP/AVRCP Audio**, **10-Band Real-Time Dynamic EQ APO**, Sample-Accurate **DAW MIDI Hub**, and Automatic **`qpwgraph`-Equivalent Signal Topology & Owner's Manual**, **Designed to remove all PipeWire-Pulse or WirePlumber audio dependencies**.  

---

## Quick 1-Command Install on Any Linux PC

Unzip the archive (for example `V1.19.1` from [VERSION](VERSION)) and run `./install-salsa.sh`. Full chronological change history is available in [PATCH_NOTES.md](PATCH_NOTES.md).

```bash
unzip "salsa-lwasapi-installer (V1.19.1).zip"
cd salsa-lwasapi
./install-salsa.sh
```

### What `./install-salsa.sh` Does Automatically:
1. **Detects Host PC Hardware & Distro**: Identifies your CPU (`/proc/cpuinfo`), Linux distribution (`CachyOS`/Arch `pacman`, Ubuntu/Debian `apt`/`dpkg`, Fedora/RHEL/openSUSE `dnf`/`zypper`/`rpm`), kernel ALSA soundcards (`/proc/asound/cards`), Bluetooth A2DP headphones (`org.bluez`), USB/analog capture inputs, and hardware MIDI controllers (`/dev/snd/midiC*D*`).
2. **Installs Any Missing Build/GUI Prerequisites**: Checks for `cargo`, `gcc`, `g++`, `make`, `pkg-config`, `readelf`, `libz`/`alsa`/`libpulse` runtime libraries, and `Qt6` (`libQt6Widgets.so.6`, `libQt6DBus.so.6` from `qt6-base`) for the native C++20 Qt6 + pure-Rust GUIs (`build/salsa-studio`, `build/salsa-debug`, `build/lwasapi-tray`), installing system prerequisites cleanly when needed.
3. **Syncs Host ELF Symbol Versions & Compiles Natively (`build/lwasapi-autosync`)**: Extracts the target PC's exact `ALSA_*` and `PULSE_*` ELF symbol versions from `libasound.so.2` and `libpulse.so.0` and builds `lwasapi-audiodg`, `liblwasapi.so`, `libasound_module_pcm_lwasapi.so`, `build/pa_reconnect/libpulse.so.0` (plus `build/pa_reconnect32/libpulse.so.0` when 32-bit multilib is present), `lwasapi-ctl`, `lwasapi-test`, `lwasapi-bench`, and all hollow ABI shims (`build/shims/`).
4. **Cuts Over Audio Directly to Salsa (`lwasapi-ctl enable-auto`)**:
   - Configures Pure Zero-PipeWire isolation with built-in ScreenCast `$XDG_RUNTIME_DIR/pipewire-0` video broker, `~/.asoundrc`, `~/.config/OpenAL/alsoft.conf`, `~/.config/environment.d/99-salsa-direct.conf`, native `$XDG_RUNTIME_DIR/pulse/native` PA v35 UNIX socket server, persistent `0600` `~/.config/lwasapi/app_volumes.conf`, and direct BlueZ `MediaEndpoint1` (**LDAC / aptX HD / aptX / AAC / SBC**) + kernel `/dev/snd/pcmC*D*`.
   - Applies scoped Flatpak `--user` overrides (`--socket=pulseaudio`, `--filesystem=xdg-run/pipewire-0:ro`) for installed gaming/media Flatpaks and tracks them for clean `--uninstall` rollback.
5. **Configures & Starts Systemd User Services & Shortcuts**:
   - `lwasapi-audiodg.service` (Real-time `SCHED_FIFO` WaveRT audio, PA v35 server, direct BlueZ, MIDI hub, & live debug telemetry socket)
   - `lwasapi-tray.service` (System tray controller for Pure Salsa Audio)
   - `lwasapi-module-watcher.path` (Automatic self-healing rebuild trigger whenever system libraries or packages update; optional root-safe ALPM trigger via `./install-salsa.sh --install-pacman-hook`)
   - Application Launcher shortcuts (`~/.local/share/applications/`): **Salsa — Linux WASAPI Studio** (`salsa-lwasapi.desktop`), **Salsa — Live Interaction Debugger** (`salsa-debug.desktop`), and **Salsa — Audio System Tray** (`lwasapi-tray.desktop`)
   - CLI Commands in `~/.local/bin/`: `salsa`, `salsa-graph`, `salsa-debug`, `lwasapi-ctl`, `lwasapi-run`, `lwasapi-bench`

---

## Useful Commands

- **Open Salsa Studio Mixer, Device Switcher, Live Debugger Tab & 10-Band Dynamic EQ GUI**:
  ```bash
  salsa
  ```
- **Open Automatic Signal Topology Graph (`qpwgraph` View) & Owner's Manual**:
  ```bash
  salsa-graph
  ```
- **Open Standalone Live Interaction Debugger or Run CLI Audits/Probes**:
  ```bash
  salsa-debug
  salsa-debug --snapshot
  salsa-debug --audit
  salsa-debug --probe --tone --midi
  salsa-debug --stream --lvl INFO
  ```
- **Inspect Engine Status, List Endpoints, or Control Devices & Per-App Streams**:
  ```bash
  lwasapi-ctl status
  lwasapi-ctl devices
  lwasapi-ctl set-sink <device-id|auto>
  lwasapi-ctl set-source <device-id|auto>
  lwasapi-ctl app-volume <session_id> <0-150>
  lwasapi-ctl app-mute <session_id> <0|1|toggle>
  lwasapi-ctl move-app <session_id> <device-id|auto>
  ```
- **Run Automated Regression/Security Suite & Live Latency Benchmark**:
  ```bash
  make test
  lwasapi-bench
  ```
- **Rebuild the Portable Installer Archive (`salsa-lwasapi-installer.zip`)**:
  ```bash
  make installer
  ```
- **Inspect Host PC Hardware Readiness Without Installing**:
  ```bash
  ./install-salsa.sh --check
  ```
- **Temporarily Revert Audio Ownership Back to Stock PipeWire-Pulse / WirePlumber**:
  ```bash
  lwasapi-ctl disable-auto
  ```
- **Full Uninstall & Clean Restore of Stock PipeWire-Pulse / WirePlumber / ALSA**:
  ```bash
  ./install-salsa.sh --uninstall
  ```

---

## Contributors

- **Linuxknows** — Creator & Maintainer
- **Antigravity (Google DeepMind — Gemini 4 Hydrogen)** — AI Pair-Programming Contributor
