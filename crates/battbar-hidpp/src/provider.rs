// SPDX-License-Identifier: GPL-3.0-or-later
//! The HID++ battery provider. Transport-agnostic: links come from [`LinkSource`]s
//! (hidapi for USB receivers and wired devices, GATT for Bluetooth LE, replay in tests).

use crate::device::{self, hex4, BatteryFeature, DeviceInfo};
use crate::protocol::{err10, Channel, HidppError, RECEIVER_INDEX};
use crate::receiver::{self, PairedSlot};
use battbar_core::devicedb::ReceiverDef;
use battbar_core::transport::{FrameTransport, RecordingTransport, TraceWriter, TransportError};
use battbar_core::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

pub type BoxTransport = Box<dyn FrameTransport>;

#[derive(Debug, Clone, PartialEq)]
pub enum LinkRole {
    Receiver(ReceiverDef),
    /// Device directly on the link (USB cable or Bluetooth LE), device index 0xFF.
    Direct {
        transport: Transport,
    },
}

pub struct LinkCandidate {
    pub id: String,
    pub role: LinkRole,
    pub long_only: bool,
    /// Extra identity keys for every device on this link (e.g. the BLE peripheral ID,
    /// so the system Battery Service report of the same peripheral merges).
    pub identity: Vec<IdentityKey>,
    pub open: Box<dyn FnOnce() -> Result<BoxTransport, TransportError> + Send>,
}

pub trait LinkSource: Send {
    fn scan(&mut self, db: &DeviceDb) -> Vec<LinkCandidate>;
}

struct OpenLink {
    role: LinkRole,
    identity: Vec<IdentityKey>,
    ch: Channel<BoxTransport>,
    devices: BTreeMap<u8, DeviceInfo>,
    notifications_checked: bool,
    broken: bool,
}

pub struct HidppProvider {
    db: Arc<DeviceDb>,
    sources: Vec<Box<dyn LinkSource>>,
    links: BTreeMap<String, OpenLink>,
    trace: Option<Arc<Mutex<TraceWriter>>>,
    errors: Vec<String>,
    checks: u64,
}

impl HidppProvider {
    pub fn new(db: Arc<DeviceDb>, sources: Vec<Box<dyn LinkSource>>) -> Self {
        HidppProvider {
            db,
            sources,
            links: BTreeMap::new(),
            trace: None,
            errors: vec![],
            checks: 0,
        }
    }

    /// Notification that should trigger a poll: receiver connection events (HID++ 1.0
    /// sub-IDs 0x40..=0x4F) or an event of the device's battery feature. Other
    /// notifications (G-keys, lighting) are ignored.
    fn relevant(link: &OpenLink, n: &[u8]) -> bool {
        if n.len() < 4 {
            return false;
        }
        if (0x40..=0x4F).contains(&n[2]) {
            return true;
        }
        link.devices.get(&n[1]).and_then(|d| d.battery).is_some_and(|b| {
            let idx = match b {
                BatteryFeature::Unified { index, .. } | BatteryFeature::Voltage { index } | BatteryFeature::Status { index } => index,
            };
            idx == n[2]
        })
    }

    /// Record every frame of every link opened from now on.
    pub fn with_trace(mut self, trace: Arc<Mutex<TraceWriter>>) -> Self {
        self.trace = Some(trace);
        self
    }

    fn refresh_links(&mut self) {
        let mut present = BTreeSet::new();
        let mut cands = vec![];
        for s in &mut self.sources {
            cands.extend(s.scan(&self.db));
        }
        for c in cands {
            present.insert(c.id.clone());
            if self.links.contains_key(&c.id) {
                continue;
            }
            match (c.open)() {
                Ok(t) => {
                    let t: BoxTransport = match &self.trace {
                        Some(w) => Box::new(RecordingTransport::new(t, w.clone())),
                        None => t,
                    };
                    self.links.insert(
                        c.id,
                        OpenLink {
                            role: c.role,
                            identity: c.identity,
                            ch: Channel::new(t, c.long_only),
                            devices: BTreeMap::new(),
                            notifications_checked: false,
                            broken: false,
                        },
                    );
                }
                Err(e) => self.errors.push(format!("{}: {e}", c.id)),
            }
        }
        self.links.retain(|id, l| present.contains(id) && !l.broken);
    }

    fn device_def(&self, vid: u16, pids: &[u16]) -> Option<&devicedb::DeviceDef> {
        pids.iter().find_map(|p| self.db.hidpp_device_by_pid(vid, *p))
    }
}

