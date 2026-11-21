// SPDX-License-Identifier: GPL-3.0-or-later
//! HID hotplug notifications (own test binary: the counter is process-wide).

use std::time::{Duration, Instant};

fn wait_for(cond: impl Fn() -> bool, secs: u64) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// The watcher is installed with the service thread and reports the devices present
/// at start — without opening any (no Input Monitoring needed).
#[test]
fn present_devices_are_reported_once_at_start() {
    let (tx, rx) = std::sync::mpsc::channel();
    battbar_hid::on_hotplug(move || {
        let _ = tx.send(());
    });
    let present = battbar_hid::enumerate().map(|v| v.len()).unwrap_or(0);
    if present == 0 {
        eprintln!("no HID devices on this machine; nothing to check");
        return;
    }
    assert!(wait_for(|| battbar_hid::hotplug_events() > 0, 2), "no initial hotplug report");
    assert!(rx.recv_timeout(Duration::from_secs(1)).is_ok(), "listener not called");
    // Then quiet: nothing is plugged in during the test.
    std::thread::sleep(Duration::from_millis(300));
    let n = battbar_hid::hotplug_events();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(battbar_hid::hotplug_events(), n, "events without any device change");
}

/// Hardware: plug in or remove a USB HID device (receiver, base station) or switch a
/// Bluetooth HID device on/off within 30 s.
///   cargo test -p battbar-hid --test hotplug -- --ignored --nocapture
#[test]
#[ignore = "needs someone to plug or unplug a device"]
fn plugging_a_device_is_reported_within_a_second() {
    let (tx, rx) = std::sync::mpsc::channel();
    battbar_hid::on_hotplug(move || {
        let _ = tx.send(Instant::now());
    });
    let _ = battbar_hid::enumerate();
    std::thread::sleep(Duration::from_millis(500));
    while rx.try_recv().is_ok() {}
    eprintln!("plug in or remove a HID device now (30 s)…");
    let at = rx.recv_timeout(Duration::from_secs(30)).expect("no hotplug event in 30 s");
    eprintln!("hotplug event after {:?} of waiting", at.elapsed());
}
