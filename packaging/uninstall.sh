#!/usr/bin/env bash
# Plasma SIP — Uninstallation Script
# Removes plasma-sip and plasma-sipd from the local user environment or system

set -euo pipefail

MODE="user"
PREFIX="${PREFIX:-${HOME}/.local}"
PURGE=false

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
        --purge)
            PURGE=true
            shift
            ;;
        *)
            shift
            ;;
    esac
done

echo "=== Plasma SIP Uninstaller ==="
echo "Mode:   ${MODE}"
echo "Prefix: ${PREFIX}"
echo "Purge:  ${PURGE}"
echo

SYSTEMCTL="${SYSTEMCTL:-systemctl}"

IS_STAGED=false
if [ -n "${PLASMA_SIP_STAGING:-}" ]; then
    IS_STAGED=true
elif [ "${MODE}" = "user" ] && [ "${PREFIX}" != "${HOME}/.local" ]; then
    IS_STAGED=true
elif [ "${MODE}" = "system" ] && [ "${PREFIX}" != "/usr" ] && [ "${PREFIX}" != "/usr/local" ]; then
    IS_STAGED=true
fi

if command -v "${SYSTEMCTL}" >/dev/null 2>&1; then
    if [ "${IS_STAGED}" = false ] || [ -n "${SYSTEMCTL_RECORD:-}" ]; then
        if [ "${PREFIX}" = "${HOME}/.local" ] || [ -n "${SYSTEMCTL_RECORD:-}" ]; then
            echo "Stopping and disabling systemd user service..."
            "${SYSTEMCTL}" --user stop plasma-sipd.service || true
            "${SYSTEMCTL}" --user disable plasma-sipd.service || true
        fi
    fi
fi

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

echo "Removing installed files..."
rm -f "${BIN_DIR}/plasma-sip"
rm -f "${BIN_DIR}/plasma-sipd"
rm -f "${APP_DIR}/org.kde.plasma_sip.desktop"
rm -f "${ICON_DIR}/org.kde.plasma_sip.svg"
rm -rf "${PREFIX}/share/plasma-sip/qml"
rmdir "${PREFIX}/share/plasma-sip" 2>/dev/null || true
rm -f "${DBUS_DIR}/org.kde.PlasmaSip.service"
rm -f "${SYSTEMD_DIR}/plasma-sipd.service"
if [ "${MODE}" = "system" ]; then
    rm -f "/etc/xdg/autostart/org.kde.plasma_sip.desktop"
elif [ "${PREFIX}" = "${HOME}/.local" ]; then
    rm -f "${HOME}/.config/autostart/org.kde.plasma_sip.desktop"
else
    rm -f "${PREFIX}/etc/xdg/autostart/org.kde.plasma_sip.desktop"
fi

if [ "${MODE}" = "system" ]; then
    rm -f "${PREFIX}/lib/firewalld/services/plasma-sip.xml"
    if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld; then
        firewall-cmd --permanent --remove-service=plasma-sip >/dev/null 2>&1 || true
        firewall-cmd --reload >/dev/null 2>&1 || true
    fi
fi

if [ "$PURGE" = true ]; then
    echo "Purging configuration and database..."
    if [ "${IS_STAGED}" = false ]; then
        rm -rf "${HOME}/.local/share/plasma-sip"
    else
        rm -rf "${PREFIX}/share/plasma-sip"
    fi
fi

if command -v "${SYSTEMCTL}" >/dev/null 2>&1; then
    if [ "${IS_STAGED}" = false ] || [ -n "${SYSTEMCTL_RECORD:-}" ]; then
        "${SYSTEMCTL}" --user daemon-reload || true
    else
        echo "Staged uninstallation detected; skipping host systemd daemon-reload."
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

echo "=== Uninstallation complete ==="
