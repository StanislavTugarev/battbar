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

/// Devices shown in the menu bar and in the device sections of the menu, in display
/// order.
pub fn visible<'a>(devices: &'a [Device], settings: &Settings) -> Vec<&'a Device> {
    ordered(devices.iter().filter(|d| is_visible(d, settings)).collect(), &settings.device_order)
}

/// Default order: keyboard, mouse, headset/headphones, everything else.
fn kind_rank(k: DeviceKind) -> u8 {
    match k {
        DeviceKind::Keyboard => 0,
        DeviceKind::Mouse => 1,
        DeviceKind::Headset | DeviceKind::Headphones => 2,
        DeviceKind::Gamepad | DeviceKind::Other => 3,
    }
}

/// Stable display order: by type, then name; with a custom order, the listed devices
/// first (in that order) and the others after them in the default order.
pub fn ordered<'a>(mut devices: Vec<&'a Device>, custom: &[String]) -> Vec<&'a Device> {
    devices.sort_by_cached_key(|d| (kind_rank(d.kind), d.name.to_lowercase(), d.id.clone()));
    if custom.is_empty() {
        return devices;
    }
    let pos = |d: &Device| custom.iter().position(|x| *x == d.id);
    let mut out: Vec<&Device> = devices.iter().copied().filter(|d| pos(d).is_some()).collect();
    out.sort_by_key(|d| pos(d));
    out.extend(devices.into_iter().filter(|d| pos(d).is_none()));
    out
}

/// The custom order after moving `id` one place (`delta` −1 = left, +1 = right) within
/// `current` (the order shown now). IDs of devices that are not present keep their
/// relative order after the present ones.
pub fn move_device(current: &[String], custom: &[String], id: &str, delta: isize) -> Vec<String> {
    let mut v = current.to_vec();
    if let Some(i) = v.iter().position(|x| x == id) {
        let j = i as isize + delta;
        if (0..v.len() as isize).contains(&j) {
            v.swap(i, j as usize);
        }
    }
    v.extend(custom.iter().filter(|x| !current.contains(x)).cloned());
    v
}

/// What the single status item shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Icon and percentage for every visible device.
    Expanded,
    /// Icons only; a percentage only on devices at or below the low-battery threshold.
    /// The automatic step between Expanded and Collapsed (not chosen by the user).
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    Compact,
    /// One icon; see [`CollapsedStyle`].
    Collapsed,
}

/// Collapsed status item while no device is at or below the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapsedStyle {
    /// The neutral battbar glyph.
    Glyph,
    /// The icon and percentage of the lowest device.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    Lowest,
}

/// Collapsed: battbar glyph until a device needs attention (see the 0.4.0 sketches).
pub const COLLAPSED_STYLE: CollapsedStyle = CollapsedStyle::Glyph;

#[derive(Debug, Clone, PartialEq)]
pub struct BarView {
    /// Drawn side by side; empty = the neutral battbar glyph.
    pub segments: Vec<ItemView>,
    pub tooltip: String,
}

/// At or below the low-battery threshold (the rule of the low-battery alert).
pub fn is_low(d: &Device, threshold: Option<u8>) -> bool {
    matches!((threshold, d.reading.percent, d.reading.status), (Some(t), Some(p), Status::Ok) if p <= t)
}

pub fn bar_view(devices: &[&Device], layout: Layout, threshold: Option<u8>, style: CollapsedStyle) -> BarView {
    let tooltip = if devices.is_empty() {
        "battbar".to_string()
    } else {
        devices.iter().map(|d| item_view(d).tooltip).collect::<Vec<_>>().join("\n")
    };
    let any_low = devices.iter().any(|d| is_low(d, threshold));
    let segments = match layout {
        Layout::Expanded => devices.iter().map(|d| item_view(d)).collect(),
        Layout::Compact => devices
            .iter()
            .map(|d| {
                let mut v = item_view(d);
                if !is_low(d, threshold) {
                    v.title.clear();
                }
                v
            })
            .collect(),
        Layout::Collapsed => {
            // First of the lowest in display order.
            let lowest = devices
                .iter()
                .filter(|d| percent_text(&d.reading).is_some())
                .min_by_key(|d| d.reading.percent);
            match lowest {
                Some(d) if any_low || style == CollapsedStyle::Lowest => vec![item_view(d)],
                _ => vec![],
            }
        }
    };
    BarView { segments, tooltip }
}

