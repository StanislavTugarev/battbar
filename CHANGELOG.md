# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-19

### Added

- Menu bar app with one icon per device: type icon, percentage (`~` for coarse or
  estimated values), charging mark, dimmed icon when asleep or disconnected.
- Logitech HID++ 2.0 over Lightspeed / Unifying receivers, USB cable and Bluetooth LE
  (vendor GATT service); Bolt receivers untested.
- SteelSeries Arctis Nova Pro Wireless through its base station, including the spare
  battery in the base; other SteelSeries headsets through the optional
  `headsetcontrol` fallback.
- Battery that macOS knows for Bluetooth devices: BLE Battery Service and classic
  Bluetooth audio, including left/right/case components.
- One device across connections: receiver ↔ Bluetooth keyboards by HID++ unit ID,
  headsets on a base station and Bluetooth (remembered link, can be unlinked).
- Values while charging and for ~10 minutes after are shown as estimates (`~`), since
  the charge voltage inflates firmware readings.
- Low-battery notifications (default 15 %, once per discharge cycle, none while
  charging or settling, 10 % hysteresis), optional
  spare-battery notification, launch at login, hide/merge/split devices.
- `battbar` command line: `probe`, `dump`, `record`, `probe --replay`.
- Device database in `devices/*.toml` with user overrides in
  `~/.config/battbar/devices/`.

[Unreleased]: https://github.com/OWNER/battbar/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OWNER/battbar/releases/tag/v0.1.0
