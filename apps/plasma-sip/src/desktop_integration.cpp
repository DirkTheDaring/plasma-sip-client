#include "desktop_integration.h"

#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QPixmap>
#include <QtCore/QFile>
#include <QtCore/QString>
#include <QtCore/QByteArray>

static const char EMBEDDED_SVG_ICON[] =
R"raw(<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" width="128" height="128">
  <defs>
    <linearGradient id="bgGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#1d99f3"/>
      <stop offset="50%" stop-color="#3daee9"/>
      <stop offset="100%" stop-color="#1b6acb"/>
    </linearGradient>
    <linearGradient id="phoneGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#ffffff"/>
      <stop offset="100%" stop-color="#e0e8f5"/>
    </linearGradient>
    <linearGradient id="accentGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#2ecc71"/>
      <stop offset="100%" stop-color="#27ae60"/>
    </linearGradient>
    <filter id="dropShadow" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="4" stdDeviation="4" flood-color="#000000" flood-opacity="0.35"/>
    </filter>
  </defs>

  <!-- Background Base with subtle Breeze drop shadow -->
  <rect x="8" y="8" width="112" height="112" rx="28" fill="url(#bgGrad)" filter="url(#dropShadow)"/>

  <!-- Subtle inner border highlight -->
  <rect x="9" y="9" width="110" height="110" rx="27" fill="none" stroke="#ffffff" stroke-width="1.5" stroke-opacity="0.3"/>

  <!-- SIP Telephony / Radio Waves (Top Right) -->
  <path d="M 82 32 A 22 22 0 0 1 96 46" fill="none" stroke="#ffffff" stroke-width="3.5" stroke-linecap="round" stroke-opacity="0.65"/>
  <path d="M 76 26 A 32 32 0 0 1 102 52" fill="none" stroke="#ffffff" stroke-width="3.5" stroke-linecap="round" stroke-opacity="0.9"/>
  <path d="M 70 20 A 42 42 0 0 1 108 58" fill="none" stroke="#ffffff" stroke-width="3" stroke-linecap="round" stroke-opacity="0.4"/>

  <!-- Phone Handset Silhouette -->
  <g transform="translate(6, 6)">
    <path d="M 40 34
             C 40 34, 46 28, 52 34
             C 58 40, 54 48, 48 52
             C 46 53, 45 55, 47 58
             C 51 66, 58 73, 66 77
             C 69 79, 71 78, 72 76
             C 76 70, 84 66, 90 72
             C 96 78, 90 84, 90 84
             C 86 88, 79 92, 70 90
             C 54 86, 38 70, 34 54
             C 32 45, 36 38, 40 34 Z"
          fill="url(#phoneGrad)"
          filter="url(#dropShadow)"/>
  </g>

  <!-- Active Call / Connection Status Badge (Bottom Right) -->
  <circle cx="98" cy="98" r="14" fill="#ffffff" filter="url(#dropShadow)"/>
  <circle cx="98" cy="98" r="11" fill="url(#accentGrad)"/>
  <!-- Checkmark or small dot inside badge -->
  <circle cx="98" cy="98" r="4.5" fill="#ffffff"/>
</svg>
)raw";

extern "C" void init_plasma_sip_desktop_integration(void) {
    // 1. Set Wayland desktop file name so KWin associates window with org.kde.plasma_sip.desktop
    QGuiApplication::setDesktopFileName(QStringLiteral("org.kde.plasma_sip"));
    QGuiApplication::setApplicationName(QStringLiteral("org.kde.plasma_sip"));
    QGuiApplication::setApplicationDisplayName(QStringLiteral("Plasma SIP"));
    QGuiApplication::setOrganizationName(QStringLiteral("KDE"));
    QGuiApplication::setOrganizationDomain(QStringLiteral("kde.org"));

    // 2. Resolve application icon with multiple robust fallbacks
    QIcon icon = QIcon::fromTheme(QStringLiteral("org.kde.plasma_sip"));

    if (icon.isNull()) {
        icon = QIcon::fromTheme(QStringLiteral("plasma-sip"));
    }

    if (icon.isNull() && QFile::exists(QStringLiteral("/usr/share/icons/hicolor/scalable/apps/org.kde.plasma_sip.svg"))) {
        icon = QIcon(QStringLiteral("/usr/share/icons/hicolor/scalable/apps/org.kde.plasma_sip.svg"));
    }

    if (icon.isNull()) {
        QPixmap pixmap;
        QByteArray data(EMBEDDED_SVG_ICON);
        if (pixmap.loadFromData(data)) {
            icon = QIcon(pixmap);
        }
    }

    if (icon.isNull()) {
        icon = QIcon::fromTheme(QStringLiteral("call-start"));
    }

    if (!icon.isNull()) {
        QGuiApplication::setWindowIcon(icon);
    }

    // 3. Keep application running in background tray even when window is minimized or closed
    QGuiApplication::setQuitOnLastWindowClosed(false);
}
