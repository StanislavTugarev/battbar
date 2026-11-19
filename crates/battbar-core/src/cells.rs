// SPDX-License-Identifier: GPL-3.0-or-later
//! Replaceable AA/AAA cells on voltage-only devices (HID++ 0x1001).
//!
//! The device only reports the pack voltage. The percentage is estimated from a
//! per-cell discharge curve of the chosen chemistry (user setting per device, else the
//! device database, else alkaline — alkaline and NiMH cannot be told apart by voltage).
//! Values are always `approximate`; the support is experimental (no real voltage-only
//! battery device measured yet, see SPEC "Battery chemistry").

use crate::devicedb::DeviceDb;
use crate::merge::Device;
use crate::model::{BatteryInfo, Chemistry, Precision, ReadingNote, Status};

/// Cells in series, inferred from the pack voltage when the database does not say.
pub fn infer_cells(mv: u16) -> u8 {
    match mv {
        0..=1800 => 1,
        1801..=3600 => 2,
        _ => mv.div_ceil(1800).clamp(3, 8) as u8,
    }
}

/// Turn "replaceable cells, voltage only" readings into estimated percentages.
pub fn apply_chemistry(devices: &mut [Device], db: &DeviceDb, chosen: impl Fn(&str) -> Option<Chemistry>) {
    for d in devices.iter_mut() {
        if d.reading.note != Some(ReadingNote::NonRechargeableBattery) {
            continue;
        }
        let Some(mv) = d.reading.voltage_mv else { continue };
        let info = d.reading.battery.unwrap_or_default();
        let chem = chosen(&d.id)
            .or(info.chemistry)
            .filter(|c| *c != Chemistry::LiIon)
            .unwrap_or(Chemistry::Alkaline);
        let cells = info.cells.unwrap_or_else(|| infer_cells(mv)).max(1);
        let Some(curve) = db.chemistry_curve(chem) else { continue };
        let per_cell = mv / cells as u16;
        d.reading.percent = Some(curve.percent(per_cell));
        d.reading.status = Status::Ok;
        d.reading.precision = Precision::Approximate;
        d.reading.note = None;
        d.reading.charging = Some(false);
        d.reading.battery = Some(BatteryInfo {
            chemistry: Some(chem),
            cells: Some(cells),
            rechargeable: Some(false),
        });
    }
}

/// Whether a device runs on replaceable cells whose chemistry the user can choose.
pub fn has_selectable_chemistry(d: &Device) -> bool {
    d.reading.voltage_mv.is_some()
        && d.reading
            .battery
            .is_some_and(|b| matches!(b.chemistry, Some(Chemistry::Alkaline | Chemistry::Nimh)) && b.rechargeable == Some(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn aa(mv: u16, info: Option<BatteryInfo>) -> Device {
        let mut r = Reading::with_status(Status::NotReported);
        r.voltage_mv = Some(mv);
        r.note = Some(ReadingNote::NonRechargeableBattery);
        r.battery = info;
        Device {
            id: "aa".into(),
            name: "AA".into(),
            kind: DeviceKind::Mouse,
            reading: r,
            percent_source: None,
            components: vec![],
            sources: vec![],
            estimate: None,
        }
    }

    #[test]
    fn cells_are_inferred_from_pack_voltage() {
        assert_eq!(infer_cells(1450), 1);
        assert_eq!(infer_cells(2900), 2);
        assert_eq!(infer_cells(4500), 3);
    }

    #[test]
    fn default_is_alkaline_and_always_approximate() {
        let db = DeviceDb::embedded().unwrap();
        let mut v = vec![aa(2900, None)]; // 2 cells × 1.45 V
        apply_chemistry(&mut v, &db, |_| None);
        let r = &v[0].reading;
        assert_eq!(r.status, Status::Ok);
        assert_eq!(r.precision, Precision::Approximate);
        assert_eq!(r.percent, Some(85));
        assert_eq!(r.battery.unwrap().chemistry, Some(Chemistry::Alkaline));
        assert_eq!(r.battery.unwrap().rechargeable, Some(false));
        assert!(has_selectable_chemistry(&v[0]));
    }

    #[test]
    fn user_choice_overrides_database_and_li_ion_is_never_used() {
        let db = DeviceDb::embedded().unwrap();
        let mut v = vec![aa(
            2500,
            Some(BatteryInfo {
                chemistry: Some(Chemistry::Alkaline),
                cells: Some(2),
                rechargeable: Some(false),
            }),
        )];
        apply_chemistry(&mut v, &db, |_| Some(Chemistry::Nimh));
        assert_eq!(v[0].reading.battery.unwrap().chemistry, Some(Chemistry::Nimh));
        assert_eq!(v[0].reading.percent, Some(65)); // 1.25 V per NiMH cell
        let mut v = vec![aa(
            2500,
            Some(BatteryInfo {
                chemistry: Some(Chemistry::LiIon),
                ..Default::default()
            }),
        )];
        apply_chemistry(&mut v, &db, |_| None);
        assert_eq!(v[0].reading.battery.unwrap().chemistry, Some(Chemistry::Alkaline));
    }

    #[test]
    fn exhausted_cells_read_zero_only_from_the_cell_curve() {
        let db = DeviceDb::embedded().unwrap();
        let mut v = vec![aa(
            1700,
            Some(BatteryInfo {
                cells: Some(2),
                ..Default::default()
            }),
        )]; // 0.85 V/cell
        apply_chemistry(&mut v, &db, |_| None);
        assert_eq!(v[0].reading.percent, Some(0));
    }
}
