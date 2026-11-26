# Salsa Patch Notes

Chronological changelog organized by installer revision.

---

## V1.24.14 — Current (`2026-11-24`)

### 1. Zero-Downtime `pipewire-0` Systemd Socket Activation & Autosync Hardening
- Added systemd `LISTEN_FDS` / `LISTEN_PID` socket-activation support (`claim_systemd_unix_listeners()`, `take_systemd_unix_listener()`, `dup_unix_listener_nonblock()`) in [`unix_sys.rs`](rust/salsa_net/src/unix_sys.rs), adopted pre-opened `$XDG_RUNTIME_DIR/pipewire-0` and `pipewire-0-manager` sockets in [`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs) without unlinking them on stop, and moved `lasapi_pw_broker_start()` ahead of `init_hardware()` in [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) so KDE Plasma's `libKPipeWire` (`MediaMonitor`) never encounters a missing `pipewire-0` socket or poisoned `PipeWireCore` singleton across engine restarts (falling back automatically to standalone socket bind on non-systemd systems).
- Added portable `lasapi-pipewire.socket` lifecycle management across [`ctl_cli.rs`](rust/salsa_net/src/ctl_cli.rs), [`salsa_installer.rs`](rust/salsa_tools/src/bin/salsa_installer.rs), and [`debug_core.rs`](rust/salsa_tools/src/debug_core.rs).
- Updated [`lasapi_autosync.rs`](rust/salsa_tools/src/bin/lasapi_autosync.rs) (`compute_fingerprint()`, `pipewire_zero_drifted()`) to replace `/var/lib/pacman/local` directory `mtime` hashing with SHA-256 content hashes of actual audio, Bluetooth codec (`libfdk-aac`, `libldacBT_enc`, `libfreeaptx`), and D-Bus `.so` libraries so unrelated package installations no longer trigger unnecessary rebuilds or daemon restarts.

### 2. Pure Native Wayland `LayerShellQt` Tray, Multi-Monitor Fractional-DPI Placement & Full Suite Wayland Support
- Upgraded [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) to run 100% natively on Wayland (`QT_QPA_PLATFORM=wayland;xcb`, zero `libQt6XcbQpa.so`) using runtime `dlopen()` of `libLayerShellQtInterface.so.6` (`zwlr_layer_surface_v1` surfaces for Quick Mixer, context menu, and click-through `osd` volume overlay) so opening the tray popup never creates a panel taskbar icon or Alt+Tab entry, while preserving `_NET_WM_STATE_SKIP_TASKBAR` / `_KDE_NET_WM_STATE_SKIP_SWITCHER` fallback in [`x11_util.rs`](rust/salsa_ui/src/x11_util.rs) for X11 and compositors without `zwlr_layer_shell_v1`.
- Added `queryKwinActiveScreen()`, `effectiveWaylandScale()` (reading live per-connector fractional scales from `org.kde.KWin.supportInformation`), `resolveTrayTarget()`, and `anchorRawLocalOffset_` layout-settle margin updates in [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) so the Quick Mixer centers directly above the Salsa tray icon across multi-monitor fractional-DPI setups and all four panel edges.
- Added `preferNativeWaylandPlatform()` and `syncResolvedPlatformEnv()` in [`salsa_common.hpp`](qt6/salsa_common.hpp), [`salsa_common.cpp`](qt6/salsa_common.cpp), [`salsa_studio.cpp`](qt6/salsa_studio.cpp), and [`salsa_debug.cpp`](qt6/salsa_debug.cpp), and guarded [`x11_util.rs`](rust/salsa_ui/src/x11_util.rs) with `window_uses_x11()` so `libX11.so.6` is never opened during native Wayland execution.

---

## V1.21.13 (`2026-11-23`)

### 1. Automatic Headphone Hotplug Switching, Card Priority & Mute-Switch Safety
- Updated [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) (`HotplugTracker`, `evaluate_playback_hotplug_with()`, `sync_hotplug_topology()`, `seed_hotplug_baseline()`): plugging in USB headphones/headsets (`hw:6,0`), connecting Bluetooth A2DP headsets, or inserting a 3.5 mm analog headphone jack (`card_has_plugged_headphone_jack()`) now automatically switches playback even when switching from another USB audio interface (e.g., Focusrite Scarlett 2i2) or a previously selected speaker device, remembers the prior speaker target to restore seamlessly on unplug (or fails over immediately to connected Bluetooth / available ALSA endpoints instead of stranding on silent `WaveRT-Virtual-Clock`), and ignores HDMI/DisplayPort monitor wakeups.
- Updated [`hw_analog.rs`](rust/salsa_net/src/hw_analog.rs) (`is_headphone_card_name()`, `card_has_plugged_headphone_jack()`, `score_card_with_meta_hp()`, `read_alsa_bool()`, `hw_analog_sync()`): added dedicated headphone/headset/jack scoring priority above desktop audio interfaces and unmutes muted ALSA DAC/headphone switches (`pb_dac_sw` / `pb_hp_sw`) on activation when Salsa master output is unmuted.
- Updated [`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs) (`bt_topo_gen`, `bluez_signal_filter()`, `sync_known_bt_devices()`, `bluez_hw_fingerprint_impl()`) so Bluetooth `Device1` / `MediaTransport1` connection transitions wake hotplug detection even while Bluetooth TX was paused on speakers or USB.
- Added an `"Auto-Priority (Smart Hotplug)"` entry at index `0` in [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp), stopped microphone selection from overwriting `target_playback_id`, and updated Auto-Priority labels in [`salsa_studio.cpp`](qt6/salsa_studio.cpp).

### 2. Smooth Video Playback Start & Underrun Recovery (PulseAudio Server + RT Pacing)
- Updated [`pa_server.rs`](rust/salsa_net/src/pa_server.rs) (`enforce_playback_attr()`, `lasapi_pa_server_mix_playback()`, `service_stream_events_and_capture()`, `CMD_CORK_PLAYBACK_STREAM`, `CMD_FLUSH_PLAYBACK_STREAM`): raised default/low-latency video playback prebuffer headroom (`>= 15 ms` prebuf with `>= 30 ms` `tlength` when `prebuf != 0`, keeping `prebuf + minreq <= tlength` to prevent request stalls), re-armed `in_prebuf` cleanly on both full and partial underruns as well as cork-to-uncork transitions when buffered frames are below `prebuf`, and reset `started_sent` across flushes and underruns so browser video playback starts smoothly without choppy machine-gun stuttering.
- Added an `had_xrun` timer-pacing guard and rate-limited `HW_XRUN` logging in [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) so recoverable ALSA PCM XRUNs never spin in a tight retry loop.

### 3. `salsa_pa_reconnect` `dlopen` `$ORIGIN`/`RUNPATH` Tail-Jump Preservation
- Updated [`lib.rs`](rust/salsa_pa_reconnect/src/lib.rs) (`salsa_dlopen_prepare()` + x86_64 tail-jump `dlopen` trampoline) so non-PulseAudio `dlopen()` passthrough preserves the caller's stack return address and `$ORIGIN` / `RUNPATH` resolution cleanly.

### 4. LASAPI / WaveRT-ASIO Stream Capture Bridge for Discord Screen Share (`PulseAudio` `doi`)
- Updated [`pa_server.rs`](rust/salsa_net/src/pa_server.rs) (`PaLasapiRtSlot`, `LasapiPaProxyMeta`, `register_lasapi_proxy()`, `unregister_lasapi_proxy()`, `put_client_fields()`, `put_sink_input_fields()`, `lasapi_pa_server_register_lasapi_session()`, `lasapi_pa_server_unregister_lasapi_session()`, `lasapi_pa_server_update_lasapi_session_vol()`, `lasapi_pa_server_lasapi_doi_mask()`, `lasapi_pa_server_push_lasapi_period()`, `lasapi_pa_server_poll_lasapi_ctl()`) and [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) (`consume_lasapi_playback_slot()`) so active native LASAPI / WaveRT-ASIO / ALSA-shim playback sessions (such as `FL64.exe:Salsa-WaveRT-ASIO`) register virtual PA `sink_input` and `client` entries with `application.process.id`, bidirectional volume/mute sync, live subscription events, and a lock-free per-session `direct_on_input` (`pa_stream_set_monitor_stream`) audio tap without duplicating sessions in `lasapi-ctl` or Salsa Studio.
- Updated `CMD_CREATE_PLAYBACK_STREAM` in [`pa_server.rs`](rust/salsa_net/src/pa_server.rs) to inherit `application.process.id`, `application.name`, and `application.process.binary` from authenticated PA clients when omitted, and updated `emit_fade_out()` to zero trailing `period_l`/`period_r` samples on zero-level partial underruns.

---

## V1.21.12 (`2026-11-21`)

### 1. Security Hardening, Non-Executable Stacks & IPC Efficiency
- Added `-D_FORTIFY_SOURCE=3`, `-fstack-protector-strong`, `-Wformat -Wformat-security` (`HARDEN_CXXFLAGS`), and `-Wl,-z,noexecstack` across C++/Rust targets in [`Makefile`](Makefile), plus `--install-pacman-hook` / `--remove-pacman-hook` handling in [`install-salsa.sh`](install-salsa.sh).
- Hardened cookie verification, D-Bus object path checks, sandboxed TCP failover, bounded settings reads, and non-finite DSP filtering across [`pa_server.rs`](rust/salsa_net/src/pa_server.rs) (`ct_eq_256()`, `sync_active_doi()`), [`bluez_ffi.rs`](rust/salsa_net/src/bluez_ffi.rs) (`is_valid_dbus_object_path()`), [`imp.rs`](rust/salsa_pa_reconnect/src/imp.rs) (`server_string_allows_tcp_fallback()`, `read_salsa_tcp_target()`, `reconnect_timer_cb()`), [`lasapi_bench.rs`](rust/lasapi_client/src/bin/lasapi_bench.rs) (`pick_sandbox_base_dir()`), [`ipc.rs`](rust/salsa_tools/src/ipc.rs) (`SettingsLockGuard`, `read_nofollow_bounded_utf8()`, `cached_proc_comm()`), [`ctl_cli.rs`](rust/salsa_net/src/ctl_cli.rs), and [`dsp.rs`](rust/salsa_net/src/dsp.rs).

