// SPDX-License-Identifier: GPL-3.0-or-later
//
// battbar-discovery: Stage 0 read-only hardware discovery tool.
//
// Safety rules (same as the shipped product):
//  * Enumeration never opens anything.
//  * Only HID interfaces whose *every* top-level collection is vendor-defined
//    (usage page >= 0xFF00) are opened. Anything that also carries Generic
//    Desktop / Keyboard collections is refused unless `--allow-mixed` is given,
//    and even then every non-HID++ input report is dropped unseen (only a counter).
//  * Devices are opened non-exclusively so G HUB / SteelSeries GG keep working.
//  * Only read-style requests are sent (pings, feature lookups, getters).
//
// HID++ request layout follows the public HID++ 2.0 documentation and the
// Solaar project (GPL-2.0-or-later). SteelSeries Arctis Nova Pro request is the
// one used by HeadsetControl (GPL-3.0).

use hidapi::{BusType, DeviceInfo, HidApi, HidDevice};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::time::{Duration, Instant};

const SW_ID: u8 = 0x0B;
const TIMEOUT: Duration = Duration::from_millis(1500);
const VID_LOGITECH: u16 = 0x046d;
const VID_STEELSERIES: u16 = 0x1038;

struct Opts {
    allow_mixed: bool,
    redact: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = Opts {
        allow_mixed: args.iter().any(|a| a == "--allow-mixed"),
        redact: !args.iter().any(|a| a == "--no-redact"),
    };
    let cmd = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(String::as_str)
        .unwrap_or("all");

    let api = HidApi::new().expect("hidapi init failed");
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);

    let groups = group_by_path(&api);
    match cmd {
        "enum" => cmd_enum(&groups),
        "hidpp" => cmd_hidpp(&api, &groups, &opts),
        "steelseries" => cmd_steelseries(&api, &groups),
        "all" => {
            cmd_enum(&groups);
            cmd_hidpp(&api, &groups, &opts);
            cmd_steelseries(&api, &groups);
        }
        other => {
            eprintln!("unknown command {other}; use enum | hidpp | steelseries | all [--allow-mixed] [--no-redact]");
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------- enumeration

type Groups<'a> = BTreeMap<CString, Vec<&'a DeviceInfo>>;

fn group_by_path(api: &HidApi) -> Groups<'_> {
    let mut g: Groups = BTreeMap::new();
    for d in api.device_list() {
        g.entry(d.path().to_owned()).or_default().push(d);
    }
    g
}

fn is_vendor_page(page: u16) -> bool {
    page >= 0xFF00
}

/// True if every top-level collection of this OS-level HID device is vendor-defined.
fn vendor_only(infos: &[&DeviceInfo]) -> bool {
    infos.iter().all(|d| is_vendor_page(d.usage_page()))
}

fn bus(d: &DeviceInfo) -> &'static str {
    match d.bus_type() {
        BusType::Usb => "USB",
        BusType::Bluetooth => "Bluetooth",
        BusType::I2c => "I2C",
        BusType::Spi => "SPI",
        BusType::Unknown => "unknown",
    }
}

fn page_name(page: u16) -> &'static str {
    match page {
        0x01 => "Generic Desktop",
        0x07 => "Keyboard",
        0x0C => "Consumer",
        p if is_vendor_page(p) => "Vendor",
        _ => "other",
    }
}

fn cmd_enum(groups: &Groups) {
    println!("== HID enumeration (hidapi, nothing opened) ==");
    for (path, infos) in groups {
        let d = infos[0];
        if d.vendor_id() == 0 {
            continue; // built-in SMC / sensors
        }
        println!(
            "{:04x}:{:04x} bus={} iface={} release=0x{:04x} product={:?} manufacturer={:?} path={}",
            d.vendor_id(),
            d.product_id(),
            bus(d),
            d.interface_number(),
            d.release_number(),
            d.product_string().unwrap_or(""),
            d.manufacturer_string().unwrap_or(""),
            path.to_string_lossy()
        );
        for i in infos {
            println!(
                "    collection usage_page=0x{:04x} ({}) usage=0x{:04x}",
                i.usage_page(),
                page_name(i.usage_page()),
                i.usage()
            );
        }
        println!(
            "    -> {}",
            if vendor_only(infos) {
                "vendor-only interface: may be opened"
            } else if infos.iter().any(|i| is_vendor_page(i.usage_page())) {
                "MIXED interface (vendor + input collections): must not be opened by default"
            } else {
                "input/consumer interface: never opened"
            }
        );
    }
    println!();
}

