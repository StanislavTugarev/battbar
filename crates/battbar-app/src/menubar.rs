// SPDX-License-Identifier: GPL-3.0-or-later
//! Is the menu bar shown on the display of battbar's item? macOS hides it for a
//! full-screen app (video, game) or with "automatically hide the menu bar"; the item is
//! then not visible either, which is not "no room" (see `overflow`).
//!
//! macOS 27 leaves the item's window where it was and takes the display's menu bar
//! window (Window Server, main-menu level, at the display's top edge) off screen;
//! macOS 26 also moved the item's window above the display. Both are checked. Reads
//! only the level and bounds of on-screen windows (no window titles, no screen
//! recording permission). DISCOVERY "Full-screen apps".

use crate::overflow::Rect;

/// Rectangles in CoreGraphics global coordinates (origin top left of the main display,
/// y down).
#[derive(Debug, Default)]
pub struct Windows {
    pub displays: Vec<Rect>,
    /// On-screen windows at the main-menu level (the menu bars).
    pub menu_bars: Vec<Rect>,
}

/// Whether the display under `item_x` (from `x` to `x + w`, AppKit and CoreGraphics
/// share x) shows its menu bar: a main-menu-level window at its top edge, no taller
/// than a menu bar, covering at least half its width (a menu bar sliding with a space
/// change counts as hidden). `None` when no display is under the item (can't tell).
pub fn menu_bar_shown(item_x: f64, item_w: f64, w: &Windows) -> Option<bool> {
    let mid = item_x + item_w / 2.0;
    let d = w
        .displays
        .iter()
        .find(|d| mid >= d.x && mid < d.x + d.w)
        .or_else(|| w.displays.iter().find(|d| item_x < d.x + d.w && item_x + item_w > d.x))?;
    Some(w.menu_bars.iter().any(|b| {
        let overlap = (b.x + b.w).min(d.x + d.w) - b.x.max(d.x);
        (b.y - d.y).abs() <= 2.0 && b.h > 0.0 && b.h <= 60.0 && overlap >= d.w / 2.0
    }))
}

#[cfg(target_os = "macos")]
pub fn windows() -> Windows {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut c_void;
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGWindowLevelForKey(key: i32) -> i32;
    }
    const ON_SCREEN_ONLY: u32 = 1;
    const MAIN_MENU_LEVEL_KEY: i32 = 8;

    let mut out = Windows::default();
    let mut ids = [0u32; 16];
    let mut n = 0u32;
    // SAFETY: the buffer holds 16 IDs; `n` receives how many were written.
    if unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut n) } == 0 {
        for id in &ids[..(n as usize).min(16)] {
            let b = unsafe { CGDisplayBounds(*id) };
            out.displays.push(Rect {
                x: b.x,
                y: b.y,
                w: b.w,
                h: b.h,
            });
        }
    }
    let level = unsafe { CGWindowLevelForKey(MAIN_MENU_LEVEL_KEY) } as i64;
    // SAFETY: a "Copy" function: we own the returned CFArray of CFDictionary
    // (toll-free bridged to NSArray / NSDictionary); null on failure.
    let raw = unsafe { CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, 0) };
    let Some(list) = (unsafe { Retained::<NSArray<NSDictionary>>::from_raw(raw.cast()) }) else {
        return out;
    };
    let get = |d: &NSDictionary, k: &str| -> Option<Retained<AnyObject>> {
        let key = NSString::from_str(k);
        let key: &AnyObject = key.as_ref();
        d.objectForKey(key)
    };
    let num = |d: &NSDictionary, k: &str| -> Option<f64> { get(d, k).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.doubleValue()) };
    for w in list.iter() {
        if num(&w, "kCGWindowLayer").map(|l| l as i64) != Some(level) {
            continue;
        }
        let Some(b) = get(&w, "kCGWindowBounds").and_then(|v| v.downcast::<NSDictionary>().ok()) else {
            continue;
        };
        if let (Some(x), Some(y), Some(wd), Some(h)) = (num(&b, "X"), num(&b, "Y"), num(&b, "Width"), num(&b, "Height")) {
            out.menu_bars.push(Rect { x, y, w: wd, h });
        }
    }
    out
}

