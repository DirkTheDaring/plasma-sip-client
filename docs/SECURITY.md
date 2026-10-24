# Security and Privacy Architecture — Plasma SIP

## 1. Threat Model & Security Posture

Plasma SIP is a desktop telephony client operating in Linux user sessions (KDE Plasma 6). The application handles sensitive telecommunication data, including SIP authentication credentials, call signaling, audio streams, and contact records.

Our architecture enforces **Defense in Depth** across five security tiers:

```
+--------------------------------------------------------------+
| 1. OS & Process Isolation: Strict Non-Root, User-Session Bus |
+--------------------------------------------------------------+
| 2. Secret Storage: Freedesktop Secret Service / KWallet      |
+--------------------------------------------------------------+
| 3. Network & Protocol: Bounded Parsers, Fuzz-Hardened SIP/RTP|
+--------------------------------------------------------------+
| 4. Memory Safety: 100% Pure Rust, Zero Unsafe in App Logic   |
+--------------------------------------------------------------+
| 5. Privacy & Logging: Zero Credential Redaction, Auto-Purge   |
+--------------------------------------------------------------+
```

---

## 2. Credential Security & Secret Storage

1. **Zero Plaintext Passwords on Disk:**
   - Account passwords are never stored in configuration files (`~/.config/plasma-sip/`) or in SQLite databases (`~/.local/share/plasma-sip/calls.db`).
   - All SIP authentication passwords are delegated to the Linux Freedesktop Secret Service specification via the `keyring` crate, which integrates natively with **KDE KWallet** or GNOME Keyring.
   - When an account is removed by the user, its corresponding secret entry in KWallet is purged immediately.

2. **Zero Secrets in Logs or Panic Traces:**
   - The `SipAccount` domain model implements a custom `Debug` trait that automatically redacts sensitive fields:
     ```rust
     f.debug_struct("SipAccount")
      .field("username", &self.username)
      .field("password", &"[REDACTED]")
      .finish()
     ```
   - Tracing logs at `DEBUG` or `TRACE` levels never print `Authorization` or `Proxy-Authorization` header parameter `response=` or cleartext hashes.
   - Rust panic handlers (`std::panic::set_hook`) strip any potential credentials from error strings prior to printing.

---

## 3. Network & Protocol Hardening

1. **Bounded Message Buffers & Allocation Protection:**
   - SIP UDP datagram parsers enforce strict size limits (maximum 65,535 bytes, typical SIP datagrams capped at 4,096 bytes).
   - SDP parsing rejects unbounded attribute floods or lines longer than 1,024 characters.
   - Parsing of numeric headers (`Content-Length`, `Expires`, `CSeq`) validates integer bounds and rejects negative or overflowing values before allocation.

2. **Fuzz-Hardened Parsers:**
   - Both the SIP/SDP parser (`crates/sip-protocol`) and RTP packet parser (`crates/sip-media`) are subjected to automated fuzz test suites (`parser_hardening_test.rs`, `rtp_fuzz_test.rs`).
   - Truncated packets, corrupt RTP version headers, oversized extension lengths, and invalid UTF-8 sequences are rejected with typed `Err(...)` variants and never trigger panics or buffer overreads.

3. **Retransmission Timers & Rate-Limiting:**
   - SIP transactions implement bounded exponential backoff timers conforming to RFC 3261 Section 17 (Timer A = 500ms doubling to Timer B = 32s).
   - Reconnection logic enforces jitter and an exponential ceiling to prevent denial-of-service / hammering against registrars or local PBXs during network outages.

---

## 4. Privilege Separation & D-Bus Access Control

1. **Enforced Non-Root Execution:**
   - Both `plasma-sip` and `plasma-sipd` inspect their process status (`/proc/self/status`) on startup using `assert_not_running_as_root()`.
   - If executed as `root` (UID 0), the process immediately exits with code 1 and logs an audit error.

2. **User-Session Bus Isolation:**
   - Plasma SIP's D-Bus interface (`org.kde.PlasmaSip`) binds exclusively to the Freedesktop User Session Bus (`DBUS_SESSION_BUS_ADDRESS`).
   - It is never exposed on the System Bus (`/var/run/dbus/system_bus_socket`), preventing unprivileged or external local system users from snooping telephony control signals.
   - All D-Bus incoming string arguments are validated against strict length and character set bounds prior to passing into the engine.

3. **systemd Sandboxing:**
   - The provided `plasma-sipd.service` user unit enables systemd security directives:
     - `NoNewPrivileges=true`
     - `ProtectSystem=strict`
     - `ProtectHome=read-only`
     - `ReadWritePaths=%h/.local/share/plasma-sip`
     - `PrivateTmp=true`

---

## 5. Call Privacy & Data Retention

1. **Call History Management:**
   - Call logs (caller URI, recipient URI, timestamp, duration, disposition) are stored in a local SQLite database (`~/.local/share/plasma-sip/calls.db`).
   - Users can query, selectively delete, or completely purge call history directly from the UI or via D-Bus (`ClearCallHistory`).
   - Call audio is processed strictly in-memory through lock-free ringbuffers and is **never written to disk or recorded**.

---

## 6. Known Protocol Limitations & Future Roadmap

1. **Unencrypted SIP / RTP (RFC 3261 / RFC 3550):**
   - In standard local PBX setups (including typical FRITZ!Box home LANs), SIP signaling uses unencrypted UDP port 5060 and media uses unencrypted RTP.
   - Attackers on the local broadcast domain or Wi-Fi network with packet capture access could potentially inspect signaling or audio streams.
2. **Roadmap to SIPS & SRTP:**
   - The `sip-transport` and `sip-media` crates are architected with clean transport traits to support TLS (SIPS) and SRTP (RFC 3711) in future revisions without redesigning the core state machine.

---

## 7. Reporting a Vulnerability

If you discover a potential security vulnerability within Plasma SIP, please **do not** open a public issue.

Instead, please disclose it responsibly via:
- **GitHub Private Vulnerability Reporting:** Navigate to the Security tab of the repository and click "Report a vulnerability".
- **Maintainers Email:** Send an encrypted or private report to `security@plasma-sip.org`.

Please include:
- Clear description of the vulnerability and attack vector.
- Reproduction steps or minimal proof of concept.
- Affected components or versions.

We appreciate your effort in responsibly disclosing issues and will respond promptly to investigate and remediate valid reports.

