# battbar — specification

Open-source menu bar indicators for the battery of wireless peripherals on macOS.

This is the working specification of the project. Changes to the sections marked
**fixed** need the maintainer's approval. Revision history is at the end.

## Goal

A free, open-source macOS menu bar app that shows the battery of wireless mice,
keyboards and headsets regardless of how they are connected: through proprietary
2.4 GHz receivers and base stations (Logitech Lightspeed / Unifying / Bolt,
SteelSeries) and over Bluetooth. One status item with a percentage per device.

Why:

- Devices behind 2.4 GHz receivers look like plain USB HID devices to macOS. The OS
  knows nothing about the radio link or the battery; it has to be queried with vendor
  protocols on top of HID (Logitech HID++ 2.0, SteelSeries vendor reports).
- Over Bluetooth macOS only knows the battery of devices that report it in standard
  ways (BLE Battery Service, HFP indicators). Many devices, e.g. Logitech, report
  details (charging state, voltage) only through their own protocol.
- Vendor software (G HUB, Options+, SteelSeries GG) is heavy and has no menu bar
  indicators.

Value: all peripherals of different vendors and transports in one lightweight indicator.

## Reference hardware

Development and acceptance happen on:

- Mouse: Logitech G502 X PLUS — via Lightspeed receiver
- Keyboard: Logitech G915 — via the same Lightspeed receiver **and over Bluetooth (BLE)**
- Headset: SteelSeries Arctis Nova Pro Wireless — via its base station over USB
  **and over Bluetooth (classic, HFP/A2DP)**
- One shared Lightspeed receiver (046d:c547) for mouse and keyboard
- macOS 26 (Tahoe), Apple Silicon; G HUB and SteelSeries GG installed and running

Keyboard and headset are the test devices for both transports. Measured details are in
[`DISCOVERY.md`](DISCOVERY.md).

## Project decisions (fixed)

- **Language**: Rust (stable). **License**: GPL-3.0-or-later.
- Porting protocol logic from HeadsetControl (GPL-3.0) and Solaar (GPL-2.0-or-later,
  compatible with GPL-3.0) is allowed, with attribution in `NOTICE` and file headers.
- All code, UI, README, commits and issue templates are **in English**.
- **No network requests and no telemetry.** Updates come via Homebrew / GitHub Releases.
- Name, icons and texts use no vendor trademarks or logos. Wording like
  "works with Logitech … devices" plus a "not affiliated" disclaimer.
- macOS is the primary platform. Code should not close the door to Windows/Linux
  (`hidapi` is cross-platform), but supporting them is not required now.
- **Bluetooth access** (revised after Stage 0):
  - No active scanning, ever: no `scanForPeripherals`. Only devices paired with and
    connected to this Mac, reached via `retrieveConnectedPeripherals(withServices:)`
    (CoreBluetooth) or IOBluetooth's paired-device list.
  - **No `IOHIDDevice` is ever opened on the Bluetooth transport** (neither through
    `hidapi` nor IOKit directly): on macOS a Bluetooth HID peripheral is a single
    `IOHIDDevice` that also carries the keyboard/mouse collections.
  - Logitech HID++ over Bluetooth (BLE) goes through the **Logitech vendor GATT
    service** (`00010000-0000-1000-8000-011F2000046D`, characteristic
    `00010001-0000-1000-8000-011F2000046D`) via CoreBluetooth (`objc2`).
  - System-known battery: CoreBluetooth **BLE Battery Service** (0x180F / 0x2A19) for
    BLE devices; **IOBluetooth** for classic Bluetooth audio;
    `system_profiler SPBluetoothDataType -json` only as a fallback for BLE.
  - `hidapi` is used only for USB devices (receivers, base stations, wired devices).
- **The CLI lives inside the `.app` bundle** (`Contents/MacOS/battbar`), symlinked into
  `PATH` by the Homebrew cask, so macOS attributes Bluetooth access to the app
  (a standalone CLI is killed by TCC when it touches Bluetooth).

## Architecture

Cargo workspace:

