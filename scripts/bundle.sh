#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build target/battbar.app (menu bar app + CLI) and sign it.
#
# Locally:  scripts/bundle.sh                 debug build of the host architecture
#           BATTBAR_LOCAL_RELEASE=1 scripts/bundle.sh
#                                             optimized build, still the development
#                                             bundle ID (soak tests, measurements)
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

# Release bundles (CI) and development bundles use different bundle IDs, so their
# macOS permission records (signed by "battbar Release" vs "battbar Development") never
# collide on the same Mac.
if [ -n "${BATTBAR_BIN_DIR:-}" ]; then
  BIN="$BATTBAR_BIN_DIR"
  BUNDLE_ID=io.github.battbar
  BUNDLE_NAME=battbar
elif [ "${BATTBAR_LOCAL_RELEASE:-0}" = "1" ]; then
  cargo build --release -p battbar-app -p battbar-cli
  BIN=target/release
  BUNDLE_ID=io.github.battbar.dev
  BUNDLE_NAME="battbar Dev"
else
  cargo build -p battbar-app -p battbar-cli
  BIN=target/debug
  BUNDLE_ID=io.github.battbar.dev
  BUNDLE_NAME="battbar Dev"
fi

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# Full version with -rc.N (release workflow: from the tag); the binaries were built
# with the same BATTBAR_RELEASE_VERSION.
FULL_VERSION="${BATTBAR_RELEASE_VERSION:-$VERSION}"
case "$FULL_VERSION" in
  "$VERSION" | "$VERSION"-rc.[0-9]*) ;;
  *) echo "BATTBAR_RELEASE_VERSION=$FULL_VERSION does not match Cargo.toml $VERSION" >&2; exit 1 ;;
esac
APP=target/battbar.app
mkdir -p target
STAGE=$(mktemp -d target/battbar.app.XXXXXX)
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "$BIN/battbar-app" "$STAGE/Contents/MacOS/battbar-app"
cp "$BIN/battbar" "$STAGE/Contents/MacOS/battbar"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@FULL_VERSION@/$FULL_VERSION/g" -e "s/@BUNDLE_ID@/$BUNDLE_ID/g" -e "s/@BUNDLE_NAME@/$BUNDLE_NAME/g" crates/battbar-app/Info.plist > "$STAGE/Contents/Info.plist"
cp LICENSE NOTICE "$STAGE/Contents/Resources/"
# App icon and menu bar template image (built by scripts/build-icon.sh, committed).
cp crates/battbar-app/resources/AppIcon.icns crates/battbar-app/resources/MenuBarIcon.png \
   crates/battbar-app/resources/MenuBarIcon@2x.png "$STAGE/Contents/Resources/"

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
codesign --force --sign "$SIGN" --identifier "$BUNDLE_ID.cli" "$STAGE/Contents/MacOS/battbar"
codesign --force --sign "$SIGN" --identifier "$BUNDLE_ID" "$STAGE"
codesign --verify --strict "$STAGE"

rm -rf "$APP.old"
if [ -d "$APP" ]; then mv "$APP" "$APP.old"; fi
mv "$STAGE" "$APP"
rm -rf "$APP.old"
echo "Built $APP ($FULL_VERSION, $BUNDLE_ID, signed with: $SIGN)"
