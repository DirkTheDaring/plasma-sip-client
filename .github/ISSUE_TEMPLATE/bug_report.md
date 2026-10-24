---
name: Bug Report
about: Create a report to help us improve Plasma SIP
title: '[BUG] '
labels: 'bug'
assignees: ''

---

**Describe the Bug**
A clear and concise description of what the bug is.

**Environment Information:**
- OS: [e.g. Fedora 44, Arch Linux, Ubuntu 24.04]
- Desktop Environment: [e.g. KDE Plasma 6.2 (Wayland / X11)]
- Plasma SIP Version: [e.g. 0.1.0 (or git commit hash)]
- SIP Provider / PBX: [e.g. AVM FRITZ!Box 7590, Asterisk 20, sipgate]
- Audio Server: [e.g. PipeWire 1.0, PulseAudio]

**To Reproduce**
Steps to reproduce the behavior:
1. Go to '...'
2. Click on '....'
3. Scroll down to '....'
4. See error

**Expected Behavior**
A clear and concise description of what you expected to happen.

**Logs & Diagnostic Output**
If applicable, run the background daemon with trace logging and include the relevant output:
```bash
RUST_LOG=debug plasma-sipd
```
*(Please ensure any private SIP passwords or personal phone numbers are redacted before posting logs)*

**Additional Context**
Add any other context about the problem here (e.g. audio device model, network configuration).
