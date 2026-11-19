// SPDX-License-Identifier: GPL-3.0-or-later
//! CoreBluetooth central restricted to peripherals that are already connected to this
//! Mac. There is deliberately no scanning API in this module:
//! `retrieveConnectedPeripheralsWithServices:` is the only way to obtain a peripheral.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicWriteType, CBManager, CBManagerAuthorization,
    CBManagerState, CBPeripheral, CBPeripheralDelegate, CBPeripheralState, CBService, CBUUID,
};
use objc2_foundation::{NSArray, NSData, NSError, NSObject, NSObjectProtocol, NSString};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BleError {
    #[error("the app has no NSBluetoothAlwaysUsageDescription (run from the app bundle)")]
    NoUsageDescription,
    #[error("Bluetooth access not authorized for battbar")]
    Unauthorized,
    #[error("Bluetooth is off")]
    PoweredOff,
    #[error("Bluetooth LE unavailable (state {0})")]
    Unavailable(isize),
    #[error("timeout waiting for {0}")]
    Timeout(&'static str),
    #[error("peripheral disconnected")]
    Disconnected,
    #[error("{0} not found")]
    NotFound(String),
    #[error("CoreBluetooth error: {0}")]
    Cb(String),
}

#[derive(Debug, Clone)]
enum PEvent {
    Connected,
    FailedToConnect(String),
    Disconnected,
    Services(Option<String>),
    Characteristics(String, Option<String>),
    Value(String, Vec<u8>, Option<String>),
    Written(String, Option<String>),
    NotifyState(String, Option<String>),
}

#[derive(Default)]
struct Router {
    state: Mutex<isize>,
    state_cv: Condvar,
    subscribers: Mutex<HashMap<String, Vec<Sender<PEvent>>>>,
}

impl Router {
    fn send(&self, id: &str, ev: PEvent) {
        if let Some(v) = self.subscribers.lock().unwrap().get_mut(id) {
            v.retain(|tx| tx.send(ev.clone()).is_ok());
        }
    }
}

fn pid(p: &CBPeripheral) -> String {
    unsafe { p.identifier().UUIDString().to_string() }
}

fn uuid_str(u: &CBUUID) -> String {
    unsafe { u.UUIDString().to_string().to_uppercase() }
}

fn err_str(e: Option<&NSError>) -> Option<String> {
    e.map(|e| e.localizedDescription().to_string())
}

pub struct DelegateIvars {
    router: Arc<Router>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the class has no Drop impl.
    #[unsafe(super(NSObject))]
    #[name = "BattbarBleDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBCentralManagerDelegate for Delegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            let r = &self.ivars().router;
            *r.state.lock().unwrap() = unsafe { central.state() }.0;
            r.state_cv.notify_all();
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _c: &CBCentralManager, p: &CBPeripheral) {
            self.ivars().router.send(&pid(p), PEvent::Connected);
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail(&self, _c: &CBCentralManager, p: &CBPeripheral, e: Option<&NSError>) {
            self.ivars()
                .router
                .send(&pid(p), PEvent::FailedToConnect(err_str(e).unwrap_or_default()));
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(&self, _c: &CBCentralManager, p: &CBPeripheral, _e: Option<&NSError>) {
            self.ivars().router.send(&pid(p), PEvent::Disconnected);
        }
    }

    unsafe impl CBPeripheralDelegate for Delegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(&self, p: &CBPeripheral, e: Option<&NSError>) {
            self.ivars().router.send(&pid(p), PEvent::Services(err_str(e)));
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_chars(&self, p: &CBPeripheral, s: &CBService, e: Option<&NSError>) {
            let su = uuid_str(&*unsafe { s.UUID() });
            self.ivars().router.send(&pid(p), PEvent::Characteristics(su, err_str(e)));
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_update_value(&self, p: &CBPeripheral, c: &CBCharacteristic, e: Option<&NSError>) {
            let cu = uuid_str(&*unsafe { c.UUID() });
            let v = unsafe { c.value() }.map(|d| d.to_vec()).unwrap_or_default();
            self.ivars().router.send(&pid(p), PEvent::Value(cu, v, err_str(e)));
        }

        #[unsafe(method(peripheral:didWriteValueForCharacteristic:error:))]
        fn did_write(&self, p: &CBPeripheral, c: &CBCharacteristic, e: Option<&NSError>) {
            let cu = uuid_str(&*unsafe { c.UUID() });
            self.ivars().router.send(&pid(p), PEvent::Written(cu, err_str(e)));
        }

        #[unsafe(method(peripheral:didUpdateNotificationStateForCharacteristic:error:))]
        fn did_notify_state(&self, p: &CBPeripheral, c: &CBCharacteristic, e: Option<&NSError>) {
            let cu = uuid_str(&*unsafe { c.UUID() });
            self.ivars().router.send(&pid(p), PEvent::NotifyState(cu, err_str(e)));
        }
    }
);

impl Delegate {
    fn new(router: Arc<Router>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { router });
        unsafe { msg_send![super(this), init] }
    }
}

/// Retained Objective-C object that we move between our worker threads. CoreBluetooth
/// objects are internally synchronised; all callbacks arrive on our private queue.
struct SendRetained<T>(Retained<T>);
unsafe impl<T> Send for SendRetained<T> {}
unsafe impl<T> Sync for SendRetained<T> {}
impl<T: objc2::Message> Clone for SendRetained<T> {
    fn clone(&self) -> Self {
        SendRetained(self.0.clone())
    }
}

pub struct Central {
    manager: SendRetained<CBCentralManager>,
    delegate: SendRetained<Delegate>,
    router: Arc<Router>,
    _queue: SendRetained<dispatch2::DispatchQueue>,
}

/// Peripheral connected to this Mac (obtained without scanning).
#[derive(Clone)]
pub struct Peripheral {
    /// CoreBluetooth identifier (random per Mac).
    pub id: String,
    pub name: Option<String>,
    p: SendRetained<CBPeripheral>,
}

impl std::fmt::Debug for Peripheral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peripheral")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish()
    }
}

