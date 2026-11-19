// SPDX-License-Identifier: GPL-3.0-or-later
//! Merging per-provider reports into physical devices.
//!
//! Rules (docs/SPEC.md, "Device identity and merging"):
//! * reports sharing any [`IdentityKey`] are one device;
//! * a [`MergeHint::BluetoothCompanion`] merges with exactly one Bluetooth report from
//!   another provider whose name matches a pattern (ambiguous → no merge);
//! * manual groups / splits from the settings override both;
//! * percent comes from the source with the best precision (ties: provider priority),
//!   flags are combined, status is the best one.

use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MergeConfig {
    /// Each group lists full source keys (`provider/source_key`) forced into one device.
    #[serde(default)]
    pub manual_groups: Vec<Vec<String>>,
    /// Full source keys that must stay on their own.
    #[serde(default)]
    pub splits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// Stable ID, derived from the strongest identity available.
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub reading: Reading,
    /// Full key of the source that provided the percentage, if any.
    pub percent_source: Option<String>,
    pub components: Vec<Component>,
    pub sources: Vec<DeviceReport>,
    /// Set when the percentage is only an estimate because of charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<crate::charge::Estimate>,
}

impl Device {
    /// Transports through which the device is currently reachable.
    pub fn live_transports(&self) -> Vec<Transport> {
        let mut t: Vec<Transport> = self
            .sources
            .iter()
            .filter(|s| !matches!(s.reading.status, Status::Disconnected | Status::Error))
            .map(|s| s.transport)
            .collect();
        t.dedup();
        t
    }
}

struct Dsu(Vec<usize>);
impl Dsu {
    fn find(&mut self, x: usize) -> usize {
        if self.0[x] != x {
            let r = self.find(self.0[x]);
            self.0[x] = r;
        }
        self.0[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }
}

/// Case-insensitive glob with `*` and `?`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn is_bluetooth(t: Transport) -> bool {
    matches!(t, Transport::BluetoothLe | Transport::BluetoothClassic)
}

pub fn merge(reports: Vec<DeviceReport>, cfg: &MergeConfig) -> Vec<Device> {
    let n = reports.len();
    let mut dsu = Dsu((0..n).collect());
    let keys: Vec<String> = reports.iter().map(|r| r.full_key()).collect();
    let split: Vec<bool> = keys.iter().map(|k| cfg.splits.contains(k)).collect();

    // 1. Shared identity keys.
    let mut by_identity: BTreeMap<&IdentityKey, usize> = BTreeMap::new();
    for (i, r) in reports.iter().enumerate() {
        if split[i] {
            continue;
        }
        for id in &r.identity {
            match by_identity.get(id) {
                Some(&j) => dsu.union(i, j),
                None => {
                    by_identity.insert(id, i);
                }
            }
        }
    }

    // 2. Conditional Bluetooth companion merge.
    for (i, r) in reports.iter().enumerate() {
        if split[i] {
            continue;
        }
        for hint in &r.hints {
            let MergeHint::BluetoothCompanion { name_patterns } = hint;
            let candidates: Vec<usize> = reports
                .iter()
                .enumerate()
                .filter(|(j, b)| {
                    *j != i
                        && !split[*j]
                        && b.provider != r.provider
                        && is_bluetooth(b.transport)
                        && name_patterns.iter().any(|p| glob_match(p, &b.name))
                })
                .map(|(j, _)| j)
                .collect();
            if candidates.len() == 1 {
                dsu.union(i, candidates[0]);
            }
        }
    }

    // 3. Manual groups.
    for group in &cfg.manual_groups {
        let idx: Vec<usize> = group.iter().filter_map(|k| keys.iter().position(|x| x == k)).collect();
        for w in idx.windows(2) {
            dsu.union(w[0], w[1]);
        }
    }

    let mut groups: BTreeMap<usize, Vec<DeviceReport>> = BTreeMap::new();
    for (i, r) in reports.into_iter().enumerate() {
        let root = dsu.find(i);
        groups.entry(root).or_default().push(r);
    }
    let mut out: Vec<Device> = groups.into_values().map(build_device).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    out
}

