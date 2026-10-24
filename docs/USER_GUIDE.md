# Plasma SIP — User Guide

Welcome to **Plasma SIP**, a modern, pure Rust SIP softphone built natively for KDE Plasma 6 on Linux.

---

## 1. Getting Started

### Installation
If installed via package manager or the included installer:
```bash
# User-level installation (installs to ~/.local/bin and ~/.config/systemd/user)
./packaging/install.sh
```

### Launching the Application
You can start Plasma SIP from the KDE Application Launcher under **Applications -> Internet -> Plasma SIP**, or by launching from the command line:
```bash
plasma-sip
```

The background telephony service (`plasma-sipd`) starts automatically via systemd or D-Bus activation, allowing you to receive incoming calls even when the main graphical window is closed.

---

## 2. Setting Up Your SIP Account

When you launch Plasma SIP for the first time, the **Account Setup Wizard** opens automatically.

### Option A: AVM FRITZ!Box Setup (Recommended for Home PBX)

1. Open your router administration interface in your browser: `http://fritz.box`.
2. Navigate to **Telephony** -> **Telephony Devices** (*Telefoniegeräte*).
3. Click **Configure New Device** (*Neues Gerät einrichten*).
4. Choose **Telephone (with and without answering machine)** and click Next.
5. Choose **LAN/WLAN (IP telephone)** and give it a name (e.g. `PlasmaSIP`).
6. Set the username (e.g. `620`) and a secure password (at least 8 characters).
7. Assign which external telephone numbers should ring this IP phone and which number it should use for outgoing calls.
8. In Plasma SIP, select **AVM FRITZ!Box** in the Account Wizard:
   - **FRITZ!Box Hostname / IP:** `fritz.box` (or `192.168.178.1`)
   - **Port:** `5060`
   - **Username:** `620` (or the extension assigned by your router)
   - **Password:** `<the password you created in step 6>`
   - **Display Name (Optional):** `Desktop Phone`
9. Click **Connect & Register**.
10. The status bar will show **Registered** with a green badge once authenticated.

### Option B: Generic SIP Provider / Asterisk Setup

1. In the Account Wizard, select **Generic SIP Provider**.
2. Enter:
   - **Display Name:** e.g. `Work SIP`
   - **Username / SIP ID:** e.g. `1001` or `my_sip_username`
   - **Domain / Registrar:** e.g. `sip.example.com` or `192.168.1.100`
   - **Port:** `5060` (UDP)
   - **Password:** Your VoIP account password
3. Click **Connect & Register**.

> **Note on Security:** Your SIP passwords are never stored in plaintext. They are encrypted and securely stored using KDE's **KWallet** via the Linux Secret Service API.

---

## 3. Making and Receiving Calls

### Dialing a Call
1. Click on the **Dial Pad** tab on the left navigation bar.
2. Enter the phone number or SIP extension using either:
   - Your keyboard (numbers `0-9`, `*`, `#`)
   - Clicking the on-screen numeric dial pad buttons
3. Examples of valid numbers:
   - **FRITZ!Box Internal Extension:** `**621` or `**1`
   - **Standard National Number:** `030 23125000`, `0172 9925904`, or `0172 9968532` (official Bundesnetzagentur drama numbers)
   - **International E.164 Number:** `+49 30 23125000`, `+49 172 9925904`, or `+49 172 9968532`
   *(Notice: These format examples cite official [Bundesnetzagentur Drama Numbers](https://www.bundesnetzagentur.de/DE/Fachthemen/Telekommunikation/Nummerierung/DramaNumbers/artikel.html) reserved for media and documentation).*
4. Click the green **Call** button (or press `Enter`).

### Incoming Calls
When someone calls your number:
- A KDE Plasma desktop notification appears with the caller ID.
- An incoming call banner pops up in the application window.
- Click **Answer** (green phone) to accept the call.
- Click **Decline** (red phone) to send a busy/reject signal.

### Active Call Controls
During a call, the active call screen provides:
- **Remote Party Information:** Displays number and display name.
- **Call Timer:** Live duration of the active call.
- **Mute Microphone:** Toggles local microphone input on and off.
- **In-Call Keypad (DTMF):** Send touch-tones (RFC 4733) for automated telephone menus (IVRs) and voicemail prompts.
- **End Call:** Hangs up the call and terminates the media session immediately.

---

## 4. Audio Settings & Device Selection

Navigate to **Audio Settings** in the left sidebar to:
- Choose your preferred **Microphone (Input Device)**.
- Choose your preferred **Speaker / Headset (Output Device)**.
- Test audio devices with a synthetic test tone.
- Plasma SIP integrates natively with **PipeWire** and **PulseAudio**, automatically switching playback routing when a USB headset is plugged in.

---

## 5. Call History

Navigate to the **History** tab:
- Review past **Incoming**, **Outgoing**, and **Missed** calls with timestamps and call duration.
- Click on any contact or phone number to redial instantly.
- Click **Clear History** to purge all stored call records.

---

## 6. Background Telephony Daemon (`plasma-sipd`)

Plasma SIP runs a lightweight background daemon so you never miss a call.
To manage the service using systemd:
```bash
# Check status
systemctl --user status plasma-sipd.service

# Stop service
systemctl --user stop plasma-sipd.service

# View daemon logs
journalctl --user -u plasma-sipd.service -f
```

### Controlling Plasma SIP via D-Bus CLI
You can control the softphone directly from scripts or the terminal using `busctl` or `qdbus`:
```bash
# Check softphone registration status
busctl --user call org.kde.PlasmaSip /org/kde/PlasmaSip org.kde.PlasmaSip GetRegistrationStatus

# Place an outgoing call
busctl --user call org.kde.PlasmaSip /org/kde/PlasmaSip org.kde.PlasmaSip StartCall s "**620"

# Hang up current call
busctl --user call org.kde.PlasmaSip /org/kde/PlasmaSip org.kde.PlasmaSip HangupCall

# Mute microphone
busctl --user call org.kde.PlasmaSip /org/kde/PlasmaSip org.kde.PlasmaSip SetMuted b true
```
