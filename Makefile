# ==============================================================================
# Plasma SIP — Makefile for Fedora 44 RPM Packaging & System Installation
# ==============================================================================
# Follows Fedora Packaging Guidelines for Rust / KDE Plasma applications.
# Supports isolated RPM building in $(CURDIR)/build/rpm without polluting ~/rpmbuild.

SHELL := /bin/bash
.DEFAULT_GOAL := help

# Package Metadata
NAME                 := plasma-sip
VERSION              ?= $(shell awk -F ' = ' '/^version/ {gsub(/"/, "", $$2); print $$2; exit}' Cargo.toml 2>/dev/null || echo 0.1.0)
RELEASE              ?= 1
DIST                 ?= $(shell rpm --eval '%{?dist}' 2>/dev/null || echo .fc44)

# Build & Installation Directories
PREFIX               ?= /usr
EXEC_PREFIX          ?= $(PREFIX)
BINDIR               ?= $(EXEC_PREFIX)/bin
DATADIR              ?= $(PREFIX)/share
APPLICATIONS_DIR     ?= $(DATADIR)/applications
ICONS_DIR            ?= $(DATADIR)/icons/hicolor/scalable/apps
DBUS_SERVICES_DIR    ?= $(DATADIR)/dbus-1/services
SYSTEMD_USER_DIR     ?= $(PREFIX)/lib/systemd/user
SYSTEMD_PRESET_DIR   ?= $(PREFIX)/lib/systemd/user-preset
SYSCONFDIR           ?= /etc
AUTOSTART_DIR        ?= $(SYSCONFDIR)/xdg/autostart
FIREWALLD_SERVICES_DIR ?= $(PREFIX)/lib/firewalld/services
QML_INSTALL_DIR      ?= $(DATADIR)/plasma-sip/qml
DESTDIR              ?=

# RPM Build Hierarchy (Isolated in workspace build/rpm by default)
RPM_TOPDIR           ?= $(CURDIR)/build/rpm
SOURCES_DIR          := $(RPM_TOPDIR)/SOURCES
SPECS_DIR            := $(RPM_TOPDIR)/SPECS
BUILD_DIR            := $(RPM_TOPDIR)/BUILD
RPMS_DIR             := $(RPM_TOPDIR)/RPMS
SRPMS_DIR            := $(RPM_TOPDIR)/SRPMS
TARBALL              := $(NAME)-$(VERSION).tar.gz
SPEC_SRC             := packaging/rpm/plasma-sip.spec
SPEC_FILE            := $(SPECS_DIR)/$(notdir $(SPEC_SRC))

# Tooling
CARGO                ?= cargo
RPMBUILD             ?= rpmbuild
RPMLINT              ?= rpmlint
DESKTOP_VALIDATE     ?= desktop-file-validate
INSTALL              ?= install
MKDIR_P              ?= mkdir -p
RM_RF                ?= rm -rf

# ------------------------------------------------------------------------------
# User Targets
# ------------------------------------------------------------------------------

.PHONY: help
help: ## Show this help message
	@echo "=============================================================================="
	@echo "Plasma SIP — Build and Packaging Targets (Fedora 44)"
	@echo "=============================================================================="
	@echo "Package:    $(NAME) version $(VERSION)-$(RELEASE)$(DIST)"
	@echo "RPM Topdir: $(RPM_TOPDIR)"
	@echo
	@echo "Available Targets:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo
	@echo "Overridable Variables:"
	@echo "  VERSION            Package version (default: from Cargo.toml)"
	@echo "  RELEASE            Package release number (default: 1)"
	@echo "  RPM_TOPDIR         RPM build directory (default: ./build/rpm)"
	@echo "  DESTDIR            Staging root for 'make install' (default: empty)"
	@echo "  PREFIX             Installation prefix (default: /usr)"
	@echo "=============================================================================="

.PHONY: all
all: build ## Build release binaries (default build task)

.PHONY: build
build: ## Compile Rust workspace and Kirigami CXX-Qt bridge in release mode
	@echo "==> Building $(NAME) workspace in release mode..."
	$(CARGO) build --workspace --release

.PHONY: test
test: ## Run comprehensive test suite across all workspace crates
	@echo "==> Running workspace test suite..."
	$(CARGO) test --workspace

.PHONY: check
check: test ## Validate tests, desktop entry metadata, and RPM spec
	@echo "==> Validating desktop application files..."
	$(DESKTOP_VALIDATE) packaging/desktop/org.kde.plasma_sip.desktop
	$(DESKTOP_VALIDATE) packaging/autostart/org.kde.plasma_sip.desktop
	@echo "==> Validating RPM spec file..."
	@if command -v $(RPMLINT) >/dev/null 2>&1; then \
		$(RPMLINT) $(SPEC_SRC); \
	fi

