// SPDX-License-Identifier: GPL-3.0-or-later
//! SteelSeries provider: native base-station protocol for supported models and an
//! optional `headsetcontrol` fallback for others.
//!
//! Protocol knowledge ported from HeadsetControl (GPL-3.0), see NOTICE.

pub mod headsetcontrol;
pub mod nova_pro;

use battbar_core::devicedb::Protocol;
use battbar_core::transport::{FrameTransport, RecordingTransport, TraceWriter, TransportError};
use battbar_core::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type BoxTransport = Box<dyn FrameTransport>;

pub struct BaseLink {
    pub id: String,
    pub device: devicedb::DeviceDef,
    pub open: Box<dyn FnOnce() -> Result<BoxTransport, TransportError> + Send>,
    /// A second, independent handle on the same interface for the event listener (the
    /// base reports headset changes by itself, see [`events`]); `None` for replays.
    pub listen: Option<Box<dyn FnOnce() -> Result<BoxTransport, TransportError> + Send>>,
}

pub mod events {
    //! Headset events the base station sends by itself (report 0x07 0xb5/0xb7: headset
    //! on/off, charging, Bluetooth; DISCOVERY "Base station events"). One long-lived
    //! thread per open base blocks on its own handle (no wake-up while nothing
    //! happens) and calls the registered callback; the app then polls. The thread
    //! ends when the base goes away.
    use super::BoxTransport;
    use battbar_core::transport::TransportError;
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::time::Duration;

    type Callback = Box<dyn Fn() + Send + Sync>;
    static CALLBACK: Mutex<Option<Callback>> = Mutex::new(None);
    /// Links with a running listener (never two for one base).
    static LISTENING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    /// A blocking read that never times out in practice (~24 days).
    const FOREVER: Duration = Duration::from_millis(i32::MAX as u64);

    /// Call `f` whenever a base station reports a headset change (on the listener
    /// thread; keep it short). Without a callback no listener is started.
    pub fn on_event(f: impl Fn() + Send + Sync + 'static) {
        *CALLBACK.lock().unwrap() = Some(Box::new(f));
    }

    pub(crate) fn enabled() -> bool {
        CALLBACK.lock().unwrap().is_some()
    }

    pub(crate) fn listen(id: String, open: Box<dyn FnOnce() -> Result<BoxTransport, TransportError> + Send>) -> Result<(), String> {
        if !LISTENING.lock().unwrap().insert(id.clone()) {
            return Ok(());
        }
        let started = std::thread::Builder::new().name("battbar-base-events".into()).spawn(move || {
            if let Ok(mut t) = open() {
                // Ends on an error: the base was unplugged.
                while let Ok(r) = t.read(FOREVER) {
                    if r.as_deref().is_some_and(crate::nova_pro::is_headset_event) {
                        log(r.as_deref().unwrap_or_default());
                        if let Some(f) = CALLBACK.lock().unwrap().as_ref() {
                            f();
                        }
                    }
                }
            }
            LISTENING.lock().unwrap().remove(&id);
        });
        started.map(|_| ()).map_err(|e| e.to_string())
    }

    #[cfg(debug_assertions)]
    fn log(r: &[u8]) {
        if std::env::var_os("BATTBAR_DEV_LOG_HIDPP").is_some() {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            eprintln!("base event {:.3}: {:02x?}", t.as_secs_f64(), &r[..r.len().min(5)]);
        }
    }
    #[cfg(not(debug_assertions))]
    fn log(_: &[u8]) {}
}

pub trait BaseSource: Send {
    fn scan(&mut self, db: &DeviceDb) -> Vec<BaseLink>;
}

/// Base stations on USB via hidapi, selected by VID/PID + vendor usage page from the DB.
#[derive(Default)]
pub struct HidapiBases;

impl BaseSource for HidapiBases {
    fn scan(&mut self, db: &DeviceDb) -> Vec<BaseLink> {
        let Ok(ifaces) = battbar_hid::enumerate() else { return vec![] };
        ifaces
            .into_iter()
            .filter_map(|i| {
                let d = db.usb_device(i.vendor_id, i.product_id)?;
                if d.protocol != Protocol::SteelseriesNovaPro
                    || !i.is_vendor_only()
                    || !d.vendor_usage_page.is_some_and(|p| i.has_usage_page(p))
                {
                    return None;
                }
                let d = d.clone();
                let second = i.clone();
                Some(BaseLink {
                    id: i.link_id(),
                    device: d,
                    open: Box::new(move || battbar_hid::open(&i).map(|t| Box::new(t) as BoxTransport)),
                    listen: Some(Box::new(move || battbar_hid::open(&second).map(|t| Box::new(t) as BoxTransport))),
                })
            })
            .collect()
    }
}

