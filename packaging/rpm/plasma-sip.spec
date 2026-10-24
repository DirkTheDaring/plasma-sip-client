Name:           plasma-sip
Version:        0.1.0
Release:        1%{?dist}
Summary:        Pure Rust SIP soft phone for KDE Plasma

License:        GPL-3.0-or-later
URL:            https://github.com/plasma-sip/plasma-sip
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust >= 1.75
BuildRequires:  gcc-c++
BuildRequires:  qt6-qtbase-devel
BuildRequires:  qt6-qtdeclarative-devel
BuildRequires:  systemd-rpm-macros
BuildRequires:  desktop-file-utils
BuildRequires:  pkgconfig(alsa)
BuildRequires:  pkgconfig(libpipewire-0.3)

Requires:       kf6-kirigami
Requires:       qt6-qtdeclarative
Requires:       pipewire

%description
Plasma SIP is a modern, pure Rust SIP soft phone tailored specifically
for KDE Plasma 6 on Linux. It features native integration with PipeWire
audio, AVM FRITZ!Box PBX support, and generic SIP provider flexibility,
operating with a background daemon service and a Kirigami user interface.

%prep
%autosetup

%build
cargo build --release --workspace

%install
# Install binaries
install -Dpm 0755 target/release/plasma-sip %{buildroot}%{_bindir}/plasma-sip
install -Dpm 0755 target/release/plasma-sipd %{buildroot}%{_bindir}/plasma-sipd

# Install desktop entry
install -Dpm 0644 packaging/desktop/org.kde.plasma_sip.desktop %{buildroot}%{_datadir}/applications/org.kde.plasma_sip.desktop
ln -sf org.kde.plasma_sip.desktop %{buildroot}%{_datadir}/applications/plasma-sip.desktop

# Install scalable SVG icon and desktop symlink
install -Dpm 0644 packaging/icons/org.kde.plasma_sip.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/org.kde.plasma_sip.svg
ln -sf org.kde.plasma_sip.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/plasma-sip.svg

# Install raster PNG icons for standard desktop and window manager resolutions
for size in 16 32 48 64 128 256; do
    if [ -f packaging/icons/org.kde.plasma_sip_${size}x${size}.png ]; then
        install -Dpm 0644 packaging/icons/org.kde.plasma_sip_${size}x${size}.png %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/org.kde.plasma_sip.png
        ln -sf org.kde.plasma_sip.png %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/plasma-sip.png
    fi
done

# Install QML assets
mkdir -p %{buildroot}%{_datadir}/plasma-sip/qml
install -Dpm 0644 qml/*.qml %{buildroot}%{_datadir}/plasma-sip/qml/

# Install D-Bus service
install -Dpm 0644 packaging/dbus/org.kde.PlasmaSip.service %{buildroot}%{_datadir}/dbus-1/services/org.kde.PlasmaSip.service

# Install systemd user service
install -Dpm 0644 packaging/systemd/plasma-sipd.service %{buildroot}%{_userunitdir}/plasma-sipd.service

# Install systemd user preset (enables plasma-sipd.service out-of-the-box for all users)
install -Dpm 0644 packaging/systemd/50-plasma-sip.preset %{buildroot}%{_prefix}/lib/systemd/user-preset/50-plasma-sip.preset

# Install XDG autostart entry (initializes system tray icon on desktop login)
install -Dpm 0644 packaging/autostart/org.kde.plasma_sip.desktop %{buildroot}%{_sysconfdir}/xdg/autostart/org.kde.plasma_sip.desktop

# Install firewalld service definition (SIP UDP/TCP 5060)
install -Dpm 0644 packaging/firewalld/plasma-sip.xml %{buildroot}%{_prefix}/lib/firewalld/services/plasma-sip.xml

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/org.kde.plasma_sip.desktop
desktop-file-validate %{buildroot}%{_sysconfdir}/xdg/autostart/org.kde.plasma_sip.desktop

%post
%systemd_user_post plasma-sipd.service
touch --no-create %{_datadir}/icons/hicolor >/dev/null 2>&1 || :
if [ -x %{_bindir}/gtk-update-icon-cache ]; then
    %{_bindir}/gtk-update-icon-cache %{_datadir}/icons/hicolor >/dev/null 2>&1 || :
fi
if [ -x %{_bindir}/update-desktop-database ]; then
    %{_bindir}/update-desktop-database %{_datadir}/applications >/dev/null 2>&1 || :
fi
if [ $1 -eq 1 ] || [ $1 -eq 2 ]; then
    if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld; then
        firewall-cmd --reload >/dev/null 2>&1 || true
        firewall-cmd --permanent --add-service=plasma-sip >/dev/null 2>&1 || true
        firewall-cmd --reload >/dev/null 2>&1 || true
    fi
fi

%preun
%systemd_user_preun plasma-sipd.service
if [ $1 -eq 0 ]; then
    if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld; then
        firewall-cmd --permanent --remove-service=plasma-sip >/dev/null 2>&1 || true
        firewall-cmd --reload >/dev/null 2>&1 || true
    fi
fi

%postun
%systemd_user_postun_with_restart plasma-sipd.service
touch --no-create %{_datadir}/icons/hicolor >/dev/null 2>&1 || :
if [ -x %{_bindir}/gtk-update-icon-cache ]; then
    %{_bindir}/gtk-update-icon-cache %{_datadir}/icons/hicolor >/dev/null 2>&1 || :
fi
if [ -x %{_bindir}/update-desktop-database ]; then
    %{_bindir}/update-desktop-database %{_datadir}/applications >/dev/null 2>&1 || :
fi

%files
%license LICENSE
%doc README.md ARCHITECTURE.md CHANGELOG.md
%{_bindir}/plasma-sip
%{_bindir}/plasma-sipd
%{_datadir}/applications/org.kde.plasma_sip.desktop
%{_datadir}/applications/plasma-sip.desktop
%{_sysconfdir}/xdg/autostart/org.kde.plasma_sip.desktop
%{_datadir}/icons/hicolor/*/apps/*
%{_datadir}/plasma-sip
%{_datadir}/dbus-1/services/org.kde.PlasmaSip.service
%{_userunitdir}/plasma-sipd.service
%{_prefix}/lib/systemd/user-preset/50-plasma-sip.preset
%{_prefix}/lib/firewalld/services/plasma-sip.xml

%changelog
* Mon Oct 19 2026 Plasma SIP Team <maintainers@plasma-sip.org> - 0.1.0-1
- Initial production-ready release of Plasma SIP softphone.