| Crate | Role |
|---|---|
| `battbar-core` | Types (`Device`, components, `Reading { percent, charging, status, precision }`, `status ∈ ok \| asleep \| disconnected \| error`, `precision ∈ exact \| coarse \| approximate`), `BatteryProvider` trait, source merging, device database loading, transport abstraction |
| `battbar-hidpp` | Logitech HID++ 2.0 provider on all transports: Unifying, Bolt, Lightspeed, direct USB, **Bluetooth via GATT** |
| `battbar-steelseries` | SteelSeries provider (native port of HeadsetControl protocols) + optional `headsetcontrol` fallback |
| `battbar-bt-system` | Battery that macOS already knows for Bluetooth devices: BLE Battery Service (CoreBluetooth), classic audio (IOBluetooth), `system_profiler` fallback. Speaks no vendor protocol |
| `battbar-macos` | Thin `objc2` bindings: CoreBluetooth central (retrieve-connected only), IOBluetooth paired devices, notifications, SMAppService |
| `battbar-cli` | `battbar probe / dump / record` |
| `battbar-app` | Menu bar app; bundles the CLI |

### Transports

`HidppTransport`-style abstraction: **frame in / frame out**, so protocol code is
shared across physical links. HID++ features, request/response matching and software-ID
filtering live once in `battbar-hidpp`; transports only move bytes.

| Implementation | Link | Framing done by the transport |
|---|---|---|
| `HidapiTransport` | USB receivers, wired devices | HID report ID 0x10/0x11 (short/long), device index from the request |
| `GattTransport` | BLE (Logitech vendor characteristic) | strips report ID and device index: 18-byte frame `[feature, fn<<4\|sw, 16 params]`; write with response, replies as notifications |
| `RecordingTransport` | wraps any of the above | logs every frame to JSONL |
| `ReplayTransport` | tests | answers from a recorded trace |

Rules enforced inside the transport layer (and covered by tests):

- `hidapi` opens non-exclusively (`set_open_exclusive(false)`).
- A HID path is opened only if **every** top-level collection of that `IOHIDDevice`
  is vendor-defined (usage page ≥ 0xFF00). Generic Desktop / Keyboard / Consumer →
  error.
- Bus type Bluetooth → `hidapi` open is refused outright.
- Responses are matched on our own non-zero software ID on **every** transport,
  including GATT notifications (G HUB / Options+ may share the characteristic).

### Providers

`BatteryProvider`: `discover()` finds its devices, `poll()` returns a `Reading`,
optionally a stream of push events. The core knows nothing about vendors or transports.

### Device identity and merging

One physical device may be visible to several providers at once (Logitech over BLE:
HID++ via GATT and the BLE Battery Service) or move between transports (keyboard from
Lightspeed to Bluetooth). The core merges such sources into one device and one icon:

- Logitech: HID++ 0x0003 Device Information **unit ID + model ID** (same on all
  transports; verified on the G915, which has no serial number feature).
- Others: a stable identifier if the provider has one; otherwise **manual** merge/split
  in settings. Merging by name alone is never done automatically.
- Headsets behind a base station (the Nova Pro has no VID/PID over Bluetooth — vendor
  0x000A / product 0xFFFF, Discovery §4.5) are merged automatically **only if both**
  hold: (a) the base reports that its headset is currently on Bluetooth (base status
  bytes 4–5, to be confirmed in Stage 1), and (b) the Bluetooth device name matches
  the model's `bt_name_patterns` in `devices/*.toml`. Otherwise only manual merge.
- When the same unit ID is reachable on another transport the devices are merged;
  there is no separate UI state for "paired on the receiver but currently elsewhere".
- Merged reading:
  - **percent** comes from the source with the best `precision`
    (`exact` > `coarse` > `approximate`); on a tie, the source priority below decides;
  - **flags** (`charging`, external power, …) are combined from all live sources;
  - `status` is the best of the sources (`ok` > `asleep` > `disconnected` > `error`).
- Source priority on ties: vendor protocol (has charging state) → system Bluetooth
  battery. Example: Nova Pro through the base is `coarse` (9 levels); if Bluetooth turns
  out to be `exact`, the percentage comes from Bluetooth and charging state from the base.

**Bluetooth source order**

| Device class | Order |
|---|---|
| Logitech over BLE | HID++ via GATT → BLE Battery Service → `system_profiler` |
| Other BLE | BLE Battery Service → `system_profiler` |
| Classic Bluetooth audio | IOBluetooth only (`system_profiler` does not carry it) |

### Device database

Data, not code: `devices/*.toml` (VID/PID of receivers and bases, PIDs on the BT
transport, model names, device type, discharge curves for voltage features, protocol
quirks). Embedded into the binary at build time; user override in
`~/.config/battbar/devices/*.toml`. Adding a device that uses an already supported
protocol requires no Rust changes.

