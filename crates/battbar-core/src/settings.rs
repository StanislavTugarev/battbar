// SPDX-License-Identifier: GPL-3.0-or-later
//! User settings, stored locally in `~/.config/battbar/config.toml`.

use crate::merge::MergeConfig;
use crate::model::Chemistry;
use std::sync::RwLock;

static CONFIG_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Use a different configuration directory for this process (development builds use
/// `~/.config/battbar-dev`, so they never touch the release settings).
pub fn set_config_dir(dir: PathBuf) {
    *CONFIG_DIR.write().unwrap() = Some(dir);
}

/// `~/.config/battbar` (release) or the directory set with [`set_config_dir`].
static SAVES_WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times this process wrote the settings file (tests, diagnostics).
pub fn saves_written() -> u64 {
    SAVES_WRITTEN.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(d) = CONFIG_DIR.read().unwrap().clone() {
        return Some(d);
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/battbar"))
}

/// Development profile: `~/.config/battbar-dev`.
pub fn use_dev_profile() {
    if let Some(h) = std::env::var_os("HOME") {
        set_config_dir(PathBuf::from(h).join(".config/battbar-dev"));
    }
}

/// How battbar appears in the menu bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    /// One icon; every device is in the menu (default for new installations).
    Collapsed,
    /// All visible devices side by side in one status item.
    Expanded,
}

/// The user's choice of cell chemistry for a device with replaceable AA/AAA cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChemistryChoice {
    pub device: String,
    pub chemistry: Chemistry,
}
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
    /// Per-installation key for the keyed hashes of identifiers (hex). Generated on first
    /// start; never leaves this file (not part of `battbar dump` or traces).
    pub salt: String,
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
    /// The first-run "Launch battbar at login?" question was answered.
    pub asked_launch_at_login: bool,
    /// The "running from a temporary location, move to Applications" notice was shown.
    pub warned_translocation: bool,
    /// Alkaline / NiMH per device (voltage-only AA/AAA devices; default alkaline).
    pub chemistry: Vec<ChemistryChoice>,
    /// Per-device voltage → firmware percentage samples (HID++ 0x1001 devices that also
    /// report a percentage on Bluetooth). See `calibration`.
    pub voltage_calibration: Vec<crate::calibration::VoltageCalibration>,
    /// Opt-in daily update check (the only network request; off by default).
    pub check_for_updates: bool,
    /// The first-run "check for updates?" offer was shown.
    pub asked_update_check: bool,
    /// Unix time of the next update check.
    pub next_update_check: Option<u64>,
    /// Newer released version found by the last check (e.g. "0.3.0").
    pub available_update: Option<String>,
    /// Version the user was already notified about (one notification per version).
    pub update_notified: Option<String>,
    /// Collapsed / Expanded. `None` only before the first start of 0.4 decided it (see
    /// [`Settings::init_display_mode`]).
    pub display_mode: Option<DisplayMode>,
    /// Device IDs in the order the user arranged them (Move Left / Right). Empty: by
    /// type. Devices not listed follow in the default order.
    pub device_order: Vec<String>,
    /// The last Expand did not fit: battbar switched back to Collapsed. Shown next to
    /// *Expand in Menu Bar* until an Expand fits.
    pub collapsed_for_space: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            salt: String::new(),
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
            asked_launch_at_login: false,
            warned_translocation: false,
            chemistry: vec![],
            voltage_calibration: vec![],
            check_for_updates: false,
            asked_update_check: false,
            next_update_check: None,
            available_update: None,
            update_notified: None,
            display_mode: None,
            device_order: vec![],
            collapsed_for_space: false,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("config.toml"))
    }

    pub fn display_mode(&self) -> DisplayMode {
        self.display_mode.unwrap_or(DisplayMode::Collapsed)
    }

    /// First start of 0.4: new installations start collapsed; an installation that
    /// already had settings (0.3 and older showed every device) starts expanded, so its
    /// menu bar looks as before. Returns whether anything changed.
    pub fn init_display_mode(&mut self, settings_existed: bool) -> bool {
        if self.display_mode.is_some() {
            return false;
        }
        self.display_mode = Some(if settings_existed {
            DisplayMode::Expanded
        } else {
            DisplayMode::Collapsed
        });
        true
    }

    pub fn chemistry_for(&self, device: &str) -> Option<Chemistry> {
        self.chemistry.iter().find(|c| c.device == device).map(|c| c.chemistry)
    }

    pub fn set_chemistry(&mut self, device: &str, chemistry: Chemistry) {
        self.chemistry.retain(|c| c.device != device);
        self.chemistry.push(ChemistryChoice {
            device: device.into(),
            chemistry,
        });
    }

    /// Settings from disk; defaults if missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Load the settings and install the per-installation hash key. On the first start
    /// (or with settings from before keyed hashes) a key is generated and IDs stored
    /// under the old scheme are dropped, since they cannot be mapped to the new one.
    pub fn load_and_init() -> Self {
        let existed = Self::path().is_some_and(|p| p.exists());
        let mut s = Self::load();
        if s.init_display_mode(existed) {
            if let Err(e) = s.save() {
                eprintln!("battbar: saving settings: {e}");
            }
        }
        if !crate::ids::set_install_salt(&s.salt) {
            s.salt = crate::ids::new_salt();
            s.hidden.clear();
            s.shown.clear();
            s.ever_reported.clear();
            s.learned_links.clear();
            s.identity_memory.clear();
            s.chemistry.clear();
            s.voltage_calibration.clear();
            s.merge = MergeConfig::default();
            crate::ids::set_install_salt(&s.salt);
            if let Err(e) = s.save() {
                eprintln!("battbar: saving settings: {e}");
            }
        }
        s
    }

    /// Atomic write (temp file + rename), readable by the owner only. Nothing is written
    /// when the file already holds exactly this content (callers save after anything
    /// that might have changed; the disk sees only real changes).
    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = Self::path() else { return Ok(()) };
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        if std::fs::read_to_string(&p).is_ok_and(|cur| cur == text) {
            return Ok(());
        }
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        SAVES_WRITTEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
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
    fn display_mode_new_installs_collapse_and_03_configs_expand() {
        // No settings file yet: a new installation.
        let mut s = Settings::default();
        assert!(s.init_display_mode(false));
        assert_eq!(s.display_mode(), DisplayMode::Collapsed);
        // A 0.3 file: no display_mode key.
        let v03 = "salt = \"ab\"\npoll_interval_secs = 60\nhidden = [\"bt:1\"]\nasked_launch_at_login = true\n";
        let mut s: Settings = toml::from_str(v03).unwrap();
        assert_eq!(s.display_mode, None);
        assert!(s.init_display_mode(true));
        assert_eq!(s.display_mode(), DisplayMode::Expanded);
        // Decided once; the user's later choice stays.
        s.display_mode = Some(DisplayMode::Collapsed);
        assert!(!s.init_display_mode(true));
        assert_eq!(s.display_mode(), DisplayMode::Collapsed);
        let t = toml::to_string_pretty(&s).unwrap();
        assert!(t.contains("display_mode = \"collapsed\""));
        assert_eq!(toml::from_str::<Settings>(&t).unwrap().display_mode(), DisplayMode::Collapsed);
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
