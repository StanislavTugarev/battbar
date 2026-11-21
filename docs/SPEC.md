# battbar — specification

Open-source menu bar indicators for the battery of wireless peripherals on macOS.

This is the working specification of the project. Changes to the sections marked
**fixed** need the maintainer's approval. Revision history is at the end.

## Goal

A free, open-source macOS menu bar app that shows the battery of wireless mice,
keyboards and headsets regardless of how they are connected: through proprietary
2.4 GHz receivers and base stations (Logitech Lightspeed / Unifying / Bolt,
SteelSeries) and over Bluetooth. One status item: every device with its percentage,
or one icon (collapsed).

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
- **No telemetry. One opt-in network request** (0.3.0): the daily update check, off by
  default (see "Privacy and security"). Updates themselves come via Homebrew / GitHub
  Releases.
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
    BLE devices; **IOBluetooth** for classic Bluetooth audio. No `system_profiler`
    (removed in 0.5: on the reference devices it carried nothing the other sources
    lack, and its cached output kept switched-off devices; see DISCOVERY.md).
  - `hidapi` is used only for USB devices (receivers, base stations, wired devices).
- **Code signing**: a stable self-signed code-signing certificate instead of ad-hoc
  signatures, so TCC permissions (Bluetooth) survive rebuilds and updates. Locally the
  certificate lives in the login keychain (`scripts/create-dev-cert.sh`); the release
  workflow imports it from CI secrets. Developer ID signing and notarization: not now.
  Binaries are always installed with `rm` + copy or an atomic `mv`, never copied over
  an existing file (macOS caches signatures per inode → SIGKILL).
- **The CLI lives inside the `.app` bundle** (`Contents/MacOS/battbar`), symlinked into
  `PATH` by the Homebrew cask. It also embeds its Info.plist and re-launches itself as
  its own responsible process before touching Bluetooth, because TCC otherwise
  attributes the access to the terminal and kills the process.

## Architecture

Cargo workspace:

| Crate | Role |
|---|---|
| `battbar-core` | Types (`Device`, components, `Reading { percent, charging, status, precision }`, `status ∈ ok \| asleep \| disconnected \| error`, `precision ∈ exact \| coarse \| approximate`), `BatteryProvider` trait, source merging, device database loading, transport abstraction |
| `battbar-hidpp` | Logitech HID++ 2.0 provider on all transports: Unifying, Bolt, Lightspeed, direct USB, **Bluetooth via GATT** |
| `battbar-steelseries` | SteelSeries provider (native port of HeadsetControl protocols) + optional `headsetcontrol` fallback |
| `battbar-bt-system` | Battery that macOS already knows for Bluetooth devices: BLE Battery Service (CoreBluetooth), classic audio (IOBluetooth). Speaks no vendor protocol |
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
  byte 5, confirmed in Stage 1), and (b) the Bluetooth device name matches
  the model's `bt_name_patterns` in `devices/*.toml`. Otherwise only manual merge.
- When the same unit ID is reachable on another transport the devices are merged;
  there is no separate UI state for "paired on the receiver but currently elsewhere".
- **Merged reading — one rule for every device and transport:**
  - **percent** comes from the live source with the best `precision`
    (`exact` > `coarse` > `approximate`);
  - on equal precision, the source priority decides: vendor protocol (HID++,
    SteelSeries base) → `headsetcontrol` → system Bluetooth battery;
  - **flags** (`charging`, external power, …) are combined from all live sources;
  - `status` is the best of the sources (`ok` > `not reported` > `asleep` >
    `disconnected` > `error`).
- A device database entry can flag a device's BLE Battery Service as unreliable
  (`ble_battery_service_unreliable`); the HID++ report then carries a hint, and the
  Battery Service reading of that device is treated as `approximate`, so the vendor
  protocol wins on the tie. The G915 is flagged: its Battery Service stayed at 100 %
  from 4148 mV down to 4054 mV while G HUB showed 91 % (macOS shows the same 100 %).
- Device IDs: a HID++ unit ID wins; otherwise a device in a manual group (including a
  learned headset ↔ Bluetooth link) takes its ID from the whole group, so it stays the
  same whether the base, Bluetooth or both are present (before, the ID followed the
  present sources, and a hidden ID could hide the headset on Bluetooth only).
