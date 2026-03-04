 #!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
BINDIR="${BINDIR:-$PREFIX/bin}"

cargo build --release -p lntrn-bar

echo "Installing lntrn-bar to $BINDIR"
sudo install -Dm755 target/release/lntrn-bar "$BINDIR/lntrn-bar"

echo "Installed successfully"
