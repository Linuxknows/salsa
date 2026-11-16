# Salsa — Linux WASAPI (`lwasapi`) Patch Notes

Chronological changelog organized by installer revision.

---

## Rev 6.6 — Current (`2026-11-16`)

### 1. Multi-Window Studio Flow: Topology Intro, Pop-Out Advanced Options & Standalone Debugger
- Split the Auto-Graph Signal Topology & Owner's Manual view into its own standalone startup intro window (`_build_topology_intro_window` in [`salsa-gui.py`](salsa-gui.py)) with a `"Don't show Topology Intro on startup"` persistence toggle (`show_topology_intro`), `"Enter Studio Mixer ->"` button, and top-bar `"Topology (F1)"` toggle to reopen anytime (or via `salsa-graph`).
- Removed the embedded `SalsaLiveDebuggerWidget` tab and `QTabWidget` wrapper from [`salsa-gui.py`](salsa-gui.py); [`salsa-debug.py`](salsa-debug.py) now launches strictly as its own separate standalone program/window via **Advanced Options**, while Topology keeps lightweight async Video PW Broker detection via `snapshot_once`.
- Moved `Hardware Analog Stage`, `Engine Ownership Toggle` (`Salsa Native` / `PipeWire`), `Send Test MIDI (C4)`, `Run Latency Benchmark`, `Run System Health Audit`, `Sync Audio Modules Now`, `Restart Audio Engine` & `Restart Tray Service`, `Open Custom Theme Studio`, `Reset Safe Defaults`, `Soft-Knee Limiter`, `Per-Device EQ`, `Import AutoEQ .txt`, and `Bauer Crossfeed` into the dedicated pop-out **Advanced Options** window (`_build_advanced_options_window`).

### 2. Customizable Keyboard Shortcuts (GUI & Standalone Debugger)
- Added customizable `QKeySequenceEdit` key-bind rows inside **Advanced Options** for 14 Studio/system actions (`DEFAULT_SHORTCUTS`, conflict auto-clearing, per-action clear, and `Restore Defaults`), persisted under `"shortcuts"` in `gui_settings.json` with live shortcut/tooltip rebinding (`_setup_shortcuts`).
- Added keyboard navigation shortcuts in [`salsa-debug.py`](salsa-debug.py) (`Ctrl+1..5` sub-tabs, `F5` refresh, `Ctrl+W`/`Ctrl+Q` close) and expanded multi-window + shortcut regression assertions in [`scripts/run_regression_tests.py`](scripts/run_regression_tests.py).

---

## Rev 6.5 (`2026-11-16`)

### 1. Independent Capture / Mic Clock & Zero-Double-Resample Capture Pipeline
- Split capture clock control (`LWASAPI_SetCaptureSampleRate`, `lwasapi-ctl set-cap-rate <44100..192000|auto>`) across [`include/lwasapi.h`](include/lwasapi.h), [`src/liblwasapi.c`](src/liblwasapi.c), [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c), [`src/lwasapi_bluez.c`](src/lwasapi_bluez.c), and [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c) so USB microphones (e.g., Focusrite Scarlett 2i2 at `48,000 Hz` for Discord/voice) run independently of a `192,000 Hz` playback Master Clock.
- Upgraded capture audio path with [`lwasapi_bluez_read_capture_native`](include/lwasapi_bluez.h) and [`lwasapi_pa_server_push_capture`](include/lwasapi_pa_server.h) so hardware capture, Mic APO (`HPF`, gate, `48 kHz` RNNoise, compressor, clarity), and native `48 kHz` PulseAudio record clients stream `1:1` without double-resampling, using Catmull-Rom upsampling only when high-rate playback sidetone or `192 kHz` capture streams are active.

