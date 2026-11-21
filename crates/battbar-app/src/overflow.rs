// SPDX-License-Identifier: GPL-3.0-or-later
//! Fallback when the expanded item does not fit: macOS hides a status item that has no
//! room in the menu bar (measured: the visible bit of the item window's occlusion state
//! clears; SPEC "Menu bar display"). battbar then switches the **setting** to Collapsed
//! and says so. It never expands by itself — only the user's Expand does.
//!
//! Event-driven, no polling: the app feeds in occlusion notifications of the item's
//! window and content/screen changes, and keeps at most **one one-shot timer**, as each
//! call tells it ([`Timer`]). When the item is shown and settled there is no timer.
//! Plain logic over times, so it is tested without AppKit.

use std::time::{Duration, Instant};

/// After the island's content, the screens or the mode change, a hidden state does not
/// count for this long (macOS places a new or resized item within ~0.5 s and reports it
/// hidden until then).
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

/// What to do with the one-shot timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// Leave it as it is (none, or the one already set).
    Keep,
    /// Remove it.
    Cancel,
    /// (Re)set it to fire once at this time; then call [`OverflowGuard::fire`].
    At(Instant),
}

#[derive(Debug, Default)]
pub struct OverflowGuard {
    /// Watching: set by `changed`, cleared by a collapse or `disarm`.
    armed: bool,
    settled_at: Option<Instant>,
    hidden_since: Option<Instant>,
}

impl OverflowGuard {
    /// The island's content (its size), the screens or the mode changed to Expanded:
    /// start over, and check once when it has settled.
    pub fn changed(&mut self, now: Instant) -> Timer {
        self.armed = true;
        self.settled_at = Some(now + SETTLE);
        self.hidden_since = None;
        Timer::At(now + SETTLE)
    }

    /// Collapsed (by the user or the fallback): nothing to watch.
    pub fn disarm(&mut self) -> Timer {
        self.armed = false;
        self.hidden_since = None;
        Timer::Cancel
    }

    fn due(&self, hidden_since: Instant) -> Instant {
        hidden_since.max(self.settled_at.unwrap_or(hidden_since)) + HIDDEN_FOR
    }

    /// The item window's occlusion state changed.
    pub fn occlusion(&mut self, now: Instant, visible: bool) -> (Option<Outcome>, Timer) {
        if !self.armed {
            return (None, Timer::Keep);
        }
        let settled = self.settled_at.is_none_or(|s| now >= s);
        if visible {
            self.hidden_since = None;
            if settled {
                return (Some(Outcome::Fits), Timer::Cancel);
            }
            // The settle check is still set (from `changed`).
            return (None, Timer::Keep);
        }
        let since = *self.hidden_since.get_or_insert(now);
        (None, Timer::At(self.due(since)))
    }

