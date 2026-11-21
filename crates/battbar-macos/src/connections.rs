// SPDX-License-Identifier: GPL-3.0-or-later
//! Bluetooth connect / disconnect notifications (IOBluetooth), so a headset or other
//! classic device that connects or goes away is noticed at once instead of by a
//! periodic list.
//!
//! IOBluetooth delivers them on the run loop of the thread that registered: register on
//! the **main thread** (long-lived, runs its run loop; docs/DEVELOPMENT.md "Threads, run
//! loops and queues"). Requires Bluetooth permission (IOBluetooth blocks while the
//! prompt is open), so callers register only once it is granted. Nothing is read from
//! the devices here; listeners only learn "something connected / disconnected".

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_io_bluetooth::{IOBluetoothDevice, IOBluetoothUserNotification};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

type Listener = Box<dyn Fn(bool)>;

thread_local! {
    // Main thread only.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    observer: Retained<Observer>,
    /// Kept alive for the whole process.
    connect: Option<Retained<IOBluetoothUserNotification>>,
    /// One per connected device (disconnect notification).
    disconnects: Vec<Retained<IOBluetoothUserNotification>>,
    listeners: Vec<Listener>,
}

static EVENTS: AtomicU64 = AtomicU64::new(0);

/// Connect + disconnect notifications received so far (those for devices already
/// connected at registration included).
pub fn events() -> u64 {
    EVENTS.load(Ordering::Relaxed)
}

fn notify(connected: bool) {
    EVENTS.fetch_add(1, Ordering::Relaxed);
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            for l in &st.listeners {
                l(connected);
            }
        }
    });
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Observer has no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "BattbarBluetoothConnectionObserver"]
    struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    impl Observer {
        #[unsafe(method(connected:device:))]
        fn connected(&self, _note: &AnyObject, device: &IOBluetoothDevice) {
            let n = unsafe { device.registerForDisconnectNotification_selector(Some(self), Some(sel!(disconnected:device:))) };
            STATE.with(|s| {
                if let (Some(st), Some(n)) = (s.borrow_mut().as_mut(), n) {
                    st.disconnects.push(n);
                }
            });
            notify(true);
        }

        #[unsafe(method(disconnected:device:))]
        fn disconnected(&self, note: &IOBluetoothUserNotification, _device: &AnyObject) {
            unsafe { note.unregister() };
            STATE.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    st.disconnects.retain(|n| !std::ptr::eq(&**n, note));
                }
            });
            notify(false);
        }
    }
);

impl Observer {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Call `f(connected)` on the main thread whenever a Bluetooth device connects or
/// disconnects. The first call registers with IOBluetooth (only with Bluetooth
/// permission); later calls add listeners.
pub fn watch(mtm: MainThreadMarker, f: impl Fn(bool) + 'static) {
    let first = STATE.with(|s| {
        let mut s = s.borrow_mut();
        match s.as_mut() {
            Some(st) => {
                st.listeners.push(Box::new(f));
                false
            }
            None => {
                *s = Some(State {
                    observer: Observer::new(mtm),
                    connect: None,
                    disconnects: vec![],
                    listeners: vec![Box::new(f)],
                });
                true
            }
        }
    });
    if !first {
        return;
    }
    // IOBluetooth may call back right away (devices already connected): no borrow of
    // STATE may be held while registering.
    let Some(observer) = STATE.with(|s| s.borrow().as_ref().map(|st| st.observer.clone())) else {
        return;
    };
    let observer_obj: &AnyObject = observer.as_ref();
    let token = unsafe { IOBluetoothDevice::registerForConnectNotifications_selector(Some(observer_obj), Some(sel!(connected:device:))) };
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.connect = token;
        }
    });
}

/// Whether [`watch`] has registered (main thread).
pub fn watching() -> bool {
    STATE.with(|s| s.borrow().is_some())
}
