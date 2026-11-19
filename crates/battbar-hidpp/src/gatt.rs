// SPDX-License-Identifier: GPL-3.0-or-later
//! HID++ over the Logitech vendor GATT characteristic (Bluetooth LE, macOS).
//!
//! No HID device is opened on this path, so keyboard input can never be received.
//! Peripherals are obtained with `retrieveConnectedPeripheralsWithServices:` only.
//!
//! Framing (docs/DISCOVERY.md §4.3): 18-byte frames without report ID and device index:
//! `[feature index, function << 4 | sw id, 16 parameter bytes]`, written with response;
//! replies and notifications arrive as value notifications in the same layout. This
//! transport converts from/to canonical long HID++ reports `[0x11, 0xFF, …]`.

use crate::protocol::{LONG_LEN, RECEIVER_INDEX, REPORT_LONG};
use crate::provider::{BoxTransport, LinkCandidate, LinkRole, LinkSource};
use battbar_core::transport::{FrameTransport, LinkInfo, TransportError};
use battbar_core::{short_hash, DeviceDb, IdentityKey, Transport};
use battbar_macos::ble::{BleError, Central, Characteristic, GattSession};
use std::time::Duration;

pub const GATT_FRAME_LEN: usize = 18;
const DEVICE_INFORMATION: &str = "180A";
const PNP_ID: &str = "2A50";

/// Canonical HID++ report → GATT frame. Only device index 0xFF exists on this link.
pub fn to_gatt(frame: &[u8]) -> Result<Vec<u8>, TransportError> {
    if frame.len() < 4 || frame[1] != RECEIVER_INDEX {
        return Err(TransportError::Refused("HID++ over GATT supports device index 0xFF only".into()));
    }
    let mut f = frame[2..].to_vec();
    f.resize(GATT_FRAME_LEN, 0);
    Ok(f)
}

/// GATT frame → canonical long HID++ report.
pub fn from_gatt(v: &[u8]) -> Vec<u8> {
    let mut f = vec![REPORT_LONG, RECEIVER_INDEX];
    f.extend_from_slice(v);
    f.resize(LONG_LEN, 0);
    f
}

pub struct GattTransport {
    session: GattSession,
    ch: Characteristic,
    info: LinkInfo,
}

fn ble_err(e: BleError) -> TransportError {
    match e {
        BleError::Disconnected => TransportError::Disconnected,
        e => TransportError::Io(e.to_string()),
    }
}

impl GattTransport {
    pub fn open(
        central: &'static Central,
        p: &battbar_macos::ble::Peripheral,
        service: &str,
        characteristic: &str,
    ) -> Result<Self, TransportError> {
        let mut session = GattSession::open(central, p).map_err(ble_err)?;
        let ch = session.characteristic(service, characteristic).map_err(ble_err)?;
        session.subscribe(&ch).map_err(ble_err)?;
        // PnP ID: [vendor id source, vid LE, pid LE, version LE]
        let (vid, pid) = session
            .characteristic(DEVICE_INFORMATION, PNP_ID)
            .and_then(|c| session.read(&c))
            .ok()
            .filter(|v| v.len() >= 5)
            .map(|v| (u16::from_le_bytes([v[1], v[2]]), u16::from_le_bytes([v[3], v[4]])))
            .unwrap_or((0, 0));
        let info = LinkInfo {
            link: format!("gatt:{}", short_hash(&p.id)),
            kind: "gatt".into(),
            vendor_id: vid,
            product_id: pid,
            product: p.name.clone().unwrap_or_default(),
            bus: "ble".into(),
            collections: vec![],
        };
        Ok(GattTransport { session, ch, info })
    }
}

impl FrameTransport for GattTransport {
    fn info(&self) -> &LinkInfo {
        &self.info
    }
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        let f = to_gatt(frame)?;
        self.session.write(&self.ch, &f).map_err(ble_err)
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        Ok(self.session.next_value(&self.ch, timeout).map_err(ble_err)?.map(|v| from_gatt(&v)))
    }
}

#[derive(Default)]
pub struct GattLinks {
    pub last_error: Option<String>,
}

impl LinkSource for GattLinks {
    fn scan(&mut self, db: &DeviceDb) -> Vec<LinkCandidate> {
        let central = match Central::shared() {
            Ok(c) => c,
            Err(e) => {
                self.last_error = Some(e.to_string());
                return vec![];
            }
        };
        let service = db.hidpp.gatt_service.clone();
        let characteristic = db.hidpp.gatt_characteristic.clone();
        let peripherals = match central.connected_peripherals(&[&service]) {
            Ok(p) => p,
            Err(e) => {
                self.last_error = Some(e.to_string());
                return vec![];
            }
        };
        peripherals
            .into_iter()
            .map(|p| {
                let (s, c) = (service.clone(), characteristic.clone());
                LinkCandidate {
                    id: format!("gatt:{}", short_hash(&p.id)),
                    role: LinkRole::Direct {
                        transport: Transport::BluetoothLe,
                    },
                    long_only: true,
                    identity: vec![IdentityKey::Stable {
                        namespace: "ble".into(),
                        value: short_hash(&p.id),
                    }],
                    open: Box::new(move || GattTransport::open(central, &p, &s, &c).map(|t| Box::new(t) as BoxTransport)),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_matches_discovery_capture() {
        // Ping as captured in docs/discovery/bt-g915-gatt-hidpp.txt.
        let canonical = [0x11, 0xFF, 0x00, 0x1B, 0x00, 0x00, 0xAA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let g = to_gatt(&canonical).unwrap();
        assert_eq!(g.len(), 18);
        assert_eq!(&g[..5], &[0x00, 0x1B, 0x00, 0x00, 0xAA]);
        let reply = [0x00, 0x1B, 0x04, 0x02, 0xAA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(&from_gatt(&reply)[..7], &[0x11, 0xFF, 0x00, 0x1B, 0x04, 0x02, 0xAA]);
    }

    #[test]
    fn receiver_device_indexes_are_refused() {
        assert!(to_gatt(&[0x11, 0x01, 0x00, 0x1B]).is_err());
    }
}