- Examples on the reference hardware: keyboard on Bluetooth — HID++ 0x1001 ~88 %
  (`approximate`) wins over the flagged BLE Battery Service (shown as ~100 %);
  headset on base + Bluetooth — Bluetooth 83 % (`exact`) wins
  over the base 8/8 (`coarse`), the spare battery and cable-charging state come from
  the base.
- Which Bluetooth sources exist per device class (all of them are merged by the rule
  above; there is no per-transport source order):

  | Device class | Sources |
  |---|---|
  | Logitech over BLE | HID++ via GATT, BLE Battery Service |
  | Other BLE | BLE Battery Service (kind "other" unless another source knows it) |
  | Classic Bluetooth audio | IOBluetooth only |

- **Learned headset ↔ Bluetooth link.** The live companion merge (base flag +
  `bt_name_patterns`, above) is remembered so the headset stays one device when its
  Bluetooth is off. A link is stored only when the base's "headset on Bluetooth" flag
  changes `00 → 01` and, within a window of a few seconds, exactly one connected
  Bluetooth device matches the model's `bt_name_patterns`. It is stored locally in the
  settings by the hash of the Bluetooth address (never shown in `dump` or traces).
  Settings offer "Unlink". With several candidates only manual merging is possible.

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
  **Curves are calibrated only from measurements on battery (no cable), outside the
  ~10-minute settling period after charging**; readings taken while charging or
  settling are never used for calibration.
- Short (0x10) and long (0x11) reports on USB.
- Own non-zero software ID in the low nibble of the function byte; filter replies by it.
  Release `0x0B`; development builds `0x0A`, so a development app next to the release
  app never takes its replies (traces are recorded and replayed with `0x0B`). Never
  open devices exclusively.
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
- Spare battery charging in the base (status byte 7, 0–8; supported by Stage 1
  measurements): component `spare` of the headset device, not a separate icon.
  Optional notification "spare battery fully charged", off by default.

### Bluetooth (system battery)

- Enumerate paired devices and take the battery macOS knows, including split
  components (left/right/case) where the system provides them; `Device` supports
  components, each with its own `Reading`.
- Classic Bluetooth audio: IOBluetooth `batteryPercentSingle`, plus
  `batteryPercentLeft` / `Right` / `Case` as components. These are private properties:
  check `respondsToSelector:` before every access. Property missing, or value 0 on a
  connected device → "battery not reported", never a crash and never 0 %.
- IOBluetooth reports these values only for ~1–2 s after a process first uses it, then
  0 for the rest of the process (any thread, with or without a run loop; measured
  2026-11-20). The app is long-running, so every poll reads them in a short-lived
  helper process: the bundled `battbar` CLI with the hidden argument
  `classic-batteries` (prints the paired devices as JSON, reads nothing else, touches
  no settings, prints `[]` without Bluetooth permission; 5 s timeout). If the helper
  is missing or fails, the provider reads in its own process.
- Precision of the classic audio value: `exact` (1 % steps measured in Stage 1, HFP 1.7
  battery HF indicator).
- Battery from `AT+IPHONEACCEV` (10 % steps) → `precision = coarse`.
- Paired but not connected → `disconnected`, no percentage.
- Connected but no battery reported → shown in the menu as "battery not reported", no
  menu bar icon by default (configurable).

### Battery values while charging

Firmware estimates the charge from the cell voltage; while charging the terminal voltage
is inflated by the charge current (V = OCV + I·R), which can read as tens of percent too
much (8 % → 52 % in 15 s on the reference mouse, in the vendor app as well), and it
relaxes for several minutes after unplugging.

- While a device is charging, its percentage is shown with `precision = approximate`
  (`~` and the charging mark) whatever the source; the menu says "Estimated while
  charging".
- For ~10 minutes after charging stops (voltage settling) the value stays
  `approximate` ("Estimated: settling after charging"), then the source's precision is
  shown again.
- Low-battery notifications are not sent while charging or settling. A device gets a
  new low-battery alert only after a measured (not estimated) value has risen above
  threshold + 10 % and then fallen to the threshold again.
- Voltage-curve calibration (0x1001) uses only measurements on battery outside the
  settling period; readings while charging or settling are excluded.

### Per-unit voltage calibration (HID++ 0x1001)

