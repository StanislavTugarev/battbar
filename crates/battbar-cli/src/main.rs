// SPDX-License-Identifier: GPL-3.0-or-later
//! `battbar` command line: one-shot probe, diagnostic dump, traffic recording.

use battbar_core::transport::{Trace, TraceWriter};
use battbar_core::*;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "battbar",
    version = battbar_core::version::FULL,
    about = "Battery of wireless mice, keyboards and headsets (receivers, base stations, Bluetooth)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Skip Bluetooth sources (no Bluetooth permission needed).
    #[arg(long, global = true)]
    no_bluetooth: bool,
    /// Do not run the external headsetcontrol fallback.
    #[arg(long, global = true)]
    no_headsetcontrol: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Poll all devices once.
    Probe {
        #[arg(long)]
        json: bool,
        /// Answer from a recorded trace instead of real hardware.
        #[arg(long, value_name = "TRACE")]
        replay: Option<PathBuf>,
        /// Overall deadline in seconds.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Diagnostic report for bug reports (no personal data).
    Dump {
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Step-by-step Bluetooth diagnostics (development).
    #[command(hide = true)]
    DebugBt,
    /// Hotplug notifications (development): print HID devices appearing/disappearing and
    /// Bluetooth connects/disconnects for N seconds (main thread run loop). Read-only.
    #[command(hide = true)]
    DebugEvents {
        #[arg(long, default_value_t = 30)]
        seconds: u64,
    },
    /// Cost of the 2 s event check (development): poll the HID++ receivers and the
    /// SteelSeries base stations once, then run each provider's event check every 2 s
    /// for N rounds on this thread and print CPU time, context switches and wall time
    /// per check. USB only (no Bluetooth). Read-only.
    #[command(hide = true)]
    DebugCheckCost {
        #[arg(long, default_value_t = 30)]
        checks: u32,
    },
    /// Listen to a base station (development): open every vendor HID interface of the
    /// SteelSeries device (VID 0x1038) and print each incoming report with its time for
    /// N seconds. Sends nothing (shows whether the base reports headset changes itself).
    #[command(hide = true)]
    DebugBaseListen {
        #[arg(long, default_value_t = 180)]
        seconds: u64,
    },
    /// Open and close the vendor HID interfaces of USB receivers and base stations N
    /// times (development), then wait W seconds (to inspect this process's memory).
    /// Sends nothing.
    #[command(hide = true)]
    DebugOpenClose {
        #[arg(long, default_value_t = 20)]
        rounds: u32,
        #[arg(long, default_value_t = 60)]
        wait: u64,
    },
    /// Stress test (development): rebuild all providers N times and poll at once each
    /// time, to catch objects bound to threads that exit. Read-only.
    #[command(hide = true)]
    DebugRebuild {
        #[arg(default_value_t = 20)]
        rounds: u32,
    },
    /// Record the traffic with vendor interfaces (HID++ incl. Bluetooth GATT, base stations).
    Record {
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Number of polls.
        #[arg(long, default_value_t = 1)]
        polls: u32,
        /// Seconds between polls.
        #[arg(long, default_value_t = 10)]
        interval: u64,
        /// Free-form note stored in the trace, e.g. "G HUB shows 11 %".
        #[arg(long)]
        note: Vec<String>,
        /// Keep device unit IDs (by default they are replaced with stable pseudonyms).
        #[arg(long)]
        keep_ids: bool,
    },
}

fn db() -> Arc<DeviceDb> {
    match DeviceDb::load_with_overrides(DeviceDb::user_dir().as_deref()) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("battbar: device database: {e}");
            std::process::exit(2);
        }
    }
}

