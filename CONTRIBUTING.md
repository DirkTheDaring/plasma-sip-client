# Contributing to Plasma SIP

Thank you for your interest in contributing to **Plasma SIP**, a 100% pure Rust SIP softphone tailored specifically for KDE Plasma 6.

We welcome bug reports, feature requests, documentation improvements, and code contributions.

---

## 1. Development Environment Prerequisites

Plasma SIP is written in Rust and integrates with KDE Plasma 6 (Kirigami), PipeWire / ALSA audio, D-Bus, and the system Secret Service.

### Fedora / RHEL / CentOS Stream
```bash
sudo dnf install -y \
    cargo rust gcc-c++ \
    qt6-qtbase-devel qt6-qtdeclarative-devel \
    kf6-kirigami-devel pipewire-devel alsa-lib-devel \
    desktop-file-utils systemd-rpm-macros rpmlint
```

### Ubuntu / Debian
```bash
sudo apt-get install -y \
    cargo rustc g++ pkg-config \
    qt6-base-dev qt6-declarative-dev \
    libpipewire-0.3-dev libasound2-dev \
    desktop-file-utils
```

### Arch Linux
```bash
sudo pacman -S --needed \
    rust gcc pkgconf \
    qt6-base qt6-declarative kirigami \
    pipewire alsa-lib desktop-file-utils
```

---

## 2. Building & Running Locally

Clone the repository and build the workspace:
```bash
# Build debug binaries
cargo build --workspace

# Build release binaries
cargo build --release --workspace

# Install user services and desktop files to ~/.local
./packaging/install.sh

# Run the Kirigami UI
plasma-sip

# Run the telephony background daemon directly
plasma-sipd
```

---

## 3. Mandatory Quality Gates

Before submitting any code, your changes must pass the full workspace verification suite:

```bash
# Run all quality checks (formatting, linter, tests, desktop validation, rpmlint)
make check
```

Or step by step:
```bash
# 1. Code formatting
cargo fmt --all -- --check

# 2. Compiler & linter warnings (must have 0 warnings)
cargo clippy --workspace --all-targets -- -D warnings

# 3. Full test suite (all crates and integration tests)
cargo test --workspace --all-targets

# 4. Freedesktop validation
desktop-file-validate packaging/desktop/org.kde.plasma_sip.desktop
desktop-file-validate packaging/autostart/org.kde.plasma_sip.desktop
```

---

## 4. Architectural Rules & Guidelines

When contributing code, please adhere to these core architectural guidelines:

1. **Zero Legacy C/C++ SIP Dependencies:**
   All SIP signaling, parsing, SDP handling, digest authentication, and RTP streaming must remain in pure Rust.
2. **Strict Non-Root Policy:**
   Plasma SIP is designed strictly for user-session operation. Root privileges are prohibited (`assert_not_running_as_root()`).
3. **Zero Plaintext Secrets on Disk:**
   Passwords must never be saved in cleartext SQLite or configuration files. Credentials must be stored exclusively via the FreeDesktop Secret Service / KWallet.
4. **Memory Zeroization:**
   In-memory passwords must be wrapped in `zeroize::Zeroizing<String>` so that secret bytes are scrubbed on drop.
5. **Decoupled Architecture:**
   The UI (`plasma-sip`) must remain decoupled from the telephony engine. All state modifications and call actions must flow via D-Bus (`org.kde.PlasmaSip`) to `plasma-sipd`.
6. **No Real PII or Credentials in Tests:**
   Never commit real phone numbers, credentials, or personal names in tests or docs. Use standard RFC 5737 / example URIs (e.g. `sip:alice@example.com` or `sip:01729925904@fritz.box`).

---

## 5. Submitting Pull Requests

1. Fork the repository and create a feature branch (`git checkout -b feature/my-new-feature`).
2. Commit your changes with clear, descriptive commit messages following Conventional Commits (e.g. `feat(audio): add opus codec support`, `fix(qml): resolve dial pad layout on mobile`).
3. Ensure `make check` passes cleanly without warnings.
4. Push your branch to GitHub and open a Pull Request.
5. Provide a summary of the change and link any related issues using the PR template.
