LANTERN_HOME := $(HOME)/.lantern
BIN_DIR      := $(LANTERN_HOME)/bin
CONFIG_DIR   := $(LANTERN_HOME)/config
ICON_DIR     := $(LANTERN_HOME)/icons
LOG_DIR      := $(LANTERN_HOME)/log
WALL_DIR     := $(LANTERN_HOME)/wallpapers
APP_DIR      := $(HOME)/.local/share/applications

# Binary crates — binary name matches crate name
BINARIES := \
	lntrn-compositor \
	lntrn-session-manager \
	lntrn-bar \
	lntrn-desktop \
	lntrn-terminal \
	lntrn-file-manager \
	lntrn-menu \
	lntrn-notepad \
	lntrn-notifyd \
	lntrn-osd \
	lntrn-system-settings \
	lntrn-image-viewer \
	lntrn-media-player \
	lntrn-screenshot \
	lntrn-portal \
	lntrn-git \
	lntrn-calculator \
	lntrn-sysmon \
	lntrn-snapshot \
	lntrn-snapshot-gui \
	lntrn-screencopy

# Extra binaries from multi-binary crates
EXTRA_BINARIES := notify-send

.PHONY: all build install install-bins install-icons install-config \
        install-desktop install-wallpaper install-session install-portal \
        install-udev install-system fresh-install dirs clean deploy-%

all: build install
	@echo ""
	@echo "🏮 Lantern DE built and installed to $(LANTERN_HOME)"

build:
	cargo build --release

dirs:
	@mkdir -p $(BIN_DIR) $(CONFIG_DIR) $(ICON_DIR) $(LOG_DIR) $(WALL_DIR) $(APP_DIR)

# ── Binaries ─────────────────────────────────────────────────────────────────

install-bins: dirs
	@for bin in $(BINARIES) $(EXTRA_BINARIES); do \
		if [ -f target/release/$$bin ]; then \
			cp target/release/$$bin /tmp/$$bin-new && \
			mv -f /tmp/$$bin-new $(BIN_DIR)/$$bin && \
			echo "  ✓ $$bin"; \
		else \
			echo "  ✗ $$bin (not built)"; \
		fi \
	done
	@# Snapshot GUI wrapper (needs sudo for btrfs operations)
	@cp lntrn-snapshot/lntrn-snapshot-gui.sh $(BIN_DIR)/lntrn-snapshot-gui-launch
	@chmod +x $(BIN_DIR)/lntrn-snapshot-gui-launch
	@echo "  ✓ lntrn-snapshot-gui-launch (wrapper)"

# ── Icons ────────────────────────────────────────────────────────────────────

