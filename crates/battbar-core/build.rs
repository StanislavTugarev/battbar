// SPDX-License-Identifier: GPL-3.0-or-later
// Embeds devices/*.toml into the binary, and the full version (with -rc.N).
use std::{env, fs, path::PathBuf};

/// `BATTBAR_FULL_VERSION`: `BATTBAR_RELEASE_VERSION` (set by the release workflow from
/// the tag, e.g. `0.5.0-rc.1`) or the crate version. It must be `X.Y.Z` or
/// `X.Y.Z-rc.N` with the crate's `X.Y.Z`.
fn full_version() {
    println!("cargo:rerun-if-env-changed=BATTBAR_RELEASE_VERSION");
    let base = env::var("CARGO_PKG_VERSION").unwrap();
    let full = env::var("BATTBAR_RELEASE_VERSION")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| base.clone());
    let ok = full == base
        || full
            .strip_prefix(&format!("{base}-rc."))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    assert!(
        ok,
        "BATTBAR_RELEASE_VERSION={full} does not match the crate version {base} (X.Y.Z or X.Y.Z-rc.N)"
    );
    println!("cargo:rustc-env=BATTBAR_FULL_VERSION={full}");
}

fn main() {
    full_version();
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../devices");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("devices/ directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    let mut out =
        String::from("/// Built-in device database files: (file name, contents).\npub static EMBEDDED_DEVICES: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let canon = f.canonicalize().unwrap();
        out += &format!(
            "    ({:?}, include_str!({:?})),\n",
            f.file_name().unwrap().to_string_lossy(),
            canon.display().to_string()
        );
    }
    out += "];\n";
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("embedded_devices.rs"), out).unwrap();
}
