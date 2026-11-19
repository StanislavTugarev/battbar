// SPDX-License-Identifier: GPL-3.0-or-later
//! HID++ links on USB via hidapi: receivers and directly attached devices. Interfaces
//! are selected by vendor ID and usage page from the device database, never by PID.

use crate::provider::{BoxTransport, LinkCandidate, LinkRole, LinkSource};
use battbar_core::{DeviceDb, Transport};
use battbar_hid::{Bus, HidInterface};

#[derive(Default)]
pub struct HidapiLinks {
    pub last_error: Option<String>,
}

/// Whether a HID interface is a HID++ link according to the database.
pub fn is_hidpp_interface(i: &HidInterface, db: &DeviceDb) -> bool {
    i.bus == Bus::Usb
        && db.hidpp.vendor_ids.contains(&i.vendor_id)
        && i.is_vendor_only()
        && i.collections
            .iter()
            .any(|(p, u)| db.is_hidpp_usage_page(*p) && (*u & 0xFF) as u8 == db.hidpp.long_usage)
}

impl LinkSource for HidapiLinks {
    fn scan(&mut self, db: &DeviceDb) -> Vec<LinkCandidate> {
        let ifaces = match battbar_hid::enumerate() {
            Ok(v) => v,
            Err(e) => {
                self.last_error = Some(e.to_string());
                return vec![];
            }
        };
        ifaces
            .into_iter()
            .filter(|i| is_hidpp_interface(i, db))
            .map(|i| {
                let role = match db.receiver(i.vendor_id, i.product_id) {
                    Some(r) => LinkRole::Receiver(r.clone()),
                    None => LinkRole::Direct { transport: Transport::Usb },
                };
                let long_only = !i
                    .collections
                    .iter()
                    .any(|(p, u)| db.is_hidpp_usage_page(*p) && (*u & 0xFF) as u8 == db.hidpp.short_usage);
                LinkCandidate {
                    id: i.link_id(),
                    role,
                    long_only,
                    identity: vec![],
                    open: Box::new(move || battbar_hid::open(&i).map(|t| Box::new(t) as BoxTransport)),
                }
            })
            .collect()
    }
}
