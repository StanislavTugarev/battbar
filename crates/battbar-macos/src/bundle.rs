// SPDX-License-Identifier: GPL-3.0-or-later
//! Facts about the running bundle.

use objc2_foundation::NSBundle;

pub fn identifier() -> Option<String> {
    NSBundle::mainBundle().bundleIdentifier().map(|s| s.to_string())
}

pub fn path() -> String {
    NSBundle::mainBundle().bundlePath().to_string()
}

/// Development build: bundle ID `….dev`, or any debug build.
pub fn is_dev_build() -> bool {
    cfg!(debug_assertions) || identifier().is_some_and(|id| id.ends_with(".dev"))
}

/// macOS runs a quarantined app that was not moved by the user from a randomised
/// read-only copy (App Translocation). Login items registered from there would point
/// to a temporary path, so first-run questions about them are postponed.
pub fn is_translocated() -> bool {
    path().contains("/AppTranslocation/")
}
