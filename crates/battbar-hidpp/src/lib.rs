// SPDX-License-Identifier: GPL-3.0-or-later
//! Logitech HID++ 2.0 battery provider for every transport: receivers (Unifying,
//! Lightspeed, Bolt), direct USB, and Bluetooth LE through the vendor GATT service.
//!
//! Protocol knowledge partly ported from Solaar (GPL-2.0-or-later), see NOTICE.

pub mod device;
#[cfg(target_os = "macos")]
pub mod gatt;
pub mod hid_links;
pub mod protocol;
pub mod provider;
pub mod receiver;
pub mod trace_ids;

#[cfg(target_os = "macos")]
pub use gatt::GattLinks;
pub use hid_links::HidapiLinks;
pub use provider::{HidppProvider, LinkCandidate, LinkRole, LinkSource, ReplaySource};
