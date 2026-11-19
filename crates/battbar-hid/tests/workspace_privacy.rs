// SPDX-License-Identifier: GPL-3.0-or-later
//! Workspace-wide privacy guarantees that are checked statically:
//! * only `battbar-hid` may use `hidapi` or IOKit HID APIs (so every HID open goes
//!   through the vendor-only guard, which also refuses Bluetooth HID devices);
//! * the Bluetooth code never scans (`scanForPeripherals`) and never opens IOHIDDevice;
//! * the only network access is the opt-in update check in `battbar-app/src/updates.rs`
//!   (no HTTP client crates, no URL-loading APIs anywhere else).

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "target") {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn workspace_crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Source text without line comments (docs may mention the forbidden names).
fn code(p: &Path) -> String {
    fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn hid_apis_only_in_battbar_hid() {
    let mut files = vec![];
    rust_files(&workspace_crates(), &mut files);
    assert!(files.len() > 10, "workspace sources not found");
    for f in files {
        if f.components().any(|c| c.as_os_str() == "battbar-hid") {
            continue;
        }
        let s = code(&f);
        for forbidden in ["hidapi::", "IOHIDDeviceOpen", "IOHIDDeviceCreate", "IOHIDManager", "objc2_io_kit"] {
            assert!(
                !s.contains(forbidden),
                "{} uses {forbidden}; HID access must go through battbar-hid",
                f.display()
            );
        }
    }
}

#[test]
fn no_crate_but_battbar_hid_depends_on_hidapi() {
    for e in fs::read_dir(workspace_crates()).unwrap().flatten() {
        let manifest = e.path().join("Cargo.toml");
        if !manifest.exists() || e.file_name() == "battbar-hid" {
            continue;
        }
        let m = fs::read_to_string(&manifest).unwrap();
        assert!(
            !m.lines().any(|l| l.trim_start().starts_with("hidapi")),
            "{} depends on hidapi",
            manifest.display()
        );
    }
}

#[test]
fn bluetooth_code_never_scans() {
    let mut files = vec![];
    rust_files(&workspace_crates(), &mut files);
    for f in files.into_iter().filter(|f| !f.ends_with("tests/workspace_privacy.rs")) {
        let s = code(&f);
        assert!(!s.contains("scanForPeripherals"), "{} scans for Bluetooth peripherals", f.display());
        assert!(!s.contains("IOBluetoothDeviceInquiry"), "{} runs a Bluetooth inquiry", f.display());
    }
}

const UPDATE_CHECK: &str = "battbar-app/src/updates.rs";

#[test]
fn network_only_in_the_update_check() {
    let mut files = vec![];
    rust_files(&workspace_crates(), &mut files);
    let mut found_update_check = false;
    for f in files.into_iter().filter(|f| !f.ends_with("tests/workspace_privacy.rs")) {
        let s = code(&f);
        if f.ends_with(UPDATE_CHECK) {
            found_update_check = true;
            assert!(
                s.contains("\"/usr/bin/curl\""),
                "update check no longer uses /usr/bin/curl; update this test"
            );
            continue;
        }
        for forbidden in [
            "\"/usr/bin/curl\"",
            "\"curl\"",
            "\"wget\"",
            "\"nc\"",
            "NSURLSession",
            "NSURLConnection",
            "NSURLRequest",
            "ContentsOfURL",
            "TcpStream",
            "UdpSocket",
            "api.github.com",
        ] {
            assert!(
                !s.contains(forbidden),
                "{} uses {forbidden}; network access belongs only to {UPDATE_CHECK}",
                f.display()
            );
        }
    }
    assert!(found_update_check, "{UPDATE_CHECK} not found");
}

#[test]
fn no_http_client_crates_or_url_loading_features() {
    for e in fs::read_dir(workspace_crates()).unwrap().flatten() {
        let manifest = e.path().join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let m = fs::read_to_string(&manifest).unwrap();
        for l in m.lines().map(str::trim_start) {
            for krate in [
                "reqwest",
                "ureq",
                "hyper",
                "curl",
                "isahc",
                "attohttpc",
                "surf",
                "minreq",
                "rustls",
                "native-tls",
                "openssl",
                "tokio",
            ] {
                assert!(
                    !l.starts_with(&format!("{krate} ")) && !l.starts_with(&format!("{krate}=")),
                    "{} depends on {krate}",
                    manifest.display()
                );
            }
            for feature in ["\"NSURLSession\"", "\"NSURLConnection\"", "\"NSURLRequest\""] {
                assert!(!l.contains(feature), "{} enables {feature}", manifest.display());
            }
        }
    }
}
