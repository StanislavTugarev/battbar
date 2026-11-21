// SPDX-License-Identifier: GPL-3.0-or-later
//! Battery values macOS already knows for paired Bluetooth devices. This provider
//! speaks no vendor protocol and never scans:
//!
//! * Bluetooth LE: standard Battery Service (0x180F/0x2A19) of peripherals connected to
//!   this Mac (CoreBluetooth, retrieve-connected only); fallback `system_profiler`.
//! * Classic audio: IOBluetooth `batteryPercentSingle/Left/Right/Case` (guarded).

pub mod system_profiler;

use battbar_core::*;
use std::time::{Duration, Instant};
use system_profiler::SpDevice;

/// Classic battery value as macOS reports it, after the decisions in SPEC.md.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicInput {
    pub name: String,
    pub address_hash: String,
    pub connected: bool,
    /// Bluetooth Class of Device.
    pub class_of_device: u32,
    pub single: Option<u8>,
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
}

/// Precision of IOBluetooth `batteryPercent*` for classic audio. Observed 1 % steps
/// (85 → 84) on the reference headset (HFP 1.7 battery indicator), see DISCOVERY.md.
pub const CLASSIC_PRECISION: Precision = Precision::Exact;

/// Device kind from the Bluetooth Class of Device (Assigned Numbers, Baseband).
pub fn kind_from_cod(cod: u32) -> DeviceKind {
    let major = (cod >> 8) & 0x1F;
    let minor = (cod >> 2) & 0x3F;
    match major {
        0x04 if minor == 0x01 => DeviceKind::Headset,
        0x04 => DeviceKind::Headphones,
        0x05 if minor & 0x0F == 0x01 || minor & 0x0F == 0x02 => DeviceKind::Gamepad,
        0x05 if minor & 0x30 == 0x10 => DeviceKind::Keyboard,
        0x05 if minor & 0x30 == 0x20 => DeviceKind::Mouse,
        _ => DeviceKind::Other,
    }
}

pub fn classic_report(d: &ClassicInput) -> DeviceReport {
    let value = |v: Option<u8>| v.filter(|p| *p > 0);
    let comps: Vec<Component> = [
        (ComponentKind::Left, d.left),
        (ComponentKind::Right, d.right),
        (ComponentKind::Case, d.case),
    ]
    .into_iter()
    .filter_map(|(k, v)| {
        value(v).map(|p| Component {
            kind: k,
            reading: Reading::ok(p, CLASSIC_PRECISION),
        })
    })
    .collect();
    let reading = if !d.connected {
        Reading::with_status(Status::Disconnected)
    } else if let Some(p) = value(d.single) {
        Reading::ok(p, CLASSIC_PRECISION)
    } else if let Some(min) = comps
        .iter()
        .filter(|c| c.kind != ComponentKind::Case)
        .filter_map(|c| c.reading.percent)
        .min()
    {
        // Earbuds: the device value is the lower bud.
        Reading::ok(min, CLASSIC_PRECISION)
    } else {
        // Property missing or 0 on a connected device: "battery not reported".
        Reading::with_status(Status::NotReported)
    };
    DeviceReport {
        provider: "bt-system".into(),
        source_key: format!("classic:{}", d.address_hash),
        source: "macOS (Bluetooth)".into(),
        name: d.name.clone(),
        kind: kind_from_cod(d.class_of_device),
        transport: Transport::BluetoothClassic,
        identity: vec![IdentityKey::Stable {
            namespace: "bt".into(),
            value: d.address_hash.clone(),
        }],
        hints: vec![],
        reading,
        components: if d.connected { comps } else { vec![] },
    }
}

pub fn ble_report(id_hash: &str, name: &str, kind: DeviceKind, percent: Option<u8>, source: &str) -> DeviceReport {
    DeviceReport {
        provider: "bt-system".into(),
        source_key: format!("ble:{id_hash}"),
        source: source.into(),
        name: name.into(),
        kind,
        transport: Transport::BluetoothLe,
        identity: if id_hash.starts_with("sp-") {
            vec![]
        } else {
            vec![IdentityKey::Stable {
                namespace: "ble".into(),
                value: id_hash.into(),
            }]
        },
        hints: vec![],
        reading: match percent {
            Some(p) => Reading::ok(p, Precision::Exact),
            None => Reading::with_status(Status::NotReported),
        },
        components: vec![],
    }
}

