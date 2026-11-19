#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Create the long-lived self-signed release certificate "battbar Release" (once per
# project, by the maintainer). Unlike the dev certificate it is NOT imported into the
# keychain: it is written to an output directory OUTSIDE the repository, to be
#   1. stored as GitHub Actions secrets (BATTBAR_RELEASE_P12_BASE64, BATTBAR_RELEASE_P12_PASSWORD),
#   2. backed up offline (password manager / encrypted storage).
# Losing it means every user gets macOS permission prompts again after the next update.
# See docs/DEVELOPMENT.md.
#
#   scripts/create-release-cert.sh [OUTPUT_DIR]   (default: ~/battbar-release-signing)
set -eu
NAME="battbar Release"
OUT="${1:-$HOME/battbar-release-signing}"
REPO=$(cd "$(dirname "$0")/.." && pwd)
case "$(cd "$(dirname "$OUT")" 2>/dev/null && pwd)/$(basename "$OUT")" in
  "$REPO"*) echo "Refusing to write signing material inside the repository." >&2; exit 1 ;;
esac
if [ -e "$OUT/battbar-release.p12" ]; then
  echo "$OUT/battbar-release.p12 already exists; not overwriting." >&2
  exit 1
fi
mkdir -p "$OUT"
chmod 700 "$OUT"
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
# 25 years: the identity must outlive many releases.
openssl req -x509 -newkey rsa:3072 -nodes -days 9125 \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -config "$TMP/cert.cnf" >/dev/null 2>&1
PASS=$(openssl rand -hex 24)
openssl pkcs12 -export -legacy -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
  -out "$OUT/battbar-release.p12" -passout "pass:$PASS" >/dev/null 2>&1 \
  || /usr/bin/openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
       -out "$OUT/battbar-release.p12" -passout "pass:$PASS"
printf '%s\n' "$PASS" > "$OUT/battbar-release.p12.password"
openssl x509 -in "$TMP/cert.pem" -noout -fingerprint -sha1 | sed 's/^.*=//; s/://g' > "$OUT/battbar-release.sha1"
chmod 600 "$OUT"/*
cat <<MSG
Created "$NAME" in $OUT
  battbar-release.p12           certificate + private key
  battbar-release.p12.password  its password
  battbar-release.sha1          certificate SHA-1 (part of the app's designated requirement)

Next steps:
  gh secret set BATTBAR_RELEASE_P12_BASE64 < <(base64 -i "$OUT/battbar-release.p12")
  gh secret set BATTBAR_RELEASE_P12_PASSWORD < "$OUT/battbar-release.p12.password"
Then back up the directory offline and delete it from this machine.
MSG
