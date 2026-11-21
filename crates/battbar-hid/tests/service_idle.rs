// SPDX-License-Identifier: GPL-3.0-or-later
//! Own test binary (own process): the wake-up counter must not see other tests' jobs.

/// The HID service thread sleeps when idle (0.3 woke every 20 ms) and a job sent while
/// it sleeps wakes it.
#[test]
fn service_thread_sleeps_when_idle_and_wakes_for_a_job() {
    let _ = battbar_hid::enumerate();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let t0 = battbar_hid::service_run_loop_ticks();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let t1 = battbar_hid::service_run_loop_ticks();
    assert!(t1 <= t0 + 1, "idle service thread woke up {} times in 0.5 s", t1 - t0);
    // Now it is waiting in its run loop: a job must still be run (no lost wake-up).
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(battbar_hid::enumerate().is_ok());
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("job sent to the sleeping service thread did not run");
    assert!(battbar_hid::service_run_loop_ticks() > t1, "the service thread woke up for the job");
}
