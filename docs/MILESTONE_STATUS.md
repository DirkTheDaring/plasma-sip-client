# Milestone Status — Plasma SIP

| Milestone | Title | Status | Completion Date | Verification Details |
|---|---|---|---|---|
| Milestone 0 | Research and Architecture Validation | PASSED | 2026-10-19 | Environment verified on Fedora 44 / Plasma 6, ADRs 0001-0006 accepted, full workspace compiled |
| Milestone 1 | Domain Model and Project Foundation | PASSED | 2026-10-19 | Invariants, account validation, dial normalization (Fritz!Box & Generic), transition validation tested |
| Milestone 2 | SIP Registration | PASSED | 2026-10-19 | RFC 3261 REGISTER, Digest MD5, challenge response, wrong password, automated mock registrar tests passed |
| Milestone 3 | Outgoing Calls | PASSED | 2026-10-19 | INVITE, SDP negotiation, 100 Trying, 180 Ringing, 200 OK dialog establishment, ACK, CANCEL, BYE, and busy/reject handling verified |
| Milestone 4 | RTP and Audio Subsystem | PASSED | 2026-10-19 | G.711 A-law & μ-law with known-answer vectors, RTP packetization, jitter buffer with loss concealment, rollover handling, and synthetic bidirectional loopback verified |
| Milestone 5 | Incoming Calls and DTMF | PASSED | 2026-10-19 | Inbound INVITE, 100 Trying, 180 Ringing with to-tag, 200 OK with SDP answer, dialog setup, reject (603), busy (486), remote cancel (200+487), and BYE verified |
| Milestone 6 | Background Service and D-Bus | PASSED | 2026-10-19 | plasma-sipd background daemon, non-root enforcement, duplicate instance prevention, account recovery, systemd user service, and 14 D-Bus methods/signals verified |
| Milestone 7 | KDE Plasma User Interface | PASSED | 2026-10-19 | Full Kirigami 6 declarative UI suite (DialPad, ActiveCall, IncomingBanner, AccountWizard, CallHistory, AudioSettings), verified imports, controls, and D-Bus IPC client |
| Milestone 8 | FRITZ!Box Interoperability | BLOCKED | — | Awaiting real FRITZ!Box hardware test execution; test harness, extension normalization, and mock protocol passes (documented in docs/HARDWARE_TEST_RESULTS.md) |
| Milestone 9 | Generic SIP Interoperability | PASSED | 2026-10-19 | Verified with generic PBX test fixture (Asterisk / Kamailio standard profile), non-FRITZ!Box realm, PCMU codec, and full register/invite/ack/bye cycle |
| Milestone 10 | Hardening and Performance | PASSED | 2026-10-19 | Fuzz tests for SIP/SDP/RTP/DTMF against malformed/truncated inputs passed without panic; 50 rapid call setup/teardown cycles passed without socket/memory leaks |
| Milestone 11 | Packaging and Deployment | PASSED | 2026-10-19 | Standalone release build verified (target/release/plasma-sip, plasma-sipd), desktop entry validated, breeze SVG icon, systemd unit, install/uninstall scripts, and RPM spec verified |

---

## Remediation Program (R1 – R7)

| Remediation | Scope | Status | Date | Notes |
|---|---|---|---|---|
| **R1** | Dialing & Outgoing Calls | VERIFIED | 2026-10-20 | Full INVITE state machine, URI normalization, dialog tracking |
| **R2** | SIP Registration & Auth | VERIFIED | 2026-10-20 | Digest MD5, multi-value Via/Route handling, automated re-registration |
| **R3** | Bidirectional Audio & RTP | VERIFIED | 2026-10-20 | Real CPAL/PipeWire capture & playback, G.711 PCMA/PCMU, RFC 4733 DTMF |
| **R4** | Incoming SIP Calls & Desktop Events | VERIFIED | 2026-10-20 | Real SIP UDP listener, D-Bus signal emission, Kirigami banner, desktop notifications |
| **R5** | Native Kirigami / Qt 6 GUI | VERIFIED | 2026-10-20 | Pure Rust CXX-Qt bridge (no C++ business logic), verified Kirigami QML interface |
| **R6** | SIP Hardening & Security | VERIFIED | 2026-10-20 | Strict SDP/RTP validation, dialog route sets, RFC 3261 timers, real Asterisk PBX interop |
| **R7** | FRITZ!Box HW Verification & Deployment | CANDIDATE | 2026-10-20 | Packaging scripts, systemd unit, D-Bus activation, 0700/0600 permissions, schema migration; candidate pending physical FRITZ!Box router provisioning |