/// `system_profiler` fallback for connected BLE devices not covered by CoreBluetooth.
pub fn sp_fallback_reports(sp: &[SpDevice], covered_names: &[String]) -> Vec<DeviceReport> {
    sp.iter()
        .filter(|d| d.connected && d.ble && !covered_names.iter().any(|n| n == &d.name))
        .map(|d| {
            let mut r = ble_report(
                &format!("sp-{}", short_hash(&d.name)),
                &d.name,
                d.kind(),
                d.battery_main,
                "macOS (system_profiler)",
            );
            r.components = [
                (ComponentKind::Left, d.battery_left),
                (ComponentKind::Right, d.battery_right),
                (ComponentKind::Case, d.battery_case),
            ]
            .into_iter()
            .filter_map(|(k, v)| {
                v.map(|p| Component {
                    kind: k,
                    reading: Reading::ok(p, Precision::Exact),
                })
            })
            .collect();
            if r.reading.percent.is_none() {
                if let Some(min) = r
                    .components
                    .iter()
                    .filter(|c| c.kind != ComponentKind::Case)
                    .filter_map(|c| c.reading.percent)
                    .min()
                {
                    r.reading = Reading::ok(min, Precision::Exact);
                }
            }
            r
        })
        .collect()
}

pub struct BtSystemProvider {
    sp_cache: Option<(Instant, Vec<SpDevice>)>,
    last_connected: Option<Vec<String>>,
    /// Safety re-check of connections (IOBluetooth notifications and HID hotplug do the
    /// real work in the app).
    rescan: battbar_core::Rescan,
    errors: Vec<String>,
    /// Show/hide is a front-end decision; the provider always reports.
    pub sp_refresh: Duration,
}

impl Default for BtSystemProvider {
    fn default() -> Self {
        BtSystemProvider {
            sp_cache: None,
            last_connected: None,
            rescan: battbar_core::Rescan::default(),
            errors: vec![],
            sp_refresh: Duration::from_secs(300),
        }
    }
}

impl BtSystemProvider {
    fn system_profiler(&mut self, force: bool) -> Vec<SpDevice> {
        let fresh = self.sp_cache.as_ref().is_some_and(|(t, _)| t.elapsed() < self.sp_refresh);
        if !fresh || force {
            if let Some(json) = system_profiler::run(Duration::from_secs(10)) {
                self.sp_cache = Some((Instant::now(), system_profiler::parse(&json)));
            }
        }
        self.sp_cache.as_ref().map(|(_, v)| v.clone()).unwrap_or_default()
    }
}

