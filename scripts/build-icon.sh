#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the app icon (.icns, all sizes 16–1024 @1x/@2x) and the menu bar template image
# from the SVG sources in design/icon. Output is committed, so CI does not need this.
#   scripts/build-icon.sh
set -eu
cd "$(dirname "$0")/.."
OUT=crates/battbar-app/resources
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
RENDER="swift design/icon/render.swift"
mkdir -p "$TMP/AppIcon.iconset" "$OUT"
for s in 16 32 128 256 512; do
  $RENDER design/icon/AppIcon.svg "$TMP/AppIcon.iconset/icon_${s}x${s}.png" "$s"
  $RENDER design/icon/AppIcon.svg "$TMP/AppIcon.iconset/icon_${s}x${s}@2x.png" "$((s * 2))"
done
iconutil -c icns -o "$TMP/AppIcon.icns" "$TMP/AppIcon.iconset"
mv -f "$TMP/AppIcon.icns" "$OUT/AppIcon.icns"
$RENDER design/icon/MenuBarIcon.svg "$TMP/MenuBarIcon.png" 18
$RENDER design/icon/MenuBarIcon.svg "$TMP/MenuBarIcon@2x.png" 36
mv -f "$TMP/MenuBarIcon.png" "$OUT/MenuBarIcon.png"
mv -f "$TMP/MenuBarIcon@2x.png" "$OUT/MenuBarIcon@2x.png"
echo "Built $OUT/AppIcon.icns, MenuBarIcon.png, MenuBarIcon@2x.png"