# ------------------------------------------------------------------------------
# RPM Packaging Targets
# ------------------------------------------------------------------------------

.PHONY: prep-rpm
prep-rpm: ## Prepare isolated RPM build directory hierarchy
	@echo "==> Preparing RPM build tree in $(RPM_TOPDIR)..."
	$(MKDIR_P) $(SOURCES_DIR) $(SPECS_DIR) $(BUILD_DIR) $(RPMS_DIR) $(SRPMS_DIR)
	@cp -f $(SPEC_SRC) $(SPEC_FILE)

.PHONY: dist
dist: tarball ## Alias for tarball

.PHONY: tarball
tarball: prep-rpm ## Create pristine source tarball from working directory
	@echo "==> Generating source tarball $(SOURCES_DIR)/$(TARBALL)..."
	@$(RM_RF) $(RPM_TOPDIR)/$(NAME)-$(VERSION)
	@$(MKDIR_P) $(RPM_TOPDIR)/$(NAME)-$(VERSION)
	@tar --exclude-vcs --exclude='./target' --exclude='./build' --exclude='*.tar.gz' \
		-cf - . | (cd $(RPM_TOPDIR)/$(NAME)-$(VERSION) && tar -xf -)
	@tar -czf $(SOURCES_DIR)/$(TARBALL) -C $(RPM_TOPDIR) $(NAME)-$(VERSION)
	@$(RM_RF) $(RPM_TOPDIR)/$(NAME)-$(VERSION)
	@echo "==> Source tarball created: $(SOURCES_DIR)/$(TARBALL)"

.PHONY: srpm
srpm: tarball ## Build Source RPM (.src.rpm)
	@echo "==> Building Source RPM (SRPM)..."
	$(RPMBUILD) -bs \
		--define "_topdir $(RPM_TOPDIR)" \
		$(SPEC_FILE)
	@echo "==> SRPM built successfully:"
	@find $(SRPMS_DIR) -type f -name "*.src.rpm"

.PHONY: rpm
rpm: tarball ## Build binary RPM (.rpm) and debuginfo packages
	@echo "==> Building Binary RPMs for Fedora 44..."
	$(RPMBUILD) -ba \
		--define "_topdir $(RPM_TOPDIR)" \
		$(SPEC_FILE)
	@echo
	@echo "=============================================================================="
	@echo "RPM Build Complete!"
	@echo "=============================================================================="
	@echo "Binary RPMs:"
	@find $(RPMS_DIR) -type f -name "*.rpm" | while read -r f; do echo "  - $$f"; done
	@echo "Source RPM:"
	@find $(SRPMS_DIR) -type f -name "*.src.rpm" | while read -r f; do echo "  - $$f"; done
	@echo "=============================================================================="

.PHONY: lint
lint: rpm ## Run rpmlint on spec file, SRPM, and binary RPMs
	@echo "==> Running rpmlint on packaging artifacts..."
	@if command -v $(RPMLINT) >/dev/null 2>&1; then \
		$(RPMLINT) $(SPEC_SRC); \
		find $(SRPMS_DIR) -type f -name "*.src.rpm" -exec $(RPMLINT) {} +; \
		find $(RPMS_DIR) -type f -name "$(NAME)-$(VERSION)*.rpm" ! -name "*debug*" -exec $(RPMLINT) {} +; \
	else \
		echo "Warning: $(RPMLINT) not installed, skipping lint."; \
	fi

# ------------------------------------------------------------------------------
# Direct System Installation (DESTDIR-aware)
# ------------------------------------------------------------------------------

