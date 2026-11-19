// SPDX-License-Identifier: GPL-3.0-or-later
//! Learns headset ↔ Bluetooth links (SPEC "Learned headset ↔ Bluetooth link").
//!
//! A link is learned only when a base station's "headset on Bluetooth" flag changes
//! from off to on, and within `window` exactly one *connected* Bluetooth device matches
//! the model's name patterns. More than one candidate at any point aborts learning.

use crate::merge::glob_match;
use crate::model::{DeviceReport, MergeHint, Status, Transport};
use crate::settings::LearnedLink;
use std::collections::HashMap;
use std::time::{Duration, Instant};

struct Pending {
    base: String,
    name: String,
    patterns: Vec<String>,
    deadline: Instant,
}

pub struct LinkLearner {
    flags: HashMap<String, bool>,
    pending: Vec<Pending>,
    pub window: Duration,
}

impl Default for LinkLearner {
    fn default() -> Self {
        LinkLearner {
            flags: HashMap::new(),
            pending: vec![],
            window: Duration::from_secs(8),
        }
    }
}

fn companion_patterns(r: &DeviceReport) -> Option<&Vec<String>> {
    r.hints
        .iter()
        .map(|MergeHint::BluetoothCompanion { name_patterns }| name_patterns)
        .next()
}

impl LinkLearner {
    /// Whether a quick follow-up poll would help (a flag just went on, no answer yet).
    pub fn wants_followup(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn observe(&mut self, now: Instant, reports: &[DeviceReport]) -> Vec<LearnedLink> {
        // 1. Flag transitions off → on start a learning window. An unknown previous
        //    state (first poll, base just plugged in) does not count.
        for r in reports.iter().filter(|r| r.transport == Transport::BaseStation) {
            let key = r.full_key();
            let on = companion_patterns(r).is_some();
            let was = self.flags.insert(key.clone(), on);
            if was == Some(false) && on {
                self.pending.retain(|p| p.base != key);
                self.pending.push(Pending {
                    base: key,
                    name: r.name.clone(),
                    patterns: companion_patterns(r).cloned().unwrap_or_default(),
                    deadline: now + self.window,
                });
            }
            if !on {
                self.pending.retain(|p| p.base != r.full_key());
            }
        }
        // 2. Evaluate pending windows.
        let mut learned = vec![];
        self.pending.retain(|p| {
            if now > p.deadline {
                return false;
            }
            let cands: Vec<&DeviceReport> = reports
                .iter()
                .filter(|b| {
                    b.provider == "bt-system"
                        && matches!(b.transport, Transport::BluetoothClassic | Transport::BluetoothLe)
                        && matches!(b.reading.status, Status::Ok | Status::NotReported)
                        && p.patterns.iter().any(|pat| glob_match(pat, &b.name))
                })
                .collect();
            match cands.len() {
                0 => true, // keep waiting within the window
                1 => {
                    learned.push(LearnedLink {
                        base: p.base.clone(),
                        bluetooth: cands[0].full_key(),
                        name: p.name.clone(),
                    });
                    false
                }
                _ => false, // ambiguous: manual merge only
            }
        });
        learned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn base(flag: bool) -> DeviceReport {
        DeviceReport {
            provider: "steelseries".into(),
            source_key: "base".into(),
            source: "".into(),
            name: "Arctis Nova Pro Wireless".into(),
            kind: DeviceKind::Headset,
            transport: Transport::BaseStation,
            identity: vec![],
            hints: if flag {
                vec![MergeHint::BluetoothCompanion {
                    name_patterns: vec!["Arctis Nova Pro*".into()],
                }]
            } else {
                vec![]
            },
            reading: Reading::ok(100, Precision::Coarse),
            components: vec![],
        }
    }

    fn bt(key: &str, name: &str, status: Status) -> DeviceReport {
        let mut r = base(false);
        r.provider = "bt-system".into();
        r.source_key = key.into();
        r.name = name.into();
        r.transport = Transport::BluetoothClassic;
        r.reading = Reading::with_status(status);
        if status == Status::Ok {
            r.reading = Reading::ok(80, Precision::Exact);
        }
        r
    }

    #[test]
    fn learns_on_transition_with_single_candidate() {
        let mut l = LinkLearner::default();
        let t = Instant::now();
        assert!(l
            .observe(t, &[base(false), bt("c:1", "Arctis Nova Pro Wireless", Status::Disconnected)])
            .is_empty());
        let got = l.observe(
            t + Duration::from_secs(60),
            &[base(true), bt("c:1", "Arctis Nova Pro Wireless", Status::Ok)],
        );
        assert_eq!(
            got,
            vec![LearnedLink {
                base: "steelseries/base".into(),
                bluetooth: "bt-system/c:1".into(),
                name: "Arctis Nova Pro Wireless".into()
            }]
        );
    }

    #[test]
    fn no_learning_without_observed_transition() {
        let mut l = LinkLearner::default();
        // Flag already on at the first poll (app start): unknown previous state.
        assert!(l
            .observe(Instant::now(), &[base(true), bt("c:1", "Arctis Nova Pro Wireless", Status::Ok)])
            .is_empty());
    }

    #[test]
    fn candidate_may_appear_within_window_but_not_after() {
        let mut l = LinkLearner::default();
        let t = Instant::now();
        l.observe(t, &[base(false)]);
        assert!(l.observe(t + Duration::from_secs(1), &[base(true)]).is_empty());
        assert!(l.wants_followup());
        let got = l.observe(
            t + Duration::from_secs(4),
            &[base(true), bt("c:1", "Arctis Nova Pro Wireless", Status::Ok)],
        );
        assert_eq!(got.len(), 1);

        let mut l = LinkLearner::default();
        l.observe(t, &[base(false)]);
        l.observe(t + Duration::from_secs(1), &[base(true)]);
        assert!(l
            .observe(
                t + Duration::from_secs(30),
                &[base(true), bt("c:1", "Arctis Nova Pro Wireless", Status::Ok)]
            )
            .is_empty());
    }

    #[test]
    fn several_candidates_abort() {
        let mut l = LinkLearner::default();
        let t = Instant::now();
        l.observe(t, &[base(false)]);
        let r = [
            base(true),
            bt("c:1", "Arctis Nova Pro Wireless", Status::Ok),
            bt("c:2", "Arctis Nova Pro X", Status::Ok),
        ];
        assert!(l.observe(t + Duration::from_secs(1), &r).is_empty());
        assert!(!l.wants_followup());
    }

    #[test]
    fn disconnected_or_non_matching_devices_are_not_candidates() {
        let mut l = LinkLearner::default();
        let t = Instant::now();
        l.observe(t, &[base(false)]);
        let r = [
            base(true),
            bt("c:1", "Arctis Nova Pro Wireless", Status::Disconnected),
            bt("c:2", "Other Headphones", Status::Ok),
        ];
        assert!(l.observe(t + Duration::from_secs(1), &r).is_empty());
        assert!(l.wants_followup());
    }
}
