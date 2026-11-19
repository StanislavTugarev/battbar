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

/// Cold start, no identity memory: in the first poll the BLE Battery Service already
/// answers but the GATT HID++ link is not open yet, so the keyboard looks like two
/// devices (receiver slot + BLE). The new-device gate must not show the BLE-only one;
/// in the follow-up poll the GATT link merges everything into one keyboard.
#[test]
fn cold_start_bas_before_gatt_never_shows_two_keyboards() {
    use std::time::{Duration, Instant};
    let ble = IdentityKey::Stable {
        namespace: "ble".into(),
        value: "p1".into(),
    };
    let receiver_kb: DeviceReport = poll(&[USB]).into_iter().find(|r| r.kind == DeviceKind::Keyboard).unwrap();
    let mut gatt_kb: DeviceReport = poll(&[BLE]).into_iter().find(|r| r.transport == Transport::BluetoothLe).unwrap();
    gatt_kb.identity.push(ble.clone());
    let mut bas = gatt_kb.clone();
    bas.provider = "bt-system".into();
    bas.source_key = "ble:p1".into();
    bas.name = "G915 KEYBOARD".into();
    bas.identity = vec![ble];
    bas.reading = Reading::ok(100, Precision::Exact);

    let mut gate = NewDeviceGate::default();
    let mut memory = IdentityMemory::default();
    let mut shown_ever: Vec<String> = vec![];
    let t0 = Instant::now();
    let polls = [
        (t0, vec![receiver_kb.clone(), bas.clone()]),
        (t0 + Duration::from_secs(4), vec![receiver_kb.clone(), bas.clone(), gatt_kb.clone()]),
        (
            t0 + Duration::from_secs(64),
            vec![receiver_kb.clone(), bas.clone(), gatt_kb.clone()],
        ),
    ];
    for (now, mut reports) in polls {
        memory.learn(&reports);
        memory.augment(&mut reports);
        let devices = merge(reports, &MergeConfig::default());
        for id in gate.admit(now, &devices, true) {
            if !shown_ever.contains(&id) {
                shown_ever.push(id);
            }
        }
    }
    assert_eq!(shown_ever.len(), 1, "{shown_ever:?}");
    assert!(shown_ever[0].starts_with("hidpp:"));
}

#[test]
fn stored_identity_memory_merges_without_gatt_link() {
    // Warm start: memory from a previous session; GATT link not open yet.
    let ble = IdentityKey::Stable {
        namespace: "ble".into(),
        value: "p1".into(),
    };
    let receiver_kb: DeviceReport = poll(&[USB]).into_iter().find(|r| r.kind == DeviceKind::Keyboard).unwrap();
    let mut gatt_kb = receiver_kb.clone();
    gatt_kb.identity.push(ble.clone());
    let mut m = IdentityMemory::default();
    m.learn(&[gatt_kb]);
    let stored = m.to_stored();
    assert!(
        stored
            .iter()
            .flatten()
            .all(|h| h.len() == 12 && h.chars().all(|c| c.is_ascii_hexdigit())),
        "only hashes are stored"
    );

    let mut bas = receiver_kb.clone();
    bas.provider = "bt-system".into();
    bas.source_key = "ble:p1".into();
    bas.identity = vec![ble];
    bas.reading = Reading::ok(100, Precision::Exact);
    let mut reports = vec![receiver_kb, bas];
    IdentityMemory::from_stored(&stored).augment(&mut reports);
    assert_eq!(merge(reports, &MergeConfig::default()).len(), 1);
}

/// Published traces must not contain real device identifiers: every unit ID the
/// provider extracts (receiver pairing register or 0x0003) must be a pseudonym, and
/// nothing that looks like a Bluetooth address may appear.
#[test]
fn repository_traces_contain_only_pseudonyms() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../traces");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let mac = |s: &str| {
        let b = s.as_bytes();
        (0..b.len().saturating_sub(16)).any(|i| {
            (0..6).all(|k| b[i + 3 * k].is_ascii_hexdigit() && b[i + 3 * k + 1].is_ascii_hexdigit())
                && (0..5).all(|k| b[i + 3 * k + 2] == b':' || b[i + 3 * k + 2] == b'-')
        })
    };
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(!mac(&text), "{name}: looks like it contains a Bluetooth address");
        assert!(!text.to_lowercase().contains("serial"), "{name}: mentions a serial number");
        // Every unit ID in the frames (including devices without battery reports).
        let t = trace(&name);
        for u in battbar_hidpp::trace_ids::unit_ids_in_trace(&t) {
            assert_eq!(&u[..2], &[0xbb, 0xa7], "{name}: unit ID {u:02x?} in the frames is not a pseudonym");
        }
        for r in poll(&[&name]) {
            for id in &r.identity {
                if let IdentityKey::HidppUnit { unit_id } = id {
                    assert!(
                        unit_id.starts_with(battbar_core::transport::PSEUDONYM_PREFIX),
                        "{name}: unit ID {unit_id} is not a pseudonym; re-record with `battbar record`"
                    );
                }
            }
        }
    }
}

