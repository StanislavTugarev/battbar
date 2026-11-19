// SPDX-License-Identifier: GPL-3.0-or-later
//! Keyed hashes for identifiers that end up in the settings (device IDs, Bluetooth
//! addresses, HID++ unit IDs, CoreBluetooth peripheral IDs).
//!
//! A plain hash is not enough: a 32-bit unit ID or a Bluetooth address with ~24 unknown
//! bits can be brute-forced from an unsalted hash in seconds. battbar therefore uses
//! HMAC-SHA256 with a random per-installation key (`salt` in the settings, generated on
//! first start). Without an installed key (tests, trace replay) a fixed default is used.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::io::Read;
use std::sync::RwLock;

static KEY: RwLock<Option<Vec<u8>>> = RwLock::new(None);
const DEFAULT_KEY: &[u8] = b"battbar-unkeyed-default";

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// A new random installation key (32 bytes, hex).
pub fn new_salt() -> String {
    let mut b = [0u8; 32];
    let ok = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok();
    if !ok {
        // Last resort (no /dev/urandom): time-based; still per-installation.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        b[..16].copy_from_slice(&t.as_nanos().to_le_bytes());
        b[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Install the per-installation key for all following hashes in this process.
pub fn set_install_salt(hex: &str) -> bool {
    match from_hex(hex).filter(|k| k.len() >= 16) {
        Some(k) => {
            *KEY.write().unwrap() = Some(k);
            true
        }
        None => false,
    }
}

/// HMAC-SHA256(key, domain ‖ 0x00 ‖ value), first 6 bytes as 12 hex digits.
pub fn keyed_hash(domain: &str, value: &str) -> String {
    let guard = KEY.read().unwrap();
    hash_with(guard.as_deref().unwrap_or(DEFAULT_KEY), domain, value)
}

fn hash_with(key: &[u8], domain: &str, value: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(domain.as_bytes());
    mac.update(&[0]);
    mac.update(value.as_bytes());
    mac.finalize().into_bytes()[..6].iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure function only: the global key is shared with tests running in parallel.
    #[test]
    fn keyed_hash_depends_on_key_domain_and_value() {
        let k1 = from_hex(&new_salt()).unwrap();
        let k2 = from_hex(&new_salt()).unwrap();
        assert_eq!(k1.len(), 32);
        assert_ne!(k1, k2);
        let a = hash_with(&k1, "unit", "1a2b3c4d");
        assert_eq!(a.len(), 12);
        assert_eq!(a, hash_with(&k1, "unit", "1a2b3c4d"), "deterministic");
        assert_ne!(a, hash_with(&k1, "bt-address", "1a2b3c4d"), "domain separated");
        assert_ne!(
            a,
            hash_with(&k2, "unit", "1a2b3c4d"),
            "different installations produce different IDs"
        );
        assert!(from_hex("zz").is_none());
        assert!(from_hex("abc").is_none());
    }
}
