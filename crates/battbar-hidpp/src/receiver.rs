// SPDX-License-Identifier: GPL-3.0-or-later
//! HID++ 1.0 receiver registers (Unifying / Lightspeed / Bolt).
//! Register layout ported from Solaar (GPL-2.0-or-later); Unifying/Lightspeed layout
//! verified on a Lightspeed receiver 046d:c547 (docs/DISCOVERY.md §2).

use crate::protocol::{Channel, HidppError, RECEIVER_INDEX};
use battbar_core::devicedb::ReceiverFamily;
use battbar_core::transport::FrameTransport;
use battbar_core::DeviceKind;

const REG_NOTIFICATIONS: u8 = 0x00;
/// HID++ 1.0 register 0x02 "connection state"; writing 0x02 makes the receiver send a
/// device-connection notification (0x41) for every device that is on line now (Solaar:
/// `Receiver.notify_devices`, register `RECEIVER_CONNECTION`).
pub const REG_CONNECTION_STATE: u8 = 0x02;
const REPLAY_CONNECTIONS: u8 = 0x02;
const REG_RECEIVER_INFO: u8 = 0xB5;
/// Wireless notifications + software present (Solaar NOTIFICATION_FLAG).
pub const WANTED_NOTIFICATION_FLAGS: u32 = 0x000100 | 0x000800;

#[derive(Debug, Clone, PartialEq)]
pub struct PairedSlot {
    /// Device index on the receiver (1-based).
    pub index: u8,
    pub wpid: u16,
    pub kind: Option<DeviceKind>,
    /// Unit ID from the extended pairing register (== HID++ 0x0003 unit ID).
    pub unit_id: Option<[u8; 4]>,
    pub codename: Option<String>,
}

fn kind_from_pairing(k: u8) -> Option<DeviceKind> {
    match k & 0x0F {
        1 | 3 => Some(DeviceKind::Keyboard),
        2 | 8 | 9 => Some(DeviceKind::Mouse),
        _ => None,
    }
}

/// Paired devices according to the receiver, whether they are online or not.
pub fn paired_slots<T: FrameTransport>(ch: &mut Channel<T>, family: ReceiverFamily, max: u8) -> Result<Vec<PairedSlot>, HidppError> {
    let mut out = vec![];
    for n in 0..max {
        let slot = match family {
            ReceiverFamily::Unifying | ReceiverFamily::Lightspeed => {
                let p = match ch.read_long_register(RECEIVER_INFO_INDEX, REG_RECEIVER_INFO, [0x20 + n, 0, 0]) {
                    Ok(p) => p,
                    Err(HidppError::Hidpp10(_)) => continue, // empty slot
                    Err(e) => return Err(e),
                };
                if p.len() < 8 || p[0] != 0x20 + n {
                    continue;
                }
                let wpid = u16::from_be_bytes([p[3], p[4]]);
                let unit_id = ch
                    .read_long_register(RECEIVER_INFO_INDEX, REG_RECEIVER_INFO, [0x30 + n, 0, 0])
                    .ok()
                    .filter(|e| e.len() >= 5 && e[0] == 0x30 + n)
                    .map(|e| [e[1], e[2], e[3], e[4]]);
                let codename = ch
                    .read_long_register(RECEIVER_INFO_INDEX, REG_RECEIVER_INFO, [0x40 + n, 0, 0])
                    .ok()
                    .filter(|c| c.len() > 2 && c[0] == 0x40 + n)
                    .map(|c| {
                        let l = (c[1] as usize).min(c.len() - 2);
                        String::from_utf8_lossy(&c[2..2 + l]).trim_end_matches('\0').to_string()
                    });
                PairedSlot {
                    index: n + 1,
                    wpid,
                    kind: kind_from_pairing(p[7]),
                    unit_id,
                    codename,
                }
            }
            ReceiverFamily::Bolt => {
                // Untested on hardware: layout per Solaar.
                let p = match ch.read_long_register(RECEIVER_INFO_INDEX, REG_RECEIVER_INFO, [0x50 + n, 0, 0]) {
                    Ok(p) => p,
                    Err(HidppError::Hidpp10(_)) => continue,
                    Err(e) => return Err(e),
                };
                if p.len() < 4 || p[0] != 0x50 + n {
                    continue;
                }
                PairedSlot {
                    index: n + 1,
                    wpid: u16::from_le_bytes([p[2], p[3]]),
                    kind: kind_from_pairing(p[1]),
                    unit_id: None,
                    codename: None,
                }
            }
        };
        if slot.wpid != 0 {
            out.push(slot);
        }
    }
    Ok(out)
}

const RECEIVER_INFO_INDEX: u8 = RECEIVER_INDEX;

/// Ask the receiver to repeat the connection notifications of the devices on line now
/// (after enabling notifications on a newly opened link: a device that connected before
/// notifications were on would otherwise never be announced). The notifications arrive
/// like real ones and trigger a poll.
pub fn replay_connections<T: FrameTransport>(ch: &mut Channel<T>) -> Result<(), HidppError> {
    ch.write_short_register(RECEIVER_INDEX, REG_CONNECTION_STATE, [REPLAY_CONNECTIONS, 0, 0])?;
    Ok(())
}

/// Make sure the receiver sends connection notifications. Returns true if a register
/// write was needed (e.g. no vendor software running).
pub fn ensure_notifications<T: FrameTransport>(ch: &mut Channel<T>) -> Result<bool, HidppError> {
    let r = ch.read_short_register(RECEIVER_INDEX, REG_NOTIFICATIONS, [0, 0, 0])?;
    let cur = u32::from_be_bytes([0, r[0], r[1], r[2]]);
    if cur & WANTED_NOTIFICATION_FLAGS == WANTED_NOTIFICATION_FLAGS {
        return Ok(false);
    }
    let new = (cur | WANTED_NOTIFICATION_FLAGS).to_be_bytes();
    ch.write_short_register(RECEIVER_INDEX, REG_NOTIFICATIONS, [new[1], new[2], new[3]])?;
    Ok(true)
}
