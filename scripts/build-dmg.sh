#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build a drag-to-Applications disk image from a built app bundle.
#   scripts/build-dmg.sh target/battbar.app dist/battbar-0.2.0.dmg
# Needs: swift (Xcode command line tools) and dmgbuild (pip install dmgbuild==2.0.0);
# set DMGBUILD to its path if it is not on PATH.
set -eu
APP="$1"
OUT="$2"
cd "$(dirname "$0")/.."
DMGBUILD="${DMGBUILD:-dmgbuild}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
swift design/icon/render.swift design/dmg/background.svg "$TMP/bg.png" 660x420
swift design/icon/render.swift design/dmg/background.svg "$TMP/bg@2x.png" 1320x840
tiffutil -cathidpicheck "$TMP/bg.png" "$TMP/bg@2x.png" -out "$TMP/background.tiff" >/dev/null
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
"$DMGBUILD" -s packaging/dmg/settings.py \
  -D app="$APP" \
  -D background="$TMP/background.tiff" \
  -D volume_icon=crates/battbar-app/resources/AppIcon.icns \
  "battbar" "$OUT"
echo "Built $OUT"
