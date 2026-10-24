# AVM FRITZ!Box Hardware Interoperability & Acceptance Test Report

**Target Router:** AVM FRITZ!Box 6591 Cable (Example Reference Testbed)  
**Firmware Version:** FRITZ!OS 8.25 (Reference Hardware Environment)  
**LAN Gateway IP:** `192.168.178.1` (`fritz.box`, default router gateway example)  
**Client IP / Interface:** `192.168.178.x` on Ethernet (Example client IP on local network)  
**Application Version:** Plasma SIP 0.1.0  
**Overall Hardware Status:** **BLOCKED (Awaiting IP Telephone Device Provisioning on FRITZ!Box)**  

---

## 1. Hardware Verification Preconditions & Findings

| Precondition | Target Parameter | Status | Evidence / Verification Method |
|---|---|---|---|
| **1. Target Hardware Confirmed** | AVM FRITZ!Box 6591 Cable | **CONFIRMED** | TR-064 probe on port 49000 & `http://192.168.178.1/jason_boxinfo.xml` |
| **2. Target Firmware Confirmed** | FRITZ!OS 8.25 | **CONFIRMED** | FRITZ!OS 8.25 detected via TR-064 / boxinfo |
| **3. Network Connectivity** | Host to `192.168.178.1` | **CONFIRMED** | Local LAN subnet on Ethernet interface |
| **4. SIP Port 5060 Status** | UDP 5060 / TCP 5060 on router | **DORMANT** | Connection timed out (FRITZ!OS keeps 5060 closed until LAN IP phone is added) |
| **5. IP Phone Account Provisioned** | LAN/WLAN IP telephone extension (e.g. `620`) | **NOT PROVISIONED** | No account configured in router; SQLite accounts table is empty (`[]`) |

---

## 2. Hardware Acceptance Test Matrix (H1 — H7)

| Test ID | Acceptance Scenario | Status | Expected Behavior | Actual Result |
|---|---|---|---|---|
| **H1** | **SIP Registration** | **BLOCKED** | Router challenges with 401 Unauthorized; client returns MD5 digest; 200 OK received; state transitions to `Registered`. | Blocked: No IP telephone device provisioned on router; port 5060 dormant. |
| **H2** | **Outgoing Internal Call** | **BLOCKED** | Internal call to another extension (e.g. `**610` or `**1`) sends INVITE, receives 180 Ringing, establishes 200 OK dialog upon answer, and terminates with BYE. | Blocked: Awaiting H1 registration. |
| **H3** | **Bidirectional Audio** | **BLOCKED** | Microphone audio stream encoded to G.711 PCMA/PCMU reaches remote phone; incoming RTP reaches selected speaker via PipeWire without clipping or jitter. | Blocked: Awaiting H2 call establishment. |
| **H4** | **Incoming Call** | **BLOCKED** | Inbound INVITE from another FRITZ!Box phone triggers KDE notification; answered via Kirigami GUI with 200 OK + SDP; audio flows bidirectionally. | Blocked: Awaiting H1 registration. |
| **H5** | **DTMF Transmission** | **BLOCKED** | In-call keypad presses generate RFC 4733 telephony event packets (payload 101) received correctly by remote endpoint or FRITZ!Box answering machine. | Blocked: Awaiting H2 call establishment. |
| **H6** | **Failure Recovery** | **BLOCKED** | Incorrect credentials trigger 401 without looping; network drop triggers backoff and re-registration; daemon restart recovers accounts from DB and Secret Service. | Blocked: Awaiting live router credentials. |
| **H7** | **Deployment & Systemd** | **PASSED** | Application installs via `packaging/install.sh`; systemd user service `plasma-sipd.service` starts and sandboxes cleanly; D-Bus ping returns `pong`; devices enumerate. | **PASSED**: Installed to `~/.local`; `systemctl --user start plasma-sipd.service` active (PID 2424650, 3.4M RAM); `plasma-sip ping` returned `pong`. |

