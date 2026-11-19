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
    version,
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

fn providers(cli: &Cli, db: &Arc<DeviceDb>, bt: bool, trace: Option<Arc<Mutex<TraceWriter>>>) -> Vec<Box<dyn BatteryProvider>> {
    let mut hid_sources: Vec<Box<dyn battbar_hidpp::LinkSource>> = vec![Box::new(battbar_hidpp::HidapiLinks::default())];
    #[cfg(target_os = "macos")]
    if bt {
        hid_sources.push(Box::new(battbar_hidpp::GattLinks::default()));
    }
    let mut hidpp = battbar_hidpp::HidppProvider::new(db.clone(), hid_sources);
    let mut ss = battbar_steelseries::SteelSeriesProvider::new(db.clone(), vec![Box::new(battbar_steelseries::HidapiBases)]);
    ss.headsetcontrol_fallback = !cli.no_headsetcontrol;
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
    ss.headsetcontrol_fallback = false;
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
        (Status::NotReported, _) => "n/r".into(),
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
        "battbar": env!("CARGO_PKG_VERSION"),
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
    let cli = Cli::parse();
    let db = db();
    let settings = Settings::load_and_init();
    match &cli.cmd {
        Cmd::Probe { json, replay, timeout } => {
            let provs = match replay {
                Some(p) => replay_providers(p, &db),
                None => {
                    let bt = bluetooth_available(&cli);
                    providers(&cli, &db, bt, None)
                }
            };
            let mut engine = Engine::new(provs);
            let res = engine.poll(Duration::from_secs(*timeout));
            let devices = merge(res.reports, &settings.merge_config());
            if *json {
                let out = json!({"battbar": env!("CARGO_PKG_VERSION"), "devices": devices, "timed_out": res.timed_out});
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                print_devices(&devices);
                if !res.timed_out.is_empty() {
                    eprintln!("(no answer in time from: {})", res.timed_out.join(", "));
                }
            }
            std::process::exit(0);
        }
        Cmd::DebugBt => {
            let bt = bluetooth_available(&cli);
            eprintln!("bt available: {bt}");
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
            let mut engine = Engine::new(providers(&cli, &db, bt, None));
            let diags = engine.diagnostics(Duration::from_secs(30));
            let res = engine.poll(Duration::from_secs(10));
            let devices = merge(res.reports, &settings.merge_config());
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
            let mut engine = Engine::new(providers(&cli, &db, bt, Some(w.clone())));
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
                let devices = merge(res.reports, &settings.merge_config());
                for d in &devices {
                    w.lock()
                        .unwrap()
                        .note(&format!("poll {}: {} = {}", i + 1, d.name, fmt_reading(&d.reading)));
                }
                print_devices(&devices);
            }
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
            "config.toml",
            home.as_str(),
        ] {
            assert!(!text.contains(word), "{word} in dump");
        }
        assert!(text.contains("~/.config/battbar/devices/my.toml"));
    }
}