## Protocol requirements

### Logitech HID++ 2.0

- USB: use the vendor-specific HID interface found by usage page, never a hard-coded
  PID. The HID++ usage pages and usages (today 0xFF00 and 0xFF43, short/long/very long)
  live in the device database / config, not in code.
- Receivers: iterate device index 0x01–0x06. Direct connection (USB cable): 0xFF.
- Receivers: read the pairing registers (0xB5 / 0x20+n) to know which slots are paired.
  Paired slot + ping error 0x08 → device not on this receiver (`disconnected`, or merged
  with the same unit ID on another transport).
- Receivers: enable receiver notification flags (HID++ 1.0 register 0x00) if they are
  not already set, to receive connect/disconnect events.
- Bluetooth: `GattTransport` as described above; no device index on that link.
- Find features via IRoot (0x0000 getFeature), no hard-coded indexes. Preference:
  **0x1004** (Unified Battery) → **0x1001** (Battery Voltage) → **0x1000**.
- 0x1001: mV → % using the curve from the device database; without a curve for the
  model, a generic Li-ion default curve, `precision = approximate`.
  **Curves are calibrated only from measurements on battery (no cable)**; readings
  taken while charging are marked `charging` and never used for calibration.
- Short (0x10) and long (0x11) reports on USB.
- Own non-zero software ID in the low nibble of the function byte; filter replies by it.
  Never open devices exclusively.
- HID++ "device unavailable" → `asleep` / `disconnected`, **never 0 %**. Error codes
  are mapped per request (0x09 means different things for a ping and a register read).
- Subscribe to 0x1004 battery notifications where supported.

### SteelSeries

- Native implementation for base station / dongle, starting with the author's model;
  structure allows adding models through the device database plus small protocol modules.
- Fallback provider: external `headsetcontrol -b -o json` if installed, for models
  without native support. Can be switched off in settings.
- Over Bluetooth the headset battery comes from `battbar-bt-system`. When the headset
  is connected through the base and over Bluetooth at the same time and the merge
  conditions hold, it is one device; percent and flags follow the merge rules above.
- Spare battery charging in the base (hypothesis: status byte 7, 0–8): if confirmed in
  Stage 1, it is the component `spare` of the headset device, not a separate icon.
  Optional notification "spare battery fully charged", off by default.

### Bluetooth (system battery)

- Enumerate paired devices and take the battery macOS knows, including split
  components (left/right/case) where the system provides them; `Device` supports
  components, each with its own `Reading`.
- Classic Bluetooth audio: IOBluetooth `batteryPercentSingle`, plus
  `batteryPercentLeft` / `Right` / `Case` as components. These are private properties:
  check `respondsToSelector:` before every access. Property missing, or value 0 on a
  connected device → "battery not reported", never a crash and never 0 %.
- Precision of the classic audio value is `coarse` until its step is measured. If the
  step turns out to be 1 % (likely the HFP 1.7 battery HF indicator rather than
  `AT+IPHONEACCEV`), it becomes `exact`.
- Battery from `AT+IPHONEACCEV` (10 % steps) → `precision = coarse`.
- Paired but not connected → `disconnected`, no percentage.
- Connected but no battery reported → shown in the menu as "battery not reported", no
  menu bar icon by default (configurable).

### General

- Per-request timeouts (1–2 s); one hung device never blocks the others.
- Survive unplugging/replugging receivers and bases, BT connect/disconnect, and a
  device switching transports (hotplug).

## Privacy and security (critical)

The keyboard receiver physically carries keystrokes. Whether the app needs **Input
Monitoring** at all is checked with the `.app` in Stage 2: if opening vendor-only USB
interfaces does not require it, the app never asks for it and the README does not
mention it.

- Read and write **only** vendor-specific interfaces (HID++ / vendor reports). Never open
  interfaces with Generic Desktop usage pages — on USB or Bluetooth.
- On Bluetooth no `IOHIDDevice` is opened at all; HID++ goes over GATT.
- Traffic recording and diagnostic dumps contain only these interfaces.
- Tests: opening a keyboard/mouse usage page → error; opening any Bluetooth HID device
  → error.
- The app needs **Bluetooth** permission (`NSBluetoothAlwaysUsageDescription`) to read
  paired-device information. The air is never scanned.