pub fn authorization() -> isize {
    unsafe { CBManager::authorization_class() }.0
}

/// Bluetooth permission granted to this process (AllowedAlways).
pub fn is_authorized() -> bool {
    authorization() == CBManagerAuthorization::AllowedAlways.0
}

/// The user has not answered the permission prompt yet.
pub fn is_not_determined() -> bool {
    authorization() == CBManagerAuthorization::NotDetermined.0
}

impl Central {
    /// Process-wide central. Creating it may show the Bluetooth permission prompt.
    pub fn shared() -> Result<&'static Central, BleError> {
        static C: OnceLock<Result<Central, BleError>> = OnceLock::new();
        let c = C.get_or_init(Central::create).as_ref().map_err(|e| e.clone())?;
        // The first use shows the permission prompt; give the user time to answer.
        let wait = if is_not_determined() {
            Duration::from_secs(60)
        } else {
            Duration::from_secs(5)
        };
        c.ready(wait)?;
        Ok(c)
    }

    fn create() -> Result<Central, BleError> {
        if !crate::tcc::bluetooth_usage_description_present() {
            return Err(BleError::NoUsageDescription);
        }
        if authorization() == CBManagerAuthorization::Denied.0 || authorization() == CBManagerAuthorization::Restricted.0 {
            return Err(BleError::Unauthorized);
        }
        let router = Arc::new(Router::default());
        let queue = dispatch2::DispatchQueue::new("io.github.battbar.ble", None);
        let delegate = Delegate::new(router.clone());
        let manager = unsafe {
            CBCentralManager::initWithDelegate_queue(CBCentralManager::alloc(), Some(ProtocolObject::from_ref(&*delegate)), Some(&queue))
        };
        let queue: Retained<dispatch2::DispatchQueue> =
            unsafe { Retained::from_raw(dispatch2::DispatchRetained::into_raw(queue).as_ptr()) }.expect("queue");
        Ok(Central {
            manager: SendRetained(manager),
            delegate: SendRetained(delegate),
            router,
            _queue: SendRetained(queue),
        })
    }

    fn ready(&self, timeout: Duration) -> Result<(), BleError> {
        let deadline = Instant::now() + timeout;
        let mut st = self.router.state.lock().unwrap();
        // 0 unknown, 1 resetting: wait for the first real state.
        while *st <= 1 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(BleError::Timeout("Bluetooth state"));
            }
            st = self.router.state_cv.wait_timeout(st, left).unwrap().0;
        }
        match *st {
            s if s == CBManagerState::PoweredOn.0 => Ok(()),
            s if s == CBManagerState::Unauthorized.0 => Err(BleError::Unauthorized),
            s if s == CBManagerState::PoweredOff.0 => Err(BleError::PoweredOff),
            s => Err(BleError::Unavailable(s)),
        }
    }

    /// Peripherals currently connected to this Mac that expose any of `services`.
    pub fn connected_peripherals(&self, services: &[&str]) -> Result<Vec<Peripheral>, BleError> {
        self.ready(Duration::from_secs(2))?;
        let uuids: Vec<Retained<CBUUID>> = services
            .iter()
            .map(|s| unsafe { CBUUID::UUIDWithString(&NSString::from_str(s)) })
            .collect();
        let arr = NSArray::from_retained_slice(&uuids);
        let list = unsafe { self.manager.0.retrieveConnectedPeripheralsWithServices(&arr) };
        Ok(list
            .iter()
            .map(|p| {
                unsafe { p.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate.0))) };
                Peripheral {
                    id: pid(&p),
                    name: unsafe { p.name() }.map(|n| n.to_string()),
                    p: SendRetained(p),
                }
            })
            .collect())
    }
}

