#!/usr/bin/env bash
# Install FOX Image Viewer system-wide to /usr/local/

set -euo pipefail

INSTALL_DIR="/usr/local/lib/fox-image-viewer"
BIN_LINK="/usr/local/bin/fox-image-viewer"
DESKTOP_FILE="/usr/share/applications/fox-image-viewer.desktop"
ICON_DIR="/usr/share/icons/hicolor"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "=== FOX Image Viewer — System Install ==="
echo ""

# Check for root
if [[ $EUID -ne 0 ]]; then
    echo "This script needs sudo. Re-running with sudo..."
    exec sudo bash "$0" "$@"
fi

# Copy application files
echo "[1/4] Copying app files to ${INSTALL_DIR}..."
mkdir -p "${INSTALL_DIR}"
cp -r "${SCRIPT_DIR}/main.py" "${INSTALL_DIR}/"
cp -r "${SCRIPT_DIR}/viewer" "${INSTALL_DIR}/"

# Make main.py executable
chmod +x "${INSTALL_DIR}/main.py"

# Create launcher script
echo "[2/4] Creating launcher at ${BIN_LINK}..."
cat > "${BIN_LINK}" << 'LAUNCHER'
#!/usr/bin/env bash
exec python3 /usr/local/lib/fox-image-viewer/main.py "$@"
LAUNCHER
chmod +x "${BIN_LINK}"

# Install .desktop file
echo "[3/4] Installing .desktop file..."
cp "${SCRIPT_DIR}/fox-image-viewer.desktop" "${DESKTOP_FILE}"

# Update desktop database
echo "[4/4] Updating desktop database..."
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database /usr/share/applications/ 2>/dev/null || true
fi

echo ""
echo "Installed successfully!"
echo "  App:      ${INSTALL_DIR}"
echo "  Launcher: ${BIN_LINK}"
echo "  Desktop:  ${DESKTOP_FILE}"
echo ""
echo "To set as default image viewer, run:"
echo "  bash set-default.sh"
