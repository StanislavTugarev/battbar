// SPDX-License-Identifier: GPL-3.0-or-later
//! The faster paths are only an acceleration: a base station whose events are not
//! confirmed (here the Xbox variant from the device database) gets no listener and is
//! read by the regular poll; a base that appears without a hotplug notification is
//! found by the safety re-scan. Own test binary: the event callback is global.

use battbar_core::transport::{FrameTransport, LinkInfo, TransportError};
use battbar_core::{BatteryProvider, DeviceDb, Status};
use battbar_steelseries::{events, BaseLink, BaseSource, SteelSeriesProvider};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn info(pid: u16) -> LinkInfo {
    LinkInfo {
        link: format!("hid:1038:{pid:04x}:4:ffc0"),
        kind: "hidapi".into(),
        vendor_id: 0x1038,
        product_id: pid,
        product: "Base".into(),
        bus: "usb".into(),
        collections: vec![(0xffc0, 1)],
    }
}

/// A base that answers the status request (06 b0) like the reference base: headset on,
/// full, spare 4/8.
struct Answering {
    info: LinkInfo,
    pending: VecDeque<Vec<u8>>,
    requests: Arc<AtomicUsize>,
}

impl FrameTransport for Answering {
    fn info(&self) -> &LinkInfo {
        &self.info
    }
    fn write(&mut self, f: &[u8]) -> Result<(), TransportError> {
        if f.starts_with(&[0x06, 0xb0]) {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let mut r = battbar_core::transport::from_hex("06 b0 00 00 01 00 08 04 08 00 02 0a 00 00 08 08").unwrap();
            r.resize(64, 0);
            self.pending.push_back(r);
        }
        Ok(())
    }
    fn read(&mut self, _: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        Ok(self.pending.pop_front())
    }
}

struct Bases {
    db: Arc<DeviceDb>,
    pids: Arc<Mutex<Vec<u16>>>,
    requests: Arc<AtomicUsize>,
    listeners: Arc<AtomicUsize>,
}

impl BaseSource for Bases {
    fn scan(&mut self, _: &DeviceDb) -> Vec<BaseLink> {
        self.pids
            .lock()
            .unwrap()
            .iter()
            .map(|&pid| {
                let (requests, listeners) = (self.requests.clone(), self.listeners.clone());
                BaseLink {
                    id: info(pid).link,
                    device: self.db.usb_device(0x1038, pid).unwrap().clone(),
                    open: Box::new(move || {
                        Ok(Box::new(Answering {
                            info: info(pid),
                            pending: VecDeque::new(),
                            requests,
                        }) as Box<dyn FrameTransport>)
                    }),
                    listen: Some(Box::new(move || {
                        listeners.fetch_add(1, Ordering::SeqCst);
                        Err(TransportError::Io("not expected".into()))
                    })),
                }
            })
            .collect()
    }
}

/// The provider, the PIDs its fake source reports, status requests, listeners opened.
type Setup = (SteelSeriesProvider, Arc<Mutex<Vec<u16>>>, Arc<AtomicUsize>, Arc<AtomicUsize>);

fn provider(pids: Vec<u16>) -> Setup {
    events::on_event(|| {});
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let pids = Arc::new(Mutex::new(pids));
    let (requests, listeners) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let mut p = SteelSeriesProvider::new(
        db.clone(),
        vec![Box::new(Bases {
            db,
            pids: pids.clone(),
            requests: requests.clone(),
            listeners: listeners.clone(),
        })],
    );
    p.set_headsetcontrol(false);
    (p, pids, requests, listeners)
}

#[test]
fn a_base_without_confirmed_events_is_read_by_the_poll_and_gets_no_listener() {
    let db = DeviceDb::embedded().unwrap();
    assert!(db.usb_device(0x1038, 0x12e0).unwrap().base_events, "reference base: confirmed");
    assert!(!db.usb_device(0x1038, 0x12e5).unwrap().base_events, "Xbox variant: not confirmed");

    let (mut p, _, requests, listeners) = provider(vec![0x12e5]);
    for _ in 0..3 {
        let r = p.poll();
        let headset = r.iter().find(|r| r.provider == "steelseries").expect("headset");
        assert_eq!(headset.reading.status, Status::Ok);
        assert_eq!(headset.reading.percent, Some(100));
    }
    assert_eq!(requests.load(Ordering::SeqCst), 3, "every poll reads the status");
    assert_eq!(listeners.load(Ordering::SeqCst), 0, "no listener for unconfirmed events");
}

#[test]
fn a_base_plugged_in_without_a_notification_is_found_by_the_safety_rescan() {
    let (mut p, pids, _, _) = provider(vec![]);
    p.set_rescan_interval(Duration::from_secs(300));
    let t0 = Instant::now();
    let _ = p.poll();
    pids.lock().unwrap().push(0x12e5); // plugged in; the hotplug notification is lost
    for s in (2..300).step_by(2) {
        assert!(
            !p.check_events_at(t0 + Duration::from_secs(s)),
            "noticed without a re-scan at {s} s"
        );
    }
    assert!(p.check_events_at(t0 + Duration::from_secs(301)), "the safety re-scan finds it");
    let r = p.poll();
    assert!(r.iter().any(|r| r.provider == "steelseries" && r.reading.percent == Some(100)));
}
