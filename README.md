# battbar

Battery indicators in the macOS menu bar for wireless mice, keyboards and headsets —
whether they are connected through a 2.4 GHz receiver or base station, or over
Bluetooth. One icon with a percentage per device. Free and open source
(GPL-3.0-or-later), no telemetry. The only network request is an optional daily
update check, off unless you turn it on.

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
brew tap StanislavTugarev/battbar
brew install --cask StanislavTugarev/battbar/battbar
```

Updating: see [Updating](#updating); uninstall with
`brew uninstall --cask StanislavTugarev/battbar/battbar`.

*Why the full name?* Since Homebrew 6, **Tap Trust** is on by default: Homebrew does not
load formulae or casks from taps outside Homebrew's own until you trust them, because a
cask is Ruby code that runs with your user's rights whenever Homebrew loads it.
Installing by the fully qualified name (`StanislavTugarev/battbar/battbar`) trusts
**only this one cask** — not the rest of the tap and nothing added to it later — and
`brew upgrade` keeps working for it. The short form `brew install --cask battbar` works
only after the cask is trusted, so it is not the recommended way. If you ever need the
trust without installing (e.g. in a script), trust just the cask:
`brew trust --cask StanislavTugarev/battbar/battbar`. There is no need to trust the
whole tap. Details: [Homebrew Tap Trust](https://docs.brew.sh/Tap-Trust).

**Manually:** download `battbar-<version>.dmg` from
[Releases](https://github.com/StanislavTugarev/battbar/releases) (optionally check it against
`SHA256SUMS`), open it and **drag battbar onto the Applications folder** in the window
that appears; then open battbar from Applications and eject the disk image. (A `.zip`
is published as well, for Homebrew.) An app opened straight from the disk image or from
Downloads runs from a temporary copy (App Translocation): battbar then shows *Move
battbar to the Applications folder* once, keeps a *Move battbar to Applications…* item
in its menu, and does not offer Launch at Login until it has been moved.

**First launch (Gatekeeper).** battbar is signed with its own certificate but not
notarized by Apple, so on current macOS (15 Sequoia, 26 Tahoe) the first start is
blocked once:

1. Open battbar. macOS shows *"battbar" Not Opened* (Apple could not verify it is free
   of malware). Click **Done** — not *Move to Trash*.
2. Open **System Settings → Privacy & Security** and scroll down to **Security**. Next to
   *"battbar" was blocked to protect your Mac* click **Open Anyway**, and confirm with
   your password or Touch ID.
3. Open battbar again and click **Open** in the dialog that follows.

This is needed only once per installation; updates signed with the same certificate
open normally and keep the permissions you granted. (Right-click → Open no longer
bypasses Gatekeeper on these macOS versions.)

The command line tool `battbar` is installed with the cask (it lives inside the app at
`battbar.app/Contents/MacOS/battbar`).

## Updating

**Homebrew:**

```
brew upgrade --cask battbar
```

Homebrew quits battbar, replaces the app and opens it again; Launch at Login and the
permissions you granted stay (updates are signed with the same certificate).

**Disk image:** download the new `battbar-<version>.dmg` from
[Releases](https://github.com/StanislavTugarev/battbar/releases), quit battbar (menu →
*Quit battbar*), drag battbar onto Applications in the disk image window and choose
**Replace**, then open it again. Settings and permissions stay.

**Hearing about new versions.** Either watch the repository on GitHub (**Watch →
Custom → Releases**), or turn on **Settings → Check for Updates** in the battbar menu
(also offered once on first start). With it on, battbar asks GitHub once a day for the
latest release (see [Privacy](#privacy) for exactly what is sent); when there is a newer
one, the menu shows *Update available: vX.Y.Z* — with the `brew upgrade` command if
battbar was installed with Homebrew, otherwise it opens the release page — and you get
one notification per version. Pre-releases are ignored.

## Using it

- Each device gets an icon by type with its battery. `~` means the value is coarse
  (e.g. 9 levels) or estimated (from voltage); ⚡︎ means charging; a dimmed icon without
  percentage means the device is asleep or disconnected.
- Click an icon for details: connection, components (e.g. the spare battery in a base
  station), where the value comes from, and actions for that device — hide it, merge it
  with another entry, split it, or unlink a remembered Bluetooth connection.
- **Settings** (in every menu): launch at login, update check (off by default),
  update interval, low-battery alert threshold (default 15 %, once per discharge),
  spare-battery alert, devices without a battery value, `headsetcontrol` fallback. **Other Devices** lists hidden devices.
- **Launch at Login.** On first start battbar asks once whether it should start at
  login (*Not Now* means it will not ask again; the switch stays in Settings). The
  switch always shows what macOS has registered — if you turn battbar off under
  System Settings → General → Login Items, it stays off. If macOS needs your approval,
  battbar says so and opens that pane.
- **Voltage calibration.** Some devices (e.g. G915) report only a voltage on the
  receiver but their own percentage on Bluetooth. While a device is on Bluetooth,
  battbar learns how its firmware maps voltage to percentage and uses that when the
  device is on the receiver, so the value stays consistent. The device menu shows
  **Voltage Calibration** with a Reset item.
- **AA/AAA batteries (experimental).** Devices whose firmware reports a percentage show
  it as is. Devices that report only a voltage get an estimate from an alkaline or NiMH
  discharge curve (choose under the device's **Batteries** menu; default alkaline — the
  two cannot be told apart by voltage). These values are always marked `~`: the curves
  come from published cell datasheets, not from measurements on a real device yet.
  Alerts for such devices say "replace the batteries".

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
- No telemetry. battbar makes **one** kind of network request, and only if you turn on
  **Settings → Check for Updates** (off by default): at most once every 24 hours,
  `GET https://api.github.com/repos/StanislavTugarev/battbar/releases/latest` with the
  headers `User-Agent: battbar/<version>` and `Accept: application/vnd.github+json` —
  no token, no cookies, no device data, no identifiers. GitHub sees your IP address and
  that User-Agent. The request is made with the system `curl` (HTTPS only, no
  redirects). Turn it off with the same switch; nothing is requested while it is off,
  and tests check that no other code in battbar uses the network.
- Settings stay in `~/.config/battbar/`; device identifiers in them are stored as
  hashes.
- `battbar dump` and `battbar record` (for bug reports) contain no serial numbers,
  Bluetooth addresses or keystrokes; unit IDs in traces are replaced with pseudonyms.

## Supported devices

See [docs/SUPPORTED_DEVICES.md](docs/SUPPORTED_DEVICES.md). In short: any Logitech
HID++ 2.0 device with a battery feature on a listed receiver, cable or Bluetooth LE;
SteelSeries Arctis Nova Pro Wireless natively, other SteelSeries headsets via
[HeadsetControl](https://github.com/Sapd/HeadsetControl) if installed; any Bluetooth
device whose battery macOS knows. Voltage-only devices on AA/AAA cells are supported
experimentally (estimates from alkaline/NiMH curves).

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
G915 WIRELESS RGB MECHANICAL GAMING KEYBOARD  [Keyboard]  ~88%, 4054 mV, approximate
  * Bluetooth LE   HID++ 0x1001               ~88%, 4054 mV, approximate
    receiver       HID++                      disconnected
    Bluetooth LE   BLE Battery Service        ~100%, approximate
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
