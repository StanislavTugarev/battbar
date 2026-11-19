// SPDX-License-Identifier: GPL-3.0-or-later
//! What the menu bar shows, as plain data (no AppKit), so it can be unit-tested.

use battbar_core::*;

#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    /// SF Symbol name.
    pub symbol: &'static str,
    /// Text next to the icon, e.g. `~85%`, `12%⚡︎`, or empty when no value.
    pub title: String,
    /// Asleep / disconnected / no value: dimmed icon without percentage.
    pub dimmed: bool,
    pub tooltip: String,
}

pub fn symbol(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Mouse => "computermouse.fill",
        DeviceKind::Keyboard => "keyboard.fill",
        DeviceKind::Headset | DeviceKind::Headphones => "headphones",
        DeviceKind::Gamepad => "gamecontroller.fill",
        DeviceKind::Other => "battery.75percent",
    }
}

pub fn percent_text(r: &Reading) -> Option<String> {
    let p = r.percent.filter(|_| r.status == Status::Ok)?;
    let tilde = if r.precision == Precision::Exact { "" } else { "~" };
    Some(format!("{tilde}{p}%"))
}

pub fn status_text(r: &Reading) -> String {
    match r.status {
        Status::Ok => match percent_text(r) {
            Some(p) => {
                let precision = match r.precision {
                    Precision::Exact => "",
                    Precision::Coarse => " (coarse)",
                    Precision::Approximate => " (estimated)",
                };
                format!("{p}{precision}")
            }
            None => "Battery not reported".into(),
        },
        Status::NotReported => match r.note {
            Some(ReadingNote::NonRechargeableBattery) => "Non-rechargeable battery — not supported yet".into(),
            Some(ReadingNote::Hidpp10Device) => "Battery not reported (older protocol, not supported yet)".into(),
            Some(ReadingNote::CalibratedVoltage) | None => "Battery not reported".into(),
        },
        Status::Asleep => "Asleep".into(),
        Status::Disconnected => "Disconnected".into(),
        Status::Error => "Error reading battery".into(),
    }
}

pub fn item_view(d: &Device) -> ItemView {
    let r = &d.reading;
    let pct = percent_text(r);
    let charging = r.charging == Some(true);
    let title = match &pct {
        Some(p) if charging => format!("{p}\u{26A1}\u{FE0E}"),
        Some(p) => p.clone(),
        None => String::new(),
    };
    ItemView {
        symbol: symbol(d.kind),
        title,
        dimmed: pct.is_none(),
        tooltip: format!("{} — {}", d.name, status_text(r)),
    }
}

pub fn component_name(k: ComponentKind) -> &'static str {
    match k {
        ComponentKind::Left => "Left",
        ComponentKind::Right => "Right",
        ComponentKind::Case => "Case",
        ComponentKind::Spare => "Spare battery",
    }
}

/// Informational lines of a device menu (disabled items).
pub fn detail_lines(d: &Device) -> Vec<String> {
    let mut v = vec![format!("Battery: {}", status_text(&d.reading))];
    if let Some(b) = d.reading.battery.filter(|b| b.rechargeable == Some(false)) {
        let chem = match b.chemistry {
            Some(Chemistry::Nimh) => "NiMH",
            _ => "Alkaline",
        };
        let cells = b.cells.map(|c| format!(" × {c}")).unwrap_or_default();
        let mv = d
            .reading
            .voltage_mv
            .map(|m| format!(", {:.2} V", m as f32 / 1000.0))
            .unwrap_or_default();
        v.push(format!("Batteries: {chem}{cells}{mv} (estimate, experimental)"));
    }
    if d.reading.note == Some(ReadingNote::CalibratedVoltage) {
        v.push("From voltage, calibrated against this device's Bluetooth readings".into());
    }
    match d.estimate {
        Some(Estimate::Charging) => v.push("Estimated while charging".into()),
        Some(Estimate::Settling) => v.push("Estimated: settling after charging".into()),
        None => {}
    }
    if d.reading.charging == Some(true) {
        v.push("Charging".into());
    } else if d.reading.external_power == Some(true) {
        v.push("On external power".into());
    }
    for c in &d.components {
        v.push(format!("{}: {}", component_name(c.kind), status_text(&c.reading)));
    }
    let live = d.live_transports();
    if !live.is_empty() {
        v.push(format!(
            "Connected via: {}",
            live.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ")
        ));
    }
    v
}

/// One line per source, e.g. `Bluetooth · macOS (Bluetooth): 83%`.
pub fn source_lines(d: &Device) -> Vec<String> {
    d.sources
        .iter()
        .map(|s| {
            let mark = if d.percent_source.as_deref() == Some(&s.full_key()) {
                " ✓"
            } else {
                ""
            };
            let mv = s.reading.voltage_mv.map(|m| format!(", {m} mV")).unwrap_or_default();
            format!("{} · {}: {}{mv}{mark}", s.transport, s.source, status_text(&s.reading))
        })
        .collect()
}

