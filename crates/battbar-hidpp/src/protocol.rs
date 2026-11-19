// SPDX-License-Identifier: GPL-3.0-or-later
//! HID++ request/response channel over any [`FrameTransport`].
//!
//! Frames are always canonical HID++ reports: `[report id, device index, b2, b3, params…]`
//! with report ID 0x10 (short, 7 bytes) or 0x11 (long, 20 bytes). Link-specific framing
//! (e.g. GATT) is done by the transport.
//!
//! HID++ 2.0: b2 = feature index, b3 = function << 4 | software ID.
//! HID++ 1.0 registers: b2 = sub-ID (0x80/0x81 short write/read, 0x82/0x83 long), b3 = register.
//! Error layout follows Solaar (GPL-2.0-or-later).

use battbar_core::transport::{FrameTransport, TransportError};
use std::time::{Duration, Instant};

pub const REPORT_SHORT: u8 = 0x10;
pub const REPORT_LONG: u8 = 0x11;
pub const SHORT_LEN: usize = 7;
pub const LONG_LEN: usize = 20;
pub const RECEIVER_INDEX: u8 = 0xFF;
/// Our software ID (low nibble of the function byte). Non-zero; 0 marks notifications.
pub const SW_ID: u8 = 0x0B;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_NOTIFICATIONS: usize = 64;

/// HID++ 1.0 error codes.
pub mod err10 {
    pub const INVALID_SUBID: u8 = 0x01;
    pub const INVALID_ADDRESS: u8 = 0x02;
    pub const INVALID_VALUE: u8 = 0x03;
    pub const UNKNOWN_DEVICE: u8 = 0x08;
    pub const RESOURCE_ERROR: u8 = 0x09;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HidppError {
    #[error("timeout")]
    Timeout,
    #[error("transport: {0}")]
    Transport(#[from] TransportError),
    #[error("HID++ 1.0 error 0x{0:02x}")]
    Hidpp10(u8),
    #[error("HID++ 2.0 error 0x{0:02x}")]
    Hidpp20(u8),
}

pub struct Channel<T: FrameTransport> {
    pub transport: T,
    /// Some links (BLE) carry long reports only.
    pub long_only: bool,
    pub timeout: Duration,
    /// Unsolicited HID++ frames (software ID 0 or receiver notifications), oldest first.
    pub notifications: Vec<Vec<u8>>,
    /// Replies addressed to other software (G HUB, Options+): counted, then dropped.
    pub foreign_replies: u64,
    /// Diagnostics (`battbar dump`, `battbar record`): HID++ 1.0 SET_REGISTER requests
    /// (sub-IDs 0x80 short, 0x82 long) are refused before anything is sent.
    pub read_only: bool,
}

/// HID++ 1.0 SET_REGISTER sub-IDs (short, long).
pub const SET_REGISTER_SUB_IDS: [u8; 2] = [0x80, 0x82];

impl<T: FrameTransport> Channel<T> {
    pub fn new(transport: T, long_only: bool) -> Self {
        Channel {
            transport,
            long_only,
            timeout: DEFAULT_TIMEOUT,
            notifications: vec![],
            foreign_replies: 0,
            read_only: false,
        }
    }

    fn push_notification(&mut self, f: Vec<u8>) {
        if self.notifications.len() >= MAX_NOTIFICATIONS {
            self.notifications.remove(0);
        }
        self.notifications.push(f);
    }

    /// Read pending frames without waiting (collect notifications).
    pub fn drain(&mut self) -> Result<(), HidppError> {
        while let Some(f) = self.transport.read(Duration::ZERO)? {
            self.classify_unsolicited(f);
        }
        Ok(())
    }

    fn classify_unsolicited(&mut self, f: Vec<u8>) {
        if f.len() < 4 || (f[0] != REPORT_SHORT && f[0] != REPORT_LONG) {
            return; // not HID++ (e.g. DJ reports on a receiver's vendor interface)
        }
        // HID++ 2.0 replies carry a software ID; notifications use 0. HID++ 1.0
        // receiver notifications use sub-IDs 0x40..=0x7F (e.g. 0x41 device connection);
        // feature indexes in practice stay below 0x40.
        let is_10_notification = (0x40..=0x7F).contains(&f[2]);
        if f[3] & 0x0F == 0 || is_10_notification {
            self.push_notification(f);
        } else {
            self.foreign_replies += 1;
        }
    }

    /// Send one request and wait for the matching reply. Returns the parameter bytes.
    pub fn request(&mut self, idx: u8, b2: u8, b3: u8, params: &[u8]) -> Result<Vec<u8>, HidppError> {
        if self.read_only && SET_REGISTER_SUB_IDS.contains(&b2) {
            return Err(HidppError::Transport(TransportError::Refused(
                "register write in read-only (diagnostic) mode".into(),
            )));
        }
        let long = self.long_only || params.len() > 3;
        let mut req = vec![if long { REPORT_LONG } else { REPORT_SHORT }, idx, b2, b3];
        req.extend_from_slice(params);
        req.resize(if long { LONG_LEN } else { SHORT_LEN }, 0);
        self.transport.write(&req)?;

        let deadline = Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(HidppError::Timeout);
            }
            let Some(r) = self.transport.read(left)? else { continue };
            if r.len() < 5 || (r[0] != REPORT_SHORT && r[0] != REPORT_LONG) {
                continue;
            }
            if r[1] == idx {
                if r[2] == 0xFF && r[3] == b2 && r[4] == b3 {
                    return Err(HidppError::Hidpp20(r.get(5).copied().unwrap_or(0)));
                }
                if r[2] == 0x8F && r[3] == b2 && r[4] == b3 {
                    return Err(HidppError::Hidpp10(r.get(5).copied().unwrap_or(0)));
                }
                if r[2] == b2 && r[3] == b3 {
                    return Ok(r[4..].to_vec());
                }
            }
            self.classify_unsolicited(r);
        }
    }

