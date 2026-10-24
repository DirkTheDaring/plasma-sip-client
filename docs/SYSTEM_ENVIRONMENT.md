# System & User Environment Specification — Plasma SIP

This document provides a comprehensive specification of all environment requirements, runtime expectations, network assumptions, system dependencies, and configuration mechanisms for **Plasma SIP** (`plasma-sip` and `plasma-sipd`).

---

## 1. Executive Summary

Plasma SIP is engineered as an unprivileged, multi-process softphone architecture specifically designed for modern Linux desktop environments running KDE Plasma 6.

```
+-----------------------------------------------------------------------------------+
|                              USER SESSION ENVIRONMENT                             |
|                                                                                   |
|  +--------------------+        Session D-Bus (IPC)        +--------------------+  |
|  |     plasma-sip     | <===============================> |    plasma-sipd     |  |
|  |  (Kirigami 6 GUI)  |    org.kde.PlasmaSip              | (Telephony Daemon) |  |
|  |  org.kde...ui      |    org.kde.plasma_sip.ui          |                    |  |
|  +--------------------+                                   +--------------------+  |
|            |                                                         |            |
|            v                                                         v            |
|     Qt 6 / Wayland / X11                                      Tokio Async Core    |
|     System Tray (SNI)                                                |            |
|     KWallet / Secret Service                                         |            |
+----------------------------------------------------------------------|------------+
                                                                       |
+----------------------------------------------------------------------|------------+
|                             HOST SYSTEM ENVIRONMENT                  |            |
|                                                                      v            |
|   +-----------------------+   +----------------------+   +---------------------+  |
|   |   PipeWire / ALSA     |   |    DNS Resolution    |   | Network Sockets     |  |
|   |   Audio Capture &     |   |   /etc/resolv.conf   |   | UDP 5060 (SIP)      |  |
|   |   Playback Streams    |   |   RFC 3263 SRV + NSS |   | UDP Ephemeral (RTP) |  |
|   +-----------------------+   +----------------------+   +---------------------+  |
+-----------------------------------------------------------------------------------+
```

---

## 2. System Environment Expectations

System-level expectations define what `plasma-sipd` and `plasma-sip` require from the host Linux kernel, network stack, and operating system services.

### 2.1 Network & DNS Host Resolution

Plasma SIP implements a dual-tier RFC 3263-compliant resolver for server location and registrar discovery:

#### Tier 1: RFC 3263 DNS SRV Resolution (`_sip._udp.<domain>`)
- **System Nameservers (`/etc/resolv.conf`)**:
  - The in-tree DNS resolver parses `/etc/resolv.conf` to discover configured nameserver IP addresses.
  - **Fallback Chain**: If `/etc/resolv.conf` is missing, unreadable, or contains no nameservers, the resolver falls back to:
    1. `127.0.0.53:53` (local `systemd-resolved` stub resolver)
    2. `1.1.1.1:53` (Cloudflare public DNS)
- **Transport**:
  - Sends standard RFC 1035 UDP datagrams to nameservers on port `53`.
  - Timeout: 800 ms per nameserver attempt.
  - Queries `QTYPE=33` (SRV) for `_sip._udp.<domain>`.
- **Negative Caching / Authoritative Response**:
  - Receives and parses authoritative negative responses (`NXDOMAIN` or `NODATA`). On negative response, terminates SRV query immediately without querying secondary servers, proceeding directly to Tier 2.

#### Tier 2: System NSS & libc Fallback (A / AAAA)
- If DNS SRV records do not exist or outbound port 53 UDP is restricted, the resolver falls back to standard operating system name resolution (`getaddrinfo` via `tokio::net::lookup_host`).
- **NSS Integration**:
  - Fully respects `/etc/nsswitch.conf` and `/etc/hosts`.
  - Supports Multicast DNS (mDNS) / Avahi `.local` domains.
  - Resolves local router hostnames such as `fritz.box` through the local DHCP-assigned gateway.

#### Target Address Rules & Formats
| Target Format | Example | Resolution Strategy |
|---|---|---|
| **Numeric IPv4** | `192.168.178.1` | Direct connection; zero DNS lookups performed (RFC 3263 §4.1). |
| **Numeric IPv6** | `::1` or `[2001:db8::1]` | Direct connection; zero DNS lookups performed. |
| **Explicit Port** | `sip.provider.com:5062` | Bypasses SRV lookup; resolves host directly via A/AAAA on specified port. |
| **Domain Name (default port 5060)** | `fritz.box` or `sipgate.de` | Queries SRV `_sip._udp.<domain>` on port 53; falls back to A/AAAA on port 5060. |

---

### 2.2 Network Sockets & Port Requirements

