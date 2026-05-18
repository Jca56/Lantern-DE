# Installing Lantern DE on Gentoo (OpenRC)

Lantern DE works on Gentoo with **OpenRC + elogind** (no systemd). The
compositor uses `libseat` (works against either logind impl). Power
actions auto-route at runtime: `loginctl` on elogind hosts,
`systemctl` on systemd hosts — detected by the presence of
`/run/systemd/system` (the canonical `sd_booted(3)` check). The
portable-by-design areas are listed at the bottom of this file.

## 1. Packages

```bash
sudo emerge -av \
  sys-auth/elogind \
  sys-auth/polkit \
  sys-apps/dbus \
  media-video/pipewire \
  media-video/wireplumber \
  net-misc/networkmanager \
  net-wireless/bluez \
  sys-apps/xdg-desktop-portal \
  x11-base/xwayland \
  dev-libs/libinput \
  media-libs/mesa
```

### Critical USE flags

In `/etc/portage/make.conf`:

```
USE="${USE} -systemd elogind dbus pipewire policykit pulseaudio screencast wayland X"
```

Then re-merge anything that picked up changes:

```bash
sudo emerge -avuDN --changed-use --with-bdeps=y @world
```

The flags that matter most for Lantern:
- **`elogind`** — required on `sys-auth/polkit`, `net-misc/networkmanager`,
  `gnome-base/gvfs` if used, anything that wants logind D-Bus.
- **`-systemd`** — globally off.
- **`pipewire`** + **`screencast`** — for portal-backed screen capture.

### Lean PipeWire build (for `lntrn-desktop`)

`lntrn-desktop` links against `libpipewire-0.3` + `libspa-0.2` (both ship
from `media-video/pipewire`) to tap the default sink monitor for the
music-bar visualizer. It only uses the core PipeWire API, so the heavy
optional bits can stay off:

```
media-video/pipewire   dbus pulseaudio screencast sound-server -systemd \
                       -gstreamer -jack-sdk -extra
```

Skipped flags and what they would have pulled in:
- **`-gstreamer`** — drops `media-libs/gstreamer` + `gst-plugins-base`
  (GStreamer ↔ PipeWire bridge; unused by Lantern).
- **`-jack-sdk`** — drops the JACK API shim (only useful for pro-audio
  JACK clients).
- **`-extra`** — drops optional modules (RTP, AVB, raop, roc,
  echo-cancel) and the extra `pipewire-pulse` pieces.

Verify after merge:

```bash
pkg-config --modversion libpipewire-0.3 libspa-0.2
```

Both should print a version; if so, `cargo build --release -p lntrn-desktop`
will link cleanly.

## 2. Services (OpenRC)

```bash
# Boot-time
sudo rc-update add elogind boot
sudo rc-update add udev boot           # eudev or sys-fs/udev

# Default runlevel
sudo rc-update add dbus default
sudo rc-update add NetworkManager default
sudo rc-update add bluetooth default   # optional

# Start them now
sudo rc-service elogind start
sudo rc-service dbus start
sudo rc-service NetworkManager start
```

PipeWire / WirePlumber do **not** need an init script — they D-Bus
auto-activate on first audio access (the `.service` files installed
to `/usr/share/dbus-1/services/` handle this).

## 3. User groups

```bash
sudo gpasswd -a $USER video
sudo gpasswd -a $USER input
sudo gpasswd -a $USER seat        # only if you use sys-auth/seatd instead of elogind
sudo gpasswd -a $USER plugdev     # USB device access
```

Log out + back in for group changes to take effect.

## 4. Build & install Lantern

```bash
cd ~/Projects/Lantern-DE
make fresh-install
```

`make fresh-install` will auto-detect OpenRC and print the right
service commands at the end — the steps above are mostly to get you
to the point where `make install-system` (which uses sudo) can drop
files into `/usr/share/wayland-sessions/`, the portal config dir,
and `/etc/udev/rules.d/`.

## 5. Shell autostart

Add to `~/.zprofile` (or `~/.bash_profile` if bash):

```sh
if [ -z "$WAYLAND_DISPLAY" ] && [ "$(tty)" = "/dev/tty1" ]; then
    exec $HOME/.lantern/bin/lntrn-session-manager
fi
```

And `~/.zshrc` (or `~/.bashrc`):

```sh
export PATH="$HOME/.lantern/bin:$PATH"
```

The session manager spawns the compositor, XWayland, and pushes env
vars into D-Bus activation env. It does **not** depend on a systemd
user manager — `systemctl --user` calls inside it are best-effort and
silently no-op on OpenRC.

## 6. Monitor config

Edit `~/.lantern/config/lantern.toml` and add a `[[monitors]]` block
matching your connector name (find it in
`~/.lantern/log/compositor.log` on first launch, or via
`ls /sys/class/drm/card*-*/`):

```toml
[[monitors]]
name = "HDMI-A-1"      # desktop monitors typically; laptops use "eDP-1"
x = 0
y = 0
resolution = "2560x1440"
refresh_rate = "144000"  # millihertz — 144000 = 144 Hz
scale = 1.0
wallpaper = "/home/yourname/.lantern/wallpapers/Lantern-DE_Wallpaper.jpeg"
```

## What's portable by design

| Subsystem | Mechanism | Works on systemd? | Works on OpenRC? |
|-----------|-----------|-------------------|------------------|
| Seat / DRM master | `libseat` (Smithay `backend_session_libseat`) | ✓ (logind) | ✓ (elogind) |
| Power actions (suspend / reboot / poweroff / hibernate) | runtime branch: `systemctl` on systemd, `loginctl` on elogind | ✓ | ✓ |
| Lock session | `loginctl lock-session` (universal) | ✓ | ✓ |
| D-Bus activation env | `dbus-update-activation-environment` (no `--systemd` flag) | ✓ | ✓ |
| XDG portal re-spawn | `pkill xdg-desktop-portal` (D-Bus re-activates) | ✓ | ✓ |
| Env propagation to user manager | `systemctl --user import-environment` (best-effort) | ✓ | no-op |
| udev rules | `udevadm control --reload-rules` | ✓ | ✓ (eudev/udev) |

## Known papercuts

- The `systemctl --user import-environment` calls in
  `lntrn-session-manager/src/main.rs` silently fail on OpenRC. This
  only matters if you're running additional services as systemd user
  units that need Lantern's env — on OpenRC there are no such
  services, so the no-ops are harmless.
- `xdg-desktop-portal` is sometimes built with a hard `systemd` USE
  flag dependency in old ebuilds. Make sure your `xdg-desktop-portal`
  was emerged with `-systemd elogind` USE.
- If `loginctl suspend` doesn't put the machine to sleep, check
  `/etc/elogind/logind.conf` — the elogind defaults may differ from
  systemd-logind. `HandleLidSwitch=suspend` and
  `IdleAction=ignore` are good starting values.
