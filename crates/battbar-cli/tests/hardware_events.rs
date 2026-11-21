// SPDX-License-Identifier: GPL-3.0-or-later
//! Hardware: hotplug notifications arrive for USB HID devices and Bluetooth devices.
//!   cargo test -p battbar-cli --test hardware_events -- --ignored --nocapture
//! While it runs (40 s): unplug and plug back a receiver or base station, and turn a
//! Bluetooth headset (classic) off and on.

use std::process::Command;

/// Time stamp of an output line like `[ 12.34s] …`.
fn at(line: &str) -> Option<f64> {
    line.strip_prefix('[')?.split("s]").next()?.trim().parse().ok()
}

#[test]
#[ignore = "needs someone to plug devices and switch Bluetooth"]
fn usb_and_bluetooth_hotplug_are_reported() {
    eprintln!("Within 40 s: unplug + replug a USB receiver/base station, and turn a Bluetooth headset off + on.");
    let out = Command::new(env!("CARGO_BIN_EXE_battbar"))
        .args(["debug-events", "--seconds", "40"])
        .output()
        .expect("run battbar debug-events");
    let text = String::from_utf8_lossy(&out.stdout);
    eprintln!("{text}");
    // Events after the initial report of the devices present at start (first second).
    let later = |what: &str| text.lines().any(|l| l.contains(what) && at(l).is_some_and(|t| t > 1.0));
    assert!(later("hid device"), "no HID hotplug event after start");
    assert!(later("bluetooth device"), "no Bluetooth connect/disconnect event after start");
}

#[test]
fn time_stamps_are_parsed() {
    assert_eq!(at("[  3.25s] hid device appeared or disappeared"), Some(3.25));
    assert_eq!(at("listening for 40 s"), None);
}
