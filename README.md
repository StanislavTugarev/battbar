# battbar

Menu bar battery indicators for wireless mice, keyboards and headsets on macOS —
whether they are connected through a 2.4 GHz receiver / base station or over Bluetooth.
One icon with a percentage per device. Free and open source (GPL-3.0-or-later).

Works with Logitech HID++ devices (Lightspeed, Unifying, Bolt, Bluetooth) and
SteelSeries wireless headsets, plus any Bluetooth device whose battery macOS already
knows. battbar is not affiliated with or endorsed by Logitech or SteelSeries.

> **Status: early development.** The menu bar app and the command line tool work on
> the reference hardware; packaging and releases come next. See [`docs/SPEC.md`](docs/SPEC.md) and [`docs/DISCOVERY.md`](docs/DISCOVERY.md).

## Command line

```
battbar probe [--json]        # poll all devices once
battbar dump [--out FILE]     # diagnostic report for bug reports (no personal data)
battbar record --out FILE     # record vendor-interface traffic for a new device
battbar probe --replay FILE   # answer from a recorded trace instead of hardware
```

`--no-bluetooth` skips Bluetooth sources (no permission prompt);
`--no-headsetcontrol` skips the optional `headsetcontrol` fallback.

Example (reference hardware):

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

`*` marks the source the percentage comes from; `~` means coarse or approximate.

## Architecture

```
            ┌──────────────────── battbar-app (menu bar) ─── battbar-cli ┐
            │                         battbar-core                       │
            │        Device / Reading, BatteryProvider, merging, DB      │
            └──────┬──────────────────────┬───────────────────────┬──────┘
          battbar-hidpp          battbar-steelseries        battbar-bt-system
     HID++ 2.0 (features,       base-station reports,     battery macOS knows:
     battery, identity)         headsetcontrol fallback   BLE Battery Service,
            │                           │                 classic BT audio
   ┌────────┴─────────┐                 │                         │
 HidapiTransport   GattTransport    HidapiTransport          CoreBluetooth /
 (USB receivers,   (BLE, Logitech   (USB base station)       IOBluetooth
  vendor iface)     GATT service)
```

- **One protocol implementation, several links.** Transports move frames; HID++ logic
  lives once. Recording and replay transports make every provider testable on traces
  without hardware.
- **One device, one icon.** A device seen through several providers or transports
  (keyboard on the receiver, then on Bluetooth) is merged by its HID++ unit ID or
  another stable identifier.
- **Device database as data**: `devices/*.toml`, embedded at build time, overridable
  in `~/.config/battbar/devices/`.

## Permissions

- **Bluetooth** — to read the battery that macOS already knows for paired Bluetooth
  devices and to talk to Logitech devices over their vendor Bluetooth service. battbar
  never scans for devices.
- **Notifications** (optional) — low-battery alerts.
- **No Input Monitoring.** battbar opens only vendor-specific HID interfaces of
  receivers and base stations, which macOS does not treat as input; it cannot read
  keystrokes and never asks for that permission.

## Privacy

- Only vendor-specific HID interfaces are opened (usage page ≥ 0xFF00). Interfaces
  that carry keyboard or mouse input are refused by the transport layer, and tests
  enforce it.
- On Bluetooth no HID device is opened at all. Logitech devices are queried through
  their separate vendor GATT service; other devices through the battery value macOS
  already has. battbar never scans for Bluetooth devices; it only looks at devices
  paired with and connected to your Mac.
- No network access, no telemetry.

## Development

```
cargo test --workspace      # all tests run on recorded traces, no hardware needed
cargo run -p battbar-cli -- probe --no-bluetooth
```

Bluetooth from a development build: macOS asks for Bluetooth permission for the
`battbar` binary itself (it re-launches itself so the permission is not attributed to
your terminal). Every rebuild is a new identity for macOS and asks again, so copy a
build to a stable path for repeated Bluetooth runs — remove the old file first
(`rm` then `cp`; overwriting a signed binary in place gets it killed).

## License

GPL-3.0-or-later. Protocol knowledge partly derived from
[Solaar](https://github.com/pwr-Solaar/Solaar) and
[HeadsetControl](https://github.com/Sapd/HeadsetControl); see [`NOTICE`](NOTICE).
