// SPDX-License-Identifier: GPL-3.0-or-later
//! Fallback when the expanded item does not fit: macOS hides a status item that has no
//! room in the menu bar (measured: the visible bit of the item window's occlusion state
//! clears; SPEC "Menu bar display"). battbar then switches the **setting** to Collapsed
//! and says so. It never expands by itself — only the user's Expand does.
//!
//! Plain logic over observations and times, so it is tested without AppKit.

use std::time::{Duration, Instant};

/// After the island's content or the screens change, observations are ignored this
/// long (macOS places a new or resized item within ~0.5 s and reports it hidden until
/// then).
pub const SETTLE: Duration = Duration::from_secs(1);
/// How long the item must stay hidden (continuously, after settling) to collapse.
pub const HIDDEN_FOR: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Hidden for long enough: switch to Collapsed and notify.
    Collapse,
    /// Shown after settling: the last Expand worked.
    Fits,
}

#[derive(Debug, Default)]
pub struct OverflowGuard {
    /// Watching (set by `changed`; cleared after a collapse until the next Expand).
    armed: bool,
    /// Observations before this time are ignored.
    settled_at: Option<Instant>,
    hidden_since: Option<Instant>,
}

impl OverflowGuard {
    /// The island's content (its size) or the screens changed, or the user chose
    /// Expand: start over.
    pub fn changed(&mut self, now: Instant) {
        self.armed = true;
        self.settled_at = Some(now + SETTLE);
        self.hidden_since = None;
    }

