// SPDX-License-Identifier: GPL-3.0-or-later
//! Provider trait and the polling engine.
//!
//! Every provider runs on its own worker thread, so one hung device or link never
//! blocks the others: `Engine::poll` waits up to a deadline and reports which
//! providers did not answer in time.

use crate::model::DeviceReport;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// How often providers re-scan for links on their own, as a safety net: since 0.5
/// hotplug notifications (IOHIDManager, IOBluetooth) trigger the checks; this only
/// catches an event that was missed.
pub const SAFETY_RESCAN: Duration = Duration::from_secs(300);

/// A rare re-scan, counted from the last scan (every poll scans anyway).
#[derive(Debug, Clone)]
pub struct Rescan {
    pub every: Duration,
    last: Instant,
}

impl Rescan {
    pub fn new(every: Duration) -> Self {
        Rescan {
            every,
            last: Instant::now(),
        }
    }

    /// Time for a safety re-scan (then counts from `now`).
    pub fn due(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.last) >= self.every {
            self.last = now;
            return true;
        }
        false
    }

    /// A scan just happened (a poll).
    pub fn scanned(&mut self, now: Instant) {
        self.last = now;
    }
}

impl Default for Rescan {
    fn default() -> Self {
        Rescan::new(SAFETY_RESCAN)
    }
}

/// Called by a provider when it learns that something changed (push notification,
/// hotplug); the front-end usually reacts with a new poll.
pub type EventSink = Arc<dyn Fn(&str) + Send + Sync>;

pub trait BatteryProvider: Send {
    /// Short stable name, e.g. `hidpp`.
    fn name(&self) -> &'static str;

    /// Find devices (handles hotplug) and return a report for each of them.
    fn poll(&mut self) -> Vec<DeviceReport>;

    /// Diagnostic information for `battbar dump` (no user data).
    fn diagnostics(&mut self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Optional push notifications.
    fn set_event_sink(&mut self, _sink: EventSink) {}

    /// Cheap check between polls: did something happen that warrants a poll now
    /// (device notification, hotplug)? Must not block for long.
    fn check_events(&mut self) -> bool {
        false
    }
}

enum Cmd {
    Poll,
    Diagnostics,
    CheckEvents,
}

enum Reply {
    Reports(Vec<DeviceReport>),
    Diagnostics(serde_json::Value),
    Events(bool),
}

struct Worker {
    name: &'static str,
    tx: Sender<Cmd>,
    rx: Receiver<Reply>,
    /// Outstanding requests not answered yet (a hung provider).
    outstanding: usize,
}

pub struct Engine {
    workers: Vec<Worker>,
}

#[derive(Debug, Default)]
pub struct PollResult {
    pub reports: Vec<DeviceReport>,
    /// Providers that did not answer before the deadline.
    pub timed_out: Vec<&'static str>,
}

impl Engine {
    pub fn new(providers: Vec<Box<dyn BatteryProvider>>) -> Self {
        let workers = providers
            .into_iter()
            .map(|mut p| {
                let name = p.name();
                let (tx, crx) = mpsc::channel::<Cmd>();
                let (rtx, rx) = mpsc::channel::<Reply>();
                thread::Builder::new()
                    .name(format!("battbar-{name}"))
                    .spawn(move || {
                        while let Ok(cmd) = crx.recv() {
                            let reply = match cmd {
                                Cmd::Poll => Reply::Reports(p.poll()),
                                Cmd::Diagnostics => Reply::Diagnostics(p.diagnostics()),
                                Cmd::CheckEvents => Reply::Events(p.check_events()),
                            };
                            if rtx.send(reply).is_err() {
                                break;
                            }
                        }
                    })
                    .expect("spawn provider thread");
                Worker {
                    name,
                    tx,
                    rx,
                    outstanding: 0,
                }
            })
            .collect();
        Engine { workers }
    }

    pub fn provider_names(&self) -> Vec<&'static str> {
        self.workers.iter().map(|w| w.name).collect()
    }

    fn drain_stale(w: &mut Worker) {
        while w.outstanding > 0 {
            match w.rx.try_recv() {
                Ok(_) => w.outstanding -= 1,
                Err(_) => break,
            }
        }
    }