fn status_for_ping_error(e: &HidppError) -> Status {
    match e {
        HidppError::Hidpp10(err10::UNKNOWN_DEVICE) => Status::Disconnected,
        HidppError::Hidpp10(err10::RESOURCE_ERROR) | HidppError::Timeout => Status::Asleep,
        HidppError::Transport(TransportError::Disconnected) => Status::Disconnected,
        _ => Status::Error,
    }
}

fn hidpp10_reading() -> Reading {
    let mut r = Reading::with_status(Status::NotReported);
    r.note = Some(ReadingNote::Hidpp10Device);
    r
}

impl BatteryProvider for HidppProvider {
    fn name(&self) -> &'static str {
        "hidpp"
    }

    fn poll(&mut self) -> Vec<DeviceReport> {
        self.refresh_links();
        let mut out = vec![];
        let ids: Vec<String> = self.links.keys().cloned().collect();
        for id in ids {
            let mut link = self.links.remove(&id).unwrap();
            let r = match link.role.clone() {
                LinkRole::Receiver(def) => self.poll_receiver(&id, &def, &mut link),
                LinkRole::Direct { transport } => self.poll_direct(&id, transport, &mut link),
            };
            match r {
                Ok(mut reports) => {
                    for rep in &mut reports {
                        rep.identity.extend(link.identity.iter().cloned());
                    }
                    out.extend(reports)
                }
                Err(HidppError::Transport(TransportError::Disconnected)) => link.broken = true,
                Err(e) => self.errors.push(format!("{id}: {e}")),
            }
            if !link.broken {
                self.links.insert(id, link);
            }
        }
        out
    }

    fn check_events(&mut self) -> bool {
        self.checks += 1;
        let mut any = false;
        for link in self.links.values_mut() {
            if let Err(HidppError::Transport(TransportError::Disconnected)) = link.ch.drain() {
                link.broken = true;
                any = true;
            }
            let notes = std::mem::take(&mut link.ch.notifications);
            any |= notes.iter().any(|n| Self::relevant(link, n));
        }
        // Hotplug: every 5th check, compare the set of available links.
        if self.checks.is_multiple_of(5) {
            let mut ids: Vec<String> = vec![];
            for s in &mut self.sources {
                ids.extend(s.scan(&self.db).into_iter().map(|c| c.id));
            }
            ids.sort();
            let open: Vec<String> = self.links.keys().cloned().collect();
            any |= ids != open;
        }
        any
    }

    fn diagnostics(&mut self) -> serde_json::Value {
        self.refresh_links();
        let mut links = vec![];
        let ids: Vec<String> = self.links.keys().cloned().collect();
        for id in ids {
            let link = self.links.get_mut(&id).unwrap();
            let info = link.ch.transport.info().clone();
            let indexes: Vec<u8> = match &link.role {
                LinkRole::Receiver(d) => {
                    let slots = receiver::paired_slots(&mut link.ch, d.family, d.max_devices).unwrap_or_default();
                    slots.iter().map(|s| s.index).collect()
                }
                LinkRole::Direct { .. } => vec![RECEIVER_INDEX],
            };
            let mut devs = vec![];
            for idx in indexes {
                let ping = link.ch.ping(idx);
                let mut d = json!({"index": idx, "ping": format!("{ping:?}")});
                if let Ok((maj, min)) = ping {
                    if maj >= 2 {
                        d["protocol"] = json!(format!("{maj}.{min}"));
                        d["features"] = json!(device::feature_list(&mut link.ch, idx)
                            .map(|v| v.iter().map(|(id, ver)| format!("0x{id:04x} v{ver}")).collect::<Vec<_>>())
                            .unwrap_or_default());
                        if let Ok(i) = device::probe(&mut link.ch, idx, (maj, min)) {
                            d["name"] = json!(i.name);
                            d["model_ids"] = json!(i.model_ids.iter().map(|p| format!("{p:04x}")).collect::<Vec<_>>());
                            d["battery_feature"] = json!(i.battery.map(|b| b.label()));
                            // Raw 0x1004 capabilities [levels, flags] (chemistry research).
                            if let Some(BatteryFeature::Unified { index, .. }) = i.battery {
                                if let Ok(c) = link.ch.call(idx, index, 0, &[]) {
                                    d["unified_battery_caps"] = json!(transport::to_hex(&c[..c.len().min(4)]));
                                }
                                if let Ok(st) = link.ch.call(idx, index, 1, &[]) {
                                    d["unified_battery_status"] = json!(transport::to_hex(&st[..st.len().min(4)]));
                                }
                            }
                        }
                    }
                }
                devs.push(d);
            }
            links.push(json!({
                "link": info.link, "kind": info.kind, "bus": info.bus,
                "vendor_id": format!("{:04x}", info.vendor_id), "product_id": format!("{:04x}", info.product_id),
                "product": info.product,
                "collections": info.collections.iter().map(|(p, u)| format!("{p:04x}/{u:04x}")).collect::<Vec<_>>(),
                "role": match &link.role { LinkRole::Receiver(d) => format!("receiver ({:?})", d.family), LinkRole::Direct { transport } => format!("direct ({transport})") },
                "devices": devs,
                "foreign_replies_dropped": link.ch.foreign_replies,
            }));
        }
        json!({"links": links, "errors": self.errors})
    }
}

