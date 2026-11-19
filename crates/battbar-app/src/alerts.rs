// SPDX-License-Identifier: GPL-3.0-or-later
//! Low-battery and spare-battery alert decisions (pure logic, unit-tested).
//! One low-battery alert per discharge cycle: the cycle resets when the device charges
//! or its level rises clearly above the threshold.

use battbar_core::*;
use std::collections::HashSet;

const HYSTERESIS: u8 = 5;

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
            if let (Some(th), Some(p), Status::Ok) = (threshold, r.percent, r.status) {
                let charging = r.charging == Some(true);
                if charging || p > th.saturating_add(HYSTERESIS) {
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
        }
    }

    #[test]
    fn once_per_discharge_cycle() {
        let mut t = AlertTracker::default();
        assert!(t.check(&[dev(20, false)], Some(15), false).is_empty());
        assert_eq!(t.check(&[dev(15, false)], Some(15), false).len(), 1);
        assert!(
            t.check(&[dev(12, false)], Some(15), false).is_empty(),
            "no repeat in the same cycle"
        );
        assert!(
            t.check(&[dev(17, false)], Some(15), false).is_empty(),
            "small rise is not a new cycle"
        );
        assert!(t.check(&[dev(14, false)], Some(15), false).is_empty());
        // Charged → new cycle.
        assert!(t.check(&[dev(30, true)], Some(15), false).is_empty());
        assert_eq!(t.check(&[dev(10, false)], Some(15), false).len(), 1);
    }

    #[test]
    fn off_threshold_and_charging_never_alert() {
        let mut t = AlertTracker::default();
        assert!(t.check(&[dev(5, false)], None, false).is_empty());
        assert!(t.check(&[dev(5, true)], Some(15), false).is_empty());
    }

    #[test]
    fn asleep_device_does_not_alert() {
        let mut t = AlertTracker::default();
        let mut d = dev(5, false);
        d.reading = Reading::with_status(Status::Asleep);
        assert!(t.check(&[d], Some(15), false).is_empty());
    }

    #[test]
    fn spare_full_alert_is_optional_and_once() {
        let mut d = dev(80, false);
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
