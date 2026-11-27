// SPDX-License-Identifier: GPL-3.0-or-later
//! battbar menu bar app: one status item ("island") with every visible device, or one
//! icon (collapsed); one menu with a section per device.

mod alerts;
mod bar;
mod menubar;
mod overflow;
mod probe;
mod ui;
mod updates;
mod worker;

use alerts::{Alert, AlertTracker};
use battbar_core::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSButton, NSCellImagePosition, NSControlStateValueOff,
    NSControlStateValueOn, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSPasteboard, NSPasteboardTypeString, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSWorkspace,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURL};
use std::cell::RefCell;
use std::sync::mpsc::Sender;
use std::time::Duration;
use worker::{PollUpdate, WorkerCmd};

const URL_BLUETOOTH_PRIVACY: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth";

const THRESHOLDS: [Option<u8>; 5] = [None, Some(10), Some(15), Some(20), Some(25)];
const INTERVALS: [u64; 4] = [30, 60, 120, 300];

#[derive(Debug, Clone)]
enum Action {
    Refresh,
    Quit,
    ToggleLaunchAtLogin,
    SetThreshold(Option<u8>),
    SetInterval(u64),
    ToggleSpareNotify,
    ToggleShowUnreported,
    ToggleHeadsetcontrol,
    Hide(String),
    Show(String),
    Split(String),
    MergeWith(String, String),
    Unlink(String),
    OpenBluetoothSettings,
    OpenNotificationSettings,
    OpenLoginItems,
    /// Running translocated: reveal the real location so the user can move the app.
    MoveToApplications,
    SetChemistry(String, Chemistry),
    ForgetCalibration(String),
    ToggleUpdateCheck,
    /// "Update available: vX.Y.Z" (version without the `v`).
    ShowUpdate(String),
    SetDisplayMode(DisplayMode),
    /// Move a device one place left (−1) or right (+1) in the menu bar and menu.
    MoveDevice(String, isize),
    ResetDeviceOrder,
}

/// Work that must run after the app state borrow is released (modal UI, terminate).
enum Post {
    Quit,
    BluetoothAlert,
    Message(String),
    /// First-run question (asked once).
    AskLaunchAtLogin,
    /// Running from a temporary (translocated) copy: explain how to move the app.
    TranslocationNotice,
    /// Launch at Login needs approval or cannot find the app: explain, offer the pane.
    LoginItemHelp(login::Status),
    /// First-run offer of the update check, for installs that already answered the
    /// Launch at Login question (it is part of that question otherwise).
    AskUpdateCheck,
    /// Installed with Homebrew: show the upgrade command.
    UpdateHelp(String),
    /// Settings switch (runs outside the app borrow, like the first-run answer).
    SetUpdateCheck(bool),
}

struct App {
    mtm: MainThreadMarker,
    handler: Retained<Handler>,
    settings: Settings,
    reports: Vec<DeviceReport>,
    devices: Vec<Device>,
    item: Option<Retained<NSStatusItem>>,
    /// Expanded item hidden by macOS → back to Collapsed (see overflow.rs).
    overflow: overflow::OverflowGuard,
    /// What the item showed last (layout + segments), to restart the overflow check
    /// when it changes.
    bar_key: String,
    /// The automatic "not enough space" fallback works on this macOS (26 and older).
    auto_collapse: bool,
    /// The overflow check's one-shot timer (none while the item is shown and settled).
    overflow_timer: Option<Retained<objc2_foundation::NSTimer>>,
    /// Occlusion notifications of the item's window: (window address, observer token).
    /// Observers of the item's window (occlusion, move), by window address.
    occlusion_watch: Option<(usize, Vec<Observer>)>,
    /// IDs of the devices shown in the menu bar / device sections, in display order.
    shown: Vec<String>,
    actions: Vec<Action>,
    tx: Sender<WorkerCmd>,
    alerts: AlertTracker,
    identities: IdentityMemory,
    gate: NewDeviceGate,
    db: DeviceDb,
    settling: SettlingTracker,
    /// Device IDs the new-device gate has admitted.
    admitted: Vec<String>,
    last_update: Option<String>,
    timed_out: Vec<&'static str>,
    bt_authorization: isize,
    bt_alert_shown: bool,
    menu_open: bool,
    dirty: bool,
    polled_once: bool,
    /// An update check request is running.
    update_in_flight: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// A token from `addObserverForName:object:queue:usingBlock:`.
type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Handler has no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "BattbarMenuHandler"]
    struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    unsafe impl NSApplicationDelegate for Handler {
        /// battbar opened again while it runs (Spotlight, Applications, `open -a`): the
        /// way out when macOS hides the expanded item for lack of room.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, _visible: bool) -> bool {
            with_app(|a| a.on_reopen());
            false
        }
    }

    unsafe impl NSMenuDelegate for Handler {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            with_app(|a| a.menu_open = true);
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _menu: &NSMenu) {
            with_app(|a| a.menu_open = false);
            // Re-render after the selected action (if any) has run.
            dispatch2::DispatchQueue::main().exec_async(|| {
                with_app(|a| {
                    if a.dirty {
                        a.render();
                    }
                });
            });
        }
    }

    impl Handler {
        #[unsafe(method(menuAction:))]
        fn menu_action(&self, sender: &NSMenuItem) {
            let tag = sender.tag();
            if let Some(Some(p)) = with_app(|a| a.perform(tag)) {
                run_post(p);
            }
        }
    }
);

impl Handler {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn open_url(u: &str) {
    if let Some(url) = NSURL::URLWithString(&ns(u)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

/// Select the app's real location (e.g. in Downloads) in Finder, so the user can drag it
/// into Applications. Falls back to the Downloads folder.
fn reveal_original_location() {
    let ws = NSWorkspace::sharedWorkspace();
    if let Some(p) = battbar_macos::bundle::original_path() {
        let url = NSURL::fileURLWithPath(&ns(&p));
        ws.activateFileViewerSelectingURLs(&objc2_foundation::NSArray::from_retained_slice(&[url]));
        return;
    }
    if let Some(h) = std::env::var_os("HOME") {
        let url = NSURL::fileURLWithPath(&ns(&format!("{}/Downloads", h.to_string_lossy())));
        ws.openURL(&url);
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Accessory checkbox for the first-run questions (unchecked: the check is opt-in).
/// The first-run question (Launch at Login), with the update checkbox when that was
/// not answered yet.
fn first_run_alert(mtm: MainThreadMarker, offer_updates: bool) -> (Retained<NSAlert>, Option<Retained<NSButton>>) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&ns("Launch battbar at login?"));
    // The checkbox title stays short so the alert keeps the width of its buttons; what
    // the update check does is said in the text above it.
    let updates = if offer_updates {
        "\n\nThe update check asks GitHub once a day whether a newer version exists: one request, no device data."
    } else {
        ""
    };
    alert.setInformativeText(&ns(&format!(
        "battbar can start automatically when you log in, so the battery icons are always there. \
         You can change this later under Settings in the battbar menu.{updates}"
    )));
    alert.addButtonWithTitle(&ns("Launch at Login"));
    alert.addButtonWithTitle(&ns("Not Now"));
    let checkbox = offer_updates.then(|| update_checkbox(mtm));
    if let Some(c) = &checkbox {
        alert.setAccessoryView(Some(c));
    }
    (alert, checkbox)
}

/// Debug builds: `--probe-first-run-alert` lays out the first-run alert without showing
/// it and prints the window, button and checkbox widths (the alert should be no wider
/// than its buttons need).
#[cfg(debug_assertions)]
fn probe_first_run_alert() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let (alert, checkbox) = first_run_alert(mtm, true);
    alert.layout();
    let win = alert.window();
    println!("window width {:.0}", win.frame().size.width);
    for b in alert.buttons().iter() {
        println!("button \"{}\" width {:.0}", b.title(), b.frame().size.width);
    }
    if let Some(c) = checkbox {
        println!("checkbox \"{}\" width {:.0}", c.title(), c.frame().size.width);
    }
}

fn update_checkbox(mtm: MainThreadMarker) -> Retained<NSButton> {
    let b = unsafe { NSButton::checkboxWithTitle_target_action(&ns("Check for updates daily"), None, None, mtm) };
    b.setState(NSControlStateValueOff);
    b.sizeToFit();
    b
}

/// Turn the update check on or off (first-run answer or the Settings switch).
fn set_update_check(on: bool) {
    with_app(|a| {
        a.settings.asked_update_check = true;
        a.settings.check_for_updates = on;
        if on {
            a.settings.next_update_check = None; // check right away
        } else {
            a.settings.available_update = None;
        }
        a.save();
        a.maybe_check_updates();
        a.dirty = true;
    });
}

fn local_time() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

/// Launch at Login through `SMAppService.mainApp`. The state shown is always the
/// system's (`status`), never a copy in our settings; `register()` is only ever called
/// on an explicit user action.
mod login {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Status {
        Enabled,
        NotRegistered,
        /// Registered, but the user has to allow it in System Settings → Login Items.
        RequiresApproval,
        /// No login item known to the system. Returned for an app that was never
        /// registered as well; only after a failed `register()` does it mean a problem.
        NotFound,
    }

