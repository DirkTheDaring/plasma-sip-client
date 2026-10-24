# Implementation Plan — Plasma SIP

## Project Overview
Plasma SIP is a production-grade, pure Rust SIP softphone crafted for KDE Plasma 6 on modern Linux (Wayland & X11). Initially targeting AVM FRITZ!Box as the primary registrar/PBX with provider-agnostic core architecture.

---

## Milestone Road Map & Quality Gates

### Milestone 0: Research & Architecture Validation
- **Goal:** Validate dependencies, licenses, toolchains, build feasibility, audio integration, and protocol parsers.
- **Deliverables:** Dependency selection matrix, risk register, ADRs, minimal compilable Cargo workspace.
- **Exit Gate:** Verified compilation and compatibility of all core technologies or pure Rust fallbacks.

### Milestone 1: Domain Model & Project Foundation
- **Goal:** Establish domain types, error handling, provider profiles, logging, and test fixtures.
- **Deliverables:** `crates/sip-domain`, `crates/sip-providers`, configuration validation, dial-string normalization, test suites.
- **Exit Gate:** 100% unit tests pass, Clippy clean, strict type invariants enforced.

### Milestone 2: SIP Registration
- **Goal:** Robust RFC 3261 SIP registration engine with Digest Authentication.
- **Deliverables:** `crates/sip-protocol`, `crates/sip-transport`, `crates/sip-auth`, registrar test fixture, registration state machine, renewal timer, backoff/retry.
- **Exit Gate:** Automated registration, challenge auth, renewal, and teardown against local SIP test fixture.

### Milestone 3: Outgoing Calls
- **Goal:** Outgoing SIP call establishment and dialog lifecycle.
- **Deliverables:** INVITE, SDP offer generation, provisional responses (100 Trying, 180 Ringing, 183 Session Progress), 200 OK, ACK, CANCEL, BYE, call state machine.
- **Exit Gate:** Automated call setup, ringing, cancellation, and remote/local termination against SIP test endpoint.

### Milestone 4: RTP & Real-Time Audio Subsystem
- **Goal:** Bidirectional audio streaming with G.711 (PCMA/PCMU), RTP transport, and jitter buffering.
- **Deliverables:** `crates/sip-media`, `crates/sip-audio`, ITU-T G.711 codecs, RFC 3550 RTP, adaptive jitter buffer, CPAL integration, synthetic loopback test.
- **Exit Gate:** Bidirectional audio loopback tests pass with zero packet drop in synthetic tests and zero resource leaks.

### Milestone 5: Incoming Calls & In-Call Signaling
- **Goal:** Incoming call handling, ringing alert, answer/reject, busy signaling, and RFC 4733 DTMF.
- **Deliverables:** Inbound INVITE processing, 180 Ringing emission, 200 OK answer, 486 Busy Here, RFC 4733 DTMF sender/receiver.
- **Exit Gate:** End-to-end inbound and outbound call flows pass with two-way media and DTMF transmission.

### Milestone 6: Background Service & D-Bus Interface
- **Goal:** Headless user session daemon (`plasma-sipd`) with stable D-Bus API.
- **Deliverables:** `apps/plasma-sipd`, `crates/sip-service`, `crates/sip-storage` (SQLite + Secret Service), D-Bus API `org.kde.PlasmaSip`, systemd user unit.
- **Exit Gate:** Headless daemon runs independently, persists state, secures secrets, and passes full D-Bus client integration tests.

### Milestone 7: KDE Plasma Desktop User Interface
- **Goal:** Native Qt 6 / Kirigami user interface (`plasma-sip`).
- **Deliverables:** Dial pad, incoming call banner/window, active call HUD, account settings wizard, audio device selector, call history, Plasma desktop notifications.
- **Exit Gate:** Desktop application connects to `plasma-sipd` via D-Bus and provides complete UI workflow for calls and configuration.

### Milestone 8: FRITZ!Box Interoperability & Hardware Matrix
- **Goal:** Comprehensive validation against AVM FRITZ!Box telephony environment.
- **Deliverables:** Hardware test matrix, FRITZ!Box test execution records, internal extension dialing (`**620`), external calling.
- **Exit Gate:** Documented test report with real FRITZ!Box or explicit BLOCKED status with documented reproduction steps.

### Milestone 9: Generic SIP Interoperability
- **Goal:** Verify provider neutrality against standard SIP servers (Asterisk / Kamailio / SIP test server).
- **Deliverables:** Interoperability test suite against RFC-compliant generic SIP registrar.
- **Exit Gate:** Generic SIP account registers and completes calls without provider-specific code branches.

### Milestone 10: Hardening, Security & Performance Profiling
- **Goal:** Protocol fuzzing, stress testing, leak checking, latency and jitter profiling.
- **Deliverables:** Fuzz tests for SIP/SDP/RTP parsers, memory profile, privacy audit (zero credential leakage).
- **Exit Gate:** Zero memory leaks, fuzzing resilience, latency within 20ms audio frame budgets.

### Milestone 11: Packaging & Deployment
- **Goal:** Production distribution packaging, desktop integration files, user docs.
- **Deliverables:** `.desktop` files, D-Bus service activation files, systemd user service unit, RPM/tarball packaging, installation and user guide.
- **Exit Gate:** Clean install on reference Linux distribution with no root privileges required.