/// Devices listed under "Other Devices".
pub fn others<'a>(devices: &'a [Device], settings: &Settings) -> Vec<&'a Device> {
    devices.iter().filter(|d| !is_visible(d, settings)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, kind: DeviceKind, percent: Option<u8>) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            kind,
            reading: match percent {
                Some(p) => Reading::ok(p, Precision::Exact),
                None => Reading::with_status(Status::Asleep),
            },
            percent_source: None,
            components: vec![],
            sources: vec![],
            estimate: None,
        }
    }

    fn ids(v: &[&Device]) -> Vec<String> {
        v.iter().map(|d| d.id.clone()).collect()
    }

    #[test]
    fn order_by_type_then_custom_then_moves() {
        let all = [
            device("pad", "DualSense", DeviceKind::Gamepad, Some(40)),
            device("hs", "Nova", DeviceKind::Headset, Some(60)),
            device("m", "G502", DeviceKind::Mouse, Some(50)),
            device("k2", "Pebble", DeviceKind::Keyboard, Some(100)),
            device("k1", "G915", DeviceKind::Keyboard, Some(88)),
        ];
        let refs: Vec<&Device> = all.iter().collect();
        assert_eq!(ids(&ordered(refs.clone(), &[])), ["k1", "k2", "m", "hs", "pad"]);
        // Custom: listed first, the rest by type; IDs of absent devices are ignored.
        let custom: Vec<String> = ["hs", "gone", "m"].iter().map(|s| s.to_string()).collect();
        assert_eq!(ids(&ordered(refs.clone(), &custom)), ["hs", "m", "k1", "k2", "pad"]);
        // Move Left / Right from the shown order; edges stay; absent IDs are kept.
        let shown = ids(&ordered(refs.clone(), &custom));
        let c = move_device(&shown, &custom, "k1", -1);
        assert_eq!(c, ["hs", "k1", "m", "k2", "pad", "gone"]);
        assert_eq!(ids(&ordered(refs.clone(), &c)), ["hs", "k1", "m", "k2", "pad"]);
        assert_eq!(move_device(&shown, &custom, "hs", -1)[..5], shown[..]);
        assert_eq!(move_device(&shown, &custom, "pad", 1)[..5], shown[..]);
        // The order does not depend on the order the devices arrive in.
        let rev: Vec<&Device> = all.iter().rev().collect();
        assert_eq!(ids(&ordered(rev, &c)), ["hs", "k1", "m", "k2", "pad"]);
    }

    #[test]
    fn bar_layouts() {
        let k = device("k", "G915", DeviceKind::Keyboard, Some(88));
        let mut m = device("m", "G502", DeviceKind::Mouse, Some(12));
        let h = device("h", "Nova", DeviceKind::Headset, None);
        let all = [&k, &m, &h];
        let titles = |b: &BarView| b.segments.iter().map(|s| s.title.clone()).collect::<Vec<_>>();

        let e = bar_view(&all, Layout::Expanded, Some(15), COLLAPSED_STYLE);
        assert_eq!(titles(&e), ["88%", "12%", ""]);
        assert_eq!(e.segments.iter().map(|s| s.dimmed).collect::<Vec<_>>(), [false, false, true]);
        assert_eq!(e.tooltip.lines().count(), 3);

        let c = bar_view(&all, Layout::Compact, Some(15), COLLAPSED_STYLE);
        assert_eq!(titles(&c), ["", "12%", ""], "percentage only below the threshold");
        assert_eq!(c.segments.len(), 3);

        // Collapsed: the low device instead of the glyph.
        let low = bar_view(&all, Layout::Collapsed, Some(15), CollapsedStyle::Glyph);
        assert_eq!(titles(&low), ["12%"]);
        assert_eq!(low.segments[0].symbol, "computermouse.fill");

        // Nobody low: the glyph, or the lowest device.
        m.reading = Reading::ok(40, Precision::Exact);
        let all = [&k, &m, &h];
        let g = bar_view(&all, Layout::Collapsed, Some(15), CollapsedStyle::Glyph);
        assert!(g.segments.is_empty());
        let l = bar_view(&all, Layout::Collapsed, Some(15), CollapsedStyle::Lowest);
        assert_eq!(titles(&l), ["40%"]);
        // Threshold off: the glyph stays, however low.
        m.reading = Reading::ok(5, Precision::Exact);
        assert!(bar_view(&[&k, &m], Layout::Collapsed, None, CollapsedStyle::Glyph)
            .segments
            .is_empty());
        // At the threshold counts as low (as the alert does).
        m.reading = Reading::ok(15, Precision::Exact);
        let at = bar_view(&[&k, &m], Layout::Collapsed, Some(15), CollapsedStyle::Glyph);
        assert_eq!(titles(&at), ["15%"]);
        // Nothing with a value, or no device at all: the glyph.
        assert!(bar_view(&[&h], Layout::Collapsed, Some(15), CollapsedStyle::Lowest)
            .segments
            .is_empty());
        let none = bar_view(&[], Layout::Expanded, Some(15), COLLAPSED_STYLE);
        assert!(none.segments.is_empty());
        assert_eq!(none.tooltip, "battbar");
    }

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
