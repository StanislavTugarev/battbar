#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Long run of battbar with its resource use sampled every few minutes.

    scripts/soak.py --app target/battbar.app --hours 6 --out soak.tsv [--every 300]

Starts the app (quitting a running copy of the same bundle first) with BATTBAR_LOG=1
(poll log on stderr, kept next to the results as <out>.log) and writes one row per
sample: elapsed time, resident memory, accumulated CPU time, context switches (top),
threads, open files, polls so far, power assertions held by the app, and the
settings file's modification time. Reads only what `ps`, `top`, `lsof` and `pmset`
show; changes nothing. The app keeps running afterwards.
"""

import argparse
import os
import re
import subprocess
import sys
import time


def run(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True).stdout


def plist(app, key):
    return run("/usr/libexec/PlistBuddy", "-c", f"Print {key}", f"{app}/Contents/Info.plist").strip()


def cpu_seconds(t):
    # ps TIME: [[dd-]hh:]mm:ss.cc
    days, _, t = t.rpartition("-")
    parts = [float(x) for x in t.split(":")]
    secs = 0.0
    for p in parts:
        secs = secs * 60 + p
    return secs + (int(days) * 86400 if days else 0)


def sample(pid, log, config):
    ps = run("ps", "-o", "rss=,time=", "-p", str(pid)).split()
    if len(ps) < 2:
        return None
    top = run("top", "-l", "1", "-pid", str(pid), "-stats", "pid,csw,th")
    m = re.search(rf"^\s*{pid}\s+(\d+)\S*\s+(\d+)", top, re.M)
    csw, threads = (int(m[1]), int(m[2])) if m else (-1, -1)
    files = max(0, len(run("lsof", "-p", str(pid)).splitlines()) - 1)
    polls = 0
    if os.path.exists(log):
        with open(log, errors="replace") as f:
            polls = sum(1 for line in f if " reason=" in line)
    assertions = run("pmset", "-g", "assertions").count(f"pid {pid}(")
    mtime = time.strftime("%H:%M:%S", time.localtime(os.path.getmtime(config))) if os.path.exists(config) else "-"
    return int(ps[0]), cpu_seconds(ps[1]), csw, threads, files, polls, assertions, mtime


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True)
    ap.add_argument("--hours", type=float, default=6)
    ap.add_argument("--every", type=float, default=300, help="seconds between samples")
    ap.add_argument("--out", required=True)
    ap.add_argument("--config", default=os.path.expanduser("~/.config/battbar-dev/config.toml"))
    a = ap.parse_args()
    app = os.path.abspath(a.app)
    exe = os.path.realpath(f"{app}/Contents/MacOS/battbar-app")
    bid = plist(app, "CFBundleIdentifier")
    log = a.out + ".log"
    subprocess.run(["osascript", "-e", f'tell application id "{bid}" to quit'], capture_output=True)
    time.sleep(2)
    subprocess.run(["open", "--env", "BATTBAR_LOG=1", "--stderr", log, app], check=True)
    pid = None
    for _ in range(100):
        out = run("pgrep", "-f", exe).split()
        if out:
            pid = int(out[0])
            break
        time.sleep(0.1)
    if not pid:
        sys.exit("app did not start")
    t0 = time.time()
    with open(a.out, "w") as f:
        f.write(f"# {app} pid {pid}, started {time.strftime('%Y-%m-%d %H:%M:%S')}\n")
        f.write("elapsed_min\tclock\trss_kb\tcpu_s\tcsw\tthreads\tfiles\tpolls\tassertions\tconfig_mtime\n")
        while time.time() - t0 <= a.hours * 3600:
            s = sample(pid, log, a.config)
            if s is None:
                f.write(f"# app exited at {time.strftime('%H:%M:%S')}\n")
                break
            row = [f"{(time.time() - t0) / 60:.0f}", time.strftime("%H:%M:%S"), *map(str, s)]
            f.write("\t".join(row) + "\n")
            f.flush()
            time.sleep(a.every)


if __name__ == "__main__":
    main()
