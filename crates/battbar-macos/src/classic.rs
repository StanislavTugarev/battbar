// SPDX-License-Identifier: GPL-3.0-or-later
//! Paired Bluetooth devices via IOBluetooth and the battery values macOS keeps for
//! classic audio devices.
//!
//! `batteryPercent*` are private IOBluetoothDevice properties. Every access is guarded
//! by `respondsToSelector:` and goes through KVC, so a future macOS that removes them
//! yields "not reported" instead of a crash.
//!
//! They are valid only for about 1–2 s after a process first uses IOBluetooth; after
//! that they read 0 for the rest of the process, whatever the thread or run loop
//! (measured 2026-11-20, macOS 26: 59 % on the first reads, 0 from +2 s on, while a
//! new process read 59–60 % every time). Long-running processes therefore read them in
//! a short-lived helper process: [`paired_devices_fresh`].

use objc2::runtime::Sel;
use objc2::{msg_send, sel};
use objc2_foundation::{NSNumber, NSObjectNSKeyValueCoding, NSString};
use objc2_io_bluetooth::IOBluetoothDevice;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// Development diagnostics: description of the calling thread's run loop (sources,
/// observers, timers), as printed by `CFCopyDescription`.
pub fn current_run_loop_description() -> String {
    use std::ffi::c_void;
    extern "C" {
        fn CFRunLoopGetCurrent() -> *const c_void;
        fn CFCopyDescription(cf: *const c_void) -> *mut c_void;
    }
    // SAFETY: CFCopyDescription returns a +1 CFStringRef, toll-free bridged to NSString.
    unsafe {
        let s = CFCopyDescription(CFRunLoopGetCurrent()) as *mut NSString;
        objc2::rc::Retained::from_raw(s).map(|s| s.to_string()).unwrap_or_default()
    }
}

/// Development diagnostics: service the calling thread's run loop for `seconds`.
pub fn run_current_run_loop(seconds: f64) {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: u8) -> i32;
    }
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, 0) };
}

/// Hidden argument of the `battbar` CLI that prints [`paired_devices`] as JSON and exits
/// (see [`print_paired_devices_json`]).
pub const HELPER_ARG: &str = "classic-batteries";
const HELPER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Helper side: print the paired devices as JSON on stdout. Prints `[]` without
/// Bluetooth permission (IOBluetooth would block while a prompt is open). Touches no
/// settings and no other device.
pub fn print_paired_devices_json() {
    let v = if crate::ble::is_authorized() { paired_devices() } else { vec![] };
    println!("{}", serde_json::to_string(&v).unwrap_or_else(|_| "[]".into()));
}

/// The `battbar` CLI next to the running executable (inside the app bundle:
/// `Contents/MacOS/battbar`; in a development build: `target/<profile>/battbar`).
pub fn helper_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("battbar");
    p.is_file().then_some(p)
}

/// Run the helper and parse its output.
pub fn paired_devices_from_helper(helper: &std::path::Path) -> Result<Vec<PairedDevice>, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(helper)
        .arg(HELPER_ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("helper: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("helper: no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let deadline = std::time::Instant::now() + HELPER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => break,
            Ok(Some(st)) => return Err(format!("helper exited with {st}")),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("helper timed out".into());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(e) => return Err(format!("helper: {e}")),
        }
    }
    let out = reader.join().map_err(|_| "helper: reader failed")?;
    parse_helper_output(&out)
}

pub fn parse_helper_output(out: &str) -> Result<Vec<PairedDevice>, String> {
    serde_json::from_str(out.trim()).map_err(|e| format!("helper output: {e}"))
}

/// Paired devices with current battery values: read by the helper process, or in this
/// process if there is no helper (then battery values are right only on the first read
/// of the process).
pub fn paired_devices_fresh() -> Vec<PairedDevice> {
    match helper_path().map(|h| paired_devices_from_helper(&h)) {
        Some(Ok(v)) => v,
        _ => paired_devices(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_output_roundtrip() {
        let d = PairedDevice {
            name: "Headset".into(),
            address: "aa-bb".into(),
            connected: true,
            class_of_device: 0x240418,
            battery_single: Some(60),
            battery_left: None,
            battery_right: Some(0),
            battery_case: None,
        };
        let text = serde_json::to_string(&vec![d.clone()]).unwrap();
        assert_eq!(parse_helper_output(&format!("{text}\n")).unwrap(), vec![d]);
        assert_eq!(parse_helper_output("[]").unwrap(), vec![]);
        assert!(parse_helper_output("battbar: error").is_err());
    }

    #[test]
    fn helper_failures_are_errors_not_hangs() {
        let t = std::time::Instant::now();
        assert!(paired_devices_from_helper(std::path::Path::new("/nonexistent/battbar")).is_err());
        // A helper that fails or prints garbage.
        assert!(paired_devices_from_helper(std::path::Path::new("/usr/bin/false")).is_err());
        assert!(paired_devices_from_helper(std::path::Path::new("/bin/echo")).is_err());
        assert!(t.elapsed() < HELPER_TIMEOUT);
    }
}
