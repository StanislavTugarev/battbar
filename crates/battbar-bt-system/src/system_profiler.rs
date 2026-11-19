// SPDX-License-Identifier: GPL-3.0-or-later
//! `system_profiler SPBluetoothDataType -json` — fallback source for Bluetooth LE
//! battery values and device type metadata. Never used for classic audio (it does not
//! carry their battery, docs/DISCOVERY.md §4.5).

use battbar_core::DeviceKind;
use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct SpDevice {
    pub name: String,
    pub connected: bool,
    pub minor_type: Option<String>,
    pub ble: bool,
    pub battery_main: Option<u8>,
    pub battery_left: Option<u8>,
    pub battery_right: Option<u8>,
    pub battery_case: Option<u8>,
}

impl SpDevice {
    pub fn kind(&self) -> DeviceKind {
        kind_from_minor(self.minor_type.as_deref())
    }
}

pub fn kind_from_minor(minor: Option<&str>) -> DeviceKind {
    match minor.unwrap_or("").to_lowercase().as_str() {
        "keyboard" => DeviceKind::Keyboard,
        "mouse" | "trackpad" | "pointing device" => DeviceKind::Mouse,
        "headphones" => DeviceKind::Headphones,
        "headset" => DeviceKind::Headset,
        "gamepad" | "joystick" => DeviceKind::Gamepad,
        _ => DeviceKind::Other,
    }
}

fn pct(v: Option<&Value>) -> Option<u8> {
    v?.as_str()?.trim_end_matches('%').trim().parse::<u8>().ok()
}

pub fn parse(json: &str) -> Vec<SpDevice> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return vec![];
    };
    let mut out = vec![];
    for ctrl in v["SPBluetoothDataType"].as_array().into_iter().flatten() {
        for (key, connected) in [("device_connected", true), ("device_not_connected", false)] {
            for item in ctrl[key].as_array().into_iter().flatten() {
                for (name, p) in item.as_object().into_iter().flatten() {
                    out.push(SpDevice {
                        name: name.clone(),
                        connected,
                        minor_type: p["device_minorType"].as_str().map(String::from),
                        ble: p["device_services"].as_str().is_some_and(|s| s.contains("BLE")),
                        battery_main: pct(p.get("device_batteryLevelMain")),
                        battery_left: pct(p.get("device_batteryLevelLeft")),
                        battery_right: pct(p.get("device_batteryLevelRight")),
                        battery_case: pct(p.get("device_batteryLevelCase")),
                    });
                }
            }
        }
    }
    out
}

pub fn run(timeout: Duration) -> Option<String> {
    let mut child = Command::new("/usr/sbin/system_profiler")
        .args(["SPBluetoothDataType", "-json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut s).ok().map(|_| s)
    });
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    reader.join().ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shape as captured in docs/discovery/bt-headset.txt (addresses removed).
    const SAMPLE: &str = r#"{"SPBluetoothDataType":[{"controller_properties":{},
      "device_connected":[
        {"Arctis Nova Pro Wireless":{"device_minorType":"Headphones","device_services":"0x800019 < HFP AVRCP A2DP ACL >","device_vendorID":"0x000A","device_productID":"0xFFFF"}},
        {"G915 KEYBOARD":{"device_batteryLevelMain":"100%","device_minorType":"Keyboard","device_productID":"0xB354","device_services":"0x400000 < BLE >","device_vendorID":"0x046D"}},
        {"Buds":{"device_batteryLevelLeft":"80%","device_batteryLevelRight":"70%","device_batteryLevelCase":"40%","device_minorType":"Headphones","device_services":"< BLE >"}}],
      "device_not_connected":[{"Pebble K380s":{"device_minorType":"Keyboard"}}]}]}"#;

    #[test]
    fn parses_reference_capture() {
        let d = parse(SAMPLE);
        assert_eq!(d.len(), 4);
        let kb = d.iter().find(|x| x.name == "G915 KEYBOARD").unwrap();
        assert!(kb.connected && kb.ble);
        assert_eq!(kb.battery_main, Some(100));
        assert_eq!(kb.kind(), DeviceKind::Keyboard);
        let hs = d.iter().find(|x| x.name.starts_with("Arctis")).unwrap();
        assert_eq!(hs.battery_main, None);
        assert!(!hs.ble);
        let buds = d.iter().find(|x| x.name == "Buds").unwrap();
        assert_eq!(
            (buds.battery_left, buds.battery_right, buds.battery_case),
            (Some(80), Some(70), Some(40))
        );
        assert!(!d.iter().find(|x| x.name.starts_with("Pebble")).unwrap().connected);
    }
}
