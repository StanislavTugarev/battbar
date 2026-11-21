// SPDX-License-Identifier: GPL-3.0-or-later
//! Headset events reported by the base station itself: a listener on its own handle
//! calls back for headset reports only, one listener per base, ending with the base.
//! Own test binary: the callback is global.

use battbar_core::transport::{FrameTransport, LinkInfo, TransportError};
use battbar_core::{BatteryProvider, DeviceDb};
use battbar_steelseries::{events, BaseLink, BaseSource, SteelSeriesProvider};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Scripted reads of the listener's handle.
type Reads = Arc<Mutex<VecDeque<Result<Vec<u8>, TransportError>>>>;

fn info() -> LinkInfo {
    LinkInfo {
        link: "hid:1038:12e0:4:ffc0".into(),
        kind: "hidapi".into(),
        vendor_id: 0x1038,
        product_id: 0x12e0,
        product: "Base".into(),
        bus: "usb".into(),
        collections: vec![(0xffc0, 1)],
    }
}

fn frame(b: &[u8]) -> Vec<u8> {
    let mut v = b.to_vec();
    v.resize(64, 0);
    v
}

/// The listener's handle: hands out scripted reports, then "unplugged".
struct Script {
    info: LinkInfo,
    reads: Reads,
    gate: Arc<Mutex<bool>>,
}

impl FrameTransport for Script {
    fn info(&self) -> &LinkInfo {
        &self.info
    }
    fn write(&mut self, _: &[u8]) -> Result<(), TransportError> {
        panic!("the listener never sends");
    }
    fn read(&mut self, _: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        // Block until the test opens the gate (like a blocking read in silence).
        while !*self.gate.lock().unwrap() {
            std::thread::sleep(Duration::from_millis(5));
        }
        match self.reads.lock().unwrap().pop_front() {
            Some(r) => r.map(Some),
            None => Err(TransportError::Disconnected),
        }
    }
}

/// The provider's own handle: answers nothing (the poll reports an error, fine here).
struct Mute(LinkInfo);
impl FrameTransport for Mute {
    fn info(&self) -> &LinkInfo {
        &self.0
    }
    fn write(&mut self, _: &[u8]) -> Result<(), TransportError> {
        Ok(())
    }
    fn read(&mut self, _: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        Ok(None)
    }
}

struct OneBase {
    db: Arc<DeviceDb>,
    reads: Reads,
    gate: Arc<Mutex<bool>>,
    listeners_opened: Arc<AtomicUsize>,
}

impl BaseSource for OneBase {
    fn scan(&mut self, _: &DeviceDb) -> Vec<BaseLink> {
        let (reads, gate, opened) = (self.reads.clone(), self.gate.clone(), self.listeners_opened.clone());
        vec![BaseLink {
            id: info().link,
            device: self.db.usb_device(0x1038, 0x12e0).unwrap().clone(),
            open: Box::new(|| Ok(Box::new(Mute(info())) as Box<dyn FrameTransport>)),
            listen: Some(Box::new(move || {
                opened.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Script { info: info(), reads, gate }) as Box<dyn FrameTransport>)
            })),
        }]
    }
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let t0 = Instant::now();
    while !f() {
        assert!(t0.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn headset_reports_call_back_once_each_and_the_listener_ends_with_the_base() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    events::on_event(move || {
        c.fetch_add(1, Ordering::SeqCst);
    });
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let reads = Arc::new(Mutex::new(VecDeque::from([
        Ok(frame(&[0x07, 0x25, 0x19])),             // volume dial: not headset state
        Ok(frame(&[0x07, 0xb7, 0x00, 0x08, 0x01])), // headset off
        Ok(frame(&[0x07, 0xb5, 0x04, 0x01, 0x08])), // Bluetooth connected
    ])));
    let gate = Arc::new(Mutex::new(false));
    let opened = Arc::new(AtomicUsize::new(0));
    let mut p = SteelSeriesProvider::new(
        db.clone(),
        vec![Box::new(OneBase {
            db,
            reads: reads.clone(),
            gate: gate.clone(),
            listeners_opened: opened.clone(),
        })],
    );
    p.set_headsetcontrol(false);
    let _ = p.poll();
    wait_for("the listener", || opened.load(Ordering::SeqCst) == 1);
    // While the base is quiet the listener blocks: no callback, and a second poll
    // (link already open) starts no second listener.
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let _ = p.poll();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(opened.load(Ordering::SeqCst), 1, "one listener per base");
    // The headset changes: one callback per headset report, none for the volume dial.
    *gate.lock().unwrap() = true;
    wait_for("two callbacks", || calls.load(Ordering::SeqCst) == 2);
    // Then the base is gone (read error): the listener has ended.
    wait_for("the listener to end", || reads.lock().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(!p.check_events(), "the 2 s check no longer asks the base anything");
}
