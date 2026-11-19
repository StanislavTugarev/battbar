#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
KC="${RUNNER_TEMP:-/tmp}/battbar-signing.keychain-db"
[ -f "$KC" ] && security delete-keychain "$KC" || true