A voltage-only device (e.g. G915) can show two different numbers: on Bluetooth LE its
firmware reports a percentage (BLE Battery Service, `exact`), on the receiver battbar
converts the voltage with the generic Li-ion table (`approximate`). Observed on the
reference keyboard: 100 % on Bluetooth, 91 % on the receiver (4080 mV), 95 % in G HUB.
Later the G915's Battery Service turned out to be the wrong one (still 100 % at
4054 mV, G HUB 91 %): it is flagged in the database, and calibration never learns from
or applies to a device whose Battery Service is flagged (samples learned before are
dropped).

- While a device reports both at the same time (HID++ 0x1001 voltage and an `exact`
  percentage from another live source), battbar records (firmware %, mV) pairs, at most
  9 recent voltages per percentage (samples within 10 mV of the previous one at the
  same percentage are repeats). Not while charging, on external power or settling.
- A percentage is used once it has 3 samples; its voltage is the median. Points that
  would make the curve non-monotonic are skipped. Because older samples drop out, a
  plateau level (firmware 100 % for a long time) drifts towards the voltage where the
  firmware leaves it, and levels follow battery ageing.
- On a transport with voltage only, the percentage comes from linear interpolation
  between the unit's points; outside them the generic curve is scaled to join the
  nearest point (towards 100 % above, towards 0 % below). The value stays
  `approximate`; the menu says it is calibrated against the device's Bluetooth readings.
- When the exact source is live, it still wins (precision rule); calibration never
  changes a value that did not come from the voltage.
- Stored in the settings by device ID (keyed hash), never in `dump` or traces. The
  device menu shows the learned state and offers Reset. The CLI applies, never learns.
- Limitation: the receiver link may draw more current than Bluetooth, so the voltage
  under load can be slightly lower there; no correction is applied.

### General

- Per-request timeouts (1–2 s); one hung device never blocks the others.
- Survive unplugging/replugging receivers and bases, BT connect/disconnect, and a
  device switching transports (hotplug).

## Privacy and security (critical)

The keyboard receiver physically carries keystrokes. battbar does **not** need Input
Monitoring: verified in Stage 2 with the signed `.app` — it reads the receiver's and the
base station's vendor-only interfaces while Input Monitoring was never requested or
granted (`IOHIDCheckAccess` = unknown). The app never asks for it; `battbar dump`
reports its status for bug reports.

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
- **Network: only the opt-in update check** (0.3.0), in `battbar-app/src/updates.rs`:
  - Settings → "Check for Updates", **off by default**; offered (unchecked) in the
    first-run dialog next to Launch at Login, or once on its own for installs that
    answered that question before 0.3.0.
  - When on: at most once per 24 h (1 h after a failure),
    `GET https://api.github.com/repos/StanislavTugarev/battbar/releases/latest` via
    `/usr/bin/curl -q` (no `~/.curlrc`), HTTPS only, no redirects, 20 s timeout, empty
    environment. Headers: `Host`, `User-Agent: battbar/<version>`,
    `Accept: application/vnd.github+json` — no token, cookies or identifiers. GitHub
    sees the IP address and the User-Agent.
  - Only `tag_name`, `prerelease` and `draft` are read; drafts and pre-releases are
    ignored. `tag_name` must match `^v\d+\.\d+\.\d+$` exactly (ASCII digits, at most
    32 bytes, no whitespace or other characters); anything else fails the check and
    nothing is shown. Compared by semver with the running **full** version (with
    `-rc.N` for a release candidate build, `battbar_core::version::FULL`), so a
    candidate sees its final release as an update (`0.4.0-rc.2 < 0.4.0`). The release page URL
    is built from the version, never taken from the response.
  - Newer version → menu item "Update available: vX.Y.Z…": Homebrew install (a
    `Caskroom/battbar` directory under `$HOMEBREW_PREFIX` if set, `/opt/homebrew` or
    `/usr/local`; `brew` is never run) → dialog with `brew upgrade --cask battbar`
    (Copy Command / Release Notes); otherwise → open the release page (DMG). One
    notification per version.
  - Off → no request at all, and a result that arrives after turning it off is
    dropped. Tests: no request while off (counting fetcher), once per day, notification
    once per version; static test: no other code uses curl, URL loading APIs or
    sockets, and no crate depends on an HTTP client.
  - Development builds only: `BATTBAR_DEV_FAKE_VERSION` replaces the running version
    (to try "Update available"); behind `cfg(debug_assertions)`, and CI checks that
    release binaries do not contain it.

## Stages

### Stage 0 — Discovery on reference hardware ✔

See [`DISCOVERY.md`](DISCOVERY.md).

