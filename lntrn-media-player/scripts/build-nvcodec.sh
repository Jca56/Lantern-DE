#!/bin/sh
# Build GStreamer's nvcodec plugin (NVDEC/NVENC: nvh264dec, nvh265dec,
# nvav1dec, ...) into ~/.lantern/lib/gstreamer-1.0.
#
# Gentoo's gst-plugins-bad has no nvcodec USE flag and no repo packages the
# plugin, so we build just that one plugin from the same source tarball
# portage already fetched, against the installed GStreamer. The plugin
# dlopens the driver's libcuda/libnvcuvid at runtime; no CUDA SDK needed.
#
# Re-run after a GStreamer version bump — the plugin must match the
# installed gstreamer-1.0 exactly (it links libgstcodecs/libgstcodecparsers
# from the system at that same version). lntrn-media-player scans the
# install dir at startup (see main.rs).
#
# Usage: scripts/build-nvcodec.sh [path/to/gst-plugins-bad-<ver>.tar.xz]
set -eu

GST_VER=$(pkg-config --modversion gstreamer-1.0)
TARBALL=${1:-/var/cache/distfiles/gst-plugins-bad-${GST_VER}.tar.xz}
DEST=${LANTERN_GST_PLUGIN_DIR:-$HOME/.lantern/lib/gstreamer-1.0}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/lntrn-nvcodec.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

[ -f "$TARBALL" ] || { echo "no source tarball at $TARBALL (installed GStreamer is $GST_VER)"; exit 1; }
for t in meson ninja patchelf; do command -v "$t" >/dev/null || { echo "missing tool: $t"; exit 1; }; done

echo "installed GStreamer $GST_VER, building nvcodec from $TARBALL"
tar xf "$TARBALL" -C "$WORK"
SRC=$(find "$WORK" -maxdepth 1 -mindepth 1 -type d -name 'gst-plugins-bad-*')
cd "$SRC"
meson setup build --buildtype=release -Dauto_features=disabled \
    -Dnvcodec=enabled -Dnvcomp=disabled -Dcuda-nvmm=disabled -Dgl=disabled \
    -Dtests=disabled -Dexamples=disabled -Dtools=disabled -Ddoc=disabled \
    -Dintrospection=disabled -Dorc=disabled > "$WORK/meson-setup.log"
ninja -C build sys/nvcodec/libgstnvcodec.so > "$WORK/ninja.log"

CUDA_LIB=$(ls build/gst-libs/gst/cuda/libgstcuda-1.0.so.0.*)
mkdir -p "$DEST"
cp build/sys/nvcodec/libgstnvcodec.so "$DEST/"
cp "$CUDA_LIB" "$DEST/"
ln -sfn "$(basename "$CUDA_LIB")" "$DEST/libgstcuda-1.0.so.0"
# The plugin finds libgstcuda next to itself; everything else comes from the system.
patchelf --set-rpath '$ORIGIN' "$DEST/libgstnvcodec.so"

echo "installed to $DEST:"
ls -la "$DEST"
GST_PLUGIN_PATH="$DEST" gst-inspect-1.0 nvh264dec | grep -E '^\s*Rank' || echo "WARNING: nvh264dec did not load"
