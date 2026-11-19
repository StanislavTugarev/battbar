#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# CI: import the release certificate from secrets into a temporary keychain.
# Env: BATTBAR_RELEASE_P12_BASE64, BATTBAR_RELEASE_P12_PASSWORD, RUNNER_TEMP.
# Prints nothing secret. The keychain is deleted by scripts/ci-cleanup-keychain.sh.
set -eu
: "${BATTBAR_RELEASE_P12_BASE64:?secret missing}"
: "${BATTBAR_RELEASE_P12_PASSWORD:?secret missing}"
KC="$RUNNER_TEMP/battbar-signing.keychain-db"
KCPASS=$(openssl rand -hex 24)
P12="$RUNNER_TEMP/battbar-release.p12"
# Tolerate line breaks/trailing newlines from `gh secret set NAME < file`.
PASS=$(printf '%s' "$BATTBAR_RELEASE_P12_PASSWORD" | tr -d '\r\n')
printf '%s' "$BATTBAR_RELEASE_P12_BASE64" | tr -d ' \r\n' | base64 --decode > "$P12"
security create-keychain -p "$KCPASS" "$KC"
security set-keychain-settings -lut 3600 "$KC"
security unlock-keychain -p "$KCPASS" "$KC"
security import "$P12" -k "$KC" -P "$PASS" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k "$KCPASS" "$KC" >/dev/null
security list-keychains -d user -s "$KC" $(security list-keychains -d user | tr -d '"')
rm -f "$P12"
security find-identity -p codesigning "$KC" | grep -q '"battbar Release"'
echo "Imported signing identity \"battbar Release\"."
