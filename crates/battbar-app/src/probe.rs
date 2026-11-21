// SPDX-License-Identifier: GPL-3.0-or-later
//! `battbar-app --probe-menu-bar [--width POINTS]` — discovery tool for automatic
//! collapsing: can an app tell that its status item is hidden by the system or behind
//! the camera housing (notch)?
//!
//! Shows one test status item ("probe") of the given width and prints one line every
//! time something relevant changes (checked every 0.5 s): the frontmost app, the item
//! window's frame, its occlusion state and visibility, and the screen with its menu
//! bar areas left and right of the notch. Reads no devices, no Bluetooth, no settings;
//! safe to start from a shell. Quit with Ctrl-C.
//!
//! It also subscribes to the item window's occlusion notifications (what the app uses)
//! and prints each as `EVENT`, so missing or late notifications show next to the
//! polled state. `--grow W --after S` widens the item to W points after S seconds.
//! (Polling here is deliberate: this is a diagnostic, not the app.)

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSScreen, NSStatusBar, NSStatusItem, NSWorkspace};
use objc2_foundation::{NSRect, NSString, NSTimer};
use std::cell::RefCell;
use std::io::Write;

fn r(rect: NSRect) -> String {
    format!(
        "({:.0},{:.0} {:.0}×{:.0})",
        rect.origin.x, rect.origin.y, rect.size.width, rect.size.height
    )
}

fn intersects(a: NSRect, b: NSRect) -> bool {
    a.size.width > 0.0
        && b.size.width > 0.0
        && a.origin.x < b.origin.x + b.size.width
        && b.origin.x < a.origin.x + a.size.width
        && a.origin.y < b.origin.y + b.size.height
        && b.origin.y < a.origin.y + a.size.height
}

/// One snapshot as a line (without the time stamp, so changes can be detected).
fn snapshot(mtm: MainThreadMarker, item: &NSStatusItem) -> String {
    let front = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|a| a.bundleIdentifier())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".into());
    let Some(win) = item.button(mtm).and_then(|b| b.window()) else {
        return format!("front={front} item=no-window");
    };
    let f = win.frame();
    let occlusion = win.occlusionState().0;
    let visible_bit = occlusion & 2 != 0;
    let mut s = format!(
        "front={front} item={} occlusion=0x{occlusion:x}({}) window_visible={} active_space={} item_visible={}",
        r(f),
        if visible_bit { "visible" } else { "NOT visible" },
        win.isVisible(),
        win.isOnActiveSpace(),
        item.isVisible()
    );
    match win.screen() {
        Some(scr) => {
            let (left, right) = (scr.auxiliaryTopLeftArea(), scr.auxiliaryTopRightArea());
            let sf = scr.frame();
            // The notch: between the two auxiliary areas (only on displays with one).
            let notch = (left.size.width > 0.0 && right.size.width > 0.0).then(|| {
                let x0 = left.origin.x + left.size.width;
                NSRect::new(
                    objc2_foundation::NSPoint::new(x0, left.origin.y),
                    objc2_foundation::NSSize::new(right.origin.x - x0, left.size.height),
                )
            });
            let inside = f.origin.x >= sf.origin.x && f.origin.x + f.size.width <= sf.origin.x + sf.size.width;
            s.push_str(&format!(
                " screen=\"{}\" scale={} frame={} visible_frame={} aux_left={} aux_right={} safe_top={:.0} notch={} overlaps_notch={} inside_screen={inside}",
                scr.localizedName(),
                scr.backingScaleFactor(),
                r(sf),
                r(scr.visibleFrame()),
                r(left),
                r(right),
                scr.safeAreaInsets().top,
                notch.map(r).unwrap_or_else(|| "none".into()),
                notch.is_some_and(|n| intersects(n, f)),
            ));
        }
        None => s.push_str(" screen=none"),
    }
    let screens = NSScreen::screens(mtm);
    s.push_str(&format!(" screens={}", screens.count()));
    s
}

pub fn run(width: f64, grow: Option<(f64, f64)>) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let item: Retained<NSStatusItem> = NSStatusBar::systemStatusBar().statusItemWithLength(width);
    item.setAutosaveName(Some(&NSString::from_str("battbar_probe")));
    if let Some(b) = item.button(mtm) {
        b.setTitle(&NSString::from_str("◀ probe ▶"));
    }
    println!("probe: status item width {width} pt; switch apps, screens and menus; Ctrl-C to quit");
    let last = RefCell::new(String::new());
    let start = std::time::Instant::now();
    let item_for_events = item.clone();
    let tick = RcBlock::new(move |_t: std::ptr::NonNull<NSTimer>| {
        let now = snapshot(mtm, &item);
        if *last.borrow() != now {
            println!("[{:>7.1}s] {now}", start.elapsed().as_secs_f64());
            let _ = std::io::stdout().flush();
            *last.borrow_mut() = now;
        }
    });
    let _timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.5, true, &tick) };
    // The app's signal: occlusion notifications of the item's window.
    let observer = item_for_events.button(mtm).and_then(|b| b.window()).map(|win| {
        let w2 = win.clone();
        let block = RcBlock::new(move |_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            let visible = w2.occlusionState().0 & 2 != 0;
            println!(
                "[{:>7.1}s] EVENT occlusion changed: visible={visible} frame={}",
                start.elapsed().as_secs_f64(),
                r(w2.frame())
            );
            let _ = std::io::stdout().flush();
        });
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(objc2_app_kit::NSWindowDidChangeOcclusionStateNotification),
                Some(&win),
                Some(&objc2_foundation::NSOperationQueue::mainQueue()),
                &block,
            )
        }
    });
    let _grow_timer = grow.map(|(w, after)| {
        let it = item_for_events.clone();
        let block = RcBlock::new(move |_t: std::ptr::NonNull<NSTimer>| {
            println!("[{:>7.1}s] GROW to {w} pt", start.elapsed().as_secs_f64());
            let _ = std::io::stdout().flush();
            it.setLength(w);
        });
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(after, false, &block) }
    });
    let _keep = observer;
    app.run();
}