    pub fn status() -> Status {
        let s = unsafe { SMAppService::mainAppService().status() };
        if s == SMAppServiceStatus::Enabled {
            Status::Enabled
        } else if s == SMAppServiceStatus::RequiresApproval {
            Status::RequiresApproval
        } else if s == SMAppServiceStatus::NotFound {
            Status::NotFound
        } else {
            Status::NotRegistered
        }
    }

    pub fn enabled() -> bool {
        status() == Status::Enabled
    }

    pub fn open_settings() {
        unsafe { SMAppService::openSystemSettingsLoginItems() };
    }
    pub fn set(on: bool) -> Result<(), String> {
        let s = unsafe { SMAppService::mainAppService() };
        let r = unsafe {
            if on {
                s.registerAndReturnError()
            } else {
                s.unregisterAndReturnError()
            }
        };
        r.map_err(|e| e.localizedDescription().to_string())
    }
}

mod notify {
    use super::{ns, Alert};
    use objc2::runtime::Bool;
    use objc2_foundation::NSError;
    use objc2_user_notifications::{UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter};

    use std::sync::atomic::{AtomicBool, Ordering};

    /// Set once macOS confirms that battbar may post notifications. Alerts are not
    /// evaluated before that, so the first low-battery alert is not lost while the
    /// permission prompt is still open.
    static AUTHORIZED: AtomicBool = AtomicBool::new(false);
    static ANSWERED: AtomicBool = AtomicBool::new(false);

    pub fn answered() -> bool {
        ANSWERED.load(Ordering::Relaxed)
    }

    pub fn authorized() -> bool {
        AUTHORIZED.load(Ordering::Relaxed)
    }

    pub fn request_authorization() {
        let c = UNUserNotificationCenter::currentNotificationCenter();
        let block = block2::RcBlock::new(|granted: Bool, err: *mut NSError| {
            AUTHORIZED.store(granted.as_bool(), Ordering::Relaxed);
            ANSWERED.store(true, Ordering::Relaxed);
            if std::env::var_os("BATTBAR_LOG").is_some() {
                let e = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string());
                eprintln!("notifications: granted={} error={e:?}", granted.as_bool());
            }
            if granted.as_bool() {
                // Evaluate alerts right away instead of waiting for the next poll.
                dispatch2::DispatchQueue::main().exec_async(|| {
                    super::with_app(|a| a.check_alerts());
                });
            }
        });
        c.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &block);
    }

    pub fn post(a: &Alert) {
        let c = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&ns(&a.title));
        content.setBody(&ns(&a.body));
        let req = UNNotificationRequest::requestWithIdentifier_content_trigger(&ns(&a.id), &content, None);
        let done = block2::RcBlock::new(|err: *mut NSError| {
            if let Some(e) = unsafe { err.as_ref() } {
                eprintln!("battbar: notification failed: {}", e.localizedDescription());
            } else if std::env::var_os("BATTBAR_LOG").is_some() {
                eprintln!("notification posted");
            }
        });
        c.addNotificationRequest_withCompletionHandler(&req, Some(&done));
    }
}

fn run_post(p: Post) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    match p {
        Post::Quit => NSApplication::sharedApplication(mtm).terminate(None),
        Post::BluetoothAlert => {
            NSApplication::sharedApplication(mtm).activate();
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&ns("battbar has no access to Bluetooth"));
            alert.setInformativeText(&ns(
                "Without it, battbar cannot show the battery of Bluetooth devices. Devices on receivers and base stations keep working.\n\n\
                 battbar only reads devices that are paired with and connected to this Mac; it never scans for other devices.",
            ));
            alert.addButtonWithTitle(&ns("Open System Settings"));
            alert.addButtonWithTitle(&ns("Not Now"));
            if alert.runModal() == 1000 {
                open_url(URL_BLUETOOTH_PRIVACY);
            }
        }
        Post::Message(m) => {
            NSApplication::sharedApplication(mtm).activate();
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&ns(&m));
            alert.runModal();
        }
        Post::AskLaunchAtLogin => {
            NSApplication::sharedApplication(mtm).activate();
            let offer_updates = with_app(|a| !a.settings.asked_update_check).unwrap_or(false);
            let (alert, checkbox) = first_run_alert(mtm, offer_updates);
            let yes = alert.runModal() == 1000;
            with_app(|a| {
                a.settings.asked_launch_at_login = true; // "Not Now" = do not ask again
                a.save();
            });
            if let Some(c) = checkbox {
                set_update_check(c.state() == NSControlStateValueOn);
            }
            if yes {
                if let Err(e) = login::set(true) {
                    run_post(Post::Message(format!("Could not turn on Launch at Login: {e}")));
                }
                let st = login::status();
                if matches!(st, login::Status::RequiresApproval | login::Status::NotFound) {
                    run_post(Post::LoginItemHelp(st));
                }
            }
            with_app(|a| a.render());
        }
        Post::TranslocationNotice => {
            NSApplication::sharedApplication(mtm).activate();
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&ns("Move battbar to the Applications folder"));
            alert.setInformativeText(&ns(
                "battbar is running from a temporary location that macOS creates for apps opened straight from a download. \
                 Move battbar.app into the Applications folder (drag it there in Finder) and open it from there, so it can \
                 start at login and keep working after a restart.",
            ));
            alert.addButtonWithTitle(&ns("Show in Finder"));
            alert.addButtonWithTitle(&ns("Later"));
            let show = alert.runModal() == 1000;
            with_app(|a| {
                a.settings.warned_translocation = true;
                a.save();
            });
            if show {
                reveal_original_location();
            }
        }
        Post::LoginItemHelp(st) => {
            NSApplication::sharedApplication(mtm).activate();
            let alert = NSAlert::new(mtm);
            match st {
                login::Status::RequiresApproval => {
                    alert.setMessageText(&ns("Allow battbar in Login Items"));
                    alert.setInformativeText(&ns(
                        "macOS needs your approval before battbar can start at login. Turn battbar on under \
                         System Settings → General → Login Items & Extensions.",
                    ));
                }
                _ => {
                    alert.setMessageText(&ns("Launch at Login is not available"));
                    alert.setInformativeText(&ns(
                        "macOS cannot find battbar's login item. Move battbar.app to the Applications folder, open it \
                         from there and try again, or check System Settings → General → Login Items & Extensions.",
                    ));
                }
            }
            alert.addButtonWithTitle(&ns("Open Login Items"));
            alert.addButtonWithTitle(&ns("Cancel"));
            if alert.runModal() == 1000 {
                login::open_settings();
            }
        }
        Post::SetUpdateCheck(on) => {
            set_update_check(on);
            with_app(|a| a.render());
        }
        Post::AskUpdateCheck => {
            NSApplication::sharedApplication(mtm).activate();
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&ns("Check for battbar updates?"));
            alert.setInformativeText(&ns(
                "Once a day battbar can ask GitHub whether a newer version exists. It is the only network request \
                 battbar makes; it carries no device data or identifiers. You can change this later under Settings \
                 in the battbar menu.",
            ));
            alert.addButtonWithTitle(&ns("Check Daily"));
            alert.addButtonWithTitle(&ns("Not Now"));
            set_update_check(alert.runModal() == 1000);
            with_app(|a| a.render());
        }
        Post::UpdateHelp(v) => {
            NSApplication::sharedApplication(mtm).activate();
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&ns(&format!("battbar {v} is available")));
            alert.setInformativeText(&ns(&format!(
                "battbar was installed with Homebrew. To update, run in Terminal:\n\n{}\n\nHomebrew quits battbar, \
                 replaces it and opens it again.",
                updates::UPGRADE_COMMAND
            )));
            alert.addButtonWithTitle(&ns("Copy Command"));
            alert.addButtonWithTitle(&ns("Release Notes"));
            alert.addButtonWithTitle(&ns("Close"));
            match alert.runModal() {
                1000 => {
                    let pb = NSPasteboard::generalPasteboard();
                    pb.clearContents();
                    pb.setString_forType(&ns(updates::UPGRADE_COMMAND), unsafe { NSPasteboardTypeString });
                }
                1001 => {
                    if let Some(ver) = updates::Version::parse(&v) {
                        open_url(&updates::release_page(&ver));
                    }
                }
                _ => {}
            }
        }
    }
}