    pub fn poll(&mut self, timeout: Duration) -> PollResult {
        let deadline = Instant::now() + timeout;
        let mut res = PollResult::default();
        let mut asked = vec![];
        for (i, w) in self.workers.iter_mut().enumerate() {
            Self::drain_stale(w);
            if w.outstanding > 0 {
                res.timed_out.push(w.name); // still stuck in a previous request
                continue;
            }
            if w.tx.send(Cmd::Poll).is_ok() {
                w.outstanding += 1;
                asked.push(i);
            }
        }
        for i in asked {
            let w = &mut self.workers[i];
            let left = deadline.saturating_duration_since(Instant::now());
            match w.rx.recv_timeout(left) {
                Ok(Reply::Reports(r)) => {
                    w.outstanding -= 1;
                    res.reports.extend(r);
                }
                Ok(Reply::Diagnostics(_)) | Ok(Reply::Events(_)) => w.outstanding -= 1,
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => res.timed_out.push(w.name),
            }
        }
        res
    }

    /// Ask every idle provider whether something happened since the last poll.
    pub fn check_events(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut asked = vec![];
        for (i, w) in self.workers.iter_mut().enumerate() {
            Self::drain_stale(w);
            if w.outstanding == 0 && w.tx.send(Cmd::CheckEvents).is_ok() {
                w.outstanding += 1;
                asked.push(i);
            }
        }
        let mut any = false;
        for i in asked {
            let w = &mut self.workers[i];
            let left = deadline.saturating_duration_since(Instant::now());
            if let Ok(r) = w.rx.recv_timeout(left) {
                w.outstanding -= 1;
                any |= matches!(r, Reply::Events(true));
            }
        }
        any
    }

    pub fn diagnostics(&mut self, timeout: Duration) -> Vec<(&'static str, serde_json::Value)> {
        let deadline = Instant::now() + timeout;
        let mut out = vec![];
        for w in self.workers.iter_mut() {
            Self::drain_stale(w);
            if w.outstanding > 0 || w.tx.send(Cmd::Diagnostics).is_err() {
                out.push((w.name, serde_json::json!({"error": "provider busy"})));
                continue;
            }
            w.outstanding += 1;
            let left = deadline.saturating_duration_since(Instant::now());
            match w.rx.recv_timeout(left) {
                Ok(Reply::Diagnostics(v)) => {
                    w.outstanding -= 1;
                    out.push((w.name, v));
                }
                Ok(Reply::Reports(_)) | Ok(Reply::Events(_)) => w.outstanding -= 1,
                Err(_) => out.push((w.name, serde_json::json!({"error": "timeout"}))),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    struct Fast;
    impl BatteryProvider for Fast {
        fn name(&self) -> &'static str {
            "fast"
        }
        fn poll(&mut self) -> Vec<DeviceReport> {
            vec![DeviceReport {
                provider: "fast".into(),
                source_key: "1".into(),
                source: "t".into(),
                name: "Fast".into(),
                kind: DeviceKind::Mouse,
                transport: Transport::Usb,
                identity: vec![],
                hints: vec![],
                reading: Reading::ok(50, Precision::Exact),
                components: vec![],
            }]
        }
    }

    struct Hung;
    impl BatteryProvider for Hung {
        fn name(&self) -> &'static str {
            "hung"
        }
        fn poll(&mut self) -> Vec<DeviceReport> {
            thread::sleep(Duration::from_secs(3600));
            vec![]
        }
    }

    #[test]
    fn hung_provider_does_not_block_others() {
        let mut e = Engine::new(vec![Box::new(Hung), Box::new(Fast)]);
        let t = Instant::now();
        let r = e.poll(Duration::from_millis(300));
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(r.reports.len(), 1);
        assert_eq!(r.timed_out, vec!["hung"]);
        // Second poll: the hung provider is skipped immediately.
        let t = Instant::now();
        let r = e.poll(Duration::from_millis(300));
        assert!(t.elapsed() < Duration::from_millis(400));
        assert_eq!(r.reports.len(), 1);
        assert_eq!(r.timed_out, vec!["hung"]);
    }
}
