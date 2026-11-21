// SPDX-License-Identifier: GPL-3.0-or-later
//! The disk sees only real changes: the app saves its settings after every poll that
//! might have changed them (learned identities, calibration samples, …); a save with
//! unchanged content writes nothing. Own test binary: the config directory is global.

use battbar_core::calibration::VoltageCalibration;
use battbar_core::settings::{saves_written, set_config_dir};
use battbar_core::Settings;

#[test]
fn polls_without_changes_write_nothing() {
    let dir = std::env::temp_dir().join(format!("battbar-settings-writes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    set_config_dir(dir.clone());
    let path = Settings::path().unwrap();

    let mut s = Settings::default();
    s.ever_reported.push("dev-1".into());
    s.save().unwrap();
    assert_eq!(saves_written(), 1, "the first save writes");
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();

    // 1000 polls that change nothing: same content, no write, file untouched.
    for _ in 0..1000 {
        s.save().unwrap();
    }
    assert_eq!(saves_written(), 1, "no write for unchanged content");
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    assert!(!dir.join("config.toml.tmp").exists(), "no temporary file left");

    // The same calibration sample again and again is not a change either.
    let mut cal = VoltageCalibration::new("dev-1");
    assert!(cal.add(60, 3800));
    for _ in 0..100 {
        assert!(!cal.add(60, 3800));
    }

    // A real change is written once.
    s.ever_reported.push("dev-2".into());
    s.save().unwrap();
    s.save().unwrap();
    assert_eq!(saves_written(), 2);
    let back: Settings = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back.ever_reported, ["dev-1", "dev-2"]);
    let _ = std::fs::remove_dir_all(&dir);
}