#[cfg(target_os = "macos")]
impl BatteryProvider for BtSystemProvider {
    fn name(&self) -> &'static str {
        "bt-system"
    }

    fn poll(&mut self) -> Vec<DeviceReport> {
        self.rescan.scanned(Instant::now());
        use battbar_macos::ble::{Central, GattSession};
        let mut out = vec![];
        self.errors.clear();
        if !battbar_macos::tcc::bluetooth_usage_description_present() {
            self.errors.push("no Bluetooth usage description; Bluetooth sources skipped".into());
            return out;
        }

        // Ask for / check the permission through CoreBluetooth first: IOBluetooth calls
        // block while the permission prompt is unanswered.
        let central = Central::shared();
        if !battbar_macos::ble::is_authorized() {
            self.errors.push("Bluetooth permission not granted".into());
            return out;
        }

        // Classic devices (IOBluetooth). Battery values are read in a short-lived helper
        // process: in a long-running one IOBluetooth reports them only once.
        for d in battbar_macos::classic::paired_devices_fresh()
            .into_iter()
            .filter(|d| d.is_classic())
        {
            out.push(classic_report(&ClassicInput {
                name: d.name.clone(),
                address_hash: keyed_hash("bt-address", &d.address),
                connected: d.connected,
                class_of_device: d.class_of_device,
                single: d.battery_single,
                left: d.battery_left,
                right: d.battery_right,
                case: d.battery_case,
            }));
        }

        // BLE Battery Service.
        let mut covered = vec![];
        let mut ble_ok = false;
        match central.and_then(|c| Ok((c, c.connected_peripherals(&["180F"])?))) {
            Ok((central, peripherals)) => {
                ble_ok = true;
                let sp = if peripherals.is_empty() {
                    vec![]
                } else {
                    self.system_profiler(false)
                };
                for p in peripherals {
                    let name = p.name.clone().unwrap_or_else(|| "Bluetooth device".into());
                    let kind = sp.iter().find(|s| s.name == name).map(|s| s.kind()).unwrap_or(DeviceKind::Other);
                    let percent = GattSession::open(central, &p).and_then(|mut s| {
                        let c = s.characteristic("180F", "2A19")?;
                        s.read(&c)
                    });
                    let percent = match percent {
                        Ok(v) => v.first().map(|b| (*b).min(100)),
                        Err(e) => {
                            self.errors.push(format!("ble {}: {e}", keyed_hash("ble", &p.id)));
                            None
                        }
                    };
                    covered.push(name.clone());
                    out.push(ble_report(&keyed_hash("ble", &p.id), &name, kind, percent, "BLE Battery Service"));
                }
            }
            Err(e) => self.errors.push(format!("CoreBluetooth: {e}")),
        }

        // Fallback for BLE devices CoreBluetooth did not cover (or could not be used).
        let sp = self.system_profiler(!ble_ok);
        out.extend(sp_fallback_reports(&sp, &covered));
        out
    }

    /// Connect/disconnect of Bluetooth devices, as a rare safety net: the app learns them
    /// from IOBluetooth connect notifications (classic) and HID hotplug (Bluetooth HID
    /// devices). Bluetooth LE devices that are not HID are seen at the next poll.
    fn check_events(&mut self) -> bool {
        if !self.rescan.due(Instant::now()) || !battbar_macos::ble::is_authorized() {
            return false;
        }
        let mut now: Vec<String> = battbar_macos::classic::paired_devices()
            .into_iter()
            .filter(|d| d.connected)
            .map(|d| keyed_hash("bt-address", &d.address))
            .collect();
        if let Ok(c) = battbar_macos::ble::Central::shared() {
            if let Ok(ps) = c.connected_peripherals(&["180F"]) {
                now.extend(ps.into_iter().map(|p| p.id));
            }
        }
        now.sort();
        let changed = self.last_connected.as_ref().is_some_and(|prev| *prev != now);
        self.last_connected = Some(now);
        changed
    }

    fn diagnostics(&mut self) -> serde_json::Value {
        if !battbar_macos::ble::is_authorized() {
            return serde_json::json!({"bluetooth_authorization": battbar_macos::ble::authorization(), "error": "Bluetooth permission not granted"});
        }
        let paired: Vec<serde_json::Value> = battbar_macos::classic::paired_devices_fresh()
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name, "connected": d.connected, "class_of_device": format!("0x{:06x}", d.class_of_device),
                    "battery_properties": {
                        "single": d.battery_single, "left": d.battery_left, "right": d.battery_right, "case": d.battery_case
                    }
                })
            })
            .collect();
        serde_json::json!({
            "bluetooth_authorization": battbar_macos::ble::authorization(),
            "usage_description": battbar_macos::tcc::bluetooth_usage_description_present(),
            "paired": paired,
            "errors": self.errors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headset(connected: bool, single: Option<u8>) -> ClassicInput {
        ClassicInput {
            name: "Arctis Nova Pro Wireless".into(),
            address_hash: "abc".into(),
            connected,
            class_of_device: 0x240418,
            single,
            left: Some(0),
            right: Some(0),
            case: Some(0),
        }
    }

    #[test]
    fn classic_headset_reference_value() {
        let r = classic_report(&headset(true, Some(84)));
        assert_eq!(r.reading.percent, Some(84));
        assert_eq!(r.transport, Transport::BluetoothClassic);
        assert_eq!(r.kind, DeviceKind::Headphones);
        assert!(r.components.is_empty(), "zero components are 'not reported'");
    }

    #[test]
    fn classic_zero_or_missing_is_not_reported() {
        assert_eq!(classic_report(&headset(true, Some(0))).reading.status, Status::NotReported);
        let mut d = headset(true, None);
        d.left = None;
        d.right = None;
        d.case = None;
        let r = classic_report(&d);
        assert_eq!(r.reading.status, Status::NotReported);
        assert_eq!(r.reading.percent, None);
    }

    #[test]
    fn classic_disconnected_has_no_percent() {
        let r = classic_report(&headset(false, Some(50)));
        assert_eq!(r.reading.status, Status::Disconnected);
        assert_eq!(r.reading.percent, None);
    }

    #[test]
    fn class_of_device_kinds() {
        assert_eq!(kind_from_cod(0x240418), DeviceKind::Headphones); // reference headset
        assert_eq!(kind_from_cod(0x002508), DeviceKind::Gamepad); // DualSense
        assert_eq!(kind_from_cod(0x002540), DeviceKind::Keyboard);
        assert_eq!(kind_from_cod(0x002580), DeviceKind::Mouse);
        assert_eq!(kind_from_cod(0x200404), DeviceKind::Headset);
    }

    #[test]
    fn earbuds_components() {
        let d = ClassicInput {
            name: "Buds".into(),
            address_hash: "x".into(),
            connected: true,
            class_of_device: 0x240418,
            single: Some(0),
            left: Some(80),
            right: Some(60),
            case: Some(30),
        };
        let r = classic_report(&d);
        assert_eq!(r.reading.percent, Some(60));
        assert_eq!(r.components.len(), 3);
    }

    #[test]
    fn sp_fallback_skips_covered_and_classic() {
        let sp = system_profiler::parse(
            r#"{"SPBluetoothDataType":[{"device_connected":[
            {"G915 KEYBOARD":{"device_batteryLevelMain":"100%","device_minorType":"Keyboard","device_services":"< BLE >"}},
            {"Other Mouse":{"device_batteryLevelMain":"40%","device_minorType":"Mouse","device_services":"< BLE >"}},
            {"Arctis":{"device_minorType":"Headphones","device_services":"< HFP A2DP >"}}]}]}"#,
        );
        let r = sp_fallback_reports(&sp, &["G915 KEYBOARD".to_string()]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "Other Mouse");
        assert_eq!(r[0].kind, DeviceKind::Mouse);
        assert_eq!(r[0].reading.percent, Some(40));
    }
}
