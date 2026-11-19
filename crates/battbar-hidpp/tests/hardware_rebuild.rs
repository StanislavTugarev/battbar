// SPDX-License-Identifier: GPL-3.0-or-later
//! Hardware regression (run with `cargo test -- --ignored` on a Mac with HID devices):
//! the app rebuilds its provider threads when a setting changes (e.g. the headsetcontrol
//! switch). The thread that first initialised hidapi then exits; enumerating again used
//! to crash in CFRunLoopAddSource (battbar 0.1.0): the new threads enumerate while the
//! run loop of the exiting initialiser thread is being torn down. Read-only HID++
//! requests only.

use battbar_core::{DeviceDb, Engine};
use std::sync::Arc;
use std::time::Duration;

#[test]
#[ignore = "needs HID hardware"]
fn rebuilding_providers_does_not_crash() {
    let db = Arc::new(DeviceDb::embedded().unwrap());
    let mut engine = new_engine(&db);
    for round in 0..60 {
        let r = engine.poll(Duration::from_secs(10));
        eprintln!("round {round}: {} reports", r.reports.len());
        // Settings change in the app: drop the providers and poll at once with new ones,
        // while the old threads are still closing devices and exiting.
        engine = new_engine(&db);
    }
}

fn new_engine(db: &Arc<DeviceDb>) -> Engine {
    let hidpp = battbar_hidpp::HidppProvider::new(db.clone(), vec![Box::new(battbar_hidpp::HidapiLinks::default())]);
    let ss = battbar_steelseries::SteelSeriesProvider::new(db.clone(), vec![Box::new(battbar_steelseries::HidapiBases)]);
    Engine::new(vec![Box::new(hidpp), Box::new(ss)])
}
