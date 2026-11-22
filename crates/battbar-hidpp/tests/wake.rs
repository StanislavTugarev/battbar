// SPDX-License-Identifier: GPL-3.0-or-later
//! A receiver device that connects or wakes up is polled at the next event check,
//! once, even when the receiver does not announce it with 0x41 (seen on the reference
//! Lightspeed receiver after plugging it in and when the mouse wakes up): disconnected
//! slots are pinged in the check, and the first frame of a device that was off line
//! counts as an event.
//!
//! The receiver answers from a trace recorded on the reference hardware (slot 1:
//! keyboard away, slot 2: mouse on line); the test feeds extra incoming frames, can
//! take a slot off line (its pings then fail like the away keyboard's), make a slot
//! answer pings, or make it silent (asleep), and counts the pings per slot.

use battbar_core::transport::{FrameTransport, LinkInfo, ReplayTransport, Trace, TransportError};
use battbar_core::*;
use battbar_hidpp::provider::BoxTransport;
use battbar_hidpp::{HidppProvider, LinkCandidate, LinkRole, LinkSource};
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TRACE: &str = "lightspeed-g502xplus-g915away_base-novapro.jsonl";
const RECEIVER: &str = "hid:046d:c547:2:ff00";
const KEYBOARD: u8 = 1;
const MOUSE: u8 = 2;

#[derive(Default)]
struct Control {
    incoming: VecDeque<Vec<u8>>,
    off_line: BTreeSet<u8>,
    /// Slots that answer a ping (HID++ 4.2), whatever the trace says.
    answering: BTreeSet<u8>,
    /// Slots whose pings get no answer at all (a device asleep).
    silent: BTreeSet<u8>,
    /// Slots whose pings get this frame instead (a receiver that answers differently).
    odd: std::collections::BTreeMap<u8, Vec<u8>>,
    pings: Vec<u8>,
}

impl Control {
    fn pings_of(&self, idx: u8) -> usize {
        self.pings.iter().filter(|i| **i == idx).count()
    }
}

struct FakeReceiver {
    inner: ReplayTransport,
    ctl: Arc<Mutex<Control>>,
    replies: VecDeque<Vec<u8>>,
}

impl FrameTransport for FakeReceiver {
    fn info(&self) -> &LinkInfo {
        self.inner.info()
    }
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        // HID++ 2.0 ping (root feature, function 1) to a slot that is off line: the
        // receiver answers "unknown device", as for the away keyboard in the trace.
        let ping = frame.len() >= 4 && frame[2] == 0x00 && frame[3] >> 4 == 1;
        if ping {
            let mut ctl = self.ctl.lock().unwrap();
            ctl.pings.push(frame[1]);
            if ctl.silent.contains(&frame[1]) {
                return Ok(());
            }
            if let Some(f) = ctl.odd.get(&frame[1]) {
                self.replies.push_back(f.clone());
                return Ok(());
            }
            if ctl.off_line.contains(&frame[1]) {
                self.replies.push_back(vec![0x10, frame[1], 0x8F, 0x00, frame[3], 0x08, 0x00]);
                return Ok(());
            }
            if ctl.answering.contains(&frame[1]) {
                let mut r = vec![0x11, frame[1], 0x00, frame[3], 0x04, 0x02, 0xAA];
                r.resize(20, 0);
                self.replies.push_back(r);
                return Ok(());
            }
        }
        self.inner.write(frame)
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        if let Some(f) = self.replies.pop_front() {
            return Ok(Some(f));
        }
        if let Some(f) = self.ctl.lock().unwrap().incoming.pop_front() {
            return Ok(Some(f));
        }
        self.inner.read(timeout)
    }
}

struct OneReceiver {
    db: Arc<DeviceDb>,
    ctl: Arc<Mutex<Control>>,
}

impl LinkSource for OneReceiver {
    fn scan(&mut self, _db: &DeviceDb) -> Vec<LinkCandidate> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../traces").join(TRACE);
        let trace = Trace::from_file(&p).unwrap();
        let info = trace.links.iter().find(|l| l.link == RECEIVER).unwrap().clone();
        let def = self.db.receiver(info.vendor_id, info.product_id).unwrap().clone();
        let ctl = self.ctl.clone();
        vec![LinkCandidate {
            id: RECEIVER.into(),
            role: LinkRole::Receiver(def),
            long_only: false,
            identity: vec![],
            open: Box::new(move || {
                let t: BoxTransport = Box::new(FakeReceiver {
                    inner: ReplayTransport::new(&trace, RECEIVER),
                    ctl,
                    replies: VecDeque::new(),
                });
                Ok(t)
            }),
        }]
    }
}

