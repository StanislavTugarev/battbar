#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Fail if certificates, keys or keychains are tracked in git (run in CI).
set -eu
cd "$(dirname "$0")/.."
bad=$(git ls-files | grep -Ei '\.(p12|pfx|cer|crt|der|key|pem|keychain|keychain-db|mobileprovision|provisionprofile)$' || true)
if [ -n "$bad" ]; then
  echo "Certificate/key files are tracked in git:" >&2
  echo "$bad" >&2
  exit 1
fi
if git grep -nI -E -e '-----BEGIN ([A-Z]+ )?PRIVATE KEY-----' -e '-----BEGIN CERTIFICATE-----' -- . ':!scripts/check-no-secrets.sh' ':!scripts/check-history.py' >/dev/null; then
  echo "PEM private key or certificate found in tracked files:" >&2
  git grep -nI -l -E -e '-----BEGIN ([A-Z]+ )?PRIVATE KEY-----' -e '-----BEGIN CERTIFICATE-----' -- . ':!scripts/check-no-secrets.sh' ':!scripts/check-history.py' >&2
  exit 1
fi
echo "No signing material tracked."