### Stage 1 — Core + CLI ✔

- Workspace, traits, transports with record/replay, source merging, device database,
  all providers.
- `battbar probe [--json]` — one-shot poll of all devices (with transport and data source).
- `battbar dump` — diagnostic report for issues (VID/PID, transport, interfaces, features,
  version; no user data).
- `battbar record --out trace.jsonl` — record traffic with vendor interfaces / GATT.
- Provider tests on recorded traces of the reference hardware; merging tests.

### Stage 2 — Menu bar app ✔

- One status item per device (until 0.3; see "Menu bar display (0.4)"); icon by type (mouse/keyboard/headset) + percentage;
  charging indicator; dimmed icon without percentage for asleep/disconnected;
  `~` before the percentage for `coarse`/`approximate`.
- Click menu: name, transport, status, components, last update, "Refresh",
  "Settings", "Quit".
- Settings: hide device, merge/split devices, low-battery threshold, poll interval,
  headsetcontrol fallback toggle, show BT devices without battery, launch at login
  (SMAppService).
- Poll every 60 s + push events where available.
- macOS notification on low battery (default 15 %, once per discharge cycle; rules in
  "Battery values while charging").
- Missing Bluetooth permission: clear screen with a button opening the right System
  Settings pane (Input Monitoring is not needed, see "Privacy and security").
- Menus: every device menu has the device's details and only actions on that device
  (Hide from Menu Bar / Merge With / Split / Unlink), plus the common entries
  Refresh, **Settings** (one global section: launch at login, update interval,
  low-battery threshold, spare-battery notification, headsetcontrol, Bluetooth devices
  without battery, permission panes), **Other Devices** and Quit.
- The app never has zero status items: with no visible device it shows one neutral
  app icon whose menu has Settings, Other Devices and Quit.
- Disconnected Bluetooth devices are hidden by default until they have reported a
  battery at least once (`ever_reported` in the settings); hidden devices are listed
  under Other Devices with "Show in Menu Bar".
- A new device gets its icon only after a complete poll (every provider answered) at
  least a few seconds after it first appeared, or after a timeout — so a transient
  half-merged state (BLE Battery Service before the GATT HID++ link) never shows.
- Identity memory (which identity keys belong to one device, e.g. CoreBluetooth
  peripheral ↔ HID++ unit ID) is stored in the settings as hashes only. Device IDs in
  the settings are hashes as well.

### Hotplug (0.5)

- **HID**: an IOHIDManager on the HID service thread's run loop matches every HID
  device and opens none (no Input Monitoring); its matching/removal callbacks tell the
  worker "a HID device appeared or disappeared" — receivers and base stations on USB,
  and Bluetooth LE / classic HID devices, which macOS represents as HID devices too.
- **Bluetooth**: IOBluetooth connect notifications (and a disconnect notification per
  connected device), registered on the main thread once Bluetooth permission is
  granted — for classic audio devices that are not HID (headsets).
- **Receivers**: the reference Lightspeed receiver sends 0x41 when a device
  disconnects, but not reliably when one connects (seen: a mouse reaching a
  just-plugged receiver sent neither 0x41 nor any frame). So:
  - the 2 s event check **pings every slot that was disconnected** at the last poll
    (right after a receiver is plugged in: every slot the first poll found absent).
    For an absent device the receiver answers "unknown device" itself, without radio;
    any other answer (or none within 0.2 s) asks for a poll;
  - **asleep** slots are not pinged: the first HID++ frame of a device that was not on
    line (asleep, disconnected, not polled yet) asks for a poll — a mouse waking up;
  - each slot asks once per connection or wake-up (until a poll sees it on line), so
    a device that answers but still looks absent cannot cause a poll loop; frames of a
    device on line ask for nothing.
  battbar enables the receiver's connection notifications (register 0x00) as before;
  register writes stay off in diagnostics (`dump`, `record`).
- **Bluetooth LE**: the system decides what is connected. A device that is not in
  `retrieveConnectedPeripherals` is not shown with a value from any source (Battery
  Service, HID++ over GATT). battbar never keeps a connection of its
  own: GATT sessions are counted per peripheral and the last one to close cancels the
  app's connection (also after a failed attempt); peripherals are only taken from the
  system's connected list.
- **SteelSeries base stations**: the 2 s event check asks each open base for its status
  and polls when the headset connected/disconnected, moved to/from Bluetooth or
  started/stopped charging.
