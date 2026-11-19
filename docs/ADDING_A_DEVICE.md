# Adding a device

## Part 1 — for users: collect data (no programming)

You need battbar installed (see the README) and about five minutes.

1. **Connect the device the way it does not work** (receiver, base station, cable or
   Bluetooth). Make sure it is awake: move the mouse, press a key, put the headset on.
2. **Write down the battery level your vendor app or the device shows** (for example
   "G HUB: 57 %", "base display: 3 of 4 bars", "charging"). This is the reference we
   compare against.
3. Open **Terminal** and run:

   ```
   battbar dump --out ~/Desktop/battbar-dump.json
   battbar record --out ~/Desktop/battbar-trace.jsonl --polls 3 --note "G HUB shows 57 %"
   ```

   Put your reference value into `--note`. If macOS asks for Bluetooth access, allow it
   (or add `--no-bluetooth` if the device is not on Bluetooth).
4. Open an issue with the **"Device not detected / new device"** template (or
   **"Wrong battery value"**) and attach both files.

**What the files contain.** `battbar-dump.json`: macOS version, which HID interfaces
exist (vendor/product IDs, usage pages), which protocol features the device reports,
device names, battery readings. `battbar-trace.jsonl`: the bytes exchanged with the
device's *vendor-specific* interface only. Neither contains keystrokes, serial numbers
or Bluetooth addresses; device unit IDs are replaced with pseudonyms, and paired-device
names that look personal ("Anna's AirPods") are redacted. Have a look before uploading —
both are plain text.

**Battery curves.** For devices that only report a voltage, a few recordings at
different levels **on battery (cable unplugged)** help us build an accurate curve:
e.g. at about 100 %, 50 % and 20 %, each with the vendor app's value in `--note`.
Recordings while charging are not used for curves.

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
notes = "Battery via 0x1001."

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