/// Links replayed from a trace.
pub struct ReplayBases {
    links: Vec<(String, devicedb::DeviceDef, Option<transport::ReplayTransport>)>,
}

impl ReplayBases {
    pub fn new(trace: &transport::Trace, db: &DeviceDb) -> Self {
        let links = trace
            .transports()
            .into_iter()
            .filter_map(|t| {
                let i = t.info().clone();
                let d = db
                    .usb_device(i.vendor_id, i.product_id)
                    .filter(|d| d.protocol == Protocol::SteelseriesNovaPro)?
                    .clone();
                Some((i.link, d, Some(t)))
            })
            .collect();
        ReplayBases { links }
    }
}

impl BaseSource for ReplayBases {
    fn scan(&mut self, _db: &DeviceDb) -> Vec<BaseLink> {
        self.links
            .iter_mut()
            .map(|(id, d, t)| {
                let t = t.take();
                BaseLink {
                    id: id.clone(),
                    device: d.clone(),
                    open: Box::new(move || {
                        t.map(|t| Box::new(t) as BoxTransport)
                            .ok_or(TransportError::Refused("already open".into()))
                    }),
                    listen: None,
                }
            })
            .collect()
    }
}

struct Open {
    device: devicedb::DeviceDef,
    t: BoxTransport,
    last_raw: Option<Vec<u8>>,
}

pub struct SteelSeriesProvider {
    db: Arc<DeviceDb>,
    sources: Vec<Box<dyn BaseSource>>,
    links: BTreeMap<String, Open>,
    trace: Option<Arc<Mutex<TraceWriter>>>,
    /// Use `headsetcontrol` for models without native support.
    /// Use `headsetcontrol` for models without native support. Shared switch: the app
    /// flips it at runtime instead of rebuilding the provider (and its thread).
    headsetcontrol: Arc<std::sync::atomic::AtomicBool>,
    errors: Vec<String>,
    /// Safety re-scan for base stations (hotplug notifications do the real work).
    rescan: battbar_core::Rescan,
}

const TIMEOUT: Duration = Duration::from_millis(1500);

impl SteelSeriesProvider {
    pub fn new(db: Arc<DeviceDb>, sources: Vec<Box<dyn BaseSource>>) -> Self {
        SteelSeriesProvider {
            db,
            sources,
            links: BTreeMap::new(),
            trace: None,
            headsetcontrol: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            errors: vec![],
            rescan: battbar_core::Rescan::default(),
        }
    }

