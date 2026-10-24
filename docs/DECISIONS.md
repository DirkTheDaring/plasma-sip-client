# Architecture Decision Records (ADRs) - Plasma SIP

## ADR-0001: Pure Rust SIP Protocol Stack Architecture
- **Status:** Accepted
- **Context:** The application requires full RFC 3261 compliance (REGISTER, INVITE, ACK, CANCEL, BYE, OPTIONS, responses, transaction matching, dialog management, authentication, and timers). Third-party C/C++ libraries (PJSIP, Sofia-SIP, Linphone) are strictly forbidden by project constraints. Existing pure Rust SIP crates either offer AST parsing without transaction/dialog engines or are incomplete/unmaintained.
- **Decision:** Use `rsip` for RFC 3261 message parsing and serialization, augmented with a purpose-built asynchronous SIP transaction engine (RFC 3261 Section 17) and dialog manager (Section 12) built on Tokio. Retransmission timers (T1, T2, T4, A-K) and transaction states are modeled explicitly as deterministic state machines with testable clocks.
- **Consequences:** Eliminates C FFI safety hazards and memory corruption risks. Guarantees 100% pure Rust memory safety and seamless integration with Tokio async runtimes. Requires rigorous unit and property testing for transactions, timers, and dialog matching.

---

## ADR-0002: Modular Workspace Architecture
- **Status:** Accepted
- **Context:** The application spans SIP signaling, network transport, audio hardware, media streaming, local storage, D-Bus IPC, and KDE desktop integration. A monolithic crate would blur layer boundaries and impede testability.
- **Decision:** Adopt a Cargo workspace with strictly unidirectional dependencies:
  - `crates/sip-domain`: Pure domain models, value types, state enums, errors. No network or UI dependencies.
  - `crates/sip-protocol`: SIP message structures, transactions, dialogs, timers, SDP (RFC 4566 / RFC 3264).
  - `crates/sip-transport`: UDP (and future TCP/TLS) transport layer, message framing, MTU handling.
  - `crates/sip-auth`: Digest authentication (MD5, RFC 2617 / RFC 7616), nonce caching, authorization header building.
  - `crates/sip-media`: RTP (RFC 3550), jitter buffer, packet loss handling, RFC 4733 DTMF, media session negotiation.
  - `crates/sip-audio`: G.711 A-law and μ-law codecs, CPAL audio capture/playback, lock-free ringbuffers.
  - `crates/sip-engine`: High-level SIP client engine coordinating registration, transactions, dialogs, and media.
  - `crates/sip-providers`: Provider profiles (FRITZ!Box and Generic SIP), dial-string normalization, registrar profiles.
  - `crates/sip-storage`: SQLite persistence for configuration and call logs; Secret Service integration for credentials.
  - `crates/sip-service`: Application service logic coordinating accounts, calls, audio, and events.
  - `crates/sip-platform`: Linux / KDE integration (freedesktop notifications, systemd user service support).
  - `apps/plasma-sipd`: Standalone background user daemon exposing the D-Bus interface.
  - `apps/plasma-sip`: KDE Plasma desktop graphical application.
- **Consequences:** High cohesion, decoupled layers, fast incremental builds, and isolated unit/integration testing without mocking dependencies.

---

## ADR-0003: Real-Time Audio Engine and Linux PipeWire/ALSA Integration
- **Status:** Accepted
- **Context:** Audio processing demands low latency, jitter resilience, zero memory allocations in real-time callbacks, and seamless operation on modern KDE Plasma distributions running PipeWire.
- **Decision:**
  - Codecs: ITU-T G.711 A-law (PCMA) and μ-law (PCMU) implemented in pure Rust with constant-time lookup tables and verified against ITU-T test vectors.
  - Device I/O: CPAL crate using ALSA/PipeWire audio backends.
  - Thread Separation: Audio device callbacks communicate with Tokio async RTP networking tasks using lock-free single-producer single-consumer (SPSC) ringbuffers (`rtrb`). No allocations, system calls, or async mutexes in the audio callback.
  - Jitter Buffering: Fixed/adaptive jitter buffer designed for 20ms packet intervals (160 samples per frame at 8 kHz), supporting sequence rollover, out-of-order reassembly, and packet loss concealment (PLC) via sample repetition / comfort interpolation.
- **Consequences:** Ensures zero audio stutter or dropouts caused by thread contention while maintaining 100% pure Rust safety.

---

## ADR-0004: Daemon-Client Separation via D-Bus IPC
- **Status:** Accepted
- **Context:** A desktop softphone must remain reachable for incoming calls even when the main graphical window is closed. KDE Plasma user sessions provide a standardized session D-Bus bus.
- **Decision:** Split the application into:
  - `plasma-sipd`: A headless background daemon running under systemd user session (`plasma-sipd.service`) that maintains SIP registration, monitors network changes, handles incoming calls, and emits desktop notifications.
  - `plasma-sip`: A lightweight desktop frontend (Kirigami / Qt 6) that discovers and controls the daemon over D-Bus (`org.kde.PlasmaSip`).
  - D-Bus library: `zbus` (pure Rust, async Tokio native).
- **Consequences:** GUI can be opened, closed, or restarted at will without dropping active calls or breaking SIP registration. The D-Bus interface allows scripting and KDE Plasma system tray or plasmoid integration.

---