fn device_id(sources: &[DeviceReport]) -> String {
    let mut ids: Vec<&IdentityKey> = sources.iter().flat_map(|s| s.identity.iter()).collect();
    ids.sort();
    match ids.first() {
        // Hashed so settings (hidden devices etc.) never hold raw unit IDs.
        Some(IdentityKey::HidppUnit { unit_id }) => format!("hidpp:{}", short_hash(&format!("battbar-unit:{unit_id}"))),
        Some(IdentityKey::Stable { namespace, value }) => format!("{namespace}:{value}"),
        None => sources.iter().map(|s| s.full_key()).min().unwrap_or_default(),
    }
}

/// Pick the reading that provides the percentage.
fn best<'a>(cands: impl Iterator<Item = (&'a DeviceReport, &'a Reading)>) -> Option<(&'a DeviceReport, &'a Reading)> {
    cands
        .filter(|(_, r)| r.has_value())
        .min_by_key(|(s, r)| (std::cmp::Reverse(r.precision.rank()), provider_priority(&s.provider)))
}

fn combine_flag(values: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let mut seen = None;
    for v in values.flatten() {
        if v {
            return Some(true);
        }
        seen = Some(false);
    }
    seen
}

fn merge_readings<'a>(items: &[(&'a DeviceReport, &'a Reading)]) -> (Reading, Option<String>) {
    let live: Vec<&Reading> = items
        .iter()
        .map(|(_, r)| *r)
        .filter(|r| matches!(r.status, Status::Ok | Status::NotReported))
        .collect();
    let charging = combine_flag(live.iter().map(|r| r.charging));
    let external_power = combine_flag(live.iter().map(|r| r.external_power));
    match best(items.iter().copied()) {
        Some((src, r)) => (
            Reading {
                percent: r.percent,
                charging,
                external_power,
                status: Status::Ok,
                precision: r.precision,
                voltage_mv: r.voltage_mv.or_else(|| items.iter().find_map(|(_, x)| x.voltage_mv)),
            },
            Some(src.full_key()),
        ),
        None => {
            let status = items
                .iter()
                .map(|(_, r)| r.status)
                .max_by_key(|s| s.rank())
                .unwrap_or(Status::Error);
            let mut r = Reading::with_status(status);
            r.charging = charging;
            r.external_power = external_power;
            (r, None)
        }
    }
}

fn build_device(mut sources: Vec<DeviceReport>) -> Device {
    sources.sort_by_key(|s| (provider_priority(&s.provider), s.full_key()));
    let items: Vec<(&DeviceReport, &Reading)> = sources.iter().map(|s| (s, &s.reading)).collect();
    let (reading, percent_source) = merge_readings(&items);

    let mut comp_kinds: Vec<ComponentKind> = sources.iter().flat_map(|s| s.components.iter().map(|c| c.kind)).collect();
    comp_kinds.sort();
    comp_kinds.dedup();
    let components = comp_kinds
        .into_iter()
        .map(|kind| {
            let items: Vec<(&DeviceReport, &Reading)> = sources
                .iter()
                .flat_map(|s| s.components.iter().filter(move |c| c.kind == kind).map(move |c| (s, &c.reading)))
                .collect();
            Component {
                kind,
                reading: merge_readings(&items).0,
            }
        })
        .collect();

    let name = sources[0].name.clone();
    let kind = sources
        .iter()
        .map(|s| s.kind)
        .find(|k| *k != DeviceKind::Other)
        .unwrap_or(DeviceKind::Other);
    let mut d = Device {
        id: device_id(&sources),
        name,
        kind,
        reading,
        percent_source,
        components,
        sources,
        estimate: None,
    };
    crate::charge::mark_charging(&mut d);
    d
}

/// Remembers which identity keys belong together (e.g. a BLE peripheral and a HID++
/// unit ID seen in the same report), so a later report that carries only one of them
/// (the GATT link is not open yet, the device is away) still merges into the same device.
///
/// Only hashes of the keys are kept, so the memory can be stored in the settings without
/// unit IDs or peripheral identifiers in clear text.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct IdentityMemory {
    classes: Vec<std::collections::BTreeSet<String>>,
}

fn key_hash(k: &IdentityKey) -> String {
    let raw = match k {
        IdentityKey::HidppUnit { unit_id } => format!("hidpp:{unit_id}"),
        IdentityKey::Stable { namespace, value } => format!("{namespace}:{value}"),
    };
    short_hash(&format!("battbar-identity:{raw}"))
}

