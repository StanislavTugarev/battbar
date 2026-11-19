# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-19

### Added

- Launch at Login: asked once on first start ("Not Now" is final; the switch stays in
  Settings). The switch shows the state macOS has registered and never turns itself
  back on; approval and "not found" cases explain what to do and open System Settings →
  Login Items. The question is postponed while macOS runs the app from a translocated
  copy.
- Experimental support for voltage-only AA/AAA devices (HID++ 0x1001): estimates from
  alkaline or NiMH per-cell curves (published datasheets, always shown with `~`),
  chosen per device under **Batteries** (default alkaline). Device database fields
  `chemistry` and `cells`.
- Low-battery alerts say "replace the batteries" for replaceable cells, "charge it" for
  rechargeable devices, and stay neutral when that is unknown.

- App icon (and a matching menu bar glyph when no device is shown), built from SVG by
  `scripts/build-icon.sh`.

### Fixed

- Crash when switching "Use headsetcontrol for Other Headsets" (present in 0.1.0).

### Changed

- Development builds keep their settings in `~/.config/battbar-dev/`.
- hidapi runs on one long-lived service thread that services its run loop; settings
  changes no longer rebuild provider threads.
- `battbar dump` and `battbar record` never write to devices (HID++ 1.0 SET_REGISTER is
  refused) and include the HID++ 1.0 battery registers 0x07 / 0x0D (read only).
- Cask: quits battbar and removes its login item on uninstall; `zap` also removes the
  settings and the menu bar item positions.
- "Show Bluetooth Devices Without Battery" is now "Show Devices Without Battery Value".

### Changed

- Installation docs and cask caveats explain Homebrew Tap Trust: install by the fully
  qualified name, which trusts only the battbar cask.
- Cask uses `depends_on macos: :ventura` (the string comparison form is deprecated).

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
- Privacy: identifiers in the settings are keyed hashes (HMAC-SHA256 with a random
  per-installation key); `battbar dump` never includes the settings; `battbar record`
  replaces every HID++ unit ID found in the recorded frames with a pseudonym.

### Known limitations

- **Non-rechargeable batteries (AA/AAA)** are not supported yet. A device that reports
  only a voltage far below any Li-ion level (below about 3.3 V over HID++ 0x1001) is
  shown as "Non-rechargeable battery — not supported yet", without a percentage and
  without low-battery alerts — never as 0 %. Devices whose firmware reports a
  percentage (e.g. Pebble Keys 2 K380s over HID++ 0x1004) already show it. Full support
  (battery chemistry, alkaline/NiMH curves) is planned for 0.2.0.
- **HID++ 1.0 devices** behind a receiver are listed without a battery value.
- **Bolt receivers** are untested.

[Unreleased]: https://github.com/StanislavTugarev/battbar/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/StanislavTugarev/battbar/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/StanislavTugarev/battbar/releases/tag/v0.1.0
