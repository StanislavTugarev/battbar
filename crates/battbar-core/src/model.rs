// SPDX-License-Identifier: GPL-3.0-or-later
//! Data model shared by all providers and front-ends.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Headphones,
    Gamepad,
    Other,
}

impl DeviceKind {
    pub fn from_hidpp_type(t: u8) -> Self {
        match t {
            0 | 2 => DeviceKind::Keyboard,
            3..=5 => DeviceKind::Mouse,
            8 => DeviceKind::Headset,
            10..=12 => DeviceKind::Gamepad,
            _ => DeviceKind::Other,
        }
    }
}

/// How the data reached this Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// 2.4 GHz receiver (Lightspeed / Unifying / Bolt).
    Receiver,
    /// Proprietary base station / dongle (SteelSeries).
    BaseStation,
    /// Direct USB cable.
    Usb,
    BluetoothLe,
    BluetoothClassic,
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Transport::Receiver => "receiver",
            Transport::BaseStation => "base station",
            Transport::Usb => "USB",
            Transport::BluetoothLe => "Bluetooth LE",
            Transport::BluetoothClassic => "Bluetooth",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Asleep,
    Disconnected,
    /// Connected, but the source has no battery value for it.
    NotReported,
    Error,
}

impl Status {
    /// Higher is better; used when merging sources.
    pub fn rank(self) -> u8 {
        match self {
            Status::Ok => 4,
            Status::NotReported => 3,
            Status::Asleep => 2,
            Status::Disconnected => 1,
            Status::Error => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    /// Percentage reported by the device in ~1 % steps.
    Exact,
    /// Percentage in large steps (e.g. 9 levels, 10 %).
    Coarse,
    /// Estimated, e.g. voltage converted through a discharge curve.
    Approximate,
}

impl Precision {
    /// Higher is better.
    pub fn rank(self) -> u8 {
        match self {
            Precision::Exact => 3,
            Precision::Coarse => 2,
            Precision::Approximate => 1,
        }
    }
}

/// Battery chemistry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Chemistry {
    LiIon,
    Alkaline,
    Nimh,
}

/// What is known about the battery itself (not its level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BatteryInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chemistry: Option<Chemistry>,
    /// Number of cells in series (AA/AAA devices).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<u8>,
    /// Charged in the device (`true`) or replaced (`false`); `None` = unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rechargeable: Option<bool>,
}

/// Why a connected device has no battery value, or how its value was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingNote {
    /// Voltage of replaceable AA/AAA cells; the percentage is computed from the
    /// chemistry curve after merging (`cells::apply_chemistry`).
    NonRechargeableBattery,
    /// HID++ 1.0 device: battery registers not supported yet.
    Hidpp10Device,
    /// Percentage from the voltage through this unit's own curve, learned while it also
    /// reported its firmware percentage on Bluetooth (`calibration::apply`).
    CalibratedVoltage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub percent: Option<u8>,
    pub charging: Option<bool>,
    pub external_power: Option<bool>,
    pub status: Status,
    pub precision: Precision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage_mv: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<ReadingNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<BatteryInfo>,
}

impl Reading {
    pub fn ok(percent: u8, precision: Precision) -> Self {
        Reading {
            percent: Some(percent.min(100)),
            charging: None,
            external_power: None,
            status: Status::Ok,
            precision,
            voltage_mv: None,
            note: None,
            battery: None,
        }
    }

    pub fn with_status(status: Status) -> Self {
        Reading {
            percent: None,
            charging: None,
            external_power: None,
            status,
            precision: Precision::Exact,
            voltage_mv: None,
            note: None,
            battery: None,
        }
    }

    pub fn has_value(&self) -> bool {
        self.status == Status::Ok && self.percent.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Left,
    Right,
    Case,
    /// Spare battery charging in a base station.
    Spare,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub kind: ComponentKind,
    pub reading: Reading,
}

/// Identity keys used to recognise the same physical device across providers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdentityKey {
    /// HID++ 0x0003 unit ID (hex). Identical on every transport, and also stored in the
    /// receiver's extended pairing register, so it is known even while the device is away.
    HidppUnit { unit_id: String },
    /// Provider-specific stable identifier (e.g. Bluetooth address hash, USB serial).
    Stable { namespace: String, value: String },
}

/// Hints that allow a weaker, conditional merge (decided by the core, not the provider).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeHint {
    /// A base station states that its headset is currently connected over Bluetooth;
    /// a Bluetooth device whose name matches one of the patterns is the same headset.
    BluetoothCompanion { name_patterns: Vec<String> },
}

/// What one provider knows about one device at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceReport {
    /// Provider name, e.g. `hidpp`, `steelseries`, `bt-system`.
    pub provider: String,
    /// Unique and stable within the provider, e.g. `receiver:046d:c547:2` or `ble:<uuid>`.
    pub source_key: String,
    /// Human readable data source, e.g. `HID++ 0x1004`, `BLE Battery Service`.
    pub source: String,
    pub name: String,
    pub kind: DeviceKind,
    pub transport: Transport,
    pub identity: Vec<IdentityKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<MergeHint>,
    pub reading: Reading,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
}

impl DeviceReport {
    pub fn full_key(&self) -> String {
        format!("{}/{}", self.provider, self.source_key)
    }
}

/// Source priority on precision ties (lower = preferred).
pub fn provider_priority(provider: &str) -> u8 {
    match provider {
        "hidpp" | "steelseries" => 0,
        "headsetcontrol" => 1,
        "bt-system" => 2,
        _ => 3,
    }
}

/// Short stable hash for identifiers that should not be shown verbatim (Bluetooth
/// addresses, CoreBluetooth UUIDs). FNV-1a 64, 12 hex digits.
pub fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:012x}", h & 0xFFFF_FFFF_FFFF)
}