/// Diagnostics (`--probe-menu-bar`): what WindowServer knows about one window
/// (`None`: not in its list at all), and how many windows at the status-item level
/// exist (on screen / in total) — without window titles.
#[cfg(target_os = "macos")]
pub fn window_server_state(window_number: isize) -> (Option<String>, usize, usize) {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
    use std::ffi::c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut c_void;
        fn CGWindowLevelForKey(key: i32) -> i32;
    }
    const ALL: u32 = 0;
    const INCLUDING_WINDOW: u32 = 8;
    const STATUS_LEVEL_KEY: i32 = 9;
    let list = |opt: u32, rel: u32| unsafe { Retained::<NSArray<NSDictionary>>::from_raw(CGWindowListCopyWindowInfo(opt, rel).cast()) };
    let get = |d: &NSDictionary, k: &str| -> Option<Retained<AnyObject>> {
        let key = NSString::from_str(k);
        let key: &AnyObject = key.as_ref();
        d.objectForKey(key)
    };
    let num = |d: &NSDictionary, k: &str| get(d, k).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.doubleValue());
    let own = (window_number > 0)
        .then(|| list(INCLUDING_WINDOW, window_number as u32))
        .flatten()
        .and_then(|l| l.iter().next())
        .map(|d| {
            format!(
                "layer={} onscreen={} alpha={}",
                num(&d, "kCGWindowLayer").unwrap_or(-1.0),
                num(&d, "kCGWindowIsOnscreen").unwrap_or(0.0),
                num(&d, "kCGWindowAlpha").unwrap_or(-1.0)
            )
        });
    let status = unsafe { CGWindowLevelForKey(STATUS_LEVEL_KEY) } as f64;
    let (mut on, mut total) = (0, 0);
    if let Some(l) = list(ALL, 0) {
        for d in l.iter() {
            if num(&d, "kCGWindowLayer") == Some(status) {
                total += 1;
                if num(&d, "kCGWindowIsOnscreen") == Some(1.0) {
                    on += 1;
                }
            }
        }
    }
    (own, on, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    /// Window lists measured on macOS 27.2.1 (2026-11-27): main display 2560×1440,
    /// second display 1920×1080 to its right at y = 131 (CoreGraphics coordinates).
    #[test]
    fn menu_bar_windows_measured_on_macos_27() {
        let displays = vec![r(0.0, 0.0, 2560.0, 1440.0), r(2560.0, 131.0, 1920.0, 1080.0)];
        let item = (1985.0, 187.0); // battbar's item on the main display, all along
        let normal = Windows {
            displays: displays.clone(),
            menu_bars: vec![r(2560.0, 131.0, 1920.0, 30.0), r(0.0, 0.0, 2560.0, 30.0)],
        };
        assert_eq!(menu_bar_shown(item.0, item.1, &normal), Some(true));
        // Chrome full screen on the main display: its menu bar window is off screen.
        let full_screen = Windows {
            displays: displays.clone(),
            menu_bars: vec![r(2560.0, 131.0, 1920.0, 30.0)],
        };
        assert_eq!(menu_bar_shown(item.0, item.1, &full_screen), Some(false));
        // The space-change slide: the menu bar window moves sideways.
        let sliding = Windows {
            displays: displays.clone(),
            menu_bars: vec![r(2560.0, 131.0, 1920.0, 30.0), r(-1396.0, 0.0, 2560.0, 30.0)],
        };
        assert_eq!(menu_bar_shown(item.0, item.1, &sliding), Some(false));
        let almost = Windows {
            displays: displays.clone(),
            menu_bars: vec![r(2560.0, 131.0, 1920.0, 30.0), r(-8.0, 0.0, 2560.0, 30.0)],
        };
        assert_eq!(menu_bar_shown(item.0, item.1, &almost), Some(true));
        // The item on the second display: its own menu bar counts, not the main one's.
        assert_eq!(menu_bar_shown(3459.0, 136.0, &full_screen), Some(true));
        let second_full_screen = Windows {
            displays: displays.clone(),
            menu_bars: vec![r(0.0, 0.0, 2560.0, 30.0)],
        };
        assert_eq!(menu_bar_shown(3459.0, 136.0, &second_full_screen), Some(false));
        // No display under the item: can't tell.
        assert_eq!(menu_bar_shown(9000.0, 34.0, &normal), None);
    }
}