impl App {
    fn recompute(&mut self) {
        self.devices = merge(self.reports.clone(), &self.settings.merge_config());
        let settings = &self.settings;
        battbar_core::cells::apply_chemistry(&mut self.devices, &self.db, |id| settings.chemistry_for(id));
        battbar_core::calibration::apply(&settings.voltage_calibration, &mut self.devices, &self.db);
        self.settling.apply(std::time::Instant::now(), &mut self.devices);
    }

    fn check_alerts(&mut self) {
        if !notify::authorized() || !self.polled_once {
            return;
        }
        for a in self
            .alerts
            .check(&self.devices, self.settings.low_battery_threshold, self.settings.notify_spare_full)
        {
            notify::post(&a);
        }
    }

    /// Start the opt-in update check if it is on and due. With the option off nothing
    /// is requested (see `updates::tick`).
    fn maybe_check_updates(&mut self) {
        if !updates::tick(&mut self.settings, unix_now(), self.update_in_flight, || {}) {
            return;
        }
        self.update_in_flight = true;
        self.save();
        std::thread::Builder::new()
            .name("battbar-update-check".into())
            .spawn(|| {
                use updates::Fetch;
                let r = updates::Curl.get(updates::LATEST_API).and_then(|body| updates::parse_latest(&body));
                dispatch2::DispatchQueue::main().exec_async(move || {
                    with_app(|a| a.on_update_check(r));
                });
            })
            .map(|_| ())
            .unwrap_or_else(|e| {
                eprintln!("battbar: update check thread: {e}");
                self.update_in_flight = false;
            });
    }

    fn on_update_check(&mut self, r: Result<Option<updates::Version>, String>) {
        self.update_in_flight = false;
        let outcome = updates::finish(&mut self.settings, unix_now(), &updates::running_version(), r);
        self.save();
        if std::env::var_os("BATTBAR_LOG").is_some() {
            eprintln!("update check: {outcome:?}");
        }
        if let updates::Outcome::Available { version, notify: true } = &outcome {
            if notify::authorized() {
                notify::post(&Alert {
                    id: format!("update-{version}"),
                    title: format!("battbar {version} is available"),
                    body: "Open the battbar menu to update.".into(),
                });
            }
        }
        if self.menu_open {
            self.dirty = true;
        } else {
            self.render();
        }
    }

    fn save(&self) {
        if let Err(e) = self.settings.save() {
            eprintln!("battbar: saving settings: {e}");
        }
    }

    fn on_update(&mut self, u: PollUpdate) -> Option<Post> {
        let mut reports = u.reports;
        let mut changed = false;
        if self.identities.learn(&reports) {
            self.settings.identity_memory = self.identities.to_stored();
            changed = true;
        }
        self.identities.augment(&mut reports);
        self.reports = reports;
        let complete = u.timed_out.is_empty();
        self.timed_out = u.timed_out;
        self.polled_once = true;
        for l in u.learned {
            if !self.settings.learned_links.iter().any(|x| x.bluetooth == l.bluetooth) && !self.settings.merge.splits.contains(&l.bluetooth)
            {
                self.settings.learned_links.push(l);
                changed = true;
            }
        }
        let before = self.devices.clone();
        self.recompute();
        self.gate.confirm_regrouped(&before, &self.devices);
        for d in &self.devices {
            if d.reading.percent.is_some() && !self.settings.ever_reported.contains(&d.id) {
                self.settings.ever_reported.push(d.id.clone());
                changed = true;
            }
        }
        changed |= battbar_core::calibration::observe(&mut self.settings.voltage_calibration, &self.devices);
        if changed {
            self.save();
        }
        self.admitted = self.gate.admit(std::time::Instant::now(), &self.devices, complete);
        if self.gate.pending() {
            let _ = self.tx.send(WorkerCmd::RefreshIn(self.gate.settle + Duration::from_millis(500)));
        }
        self.check_alerts();
        self.maybe_check_updates();
        self.last_update = Some(local_time());
        if std::env::var_os("BATTBAR_LOG").is_some() {
            eprintln!(
                "[{}] input_monitoring={} bt_auth={} timed_out={:?} reason={}",
                local_time(),
                battbar_hid::input_monitoring_status(),
                u.bt_authorization,
                self.timed_out,
                u.reason
            );
            for d in &self.devices {
                let spare = d
                    .components
                    .iter()
                    .map(|c| format!(" {:?}={}", c.kind, ui::status_text(&c.reading)))
                    .collect::<String>();
                let sources = d
                    .sources
                    .iter()
                    .map(|s| format!("{}={}", s.source, ui::status_text(&s.reading)))
                    .collect::<Vec<_>>()
                    .join("; ");
                eprintln!(
                    "  {} [{:?}] {} via {:?}{spare} sources: {sources}",
                    d.name,
                    d.kind,
                    ui::status_text(&d.reading),
                    d.live_transports()
                );
            }
        }
        self.bt_authorization = u.bt_authorization;
        self.watch_bluetooth();
        if self.menu_open {
            self.dirty = true;
        } else {
            self.render();
        }
        // 2 = denied, 1 = restricted.
        if matches!(u.bt_authorization, 1 | 2) && !self.bt_alert_shown {
            self.bt_alert_shown = true;
            return Some(Post::BluetoothAlert);
        }
        None
    }