### 2. Bluetooth Playback Drive Parity & AVRCP Acquire Preservation
- Added `apply_bt_hw_analog_drive()` in [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) so Bluetooth A2DP TX streams receive the same hardware analog drive and soft-knee shaping as ALSA playback while scrubbing non-finite samples.
- Added `apply_peer_avrcp_vol()` in [`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs) so stale peer AVRCP volume notifications during `MediaTransport1.Acquire` preserve pending host volume sync and boost levels.

### 3. Zero-Flicker PipeWire ScreenCast Preview & Wayland Single-Instance Activation
- Updated [`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs) multi-slot `spa_io_buffers` handling (`io_read_slot()`, `io_take_ready_slot()`, `io_write_all_slots()`, `io_recycle_buf()`), [`debug_core.rs`](rust/salsa_tools/src/debug_core.rs) (`read_frame_since()`), [`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs) (`salsa_dbg_shm_reset_cache()`), and [`salsa_debugger_widget.cpp`](qt6/salsa_debugger_widget.cpp) (`withinLiveGrace`) so Portal ScreenCast streams capture frame #0 cleanly and switch screens without visual flicker.
- Added [`SingleInstanceServer`](qt6/salsa_common.hpp), `tryActivateExistingSingleInstance()`, runtime `dlopen` of `libKF6WindowSystem.so.6` (`KWaylandExtras` / `KWindowSystem` XDG activation tokens), and `raiseAndActivateWindow()` in [`salsa_common.cpp`](qt6/salsa_common.cpp), [`salsa_studio.cpp`](qt6/salsa_studio.cpp), and [`salsa_debug.cpp`](qt6/salsa_debug.cpp) so second launches raise and focus the original window on KDE Plasma Wayland instead of only flashing the panel icon.
- Switched boost and crossfeed labels (`lblHwBoost_`, `lblHwMicBoost_`, `lblCrossfeed_`) to fixed-width monospace `ValMonoLabel` formatting (`formatBoostDbText()` / `formatPctText()`), set `QSizePolicy::Ignored` on `lblHwAnalogDiag_`, and flushed hardware analog sliders synchronously on release/close before polling so `+9.5 dB` -> `+10.0+ dB` adjustments stay pixel-stationary and persist reliably.

### 4. Unified Security-Hardened `salsa_settings.json` Store, Config Controls & Qt6 Readability Fixes
- Replaced the split binary/JSON persistence (`salsa_state.bin` + `gui_settings.json`) with a single canonical, schema-validated `~/.config/lasapi/salsa_settings.json` store in [`state_conf.rs`](rust/salsa_net/src/state_conf.rs), [`unix_sys.rs`](rust/salsa_net/src/unix_sys.rs), [`json.rs`](rust/salsa_net/src/json.rs), and [`ipc.rs`](rust/salsa_tools/src/ipc.rs), adding automatic first-launch migration with one-time `.bak` backups (`salsa_state.bin.bak` / `gui_settings.json.bak`), strict `0700`/`0600` atomic locked writes, and `O_NOFOLLOW` symlink/ownership rejection.
- Added live daemon config persistence commands (`LASAPI_CMD_SAVE_CONFIG` / `LASAPI_CMD_RELOAD_CONFIG`) across [`c_abi.rs`](rust/salsa_net/src/c_abi.rs), [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs), [`lib.rs`](rust/lasapi_client/src/lib.rs), [`ctl_cli.rs`](rust/salsa_net/src/ctl_cli.rs) (`save-config` / `reload-config`), and [`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs) (`salsa_overwrite_settings_from_ui()`).
- Added explicit **Save Settings** (`Ctrl+S` in the 2x2 header grid, Hardware Analog Stage, and Engine card), **Reload Saved Config**, and **Open Config File** controls in [`salsa_studio.cpp`](qt6/salsa_studio.cpp) with stationary confirmation width (`flashSaveFeedback()`), plus fixed `ValMonoLabel` dark-on-dark color (`text_bright`), transparent `QSlider`/`QScrollArea` tracks, theme-tinted `QComboBox::down-arrow` chevrons in [`salsa_common.cpp`](qt6/salsa_common.cpp), full-width wrapping diagnostic labels (`lblBtDiag_`, `lblHwAnalogDiag_`, `lblEndpoints_`, `lblCfgSaveStatus_`), and valid theme-aware swatch hint contrast in [`salsa_widgets.cpp`](qt6/salsa_widgets.cpp).

---

## V1.21.11 (`2026-11-21`)

### 1. Container-Resilient PulseAudio Loopback TCP Failover Across Daemon Restarts
- Added a cookie-authenticated loopback TCP listener (`127.0.0.1:0` with `TCP_NODELAY`) to [`pa_server.rs`](rust/salsa_net/src/pa_server.rs) (`bind_loopback_tcp_and_write_port()`, `PaState::accept_client_fd()`, `server_thread_main()`, `lasapi_pa_server_stop()`): alongside `/run/user/$UID/pulse/native`, `lasapi-audiodg` now binds an ephemeral `127.0.0.1` port whenever `~/.config/pulse/cookie` (`0600`, `256` bytes) is valid, writes that port atomically (`0600`, `O_NOFOLLOW | O_CLOEXEC`) to `~/.config/pulse/salsa-tcp-port`, and enforces strict 256-byte cookie validation in `CMD_AUTH` (`uid_opt = None`).
- Updated [`imp.rs`](rust/salsa_pa_reconnect/src/imp.rs) (`should_try_tcp_fallback()`, `read_salsa_tcp_target()`, `reconnect_timer_cb()`) so Steam Pressure-Vessel (`srt-bwrap`) and other mount-sandboxed apps whose bind-mounted `/run/pressure-vessel/pulse/native` socket inode becomes orphaned during a rebuild or daemon restart automatically load `~/.config/pulse/cookie`, fail over seamlessly to `tcp4:127.0.0.1:<port>` on even reconnect attempts while keeping direct `SO_PEERCRED` UNIX connections first for host apps, and resume playback streams without dropping game audio.

### 2. `pa_context_rttime_*` Timer Continuity & Stream Disconnect Callback Fixes
- Updated [`imp.rs`](rust/salsa_pa_reconnect/src/imp.rs) (`rttime_to_timeval()`, `pa_context_rttime_new()`, `pa_context_rttime_restart()`): when `real_ctx` is `NULL` (before initial connect or during a reconnect window), `pa_context_rttime_new` and `pa_context_rttime_restart` now convert monotonic `usec` with `PA_TIMEVAL_RTCLOCK` (`1 << 30`, or `NULL` for `PA_USEC_INVALID`) and delegate directly to `(*wctx).api.time_new` / `time_restart` instead of dropping timer restarts. This prevents Proton's `winepulse.so` `pa_streams_timer_cb` 10 ms period pump from dying permanently across `lasapi-audiodg` restarts and deadlocking Wwise/LASAPI shutdown.
- Hardened `pa_stream_disconnect()` in [`imp.rs`](rust/salsa_pa_reconnect/src/imp.rs) so if `real().pa_stream_disconnect` returns `< 0` (such as `-PA_ERR_BADSTATE` on an unconnected or failed underlying stream), it detaches and unrefs the stale `real_stream`, keeps `reported_state = PA_STREAM_TERMINATED`, and invokes `state_cb` so callers waiting on `PA_STREAM_IS_GOOD` wake up cleanly.

---

## V1.21.10 (`2026-11-21`)

### 1. Bluetooth A2DP Sink Preservation & Reconnect Reliability Across TV / Wired Switches
- Fixed premature Bluetooth transport teardown when reconnecting from TV/HDMI or wired outputs in [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) (`apply_playback_endpoint()`, `should_try_bt_on_apply()`, `should_disable_bt_tx_on_fallback()`): widened the `MediaTransport1.Acquire` readiness poll window from `600 ms` to `3.5 s` (`140 * 25 ms`, returning early on ready socket), stopped forcing `lasapi_bluez_set_tx_enabled(false)` during transient fallback on explicit Bluetooth or `Auto-Priority` targets so hotplug finishes restoring Bluetooth cleanly, allowed `Auto-Priority` to resume connected Bluetooth headsets even when `Optional BT` mode is enabled, and put Bluetooth TX into standby when `LASAPI_CMD_BT_CONNECTION` connects while pinned to a non-Bluetooth sink.
- Fixed Bluetooth endpoint disappearance during `MediaTransport1` standby in [`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs) (`bluez_enumerate_devices_impl()`, `should_enumerate_bt_a2dp_device()`, `format_bt_enum_entry_name()`, `activate_bluez_target()`, `classify_tx_enable_action()`, `pick_best_paired_a2dp_device()`, `dbus_poll_device_connected()`): connected paired A2DP audio sinks remain listed in Salsa (`Bose QC Ultra 2 HP (A2DP Standby)`) after transport release, re-arm BR/EDR `PreferredBearer` and `A2DP_SINK_UUID` activation automatically via `need_activate` (including after `UnknownObject`), filter BT reset/connection operations strictly to paired A2DP audio sinks instead of unrelated LE peripherals, and share `Connected` polling across connect/disconnect/reset flows.
- Updated `salsa_bt_reset_sync()` in [`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs) to call `bluez_reset_device_impl()` directly rather than picking the first MAC from `bluetoothctl devices Connected`, and guarded `cmbSink_` / `cmbSource_` refreshes in [`salsa_studio.cpp`](qt6/salsa_studio.cpp) while either dropdown popup is open.

### 2. Isolated Virtual Sandbox Latency Benchmark & Volume Telemetry
- Updated [`lasapi_bench.rs`](rust/lasapi_client/src/bin/lasapi_bench.rs) (`run_in_virtual_sandbox()`, `SandboxGuard`, `LiveStateRestoreGuard`) so `lasapi-bench` runs by default inside an isolated virtual `lasapi-audiodg --virtual` sandbox without altering live system volume, hardware clock settings, or active playback (`--live` remains opt-in with state restoration), forwarded CLI options from `lasapi-ctl bench` in [`ctl_cli.rs`](rust/salsa_net/src/ctl_cli.rs), and ran Studio's `Run Latency Benchmark` asynchronously via `QThread` in [`salsa_studio.cpp`](qt6/salsa_studio.cpp).
- Added `PA_SET_SINK_VOL` volume telemetry in [`pa_server.rs`](rust/salsa_net/src/pa_server.rs) and `client_pid` / `client_name` attribution in [`ipc.rs`](rust/salsa_tools/src/ipc.rs) (`send_volume_cmd()`).

---

## V1.19.9 (`2026-11-19`)

