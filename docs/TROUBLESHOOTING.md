# Plasma SIP — Troubleshooting Guide

This guide provides diagnostic solutions for common SIP signaling, audio, and KDE desktop integration issues.

---

## 1. SIP Registration Issues

### Symptom: Registration fails with "401 Unauthorized" or "Authentication Failed"
- **Cause:** The SIP username or password does not match the credentials configured in the registrar.
- **FRITZ!Box Fix:**
  1. Open `http://fritz.box` -> **Telephony** -> **Telephony Devices**.
  2. Edit your IP phone device.
  3. Ensure the username is the exact internal ID (e.g. `620`) and set a new password.
  4. Ensure your password is at least 8 characters long and contains alphanumeric characters.
  5. In Plasma SIP, click **Edit Account**, enter the new password, and click **Save & Re-register**.

### Symptom: Registration fails with "408 Request Timeout" or "Host Unreachable"
- **Cause:** The softphone cannot reach the registrar over UDP port 5060, or DNS resolution failed.
- **Checklist:**
  - Verify that `fritz.box` resolves on your system:
    ```bash
    ping -c 2 fritz.box
    ```
    If DNS fails, use the direct gateway IP address instead: `192.168.178.1`.
  - Check if a local Linux firewall (e.g. `firewalld` or `nftables`) is blocking outgoing or incoming UDP:
    ```bash
    sudo firewall-cmd --state
    ```
  - Ensure you are connected to the same LAN or Wi-Fi network as your PBX/FRITZ!Box.

### Symptom: Registration fails with "403 Forbidden"
- **Cause:** The registrar rejected the registration request. On FRITZ!Box, this occurs if:
  - The IP phone is configured to only allow calls from specific IP subnets.
  - The IP phone has been temporarily locked due to too many failed authentication attempts. Wait 5 minutes and try again.

---

## 2. Audio & Media Issues

### Symptom: One-Way Audio (I can hear the other party, but they cannot hear me)
- **Cause:** The remote party's RTP stream is arriving, but local microphone packets cannot reach the remote PBX, or the microphone is muted.
- **Checklist:**
  1. Check if the microphone is muted in Plasma SIP or in KDE Plasma's System Tray volume applet.
  2. Check PipeWire microphone levels:
     ```bash
     wpctl status
     ```
  3. In Plasma SIP, go to **Audio Settings** and ensure the correct input device is selected.

### Symptom: Audio Choppiness, Glitches, or High Jitter
- **Cause:** Network packet loss on Wi-Fi or CPU latency spikes during audio buffer processing.
- **Solutions:**
  - Connect to your local network via Ethernet cable rather than 2.4 GHz Wi-Fi to eliminate radio interference.
  - Plasma SIP includes an adaptive G.711 jitter buffer that conceals isolated packet losses. High loss (>10%) will degrade speech quality.

### Symptom: No Audio Devices Found
- **Cause:** PipeWire or ALSA sound subsystem is unresponsive or permissions are missing.
- **Fix:**
  - Restart the user PipeWire sound service:
    ```bash
    systemctl --user restart pipewire.service wireplumber.service
    ```

---

## 3. Background Daemon (`plasma-sipd`) Issues

### Symptom: "Failed to connect to telephony service via D-Bus"
- **Cause:** The `plasma-sipd` daemon is not running or the session bus is unreachable.
- **Check status:**
  ```bash
  systemctl --user status plasma-sipd.service
  ```
- **Start service manually:**
  ```bash
  systemctl --user start plasma-sipd.service
  ```
- **Inspect daemon log output:**
  ```bash
  journalctl --user -u plasma-sipd.service -e
  ```

### Symptom: Daemon exits immediately with "Process cannot run as root"
- **Cause:** Security enforcement policy prevents `plasma-sipd` from executing as UID 0 (root).
- **Fix:** Run the application exclusively under your standard desktop user account. Never invoke `plasma-sip` or `plasma-sipd` with `sudo`.

---

## 4. Keyring & Secret Storage Issues

### Symptom: Password is asked every time the daemon restarts
- **Cause:** KDE KWallet or the Secret Service provider is locked or not auto-unlocking on login.
- **Fix:**
  - Open **KDE System Settings** -> **KDE Wallet** and ensure the default wallet is open.
  - Ensure the `pam_kwallet5` or `pam_kwallet6` PAM module is enabled in `/etc/pam.d/sddm` or `/etc/pam.d/login` so KWallet unlocks automatically upon desktop login.

---

## 5. Generating Diagnostic Information

When reporting an issue, you can run Plasma SIP with structured debugging enabled:
```bash
RUST_LOG=debug plasma-sipd
```

> **Privacy Note:** Plasma SIP automatically redacts all authentication passwords, hashes, and sensitive credential tokens from log output.
