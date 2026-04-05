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
	lntrn-browser \
	lntrn-git \
	lntrn-calculator \
	lntrn-sysmon \
	lntrn-snapshot \
	lntrn-snapshot-gui

# Extra binaries from multi-binary crates
EXTRA_BINARIES := lntrn-copy lntrn-paste notify-send

.PHONY: all build install install-bins install-icons install-config \
        install-desktop install-wallpaper install-session install-portal \
        dirs clean deploy-%

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
		lntrn-browser/lntrn-browser.desktop \
		lntrn-sysmon/lntrn-sysmon.desktop \
		lntrn-git/lntrn-git.desktop \
	; do \
		if [ -f "$$f" ]; then \
			cp "$$f" $(APP_DIR)/ && echo "  ✓ $$(basename $$f)"; \
		fi \
	done

# ── Default wallpaper ────────────────────────────────────────────────────────

install-wallpaper: dirs
	@if [ -f Lantern-DE_Wallpaper.jpeg ] && [ ! -f $(WALL_DIR)/Lantern-DE_Wallpaper.jpeg ]; then \
		cp Lantern-DE_Wallpaper.jpeg $(WALL_DIR)/ && echo "  ✓ default wallpaper"; \
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

# ── Full install ─────────────────────────────────────────────────────────────

install: install-bins install-icons install-config install-desktop install-wallpaper
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

deploy-%:
	cargo build --release -p lntrn-$*
	@if [ -f target/release/lntrn-$* ]; then \
		cp target/release/lntrn-$* /tmp/lntrn-$*-new && \
		mv -f /tmp/lntrn-$*-new $(BIN_DIR)/lntrn-$* && \
		echo "  ✓ deployed lntrn-$*"; \
	fi