### 1. Dual USB+BT Device Priority, Optional Bluetooth Link Policy & Radio Contention Fix
- Added automatic **USB-over-Bluetooth priority** in [`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs) (`usb_device_matches_bt_name()`, `find_matching_usb_pb_idx_for_list()`, `any_usb_matches_bt()`): when a plugged USB DAC/headset shares hardware naming with a paired or connected BlueZ A2DP device (such as `Bose Corporation Bose QC Ultra 2 HP - USB Audio` paired alongside `Bose QC Ultra 2 HP`), `Auto-Priority` adds a `+50,000` card priority boost to that USB interface, automatically upgrades from Bluetooth to USB on plug-in (`auto_upgrade_matching_usb`), avoids re-switching to Bluetooth while USB is plugged, and pauses/releases BlueZ `MediaTransport1` TX so idle AVDTP/L2CAP traffic cannot destabilize shared USB Bluetooth controllers (`btusb`) or Bluetooth LE mice.
- Made BlueZ audio connections **optional / non-mandatory by default** (`bt_optional_mode = 1`, persisted in `PERSISTED_V6_SIZE` / `MAGIC_V6` inside [`state_conf.rs`](rust/salsa_net/src/state_conf.rs)), backed by [`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs) (`need_release`, `release_bluez_transport()`, `sync_known_bt_devices()`, `bluez_connection_ctl_impl()`), [`c_abi.rs`](rust/salsa_net/src/c_abi.rs) (`LASAPI_CMD_BT_CONNECTION`), [`lib.rs`](rust/lasapi_client/src/lib.rs) (`LASAPI_BtConnectionCtl`), [`ipc.rs`](rust/salsa_tools/src/ipc.rs), [`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs), and [`salsa_bridge.h`](qt6/salsa_bridge.h).
- Added the `Optional BT` checkbox, `Turn On BT` / `Disconnect BT` button (`chkBtOptional_`, `btnBtLink_`), and `Auto-Priority (Dual USB > BT A2DP > USB > Analog > HDMI)` selector in [`salsa_studio.cpp`](qt6/salsa_studio.cpp), plus `lasapi-ctl bt-link <status|on|off|optional|auto>` and policy telemetry in `lasapi-ctl status` ([`ctl_cli.rs`](rust/salsa_net/src/ctl_cli.rs)).

### 2. Single-Instance Studio Window Restore & Debugger PA Client Fallback
- Implemented UNIX socket single-instance activation (`$XDG_RUNTIME_DIR/lasapi/salsa-studio.sock` with `SO_PEERCRED` check and `XDG_ACTIVATION_TOKEN` forwarding) in [`salsa_studio.cpp`](qt6/salsa_studio.cpp): launching `salsa` / `salsa-studio` while already open or minimized un-minimizes, shows, raises, and activates the existing window instead of spawning a duplicate process.
- Updated `tblPaClients_` in [`salsa_debugger_widget.cpp`](qt6/salsa_debugger_widget.cpp) to fall back to the executable `binary` name (or `PID <n>`) with a diagnostic tooltip when PulseAudio clients (such as `kded6`, `polkit-kde-authentication-agent-1`, or `plasmashell`) omit `application.name`.

---

## V1.19.8 (`2026-11-19`)

### 1. Installer Terminal Auto-Launch, Step Progress, Completion Notice & Launch Guide
- Updated [`install-salsa.sh`](install-salsa.sh) to automatically open a visible desktop terminal window (`konsole`, `gnome-terminal`, `kgx`, `xfce4-terminal`, `kitty`, `alacritty`, `wezterm`, `foot`, `xterm`, or `$TERMINAL`) when launched outside an interactive TTY on X11/Wayland, show `[0/6]` bootstrap compilation progress when needed, and hold the window open until Enter is pressed so users can inspect everything installed (supports `--no-terminal` / `SALSA_INSTALLER_NO_TERMINAL=1` for scripted use).
- Updated [`salsa_installer.rs`](rust/salsa_tools/src/bin/salsa_installer.rs) with a structured `[1/6]`–`[6/6]` progress indicator (`print_stage()`), descriptive Flatpak & KDE Plasma setup notes from `configure_flatpak_and_kde()`, live user-service status checks (`lasapi-audiodg.service`, `lasapi-tray.service`, `lasapi-module-watcher.path`), a clear **What Was Installed & Configured** ledger, an explicit **Where To Find & Open Salsa** guide (System Tray, Application Launcher `.desktop` files, `~/.local/bin/` CLI/GUI commands, `$PATH` check, and `--check` / `disable-auto` commands), and a desktop completion notification via `notify-send` / `kdialog` / `zenity` (`--no-notify` / `SALSA_INSTALLER_NO_NOTIFY=1` opt-out).
- Added top navigation links and an expandable Architecture Breakdown preview to [`README.md`](README.md) linking directly to [`SALSA Architecture Breakdown.md`](SALSA%20Architecture%20Breakdown.md), [`PATCH_NOTES.md`](PATCH_NOTES.md), [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md), and [`COPYING.LESSER`](COPYING.LESSER).

---

## V1.19.7 (`2026-11-19`)

### 1. PulseAudio Reconnect Latency Fallback & Benchmark Accuracy Fixes
- Fixed a boolean gating bug in [`imp.rs`](rust/salsa_pa_reconnect/src/imp.rs) (`pa_stream_get_time()`, `pa_stream_get_latency()`, and `pa_stream_get_timing_info()`) via `stream_is_reconnecting_fallback()`: healthy live PulseAudio streams no longer swallow `-PA_ERR_NODATA` or inject a hardcoded `20_000` µs (`20.00 ms`) fallback before initial server timing arrives, while preserving monotonic stream time and `20_000` µs fallback strictly during active daemon reconnects (covered by a new `fallback_requires_active_reconnect` unit test).
- Updated [`lasapi_bench.rs`](rust/lasapi_client/src/bin/lasapi_bench.rs) `[3/3]` end-to-end latency measurements to average `4` phase-distributed WaveRT impulse trials cleanly (`RtPriorityGuard`, `drain_loopback()`, `build_test_impulse()`, `wait_for_loopback_impulse_ms()`), benchmark real end-to-end impulse round-trips through both `LASAPI Hollow Pulse Shim (libpulse-simple)` (`lasapi-run` zero-IPC SHM) and low-latency `Salsa Native PA Socket (Low-Lat Impulse)` (`pa_buffer_attr`), and clearly label the unconfigured `attr = NULL` probe as `Salsa Native PA Socket (Default attr=NULL)`.

### 2. Tray Quick Mixer Layout Clipping & Friendly Hardware Labels
- Resolved right-edge clipping in the Qt6 tray popup ([`lasapi_tray.cpp`](qt6/lasapi_tray.cpp)) by computing popup width dynamically from `card_->minimumSizeHint()` (`computePopupWidth()`), keeping device/preset `QComboBox` widgets compact with `Qt::ElideRight`, ignoring long Bluetooth pill minimum-width expansion, and re-anchoring cleanly on `keepBottomAnchored()`, backed by `verifyLayoutSelfTest()` in `--self-test`.
- Added shared `friendlyDeviceLabel()` in [`salsa_common.hpp`](qt6/salsa_common.hpp) / [`salsa_common.cpp`](qt6/salsa_common.cpp) and wired it into [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) and [`salsa_studio.cpp`](qt6/salsa_studio.cpp) so `/dev/snd/pcmC*D*` playback and capture paths display human-friendly endpoint names in tray faders, OSD volume overlays, and Studio headers.

---

## V1.19.6 (`2026-11-19`)

### 1. Full GNU LGPL-3.0 (`LGPL-3.0-only`) Compliance
- Replaced standalone `LICENSE` with the canonical FSF license pair [`COPYING`](COPYING) (verbatim GNU GPL-3.0 base terms) and [`COPYING.LESSER`](COPYING.LESSER) (verbatim GNU LGPL-3.0 permissions), and added [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) documenting Salsa's zero external `crates.io` dependencies, dynamically linked frameworks (Qt6 under `LGPL-3.0-only`), library relinking / Corresponding Source terms (`LGPL-3.0` §4), and optional runtime `dlopen()` codec policy (`libfdk-aac.so.2`, `libldac`, `libfreeaptx`, `libsbc`, `librnnoise`, `libdbus-1` — never bundled or vendored).
- Added standard `SPDX-License-Identifier: LGPL-3.0-only` and `Copyright (C) 2026 Linuxknows` headers across all Rust, C++, C ABI header, and build/install source files ([`Makefile`](Makefile), [`install-salsa.sh`](install-salsa.sh), `qt6/*`, and `rust/*/src/**/*.rs`).
- Added centralized legal notices ([`legal.rs`](rust/salsa_net/src/legal.rs)), `--version` / `--license` flags across CLI and GUI binaries (`lasapi-ctl`, `lasapi-audiodg`, `lasapi-run`, `lasapi-autosync`, `salsa-installer`, `salsa-debug`, `salsa-studio`, `lasapi-tray`), a shared themed About & License viewer dialog (`About / LGPL-3.0`) in [`salsa_studio.cpp`](qt6/salsa_studio.cpp), [`salsa_debug.cpp`](qt6/salsa_debug.cpp), and [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp), plus **Chapter 13 (`13. License`)** in [`studio_manual.rs`](rust/salsa_ui/src/studio_manual.rs).
- Updated [`salsa_installer.rs`](rust/salsa_tools/src/bin/salsa_installer.rs) to install `COPYING`, `COPYING.LESSER`, and `THIRD_PARTY_LICENSES.md` into `~/.local/share/licenses/salsa/` (and verify/uninstall cleanly), and added automated SPDX/license/CLI compliance verification to [`salsa_regression.rs`](rust/salsa_tools/src/bin/salsa_regression.rs).

---

## V1.19.5 (`2026-11-19`)

### 1. Bluetooth A2DP Dropout, Socket Backpressure & Lock-Free RT Fixes
- Hardened the BlueZ A2DP transmit loop across [`bluez_media.rs`](rust/salsa_net/src/bluez_media.rs), [`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs), and [`unix_sys.rs`](rust/salsa_net/src/unix_sys.rs): raised the target TX ring jitter headroom from `20 ms` (`960` frames, smaller than one `1024`-frame AAC packet) to `45 ms` / `2x` packet size (`bt_tx_margin_frames`) consistently across prebuffering and overflow trimming, widened/softened drift scaling (`[0.70 .. 1.65]` deadband, `1.0006`/`0.9994`), avoided blacking out the RF stream on isolated partial pops (`should_reprebuffer_after_partial_pop`), tuned L2CAP socket buffers/priority (`SO_SNDBUF`, `SO_PRIORITY`), added bounded 4-slice `POLLOUT` retries on transient `EAGAIN` (`send_l2cap_packet`) with `BT_TX_CONGEST` logging and `period_ns * 2` cadence recovery, preserved queued TX ring frames in `set_tx_enabled_impl` unless unpausing, and replaced the `rt.state.lock()` on the `SCHED_FIFO 65` audio thread (`bluez_write_period_impl`) with a lock-free `tx_transport_ready` atomic flag.
- Added host Bluetooth USB power inspection (`inspect_bt_usb_power`) to [`debug_core.rs`](rust/salsa_tools/src/debug_core.rs) and [`salsa_installer.rs`](rust/salsa_tools/src/bin/salsa_installer.rs) so `salsa-debug-cli` and `salsa-installer --check` surface `btusb` / `power/control=auto` autosuspend warnings automatically.

