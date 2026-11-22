# Adding a device

## Part 1 — for users: collect data (no programming)

You need battbar installed (see the README) and about five minutes. Connect the device
the way it does not work (receiver, base station, cable or Bluetooth), make sure it is
awake, and note the battery level your vendor app or the device shows (e.g. "G HUB:
57 %", "charging"). Then collect diagnostics as below and open an issue with the
**"Device not detected / new device"** or **"Wrong battery value"** template.

## How to collect diagnostics

Run in **Terminal** (if macOS asks for Bluetooth access, allow it):

1. Version: `battbar --version`
2. Report: `battbar dump > ~/Desktop/dump.txt`
3. Trace, about 2 minutes:
   `battbar record --out ~/Desktop/trace.jsonl --polls 24 --interval 5 --note "G HUB shows 57 %"`
   While it runs: switch the device off and on, plug the charger in and out, and for a
   receiver or base station unplug it and plug it back in. Put the reference value
   into `--note`.
4. Attach `dump.txt` and `trace.jsonl` to the issue (drag them into the text field).

**Privacy.** The files contain vendor and product IDs of HID interfaces, the protocol
frames exchanged with the device's *vendor-specific* interfaces, device names and
battery readings; serial numbers and unit IDs are replaced with pseudonyms. They contain
no keystrokes or mouse input, no Bluetooth addresses and no user name (your home folder
appears as `~`); paired-device names that look personal ("Anna's AirPods") are redacted.
Both are plain text — have a look before uploading. See also
[Privacy](../README.md#privacy) in the README.

**Battery curves.** For devices that only report a voltage, a few recordings at
different levels **on battery (cable unplugged)** help us build an accurate curve:
e.g. at about 100 %, 50 % and 20 %, each with the vendor app's value in `--note`.
Recordings while charging, or within about 10 minutes after unplugging (the voltage
is still settling), are not used for curves.

## Part 2 — for contributors: add the device

### Is the protocol already supported?

| Device | What to add |
|---|---|
| Logitech device on a known receiver | Nothing is required — HID++ devices are discovered from the device itself. Add an entry to `devices/logitech-devices.toml` for PIDs per transport, notes, or a calibrated curve. |
| Logitech receiver with a new PID | One `[[receiver]]` entry in `devices/logitech-receivers.toml`. |
| HID++ on a new vendor usage page | Add it to `usage_pages` in `devices/hidpp.toml`. |
| SteelSeries headset using an existing base-station protocol | One `[[device]]` entry in `devices/steelseries.toml`. |
| New protocol or protocol variant | A small protocol module + a `Protocol` variant (code change). |

Users can try an entry before it is released by putting the TOML file into
`~/.config/battbar/devices/`.

### Examples

```toml
# devices/logitech-receivers.toml
[[receiver]]
vendor_id = 0x046D
product_id = 0xC548
name = "Bolt receiver"
family = "bolt"          # unifying | lightspeed | bolt

# devices/logitech-devices.toml
[[device]]
name = "Example Keyboard"
kind = "keyboard"        # mouse | keyboard | headset | headphones | gamepad | other
protocol = "hidpp20"
vendor_id = 0x046D
wpid = [0x4080]          # PID behind a receiver (from `battbar dump`)
ble_pid = [0xB380]       # PID on Bluetooth LE
usb_pid = [0xC340]       # PID on a USB cable
voltage_curve = "example-keyboard"   # only for 0x1001 (voltage) devices
# Set only if its BLE Battery Service value does not follow the charge (e.g. stays at
# 100 % while the vendor app shows less): it is then never preferred or learned from.
# ble_battery_service_unreliable = true
notes = "Battery via 0x1001."
# Who checked it: `verified_on` (maintainers, on real hardware) or
# `reported_by_users = true` (from a "Works with my device" issue).
reported_by_users = true

[[curve]]
id = "example-keyboard"
precision = "approximate" # becomes "exact" only with a well-measured curve
points = [[4150, 100], [3900, 70], [3780, 40], [3680, 10], [3500, 0]]  # [mV, %], on battery

# devices/steelseries.toml
[[device]]
name = "Example Headset"
kind = "headset"
protocol = "steelseries-nova-pro"
vendor_id = 0x1038
usb_pid = [0x12FF]
vendor_usage_page = 0xFFC0
bt_name_patterns = ["Example Headset*"]   # its name on Bluetooth, for merging
# Only once a recording shows that this base sends headset events by itself
# (07 b5 / 07 b7, `battbar debug-base-listen`); otherwise the poll reads the headset.
# base_events = true
```

### Add a trace test

1. Copy the user's trace to `traces/<transport>-<device>.jsonl` (keep the note with the
   reference value). Traces must only contain pseudonymized unit IDs — CI checks this;
   a trace recorded with `battbar record` already is.
2. Add a test next to the existing ones in `crates/battbar-hidpp/tests/traces.rs` or
   `crates/battbar-steelseries/tests/traces.rs`:

   ```rust
   #[test]
   fn example_keyboard_on_bolt() {
       let r = poll(&["bolt-example-keyboard.jsonl"]);
       let kb = r.iter().find(|r| r.kind == DeviceKind::Keyboard).unwrap();
       assert_eq!(kb.reading.percent, Some(57)); // reference value from the note
   }
   ```

3. Run `cargo xtask supported-devices` to regenerate
   [SUPPORTED_DEVICES.md](SUPPORTED_DEVICES.md), then `cargo test --workspace`.

You can check a trace without hardware: `battbar probe --replay battbar-trace.jsonl`.