    fn perform(&mut self, tag: isize) -> Option<Post> {
        let a = self.actions.get(tag as usize)?.clone();
        let mut reconfigure = false;
        match a {
            Action::Refresh => {
                let _ = self.tx.send(WorkerCmd::Refresh);
                return None;
            }
            Action::Quit => return Some(Post::Quit),
            Action::OpenBluetoothSettings => {
                open_url(URL_BLUETOOTH_PRIVACY);
                return None;
            }
            Action::OpenNotificationSettings => {
                // Release and development builds have different bundle IDs.
                let id = objc2_foundation::NSBundle::mainBundle()
                    .bundleIdentifier()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "io.github.battbar".into());
                open_url(&format!(
                    "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}"
                ));
                return None;
            }
            // Never register a login item for a translocated (temporary) copy.
            Action::ToggleLaunchAtLogin if battbar_macos::bundle::is_translocated() => {
                return Some(Post::TranslocationNotice);
            }
            Action::MoveToApplications => {
                reveal_original_location();
                return None;
            }
            Action::ToggleLaunchAtLogin => {
                if let Err(e) = login::set(!login::enabled()) {
                    return Some(Post::Message(format!("Could not change Launch at Login: {e}")));
                }
                let st = login::status();
                if matches!(st, login::Status::RequiresApproval | login::Status::NotFound) {
                    self.dirty = true;
                    return Some(Post::LoginItemHelp(st));
                }
            }
            Action::OpenLoginItems => {
                login::open_settings();
                return None;
            }
            Action::SetChemistry(id, chem) => self.settings.set_chemistry(&id, chem),
            Action::ToggleUpdateCheck => {
                let on = !self.settings.check_for_updates;
                return Some(Post::SetUpdateCheck(on));
            }
            Action::ShowUpdate(v) => {
                if updates::installed_with_homebrew() {
                    return Some(Post::UpdateHelp(v));
                }
                if let Some(ver) = updates::Version::parse(&v) {
                    open_url(&updates::release_page(&ver));
                }
                return None;
            }
            // render() restarts or stops the overflow check (the layout changes).
            Action::SetDisplayMode(m) => self.settings.display_mode = Some(m),
            Action::MoveDevice(id, delta) => {
                self.settings.device_order = ui::move_device(&self.shown, &self.settings.device_order, &id, delta);
            }
            Action::ResetDeviceOrder => self.settings.device_order.clear(),
            Action::ForgetCalibration(id) => {
                battbar_core::calibration::forget(&mut self.settings.voltage_calibration, &id);
            }
            Action::SetThreshold(t) => self.settings.low_battery_threshold = t,
            Action::SetInterval(s) => {
                self.settings.poll_interval_secs = s;
                reconfigure = true;
            }
            Action::ToggleSpareNotify => self.settings.notify_spare_full = !self.settings.notify_spare_full,
            Action::ToggleShowUnreported => self.settings.show_unreported = !self.settings.show_unreported,
            Action::ToggleHeadsetcontrol => {
                self.settings.headsetcontrol_fallback = !self.settings.headsetcontrol_fallback;
                reconfigure = true;
            }
            Action::Hide(id) => {
                self.settings.shown.retain(|x| *x != id);
                if !self.settings.hidden.contains(&id) {
                    self.settings.hidden.push(id);
                }
            }
            Action::Show(id) => {
                self.settings.hidden.retain(|x| *x != id);
                if let Some(d) = self.devices.iter().find(|d| d.id == id) {
                    if ui::hidden_by_default(d, &self.settings) {
                        self.settings.shown.push(id);
                    }
                }
            }
            Action::Split(id) => {
                if let Some(d) = self.devices.iter().find(|d| d.id == id) {
                    let keys: Vec<String> = d.sources.iter().map(|s| s.full_key()).collect();
                    self.settings.split_sources(&keys);
                }
            }
            Action::MergeWith(a, b) => {
                let keys: Vec<String> = self
                    .devices
                    .iter()
                    .filter(|d| d.id == a || d.id == b)
                    .flat_map(|d| d.sources.iter().map(|s| s.full_key()))
                    .collect();
                self.settings.merge_sources(keys);
            }
            Action::Unlink(bt) => {
                self.settings.unlink(&bt);
                if !self.settings.merge.splits.contains(&bt) {
                    // Keep it apart from now on (otherwise the live merge could re-learn it).
                    self.settings.merge.splits.push(bt);
                }
            }
        }
        if reconfigure {
            let _ = self.tx.send(WorkerCmd::Reconfigure {
                headsetcontrol: self.settings.headsetcontrol_fallback,
                interval: Duration::from_secs(self.settings.poll_interval_secs),
            });
        }
        self.save();
        let before = self.devices.clone();
        self.recompute();
        self.admit_regrouped(&before);
        self.dirty = true;
        None
    }

    /// Devices regrouped from shown ones (Merge With, Split, Unlink, learned links) stay
    /// visible instead of waiting for the new-device gate.
    fn admit_regrouped(&mut self, before: &[Device]) {
        for id in self.gate.confirm_regrouped(before, &self.devices) {
            if !self.admitted.contains(&id) {
                self.admitted.push(id);
            }
        }
    }

    // ------------------------------------------------------------- rendering

    fn add(&mut self, menu: &NSMenu, title: &str, action: Option<Action>, checked: bool, key: &str) -> Retained<NSMenuItem> {
        let sel = action.as_ref().map(|_| sel!(menuAction:));
        let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(self.mtm), &ns(title), sel, &ns(key)) };
        if let Some(a) = action {
            self.actions.push(a);
            item.setTag((self.actions.len() - 1) as isize);
            unsafe { item.setTarget(Some(&self.handler as &AnyObject)) };
        }
        if checked {
            item.setState(NSControlStateValueOn);
        } else {
            item.setState(NSControlStateValueOff);
        }
        menu.addItem(&item);
        item
    }

    fn submenu(&mut self, menu: &NSMenu, title: &str) -> Retained<NSMenu> {
        let item = self.add(menu, title, None, false, "");
        let sub = NSMenu::new(self.mtm);
        sub.setAutoenablesItems(true);
        item.setSubmenu(Some(&sub));
        sub
    }

    fn separator(&self, menu: &NSMenu) {
        menu.addItem(&NSMenuItem::separatorItem(self.mtm));
    }

    fn settings_menu(&mut self, menu: &NSMenu) {
        let s = self.settings.clone();
        if battbar_macos::bundle::is_translocated() {
            self.add(
                menu,
                "Launch at Login — Move battbar to Applications First…",
                Some(Action::MoveToApplications),
                false,
                "",
            );
        } else {
            self.launch_at_login_item(menu);
        }
        self.add(menu, "Check for Updates", Some(Action::ToggleUpdateCheck), s.check_for_updates, "");
        let mb = self.submenu(menu, "Menu Bar");
        let mode = s.display_mode();
        self.add(
            &mb,
            "Collapsed — One Icon",
            Some(Action::SetDisplayMode(DisplayMode::Collapsed)),
            mode == DisplayMode::Collapsed,
            "",
        );
        self.add(
            &mb,
            "Expanded — All Devices",
            Some(Action::SetDisplayMode(DisplayMode::Expanded)),
            mode == DisplayMode::Expanded,
            "",
        );
        self.separator(&mb);
        let reset = (!s.device_order.is_empty()).then_some(Action::ResetDeviceOrder);
        self.add(&mb, "Reset Device Order (by Type)", reset, false, "");
        let low = self.submenu(menu, "Low Battery Alert");
        for t in THRESHOLDS {
            let title = t.map(|v| format!("At {v}%")).unwrap_or_else(|| "Off".into());
            self.add(&low, &title, Some(Action::SetThreshold(t)), s.low_battery_threshold == t, "");
        }
        let iv = self.submenu(menu, "Update Every");
        for i in INTERVALS {
            let title = if i < 60 {
                format!("{i} seconds")
            } else {
                format!("{} minute{}", i / 60, if i >= 120 { "s" } else { "" })
            };
            self.add(&iv, &title, Some(Action::SetInterval(i)), s.poll_interval_secs == i, "");
        }
        self.add(
            menu,
            "Notify When Spare Battery Is Charged",
            Some(Action::ToggleSpareNotify),
            s.notify_spare_full,
            "",
        );
        self.add(
            menu,
            "Show Devices Without Battery Value",
            Some(Action::ToggleShowUnreported),
            s.show_unreported,
            "",
        );
        self.add(
            menu,
            "Use headsetcontrol for Other Headsets",
            Some(Action::ToggleHeadsetcontrol),
            s.headsetcontrol_fallback,
            "",
        );
        self.separator(menu);

        self.add(menu, "Bluetooth Privacy Settings…", Some(Action::OpenBluetoothSettings), false, "");
        self.add(menu, "Notification Settings…", Some(Action::OpenNotificationSettings), false, "");
    }

    fn launch_at_login_item(&mut self, menu: &NSMenu) {
        match login::status() {
            login::Status::Enabled => self.add(menu, "Launch at Login", Some(Action::ToggleLaunchAtLogin), true, ""),
            login::Status::NotRegistered | login::Status::NotFound => {
                self.add(menu, "Launch at Login", Some(Action::ToggleLaunchAtLogin), false, "")
            }
            login::Status::RequiresApproval => self.add(
                menu,
                "Launch at Login — Allow in System Settings…",
                Some(Action::OpenLoginItems),
                false,
                "",
            ),
        };
    }

    /// Actions on one device: Hide, Merge With, Split, Unlink.
    fn device_actions(&mut self, menu: &NSMenu, d: &Device, visible: bool) {
        if battbar_core::cells::has_selectable_chemistry(d) {
            let current = d.reading.battery.and_then(|b| b.chemistry);
            let sub = self.submenu(menu, "Batteries");
            self.add(
                &sub,
                "Alkaline",
                Some(Action::SetChemistry(d.id.clone(), Chemistry::Alkaline)),
                current == Some(Chemistry::Alkaline),
                "",
            );
            self.add(
                &sub,
                "NiMH (rechargeable AA/AAA)",
                Some(Action::SetChemistry(d.id.clone(), Chemistry::Nimh)),
                current == Some(Chemistry::Nimh),
                "",
            );
        }
        if let Some(cal) = self.settings.voltage_calibration.iter().find(|c| c.device == d.id) {
            let levels = cal.usable_levels();
            let sub = self.submenu(menu, "Voltage Calibration");
            let state = if levels == 0 {
                "Learning on Bluetooth…".to_string()
            } else {
                format!("Learned from Bluetooth ({levels} level{})", if levels == 1 { "" } else { "s" })
            };
            self.add(&sub, &state, None, false, "");
            self.add(&sub, "Reset", Some(Action::ForgetCalibration(d.id.clone())), false, "");
        }
        if visible {
            if let Some(i) = self.shown.iter().position(|x| *x == d.id).filter(|_| self.shown.len() > 1) {
                let (back, fwd) = match self.settings.display_mode() {
                    DisplayMode::Expanded => ("Move Left", "Move Right"),
                    DisplayMode::Collapsed => ("Move Up", "Move Down"),
                };
                let left = (i > 0).then(|| Action::MoveDevice(d.id.clone(), -1));
                let right = (i + 1 < self.shown.len()).then(|| Action::MoveDevice(d.id.clone(), 1));
                self.add(menu, back, left, false, "");
                self.add(menu, fwd, right, false, "");
            }
            self.add(menu, "Hide from Menu Bar", Some(Action::Hide(d.id.clone())), false, "");
        } else {
            self.add(menu, "Show in Menu Bar", Some(Action::Show(d.id.clone())), false, "");
        }
        let devices = self.devices.clone();
        let others: Vec<&Device> = devices.iter().filter(|o| o.id != d.id).collect();
        if !others.is_empty() {
            let m = self.submenu(menu, "Merge With");
            for o in others {
                let via = o.sources.iter().map(|s| s.transport.to_string()).collect::<Vec<_>>().join(", ");
                self.add(
                    &m,
                    &format!("{} ({via})", o.name),
                    Some(Action::MergeWith(d.id.clone(), o.id.clone())),
                    false,
                    "",
                );
            }
        }
        if d.sources.len() > 1 {
            self.add(menu, "Split Into Separate Devices", Some(Action::Split(d.id.clone())), false, "");
        }
        let links: Vec<String> = self
            .settings
            .learned_links
            .iter()
            .filter(|l| d.sources.iter().any(|x| x.full_key() == l.bluetooth))
            .map(|l| l.bluetooth.clone())
            .collect();
        for bt in links {
            self.add(menu, "Unlink Bluetooth Connection", Some(Action::Unlink(bt)), false, "");
        }
    }

    fn other_devices_menu(&mut self, menu: &NSMenu) {
        let others: Vec<Device> = ui::others(&self.devices, &self.settings).into_iter().cloned().collect();
        if others.is_empty() {
            self.add(menu, "None", None, false, "");
        }
        for d in &others {
            let sub = self.submenu(menu, &format!("{} — {}", d.name, ui::status_text(&d.reading)));
            self.device_actions(&sub, d, false);
        }
    }

    /// The one menu: a section per device (details, sources; actions in the device's
    /// submenu), status lines, Expand/Collapse, Refresh, Settings, Other Devices, Quit.
    fn main_menu(&mut self, devices: &[Device]) -> Retained<NSMenu> {
        let menu = NSMenu::new(self.mtm);
        menu.setAutoenablesItems(true);
        menu.setDelegate(Some(ProtocolObject::from_ref(&*self.handler)));
        if devices.is_empty() {
            let text = if self.polled_once {
                "No devices found"
            } else {
                "Looking for devices…"
            };
            self.add(&menu, text, None, false, "");
        }
        for (i, d) in devices.iter().enumerate() {
            if i > 0 {
                self.separator(&menu);
            }
            // Header: icon + name; the device's actions in its submenu.
            let actions = self.submenu(&menu, &d.name);
            if let Some(item) = menu.itemArray().lastObject() {
                let img = NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(ui::symbol(d.kind)), Some(&ns(&d.name)));
                item.setImage(img.as_deref());
            }
            self.device_actions(&actions, d, true);
            for l in ui::detail_lines(d) {
                self.add(&menu, &format!("    {l}"), None, false, "");
            }
            let src = self.submenu(&menu, "    Sources");
            for l in ui::source_lines(d) {
                self.add(&src, &l, None, false, "");
            }
        }
        // Status lines (informational or with an action), after a separator if any.
        let mut status: Vec<(String, Option<Action>)> = vec![];
        if let Some(t) = &self.last_update {
            status.push((format!("Updated {t}"), None));
        }
        if !self.timed_out.is_empty() {
            status.push((format!("No answer from: {}", self.timed_out.join(", ")), None));
        }
        let wants_alerts = self.settings.low_battery_threshold.is_some() || self.settings.notify_spare_full;
        if wants_alerts && notify::answered() && !notify::authorized() {
            status.push(("Notifications are off for battbar…".into(), Some(Action::OpenNotificationSettings)));
        }
        if battbar_macos::bundle::is_translocated() {
            status.push(("Move battbar to Applications…".into(), Some(Action::MoveToApplications)));
        }
        if matches!(self.bt_authorization, 1 | 2) {
            status.push(("Bluetooth access needed…".into(), Some(Action::OpenBluetoothSettings)));
        }
        if !status.is_empty() {
            self.separator(&menu);
        }
        for (title, action) in status {
            self.add(&menu, &title, action, false, "");
        }
        self.separator(&menu);
        match self.settings.display_mode() {
            DisplayMode::Collapsed => {
                self.add(
                    &menu,
                    "Expand in Menu Bar",
                    Some(Action::SetDisplayMode(DisplayMode::Expanded)),
                    false,
                    "",
                );
                if self.settings.collapsed_for_space {
                    self.add(&menu, "    Not enough space last time", None, false, "");
                }
            }
            DisplayMode::Expanded => {
                self.add(
                    &menu,
                    "Collapse in Menu Bar",
                    Some(Action::SetDisplayMode(DisplayMode::Collapsed)),
                    false,
                    "",
                );
            }
        }
        self.add(&menu, "Refresh", Some(Action::Refresh), false, "r");
        let s = self.submenu(&menu, "Settings");
        self.settings_menu(&s);
        let o = self.submenu(&menu, "Other Devices");
        self.other_devices_menu(&o);
        if let Some(v) = updates::available(&self.settings, &updates::running_version()) {
            self.add(
                &menu,
                &format!("Update available: v{v}…"),
                Some(Action::ShowUpdate(v.to_string())),
                false,
                "",
            );
        }
        self.separator(&menu);
        self.add(&menu, &format!("battbar {}", battbar_core::version::FULL), None, false, "");
        self.add(&menu, "Quit battbar", Some(Action::Quit), false, "q");
        menu
    }

    fn status_item(&mut self) -> Retained<NSStatusItem> {
        if let Some(it) = &self.item {
            return it.clone();
        }
        let it = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        // Keeps the position the user chose with ⌘-drag.
        it.setAutosaveName(Some(&ns(STATUS_ITEM_AUTOSAVE)));
        self.item = Some(it.clone());
        it
    }

    fn layout(&self) -> ui::Layout {
        match self.settings.display_mode() {
            DisplayMode::Collapsed => ui::Layout::Collapsed,
            DisplayMode::Expanded => ui::Layout::Expanded,
        }
    }

    fn render(&mut self) {
        self.dirty = false;
        self.actions.clear();
        let visible: Vec<Device> = ui::visible(&self.devices, &self.settings)
            .into_iter()
            .filter(|d| self.admitted.contains(&d.id))
            .cloned()
            .collect();
        self.shown = visible.iter().map(|d| d.id.clone()).collect();
        let refs: Vec<&Device> = visible.iter().collect();
        let view = ui::bar_view(&refs, self.layout(), self.settings.low_battery_threshold, ui::COLLAPSED_STYLE);
        // A new size or layout: macOS places the item again; restart the overflow check.
        let key = format!(
            "{:?}{:?}",
            self.layout(),
            view.segments.iter().map(|s| (s.symbol, &s.title, s.dimmed)).collect::<Vec<_>>()
        );
        let restart = key != self.bar_key;
        self.bar_key = key;
        let it = self.status_item();
        if let Some(b) = it.button(self.mtm) {
            b.setImage(bar::image(&view).as_deref());
            b.setImagePosition(NSCellImagePosition::ImageOnly);
            b.setTitle(&ns(""));
            b.setToolTip(Some(&ns(&view.tooltip)));
        }
        self.watch_item_window();
        if restart {
            let timer = match self.settings.display_mode() {
                DisplayMode::Expanded if self.auto_collapse => self.overflow.changed(std::time::Instant::now()),
                _ => self.overflow.disarm(),
            };
            self.overflow_apply(None, timer);
        }
        let menu = self.main_menu(&visible);
        #[cfg(debug_assertions)]
        if std::env::var_os("BATTBAR_DEV_DUMP_MENU").is_some() {
            eprintln!("--- menu ({:?}, {} segment(s))", self.layout(), view.segments.len());
            let win = it.button(self.mtm).and_then(|b| b.window());
            eprintln!(
                "item visible={} length={:?} image={:?} window={:?} occlusion={:?}",
                it.isVisible(),
                it.length(),
                it.button(self.mtm).and_then(|b| b.image()).map(|i| i.size()),
                win.as_ref().map(|w| w.frame()),
                win.as_ref().map(|w| w.occlusionState())
            );
            dump_menu(&menu, 0);
        }
        it.setMenu(Some(&menu));
    }
}

