# SALSA (LASAPI)

SALSA provides zero-patching Linux audio routing, real-time DSP mixing, and PipeWire-free Wayland screen casting through the Linux Audio Session API.

## Language

**Audio Session**:
An active playback or capture stream registered by an application with the daemon over shared memory or PulseAudio protocol.
_Avoid_: Stream connection, audio client slot

**WaveRT Ring**:
The shared-memory circular PCM and MIDI buffer shared directly between an **Audio Session** and the real-time engine for zero-copy audio transfer.
_Avoid_: SHM queue, ring pipe

**Master Clock**:
The sample rate, quantum period, and sample depth driving hardware playback across all active playback **Audio Sessions**.
_Avoid_: Global rate, output clock

**Capture Clock**:
The independent sample rate applied to microphone capture when split clocking differs from the **Master Clock**.
_Avoid_: Input frequency, mic rate

**Dynamic EQ**:
The ten-band parametric equalizer, per-band envelope compressor/expander, crossfeed, and mastering limiter applied to desktop playback.
_Avoid_: DSP filter bank, post-FX

**Per-Device EQ Profile**:
A saved **Dynamic EQ** configuration keyed by hardware playback identifier and automatically loaded whenever that output device becomes active.
_Avoid_: Sink preset map, device EQ override

**Direct-on-Input Tap**:
A virtual PulseAudio capture source that mirrors an active native playback **Audio Session** so screen-sharing apps can record per-application audio.
_Avoid_: DOI loopback, virtual monitor sink

**Hollow Shim**:
A drop-in replacement shared library for ALSA, PulseAudio, or JACK that forwards client audio directly into a **WaveRT Ring**.
_Avoid_: Compatibility wrapper, preload hook

**Reconnect Proxy**:
A library interposer for host PulseAudio clients that transparently rebuilds lost connections and preserves real-time timers across daemon restarts.
_Avoid_: PA proxy, socket keepalive shim

**ScreenCast Broker**:
The video-only PipeWire protocol handler that forwards Wayland portal frame buffers without admitting PipeWire audio clients.
_Avoid_: Video relay, PW stub

**Hotplug Tracker**:
The subsystem that detects hardware or Bluetooth device arrivals and departures and switches active playback or capture endpoints while preserving user preferences.
_Avoid_: Device poller, plug watcher

**Analog Stage**:
The hardware output gain tier, DAC unity lock, discrete headphone amplifier drive, and physical USB/HID volume stage applied after digital mixing.
_Avoid_: ALSA mixer controls, HW analog ctx

**Zero-PipeWire Cutover**:
The reversible host routing state in which SALSA claims desktop audio and video screen-casting sockets while masking conflicting upstream audio daemons.
_Avoid_: Auto-enable mode, PipeWire zero hack

**Preview Seqlock**:
The lock-free shared-memory frame channel carrying live **ScreenCast Broker** video thumbnails from the daemon to diagnostic and studio viewers.
_Avoid_: Preview SHM buffer, frame dump slot

**Control IPC**:
The typed length-prefixed request and response channel over `audiodg.sock` (`LASAPI_CMD_CONTROL_V2`) used for device routing, EQ, analog, and Bluetooth management without mutating fixed C activation packets.
_Avoid_: Raw control struct, ad-hoc socket command

**Verified Host Loader**:
The runtime `dlopen` gate (`salsa_host_dl`) that checks `(st_dev, st_ino)` identity and trusted library directories so shims and optional codec/UI loaders never recurse into SALSA's own libraries.
_Avoid_: Blind dlopen, `dl_open_any`

**A2DP Codec Adapter**:
The runtime-loaded Bluetooth audio encoding and mSBC voice transcoding boundary isolating optional host codec pointers from the real-time Bluetooth transport state machine.
_Avoid_: Raw encoder pointer bag, `BtEncoders`