#[derive(Clone)]
pub struct Characteristic {
    pub uuid: String,
    c: SendRetained<CBCharacteristic>,
}

/// App-level GATT access to one connected peripheral.
pub struct GattSession {
    central: &'static Central,
    peripheral: Peripheral,
    rx: Receiver<PEvent>,
    values: VecDeque<(String, Vec<u8>)>,
    pub timeout: Duration,
}

impl GattSession {
    pub fn open(central: &'static Central, peripheral: &Peripheral) -> Result<Self, BleError> {
        let (tx, rx) = mpsc::channel();
        central
            .router
            .subscribers
            .lock()
            .unwrap()
            .entry(peripheral.id.clone())
            .or_default()
            .push(tx);
        let mut s = GattSession {
            central,
            peripheral: peripheral.clone(),
            rx,
            values: VecDeque::new(),
            timeout: Duration::from_millis(2000),
        };
        let p = &s.peripheral.p.0;
        if unsafe { p.state() } != CBPeripheralState::Connected {
            unsafe { s.central.manager.0.connectPeripheral_options(p, None) };
            s.wait("connection", |e| match e {
                PEvent::Connected => Some(Ok(())),
                PEvent::FailedToConnect(m) => Some(Err(BleError::Cb(m.clone()))),
                _ => None,
            })?;
        }
        Ok(s)
    }

    pub fn peripheral(&self) -> &Peripheral {
        &self.peripheral
    }

