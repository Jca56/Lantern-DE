#!/bin/sh
exec sudo --preserve-env=WAYLAND_DISPLAY,XDG_RUNTIME_DIR,HOME,USER "$HOME/.lantern/bin/lntrn-snapshot-gui" "$@"