- README has a section explaining why Input Monitoring and Bluetooth are needed and why
  the app cannot read keystrokes.

## Stages

### Stage 0 — Discovery on reference hardware ✔

See [`DISCOVERY.md`](DISCOVERY.md).

### Stage 1 — Core + CLI

- Workspace, traits, transports with record/replay, source merging, device database,
  all providers.
- `battbar probe [--json]` — one-shot poll of all devices (with transport and data source).
- `battbar dump` — diagnostic report for issues (VID/PID, transport, interfaces, features,
  version; no user data).
- `battbar record --out trace.jsonl` — record traffic with vendor interfaces / GATT.
- Provider tests on recorded traces of the reference hardware; merging tests.

### Stage 2 — Menu bar app

- One status item per device; icon by type (mouse/keyboard/headset) + percentage;
  charging indicator; dimmed icon without percentage for asleep/disconnected;
  `~` before the percentage for `coarse`/`approximate`.
- Click menu: name, transport, status, components, last update, "Refresh",
  "Settings", "Quit".
- Settings: hide device, merge/split devices, low-battery threshold, poll interval,
  headsetcontrol fallback toggle, show BT devices without battery, launch at login
  (SMAppService).
- Poll every 60 s + push events where available.
- macOS notification on low battery (default 15 %, once per discharge cycle).
- Missing Input Monitoring / Bluetooth permission: clear screen with a button opening
  the right System Settings pane.

### Stage 3 — Open-source readiness

- Repository: `README.md`, `CONTRIBUTING.md`, `LICENSE`, `NOTICE`, `CHANGELOG.md`
  (Keep a Changelog), semver.
- `docs/SUPPORTED_DEVICES.md` generated from the device database (`cargo xtask`), CI
  checks it is up to date.
- `docs/ADDING_A_DEVICE.md`: user runs `battbar dump` and `battbar record` and attaches
  them to an issue; a contributor adds TOML and a trace test.
- Issue templates: "Device not detected / new device", "Wrong battery value", "Bug".
- CI (GitHub Actions, macOS runner): `fmt`, `clippy -D warnings`, trace tests,
  `cargo deny` (GPL-3.0-compatible licenses only).
- Releases: tag-triggered universal binary (aarch64 + x86_64 via `lipo`), `.app` bundle
  with ad-hoc signature and stable bundle identifier, CLI inside the bundle, zip +
  SHA-256 checksums, GitHub Releases.
- Homebrew: own tap with a cask (installs the app and symlinks the CLI).
- Developer ID signing and notarization: **not now**; README explains the Gatekeeper
  workaround; the release workflow is designed so they can be added in one step.

## Acceptance criteria

- On the reference hardware `battbar probe` matches G HUB / the base display within
  ±5 % (±10 % for `coarse`).
- Separate menu bar icons for all devices, updates without restart, correct handling of
  sleep, disconnect and hotplug.
- The Logitech keyboard switched from Lightspeed to Bluetooth and back stays **one**
  icon with a correct value on both transports.
- The SteelSeries headset shows its battery both via the base and over Bluetooth; when
  connected both ways, one icon.
- Any BT mouse/keyboard/headphones whose battery macOS knows is shown without code
  changes.
- A new Logitech device with supported features works without code changes; a device
  with a new receiver PID is added with one TOML file.
- Tests pass in CI without hardware (on traces).
- Tests guarantee that keyboard/mouse usage pages and Bluetooth HID devices are never
  opened.
- Releases are fully automatic on tag; `brew install --cask` from the tap works.
- A third party can collect data for a new device by following
  `docs/ADDING_A_DEVICE.md` without reading code.

## Process

- Commit at the end of every stage and before large refactorings.

## Revision history

- **2026-11-19** — Stage 0 review: classic BT audio via IOBluetooth only (guarded
  private properties, components, precision rule); merged reading takes percent from the
  best precision and combines flags; headset auto-merge only with base flag + name
  pattern; spare battery as component `spare`; HID++ usage pages in data; Input
  Monitoring only if Stage 2 shows it is needed.

- **2026-11-19** — after Stage 0: HID++ over BLE goes through the Logitech GATT
  service via CoreBluetooth instead of `hidapi`; no `IOHIDDevice` on Bluetooth; BT
  source order; CLI inside the `.app` bundle; receiver pairing registers and
  notification flags; 0x1001 curve calibration only off-cable; HID++ also on usage
  page 0xFF43.
