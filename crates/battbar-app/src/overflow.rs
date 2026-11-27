// SPDX-License-Identifier: GPL-3.0-or-later
//! Fallback when the expanded item does not fit: macOS hides a status item that has no
//! room in the menu bar (measured: the visible bit of the item window's occlusion state
//! clears; SPEC "Menu bar display"). battbar then switches the **setting** to Collapsed
//! and says so. It never expands by itself — only the user's Expand does.
//!
//! A full-screen app (a video, a game) or "automatically hide the menu bar" hides the
//! **whole** menu bar: the item is not visible then either, but that says nothing about
//! space. The item's window then lies above the top edge of its screen (measured:
//! y = screen top while hidden, in the menu bar band when shown or hidden for lack of
//! space; DISCOVERY "Full-screen apps"), so the app reports [`Seen::MenuBarHidden`] and
//! the guard waits until the menu bar is back.
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

/// What the app sees of the item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// Visible in the menu bar.
    Shown,
    /// Not visible while the menu bar is shown: no room for it.
    Hidden,
    /// The whole menu bar is hidden (full-screen app, auto-hide): nothing to judge.
    MenuBarHidden,
}

/// A rectangle in screen coordinates (points, origin bottom left, like AppKit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The menu bar is at most this tall (37–38 pt with a notch, 24–30 pt without).
const MENU_BAR_MAX: f64 = 60.0;

