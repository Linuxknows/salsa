# Third-Party Libraries, Linking & LGPL-3.0 Compliance

Salsa (LWASAPI) is Copyright (C) 2026 Linuxknows and is licensed under the
**GNU Lesser General Public License, version 3.0 only (`LGPL-3.0-only`)**.
The complete license terms consist of [COPYING.LESSER](COPYING.LESSER) (LGPL-3.0
additional permissions) together with [COPYING](COPYING) (GNU GPL-3.0 base terms).

---

## 1. Rust Workspace (`rust/Cargo.lock`)

- **Zero external `crates.io` dependencies.** All 10 crates in `rust/`
  (`alsa_plugin_lwasapi`, `lwasapi_client`, `salsa_net`, `salsa_pa_reconnect`,
  `salsa_pa_reconnect_proxy`, `salsa_tools`, `salsa_ui`, `shim_alsa`, `shim_jack`,
  `shim_pulse`) are authored within this repository and declare
  `license = "LGPL-3.0-only"`.

---

## 2. Dynamically Linked System Frameworks & Libraries (LGPL-3.0 Section 4)

Salsa links dynamically against standard host shared libraries and provides full
Corresponding Source for all static archives (`libsalsa_wire.a`, `libsalsa_ui.a`)
and Qt6 frontends so recipients may modify, relink, or replace any Salsa library
component or Qt6 library cleanly (`Makefile`, `rust/`, `qt6/`):

| Library / Package | Upstream License | How Used by Salsa |
|---|---|---|
| **Qt 6 (`qt6-base`)** | `LGPL-3.0-only` / `GPL-3.0-only` | Dynamically linked shared GUI framework (`Qt6Core`, `Qt6Gui`, `Qt6Widgets`, `Qt6DBus`). Used strictly under `LGPL-3.0-only`. |
| **ALSA (`alsa-lib`)** | `LGPL-2.1-or-later` | Dynamically linked host PCM/MIDI/mixer interface (`libasound.so.2`). |
| **PulseAudio Client (`libpulse`)** | `LGPL-2.1-or-later` | Host compatibility proxy target in `salsa_pa_reconnect`. |
| **JACK2 Client (`jack2`)** | `GPL-2.0-or-later` / `LGPL-2.1-or-later` | ABI-compatible shim target (`shim_jack`). |

---

## 3. Optional Runtime `dlopen` Host Libraries (Never Bundled or Vendored)

Salsa **does not vendor, redistribute, or statically link** any third-party codec,
DSP, D-Bus, or graphics binaries or headers. Optional host libraries are loaded
solely at runtime via `dlopen()` if already installed on your system, and Salsa
degrades gracefully with safe fallbacks if any library is absent:

| Shared Object | Upstream License | Notes |
|---|---|---|
| **`libfdk-aac.so.2`** (`fdk-aac` / `libfdk-aac`) | `LicenseRef-Fraunhofer` (Fraunhofer FDK AAC Codec Library for Android) | **Not bundled or distributed by Salsa.** Loaded optionally via `dlopen("libfdk-aac.so.2")` in `rust/salsa_net/src/bluez_ffi.rs` only when present on the user's host for Bluetooth A2DP AAC encoding/decoding; falls back to SBC/other codecs if absent. See your distribution's `/usr/share/licenses/fdk-aac/NOTICE` or `/usr/share/licenses/libfdk-aac/NOTICE` for Fraunhofer copyright and patent terms. |
| **`libldacBT_enc.so.2`, `libldacBT_abr.so.2`** (`libldac`) | `Apache-2.0` | Optional runtime `dlopen` for Bluetooth LDAC A2DP encoding and ABR rate adaptation (Apache-2.0 is compatible with GPLv3/LGPLv3). |
| **`libfreeaptx.so.0`** (`libfreeaptx`) | `LGPL-2.1-or-later` | Optional runtime `dlopen` for Bluetooth aptX / aptX HD encoding and decoding. |
| **`libsbc.so.1`** (`sbc`) | `LGPL-2.1-or-later` | Runtime `dlopen` for Bluetooth A2DP SBC & HFP mSBC codec frames. |
| **`librnnoise.so.0`** (`rnnoise`) | `BSD-3-Clause` | Optional runtime `dlopen` for neural microphone noise suppression in the Capture Mic APO. |
| **`libdbus-1.so.3`** (`dbus`) | `AFL-2.1 OR GPL-2.0-or-later` | Runtime `dlopen` for BlueZ D-Bus A2DP MediaEndpoint / MediaTransport coordination. |
| **`libglib-2.0.so.0`, `libgobject-2.0.so.0`, `libgio-2.0.so.0`** (`glib2`) | `LGPL-2.1-or-later` | Runtime `dlopen` for StatusNotifierItem tray integration and xdg-desktop-portal ScreenCast D-Bus coordination. |
| **`libgbm.so.1`, `libEGL.so.1`, `libGLESv2.so.2`** (`mesa`, `libglvnd`) | `MIT` / `BSD-3-Clause` | Runtime `dlopen` for zero-copy DMA-BUF Wayland ScreenCast preview conversion. |
| **`libLayerShellQtInterface.so.6`** (`layer-shell-qt`) | `LGPL-3.0-only` / `LGPL-2.1-only` | Optional runtime `dlopen` in `qt6/lwasapi_tray.cpp` for native Wayland `zwlr_layer_shell_v1` tray popups, menus, and click-through OSD overlays (zero panel taskbar icon); falls back to standard Qt/EWMH windows if absent. |
| **`libKF6WindowSystem.so.6`** (`kwindowsystem`) | `LGPL-2.1-only OR LGPL-3.0-only` | Optional runtime `dlopen` in `qt6/salsa_common.cpp` to mint Wayland `XDG_ACTIVATION_TOKEN` focus tokens when raising single-instance windows. |
| **`libX11.so.6`, `libXrandr.so.2`** (`libx11`, `libxrandr`) | `MIT` / `X11` | Optional runtime `dlopen` used exclusively on X11/XCB fallback sessions (`window_uses_x11()`) for EWMH popup hints and X11 monitor geometry; never opened during native Wayland execution. |
| **`libz.so.1`** (`zlib`) | `Zlib` | Runtime `dlopen` for PNG asset encoding/decoding in installer & tray icons. |

---

## 4. Library Relinking & Corresponding Source (`LGPL-3.0` §4)

Every library and binary in Salsa — including `build/liblwasapi.so`,
`build/libasound_module_pcm_lwasapi.so`, `build/libsalsa_pa_reconnect.so`,
`build/shims/*.so`, `build/libsalsa_wire.a`, `build/libsalsa_ui.a`, and all Qt6
frontends — builds cleanly offline from the complete source tree in this
repository:

```bash
make -j"$(nproc)" all
```

Shared components (`liblwasapi.so`, `libasound_module_pcm_lwasapi.so`,
`libsalsa_pa_reconnect.so`, `build/shims/*.so`) are standard ELF shared objects
that users can modify, recompile, and replace directly in-place.