/// Development: print a menu as an indented outline (`·` = informational line,
/// `✓` = checked, `▸` = submenu).
#[cfg(debug_assertions)]
fn dump_menu(menu: &NSMenu, depth: usize) {
    for item in menu.itemArray().iter() {
        if item.isSeparatorItem() {
            eprintln!("{}────", "  ".repeat(depth));
            continue;
        }
        let sub = item.submenu();
        let mark = if sub.is_some() {
            "▸"
        } else if item.action().is_none() {
            "·"
        } else if item.state() == NSControlStateValueOn {
            "✓"
        } else {
            " "
        };
        eprintln!("{}{mark} {}", "  ".repeat(depth), item.title());
        if let Some(sub) = sub {
            dump_menu(&sub, depth + 1);
        }
    }
}

impl App {
    /// Bluetooth connect/disconnect notifications → poll soon (once permission is
    /// granted: IOBluetooth blocks while the prompt is open). Main thread.
    fn watch_bluetooth(&self) {
        if battbar_macos::connections::watching() || !battbar_macos::ble::is_authorized() || battbar_core::dev_no_hotplug() {
            return;
        }
        let tx = self.tx.clone();
        battbar_macos::connections::watch(self.mtm, move |_connected| {
            let _ = tx.send(WorkerCmd::Hotplug("bluetooth"));
        });
    }
}