## ADR-0005: Credential Security and Secret Storage
- **Status:** Accepted
- **Context:** Plaintext storage of SIP passwords is prohibited. SIP passwords must be protected using standard Linux desktop security facilities.
- **Decision:**
  - Passwords are stored in the user's Secret Service / KWallet via the `keyring` crate.
  - The SQLite database (`plasma-sip.db`) stores account metadata, SIP registrar URLs, identities, preferences, and call history, but strictly excludes passwords.
  - In-memory credentials are zeroed on drop where feasible (`zeroize`).
  - Logs redact authentication headers, passwords, and sensitive credentials by default.
- **Consequences:** Hardens application against local data extraction while respecting native KDE Plasma KWallet workflows.

---

## ADR-0006: Provider Profile Abstraction (FRITZ!Box and Generic SIP)
- **Status:** Accepted
- **Context:** The SIP protocol engine must not include hardcoded vendor conditionals. At the same time, FRITZ!Box PBX features (internal extensions `**620`, registrar defaults `fritz.box`, digest realm requirements) require specialized normalization and validation.
- **Decision:** Define a `SipProviderProfile` trait in `sip-providers`. Implement `FritzBoxProvider` and `GenericSipProvider`. The provider profile validates configurations, generates default registrar addresses, normalizes dial strings (handling `**` extensions vs international E.164 formats), and provides guided setup metadata.
- **Consequences:** Clean separation of concerns; FRITZ!Box works out of the box with zero generic SIP engine pollution.

---

## ADR-0007: CXX-Qt & KDE Kirigami Architecture for Desktop Application
- **Status:** Accepted
- **Context:** The user-facing softphone must deliver a native KDE Plasma 6 experience conforming to KDE Human Interface Guidelines (HIG) with declarative QML and Kirigami. At the same time, architectural requirements mandate that all business logic remain 100% pure Rust, with zero business logic in C++.
- **Decision:**
  - Use `cxx-qt 0.10.0` with `cxx-qt-lib` and `cxx-qt-build` for pure Rust Qt 6 bindings.
  - Implement `SipUiBridge` as a Rust QObject exposing reactive properties (`is_registered`, `active_call_state`, `active_remote_party`, `call_duration_seconds`, `incoming_call_id`, `incoming_caller`, `call_history_json`, `audio_devices_json`) and invokable methods (`dial`, `answer_call`, `reject_call`, `hangup_call`, `toggle_mute`, `send_dtmf`, `save_account`, `delete_account`).
  - Embed all 7 Kirigami QML files into the binary via the Qt Resource system using `CxxQtBuilder::new_qml_module("org.kde.plasma_sip")`.
  - The UI bridge spawns an isolated Tokio runtime on a dedicated thread to communicate with `plasma-sipd` over Session D-Bus (`org.kde.PlasmaSip`), dispatching signal updates to the Qt event loop via `cxx_qt::CxxQtThread::queue`.
  - Implement dual-mode CLI and GUI execution: subcommand invocations (`ping`, `call`, `hangup`, `devices`, `history`) execute CLI commands directly, while default execution launches `QGuiApplication` and `QQmlApplicationEngine`.
- **Consequences:** Zero business logic in C++, full Rust memory safety in UI bridging, native KDE Plasma 6 look and feel (including dynamic light/dark theming and Wayland/X11 rendering), and complete independence from daemon lifecycle.

---

## ADR-0008: Telephony & Desktop Security Architecture (STRIDE Mitigations)
- **Status:** Accepted
- **Context:** Softphones operate in hostile network and IPC environments. External applications can trigger URL handlers (`sip:`, `tel:`), internet scanners broadcast unauthorized INVITE requests to UDP port 5060 ("phantom calls"), adversaries can attempt CRLF header injection, and rogue RTP sources can attempt to inject unauthenticated audio into open RTP receiver sockets.
- **Decision:**
  - **Zero-Click Toll Fraud Prevention (SEC-01):** Desktop URL handlers (`tel:`, `sip:`) and single-instance IPC invocations must NEVER initiate outbound calls automatically. Instead, they trigger `TrayAction::PrefillAndShow`, pre-populating the dialed digits in the Kirigami dialpad and bringing the window to focus, strictly requiring manual confirmation by the user before placing the call.
  - **SIP CRLF & Header Injection Neutralization (SEC-02):** Enforce strict input validation in `SipProviderProfile::normalize_dial_target` rejecting control characters (`\r`, `\n`, whitespace, and ASCII control characters). Provide defense-in-depth by stripping `\r` and `\n` in `SipRequest::serialize` and `SipResponse::serialize` across Request-URIs, status reason phrases, and all headers.
  - **Phantom Call & Scanner Defense (SEC-03):** Reject incoming SIP `INVITE` requests arriving from IP addresses not associated with any registered PBX or registrar. Source IPs matching registered provider endpoints or local loopback are accepted; unsolicited public internet scans on UDP 5060 are rejected with `403 Forbidden` and discarded before session allocation.
  - **RTP Peer Address Filtering & Latching (SEC-04):** The RTP media session receiver loop validates incoming UDP packet source addresses against negotiated SDP remote peer addresses (RFC 3550 / RFC 4961), dropping unauthorized media packets from rogue external hosts.
  - **In-Memory Credential Scrubbing (SEC-06):** In-memory SIP authentication credentials in active call sessions are wrapped in `zeroize::Zeroizing<String>` ensuring that heap allocations are securely zeroed upon session termination or drop.
- **Consequences:** Guarantees protection against automated toll fraud, phantom call spam, SIP protocol manipulation, audio injection attacks, and memory dumps while preserving seamless PBX interoperability and testability.


