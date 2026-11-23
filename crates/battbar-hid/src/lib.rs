// SPDX-License-Identifier: GPL-3.0-or-later
//! The only place in battbar that touches `hidapi`.
//!
//! Privacy guard (docs/SPEC.md, "Privacy and security"): an OS-level HID device is
//! opened only if it is on USB and **every** top-level collection is vendor-defined
//! (usage page >= 0xFF00). Keyboard, mouse, consumer and any Bluetooth HID device are
//! refused before `hidapi` is asked to open anything.

use battbar_core::transport::{FrameTransport, LinkInfo, TransportError};
use hidapi::{BusType, HidApi, HidDevice};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const VENDOR_USAGE_PAGE_MIN: u16 = 0xFF00;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Usb,
    Bluetooth,
    Other,
}

/// One OS-level HID device (on macOS: one `IOHIDDevice`) with all its collections.
#[derive(Debug, Clone, PartialEq)]
pub struct HidInterface {
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus: Bus,
    pub interface_number: i32,
    pub release: u16,
    pub product: String,
    pub manufacturer: String,
    pub path: CString,
    /// Top-level collections `(usage_page, usage)`.
    pub collections: Vec<(u16, u16)>,
}

impl HidInterface {
    pub fn is_vendor_only(&self) -> bool {
        !self.collections.is_empty() && self.collections.iter().all(|(p, _)| *p >= VENDOR_USAGE_PAGE_MIN)
    }

    pub fn has_usage_page(&self, page: u16) -> bool {
        self.collections.iter().any(|(p, _)| *p == page)
    }

    /// Stable link ID within this machine session (no serials).
    pub fn link_id(&self) -> String {
        format!(
            "hid:{:04x}:{:04x}:{}:{}",
            self.vendor_id,
            self.product_id,
            self.interface_number,
            self.collections.first().map(|c| format!("{:04x}", c.0)).unwrap_or_default()
        )
    }

    pub fn link_info(&self) -> LinkInfo {
        LinkInfo {
            link: self.link_id(),
            kind: "hidapi".into(),
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            product: self.product.clone(),
            bus: match self.bus {
                Bus::Usb => "usb",
                Bus::Bluetooth => "bluetooth",
                Bus::Other => "other",
            }
            .into(),
            collections: self.collections.clone(),
        }
    }
}

/// Why an interface may not be opened. Pure function, unit-tested.
pub fn check_openable(i: &HidInterface) -> Result<(), String> {
    if i.bus == Bus::Bluetooth {
        return Err("Bluetooth HID devices are never opened (keyboard/mouse reports share the device)".into());
    }
    if i.bus != Bus::Usb {
        return Err("only USB HID devices are opened".into());
    }
    if i.collections.is_empty() {
        return Err("collections unknown".into());
    }
    if let Some((p, u)) = i.collections.iter().find(|(p, _)| *p < VENDOR_USAGE_PAGE_MIN) {
        return Err(format!("interface carries non-vendor collection 0x{p:04x}/0x{u:04x}"));
    }
    Ok(())
}

