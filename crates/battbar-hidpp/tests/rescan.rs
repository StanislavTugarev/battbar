// SPDX-License-Identifier: GPL-3.0-or-later
//! Hotplug safety net: without a notification, a new link is found only by the rare
//! re-scan (0.5: notifications trigger polls; the re-scan catches a missed one).

use battbar_core::transport::TransportError;
use battbar_core::{BatteryProvider, DeviceDb, Transport};
use battbar_hidpp::{HidppProvider, LinkCandidate, LinkRole, LinkSource};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct FakeSource {
    ids: Arc<Mutex<Vec<String>>>,
    scans: Arc<AtomicUsize>,
}

impl LinkSource for FakeSource {
    fn scan(&mut self, _db: &DeviceDb) -> Vec<LinkCandidate> {
        self.scans.fetch_add(1, Ordering::Relaxed);
        self.ids
            .lock()
            .unwrap()
            .iter()
            .map(|id| LinkCandidate {
                id: id.clone(),
                role: LinkRole::Direct { transport: Transport::Usb },
                long_only: false,
                identity: vec![],
                open: Box::new(|| Err(TransportError::Io("fake link".into()))),
            })
            .collect()
    }
}

#[test]
fn a_missed_hotplug_is_caught_by_the_safety_rescan_only() {
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let ids = Arc::new(Mutex::new(vec![]));
    let scans = Arc::new(AtomicUsize::new(0));
    let mut p = HidppProvider::new(
        db,
        vec![Box::new(FakeSource {
            ids: ids.clone(),
            scans: scans.clone(),
        })],
    );
    p.set_rescan_interval(Duration::from_secs(300));
    let t0 = Instant::now();
    // A receiver is plugged in, but its notification was missed.
    ids.lock().unwrap().push("hid:046d:c547:2:ff00".into());
    // Checks every 2 s for almost five minutes: no scan, nothing noticed.
    for s in (2..300).step_by(2) {
        assert!(
            !p.check_events_at(t0 + Duration::from_secs(s)),
            "noticed without a re-scan at {s} s"
        );
    }
    assert_eq!(scans.load(Ordering::Relaxed), 0, "no periodic scans between safety re-scans");
    // The safety re-scan finds the new link and asks for a poll.
    assert!(p.check_events_at(t0 + Duration::from_secs(301)));
    assert_eq!(scans.load(Ordering::Relaxed), 1);
    // Next check: no re-scan until another interval has passed.
    assert!(!p.check_events_at(t0 + Duration::from_secs(303)));
    assert_eq!(scans.load(Ordering::Relaxed), 1);
    // A poll scans and opens (here: fails to open the fake link, so it stays unknown).
    let _ = p.poll();
    assert_eq!(scans.load(Ordering::Relaxed), 2);
}

#[test]
fn an_unchanged_link_set_does_not_ask_for_a_poll() {
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let scans = Arc::new(AtomicUsize::new(0));
    let mut p = HidppProvider::new(
        db,
        vec![Box::new(FakeSource {
            ids: Arc::new(Mutex::new(vec![])),
            scans: scans.clone(),
        })],
    );
    p.set_rescan_interval(Duration::from_secs(300));
    let t0 = Instant::now();
    assert!(!p.check_events_at(t0 + Duration::from_secs(301)));
    assert_eq!(scans.load(Ordering::Relaxed), 1);
}