impl App {
    // ------------------------------------------------- not enough space (overflow.rs)
    //
    // Event-driven: occlusion notifications of the item's window, screen changes and
    // content changes feed the guard; it asks for at most one one-shot timer. Nothing
    // runs periodically (SPEC "Menu bar display", DEVELOPMENT "Timers and wake-ups").

    /// Whether macOS shows the item (the visible bit of its window's occlusion state).
    /// The item as the overflow check sees it: shown, hidden for lack of room, or
    /// hidden with the whole menu bar (full-screen app, auto-hide).
    fn item_seen(&self) -> Option<overflow::Seen> {
        let win = self.item.as_ref()?.button(self.mtm)?.window()?;
        let rect = |r: objc2_foundation::NSRect| overflow::Rect {
            x: r.origin.x,
            y: r.origin.y,
            w: r.size.width,
            h: r.size.height,
        };
        let screens: Vec<overflow::Rect> = objc2_app_kit::NSScreen::screens(self.mtm).iter().map(|s| rect(s.frame())).collect();
        let frame = rect(win.frame());
        // macOS 26 moves the item's window above the display; macOS 27 leaves it and
        // takes the display's menu bar window off screen.
        let bar_shown = menubar::menu_bar_shown(frame.x, frame.w, &menubar::windows()).unwrap_or(true);
        if !overflow::in_menu_bar(frame, &screens) || !bar_shown {
            return Some(overflow::Seen::MenuBarHidden);
        }
        let visible = win.occlusionState().0 & objc2_app_kit::NSWindowOcclusionState::Visible.0 != 0;
        Some(if visible { overflow::Seen::Shown } else { overflow::Seen::Hidden })
    }

