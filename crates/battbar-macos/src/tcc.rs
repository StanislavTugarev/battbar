// SPDX-License-Identifier: GPL-3.0-or-later
//! TCC (privacy permission) helpers.
//!
//! macOS attributes Bluetooth access to the *responsible* process. A CLI started from a
//! terminal is attributed to the terminal app, which has no Bluetooth usage
//! description, so the CLI is killed on its first CoreBluetooth/IOBluetooth call. The
//! CLI therefore re-launches itself as its own responsible process (the same technique
//! terminal emulators use) so that the battbar Info.plist applies.

use objc2_foundation::{NSBundle, NSString};
use std::ffi::{c_char, c_int, CString};

pub const DISCLAIMED_ENV: &str = "BATTBAR_RESPONSIBLE";

/// Whether the main bundle (or the embedded `__info_plist`) declares the Bluetooth
/// usage description. Without it any Bluetooth call aborts the process.
pub fn bluetooth_usage_description_present() -> bool {
    let b = NSBundle::mainBundle();
    b.objectForInfoDictionaryKey(&NSString::from_str("NSBluetoothAlwaysUsageDescription"))
        .is_some()
}

type SetDisclaim = unsafe extern "C" fn(*mut libc::posix_spawnattr_t, c_int) -> c_int;

fn set_disclaim_fn() -> Option<SetDisclaim> {
    let sym = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"responsibility_spawnattrs_setdisclaim".as_ptr()) };
    if sym.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut libc::c_void, SetDisclaim>(sym) })
    }
}

/// Whether this process is already the responsible one (re-launched by us).
pub fn is_disclaimed() -> bool {
    std::env::var_os(DISCLAIMED_ENV).is_some()
}

/// Re-run the current executable with the same arguments as its own responsible
/// process, wait for it and return its exit code. `None` if that is not possible (the
/// caller then continues without Bluetooth).
pub fn respawn_as_responsible() -> Option<i32> {
    if is_disclaimed() {
        return None;
    }
    let disclaim = set_disclaim_fn()?;
    let exe = std::env::current_exe().ok()?;
    let exe_c = CString::new(exe.as_os_str().as_encoded_bytes()).ok()?;
    let args: Vec<CString> = std::env::args_os()
        .filter_map(|a| CString::new(a.as_encoded_bytes()).ok())
        .collect();
    let mut argv: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr() as *mut c_char).collect();
    argv.push(std::ptr::null_mut());
    let mut envs: Vec<CString> = std::env::vars_os()
        .filter_map(|(k, v)| {
            let mut s = k.as_encoded_bytes().to_vec();
            s.push(b'=');
            s.extend_from_slice(v.as_encoded_bytes());
            CString::new(s).ok()
        })
        .collect();
    envs.push(CString::new(format!("{DISCLAIMED_ENV}=1")).ok()?);
    let mut envp: Vec<*mut c_char> = envs.iter().map(|a| a.as_ptr() as *mut c_char).collect();
    envp.push(std::ptr::null_mut());

    unsafe {
        let mut attr: libc::posix_spawnattr_t = std::mem::zeroed();
        if libc::posix_spawnattr_init(&mut attr) != 0 {
            return None;
        }
        if disclaim(&mut attr, 1) != 0 {
            libc::posix_spawnattr_destroy(&mut attr);
            return None;
        }
        let mut pid: libc::pid_t = 0;
        let rc = libc::posix_spawn(&mut pid, exe_c.as_ptr(), std::ptr::null(), &attr, argv.as_ptr(), envp.as_ptr());
        libc::posix_spawnattr_destroy(&mut attr);
        if rc != 0 {
            return None;
        }
        let mut status: c_int = 0;
        loop {
            let r = libc::waitpid(pid, &mut status, 0);
            if r == pid {
                break;
            }
            if r == -1 && *libc::__error() != libc::EINTR {
                return Some(1);
            }
        }
        if libc::WIFEXITED(status) {
            Some(libc::WEXITSTATUS(status))
        } else {
            Some(128 + libc::WTERMSIG(status))
        }
    }
}