    /// One observation while the mode is Expanded. `visible`: the item window's
    /// occlusion state has the visible bit.
    pub fn observe(&mut self, now: Instant, visible: bool) -> Option<Outcome> {
        if !self.armed {
            return None;
        }
        let settled_at = self.settled_at.get_or_insert(now + SETTLE);
        if now < *settled_at {
            self.hidden_since = None;
            return None;
        }
        if visible {
            self.hidden_since = None;
            return Some(Outcome::Fits);
        }
        let since = *self.hidden_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= HIDDEN_FOR {
            // Nothing more until the user expands again (a new `changed`).
            self.armed = false;
            self.hidden_since = None;
            return Some(Outcome::Collapse);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battbar_core::{DisplayMode, Settings};

    /// What the app does with the outcomes (mirrors `App::check_overflow`).
    struct Sim {
        settings: Settings,
        guard: OverflowGuard,
        notifications: u32,
        t0: Instant,
    }

    impl Sim {
        fn new() -> Self {
            let settings = Settings {
                display_mode: Some(DisplayMode::Expanded),
                ..Default::default()
            };
            let t0 = Instant::now();
            let mut guard = OverflowGuard::default();
            guard.changed(t0);
            Sim {
                settings,
                guard,
                notifications: 0,
                t0,
            }
        }
        fn at(&self, ms: u64) -> Instant {
            self.t0 + Duration::from_millis(ms)
        }
        /// Observation every 500 ms, as the app's timer.
        fn observe(&mut self, ms: u64, visible: bool) {
            if self.settings.display_mode() != DisplayMode::Expanded {
                return;
            }
            match self.guard.observe(self.at(ms), visible) {
                Some(Outcome::Collapse) => {
                    self.settings.display_mode = Some(DisplayMode::Collapsed);
                    self.settings.collapsed_for_space = true;
                    self.notifications += 1;
                }
                Some(Outcome::Fits) => self.settings.collapsed_for_space = false,
                None => {}
            }
        }
        fn user_expands(&mut self, ms: u64) {
            self.settings.display_mode = Some(DisplayMode::Expanded);
            self.guard.changed(self.at(ms));
        }
    }

    #[test]
    fn hidden_for_a_second_collapses_and_notifies() {
        let mut s = Sim::new();
        for ms in (0..=1000).step_by(500) {
            s.observe(ms, true);
        }
        // Chrome comes to the front; the item is hidden from 1500 ms on.
        s.observe(1500, false);
        s.observe(2000, false);
        assert_eq!(s.settings.display_mode(), DisplayMode::Expanded, "0.5 s hidden is not enough");
        s.observe(2500, false);
        assert_eq!(s.settings.display_mode(), DisplayMode::Collapsed);
        assert!(s.settings.collapsed_for_space);
        assert_eq!(s.notifications, 1);
    }

    #[test]
    fn a_short_hide_does_not_collapse() {
        let mut s = Sim::new();
        s.observe(1500, false);
        s.observe(2000, true);
        s.observe(2500, false);
        s.observe(3000, true);
        assert_eq!(s.settings.display_mode(), DisplayMode::Expanded);
        assert_eq!(s.notifications, 0);
    }

    #[test]
    fn initial_placement_does_not_collapse() {
        // macOS reports a new item hidden until it is placed (~0.5 s).
        let mut s = Sim::new();
        s.observe(0, false);
        s.observe(500, false);
        s.observe(1000, true);
        s.observe(1500, true);
        assert_eq!(s.settings.display_mode(), DisplayMode::Expanded);
        // The same after a content change (a device appears, the item grows).
        s.guard.changed(s.at(1600));
        s.observe(2000, false);
        s.observe(2500, false);
        s.observe(3000, true);
        assert_eq!(s.settings.display_mode(), DisplayMode::Expanded);
        assert_eq!(s.notifications, 0);
    }

    #[test]
    fn hidden_from_the_start_collapses_after_settling() {
        let mut s = Sim::new();
        for ms in (0..=1500).step_by(500) {
            s.observe(ms, false);
        }
        assert_eq!(s.settings.display_mode(), DisplayMode::Expanded, "settle 1 s + hidden 1 s");
        s.observe(2000, false);
        assert_eq!(s.settings.display_mode(), DisplayMode::Collapsed);
    }

    #[test]
    fn the_guard_itself_stays_quiet_after_a_collapse() {
        let mut g = OverflowGuard::default();
        let t = Instant::now();
        g.changed(t);
        assert_eq!(g.observe(t + Duration::from_millis(1000), false), None);
        assert_eq!(g.observe(t + Duration::from_millis(2000), false), Some(Outcome::Collapse));
        // Even if the app kept asking: no second collapse, no "fits".
        assert_eq!(g.observe(t + Duration::from_millis(5000), false), None);
        assert_eq!(g.observe(t + Duration::from_millis(6000), true), None);
        // Never armed: silent.
        assert_eq!(OverflowGuard::default().observe(t, false), None);
    }

    #[test]
    fn nothing_expands_by_itself_after_the_fallback() {
        let mut s = Sim::new();
        for ms in (1000..=2000).step_by(500) {
            s.observe(ms, false);
        }
        assert_eq!(s.settings.display_mode(), DisplayMode::Collapsed);
        // Room again (Finder in front), screens change, a long time passes.
        for ms in (2500..=60_000).step_by(500) {
            s.observe(ms, ms % 1000 == 0);
            if ms == 30_000 {
                s.guard.changed(s.at(ms));
            }
        }
        assert_eq!(s.settings.display_mode(), DisplayMode::Collapsed);
        assert!(s.settings.collapsed_for_space, "the note stays until an Expand that fits");
        assert_eq!(s.notifications, 1);
    }

    #[test]
    fn expanding_again_without_room_collapses_again() {
        let mut s = Sim::new();
        for ms in (1000..=2000).step_by(500) {
            s.observe(ms, false);
        }
        assert_eq!(s.notifications, 1);
        s.user_expands(10_000);
        for ms in (10_000..=12_000).step_by(500) {
            s.observe(ms, false);
        }
        assert_eq!(s.settings.display_mode(), DisplayMode::Collapsed);
        assert_eq!(s.notifications, 2, "a new notification every time");
        // Expand once there is room: the note goes away.
        s.user_expands(20_000);
        s.observe(20_500, true);
        s.observe(21_000, true);
        assert_eq!(s.settings.display_mode(), DisplayMode::Expanded);
        assert!(!s.settings.collapsed_for_space);
        // The guard keeps watching: later overflow collapses again.
        s.observe(30_000, false);
        s.observe(31_000, false);
        assert_eq!(s.settings.display_mode(), DisplayMode::Collapsed);
        assert_eq!(s.notifications, 3);
    }
}
