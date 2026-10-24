# Plasma SIP Architecture

## System Overview
Plasma SIP is designed with a layered, modular architecture written in 100% pure Rust. It separates the background telephony engine (`plasma-sipd`) from the KDE Plasma Kirigami user interface (`plasma-sip`) via D-Bus IPC.

```
+-------------------------------------------------------------------+
|               KDE Plasma UI (plasma-sip / Kirigami)               |
+---------------------------------+---------------------------------+
                                  | D-Bus IPC (org.kde.PlasmaSip)
+---------------------------------v---------------------------------+
|                   Background Daemon (plasma-sipd)                 |
|  +-------------------------------------------------------------+  |
|  | sip-service: AccountManager, CallManager, AudioManager      |  |
|  +------------------+-----------------------+------------------+  |
|                     |                       |                     |
|  +------------------v---------+   +---------v------------------+  |
|  | sip-storage:               |   | sip-engine:                |  |
|  | - SQLite (accounts, calls) |   | - RegistrationManager      |  |
|  | - Keyring (Secret Service) |   | - CallStateMachine         |  |
|  +----------------------------+   +---------+------------------+  |
|                                             |                     |
|  +------------------------+-----------------+------------------+  |
|  |                        |                                    |  |
|  v                        v                                    v  |
| +--------------------+   +-------------------+  +---------------+ |
| | sip-protocol:      |   | sip-media:        |  | sip-providers:| |
| | - RFC 3261 Parse   |   | - RTP (RFC 3550)  |  | - FritzBox    | |
| | - Transactions     |   | - Jitter Buffer   |  | - Generic SIP | |
| | - Dialogs          |   | - RFC 4733 DTMF   |  +---------------+ |
| | - SDP Negotiation  |   +---------+---------+                    |
| +----------+---------+             |                              |
|            |             +---------v---------+                    |
| +----------v---------+   | sip-audio:        |                    |
| | sip-transport:     |   | - G.711 PCMA/PCMU |                    |
| | - UDP Sockets      |   | - CPAL (PipeWire) |                    |
| | - Retransmissions  |   | - Ringbuffer SPSC |                    |
| +----------+---------+   +-------------------+                    |
|            |                                                      |
| +----------v---------+                                            |
| | sip-auth:          |                                            |
| | - Digest Auth MD5  |                                            |
| +--------------------+                                            |
|                                                                   |
| +--------------------------------------------------------------+  |
| | sip-domain: Pure types, CallId, AccountId, States, Invariants |  |
| +--------------------------------------------------------------+  |
+-------------------------------------------------------------------+
```

## Layer Descriptions

1. **`sip-domain`**: Pure, decoupled domain entities (AccountId, CallId, RegistrationState, CallState, SipUri, DeviceId, Error definitions). Zero network or OS dependencies.
2. **`sip-protocol`**: RFC 3261 parser/serializer, transaction layer (RFC 3261 Section 17), dialog state machine (Section 12), timers, and SDP negotiation (RFC 4566 / RFC 3264).
3. **`sip-transport`**: Asynchronous UDP packet transport, reliable message transmission, MTU handling.
4. **`sip-auth`**: HTTP Digest Authentication (RFC 2617 / RFC 7616) with MD5 algorithm, cnonce/nonce-count handling.
5. **`sip-media`**: RTP packet generation and parsing (RFC 3550), adaptive jitter buffer, packet loss handling, RFC 4733 DTMF event payload generation and reception.
6. **`sip-audio`**: ITU-T G.711 A-law and μ-law codecs, CPAL audio stream management, lock-free SPSC ringbuffer between audio callbacks and Tokio tasks.
7. **`sip-engine`**: High-level telephony client coordinator tying together signaling, media sessions, and accounts.
8. **`sip-providers`**: Provider abstraction with specialized profiles for AVM FRITZ!Box (extension numbers, `fritz.box` registrar, digest realms) and generic SIP servers.
9. **`sip-storage`**: SQLite database for persistent account settings and call history; Freedesktop Secret Service (KWallet) integration for secure password storage.
10. **`sip-service`**: Application service daemon core coordinating background accounts, auto-reconnect, and D-Bus interfaces.
11. **`sip-platform`**: Linux/KDE platform integrations (Freedesktop Notifications, systemd user units).
12. **`plasma-sipd`**: Daemon executable running in user session.
13. **`plasma-sip`**: Graphical Kirigami interface communicating via D-Bus.