fn new_api() -> Result<HidApi, String> {
    HidApi::new()
        .inspect(|_a| {
            // hidapi opens devices exclusively on macOS by default; that would break
            // G HUB / GG and the vendor apps would break us.
            #[cfg(target_os = "macos")]
            _a.set_open_exclusive(false);
        })
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ HID service
//
// On macOS hidapi schedules its IOHIDManager on the run loop of the thread that creates
// it (`IOHIDManagerScheduleWithRunLoop(…, CFRunLoopGetCurrent(), …)`), and every
// enumeration schedules devices on that run loop. battbar 0.1.0 created it on a provider
// thread; when that thread exited, new threads enumerating while its run loop was torn
// down crashed in CFRunLoopAddSource.
//
// Rule (docs/DEVELOPMENT.md): system objects bound to a run loop live on long-lived
// threads that run that loop. Here one service thread owns the HidApi for the whole
// process: every hidapi call (enumerate, open) runs on it as a job, and between jobs it
// waits in its run loop, so IOKit events queued on the manager are handled instead of
// piling up — without waking up when nothing happens.

type Job = Box<dyn FnOnce(&mut Result<HidApi, String>) + Send>;

struct Service {
    tx: Mutex<std::sync::mpsc::Sender<Job>>,
    /// Wakes the service thread (macOS: signals its run loop source).
    waker: Waker,
}

static RUN_LOOP_TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How often the service thread's run loop has returned (once per wake-up: a job or an
/// IOKit event). Tests check that it wakes for jobs and stays asleep otherwise.
pub fn service_run_loop_ticks() -> u64 {
    RUN_LOOP_TICKS.load(std::sync::atomic::Ordering::Relaxed)
}

// The service thread sleeps in its run loop until IOKit has an event for the HID
// manager or a job arrives — no periodic wake-ups (0.3 polled every 20 ms). A job
// signals a run loop source of its own: a signal stays pending until the source is
// handled, so a job sent just before the thread starts waiting is not lost (a
// CFRunLoopStop at that moment would be).
#[cfg(target_os = "macos")]
mod cf {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct SourceContext {
        pub version: isize,
        pub info: *mut c_void,
        pub retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
        pub release: Option<extern "C" fn(*const c_void)>,
        pub copy_description: Option<extern "C" fn(*const c_void) -> *const c_void>,
        pub equal: Option<extern "C" fn(*const c_void, *const c_void) -> u8>,
        pub hash: Option<extern "C" fn(*const c_void) -> usize>,
        pub schedule: Option<extern "C" fn(*mut c_void, *const c_void, *const c_void)>,
        pub cancel: Option<extern "C" fn(*mut c_void, *const c_void, *const c_void)>,
        pub perform: Option<extern "C" fn(*mut c_void)>,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub static kCFRunLoopDefaultMode: *const c_void;
        pub fn CFRunLoopGetCurrent() -> *const c_void;
        pub fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: u8) -> i32;
        pub fn CFRunLoopSourceCreate(allocator: *const c_void, order: isize, context: *mut SourceContext) -> *const c_void;
        pub fn CFRunLoopAddSource(rl: *const c_void, source: *const c_void, mode: *const c_void);
        pub fn CFRunLoopSourceSignal(source: *const c_void);
        pub fn CFRunLoopWakeUp(rl: *const c_void);
    }

    extern "C" fn nothing(_info: *mut c_void) {}

    /// Add a source that only wakes this thread's run loop; returns (run loop, source).
    /// Both live for the whole process (the service thread never exits).
    pub fn install_wake_source() -> (usize, usize) {
        let mut ctx = SourceContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: None,
            release: None,
            copy_description: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(nothing),
        };
        unsafe {
            let rl = CFRunLoopGetCurrent();
            let src = CFRunLoopSourceCreate(std::ptr::null(), 0, &mut ctx);
            CFRunLoopAddSource(rl, src, kCFRunLoopDefaultMode);
            (rl as usize, src as usize)
        }
    }

    /// Sleep until an IOKit event or [`wake`]. The own source keeps the run loop from
    /// being empty, so this never returns at once for lack of sources.
    pub fn wait() {
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 1.0e10, 1) };
    }

    pub fn wake(rl: usize, src: usize) {
        unsafe {
            CFRunLoopSourceSignal(src as *const c_void);
            CFRunLoopWakeUp(rl as *const c_void);
        }
    }
}

struct Waker {
    #[cfg(target_os = "macos")]
    run_loop: usize,
    #[cfg(target_os = "macos")]
    source: usize,
}

impl Waker {
    fn wake(&self) {
        #[cfg(target_os = "macos")]
        cf::wake(self.run_loop, self.source);
    }
}

fn service() -> &'static Service {
    static SERVICE: OnceLock<Service> = OnceLock::new();
    SERVICE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Waker>(1);
        std::thread::Builder::new()
            .name("battbar-hid-service".into())
            .spawn(move || {
                #[cfg(target_os = "macos")]
                let waker = {
                    let (run_loop, source) = cf::install_wake_source();
                    Waker { run_loop, source }
                };
                #[cfg(not(target_os = "macos"))]
                let waker = Waker {};
                // HID hotplug notifications on this thread's run loop (see `on_hotplug`).
                #[cfg(target_os = "macos")]
                hotplug::install();
                let _ = ready_tx.send(waker);
                let mut api = new_api();
                // IOHIDLib (hidapi's open/enumerate, the hotplug callbacks) is partly
                // Objective-C and autoreleases objects: every round of jobs and run-loop
                // events runs in its own autorelease pool, or those objects would
                // accumulate for the life of the thread (seen: +2 NSData per open).
                loop {
                    let stop = battbar_core::objc_pool::with_pool(|| {
                        // Every job sent so far, then sleep until the next event or job.
                        loop {
                            match rx.try_recv() {
                                Ok(job) => job(&mut api),
                                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => return true,
                            }
                        }
                        #[cfg(target_os = "macos")]
                        cf::wait();
                        #[cfg(not(target_os = "macos"))]
                        match rx.recv() {
                            Ok(job) => job(&mut api),
                            Err(_) => return true,
                        }
                        false
                    });
                    if stop {
                        return;
                    }
                    RUN_LOOP_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            })
            .expect("spawn HID service thread");
        let waker = ready_rx.recv().expect("HID service thread started");
        Service { tx: Mutex::new(tx), waker }
    })
}

