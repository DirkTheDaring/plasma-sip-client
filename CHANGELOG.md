# Changelog — Plasma SIP

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Contacts & FRITZ!Box Phonebook Synchronization:**
  - Pure-Rust TR-064 protocol client (`X_AVM-DE_OnTel:1`) with HTTP Digest MD5 authentication.
  - Automatic phonebook synchronization from AVM FRITZ!Boxes and local PBXs into SQLite schema migration 3.
  - T9 predictive and vanity number search algorithm with normalized digits index.
  - Automatic caller name resolution for incoming calls and call history entries.
  - Dedicated `ContactsView.qml` with instant search, initials avatar bubbles, phone number type chips (`Home`, `Mobile`, `Work`, `Internal`), and one-tap dialing.
  - `sync-phonebook` and `contacts [query]` CLI subcommands in `plasma-sip`.
- **Convergent 4-Tab Interface & Visual Redesign:**
  - Modern bottom navigation bar with 4 convergent tabs: Keypad, History, Contacts, Settings (`Ctrl+1..4`).
  - Tactile squircle keypad buttons with subtle Breeze Dark borders and ambient hover glow.
  - Real-time T9 contact match suggestion strip above the keypad.
  - Unified `SettingsView.qml` consolidating SIP Accounts, FRITZ!Box Phonebook, and Audio settings.

## [0.1.0] - 2026-10-19

### Added
- **Core Architecture & Workspace:**
  - 11 modular pure-Rust crates (`sip-domain`, `sip-protocol`, `sip-transport`, `sip-auth`, `sip-media`, `sip-audio`, `sip-engine`, `sip-providers`, `sip-storage`, `sip-service`, `sip-platform`).
  - Architecture Decision Records ADR 0001 through ADR 0006 (`docs/DECISIONS.md`).
- **SIP Signaling Engine:**
  - RFC 3261 parser and serializer supporting REGISTER, INVITE, ACK, CANCEL, BYE.
  - RFC 2617 Digest MD5 challenge-response authentication.
  - Client transaction state machine with standard RFC 3261 timers.
  - Inbound and Outgoing SIP Dialog management with local/remote tag tracking.
- **Media & Audio Subsystem:**
  - RFC 3550 RTP packetizer with sequence and timestamp rollover support.
  - Pure Rust ITU-T G.711 A-law (PCMA) and μ-law (PCMU) codecs with ITU test vector verification.
  - Adaptive jitter buffer with loss concealment and packet reordering.
  - RFC 4733 in-band DTMF telephony-event generation and reception.
  - Synthetic bidirectional loopback testing.
- **Provider Profiles:**
  - AVM FRITZ!Box provider profile with internal extension normalization (`**620`), local gateway defaults, and PBX realm handling.
  - Generic SIP provider profile for Asterisk, FreePBX, and standard VoIP services.
- **Background Daemon (`plasma-sipd`):**
  - Headless user-session daemon running under systemd.
  - Freedesktop D-Bus API (`org.kde.PlasmaSip`) providing 14 methods and event signals.
  - Non-root execution policy enforcement (`assert_not_running_as_root`).
  - Duplicate instance prevention via D-Bus well-known name acquisition.
  - Automatic account and credential recovery upon service restart.
- **KDE Plasma 6 User Interface (`plasma-sip`):**
  - Kirigami 6 QML desktop interface supporting light/dark theme transitions.
  - Native numeric dial pad supporting mouse and keyboard input.
  - Active call view with duration timer, mute toggle, in-call DTMF keypad, and hangup.
  - Inbound call notification banner with answer and reject buttons.
  - Account configuration wizard for FRITZ!Box and generic SIP accounts.
  - Audio device selection view and SQLite-backed call history.
- **Security & Secret Storage:**
  - Freedesktop Secret Service / KDE KWallet integration for zero plaintext passwords.
  - Automatic redaction of sensitive credentials in debug logs and panic traces.
- **Linux Packaging & Deployment:**
  - Freedesktop desktop entry (`org.kde.plasma_sip.desktop`) with `desktop-file-validate` verification.
  - Modern KDE Breeze-styled SVG application icon (`org.kde.plasma_sip.svg`).
  - D-Bus activation service file (`org.kde.PlasmaSip.service`).
  - systemd user service unit (`plasma-sipd.service`) with sandboxing.
  - Automated user and system installation scripts (`install.sh`, `uninstall.sh`).
  - RPM spec file for Fedora Linux (`packaging/rpm/plasma-sip.spec`).
- **Comprehensive Documentation:**
  - Architecture documentation (`ARCHITECTURE.md`).
  - User guide (`docs/USER_GUIDE.md`).
  - Troubleshooting guide (`docs/TROUBLESHOOTING.md`).
  - Security and privacy architecture (`docs/SECURITY.md`).
  - FRITZ!Box hardware test report (`docs/HARDWARE_TEST_RESULTS.md`).
  - Final deployment verification report (`docs/DEPLOYMENT_REPORT.md`).