### 2. USB Capture Telemetry Watchdog, MIDI Key-Press Counter & Quick Mic APO Presets
- Added capture rate telemetry (`pref_rate`, `measured_rate`, `kick_count`, `is_usb`, `duplex_locked`) to [`include/lwasapi_debug.h`](include/lwasapi_debug.h), [`src/lwasapi_debug.c`](src/lwasapi_debug.c), and [`salsa_debug_ui.py`](salsa_debug_ui.py).
- Updated [`include/lwasapi_midi.h`](include/lwasapi_midi.h) and [`src/lwasapi_midi.c`](src/lwasapi_midi.c) so routed MIDI telemetry counts actual Note-On key-press events (`0x90..0x9F` with non-zero velocity) instead of idle MIDI clock pulses.
- Added quick reversible Mic APO presets (`lwasapi-ctl mic-preset <discord|broadcast|clean|bypass>`, plus `cmb_mic_preset` / `cmb_mic_preset_tray` and `cmb_cap_rate` selectors in [`salsa-gui.py`](salsa-gui.py) and [`lwasapi-tray.py`](lwasapi-tray.py)), centralized shared ctypes bindings/presets into [`salsa_ipc.py`](salsa_ipc.py), retired legacy Bose PipeWire workaround restore logic in `disable-auto`, and expanded live regression tests in [`scripts/run_regression_tests.py`](scripts/run_regression_tests.py).

---

## Rev 6.3 (`2026-11-14`)

### 1. Periodic Full-Duplex Static & Cross-Period Continuity Fixes
- Eliminated periodic full-duplex playback and capture static/clicks in [`src/lwasapi_pa_server.c`](src/lwasapi_pa_server.c) and [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c) using rate-scaled ~0.5 ms de-click fade-outs (`pa_stream_emit_fade_out`) on flush/cork/underrun transitions, Catmull-Rom cubic history continuity (`pa_stream_save_hist`), per-frame capture gain smoothing (`cap_smooth_gain`), and cross-period boundary linear interpolation (`cap_hist_l`/`cap_hist_r` with `clamp_cap_unit`) in `lwasapi_pa_server_push_capture`.

### 2. Full Restart State Persistence (Daemon, Studio GUI & System Tray)
- Upgraded daemon binary persistence in [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c) to `'SALW'` v5 (`salsa_persisted_state_v5_t`) stored at `0600` in `~/.config/lwasapi/salsa_state.bin` with 400 ms debounced `g_state_dirty` flushes and clean-shutdown saving, restoring `desktop_volume_pct`, `desktop_muted`, `desktop_mic_volume_pct`, `desktop_mic_muted`, and `desktop_mic_gain_ppm` alongside clock, routing, EQ, Mic APO, BT codec, and HW Analog settings.
- Updated `SalsaStudioWindow` in [`salsa-gui.py`](salsa-gui.py) to persist and restore `window_width`, `window_height`, and `last_tab` via `save_gui_settings` (preserving disk `tray_*` keys), and synchronized `eq_per_device` / `eq_per_device_enabled` and `chk_eq_per_dev` across [`salsa-gui.py`](salsa-gui.py) and [`lwasapi-tray.py`](lwasapi-tray.py) using `_read_gui_settings_mtime`.

### 3. Virtual Mix Bus Removal & Simplified Tab-Bar Controls (`Parametric Band` Strip Removed)
- Removed the 4 Virtual Broadcast Mix Buses (`salsa_bus.*`, `LWASAPI_CMD_SET_MIX_BUS`, `LWASAPI_SetMixBus`, `lwasapi_mix_bus_info_t`, `mix-bus` CLI, virtual bus PA sinks/monitors, and the `VIRTUAL BROADCAST MIX BUSES` GUI panel) across [`include/lwasapi.h`](include/lwasapi.h), [`include/lwasapi_pa_server.h`](include/lwasapi_pa_server.h), [`src/liblwasapi.c`](src/liblwasapi.c), [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c), [`src/lwasapi_pa_server.c`](src/lwasapi_pa_server.c), [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c), [`lwasapi-tray.py`](lwasapi-tray.py), and [`salsa-gui.py`](salsa-gui.py) while retaining per-app `play:`/`rec:` routing and `--virtual` fallback, and updated [`VERSION`](VERSION) to `6.3`.
- Removed the `Parametric Band:` / `Freq:` / `Q:` / `Shape:` widgets (`cmb_param_band`, `spn_param_freq`, `spn_param_q`, `cmb_param_shape`) from `TabsCornerSlot` in [`salsa-gui.py`](salsa-gui.py), keeping **Bauer Crossfeed** (`chk_crossfeed`, `sld_crossfeed`, `cmb_crossfeed_cut`), `Import AutoEQ .txt`, and backend `param-eq` CLI / `LWASAPI_SetParametricEQ` DSP support intact.