/// A device seen only by the system Bluetooth provider.
pub fn bluetooth_only(d: &Device) -> bool {
    !d.sources.is_empty() && d.sources.iter().all(|s| s.provider == "bt-system")
}

/// Hidden unless the user asks for it: Bluetooth devices without a battery value
/// (option), and disconnected Bluetooth devices that never reported a battery.
pub fn hidden_by_default(d: &Device, settings: &Settings) -> bool {
    if settings.shown.contains(&d.id) {
        return false;
    }
    (d.reading.status == Status::NotReported && !settings.show_unreported)
        || (d.reading.status == Status::Disconnected && bluetooth_only(d) && !settings.ever_reported.contains(&d.id))
}

pub fn is_visible(d: &Device, settings: &Settings) -> bool {
    !settings.hidden.contains(&d.id) && !hidden_by_default(d, settings)
}

/// Devices that get their own status item.
pub fn visible<'a>(devices: &'a [Device], settings: &Settings) -> Vec<&'a Device> {
    devices.iter().filter(|d| is_visible(d, settings)).collect()
}

/// Devices listed under "Other Devices".
pub fn others<'a>(devices: &'a [Device], settings: &Settings) -> Vec<&'a Device> {
    devices.iter().filter(|d| !is_visible(d, settings)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(kind: DeviceKind, reading: Reading) -> Device {
        Device {
            id: "x".into(),
            name: "Dev".into(),
            kind,
            reading,
            percent_source: None,
            components: vec![],
            sources: vec![],
            estimate: None,
        }
    }

    #[test]
    fn exact_coarse_approximate_and_charging() {
        let v = item_view(&dev(DeviceKind::Mouse, Reading::ok(11, Precision::Exact)));
        assert_eq!((v.symbol, v.title.as_str(), v.dimmed), ("computermouse.fill", "11%", false));
        assert_eq!(
            item_view(&dev(DeviceKind::Headset, Reading::ok(100, Precision::Coarse))).title,
            "~100%"
        );
        assert_eq!(
            item_view(&dev(DeviceKind::Keyboard, Reading::ok(97, Precision::Approximate))).title,
            "~97%"
        );
        let mut r = Reading::ok(40, Precision::Exact);
        r.charging = Some(true);
        assert!(item_view(&dev(DeviceKind::Mouse, r)).title.starts_with("40%"));
    }

    #[test]
    fn asleep_and_disconnected_are_dimmed_without_percent() {
        for s in [Status::Asleep, Status::Disconnected, Status::Error] {
            let v = item_view(&dev(DeviceKind::Mouse, Reading::with_status(s)));
            assert!(v.dimmed);
            assert_eq!(v.title, "");
        }
    }

    #[test]
    fn calibrated_voltage_is_explained_in_the_details() {
        let mut r = Reading::ok(97, Precision::Approximate);
        r.voltage_mv = Some(4080);
        r.note = Some(ReadingNote::CalibratedVoltage);
        let d = dev(DeviceKind::Keyboard, r);
        assert_eq!(item_view(&d).title, "~97%");
        assert!(detail_lines(&d).iter().any(|l| l.contains("calibrated")));
    }

    #[test]
    fn gamepad_has_its_own_icon() {
        assert_eq!(symbol(DeviceKind::Gamepad), "gamecontroller.fill");
    }

    fn bt(status: Status) -> Device {
        let mut d = dev(DeviceKind::Gamepad, Reading::with_status(status));
        d.sources = vec![DeviceReport {
            provider: "bt-system".into(),
            source_key: "classic:x".into(),
            source: "".into(),
            name: "Pad".into(),
            kind: DeviceKind::Gamepad,
            transport: Transport::BluetoothClassic,
            identity: vec![],
            hints: vec![],
            reading: d.reading.clone(),
            components: vec![],
        }];
        d
    }

    #[test]
    fn disconnected_bluetooth_never_reported_is_hidden_until_shown() {
        let devs = vec![bt(Status::Disconnected)];
        let mut s = Settings::default();
        assert!(visible(&devs, &s).is_empty());
        assert_eq!(others(&devs, &s).len(), 1);
        s.ever_reported.push("x".into());
        assert_eq!(visible(&devs, &s).len(), 1, "reported once: shown dimmed when away");
        s.ever_reported.clear();
        s.shown.push("x".into());
        assert_eq!(visible(&devs, &s).len(), 1, "explicit Show in Menu Bar");
    }

    #[test]
    fn disconnected_receiver_devices_stay_visible() {
        let mut d = bt(Status::Disconnected);
        d.sources[0].provider = "hidpp".into();
        assert_eq!(visible(&[d], &Settings::default()).len(), 1);
    }

    #[test]
    fn unreported_devices_hidden_by_default() {
        let devs = vec![dev(DeviceKind::Headphones, Reading::with_status(Status::NotReported))];
        let mut s = Settings::default();
        assert!(visible(&devs, &s).is_empty());
        s.show_unreported = true;
        assert_eq!(visible(&devs, &s).len(), 1);
        s.hidden.push("x".into());
        assert!(visible(&devs, &s).is_empty());
    }
}