impl HidppProvider {
    #[allow(clippy::too_many_arguments)]
    fn report(
        &self,
        link_id: &str,
        idx: u8,
        transport: Transport,
        vid: u16,
        pids: &[u16],
        info: Option<&DeviceInfo>,
        slot: Option<&PairedSlot>,
        reading: Reading,
        source: &str,
    ) -> DeviceReport {
        let def = self.device_def(vid, pids);
        let name = info
            .and_then(|i| i.name.clone())
            .or_else(|| def.map(|d| d.name.clone()))
            .or_else(|| slot.and_then(|s| s.codename.clone()))
            .unwrap_or_else(|| "Logitech device".into());
        let kind = info
            .and_then(|i| i.kind)
            .or_else(|| def.map(|d| d.kind))
            .or_else(|| slot.and_then(|s| s.kind))
            .unwrap_or(DeviceKind::Other);
        let unit = info.and_then(|i| i.unit_id).or_else(|| slot.and_then(|s| s.unit_id));
        DeviceReport {
            provider: "hidpp".into(),
            source_key: format!("{link_id}:{idx}"),
            source: source.into(),
            name,
            kind,
            transport,
            identity: unit.map(|u| vec![IdentityKey::HidppUnit { unit_id: hex4(&u) }]).unwrap_or_default(),
            hints: vec![],
            reading,
            components: vec![],
        }
    }

    fn battery(&self, link: &mut OpenLink, idx: u8, vid: u16, pids: &[u16], feature: BatteryFeature) -> Result<Reading, HidppError> {
        let def = self.device_def(vid, pids);
        let curve = self.db.curve_for(def);
        device::read_battery(&mut link.ch, idx, feature, curve)
    }

    fn poll_receiver(&mut self, link_id: &str, def: &ReceiverDef, link: &mut OpenLink) -> Result<Vec<DeviceReport>, HidppError> {
        link.ch.drain()?;
        if !link.notifications_checked {
            match receiver::ensure_notifications(&mut link.ch) {
                Ok(_) => link.notifications_checked = true,
                Err(e @ HidppError::Transport(_)) => return Err(e),
                Err(e) => self.errors.push(format!("{link_id}: notification flags: {e}")),
            }
        }
        let vid = def.vendor_id;
        let slots = receiver::paired_slots(&mut link.ch, def.family, def.max_devices)?;
        let mut out = vec![];
        for slot in &slots {
            let idx = slot.index;
            let pids = [slot.wpid];
            match link.ch.ping(idx) {
                Ok((maj, min)) if maj >= 2 => {
                    let cached = link.devices.get(&idx).filter(|i| i.protocol == (maj, min)).cloned();
                    let info = match cached {
                        Some(i) => i,
                        None => match device::probe(&mut link.ch, idx, (maj, min)) {
                            Ok(i) => {
                                link.devices.insert(idx, i.clone());
                                i
                            }
                            Err(e) => {
                                let r = Reading::with_status(status_for_ping_error(&e));
                                out.push(self.report(link_id, idx, Transport::Receiver, vid, &pids, None, Some(slot), r, "HID++"));
                                continue;
                            }
                        },
                    };
                    let mut all_pids = pids.to_vec();
                    all_pids.extend(&info.model_ids);
                    let (reading, src) = match info.battery {
                        Some(f) => match self.battery(link, idx, vid, &all_pids, f) {
                            Ok(r) => (r, f.label()),
                            Err(e) => {
                                link.devices.remove(&idx);
                                (Reading::with_status(status_for_ping_error(&e)), f.label())
                            }
                        },
                        None => (Reading::with_status(Status::NotReported), "HID++"),
                    };
                    out.push(self.report(
                        link_id,
                        idx,
                        Transport::Receiver,
                        vid,
                        &all_pids,
                        Some(&info),
                        Some(slot),
                        reading,
                        src,
                    ));
                }
                // HID++ 1.0 devices answer the 2.0 ping with "invalid sub-ID" (or an
                // old protocol version): listed as "battery not reported", never as garbage.
                Ok(_) | Err(HidppError::Hidpp10(err10::INVALID_SUBID)) => {
                    let info = link.devices.get(&idx).cloned();
                    out.push(self.report(
                        link_id,
                        idx,
                        Transport::Receiver,
                        vid,
                        &pids,
                        info.as_ref(),
                        Some(slot),
                        hidpp10_reading(),
                        "HID++ 1.0",
                    ));
                }
                Err(e @ HidppError::Transport(_)) => return Err(e),
                Err(e) => {
                    let info = link.devices.get(&idx).cloned();
                    let r = Reading::with_status(status_for_ping_error(&e));
                    out.push(self.report(link_id, idx, Transport::Receiver, vid, &pids, info.as_ref(), Some(slot), r, "HID++"));
                }
            }
        }
        // Forget cached devices that were unpaired.
        link.devices.retain(|idx, _| slots.iter().any(|s| s.index == *idx));
        Ok(out)
    }

