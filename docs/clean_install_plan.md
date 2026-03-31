# Lantern DE Clean Install Plan

## Part 1: Minimal Arch Install

### Base Packages (~40 packages instead of 756)

```bash
# Core system
base base-devel linux linux-firmware intel-ucode

# Filesystem
btrfs-progs dosfstools  # adjust if not using btrfs

# Boot
grub efibootmgr  # or systemd-boot if preferred

# Network
networkmanager iwd

# Shell & terminal
zsh

# Build tools
rustup git

# Wayland + GPU
mesa vulkan-intel libinput wayland wayland-protocols

# Audio
pipewire pipewire-pulse wireplumber

# Fonts
ttf-dejavu noto-fonts

# Lantern build dependencies (libraries we link against)
libxkbcommon libinput dbus

# Misc tools you'll probably want
unzip wget man-db
```

### Post-Install Steps

```bash
# 1. Set timezone & locale
ln -sf /usr/share/zoneinfo/YOUR_TIMEZONE /etc/localtime
hwclock --systohc
# Edit /etc/locale.gen, uncomment en_US.UTF-8
locale-gen

# 2. Enable services
systemctl enable NetworkManager

# 3. Create user
useradd -m -G wheel,video,input -s /bin/zsh alva
passwd alva

# 4. Setup sudo
# Uncomment %wheel ALL=(ALL:ALL) ALL in /etc/sudoers via visudo

# 5. Install Rust
rustup default stable

# 6. TTY auto-login (no display manager needed)
sudo mkdir -p /etc/systemd/system/getty@tty1.service.d
sudo tee /etc/systemd/system/getty@tty1.service.d/autologin.conf << 'EOF'
[Service]
ExecStart=
ExecStart=-/usr/bin/agetty --autologin alva --noclear %I $TERM
EOF

# 7. Auto-start compositor from zsh profile
# Add to ~/.zprofile:
if [ -z "$WAYLAND_DISPLAY" ] && [ "$(tty)" = "/dev/tty1" ]; then
    exec ~/.lantern/bin/lntrn-compositor
fi
```

---

## Part 2: ~/.lantern/ Directory Structure

```
~/.lantern/
├── bin/                    # all Lantern binaries
│   ├── lntrn-compositor
│   ├── lntrn-bar
│   ├── lntrn-terminal
│   ├── lntrn-file-manager
│   ├── lntrn-menu
│   ├── lntrn-notepad
│   ├── lntrn-notifyd
│   ├── lntrn-osd
│   ├── lntrn-system-settings
│   ├── lntrn-image-viewer
│   ├── lntrn-media-player
│   ├── lntrn-screenshot
│   ├── lntrn-snapshot
│   ├── lntrn-snapshot-gui
│   ├── lntrn-session-manager
│   ├── lntrn-browser
│   ├── lntrn-portal
│   ├── lntrn-git
│   ├── lntrn-copy
│   └── lntrn-paste
├── config/                 # all config files
│   ├── compositor.toml
│   ├── bar.toml
│   ├── terminal.toml
│   ├── theme.toml
│   └── keybinds.toml
├── icons/                  # Lantern icon set
│   ├── apps/
│   ├── actions/
│   └── status/
├── wallpapers/             # wallpaper storage
└── themes/                 # color theme presets
    ├── dark.toml
    └── light.toml
```

### PATH Setup

Add to `~/.zshrc`:
```bash
export PATH="$HOME/.lantern/bin:$PATH"
```

---

## Part 3: Makefile

The Makefile lives at the repo root (`Lantern-DE/Makefile`).

