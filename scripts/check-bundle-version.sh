#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Fail unless the bundle carries the release's full version, -rc.N included:
# Info.plist BattbarVersion and `--version` of both binaries = EXPECTED,
# CFBundleShortVersionString = its X.Y.Z.
#   scripts/check-bundle-version.sh target/battbar.app 0.5.0-rc.1
set -eu
APP="$1"
EXPECTED="$2"
BASE="${EXPECTED%%-rc.*}"
PLIST="$APP/Contents/Info.plist"
get() { /usr/libexec/PlistBuddy -c "Print $1" "$PLIST" 2>/dev/null || echo "(missing)"; }
FAIL=0
check() {
  if [ "$2" = "$3" ]; then echo "$1: $2"; else echo "$1: $2, expected $3" >&2; FAIL=1; fi
}
check BattbarVersion "$(get BattbarVersion)" "$EXPECTED"
check CFBundleShortVersionString "$(get CFBundleShortVersionString)" "$BASE"
check "battbar --version" "$("$APP/Contents/MacOS/battbar" --version)" "battbar $EXPECTED"
check "battbar-app --version" "$("$APP/Contents/MacOS/battbar-app" --version)" "battbar $EXPECTED"
[ "$FAIL" = 0 ] && echo "Bundle version matches $EXPECTED."
exit "$FAIL"
