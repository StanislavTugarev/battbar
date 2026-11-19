// SPDX-License-Identifier: GPL-3.0-or-later
//! User settings, stored locally in `~/.config/battbar/config.toml`.

use crate::merge::MergeConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A remembered headset ↔ Bluetooth link (see SPEC "Learned headset ↔ Bluetooth link").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedLink {
    /// Full source key of the base station report (`steelseries/…`).
    pub base: String,
    /// Full source key of the Bluetooth report; contains only a hash of the address.
    pub bluetooth: String,
    /// Display name at the time of learning.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub poll_interval_secs: u64,
    /// Low battery notification threshold in percent; `None` = off.
    pub low_battery_threshold: Option<u8>,
    /// Notify when a spare battery in a base station is fully charged.
    pub notify_spare_full: bool,
    pub headsetcontrol_fallback: bool,
    /// Show Bluetooth devices that are connected but report no battery.
    pub show_unreported: bool,
    /// Device IDs hidden from the menu bar by the user.
    pub hidden: Vec<String>,
    /// Device IDs the user chose to show although they would be hidden by default.
    pub shown: Vec<String>,
    /// Device IDs that reported a battery value at least once. Disconnected Bluetooth
    /// devices that never did are hidden by default.
    pub ever_reported: Vec<String>,
    /// Hashed identity keys known to belong to the same device (IdentityMemory).
    pub identity_memory: Vec<Vec<String>>,
    pub merge: MergeConfig,
    pub learned_links: Vec<LearnedLink>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            poll_interval_secs: 60,
            low_battery_threshold: Some(15),
            notify_spare_full: false,
            headsetcontrol_fallback: true,
            show_unreported: false,
            hidden: vec![],
            shown: vec![],
            ever_reported: vec![],
            identity_memory: vec![],
            merge: MergeConfig::default(),
            learned_links: vec![],
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/battbar/config.toml"))
    }

    /// Settings from disk; defaults if missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Atomic write (temp file + rename).
    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = Self::path() else { return Ok(()) };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, p)
    }

    /// Effective merge configuration: manual groups, learned links, splits.
    pub fn merge_config(&self) -> MergeConfig {
        let mut m = self.merge.clone();
        for l in &self.learned_links {
            m.manual_groups.push(vec![l.base.clone(), l.bluetooth.clone()]);
        }
        m
    }

    /// Force all `keys` into one device (and lift splits on them).
    pub fn merge_sources(&mut self, keys: Vec<String>) {
        self.merge.splits.retain(|k| !keys.contains(k));
        self.merge.manual_groups.retain(|g| !g.iter().any(|k| keys.contains(k)));
        self.merge.manual_groups.push(keys);
    }

    /// Keep every one of `keys` on its own; drop manual groups and learned links on them.
    pub fn split_sources(&mut self, keys: &[String]) {
        self.merge.manual_groups.retain(|g| !g.iter().any(|k| keys.contains(k)));
        self.learned_links
            .retain(|l| !keys.contains(&l.base) && !keys.contains(&l.bluetooth));
        for k in keys {
            if !self.merge.splits.contains(k) {
                self.merge.splits.push(k.clone());
            }
        }
    }

    pub fn unlink(&mut self, bluetooth_key: &str) {
        self.learned_links.retain(|l| l.bluetooth != bluetooth_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults() {
        let s = Settings::default();
        let t = toml::to_string_pretty(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&t).unwrap(), s);
        // Partial files keep defaults for missing keys.
        let p: Settings = toml::from_str("poll_interval_secs = 30").unwrap();
        assert_eq!(p.poll_interval_secs, 30);
        assert_eq!(p.low_battery_threshold, Some(15));
    }

    #[test]
    fn learned_links_become_groups_and_split_removes_them() {
        let mut s = Settings::default();
        s.learned_links.push(LearnedLink {
            base: "steelseries/b".into(),
            bluetooth: "bt-system/classic:h".into(),
            name: "H".into(),
        });
        assert_eq!(
            s.merge_config().manual_groups,
            vec![vec!["steelseries/b".to_string(), "bt-system/classic:h".to_string()]]
        );
        s.split_sources(&["bt-system/classic:h".into()]);
        assert!(s.learned_links.is_empty());
        assert_eq!(s.merge.splits, vec!["bt-system/classic:h".to_string()]);
        s.merge_sources(vec!["bt-system/classic:h".into(), "steelseries/b".into()]);
        assert!(s.merge.splits.is_empty());
    }
}
