# Risk Register — Plasma SIP

| ID | Risk | Severity | Likelihood | Mitigation Strategy | Status |
|---|---|---|---|---|---|
| **R-01** | Pure Rust SIP stack lacks complete RFC 3261 transaction engine | High | High | Implement deterministic transaction state machine (RFC 3261 Section 17) directly in `sip-protocol` with virtual clock test fixtures. | Open |
| **R-02** | Real-time audio jitter and underruns in PipeWire | Medium | Medium | Use lock-free SPSC ringbuffers (`rtrb`), preallocated buffers, adaptive jitter buffer, and G.711 packet loss concealment. | Open |
| **R-03** | Secret Service / KWallet unavailable or locked | Medium | Low | Graceful fallback to prompt/session cache, actionable error states without crashing daemon. | Open |
| **R-04** | FRITZ!Box firmware variations and quirks | Medium | Medium | Decoupled provider profile (`FritzBoxProvider`) with configurable registrar, internal extension parsing (`**620`), and dial string normalization. | Open |
| **R-05** | Lack of physical hardware during development | High | High | Automated SIP registrar and call-endpoint test fixtures simulating challenges, timeouts, and media loopback. Explicit BLOCKED status for physical hardware verification until tested on real device. | Open |