fn setup() -> (HidppProvider, Arc<Mutex<Control>>) {
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let ctl = Arc::new(Mutex::new(Control::default()));
    let mut p = HidppProvider::new(db.clone(), vec![Box::new(OneReceiver { db, ctl: ctl.clone() })]);
    p.set_rescan_interval(Duration::from_secs(3600));
    (p, ctl)
}

fn status(reports: &[DeviceReport], kind: DeviceKind) -> Status {
    reports.iter().find(|r| r.kind == kind).expect("device").reading.status
}

/// A frame of a feature notification from a device (long report, feature index 0x0D:
/// what the mouse sends while it moves).
fn feature_frame(idx: u8) -> Vec<u8> {
    let mut f = vec![0x11, idx, 0x0D, 0x00, 0x00, 0x01];
    f.resize(20, 0);
    f
}

/// Run the 2 s event checks for a while; count those that asked for a poll.
fn checks_asking(p: &mut HidppProvider, t0: Instant) -> usize {
    (1..=5).filter(|i| p.check_events_at(t0 + Duration::from_secs(2 * i))).count()
}

#[test]
fn first_frame_of_an_absent_device_asks_for_one_poll() {
    let (mut p, ctl) = setup();
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Keyboard), Status::Disconnected);
    assert_eq!(status(&r, DeviceKind::Mouse), Status::Ok);
    let t0 = Instant::now();
    assert_eq!(checks_asking(&mut p, t0), 0, "quiet receiver");

    // The away keyboard connects by radio; the receiver sends no 0x41, only the
    // keyboard's own frames arrive.
    ctl.lock().unwrap().incoming.push_back(feature_frame(KEYBOARD));
    assert!(p.check_events_at(t0 + Duration::from_secs(12)), "one poll for the keyboard");

    // More frames from it: nothing more (still before the poll, and after a poll that
    // did not see it on line: the replayed receiver keeps reporting it away).
    ctl.lock()
        .unwrap()
        .incoming
        .extend([feature_frame(KEYBOARD), feature_frame(KEYBOARD)]);
    assert_eq!(checks_asking(&mut p, t0 + Duration::from_secs(12)), 0);
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Keyboard), Status::Disconnected);
    ctl.lock()
        .unwrap()
        .incoming
        .extend([feature_frame(KEYBOARD), feature_frame(KEYBOARD)]);
    assert_eq!(checks_asking(&mut p, t0 + Duration::from_secs(30)), 0, "one poll per connection");
}

#[test]
fn frames_of_a_device_on_line_ask_for_nothing() {
    let (mut p, ctl) = setup();
    let _ = p.poll();
    let t0 = Instant::now();
    for _ in 0..50 {
        ctl.lock().unwrap().incoming.push_back(feature_frame(MOUSE));
    }
    assert_eq!(checks_asking(&mut p, t0), 0, "a moving mouse that is on line");
}

#[test]
fn a_device_that_went_away_and_wakes_up_asks_again() {
    let (mut p, ctl) = setup();
    let _ = p.poll();
    let t0 = Instant::now();
    // The mouse goes off line; the receiver says so with 0x41 "link not established"
    // (flags bit 0x40): a poll, and the mouse is shown disconnected.
    ctl.lock().unwrap().off_line.insert(MOUSE);
    ctl.lock()
        .unwrap()
        .incoming
        .push_back(vec![0x10, MOUSE, 0x41, 0x11, 0x72, 0x99, 0x40]);
    assert!(p.check_events_at(t0 + Duration::from_secs(2)));
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Mouse), Status::Disconnected);

    // It comes back without a 0x41: its first frame asks for one poll, which shows it.
    ctl.lock().unwrap().off_line.clear();
    ctl.lock().unwrap().incoming.extend([feature_frame(MOUSE), feature_frame(MOUSE)]);
    assert!(p.check_events_at(t0 + Duration::from_secs(4)));
    assert!(!p.check_events_at(t0 + Duration::from_secs(6)));
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Mouse), Status::Ok);
    // On line again: its frames ask for nothing.
    ctl.lock().unwrap().incoming.push_back(feature_frame(MOUSE));
    assert!(!p.check_events_at(t0 + Duration::from_secs(8)));
}

#[test]
fn a_disconnected_slot_is_pinged_and_absent_gives_nothing() {
    let (mut p, ctl) = setup();
    let _ = p.poll();
    let before = ctl.lock().unwrap().pings_of(KEYBOARD);
    let t0 = Instant::now();
    // The receiver keeps answering "unknown device" for the away keyboard.
    assert_eq!(checks_asking(&mut p, t0), 0);
    assert_eq!(ctl.lock().unwrap().pings_of(KEYBOARD) - before, 5, "one ping per check");
    assert_eq!(ctl.lock().unwrap().pings_of(MOUSE), 1, "the mouse on line: only the poll's ping");
}

