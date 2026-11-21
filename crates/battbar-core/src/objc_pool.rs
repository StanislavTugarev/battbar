// SPDX-License-Identifier: GPL-3.0-or-later
//! Autorelease pool for one unit of work on a thread that is not the main thread.
//!
//! Objective-C APIs (CoreBluetooth, IOBluetooth, Foundation) autorelease objects into
//! the innermost pool of the calling thread. The main thread's run loop drains its pool
//! after every event, GCD queues after every block; a plain thread has none, so each
//! loop iteration of a long-lived thread that may call those APIs runs in its own pool
//! (otherwise autoreleased objects pile up for the life of the thread).
//! No Objective-C dependency here: the runtime's two C entry points are enough.

/// Run `f` inside a fresh autorelease pool (drained when `f` returns or panics).
/// On other platforms just runs `f`.
pub fn with_pool<R>(f: impl FnOnce() -> R) -> R {
    #[cfg(target_os = "macos")]
    {
        let _pool = Pool::push();
        f()
    }
    #[cfg(not(target_os = "macos"))]
    {
        f()
    }
}

#[cfg(target_os = "macos")]
struct Pool(*mut std::ffi::c_void);

#[cfg(target_os = "macos")]
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_autoreleasePoolPush() -> *mut std::ffi::c_void;
    fn objc_autoreleasePoolPop(pool: *mut std::ffi::c_void);
}

#[cfg(target_os = "macos")]
impl Pool {
    fn push() -> Self {
        // SAFETY: plain runtime calls; the token is popped exactly once, on this thread
        // (the guard is not Send), in LIFO order with any pool `f` opens itself.
        Pool(unsafe { objc_autoreleasePoolPush() })
    }
}

#[cfg(target_os = "macos")]
impl Drop for Pool {
    fn drop(&mut self) {
        // SAFETY: see `push`.
        unsafe { objc_autoreleasePoolPop(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_value_and_nests() {
        assert_eq!(with_pool(|| with_pool(|| 41) + 1), 42);
    }

    #[test]
    fn drained_on_panic() {
        let r = std::panic::catch_unwind(|| with_pool(|| panic!("inside")));
        assert!(r.is_err());
        assert_eq!(with_pool(|| 1), 1, "the pool stack is still balanced");
    }
}
