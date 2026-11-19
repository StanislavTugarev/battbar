// SPDX-License-Identifier: GPL-3.0-or-later
// Embeds devices/*.toml into the binary.
use std::{env, fs, path::PathBuf};

fn main() {
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
