#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Create a self-signed code-signing certificate "battbar Development" in the login
# keychain (once per machine). A stable signing identity keeps macOS privacy
# permissions (Bluetooth, notifications) across rebuilds. See docs/DEVELOPMENT.md.
#
#   scripts/create-dev-cert.sh [--export FILE.p12]   # --export: also write a .p12 for CI
set -eu
NAME="${BATTBAR_SIGN_IDENTITY:-battbar Development}"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -q "\"$NAME\""; then
  echo "Identity \"$NAME\" already exists."
  exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/cert.cnf" <<CNF
[req]
distinguished_name = dn
prompt = no
x509_extensions = ext
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -config "$TMP/cert.cnf" >/dev/null 2>&1
PASS=$(openssl rand -hex 16)
# macOS `security import` only understands the legacy PKCS#12 format (3DES/SHA1 MAC):
# OpenSSL 3 needs -legacy, LibreSSL (/usr/bin/openssl) produces it by default.
openssl pkcs12 -export -legacy -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
  -out "$TMP/cert.p12" -passout "pass:$PASS" >/dev/null 2>&1 \
  || /usr/bin/openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
       -out "$TMP/cert.p12" -passout "pass:$PASS"
# -T: allow codesign to use the key without a prompt.
security import "$TMP/cert.p12" -k "$KEYCHAIN" -P "$PASS" -T /usr/bin/codesign >/dev/null

if [ "${1:-}" = "--export" ] && [ -n "${2:-}" ]; then
  cp "$TMP/cert.p12" "$2"
  echo "Exported to $2 (password: $PASS) — store both as CI secrets, then delete the file."
fi
echo "Created code-signing identity \"$NAME\"."