| Protocol | Port | Direction | Purpose | Configuration / Overrides |
|---|---|---|---|---|
| **UDP** | `5060` | Inbound / Outbound | SIP Signaling (INVITE, REGISTER, BYE, ACK, CANCEL) | Configurable via `PLASMA_SIP_BIND_ADDR`. Default: `0.0.0.0:5060`. |
| **UDP** | Ephemeral | Inbound / Outbound | Fallback SIP Signaling | If `5060` is in use, automatically falls back to `0.0.0.0:0`. |
| **UDP** | Dynamic (`>1024`) | Inbound / Outbound | RTP Audio Media (G.711 PCMA/PCMU) | Bound per active call. Default RTP base: `16384` or kernel-assigned. |
| **UDP** | `53` | Outbound | RFC 3263 DNS SRV queries | Directed to local resolver (`127.0.0.53:53` or router gateway). |

#### Firewall & NAT Requirements
- **Local Firewall (`firewalld` / `nftables` / `iptables`)**:
  - **Outbound**: UDP traffic to registrar port `5060`, remote RTP ports (e.g. `7078–7087` on FRITZ!Box, `10000–20000` on Asterisk), and DNS port `53` must be allowed.
  - **Inbound**: If receiving unsolicited inbound SIP calls directly over LAN, port `5060/udp` should be open (`firewall-cmd --add-port=5060/udp`).
- **Symmetric RTP & NAT Traversal (RFC 4961)**:
  - Plasma SIP implements RFC 4961 symmetric latching: outgoing RTP packets open outbound NAT state in the router/firewall; the client latches incoming RTP destination to the peer's actual source address upon first packet arrival (with SEC-04 IP spoofing protection).

---

### 2.3 Kernel & Security Execution Invariants

1. **Mandatory Non-Root Execution Policy**:
   - Both `plasma-sip` and `plasma-sipd` inspect `/proc/self/status` on startup.
   - If effective UID is `0` (`root`), the process immediately terminates with an error. Neither daemon nor client will ever run with administrative privileges.
2. **File System Permissions (POSIX)**:
   - Data directory permissions are explicitly set to `0700` (`rwx------`).
   - SQLite database file permissions are explicitly set to `0600` (`rw-------`).

---

## 3. User System Environment Expectations

User-level expectations describe what the software requires from the logged-in desktop user session.

### 3.1 D-Bus Session Bus (`$DBUS_SESSION_BUS_ADDRESS`)

Both binaries require an active standard FreeDesktop D-Bus session bus.

#### Telephony Daemon (`plasma-sipd`)
- **Well-Known Bus Name**: `org.kde.PlasmaSip`
- **Object Path**: `/org/kde/PlasmaSip`
- **Interface**: `org.kde.PlasmaSip`
- **Duplicate Instance Protection**: Configured with `allow_name_replacements(false)` and `replace_existing_names(false)`. If an instance is already running, secondary spawns exit immediately.
- **NameLost Monitoring**: The daemon monitors D-Bus for `NameLost` signals and performs a graceful shutdown if ownership is revoked.
- **Service Activation**:
  - D-Bus service file: `/usr/share/dbus-1/services/org.kde.PlasmaSip.service` (or `~/.local/share/dbus-1/services/`).
  - Allows D-Bus clients to automatically activate the daemon on demand.

#### Graphical User Interface (`plasma-sip`)
- **Single-Instance IPC Bus Name**: `org.kde.plasma_sip.ui`
- **Object Path**: `/org/kde/plasma_sip/ui`
- **Single-Instance Behavior**:
  - When a user launches a second GUI instance (e.g. from the application menu or via `plasma-sip --dial <number>`), the new process calls `Activate()` or `Dial(target)` on the existing instance over D-Bus and exits with code `0`.

---

### 3.2 systemd User Session (`systemctl --user`)

- **Unit File**: `plasma-sipd.service`
  - Located in `/usr/lib/systemd/user/` (system install) or `~/.config/systemd/user/` (user install).
  - Target: `default.target`.
- **Preset File**: `50-plasma-sip.preset`
  - Located in `/usr/lib/systemd/user-preset/`.
  - Enables `plasma-sipd.service` automatically for all user login sessions upon package installation.
- **Graceful Lifecycle Management**:
  - On `systemctl --user stop` or `restart`, systemd delivers `SIGTERM`.
  - `plasma-sipd` intercepts `SIGTERM`, transmits a SIP `REGISTER` with `Expires: 0` to gracefully unregister active accounts with the PBX, closes SQLite storage, and exits cleanly.

---

### 3.3 Audio Subsystem (PipeWire / ALSA)

- **Sound Server**:
  - Designed for **PipeWire** (`pipewire.service`, `wireplumber.service`) with `pipewire-pulse` and `pipewire-alsa`.
  - Compatible with direct ALSA device access if PipeWire is not in use.
- **Device Access & Permissions**:
  - Requires user access to audio device nodes (`/dev/snd/*`). On modern Linux distributions, device ACLs are granted automatically to the active seat by `systemd-logind`.
- **Sample Rate Conversion**:
  - Hardware devices running at 44,100 Hz, 48,000 Hz, or 96,000 Hz are supported out-of-the-box.
  - In-tree 21-tap windowed-sinc Low-Pass FIR anti-aliasing filter and linear resampler downmix and downsample device streams to 8,000 Hz ITU-T G.711 PCMA/PCMU without requiring external resampling daemons.

