#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Render the cask for VERSION/SHA256 and push it to the tap repository over SSH with a
# deploy key that has write access to the tap only.
#   scripts/update-cask.sh VERSION SHA256
# Env: HOMEBREW_TAP_DEPLOY_KEY (OpenSSH private key, as is), RUNNER_TEMP (optional).
set -eu
VERSION="$1"; SHA="$2"
OWNER=StanislavTugarev
: "${HOMEBREW_TAP_DEPLOY_KEY:?secret missing}"
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d "${RUNNER_TEMP:-/tmp}/tap.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
umask 077
printf '%s\n' "$HOMEBREW_TAP_DEPLOY_KEY" > "$WORK/key"
# GitHub's published SSH host key (https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints).
echo "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl" > "$WORK/known_hosts"
export GIT_SSH_COMMAND="ssh -i $WORK/key -o IdentitiesOnly=yes -o UserKnownHostsFile=$WORK/known_hosts -o StrictHostKeyChecking=yes"
git clone --depth 1 "git@github.com:$OWNER/homebrew-battbar.git" "$WORK/tap"
mkdir -p "$WORK/tap/Casks"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHA256@/$SHA/g" -e "s/@OWNER@/$OWNER/g" \
  "$ROOT/packaging/homebrew/battbar.rb.in" > "$WORK/tap/Casks/battbar.rb"
cd "$WORK/tap"
git config user.name "battbar release"
git config user.email "121640176+StanislavTugarev@users.noreply.github.com"
git add Casks/battbar.rb
if git diff --cached --quiet; then
  echo "Cask already at $VERSION."
  exit 0
fi
git commit -m "battbar $VERSION"
git push origin HEAD
