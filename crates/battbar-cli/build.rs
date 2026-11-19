// SPDX-License-Identifier: GPL-3.0-or-later
// Embed Info.plist (__TEXT,__info_plist) so macOS finds the Bluetooth usage
// description when the CLI runs outside the app bundle.
fn main() {
    let plist = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
}
