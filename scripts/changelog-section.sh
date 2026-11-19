#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Print the CHANGELOG.md section for a version; fail if there is none.
set -eu
V="$1"
cd "$(dirname "$0")/.."
OUT=$(awk -v v="$V" '
  $0 ~ "^## \\[" v "\\]" { on = 1; next }
  on && /^## \[/ { exit }
  on { print }
' CHANGELOG.md)
if [ -z "$(printf '%s' "$OUT" | tr -d '[:space:]')" ]; then
  echo "CHANGELOG.md has no section for [$V]" >&2
  exit 1
fi
printf '%s\n' "$OUT"
