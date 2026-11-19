// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional fallback: the external `headsetcontrol` tool (if installed) for models
//! without native support. Runs `headsetcontrol -b -o json` with a timeout.

use battbar_core::{DeviceKind, DeviceReport, Precision, Reading, Status, Transport};
use serde::Deserialize;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
struct Output {
    #[serde(default)]
    devices: Vec<Dev>,
}

#[derive(Debug, Deserialize)]
struct Dev {
    #[serde(default)]
    device: String,
    #[serde(default)]
    id_vendor: String,
    #[serde(default)]
    id_product: String,
    #[serde(default)]
    battery: Option<Battery>,
}

#[derive(Debug, Deserialize)]
struct Battery {
    #[serde(default)]
    status: String,
    #[serde(default)]
    level: i64,
}

fn parse_id(s: &str) -> u16 {
    u16::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0)
}

/// Parse `headsetcontrol -b -o json`; `skip(vid, pid)` excludes natively supported models.
pub fn parse(json: &str, skip: impl Fn(u16, u16) -> bool) -> Vec<DeviceReport> {
    let Ok(o) = serde_json::from_str::<Output>(json) else {
        return vec![];
    };
    o.devices
        .into_iter()
        .filter(|d| !skip(parse_id(&d.id_vendor), parse_id(&d.id_product)))
        .map(|d| {
            let reading = match &d.battery {
                Some(b) if b.status == "BATTERY_AVAILABLE" && b.level >= 0 => Reading::ok(b.level.clamp(0, 100) as u8, Precision::Coarse),
                Some(b) if b.status == "BATTERY_CHARGING" => {
                    let mut r = Reading::ok(b.level.clamp(0, 100) as u8, Precision::Coarse);
                    r.charging = Some(true);
                    if b.level < 0 {
                        r.percent = None;
                        r.status = Status::NotReported;
                    }
                    r
                }
                Some(b) if b.status == "BATTERY_UNAVAILABLE" => Reading::with_status(Status::Disconnected),
                _ => Reading::with_status(Status::NotReported),
            };
            DeviceReport {
                provider: "headsetcontrol".into(),
                source_key: format!("{}:{}", d.id_vendor, d.id_product),
                source: "headsetcontrol".into(),
                name: d.device,
                kind: DeviceKind::Headset,
                transport: Transport::BaseStation,
                identity: vec![],
                hints: vec![],
                reading,
                components: vec![],
            }
        })
        .collect()
}

/// Run the tool if it is on PATH. `None` if not installed.
pub fn run(timeout: Duration) -> Option<String> {
    let mut child = Command::new("headsetcontrol")
        .args(["-b", "-o", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
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
    let out = child.wait_with_output().ok()?;
    String::from_utf8(out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"name":"HeadsetControl","version":"3.0.0","device_count":2,"devices":[
      {"status":"success","device":"SteelSeries Arctis 7","vendor":"SteelSeries","product":"Arctis 7","id_vendor":"0x1038","id_product":"0x12ad",
       "battery":{"status":"BATTERY_AVAILABLE","level":75}},
      {"status":"success","device":"SteelSeries Arctis Nova Pro Wireless","id_vendor":"0x1038","id_product":"0x12e0",
       "battery":{"status":"BATTERY_AVAILABLE","level":100}}]}"#;

    #[test]
    fn parses_and_skips_native_models() {
        let r = parse(SAMPLE, |v, p| v == 0x1038 && p == 0x12e0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "SteelSeries Arctis 7");
        assert_eq!(r[0].reading.percent, Some(75));
        assert_eq!(r[0].reading.precision, Precision::Coarse);
    }

    #[test]
    fn garbage_is_ignored() {
        assert!(parse("not json", |_, _| false).is_empty());
    }
}
