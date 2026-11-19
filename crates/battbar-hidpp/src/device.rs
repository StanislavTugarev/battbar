// SPDX-License-Identifier: GPL-3.0-or-later
//! HID++ 2.0 device probing and battery decoding.
//! Battery decoding follows the public feature descriptions and Solaar
//! (GPL-2.0-or-later).

use crate::protocol::{Channel, HidppError};
use battbar_core::devicedb::CurveDef;
use battbar_core::transport::FrameTransport;
use battbar_core::{BatteryInfo, Chemistry, DeviceKind, Precision, Reading, Status};

pub const F_DEVICE_INFO: u16 = 0x0003;
pub const F_DEVICE_NAME: u16 = 0x0005;
pub const F_BATTERY_STATUS: u16 = 0x1000;
pub const F_BATTERY_VOLTAGE: u16 = 0x1001;
pub const F_UNIFIED_BATTERY: u16 = 0x1004;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryFeature {
    /// 0x1004; `soc` = state of charge in % supported.
    /// `rechargeable`: byte1 bit0 of getCapabilities — a hypothesis from two samples
    /// (docs/DISCOVERY.md), used only to word alerts ("charge" / "replace batteries").
    Unified { index: u8, soc: bool, rechargeable: Option<bool> },
    /// 0x1001; percent from a discharge curve.
    Voltage { index: u8 },
    /// 0x1000.
    Status { index: u8 },
}