    /// Subscribe to occlusion changes of the item's window; again if AppKit replaced
    /// the window.
    fn watch_item_window(&mut self) {
        let Some(win) = self.item.as_ref().and_then(|i| i.button(self.mtm)).and_then(|b| b.window()) else {
            return;
        };
        let addr = Retained::as_ptr(&win) as usize;
        if self.occlusion_watch.as_ref().is_some_and(|(a, _)| *a == addr) {
            return;
        }
        let center = objc2_foundation::NSNotificationCenter::defaultCenter();
        if let Some((_, tokens)) = self.occlusion_watch.take() {
            for token in tokens {
                let token: &AnyObject = token.as_ref();
                unsafe { center.removeObserver(token) };
            }
        }
        let block = block2::RcBlock::new(|_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            with_app(|a| a.on_occlusion());
        });
        // Occlusion: shown/hidden. Move: the menu bar slid in or out (a hidden item stays
        // hidden when a full-screen app ends, so occlusion alone says nothing then).
        let tokens = [unsafe { objc2_app_kit::NSWindowDidChangeOcclusionStateNotification }, unsafe {
            objc2_app_kit::NSWindowDidMoveNotification
        }]
        .into_iter()
        .map(|name| unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(name),
                Some(&win),
                Some(&objc2_foundation::NSOperationQueue::mainQueue()),
                &block,
            )
        })
        .collect();
        self.occlusion_watch = Some((addr, tokens));
    }

    fn log_visibility(&self, why: &str, visible: overflow::Seen) {
        if std::env::var_os("BATTBAR_LOG").is_some() {
            let frame = self
                .item
                .as_ref()
                .and_then(|i| i.button(self.mtm))
                .and_then(|b| b.window())
                .map(|w| w.frame());
            eprintln!("[{}] {why}: status item {visible:?} frame={frame:?}", local_time());
        }
    }

    /// Re-opened while running: collapse (see `overflow::reopen`) and say so.
    fn on_reopen(&mut self) {
        let outcome = overflow::reopen(&mut self.settings);
        if std::env::var_os("BATTBAR_LOG").is_some() {
            eprintln!("[{}] re-opened: {outcome:?}", local_time());
        }
        let (title, body) = match outcome {
            overflow::Reopen::Collapsed => {
                self.save();
                let timer = self.overflow.disarm();
                self.overflow_apply(None, timer);
                if self.menu_open {
                    self.dirty = true;
                } else {
                    self.render();
                }
                (
                    "battbar collapsed to fit the menu bar",
                    "All devices are behind one icon now. Choose Expand in the battbar menu to show them side by side again.",
                )
            }
            overflow::Reopen::AlreadyCollapsed => (
                "battbar is running",
                "Its icon is in the menu bar. If you don't see it, the menu bar is full: remove other icons to make room.",
            ),
        };
        notify::post(&alerts::Alert {
            id: format!("reopen-{}", unix_now()),
            title: title.into(),
            body: body.into(),
        });
    }

    fn on_occlusion(&mut self) {
        if !self.auto_collapse || self.settings.display_mode() != DisplayMode::Expanded {
            return;
        }
        let Some(seen) = self.item_seen() else { return };
        self.log_visibility("item window changed", seen);
        let (outcome, timer) = self.overflow.occlusion(std::time::Instant::now(), seen);
        self.overflow_apply(outcome, timer);
    }

    fn on_screens_changed(&mut self) {
        if self.auto_collapse && self.settings.display_mode() == DisplayMode::Expanded {
            let timer = self.overflow.changed(std::time::Instant::now());
            self.overflow_apply(None, timer);
        }
    }

    fn overflow_fire(&mut self) {
        self.overflow_timer = None;
        if !self.auto_collapse || self.settings.display_mode() != DisplayMode::Expanded {
            return;
        }
        let seen = self.item_seen().unwrap_or(overflow::Seen::Shown);
        self.log_visibility("overflow check", seen);
        let (outcome, timer) = self.overflow.fire(std::time::Instant::now(), seen);
        self.overflow_apply(outcome, timer);
    }

    fn overflow_apply(&mut self, outcome: Option<overflow::Outcome>, timer: overflow::Timer) {
        match timer {
            overflow::Timer::Keep => {}
            overflow::Timer::Cancel => {
                if let Some(t) = self.overflow_timer.take() {
                    t.invalidate();
                }
            }
            overflow::Timer::At(at) => {
                if let Some(t) = self.overflow_timer.take() {
                    t.invalidate();
                }
                let secs = at.saturating_duration_since(std::time::Instant::now()).as_secs_f64().max(0.01);
                let block = block2::RcBlock::new(|_t: std::ptr::NonNull<objc2_foundation::NSTimer>| {
                    with_app(|a| a.overflow_fire());
                });
                // One-shot (repeats: false).
                let t = unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(secs, false, &block) };
                t.setTolerance(0.1);
                self.overflow_timer = Some(t);
            }
        }
        match outcome {
            Some(overflow::Outcome::Collapse) => {
                if std::env::var_os("BATTBAR_LOG").is_some() {
                    eprintln!("[{}] expanded item hidden by macOS for 1 s: collapsing", local_time());
                }
                self.settings.display_mode = Some(DisplayMode::Collapsed);
                self.settings.collapsed_for_space = true;
                self.save();
                if self.menu_open {
                    self.dirty = true;
                } else {
                    self.render();
                }
                notify::post(&alerts::Alert {
                    id: format!("collapsed-{}", unix_now()),
                    title: "battbar collapsed".into(),
                    body: "Not enough space in the menu bar to show all devices. Free up space and choose Expand in \
                           the battbar menu to try again."
                        .into(),
                });
            }
            Some(overflow::Outcome::Fits) if self.settings.collapsed_for_space => {
                self.settings.collapsed_for_space = false;
                self.save();
                if self.menu_open {
                    self.dirty = true;
                } else {
                    self.render();
                }
            }
            _ => {}
        }
    }
}

/// Autosave name of the single status item. Up to 0.3 every device had its own item
/// (`battbar_<id>`, and `battbar___battbar` for the neutral icon).
const STATUS_ITEM_AUTOSAVE: &str = "battbar";

/// User defaults keys of the per-device status items of 0.3 and older
/// (`NSStatusItem Preferred Position battbar_…`, `NSStatusItem Visible battbar_…`).
fn is_old_status_item_key(key: &str) -> bool {
    key.starts_with("NSStatusItem ") && key.contains(" battbar_")
}

/// Remove the saved positions of the old per-device status items.
fn forget_old_status_items() {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    let keys = defaults.dictionaryRepresentation().allKeys();
    for k in keys.iter() {
        if is_old_status_item_key(&k.to_string()) {
            defaults.removeObjectForKey(&k);
        }
    }
}

fn deliver(u: PollUpdate, retries: u8) {
    APP.with(|c| match c.try_borrow_mut() {
        Ok(mut g) => {
            let post = g.as_mut().and_then(|a| a.on_update(u));
            drop(g);
            if let Some(p) = post {
                run_post(p);
            }
        }
        Err(_) if retries > 0 => dispatch2::DispatchQueue::main().exec_async(move || deliver(u, retries - 1)),
        Err(_) => {}
    });
}

