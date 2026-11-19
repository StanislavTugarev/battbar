# battbar

Menu bar battery indicators for wireless mice, keyboards and headsets on macOS —
whether they are connected through a 2.4 GHz receiver / base station or over Bluetooth.
One icon with a percentage per device. Free and open source (GPL-3.0-or-later).

Works with Logitech HID++ devices (Lightspeed, Unifying, Bolt, Bluetooth) and
SteelSeries wireless headsets, plus any Bluetooth device whose battery macOS already
knows. battbar is not affiliated with or endorsed by Logitech or SteelSeries.

> **Status: early development.** Stage 0 (hardware discovery) is done; nothing is
> usable yet. See [`docs/SPEC.md`](docs/SPEC.md) and [`docs/DISCOVERY.md`](docs/DISCOVERY.md).

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

## Privacy

- Only vendor-specific HID interfaces are opened (usage page ≥ 0xFF00). Interfaces
  that carry keyboard or mouse input are refused by the transport layer, and tests
  enforce it.
- On Bluetooth no HID device is opened at all. Logitech devices are queried through
  their separate vendor GATT service; other devices through the battery value macOS
  already has. battbar never scans for Bluetooth devices; it only looks at devices
  paired with and connected to your Mac.
- No network access, no telemetry.

## License

GPL-3.0-or-later. Protocol knowledge partly derived from
[Solaar](https://github.com/pwr-Solaar/Solaar) and
[HeadsetControl](https://github.com/Sapd/HeadsetControl); see `NOTICE` (to be added).