- The worker polls **0.3 s** after such a notification (a burst gives one poll); the
  poll log names the reason (`interval`, `hid`, `bluetooth`, `device event`,
  `follow-up`, `refresh`).
- Bluetooth LE devices that are not HID (only a Battery Service) appear at the next
  poll.
- **Safety re-scans** every 5 min (HID links, base stations, Bluetooth connections),
  counted from the last poll, catch a missed notification.
- Periodic waits use timers with leeway (worker: GCD timer, 15 %).
- Measurements before/after: DISCOVERY "Hotplug and wake-ups (0.5)".

### Menu bar display (0.4)

- **One status item** (`NSVariableStatusItemLength`, autosave name `battbar`, so a
  ⌘-dragged position is kept). Its image is drawn by the app: every segment (SF Symbol
  by type + percentage text) side by side in one **template** image, drawn on demand
  at the screen's scale, so macOS colours it for light/dark menu bars and the open
  state. Dimmed segments (asleep, disconnected, no value) use alpha 0.35. With no
  segment the item shows battbar's glyph.
- **Order**: by type (keyboard, mouse, headset/headphones, others), then name. *Move
  Left / Right* (collapsed: *Move Up / Down*) in a device's submenu stores a custom
  order (`device_order`); devices not in it follow in the default order; Settings →
  Menu Bar → *Reset Device Order* clears it.
- **Menu**: one menu. Per visible device a section: header (icon + name) whose
  submenu holds the device's actions (Move, Hide, Merge With, Split, Unlink,
  Batteries, Voltage Calibration); detail lines (battery, components, charging,
  connection); *Sources*. Then status lines (*Updated HH:MM* — one poll covers all
  devices, so one time —, providers without answer, permission and translocation
  hints), *Expand / Collapse in Menu Bar*, Refresh, Settings, Other Devices, *Update
  available* (when found), Quit.
- **Modes** (`display_mode`): *Collapsed* (default for new installations) and
  *Expanded*. On the first start of 0.4 an installation that already has a settings
  file gets *Expanded* (its menu bar looks as in 0.3); a new one *Collapsed*
  (`Settings::init_display_mode`, tested).
- **Collapsed** item: battbar's glyph while no visible device is at or below the
  low-battery threshold (same rule as the alert: `percent ≤ threshold`, threshold
  off = never); otherwise the lowest device's icon and percentage. Always
  monochrome (template image), no colours — maintainer's decision 2026-11-21 (a red
  variant was tried; note: the macOS 26 menu bar ignores `contentTintColor` on
  template images, colour would need a non-template image). Alternative considered:
  always the lowest device's percentage (`CollapsedStyle::Lowest`).
- **Migration from 0.3**: the old per-device items (`NSStatusItem … battbar_<id>` in
  the user defaults) are removed at start.
- **Not enough space** (no automatic shrinking or growing): while Expanded, the app
  watches whether macOS still shows the item — **event-driven, no polling**: it
  subscribes to `NSWindowDidChangeOcclusionStateNotification` of the item's window
  (again if AppKit replaces the window) and to
  `NSApplicationDidChangeScreenParametersNotification`, and keeps at most one one-shot
  timer (hidden → timer for the 1 s below; shown again → cancelled; shown and settled
  → no timer at all). If the visible bit of the
  item window's occlusion state stays clear for **1 s** — counted only from **1 s
  after** the island's content, the screen configuration or the mode last changed
  (macOS reports a new or resized item hidden until it is placed, ~0.5 s) — battbar
  switches the **setting** to Collapsed and posts a notification, every time: "battbar
  collapsed — Not enough space in the menu bar to show all devices. Free up space and
  choose Expand in the battbar menu to try again." It returns to Expanded only when
  the user chooses Expand (menu or Settings); nothing expands by itself and screen
  configurations are not tracked. Until an Expand fits, the menu shows "Not enough
  space last time" under *Expand in Menu Bar* (`collapsed_for_space`; for turned-off
  notifications). Logic in `overflow.rs`, tested (collapse after 1 s hidden, initial
  placement ignored, no self-expansion, repeated Expand → new collapse and
  notification).