    /// HID++ 2.0 function call.
    pub fn call(&mut self, idx: u8, feature_index: u8, function: u8, params: &[u8]) -> Result<Vec<u8>, HidppError> {
        self.request(idx, feature_index, (function << 4) | SW_ID, params)
    }

    /// IRoot.ping → protocol version (major, minor).
    pub fn ping(&mut self, idx: u8) -> Result<(u8, u8), HidppError> {
        let r = self.call(idx, 0x00, 1, &[0, 0, 0xAA])?;
        Ok((r[0], r[1]))
    }

    /// IRoot.getFeature → (feature index, version), `None` if not supported.
    pub fn feature(&mut self, idx: u8, id: u16) -> Result<Option<(u8, u8)>, HidppError> {
        let r = self.call(idx, 0x00, 0, &[(id >> 8) as u8, id as u8])?;
        Ok(if r[0] == 0 {
            None
        } else {
            Some((r[0], r.get(2).copied().unwrap_or(0)))
        })
    }

    pub fn read_short_register(&mut self, idx: u8, reg: u8, params: [u8; 3]) -> Result<Vec<u8>, HidppError> {
        self.request(idx, 0x81, reg, &params)
    }

    pub fn write_short_register(&mut self, idx: u8, reg: u8, params: [u8; 3]) -> Result<Vec<u8>, HidppError> {
        self.request(idx, 0x80, reg, &params)
    }

    pub fn read_long_register(&mut self, idx: u8, reg: u8, params: [u8; 3]) -> Result<Vec<u8>, HidppError> {
        self.request(idx, 0x83, reg, &params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battbar_core::transport::LinkInfo;
    use std::collections::VecDeque;

    /// Scripted link: each write pops a batch of frames to deliver.
    struct Script {
        info: LinkInfo,
        batches: VecDeque<Vec<Vec<u8>>>,
        pending: VecDeque<Vec<u8>>,
        written: Vec<Vec<u8>>,
    }
    impl FrameTransport for Script {
        fn info(&self) -> &LinkInfo {
            &self.info
        }
        fn write(&mut self, f: &[u8]) -> Result<(), TransportError> {
            self.written.push(f.to_vec());
            if let Some(b) = self.batches.pop_front() {
                self.pending.extend(b);
            }
            Ok(())
        }
        fn read(&mut self, _t: Duration) -> Result<Option<Vec<u8>>, TransportError> {
            Ok(self.pending.pop_front())
        }
    }

    fn script(batches: Vec<Vec<Vec<u8>>>) -> Channel<Script> {
        let info = LinkInfo {
            link: "s".into(),
            kind: "test".into(),
            vendor_id: 0,
            product_id: 0,
            product: "".into(),
            bus: "usb".into(),
            collections: vec![],
        };
        let mut c = Channel::new(
            Script {
                info,
                batches: batches.into(),
                pending: VecDeque::new(),
                written: vec![],
            },
            false,
        );
        c.timeout = Duration::from_millis(50);
        c
    }

    #[test]
    fn replies_for_other_software_ids_are_ignored() {
        // G HUB (sw id 0x1) asked the same thing; its reply arrives first.
        let mut c = script(vec![vec![
            vec![0x11, 0x02, 0x00, 0x11, 0x04, 0x02, 0xAA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![0x10, 0x02, 0x06, 0x00, 0x0b, 0x01, 0x00], // 0x1004 notification (sw id 0)
            vec![0x11, 0x02, 0x00, 0x1B, 0x04, 0x02, 0xAA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ]]);
        assert_eq!(c.ping(2).unwrap(), (4, 2));
        assert_eq!(c.foreign_replies, 1);
        assert_eq!(c.notifications.len(), 1);
    }

    #[test]
    fn errors_are_decoded() {
        let mut c = script(vec![
            vec![vec![0x10, 0x03, 0x8F, 0x00, 0x1B, 0x08, 0x00]],
            vec![vec![0x11, 0x02, 0xFF, 0x06, 0x1B, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
        ]);
        assert_eq!(c.ping(3), Err(HidppError::Hidpp10(err10::UNKNOWN_DEVICE)));
        assert_eq!(c.call(2, 0x06, 1, &[]), Err(HidppError::Hidpp20(0x05)));
    }

    #[test]
    fn read_only_channel_never_sends_set_register() {
        let mut c = script(vec![]);
        c.read_only = true;
        assert!(matches!(
            c.write_short_register(RECEIVER_INDEX, 0x00, [0, 9, 0]),
            Err(HidppError::Transport(TransportError::Refused(_)))
        ));
        assert!(matches!(
            c.request(RECEIVER_INDEX, 0x82, 0xB2, &[0; 16]),
            Err(HidppError::Transport(TransportError::Refused(_)))
        ));
        assert!(c.transport.written.is_empty(), "nothing may reach the device");
        // Reads are allowed.
        let _ = c.read_short_register(RECEIVER_INDEX, 0x00, [0, 0, 0]);
        assert_eq!(c.transport.written.len(), 1);
    }

    #[test]
    fn timeout_when_nobody_answers() {
        let mut c = script(vec![]);
        assert_eq!(c.ping(1), Err(HidppError::Timeout));
    }

    #[test]
    fn requests_use_our_software_id_and_long_only_links() {
        let mut c = script(vec![]);
        c.long_only = true;
        let _ = c.ping(0xFF);
        let w = &c.transport.written[0];
        assert_eq!(w.len(), LONG_LEN);
        assert_eq!(&w[..7], &[0x11, 0xFF, 0x00, 0x1B, 0x00, 0x00, 0xAA]);
    }
}
