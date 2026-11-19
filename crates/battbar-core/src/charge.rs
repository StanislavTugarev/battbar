// SPDX-License-Identifier: GPL-3.0-or-later
//! Battery values while charging and shortly after.
//!
//! Firmware estimates the state of charge from the cell voltage. While charging, the
//! terminal voltage is raised by the charge current (V = OCV + I·R), which on the flat
//! part of a Li-ion curve reads as tens of percent more than the real charge (observed:
//! 8 % → 52 % within 15 s of plugging in, in the vendor app as well). After unplugging,
//! the voltage relaxes for several minutes. Values in both phases are therefore shown
//! as estimates, whatever their source.

use crate::merge::Device;
use crate::model::Precision;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Why a value is shown as an estimate although its source may claim more precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Estimate {
    /// The device is charging.
    Charging,
    /// Charging stopped recently; the voltage is still settling.
    Settling,
}

/// Default voltage relaxation period after charging stops.
pub const SETTLING: Duration = Duration::from_secs(600);

/// Stateless part: a charging device's percentage is approximate.
pub fn mark_charging(d: &mut Device) {
    if d.reading.charging == Some(true) && d.reading.percent.is_some() {
        d.reading.precision = Precision::Approximate;
        d.estimate = Some(Estimate::Charging);
    }
    for c in &mut d.components {
        if c.reading.charging == Some(true) && c.reading.percent.is_some() {
            c.reading.precision = Precision::Approximate;
        }
    }
}

/// Stateful part: keep values approximate for `window` after charging stops.
#[derive(Debug)]
pub struct SettlingTracker {
    last_charging: HashMap<String, Instant>,
    pub window: Duration,
}

impl Default for SettlingTracker {
    fn default() -> Self {
        SettlingTracker {
            last_charging: HashMap::new(),
            window: SETTLING,
        }
    }
}

impl SettlingTracker {
    pub fn apply(&mut self, now: Instant, devices: &mut [Device]) {
        for d in devices.iter_mut() {
            if d.reading.charging == Some(true) {
                self.last_charging.insert(d.id.clone(), now);
                continue;
            }
            let Some(t) = self.last_charging.get(&d.id).copied() else {
                continue;
            };
            if now.saturating_duration_since(t) >= self.window {
                self.last_charging.remove(&d.id);
            } else if d.reading.percent.is_some() {
                d.reading.precision = Precision::Approximate;
                d.estimate = Some(Estimate::Settling);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn dev(p: u8, charging: bool) -> Device {
        let mut r = Reading::ok(p, Precision::Exact);
        r.charging = Some(charging);
        Device {
            id: "m".into(),
            name: "Mouse".into(),
            kind: DeviceKind::Mouse,
            reading: r,
            percent_source: None,
            components: vec![],
            sources: vec![],
            estimate: None,
        }
    }

    #[test]
    fn charging_value_is_approximate_whatever_the_source() {
        let mut d = dev(52, true);
        mark_charging(&mut d);
        assert_eq!(d.reading.precision, Precision::Approximate);
        assert_eq!(d.estimate, Some(Estimate::Charging));
        let mut d = dev(52, false);
        mark_charging(&mut d);
        assert_eq!((d.reading.precision, d.estimate), (Precision::Exact, None));
    }

    #[test]
    fn settling_after_unplug_then_exact_again() {
        let mut t = SettlingTracker::default();
        let t0 = Instant::now();
        let mut v = vec![dev(52, true)];
        t.apply(t0, &mut v);
        let mut v = vec![dev(45, false)];
        t.apply(t0 + Duration::from_secs(60), &mut v);
        assert_eq!(v[0].reading.precision, Precision::Approximate);
        assert_eq!(v[0].estimate, Some(Estimate::Settling));
        let mut v = vec![dev(30, false)];
        t.apply(t0 + Duration::from_secs(601), &mut v);
        assert_eq!((v[0].reading.precision, v[0].estimate), (Precision::Exact, None));
    }

    #[test]
    fn devices_that_never_charged_are_untouched() {
        let mut t = SettlingTracker::default();
        let mut v = vec![dev(30, false)];
        t.apply(Instant::now(), &mut v);
        assert_eq!(v[0].estimate, None);
    }
}
