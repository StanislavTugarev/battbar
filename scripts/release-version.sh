#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Validate a release tag against Cargo.toml and print shell assignments:
#   VERSION=0.1.0  RELEASE_VERSION=0.1.0-rc.1  PRERELEASE=true|false
# Accepted tags: vX.Y.Z and vX.Y.Z-rc.N; X.Y.Z must equal the workspace version.
set -eu
TAG="$1"
cd "$(dirname "$0")/.."
CARGO=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case "$TAG" in
  v*) REL="${TAG#v}" ;;
  *) echo "tag must start with v: $TAG" >&2; exit 1 ;;
esac
if printf '%s' "$REL" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  BASE="$REL"; PRE=false
elif printf '%s' "$REL" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+-rc\.[0-9]+$'; then
  BASE="${REL%%-rc.*}"; PRE=true
else
  echo "unsupported tag $TAG (use vX.Y.Z or vX.Y.Z-rc.N)" >&2; exit 1
fi
if [ "$BASE" != "$CARGO" ]; then
  echo "tag $TAG does not match Cargo.toml version $CARGO" >&2; exit 1
fi
echo "VERSION=$BASE"
echo "RELEASE_VERSION=$REL"
echo "PRERELEASE=$PRE"