```makefile
LANTERN_HOME := $(HOME)/.lantern
BIN_DIR      := $(LANTERN_HOME)/bin
CONFIG_DIR   := $(LANTERN_HOME)/config
ICON_DIR     := $(LANTERN_HOME)/icons
WALL_DIR     := $(LANTERN_HOME)/wallpapers
THEME_DIR    := $(LANTERN_HOME)/themes

# All binary crates to build and install
BINARIES := \
    lntrn-compositor \
    lntrn-bar \
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
    lntrn-session-manager \
    lntrn-browser \
    lntrn-portal \
    lntrn-git

# Crates that produce binaries with different names
# lntrn-clipboard -> lntrn-copy, lntrn-paste
# lntrn-snapshot  -> lntrn-snapshot, lntrn-snapshot-gui
EXTRA_BINARIES := lntrn-copy lntrn-paste lntrn-snapshot lntrn-snapshot-gui

.PHONY: all build install install-bins install-config install-icons dirs clean

all: build install

build:
	cargo build --release

dirs:
	mkdir -p $(BIN_DIR) $(CONFIG_DIR) $(ICON_DIR) $(WALL_DIR) $(THEME_DIR)

install-bins: dirs build
	@for bin in $(BINARIES) $(EXTRA_BINARIES); do \
		if [ -f target/release/$$bin ]; then \
			cp target/release/$$bin /tmp/$$bin-new && \
			mv -f /tmp/$$bin-new $(BIN_DIR)/$$bin && \
			echo "  installed $$bin"; \
		fi \
	done

# Only copy configs if they don't already exist (don't overwrite user customizations)
install-config: dirs
	@for cfg in config/*.toml; do \
		if [ -f "$$cfg" ]; then \
			dest=$(CONFIG_DIR)/$$(basename $$cfg); \
			if [ ! -f "$$dest" ]; then \
				cp "$$cfg" "$$dest" && echo "  installed $$cfg"; \
			else \
				echo "  skipped $$cfg (already exists)"; \
			fi \
		fi \
	done

install-icons: dirs
	@if [ -d icons ]; then \
		cp -r icons/* $(ICON_DIR)/ && echo "  installed icons"; \
	fi

install: install-bins install-config install-icons
	@echo ""
	@echo "Lantern DE installed to $(LANTERN_HOME)"
	@echo "Make sure ~/.lantern/bin is in your PATH:"
	@echo '  export PATH="$$HOME/.lantern/bin:$$PATH"'

clean:
	cargo clean

# Deploy a single component (usage: make deploy-bar, make deploy-compositor, etc.)
deploy-%:
	cargo build --release -p lntrn-$*
	@cp target/release/lntrn-$* /tmp/lntrn-$*-new
	@mv -f /tmp/lntrn-$*-new $(BIN_DIR)/lntrn-$*
	@echo "deployed lntrn-$* to $(BIN_DIR)"
```

### Usage

```bash
# First time: build everything and install
make

# Rebuild and deploy just one component
make deploy-bar
make deploy-compositor
make deploy-terminal

# Just rebuild, don't install
make build
```

---

## Part 4: Code Changes Needed

These are the changes needed in the Lantern DE codebase to support `~/.lantern/`:

### 1. Lantern home path constant

Add to `lntrn-theme/src/lib.rs` (or create a tiny `lntrn-paths` crate):

```rust
pub fn lantern_home() -> PathBuf {
    dirs::home_dir().unwrap().join(".lantern")
}
pub fn lantern_config() -> PathBuf {
    lantern_home().join("config")
}
pub fn lantern_icons() -> PathBuf {
    lantern_home().join("icons")
}
```

### 2. Update config loading

Every app that reads config files needs to look in `~/.lantern/config/` instead of
wherever it currently looks. Grep the codebase for config file paths and update them.

### 3. Update icon lookup

The icon panel in system-settings and any icon rendering code needs to check
`~/.lantern/icons/` as the primary icon path.

### 4. Update compositor child process spawning

The compositor spawns apps like lntrn-bar, lntrn-notifyd, etc. These spawn paths
need to either:
- Rely on PATH (if ~/.lantern/bin is in PATH, just spawn "lntrn-bar")
- Or use the full path: ~/.lantern/bin/lntrn-bar

### 5. Update deploy skills

The Claude Code deploy skills (deploy-bar, deploy-terminal, deploy-fox) need to
copy to `~/.lantern/bin/` instead of `~/.local/bin/`.

### 6. Move existing icons

Copy Tela icons we use from `/usr/share/icons/Tela/` to `~/.lantern/icons/`.

---

## Part 5: Migration Checklist

Do this AFTER the clean Arch install:

1. [ ] Install base packages (Part 1)
2. [ ] Setup TTY auto-login
3. [ ] Clone Lantern-DE repo with submodules: `git clone --recursive <url>`
4. [ ] Run `make` to build and install everything
5. [ ] Add `~/.lantern/bin` to PATH in `~/.zshrc`
6. [ ] Add compositor auto-start to `~/.zprofile`
7. [ ] Copy over wallpapers to `~/.lantern/wallpapers/`
8. [ ] Copy Tela icons to `~/.lantern/icons/`
9. [ ] Reboot and pray to the Wayland gods

---

## Notes

- **Third-party apps** (Firefox, etc.) still use XDG paths (`~/.config/`, `~/.local/`) on
  their own. We don't need to manage that.
- **No display manager needed** - TTY auto-login + .zprofile is simpler and faster.
- **The Makefile `deploy-%` target** replaces all the individual deploy scripts/skills.
- **Config files use TOML** - human readable, easy to edit, no external parser needed.
- **`install-config` won't overwrite** existing configs, so `make install` is safe to
  re-run after updates without losing customizations.
