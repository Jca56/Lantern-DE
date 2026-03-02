#!/usr/bin/env bash
# Set FOX Image Viewer as the default handler for image MIME types.
# This runs as regular user (no sudo needed).

set -euo pipefail

DESKTOP="fox-image-viewer.desktop"

IMAGE_MIMES=(
    image/png
    image/jpeg
    image/gif
    image/bmp
    image/webp
    image/svg+xml
    image/tiff
    image/x-icon
    image/x-portable-bitmap
    image/x-portable-graymap
    image/x-portable-pixmap
    image/x-xbitmap
    image/x-xpixmap
)

echo "=== Setting FOX Image Viewer as default ==="
echo ""

for mime in "${IMAGE_MIMES[@]}"; do
    echo "  ${mime} → ${DESKTOP}"
    xdg-mime default "${DESKTOP}" "${mime}"
done

echo ""

# Verify one
CURRENT=$(xdg-mime query default image/png 2>/dev/null || echo "unknown")
echo "Verification: image/png default is now '${CURRENT}'"

if [[ "${CURRENT}" == "${DESKTOP}" ]]; then
    echo "Success! FOX Image Viewer is your default image viewer."
else
    echo "Warning: verification did not match. You may need to log out and back in."
fi
