// SPDX-License-Identifier: GPL-3.0-or-later
//! HID++ provider tests on traces recorded on the reference hardware (traces/).

use battbar_core::transport::Trace;
use battbar_core::*;
use battbar_hidpp::{HidppProvider, ReplaySource};
use std::path::PathBuf;
use std::sync::Arc;

pub fn trace(name: &str) -> Trace {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../traces").join(name);
    Trace::from_file(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

pub fn poll(names: &[&str]) -> Vec<DeviceReport> {
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let mut out = vec![];
    for n in names {
        let t = trace(n);
        let mut p = HidppProvider::new(db.clone(), vec![Box::new(ReplaySource::new(&t, &db))]);
        out.extend(p.poll());
    }
    out
}

const USB: &str = "lightspeed-g502xplus-g915away_base-novapro.jsonl";

#[test]
fn lightspeed_mouse_matches_g_hub() {
    let r = poll(&[USB]);
    let mouse = r.iter().find(|r| r.kind == DeviceKind::Mouse).expect("mouse");
    assert_eq!(mouse.name, "G502 X PLUS");
    assert_eq!(mouse.transport, Transport::Receiver);
    assert_eq!(mouse.source, "HID++ 0x1004");
    assert_eq!(mouse.reading.percent, Some(11)); // G HUB: 11 %
    assert_eq!(mouse.reading.precision, Precision::Exact);
    assert_eq!(mouse.reading.charging, Some(false));
    assert!(matches!(mouse.identity[0], IdentityKey::HidppUnit { .. }));
}

#[test]
fn keyboard_away_from_receiver_is_disconnected_with_identity() {
    let r = poll(&[USB]);
    let kb = r.iter().find(|r| r.kind == DeviceKind::Keyboard).expect("keyboard slot");
    assert_eq!(kb.reading.status, Status::Disconnected);
    assert_eq!(kb.reading.percent, None, "never 0 %");
    // Unit ID comes from the receiver's extended pairing register, so the keyboard can be
    // merged with the same unit seen over Bluetooth.
    assert!(matches!(kb.identity.first(), Some(IdentityKey::HidppUnit { .. })));
}

#[test]
fn non_hidpp_links_in_trace_are_ignored() {
    // The base station link (VID 1038) is in the same trace but is not HID++.
    let r = poll(&[USB]);
    assert!(r.iter().all(|r| r.provider == "hidpp"));
    assert_eq!(r.len(), 2);
}

const BLE: &str = "ble-g915-gatt_lightspeed-g502xplus_base-novapro-bt.jsonl";

#[test]
fn keyboard_over_gatt_reads_voltage() {
    let r = poll(&[BLE]);
    let kb = r
        .iter()
        .find(|r| r.transport == Transport::BluetoothLe)
        .expect("keyboard over GATT");
    assert_eq!(kb.kind, DeviceKind::Keyboard);
    assert_eq!(kb.source, "HID++ 0x1001");
    let mv = kb.reading.voltage_mv.expect("voltage");
    assert!((4100..=4200).contains(&mv), "{mv}");
    assert_eq!(kb.reading.precision, Precision::Approximate);
    // G HUB: 100 %; acceptance ±5 % (approximate curve until calibrated).
    assert!(kb.reading.percent.unwrap() >= 95);
    assert_eq!(kb.reading.charging, Some(false));
}

#[test]
fn keyboard_is_one_device_across_receiver_and_bluetooth() {
    // Receiver trace: keyboard paired but away. BLE trace: same keyboard over GATT.
    let mut reports: Vec<DeviceReport> = poll(&[USB]).into_iter().filter(|r| r.kind == DeviceKind::Keyboard).collect();
    reports.extend(poll(&[BLE]).into_iter().filter(|r| r.transport == Transport::BluetoothLe));
    assert_eq!(reports.len(), 2);
    let devices = merge(reports, &MergeConfig::default());
    assert_eq!(devices.len(), 1, "{devices:#?}");
    let d = &devices[0];
    assert_eq!(d.reading.status, Status::Ok);
    assert_eq!(d.live_transports(), vec![Transport::BluetoothLe]);
}

#[test]
fn ble_battery_service_beats_voltage_curve_but_keeps_flags() {
    let mut reports: Vec<DeviceReport> = poll(&[BLE]).into_iter().filter(|r| r.transport == Transport::BluetoothLe).collect();
    // What bt-system reports for the same peripheral (identity via the BLE peripheral ID).
    let shared = IdentityKey::Stable {
        namespace: "ble".into(),
        value: "p1".into(),
    };
    reports[0].identity.push(shared.clone());
    let mut bas = reports[0].clone();
    bas.provider = "bt-system".into();
    bas.source_key = "ble:p1".into();
    bas.source = "BLE Battery Service".into();
    bas.identity = vec![shared];
    bas.reading = Reading::ok(100, Precision::Exact);
    let d = &merge(vec![reports[0].clone(), bas], &MergeConfig::default())[0];
    assert_eq!(d.reading.percent, Some(100));
    assert_eq!(d.reading.precision, Precision::Exact);
    assert_eq!(d.reading.charging, Some(false));
    assert!(d.reading.voltage_mv.is_some());
}
