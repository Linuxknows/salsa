# Support & Troubleshooting Guide

Need help diagnosing **Salsa (`lasapi`)**? Use Salsa's built-in **Live Architecture & Interaction Debugger (`salsa-debug`)** first so you can inspect live routing, probes, and telemetry right on your machine.

---

## 1. Diagnosing Issues with `salsa-debug` (GUI & CLI)

Salsa ships with a standalone zero-overhead debugger (**`salsa-debug`**, also accessible from the system tray Quick Mixer **`Debug`** button):

- **GUI Tabs & Interactive Probes (`salsa-debug`)**:
  - **Tab 1 — Live Event Stream (`--tab 0`)**: Real-time stream of `LASAPI`, `PA`, `BLUEZ`, `MIDI`, `CAPTURE`, `ALSA`, `IPC`, `RT`, and `PW` events over `debug.sock`.
  - **Tab 2 — App & Process Routing Audit (`--tab 1`)**: Scans `/proc` to show how every running game, browser, or DAW connects to Salsa (`WaveRT SHM`, `PA v35`, `ALSA plugin`, or `lasapi-run` shim).
  - **Tab 3 — Streams, Rings & RT Timing (`--tab 2`)**: Live `SCHED_FIFO` wakeup jitter, WaveRT shared-memory ring fill levels, and XRUN counters.
  - **Tab 4 — Boundary, Configs & Probes (`--tab 3`)**: One-click **`IPC + Debug Ping RTT`**, **`Play 440Hz LASAPI Probe Tone`**, **`Send MIDI Note Ping (C4)`**, and **`Run All Socket & PA/PW Probes`**.
  - **Tab 5 — ScreenCast Broker & Live Feed (`--tab 4`)**: Live Wayland `pipewire-0` `DmaBuf`/`MemFd` video preview and **`Run PW Self-Test`**.
  - **`Export Bundle` Button**: Saves a complete timestamped `.zip` archive (`report.txt`, JSON snapshots, routing audit, configs, and logs) ready to attach directly to a GitHub Bug Report.

- **Headless / Terminal Debugger Commands**:
  ```bash
  salsa-debug --export          # Write full diagnostic .zip bundle for GitHub bug reports
  salsa-debug --text            # Print detailed plain-text debug report to stdout
  salsa-debug --probe           # Run IPC, dlopen, PA v35, and PW socket health probes
  salsa-debug --audit           # Run full host & 23-artifact build freshness audit
  salsa-debug --stream --telem  # Stream live events + RT telemetry in the terminal
  ./build/salsa-installer --check
  ```

- **Interactive Owner's Manual & Architecture Docs**:
  - Run `salsa --topology` (or `salsa-graph`) to browse all 13 built-in Owner's Manual chapters alongside your live signal graph.
  - Read [README.md](README.md), [SALSA Architecture Breakdown.md](SALSA%20Architecture%20Breakdown.md), and [PATCH_NOTES.md](PATCH_NOTES.md).

---

## 2. Getting Help & Filing Issues

- **Bug Reports**: Run **`Export Bundle`** in `salsa-debug` (or `salsa-debug --export` / `salsa-debug --text`), then open a [Bug Report Issue](https://github.com/Linuxknows/salsa/issues/new?template=bug_report.yml) and attach the report or bundle.
- **Feature Requests & Hardware Compatibility**: Open a [Feature Request Issue](https://github.com/Linuxknows/salsa/issues/new?template=feature_request.yml).
- **Security Vulnerabilities**: Do **not** open a public issue — please use [Private Vulnerability Reporting](SECURITY.md).
