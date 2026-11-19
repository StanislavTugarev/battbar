#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check EVERY commit reachable from any ref (not only HEAD):

* no certificate / key / keychain files and no PEM private keys or certificates;
* traces (traces/*.jsonl): every HID++ unit ID in the frames is a pseudonym (prefix
  bb a7) and nothing looks like a Bluetooth address.

Unit IDs are found by parsing the frames (receiver register 0xB5/0x3n and 0x0003
getDeviceInfo after IRoot.getFeature), the same logic as
crates/battbar-hidpp/src/trace_ids.rs. Run: python3 scripts/check-history.py
"""
import json
import re
import subprocess
import sys

SECRET_EXT = re.compile(r"\.(p12|pfx|cer|crt|der|key|pem|keychain|keychain-db|mobileprovision|provisionprofile)$", re.I)
PEM = re.compile(r"-----BEGIN ([A-Z]+ )?PRIVATE KEY-----|-----BEGIN CERTIFICATE-----")
MAC = re.compile(r"\b[0-9A-Fa-f]{2}([:-])(?:[0-9A-Fa-f]{2}\1){4}[0-9A-Fa-f]{2}\b")
SELF = {"scripts/check-history.py", "scripts/check-no-secrets.sh"}


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True).stdout


def unit_ids(text):
    ids, pending, info = [], set(), {}
    for line in text.splitlines():
        try:
            o = json.loads(line)
        except ValueError:
            continue
        if o.get("type") != "frame":
            continue
        f = bytes.fromhex(o["hex"].replace(" ", ""))
        link = o["link"]
        if len(f) < 7 or f[0] not in (0x10, 0x11):
            continue
        if o["dir"] == "out":
            if f[2] == 0 and f[3] >> 4 == 0 and f[4] == 0 and f[5] == 3:
                pending.add((link, f[1], f[3]))
            continue
        if len(f) >= 9 and f[1] == 0xFF and f[2] == 0x83 and f[3] == 0xB5 and 0x30 <= f[4] <= 0x3F:
            ids.append(f[5:9])
            continue
        if f[2] == 0 and (link, f[1], f[3]) in pending:
            pending.discard((link, f[1], f[3]))
            if f[4]:
                info[(link, f[1])] = f[4]
            continue
        if len(f) >= 9 and info.get((link, f[1])) == f[2] and f[3] >> 4 == 0 and f[3] & 0x0F:
            ids.append(f[5:9])
    return [u for u in ids if u != b"\0\0\0\0"]


def main():
    commits = git("rev-list", "--all").decode().split()
    problems = []
    for c in commits:
        short = c[:8]
        files = git("ls-tree", "-r", "--name-only", c).decode().splitlines()
        for f in files:
            if SECRET_EXT.search(f):
                problems.append(f"{short}: signing material file {f}")
        for f in files:
            if f in SELF:
                continue
            blob = git("show", f"{c}:{f}")
            if b"\0" in blob[:8000]:
                continue
            text = blob.decode("utf-8", "replace")
            if PEM.search(text):
                problems.append(f"{short}: PEM key/certificate in {f}")
            if f.startswith("traces/") and f.endswith(".jsonl"):
                if MAC.search(text):
                    problems.append(f"{short}: Bluetooth-address-like value in {f}")
                for u in unit_ids(text):
                    if u[:2] != b"\xbb\xa7":
                        problems.append(f"{short}: real-looking unit ID {u.hex()} in {f}")
    for p in sorted(set(problems)):
        print(p)
    print(f"checked {len(commits)} commits: {'FAILED' if problems else 'ok'}")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
