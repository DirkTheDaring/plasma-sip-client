#!/usr/bin/env bash
# Plasma SIP — Installation Script
# Installs plasma-sip (desktop UI) and plasma-sipd (background service)

set -euo pipefail

MODE="user"
PREFIX="${PREFIX:-${HOME}/.local}"

NO_BUILD=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --system)
            MODE="system"
            PREFIX="/usr/local"
            shift
            ;;
        --prefix)
            PREFIX="$2"
            shift 2
            ;;
        --no-build)
            NO_BUILD=true
            shift
            ;;
        *)
            shift
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "=== Plasma SIP Installer ==="
echo "Mode:   ${MODE}"
echo "Prefix: ${PREFIX}"
echo

# 1. Check or build binaries (prefer release, fallback to debug if already compiled)
BIN_SIP="${ROOT_DIR}/target/release/plasma-sip"
BIN_SIPD="${ROOT_DIR}/target/release/plasma-sipd"

if [ ! -f "${BIN_SIP}" ] || [ ! -f "${BIN_SIPD}" ]; then
    if [ -f "${ROOT_DIR}/target/debug/plasma-sip" ] && [ -f "${ROOT_DIR}/target/debug/plasma-sipd" ]; then
        BIN_SIP="${ROOT_DIR}/target/debug/plasma-sip"
        BIN_SIPD="${ROOT_DIR}/target/debug/plasma-sipd"
    fi
fi

if [ "${NO_BUILD}" = false ] && ([ ! -f "${BIN_SIP}" ] || [ ! -f "${BIN_SIPD}" ]); then
    echo "Building workspace in release mode..."
    cargo build --workspace --release --manifest-path "${ROOT_DIR}/Cargo.toml"
    BIN_SIP="${ROOT_DIR}/target/release/plasma-sip"
    BIN_SIPD="${ROOT_DIR}/target/release/plasma-sipd"
elif [ ! -f "${BIN_SIP}" ] || [ ! -f "${BIN_SIPD}" ]; then
    echo "ERROR: --no-build was specified, but binaries were not found in ${ROOT_DIR}/target/release or ${ROOT_DIR}/target/debug." >&2
    exit 1
fi

# 2. Determine target directories
BIN_DIR="${PREFIX}/bin"
APP_DIR="${PREFIX}/share/applications"
ICON_DIR="${PREFIX}/share/icons/hicolor/scalable/apps"
DBUS_DIR="${PREFIX}/share/dbus-1/services"

if [ "${MODE}" = "system" ]; then
    SYSTEMD_DIR="${PREFIX}/lib/systemd/user"
else
    if [ "${PREFIX}" = "${HOME}/.local" ]; then
        SYSTEMD_DIR="${HOME}/.config/systemd/user"
    else
        SYSTEMD_DIR="${PREFIX}/lib/systemd/user"
    fi
fi

echo "Installing binaries to ${BIN_DIR}..."
mkdir -p "${BIN_DIR}"
install -m 755 "${BIN_SIP}" "${BIN_DIR}/plasma-sip"
install -m 755 "${BIN_SIPD}" "${BIN_DIR}/plasma-sipd"

echo "Installing desktop application entry to ${APP_DIR}..."
mkdir -p "${APP_DIR}"
install -m 644 "${SCRIPT_DIR}/desktop/org.kde.plasma_sip.desktop" "${APP_DIR}/org.kde.plasma_sip.desktop"

echo "Installing icon to ${ICON_DIR}..."
mkdir -p "${ICON_DIR}"
install -m 644 "${SCRIPT_DIR}/icons/org.kde.plasma_sip.svg" "${ICON_DIR}/org.kde.plasma_sip.svg"

QML_DIR="${PREFIX}/share/plasma-sip/qml"
echo "Installing QML assets to ${QML_DIR}..."
mkdir -p "${QML_DIR}"
install -m 644 "${ROOT_DIR}/qml/"*.qml "${QML_DIR}/"

echo "Installing D-Bus service activation to ${DBUS_DIR}..."
mkdir -p "${DBUS_DIR}"
# Adjust Exec path if user install
sed "s|/usr/bin/plasma-sipd|${BIN_DIR}/plasma-sipd|g" "${SCRIPT_DIR}/dbus/org.kde.PlasmaSip.service" > "${DBUS_DIR}/org.kde.PlasmaSip.service"
chmod 644 "${DBUS_DIR}/org.kde.PlasmaSip.service"

