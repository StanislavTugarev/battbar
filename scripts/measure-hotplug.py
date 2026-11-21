#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Measure how long battbar takes to show a device appearing or disappearing.

    scripts/measure-hotplug.py [--app /Applications/battbar.app] [--out results.tsv]
                               [--env NAME=VALUE ...] [--only TEXT ...]

Quits the running battbar, starts the given app with BATTBAR_LOG=1 (its poll log on
stderr goes to a temporary file) and walks through the scenarios below. For each one:
press Enter at the moment you act (switch the device on/off, plug/unplug); the script
waits until the poll log shows the expected state and prints the delay. `s` skips a
scenario. The app is started normally again at the end. If the poll log already
showed the new state before Enter (you pressed late, or a poll happened between the
action and Enter), the scenario is recorded as `already` instead of a time.

`--env` passes extra variables to the app (development builds: e.g.
`BATTBAR_DEV_NO_HOTPLUG=1 BATTBAR_DEV_RESCAN_SECS=20 BATTBAR_DEV_LOG_HIDPP=1`), `--only`
keeps the scenarios whose prompt contains the text. The results also give the clock
time of each Enter, to find it in the log (kept, path printed at the start).

The log line of a poll is written right before the menu bar is redrawn, so the time
it arrives is the time the change is visible (a device that is new to the running app
can be held back up to ~4 s more by the new-device check; the scenarios here use known
devices). Precision: your Enter press (~0.2 s) and 50 ms reading steps.

Reads nothing but battbar's own log; changes no settings.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
import time

SCENARIOS = [
    # (prompt, device name substring, expected state after the action)
    ("Turn OFF the mouse (G502)", "G502", "off"),
    ("Turn ON the mouse (G502)", "G502", "on"),
    ("Turn OFF the keyboard on the receiver (G915)", "G915", "off"),
    ("Turn ON the keyboard on the receiver (G915)", "G915", "on"),
    ("Turn OFF the Bluetooth keyboard (Pebble Keys)", "Pebble", "off"),
    ("Turn ON the Bluetooth keyboard (Pebble Keys)", "Pebble", "on"),
    ("Headset on Bluetooth: turn Bluetooth ON on the headset (base stays plugged in)", "Arctis", "bt-on"),
    ("Headset: turn Bluetooth OFF on the headset", "Arctis", "bt-off"),
    ("UNPLUG the mouse's USB receiver", "G502", "off"),
    ("PLUG the mouse's USB receiver back in", "G502", "on"),
    ("UNPLUG the headset base station (USB); headset Bluetooth off", "Arctis", "off"),
    ("PLUG the headset base station back in", "Arctis", "on"),
]

HEADER = re.compile(r"^\[\d\d:\d\d:\d\d\] input_monitoring=")
DEVICE = re.compile(r"^  (?P<name>.+?) \[(?P<kind>\w+)\] (?P<status>.*?) via \[(?P<via>[^\]]*)\]")


def plist(app, key):
    return subprocess.run(
        ["/usr/libexec/PlistBuddy", "-c", f"Print {key}", f"{app}/Contents/Info.plist"], capture_output=True, text=True
    ).stdout.strip()


def quit_app(app):
    """Quit the app under test (by its bundle ID: release or development build)."""
    bid = plist(app, "CFBundleIdentifier")
    subprocess.run(["osascript", "-e", f'tell application id "{bid}" to quit'], capture_output=True)
    exe = os.path.realpath(f"{app}/Contents/MacOS/battbar-app")
    for _ in range(50):
        if subprocess.run(["pgrep", "-f", exe], capture_output=True).returncode:
            return
        time.sleep(0.1)


class Log:
    """Poll blocks of the app's log as they arrive, with arrival times."""

    def __init__(self, path):
        self.f = open(path, "r", errors="replace")
        self.buf = ""
        self.block = None  # (arrival, [device lines])
        self.last_line = 0.0
        self.latest = []  # device lines of the last complete block

    def blocks(self):
        """Complete blocks read so far (a block ends at the next header or after
        0.3 s without new lines)."""
        out = []
        chunk = self.f.read()
        now = time.monotonic()
        if chunk:
            self.buf += chunk
            *lines, self.buf = self.buf.split("\n")
            for line in lines:
                self.last_line = now
                if HEADER.match(line):
                    if self.block:
                        out.append(self.block)
                    self.block = (now, [])
                elif self.block and DEVICE.match(line):
                    self.block[1].append(line)
        if self.block and now - self.last_line > 0.3:
            out.append(self.block)
            self.block = None
        if out:
            self.latest = out[-1][1]
        return out


def state(lines, name, expected):
    """Whether a block shows the expected state of the device."""
    devs = [DEVICE.match(l) for l in lines]
    devs = [d for d in devs if d and name.lower() in d["name"].lower()]
    has_value = [d for d in devs if "%" in d["status"]]
    if expected == "on":
        return bool(has_value)
    if expected == "off":
        return not has_value
    if expected == "bt-on":
        return any("Bluetooth" in d["via"] for d in has_value)
    if expected == "bt-off":
        return not any("Bluetooth" in d["via"] for d in has_value)
    raise ValueError(expected)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", default="/Applications/battbar.app")
    ap.add_argument("--out", default="hotplug-results.tsv")
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("--env", action="append", default=[], metavar="NAME=VALUE")
    ap.add_argument("--only", action="append", default=[], metavar="TEXT")
    a = ap.parse_args()
    scenarios = [s for s in SCENARIOS if not a.only or any(o.lower() in s[0].lower() for o in a.only)]
    version = plist(a.app, "BattbarVersion") or plist(a.app, "CFBundleShortVersionString")
    log_path = tempfile.mktemp(prefix="battbar-hotplug-", suffix=".log")
    print(f"battbar {version} from {a.app}\nlog: {log_path}")
    quit_app(a.app)
    env = [x for e in ["BATTBAR_LOG=1", *a.env] for x in ("--env", e)]
    subprocess.run(["open", *env, "--stderr", log_path, a.app], check=True)
    while not os.path.exists(log_path):
        time.sleep(0.1)
    log = Log(log_path)
    print("Waiting for the first poll…")
    while not log.blocks():
        time.sleep(0.05)
    results = []
    try:
        for prompt, name, expected in scenarios:
            ans = input(f"\n{prompt}\n  press Enter as you do it (s = skip, q = quit): ").strip().lower()
            if ans == "q":
                break
            if ans == "s":
                results.append((prompt, expected, "skipped", ""))
                continue
            t0 = time.monotonic()
            at = time.strftime("%H:%M:%S")
            log.blocks()  # what came before the action
            if state(log.latest, name, expected):
                print("  -> already shown before Enter (not timed)")
                results.append((prompt, expected, "already", at))
                continue
            took = None
            while time.monotonic() - t0 < a.timeout:
                for arrival, lines in log.blocks():
                    if state(lines, name, expected):
                        took = arrival - t0
                        break
                if took is not None:
                    break
                time.sleep(0.05)
            shown = f"{took:.1f}" if took is not None else f">{a.timeout:.0f}"
            print(f"  -> shown after {shown} s")
            results.append((prompt, expected, shown, at))
    finally:
        with open(a.out, "w") as f:
            extra = f" ({' '.join(a.env)})" if a.env else ""
            f.write(f"# battbar {version}{extra}\nscenario\texpected\tseconds\tat\n")
            for r in results:
                f.write("\t".join(r) + "\n")
        print(f"\nResults: {os.path.abspath(a.out)}")
        quit_app(a.app)
        subprocess.run(["open", a.app])


if __name__ == "__main__":
    sys.exit(main())
