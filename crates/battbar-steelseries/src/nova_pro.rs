// SPDX-License-Identifier: GPL-3.0-or-later
//! Arctis Nova Pro Wireless base station status report.
//!
//! Request and the meaning of bytes 6 and 15 are ported from HeadsetControl
//! (GPL-3.0, `steelseries_arctis_nova_pro_wireless.c`). Bytes 4, 5 and 7 were
//! identified in battbar's own discovery (docs/DISCOVERY.md §3).

use battbar_core::{Component, ComponentKind, Precision, Reading, Status};

pub const STATUS_REQUEST: [u8; 2] = [0x06, 0xb0];
const BATTERY_MAX: u8 = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct NovaProStatus {
    pub headset: Reading,
    /// Spare battery in the base slot (byte 7, 0–8).
    pub spare: Option<Reading>,
    /// The base reports that its headset is also connected over Bluetooth (byte 5).
    pub headset_on_bluetooth: bool,
}

pub fn is_status_reply(r: &[u8]) -> bool {
    r.len() >= 16 && r[0] == STATUS_REQUEST[0] && r[1] == STATUS_REQUEST[1]
}

fn level_to_percent(level: u8) -> u8 {
    ((level.min(BATTERY_MAX) as u32 * 100 + BATTERY_MAX as u32 / 2) / BATTERY_MAX as u32) as u8
}

pub fn decode(r: &[u8]) -> NovaProStatus {
    let state = r[15];
    let headset = match state {
        // 0x01: headset off / out of range
        0x01 => Reading::with_status(Status::Disconnected),
        0x02 | 0x08 => {
            let mut rd = Reading::ok(level_to_percent(r[6]), Precision::Coarse);
            rd.charging = Some(state == 0x02);
            rd.external_power = Some(state == 0x02);
            rd
        }
        _ => Reading::with_status(Status::Error),
    };
    let mut headset = headset;
    headset.battery = Some(battbar_core::BatteryInfo {
        rechargeable: Some(true),
        ..Default::default()
    });
    let spare = (r[7] <= BATTERY_MAX).then(|| Reading::ok(level_to_percent(r[7]), Precision::Coarse));
    NovaProStatus {
        headset,
        spare,
        headset_on_bluetooth: r[5] == 0x01,
    }
}

pub fn spare_component(s: &NovaProStatus) -> Vec<Component> {
    s.spare
        .clone()
        .map(|reading| {
            vec![Component {
                kind: ComponentKind::Spare,
                reading,
            }]
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(s: &str) -> Vec<u8> {
        let mut v = battbar_core::transport::from_hex(s).unwrap();
        v.resize(64, 0);
        v
    }

    #[test]
    fn reference_replies_from_discovery() {
        // Headset online, full; spare 4/8; headset not on Bluetooth.
        let s = decode(&frame("06 b0 00 00 01 00 08 04 08 00 02 0a 00 00 08 08"));
        assert_eq!(s.headset.percent, Some(100));
        assert_eq!(s.headset.precision, Precision::Coarse);
        assert_eq!(s.headset.charging, Some(false));
        assert_eq!(s.spare.as_ref().unwrap().percent, Some(50));
        assert!(!s.headset_on_bluetooth);
        // Same headset, Bluetooth connected as well.
        let s = decode(&frame("06 b0 00 00 04 01 08 06 08 00 02 0a 00 00 08 08"));
        assert!(s.headset_on_bluetooth);
        assert_eq!(s.spare.unwrap().percent, Some(75));
    }

    #[test]
    fn offline_headset_is_disconnected_not_zero() {
        let s = decode(&frame("06 b0 00 00 01 00 00 08 08 00 02 0a 00 00 08 01"));
        assert_eq!(s.headset.status, Status::Disconnected);
        assert_eq!(s.headset.percent, None);
    }

    #[test]
    fn cable_charging() {
        let s = decode(&frame("06 b0 00 00 01 00 03 08 08 00 02 0a 00 00 08 02"));
        assert_eq!(s.headset.percent, Some(38));
        assert_eq!(s.headset.charging, Some(true));
    }
}