// ------------------------------------------------------------------ hotplug
//
// A second IOHIDManager on the service thread's run loop, matching every HID device and
// **never opening any** (like hidapi's own manager; no Input Monitoring needed): its
// matching/removal callbacks say "a HID device appeared or disappeared" — a receiver or
// base station on USB, and also Bluetooth LE / classic HID devices (keyboards, mice,
// gamepads), which macOS represents as HID devices too. Listeners only get that signal
// and re-check; nothing about the device is read here. On registration the manager
// reports every device already present once.

static HOTPLUG_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
type Listener = Box<dyn Fn() + Send>;
static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());

/// HID devices that appeared or disappeared so far (the initial report of devices
/// present at start included).
pub fn hotplug_events() -> u64 {
    HOTPLUG_EVENTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Call `f` whenever a HID device appears or disappears (on the HID service thread;
/// keep it short, e.g. send on a channel). Starts the service thread if needed.
pub fn on_hotplug(f: impl Fn() + Send + 'static) {
    LISTENERS.lock().unwrap().push(Box::new(f));
    let _ = service();
}

fn hotplug_event() {
    HOTPLUG_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Ok(ls) = LISTENERS.lock() {
        for l in ls.iter() {
            l();
        }
    }
}

#[cfg(target_os = "macos")]
mod hotplug {
    use std::ffi::c_void;

    type Callback = extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void);

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOHIDManagerCreate(allocator: *const c_void, options: u32) -> *mut c_void;
        fn IOHIDManagerSetDeviceMatching(manager: *mut c_void, matching: *const c_void);
        fn IOHIDManagerRegisterDeviceMatchingCallback(manager: *mut c_void, callback: Callback, context: *mut c_void);
        fn IOHIDManagerRegisterDeviceRemovalCallback(manager: *mut c_void, callback: Callback, context: *mut c_void);
        fn IOHIDManagerScheduleWithRunLoop(manager: *mut c_void, run_loop: *const c_void, mode: *const c_void);
    }

    extern "C" fn changed(_ctx: *mut c_void, _result: i32, _sender: *mut c_void, _device: *mut c_void) {
        super::hotplug_event();
    }

    /// On the service thread, once. The manager lives for the whole process.
    pub fn install() {
        unsafe {
            let m = IOHIDManagerCreate(std::ptr::null(), 0);
            if m.is_null() {
                return;
            }
            IOHIDManagerSetDeviceMatching(m, std::ptr::null()); // every HID device
            IOHIDManagerRegisterDeviceMatchingCallback(m, changed, std::ptr::null_mut());
            IOHIDManagerRegisterDeviceRemovalCallback(m, changed, std::ptr::null_mut());
            IOHIDManagerScheduleWithRunLoop(m, super::cf::CFRunLoopGetCurrent(), super::cf::kCFRunLoopDefaultMode);
        }
    }
}

/// Run `f` on the HID service thread and wait for its result.
fn with_api<R: Send + 'static>(f: impl FnOnce(&mut HidApi) -> Result<R, TransportError> + Send + 'static) -> Result<R, TransportError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let job: Job = Box::new(move |api| {
        let r = match api {
            Ok(a) => f(a),
            Err(e) => Err(TransportError::Io(e.clone())),
        };
        let _ = tx.send(r);
    });
    let svc = service();
    svc.tx
        .lock()
        .unwrap()
        .send(job)
        .map_err(|_| TransportError::Io("HID service stopped".into()))?;
    svc.waker.wake();
    rx.recv().map_err(|_| TransportError::Io("HID service stopped".into()))?
}

/// All HID devices, grouped per OS-level device. Nothing is opened.
pub fn enumerate() -> Result<Vec<HidInterface>, TransportError> {
    with_api(enumerate_on_service)
}

fn enumerate_on_service(a: &mut HidApi) -> Result<Vec<HidInterface>, TransportError> {
    a.refresh_devices().map_err(|e| TransportError::Io(e.to_string()))?;
    let mut by_path: BTreeMap<CString, HidInterface> = BTreeMap::new();
    for d in a.device_list() {
        let e = by_path.entry(d.path().to_owned()).or_insert_with(|| HidInterface {
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            bus: match d.bus_type() {
                BusType::Usb => Bus::Usb,
                BusType::Bluetooth => Bus::Bluetooth,
                _ => Bus::Other,
            },
            interface_number: d.interface_number(),
            release: d.release_number(),
            product: d.product_string().unwrap_or("").to_string(),
            manufacturer: d.manufacturer_string().unwrap_or("").to_string(),
            path: d.path().to_owned(),
            collections: vec![],
        });
        e.collections.push((d.usage_page(), d.usage()));
    }
    Ok(by_path.into_values().filter(|i| i.vendor_id != 0).collect())
}