install-icons: dirs
	@cp -r icons/apps/*.svg icons/apps/*.png $(ICON_DIR)/ 2>/dev/null && \
		echo "  ✓ app icons" || true
	@cp -r icons/bar/*.svg $(ICON_DIR)/ 2>/dev/null && \
		echo "  ✓ bar icons" || true
	@mkdir -p $(ICON_DIR)/cursors && \
		cp -r icons/cursors/*.svg $(ICON_DIR)/cursors/ 2>/dev/null && \
		echo "  ✓ cursor icons" || true
	@mkdir -p $(ICON_DIR)/folders && \
		cp -r icons/folders/* $(ICON_DIR)/folders/ 2>/dev/null && \
		echo "  ✓ folder icons" || true

# ── Config (won't overwrite existing) ────────────────────────────────────────

install-config: dirs
	@if [ ! -f $(CONFIG_DIR)/lantern.toml ]; then \
		cp config/lantern.toml $(CONFIG_DIR)/lantern.toml && \
		echo "  ✓ lantern.toml (default)"; \
	else \
		echo "  · lantern.toml (kept existing)"; \
	fi

# ── Desktop entries ──────────────────────────────────────────────────────────

install-desktop: dirs
	@for f in \
		lntrn-terminal/lntrn-terminal.desktop \
		lntrn-file-manager/lntrn-file-manager.desktop \
		lntrn-image-viewer/lntrn-image-viewer.desktop \
		lntrn-media-player/org.lantern.MediaPlayer.desktop \
		lntrn-system-settings/lntrn-system-settings.desktop \
		lntrn-snapshot/lntrn-snapshot-gui.desktop \
		lntrn-calculator/lntrn-calculator.desktop \
		lntrn-notepad/lntrn-notepad.desktop \
		lntrn-sysmon/lntrn-sysmon.desktop \
		lntrn-git/lntrn-git.desktop \
	; do \
		if [ -f "$$f" ]; then \
			cp "$$f" $(APP_DIR)/ && echo "  ✓ $$(basename $$f)"; \
		fi \
	done

# ── Wallpapers (curated ship-set) ───────────────────────────────────────────
# Each file in wallpapers/ is copied to ~/.lantern/wallpapers/ unless a file
# of the same name already exists — so the user's personal additions/swaps
# are never clobbered.

install-wallpaper: dirs
	@if [ -d wallpapers ]; then \
		for f in wallpapers/*; do \
			[ -f "$$f" ] || continue; \
			name=$$(basename "$$f"); \
			if [ ! -f "$(WALL_DIR)/$$name" ]; then \
				cp "$$f" "$(WALL_DIR)/" && echo "  ✓ $$name"; \
			else \
				echo "  · $$name (kept existing)"; \
			fi; \
		done; \
	fi

# ── System-level installs (require sudo) ─────────────────────────────────────

install-session:
	@echo "Installing Wayland session entry..."
	@sudo mkdir -p /usr/share/wayland-sessions
	@sudo cp lntrn-session-manager/lantern.desktop /usr/share/wayland-sessions/lantern.desktop
	@echo "  ✓ /usr/share/wayland-sessions/lantern.desktop"

install-portal:
	@echo "Installing XDG portal config..."
	@sudo mkdir -p /usr/share/xdg-desktop-portal/portals
	@sudo cp lntrn-portal/config/lantern.portal /usr/share/xdg-desktop-portal/portals/
	@sudo cp lntrn-portal/config/lantern-portals.conf /usr/share/xdg-desktop-portal/portals/
	@sudo cp lntrn-portal/config/org.freedesktop.impl.portal.desktop.lantern.service \
		/usr/share/dbus-1/services/
	@echo "  ✓ portal config installed"

install-udev:
	@echo "Installing udev rules..."
	@sudo mkdir -p /etc/udev/rules.d
	@sudo cp system/udev/*.rules /etc/udev/rules.d/
	@sudo udevadm control --reload-rules
	@sudo udevadm trigger
	@echo "  ✓ udev rules installed (backlight + battery)"

# ── All system-level installs ───────────────────────────────────────────────

install-system: install-session install-portal install-udev
	@echo ""
	@echo "🏮 All system-level components installed"

# ── Fresh install (full setup from scratch) ─────────────────────────────────

fresh-install: build install install-system
	@echo ""; \
	echo "🔍 Detecting init system..."; \
	if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then \
		echo "  → systemd"; INIT=systemd; \
	elif command -v rc-service >/dev/null 2>&1; then \
		echo "  → OpenRC"; INIT=openrc; \
	elif command -v sv >/dev/null 2>&1; then \
		echo "  → runit"; INIT=runit; \
	elif command -v dinitctl >/dev/null 2>&1; then \
		echo "  → dinit"; INIT=dinit; \
	elif command -v s6-rc >/dev/null 2>&1; then \
		echo "  → s6"; INIT=s6; \
	else \
		echo "  → unknown — see docs/GENTOO-OPENRC.md for non-systemd setup"; INIT=unknown; \
	fi; \
	echo ""; \
	echo "🔍 Checking required packages..."; \
	if command -v pacman >/dev/null 2>&1; then \
		for pkg in pipewire wireplumber networkmanager bluez polkit xdg-desktop-portal; do \
			if pacman -Qi $$pkg >/dev/null 2>&1; then echo "  ✓ $$pkg"; \
			else echo "  ✗ $$pkg (missing — sudo pacman -S $$pkg)"; fi; \
		done; \
	elif command -v equery >/dev/null 2>&1; then \
		for pkg in media-video/pipewire media-video/wireplumber net-misc/networkmanager net-wireless/bluez sys-auth/polkit sys-apps/xdg-desktop-portal sys-auth/elogind; do \
			if equery -q list $$pkg >/dev/null 2>&1; then echo "  ✓ $$pkg"; \
			else echo "  ✗ $$pkg (missing — sudo emerge $$pkg)"; fi; \
		done; \
	else \
		echo "  · unknown package manager — install: pipewire wireplumber networkmanager bluez polkit xdg-desktop-portal (+ elogind on non-systemd)"; \
	fi; \
	echo ""; \
	echo "🔍 Checking user groups..."; \
	for grp in video input seat; do \
		if id -nG | grep -qw $$grp; then echo "  ✓ member of $$grp"; \
		else echo "  ✗ not in $$grp (fix: sudo usermod -aG $$grp $$USER)"; fi; \
	done; \
	echo ""; \
	echo "📋 Remaining setup:"; \
	case "$$INIT" in \
		systemd) \
			echo "  1. Enable services:"; \
			echo "     sudo systemctl enable --now NetworkManager"; \
			echo "     systemctl --user enable --now pipewire wireplumber" ;; \
		openrc) \
			echo "  1. Enable services (OpenRC):"; \
			echo "     sudo rc-update add elogind boot"; \
			echo "     sudo rc-update add dbus default"; \
			echo "     sudo rc-update add NetworkManager default"; \
			echo "     sudo rc-service elogind start && sudo rc-service dbus start && sudo rc-service NetworkManager start"; \
			echo "     # pipewire/wireplumber auto-launch via D-Bus on first audio access" ;; \
		runit) \
			echo "  1. Enable services (runit):"; \
			echo "     sudo ln -s /etc/sv/elogind /var/service/"; \
			echo "     sudo ln -s /etc/sv/dbus /var/service/"; \
			echo "     sudo ln -s /etc/sv/NetworkManager /var/service/" ;; \
		dinit|s6|unknown) \
			echo "  1. Enable elogind, dbus, NetworkManager via your init system" ;; \
	esac; \
	echo ""; \
	echo "  2. Add to ~/.zprofile:"; \
	echo '     if [ -z "$$WAYLAND_DISPLAY" ] && [ "$$(tty)" = "/dev/tty1" ]; then'; \
	echo '         exec $$HOME/.lantern/bin/lntrn-session-manager'; \
	echo '     fi'; \
	echo ""; \
	echo "  3. Add to ~/.zshrc:"; \
	echo '     export PATH="$$HOME/.lantern/bin:$$PATH"'; \
	echo ""; \
	echo "🏮 Lantern DE is ready!"

# ── Full install ─────────────────────────────────────────────────────────────

install: install-bins install-config install-desktop install-wallpaper
	@echo ""
	@echo "🏮 Lantern DE installed to $(LANTERN_HOME)"
	@echo ""
	@echo "Remaining steps:"
	@echo "  1. sudo make install-session install-portal"
	@echo "  2. Add to ~/.zprofile:"
	@echo '     if [ -z "$$WAYLAND_DISPLAY" ] && [ "$$(tty)" = "/dev/tty1" ]; then'
	@echo '         exec $$HOME/.lantern/bin/lntrn-session-manager'
	@echo "     fi"
	@echo "  3. Add to ~/.zshrc:"
	@echo '     export PATH="$$HOME/.lantern/bin:$$PATH"'

clean:
	cargo clean

# ── Deploy single component ──────────────────────────────────────────────────

deploy-%: dirs
	cargo build --release -p lntrn-$*
	@if [ -f target/release/lntrn-$* ]; then \
		cp target/release/lntrn-$* /tmp/lntrn-$*-new && \
		mv -f /tmp/lntrn-$*-new $(BIN_DIR)/lntrn-$* && \
		echo "  ✓ deployed lntrn-$*"; \
	fi