// --------------------------------------------------------------------- HID++

#[derive(Debug)]
enum HErr {
    Timeout,
    Io(String),
    Hidpp10(u8),
    Hidpp20(u8),
}

struct Hidpp<'a> {
    dev: &'a HidDevice,
    long_only: bool,
    other_hidpp: Vec<Vec<u8>>,
    dropped_non_hidpp: usize,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

impl<'a> Hidpp<'a> {
    /// Generic request. For HID++ 2.0: b2 = feature index, b3 = (function << 4) | sw id.
    /// For HID++ 1.0 registers: b2 = sub id, b3 = register address.
    fn raw(&mut self, idx: u8, b2: u8, b3: u8, params: &[u8]) -> Result<Vec<u8>, HErr> {
        let long = self.long_only || params.len() > 3;
        let mut req = vec![if long { 0x11 } else { 0x10 }, idx, b2, b3];
        req.extend_from_slice(params);
        req.resize(if long { 20 } else { 7 }, 0);
        self.dev.write(&req).map_err(|e| HErr::Io(e.to_string()))?;

        let deadline = Instant::now() + TIMEOUT;
        let mut buf = [0u8; 64];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(HErr::Timeout);
            }
            let n = self
                .dev
                .read_timeout(&mut buf, left.as_millis() as i32)
                .map_err(|e| HErr::Io(e.to_string()))?;
            if n == 0 {
                continue;
            }
            let r = &buf[..n];
            if r[0] != 0x10 && r[0] != 0x11 {
                // Not an HID++ report. On a vendor-only interface these are DJ / vendor
                // reports; on a mixed interface they may be keystrokes: never print them.
                self.dropped_non_hidpp += 1;
                continue;
            }
            if n < 7 || r[1] != idx {
                self.other_hidpp.push(r.to_vec());
                continue;
            }
            if r[2] == 0xFF && r[3] == b2 && r[4] == b3 {
                return Err(HErr::Hidpp20(r[5]));
            }
            if r[2] == 0x8F && r[3] == b2 && r[4] == b3 {
                return Err(HErr::Hidpp10(r[5]));
            }
            if r[2] == b2 && r[3] == b3 {
                return Ok(r[4..].to_vec());
            }
            self.other_hidpp.push(r.to_vec());
        }
    }

    fn call(&mut self, idx: u8, feat: u8, func: u8, params: &[u8]) -> Result<Vec<u8>, HErr> {
        self.raw(idx, feat, (func << 4) | SW_ID, params)
    }

    /// IRoot.getFeature -> (index, type, version)
    fn feature(&mut self, idx: u8, id: u16) -> Result<Option<(u8, u8, u8)>, HErr> {
        let r = self.call(idx, 0x00, 0, &[(id >> 8) as u8, id as u8])?;
        Ok(if r[0] == 0 { None } else { Some((r[0], r[1], r[2])) })
    }
}