pub struct HidapiTransport {
    dev: HidDevice,
    info: LinkInfo,
    buf: Vec<u8>,
}

/// Opens a HID interface after the privacy guard approved it.
pub fn open(i: &HidInterface) -> Result<HidapiTransport, TransportError> {
    check_openable(i).map_err(TransportError::Refused)?;
    let path = i.path.clone();
    let dev = with_api(move |a| a.open_path(&path).map_err(|e| TransportError::Io(e.to_string())))?;
    Ok(HidapiTransport {
        dev,
        info: i.link_info(),
        buf: vec![0u8; 256],
    })
}

impl FrameTransport for HidapiTransport {
    fn info(&self) -> &LinkInfo {
        &self.info
    }
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        self.dev.write(frame).map(|_| ()).map_err(|e| map_err(e.to_string()))
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, TransportError> {
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        match self.dev.read_timeout(&mut self.buf, ms) {
            Ok(0) => Ok(None),
            Ok(n) => Ok(Some(self.buf[..n].to_vec())),
            Err(e) => Err(map_err(e.to_string())),
        }
    }
}

fn map_err(s: String) -> TransportError {
    let l = s.to_lowercase();
    if l.contains("disconnect") || l.contains("not connected") || l.contains("no such device") || l.contains("0xe00002c0") {
        TransportError::Disconnected
    } else {
        TransportError::Io(s)
    }
}

/// Input Monitoring (TCC "ListenEvent") status of this process: `granted`, `denied`
/// or `unknown` (never asked). battbar never requests it; this is reported in
/// diagnostics to show that reading vendor interfaces does not depend on it.
#[cfg(target_os = "macos")]
pub fn input_monitoring_status() -> &'static str {
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOHIDCheckAccess(request_type: u32) -> u32;
    }
    // kIOHIDRequestTypeListenEvent = 1; kIOHIDAccessTypeGranted = 0, Denied = 1, Unknown = 2.
    match unsafe { IOHIDCheckAccess(1) } {
        0 => "granted",
        1 => "denied",
        _ => "unknown",
    }
}

#[cfg(not(target_os = "macos"))]
pub fn input_monitoring_status() -> &'static str {
    "n/a"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(bus: Bus, collections: Vec<(u16, u16)>) -> HidInterface {
        HidInterface {
            vendor_id: 0x046d,
            product_id: 0xc547,
            bus,
            interface_number: 0,
            release: 0,
            product: "t".into(),
            manufacturer: "t".into(),
            // A path that does not exist: if the guard ever let it through, open would
            // fail with Io, not Refused, and the tests below would catch it.
            path: CString::new("DevSrvsID:0").unwrap(),
            collections,
        }
    }

    fn refused(i: &HidInterface) -> bool {
        matches!(open(i), Err(TransportError::Refused(_)))
    }

    #[test]
    fn keyboard_and_mouse_usage_pages_are_never_opened() {
        assert!(refused(&iface(Bus::Usb, vec![(0x01, 0x06)])), "keyboard");
        assert!(refused(&iface(Bus::Usb, vec![(0x01, 0x02)])), "mouse");
        assert!(refused(&iface(Bus::Usb, vec![(0x07, 0x00)])), "keyboard page");
        assert!(refused(&iface(Bus::Usb, vec![(0x0c, 0x01)])), "consumer");
    }

    #[test]
    fn mixed_interfaces_are_never_opened() {
        assert!(refused(&iface(Bus::Usb, vec![(0xff00, 0x01), (0x01, 0x06)])));
        assert!(refused(&iface(Bus::Usb, vec![(0x01, 0x80), (0xff43, 0x0202)])));
    }

    #[test]
    fn bluetooth_hid_devices_are_never_opened() {
        // Even a vendor-only Bluetooth HID device: on macOS a BT peripheral is one
        // IOHIDDevice and HID++ goes over GATT instead.
        assert!(refused(&iface(Bus::Bluetooth, vec![(0xff43, 0x0202)])));
        assert!(refused(&iface(Bus::Bluetooth, vec![(0x01, 0x06), (0xff43, 0x0202)])));
    }

    #[test]
    fn unknown_collections_are_never_opened() {
        assert!(refused(&iface(Bus::Usb, vec![])));
        assert!(refused(&iface(Bus::Other, vec![(0xff00, 1)])));
    }

    #[test]
    fn vendor_only_usb_passes_the_guard() {
        assert!(check_openable(&iface(Bus::Usb, vec![(0xff00, 1), (0xff00, 2)])).is_ok());
        assert!(check_openable(&iface(Bus::Usb, vec![(0xffc0, 1), (0xff00, 1)])).is_ok());
    }
}