### 4. Fixed Master Clock Restoration & Bit Depth Switching / Hardware Lock Feedback
- Restored Rev 6.0's fixed **Master Sample Rate** selector (`44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz` via `LWASAPI_SetEngineConfig`) across [`include/lwasapi.h`](include/lwasapi.h), [`src/liblwasapi.c`](src/liblwasapi.c), [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c), [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c), [`lwasapi-tray.py`](lwasapi-tray.py), [`salsa-gui.py`](salsa-gui.py), [`src/lwasapi_test.c`](src/lwasapi_test.c), and [`scripts/run_regression_tests.py`](scripts/run_regression_tests.py), removing Adaptive Lossless auto-rate switching while preserving `salsa_state.bin` (`'SALW'` v5) compatibility and Bluetooth Wireless ABR (`wireless_abr_enabled`).
- Fixed **Bit Depth / Precision** switching and persistence (`g_pref_format_bits`) in [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c) so `reconfigure_engine_rate_impl` detects `bits_changed`, `open_kernel_pcm_device` negotiates ALSA formats ordered by requested bit depth (with `SNDRV_PCM_FORMAT_S24_LE` encoding support alongside `S16_LE`, `S24_3LE`, `S32_LE`, and `FLOAT_LE`) and steps fallback rates down within the same sample-rate family (`192→96→48 kHz`, `176.4→88.2→44.1 kHz`), and Virtual / BlueZ endpoints update and persist requested `16 / 24 / 32`-bit precision.
- Added `/proc/asound/card*/stream0` playback capability probing plus clear `HW Locked` status/tooltip feedback and `(DAC N/A)` dropdown annotations for unsupported hardware rates and bit depths in [`salsa-gui.py`](salsa-gui.py), [`lwasapi-tray.py`](lwasapi-tray.py), and `cmd_set_rate` in [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c).

---

## Rev 6.2 (`2026-11-14`)

### 1. Hardware Analog Architecture, DAC Unity Attenuation, Amp Stage & USB-HID Control Box Support
- Added [`include/lwasapi_hw_analog.h`](include/lwasapi_hw_analog.h) and [`src/lwasapi_hw_analog.c`](src/lwasapi_hw_analog.c) with non-invasive kernel card/mixer probing (`lwasapi_hw_analog_probe`), safe ALSA control syncing (`lwasapi_hw_analog_sync` via `SNDRV_CTL_IOCTL_ELEM_WRITE`), DAC unity volume handling (`lwasapi_hw_analog_is_dac_unity_active`), analog soft-knee amp boost (`lwasapi_hw_analog_softknee_active`), capture makeup (`lwasapi_hw_analog_get_cap_makeup_linear`), ASUS Strix Raid DLX/Soar CM6632AE UAC2 + `/dev/hidraw*` USB-HID volume/mute dial polling (`lwasapi_hw_analog_poll_hid`), and card scoring (`lwasapi_hw_analog_score_card`).
- Integrated `LWASAPI_SetHwAnalog` and `lwasapi-ctl hw-analog` controls and telemetry into [`include/lwasapi.h`](include/lwasapi.h), [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c), [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c), [`salsa-gui.py`](salsa-gui.py), [`src/lwasapi_test.c`](src/lwasapi_test.c), and [`scripts/run_regression_tests.py`](scripts/run_regression_tests.py).

---

## Rev 6.1 (`2026-11-14`)

