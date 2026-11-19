// SPDX-License-Identifier: GPL-3.0-or-later
//! battbar menu bar app: one status item per device.

mod alerts;
mod ui;
mod worker;

use alerts::{Alert, AlertTracker};
use battbar_core::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSCellImagePosition, NSControlStateValueOff, NSControlStateValueOn, NSImage,
    NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSWorkspace,
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
    SetChemistry(String, Chemistry),
}

/// Work that must run after the app state borrow is released (modal UI, terminate).
enum Post {
    Quit,
    BluetoothAlert,
    Message(String),
    /// First-run question (asked once).
    AskLaunchAtLogin,
    /// Launch at Login needs approval or cannot find the app: explain, offer the pane.
    LoginItemHelp(login::Status),
}

struct App {
    mtm: MainThreadMarker,
    handler: Retained<Handler>,
    settings: Settings,
    reports: Vec<DeviceReport>,
    devices: Vec<Device>,
    items: Vec<(String, Retained<NSStatusItem>)>,
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
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

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
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&ns("Launch battbar at login?"));
            alert.setInformativeText(&ns(
                "battbar can start automatically when you log in, so the battery icons are always there. \
                 You can change this later under Settings in the battbar menu.",
            ));
            alert.addButtonWithTitle(&ns("Launch at Login"));
            alert.addButtonWithTitle(&ns("Not Now"));
            let yes = alert.runModal() == 1000;
            with_app(|a| {
                a.settings.asked_launch_at_login = true; // "Not Now" = do not ask again
                a.save();
            });
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
    }
}

