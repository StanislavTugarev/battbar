// SPDX-License-Identifier: GPL-3.0-or-later
//! The only place in battbar that touches `hidapi`.
//!
//! Privacy guard (docs/SPEC.md, "Privacy and security"): an OS-level HID device is
//! opened only if it is on USB and **every** top-level collection is vendor-defined
//! (usage page >= 0xFF00). Keyboard, mouse, consumer and any Bluetooth HID device are
//! refused before `hidapi` is asked to open anything.

use battbar_core::transport::{FrameTransport, LinkInfo, TransportError};
use hidapi::{BusType, HidApi, HidDevice};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const VENDOR_USAGE_PAGE_MIN: u16 = 0xFF00;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Usb,
    Bluetooth,
    Other,
}

/// One OS-level HID device (on macOS: one `IOHIDDevice`) with all its collections.
#[derive(Debug, Clone, PartialEq)]
pub struct HidInterface {
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus: Bus,
    pub interface_number: i32,
    pub release: u16,
    pub product: String,
    pub manufacturer: String,
    pub path: CString,
    /// Top-level collections `(usage_page, usage)`.
    pub collections: Vec<(u16, u16)>,
}

impl HidInterface {
    pub fn is_vendor_only(&self) -> bool {
        !self.collections.is_empty() && self.collections.iter().all(|(p, _)| *p >= VENDOR_USAGE_PAGE_MIN)
    }

    pub fn has_usage_page(&self, page: u16) -> bool {
        self.collections.iter().any(|(p, _)| *p == page)
    }

    /// Stable link ID within this machine session (no serials).
    pub fn link_id(&self) -> String {
        format!(
            "hid:{:04x}:{:04x}:{}:{}",
            self.vendor_id,
            self.product_id,
            self.interface_number,
            self.collections.first().map(|c| format!("{:04x}", c.0)).unwrap_or_default()
        )
    }

    pub fn link_info(&self) -> LinkInfo {
        LinkInfo {
            link: self.link_id(),
            kind: "hidapi".into(),
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            product: self.product.clone(),
            bus: match self.bus {
                Bus::Usb => "usb",
                Bus::Bluetooth => "bluetooth",
                Bus::Other => "other",
            }
            .into(),
            collections: self.collections.clone(),
        }
    }
}

/// Why an interface may not be opened. Pure function, unit-tested.
pub fn check_openable(i: &HidInterface) -> Result<(), String> {
    if i.bus == Bus::Bluetooth {
        return Err("Bluetooth HID devices are never opened (keyboard/mouse reports share the device)".into());
    }
    if i.bus != Bus::Usb {
        return Err("only USB HID devices are opened".into());
    }
    if i.collections.is_empty() {
        return Err("collections unknown".into());
    }
    if let Some((p, u)) = i.collections.iter().find(|(p, _)| *p < VENDOR_USAGE_PAGE_MIN) {
        return Err(format!("interface carries non-vendor collection 0x{p:04x}/0x{u:04x}"));
    }
    Ok(())
}

fn new_api() -> Result<HidApi, String> {
    HidApi::new()
        .inspect(|_a| {
            // hidapi opens devices exclusively on macOS by default; that would break
            // G HUB / GG and the vendor apps would break us.
            #[cfg(target_os = "macos")]
            _a.set_open_exclusive(false);
        })
        .map_err(|e| e.to_string())
}

fn api() -> Result<&'static Mutex<HidApi>, TransportError> {
    static API: OnceLock<Result<Mutex<HidApi>, String>> = OnceLock::new();
    API.get_or_init(|| {
        // On macOS hidapi schedules its IOHIDManager on the run loop of the thread that
        // creates it (`IOHIDManagerScheduleWithRunLoop(…, CFRunLoopGetCurrent(), …)`), and
        // every later enumeration schedules devices on that run loop. If that thread
        // exits — the app rebuilds its provider threads when a setting changes — the
        // run loop is freed and the next enumeration crashes in CFRunLoopAddSource.
        // So the manager is created on a dedicated thread that lives for the whole process.
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("battbar-hid-manager".into()).spawn(move || {
            let _ = tx.send(new_api());
            loop {
                std::thread::park();
            }
        });
        match spawned {
            Ok(_) => rx
                .recv()
                .unwrap_or_else(|_| Err("HID manager thread failed".into()))
                .map(Mutex::new),
            Err(e) => Err(e.to_string()),
        }
    })
    .as_ref()
    .map_err(|e| TransportError::Io(e.clone()))
}

/// All HID devices, grouped per OS-level device. Nothing is opened.
pub fn enumerate() -> Result<Vec<HidInterface>, TransportError> {
    let mut a = api()?.lock().unwrap();
    a.refresh_devices().map_err(|e| TransportError::Io(e.to_string()))?;
    let mut by_path: BTreeMap<CString, HidInterface> = BTreeMap::new();
    for d in a.device_list() {
        let e = by_path.entry(d.path().to_owned()).or_insert_with(|| HidInterface {
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            bus: match d.bus_type() {
                BusType::Usb => Bus::Usb,
                BusType::Bluetooth => Bus::Bluetooth,
                _ => Bus::Other,
            },
            interface_number: d.interface_number(),
            release: d.release_number(),
            product: d.product_string().unwrap_or("").to_string(),
            manufacturer: d.manufacturer_string().unwrap_or("").to_string(),
            path: d.path().to_owned(),
            collections: vec![],
        });
        e.collections.push((d.usage_page(), d.usage()));
    }
    Ok(by_path.into_values().filter(|i| i.vendor_id != 0).collect())
}

