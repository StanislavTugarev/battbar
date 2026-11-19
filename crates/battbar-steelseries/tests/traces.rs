// SPDX-License-Identifier: GPL-3.0-or-later
//! SteelSeries provider tests on traces recorded on the reference hardware.

use battbar_core::transport::Trace;
use battbar_core::*;
use battbar_steelseries::{ReplayBases, SteelSeriesProvider};
use std::path::PathBuf;
use std::sync::Arc;

fn poll(name: &str) -> Vec<DeviceReport> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../traces").join(name);
    let t = Trace::from_file(&p).unwrap();
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let mut prov = SteelSeriesProvider::new(db.clone(), vec![Box::new(ReplayBases::new(&t, &db))]);
    prov.set_headsetcontrol(false);
    prov.poll()
}

#[test]
fn nova_pro_base_reference_reading() {
    let r = poll("lightspeed-g502xplus-g915away_base-novapro.jsonl");
    assert_eq!(r.len(), 1);
    let h = &r[0];
    assert_eq!(h.name, "Arctis Nova Pro Wireless");
    assert_eq!(h.transport, Transport::BaseStation);
    // Base display: ~85–100 %; base reports 8/8.
    assert_eq!(h.reading.percent, Some(100));
    assert_eq!(h.reading.precision, Precision::Coarse);
    let spare = h.components.iter().find(|c| c.kind == ComponentKind::Spare).expect("spare");
    assert!(spare.reading.percent.is_some());
    // Headset is also on Bluetooth at recording time: companion hint present.
    assert!(matches!(h.hints.first(), Some(MergeHint::BluetoothCompanion { .. })));
}