fn feature_name(id: u16) -> &'static str {
    match id {
        0x0000 => "IRoot",
        0x0001 => "IFeatureSet",
        0x0002 => "IFeatureInfo",
        0x0003 => "DeviceInformation",
        0x0005 => "DeviceNameType",
        0x0007 => "DeviceFriendlyName",
        0x0008 => "KeepAlive",
        0x0020 => "ConfigChange",
        0x0021 => "UniqueRandomIdentifier",
        0x00C2 => "DfuControl(signed)",
        0x00C3 => "DfuControl",
        0x00D0 => "Dfu",
        0x1000 => "BatteryLevelStatus",
        0x1001 => "BatteryVoltage",
        0x1004 => "UnifiedBattery",
        0x1010 => "ChargingControl",
        0x1300 => "LedControl",
        0x1802 => "DeviceReset",
        0x1803 => "GpioAccess",
        0x1805 => "OobState",
        0x1806 => "ConfigDeviceProps",
        0x1814 => "ChangeHost",
        0x1815 => "HostsInfo",
        0x1981 => "Backlight",
        0x1982 => "Backlight2",
        0x1983 => "Backlight3",
        0x1990 => "Illumination",
        0x1B04 => "ReprogControlsV4",
        0x1D4B => "WirelessDeviceStatus",
        0x1DF3 => "EquadDJDebugInfo",
        0x1E00 => "EnableHiddenFeatures",
        0x1E02 => "ManageDeactivatableFeatures",
        0x1E22 => "SPIDirectAccess",
        0x1EB0 => "TDETestModeInit",
        0x1F1F => "FirmwareProperties",
        0x1F20 => "ADCMeasurement",
        0x2110 => "SmartShift",
        0x2111 => "SmartShiftEnhanced",
        0x2201 => "AdjustableDPI",
        0x2202 => "ExtendedAdjustableDPI",
        0x2250 => "AnalysisMode",
        0x40A3 => "Fn Inversion",
        0x4100 => "Encryption",
        0x4220 => "LockKeyState",
        0x4521 => "DisableKeys",
        0x4522 => "DisableKeysByUsage",
        0x4530 => "DualPlatform",
        0x4531 => "MultiPlatform",
        0x8010 => "Gkey",
        0x8020 => "Mkeys",
        0x8030 => "MR",
        0x8040 => "BrightnessControl",
        0x8060 => "ReportRate",
        0x8061 => "ExtendedReportRate",
        0x8070 => "ColorLedEffects",
        0x8071 => "RGBEffects",
        0x8080 => "PerKeyLighting",
        0x8081 => "PerKeyLightingV2",
        0x8090 => "ModeStatus",
        0x80A4 => "AxisResponseCurve",
        0x8100 => "OnboardProfiles",
        0x8110 => "MouseButtonSpy",
        0x8111 => "LatencyMonitoring",
        0x92E2 => "GamingAttachments",
        _ => "?",
    }
}

fn device_type_name(t: u8) -> &'static str {
    match t {
        0 => "keyboard",
        1 => "remote control",
        2 => "numpad",
        3 => "mouse",
        4 => "trackpad",
        5 => "trackball",
        6 => "presenter",
        7 => "receiver",
        8 => "headset",
        9 => "webcam",
        10 => "steering wheel",
        11 => "joystick",
        12 => "gamepad",
        13 => "dock",
        14 => "speaker",
        15 => "microphone",
        _ => "?",
    }
}

/// Stable short fingerprint so DISCOVERY.md can compare identities without publishing them.
fn fp(bytes: &[u8], redact: bool) -> String {
    if !redact {
        return hex(bytes);
    }
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("<fp:{:06x}>", h & 0xFFFFFF)
}

