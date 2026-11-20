# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-11-21

### Changed

- One menu bar item instead of one per device. **Expanded** shows every device side by
  side in it (icon and percentage, with the same `~`, ⚡︎ and dimming as before);
  **Collapsed** shows one icon — battbar's glyph while all devices are fine, the lowest
  device's icon and percentage once one is at or below the low-battery threshold. The
  item stays black or white like the rest of the menu bar. Switch with
  *Expand / Collapse in Menu Bar* in the menu or under Settings → Menu Bar.
- New installations start collapsed; installations updated from 0.3 start expanded,
  so the menu bar looks as before. The saved positions of the old per-device icons are
  removed; ⌘-drag places the new item and battbar remembers where.
- One menu with a section per device (battery, components, connection, sources); each
  device's actions (Move, Hide, Merge With, Split, Unlink, Batteries, Voltage
  Calibration) are in the submenu of its name.
- Devices are ordered by type (keyboard, mouse, headset, others); *Move Left / Right*
  in a device's submenu changes the order, Settings → Menu Bar resets it.

### Added

- `battbar-app --probe-menu-bar [--width N]`: a diagnostic that shows a test menu bar
  item and logs whether macOS shows or hides it (notch, long app menus, external
  displays) — groundwork for collapsing automatically when the menu bar is full.

## [0.3.0] - 2026-11-20

### Added

- Voltage calibration per device: a keyboard or mouse that reports only its voltage
  on the receiver but a reliable percentage of its own on Bluetooth no longer jumps
  between the two. While both are available, battbar learns which voltage the device
  firmware calls which percentage and uses that on the receiver. Shown under the
  device's **Voltage Calibration** menu, with Reset. Devices whose Bluetooth
  percentage is known to be wrong (G915, see Fixed) are never calibrated from it.
- Optional update check, **off by default**: Settings → *Check for Updates* (also
  offered once on first start). When on, battbar asks GitHub once a day for the latest
  release — the only network request battbar makes; it sends no device data or
  identifiers (README → Privacy lists exactly what is sent). A newer version shows
  *Update available: vX.Y.Z* in the menu (the `brew upgrade` command for Homebrew
  installs, otherwise the release page) and one notification per version.
- README: *Updating* section for Homebrew and the disk image.

### Changed

- New app icon and menu bar glyph: a bat — battbar — with echolocation arcs, in the
  same white and periwinkle palette.
- Development builds use their own HID++ software ID, so they can run next to an
  installed release without mixing up replies over Bluetooth.
- Privacy statement: battbar is no longer "no network access" but "no telemetry, one
  opt-in request" (the update check above); a test fails if any other code uses the
  network.

### Fixed

- Classic Bluetooth headsets (e.g. Arctis Nova Pro on Bluetooth without the base
  station) showed "Battery not reported" in the app while `battbar probe` showed the
  value: macOS's IOBluetooth gives a process the battery values only during its first
  seconds, then 0. The app now reads them on every poll in a short-lived helper process
  (the bundled `battbar` CLI).
- G915 on Bluetooth always showed 100 %: its BLE Battery Service does not follow the
  charge (still 100 % while G HUB shows 91 %). The device database now flags it, so the
  value comes from the keyboard's voltage as on the receiver; voltage calibration no
  longer learns from it (samples learned by 0.3.0-rc.1 are dropped).
- A SteelSeries headset with a learned Bluetooth link could disappear on Bluetooth
  (base station unplugged): its device ID depended on which connections were present,
  so a hidden ID from an earlier state could hide it. Linked and manually merged
  devices now keep one ID. Devices hidden under the old IDs may need to be hidden
  again.

## [0.2.0] - 2026-11-19

### Added

- Launch at Login: asked once on first start ("Not Now" is final; the switch stays in
  Settings). The switch shows the state macOS has registered and never turns itself
  back on; approval and "not found" cases explain what to do and open System Settings →
  Login Items. The question is postponed while macOS runs the app from a translocated
  copy.
- Disk image (`battbar-<version>.dmg`) with the usual drag-to-Applications window,
  signed like the app; the `.zip` stays for Homebrew.
- When opened straight from a download (App Translocation), battbar explains once how to
  move it to the Applications folder (*Show in Finder* selects the downloaded copy),
  keeps *Move battbar to Applications…* in its menu, and does not register Launch at
  Login for the temporary copy.
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

[Unreleased]: https://github.com/StanislavTugarev/battbar/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/StanislavTugarev/battbar/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/StanislavTugarev/battbar/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/StanislavTugarev/battbar/releases/tag/v0.1.0
