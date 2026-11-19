// SPDX-License-Identifier: GPL-3.0-or-later
//! Device database: `devices/*.toml`, embedded at build time, with user overrides in
//! `~/.config/battbar/devices/*.toml`. Adding a device that speaks an already supported
//! protocol is a data change only.

use crate::model::{Chemistry, DeviceKind, Precision};
use serde::Deserialize;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/embedded_devices.rs"));

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbFile {
    #[serde(default)]
    pub hidpp: Option<HidppConfig>,
    #[serde(default, rename = "receiver")]
    pub receivers: Vec<ReceiverDef>,
    #[serde(default, rename = "device")]
    pub devices: Vec<DeviceDef>,
    #[serde(default, rename = "curve")]
    pub curves: Vec<CurveDef>,
}

/// Where HID++ lives on HID interfaces and on BLE.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HidppConfig {
    /// USB vendor IDs whose vendor interfaces may speak HID++.
    pub vendor_ids: Vec<u16>,
    /// Vendor usage pages that carry HID++ collections.
    pub usage_pages: Vec<u16>,
    /// Low byte of the collection usage for short (0x10) reports.
    pub short_usage: u8,
    /// Low byte of the collection usage for long (0x11) reports.
    pub long_usage: u8,
    /// Low byte of the collection usage for very long (0x12) reports.
    #[serde(default)]
    pub very_long_usage: Option<u8>,
    /// Logitech vendor GATT service / characteristic for HID++ over BLE.
    pub gatt_service: String,
    pub gatt_characteristic: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverFamily {
    Unifying,
    Lightspeed,
    Bolt,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReceiverDef {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: String,
    pub family: ReceiverFamily,
    #[serde(default = "default_max_devices")]
    pub max_devices: u8,
}

fn default_max_devices() -> u8 {
    6
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    Hidpp20,
    SteelseriesNovaPro,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DeviceDef {
    pub name: String,
    pub kind: DeviceKind,
    pub protocol: Protocol,
    pub vendor_id: u16,
    /// Wireless PIDs as seen through a receiver (HID++ wpid).
    #[serde(default)]
    pub wpid: Vec<u16>,
    /// PIDs on Bluetooth LE.
    #[serde(default)]
    pub ble_pid: Vec<u16>,
    /// PIDs on Bluetooth Classic.
    #[serde(default)]
    pub bt_pid: Vec<u16>,
    /// PIDs on USB (cable or base station).
    #[serde(default)]
    pub usb_pid: Vec<u16>,
    /// Vendor usage page of the status interface (base stations).
    #[serde(default)]
    pub vendor_usage_page: Option<u16>,
    /// Discharge curve ID for voltage-only battery features.
    #[serde(default)]
    pub voltage_curve: Option<String>,
    /// Battery chemistry; `alkaline` / `nimh` for AA/AAA devices.
    #[serde(default)]
    pub chemistry: Option<Chemistry>,
    /// Cells in series (AA/AAA devices); inferred from the voltage if absent.
    #[serde(default)]
    pub cells: Option<u8>,
    /// Bluetooth names of the same device (for conditional merging).
    #[serde(default)]
    pub bt_name_patterns: Vec<String>,
    /// The BLE Battery Service value of this device does not follow its charge (e.g.
    /// stays at 100 %); the vendor protocol is preferred and it is never used for
    /// voltage calibration.
    #[serde(default)]
    pub ble_battery_service_unreliable: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CurveDef {
    pub id: String,
    pub precision: Precision,
    /// Chemistry this curve describes (used to pick a curve by chemistry).
    #[serde(default)]
    pub chemistry: Option<Chemistry>,
    /// Points are per cell; the pack voltage is divided by the number of cells.
    #[serde(default)]
    pub per_cell: bool,
    /// `[millivolts, percent]`, any order.
    pub points: Vec<[u16; 2]>,
    #[serde(default)]
    pub source: Option<String>,
}

impl CurveDef {
    /// Linear interpolation over the curve, clamped to 0..=100.
    pub fn percent(&self, mv: u16) -> u8 {
        let mut pts = self.points.clone();
        pts.sort_by_key(|p| p[0]);
        let Some(first) = pts.first() else { return 0 };
        if mv <= first[0] {
            return first[1].min(100) as u8;
        }
        for w in pts.windows(2) {
            let ([v0, p0], [v1, p1]) = (w[0], w[1]);
            if mv <= v1 {
                let (v0, p0, v1, p1, mv) = (v0 as f64, p0 as f64, v1 as f64, p1 as f64, mv as f64);
                let p = p0 + (p1 - p0) * (mv - v0) / (v1 - v0).max(1.0);
                return p.round().clamp(0.0, 100.0) as u8;
            }
        }
        pts.last().map(|p| p[1].min(100) as u8).unwrap_or(0)
    }
}

pub const DEFAULT_CURVE_ID: &str = "li-ion-default";

#[derive(Debug, Clone)]
pub struct DeviceDb {
    pub hidpp: HidppConfig,
    pub receivers: Vec<ReceiverDef>,
    pub devices: Vec<DeviceDef>,
    pub curves: Vec<CurveDef>,
    /// Files that were loaded, in order (for `battbar dump`).
    pub sources: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("{file}: {err}")]
    Parse { file: String, err: String },
    #[error("no [hidpp] section in any device database file")]
    MissingHidpp,
    #[error("device {device:?} refers to unknown curve {curve:?}")]
    UnknownCurve { device: String, curve: String },
}

impl DeviceDb {
    /// Built-in database only.
    pub fn embedded() -> Result<Self, DbError> {
        Self::from_files(EMBEDDED_DEVICES.iter().map(|(n, c)| (format!("builtin:{n}"), c.to_string())))
    }

    /// Built-in database plus user overrides from `dir` (usually `~/.config/battbar/devices`).
    pub fn load_with_overrides(dir: Option<&Path>) -> Result<Self, DbError> {
        let mut files: Vec<(String, String)> = EMBEDDED_DEVICES
            .iter()
            .map(|(n, c)| (format!("builtin:{n}"), c.to_string()))
            .collect();
        if let Some(dir) = dir {
            if let Ok(rd) = std::fs::read_dir(dir) {
                let mut paths: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                    .collect();
                paths.sort();
                for p in paths {
                    if let Ok(c) = std::fs::read_to_string(&p) {
                        files.push((p.display().to_string(), c));
                    }
                }
            }
        }
        Self::from_files(files)
    }

    pub fn user_dir() -> Option<PathBuf> {
        crate::settings::config_dir().map(|d| d.join("devices"))
    }

    pub fn from_files(files: impl IntoIterator<Item = (String, String)>) -> Result<Self, DbError> {
        let mut hidpp: Option<HidppConfig> = None;
        let mut receivers: Vec<ReceiverDef> = vec![];
        let mut devices: Vec<DeviceDef> = vec![];
        let mut curves: Vec<CurveDef> = vec![];
        let mut sources = vec![];
        for (name, content) in files {
            let f: DbFile = toml::from_str(&content).map_err(|e| DbError::Parse {
                file: name.clone(),
                err: e.to_string(),
            })?;
            if let Some(h) = f.hidpp {
                hidpp = Some(match hidpp {
                    // Later files may add usage pages; scalar fields are overridden.
                    Some(mut prev) => {
                        for p in &h.usage_pages {
                            if !prev.usage_pages.contains(p) {
                                prev.usage_pages.push(*p);
                            }
                        }
                        for v in &h.vendor_ids {
                            if !prev.vendor_ids.contains(v) {
                                prev.vendor_ids.push(*v);
                            }
                        }
                        HidppConfig {
                            usage_pages: prev.usage_pages,
                            vendor_ids: prev.vendor_ids,
                            ..h
                        }
                    }
                    None => h,
                });
            }
            // Later definitions override earlier ones with the same key.
            for r in f.receivers {
                receivers.retain(|x| !(x.vendor_id == r.vendor_id && x.product_id == r.product_id));
                receivers.push(r);
            }
            for d in f.devices {
                devices.retain(|x| !(x.protocol == d.protocol && x.name == d.name));
                devices.push(d);
            }
            for c in f.curves {
                curves.retain(|x| x.id != c.id);
                curves.push(c);
            }
            sources.push(name);
        }
        let db = DeviceDb {
            hidpp: hidpp.ok_or(DbError::MissingHidpp)?,
            receivers,
            devices,
            curves,
            sources,
        };
        for d in &db.devices {
            if let Some(c) = &d.voltage_curve {
                if db.curve(c).is_none() {
                    return Err(DbError::UnknownCurve {
                        device: d.name.clone(),
                        curve: c.clone(),
                    });
                }
            }
        }
        Ok(db)
    }

    pub fn receiver(&self, vid: u16, pid: u16) -> Option<&ReceiverDef> {
        self.receivers.iter().find(|r| r.vendor_id == vid && r.product_id == pid)
    }

    pub fn curve(&self, id: &str) -> Option<&CurveDef> {
        self.curves.iter().find(|c| c.id == id)
    }

    /// Curve for a device, or the default Li-ion curve (always `approximate`).
    pub fn curve_for(&self, dev: Option<&DeviceDef>) -> Option<(&CurveDef, Precision)> {
        if let Some(c) = dev.and_then(|d| d.voltage_curve.as_deref()).and_then(|id| self.curve(id)) {
            return Some((c, c.precision));
        }
        self.curve(DEFAULT_CURVE_ID).map(|c| (c, Precision::Approximate))
    }

    /// Per-cell curve for a replaceable-battery chemistry.
    pub fn chemistry_curve(&self, chem: Chemistry) -> Option<&CurveDef> {
        self.curves.iter().find(|c| c.chemistry == Some(chem) && c.per_cell)
    }

    /// Find a HID++ device by any of its PIDs (wpid, BLE, BT, USB).
    pub fn hidpp_device_by_pid(&self, vid: u16, pid: u16) -> Option<&DeviceDef> {
        self.devices.iter().find(|d| {
            d.protocol == Protocol::Hidpp20
                && d.vendor_id == vid
                && (d.wpid.contains(&pid) || d.ble_pid.contains(&pid) || d.bt_pid.contains(&pid) || d.usb_pid.contains(&pid))
        })
    }

    pub fn usb_device(&self, vid: u16, pid: u16) -> Option<&DeviceDef> {
        self.devices.iter().find(|d| d.vendor_id == vid && d.usb_pid.contains(&pid))
    }

    pub fn is_hidpp_usage_page(&self, page: u16) -> bool {
        self.hidpp.usage_pages.contains(&page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_database_loads() {
        let db = DeviceDb::embedded().unwrap();
        assert!(db.receiver(0x046d, 0xc547).is_some());
        assert!(db.is_hidpp_usage_page(0xff00));
        assert!(db.is_hidpp_usage_page(0xff43));
        assert!(db.curve(DEFAULT_CURVE_ID).is_some());
    }

    #[test]
    fn curve_interpolates_and_clamps() {
        let c = CurveDef {
            id: "t".into(),
            precision: Precision::Approximate,
            chemistry: None,
            per_cell: false,
            points: vec![[3500, 0], [4200, 100], [3850, 50]],
            source: None,
        };
        assert_eq!(c.percent(4300), 100);
        assert_eq!(c.percent(3000), 0);
        assert_eq!(c.percent(3850), 50);
        assert_eq!(c.percent(4025), 75);
    }

    #[test]
    fn default_curve_is_approximate_and_sane() {
        let db = DeviceDb::embedded().unwrap();
        let (c, p) = db.curve_for(None).unwrap();
        assert_eq!(p, Precision::Approximate);
        assert_eq!(c.percent(4200), 100);
        assert!(c.percent(4158) >= 95);
        assert!(c.percent(3700) < 30);
    }

    #[test]
    fn user_override_adds_receiver_without_code() {
        let extra = r#"
            [[receiver]]
            vendor_id = 0x046d
            product_id = 0xc5ff
            name = "Future receiver"
            family = "bolt"
        "#;
        let files = EMBEDDED_DEVICES
            .iter()
            .map(|(n, c)| (n.to_string(), c.to_string()))
            .chain([("user.toml".to_string(), extra.to_string())]);
        let db = DeviceDb::from_files(files).unwrap();
        assert_eq!(db.receiver(0x046d, 0xc5ff).unwrap().family, ReceiverFamily::Bolt);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let bad = "[[receiver]]\nvendor_id = 1\nproduct_id = 2\nname = \"x\"\nfamily = \"bolt\"\ntypo = 1\n";
        let files = EMBEDDED_DEVICES
            .iter()
            .map(|(n, c)| (n.to_string(), c.to_string()))
            .chain([("bad.toml".to_string(), bad.to_string())]);
        assert!(DeviceDb::from_files(files).is_err());
    }

    #[test]
    fn g915_battery_service_is_flagged_unreliable() {
        let db = DeviceDb::embedded().unwrap();
        let g915 = db.hidpp_device_by_pid(0x046D, 0xB354).unwrap();
        assert!(g915.ble_battery_service_unreliable);
        assert_eq!(db.hidpp_device_by_pid(0x046D, 0x407C).unwrap().name, g915.name);
    }
}