/// Synthetic: a HID++ 2.0 device that only has 0x1001 and reports 2.9 V — far below
/// any Li-ion level (AA/AAA cells). Must not be mapped to 0 %.
#[test]
fn voltage_below_li_ion_range_is_not_reported_never_zero() {
    let r = poll(&["synthetic-receiver-aa-battery-0x1001.jsonl"]);
    assert_eq!(r.len(), 1, "{r:#?}");
    let m = &r[0];
    assert_eq!(m.name, "AA MOUSE");
    assert_eq!(m.kind, DeviceKind::Mouse);
    assert_eq!(m.reading.status, Status::NotReported);
    assert_eq!(m.reading.percent, None, "never 0 %");
    assert_eq!(m.reading.voltage_mv, Some(2900));
    assert_eq!(m.reading.note, Some(ReadingNote::NonRechargeableBattery));
    let mut d = merge(r.clone(), &MergeConfig::default());
    assert_eq!(d[0].reading.note, Some(ReadingNote::NonRechargeableBattery));
    assert_eq!(d[0].reading.percent, None);
    // 0.2.0: estimated from the default alkaline curve, 2 cells × 1.45 V, experimental.
    let db = DeviceDb::embedded().unwrap();
    battbar_core::cells::apply_chemistry(&mut d, &db, |_| None);
    assert_eq!(d[0].reading.percent, Some(85));
    assert_eq!(d[0].reading.precision, Precision::Approximate);
    assert_eq!(d[0].reading.battery.unwrap().rechargeable, Some(false));
    assert_eq!(d[0].reading.battery.unwrap().chemistry, Some(Chemistry::Alkaline));
    assert_eq!(d[0].reading.battery.unwrap().cells, Some(2));
}

#[test]
fn unified_battery_rechargeable_flag_hypothesis() {
    // G502 X PLUS: capabilities byte1 = 0x0f -> bit0 set (rechargeable, hypothesis).
    let r = poll(&[USB]);
    let mouse = r.iter().find(|r| r.kind == DeviceKind::Mouse).unwrap();
    assert_eq!(mouse.reading.battery.and_then(|b| b.rechargeable), Some(true));
}

/// Synthetic: a HID++ 1.0 device behind a receiver is listed as "not reported" without
/// garbage values, and does not break the poll.
#[test]
fn hidpp10_device_is_listed_without_values() {
    let r = poll(&["synthetic-receiver-hidpp10-device.jsonl"]);
    assert_eq!(r.len(), 1, "{r:#?}");
    let d = &r[0];
    assert_eq!(d.reading.status, Status::NotReported);
    assert_eq!(d.reading.percent, None);
    assert_eq!(d.reading.voltage_mv, None);
    assert_eq!(d.reading.note, Some(ReadingNote::Hidpp10Device));
    assert_eq!(d.name, "AA MOUSE"); // codename from the receiver
    assert_eq!(d.source, "HID++ 1.0");
}

/// Diagnostics never write: with a recording transport (`battbar record`) the provider is
/// read-only, so no HID++ 1.0 SET_REGISTER frame (sub-ID 0x80 / 0x82) is ever sent —
/// not even the notification-flag write a normal poll would make here (flags are off in
/// this trace) — while the HID++ 1.0 battery registers are read (GET_REGISTER 0x81).
#[test]
fn recording_is_read_only_and_reads_hidpp10_registers() {
    use battbar_core::transport::{Dir, TraceLine, TraceWriter};
    use std::io::Write;
    #[derive(Clone)]
    struct Buf(Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Buf(Arc::new(std::sync::Mutex::new(vec![])));
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let t = trace("synthetic-receiver-hidpp10-device.jsonl");
    let mut p =
        HidppProvider::new(db.clone(), vec![Box::new(ReplaySource::new(&t, &db))]).with_trace(TraceWriter::new(Box::new(buf.clone())));
    let reports = p.poll();
    let diag = p.diagnostics();
    assert_eq!(reports.len(), 1);

    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let recorded = Trace::parse(text.as_bytes()).unwrap();
    let mut out = 0;
    for l in &recorded.lines {
        if let TraceLine::Frame { dir: Dir::Out, hex, .. } = l {
            out += 1;
            let f = battbar_core::transport::from_hex(hex).unwrap();
            assert!(
                !battbar_hidpp::protocol::SET_REGISTER_SUB_IDS.contains(&f[2]),
                "register write recorded: {hex}"
            );
        }
    }
    assert!(out > 5, "nothing recorded");
    let regs = &diag["links"][0]["devices"][0]["hidpp10_registers"];
    assert_eq!(regs["0x07_battery_status"], "05 00 00", "{diag:#}");
    assert_eq!(regs["0x0d_battery_charge"], "32 00 00");
}

#[test]
fn normal_polling_still_enables_receiver_notifications() {
    // Contrast: the app (not read-only) writes the flags when they are off.
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let t = trace("synthetic-receiver-hidpp10-device.jsonl");
    let mut transports = t.transports();
    let tr = transports.pop().unwrap();
    let mut ch = battbar_hidpp::protocol::Channel::new(tr, false);
    assert_eq!(battbar_hidpp::receiver::ensure_notifications(&mut ch), Ok(true));
    let _ = db;
}
