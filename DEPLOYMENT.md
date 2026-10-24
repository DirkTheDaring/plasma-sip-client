# Deployment Guide — Plasma SIP

## 1. Reference Deployment Target
- **Operating System:** Linux (Fedora 44 / Linux Kernel 7.2.11 / x86_64)
- **Desktop Environment:** KDE Plasma 6 (Wayland & X11)
- **Init System:** systemd user session (`~/.config/systemd/user/plasma-sipd.service`)
- **D-Bus:** Freedesktop User Session Bus (`org.kde.PlasmaSip`)
- **Audio Server:** PipeWire (via WirePlumber) / ALSA compatibility
- **Secret Storage:** KDE KWallet / Freedesktop Secret Service API
- **Detailed Environment Specification:** See [System & User Environment Specification](docs/SYSTEM_ENVIRONMENT.md) for full network, DNS, port, socket, and D-Bus expectations.

---

## 2. Fast Installation (Recommended)

### User Mode (No Root Privileges Required)
To install Plasma SIP into your user environment (`~/.local/` and `~/.config/`):
```bash
./packaging/install.sh
```
This script installs:
- Binaries: `~/.local/bin/plasma-sip` and `~/.local/bin/plasma-sipd`
- Desktop Entry: `~/.local/share/applications/org.kde.plasma_sip.desktop`
- Breeze Icon: `~/.local/share/icons/hicolor/scalable/apps/org.kde.plasma_sip.svg`
- D-Bus Service: `~/.local/share/dbus-1/services/org.kde.PlasmaSip.service`
- systemd Unit: `~/.config/systemd/user/plasma-sipd.service`

### Enable Background Daemon
```bash
systemctl --user enable --now plasma-sipd.service
```

### Launch Desktop Application
```bash
plasma-sip
```

---

## 3. System-Wide Installation
To install system-wide under `/usr/local/`:
```bash
sudo ./packaging/install.sh --system
```

---

## 4. Uninstallation
To completely remove Plasma SIP:
```bash
# User mode uninstallation
./packaging/uninstall.sh

# User mode uninstallation including purging stored history & databases
./packaging/uninstall.sh --purge
```

---

## 5. Building an RPM Package (Fedora)
A complete RPM spec file is provided in [`packaging/rpm/plasma-sip.spec`](packaging/rpm/plasma-sip.spec):
```bash
rpmbuild -ba packaging/rpm/plasma-sip.spec
```

---

## 6. Security & Execution Boundaries
- **Strict Non-Root Execution:** `plasma-sip` and `plasma-sipd` refuse to execute as `root`.
- **Zero Plaintext Passwords:** Credentials are stored securely in KWallet via Secret Service.
- **Privacy First:** Logs automatically redact all credentials and sensitive authentication parameters.
