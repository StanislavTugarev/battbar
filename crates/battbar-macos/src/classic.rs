// SPDX-License-Identifier: GPL-3.0-or-later
//! Paired Bluetooth devices via IOBluetooth and the battery values macOS keeps for
//! classic audio devices.
//!
//! `batteryPercent*` are private IOBluetoothDevice properties. Every access is guarded
//! by `respondsToSelector:` and goes through KVC, so a future macOS that removes them
//! yields "not reported" instead of a crash.

use objc2::runtime::Sel;
use objc2::{msg_send, sel};
use objc2_foundation::{NSNumber, NSObjectNSKeyValueCoding, NSString};
use objc2_io_bluetooth::IOBluetoothDevice;

#[derive(Debug, Clone, PartialEq)]
pub struct PairedDevice {
    pub name: String,
    /// Bluetooth address; callers hash it before showing or storing it.
    pub address: String,
    pub connected: bool,
    /// Class of Device; 0 for Bluetooth LE devices.
    pub class_of_device: u32,
    /// `batteryPercentSingle`, `Left`, `Right`, `Case` — `None` if the property does
    /// not exist on this macOS, `Some(0)` if it exists but is not reported.
    pub battery_single: Option<u8>,
    pub battery_left: Option<u8>,
    pub battery_right: Option<u8>,
    pub battery_case: Option<u8>,
}

impl PairedDevice {
    pub fn is_classic(&self) -> bool {
        self.class_of_device != 0
    }

    /// Major device class 0x04 = Audio/Video.
    pub fn is_audio(&self) -> bool {
        (self.class_of_device >> 8) & 0x1F == 0x04
    }
}

fn guarded_percent(dev: &IOBluetoothDevice, selector: Sel, key: &str) -> Option<u8> {
    let responds: bool = unsafe { msg_send![dev, respondsToSelector: selector] };
    if !responds {
        return None;
    }
    let v = dev.valueForKey(&NSString::from_str(key))?;
    let n = v.downcast::<NSNumber>().ok()?;
    Some(n.as_i64().clamp(0, 100) as u8)
}

/// All devices paired with this Mac (classic and LE). Requires Bluetooth permission.
pub fn paired_devices() -> Vec<PairedDevice> {
    let Some(list) = (unsafe { IOBluetoothDevice::pairedDevices() }) else {
        return vec![];
    };
    let mut out = vec![];
    for obj in list.iter() {
        let Ok(dev) = obj.downcast::<IOBluetoothDevice>() else { continue };
        out.push(PairedDevice {
            name: unsafe { dev.name() }.to_string(),
            address: unsafe { dev.addressString() }.map(|s| s.to_string()).unwrap_or_default(),
            connected: unsafe { dev.isConnected() },
            class_of_device: unsafe { dev.classOfDevice() },
            battery_single: guarded_percent(&dev, sel!(batteryPercentSingle), "batteryPercentSingle"),
            battery_left: guarded_percent(&dev, sel!(batteryPercentLeft), "batteryPercentLeft"),
            battery_right: guarded_percent(&dev, sel!(batteryPercentRight), "batteryPercentRight"),
            battery_case: guarded_percent(&dev, sel!(batteryPercentCase), "batteryPercentCase"),
        });
    }
    out
}
