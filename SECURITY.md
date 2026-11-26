# Security Policy

## Supported Versions

Security fixes are maintained on `main` and shipped in the latest stable release series:

| Version | Supported |
| :--- | :--- |
| `V1.26.x` (`main` / Latest Release) | :white_check_mark: Supported |
| `< V1.26.0` | :x: Unsupported — please upgrade to the latest release |

---

## Reporting a Vulnerability

**Please do not open a public GitHub Issue or Pull Request for suspected security vulnerabilities.**

Instead, report vulnerabilities privately through **[GitHub Private Vulnerability Reporting](https://github.com/Linuxknows/salsa/security/advisories/new)**.

When submitting a report, please include:

1. Exact Salsa version (`lasapi-ctl --version` or commit SHA) and Linux distribution / kernel version.
2. Affected component (e.g., `pa_server.rs`, `pw_broker.rs`, `bluez_wire.rs`, `salsa_installer.rs`, `liblasapi.so`).
3. Clear reproduction steps or a minimal proof-of-concept (such as a `fuzz_wire` input or local socket payload).
4. Impact assessment (local privilege escalation, cross-UID socket access, memory corruption, or denial of service in `lasapi-audiodg`).

### Response & Coordinated Disclosure

- **Initial acknowledgment**: Targeted within **72 hours**.
- **Triage & fix validation**: Confirmed issues are reproduced against `fuzz_wire` and `salsa-regression`, patched with regression tests, and released in a new tagged version before public advisory disclosure.

---

## Salsa Security Architecture & Scope

Salsa operates purely as an unprivileged per-user audio/video daemon (`uid == getuid()`, with optional `RLIMIT_RTPRIO` / RealtimeKit `SCHED_FIFO` scheduling) and enforces strict local isolation boundaries:

- **Socket & Runtime Directory Permissions**: Runtime directories (`$XDG_RUNTIME_DIR/lasapi`, `$XDG_RUNTIME_DIR/pulse`) are enforced `0700` and UNIX domain sockets (`native`, `pipewire-0`, single-instance UI sockets) are enforced `0600` with kernel `SO_PEERCRED` same-UID verification.
- **Authentication & Loopback Isolation**: PulseAudio protocol authentication uses constant-time `256`-byte cookie verification (`ct_eq_256`), and container failover binds strictly to `127.0.0.1` loopback.
- **Filesystem & Symlink Hardening**: Configuration and volume state files (`~/.config/lasapi/`, `0700` dirs / `0600` files) reject symlinks (`O_NOFOLLOW` / `symlink_metadata`), enforce bounded read sizes, and write atomically.
- **Wire Protocol & Memory Hardening**: Untrusted IPC frames (`pa_wire.rs`, `pw_wire.rs`, `bluez_wire.rs`, JSON depth/surrogate parsers) enforce strict length, descriptor-count (`SCM_RIGHTS` FD quota), and recursion bounds, verified continuously by `fuzz_wire`. Native C/C++ and Rust artifacts compile with `-D_FORTIFY_SOURCE=3`, `-fstack-protector-strong`, full RELRO (`-Wl,-z,relro,-z,now`), and non-executable stacks (`-Wl,-z,noexecstack`).