### 2. Full Interactive Control Tooltips (Studio, Tray Quick Mixer & Live Debugger)
- Added descriptive tooltips explaining purpose and operation across every interactive button, dropdown, slider, fader, spinbox, checkbox, shortcut capture input, tab, context menu action, theme preview control, and hoverable Topology node across [`salsa_studio.cpp`](qt6/salsa_studio.cpp), [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp), [`salsa_debugger_widget.cpp`](qt6/salsa_debugger_widget.cpp), [`salsa_debug.cpp`](qt6/salsa_debug.cpp), [`salsa_widgets.cpp`](qt6/salsa_widgets.cpp), and [`salsa_common.cpp`](qt6/salsa_common.cpp).

### 3. Silent Automated Test Suites & Quiet Loopback Probe
- Updated [`lasapi_test.rs`](rust/lasapi_client/src/bin/lasapi_test.rs) so Tests 1–6 stream digital silence and Test 7's loopback verification uses a quiet ultrasonic `19.8 kHz` (`0.01`) probe signal, and updated [`lasapi_bench.rs`](rust/lasapi_client/src/bin/lasapi_bench.rs) to probe loopback timing with the same quiet ultrasonic wave rather than a `0.55` DC pop.

### 4. License Change (`LGPL-3.0`)
- Switched project licensing from MIT to **GNU Lesser General Public License v3.0 (`LGPL-3.0`)** in `LICENSE`, `rust/*/Cargo.toml`, and [`README.md`](README.md).

---

## V1.19.4 (`2026-11-19`)

### 1. Fixed-Width Studio Output Peak Readout (`dynRow` Layout Stability)
- Locked `lblPeaks_` (`Output Peak L/R: ... dBFS`) in [`salsa_studio.cpp`](qt6/salsa_studio.cpp) with `Qt::AlignRight | Qt::AlignVCenter`, `QSizePolicy::Fixed`, and a font-scale-aware monospace `setFixedWidth` computed from `"Output Peak L/R: -100.0 / -100.0 dBFS"` in `applyThemeAndFontAll`, alongside minimum widths on `lblPreamp_`, `lblThresh_`, and `lblRatio_`, preventing `120 Hz` peak updates across `-10 dBFS` / `-100 dBFS` thresholds from shifting the dynamics slider bar horizontally.
- Replaced the discontinuous `1e-6` (`-120 dBFS` to `-60.0 dBFS`) jump in `pollEngine` with continuous clamping over `[-100.0, 6.0]` dBFS, fixed `6`-character field padding (`arg(..., 6, 'f', 1)`), and a dirty check before calling `setText`.

---

## V1.19.3 (`2026-11-19`)

### 1. Clean Device Dropdowns & Quick Mixer Enclosure Layout Polish
- Removed redundant ` [Bluetooth]` and ` [alsa-kernel]` driver suffixes from Studio's `cmbSink_` (`Output Sink`) and `cmbSource_` (`Mic Input`) dropdown labels in [`salsa_studio.cpp`](qt6/salsa_studio.cpp) (keeping device IDs in `Qt::UserRole` item data and adding `d.name` to `lastDevSig_` for live codec refreshes).
- Removed the `setMaximumHeight(320)` cap on `appsScroll_` in [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) and aligned `appsScroll_` / `appsLay_` to `Qt::AlignTop`, keeping `ACTIVE APP STREAMS` and stream faders anchored flush at the top of the Applications enclosure; unified `TabUnderline` QSS font weight (`700`) and computed explicit `QFontMetrics` `minimumWidth` values for `btnTabDev_` / `btnTabApps_` so `Applications (N)` never clips its closing parenthesis.

### 2. Perceptual Logarithmic Tray Peak Meter Scaling
- Updated `PeakBarWidget::setPeaks` in [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) to convert raw linear `output_peak_l` / `output_peak_r` amplitudes onto a logarithmic `-56 dBFS` to `0 dBFS` meter scale with `8 ms` rise and `85 ms` fall time constants so quiet-to-normal playback animates responsively across the stereo bars and transitions cleanly into amber/red on hot transients.

---

## V1.19.2 (`2026-11-19`)

