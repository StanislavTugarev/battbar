#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build target/battbar.app (menu bar app + CLI) and sign it.
#
# Locally:  scripts/bundle.sh                 debug build of the host architecture
# In CI:    BATTBAR_BIN_DIR=target/universal  use prebuilt (release, universal) binaries
#
# Release and universal builds are made only in CI (.github/workflows/release.yml).
#
# Signing identity: $BATTBAR_SIGN_IDENTITY (default "battbar Development", see
# scripts/create-dev-cert.sh). Without it the bundle is signed ad hoc with a warning,
# unless BATTBAR_REQUIRE_IDENTITY=1 (CI), which fails instead.
# The bundle is assembled in a fresh directory and moved into place, so no binary is
# ever copied over an existing file (see docs/DEVELOPMENT.md).
set -eu
cd "$(dirname "$0")/.."

if [ -n "${BATTBAR_BIN_DIR:-}" ]; then
  BIN="$BATTBAR_BIN_DIR"
else
  cargo build -p battbar-app -p battbar-cli
  BIN=target/debug
fi

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/battbar.app
mkdir -p target
STAGE=$(mktemp -d target/battbar.app.XXXXXX)
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "$BIN/battbar-app" "$STAGE/Contents/MacOS/battbar-app"
cp "$BIN/battbar" "$STAGE/Contents/MacOS/battbar"
sed "s/@VERSION@/$VERSION/g" crates/battbar-app/Info.plist > "$STAGE/Contents/Info.plist"
cp LICENSE NOTICE "$STAGE/Contents/Resources/"

IDENTITY="${BATTBAR_SIGN_IDENTITY:-battbar Development}"
if security find-identity -p codesigning 2>/dev/null | grep -q "\"$IDENTITY\""; then
  SIGN="$IDENTITY"
elif [ "${BATTBAR_REQUIRE_IDENTITY:-0}" = "1" ]; then
  echo "error: signing identity \"$IDENTITY\" not found" >&2
  rm -rf "$STAGE"
  exit 1
else
  echo "warning: signing identity \"$IDENTITY\" not found; using ad-hoc signature" >&2
  echo "         (macOS will ask for permissions again after every build; see docs/DEVELOPMENT.md)" >&2
  SIGN="-"
fi
# Nested code first, then the bundle. A Developer ID / notarization step can replace
# this block later (add --options runtime and --timestamp).
codesign --force --sign "$SIGN" --identifier io.github.battbar.cli "$STAGE/Contents/MacOS/battbar"
codesign --force --sign "$SIGN" --identifier io.github.battbar "$STAGE"
codesign --verify --strict "$STAGE"

rm -rf "$APP.old"
if [ -d "$APP" ]; then mv "$APP" "$APP.old"; fi
mv "$STAGE" "$APP"
rm -rf "$APP.old"
echo "Built $APP ($VERSION, signed with: $SIGN)"
