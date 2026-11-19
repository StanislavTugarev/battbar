#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Stage 0 helper: capture everything macOS knows about Bluetooth peripherals and their
# battery, plus the HID view of them. Read-only. Bluetooth addresses are stripped.
#
# usage: tools/discovery/bt-snapshot.sh > docs/discovery/bt-<label>.txt
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN="$ROOT/target/debug/battbar-discovery"

echo "== date: $(date -u +%Y-%m-%dT%H:%MZ)  macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
echo
echo "== system_profiler SPBluetoothDataType (addresses removed) =="
system_profiler SPBluetoothDataType -json 2>/dev/null | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
for c in d.get("SPBluetoothDataType", []):
    for key in ("device_connected", "device_not_connected"):
        for item in c.get(key, []):
            for name, props in item.items():
                props = {k: v for k, v in props.items() if "address" not in k.lower()}
                print(key, "|", name, "|", json.dumps(props, sort_keys=True))
'
echo
echo "== IORegistry: battery-related keys (with owning class / product) =="
ioreg -l -w0 2>/dev/null | python3 -I -c '
import re, sys
cls = None; product = None; transport = None
for line in sys.stdin:
    m = re.search(r"\+-o (\S+)\s+<class (\w+)", line)
    if m:
        cls = m.group(2) + " (" + m.group(1) + ")"; product = None; transport = None
        continue
    m = re.search(r"\"(Product|Transport)\" = \"([^\"]*)\"", line)
    if m:
        if m.group(1) == "Product": product = m.group(2)
        else: transport = m.group(2)
    m = re.search(r"\"([A-Za-z]*Battery[A-Za-z]*)\" = (.*)$", line)
    if m and (product is not None or m.group(1).startswith("BatteryPercent")) and "PPM" not in (cls or "") and "SmartBattery" not in (cls or ""):
        print(f"{cls} product={product!r} transport={transport!r}: {m.group(1)} = {m.group(2).strip()[:200]}")
'
echo
echo "== hidutil (external devices) =="
hidutil list 2>/dev/null | awk 'NR<=2 || ($1 != "0x0" && $1 != "0x5ac")'
echo
if [ -x "$BIN" ]; then
  "$BIN" enum | sed -E 's/path=DevSrvsID:[0-9]+/path=<redacted>/'
  "$BIN" hidpp "$@"
else
  echo "(build tools/discovery first: cargo build -p battbar-discovery)"
fi