    pub fn set_headsetcontrol(&mut self, on: bool) {
        self.headsetcontrol.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Handle to switch the fallback while the provider runs on its worker thread.
    pub fn headsetcontrol_switch(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.headsetcontrol.clone()
    }

    pub fn with_trace(mut self, w: Arc<Mutex<TraceWriter>>) -> Self {
        self.trace = Some(w);
        self
    }

    fn refresh(&mut self) {
        self.rescan.scanned(std::time::Instant::now());
        let mut present = vec![];
        let mut cands = vec![];
        for s in &mut self.sources {
            cands.extend(s.scan(&self.db));
        }
        for c in cands {
            present.push(c.id.clone());
            if self.links.contains_key(&c.id) {
                continue;
            }
            match (c.open)() {
                Ok(t) => {
                    // Only for models whose event format is confirmed (`base_events`);
                    // others are read by the regular poll.
                    if let Some(listen) = c.listen.filter(|_| events::enabled() && c.device.base_events) {
                        if let Err(e) = events::listen(c.id.clone(), listen) {
                            self.errors.push(format!("{}: event listener: {e}", c.id));
                        }
                    }
                    let t: BoxTransport = match &self.trace {
                        Some(w) => Box::new(RecordingTransport::new(t, w.clone())),
                        None => t,
                    };
                    self.links.insert(
                        c.id,
                        Open {
                            device: c.device,
                            t,
                            last_raw: None,
                        },
                    );
                }
                Err(e) => self.errors.push(format!("{}: {e}", c.id)),
            }
        }
        self.links.retain(|id, _| present.contains(id));
    }

    /// Set the safety re-scan interval (tests).
    pub fn set_rescan_interval(&mut self, every: Duration) {
        self.rescan.every = every;
    }

    /// The event check at `now`: only the safety re-scan of base stations. Headset
    /// changes arrive as events from the base itself (see [`events`]) or at the next
    /// poll; hotplug notifications trigger polls for new bases.
    pub fn check_events_at(&mut self, now: Instant) -> bool {
        if !self.rescan.due(now) {
            return false;
        }
        let mut ids: Vec<String> = vec![];
        for s in &mut self.sources {
            ids.extend(s.scan(&self.db).into_iter().map(|c| c.id));
        }
        ids.sort();
        ids != self.links.keys().cloned().collect::<Vec<_>>()
    }

    fn query(o: &mut Open) -> Result<Option<Vec<u8>>, TransportError> {
        // Drop stale input (volume/ANC events) before asking.
        while o.t.read(Duration::ZERO)?.is_some() {}
        o.t.write(&nova_pro::STATUS_REQUEST)?;
        let deadline = Instant::now() + TIMEOUT;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match o.t.read(left)? {
                Some(r) if nova_pro::is_status_reply(&r) => return Ok(Some(r)),
                Some(_) => continue,
                None => break,
            }
        }
        Ok(None)
    }
}

impl BatteryProvider for SteelSeriesProvider {
    fn name(&self) -> &'static str {
        "steelseries"
    }

    fn poll(&mut self) -> Vec<DeviceReport> {
        self.refresh();
        let mut out = vec![];
        let mut broken = vec![];
        for (id, o) in self.links.iter_mut() {
            let (reading, components, hints) = match Self::query(o) {
                Ok(Some(r)) => {
                    let s = nova_pro::decode(&r);
                    o.last_raw = Some(r);
                    let hints = if s.headset_on_bluetooth && !o.device.bt_name_patterns.is_empty() {
                        vec![MergeHint::BluetoothCompanion {
                            name_patterns: o.device.bt_name_patterns.clone(),
                        }]
                    } else {
                        vec![]
                    };
                    (s.headset.clone(), nova_pro::spare_component(&s), hints)
                }
                Ok(None) => (Reading::with_status(Status::Error), vec![], vec![]),
                Err(TransportError::Disconnected) => {
                    broken.push(id.clone());
                    continue;
                }
                Err(e) => {
                    self.errors.push(format!("{id}: {e}"));
                    (Reading::with_status(Status::Error), vec![], vec![])
                }
            };
            out.push(DeviceReport {
                provider: "steelseries".into(),
                source_key: id.clone(),
                source: "SteelSeries base station".into(),
                name: o.device.name.clone(),
                kind: o.device.kind,
                transport: Transport::BaseStation,
                identity: vec![],
                hints,
                reading,
                components,
            });
        }
        for id in broken {
            self.links.remove(&id);
        }
        if self.headsetcontrol.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(json) = headsetcontrol::run(Duration::from_secs(5)) {
                let db = self.db.clone();
                out.extend(headsetcontrol::parse(&json, |v, p| db.usb_device(v, p).is_some()));
            }
        }
        out
    }

    fn check_events(&mut self) -> bool {
        self.check_events_at(Instant::now())
    }

    fn diagnostics(&mut self) -> serde_json::Value {
        self.refresh();
        let links: Vec<serde_json::Value> = self
            .links
            .iter_mut()
            .map(|(id, o)| {
                let raw = Self::query(o).ok().flatten();
                let info = o.t.info().clone();
                serde_json::json!({
                    "link": id, "model": o.device.name,
                    "vendor_id": format!("{:04x}", info.vendor_id), "product_id": format!("{:04x}", info.product_id),
                    "collections": info.collections.iter().map(|(p, u)| format!("{p:04x}/{u:04x}")).collect::<Vec<_>>(),
                    "status_reply": raw.map(|r| transport::to_hex(&r[..r.len().min(32)])),
                })
            })
            .collect();
        serde_json::json!({"links": links, "headsetcontrol_installed": headsetcontrol::run(Duration::from_secs(5)).is_some(), "errors": self.errors})
    }
}
