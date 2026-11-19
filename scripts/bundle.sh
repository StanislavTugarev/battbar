#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build target/battbar.app (menu bar app + CLI) and sign it.
#
#   scripts/bundle.sh [--debug]
#
# Signing identity: $BATTBAR_SIGN_IDENTITY (default "battbar Development", see
# scripts/create-dev-cert.sh); falls back to ad-hoc with a warning.
# Binaries are never copied over existing files (the bundle is rebuilt from scratch),
# see docs/DEVELOPMENT.md.
set -eu
cd "$(dirname "$0")/.."

PROFILE=release
CARGO_FLAGS="--release"
if [ "${1:-}" = "--debug" ]; then PROFILE=debug; CARGO_FLAGS=""; fi

cargo build $CARGO_FLAGS -p battbar-app -p battbar-cli

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/battbar.app
STAGE=$(mktemp -d target/battbar.app.XXXXXX)
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "target/$PROFILE/battbar-app" "$STAGE/Contents/MacOS/battbar-app"
cp "target/$PROFILE/battbar" "$STAGE/Contents/MacOS/battbar"
sed "s/@VERSION@/$VERSION/g" crates/battbar-app/Info.plist > "$STAGE/Contents/Info.plist"
cp LICENSE NOTICE "$STAGE/Contents/Resources/"

IDENTITY="${BATTBAR_SIGN_IDENTITY:-battbar Development}"
if security find-identity -p codesigning 2>/dev/null | grep -q "\"$IDENTITY\""; then
  SIGN="$IDENTITY"
else
  echo "warning: signing identity \"$IDENTITY\" not found; using ad-hoc signature" >&2
  echo "         (macOS will ask for permissions again after every build; see docs/DEVELOPMENT.md)" >&2
  SIGN="-"
fi
codesign --force --sign "$SIGN" --identifier io.github.battbar.cli "$STAGE/Contents/MacOS/battbar"
codesign --force --sign "$SIGN" --identifier io.github.battbar "$STAGE"
codesign --verify --strict "$STAGE"

# Atomic replace of the whole bundle.
rm -rf "$APP.old"
[ -d "$APP" ] && mv "$APP" "$APP.old"
mv "$STAGE" "$APP"
rm -rf "$APP.old"
echo "Built $APP ($VERSION, signed with: $SIGN)"
