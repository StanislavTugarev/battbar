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