### 1. Preset & Custom Theme Studio Color Fix
- Fixed `themeFromJsonObj` in [`salsa_common.cpp`](qt6/salsa_common.cpp) to unwrap the nested `"theme"` object returned by [`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs) (`theme_to_wire_json`) while remaining compatible with flat `custom_theme` JSON, and preserved `SalsaThemeData::name` across [`salsa_common.hpp`](qt6/salsa_common.hpp) and `themeToJsonObj` — fixing preset switching (`Catppuccin Mocha`, `Nordic Frost`, `Daylight Mastering`, etc.) and eliminating all-`#ffffff` swatches in `SalsaCustomThemeDialog`.
- Updated [`salsa_widgets.cpp`](qt6/salsa_widgets.cpp), [`salsa_studio.cpp`](qt6/salsa_studio.cpp), and [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) so `ColorSwatchRow` preserves swatch colors on hover/press, `SalsaCustomThemeDialog` syncs `cmbClone_` and cleanly reverts mirroring on cancel, `StudioController` initializes from `currentTheme_.name` and reloads external theme updates, and `TrayOsdWindow` / tray badges derive their fills and accents from the active theme.

### 2. Output-Matched Telemetry FPS (`Studio` + `Tray`) & Edge-to-Edge EQ Curve
- Added display-refresh telemetry helpers (`salsaDetectedOutputHz`, `salsaTelemetryIntervalMs`) in [`salsa_common.cpp`](qt6/salsa_common.cpp) and switched polling in [`salsa_studio.cpp`](qt6/salsa_studio.cpp) and [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) to `Qt::PreciseTimer` at display-matched intervals (up to `120 Hz` / `8 ms` when visible) with delta-time-normalized meter/spectrum smoothing in `SalsaSpectrumCanvas::updateTelemetry` and `PeakBarWidget::setPeaks` plus dirty checks to cut idle repaints.
- Updated `SalsaSpectrumCanvas::paintEvent` in [`salsa_widgets.cpp`](qt6/salsa_widgets.cpp) so the spectrum plot clip, gridlines, EQ response fill, and curve span cleanly edge-to-edge (`w - 3.0`) instead of stopping short of the right border.

### 3. ScreenCast Live Video Feed (GBM/EGL DMA-BUF Detiling) & Output-Matched FPS
- Added [`dmabuf_gl.rs`](rust/salsa_net/src/dmabuf_gl.rs) (`DmabufGlBlitter` via Mesa GBM/EGL/GLES2 `GL_TEXTURE_EXTERNAL_OES`) to detile GPU DMA-BUF ScreenCast buffers into preview SHM frames cleanly, and wired it into [`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs) alongside fixing premature `preview_expiry_ns` timeouts.
- Expanded PipeWire video format negotiation in [`pw_wire.rs`](rust/salsa_net/src/pw_wire.rs) (`IntersectedVfmt`, `build_negotiated_format_pod_iv`) and [`ipc.rs`](rust/salsa_tools/src/ipc.rs) (`is_valid_sc_fps`) up to `240/1` (`Source Hz` default `0`), added output-matched frame pacing and `lastShmSeq_` deduplication in [`salsa_debugger_widget.cpp`](qt6/salsa_debugger_widget.cpp), and added `--live-feed` to [`salsa_debug.cpp`](qt6/salsa_debug.cpp).

### 4. Exhaustive Headless Text Debugger Export (`--export-text`)
- Added `DiagnosticProbes::build_detailed_text_report` and `export_text_file` in [`debug_core.rs`](rust/salsa_tools/src/debug_core.rs) (also bundling `report.txt` into `--export` zip archives, ANSI-stripped `kscreen-doctor -o`, and `ss -xanH` socket scans), CLI flags `--export-text` / `--text` + `BrokenPipe` handling in [`debug_cli.rs`](rust/salsa_tools/src/debug_cli.rs) and [`salsa_debug.cpp`](qt6/salsa_debug.cpp), and regression coverage in [`salsa_regression.rs`](rust/salsa_tools/src/bin/salsa_regression.rs).

---

## V1.19.1 (`2026-11-19`)

### 1. Synchronized Global Font Scaling & Debugger Telemetry Layout Polish
- Unified font scale persistence on `ui_font_scale_pct` (syncing `font_scale_pct` in [`theme.rs`](rust/salsa_ui/src/theme.rs)) and switched `checkSettingsSync` / `checkSettingsFileSync` across [`salsa_studio.cpp`](qt6/salsa_studio.cpp), [`salsa_debug.cpp`](qt6/salsa_debug.cpp), and [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) to read `salsa_load_font_scale()`, fixing `Aa-`/`Aa+` snap-back across open windows.
- Added `Ctrl+=` / `Ctrl+-` bindings alongside `ZoomIn`/`ZoomOut`, dynamic table row height & mixer label width scaling, wider `FontStepBtn` minimums in [`salsa_common.cpp`](qt6/salsa_common.cpp), two-row Debugger action bars, responsive zero-clipping telemetry cards, `0.01%`-resolution `% RT CPU` progress bars, and `2:3` `tblPaClients_` / `tblPaStreams_` splitter sizing in [`salsa_debugger_widget.cpp`](qt6/salsa_debugger_widget.cpp).

### 2. Pacman Hook Fix, `v7.0-beta` Tray Meter/Fader Parity & Smooth Studio Sliders
- Fixed Pacman hook status/toggle detection in [`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs) (`99-salsa-autosync.hook`), switched [`salsa_installer.rs`](rust/salsa_tools/src/bin/salsa_installer.rs) from metadata-only `touch` to nanosecond timestamp writes + `PathModified=` watcher triggers (including `qt6-base`, `dbus`, `lib32-alsa-lib`, `pipewire-alsa`), and added parallel `-j<N>` rebuilds + `lasapi-tray.service` restarts to [`lasapi_autosync.rs`](rust/salsa_tools/src/bin/lasapi_autosync.rs).
- Restored exact `v7.0-beta` linear `8px` dual `PeakBarWidget` meters, capsule `SwipeFader` geometry with `#F8FAFC` circular thumb knobs, vector `SalsaOsdOverlay` speaker/mic glyphs, and `Shift`/horizontal tray Mic scroll, installed global `SalsaSliderProxyStyle` left-click seek-and-drag across all Qt6 sliders, raised Studio's 10-band EQ faders, fixed `tblSessions_` column clipping, and removed user-facing Mic APO controls cleanly.

---

## V1.19.0 (`2026-11-19`)

### 1. Native C++20 Qt6 GUIs (`salsa-studio`, `salsa-debug`, `lasapi-tray`) & Global Font Scaling
- Replaced legacy Python/PyQt6 scripts with native C++20 Qt6 frontends ([`qt6/`](qt6/)) linked directly to the Rust FFI bridge ([`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs) / [`salsa_bridge.h`](qt6/salsa_bridge.h)) with `Ctrl+=`/`Ctrl+-`/`Ctrl+0` & synchronized `Aa-`/`Aa+` global font scaling (`ui_font_scale_pct`) across Studio, Topology, Advanced Options, Live Debugger, Quick Mixer popup, context menu, and OSD, plus responsive telemetry card wrapping, `0.01%`-resolution RT CPU metering, and unclipped stream/session tables.
- Restored the `v6.9` Signal Topology & Owner's Manual window (`--topology` / `--graph`) with live signal flow animation, translucent tinted node cards, pixel-exact typography, and 12 interactive manual chapters ([`salsa_widgets.cpp`](qt6/salsa_widgets.cpp), [`salsa_studio.cpp`](qt6/salsa_studio.cpp)).
- Fixed imported AutoEQ `.txt` profile persistence & reload ([`studio_helpers.rs`](rust/salsa_ui/src/studio_helpers.rs)) and per-device EQ switching so custom imported AutoEQ curves are remembered rather than overwritten.

### 2. Full Quick Mixer Tray Parity, Meters/Mouse Interaction, Pacman Autosync & Direct Mic Capture
- Restored exact [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp) `v7.0-beta` `SwipeFader` capsule pills (rounded capsule groove, `100%` unity tick, `#F8FAFC` circular thumb knob with colored ring, and 35ms coalesced IPC + hold guards), `8px` dual linear `PeakBarWidget` stereo meters (`#0f172a` rails), vector `SalsaOsdOverlay` speaker/waves/mic glyphs, and `Shift`/horizontal Mic tray icon scrolling.
- Added global `SalsaSliderProxyStyle` (`SH_Slider_AbsoluteSetButtons` on left-click) and bordered groove + active fill QSS tracks in [`salsa_common.cpp`](qt6/salsa_common.cpp), raised EQ vertical faders to full height (`scaledPx(112)`), fixed `StepVolumeSlider` notch scaling + commit signals, added 40ms coalescing/hold guards and proper column widths in Studio's `tblSessions_`, and streamlined [`lasapi_tray.cpp`](qt6/lasapi_tray.cpp), [`salsa_studio.cpp`](qt6/salsa_studio.cpp), [`salsa_widgets.cpp`](qt6/salsa_widgets.cpp), [`studio_manual.rs`](rust/salsa_ui/src/studio_manual.rs), and [`ctl_cli.rs`](rust/salsa_net/src/ctl_cli.rs) by removing user-facing Mic APO controls in favor of clean raw hardware capture.
- Fixed Pacman hook detection in [`qt_bridge.rs`](rust/salsa_ui/src/qt_bridge.rs) (`99-salsa-autosync.hook`), switched `PACMAN_HOOK_CONTENT` from metadata-only `touch` to nanosecond timestamp writes (`date +%s%N > ...`) + `PathModified=` watcher triggers (including `qt6-base`, `dbus`, `lib32-alsa-lib`, `pipewire-alsa`) in [`salsa_installer.rs`](rust/salsa_tools/src/bin/salsa_installer.rs), and enabled parallel `-j<N>` rebuilds + `lasapi-tray.service` restarts in [`lasapi_autosync.rs`](rust/salsa_tools/src/bin/lasapi_autosync.rs).
- Fixed Wayland-to-Qt DPR popup/OSD placement and C++ argument evaluation ordering for the active header icon, and updated [`salsa_common.cpp`](qt6/salsa_common.cpp) to detach child Wayland apps into transient `systemd-run --user --scope` units cleanly outside `lasapi-tray.service`.

### 3. 100% Pure Rust Client Shims, Tools, Regression & Audit Hardening
- Completed migration of `liblasapi.so`, `libasound_module_pcm_lasapi.so`, `libsalsa_pa_reconnect.so` + 200-stub `pa_reconnect/libpulse.so.0`, hollow ALSA/Pulse/JACK shims, `fuzz_wire`, `salsa-installer`, `salsa-dist`, `salsa-debug-cli`, and `salsa-regression` into the offline Cargo workspace ([`rust/`](rust/Cargo.toml)) with zero C runtime sources remaining outside shim stub assembly.
- Updated [`debug_core.rs`](rust/salsa_tools/src/debug_core.rs), [`host.rs`](rust/salsa_tools/src/host.rs), and [`salsa_debug.cpp`](qt6/salsa_debug.cpp) with per-binary Qt6 dependency tracking, `SOCK_DGRAM` `debug-in.sock` audit verification, and complete CLI flag forwarding (`--probe`, `--audit`, `--snapshot`, `--stream`, etc.).

---

## v7.0-beta (`2026-11-18`)

### 1. Local Video Playback (VLC / `mpv` / Haruna) — PulseAudio `pa_format_info` Negotiation
- Added full protocol v21+ `pa_format_info` parsing/serialization (`PaFormatInfo`, `read_format_info`, `put_pcm_format_spec`) in [`pa_wire.rs`](rust/salsa_net/src/pa_wire.rs) and wired it into playback/record stream creation and sink-input/source-output replies in [`pa_server.rs`](rust/salsa_net/src/pa_server.rs), fixing `pa_stream_new_extended` clients that previously received `PA_SAMPLE_INVALID` / `0 Hz` and aborted (`SRC ratio outside [1/256, 256] range`, crashes in `pa_cvolume_set()` / `pa_bytes_to_usec()`).
- Hardened `salsa_pa_reconnect.c` so reconnects never expose a zero-initialized sample spec, enlarged the per-stream ring to absorb VLC's ~1.25 s upfront write burst (no more `playback way too early` silence loops), and decoded all channels in `handle_data_memblock` so 5.1/7.1 MKV center/LFE/surrounds are no longer dropped.

### 2. Discord / PipeWire ScreenCast — Full Resolution, Smooth FPS & A/V Sync
- [`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs) now renegotiates format and buffers on `PORT_UPDATE` for active links (resize / fullscreen), forwards `SPA_META_VideoCrop` ([`pw_wire.rs`](rust/salsa_net/src/pw_wire.rs)), raises buffer headroom from 3 to 4–6, and recycles unconsumed consumer buffers back to KWin instead of leaking them on 120/240 Hz displays.
- Stamps monotonic `clock.nsec` / `clock.position` and `SPA_META_Header.pts` / `seq` on every frame for accurate WebRTC A/V sync, and reports live relay FPS (`relay_actual_fps_x100`) whenever a link is actively casting, not only while preview is open.

### 3. Debug Audit Reliability Fixes
- Atomic `.tmp` + `mv -f` installs for every binary, archive, shared library and symlink in [`Makefile`](Makefile), with stale `.tmp` sweeps in `lasapi-autosync.sh` — rebuilding while the daemon runs no longer hits `ETXTBSY` or crashes running clients.
- [`pa_server.rs`](rust/salsa_net/src/pa_server.rs) validates default sink/source IDs against cached devices, maps `salsa_out.virtual(.monitor)` to the active hardware sink, falls back gracefully on stale `salsa_in.*` names, and logs the failing PA command name while downgrading benign teardown races to `TRACE` (silences `ERR_NOENTITY` bursts from `plasmashell` / Discord).
- [`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs) waits for a two-packet A2DP startup prebuffer and extends the inline refill wait to `24 ms`, eliminating startup `BT_TX_UNDERRUN`s.
- `salsa_debug_core.py` / `salsa_debug_ui.py` track `pa_reconnect` build freshness, label the "Salsa Pure-Rust Video Broker", preserve saved cast-limit preferences on close, and show live ScreenCast FPS; `lasapi-tray.py` / [`install-salsa.sh`](install-salsa.sh) add single-instance session-restore guards.

---

## v6.9-beta (`2026-11-16`)

### 1. 100% Pure Rust `lasapi-audiodg` Daemon & `lasapi-ctl` CLI
- Completed full migration of the engine, controller, BlueZ A2DP/AVRCP/HFP SCO runtime, ALSA/capture pipeline, HW analog stage, Dynamic EQ/Mic APO DSP, persistent state, debug server, and DAW MIDI hub into the `salsa_net` Rust crate ([`audiodg_engine.rs`](rust/salsa_net/src/audiodg_engine.rs), [`bluez_ffi.rs`](rust/salsa_net/src/bluez_ffi.rs), [`bluez_cap.rs`](rust/salsa_net/src/bluez_cap.rs), [`bluez_runtime.rs`](rust/salsa_net/src/bluez_runtime.rs), [`ctl_cli.rs`](rust/salsa_net/src/ctl_cli.rs), [`debug_server.rs`](rust/salsa_net/src/debug_server.rs), [`dsp.rs`](rust/salsa_net/src/dsp.rs), [`hw_analog.rs`](rust/salsa_net/src/hw_analog.rs), [`midi_hub.rs`](rust/salsa_net/src/midi_hub.rs), [`state_conf.rs`](rust/salsa_net/src/state_conf.rs)) with native `[[bin]]` targets [`lasapi_audiodg.rs`](rust/salsa_net/src/bin/lasapi_audiodg.rs) and [`lasapi_ctl.rs`](rust/salsa_net/src/bin/lasapi_ctl.rs).
- Replaced static `-ldbus-1` / `-lsbc` linkage in the daemon and CLI with runtime `dlopen`/`dlsym` loaders across [`bluez_ffi.rs`](rust/salsa_net/src/bluez_ffi.rs) and non-variadic `DBusMessageIter` bindings while preserving `BIND_NOW` / PIE Full RELRO hardening (`RUSTFLAGS="-C link-arg=-Wl,-z,relro,-z,now"`).

### 2. Removal of Legacy C-Fallback Backend & Direct Rust Fuzzing
- Removed the `24,316`-line legacy C backend/fallback/bridge files (`lasapi_audiodg*.c`, `lasapi_bluez.c`, `lasapi_ctl*.c`, `lasapi_debug*.c`, `lasapi_dsp_c.c`, `lasapi_hw_analog.c`, `lasapi_midi.c`, `lasapi_pa_server.c`, `lasapi_pw_broker.c`, and `salsa_wire.c`) along with `USE_RUST_WIRE=0` conditional rules in [`Makefile`](Makefile), leaving C code strictly in the external client `.so` shims/proxies and standalone C bench/test tools.
- Replaced differential C-reference fuzzing with standalone ASan/UBSan `libFuzzer` verification directly against `libsalsa_wire.a` in `tests/fuzz/fuzz_wire.c`, updated `salsa_debug_core.py` build freshness checks to track Rust crate sources, and updated [`install-salsa.sh`](install-salsa.sh) dependency checks for `cargo`/`rustc`.

---

## v6.8-beta (`2026-11-16`)

### 1. Memory-Safe Rust Wire Core, PulseAudio v35 Server & PipeWire-0 Video Broker
- Added zero-dependency offline Rust static library `salsa_net` (`libsalsa_wire.a` in [`rust/salsa_net/`](rust/salsa_net/Cargo.toml)) exposing memory-safe bounds-checked parsers, builders, FD-quota accounting, and full UNIX socket daemons across [`pa_server.rs`](rust/salsa_net/src/pa_server.rs), [`pw_broker.rs`](rust/salsa_net/src/pw_broker.rs), [`pa_wire.rs`](rust/salsa_net/src/pa_wire.rs), [`pw_wire.rs`](rust/salsa_net/src/pw_wire.rs), [`bluez_wire.rs`](rust/salsa_net/src/bluez_wire.rs), [`failsafe.rs`](rust/salsa_net/src/failsafe.rs), [`unix_sys.rs`](rust/salsa_net/src/unix_sys.rs), [`c_abi.rs`](rust/salsa_net/src/c_abi.rs), and [`ffi.rs`](rust/salsa_net/src/ffi.rs), declared in `include/salsa_wire.h` with automatic/selectable `make USE_RUST_WIRE=0` C11 fallback.
- Replaced active C PulseAudio v35 server (`lasapi_pa_server.c`), PipeWire-0 ScreenCast video broker (`lasapi_pw_broker.c`), and BlueZ A2DP SEP codec selection / BDADDR parsing (`select_a2dp_config_for_endpoint`, `parse_bdaddr_str` in `src/lasapi_bluez.c`) with hardened Rust implementations compiled into `lasapi-audiodg` and `lasapi-ctl` via [`Makefile`](Makefile).

### 2. Quiet Wire Failsafe Bar, Non-Disruptive Client Flush & Differential Fuzzing
- Added silent automatic failsafe telemetry (`"wire"` object emitted by `src/lasapi_debug.c`) and `RESET_NET_CLIENTS` IPC handling in `src/lasapi_audiodg.c` to flush hung PulseAudio / PipeWire-0 sockets without interrupting active WaveRT hardware audio playback.
- Added the compact Wire Core & Failsafe status bar (`Reset Hung Clients`) to the standalone debugger in `salsa_debug_ui.py`, added Clang `libFuzzer` + ASan/UBSan differential fuzzer (`tests/fuzz/fuzz_wire_diff.c`, `make test-sanitized`), and expanded live regression tests in `scripts/run_regression_tests.py`.

---

## v6.7-beta (`2026-11-16`)

### 1. Option C Pacman ALPM Hook Indicator, Toggles & Instant Sync
- Replaced the read-only Diagnostics integration text in `salsa-gui.py` (`_build_advanced_options_window`, `_refresh_adv_integration_status`) with themed status pills (`● Steam`, `● Launchers`, `● Pacman Hook: ACTIVE / OFF`), a clickable `Toggle Pacman Hook (pkexec)` button (`_toggle_pacman_hook`) running non-blocking `install-salsa.sh --install-pacman-hook` / `--remove-pacman-hook`, and an **Auto-Sync Watcher + Sync Now** toggle (`chk_autosync_now` / `_on_autosync_watcher_clicked`) that enables or disables `lasapi-module-watcher.path` and runs `lasapi-autosync.sh --force` immediately.

### 2. Configurable 5% Mouse-Wheel & Keyboard Audio Step Sizing + `vX.X-beta` Versioning
- Added shared `DEFAULT_VOL_STEP_PCT` (`5`), `SUPPORTED_VOL_STEPS` (`1, 2, 5, 10`), and `normalize_vol_step_pct` in `salsa_ipc.py` synced via `gui_settings.json` (`"volume_step_pct"`).
- Upgraded Studio's Speaker, Mic Gain, and per-app session sliders to `StepVolumeSlider` (`±5%` wheel & arrow/`PageUp`/`PageDown` steps matching global keyboard shortcuts) alongside the `Wheel/Key Step` header dropdown (`1% / 2% / 5% / 10%`), updated `SwipeFader` and `_on_sni_scroll` in `lasapi-tray.py` to `±5%` with the synchronized `Step ±5%` badge pill, simplified Chapter 1 Philosophy branding, and transitioned release/installer/UI versioning (`VERSION`, `Makefile`, `install-salsa.sh`, `README.md`, `PATCH_NOTES.md`, `salsa-gui.py`, `src/lasapi_ctl.c`) to `vX.X-beta`.

---

## v6.6-beta (`2026-11-16`)

### 1. Multi-Window Studio Flow: Topology Intro, Pop-Out Advanced Options & Standalone Debugger
- Split the Auto-Graph Signal Topology & Owner's Manual view into its own standalone startup intro window (`_build_topology_intro_window` in `salsa-gui.py`) with a `"Don't show Topology Intro on startup"` persistence toggle (`show_topology_intro`), `"Enter Studio Mixer ->"` button, and top-bar `"Topology (F1)"` toggle to reopen anytime (or via `salsa-graph`).
- Removed the embedded `SalsaLiveDebuggerWidget` tab and `QTabWidget` wrapper from `salsa-gui.py`; `salsa-debug.py` now launches strictly as its own separate standalone program/window via **Advanced Options**, while Topology keeps lightweight async Video PW Broker detection via `snapshot_once`.
- Moved `Hardware Analog Stage`, `Engine Ownership Toggle` (`Salsa Native` / `PipeWire`), `Send Test MIDI (C4)`, `Run Latency Benchmark`, `Run System Health Audit`, `Sync Audio Modules Now`, `Restart Audio Engine` & `Restart Tray Service`, `Open Custom Theme Studio`, `Reset Safe Defaults`, `Soft-Knee Limiter`, `Per-Device EQ`, `Import AutoEQ .txt`, and `Bauer Crossfeed` into the dedicated pop-out **Advanced Options** window (`_build_advanced_options_window`).

### 2. Customizable Keyboard Shortcuts (GUI & Standalone Debugger)
- Added customizable `QKeySequenceEdit` key-bind rows inside **Advanced Options** for 14 Studio/system actions (`DEFAULT_SHORTCUTS`, conflict auto-clearing, per-action clear, and `Restore Defaults`), persisted under `"shortcuts"` in `gui_settings.json` with live shortcut/tooltip rebinding (`_setup_shortcuts`).
- Added keyboard navigation shortcuts in `salsa-debug.py` (`Ctrl+1..5` sub-tabs, `F5` refresh, `Ctrl+W`/`Ctrl+Q` close) and expanded multi-window + shortcut regression assertions in `scripts/run_regression_tests.py`.

---

## v6.5-beta (`2026-11-16`)

### 1. Independent Capture / Mic Clock & Zero-Double-Resample Capture Pipeline
- Split capture clock control (`LASAPI_SetCaptureSampleRate`, `lasapi-ctl set-cap-rate <44100..192000|auto>`) across `include/lasapi.h`, `src/liblasapi.c`, `src/lasapi_audiodg.c`, `src/lasapi_bluez.c`, and `src/lasapi_ctl.c` so USB microphones (e.g., Focusrite Scarlett 2i2 at `48,000 Hz` for Discord/voice) run independently of a `192,000 Hz` playback Master Clock.
- Upgraded capture audio path with `lasapi_bluez_read_capture_native` and `lasapi_pa_server_push_capture` so hardware capture, Mic APO (`HPF`, gate, `48 kHz` RNNoise, compressor, clarity), and native `48 kHz` PulseAudio record clients stream `1:1` without double-resampling, using Catmull-Rom upsampling only when high-rate playback sidetone or `192 kHz` capture streams are active.

### 2. USB Capture Telemetry Watchdog, MIDI Key-Press Counter & Quick Mic APO Presets
- Added capture rate telemetry (`pref_rate`, `measured_rate`, `kick_count`, `is_usb`, `duplex_locked`) to `include/lasapi_debug.h`, `src/lasapi_debug.c`, and `salsa_debug_ui.py`.
- Updated `include/lasapi_midi.h` and `src/lasapi_midi.c` so routed MIDI telemetry counts actual Note-On key-press events (`0x90..0x9F` with non-zero velocity) instead of idle MIDI clock pulses.
- Added quick reversible Mic APO presets (`lasapi-ctl mic-preset <discord|broadcast|clean|bypass>`, plus `cmb_mic_preset` / `cmb_mic_preset_tray` and `cmb_cap_rate` selectors in `salsa-gui.py` and `lasapi-tray.py`), centralized shared ctypes bindings/presets into `salsa_ipc.py`, retired legacy Bose PipeWire workaround restore logic in `disable-auto`, and expanded live regression tests in `scripts/run_regression_tests.py`.

---

## v6.3-beta (`2026-11-14`)

### 1. Periodic Full-Duplex Static & Cross-Period Continuity Fixes
- Eliminated periodic full-duplex playback and capture static/clicks in `src/lasapi_pa_server.c` and `src/lasapi_audiodg.c` using rate-scaled ~0.5 ms de-click fade-outs (`pa_stream_emit_fade_out`) on flush/cork/underrun transitions, Catmull-Rom cubic history continuity (`pa_stream_save_hist`), per-frame capture gain smoothing (`cap_smooth_gain`), and cross-period boundary linear interpolation (`cap_hist_l`/`cap_hist_r` with `clamp_cap_unit`) in `lasapi_pa_server_push_capture`.

### 2. Full Restart State Persistence (Daemon, Studio GUI & System Tray)
- Upgraded daemon binary persistence in `src/lasapi_audiodg.c` to `'SALW'` v5 (`salsa_persisted_state_v5_t`) stored at `0600` in `~/.config/lasapi/salsa_state.bin` with 400 ms debounced `g_state_dirty` flushes and clean-shutdown saving, restoring `desktop_volume_pct`, `desktop_muted`, `desktop_mic_volume_pct`, `desktop_mic_muted`, and `desktop_mic_gain_ppm` alongside clock, routing, EQ, Mic APO, BT codec, and HW Analog settings.
- Updated `SalsaStudioWindow` in `salsa-gui.py` to persist and restore `window_width`, `window_height`, and `last_tab` via `save_gui_settings` (preserving disk `tray_*` keys), and synchronized `eq_per_device` / `eq_per_device_enabled` and `chk_eq_per_dev` across `salsa-gui.py` and `lasapi-tray.py` using `_read_gui_settings_mtime`.

### 3. Virtual Mix Bus Removal & Simplified Tab-Bar Controls (`Parametric Band` Strip Removed)
- Removed the 4 Virtual Broadcast Mix Buses (`salsa_bus.*`, `LASAPI_CMD_SET_MIX_BUS`, `LASAPI_SetMixBus`, `lasapi_mix_bus_info_t`, `mix-bus` CLI, virtual bus PA sinks/monitors, and the `VIRTUAL BROADCAST MIX BUSES` GUI panel) across `include/lasapi.h`, `include/lasapi_pa_server.h`, `src/liblasapi.c`, `src/lasapi_audiodg.c`, `src/lasapi_pa_server.c`, `src/lasapi_ctl.c`, `lasapi-tray.py`, and `salsa-gui.py` while retaining per-app `play:`/`rec:` routing and `--virtual` fallback, and updated [`VERSION`](VERSION) to `v6.3-beta`.
- Removed the `Parametric Band:` / `Freq:` / `Q:` / `Shape:` widgets (`cmb_param_band`, `spn_param_freq`, `spn_param_q`, `cmb_param_shape`) from `TabsCornerSlot` in `salsa-gui.py`, keeping **Bauer Crossfeed** (`chk_crossfeed`, `sld_crossfeed`, `cmb_crossfeed_cut`), `Import AutoEQ .txt`, and backend `param-eq` CLI / `LASAPI_SetParametricEQ` DSP support intact.

### 4. Fixed Master Clock Restoration & Bit Depth Switching / Hardware Lock Feedback
- Restored `v6.0-beta`'s fixed **Master Sample Rate** selector (`44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz` via `LASAPI_SetEngineConfig`) across `include/lasapi.h`, `src/liblasapi.c`, `src/lasapi_audiodg.c`, `src/lasapi_ctl.c`, `lasapi-tray.py`, `salsa-gui.py`, `src/lasapi_test.c`, and `scripts/run_regression_tests.py`, removing Adaptive Lossless auto-rate switching while preserving `salsa_state.bin` (`'SALW'` v5) compatibility and Bluetooth Wireless ABR (`wireless_abr_enabled`).
- Fixed **Bit Depth / Precision** switching and persistence (`g_pref_format_bits`) in `src/lasapi_audiodg.c` so `reconfigure_engine_rate_impl` detects `bits_changed`, `open_kernel_pcm_device` negotiates ALSA formats ordered by requested bit depth (with `SNDRV_PCM_FORMAT_S24_LE` encoding support alongside `S16_LE`, `S24_3LE`, `S32_LE`, and `FLOAT_LE`) and steps fallback rates down within the same sample-rate family (`192→96→48 kHz`, `176.4→88.2→44.1 kHz`), and Virtual / BlueZ endpoints update and persist requested `16 / 24 / 32`-bit precision.
- Added `/proc/asound/card*/stream0` playback capability probing plus clear `HW Locked` status/tooltip feedback and `(DAC N/A)` dropdown annotations for unsupported hardware rates and bit depths in `salsa-gui.py`, `lasapi-tray.py`, and `cmd_set_rate` in `src/lasapi_ctl.c`.

---

## v6.2-beta (`2026-11-14`)

### 1. Hardware Analog Architecture, DAC Unity Attenuation, Amp Stage & USB-HID Control Box Support
- Added `include/lasapi_hw_analog.h` and `src/lasapi_hw_analog.c` with non-invasive kernel card/mixer probing (`lasapi_hw_analog_probe`), safe ALSA control syncing (`lasapi_hw_analog_sync` via `SNDRV_CTL_IOCTL_ELEM_WRITE`), DAC unity volume handling (`lasapi_hw_analog_is_dac_unity_active`), analog soft-knee amp boost (`lasapi_hw_analog_softknee_active`), capture makeup (`lasapi_hw_analog_get_cap_makeup_linear`), ASUS Strix Raid DLX/Soar CM6632AE UAC2 + `/dev/hidraw*` USB-HID volume/mute dial polling (`lasapi_hw_analog_poll_hid`), and card scoring (`lasapi_hw_analog_score_card`).
- Integrated `LASAPI_SetHwAnalog` and `lasapi-ctl hw-analog` controls and telemetry into `include/lasapi.h`, `src/lasapi_audiodg.c`, `src/lasapi_ctl.c`, `salsa-gui.py`, `src/lasapi_test.c`, and `scripts/run_regression_tests.py`.

---

## v6.1-beta (`2026-11-14`)

### 1. Adaptive Lossless Clock Default + Master Resample Override + Wireless Bitrate Adaptation
- Added Adaptive Lossless stream rate & depth tracking (`adaptive_rate_enabled = 1` default) in `src/lasapi_audiodg.c` and `src/lasapi_pa_server.c`, automatically matching the dominant active playback rate (`44.1 / 48 / 88.2 / 96 / 176.4 / 192 kHz`) while keeping **Master Resample Override** (`LASAPI_SetEngineAdaptive` / `lasapi-ctl set-rate <hz|auto>`) one click away.
- Added automatic **Wireless Bitrate Adaptation (ABR)** in `src/lasapi_bluez.c` scaling LDAC (`990 <-> 660 <-> 330 kbps` via `libldacBT_abr.so.2` / `ldacBT_set_eqmid`) and AAC (`256 <-> 192 <-> 128 kbps`) dynamically on L2CAP write stalls and link recovery.

### 2. Active BlueZ Codec Switcher, Diagnostics & Optional Two-Way HFP `BTPROTO_SCO`
- Added remote SEP probing (`supported_codecs_mask`), codec negotiation diagnostics (`neg_reason`), on-demand preferred codec switching (`Auto | LDAC | aptX HD | aptX | AAC | SBC`) with LDAC bitrate tier selection (`ABR | 990 | 660 | 330 kbps`), and optional two-way Bluetooth HFP `BTPROTO_SCO` (`mSBC`/`CVSD`) headset mic support (`hfp_sco_enabled = 0` by default so Focusrite Scarlett remains default) across `src/lasapi_bluez.c`, `src/lasapi_audiodg.c`, `src/lasapi_ctl.c`, `salsa-gui.py`, and `lasapi-tray.py`.

### 3. Full Parametric EQ, AutoEQ / Oratory1990 `.txt` Importer & Bauer Headphone Crossfeed
- Upgraded the real-time 10-band APO in `src/lasapi_audiodg.c` (`LASAPI_SetParametricEQ`) with per-band frequency (`15 Hz–22 kHz`), custom `Q` (`0.2–10.0`), filter topology (`Peak`, `Low-Shelf`, `High-Shelf`), and bypassable Bauer/BS2B stereo crossfeed (`process_crossfeed`), plus an EqualizerAPO / Oratory1990 `.txt` importer and Parametric Band Inspector in `salsa-gui.py`.

### 4. Real-Time Capture Microphone APO & Virtual Broadcast Mix Channels
- Added a bypassable Capture Microphone APO (`enabled = 0` by default so hardware mics like Focusrite Scarlett 2i2 pass raw) in `src/lasapi_audiodg.c` combining Voice HPF, hysteresis Noise Gate, Neural `librnnoise.so.0` denoising (with spectral fallback), Clarity shelf, Soft Compressor, and post-loopback low-latency headphone Sidetone (echo-free for OBS/loopback).
- Added 4 Virtual Broadcast Mix Channels (`salsa_bus.game`, `salsa_bus.chat`, `salsa_bus.browser`, `salsa_bus.music` + `.monitor` sources) in `src/lasapi_pa_server.c` with persisted `0600` faders, mutes, live meters, optional auto-routing, `'SALU'` v3 `0600` atomic engine persistence, and updated Tab 2 Signal Topology & Owner's Manual (`salsa-gui.py`).

---

## v6.0-beta (`2026-11-14`)

### 1. KDE Plasma 6 System Tray, Emerald Glow Icon, Swipe Faders, Floating OSD & Quick Mixer
- Rebuilt `lasapi-tray.py` around native `org.kde.StatusNotifierItem` D-Bus export (with automatic `QSystemTrayIcon` fallback when `org.kde.StatusNotifierWatcher` is absent) and high-DPI icon rendering in `TrayIconBank` (`0600` PNGs inside `0700` runtime directory).
- Added the active emerald-glow icon mode (`"tray_green_glow": true` default, toggleable to plain icon via right-click *"Green Glow on Active Icon"*), plus muted red-slash and stopped amber-dot states.
- Added vertical scroll for Master Output (`+/-3%`), horizontal scroll/swipe or `Shift`+scroll for Mic Input Gain (`+/-3%`), middle-click Master Mute toggle, and floating Plasma-style translucent OSD pills (`SalsaOsdOverlay`).
- Added `SalsaQuickMixerPopup` on left-click (revertable to direct Studio launch via right-click *"Left-Click Opens Quick Mixer"*), popping cleanly above the Plasma panel/tray (`QT_QPA_PLATFORM=xcb` default on XWayland so panel-aware positioning and smooth window dragging work reliably across multi-monitor setups; revertable anytime with `QT_QPA_PLATFORM=wayland`). The popover can be dragged freely anywhere by its header grip (`⠿`) or background with **Pin** / **Close** controls, displays live per-application program icons resolved securely via `AppIconResolver` (`/proc/<pid>` + XDG `.desktop` + system icon theme) alongside active stream swipeable faders, and defaults to the new **"Plasma Breeze Emerald"** charcoal-slate/emerald/sky-blue theme (added to both `salsa-gui.py` and `lasapi-tray.py`, toggleable back to Studio's active theme via right-click *"Quick Mixer: Breeze Slate Theme"*). All settings persist safely via `0600` atomic `gui_settings.json` writes.

### 2. Steam Reconnect Crash Fix & Pressure-Vessel Auto-Preload Wrapper
- Fixed lifetime, use-after-free, double-free, and locking hazards in `src/salsa_pa_reconnect.c` across rapid engine resets (`pa_threaded_mainloop`), including global thread-safe `tee_req_t` ownership tracking across `pa_context_fail()`, outside-of-lock dead handle flushing, global `wstream_t` registry validation, `pa_context_ref`/`pa_stream_ref` guards around async callbacks and timers, and `wctx->lock` synchronization on draining/sample/autoload/stat calls.
- Added an automatic, reversible `pressure-vessel` Steam wrapper in `src/lasapi_ctl.c` (`~/.local/bin/salsa-steam-launch` + updated `Exec=` in `~/.local/share/applications/steam.desktop` and `~/.config/autostart/steam.desktop` with `0600` `.bak` backups and desktop database refresh) managed automatically by `lasapi-ctl enable-auto`, `lasapi-ctl disable-auto`, and [`install-salsa.sh`](install-salsa.sh) `--uninstall`.

### 3. Full-Bleed Tray Icons, Redesigned Translucent OSD, KDE OSD Suppression & Bluetooth Battery Readout
- Updated `TrayIconBank` in `lasapi-tray.py` to tight-crop transparent padding and install crisp full-bleed `salsa-tray-*` icons (`0644`) into `~/.local/share/icons/hicolor/` with `KIconLoader.iconChanged` cache invalidation (cleaned up automatically by [`install-salsa.sh`](install-salsa.sh) `--uninstall`).
- Redesigned `SalsaOsdOverlay` as a `330x66` frosted-glass pill with vector speaker/mic icon, device name, emerald gradient progress bar, X11 KDE blur hint, and `300ms` external volume watcher, and updated [`install-salsa.sh`](install-salsa.sh) to automatically suppress duplicate KDE Plasma audio OSDs in `~/.config/plasmaparc` (saving `plasmaparc_osd.state` `0600` for clean rollback on `--uninstall`) and reload `kded6` `audioshortcutsservice`.
- Fixed `read_bluez_battery_pct` (with `_parse_dbus_pct` and UPower fallback) to decode PyQt6 `QtDBus` single-byte `org.bluez.Battery1.Percentage` `bytes` values and prioritize the active Bluetooth audio endpoint over unrelated peripherals, surfacing battery % in both `SalsaQuickMixerPopup` and the tray tooltip.

### 4. Steam `dlopen` Recursion & PulseAudio Extension (`pa_ext_*`) Crash Fixes
- Fixed `dlopen` constructor recursion in `src/salsa_pa_reconnect.c` when both `build/pa_reconnect/libpulse.so.0` and `LD_PRELOAD` `build/libsalsa_pa_reconnect.so` are active (`salsa_real_dlopen_passthrough` + multiarch SteamRT `libpulse.so.0` paths).
- Wrapped all 20 PulseAudio extension APIs from `ext-device-restore.h`, `ext-device-manager.h`, and `ext-stream-restore.h` (including `pa_ext_device_restore_read_formats_all` called by `steamrt64/libaudio.so`) in `src/salsa_pa_reconnect.c` with `wctx_t` callback unwrapping and reconnect re-subscription so Steam and `steamwebhelper` launch with zero coredumps inside `pressure-vessel`.

---

## v5.9-beta (`2026-11-14`)

### 1. Persistent Per-App Mixer, Mute & Stream Pinning + Per-Device EQ Auto-Switching
- Added persistent per-app volume/mute storage in `src/lasapi_pa_server.c` (`~/.config/lasapi/app_volumes.conf`, saved via atomic `O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW` `0600` temp-file rename inside `0700` dir, `O_NOFOLLOW` + bounded `<= 262144` byte reload, and `sanitize_app_vol_key` against injection/traversal).
- Added `LASAPI_CMD_SET_SESSION_CONTROL` and `LASAPI_SetSessionControl()` in `include/lasapi.h`, `src/liblasapi.c`, and `src/lasapi_audiodg.c`, plus CLI subcommands `app-volume`, `app-mute`, and `move-app` in `src/lasapi_ctl.c`.
- Upgraded `salsa-gui.py` `tbl_sessions` to 8 columns with flicker-free interactive Volume sliders (`0–150%`), Mute toggles, and Route Target dropdowns (`auto` or pinned device), hardened `load_gui_settings`/`save_gui_settings` (`0600` atomic write + `O_NOFOLLOW`), and added **Per-Device EQ** auto-switching (`eq_device_profiles` keyed by active sink ID).

### 2. High-Bitrate BlueZ Codecs (`LDAC`, `aptX HD`, `aptX`)
- Extended `include/lasapi_bluez.h` and `src/lasapi_bluez.c` to advertise and negotiate BlueZ D-Bus endpoints for **LDAC** (`0x0000012D`/`0x00AA`, 44.1–96 kHz), **aptX HD** (`0x000000D7`/`0x0024`), and **aptX** (`0x0000004F`/`0x0001`) alongside `AAC` and `SBC`, loading trusted system encoder libraries via `dlopen(..., RTLD_NOW|RTLD_LOCAL)` (`libldacBT_enc.so.2`, `libfreeaptx.so.0`) with strict `rcaps`/`cfg_len`/MTU bounds validation.

### 3. 32-Bit Multilib Proxy, Scoped Flatpak Overrides, Root-Safe Pacman Trigger & Regression Suite
- Updated `scripts/gen_pa_proxy.py` with strict C-identifier regex validation (`^[A-Za-z_][A-Za-z0-9_]*$`), `--arch i386` PIC stub emission, and atomic temp-file writes; updated [`Makefile`](Makefile), `src/salsa_pa_reconnect.c`, `lasapi-autosync.sh`, and `src/lasapi_ctl.c` to build and activate `build/pa_reconnect32` automatically when multilib `libpulse.so.0` + `gcc -m32` are installed.
- Updated [`install-salsa.sh`](install-salsa.sh) with scoped per-app Flatpak `--user` overrides (`--socket=pulseaudio`, `--filesystem=xdg-run/pipewire-0:ro`) tracked for clean `--uninstall` rollback, plus `--install-pacman-hook` / `--remove-pacman-hook` via `pkexec` using an unprivileged timestamp trigger (`/usr/bin/touch /var/lib/salsa-autosync/libpulse-updated`) so `root` never executes user-owned scripts.
- Added automated regression and security suite `scripts/run_regression_tests.py` wired into `make test`.

---

## v5.8-beta (`2026-11-14`)

### 1. Installer Revision Tracking & Patch Notes
- Added [`VERSION`](VERSION) (`v5.8-beta`) and updated [`Makefile`](Makefile) `installer` target to package `salsa-lasapi-installer (<tag>).zip` automatically.
- Updated [`install-salsa.sh`](install-salsa.sh) (`print_pc_report`) to display `(v5.8-beta)`.
- Created [`PATCH_NOTES.md`](PATCH_NOTES.md) changelog.

### 2. Seamless Firefox Audio Recovery Across Service Restarts & Reboots
- Fixed `cubeb-pulse-rs` crash in `src/salsa_pa_reconnect.c` (`refresh_defer_cb` now filters synthetic subscribe events against `wctx->has_subscribe` and `wctx->sub_mask`).
- Added monotonic `pa_stream_get_time()` tracking across stream recreations (`ever_ready`, `time_offset_us`, `snapshot_stream_time_locked`), `SYNTH_KIND_STREAM_SUCCESS` for `pa_stream_cork` / `pa_stream_update_timing_info` / attribute ops, cork reconciliation on stream ready, and mid-reconnect stream creation.
- Added `scripts/gen_pa_proxy.py` and `build/pa_reconnect/libpulse.so.0` (`220` PIC forwarding stubs + `-Wl,-z,nodelete` + host `libpulse.so.0` resolution) so KDE session restores (`/usr/lib/firefox/firefox --sm-client-id ...`) and desktop launches inherit reconnection transparently via `LD_LIBRARY_PATH` in `~/.config/environment.d/99-salsa-direct.conf` (configured by `src/lasapi_ctl.c` `enable-auto` / `disable-auto`).
- Removed legacy `LD_PRELOAD` `firefox`/`discord` wrappers in [`install-salsa.sh`](install-salsa.sh) to prevent double-wrapping.

### 3. Bluetooth Headphone Battery Percentage
- Fixed Bluetooth battery telemetry in `src/lasapi_bluez.c` so Bose QC Ultra 2 HP battery percentage reports accurately while WirePlumber runs in `video-only` / Zero-PipeWire mode.

### 4. Headphone Source Recovery & Update-Protection Hardening
- Updated `src/lasapi_bluez.c` to automatically recover and re-register endpoints when `bluetooth.service` (`bluetoothd`) restarts while `lasapi-audiodg.service` stays active.
- Updated `lasapi-autosync.sh` to wait cleanly for active package manager locks (`pacman`, `dpkg`, `yum`) in `--auto` mode and filter out `lib32` paths in `find_host_lib`.

### 5. Hardware & Idle XRUN Fix
- Fixed false idle XRUN inflation in `consume_session_frames` and resolved real ALSA write stalls in `src/lasapi_audiodg.c`.

### 6. Security Hardening, Private IPC & Zero-PipeWire Direct BlueZ
- Added native Bluetooth A2DP startup auto-connect, stuck-link self-healing, and manual BR/EDR reset in `src/lasapi_bluez.c` (replacing `reset-bose-audio.sh` and `bose-tray.py`).
- Hardened IPC/SHM paths with private `0700` runtime directories (`$XDG_RUNTIME_DIR/lasapi`), `0600` socket permissions, peer credential / cookie validation (`SO_PEERCRED`), and safe shared-memory bounds checks.
- Added per-target build freshness checks in `salsa_debug_core.py` and automatic `liblasapi.so` reload/re-exec protection in `salsa-gui.py` `_toggle_engine_mode`.

---

## v5.7-beta (`2026-11-13`)

### 7. KDE Panel Reset Fix
- Fixed leaked file descriptors in `src/lasapi_pw_broker.c` so `plasmashell` KDE panels remain stable without crashing or resetting.

### 8. 220ms Delay Verification
- Audited Bluetooth AVDTP Delay Reporting telemetry and verified `220 ms` is the hardware sink buffer reported by the Bose QC Ultra 2 HP headphones (separate from Salsa's `2.67 ms` engine period quantum).

### 9. Discord `>= 30 fps` Video Stream Fix
- Updated `src/lasapi_pw_broker.c` frame-rate negotiation so `kwin_wayland` / ScreenCast consumers negotiate `>= 30 fps` / `60 fps` instead of clamping at `25 fps`.

### 10. Discord Auto-Reconnect & Device-List Cache
- Created `src/salsa_pa_reconnect.c` (`libsalsa_pa_reconnect.so`) to suppress fatal `PA_CONTEXT_FAILED` / `PA_STREAM_FAILED` transitions once connected, serve cached sink/source/server lists during daemon restarts, reconnect active contexts/streams cleanly, and integrated it into [`install-salsa.sh`](install-salsa.sh).

---

## Pre-v5.7-beta History (`2026-11-11` – `2026-11-13`)

### 11. Pure Zero-PipeWire ScreenCast Video Broker & Crash Fixes (`2026-11-13`)
- Implemented pure-C `src/lasapi_pw_broker.c` serving `/run/user/$UID/pipewire-0` (blocking audio nodes on `pipewire-0` while relaying `DmaBuf`/`MemFd` ScreenCast frames), updated `salsa-debug.py` preview & telemetry, and updated the signal topology view in `salsa-gui.py`.

### 12. Steam / Proton Direct Audio, Custom Theme Editor & Saved Settings (`2026-11-12` – `2026-11-13`)
- Added `PROTON_USE_PIPEWIRE=0` / `SDL_AUDIODRIVER=pulse` / `ALSOFT_DRIVERS=pulse,alsa` environment routing in `src/lasapi_ctl.c`, added live theme editor `salsa_theme_editor.py`, persisted preset/dynamic labels in `salsa-gui.py`, and tuned Steam audio buffering.

### 13. Live Interaction Debugger, Tray & Icon Branding (`2026-11-11`)
- Created `salsa-debug.py`, `salsa_debug_core.py`, transparent [`assets/salsa-lasapi.png`](assets/salsa-lasapi.png), desktop entries, and tray indicator integration in `lasapi-tray.py`.

### 14. Salsa Studio GUI, Dynamic EQ, Unplugged Hiding, Capture & Cross-PC Installer (`2026-11-11`)
- Built the WaveRT SHM core, native PA v35 server (`src/lasapi_pa_server.c`), hollow ALSA/Pulse/JACK shims, hardware plug/unplug filtering, `salsa-gui.py`, `SALSA_OWNERS_MANUAL.md`, [`install-salsa.sh`](install-salsa.sh), and `lasapi-autosync.sh`.
