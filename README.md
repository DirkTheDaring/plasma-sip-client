# Plasma SIP — Pure Rust SIP Softphone for KDE Plasma

[![Rust Quality Gate](https://img.shields.io/badge/Quality%20Gate-Passed-brightgreen)](TESTING.md)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/License-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Target: KDE Plasma 6](https://img.shields.io/badge/KDE-Plasma%206-blue)](https://kde.org/plasma-desktop/)
[![Platform: Linux](https://img.shields.io/badge/Platform-Linux%20Wayland%20%2F%20X11-purple)](#)

Plasma SIP is a modern, production-grade, pure Rust SIP softphone tailored specifically for KDE Plasma 6 on Linux (Wayland and X11). It features native integration with PipeWire audio, first-class AVM FRITZ!Box PBX support, and generic SIP provider flexibility.

---

## Key Features

- **100% Pure Rust:** Zero legacy C/C++ SIP libraries (no PJSIP, Sofia-SIP, or Linphone). Memory safe, highly concurrent, and leak-free.
- **AVM FRITZ!Box Ready:** Native support for local FRITZ!Box PBX registration, internal extension dialing (`**620`), and local IP phone authentication.
- **Provider Agnostic:** Clean provider profile abstraction seamlessly connects with Asterisk, FreePBX, and standard VoIP services.
- **PipeWire Integration:** Low-latency audio processing using pure Rust ITU-T G.711 PCMA/PCMU and lock-free audio ringbuffers.
- **Background Daemon (`plasma-sipd`):** Uninterrupted call reception and registration maintenance independently of the graphical UI.
- **KDE Plasma 6 UI (`plasma-sip`):** Beautiful Kirigami 6 user interface supporting light/dark themes, responsive sizing, dial pad, call history, and desktop notifications.
- **KWallet / Secret Service Security:** Passwords are never stored in plaintext on disk.
- **Strict Non-Root Policy:** Security-by-design architecture that strictly enforces user-session execution.

---

## Documentation Directory

| Document | Purpose |
|---|---|
| [User Guide](docs/USER_GUIDE.md) | Step-by-step setup wizard for FRITZ!Box, call management, dial pad, and D-Bus CLI commands |
| [Architecture Documentation](ARCHITECTURE.md) | Comprehensive 11-crate layered architectural specification |
| [Troubleshooting Guide](docs/TROUBLESHOOTING.md) | Diagnostic solutions for SIP registration, audio devices, and KWallet |
| [Security & Privacy](docs/SECURITY.md) | Threat model, zero-plaintext storage, parser hardening, and sandbox policies |
| [Hardware Test Results](docs/HARDWARE_TEST_RESULTS.md) | AVM FRITZ!Box hardware interoperability matrix and physical test plan |
| [Deployment Report](docs/DEPLOYMENT_REPORT.md) | Final software verification report and quality gate audit |
| [System & User Environment](docs/SYSTEM_ENVIRONMENT.md) | Host OS, DNS resolution, network sockets, D-Bus, PipeWire, and desktop session expectations |
| [Deployment Guide](DEPLOYMENT.md) | Installation scripts, systemd configuration, and RPM packaging |
| [Testing Strategy](TESTING.md) | Test principles, quality gates, and automated test execution |
| [Changelog](CHANGELOG.md) | Release notes and version history |

---

## Quick Start

### 1. Build and Run
```bash
# Clone the repository and build release binaries
cargo build --release --workspace

# Install into your user environment (~/.local/bin)
./packaging/install.sh

# Launch the softphone
plasma-sip
```

### 2. Run Test Suite
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

---

## License

Plasma SIP is licensed under the **GNU General Public License v3.0 or later** (GPL-3.0-or-later). See [LICENSE](LICENSE) for details.
