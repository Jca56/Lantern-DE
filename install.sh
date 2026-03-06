 #!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
BINDIR="${BINDIR:-$PREFIX/bin}"
SHAREDIR="${SHAREDIR:-$PREFIX/share}"

echo "Building Lantern DE..."
cargo build --release \
    -p lntrn-session-manager \
    -p lntrn-window-manager

echo "Installing binaries to $BINDIR"
sudo install -Dm755 target/release/lntrn-session-manager "$BINDIR/lntrn-session-manager"
sudo install -Dm755 target/release/lntrn-window-manager  "$BINDIR/lntrn-window-manager"

echo "Installing session desktop file"
sudo install -Dm644 lntrn-session-manager/lantern.desktop "$SHAREDIR/xsessions/lantern.desktop"

echo "Installing default wallpaper"
sudo install -Dm644 Lantern-Night.png "$SHAREDIR/lantern/wallpapers/Lantern-Night.png"

echo "Installed successfully"