---

### 3.4 Secret Storage & Credentials (KWallet / Secret Service)

- **Zero Plaintext Passwords on Disk**:
  - Account passwords are **never** stored in the SQLite database or any configuration file.
- **Provider**:
  - Integrates with the FreeDesktop Secret Service API (`org.freedesktop.secrets`).
  - On KDE Plasma: backed by **KWallet** (`kwalletd6`).
  - On other desktop environments: compatible with GNOME Keyring.
- **User Session Expectation**:
  - The user's default wallet should be configured to unlock automatically upon desktop login (via `pam_kwallet5` / `pam_kwallet6` in PAM configuration).
  - If locked, the desktop Secret Service prompter will request the user's master password when `plasma-sipd` starts.

---

### 3.5 File System Hierarchy & XDG Standard Paths

Plasma SIP adheres strictly to the XDG Base Directory Specification:

| Path | XDG Variable | Purpose | Permissions |
|---|---|---|---|
| `~/.local/share/plasma-sip/plasma-sip.db` | `$XDG_DATA_HOME` | SQLite database (SIP accounts, call logs) | `0600` (file), `0700` (dir) |
| `~/.config/systemd/user/plasma-sipd.service` | `$XDG_CONFIG_HOME` | systemd user service unit (user mode) | `0644` |
| `~/.config/autostart/org.kde.plasma_sip.desktop` | `$XDG_CONFIG_HOME` | Autostart launcher for system tray icon | `0644` |
| `/usr/share/plasma-sip/qml/` | `$XDG_DATA_DIRS` | QML user interface modules and components | `0644` (system-wide) |
| `/usr/share/applications/org.kde.plasma_sip.desktop` | `$XDG_DATA_DIRS` | Desktop launcher and URI scheme handler | `0644` (system-wide) |
| `/usr/share/icons/hicolor/scalable/apps/org.kde.plasma_sip.svg` | `$XDG_DATA_DIRS` | Vector application icon | `0644` (system-wide) |

---

### 3.6 Desktop Environment & KDE Plasma 6 Services

- **Display Protocol**: Fully compatible with both **Wayland** (`XDG_SESSION_TYPE=wayland`) and **X11** (`XDG_SESSION_TYPE=x11`).
- **Frameworks & Libraries**:
  - Qt 6 Core, Gui, and Qml (`qt6-qtbase`, `qt6-qtdeclarative`).
  - KDE Frameworks 6 Kirigami (`kf6-kirigami`).
- **System Tray (StatusNotifierItem)**:
  - Registers an SNI tray icon via `org.kde.StatusNotifierWatcher` (provided by `plasmashell`).
  - Reflects real-time telephony status:
    - Registered / Idle (phone icon)
    - Incoming call (ringing animation)
    - Active call (call in progress indicator with remote party display)
- **Desktop Notifications**:
  - Connects to `org.freedesktop.Notifications`.
  - Displays incoming call alerts with interactive action buttons (**Open**, **Answer**, **Reject**).

---

## 4. Environment Variables Reference

The following environment variables can be set to customize or override runtime behavior:

| Variable | Target Component | Default Value | Description |
|---|---|---|---|
| `PLASMA_SIP_BIND_ADDR` | `plasma-sipd` | `0.0.0.0:5060` | SIP UDP local listening socket address and port. |
| `PLASMA_SIP_DATA_DIR` | `plasma-sipd` | `~/.local/share/plasma-sip` | Directory path for SQLite database storage. |
| `RUST_LOG` | Both binaries | `info` | Logging verbosity filter (e.g. `sip_transport=debug,sip_engine=trace`). |
| `QT_LOGGING_RULES` | `plasma-sip` | `qt.qml.binding=false` | Qt logging category filter (suppresses benign Kirigami internal binding warnings). |
| `DBUS_SESSION_BUS_ADDRESS` | Both binaries | (Set by desktop session) | Address of the user D-Bus session socket. |
| `HOME` | Both binaries | (Set by PAM/login) | User home directory path used for XDG fallbacks. |

---

## 5. Diagnostic Checklist for Administrators & Users

When troubleshooting environmental issues, verify the following:

```bash
# 1. Verify D-Bus session bus connectivity
dbus-send --session --dest=org.freedesktop.DBus --type=method_call \
    --print-reply /org/freedesktop/DBus org.freedesktop.DBus.ListNames | grep -i plasmasip

# 2. Check system nameserver accessibility
cat /etc/resolv.conf
dig +short _sip._udp.fritz.box SRV

# 3. Verify SIP port binding availability
ss -ulnp | grep 5060

# 4. Check PipeWire user audio service status
systemctl --user status pipewire.service wireplumber.service
wpctl status

# 5. Check KWallet / Secret Service availability
qdbus org.kde.kwalletd6 /modules/kwalletd6 org.kde.KWallet.isOpen kdewallet

# 6. Verify systemd user service status
systemctl --user status plasma-sipd.service
journalctl --user -u plasma-sipd.service -n 50 --no-pager
```
