#!/usr/bin/env bash
# Uninstall FOX Image Viewer from system.

set -euo pipefail

INSTALL_DIR="/usr/local/lib/fox-image-viewer"
BIN_LINK="/usr/local/bin/fox-image-viewer"
DESKTOP_FILE="/usr/share/applications/fox-image-viewer.desktop"

if [[ $EUID -ne 0 ]]; then
    exec sudo bash "$0" "$@"
fi

echo "=== Uninstalling FOX Image Viewer ==="

rm -rf "${INSTALL_DIR}" && echo "  Removed ${INSTALL_DIR}"
rm -f  "${BIN_LINK}"    && echo "  Removed ${BIN_LINK}"
rm -f  "${DESKTOP_FILE}" && echo "  Removed ${DESKTOP_FILE}"

if command -v update-desktop-database &>/dev/null; then
    update-desktop-database /usr/share/applications/ 2>/dev/null || true
fi

echo ""
echo "Uninstalled. To reset default image viewer:"
echo "  xdg-mime default org.kde.gwenview.desktop image/png"
