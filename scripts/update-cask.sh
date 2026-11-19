#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Render the cask for VERSION/SHA256 and push it to the tap repository.
#   scripts/update-cask.sh VERSION SHA256
# Env: HOMEBREW_TAP_TOKEN (push access to <owner>/homebrew-battbar), GITHUB_REPOSITORY_OWNER.
set -eu
VERSION="$1"; SHA="$2"
OWNER="${GITHUB_REPOSITORY_OWNER:?}"
cd "$(dirname "$0")/.."
TAP=$(mktemp -d)
git clone --depth 1 "https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/$OWNER/homebrew-battbar.git" "$TAP"
mkdir -p "$TAP/Casks"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHA256@/$SHA/g" -e "s/@OWNER@/$OWNER/g" packaging/homebrew/battbar.rb.in > "$TAP/Casks/battbar.rb"
cd "$TAP"
git config user.name "battbar release bot"
git config user.email "battbar-release@users.noreply.github.com"
git add Casks/battbar.rb
git commit -m "battbar $VERSION"
git push
