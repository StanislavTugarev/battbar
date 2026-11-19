#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Fail if the bundle is not signed by the pinned "battbar Release" certificate.
# A different certificate would make macOS ask every user for permissions again.
#   scripts/check-release-signature.sh target/battbar.app
set -eu
APP="$1"
cd "$(dirname "$0")/.."
EXPECTED=$(tr 'A-F' 'a-f' < packaging/release-cert.sha1 | tr -d ' \n')
REQ=$(codesign -d -r- "$APP" 2>&1 | sed -n 's/^designated => //p')
LEAF=$(printf '%s' "$REQ" | sed -n 's/.*certificate leaf = H"\([0-9a-fA-F]*\)".*/\1/p' | tr 'A-F' 'a-f')
ID=$(printf '%s' "$REQ" | sed -n 's/.*identifier "\([^"]*\)".*/\1/p')
echo "designated requirement: $REQ"
[ "$ID" = "io.github.battbar" ] || { echo "unexpected bundle identifier: $ID" >&2; exit 1; }
[ "$LEAF" = "$EXPECTED" ] || { echo "signed by certificate $LEAF, expected pinned $EXPECTED (packaging/release-cert.sha1)" >&2; exit 1; }
echo "Signed by the pinned battbar Release certificate."