### 1. Adaptive Lossless Clock Default + Master Resample Override + Wireless Bitrate Adaptation
- Added Adaptive Lossless stream rate & depth tracking (`adaptive_rate_enabled = 1` default) in [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c) and [`src/lwasapi_pa_server.c`](src/lwasapi_pa_server.c), automatically matching the dominant active playback rate (`44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz`) while keeping **Master Resample Override** (`LWASAPI_SetEngineAdaptive` / `lwasapi-ctl set-rate <hz|auto>`) one click away.
- Added automatic **Wireless Bitrate Adaptation (ABR)** in [`src/lwasapi_bluez.c`](src/lwasapi_bluez.c) scaling LDAC (`990 <-> 660 <-> 330 kbps` via `libldacBT_abr.so.2` / `ldacBT_set_eqmid`) and AAC (`256 <-> 192 <-> 128 kbps`) dynamically on L2CAP write stalls and link recovery.

### 2. Active BlueZ Codec Switcher, Diagnostics & Optional Two-Way HFP `BTPROTO_SCO`
- Added remote SEP probing (`supported_codecs_mask`), codec negotiation diagnostics (`neg_reason`), on-demand preferred codec switching (`Auto | LDAC | aptX HD | aptX | AAC | SBC`) with LDAC bitrate tier selection (`ABR | 990 | 660 | 330 kbps`), and optional two-way Bluetooth HFP `BTPROTO_SCO` (`mSBC`/`CVSD`) headset mic support (`hfp_sco_enabled = 0` by default so Focusrite Scarlett remains default) across [`src/lwasapi_bluez.c`](src/lwasapi_bluez.c), [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c), [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c), [`salsa-gui.py`](salsa-gui.py), and [`lwasapi-tray.py`](lwasapi-tray.py).

### 3. Full Parametric EQ, AutoEQ / Oratory1990 `.txt` Importer & Bauer Headphone Crossfeed
- Upgraded the real-time 10-band APO in [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c) (`LWASAPI_SetParametricEQ`) with per-band frequency (`15 Hz–22 kHz`), custom `Q` (`0.2–10.0`), filter topology (`Peak`, `Low-Shelf`, `High-Shelf`), and bypassable Bauer/BS2B stereo crossfeed (`process_crossfeed`), plus an EqualizerAPO / Oratory1990 `.txt` importer and Parametric Band Inspector in [`salsa-gui.py`](salsa-gui.py).

### 4. Real-Time Capture Microphone APO & Virtual Broadcast Mix Channels
- Added a bypassable Capture Microphone APO (`enabled = 0` by default so hardware mics like Focusrite Scarlett 2i2 pass raw) in [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c) combining Voice HPF, hysteresis Noise Gate, Neural `librnnoise.so.0` denoising (with spectral fallback), Clarity shelf, Soft Compressor, and post-loopback low-latency headphone Sidetone (echo-free for OBS/loopback).
- Added 4 Virtual Broadcast Mix Channels (`salsa_bus.game`, `salsa_bus.chat`, `salsa_bus.browser`, `salsa_bus.music` + `.monitor` sources) in [`src/lwasapi_pa_server.c`](src/lwasapi_pa_server.c) with persisted `0600` faders, mutes, live meters, optional auto-routing, `'SALU'` v3 `0600` atomic engine persistence, and updated Tab 2 Signal Topology & Owner's Manual ([`salsa-gui.py`](salsa-gui.py)).

---

## Rev 6.0 (`2026-11-14`)