.PHONY: install
install: build ## Install compiled binaries and assets (supports DESTDIR for packaging)
	@echo "==> Installing $(NAME) to $(DESTDIR)$(PREFIX)..."
	$(MKDIR_P) $(DESTDIR)$(BINDIR)
	$(INSTALL) -Dpm 0755 target/release/plasma-sip $(DESTDIR)$(BINDIR)/plasma-sip
	$(INSTALL) -Dpm 0755 target/release/plasma-sipd $(DESTDIR)$(BINDIR)/plasma-sipd
	$(MKDIR_P) $(DESTDIR)$(APPLICATIONS_DIR)
	$(INSTALL) -Dpm 0644 packaging/desktop/org.kde.plasma_sip.desktop $(DESTDIR)$(APPLICATIONS_DIR)/org.kde.plasma_sip.desktop
	ln -sf org.kde.plasma_sip.desktop $(DESTDIR)$(APPLICATIONS_DIR)/plasma-sip.desktop
	$(MKDIR_P) $(DESTDIR)$(ICONS_DIR)
	$(INSTALL) -Dpm 0644 packaging/icons/org.kde.plasma_sip.svg $(DESTDIR)$(ICONS_DIR)/org.kde.plasma_sip.svg
	ln -sf org.kde.plasma_sip.svg $(DESTDIR)$(ICONS_DIR)/plasma-sip.svg
	@for size in 16 32 48 64 128 256; do \
		if [ -f packaging/icons/org.kde.plasma_sip_$${size}x$${size}.png ]; then \
			$(MKDIR_P) $(DESTDIR)$(DATADIR)/icons/hicolor/$${size}x$${size}/apps; \
			$(INSTALL) -Dpm 0644 packaging/icons/org.kde.plasma_sip_$${size}x$${size}.png $(DESTDIR)$(DATADIR)/icons/hicolor/$${size}x$${size}/apps/org.kde.plasma_sip.png; \
			ln -sf org.kde.plasma_sip.png $(DESTDIR)$(DATADIR)/icons/hicolor/$${size}x$${size}/apps/plasma-sip.png; \
		fi \
	done
	$(MKDIR_P) $(DESTDIR)$(QML_INSTALL_DIR)
	$(INSTALL) -Dpm 0644 qml/*.qml $(DESTDIR)$(QML_INSTALL_DIR)/
	$(MKDIR_P) $(DESTDIR)$(DBUS_SERVICES_DIR)
	$(INSTALL) -Dpm 0644 packaging/dbus/org.kde.PlasmaSip.service $(DESTDIR)$(DBUS_SERVICES_DIR)/org.kde.PlasmaSip.service
	$(MKDIR_P) $(DESTDIR)$(SYSTEMD_USER_DIR)
	$(INSTALL) -Dpm 0644 packaging/systemd/plasma-sipd.service $(DESTDIR)$(SYSTEMD_USER_DIR)/plasma-sipd.service
	$(MKDIR_P) $(DESTDIR)$(SYSTEMD_PRESET_DIR)
	$(INSTALL) -Dpm 0644 packaging/systemd/50-plasma-sip.preset $(DESTDIR)$(SYSTEMD_PRESET_DIR)/50-plasma-sip.preset
	$(MKDIR_P) $(DESTDIR)$(AUTOSTART_DIR)
	$(INSTALL) -Dpm 0644 packaging/autostart/org.kde.plasma_sip.desktop $(DESTDIR)$(AUTOSTART_DIR)/org.kde.plasma_sip.desktop
	$(MKDIR_P) $(DESTDIR)$(FIREWALLD_SERVICES_DIR)
	$(INSTALL) -Dpm 0644 packaging/firewalld/plasma-sip.xml $(DESTDIR)$(FIREWALLD_SERVICES_DIR)/plasma-sip.xml

.PHONY: uninstall
uninstall: ## Remove installed binaries and assets from $(DESTDIR)$(PREFIX)
	@echo "==> Uninstalling $(NAME) from $(DESTDIR)$(PREFIX)..."
	$(RM_RF) $(DESTDIR)$(BINDIR)/plasma-sip
	$(RM_RF) $(DESTDIR)$(BINDIR)/plasma-sipd
	$(RM_RF) $(DESTDIR)$(APPLICATIONS_DIR)/org.kde.plasma_sip.desktop
	$(RM_RF) $(DESTDIR)$(AUTOSTART_DIR)/org.kde.plasma_sip.desktop
	$(RM_RF) $(DESTDIR)$(ICONS_DIR)/org.kde.plasma_sip.svg
	$(RM_RF) $(DESTDIR)$(QML_INSTALL_DIR)
	$(RM_RF) $(DESTDIR)$(DBUS_SERVICES_DIR)/org.kde.PlasmaSip.service
	$(RM_RF) $(DESTDIR)$(SYSTEMD_USER_DIR)/plasma-sipd.service
	$(RM_RF) $(DESTDIR)$(SYSTEMD_PRESET_DIR)/50-plasma-sip.preset
	$(RM_RF) $(DESTDIR)$(FIREWALLD_SERVICES_DIR)/plasma-sip.xml

# ------------------------------------------------------------------------------
# Cleanup Targets
# ------------------------------------------------------------------------------

.PHONY: clean-rpm
clean-rpm: ## Remove RPM build artifacts and staging directories
	@echo "==> Cleaning RPM build directory ($(RPM_TOPDIR))..."
	$(RM_RF) $(RPM_TOPDIR)

.PHONY: clean
clean: clean-rpm ## Clean RPM build artifacts
	@echo "==> Cleaning build artifacts..."
	$(RM_RF) build

.PHONY: distclean
distclean: clean ## Clean all build artifacts and Cargo target directory
	@echo "==> Running cargo clean and removing all build artifacts..."
	$(CARGO) clean
	$(RM_RF) build