/// Sample devices in several states, each layout written as `<name>.png` (2×, black +
/// alpha) plus `index.tsv` (names).
#[cfg(debug_assertions)]
fn render_bar_preview(dir: &std::path::Path) {
    let dev = |id: &str, name: &str, kind: DeviceKind, r: Reading| Device {
        id: id.into(),
        name: name.into(),
        kind,
        reading: r,
        percent_source: None,
        components: vec![],
        sources: vec![],
        estimate: None,
    };
    let mut approx = Reading::ok(88, Precision::Approximate);
    approx.voltage_mv = Some(4054);
    let mut charging = Reading::ok(60, Precision::Exact);
    charging.charging = Some(true);
    let k = dev("k", "G915", DeviceKind::Keyboard, approx);
    let m = dev("m", "G502 X PLUS", DeviceKind::Mouse, Reading::ok(52, Precision::Exact));
    let m_low = dev("m", "G502 X PLUS", DeviceKind::Mouse, Reading::ok(12, Precision::Exact));
    let h = dev("h", "Arctis Nova Pro", DeviceKind::Headset, charging);
    let h_ok = dev("h", "Arctis Nova Pro", DeviceKind::Headset, Reading::ok(83, Precision::Exact));
    let p = dev("p", "DualSense", DeviceKind::Gamepad, Reading::with_status(Status::Asleep));
    use ui::{CollapsedStyle as S, Layout as L};
    let cases: Vec<(&str, Vec<&Device>, L, S)> = vec![
        ("expanded", vec![&k, &m, &h_ok], L::Expanded, S::Glyph),
        ("expanded-charging-asleep", vec![&k, &m, &h, &p], L::Expanded, S::Glyph),
        ("collapsed-glyph", vec![&k, &m, &h_ok], L::Collapsed, S::Glyph),
        ("collapsed-lowest", vec![&k, &m, &h_ok], L::Collapsed, S::Lowest),
        ("collapsed-low", vec![&k, &m_low, &h_ok], L::Collapsed, S::Glyph),
        ("no-devices", vec![], L::Expanded, S::Glyph),
    ];
    let _ = std::fs::create_dir_all(dir);
    let mut index = String::new();
    for (name, devices, layout, style) in cases {
        let view = ui::bar_view(&devices, layout, Some(15), style);
        let Some(img) = bar::image(&view) else { continue };
        match bar::write_png(&img, &dir.join(format!("{name}.png")), 2.0) {
            Ok(()) => index.push_str(&format!("{name}\n")),
            Err(e) => eprintln!("{name}: {e}"),
        }
    }
    let _ = std::fs::write(dir.join("index.tsv"), index);
    println!("wrote {}", dir.display());
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("battbar {}", battbar_core::version::FULL);
        return;
    }
    // Used by the Homebrew cask on uninstall: remove the login item, no UI.
    if std::env::args().any(|a| a == "--unregister-login-item") {
        if matches!(login::status(), login::Status::Enabled | login::Status::RequiresApproval) {
            if let Err(e) = login::set(false) {
                eprintln!("battbar: {e}");
            }
        }
        return;
    }
    // Development check: `--translocation-info <path of a running translocated app>`.
    if let Some(i) = std::env::args().position(|a| a == "--translocation-info") {
        let p = std::env::args().nth(i + 1).unwrap_or_else(battbar_macos::bundle::path);
        println!("{p} -> {:?}", battbar_macos::bundle::original_path_of(&p));
        return;
    }
    // Discovery for automatic collapsing (see probe.rs). No devices, no settings.
    if let Some(i) = std::env::args().position(|a| a == "--probe-menu-bar") {
        let args: Vec<String> = std::env::args().skip(i + 1).collect();
        let num = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|j| args.get(j + 1))
                .and_then(|w| w.parse::<f64>().ok())
        };
        let width = num("--width").unwrap_or(120.0);
        let grow = num("--grow").map(|w| (w, num("--after").unwrap_or(10.0)));
        let ramp = num("--ramp").map(|step| (step, num("--every").unwrap_or(3.0)));
        probe::run(width, grow, ramp);
        return;
    }
    // Development: render the status item for sample devices (all layouts) as PNGs.
    #[cfg(debug_assertions)]
    if let Some(i) = std::env::args().position(|a| a == "--render-bar-preview") {
        let dir = std::path::PathBuf::from(std::env::args().nth(i + 1).unwrap_or_else(|| ".".into()));
        render_bar_preview(&dir);
        return;
    }
    #[cfg(debug_assertions)]
    if std::env::args().any(|a| a == "--probe-first-run-alert") {
        probe_first_run_alert();
        return;
    }
    if std::env::args().any(|a| a == "--login-status") {
        println!(
            "{:?} translocated={} path={}",
            login::status(),
            battbar_macos::bundle::is_translocated(),
            battbar_macos::bundle::path()
        );
        return;
    }
    if battbar_macos::bundle::is_dev_build() {
        battbar_core::settings::use_dev_profile();
        // Do not take the release app's HID++ replies (and vice versa).
        battbar_hidpp::protocol::use_development_software_id();
    }
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let settings = Settings::load_and_init();
    let settings_identity = settings.identity_memory.clone();
    let tx = worker::spawn(&settings, |u| {
        dispatch2::DispatchQueue::main().exec_async(move || deliver(u, 5));
    });
    let state = App {
        mtm,
        handler: Handler::new(mtm),
        settings,
        reports: vec![],
        devices: vec![],
        item: None,
        overflow: overflow::OverflowGuard::default(),
        bar_key: String::new(),
        auto_collapse: overflow::auto_collapse_supported(
            objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion().majorVersion,
        ),
        overflow_timer: None,
        occlusion_watch: None,
        shown: vec![],
        actions: vec![],
        tx,
        alerts: AlertTracker::default(),
        identities: IdentityMemory::from_stored(&settings_identity),
        gate: NewDeviceGate::default(),
        db: DeviceDb::load_with_overrides(DeviceDb::user_dir().as_deref())
            .unwrap_or_else(|_| DeviceDb::embedded().expect("built-in device database")),
        settling: SettlingTracker::default(),
        admitted: vec![],
        last_update: None,
        timed_out: vec![],
        bt_authorization: battbar_macos::ble::authorization(),
        bt_alert_shown: false,
        menu_open: false,
        dirty: false,
        polled_once: false,
        update_in_flight: false,
    };
    APP.with(|c| *c.borrow_mut() = Some(state));
    // Re-opening battbar while it runs collapses it (see `on_reopen`). The delegate is
    // weak; the handler lives in the app state.
    with_app(|a| app.setDelegate(Some(ProtocolObject::from_ref(&*a.handler))));
    forget_old_status_items();
    with_app(|a| a.watch_bluetooth());
    with_app(|a| a.render());
    // Screens or resolution changed: the item is placed again; restart the check.
    let screens_changed = block2::RcBlock::new(|_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
        with_app(|a| a.on_screens_changed());
    });
    let _screens_observer = unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(objc2_app_kit::NSApplicationDidChangeScreenParametersNotification),
            None,
            Some(&objc2_foundation::NSOperationQueue::mainQueue()),
            &screens_changed,
        )
    };
    // Another space (leaving a full-screen app): the menu bar may be back with the item
    // still hidden; look again.
    let space_changed = block2::RcBlock::new(|_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
        with_app(|a| a.on_occlusion());
    });
    let _space_observer = unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(objc2_app_kit::NSWorkspaceActiveSpaceDidChangeNotification),
                None,
                Some(&objc2_foundation::NSOperationQueue::mainQueue()),
                &space_changed,
            )
    };
    notify::request_authorization();
    // First-run question, once the run loop is up. Postponed while App Translocation is
    // active (a login item registered from the randomised copy would break); answered
    // silently if Launch at Login is already on.
    dispatch2::DispatchQueue::main().exec_async(|| {
        // Translocated: no login question (it would register a temporary path); tell the
        // user once how to move the app instead. The menu keeps "Move battbar to
        // Applications…" for later.
        if battbar_macos::bundle::is_translocated() {
            if with_app(|a| !a.settings.warned_translocation) == Some(true) {
                run_post(Post::TranslocationNotice);
            }
            return;
        }
        let ask = with_app(|a| {
            if a.settings.asked_launch_at_login {
                return false;
            }
            // Already on (or waiting for approval): nothing to ask. NotFound is what an app
            // that never registered gets, so it is asked like NotRegistered.
            if matches!(login::status(), login::Status::Enabled | login::Status::RequiresApproval) {
                a.settings.asked_launch_at_login = true;
                a.save();
                return false;
            }
            true
        });
        if ask == Some(true) {
            // Also offers the update check (checkbox, unchecked).
            run_post(Post::AskLaunchAtLogin);
        } else if with_app(|a| !a.settings.asked_update_check) == Some(true) {
            // Installs that answered the login question before 0.3.0.
            run_post(Post::AskUpdateCheck);
        }
    });
    app.run();
}

#[cfg(test)]
mod status_item_tests {
    use super::*;

    /// The app never schedules a repeating timer (energy: no periodic wake-ups); only
    /// the diagnostic probe does. Checked in the sources.
    #[test]
    fn no_repeating_timers_in_the_app() {
        let sources = [
            ("main.rs", include_str!("main.rs")),
            ("bar.rs", include_str!("bar.rs")),
            ("overflow.rs", include_str!("overflow.rs")),
            ("ui.rs", include_str!("ui.rs")),
            ("worker.rs", include_str!("worker.rs")),
            ("alerts.rs", include_str!("alerts.rs")),
            ("updates.rs", include_str!("updates.rs")),
        ];
        let call = concat!("scheduledTimerWithTimeInterval_repeats", "_block(");
        for (name, src) in sources {
            for (i, _) in src.match_indices(call) {
                let args: String = src[i + call.len()..].chars().take_while(|c| *c != ')').collect();
                let repeats = args.split(',').nth(1).map(str::trim);
                assert_eq!(repeats, Some("false"), "{name}: repeating NSTimer: {args}");
            }
        }
    }

    #[test]
    fn only_the_old_per_device_items_are_forgotten() {
        assert!(is_old_status_item_key("NSStatusItem Preferred Position battbar_hidpp_9458c379bb62"));
        assert!(is_old_status_item_key("NSStatusItem Visible battbar___battbar"));
        assert!(!is_old_status_item_key("NSStatusItem Preferred Position battbar"), "the new item");
        assert!(!is_old_status_item_key("NSStatusItem Visible battbar"));
        assert!(!is_old_status_item_key("battbar_something"));
        assert!(!is_old_status_item_key("NSStatusItem Preferred Position other_battbar"));
    }
}
