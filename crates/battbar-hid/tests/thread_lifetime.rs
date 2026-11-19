// SPDX-License-Identifier: GPL-3.0-or-later
//! Regression: hidapi (macOS) schedules its IOHIDManager on the run loop of the thread
//! that first initialised it. If that thread exits (e.g. the app rebuilds its providers
//! when a setting changes) and another thread enumerates again, IOKit used to touch the
//! freed run loop and the process crashed (EXC_BREAKPOINT in CFRunLoopAddSource).
//! Enumeration only; nothing is opened.

#[test]
fn enumerate_after_the_initialising_thread_exited() {
    let first = std::thread::spawn(|| battbar_hid::enumerate().map(|v| v.len()));
    let n = first.join().expect("first thread");
    // The first thread (and its run loop) is gone now. Churn threads and memory so a
    // freed run loop would be reused, as in the app after its providers were rebuilt.
    for _ in 0..200 {
        let again = std::thread::spawn(|| {
            let junk: Vec<Vec<u8>> = (0..64).map(|i| vec![i as u8; 4096]).collect();
            std::hint::black_box(&junk);
            battbar_hid::enumerate().map(|v| v.len())
        })
        .join()
        .expect("later thread");
        assert_eq!(again.is_ok(), n.is_ok());
    }
}

/// The HID service thread services its run loop (IOKit events on the manager are
/// handled, not queued forever).
#[test]
fn service_thread_runs_its_run_loop() {
    let _ = battbar_hid::enumerate();
    let t0 = battbar_hid::service_run_loop_ticks();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(battbar_hid::service_run_loop_ticks() >= t0 + 3, "run loop not serviced");
}
