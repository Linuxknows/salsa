# Support & Troubleshooting Guide

Need help installing, configuring, or diagnosing **Salsa (`lwasapi`)**? Use the built-in diagnostic tools below first so you get instant answers on your machine.

---

## 1. Built-In Diagnostics & Documentation

- **Interactive Owner's Manual & Signal Topology**: Run `salsa --topology` (or `salsa-graph`) to inspect your live signal path and browse all 15 built-in Owner's Manual chapters.
- **Installation & Service Health Check**:
  ```bash
  ~/.local/bin/lwasapi-ctl status
  ./build/salsa-installer --check
  ```
- **Live Architecture & Routing Debugger**: Run `salsa-debug` (GUI) or `salsa-debug-cli` (terminal report) to inspect per-process audio routing, `SCHED_FIFO` real-time jitter, BlueZ A2DP codecs, and Wayland `ScreenCast` frames.
- **Architecture Reference**: Read [README.md](README.md), [SALSA Architecture Breakdown.md](SALSA%20Architecture%20Breakdown.md), and [PATCH_NOTES.md](PATCH_NOTES.md).

---

## 2. Getting Help & Filing Issues

- **Bug Reports**: Open a [Bug Report Issue](https://github.com/Linuxknows/salsa/issues/new?template=bug_report.yml) and attach the output of `lwasapi-ctl --version` and `./build/salsa-installer --check`.
- **Feature Requests & Hardware Compatibility**: Open a [Feature Request Issue](https://github.com/Linuxknows/salsa/issues/new?template=feature_request.yml).
- **Security Vulnerabilities**: Do **not** open a public issue — please use [Private Vulnerability Reporting](SECURITY.md).