impl IdentityMemory {
    pub fn from_stored(classes: &[Vec<String>]) -> Self {
        IdentityMemory {
            classes: classes.iter().map(|c| c.iter().cloned().collect()).collect(),
        }
    }

    pub fn to_stored(&self) -> Vec<Vec<String>> {
        let mut v: Vec<Vec<String>> = self.classes.iter().map(|c| c.iter().cloned().collect()).collect();
        v.sort();
        v
    }

    /// Record keys that appear together in one report. Returns true if anything new
    /// was learned.
    pub fn learn(&mut self, reports: &[DeviceReport]) -> bool {
        let before = self.to_stored();
        for r in reports.iter().filter(|r| r.identity.len() > 1) {
            let mut set: std::collections::BTreeSet<String> = r.identity.iter().map(key_hash).collect();
            let mut i = 0;
            while i < self.classes.len() {
                if !self.classes[i].is_disjoint(&set) {
                    set.extend(self.classes.swap_remove(i));
                } else {
                    i += 1;
                }
            }
            self.classes.push(set);
        }
        self.to_stored() != before
    }

    /// Give reports whose keys are known to belong together each other's keys, so
    /// `merge` puts them into one device.
    pub fn augment(&self, reports: &mut [DeviceReport]) {
        let class_of = |r: &DeviceReport| -> Option<usize> {
            let hs: Vec<String> = r.identity.iter().map(key_hash).collect();
            self.classes.iter().position(|c| hs.iter().any(|h| c.contains(h)))
        };
        let classes: Vec<Option<usize>> = reports.iter().map(class_of).collect();
        let mut extra: Vec<Vec<IdentityKey>> = vec![vec![]; reports.len()];
        for (i, ci) in classes.iter().enumerate() {
            let Some(ci) = ci else { continue };
            for (j, cj) in classes.iter().enumerate() {
                if i != j && cj == &Some(*ci) {
                    extra[i].extend(reports[j].identity.iter().cloned());
                }
            }
        }
        for (r, e) in reports.iter_mut().zip(extra) {
            for k in e {
                if !r.identity.contains(&k) {
                    r.identity.push(k);
                }
            }
        }
    }
}

/// Delays the first appearance of a device until its picture is complete: a new device
/// is shown once it has been seen in a complete poll (every provider answered) at least
/// `settle` after it first appeared, or after `max_wait` in any case. This hides
/// transient splits such as a BLE Battery Service report arriving before the GATT
/// HID++ link that would merge it with a known device.
#[derive(Debug)]
pub struct NewDeviceGate {
    first_seen: std::collections::HashMap<String, std::time::Instant>,
    confirmed: std::collections::HashSet<String>,
    pub settle: std::time::Duration,
    pub max_wait: std::time::Duration,
}

impl Default for NewDeviceGate {
    fn default() -> Self {
        NewDeviceGate {
            first_seen: Default::default(),
            confirmed: Default::default(),
            settle: std::time::Duration::from_secs(3),
            max_wait: std::time::Duration::from_secs(10),
        }
    }
}

impl NewDeviceGate {
    /// IDs of `devices` that may be shown now.
    pub fn admit(&mut self, now: std::time::Instant, devices: &[Device], poll_complete: bool) -> Vec<String> {
        let present: std::collections::HashSet<&String> = devices.iter().map(|d| &d.id).collect();
        self.first_seen.retain(|id, _| present.contains(id));
        let mut out = vec![];
        for d in devices {
            if self.confirmed.contains(&d.id) {
                out.push(d.id.clone());
                continue;
            }
            let t0 = *self.first_seen.entry(d.id.clone()).or_insert(now);
            let waited = now.saturating_duration_since(t0);
            if (poll_complete && waited >= self.settle) || waited >= self.max_wait {
                self.confirmed.insert(d.id.clone());
                self.first_seen.remove(&d.id);
                out.push(d.id.clone());
            }
        }
        out
    }