pub struct HidapiTransport {
    dev: HidDevice,
    info: LinkInfo,
    buf: Vec<u8>,
}

/// Opens a HID interface after the privacy guard approved it.
pub fn open(i: &HidInterface) -> Result<HidapiTransport, TransportError> {
    check_openable(i).map_err(TransportError::Refused)?;
    let a = api()?.lock().unwrap();
    let dev = a.open_path(&i.path).map_err(|e| TransportError::Io(e.to_string()))?;
    Ok(HidapiTransport {
        dev,
        info: i.link_info(),
        buf: vec![0u8; 256],
    })
}

impl FrameTransport for HidapiTransport {
    fn info(&self) -> &LinkInfo {
        &self.info
    }
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        self.dev.write(frame).map(|_| ()).map_err(|e| map_err(e.to_string()))
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        match self.dev.read_timeout(&mut self.buf, ms) {
            Ok(0) => Ok(None),
            Ok(n) => Ok(Some(self.buf[..n].to_vec())),
            Err(e) => Err(map_err(e.to_string())),
        }
    }
}

fn map_err(s: String) -> TransportError {
    let l = s.to_lowercase();
    if l.contains("disconnect") || l.contains("not connected") || l.contains("no such device") || l.contains("0xe00002c0") {
        TransportError::Disconnected
    } else {
        TransportError::Io(s)
    }
}

/// Input Monitoring (TCC "ListenEvent") status of this process: `granted`, `denied`
/// or `unknown` (never asked). battbar never requests it; this is reported in
/// diagnostics to show that reading vendor interfaces does not depend on it.
#[cfg(target_os = "macos")]
pub fn input_monitoring_status() -> &'static str {
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOHIDCheckAccess(request_type: u32) -> u32;
    }
    // kIOHIDRequestTypeListenEvent = 1; kIOHIDAccessTypeGranted = 0, Denied = 1, Unknown = 2.
    match unsafe { IOHIDCheckAccess(1) } {
        0 => "granted",
        1 => "denied",
        _ => "unknown",
    }
}

#[cfg(not(target_os = "macos"))]
pub fn input_monitoring_status() -> &'static str {
    "n/a"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(bus: Bus, collections: Vec<(u16, u16)>) -> HidInterface {
        HidInterface {
            vendor_id: 0x046d,
            product_id: 0xc547,
            bus,
            interface_number: 0,
            release: 0,
            product: "t".into(),
            manufacturer: "t".into(),
            // A path that does not exist: if the guard ever let it through, open would
            // fail with Io, not Refused, and the tests below would catch it.
            path: CString::new("DevSrvsID:0").unwrap(),
            collections,
        }
    }

    fn refused(i: &HidInterface) -> bool {
        matches!(open(i), Err(TransportError::Refused(_)))
    }

    #[test]
    fn keyboard_and_mouse_usage_pages_are_never_opened() {
        assert!(refused(&iface(Bus::Usb, vec![(0x01, 0x06)])), "keyboard");
        assert!(refused(&iface(Bus::Usb, vec![(0x01, 0x02)])), "mouse");
        assert!(refused(&iface(Bus::Usb, vec![(0x07, 0x00)])), "keyboard page");
        assert!(refused(&iface(Bus::Usb, vec![(0x0c, 0x01)])), "consumer");
    }

    #[test]
    fn mixed_interfaces_are_never_opened() {
        assert!(refused(&iface(Bus::Usb, vec![(0xff00, 0x01), (0x01, 0x06)])));
        assert!(refused(&iface(Bus::Usb, vec![(0x01, 0x80), (0xff43, 0x0202)])));
    }

    #[test]
    fn bluetooth_hid_devices_are_never_opened() {
        // Even a vendor-only Bluetooth HID device: on macOS a BT peripheral is one
        // IOHIDDevice and HID++ goes over GATT instead.
        assert!(refused(&iface(Bus::Bluetooth, vec![(0xff43, 0x0202)])));
        assert!(refused(&iface(Bus::Bluetooth, vec![(0x01, 0x06), (0xff43, 0x0202)])));
    }

    #[test]
    fn unknown_collections_are_never_opened() {
        assert!(refused(&iface(Bus::Usb, vec![])));
        assert!(refused(&iface(Bus::Other, vec![(0xff00, 1)])));
    }

    #[test]
    fn vendor_only_usb_passes_the_guard() {
        assert!(check_openable(&iface(Bus::Usb, vec![(0xff00, 1), (0xff00, 2)])).is_ok());
        assert!(check_openable(&iface(Bus::Usb, vec![(0xffc0, 1), (0xff00, 1)])).is_ok());
    }
}