### 1. KDE Plasma 6 System Tray, Emerald Glow Icon, Swipe Faders, Floating OSD & Quick Mixer
- Rebuilt [`lwasapi-tray.py`](lwasapi-tray.py) around native `org.kde.StatusNotifierItem` D-Bus export (with automatic `QSystemTrayIcon` fallback when `org.kde.StatusNotifierWatcher` is absent) and high-DPI icon rendering in [`TrayIconBank`](lwasapi-tray.py) (`0600` PNGs inside `0700` runtime directory).
- Added the active emerald-glow icon mode (`"tray_green_glow": true` default, toggleable to plain icon via right-click *"Green Glow on Active Icon"*), plus muted red-slash and stopped amber-dot states.
- Added vertical scroll for Master Output (`+/-3%`), horizontal scroll/swipe or `Shift`+scroll for Mic Input Gain (`+/-3%`), middle-click Master Mute toggle, and floating Plasma-style translucent OSD pills ([`SalsaOsdOverlay`](lwasapi-tray.py)).
- Added [`SalsaQuickMixerPopup`](lwasapi-tray.py) on left-click (revertable to direct Studio launch via right-click *"Left-Click Opens Quick Mixer"*), popping cleanly above the Plasma panel/tray (`QT_QPA_PLATFORM=xcb` default on XWayland so panel-aware positioning and smooth window dragging work reliably across multi-monitor setups; revertable anytime with `QT_QPA_PLATFORM=wayland`). The popover can be dragged freely anywhere by its header grip (`⠿`) or background with **Pin** / **Close** controls, displays live per-application program icons resolved securely via [`AppIconResolver`](lwasapi-tray.py) (`/proc/<pid>` + XDG `.desktop` + system icon theme) alongside active stream swipeable faders, and defaults to the new **"Plasma Breeze Emerald"** charcoal-slate/emerald/sky-blue theme (added to both [`salsa-gui.py`](salsa-gui.py) and [`lwasapi-tray.py`](lwasapi-tray.py), toggleable back to Studio's active theme via right-click *"Quick Mixer: Breeze Slate Theme"*). All settings persist safely via `0600` atomic `gui_settings.json` writes.

### 2. Steam Reconnect Crash Fix & Pressure-Vessel Auto-Preload Wrapper
- Fixed lifetime, use-after-free, double-free, and locking hazards in [`src/salsa_pa_reconnect.c`](src/salsa_pa_reconnect.c) across rapid engine resets (`pa_threaded_mainloop`), including global thread-safe `tee_req_t` ownership tracking across `pa_context_fail()`, outside-of-lock dead handle flushing, global `wstream_t` registry validation, `pa_context_ref`/`pa_stream_ref` guards around async callbacks and timers, and `wctx->lock` synchronization on draining/sample/autoload/stat calls.
- Added an automatic, reversible `pressure-vessel` Steam wrapper in [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c) (`~/.local/bin/salsa-steam-launch` + updated `Exec=` in `~/.local/share/applications/steam.desktop` and `~/.config/autostart/steam.desktop` with `0600` `.bak` backups and desktop database refresh) managed automatically by `lwasapi-ctl enable-auto`, `lwasapi-ctl disable-auto`, and [`install-salsa.sh`](install-salsa.sh) `--uninstall`.

### 3. Full-Bleed Tray Icons, Redesigned Translucent OSD, KDE OSD Suppression & Bluetooth Battery Readout
- Updated [`TrayIconBank`](lwasapi-tray.py) in [`lwasapi-tray.py`](lwasapi-tray.py) to tight-crop transparent padding and install crisp full-bleed `salsa-tray-*` icons (`0644`) into `~/.local/share/icons/hicolor/` with `KIconLoader.iconChanged` cache invalidation (cleaned up automatically by [`install-salsa.sh`](install-salsa.sh) `--uninstall`).
- Redesigned [`SalsaOsdOverlay`](lwasapi-tray.py) as a `330x66` frosted-glass pill with vector speaker/mic icon, device name, emerald gradient progress bar, X11 KDE blur hint, and `300ms` external volume watcher, and updated [`install-salsa.sh`](install-salsa.sh) to automatically suppress duplicate KDE Plasma audio OSDs in `~/.config/plasmaparc` (saving `plasmaparc_osd.state` `0600` for clean rollback on `--uninstall`) and reload `kded6` `audioshortcutsservice`.
- Fixed [`read_bluez_battery_pct`](lwasapi-tray.py) (with [`_parse_dbus_pct`](lwasapi-tray.py) and UPower fallback) to decode PyQt6 `QtDBus` single-byte `org.bluez.Battery1.Percentage` `bytes` values and prioritize the active Bluetooth audio endpoint over unrelated peripherals, surfacing battery % in both [`SalsaQuickMixerPopup`](lwasapi-tray.py) and the tray tooltip.

### 4. Steam `dlopen` Recursion & PulseAudio Extension (`pa_ext_*`) Crash Fixes
- Fixed `dlopen` constructor recursion in [`src/salsa_pa_reconnect.c`](src/salsa_pa_reconnect.c) when both `build/pa_reconnect/libpulse.so.0` and `LD_PRELOAD` `build/libsalsa_pa_reconnect.so` are active (`salsa_real_dlopen_passthrough` + multiarch SteamRT `libpulse.so.0` paths).
- Wrapped all 20 PulseAudio extension APIs from `ext-device-restore.h`, `ext-device-manager.h`, and `ext-stream-restore.h` (including `pa_ext_device_restore_read_formats_all` called by `steamrt64/libaudio.so`) in [`src/salsa_pa_reconnect.c`](src/salsa_pa_reconnect.c) with [`wctx_t`](src/salsa_pa_reconnect.c) callback unwrapping and reconnect re-subscription so Steam and `steamwebhelper` launch with zero coredumps inside `pressure-vessel`.

---

## Rev 5.9 (`2026-11-14`)

### 1. Persistent Per-App Mixer, Mute & Stream Pinning + Per-Device EQ Auto-Switching
- Added persistent per-app volume/mute storage in [`src/lwasapi_pa_server.c`](src/lwasapi_pa_server.c) (`~/.config/lwasapi/app_volumes.conf`, saved via atomic `O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW` `0600` temp-file rename inside `0700` dir, `O_NOFOLLOW` + bounded `<= 262144` byte reload, and `sanitize_app_vol_key` against injection/traversal).
- Added [`LWASAPI_CMD_SET_SESSION_CONTROL`](include/lwasapi.h) and `LWASAPI_SetSessionControl()` in [`include/lwasapi.h`](include/lwasapi.h), [`src/liblwasapi.c`](src/liblwasapi.c), and [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c), plus CLI subcommands `app-volume`, `app-mute`, and `move-app` in [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c).
- Upgraded [`salsa-gui.py`](salsa-gui.py) `tbl_sessions` to 8 columns with flicker-free interactive Volume sliders (`0–150%`), Mute toggles, and Route Target dropdowns (`auto` or pinned device), hardened `load_gui_settings`/`save_gui_settings` (`0600` atomic write + `O_NOFOLLOW`), and added **Per-Device EQ** auto-switching (`eq_device_profiles` keyed by active sink ID).

### 2. High-Bitrate BlueZ Codecs (`LDAC`, `aptX HD`, `aptX`)
- Extended [`include/lwasapi_bluez.h`](include/lwasapi_bluez.h) and [`src/lwasapi_bluez.c`](src/lwasapi_bluez.c) to advertise and negotiate BlueZ D-Bus endpoints for **LDAC** (`0x0000012D`/`0x00AA`, 44.1–96 kHz), **aptX HD** (`0x000000D7`/`0x0024`), and **aptX** (`0x0000004F`/`0x0001`) alongside `AAC` and `SBC`, loading trusted system encoder libraries via `dlopen(..., RTLD_NOW|RTLD_LOCAL)` (`libldacBT_enc.so.2`, `libfreeaptx.so.0`) with strict `rcaps`/`cfg_len`/MTU bounds validation.

### 3. 32-Bit Multilib Proxy, Scoped Flatpak Overrides, Root-Safe Pacman Trigger & Regression Suite
- Updated [`scripts/gen_pa_proxy.py`](scripts/gen_pa_proxy.py) with strict C-identifier regex validation (`^[A-Za-z_][A-Za-z0-9_]*$`), `--arch i386` PIC stub emission, and atomic temp-file writes; updated [`Makefile`](Makefile), [`src/salsa_pa_reconnect.c`](src/salsa_pa_reconnect.c), [`lwasapi-autosync.sh`](lwasapi-autosync.sh), and [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c) to build and activate `build/pa_reconnect32` automatically when multilib `libpulse.so.0` + `gcc -m32` are installed.
- Updated [`install-salsa.sh`](install-salsa.sh) with scoped per-app Flatpak `--user` overrides (`--socket=pulseaudio`, `--filesystem=xdg-run/pipewire-0:ro`) tracked for clean `--uninstall` rollback, plus `--install-pacman-hook` / `--remove-pacman-hook` via `pkexec` using an unprivileged timestamp trigger (`/usr/bin/touch /var/lib/salsa-autosync/libpulse-updated`) so `root` never executes user-owned scripts.
- Added automated regression and security suite [`scripts/run_regression_tests.py`](scripts/run_regression_tests.py) wired into `make test`.

---

## Rev 5.8 (`2026-11-14`)

### 1. Installer Revision Tracking & Patch Notes
- Added [`VERSION`](VERSION) (`5.8`) and updated [`Makefile`](Makefile) `installer` target to package `salsa-lwasapi-installer (Rev_<REV>).zip` automatically.
- Updated [`install-salsa.sh`](install-salsa.sh) (`print_pc_report`) to display `(Rev 5.8)`.
- Created [`PATCH_NOTES.md`](PATCH_NOTES.md) changelog.

### 2. Seamless Firefox Audio Recovery Across Service Restarts & Reboots
- Fixed `cubeb-pulse-rs` crash in [`src/salsa_pa_reconnect.c`](src/salsa_pa_reconnect.c) (`refresh_defer_cb` now filters synthetic subscribe events against `wctx->has_subscribe` and `wctx->sub_mask`).
- Added monotonic `pa_stream_get_time()` tracking across stream recreations (`ever_ready`, `time_offset_us`, `snapshot_stream_time_locked`), `SYNTH_KIND_STREAM_SUCCESS` for `pa_stream_cork` / `pa_stream_update_timing_info` / attribute ops, cork reconciliation on stream ready, and mid-reconnect stream creation.
- Added [`scripts/gen_pa_proxy.py`](scripts/gen_pa_proxy.py) and `build/pa_reconnect/libpulse.so.0` (`220` PIC forwarding stubs + `-Wl,-z,nodelete` + host `libpulse.so.0` resolution) so KDE session restores (`/usr/lib/firefox/firefox --sm-client-id ...`) and desktop launches inherit reconnection transparently via `LD_LIBRARY_PATH` in `~/.config/environment.d/99-salsa-direct.conf` (configured by [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c) `enable-auto` / `disable-auto`).
- Removed legacy `LD_PRELOAD` `firefox`/`discord` wrappers in [`install-salsa.sh`](install-salsa.sh) to prevent double-wrapping.

### 3. Bluetooth Headphone Battery Percentage
- Fixed Bluetooth battery telemetry in [`src/lwasapi_bluez.c`](src/lwasapi_bluez.c) so Bose QC Ultra 2 HP battery percentage reports accurately while WirePlumber runs in `video-only` / Zero-PipeWire mode.

### 4. Headphone Source Recovery & Update-Protection Hardening
- Updated [`src/lwasapi_bluez.c`](src/lwasapi_bluez.c) to automatically recover and re-register endpoints when `bluetooth.service` (`bluetoothd`) restarts while `lwasapi-audiodg.service` stays active.
- Updated [`lwasapi-autosync.sh`](lwasapi-autosync.sh) to wait cleanly for active package manager locks (`pacman`, `dpkg`, `yum`) in `--auto` mode and filter out `lib32` paths in `find_host_lib`.

### 5. Hardware & Idle XRUN Fix
- Fixed false idle XRUN inflation in `consume_session_frames` and resolved real ALSA write stalls in [`src/lwasapi_audiodg.c`](src/lwasapi_audiodg.c).

### 6. Security Hardening, Private IPC & Zero-PipeWire Direct BlueZ
- Added native Bluetooth A2DP startup auto-connect, stuck-link self-healing, and manual BR/EDR reset in [`src/lwasapi_bluez.c`](src/lwasapi_bluez.c) (replacing `reset-bose-audio.sh` and `bose-tray.py`).
- Hardened IPC/SHM paths with private `0700` runtime directories (`$XDG_RUNTIME_DIR/lwasapi`), `0600` socket permissions, peer credential / cookie validation (`SO_PEERCRED`), and safe shared-memory bounds checks.
- Added per-target build freshness checks in [`salsa_debug_core.py`](salsa_debug_core.py) and automatic `liblwasapi.so` reload/re-exec protection in [`salsa-gui.py`](salsa-gui.py) `_toggle_engine_mode`.

---

## Rev 5.7 (`2026-11-13`)

### 7. KDE Panel Reset Fix
- Fixed leaked file descriptors in [`src/lwasapi_pw_broker.c`](src/lwasapi_pw_broker.c) so `plasmashell` KDE panels remain stable without crashing or resetting.

### 8. 220ms Delay Verification
- Audited Bluetooth AVDTP Delay Reporting telemetry and verified `220 ms` is the hardware sink buffer reported by the Bose QC Ultra 2 HP headphones (separate from Salsa's `2.67 ms` engine period quantum).

### 9. Discord `>= 30 fps` Video Stream Fix
- Updated [`src/lwasapi_pw_broker.c`](src/lwasapi_pw_broker.c) frame-rate negotiation so `kwin_wayland` / ScreenCast consumers negotiate `>= 30 fps` / `60 fps` instead of clamping at `25 fps`.

### 10. Discord Auto-Reconnect & Device-List Cache
- Created [`src/salsa_pa_reconnect.c`](src/salsa_pa_reconnect.c) (`libsalsa_pa_reconnect.so`) to suppress fatal `PA_CONTEXT_FAILED` / `PA_STREAM_FAILED` transitions once connected, serve cached sink/source/server lists during daemon restarts, reconnect active contexts/streams cleanly, and integrated it into [`install-salsa.sh`](install-salsa.sh).

---

## Pre-Rev 5.7 History (`2026-11-11` – `2026-11-13`)

### 11. Pure Zero-PipeWire ScreenCast Video Broker & Crash Fixes (`2026-11-13`)
- Implemented pure-C [`src/lwasapi_pw_broker.c`](src/lwasapi_pw_broker.c) serving `/run/user/$UID/pipewire-0` (blocking audio nodes on `pipewire-0` while relaying `DmaBuf`/`MemFd` ScreenCast frames), updated `salsa-debug.py` preview & telemetry, and updated the signal topology view in `salsa-gui.py`.

### 12. Steam / Proton Direct Audio, Custom Theme Editor & Saved Settings (`2026-11-12` – `2026-11-13`)
- Added `PROTON_USE_PIPEWIRE=0` / `SDL_AUDIODRIVER=pulse` / `ALSOFT_DRIVERS=pulse,alsa` environment routing in [`src/lwasapi_ctl.c`](src/lwasapi_ctl.c), added live theme editor [`salsa_theme_editor.py`](salsa_theme_editor.py), persisted preset/dynamic labels in [`salsa-gui.py`](salsa-gui.py), and tuned Steam audio buffering.

### 13. Live Interaction Debugger, Tray & Icon Branding (`2026-11-11`)
- Created [`salsa-debug.py`](salsa-debug.py), [`salsa_debug_core.py`](salsa_debug_core.py), transparent [`assets/salsa-lwasapi.png`](assets/salsa-lwasapi.png), desktop entries, and tray indicator integration in [`lwasapi-tray.py`](lwasapi-tray.py).

### 14. Salsa Studio GUI, Dynamic EQ, Unplugged Hiding, Capture & Cross-PC Installer (`2026-11-11`)
- Built the WaveRT SHM core, native PA v35 server ([`src/lwasapi_pa_server.c`](src/lwasapi_pa_server.c)), hollow ALSA/Pulse/JACK shims, hardware plug/unplug filtering, [`salsa-gui.py`](salsa-gui.py), [`SALSA_OWNERS_MANUAL.md`](SALSA_OWNERS_MANUAL.md), [`install-salsa.sh`](install-salsa.sh), and [`lwasapi-autosync.sh`](lwasapi-autosync.sh).