    fn poll_direct(&mut self, link_id: &str, transport: Transport, link: &mut OpenLink) -> Result<Vec<DeviceReport>, HidppError> {
        link.ch.drain()?;
        let info_link = link.ch.transport.info().clone();
        let (vid, pid) = (info_link.vendor_id, info_link.product_id);
        let idx = RECEIVER_INDEX;
        let (maj, min) = match link.ch.ping(idx) {
            Ok(v) => v,
            Err(e @ HidppError::Transport(_)) => return Err(e),
            Err(e) => {
                let Some(info) = link.devices.get(&idx).cloned() else {
                    return Ok(vec![]);
                };
                let r = Reading::with_status(status_for_ping_error(&e));
                return Ok(vec![self.report(
                    link_id,
                    idx,
                    transport,
                    vid,
                    &[pid],
                    Some(&info),
                    None,
                    r,
                    "HID++",
                )]);
            }
        };
        let info = match link.devices.get(&idx).filter(|i| i.protocol == (maj, min)).cloned() {
            Some(i) => i,
            None => {
                let i = device::probe(&mut link.ch, idx, (maj, min))?;
                link.devices.insert(idx, i.clone());
                i
            }
        };
        // Devices without battery features (wired mice, microphones) are skipped silently.
        let Some(f) = info.battery else { return Ok(vec![]) };
        let mut pids = vec![pid];
        pids.extend(&info.model_ids);
        let reading = match self.battery(link, idx, vid, &pids, f) {
            Ok(r) => r,
            Err(e) => {
                link.devices.remove(&idx);
                Reading::with_status(status_for_ping_error(&e))
            }
        };
        Ok(vec![self.report(
            link_id,
            idx,
            transport,
            vid,
            &pids,
            Some(&info),
            None,
            reading,
            f.label(),
        )])
    }
}

/// Links replayed from a recorded trace (tests, `battbar probe --replay`).
pub struct ReplaySource {
    links: Vec<(String, LinkRole, bool, Option<transport::ReplayTransport>)>,
}

impl ReplaySource {
    pub fn new(trace: &transport::Trace, db: &DeviceDb) -> Self {
        let mut links = vec![];
        for t in trace.transports() {
            let info = t.info().clone();
            let role = if info.kind == "gatt" {
                LinkRole::Direct {
                    transport: Transport::BluetoothLe,
                }
            } else if let Some(r) = db.receiver(info.vendor_id, info.product_id) {
                LinkRole::Receiver(r.clone())
            } else if db.hidpp.vendor_ids.contains(&info.vendor_id) && info.collections.iter().any(|(p, _)| db.is_hidpp_usage_page(*p)) {
                LinkRole::Direct { transport: Transport::Usb }
            } else {
                continue;
            };
            let long_only = info.kind == "gatt"
                || !info
                    .collections
                    .iter()
                    .any(|(p, u)| db.is_hidpp_usage_page(*p) && (*u & 0xFF) as u8 == db.hidpp.short_usage);
            links.push((info.link.clone(), role, long_only, Some(t)));
        }
        ReplaySource { links }
    }
}

impl LinkSource for ReplaySource {
    fn scan(&mut self, _db: &DeviceDb) -> Vec<LinkCandidate> {
        self.links
            .iter_mut()
            .map(|(id, role, long_only, t)| {
                let t = t.take();
                LinkCandidate {
                    id: id.clone(),
                    role: role.clone(),
                    long_only: *long_only,
                    identity: vec![],
                    open: Box::new(move || {
                        t.map(|t| Box::new(t) as BoxTransport)
                            .ok_or(TransportError::Refused("already open".into()))
                    }),
                }
            })
            .collect()
    }
}
