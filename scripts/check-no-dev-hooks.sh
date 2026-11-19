#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Development-only hooks (compiled with cfg(debug_assertions)) must not exist in release
# binaries. Their names are the marker: if the code is compiled in, so is the string.
#   scripts/check-no-dev-hooks.sh BINARY...                  # release: must be absent
#   scripts/check-no-dev-hooks.sh --expect-present BINARY... # self-test on a debug build
set -eu
HOOKS="BATTBAR_DEV_FAKE_VERSION"
EXPECT=absent
if [ "${1:-}" = "--expect-present" ]; then EXPECT=present; shift; fi
[ $# -gt 0 ] || { echo "usage: $0 [--expect-present] BINARY..." >&2; exit 2; }
fail=0
for bin in "$@"; do
  [ -f "$bin" ] || { echo "missing: $bin" >&2; exit 2; }
  for h in $HOOKS; do
    if LC_ALL=C grep -a -q "$h" "$bin"; then found=yes; else found=no; fi
    if [ "$EXPECT" = absent ] && [ "$found" = yes ]; then
      echo "FAIL: $bin contains development hook $h" >&2; fail=1
    elif [ "$EXPECT" = present ] && [ "$found" = no ]; then
      echo "FAIL: $bin lacks $h (the check would not detect it)" >&2; fail=1
    fi
  done
done
[ $fail -eq 0 ] && echo "Development hooks $EXPECT as expected in: $*"
exit $fail
