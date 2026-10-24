# Dependency Selection Matrix — Plasma SIP

| Domain | Candidate Crate(s) | Selected | License | Evaluation & Justification |
|---|---|---|---|---|
| **SIP Parsing & AST** | `rsip`, `sip-core` | `rsip` (0.4) | MIT / Apache-2.0 | RFC 3261 compliant message, header, and URI parsing in pure Rust. Highly typed. |
| **SIP Transactions/Dialogs** | None mature | In-tree `sip-protocol` & `sip-engine` | GPL-3.0-or-later | Existing pure Rust crates lack RFC 3261 Section 17 transaction / Section 12 dialog engines. Built on Tokio. |
| **Async Runtime** | `tokio` (1.x) | `tokio` | MIT | Industry standard async runtime; full timer, UDP socket, task spawning support. |
| **RTP Media** | `rtp-rs`, custom parser | `rtp-rs` (0.6) + in-tree `sip-media` | MIT | RFC 3550 RTP parsing & serialization. Pure Rust. |
| **G.711 Audio Codecs** | `nanoaudio-g711`, custom | In-tree `sip-audio` ITU-T tables | MIT / GPL-3.0 | ITU-T G.711 A-law and μ-law with deterministic tables and zero allocation. Verified with standard vectors. |
| **Audio I/O** | `cpal` (0.18) | `cpal` | Apache-2.0 | Pure Rust cross-platform audio. Seamless PipeWire/ALSA support without low-level C bindings. |
| **Audio SPSC Queue** | `rtrb` | `rtrb` (0.3) | MIT / Apache-2.0 | Lock-free, allocation-free, wait-free ringbuffer designed for real-time audio threads. |
| **D-Bus IPC** | `zbus` (5.x) | `zbus` | MIT | Pure Rust asynchronous D-Bus implementation. Native integration with Tokio. |
| **Database Storage** | `rusqlite` (0.40) | `rusqlite` | MIT | Embedded SQLite for account metadata and call history. Clean schema migrations. |
| **Secret Storage** | `keyring` (4.x) | `keyring` | MIT / Apache-2.0 | Native Freedesktop Secret Service / KWallet integration. No plaintext passwords on disk. |
| **Desktop UI** | Qt 6 Kirigami QML | Qt 6 / Kirigami 6 via D-Bus client | LGPL-3.0 / GPL-3.0 | Native KDE Plasma look and feel, Wayland native, zero C++ business logic. |

---

## System & User Environment Dependencies

For a full breakdown of host environment requirements, network ports, and DNS resolution, see [System & User Environment Specification](SYSTEM_ENVIRONMENT.md).

| System Resource | Provider / Standard | Expectation | Fallback / Behavior |
|---|---|---|---|
| **DNS Nameservers** | `/etc/resolv.conf` | Outbound UDP `53` to nameservers for RFC 3263 SRV queries | `127.0.0.53:53` -> `1.1.1.1:53` -> libc `getaddrinfo` A/AAAA |
| **SIP Signaling Port** | Linux Network Stack | UDP `5060` (`PLASMA_SIP_BIND_ADDR`) | Unprivileged ephemeral port `0.0.0.0:0` |
| **RTP Media Ports** | Linux Network Stack | Dynamic unprivileged UDP (`>1024`, default `16384`) | Dynamic port assignment by kernel + RFC 4961 symmetric latching |
| **D-Bus Session Bus** | `dbus-broker` / `dbus-daemon` | Active session bus at `$DBUS_SESSION_BUS_ADDRESS` | Mandatory; daemon exits if session bus unavailable |
| **systemd User Session** | `systemd --user` | User service unit execution (`plasma-sipd.service`) | Standard login session management |
| **Audio Server** | PipeWire / WirePlumber / ALSA | Active sound stream endpoints for capture and playback | Real-time adaptive resampling to 8000 Hz |
| **Secret Service** | FreeDesktop Secret Service / KWallet | `org.freedesktop.secrets` interface on session bus | User password prompt on locked wallet |
| **Desktop Session** | KDE Plasma 6 (Wayland / X11) | Kirigami 6, StatusNotifierItem (SNI), desktop notifications | Headless mode operates independently via CLI |

