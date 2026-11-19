// SPDX-License-Identifier: GPL-3.0-or-later
//! Find every HID++ unit ID in a recorded trace by parsing the frames, independent of
//! which devices produced battery reports (e.g. a wired HID++ microphone has no battery
//! but still answers 0x0003). Used by `battbar record` to pseudonymize traces and by
//! the CI check that published traces contain pseudonyms only.

use battbar_core::transport::{from_hex, Dir, Trace, TraceLine};
use std::collections::HashMap;

/// Unit IDs found in the trace, in order of first appearance.
pub fn unit_ids_in_trace(t: &Trace) -> Vec<[u8; 4]> {
    let mut out: Vec<[u8; 4]> = vec![];
    let push = |u: [u8; 4], out: &mut Vec<[u8; 4]>| {
        if u != [0; 4] && !out.contains(&u) {
            out.push(u);
        }
    };
    // (link, device index, function byte) of pending IRoot.getFeature(0x0003) requests.
    let mut pending: Vec<(String, u8, u8)> = vec![];
    // (link, device index) → feature index of 0x0003.
    let mut info_index: HashMap<(String, u8), u8> = HashMap::new();
    for l in &t.lines {
        let TraceLine::Frame { link, dir, hex, .. } = l else { continue };
        let Some(f) = from_hex(hex) else { continue };
        if f.len() < 7 || (f[0] != 0x10 && f[0] != 0x11) {
            continue;
        }
        match dir {
            Dir::Out => {
                // IRoot (index 0) function 0 = getFeature, parameter 0x0003.
                if f[2] == 0x00 && f[3] >> 4 == 0 && f[4] == 0x00 && f[5] == 0x03 {
                    pending.push((link.clone(), f[1], f[3]));
                }
            }
            Dir::In => {
                // Receiver extended pairing information: 0xB5 / 0x30+n, bytes 1..5 = unit ID.
                if f.len() >= 9 && f[1] == 0xFF && f[2] == 0x83 && f[3] == 0xB5 && (0x30..=0x3F).contains(&f[4]) {
                    push([f[5], f[6], f[7], f[8]], &mut out);
                    continue;
                }
                if f[2] == 0x00 {
                    if let Some(p) = pending.iter().position(|(lk, idx, fb)| lk == link && *idx == f[1] && *fb == f[3]) {
                        pending.remove(p);
                        if f[4] != 0 {
                            info_index.insert((link.clone(), f[1]), f[4]);
                        }
                        continue;
                    }
                }
                // DeviceInformation.getDeviceInfo (function 0): [entities, unit ID ×4, …].
                if f.len() >= 9 && info_index.get(&(link.clone(), f[1])) == Some(&f[2]) && f[3] >> 4 == 0 && f[3] & 0x0F != 0 {
                    push([f[5], f[6], f[7], f[8]], &mut out);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ids_from_pairing_register_and_device_information() {
        let text = r#"{"type":"header","battbar":"0","link":"r","kind":"hidapi","vendor_id":1133,"product_id":50503,"product":"","bus":"usb","collections":[]}
{"type":"frame","t_ms":0,"link":"r","dir":"out","hex":"10 ff 83 b5 30 00 00"}
{"type":"frame","t_ms":1,"link":"r","dir":"in","hex":"11 ff 83 b5 30 11 22 33 44 18 40 a0 01 05 00 00 00 00 00 00"}
{"type":"frame","t_ms":2,"link":"r","dir":"out","hex":"10 ff 00 0b 00 03 00"}
{"type":"frame","t_ms":3,"link":"r","dir":"in","hex":"11 ff 00 0b 02 00 07 00 00 00 00 00 00 00 00 00 00 00 00 00"}
{"type":"frame","t_ms":4,"link":"r","dir":"out","hex":"10 ff 02 0b 00 00 00"}
{"type":"frame","t_ms":5,"link":"r","dir":"in","hex":"11 ff 02 0b 02 0a 0b 0c 0d 00 08 0a fc 00 00 00 00 00 00 01"}"#;
        let t = Trace::parse(text.as_bytes()).unwrap();
        assert_eq!(unit_ids_in_trace(&t), vec![[0x11, 0x22, 0x33, 0x44], [0x0a, 0x0b, 0x0c, 0x0d]]);
    }
}
