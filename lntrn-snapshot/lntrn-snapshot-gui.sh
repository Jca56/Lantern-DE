#!/bin/sh
exec sudo --preserve-env=WAYLAND_DISPLAY,XDG_RUNTIME_DIR,HOME,USER /home/alva/.lantern/bin/lntrn-snapshot-gui "$@"