/// Bluetooth needs this process to be its own "responsible process" so that the
/// embedded Info.plist applies. Re-launch once if needed.
fn bluetooth_available(cli: &Cli) -> bool {
    if cli.no_bluetooth {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        if !battbar_macos::tcc::is_disclaimed() {
            if let Some(code) = battbar_macos::tcc::respawn_as_responsible() {
                std::process::exit(code);
            }
        }
        if !battbar_macos::tcc::bluetooth_usage_description_present() {
            eprintln!("battbar: Bluetooth sources disabled (no usage description in this build)");
            return false;
        }
        if battbar_macos::ble::is_not_determined() {
            eprintln!("battbar: macOS asks for Bluetooth permission for battbar; answer the dialog (waiting up to 60 s)");
            let _ = battbar_macos::ble::Central::shared();
        }
        if !battbar_macos::ble::is_authorized() {
            eprintln!(
                "battbar: Bluetooth permission not granted; Bluetooth sources skipped (System Settings > Privacy & Security > Bluetooth)"
            );
            return false;
        }
        true
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// `diagnostic`: `dump` / `record` — the HID++ provider never writes registers.
fn providers(
    cli: &Cli,
    db: &Arc<DeviceDb>,
    bt: bool,
    trace: Option<Arc<Mutex<TraceWriter>>>,
    diagnostic: bool,
) -> Vec<Box<dyn BatteryProvider>> {
    let mut hid_sources: Vec<Box<dyn battbar_hidpp::LinkSource>> = vec![Box::new(battbar_hidpp::HidapiLinks::default())];
    #[cfg(target_os = "macos")]
    if bt {
        hid_sources.push(Box::new(battbar_hidpp::GattLinks::default()));
    }
    let mut hidpp = battbar_hidpp::HidppProvider::new(db.clone(), hid_sources);
    if diagnostic {
        hidpp = hidpp.read_only();
    }
    let mut ss = battbar_steelseries::SteelSeriesProvider::new(db.clone(), vec![Box::new(battbar_steelseries::HidapiBases)]);
    ss.set_headsetcontrol(!cli.no_headsetcontrol);
    if let Some(w) = trace {
        hidpp = hidpp.with_trace(w.clone());
        ss = ss.with_trace(w);
    }
    let mut v: Vec<Box<dyn BatteryProvider>> = vec![Box::new(hidpp), Box::new(ss)];
    #[cfg(target_os = "macos")]
    if bt {
        v.push(Box::new(battbar_bt_system::BtSystemProvider::default()));
    }
    v
}

fn replay_providers(path: &PathBuf, db: &Arc<DeviceDb>) -> Vec<Box<dyn BatteryProvider>> {
    let trace = Trace::from_file(path).unwrap_or_else(|e| {
        eprintln!("battbar: {}: {e}", path.display());
        std::process::exit(2);
    });
    let mut ss =
        battbar_steelseries::SteelSeriesProvider::new(db.clone(), vec![Box::new(battbar_steelseries::ReplayBases::new(&trace, db))]);
    ss.set_headsetcontrol(false);
    vec![
        Box::new(battbar_hidpp::HidppProvider::new(
            db.clone(),
            vec![Box::new(battbar_hidpp::ReplaySource::new(&trace, db))],
        )),
        Box::new(ss),
    ]
}

fn fmt_percent(r: &Reading) -> String {
    match (r.status, r.percent) {
        (Status::Ok, Some(p)) => {
            let tilde = if r.precision == Precision::Exact { "" } else { "~" };
            format!("{tilde}{p}%")
        }
        (Status::NotReported, _) => match r.note {
            Some(ReadingNote::NonRechargeableBattery) => "non-rechargeable battery (not supported yet)".into(),
            Some(ReadingNote::Hidpp10Device) => "n/r (HID++ 1.0)".into(),
            Some(ReadingNote::CalibratedVoltage) | None => "n/r".into(),
        },
        (s, _) => format!("{s:?}").to_lowercase(),
    }
}

fn fmt_reading(r: &Reading) -> String {
    let mut parts = vec![fmt_percent(r)];
    if r.charging == Some(true) {
        parts.push("charging".into());
    } else if r.external_power == Some(true) {
        parts.push("external power".into());
    }
    if let Some(mv) = r.voltage_mv {
        parts.push(format!("{mv} mV"));
    }
    if r.status == Status::Ok && r.percent.is_some() {
        parts.push(format!("{:?}", r.precision).to_lowercase());
    }
    if r.note == Some(ReadingNote::CalibratedVoltage) {
        parts.push("calibrated".into());
    }
    parts.join(", ")
}

fn print_devices(devs: &[Device]) {
    if devs.is_empty() {
        println!("No devices found.");
    }
    for d in devs {
        let note = match d.estimate {
            Some(Estimate::Charging) => "  (estimated while charging)",
            Some(Estimate::Settling) => "  (estimated, settling after charging)",
            None => "",
        };
        println!("{}  [{:?}]  {}{note}", d.name, d.kind, fmt_reading(&d.reading));
        for c in &d.components {
            println!("    {:?}: {}", c.kind, fmt_reading(&c.reading));
        }
        for s in &d.sources {
            let mark = if d.percent_source.as_deref() == Some(&s.full_key()) {
                "*"
            } else {
                " "
            };
            println!(
                "  {mark} {:<14} {:<26} {}",
                s.transport.to_string(),
                s.source,
                fmt_reading(&s.reading)
            );
        }
    }
}

fn redact_name(n: &str) -> String {
    // Paired-device names can contain people's names ("Anna's AirPods").
    if n.contains('\'') || n.contains('’') || n.to_lowercase().contains("iphone") {
        format!("<redacted:{}>", short_hash(n))
    } else {
        n.to_string()
    }
}

fn redact(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if k == "name" {
                    if let serde_json::Value::String(s) = x {
                        *s = redact_name(s);
                    }
                } else {
                    redact(x);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(redact),
        _ => {}
    }
}

/// Pseudonym for a unit ID in traces: the reserved prefix `bba7` (checked in CI)
/// followed by 4 hex digits of a salted hash.
fn pseudonym(id: &str) -> String {
    if id.starts_with(battbar_core::transport::PSEUDONYM_PREFIX) {
        return id.to_string(); // already a pseudonym (replayed trace)
    }
    format!(
        "{}{}",
        battbar_core::transport::PSEUDONYM_PREFIX,
        &short_hash(&format!("battbar-trace:{id}"))[..4]
    )
}

/// Replace HID++ unit IDs in a trace with stable pseudonyms (same ID → same pseudonym,
/// so merging still works when the trace is replayed).
fn pseudonymize(path: &PathBuf, unit_ids: &[String]) -> std::io::Result<()> {
    let mut text = std::fs::read_to_string(path)?;
    for id in unit_ids {
        let Some(bytes) = transport::from_hex(
            &id.as_bytes()
                .chunks(2)
                .map(|c| std::str::from_utf8(c).unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" "),
        ) else {
            continue;
        };
        let pseudo = pseudonym(id);
        let pseudo = pseudo.as_str();
        let from = transport::to_hex(&bytes);
        let to = pseudo
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        text = text.replace(&from, &to);
        text = text.replace(id.as_str(), pseudo);
    }
    std::fs::write(path, text)
}

/// Merge reports, estimate replaceable-cell levels and apply learned voltage
/// calibration (same steps as the app; the CLI never learns or stores anything).
fn merged(reports: Vec<DeviceReport>, settings: &Settings, db: &DeviceDb) -> Vec<Device> {
    let mut d = merge(reports, &settings.merge_config());
    battbar_core::cells::apply_chemistry(&mut d, db, |id| settings.chemistry_for(id));
    battbar_core::calibration::apply(&settings.voltage_calibration, &mut d, db);
    d
}

/// `battbar dump` content. Built from diagnostics and merged devices only: the
/// settings (hash key, device IDs, links) are deliberately not an input.
fn dump_report(bt: bool, db_sources: &[String], diags: Vec<(&'static str, serde_json::Value)>, devices: &[Device]) -> serde_json::Value {
    let home = std::env::var("HOME").unwrap_or_default();
    let db: Vec<String> = db_sources
        .iter()
        .map(|s| if !home.is_empty() { s.replace(&home, "~") } else { s.clone() })
        .collect();
    let devices: Vec<serde_json::Value> = devices
        .iter()
        .map(|d| {
            json!({
                "name": d.name, "kind": d.kind, "reading": d.reading, "components": d.components, "estimate": d.estimate,
                "sources": d.sources.iter().map(|s| json!({"provider": s.provider, "source": s.source, "transport": s.transport, "reading": s.reading})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "battbar": battbar_core::version::FULL,
        "os": {"macos": os_version(), "arch": std::env::consts::ARCH},
        "device_database": db,
        "bluetooth_enabled": bt,
        "input_monitoring": battbar_hid::input_monitoring_status(),
        "providers": diags.into_iter().map(|(n, v)| (n.to_string(), v)).collect::<serde_json::Map<_, _>>(),
        "devices": devices,
    })
}

fn os_version() -> String {
    std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn main() {
    // Helper for long-running processes (the app): print paired Bluetooth devices with
    // current battery values and exit. No settings, no other sources, no re-launch.
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some(battbar_macos::classic::HELPER_ARG) {
        battbar_macos::classic::print_paired_devices_json();
        return;
    }
    let cli = Cli::parse();
    let db = db();
    #[cfg(target_os = "macos")]
    if battbar_macos::bundle::is_dev_build() {
        battbar_core::settings::use_dev_profile();
        // Own HID++ software ID next to a running release app; traces keep the release
        // ID so they replay in tests.
        let traces = matches!(&cli.cmd, Cmd::Record { .. } | Cmd::Probe { replay: Some(_), .. });
        if !traces {
            battbar_hidpp::protocol::use_development_software_id();
        }
    }
    let settings = Settings::load_and_init();
    match &cli.cmd {
        Cmd::Probe { json, replay, timeout } => {
            let provs = match replay {
                Some(p) => replay_providers(p, &db),
                None => {
                    let bt = bluetooth_available(&cli);
                    providers(&cli, &db, bt, None, false)
                }
            };
            let mut engine = Engine::new(provs);
            let res = engine.poll(Duration::from_secs(*timeout));
            let devices = merged(res.reports, &settings, &db);
            if *json {
                let out = json!({"battbar": battbar_core::version::FULL, "devices": devices, "timed_out": res.timed_out});
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                print_devices(&devices);
                if !res.timed_out.is_empty() {
                    eprintln!("(no answer in time from: {})", res.timed_out.join(", "));
                }
            }
            std::process::exit(0);
        }
        Cmd::DebugRebuild { rounds } => {
            let bt = bluetooth_available(&cli);
            let mut engine = Engine::new(providers(&cli, &db, bt, None, true));
            for i in 0..*rounds {
                let r = engine.poll(Duration::from_secs(15));
                let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
                for rep in &r.reports {
                    *by.entry(rep.provider.as_str()).or_default() += 1;
                }
                eprintln!("round {i}: {by:?} timed out: {:?}", r.timed_out);
                // Drop every provider thread and poll at once with new ones.
                engine = Engine::new(providers(&cli, &db, bt, None, true));
            }
            eprintln!("ok: {rounds} rebuilds, bluetooth={bt}");
            std::process::exit(0);
        }
        Cmd::DebugBaseListen { seconds } => base_listen(*seconds),
        Cmd::DebugOpenClose { rounds, wait } => {
            let ifaces: Vec<_> = battbar_hid::enumerate()
                .unwrap_or_default()
                .into_iter()
                .filter(|i| matches!(i.vendor_id, 0x046d | 0x1038) && i.is_vendor_only())
                .collect();
            println!("pid {}; {} interface(s)", std::process::id(), ifaces.len());
            for i in &ifaces {
                println!("  {}", i.link_id());
            }
            println!("ready (before opening)");
            std::thread::sleep(Duration::from_secs(*wait));
            for _ in 0..*rounds {
                for i in &ifaces {
                    // Closed in a pool, like on the app's provider threads.
                    battbar_core::objc_pool::with_pool(|| drop(battbar_hid::open(i)));
                }
            }
            println!("opened and closed {rounds} times");
            std::thread::sleep(Duration::from_secs(*wait));
            println!("done");
        }
        Cmd::DebugCheckCost { checks } => {
            check_cost(&db, *checks);
        }
        Cmd::DebugEvents { seconds } => {
            let bt = bluetooth_available(&cli);
            let start = std::time::Instant::now();
            battbar_hid::on_hotplug(move || println!("[{:>6.2}s] hid device appeared or disappeared", start.elapsed().as_secs_f64()));
            #[cfg(target_os = "macos")]
            {
                if bt {
                    let mtm = objc2::MainThreadMarker::new().expect("main thread");
                    battbar_macos::connections::watch(mtm, move |c| {
                        println!(
                            "[{:>6.2}s] bluetooth device {}",
                            start.elapsed().as_secs_f64(),
                            if c { "connected" } else { "disconnected" }
                        )
                    });
                }
                let _ = battbar_hid::enumerate(); // start the HID service thread
                println!("listening for {seconds} s (devices present now are reported once at start)");
                battbar_macos::classic::run_current_run_loop(*seconds as f64);
            }
            println!("events: hid={} bluetooth={}", battbar_hid::hotplug_events(), {
                #[cfg(target_os = "macos")]
                {
                    battbar_macos::connections::events()
                }
                #[cfg(not(target_os = "macos"))]
                0
            });
        }
        Cmd::DebugBt => {
            let bt = bluetooth_available(&cli);
            eprintln!("bt available: {bt}");
            // Watch classic battery values the way the app reads them: a long-lived
            // thread reads every 5 s while the main thread runs its run loop; a new
            // thread reads alongside for comparison.
            #[cfg(target_os = "macos")]
            if bt {
                if let Some(secs) = std::env::var("BATTBAR_DEBUG_WATCH").ok().and_then(|s| s.parse::<f64>().ok()) {
                    let start = std::time::Instant::now();
                    std::thread::spawn(move || loop {
                        let t = start.elapsed().as_secs();
                        for d in battbar_macos::classic::paired_devices().iter().filter(|d| d.is_audio()) {
                            eprintln!(
                                "[{t:>4}s] long-lived thread: {} connected={} single={:?}",
                                d.name, d.connected, d.battery_single
                            );
                        }
                        let fresh = std::thread::spawn(battbar_macos::classic::paired_devices)
                            .join()
                            .unwrap_or_default();
                        for d in fresh.iter().filter(|d| d.is_audio()) {
                            eprintln!(
                                "[{t:>4}s] new thread:         {} connected={} single={:?}",
                                d.name, d.connected, d.battery_single
                            );
                        }
                        for d in battbar_macos::classic::paired_devices_fresh().iter().filter(|d| d.is_audio()) {
                            eprintln!(
                                "[{t:>4}s] helper process:     {} connected={} single={:?}",
                                d.name, d.connected, d.battery_single
                            );
                        }
                        std::thread::sleep(std::time::Duration::from_secs(5));
                    });
                    if std::env::var_os("BATTBAR_DEBUG_NO_RUNLOOP").is_some() {
                        std::thread::sleep(std::time::Duration::from_secs_f64(secs));
                    } else {
                        battbar_macos::classic::run_current_run_loop(secs);
                    }
                    std::process::exit(0);
                }
            }
            #[cfg(target_os = "macos")]
            if bt && std::env::var_os("BATTBAR_DEBUG_RUNLOOP").is_some() {
                // Does IOBluetooth attach run loop sources to the calling thread (so its
                // cached device properties update only while that run loop runs)?
                let h = std::thread::spawn(|| {
                    use battbar_macos::classic::*;
                    let count = |d: &str| d.matches("CFRunLoopSource").count();
                    let before = current_run_loop_description();
                    let first = paired_devices();
                    let after = current_run_loop_description();
                    eprintln!(
                        "run loop sources on the reading thread: before {} after {}",
                        count(&before),
                        count(&after)
                    );
                    for l in after.lines().filter(|l| l.contains("IOBluetooth") || l.contains("Bluetooth")) {
                        eprintln!("  {}", l.trim().chars().take(160).collect::<String>());
                    }
                    let show = |tag: &str, v: &[PairedDevice]| {
                        for d in v.iter().filter(|d| d.is_audio()) {
                            eprintln!("  {tag}: {} connected={} single={:?}", d.name, d.connected, d.battery_single);
                        }
                    };
                    show("first read", &first);
                    show("second read, immediately", &paired_devices());
                    for ms in [200u64, 1000, 2000] {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                        show(&format!("read after +{ms} ms"), &paired_devices());
                    }
                    let secs: f64 = std::env::var("BATTBAR_DEBUG_RUNLOOP")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    if secs > 0.0 {
                        eprintln!("waiting {secs} s without running the run loop…");
                        std::thread::sleep(std::time::Duration::from_secs_f64(secs));
                        show("no run loop", &paired_devices());
                        run_current_run_loop(1.0);
                        show("after run loop", &paired_devices());
                    }
                });
                let _ = h.join();
            }
            #[cfg(target_os = "macos")]
            {
                eprintln!("authorization: {}", battbar_macos::ble::authorization());
                eprintln!("main thread? {}", std::thread::current().name().unwrap_or("?"));
                let t = std::time::Instant::now();
                let c = battbar_macos::ble::Central::shared();
                eprintln!("central: {:?} in {:?}", c.as_ref().map(|_| "ok"), t.elapsed());
                if let Ok(c) = c {
                    let p = c.connected_peripherals(&["180F", &db.hidpp.gatt_service]);
                    eprintln!("peripherals: {p:?}");
                }
                let t = std::time::Instant::now();
                let h = std::thread::spawn(|| battbar_macos::classic::paired_devices().len());
                eprintln!("classic (worker thread): {:?} in {:?}", h.join(), t.elapsed());
            }
            std::process::exit(0);
        }
        Cmd::Dump { out } => {
            let bt = bluetooth_available(&cli);
            let mut engine = Engine::new(providers(&cli, &db, bt, None, true));
            let diags = engine.diagnostics(Duration::from_secs(30));
            let res = engine.poll(Duration::from_secs(10));
            let devices = merged(res.reports, &settings, &db);
            let mut report = dump_report(bt, &db.sources, diags, &devices);
            redact(&mut report);
            let text = serde_json::to_string_pretty(&report).unwrap();
            match out {
                Some(p) => {
                    std::fs::write(p, text).unwrap_or_else(|e| {
                        eprintln!("battbar: {}: {e}", p.display());
                        std::process::exit(1)
                    });
                    eprintln!("Wrote {}", p.display());
                }
                None => println!("{text}"),
            }
            std::process::exit(0);
        }
        Cmd::Record {
            out,
            polls,
            interval,
            note,
            keep_ids,
        } => {
            let bt = bluetooth_available(&cli);
            let file = std::fs::File::create(out).unwrap_or_else(|e| {
                eprintln!("battbar: {}: {e}", out.display());
                std::process::exit(1)
            });
            let w = TraceWriter::new(Box::new(file));
            for n in note {
                w.lock().unwrap().note(n);
            }
            let mut engine = Engine::new(providers(&cli, &db, bt, Some(w.clone()), true));
            let mut unit_ids: Vec<String> = vec![];
            for i in 0..*polls {
                if i > 0 {
                    std::thread::sleep(Duration::from_secs(*interval));
                }
                let res = engine.poll(Duration::from_secs(15));
                for r in &res.reports {
                    for id in &r.identity {
                        if let IdentityKey::HidppUnit { unit_id } = id {
                            if !unit_ids.contains(unit_id) {
                                unit_ids.push(unit_id.clone());
                            }
                        }
                    }
                }
                let devices = merged(res.reports, &settings, &db);
                for d in &devices {
                    w.lock()
                        .unwrap()
                        .note(&format!("poll {}: {} = {}", i + 1, d.name, fmt_reading(&d.reading)));
                }
                print_devices(&devices);
            }
            // Feature tables and HID++ 1.0 battery registers (read only) into the trace too.
            let _ = engine.diagnostics(Duration::from_secs(30));
            w.lock().unwrap().note("diagnostics captured (read only)");
            drop(engine);
            drop(w);
            if !keep_ids {
                // Also every unit ID in the frames themselves (devices without battery
                // reports answer 0x0003 too).
                match Trace::from_file(out) {
                    Ok(t) => {
                        for u in battbar_hidpp::trace_ids::unit_ids_in_trace(&t) {
                            let id: String = u.iter().map(|b| format!("{b:02x}")).collect();
                            if !unit_ids.contains(&id) {
                                unit_ids.push(id);
                            }
                        }
                    }
                    Err(e) => eprintln!("battbar: reading back the trace: {e}"),
                }
                if let Err(e) = pseudonymize(out, &unit_ids) {
                    // Fail closed: never leave a trace with real unit IDs behind.
                    let _ = std::fs::remove_file(out);
                    eprintln!("battbar: could not redact unit IDs ({e}); trace removed");
                    std::process::exit(1);
                }
            }
            eprintln!("Recorded vendor-interface traffic to {}", out.display());
            std::process::exit(0);
        }
    }
}

fn base_listen(seconds: u64) {
    use battbar_core::transport::FrameTransport;
    let ifaces: Vec<_> = battbar_hid::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| i.vendor_id == 0x1038)
        .collect();
    let start = std::time::Instant::now();
    let deadline = start + Duration::from_secs(seconds);
    let mut threads = vec![];
    for i in ifaces {
        let name = format!("{} {:?}", i.link_id(), i.collections);
        match battbar_hid::open(&i) {
            Ok(mut t) => {
                println!("listening: {name}");
                threads.push(std::thread::spawn(move || {
                    while std::time::Instant::now() < deadline {
                        match t.read(Duration::from_millis(500)) {
                            Ok(Some(f)) => {
                                let end = f.iter().rposition(|b| *b != 0).map_or(1, |p| p + 1);
                                let clock = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default();
                                println!(
                                    "[{:>7.2}s | {:.3}] {} ({} bytes): {}",
                                    start.elapsed().as_secs_f64(),
                                    clock.as_secs_f64(),
                                    t.info().link,
                                    f.len(),
                                    battbar_core::transport::to_hex(&f[..end])
                                );
                            }
                            Ok(None) => {}
                            Err(e) => {
                                println!("{}: {e}", t.info().link);
                                break;
                            }
                        }
                    }
                }));
            }
            Err(e) => println!("not opened: {name}: {e}"),
        }
    }
    for t in threads {
        let _ = t.join();
    }
    println!("done after {:.0} s", start.elapsed().as_secs_f64());
}

/// CPU time of this thread (s) and context switches of this process.
fn usage_now() -> (f64, i64) {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: plain libc calls filling the structs passed in.
    unsafe {
        libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts);
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
    }
    (ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9, ru.ru_nvcsw + ru.ru_nivcsw)
}

fn check_cost(db: &Arc<DeviceDb>, checks: u32) {
    let hidpp = battbar_hidpp::HidppProvider::new(db.clone(), vec![Box::new(battbar_hidpp::HidapiLinks::default())]).read_only();
    let mut ss = battbar_steelseries::SteelSeriesProvider::new(db.clone(), vec![Box::new(battbar_steelseries::HidapiBases)]);
    ss.set_headsetcontrol(false);
    let provs: Vec<Box<dyn BatteryProvider>> = vec![Box::new(hidpp), Box::new(ss)];
    for mut p in provs {
        let reports = p.poll();
        println!("{}: {} device(s) after the poll", p.name(), reports.len());
        for r in &reports {
            println!("  {} [{:?}] {:?}", r.name, r.kind, r.reading.status);
        }
        let (mut cpu, mut csw, mut wall, mut events, mut max_cpu) = (0.0, 0i64, 0.0, 0u32, 0.0f64);
        for _ in 0..checks {
            std::thread::sleep(Duration::from_secs(2));
            let (c0, s0) = usage_now();
            let t0 = std::time::Instant::now();
            events += p.check_events() as u32;
            let w = t0.elapsed().as_secs_f64();
            let (c1, s1) = usage_now();
            cpu += c1 - c0;
            max_cpu = max_cpu.max(c1 - c0);
            csw += s1 - s0;
            wall += w;
        }
        let n = checks.max(1) as f64;
        println!(
            "  per check ({checks} checks): CPU {:.0} µs (max {:.0} µs), context switches {:.1}, wall {:.2} ms; checks asking for a poll: {events}",
            cpu / n * 1e6,
            max_cpu * 1e6,
            csw as f64 / n,
            wall / n * 1e3
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(provider: &str, key: &str, identity: Vec<IdentityKey>) -> DeviceReport {
        DeviceReport {
            provider: provider.into(),
            source_key: key.into(),
            source: "t".into(),
            name: "Headset".into(),
            kind: DeviceKind::Headset,
            transport: Transport::BluetoothClassic,
            identity,
            hints: vec![],
            reading: Reading::ok(80, Precision::Exact),
            components: vec![],
        }
    }

    /// The settings never reach `battbar dump`: no hash key, no device IDs, no links,
    /// no home directory path.
    #[test]
    fn dump_contains_nothing_from_the_settings() {
        let mut s = Settings {
            salt: battbar_core::ids::new_salt(),
            ..Settings::default()
        };
        battbar_core::ids::set_install_salt(&s.salt);
        let bt = keyed_hash("bt-address", "AA-BB-CC-DD-EE-FF");
        let reports = vec![
            report(
                "bt-system",
                &format!("classic:{bt}"),
                vec![IdentityKey::Stable {
                    namespace: "bt".into(),
                    value: bt.clone(),
                }],
            ),
            report(
                "hidpp",
                "receiver:1",
                vec![IdentityKey::HidppUnit {
                    unit_id: "1a2b3c4d".into(),
                }],
            ),
        ];
        let devices = merge(reports, &s.merge_config());
        s.hidden = devices.iter().map(|d| d.id.clone()).collect();
        s.ever_reported = s.hidden.clone();
        s.learned_links.push(LearnedLink {
            base: "steelseries/base".into(),
            bluetooth: format!("bt-system/classic:{bt}"),
            name: "Headset".into(),
        });
        for id in &s.hidden {
            let mut c = battbar_core::calibration::VoltageCalibration::new(id);
            for mv in [4321, 4301, 4281] {
                c.add(100, mv);
            }
            s.voltage_calibration.push(c);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/someone".into());
        let db_sources = vec!["builtin:hidpp.toml".to_string(), format!("{home}/.config/battbar/devices/my.toml")];
        let text = serde_json::to_string(&dump_report(true, &db_sources, vec![], &devices)).unwrap();
        assert!(!text.contains(&s.salt), "hash key leaked");
        assert!(!text.contains(&bt), "Bluetooth address hash leaked");
        assert!(!text.contains("1a2b3c4d"), "unit ID leaked");
        for id in &s.hidden {
            assert!(!text.contains(id.as_str()), "device ID {id} leaked");
        }
        for word in [
            "salt",
            "learned_links",
            "identity_memory",
            "ever_reported",
            "voltage_calibration",
            "4321",
            "config.toml",
            home.as_str(),
        ] {
            assert!(!text.contains(word), "{word} in dump");
        }
        assert!(text.contains("~/.config/battbar/devices/my.toml"));
    }
}
