// SPDX-License-Identifier: GPL-3.0-or-later
//! Low-battery and spare-battery alert decisions (pure logic, unit-tested).
//!
//! * One low-battery alert per discharge cycle.
//! * A new cycle starts only when a *measured* value rises above threshold + 10 % and
//!   then falls below the threshold again (no flapping around the threshold).
//! * Values estimated because of charging or the settling period after it never alert
//!   and never start a new cycle (they are inflated by the charge voltage).

use battbar_core::*;
use std::collections::HashSet;

const HYSTERESIS: u8 = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[derive(Default)]
pub struct AlertTracker {
    low_sent: HashSet<String>,
    spare_sent: HashSet<String>,
}

impl AlertTracker {
    pub fn check(&mut self, devices: &[Device], threshold: Option<u8>, notify_spare: bool) -> Vec<Alert> {
        let mut out = vec![];
        for d in devices {
            let r = &d.reading;
            if let (Some(th), Some(p), Status::Ok, None) = (threshold, r.percent, r.status, d.estimate) {
                if p > th.saturating_add(HYSTERESIS) {
                    self.low_sent.remove(&d.id);
                } else if p <= th && !self.low_sent.contains(&d.id) {
                    self.low_sent.insert(d.id.clone());
                    out.push(Alert {
                        id: format!("low-{}", d.id),
                        title: format!("{} battery low", d.name),
                        body: format!("{p}% remaining."),
                    });
                }
            }
            for c in d.components.iter().filter(|c| c.kind == ComponentKind::Spare) {
                match c.reading.percent {
                    Some(100) if notify_spare && !self.spare_sent.contains(&d.id) => {
                        self.spare_sent.insert(d.id.clone());
                        out.push(Alert {
                            id: format!("spare-{}", d.id),
                            title: format!("{}: spare battery charged", d.name),
                            body: "The spare battery in the base station is fully charged.".into(),
                        });
                    }
                    Some(p) if p < 100 => {
                        self.spare_sent.remove(&d.id);
                    }
                    _ => {}
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(p: u8, estimate: Option<Estimate>) -> Device {
        let mut r = Reading::ok(
            p,
            if estimate.is_some() {
                Precision::Approximate
            } else {
                Precision::Exact
            },
        );
        r.charging = Some(estimate == Some(Estimate::Charging));
        Device {
            id: "m".into(),
            name: "Mouse".into(),
            kind: DeviceKind::Mouse,
            reading: r,
            percent_source: None,
            components: vec![],
            sources: vec![],
            estimate,
        }
    }

    fn check(t: &mut AlertTracker, p: u8, e: Option<Estimate>) -> usize {
        t.check(&[dev(p, e)], Some(15), false).len()
    }

    #[test]
    fn once_per_cycle_with_ten_percent_hysteresis() {
        let mut t = AlertTracker::default();
        assert_eq!(check(&mut t, 20, None), 0);
        assert_eq!(check(&mut t, 15, None), 1);
        assert_eq!(check(&mut t, 12, None), 0, "no repeat in the same cycle");
        assert_eq!(check(&mut t, 24, None), 0, "below threshold + 10: same cycle");
        assert_eq!(check(&mut t, 14, None), 0, "no flapping alert");
        assert_eq!(check(&mut t, 26, None), 0, "above threshold + 10: new cycle starts");
        assert_eq!(check(&mut t, 15, None), 1);
    }

    #[test]
    fn charging_and_settling_never_alert_nor_reset() {
        let mut t = AlertTracker::default();
        assert_eq!(check(&mut t, 8, None), 1);
        // Plugged in: 8 % → 52 % within seconds (voltage-inflated).
        assert_eq!(check(&mut t, 52, Some(Estimate::Charging)), 0);
        // Unplugged after 15 s: value relaxes back; still settling.
        assert_eq!(check(&mut t, 12, Some(Estimate::Settling)), 0);
        // Settled at 10 %: same cycle, no second alert.
        assert_eq!(check(&mut t, 10, None), 0);
    }

    #[test]
    fn first_alert_suppressed_while_settling() {
        let mut t = AlertTracker::default();
        assert_eq!(check(&mut t, 9, Some(Estimate::Settling)), 0);
        assert_eq!(check(&mut t, 9, None), 1, "alert once the value is measured again");
    }

    #[test]
    fn non_rechargeable_battery_never_alerts() {
        // 2.9 V on 0x1001 (AA cells): not reported, no percentage, so no alert.
        let mut t = AlertTracker::default();
        let mut d = dev(5, None);
        d.reading = Reading::with_status(Status::NotReported);
        d.reading.voltage_mv = Some(2900);
        d.reading.note = Some(ReadingNote::NonRechargeableBattery);
        assert!(t.check(&[d], Some(15), false).is_empty());
    }

    #[test]
    fn off_threshold_and_asleep_never_alert() {
        let mut t = AlertTracker::default();
        assert!(t.check(&[dev(5, None)], None, false).is_empty());
        let mut d = dev(5, None);
        d.reading = Reading::with_status(Status::Asleep);
        assert!(t.check(&[d], Some(15), false).is_empty());
    }

    #[test]
    fn spare_full_alert_is_optional_and_once() {
        let mut d = dev(80, None);
        d.components = vec![Component {
            kind: ComponentKind::Spare,
            reading: Reading::ok(100, Precision::Coarse),
        }];
        let mut t = AlertTracker::default();
        assert!(t.check(std::slice::from_ref(&d), Some(15), false).is_empty(), "off by default");
        assert_eq!(t.check(std::slice::from_ref(&d), Some(15), true).len(), 1);
        assert!(t.check(std::slice::from_ref(&d), Some(15), true).is_empty());
    }
}