impl App {
    fn recompute(&mut self) {
        self.devices = merge(self.reports.clone(), &self.settings.merge_config());
        let settings = &self.settings;
        battbar_core::cells::apply_chemistry(&mut self.devices, &self.db, |id| settings.chemistry_for(id));
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
        self.recompute();
        for d in &self.devices {
            if d.reading.percent.is_some() && !self.settings.ever_reported.contains(&d.id) {
                self.settings.ever_reported.push(d.id.clone());
                changed = true;
            }
        }
        if changed {
            self.save();
        }
        self.admitted = self.gate.admit(std::time::Instant::now(), &self.devices, complete);
        if self.gate.pending() {
            let _ = self.tx.send(WorkerCmd::RefreshIn(self.gate.settle + Duration::from_millis(500)));
        }
        self.check_alerts();
        self.last_update = Some(local_time());
        if std::env::var_os("BATTBAR_LOG").is_some() {
            eprintln!(
                "[{}] input_monitoring={} bt_auth={} timed_out={:?}",
                local_time(),
                battbar_hid::input_monitoring_status(),
                u.bt_authorization,
                self.timed_out
            );
            for d in &self.devices {
                let spare = d
                    .components
                    .iter()
                    .map(|c| format!(" {:?}={}", c.kind, ui::status_text(&c.reading)))
                    .collect::<String>();
                eprintln!(
                    "  {} [{:?}] {} via {:?}{spare}",
                    d.name,
                    d.kind,
                    ui::status_text(&d.reading),
                    d.live_transports()
                );
            }
        }
        self.bt_authorization = u.bt_authorization;
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
        self.recompute();
        self.dirty = true;
        None
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
        if visible {
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

    fn device_menu(&mut self, d: Option<&Device>) -> Retained<NSMenu> {
        let menu = NSMenu::new(self.mtm);
        menu.setAutoenablesItems(true);
        menu.setDelegate(Some(ProtocolObject::from_ref(&*self.handler)));
        match d {
            Some(d) => {
                self.add(&menu, &d.name, None, false, "");
                for l in ui::detail_lines(d) {
                    self.add(&menu, &format!("    {l}"), None, false, "");
                }
                let src = self.submenu(&menu, "    Sources");
                for l in ui::source_lines(d) {
                    self.add(&src, &l, None, false, "");
                }
                self.separator(&menu);
                self.device_actions(&menu, d, true);
            }
            None => {
                let text = if self.polled_once {
                    "No devices found"
                } else {
                    "Looking for devices…"
                };
                self.add(&menu, text, None, false, "");
            }
        }
        if let Some(t) = &self.last_update {
            self.add(&menu, &format!("Updated {t}"), None, false, "");
        }
        if !self.timed_out.is_empty() {
            self.add(&menu, &format!("No answer from: {}", self.timed_out.join(", ")), None, false, "");
        }
        let wants_alerts = self.settings.low_battery_threshold.is_some() || self.settings.notify_spare_full;
        if wants_alerts && notify::answered() && !notify::authorized() {
            self.add(
                &menu,
                "Notifications are off for battbar…",
                Some(Action::OpenNotificationSettings),
                false,
                "",
            );
        }
        if matches!(self.bt_authorization, 1 | 2) {
            self.add(&menu, "Bluetooth access needed…", Some(Action::OpenBluetoothSettings), false, "");
        }
        self.separator(&menu);
        self.add(&menu, "Refresh", Some(Action::Refresh), false, "r");
        let s = self.submenu(&menu, "Settings");
        self.settings_menu(&s);
        let o = self.submenu(&menu, "Other Devices");
        self.other_devices_menu(&o);
        self.separator(&menu);
        self.add(&menu, "Quit battbar", Some(Action::Quit), false, "q");
        menu
    }

    fn status_item(&mut self, key: &str) -> Retained<NSStatusItem> {
        if let Some((_, it)) = self.items.iter().find(|(k, _)| k == key) {
            return it.clone();
        }
        let it = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let autosave: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        it.setAutosaveName(Some(&ns(&format!("battbar_{autosave}"))));
        self.items.push((key.to_string(), it.clone()));
        it
    }

    fn render(&mut self) {
        self.dirty = false;
        self.actions.clear();
        // The app always keeps at least one status item (the neutral icon below).
        let visible: Vec<Device> = ui::visible(&self.devices, &self.settings)
            .into_iter()
            .filter(|d| self.admitted.contains(&d.id))
            .cloned()
            .collect();
        let keys: Vec<String> = if visible.is_empty() {
            vec!["__battbar".into()]
        } else {
            visible.iter().map(|d| d.id.clone()).collect()
        };

        let bar = NSStatusBar::systemStatusBar();
        self.items.retain(|(k, it)| {
            let keep = keys.contains(k);
            if !keep {
                bar.removeStatusItem(it);
            }
            keep
        });

        if visible.is_empty() {
            let it = self.status_item("__battbar");
            if let Some(b) = it.button(self.mtm) {
                // The app's own glyph (template image from the bundle); SF Symbol fallback.
                let img = NSImage::imageNamed(&ns("MenuBarIcon")).or_else(|| {
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns("battery.100percent"), Some(&ns("battbar")))
                });
                if let Some(i) = &img {
                    i.setTemplate(true);
                }
                b.setImage(img.as_deref());
                b.setTitle(&ns(""));
                b.setToolTip(Some(&ns("battbar")));
            }
            let menu = self.device_menu(None);
            it.setMenu(Some(&menu));
            return;
        }

        for d in &visible {
            let v = ui::item_view(d);
            let it = self.status_item(&d.id);
            if let Some(b) = it.button(self.mtm) {
                let img = NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(v.symbol), Some(&ns(&d.name)));
                if let Some(i) = &img {
                    i.setTemplate(true);
                }
                b.setImage(img.as_deref());
                b.setImagePosition(if v.title.is_empty() {
                    NSCellImagePosition::ImageOnly
                } else {
                    NSCellImagePosition::ImageLeft
                });
                b.setTitle(&ns(&v.title));
                b.setEnabled(true);
                // Native dimming (macOS additionally dims the whole menu bar on an
                // inactive display; that is not ours).
                b.setAppearsDisabled(v.dimmed);
                b.setToolTip(Some(&ns(&v.tooltip)));
            }
            let menu = self.device_menu(Some(d));
            it.setMenu(Some(&menu));
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

fn main() {
    // Used by the Homebrew cask on uninstall: remove the login item, no UI.
    if std::env::args().any(|a| a == "--unregister-login-item") {
        if matches!(login::status(), login::Status::Enabled | login::Status::RequiresApproval) {
            if let Err(e) = login::set(false) {
                eprintln!("battbar: {e}");
            }
        }
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
        items: vec![],
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
    };
    APP.with(|c| *c.borrow_mut() = Some(state));
    with_app(|a| a.render());
    notify::request_authorization();
    // First-run question, once the run loop is up. Postponed while App Translocation is
    // active (a login item registered from the randomised copy would break); answered
    // silently if Launch at Login is already on.
    dispatch2::DispatchQueue::main().exec_async(|| {
        let ask = with_app(|a| {
            if a.settings.asked_launch_at_login || battbar_macos::bundle::is_translocated() {
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
            run_post(Post::AskLaunchAtLogin);
        }
    });
    app.run();
}