---

## 3. Evidence Log: Test H7 (Deployment & Runtime Services)

```text
=== Plasma SIP Installer ===
Mode:   user
Prefix: /home/user/.local

Installing binaries to /home/user/.local/bin...
Installing desktop application entry to /home/user/.local/share/applications...
Installing icon to /home/user/.local/share/icons/hicolor/scalable/apps...
Installing QML assets to /home/user/.local/share/plasma-sip/qml...
Installing D-Bus service activation to /home/user/.local/share/dbus-1/services...
Installing systemd user service to /home/user/.config/systemd/user...
Reloading systemd user daemon...
=== Installation complete ===

$ systemctl --user start plasma-sipd.service
$ systemctl --user status plasma-sipd.service
● plasma-sipd.service - Plasma SIP Background Telephony Daemon
     Loaded: loaded (/home/user/.config/systemd/user/plasma-sipd.service; disabled; preset: disabled)
     Active: active (running) since Wed 2026-10-21 17:31:59 CEST
   Main PID: 2424650 (plasma-sipd)
     Memory: 3.4M (peak: 4M)
     CGroup: /user.slice/user-1000.slice/user@1000.service/app.slice/plasma-sipd.service
             └─2424650 /home/user/.local/bin/plasma-sipd

$ ~/.local/bin/plasma-sip ping
Response: pong

$ ~/.local/bin/plasma-sip devices
Audio Devices:
  Inputs: PipeWire Sound Server, USB Communication Headset, Webcam Microphone, Internal Audio Analog Stereo
  Outputs: PipeWire Sound Server, USB Communication Headset, Desktop Stereo Speakers, Internal Audio Analog Stereo

$ systemctl --user stop plasma-sipd.service
Clean shutdown complete. Goodbye!
```

---

## 4. FRITZ!Box Provisioning & Secure Credential Entry Instructions

To unblock tests H1 through H6 on your **AVM FRITZ!Box 6591 Cable**:

### Step 1: Provision IP Telephone on FRITZ!Box Web UI
1. In your browser, open `http://fritz.box` (or `http://192.168.178.1`).
2. Log into your FRITZ!Box admin console.
3. In the left menu, navigate to **Telefonie** -> **Telefoniegeräte** (*Telephony Devices*).
4. Click **Neues Gerät einrichten** (*Configure New Device*).
5. Select **Telefon (mit und ohne Anrufbeantworter)** and click **Weiter**.
6. Select **LAN/WLAN (IP-Telefon)**, enter name `Plasma SIP`, and click **Weiter**.
7. The FRITZ!Box assigns an internal extension number (typically `620` or `621`). Note this extension.
8. Enter a secure password (minimum 8 characters, alphanumeric + symbols).
9. Assign the incoming and outgoing phone numbers for this extension, then click **Weiter** -> **Übernehmen**.
10. If prompted by the FRITZ!Box, confirm the operation using a physical button on the router or with a connected phone.

### Step 2: Configure Plasma SIP via Secure GUI (No Cleartext Files)
1. Launch the Plasma SIP application:
   ```bash
   plasma-sip
   ```
2. In the global navigation drawer, open **Accounts & PBX** -> **Add Account**.
3. Select the **AVM FRITZ!Box** provider profile:
   - **Account Name:** `FRITZ!Box Home`
   - **Registrar:** `fritz.box` (or `192.168.178.1`)
   - **Username / Extension:** `620` (or assigned extension)
   - **Password:** `<Enter your password>`
4. Click **Save & Connect**.
   - The password is sent over D-Bus directly to `plasma-sipd` and committed directly into the FreeDesktop Secret Service / KWallet via the `keyring` crate.
   - At no point are passwords saved in cleartext files, logs, or pasted into chat.
5. The registration status indicator on the dial pad will transition from yellow to green (**Registered**).
6. Proceed to execute test scenarios H1 through H6.