    /// The one-shot timer fired; `visible` is the item's state now.
    pub fn fire(&mut self, now: Instant, visible: bool) -> (Option<Outcome>, Timer) {
        if !self.armed {
            return (None, Timer::Cancel);
        }
        if let Some(s) = self.settled_at.filter(|s| now < *s) {
            return (None, Timer::At(s));
        }
        if visible {
            self.hidden_since = None;
            return (Some(Outcome::Fits), Timer::Cancel);
        }
        let since = *self.hidden_since.get_or_insert(now);
        let due = self.due(since);
        if now >= due {
            // Nothing more until the user expands again (a new `changed`).
            self.armed = false;
            self.hidden_since = None;
            return (Some(Outcome::Collapse), Timer::Cancel);
        }
        (None, Timer::At(due))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battbar_core::{DisplayMode, Settings};

    /// The app around the guard (mirrors `App::overflow_*`): one one-shot timer,
    /// occlusion events, the setting and notifications. Time in ms from `t0`.
    struct Sim {
        settings: Settings,
        guard: OverflowGuard,
        timer: Option<Instant>,
        /// Timers set (each one fires at most once).
        timers_set: u32,
        fired: u32,
        visible: bool,
        notifications: u32,
        t0: Instant,
    }

    impl Sim {
        /// Expanded, the item just created (macOS reports it hidden until placed).
        fn new() -> Self {
            let t0 = Instant::now();
            let mut s = Sim {
                settings: Settings {
                    display_mode: Some(DisplayMode::Expanded),
                    ..Default::default()
                },
                guard: OverflowGuard::default(),
                timer: None,
                timers_set: 0,
                fired: 0,
                visible: false,
                notifications: 0,
                t0,
            };
            let t = s.guard.changed(t0);
            s.apply(None, t);
            s
        }
        fn at(&self, ms: u64) -> Instant {
            self.t0 + Duration::from_millis(ms)
        }
        fn apply(&mut self, outcome: Option<Outcome>, timer: Timer) {
            match timer {
                Timer::Keep => {}
                Timer::Cancel => self.timer = None,
                Timer::At(t) => {
                    self.timer = Some(t);
                    self.timers_set += 1;
                }
            }
            match outcome {
                Some(Outcome::Collapse) => {
                    self.settings.display_mode = Some(DisplayMode::Collapsed);
                    self.settings.collapsed_for_space = true;
                    self.notifications += 1;
                    let t = self.guard.disarm();
                    self.apply(None, t);
                }
                Some(Outcome::Fits) => self.settings.collapsed_for_space = false,
                None => {}
            }
        }
        /// Let time pass to `ms`, firing the timer when it is due.
        fn advance(&mut self, ms: u64) {
            while let Some(t) = self.timer.filter(|t| *t <= self.at(ms)) {
                self.timer = None;
                self.fired += 1;
                let (o, timer) = self.guard.fire(t, self.visible);
                self.apply(o, timer);
            }
        }
        /// macOS shows or hides the item at `ms` (occlusion notification).
        fn occlusion(&mut self, ms: u64, visible: bool) {
            self.advance(ms);
            self.visible = visible;
            if self.settings.display_mode() == DisplayMode::Expanded {
                let (o, t) = self.guard.occlusion(self.at(ms), visible);
                self.apply(o, t);
            }
        }
        fn user_expands(&mut self, ms: u64) {
            self.advance(ms);
            self.settings.display_mode = Some(DisplayMode::Expanded);
            let t = self.guard.changed(self.at(ms));
            self.apply(None, t);
        }
        fn mode(&self) -> DisplayMode {
            self.settings.display_mode()
        }
    }

    #[test]
    fn hidden_for_a_second_collapses_and_notifies() {
        let mut s = Sim::new();
        s.occlusion(400, true); // placed
        s.advance(1000); // settle check: shown -> Fits, no timer left
        assert_eq!(s.timer, None);
        // Chrome comes to the front: hidden from 5000 ms on.
        s.occlusion(5000, false);
        s.advance(5900);
        assert_eq!(s.mode(), DisplayMode::Expanded, "0.9 s hidden is not enough");
        s.advance(6000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        assert!(s.settings.collapsed_for_space);
        assert_eq!(s.notifications, 1);
        assert_eq!(s.timer, None);
    }

    #[test]
    fn shown_again_before_a_second_cancels() {
        let mut s = Sim::new();
        s.occlusion(400, true);
        s.occlusion(5000, false);
        s.occlusion(5600, true); // Finder in front again
        assert_eq!(s.timer, None, "the one-shot timer is cancelled");
        s.advance(60_000);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        assert_eq!(s.notifications, 0);
    }

    #[test]
    fn initial_placement_does_not_collapse() {
        // macOS reports a new item hidden until it is placed (~0.5 s).
        let mut s = Sim::new();
        s.occlusion(0, false);
        s.occlusion(500, true);
        s.advance(10_000);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        // The same after a content change (a device appears, the item grows).
        let t = s.guard.changed(s.at(20_000));
        s.apply(None, t);
        s.occlusion(20_100, false);
        s.occlusion(20_600, true);
        s.advance(30_000);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        assert_eq!(s.notifications, 0);
    }

    #[test]
    fn hidden_from_the_start_collapses_after_settling() {
        // No occlusion notification at all (hidden from creation): the settle check
        // finds it hidden and checks again one second later.
        let mut s = Sim::new();
        s.advance(1900);
        assert_eq!(s.mode(), DisplayMode::Expanded, "settle 1 s + hidden 1 s");
        s.advance(2000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        assert_eq!(s.notifications, 1);
        // Hidden before settling counts only from the end of settling.
        let mut s = Sim::new();
        s.occlusion(100, false);
        s.advance(1900);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        s.advance(2000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
    }

    #[test]
    fn no_repeating_timers_when_calm_or_collapsed() {
        // Calm Expanded: shown and settled -> no timer, nothing fires for a minute.
        let mut s = Sim::new();
        s.occlusion(300, true);
        s.advance(1000);
        let (set, fired) = (s.timers_set, s.fired);
        assert_eq!(s.timer, None);
        s.advance(60_000);
        assert_eq!((s.timers_set, s.fired), (set, fired), "calm Expanded: no timers");
        // Collapsed by the fallback: events and time change nothing.
        s.occlusion(61_000, false);
        s.advance(63_000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        let (set, fired) = (s.timers_set, s.fired);
        for ms in (64_000..120_000).step_by(700) {
            s.occlusion(ms, ms % 1400 == 0);
        }
        s.advance(180_000);
        assert_eq!(s.timer, None);
        assert_eq!((s.timers_set, s.fired), (set, fired), "Collapsed: no timers");
        // A disarmed guard asks for no timer, whatever happens.
        let mut g = OverflowGuard::default();
        let t = Instant::now();
        assert_eq!(g.occlusion(t, false), (None, Timer::Keep));
        assert_eq!(g.fire(t, false), (None, Timer::Cancel));
    }

    #[test]
    fn a_hidden_spell_sets_one_timer_and_it_fires_once() {
        let mut s = Sim::new();
        s.occlusion(300, true);
        s.advance(1000);
        let (set, fired) = (s.timers_set, s.fired);
        s.occlusion(5000, false);
        s.advance(6000);
        assert_eq!((s.timers_set - set, s.fired - fired), (1, 1));
    }

    #[test]
    fn nothing_expands_by_itself_after_the_fallback() {
        let mut s = Sim::new();
        s.advance(2000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        // Room again (Finder in front), shown/hidden many times, a long time passes.
        for ms in (2500..=60_000).step_by(500) {
            s.occlusion(ms, ms % 1000 == 0);
        }
        s.advance(600_000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        assert!(s.settings.collapsed_for_space, "the note stays until an Expand that fits");
        assert_eq!(s.notifications, 1);
    }

    #[test]
    fn expanding_again_without_room_collapses_again() {
        let mut s = Sim::new();
        s.advance(2000);
        assert_eq!(s.notifications, 1);
        s.user_expands(10_000);
        s.occlusion(10_300, false);
        s.advance(12_000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        assert_eq!(s.notifications, 2, "a new notification every time");
        // Expand once there is room: the note goes away.
        s.user_expands(20_000);
        s.occlusion(20_400, true);
        s.advance(21_000);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        assert!(!s.settings.collapsed_for_space);
        assert_eq!(s.timer, None);
        // Still watching: a later overflow collapses again.
        s.occlusion(30_000, false);
        s.advance(31_000);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        assert_eq!(s.notifications, 3);
    }
}