    /// Whether some device is waiting (the caller schedules a follow-up poll).
    pub fn pending(&self) -> bool {
        !self.first_seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(provider: &str, key: &str, name: &str, t: Transport, reading: Reading) -> DeviceReport {
        DeviceReport {
            provider: provider.into(),
            source_key: key.into(),
            source: "test".into(),
            name: name.into(),
            kind: DeviceKind::Other,
            transport: t,
            identity: vec![],
            hints: vec![],
            reading,
            components: vec![],
        }
    }

    fn unit(u: &str) -> IdentityKey {
        IdentityKey::HidppUnit { unit_id: u.into() }
    }

    #[test]
    fn glob() {
        assert!(glob_match("Arctis Nova Pro*", "arctis nova pro wireless"));
        assert!(glob_match("*Nova?Pro*", "Arctis Nova Pro Wireless"));
        assert!(!glob_match("Arctis Nova 7*", "Arctis Nova Pro Wireless"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn same_unit_id_on_two_transports_is_one_device() {
        let mut a = rep(
            "hidpp",
            "receiver:c547:1",
            "Keyboard",
            Transport::Receiver,
            Reading::with_status(Status::Disconnected),
        );
        a.identity = vec![unit("u1")];
        let mut mut_b = Reading::ok(98, Precision::Approximate);
        mut_b.voltage_mv = Some(4158);
        let mut b = rep("hidpp", "gatt:abc", "Keyboard", Transport::BluetoothLe, mut_b);
        b.identity = vec![unit("u1")];
        let devs = merge(vec![a, b], &MergeConfig::default());
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].reading.percent, Some(98));
        assert_eq!(devs[0].reading.status, Status::Ok);
        assert_eq!(devs[0].live_transports(), vec![Transport::BluetoothLe]);
        assert!(devs[0].id.starts_with("hidpp:") && !devs[0].id.contains("u1"));
    }

    #[test]
    fn best_precision_wins_and_flags_are_combined() {
        let mut base = Reading::ok(100, Precision::Coarse);
        base.charging = Some(true);
        let mut a = rep("steelseries", "base:12e0", "Arctis Nova Pro Wireless", Transport::BaseStation, base);
        a.hints = vec![MergeHint::BluetoothCompanion {
            name_patterns: vec!["Arctis Nova Pro*".into()],
        }];
        let b = rep(
            "bt-system",
            "classic:1",
            "Arctis Nova Pro Wireless",
            Transport::BluetoothClassic,
            Reading::ok(85, Precision::Exact),
        );
        let devs = merge(vec![a, b], &MergeConfig::default());
        assert_eq!(devs.len(), 1);
        let d = &devs[0];
        assert_eq!(d.reading.percent, Some(85));
        assert_eq!(d.reading.charging, Some(true));
        // Charging (flag from the base): the percentage is shown as an estimate.
        assert_eq!(d.reading.precision, Precision::Approximate);
        assert_eq!(d.estimate, Some(crate::charge::Estimate::Charging));
        assert_eq!(d.percent_source.as_deref(), Some("bt-system/classic:1"));
        assert_eq!(d.name, "Arctis Nova Pro Wireless");
    }

    #[test]
    fn precision_tie_prefers_vendor_protocol() {
        let mut a = rep(
            "bt-system",
            "ble:1",
            "Kbd",
            Transport::BluetoothLe,
            Reading::ok(100, Precision::Exact),
        );
        a.identity = vec![IdentityKey::Stable {
            namespace: "x".into(),
            value: "1".into(),
        }];
        let mut b = rep("hidpp", "gatt:1", "Kbd", Transport::BluetoothLe, Reading::ok(97, Precision::Exact));
        b.identity = a.identity.clone();
        let d = &merge(vec![a, b], &MergeConfig::default())[0];
        assert_eq!(d.reading.percent, Some(97));
    }

    #[test]
    fn no_companion_merge_by_name_alone() {
        // Base does not report "headset on Bluetooth" -> no hint -> two devices.
        let a = rep(
            "steelseries",
            "base:12e0",
            "Arctis Nova Pro Wireless",
            Transport::BaseStation,
            Reading::ok(100, Precision::Coarse),
        );
        let b = rep(
            "bt-system",
            "classic:1",
            "Arctis Nova Pro Wireless",
            Transport::BluetoothClassic,
            Reading::ok(85, Precision::Coarse),
        );
        assert_eq!(merge(vec![a, b], &MergeConfig::default()).len(), 2);
    }

    #[test]
    fn companion_requires_pattern_match_and_is_unambiguous() {
        let hint = vec![MergeHint::BluetoothCompanion {
            name_patterns: vec!["Arctis Nova Pro*".into()],
        }];
        let mut a = rep(
            "steelseries",
            "base",
            "Arctis Nova Pro Wireless",
            Transport::BaseStation,
            Reading::ok(100, Precision::Coarse),
        );
        a.hints = hint.clone();
        let b = rep(
            "bt-system",
            "c1",
            "Other Headphones",
            Transport::BluetoothClassic,
            Reading::ok(50, Precision::Coarse),
        );
        assert_eq!(merge(vec![a.clone(), b], &MergeConfig::default()).len(), 2);

        let b1 = rep(
            "bt-system",
            "c1",
            "Arctis Nova Pro Wireless",
            Transport::BluetoothClassic,
            Reading::ok(50, Precision::Coarse),
        );
        let b2 = rep(
            "bt-system",
            "c2",
            "Arctis Nova Pro Wireless",
            Transport::BluetoothClassic,
            Reading::ok(60, Precision::Coarse),
        );
        assert_eq!(merge(vec![a, b1, b2], &MergeConfig::default()).len(), 3);
    }

    #[test]
    fn manual_group_and_split() {
        let a = rep(
            "bt-system",
            "c1",
            "Headphones A",
            Transport::BluetoothClassic,
            Reading::ok(50, Precision::Coarse),
        );
        let b = rep(
            "steelseries",
            "base",
            "Headset",
            Transport::BaseStation,
            Reading::ok(60, Precision::Coarse),
        );
        let cfg = MergeConfig {
            manual_groups: vec![vec!["bt-system/c1".into(), "steelseries/base".into()]],
            splits: vec![],
        };
        assert_eq!(merge(vec![a.clone(), b.clone()], &cfg).len(), 1);

        let mut a2 = a.clone();
        a2.identity = vec![unit("u")];
        let mut b2 = b.clone();
        b2.identity = vec![unit("u")];
        let cfg = MergeConfig {
            manual_groups: vec![],
            splits: vec!["bt-system/c1".into()],
        };
        assert_eq!(merge(vec![a2, b2], &cfg).len(), 2);
    }

    #[test]
    fn asleep_never_becomes_zero_percent() {
        let a = rep("hidpp", "r:2", "Mouse", Transport::Receiver, Reading::with_status(Status::Asleep));
        let d = &merge(vec![a], &MergeConfig::default())[0];
        assert_eq!(d.reading.percent, None);
        assert_eq!(d.reading.status, Status::Asleep);
    }

    #[test]
    fn components_merge_per_kind() {
        let mut a = rep(
            "steelseries",
            "base",
            "H",
            Transport::BaseStation,
            Reading::ok(100, Precision::Coarse),
        );
        a.components = vec![Component {
            kind: ComponentKind::Spare,
            reading: Reading::ok(75, Precision::Coarse),
        }];
        let d = &merge(vec![a], &MergeConfig::default())[0];
        assert_eq!(d.components.len(), 1);
        assert_eq!(d.components[0].reading.percent, Some(75));
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    fn rep(provider: &str, key: &str, t: Transport, identity: Vec<IdentityKey>) -> DeviceReport {
        DeviceReport {
            provider: provider.into(),
            source_key: key.into(),
            source: "t".into(),
            name: "Keyboard".into(),
            kind: DeviceKind::Keyboard,
            transport: t,
            identity,
            hints: vec![],
            reading: Reading::ok(100, Precision::Exact),
            components: vec![],
        }
    }

    #[test]
    fn ble_report_without_gatt_link_still_merges_with_receiver_slot() {
        let unit = IdentityKey::HidppUnit { unit_id: "u".into() };
        let ble = IdentityKey::Stable {
            namespace: "ble".into(),
            value: "p".into(),
        };
        let mut mem = IdentityMemory::default();
        // Earlier poll: GATT link reported both keys.
        mem.learn(&[rep("hidpp", "gatt:p", Transport::BluetoothLe, vec![unit.clone(), ble.clone()])]);
        // Now: GATT link not open yet; only the Battery Service report and the receiver slot.
        let mut now = vec![
            rep("bt-system", "ble:p", Transport::BluetoothLe, vec![ble.clone()]),
            rep("hidpp", "receiver:1", Transport::Receiver, vec![unit.clone()]),
        ];
        assert_eq!(merge(now.clone(), &MergeConfig::default()).len(), 2);
        mem.augment(&mut now);
        assert_eq!(merge(now, &MergeConfig::default()).len(), 1);
    }
}
