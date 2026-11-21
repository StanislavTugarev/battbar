// SPDX-License-Identifier: GPL-3.0-or-later
//! Thin macOS bindings used by the battbar providers:
//! * `ble` — CoreBluetooth central limited to already-connected peripherals (no scanning);
//! * `classic` — IOBluetooth paired devices and guarded battery properties;
//! * `tcc` — permission helpers (usage description, responsible-process relaunch).

#[cfg(target_os = "macos")]
pub mod ble;
#[cfg(target_os = "macos")]
pub mod bundle;
#[cfg(target_os = "macos")]
pub mod classic;
#[cfg(target_os = "macos")]
pub mod connections;
#[cfg(target_os = "macos")]
pub mod tcc;
