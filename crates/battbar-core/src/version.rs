// SPDX-License-Identifier: GPL-3.0-or-later
//! The version of this build, including a pre-release suffix (`0.5.0-rc.1`).
//!
//! `CFBundleShortVersionString` stays numeric (`0.5.0`, what macOS shows); the full
//! version is compiled in from the release tag (build.rs, `BATTBAR_RELEASE_VERSION`)
//! and written to the `BattbarVersion` key of the app's Info.plist. Menus, `--version`,
//! `battbar dump` and the update check use it.

/// `X.Y.Z` or `X.Y.Z-rc.N`.
pub const FULL: &str = env!("BATTBAR_FULL_VERSION");

/// The numeric part (the crate version).
pub const BASE: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_version_extends_the_crate_version() {
        assert!(FULL == BASE || FULL.starts_with(&format!("{BASE}-rc.")), "{FULL} vs {BASE}");
    }
}