fn cmd_hidpp(api: &HidApi, groups: &Groups, opts: &Opts) {
    println!("== HID++ probe ==");
    for (path, infos) in groups {
        let d = infos[0];
        if d.vendor_id() != VID_LOGITECH {
            continue;
        }
        let has_hidpp = infos
            .iter()
            .any(|i| i.usage_page() == 0xFF00 || i.usage_page() == 0xFF43);
        if !has_hidpp {
            continue;
        }
        println!(
            "-- {:04x}:{:04x} {:?} bus={} collections=[{}]",
            d.vendor_id(),
            d.product_id(),
            d.product_string().unwrap_or(""),
            bus(d),
            infos
                .iter()
                .map(|i| format!("{:04x}/{:04x}", i.usage_page(), i.usage()))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !vendor_only(infos) {
            if !opts.allow_mixed {
                println!("   REFUSED: interface also carries non-vendor collections (pass --allow-mixed to probe; non-HID++ reports are dropped unseen)");
                continue;
            }
            println!("   WARNING: mixed interface opened because of --allow-mixed");
        }
        let dev = match api.open_path(path) {
            Ok(dv) => dv,
            Err(e) => {
                println!("   open failed: {e}");
                continue;
            }
        };
        // Long-only if no short-report collection (0xFF00/0x0001 or 0xFF43/0x0201) is present.
        let long_only = !infos.iter().any(|i| {
            (i.usage_page() == 0xFF00 && i.usage() == 0x0001)
                || (i.usage_page() == 0xFF43 && i.usage() & 0x00FF == 0x01)
        });
        let mut h = Hidpp {
            dev: &dev,
            long_only,
            other_hidpp: vec![],
            dropped_non_hidpp: 0,
        };
        println!("   report mode: {}", if long_only { "long (0x11) only" } else { "short (0x10) + long (0x11)" });

        let is_receiver = matches!(d.bus_type(), BusType::Usb)
            && d.product_string().unwrap_or("").to_lowercase().contains("receiver");
        if is_receiver {
            probe_receiver(&mut h);
            for idx in 1..=6u8 {
                probe_device(&mut h, idx, opts);
            }
        } else {
            probe_device(&mut h, 0xFF, opts);
        }
        if !h.other_hidpp.is_empty() {
            println!("   unsolicited/other HID++ reports seen ({}):", h.other_hidpp.len());
            for r in h.other_hidpp.iter().take(20) {
                println!("     {}", hex(r));
            }
        }
        println!("   non-HID++ reports dropped unseen: {}", h.dropped_non_hidpp);
    }
    println!();
}

fn probe_receiver(h: &mut Hidpp) {
    println!("   [receiver idx 0xFF]");
    match h.call(0xFF, 0x00, 1, &[0, 0, 0xAA]) {
        Ok(r) => println!("     ping: HID++ {}.{} (receiver speaks 2.0)", r[0], r[1]),
        Err(HErr::Hidpp10(e)) => println!("     ping: HID++ 1.0 receiver (err 0x{e:02x})"),
        Err(e) => println!("     ping: {e:?}"),
    }
    // HID++ 1.0 register reads only.
    for (name, sub, reg, p) in [
        ("notification flags (short reg 0x00)", 0x81u8, 0x00u8, [0u8, 0, 0]),
        ("connection state  (short reg 0x02)", 0x81, 0x02, [0, 0, 0]),
        ("fw major/minor    (short reg 0xF1/01)", 0x81, 0xF1, [0x01, 0, 0]),
        ("fw build          (short reg 0xF1/02)", 0x81, 0xF1, [0x02, 0, 0]),
        ("receiver info     (long  reg 0xB5/03)", 0x83, 0xB5, [0x03, 0, 0]),
    ] {
        match h.raw(0xFF, sub, reg, &p) {
            Ok(r) => println!("     {name}: {}", hex(&r)),
            Err(e) => println!("     {name}: {e:?}"),
        }
    }
    for slot in 0..6u8 {
        for (label, base) in [("pairing", 0x20u8), ("ext-pairing", 0x30), ("codename", 0x40)] {
            match h.raw(0xFF, 0x83, 0xB5, &[base + slot, 0, 0]) {
                Ok(r) => {
                    let shown = if label == "ext-pairing" { fp(&r, true) } else { hex(&r) };
                    let txt = if label == "codename" && r.len() > 2 {
                        let l = (r[1] as usize).min(r.len() - 2);
                        format!("  {:?}", String::from_utf8_lossy(&r[2..2 + l]))
                    } else {
                        String::new()
                    };
                    println!("     slot {} {label} (0xB5/{:02x}): {shown}{txt}", slot + 1, base + slot);
                }
                Err(HErr::Hidpp10(0x03)) | Err(HErr::Hidpp10(0x02)) => {}
                Err(e) => println!("     slot {} {label}: {e:?}", slot + 1),
            }
        }
    }
}

fn probe_device(h: &mut Hidpp, idx: u8, opts: &Opts) {
    let ping = h.call(idx, 0x00, 1, &[0, 0, 0xAA]);
    let (maj, min) = match ping {
        Ok(r) => (r[0], r[1]),
        Err(HErr::Hidpp10(0x08)) => { println!("   [device idx 0x{idx:02x}] no device (HID++1.0 err 0x08 UnknownDevice)"); return; }
        Err(HErr::Hidpp10(0x09)) => {
            println!("   [device idx 0x{idx:02x}] paired but unreachable/asleep (HID++1.0 err 0x09 ResourceError)");
            return;
        }
        Err(HErr::Hidpp10(0x01)) => {
            println!("   [device idx 0x{idx:02x}] HID++ 1.0 device");
            return;
        }
        Err(HErr::Timeout) if idx != 0xFF => { println!("   [device idx 0x{idx:02x}] ping timeout"); return; }
        Err(e) => {
            println!("   [device idx 0x{idx:02x}] ping: {e:?}");
            return;
        }
    };
    println!("   [device idx 0x{idx:02x}] HID++ {maj}.{min}");

    // Feature table
    let mut have: BTreeMap<u16, (u8, u8)> = BTreeMap::new();
    match h.feature(idx, 0x0001) {
        Ok(Some((fs, _, _))) => match h.call(idx, fs, 0, &[]) {
            Ok(r) => {
                let count = r[0];
                println!("     features ({} + root):", count);
                for i in 0..=count {
                    match h.call(idx, fs, 1, &[i]) {
                        Ok(r) => {
                            let id = u16::from_be_bytes([r[0], r[1]]);
                            let flags = r[2];
                            let ver = r[3];
                            have.insert(id, (i, ver));
                            let mut f = vec![];
                            if flags & 0x80 != 0 { f.push("obsolete"); }
                            if flags & 0x40 != 0 { f.push("hidden"); }
                            if flags & 0x20 != 0 { f.push("engineering"); }
                            println!(
                                "       0x{i:02x}: 0x{id:04x} v{ver} {}{}",
                                feature_name(id),
                                if f.is_empty() { String::new() } else { format!(" [{}]", f.join(",")) }
                            );
                        }
                        Err(e) => println!("       0x{i:02x}: {e:?}"),
                    }
                }
            }
            Err(e) => println!("     FeatureSet.getCount: {e:?}"),
        },
        other => println!("     IFeatureSet lookup: {other:?}"),
    }

    // Also confirm IRoot lookups for the battery features (the product path).
    for id in [0x1004u16, 0x1001, 0x1000] {
        match h.feature(idx, id) {
            Ok(Some((fi, _, ver))) => println!("     IRoot.getFeature(0x{id:04x}) -> index 0x{fi:02x} v{ver}"),
            Ok(None) => println!("     IRoot.getFeature(0x{id:04x}) -> not present"),
            Err(e) => println!("     IRoot.getFeature(0x{id:04x}) -> {e:?}"),
        }
    }

    if let Some(&(fi, ver)) = have.get(&0x0003) {
        match h.call(idx, fi, 0, &[]) {
            Ok(r) => {
                let transport = u16::from_be_bytes([r[5], r[6]]);
                let mut t = vec![];
                if transport & 0x01 != 0 { t.push("BT"); }
                if transport & 0x02 != 0 { t.push("BLE"); }
                if transport & 0x04 != 0 { t.push("eQuad/Lightspeed"); }
                if transport & 0x08 != 0 { t.push("USB"); }
                println!(
                    "     DeviceInformation v{ver}: entities={} unitId={} transports=0x{transport:04x} [{}] modelIds={} {} {} extModel=0x{:02x} caps=0x{:02x}",
                    r[0],
                    fp(&r[1..5], opts.redact),
                    t.join("|"),
                    hex(&r[7..9]),
                    hex(&r[9..11]),
                    hex(&r[11..13]),
                    r[13],
                    r.get(14).copied().unwrap_or(0)
                );
                for ent in 0..r[0] {
                    if let Ok(f) = h.call(idx, fi, 1, &[ent]) {
                        println!(
                            "       fw entity {ent}: type={} {}{:02x}.{:02x} build {:02x}{:02x} trPid={}",
                            f[0],
                            f[1..4].iter().map(|&c| if c.is_ascii_graphic() { c as char } else { '-' }).collect::<String>(),
                            f[4],
                            f[5],
                            f[6],
                            f[7],
                            hex(&f[9..11])
                        );
                    }
                }
                if r.get(14).copied().unwrap_or(0) & 0x01 != 0 {
                    match h.call(idx, fi, 2, &[]) {
                        Ok(s) => println!("       serial: {}", fp(&s[..12.min(s.len())], opts.redact)),
                        Err(e) => println!("       serial: {e:?}"),
                    }
                }
            }
            Err(e) => println!("     DeviceInformation: {e:?}"),
        }
    }

    if let Some(&(fi, _)) = have.get(&0x0005) {
        let mut name = String::new();
        if let Ok(r) = h.call(idx, fi, 0, &[]) {
            let len = r[0] as usize;
            while name.len() < len {
                match h.call(idx, fi, 1, &[name.len() as u8]) {
                    Ok(c) => {
                        let chunk: Vec<u8> = c.into_iter().take_while(|b| *b != 0).collect();
                        if chunk.is_empty() { break; }
                        name.push_str(&String::from_utf8_lossy(&chunk));
                    }
                    Err(_) => break,
                }
            }
            name.truncate(len);
        }
        let ty = h.call(idx, fi, 2, &[]).map(|r| r[0]).unwrap_or(0xFF);
        println!("     DeviceName: {name:?} type={} ({})", ty, device_type_name(ty));
    }

    if let Some(&(fi, ver)) = have.get(&0x1004) {
        let caps = h.call(idx, fi, 0, &[]);
        let st = h.call(idx, fi, 1, &[]);
        match (caps, st) {
            (Ok(c), Ok(s)) => println!(
                "     UnifiedBattery v{ver}: caps levels=0b{:04b} flags=0x{:02x} (soc_supported={}) | status soc={}% level=0b{:04b} charging={} ext_power={} raw={}",
                c[0], c[1], c[1] & 0x02 != 0, s[0], s[1], s[2], s[3], hex(&s[..6])
            ),
            (c, s) => println!("     UnifiedBattery: caps={c:?} status={s:?}"),
        }
    }
    if let Some(&(fi, ver)) = have.get(&0x1001) {
        match h.call(idx, fi, 0, &[]) {
            Ok(r) => println!(
                "     BatteryVoltage v{ver}: {} mV flags=0x{:02x} raw={}",
                u16::from_be_bytes([r[0], r[1]]),
                r[2],
                hex(&r[..6])
            ),
            Err(e) => println!("     BatteryVoltage: {e:?}"),
        }
    }
    if let Some(&(fi, ver)) = have.get(&0x1000) {
        match h.call(idx, fi, 0, &[]) {
            Ok(r) => println!("     BatteryLevelStatus v{ver}: level={}% next={}% status={} raw={}", r[0], r[1], r[2], hex(&r[..6])),
            Err(e) => println!("     BatteryLevelStatus: {e:?}"),
        }
    }
    if let Some(&(fi, ver)) = have.get(&0x1F20) {
        match h.call(idx, fi, 0, &[]) {
            Ok(r) => println!("     ADCMeasurement v{ver}: raw={}", hex(&r[..6])),
            Err(e) => println!("     ADCMeasurement: {e:?}"),
        }
    }
}

// --------------------------------------------------------------- SteelSeries

fn cmd_steelseries(api: &HidApi, groups: &Groups) {
    println!("== SteelSeries probe ==");
    for (path, infos) in groups {
        let d = infos[0];
        if d.vendor_id() != VID_STEELSERIES {
            continue;
        }
        if !vendor_only(infos) {
            continue;
        }
        println!(
            "-- {:04x}:{:04x} {:?} iface={} collections=[{}]",
            d.vendor_id(),
            d.product_id(),
            d.product_string().unwrap_or(""),
            d.interface_number(),
            infos
                .iter()
                .map(|i| format!("{:04x}/{:04x}", i.usage_page(), i.usage()))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let dev = match api.open_path(path) {
            Ok(dv) => dv,
            Err(e) => {
                println!("   open failed: {e}");
                continue;
            }
        };
        // Arctis Nova Pro Wireless base station: status request 0x06 0xb0 (HeadsetControl).
        for (label, len) in [("2-byte write", 2usize), ("64-byte padded write", 64)] {
            let mut req = vec![0u8; len];
            req[0] = 0x06;
            req[1] = 0xb0;
            match dev.write(&req) {
                Ok(n) => println!("   {label}: wrote {n} bytes"),
                Err(e) => {
                    println!("   {label}: write failed: {e}");
                    continue;
                }
            }
            let deadline = Instant::now() + TIMEOUT;
            let mut buf = [0u8; 128];
            let mut got = 0;
            while Instant::now() < deadline && got < 6 {
                let left = deadline.saturating_duration_since(Instant::now());
                match dev.read_timeout(&mut buf, left.as_millis() as i32) {
                    Ok(0) => break,
                    Ok(n) => {
                        got += 1;
                        let r = &buf[..n];
                        let tag = if n > 15 && r[0] == 0x06 && r[1] == 0xb0 {
                            format!(
                                "  <- status reply: byte6(battery 0..8)={} -> {}% byte15(status)=0x{:02x} ({})",
                                r[6],
                                (r[6] as u32 * 100 / 8),
                                r[15],
                                match r[15] {
                                    0x01 => "headset offline",
                                    0x02 => "cable charging",
                                    0x08 => "online",
                                    _ => "?",
                                }
                            )
                        } else {
                            String::new()
                        };
                        println!("   read {n:3}: {}{tag}", hex(&r[..n.min(32)]));
                    }
                    Err(e) => {
                        println!("   read error: {e}");
                        break;
                    }
                }
            }
            if got > 0 {
                break;
            }
            println!("   no reply");
        }
    }
    println!();
}
