// SPDX-License-Identifier: GPL-3.0-or-later
//! Facts about the running bundle.

use objc2_foundation::NSBundle;

pub fn identifier() -> Option<String> {
    NSBundle::mainBundle().bundleIdentifier().map(|s| s.to_string())
}

pub fn path() -> String {
    NSBundle::mainBundle().bundlePath().to_string()
}

/// Development build: bundle ID `….dev`, or any debug build.
pub fn is_dev_build() -> bool {
    cfg!(debug_assertions) || identifier().is_some_and(|id| id.ends_with(".dev"))
}

/// macOS runs a quarantined app that was not moved by the user from a randomised
/// read-only copy (App Translocation). Login items registered from there would point
/// to a temporary path, so first-run questions about them are postponed.
pub fn is_translocated() -> bool {
    path().contains("/AppTranslocation/")
}

/// Where the user actually put a translocated app (e.g. `~/Downloads/battbar.app`),
/// asked from Security.framework (`SecTranslocateCreateOriginalPathForURL`, read only;
/// resolved at run time because it is not public API). `None` if not translocated or
/// unavailable.
pub fn original_path_of(path: &str) -> Option<String> {
    use objc2::rc::Retained;
    use objc2_foundation::{NSString, NSURL};
    use std::ffi::c_void;
    type CreateOriginal = unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> *const c_void;
    if !path.contains("/AppTranslocation/") {
        return None;
    }
    let f: CreateOriginal = unsafe {
        let h = libc::dlopen(c"/System/Library/Frameworks/Security.framework/Security".as_ptr(), libc::RTLD_LAZY);
        if h.is_null() {
            return None;
        }
        let sym = libc::dlsym(h, c"SecTranslocateCreateOriginalPathForURL".as_ptr());
        if sym.is_null() {
            return None;
        }
        std::mem::transmute::<*mut c_void, CreateOriginal>(sym)
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    // NSURL and CFURL are toll-free bridged; the result follows the Create rule.
    let raw = unsafe { f(Retained::as_ptr(&url) as *const c_void, std::ptr::null_mut()) };
    if raw.is_null() {
        return None;
    }
    let original: Retained<NSURL> = unsafe { Retained::from_raw(raw as *mut NSURL) }?;
    original.path().map(|p| p.to_string())
}

/// Original location of this app if it runs translocated.
pub fn original_path() -> Option<String> {
    original_path_of(&path())
}