- **Discovery** (2026-11-21, `battbar-app --probe-menu-bar`, Mac mini with two
  external displays, macOS 26): items of 200 and 400 pt were always shown; an item of
  1200 pt was shown with short app menus (Terminal, Finder, Claude) and hidden with
  Chrome in front. Hidden ⇔ occlusion visible bit clear, and at the same time the
  window frame left the screen's menu bar (`inside_screen=false`); both changed
  within 0.5 s, no false signals. A new item reports hidden (`screen=nil`, frame
  `(0,-30)`) for ~0.5 s before placement. Verified in the app with a padded
  development item (`BATTBAR_DEV_PAD_WIDTH`): hidden at t, collapsed at t + 2 s,
  notification posted, note in the menu; restart stays collapsed. **Displays with a
  notch: not measured** (no MacBook); the same occlusion signal is expected, since
  macOS hides an item behind the notch like one without room.
- **Occlusion notifications** (2026-11-21, probe with notifications next to 0.5 s
  polling): every change of the visible bit seen by polling came with a notification
  at the same time or earlier (≤ 0.1 s), also when the item grew past the available
  room (`--grow`); moving without a visibility change sends none. Which item fits
  depends on the display whose menu bar is active: with Finder/Chrome on the 2560 pt
  display a 1640 pt island fit, with Claude on the 1920 pt display it was hidden. That
  explains the one "slow" fallback seen while testing (23 s): the island fit on the
  wide display and was hidden only when the active app moved to the narrow one — not
  a detection delay. App verified: Finder in front → shown, two settle checks only;
  Claude in front → notification, collapse 1 s later, notification posted.
- Clicking a segment to open the menu at that device's section: in the backlog — a
  status item with a menu cannot see where it was clicked, and AppKit has no public
  API to open a menu with a given item highlighted.

### Stage 3 — Open-source readiness ✔ (publishing pending)

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
- Release signing: a separate long-lived self-signed "battbar Release" certificate,
  kept in GitHub Actions secrets (imported into a temporary keychain) and in an offline
  backup. Release/universal builds only in CI.
- CI checks that traces in the repository contain only pseudonymized unit IDs (prefix
  `bba7`) and no Bluetooth addresses, and that no certificates or keys are tracked.
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

## Backlog (not scheduled)

- **Kind of Bluetooth LE devices with only a Battery Service.** Check whether
  CoreBluetooth exposes the Appearance characteristic (0x2A01, Generic Access 0x1800)
  of a connected peripheral; if so, derive the kind from it. If macOS hides it (it
  usually keeps Generic Access to itself), keep "other" (since 0.5 there is no
  `system_profiler` lookup).

- **Device events without the 2 s check.** Hotplug is event-driven since 0.5; the
  worker still asks the providers every 2 s for buffered HID++ notifications (a device
  switched on/off behind a receiver, charging events). Event-driven instead: a blocking
  reader per HID++ link that wakes the worker.
- **Compact step.** Between Expanded and Collapsed: icons only, a percentage only at
  or below the low-battery threshold — for menu bars where the full island does not
  fit but icons would. Needs a rule for growing back without a loop (items hidden by
  one app's menu, shown with another's); 0.4 collapses straight to Collapsed instead.
- **Click a segment → its section.** Open the menu with the clicked device's section
  highlighted. Needs the item without an attached menu (to get the click position) and
  a way to highlight an item that AppKit does not offer publicly; possible fallback:
  open a menu with only that device's section.
- **Automatic updates (Sparkle).** Download and install updates in the app instead of
  only announcing them. Notes for when it is scheduled:
  - the cask must then declare `auto_updates true` (so `brew upgrade` leaves the app to
    Sparkle and `brew outdated` does not report it);
  - a separate **EdDSA (ed25519) signing key** for the appcast, stored exactly like the
    "battbar Release" certificate: only in GitHub secrets and an offline backup, never
    in the repository, used only in CI;
  - the appcast request replaces the GitHub API check and stays opt-in, with the same
    privacy description.

- ~~Development builds use their own configuration directory~~ — done in 0.2.0
  (`~/.config/battbar-dev`); own HID++ software ID for development builds — done in
  0.3.0.