    fn wait<R>(&mut self, what: &'static str, mut f: impl FnMut(&PEvent) -> Option<Result<R, BleError>>) -> Result<R, BleError> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(ev) => {
                    if let Some(r) = f(&ev) {
                        return r;
                    }
                    match ev {
                        PEvent::Value(c, v, None) => {
                            if self.values.len() > 256 {
                                self.values.pop_front();
                            }
                            self.values.push_back((c, v));
                        }
                        PEvent::Disconnected => return Err(BleError::Disconnected),
                        _ => {}
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Err(BleError::Timeout(what)),
                Err(RecvTimeoutError::Disconnected) => return Err(BleError::Disconnected),
            }
        }
    }

    fn find_service(&self, uuid: &str) -> Option<Retained<CBService>> {
        unsafe { self.peripheral.p.0.services() }?
            .iter()
            .find(|s| uuid_str(&*unsafe { s.UUID() }) == uuid)
    }

    pub fn characteristic(&mut self, service: &str, characteristic: &str) -> Result<Characteristic, BleError> {
        let (su, cu) = (service.to_uppercase(), characteristic.to_uppercase());
        let svc = match self.find_service(&su) {
            Some(s) => s,
            None => {
                let arr = NSArray::from_retained_slice(&[unsafe { CBUUID::UUIDWithString(&NSString::from_str(&su)) }]);
                unsafe { self.peripheral.p.0.discoverServices(Some(&arr)) };
                self.wait("services", |e| match e {
                    PEvent::Services(None) => Some(Ok(())),
                    PEvent::Services(Some(m)) => Some(Err(BleError::Cb(m.clone()))),
                    _ => None,
                })?;
                self.find_service(&su).ok_or_else(|| BleError::NotFound(format!("service {su}")))?
            }
        };
        let find = |s: &CBService| unsafe { s.characteristics() }.and_then(|cs| cs.iter().find(|c| uuid_str(&*unsafe { c.UUID() }) == cu));
        let c = match find(&svc) {
            Some(c) => c,
            None => {
                let arr = NSArray::from_retained_slice(&[unsafe { CBUUID::UUIDWithString(&NSString::from_str(&cu)) }]);
                unsafe { self.peripheral.p.0.discoverCharacteristics_forService(Some(&arr), &svc) };
                let su2 = su.clone();
                self.wait("characteristics", |e| match e {
                    PEvent::Characteristics(s, None) if *s == su2 => Some(Ok(())),
                    PEvent::Characteristics(s, Some(m)) if *s == su2 => Some(Err(BleError::Cb(m.clone()))),
                    _ => None,
                })?;
                find(&svc).ok_or_else(|| BleError::NotFound(format!("characteristic {cu}")))?
            }
        };
        Ok(Characteristic {
            uuid: cu,
            c: SendRetained(c),
        })
    }

    pub fn read(&mut self, ch: &Characteristic) -> Result<Vec<u8>, BleError> {
        unsafe { self.peripheral.p.0.readValueForCharacteristic(&ch.c.0) };
        let u = ch.uuid.clone();
        self.wait("read", |e| match e {
            PEvent::Value(c, v, None) if *c == u => Some(Ok(v.clone())),
            PEvent::Value(c, _, Some(m)) if *c == u => Some(Err(BleError::Cb(m.clone()))),
            _ => None,
        })
    }

    pub fn write(&mut self, ch: &Characteristic, data: &[u8]) -> Result<(), BleError> {
        let d = NSData::with_bytes(data);
        unsafe {
            self.peripheral
                .p
                .0
                .writeValue_forCharacteristic_type(&d, &ch.c.0, CBCharacteristicWriteType::WithResponse)
        };
        let u = ch.uuid.clone();
        self.wait("write", |e| match e {
            PEvent::Written(c, None) if *c == u => Some(Ok(())),
            PEvent::Written(c, Some(m)) if *c == u => Some(Err(BleError::Cb(m.clone()))),
            _ => None,
        })
    }

    pub fn subscribe(&mut self, ch: &Characteristic) -> Result<(), BleError> {
        unsafe { self.peripheral.p.0.setNotifyValue_forCharacteristic(true, &ch.c.0) };
        let u = ch.uuid.clone();
        self.wait("subscribe", |e| match e {
            PEvent::NotifyState(c, None) if *c == u => Some(Ok(())),
            PEvent::NotifyState(c, Some(m)) if *c == u => Some(Err(BleError::Cb(m.clone()))),
            _ => None,
        })
    }

    /// Next value notification for `ch` (buffered ones first).
    pub fn next_value(&mut self, ch: &Characteristic, timeout: Duration) -> Result<Option<Vec<u8>>, BleError> {
        if let Some(pos) = self.values.iter().position(|(c, _)| *c == ch.uuid) {
            return Ok(self.values.remove(pos).map(|(_, v)| v));
        }
        let saved = self.timeout;
        self.timeout = timeout;
        let u = ch.uuid.clone();
        let r = self.wait("notification", |e| match e {
            PEvent::Value(c, v, None) if *c == u => Some(Ok(v.clone())),
            _ => None,
        });
        self.timeout = saved;
        match r {
            Ok(v) => Ok(Some(v)),
            Err(BleError::Timeout(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Drop for GattSession {
    fn drop(&mut self) {
        // Unsubscribe our channel; the system keeps its own HID connection, so the
        // app-level connection is left alone.
        if let Some(v) = self.central.router.subscribers.lock().unwrap().get_mut(&self.peripheral.id) {
            v.retain(|tx| tx.send(PEvent::Services(None)).is_ok());
        }
    }
}
