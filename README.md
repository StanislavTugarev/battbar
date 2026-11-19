# battbar

Battery indicators in the macOS menu bar for wireless mice, keyboards and headsets —
whether they are connected through a 2.4 GHz receiver or base station, or over
Bluetooth. One icon with a percentage per device. Free and open source
(GPL-3.0-or-later), no network access, no telemetry.

![battbar in the menu bar: keyboard ~96 %, mouse 59 % charging, headset ~100 %](docs/images/menu-bar.png)

Works with Logitech HID++ devices (Lightspeed, Unifying, Bolt receivers, USB cable and
Bluetooth), SteelSeries wireless headsets, and any Bluetooth device whose battery macOS
already knows. **battbar is not affiliated with or endorsed by Logitech or SteelSeries.**

## Why

- Devices behind 2.4 GHz receivers are plain USB devices to macOS: it knows nothing
  about their battery. battbar asks them with the vendor protocols (Logitech HID++ 2.0,
  SteelSeries base station reports).
- Over Bluetooth macOS shows the battery only for devices that report it in standard
  ways, and only in its own UI. battbar shows those too, and talks to Logitech devices
  over their own Bluetooth service for charging state and details.
- One device on two connections (a keyboard switched from its receiver to Bluetooth, a
  headset on its base station and on Bluetooth) stays **one icon**.
- Vendor apps are heavy and have no menu bar indicators. battbar runs next to them
  without interfering.

## Install

**Homebrew**

```
brew install --cask OWNER/battbar/battbar
```

**Manually:** download `battbar-<version>.zip` from
[Releases](https://github.com/OWNER/battbar/releases), check it against `SHA256SUMS`,
unzip and move `battbar.app` to `/Applications`.

**First launch.** battbar is signed with its own certificate but not notarized by
Apple, so macOS refuses to open it the first time. Open **System Settings → Privacy &
Security**, scroll down and click **Open Anyway** next to battbar (or, from a terminal:
`xattr -dr com.apple.quarantine /Applications/battbar.app`). Updates signed with the
same certificate keep the permissions you granted.

The command line tool `battbar` is installed with the cask (it lives inside the app at
`battbar.app/Contents/MacOS/battbar`).

## Using it

- Each device gets an icon by type with its battery. `~` means the value is coarse
  (e.g. 9 levels) or estimated (from voltage); ⚡︎ means charging; a dimmed icon without
  percentage means the device is asleep or disconnected.
- Click an icon for details: connection, components (e.g. the spare battery in a base
  station), where the value comes from, and actions for that device — hide it, merge it
  with another entry, split it, or unlink a remembered Bluetooth connection.
- **Settings** (in every menu): launch at login, update interval, low-battery alert
  threshold (default 15 %, once per discharge), spare-battery alert, Bluetooth devices
  without battery, `headsetcontrol` fallback. **Other Devices** lists hidden devices.

## Permissions

- **Bluetooth** — to read the battery that macOS already knows for paired Bluetooth
  devices and to talk to Logitech devices over their vendor Bluetooth service. battbar
  only looks at devices paired with and connected to this Mac; it never scans.
- **Notifications** (optional) — low-battery alerts.
- **No Input Monitoring.** battbar opens only the vendor-specific interfaces of
  receivers and base stations, which macOS does not treat as input. It cannot read
  keystrokes and never asks for that permission.

## Privacy

- Only vendor-specific HID interfaces are opened (usage page ≥ 0xFF00). Interfaces that
  carry keyboard or mouse input are refused by the transport layer, and tests enforce it.
- On Bluetooth no HID device is opened at all: Logitech devices are queried through
  their separate vendor GATT service, everything else through the battery value macOS
  already has.
- No network access, no telemetry. Settings stay in `~/.config/battbar/`; device
  identifiers in them are stored as hashes.
- `battbar dump` and `battbar record` (for bug reports) contain no serial numbers,
  Bluetooth addresses or keystrokes; unit IDs in traces are replaced with pseudonyms.

## Supported devices

See [docs/SUPPORTED_DEVICES.md](docs/SUPPORTED_DEVICES.md). In short: any Logitech
HID++ 2.0 device with a battery feature on a listed receiver, cable or Bluetooth LE;
SteelSeries Arctis Nova Pro Wireless natively, other SteelSeries headsets via
[HeadsetControl](https://github.com/Sapd/HeadsetControl) if installed; any Bluetooth
device whose battery macOS knows.

Device missing or value wrong? [docs/ADDING_A_DEVICE.md](docs/ADDING_A_DEVICE.md)
explains how to collect what we need — no programming required.

## Command line

```
battbar probe [--json]        # poll all devices once
battbar dump [--out FILE]     # diagnostic report for bug reports
battbar record --out FILE     # record vendor-interface traffic for a new device
battbar probe --replay FILE   # answer from a recorded trace instead of hardware
```

`--no-bluetooth` skips Bluetooth sources; `--no-headsetcontrol` skips the
`headsetcontrol` fallback.

```
G502 X PLUS  [Mouse]  11%, exact
  * receiver       HID++ 0x1004               11%, exact
G915 WIRELESS RGB MECHANICAL GAMING KEYBOARD  [Keyboard]  100%, 4148 mV, exact
    Bluetooth LE   HID++ 0x1001               ~97%, 4148 mV, approximate
    receiver       HID++                      disconnected
  * Bluetooth LE   BLE Battery Service        100%, exact
Arctis Nova Pro Wireless  [Headset]  83%, exact
    Spare: ~88%, coarse
    base station   SteelSeries base station   ~100%, coarse
  * Bluetooth      macOS (Bluetooth)          83%, exact
```

`*` marks the source the percentage comes from.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) and
the design in [docs/SPEC.md](docs/SPEC.md). Changes are listed in
[CHANGELOG.md](CHANGELOG.md).

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). Protocol knowledge partly derived from
[Solaar](https://github.com/pwr-Solaar/Solaar) and
[HeadsetControl](https://github.com/Sapd/HeadsetControl); see [NOTICE](NOTICE).
Logitech, Lightspeed, Unifying, Bolt, SteelSeries and Arctis are trademarks of their
respective owners.
