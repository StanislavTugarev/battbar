// SPDX-License-Identifier: GPL-3.0-or-later
//! Background polling: the providers live here, the main thread only renders.
//! Polls every `interval`; polls soon after a hotplug notification (a HID device or a
//! Bluetooth connection appeared or went away); checks cheaply for buffered device
//! events every 2 s; and does a quick follow-up poll while a headset ↔ Bluetooth link is
//! being learned. When to do what is decided by [`Schedule`] (plain logic, tested).

use battbar_core::*;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub enum WorkerCmd {
    Refresh,
    /// Poll again after the given delay (e.g. while a new device settles).
    RefreshIn(Duration),
    Reconfigure {
        headsetcontrol: bool,
        interval: Duration,
    },
    /// A device appeared or disappeared (`"hid"`: IOHIDManager; `"bluetooth"`:
    /// IOBluetooth connect/disconnect).
    Hotplug(&'static str),
    /// The wake-up timer fired (see [`WakeTimer`]).
    Tick,
}

/// Leeway for a wake-up `after` from now: 15 % (at most 15 s), so macOS can coalesce
/// it with other wake-ups (the 2 s event check fires within 2.0–2.3 s, the minute poll
/// within 60–69 s).
pub fn leeway_for(after: Duration) -> Duration {
    after.mul_f64(0.15).min(Duration::from_secs(15))
}

/// One GCD timer source with leeway that wakes the worker: re-armed (one-shot) for the
/// next time the [`Schedule`] needs; the worker itself waits for commands without a
/// timeout. (A `recv_timeout` cannot be coalesced.)
struct WakeTimer {
    source: dispatch2::DispatchRetained<dispatch2::DispatchSource>,
}

extern "C" fn wake_timer_fired(ctx: *mut std::ffi::c_void) {
    // SAFETY: the context is the leaked `Sender` set in `WakeTimer::new`.
    let tx = unsafe { &*(ctx as *const Sender<WorkerCmd>) };
    let _ = tx.send(WorkerCmd::Tick);
}

impl WakeTimer {
    fn new(tx: Sender<WorkerCmd>) -> Self {
        use dispatch2::{DispatchObject, DispatchQoS, DispatchQueue, DispatchTime, GlobalQueueIdentifier};
        let queue = DispatchQueue::global_queue(GlobalQueueIdentifier::QualityOfService(DispatchQoS::Utility));
        let timer_type = (&raw const dispatch2::_dispatch_source_type_timer) as dispatch2::dispatch_source_type_t;
        let source = unsafe { dispatch2::DispatchSource::new(timer_type, 0, 0, Some(&queue)) };
        // The worker (and this timer) live for the whole process: leak the sender.
        let ctx = Box::into_raw(Box::new(tx)) as *mut std::ffi::c_void;
        unsafe { source.set_context(ctx) };
        source.set_event_handler_f(wake_timer_fired);
        source.set_timer(DispatchTime::FOREVER, u64::MAX, 0); // disarmed until `arm`
        source.activate();
        WakeTimer { source }
    }

    /// Fire once, `after` from now, with [`leeway_for`].
    fn arm(&self, after: Duration) {
        use dispatch2::DispatchTime;
        let start = DispatchTime::NOW.time(after.as_nanos().min(i64::MAX as u128) as i64);
        self.source.set_timer(start, u64::MAX, leeway_for(after).as_nanos() as u64);
    }
}

pub struct PollUpdate {
    pub reports: Vec<DeviceReport>,
    pub learned: Vec<LearnedLink>,
    pub timed_out: Vec<&'static str>,
    /// CoreBluetooth authorization (0 not determined, 2 denied, 3 allowed, …).
    pub bt_authorization: isize,
    /// What started this poll (for the log).
    pub reason: &'static str,
}

const EVENT_CHECK: Duration = Duration::from_secs(2);
const FOLLOWUP: Duration = Duration::from_secs(3);
const POLL_DEADLINE: Duration = Duration::from_secs(15);
/// A plug-in reports several interfaces in a row: one poll for the burst.
pub const HOTPLUG_DEBOUNCE: Duration = Duration::from_millis(300);

/// What the worker does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Poll all providers now; the reason is logged.
    Poll(&'static str),
    /// Ask the providers whether something happened (cheap).
    CheckEvents,
    /// Nothing until [`Schedule::wake_at`] or a command.
    Wait,
}

/// When to poll and when to check for events.
#[derive(Debug, Clone)]
pub struct Schedule {
    interval: Duration,
    next_poll: Instant,
    followup: Option<Instant>,
    last_check: Instant,
    /// Poll soon (after a hotplug notification), with its reason.
    soon: Option<(Instant, &'static str)>,
    /// A poll was asked for directly (Refresh, Reconfigure).
    asked: bool,
}

impl Schedule {
    pub fn new(now: Instant, interval: Duration) -> Self {
        Schedule {
            interval,
            next_poll: now,
            followup: None,
            last_check: now,
            soon: None,
            asked: false,
        }
    }

    /// The latest time to wake up (commands wake earlier).
    pub fn wake_at(&self) -> Instant {
        let mut t = self.next_poll.min(self.last_check + EVENT_CHECK);
        if let Some(f) = self.followup {
            t = t.min(f);
        }
        if let Some((s, _)) = self.soon {
            t = t.min(s);
        }
        t
    }

    pub fn refresh_now(&mut self, now: Instant) {
        self.next_poll = now;
        self.asked = true;
    }

    pub fn refresh_in(&mut self, now: Instant, d: Duration) {
        let t = now + d;
        self.followup = Some(self.followup.map_or(t, |f| f.min(t)));
    }

    pub fn set_interval(&mut self, now: Instant, interval: Duration) {
        self.interval = interval;
        self.refresh_now(now);
    }

    /// A hotplug notification: poll after the debounce (a burst gives one poll).
    pub fn hotplug(&mut self, now: Instant, what: &'static str) {
        if self.soon.is_none() {
            self.soon = Some((now + HOTPLUG_DEBOUNCE, what));
        }
    }

    pub fn step(&mut self, now: Instant) -> Step {
        if self.asked && now >= self.next_poll {
            return Step::Poll("refresh");
        }
        if let Some((s, what)) = self.soon {
            if now >= s {
                return Step::Poll(what);
            }
        }
        if self.followup.is_some_and(|f| now >= f) {
            return Step::Poll("follow-up");
        }
        if now >= self.next_poll {
            return Step::Poll("interval");
        }
        if now >= self.last_check + EVENT_CHECK {
            self.last_check = now;
            return Step::CheckEvents;
        }
        Step::Wait
    }

    /// A poll finished at `now`.
    pub fn polled(&mut self, now: Instant, wants_followup: bool) {
        self.next_poll = now + self.interval;
        self.followup = wants_followup.then(|| now + FOLLOWUP);
        self.last_check = now;
        self.soon = None;
        self.asked = false;
    }
}

/// Providers live on their engine threads for the whole run of the app: settings are
/// applied through shared switches, never by rebuilding the engine, so no thread that may
/// own system objects (run loops, queues) ever exits (docs/DEVELOPMENT.md).
fn engine(db: &Arc<DeviceDb>, headsetcontrol: bool) -> (Engine, Arc<std::sync::atomic::AtomicBool>) {
    let hidpp = battbar_hidpp::HidppProvider::new(
        db.clone(),
        vec![
            Box::new(battbar_hidpp::HidapiLinks::default()),
            Box::new(battbar_hidpp::GattLinks::default()),
        ],
    );
    let mut ss = battbar_steelseries::SteelSeriesProvider::new(db.clone(), vec![Box::new(battbar_steelseries::HidapiBases)]);
    ss.set_headsetcontrol(headsetcontrol);
    let switch = ss.headsetcontrol_switch();
    let engine = Engine::new(vec![
        Box::new(hidpp),
        Box::new(ss),
        Box::new(battbar_bt_system::BtSystemProvider::default()),
    ]);
    (engine, switch)
}

pub fn spawn(settings: &Settings, on_update: impl Fn(PollUpdate) + Send + 'static) -> Sender<WorkerCmd> {
    let (tx, rx) = mpsc::channel();
    // HID hotplug (receivers, base stations, Bluetooth HID devices): poll soon.
    let hid_tx = tx.clone();
    if !dev_no_hotplug() {
        battbar_hid::on_hotplug(move || {
            let _ = hid_tx.send(WorkerCmd::Hotplug("hid"));
        });
        // Headset changes reported by a base station itself (before the first poll
        // opens the bases, so their listeners start).
        let base_tx = tx.clone();
        battbar_steelseries::events::on_event(move || {
            let _ = base_tx.send(WorkerCmd::Hotplug("base"));
        });
    }
    let timer_tx = tx.clone();
    let headsetcontrol = settings.headsetcontrol_fallback;
    let interval = Duration::from_secs(settings.poll_interval_secs.max(10));
    std::thread::Builder::new()
        .name("battbar-worker".into())
        .spawn(move || {
            let db = Arc::new(
                DeviceDb::load_with_overrides(DeviceDb::user_dir().as_deref())
                    .unwrap_or_else(|_| DeviceDb::embedded().expect("built-in device database")),
            );
            let (mut engine, headsetcontrol_switch) = engine(&db, headsetcontrol);
            let mut learner = LinkLearner::default();
            let mut sched = Schedule::new(Instant::now(), interval);
            let timer = WakeTimer::new(timer_tx);
            loop {
                timer.arm(sched.wake_at().saturating_duration_since(Instant::now()));
                match rx.recv() {
                    Ok(WorkerCmd::Refresh) => sched.refresh_now(Instant::now()),
                    Ok(WorkerCmd::RefreshIn(d)) => sched.refresh_in(Instant::now(), d),
                    Ok(WorkerCmd::Reconfigure {
                        headsetcontrol: h,
                        interval: i,
                    }) => {
                        headsetcontrol_switch.store(h, std::sync::atomic::Ordering::Relaxed);
                        sched.set_interval(Instant::now(), i.max(Duration::from_secs(10)));
                    }
                    Ok(WorkerCmd::Hotplug(what)) => sched.hotplug(Instant::now(), what),
                    Ok(WorkerCmd::Tick) => {}
                    Err(_) => return,
                }
                let reason = match sched.step(Instant::now()) {
                    Step::Poll(r) => r,
                    Step::CheckEvents if engine.check_events(Duration::from_millis(1500)) => "device event",
                    Step::CheckEvents | Step::Wait => continue,
                };
                let res = engine.poll(POLL_DEADLINE);
                let learned = learner.observe(Instant::now(), &res.reports);
                sched.polled(Instant::now(), learner.wants_followup());
                // CoreBluetooth call on this thread: in its own autorelease pool.
                let bt_authorization = battbar_core::objc_pool::with_pool(battbar_macos::ble::authorization);
                on_update(PollUpdate {
                    reports: res.reports,
                    learned,
                    timed_out: res.timed_out,
                    bt_authorization,
                    reason,
                });
            }
        })
        .expect("spawn worker");
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Run the schedule from `from` to `to` ms in 10 ms steps; the steps taken.
    fn run(s: &mut Schedule, t0: Instant, from: u64, to: u64) -> Vec<(u64, Step)> {
        let mut out = vec![];
        let mut t = from;
        while t <= to {
            match s.step(ms(t0, t)) {
                Step::Wait => {}
                Step::Poll(r) => {
                    out.push((t, Step::Poll(r)));
                    s.polled(ms(t0, t), false);
                }
                st => out.push((t, st)),
            }
            t += 10;
        }
        out
    }

    #[test]
    fn hotplug_polls_within_the_debounce_without_a_rescan_or_event_check() {
        let t0 = Instant::now();
        let mut s = Schedule::new(t0, Duration::from_secs(60));
        assert_eq!(s.step(t0), Step::Poll("interval"));
        s.polled(t0, false);
        // A receiver is plugged in at 10.0 s: three interfaces in a row.
        for i in 0..3 {
            s.hotplug(ms(t0, 10_000 + i * 5), "hid");
        }
        assert!(s.wake_at() <= ms(t0, 10_300));
        let steps = run(&mut s, t0, 10_000, 11_000);
        let polls: Vec<_> = steps.iter().filter(|(_, st)| matches!(st, Step::Poll(_))).collect();
        assert_eq!(polls.len(), 1, "one poll for the burst: {steps:?}");
        assert_eq!(polls[0].1, Step::Poll("hid"));
        assert!(polls[0].0 <= 10_300, "within the debounce: {} ms", polls[0].0);
    }

    #[test]
    fn calm_runs_poll_at_the_interval_and_check_every_two_seconds() {
        let t0 = Instant::now();
        let mut s = Schedule::new(t0, Duration::from_secs(60));
        let steps = run(&mut s, t0, 0, 60_000);
        let polls = steps.iter().filter(|(_, st)| matches!(st, Step::Poll(_))).count();
        let checks = steps.iter().filter(|(_, st)| *st == Step::CheckEvents).count();
        assert_eq!(polls, 2, "at 0 and 60 s: {steps:?}");
        assert_eq!(checks, 29, "every 2 s between polls");
    }

    #[test]
    fn leeway_is_a_share_of_the_period() {
        assert_eq!(leeway_for(Duration::from_secs(2)), Duration::from_millis(300));
        assert_eq!(leeway_for(Duration::from_secs(60)), Duration::from_secs(9));
        assert_eq!(leeway_for(Duration::from_millis(300)), Duration::from_millis(45));
        assert_eq!(leeway_for(Duration::from_secs(300)), Duration::from_secs(15), "capped");
        assert_eq!(leeway_for(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn refresh_and_follow_up() {
        let t0 = Instant::now();
        let mut s = Schedule::new(t0, Duration::from_secs(60));
        s.step(t0);
        s.polled(t0, true); // learning a link: follow-up in 3 s
        assert_eq!(
            run(&mut s, t0, 10, 3_000)
                .iter()
                .find(|(_, st)| matches!(st, Step::Poll(_)))
                .map(|x| x.1),
            Some(Step::Poll("follow-up"))
        );
        s.refresh_now(ms(t0, 5_000));
        assert_eq!(s.step(ms(t0, 5_000)), Step::Poll("refresh"));
    }
}