/// Whether the item's window sits in the menu bar band at the top of a screen — i.e.
/// the menu bar is shown. When macOS hides the menu bar, the window moves above the
/// screen's top edge (or gets zero height). The screens it overlaps horizontally count;
/// if it overlaps none (an item without room can be pushed sideways off every screen),
/// any screen does.
pub fn in_menu_bar(item: Rect, screens: &[Rect]) -> bool {
    if item.h <= 0.0 {
        return false;
    }
    let top = item.y + item.h;
    let in_band = |s: &&Rect| {
        let screen_top = s.y + s.h;
        top <= screen_top + 0.5 && top >= screen_top - MENU_BAR_MAX
    };
    let overlapping: Vec<&Rect> = screens.iter().filter(|s| item.x < s.x + s.w && item.x + item.w > s.x).collect();
    if overlapping.is_empty() {
        screens.iter().any(|s| in_band(&s))
    } else {
        overlapping.iter().any(in_band)
    }
}

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

    /// The item window's occlusion state changed, or it moved, or the active space
    /// changed (the menu bar may be back while the item stays hidden).
    pub fn occlusion(&mut self, now: Instant, seen: Seen) -> (Option<Outcome>, Timer) {
        if !self.armed {
            return (None, Timer::Keep);
        }
        let settled = self.settled_at.is_none_or(|s| now >= s);
        if seen == Seen::MenuBarHidden {
            // Hidden with the whole menu bar: not "no room". Start over when it is back.
            self.hidden_since = None;
            return (None, if settled { Timer::Cancel } else { Timer::Keep });
        }
        if seen == Seen::Shown {
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

    /// The one-shot timer fired; `seen` is the item's state now.
    pub fn fire(&mut self, now: Instant, seen: Seen) -> (Option<Outcome>, Timer) {
        if !self.armed {
            return (None, Timer::Cancel);
        }
        if let Some(s) = self.settled_at.filter(|s| now < *s) {
            return (None, Timer::At(s));
        }
        if seen == Seen::MenuBarHidden {
            self.hidden_since = None;
            return (None, Timer::Cancel);
        }
        if seen == Seen::Shown {
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

/// The automatic fallback needs macOS to say that it hid the item. macOS 26 and older
/// do (occlusion); macOS 27 hides an item without room but reports it visible
/// (DISCOVERY "Full-screen apps"), so the fallback is off there: re-opening battbar is
/// the way out (see [`reopen`]).
pub fn auto_collapse_supported(macos_major: isize) -> bool {
    macos_major <= 26
}

/// What re-opening the running battbar (Spotlight, Applications, `open -a battbar`) did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reopen {
    /// It was Expanded (and perhaps hidden by macOS for lack of room): now Collapsed.
    Collapsed,
    /// Already Collapsed: nothing to change.
    AlreadyCollapsed,
}

/// Re-opening battbar while it runs is the way out when its expanded item does not fit
/// and macOS hides it: switch to Collapsed (one icon), with the same note in the menu
/// as the automatic fallback.
pub fn reopen(settings: &mut battbar_core::Settings) -> Reopen {
    if settings.display_mode() == battbar_core::DisplayMode::Collapsed {
        return Reopen::AlreadyCollapsed;
    }
    settings.display_mode = Some(battbar_core::DisplayMode::Collapsed);
    settings.collapsed_for_space = true;
    Reopen::Collapsed
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
        seen: Seen,
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
                seen: Seen::Hidden,
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
                let (o, timer) = self.guard.fire(t, self.seen);
                self.apply(o, timer);
            }
        }
        /// macOS shows or hides the item at `ms` (occlusion notification), with the
        /// menu bar shown.
        fn occlusion(&mut self, ms: u64, visible: bool) {
            self.seen_at(ms, if visible { Seen::Shown } else { Seen::Hidden });
        }
        /// What the app sees at `ms` (occlusion, move or space-change notification).
        fn seen_at(&mut self, ms: u64, seen: Seen) {
            self.advance(ms);
            self.seen = seen;
            if self.settings.display_mode() == DisplayMode::Expanded {
                let (o, t) = self.guard.occlusion(self.at(ms), seen);
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
        assert_eq!(g.occlusion(t, Seen::Hidden), (None, Timer::Keep));
        assert_eq!(g.fire(t, Seen::Hidden), (None, Timer::Cancel));
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

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    /// Frames measured with `--probe-menu-bar` on the reference Mac (two displays:
    /// 2560×1440 at the origin, 1920×1080 to its right at y = 229).
    #[test]
    fn menu_bar_band_from_measured_frames() {
        let screens = [rect(0.0, 0.0, 2560.0, 1440.0), rect(2560.0, 229.0, 1920.0, 1080.0)];
        // Shown (or hidden for lack of space) in the menu bar of either display.
        assert!(in_menu_bar(rect(1539.0, 1410.0, 136.0, 30.0), &screens));
        assert!(in_menu_bar(rect(3459.0, 1279.0, 136.0, 30.0), &screens));
        // Too wide to fit (2416 pt): hidden in the menu bar band, partly off its display.
        assert!(in_menu_bar(rect(1179.0, 1279.0, 2416.0, 30.0), &screens));
        // Full-screen video: the menu bar slid up, the window lies above the top edge.
        assert!(!in_menu_bar(rect(1539.0, 1440.0, 136.0, 30.0), &screens));
        assert!(
            !in_menu_bar(rect(1539.0, 1440.0, 136.0, 0.0), &screens),
            "zero height while sliding"
        );
        // A display placed above the main one: still above the main one's menu bar, and
        // nowhere near the top of the upper display.
        let stacked = [rect(0.0, 0.0, 2560.0, 1440.0), rect(0.0, 1440.0, 2560.0, 1440.0)];
        assert!(!in_menu_bar(rect(1539.0, 1440.0, 136.0, 30.0), &stacked));
        assert!(in_menu_bar(rect(1539.0, 2850.0, 136.0, 30.0), &stacked));
        // Side by side with different tops (the right one 30 pt higher): full screen on
        // the left puts the window at the right one's menu bar height, but not above it.
        let uneven = [rect(0.0, 0.0, 2560.0, 1440.0), rect(2560.0, 0.0, 2560.0, 1470.0)];
        assert!(!in_menu_bar(rect(1539.0, 1440.0, 136.0, 30.0), &uneven));
        // Without room, pushed sideways off every screen but still at menu bar height.
        assert!(in_menu_bar(rect(-400.0, 1410.0, 136.0, 30.0), &screens));
    }

    /// A full-screen app hides the whole menu bar for minutes: no collapse, no
    /// notification; back on the desktop with the item shown, nothing happened.
    #[test]
    fn full_screen_app_does_not_collapse() {
        let mut s = Sim::new();
        s.occlusion(400, true);
        s.advance(5_000);
        // A video goes full screen: the item is hidden first while the menu bar slides
        // up (still in the band), then the window is above the screen.
        s.occlusion(10_000, false);
        s.seen_at(10_400, Seen::MenuBarHidden);
        s.advance(600_000);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        assert_eq!(s.notifications, 0);
        assert_eq!(s.timer, None, "no timer while the menu bar is hidden");
        // Back: the item is shown again.
        s.occlusion(600_500, true);
        s.advance(700_000);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        assert_eq!(s.notifications, 0);
    }

    /// Even when the timer fires during the slide (no move notification in time), the
    /// state at that moment decides.
    #[test]
    fn timer_during_full_screen_does_not_collapse() {
        let mut s = Sim::new();
        s.occlusion(400, true);
        s.occlusion(10_000, false);
        s.seen = Seen::MenuBarHidden; // moved, without a notification
        s.advance(20_000);
        assert_eq!(s.mode(), DisplayMode::Expanded);
        assert_eq!(s.notifications, 0);
    }

    /// Leaving a full-screen app to a desktop where the item really has no room: the
    /// item was hidden all along (no occlusion change), but the move or space change
    /// says the menu bar is back, and the usual 1 s rule applies.
    #[test]
    fn no_room_after_leaving_full_screen_still_collapses() {
        let mut s = Sim::new();
        s.occlusion(400, true);
        s.occlusion(10_000, false);
        s.seen_at(10_400, Seen::MenuBarHidden);
        s.advance(60_000);
        s.seen_at(60_000, Seen::Hidden); // back in the band, still hidden
        s.advance(60_900);
        assert_eq!(s.mode(), DisplayMode::Expanded, "not before 1 s");
        s.advance(61_100);
        assert_eq!(s.mode(), DisplayMode::Collapsed);
        assert_eq!(s.notifications, 1);
    }

    #[test]
    fn auto_collapse_only_up_to_macos_26() {
        assert!(auto_collapse_supported(14));
        assert!(auto_collapse_supported(26));
        assert!(!auto_collapse_supported(27));
        assert!(!auto_collapse_supported(28));
    }

    /// Re-opening the running app collapses an expanded island (the way out when macOS
    /// hides it), once; a collapsed one stays as it is.
    #[test]
    fn reopening_collapses_once() {
        let mut s = Settings {
            display_mode: Some(DisplayMode::Expanded),
            ..Default::default()
        };
        assert_eq!(reopen(&mut s), Reopen::Collapsed);
        assert_eq!(s.display_mode(), DisplayMode::Collapsed);
        assert!(s.collapsed_for_space, "the menu says why");
        assert_eq!(reopen(&mut s), Reopen::AlreadyCollapsed);
        assert_eq!(s.display_mode(), DisplayMode::Collapsed);
        // A new installation starts collapsed: re-opening changes nothing.
        let mut fresh = Settings::default();
        assert_eq!(reopen(&mut fresh), Reopen::AlreadyCollapsed);
        assert!(!fresh.collapsed_for_space);
    }
}
