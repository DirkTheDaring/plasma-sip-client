# Development Guide — Plasma SIP

## Prerequisites
- **Rust Toolchain:** 1.80+ (stable). Current system toolchain: 1.99.0.
- **KDE Plasma 6 / Qt 6:** On Linux with Wayland or X11.
- **Audio:** PipeWire or ALSA (`libasound2` / `alsa-lib-devel`).
- **Database:** SQLite development libraries (`sqlite-devel`).
- **D-Bus:** User session D-Bus.

## Workspace Layout
```text
plasma-sip/
├── crates/
│   ├── sip-domain/      # Pure domain types & value objects
│   ├── sip-protocol/    # RFC 3261 messages, transactions, dialogs, SDP
│   ├── sip-transport/   # UDP socket abstractions and transmission
│   ├── sip-auth/        # HTTP Digest Authentication MD5
│   ├── sip-media/       # RTP (RFC 3550), Jitter buffer, RFC 4733 DTMF
│   ├── sip-audio/       # G.711 PCMA/PCMU codecs, CPAL audio streams
│   ├── sip-engine/      # High-level client engine
│   ├── sip-providers/   # Fritz!Box & Generic SIP provider profiles
│   ├── sip-storage/     # SQLite database and Secret Service keyring
│   ├── sip-service/     # Core application service and D-Bus interfaces
│   └── sip-platform/    # Systemd & KDE notifications integration
├── apps/
│   ├── plasma-sipd/     # Telephony background daemon
│   └── plasma-sip/      # KDE Kirigami desktop application
├── qml/                 # Kirigami QML UI components
├── dbus/                # D-Bus introspection XML specs
├── tests/               # Integration & interoperability tests
└── docs/                # Architecture, ADRs, test logs
```

## Building
```bash
cargo build --workspace
cargo test --workspace
```
