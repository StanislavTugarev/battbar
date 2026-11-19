// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-unit voltage calibration for HID++ 0x1001 (voltage-only) devices.
//!
//! The same keyboard can report two numbers: on Bluetooth LE its firmware sends a
//! percentage (BLE Battery Service, `exact`), while on the receiver only the cell
//! voltage is available, which battbar converts with a generic Li-ion table
//! (`approximate`). The two scales differ, so the value jumped when the device moved
//! between transports (e.g. 100 % on Bluetooth, 91 % on the receiver).
//!
//! While both are live at the same time, battbar records which voltages the firmware
//! maps to which percentage, keeping the most recent samples per percentage. On a
//! transport with voltage only, that unit's own points replace the generic table; the
//! generic table is used only outside the learned range, scaled to join it smoothly.
//! Values stay `approximate`. Nothing is learned while charging or while the voltage
//! settles afterwards (the charge current raises the voltage).
//!
//! Stored in the settings by device ID (already a keyed hash); never part of
//! `battbar dump` or traces.

use crate::devicedb::{CurveDef, DeviceDb, DEFAULT_CURVE_ID};
use crate::merge::Device;
use crate::model::{DeviceReport, Precision, ReadingNote, Status};
use serde::{Deserialize, Serialize};

/// Recent voltages kept per firmware percentage. Older samples drop out, so a level
/// follows the battery as it ages and, on a plateau (firmware says 100 % for a long
/// time), drifts towards the voltage at which the firmware leaves that level.
pub const SAMPLES_PER_LEVEL: usize = 9;
/// Samples a level needs before it is used.
pub const MIN_SAMPLES: usize = 3;
/// A new sample within this distance of the previous one at the same level is a repeat.
const REPEAT_MV: u16 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level {
    /// Percentage reported by the device's own firmware.
    pub percent: u8,
    /// Voltages (mV) seen at that percentage, oldest first.
    pub mv: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoltageCalibration {
    /// Device ID (`hidpp:<keyed hash>`).
    pub device: String,
    pub levels: Vec<Level>,
}

impl VoltageCalibration {
    pub fn new(device: &str) -> Self {
        VoltageCalibration {
            device: device.into(),
            levels: vec![],
        }
    }

    /// Record one (firmware percentage, voltage) pair. Returns whether anything changed.
    pub fn add(&mut self, percent: u8, mv: u16) -> bool {
        let percent = percent.min(100);
        let level = match self.levels.iter().position(|l| l.percent == percent) {
            Some(i) => &mut self.levels[i],
            None => {
                self.levels.push(Level { percent, mv: vec![] });
                self.levels.sort_by_key(|l| l.percent);
                let i = self.levels.iter().position(|l| l.percent == percent).unwrap();
                &mut self.levels[i]
            }
        };
        if level.mv.last().is_some_and(|last| last.abs_diff(mv) < REPEAT_MV) {
            return false;
        }
        level.mv.push(mv);
        if level.mv.len() > SAMPLES_PER_LEVEL {
            level.mv.remove(0);
        }
        true
    }

    /// Learned curve points `(mV, percent)`, ascending in both. A level is used once it
    /// has [`MIN_SAMPLES`]; its voltage is the median. Levels that would make the curve
    /// non-monotonic (noise between neighbouring percentages) are skipped.
    pub fn points(&self) -> Vec<(u16, u8)> {
        let mut out: Vec<(u16, u8)> = vec![];
        for l in &self.levels {
            if l.mv.len() < MIN_SAMPLES {
                continue;
            }
            let mut v = l.mv.clone();
            v.sort_unstable();
            let median = v[v.len() / 2];
            if out.last().is_none_or(|&(mv, _)| median > mv) {
                out.push((median, l.percent));
            }
        }
        out
    }

    /// Percentage for `mv` from the learned points; `fallback` (the generic curve)
    /// extends the curve beyond them. `None` until at least one level is usable.
    pub fn percent(&self, mv: u16, fallback: &CurveDef) -> Option<u8> {
        let pts = self.points();
        let (&(v_lo, p_lo), &(v_hi, p_hi)) = (pts.first()?, pts.last()?);
        let f = |v: u16| fallback.percent(v) as f64;
        let p = if mv >= v_hi {
            // Above the learned range: continue along the generic curve, reaching 100 %
            // where it does.
            let f_hi = f(v_hi);
            if p_hi >= 100 || f_hi >= 100.0 {
                p_hi as f64
            } else {
                p_hi as f64 + (100.0 - p_hi as f64) * (f(mv) - f_hi) / (100.0 - f_hi)
            }
        } else if mv <= v_lo {
            // Below: generic curve scaled so it joins the lowest learned point.
            let f_lo = f(v_lo);
            if f_lo <= 0.0 {
                p_lo as f64
            } else {
                p_lo as f64 * f(mv) / f_lo
            }
        } else {
            let w = pts.windows(2).find(|w| mv <= w[1].0)?;
            let ((v0, p0), (v1, p1)) = (w[0], w[1]);
            p0 as f64 + (p1 as f64 - p0 as f64) * (mv - v0) as f64 / (v1 - v0) as f64
        };
        Some(p.round().clamp(0.0, 100.0) as u8)
    }

    /// Number of firmware percentages that currently shape the curve (for the menu).
    pub fn usable_levels(&self) -> usize {
        self.points().len()
    }
}

/// Live Li-ion voltage report of a device (HID++ 0x1001), if any.
fn voltage_source(d: &Device) -> Option<&DeviceReport> {
    d.sources.iter().find(|s| {
        let r = &s.reading;
        s.provider == "hidpp"
            && r.status == Status::Ok
            && r.precision == Precision::Approximate
            && r.voltage_mv.is_some()
            && r.note.is_none()
            && !r.battery.is_some_and(|b| b.rechargeable == Some(false))
    })
}

/// A simultaneous (firmware percentage, voltage) pair, if the device offers one now.
pub fn sample(d: &Device) -> Option<(u8, u16)> {
    if d.estimate.is_some() || d.reading.charging == Some(true) || d.reading.external_power == Some(true) {
        return None;
    }
    let mv = voltage_source(d)?.reading.voltage_mv?;
    let exact = d
        .sources
        .iter()
        .find(|s| s.reading.precision == Precision::Exact && s.reading.has_value() && s.reading.voltage_mv.is_none())?;
    if exact.reading.charging == Some(true) {
        return None;
    }
    Some((exact.reading.percent?, mv))
}

/// Learn from every device that reports both values. Returns whether anything changed.
pub fn observe(cals: &mut Vec<VoltageCalibration>, devices: &[Device]) -> bool {
    let mut changed = false;
    for d in devices {
        let Some((percent, mv)) = sample(d) else { continue };
        let cal = match cals.iter().position(|c| c.device == d.id) {
            Some(i) => &mut cals[i],
            None => {
                cals.push(VoltageCalibration::new(&d.id));
                cals.last_mut().unwrap()
            }
        };
        changed |= cal.add(percent, mv);
    }
    changed
}

/// Replace voltage-table estimates with the unit's learned curve where one exists.
/// Only affects devices whose percentage currently comes from the voltage.
pub fn apply(cals: &[VoltageCalibration], devices: &mut [Device], db: &DeviceDb) {
    let Some(fallback) = db.curve(DEFAULT_CURVE_ID) else { return };
    for d in devices.iter_mut() {
        let Some(cal) = cals.iter().find(|c| c.device == d.id) else {
            continue;
        };
        let Some(src) = voltage_source(d) else { continue };
        if d.percent_source.as_deref() != Some(&src.full_key()) {
            continue; // an exact source is live and wins anyway
        }
        let Some(mv) = src.reading.voltage_mv else { continue };
        if let Some(p) = cal.percent(mv, fallback) {
            d.reading.percent = Some(p);
            d.reading.note = Some(ReadingNote::CalibratedVoltage);
        }
    }
}

pub fn forget(cals: &mut Vec<VoltageCalibration>, device: &str) -> bool {
    let n = cals.len();
    cals.retain(|c| c.device != device);
    cals.len() != n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn fallback() -> CurveDef {
        DeviceDb::embedded().unwrap().curve(DEFAULT_CURVE_ID).unwrap().clone()
    }

    fn report(provider: &str, key: &str, transport: Transport, r: Reading) -> DeviceReport {
        DeviceReport {
            provider: provider.into(),
            source_key: key.into(),
            source: provider.into(),
            name: "G915".into(),
            kind: DeviceKind::Keyboard,
            transport,
            identity: vec![IdentityKey::HidppUnit {
                unit_id: "bba70001".into(),
            }],
            hints: vec![],
            reading: r,
            components: vec![],
        }
    }

    fn volt(mv: u16) -> Reading {
        let mut r = Reading::ok(fallback().percent(mv), Precision::Approximate);
        r.voltage_mv = Some(mv);
        r.charging = Some(false);
        r.battery = Some(BatteryInfo {
            chemistry: Some(Chemistry::LiIon),
            cells: None,
            rechargeable: Some(true),
        });
        r
    }

    /// Keyboard on Bluetooth: HID++ over GATT (voltage) + BLE Battery Service (percent).
    fn on_bluetooth(mv: u16, firmware: u8) -> Device {
        let bas = report(
            "bt-system",
            "ble:x",
            Transport::BluetoothLe,
            Reading::ok(firmware, Precision::Exact),
        );
        let gatt = report("hidpp", "gatt:x", Transport::BluetoothLe, volt(mv));
        crate::merge(vec![gatt, bas], &Default::default()).remove(0)
    }

    fn on_receiver(mv: u16) -> Device {
        crate::merge(
            vec![report("hidpp", "hid:046d:c547:2:ff00:1", Transport::Receiver, volt(mv))],
            &Default::default(),
        )
        .remove(0)
    }

    #[test]
    fn learns_on_bluetooth_and_applies_on_the_receiver() {
        let db = DeviceDb::embedded().unwrap();
        let mut cals = vec![];
        for mv in [4170, 4150, 4130, 4110, 4100] {
            assert!(observe(&mut cals, &[on_bluetooth(mv, 100)]));
        }
        for mv in [4060, 4050, 4040] {
            observe(&mut cals, &[on_bluetooth(mv, 95)]);
        }
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].points(), vec![(4050, 95), (4130, 100)]);

        let mut v = vec![on_receiver(4080)];
        assert_eq!(v[0].reading.percent, Some(91)); // generic table
        apply(&cals, &mut v, &db);
        let r = &v[0].reading;
        assert_eq!(r.percent, Some(97)); // 95 + 5 × (4080 − 4050) / (4130 − 4050)
        assert_eq!(r.precision, Precision::Approximate);
        assert_eq!(r.note, Some(ReadingNote::CalibratedVoltage));
    }

    #[test]
    fn bluetooth_firmware_value_still_wins_while_live() {
        let db = DeviceDb::embedded().unwrap();
        let mut cals = vec![];
        for mv in [4170, 4150, 4130] {
            observe(&mut cals, &[on_bluetooth(mv, 100)]);
        }
        let mut v = vec![on_bluetooth(4080, 97)];
        apply(&cals, &mut v, &db);
        assert_eq!(v[0].reading.percent, Some(97));
        assert_eq!(v[0].reading.precision, Precision::Exact);
        assert_eq!(v[0].reading.note, None);
    }

    #[test]
    fn single_plateau_level_scales_the_generic_curve() {
        let f = fallback();
        let mut c = VoltageCalibration::new("d");
        for mv in [4148, 4120, 4100] {
            c.add(100, mv);
        }
        assert_eq!(c.points(), vec![(4120, 100)]);
        assert_eq!(c.percent(4186, &f), Some(100));
        assert_eq!(c.percent(4120, &f), Some(100));
        // Generic 4080 mV = 91 %, 4120 mV = 94 %: scaled to join 100 % at 4120 mV.
        assert_eq!(c.percent(4080, &f), Some(97));
        assert_eq!(c.percent(3500, &f), Some(0));
        let mut prev = 0;
        for mv in (3500..=4200).step_by(5) {
            let p = c.percent(mv, &f).unwrap();
            assert!(p >= prev, "monotonic at {mv}");
            prev = p;
        }
    }

    #[test]
    fn needs_enough_samples_and_ignores_repeats() {
        let f = fallback();
        let mut c = VoltageCalibration::new("d");
        assert!(c.add(100, 4150));
        assert!(!c.add(100, 4155)); // repeat within 10 mV
        assert!(c.add(100, 4140));
        assert_eq!(c.percent(4100, &f), None);
        assert!(c.add(100, 4130));
        assert!(c.percent(4100, &f).is_some());
        for mv in (0..20).map(|i| 4000 - i * 20) {
            c.add(80, mv);
        }
        assert_eq!(c.levels[0].mv.len(), SAMPLES_PER_LEVEL);
        assert_eq!(c.levels[0].mv[0], 4000 - 11 * 20, "oldest samples drop out");
    }

    #[test]
    fn noisy_non_monotonic_levels_are_skipped() {
        let mut c = VoltageCalibration::new("d");
        for mv in [4000, 4002 + 10, 4030] {
            c.add(90, mv);
        }
        for mv in [3990, 3970, 3950] {
            c.add(95, mv); // lower voltage than the 90 % level: noise
        }
        for mv in [4150, 4130, 4110] {
            c.add(100, mv);
        }
        assert_eq!(c.points(), vec![(4012, 90), (4130, 100)]);
    }

    #[test]
    fn nothing_is_learned_while_charging_or_settling() {
        let mut cals = vec![];
        let mut d = on_bluetooth(4190, 100);
        d.reading.charging = Some(true);
        assert!(!observe(&mut cals, &[d]));
        let mut d = on_bluetooth(4190, 100);
        d.estimate = Some(crate::charge::Estimate::Settling);
        assert!(!observe(&mut cals, &[d]));
        // Receiver only: no firmware percentage to learn from.
        assert!(!observe(&mut cals, &[on_receiver(4100)]));
        assert!(cals.is_empty());
    }

    #[test]
    fn settings_roundtrip_and_forget() {
        let mut s = crate::Settings::default();
        let mut c = VoltageCalibration::new("hidpp:abc");
        c.add(100, 4150);
        s.voltage_calibration.push(c);
        let t = toml::to_string_pretty(&s).unwrap();
        assert_eq!(toml::from_str::<crate::Settings>(&t).unwrap(), s);
        assert!(forget(&mut s.voltage_calibration, "hidpp:abc"));
        assert!(s.voltage_calibration.is_empty());
    }
}