echo "Installing systemd user service to ${SYSTEMD_DIR}..."
mkdir -p "${SYSTEMD_DIR}"
# Adjust ExecStart path if user install
sed "s|/usr/bin/plasma-sipd|${BIN_DIR}/plasma-sipd|g" "${SCRIPT_DIR}/systemd/plasma-sipd.service" > "${SYSTEMD_DIR}/plasma-sipd.service"
chmod 644 "${SYSTEMD_DIR}/plasma-sipd.service"

if [ "${MODE}" = "system" ]; then
    AUTOSTART_DIR="/etc/xdg/autostart"
else
    if [ "${PREFIX}" = "${HOME}/.local" ]; then
        AUTOSTART_DIR="${HOME}/.config/autostart"
    else
        AUTOSTART_DIR="${PREFIX}/etc/xdg/autostart"
    fi
fi
echo "Installing desktop autostart entry to ${AUTOSTART_DIR}..."
mkdir -p "${AUTOSTART_DIR}"
sed "s|plasma-sip --minimized|${BIN_DIR}/plasma-sip --minimized|g" "${SCRIPT_DIR}/autostart/org.kde.plasma_sip.desktop" > "${AUTOSTART_DIR}/org.kde.plasma_sip.desktop"
if [ "${MODE}" = "system" ]; then
    FIREWALLD_DIR="${PREFIX}/lib/firewalld/services"
    echo "Installing firewalld service to ${FIREWALLD_DIR}..."
    mkdir -p "${FIREWALLD_DIR}"
    install -m 644 "${SCRIPT_DIR}/firewalld/plasma-sip.xml" "${FIREWALLD_DIR}/plasma-sip.xml"
    if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld; then
        firewall-cmd --reload >/dev/null 2>&1 || true
        firewall-cmd --permanent --add-service=plasma-sip >/dev/null 2>&1 || true
        firewall-cmd --reload >/dev/null 2>&1 || true
    fi
fi

SYSTEMCTL="${SYSTEMCTL:-systemctl}"

IS_STAGED=false
if [ -n "${PLASMA_SIP_STAGING:-}" ]; then
    IS_STAGED=true
elif [ "${MODE}" = "user" ] && [ "${PREFIX}" != "${HOME}/.local" ]; then
    IS_STAGED=true
elif [ "${MODE}" = "system" ] && [ "${PREFIX}" != "/usr" ] && [ "${PREFIX}" != "/usr/local" ]; then
    IS_STAGED=true
fi

# 3. Post-install caches and reload
if command -v "${SYSTEMCTL}" >/dev/null 2>&1; then
    if [ "${IS_STAGED}" = false ] || [ -n "${SYSTEMCTL_RECORD:-}" ]; then
        echo "Reloading systemd user daemon..."
        "${SYSTEMCTL}" --user daemon-reload || true
        if [ "${MODE}" = "user" ] && { [ "${PREFIX}" = "${HOME}/.local" ] || [ -n "${SYSTEMCTL_RECORD:-}" ]; }; then
            echo "Enabling and starting plasma-sipd.service..."
            "${SYSTEMCTL}" --user enable --now plasma-sipd.service || true
        fi
    else
        echo "Staged installation detected; skipping host systemd daemon-reload."
    fi
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APP_DIR}" || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -d "${PREFIX}/share/icons/hicolor" ]; then
    gtk-update-icon-cache -q -f -t "${PREFIX}/share/icons/hicolor" 2>/dev/null || true
fi

if command -v kbuildsycoca6 >/dev/null 2>&1 && [ "${MODE}" = "user" ] && [ "${PREFIX}" = "${HOME}/.local" ]; then
    kbuildsycoca6 --noincremental 2>/dev/null || true
fi

echo
echo "=== Installation complete ==="
if [ "${MODE}" = "user" ] && [ "${PREFIX}" = "${HOME}/.local" ]; then
    echo "Plasma SIP telephony daemon is active and running in the background."
    echo "Launch the Kirigami UI directly:"
    echo "    ${BIN_DIR}/plasma-sip"
    echo "Or open 'Plasma SIP' from the KDE Plasma application menu."
else
    echo "To start the background daemon:"
    echo "    systemctl --user enable --now plasma-sipd.service"
    echo "To launch the UI:"
    echo "    ${BIN_DIR}/plasma-sip"
fi
