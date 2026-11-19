// SPDX-License-Identifier: GPL-3.0-or-later
//! Background polling: the providers live here, the main thread only renders.
//! Polls every `interval`, checks cheaply for push events / hotplug every 2 s, and
//! does a quick follow-up poll while a headset ↔ Bluetooth link is being learned.

use battbar_core::*;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
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
}

pub struct PollUpdate {
    pub reports: Vec<DeviceReport>,
    pub learned: Vec<LearnedLink>,
    pub timed_out: Vec<&'static str>,
    /// CoreBluetooth authorization (0 not determined, 2 denied, 3 allowed, …).
    pub bt_authorization: isize,
}

const EVENT_CHECK: Duration = Duration::from_secs(2);
const FOLLOWUP: Duration = Duration::from_secs(3);
const POLL_DEADLINE: Duration = Duration::from_secs(15);

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
    let headsetcontrol = settings.headsetcontrol_fallback;
    let mut interval = Duration::from_secs(settings.poll_interval_secs.max(10));
    std::thread::Builder::new()
        .name("battbar-worker".into())
        .spawn(move || {
            let db = Arc::new(
                DeviceDb::load_with_overrides(DeviceDb::user_dir().as_deref())
                    .unwrap_or_else(|_| DeviceDb::embedded().expect("built-in device database")),
            );
            let (mut engine, headsetcontrol_switch) = engine(&db, headsetcontrol);
            let mut learner = LinkLearner::default();
            let mut next_poll = Instant::now();
            let mut followup: Option<Instant> = None;
            let mut last_check = Instant::now();
            loop {
                let now = Instant::now();
                let mut wake = next_poll.min(last_check + EVENT_CHECK);
                if let Some(f) = followup {
                    wake = wake.min(f);
                }
                match rx.recv_timeout(wake.saturating_duration_since(now)) {
                    Ok(WorkerCmd::Refresh) => next_poll = Instant::now(),
                    Ok(WorkerCmd::RefreshIn(d)) => {
                        let t = Instant::now() + d;
                        followup = Some(followup.map_or(t, |f| f.min(t)));
                    }
                    Ok(WorkerCmd::Reconfigure {
                        headsetcontrol: h,
                        interval: i,
                    }) => {
                        headsetcontrol_switch.store(h, std::sync::atomic::Ordering::Relaxed);
                        interval = i.max(Duration::from_secs(10));
                        next_poll = Instant::now();
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                let now = Instant::now();
                let mut due = now >= next_poll || followup.is_some_and(|f| now >= f);
                if !due && now >= last_check + EVENT_CHECK {
                    last_check = now;
                    due = engine.check_events(Duration::from_millis(1500));
                }
                if !due {
                    continue;
                }
                let res = engine.poll(POLL_DEADLINE);
                let learned = learner.observe(Instant::now(), &res.reports);
                followup = learner.wants_followup().then(|| Instant::now() + FOLLOWUP);
                next_poll = Instant::now() + interval;
                last_check = Instant::now();
                on_update(PollUpdate {
                    reports: res.reports,
                    learned,
                    timed_out: res.timed_out,
                    bt_authorization: battbar_macos::ble::authorization(),
                });
            }
        })
        .expect("spawn worker");
    tx
}