#[test]
fn a_disconnected_slot_that_answers_gets_one_poll() {
    let (mut p, ctl) = setup();
    let _ = p.poll();
    let t0 = Instant::now();
    // The keyboard reaches the receiver silently (no 0x41, no frame): the next check's
    // ping finds it.
    ctl.lock().unwrap().answering.insert(KEYBOARD);
    assert!(p.check_events_at(t0 + Duration::from_secs(2)));
    let pings = ctl.lock().unwrap().pings_of(KEYBOARD);
    assert_eq!(checks_asking(&mut p, t0 + Duration::from_secs(2)), 0, "one poll, not one per check");
    assert_eq!(ctl.lock().unwrap().pings_of(KEYBOARD), pings, "no more pings before the poll");
}

#[test]
fn an_asleep_slot_is_not_pinged() {
    let (mut p, ctl) = setup();
    ctl.lock().unwrap().silent.insert(MOUSE);
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Mouse), Status::Asleep);
    let pings = ctl.lock().unwrap().pings_of(MOUSE);
    assert_eq!(checks_asking(&mut p, Instant::now()), 0);
    assert_eq!(ctl.lock().unwrap().pings_of(MOUSE), pings, "asleep: waits for its first frame");
    // Waking up, it speaks: one poll.
    ctl.lock().unwrap().incoming.push_back(feature_frame(MOUSE));
    assert!(p.check_events_at(Instant::now() + Duration::from_secs(12)));
}

/// Take the mouse off line and poll, so its slot is "disconnected".
fn mouse_gone(p: &mut HidppProvider, ctl: &Arc<Mutex<Control>>) {
    ctl.lock().unwrap().off_line.insert(MOUSE);
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Mouse), Status::Disconnected);
    ctl.lock().unwrap().off_line.clear();
}

/// A receiver that answers the ping of an absent device differently from the reference
/// Lightspeed (another HID++ 1.0 error, a reply of another format, no answer): no
/// poll storm, nothing breaks, and the device is found by the regular poll.
#[test]
fn other_ping_answers_break_nothing_and_the_poll_still_finds_the_device() {
    let (mut p, ctl) = setup();
    let _ = p.poll();
    mouse_gone(&mut p, &ctl);
    let t0 = Instant::now();
    // HID++ 1.0 errors other than "unknown device": ignored, as "absent".
    for err in [
        0x09, /* resource error */
        0x01, /* invalid sub-ID */
        0x02, /* invalid address */
    ] {
        ctl.lock()
            .unwrap()
            .odd
            .insert(MOUSE, vec![0x10, MOUSE, 0x8F, 0x00, 0x1B, err, 0x00]);
        assert_eq!(checks_asking(&mut p, t0), 0, "error 0x{err:02x}");
    }
    // A reply in another format (here: another software ID): not ours, so no answer in
    // time; that counts as "maybe there": one poll, then the slot is not pinged again
    // until a poll sees the device on line.
    let mut foreign = vec![0x11, MOUSE, 0x00, 0x2B, 0x04, 0x02, 0xAA];
    foreign.resize(20, 0);
    ctl.lock().unwrap().odd.insert(MOUSE, foreign);
    assert_eq!(checks_asking(&mut p, t0 + Duration::from_secs(20)), 1);
    let pings = ctl.lock().unwrap().pings_of(MOUSE);
    assert_eq!(checks_asking(&mut p, t0 + Duration::from_secs(40)), 0);
    assert_eq!(ctl.lock().unwrap().pings_of(MOUSE), pings, "no ping loop");
    // The device comes back: the regular poll shows it whatever the check did.
    ctl.lock().unwrap().odd.clear();
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Mouse), Status::Ok);
    assert_eq!(checks_asking(&mut p, t0 + Duration::from_secs(60)), 0);
}

/// A receiver that does announce a connection with 0x41 ("link established", flag 0x40
/// clear): one poll, not one for the 0x41 and another for the ping or the device's
/// first frames.
#[test]
fn a_connection_announced_with_0x41_gives_one_poll() {
    let (mut p, ctl) = setup();
    let _ = p.poll();
    mouse_gone(&mut p, &ctl);
    let t0 = Instant::now();
    ctl.lock()
        .unwrap()
        .incoming
        .extend([vec![0x10, MOUSE, 0x41, 0x11, 0x32, 0x99, 0x40], feature_frame(MOUSE)]);
    assert!(p.check_events_at(t0 + Duration::from_secs(2)), "0x41 asks for a poll");
    // The worker polls right away; the device is on line.
    let r = p.poll();
    assert_eq!(status(&r, DeviceKind::Mouse), Status::Ok);
    // Its further frames, and checks, ask for nothing more.
    ctl.lock().unwrap().incoming.extend([feature_frame(MOUSE), feature_frame(MOUSE)]);
    assert_eq!(checks_asking(&mut p, t0 + Duration::from_secs(2)), 0);
}