impl BatteryFeature {
    pub fn label(&self) -> &'static str {
        match self {
            BatteryFeature::Unified { .. } => "HID++ 0x1004",
            BatteryFeature::Voltage { .. } => "HID++ 0x1001",
            BatteryFeature::Status { .. } => "HID++ 0x1000",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub protocol: (u8, u8),
    pub name: Option<String>,
    pub kind: Option<DeviceKind>,
    pub unit_id: Option<[u8; 4]>,
    /// PIDs per transport from 0x0003 (BT, BLE, eQuad, USB order of set bits).
    pub model_ids: Vec<u16>,
    pub battery: Option<BatteryFeature>,
}

pub fn hex4(b: &[u8; 4]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Read identity, name and the preferred battery feature (0x1004 → 0x1001 → 0x1000).
pub fn probe<T: FrameTransport>(ch: &mut Channel<T>, idx: u8, protocol: (u8, u8)) -> Result<DeviceInfo, HidppError> {
    let mut info = DeviceInfo {
        protocol,
        name: None,
        kind: None,
        unit_id: None,
        model_ids: vec![],
        battery: None,
    };

    if let Some((fi, _)) = ch.feature(idx, F_DEVICE_INFO)? {
        let r = ch.call(idx, fi, 0, &[])?;
        if r.len() >= 13 {
            let unit = [r[1], r[2], r[3], r[4]];
            if unit != [0; 4] {
                info.unit_id = Some(unit);
            }
            info.model_ids = (0..3)
                .map(|i| u16::from_be_bytes([r[7 + 2 * i], r[8 + 2 * i]]))
                .filter(|p| *p != 0)
                .collect();
        }
    }

    if let Some((fi, _)) = ch.feature(idx, F_DEVICE_NAME)? {
        let len = ch.call(idx, fi, 0, &[])?[0] as usize;
        let mut name: Vec<u8> = vec![];
        while name.len() < len {
            let chunk = ch.call(idx, fi, 1, &[name.len() as u8])?;
            let part: Vec<u8> = chunk.into_iter().take_while(|b| *b != 0).collect();
            if part.is_empty() {
                break;
            }
            name.extend(part);
        }
        name.truncate(len);
        let s = String::from_utf8_lossy(&name).trim().to_string();
        if !s.is_empty() {
            info.name = Some(s);
        }
        if let Ok(t) = ch.call(idx, fi, 2, &[]) {
            info.kind = Some(DeviceKind::from_hidpp_type(t[0]));
        }
    }

    info.battery = if let Some((fi, _)) = ch.feature(idx, F_UNIFIED_BATTERY)? {
        let caps = ch.call(idx, fi, 0, &[])?;
        Some(BatteryFeature::Unified {
            index: fi,
            soc: caps.get(1).is_some_and(|f| f & 0x02 != 0),
            rechargeable: caps.get(1).map(|f| f & 0x01 != 0),
        })
    } else if let Some((fi, _)) = ch.feature(idx, F_BATTERY_VOLTAGE)? {
        Some(BatteryFeature::Voltage { index: fi })
    } else if let Some((fi, _)) = ch.feature(idx, F_BATTERY_STATUS)? {
        Some(BatteryFeature::Status { index: fi })
    } else {
        None
    };
    Ok(info)
}

pub fn read_battery<T: FrameTransport>(
    ch: &mut Channel<T>,
    idx: u8,
    feature: BatteryFeature,
    curve: Option<(&CurveDef, Precision)>,
    chemistry: Option<Chemistry>,
    cells: Option<u8>,
) -> Result<Reading, HidppError> {
    Ok(match feature {
        BatteryFeature::Unified { index, soc, rechargeable } => {
            let mut r = decode_unified(soc, &ch.call(idx, index, 1, &[])?);
            r.battery = Some(BatteryInfo {
                rechargeable,
                ..Default::default()
            });
            r
        }
        BatteryFeature::Voltage { index } => decode_voltage(&ch.call(idx, index, 0, &[])?, curve, chemistry, cells),
        BatteryFeature::Status { index } => decode_status(&ch.call(idx, index, 0, &[])?),
    })
}

/// 0x1004 getStatus: `[soc %, level bits, charging status, external power]`.
pub fn decode_unified(soc_supported: bool, r: &[u8]) -> Reading {
    let (soc, level, cs, ext) = (r[0], r[1], r[2], r.get(3).copied().unwrap_or(0));
    let mut reading = if soc_supported {
        Reading::ok(soc, Precision::Exact)
    } else {
        // critical / low / good / full
        let p = if level & 0x08 != 0 {
            90
        } else if level & 0x04 != 0 {
            50
        } else if level & 0x02 != 0 {
            20
        } else {
            5
        };
        Reading::ok(p, Precision::Coarse)
    };
    reading.charging = Some(matches!(cs, 1 | 2));
    reading.external_power = Some(ext != 0 || matches!(cs, 1..=3));
    if cs == 3 && reading.percent.is_some_and(|p| p < 100) && !soc_supported {
        reading.percent = Some(100);
    }
    if cs == 4 {
        reading.status = Status::Error;
        reading.percent = None;
    }
    reading
}

/// 0x1001 getBatteryInfo: `[mV hi, mV lo, flags]`. flags bit 7 = external power;
/// bits 0–2 = charge status (0 charging, 1 full, 2 not charging, 7 error).
/// Below this a 0x1001 voltage cannot come from a Li-ion cell (empty ≈ 3.4–3.5 V):
/// AA/AAA cells (1.2–1.6 V per cell). Non-rechargeable batteries are not supported yet.
pub const NON_LI_ION_BELOW_MV: u16 = 3300;

/// `chemistry` / `cells` come from the device database. Replaceable cells (database
/// says alkaline/NiMH, or the voltage is below any Li-ion level) are never mapped
/// through the Li-ion curve: the reading carries the voltage, and the percentage is
/// estimated after merging from the chosen chemistry (`battbar_core::cells`).
pub fn decode_voltage(r: &[u8], curve: Option<(&CurveDef, Precision)>, chemistry: Option<Chemistry>, cells: Option<u8>) -> Reading {
    let mv = u16::from_be_bytes([r[0], r[1]]);
    let replaceable = matches!(chemistry, Some(Chemistry::Alkaline | Chemistry::Nimh));
    if mv > 0 && (replaceable || mv < NON_LI_ION_BELOW_MV) {
        let mut reading = Reading::with_status(Status::NotReported);
        reading.voltage_mv = Some(mv);
        reading.note = Some(battbar_core::ReadingNote::NonRechargeableBattery);
        reading.battery = Some(BatteryInfo {
            chemistry: chemistry.filter(|_| replaceable),
            cells,
            rechargeable: Some(false),
        });
        return reading;
    }
    let flags = r[2];
    let ext = flags & 0x80 != 0;
    let cs = flags & 0x07;
    let (percent, precision) = match curve {
        Some((c, p)) => (c.percent(mv), p),
        None => (0, Precision::Approximate),
    };
    let mut reading = Reading::ok(percent, precision);
    reading.voltage_mv = Some(mv);
    reading.battery = Some(BatteryInfo {
        chemistry: Some(Chemistry::LiIon),
        cells: None,
        rechargeable: Some(true),
    });
    reading.external_power = Some(ext);
    reading.charging = Some(ext && cs == 0);
    if ext && cs == 1 {
        reading.percent = Some(100);
    }
    if ext && cs == 7 {
        reading.status = Status::Error;
        reading.percent = None;
    }
    if curve.is_none() {
        reading.percent = None;
        reading.status = Status::NotReported;
    }
    reading
}

/// 0x1000 getBatteryLevelStatus: `[level %, next level %, status]`.
pub fn decode_status(r: &[u8]) -> Reading {
    let (level, status) = (r[0], r[2]);
    let mut reading = Reading::ok(level, Precision::Coarse);
    reading.charging = Some(matches!(status, 1 | 2 | 4));
    reading.external_power = Some(matches!(status, 1..=4));
    if status == 3 {
        reading.percent = Some(100);
    }
    if matches!(status, 5 | 6) {
        reading.status = Status::Error;
        reading.percent = None;
    }
    reading
}

/// Full feature table via IFeatureSet (diagnostics only): `(feature id, version)`.
pub fn feature_list<T: FrameTransport>(ch: &mut Channel<T>, idx: u8) -> Result<Vec<(u16, u8)>, HidppError> {
    let Some((fs, _)) = ch.feature(idx, 0x0001)? else {
        return Ok(vec![]);
    };
    let count = ch.call(idx, fs, 0, &[])?[0];
    let mut out = vec![];
    for i in 0..=count {
        let r = ch.call(idx, fs, 1, &[i])?;
        out.push((u16::from_be_bytes([r[0], r[1]]), r[3]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve() -> CurveDef {
        battbar_core::DeviceDb::embedded().unwrap().curve("li-ion-default").unwrap().clone()
    }

    #[test]
    fn unified_battery_mouse_reference_reading() {
        // G502 X PLUS via Lightspeed, Stage 0: 11 %, critical, discharging.
        let r = decode_unified(true, &[0x0b, 0x01, 0x00, 0x00, 0, 0]);
        assert_eq!(r.percent, Some(11));
        assert_eq!(r.precision, Precision::Exact);
        assert_eq!(r.charging, Some(false));
        assert_eq!(r.external_power, Some(false));
    }

    #[test]
    fn unified_battery_levels_only_is_coarse() {
        let r = decode_unified(false, &[0, 0x04, 0x01, 0x01]);
        assert_eq!((r.percent, r.precision, r.charging), (Some(50), Precision::Coarse, Some(true)));
    }

    #[test]
    fn voltage_keyboard_reference_readings() {
        let c = curve();
        // G915 on BLE (GATT), Stage 0: 4158 mV, on battery. G HUB: 100 %.
        let r = decode_voltage(&[0x10, 0x3e, 0x00], Some((&c, Precision::Approximate)), None, None);
        assert!(r.percent.unwrap() >= 95, "{r:?}");
        assert_eq!(r.precision, Precision::Approximate);
        assert_eq!(r.charging, Some(false));
        assert_eq!(r.voltage_mv, Some(4158));
        // G915 via Lightspeed with cable, Stage 0: 4200 mV, flags 0x81 (ext power, full).
        let r = decode_voltage(&[0x10, 0x68, 0x81], Some((&c, Precision::Approximate)), None, None);
        assert_eq!(r.percent, Some(100));
        assert_eq!(r.charging, Some(false));
        assert_eq!(r.external_power, Some(true));
        // Charging.
        let r = decode_voltage(&[0x0f, 0x00, 0x80], Some((&c, Precision::Approximate)), None, None);
        assert_eq!(r.charging, Some(true));
    }

    #[test]
    fn battery_error_is_never_zero_percent() {
        assert_eq!(decode_unified(true, &[0, 0, 4, 0]).percent, None);
        assert_eq!(decode_status(&[0, 0, 6]).status, Status::Error);
        assert_eq!(
            decode_voltage(&[0x0f, 0, 0x87], Some((&curve(), Precision::Approximate)), None, None).percent,
            None
        );
    }
}