- **0.2.0: battery chemistry and non-rechargeable batteries** — implemented in 0.2.0
  as listed, except item 7 (HID++ 1.0: "help wanted" issue first). Alkaline/NiMH
  curves are per cell, from published datasheets, always `approximate`, marked
  experimental. The 0x1004 "rechargeable" bit is used only for alert wording. (Original
  requirements; not
  implemented in 0.1.0, which only guards: 0x1001 below ~3.3 V → "not reported", no
  alerts):
  1. 0x1004 capabilities: before treating any bit as "rechargeable", confirm the parsing
     of getCapabilities — byte0 = supported-level mask (critical/low/good/full, 4 bits),
     byte1 = flags (DISCOVERY "0x1004 getCapabilities"). Solaar does not read
     capabilities, so it cannot serve as the reference.
  2. Devices with 0x1004: the firmware accounts for the chemistry — the percentage is
     used as reported; battbar's curves are not applied.
  3. `devices/*.toml`: field `chemistry = "li-ion" | "alkaline" | "nimh"`; discharge
     curves per chemistry. For unknown battery-powered models the Li-ion curve is never
     applied.
  4. Battery-powered 0x1001 devices: a per-device menu setting Alkaline / NiMH (default
     alkaline) — the two cannot be told apart by voltage.
  5. 0x1000 with discrete levels → `precision = coarse`.
  6. Alerts for battery-powered devices say "replace batteries", not "charge"; the
     charging/settling logic does not apply to them.
  7. HID++ 1.0: estimate the scope of battery support (receiver/device battery
     registers) separately and present it before implementing.

## Process

- Commit at the end of every stage and before large refactorings.

## Revision history

- **2026-11-21** — 0.4.0: one status item (expanded / collapsed), one sectioned menu,
  device order, 0.3 migration, monochrome collapsed item (no red); discovery with
  `--probe-menu-bar` (occlusion signal; notch not measured); instead of an automatic
  Expanded → Compact → Collapsed state machine a simple fallback: hidden 1 s →
  setting Collapsed + notification, Expand only by the user; Compact step and segment
  click → backlog.
- **2026-11-20** — 0.3.0-rc.2: G915 BLE Battery Service flagged unreliable (database
  flag, never calibrated from); device IDs of linked/merged devices from the whole
  group; classic Bluetooth battery read in a helper process (IOBluetooth gives a
  process the values only in its first seconds); development builds use HID++
  software ID `0x0A`.
- **2026-11-19** — 0.3.0: per-unit voltage calibration for 0x1001 devices
  from simultaneous Bluetooth firmware percentages (moved from the backlog); opt-in
  daily update check (the only network request; replaces "no network requests");
  Sparkle added to the backlog.

- **2026-11-19** — 0.2.0: Launch at Login (asked once, status from SMAppService, never
  re-enabled automatically, postponed under App Translocation, approval/not-found help,
  cask removes the login item); chemistry (`chemistry`/`cells` in TOML, per-cell
  alkaline/NiMH curves, Batteries menu, "replace the batteries" alerts); separate dev
  configuration directory.

- **2026-11-19** — before publishing: identifiers stored in the settings use HMAC-SHA256
  with a random per-installation key (`salt`, settings file mode 0600; never in `dump`);
  development bundles use bundle ID `io.github.battbar.dev`; pre-release tags
  `vX.Y.Z-rc.N`; Homebrew tap updated via an SSH deploy key; CI checks every commit
  for signing material and non-pseudonymized unit IDs in traces (frame-level parsing).

- **2026-11-19** — Charging: values approximate while charging and for ~10 min after;
  no low-battery alerts then; alert hysteresis threshold + 10 %; calibration excludes
  the settling period.

- **2026-11-19** — Stage 2 review: never-reported disconnected Bluetooth devices hidden
  by default (Other Devices menu); global Settings separate from per-device actions;
  neutral icon when nothing is visible; persistent hashed identity memory; new-device
  gate; Stage 3 signing with a separate long-lived "battbar Release" certificate.

- **2026-11-19** — Stage 2: menu bar app; Input Monitoring verified as not needed;
  session identity memory (BLE peripheral ↔ HID++ unit ID) against transient splits;
  spare battery seen at 8/8 after charging.

- **2026-11-19** — before Stage 2: one merge rule (best precision, then priority,
  flags combined) replaces per-transport source order; stable self-signed signing;
  learned headset link with strict conditions and Unlink; `gamepad` kind from Class of
  Device; backlog: per-unit voltage curves.

- **2026-11-19** — Stage 1: base byte 5 confirmed as "headset on Bluetooth"; spare
  battery byte 7 implemented as component; classic audio precision `exact`; CLI
  relaunches itself as responsible process for Bluetooth; trace unit IDs pseudonymized
  by `battbar record`.

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
