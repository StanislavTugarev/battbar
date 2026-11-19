# Stage 0 — Hardware discovery

Status: **done, awaiting review.** All reference devices captured on both transports.
Decisions taken on the basis of this document are listed in [Decisions](#decisions)
and folded into [`SPEC.md`](SPEC.md).

Raw captures (identifiers redacted / fingerprinted):

| File | What |
|---|---|
| [`discovery/usb-baseline.txt`](discovery/usb-baseline.txt) | HID enumeration, HID++ probe via receiver, SteelSeries probe |
| [`discovery/bt-none-connected.txt`](discovery/bt-none-connected.txt) | Bluetooth snapshot, no BT peripheral connected |
| [`discovery/bt-g915.txt`](discovery/bt-g915.txt) | Bluetooth snapshot with the keyboard on Bluetooth |
| [`discovery/receiver-while-g915-on-bt.txt`](discovery/receiver-while-g915-on-bt.txt) | Receiver view of the keyboard slot while the keyboard is on Bluetooth |
| [`discovery/bt-g915-iobluetooth.txt`](discovery/bt-g915-iobluetooth.txt) | IOBluetooth `pairedDevices` + private battery properties |
| [`discovery/bt-g915-corebluetooth.txt`](discovery/bt-g915-corebluetooth.txt) | CoreBluetooth: GATT services visible to an app, Battery Service read |
| [`discovery/bt-g915-gatt-hidpp.txt`](discovery/bt-g915-gatt-hidpp.txt) | HID++ 2.0 over the vendor GATT characteristic, full request/response log |
| [`discovery/bt-headset.txt`](discovery/bt-headset.txt) | Bluetooth snapshot with the headset also on Bluetooth |
| [`discovery/bt-headset-iobluetooth.txt`](discovery/bt-headset-iobluetooth.txt) | IOBluetooth battery properties with the headset on Bluetooth |
| [`discovery/base-with-headset-on-bt.txt`](discovery/base-with-headset-on-bt.txt) | Base station reply while the headset is also on Bluetooth |

Tools (read-only, not shipped):

- `cargo run -p battbar-discovery -- enum | hidpp | steelseries | all [--allow-mixed] [--no-redact]`
- `tools/discovery/bt-snapshot.sh > docs/discovery/bt-<label>.txt`
- `tools/discovery/swift/{iobt-battery,cb-battery,gatt-hidpp}.swift` — run as a small
  ad-hoc-signed `.app` (see [Bluetooth permission](#bluetooth-permission-tcc)).

Identifiers such as unit IDs and serial numbers are replaced by `<fp:xxxxxx>`
(24-bit FNV-1a fingerprint) so captures can be compared across transports without
publishing them.

## Summary

| Device | Transport | Battery source | Value | Reference | Precision |
|---|---|---|---|---|---|
| Mouse (G502 X PLUS) | Lightspeed, idx 2 | HID++ 0x1004 | 11 %, discharging | G HUB 11 % | exact |
| Keyboard (G915) | Lightspeed, idx 1 | HID++ 0x1001 | 4200 mV, ext. power, "full" | G HUB 100 % | approximate (curve) |
| Keyboard (G915) | BLE, HID++ over vendor GATT | HID++ 0x1001 | 4158 mV, discharging | G HUB 100 % | approximate (curve) |
| Keyboard (G915) | BLE, system | GATT Battery Service 0x180F / `system_profiler` | 100 % | G HUB 100 % | exact? (see below) |
| Headset (Arctis Nova Pro) | base station USB | vendor report `06 b0` | 8/8 → 100 %, online | base display ≈ 85–100 % (no exact figure) | coarse |
| Headset (Arctis Nova Pro) | classic BT (HFP) | IOBluetooth `batteryPercentSingle` | 85 % | base display ≈ 85–100 % | coarse? (step not yet known) |

Base (100 %, bucket 8/8 = 87.5–100 %) and Bluetooth (85 %) agree within the ±10 %
acceptance band for `coarse`.

## Environment

| | |
|---|---|
| Host | Apple Silicon (arm64), macOS 26.1 (25B78) |
| Bluetooth controller | BCM_4388C2, fw 23.3.214.1296 |
| Vendor software running | G HUB 2026.7 (`lghub_agent`), SteelSeries GG |
| Toolchain | Rust 1.98.0, `hidapi` crate 2.6.7 (IOHIDManager backend), Swift (Xcode toolchain) |

All probes ran **while G HUB and GG were running**, with a non-exclusive open and HID++
software ID `0x0B`. No cross-talk was observed: zero foreign HID++ replies arrived,
nothing had to be filtered.

## 1. HID devices

Every row is one OS-level `IOHIDDevice` (one hidapi path). On macOS each USB *interface*
is a separate `IOHIDDevice`; hidapi lists one entry per top-level collection of it.

| VID:PID | Product | Bus / iface | Top-level collections | Verdict |
|---|---|---|---|---|
| 046d:c547 | USB Receiver (Lightspeed), rel 0x0402 | USB 0 | 0x01/0x02 mouse, 0x01/0x01 pointer | input — never opened |
| 046d:c547 | USB Receiver | USB 1 | 0x01/0x06 keyboard, 0x0C/0x01 consumer, 0x01/0x80 system control | input — never opened |
| 046d:c547 | USB Receiver | USB **2** | **0xFF00/0x0001 (short HID++), 0xFF00/0x0002 (long HID++)** | vendor-only → HID++ |
| 1038:12e0 | Arctis Nova Pro Wireless (base station), rel 0x0130 | USB 3 | 0x0C/0x01 consumer | input — never opened |
| 1038:12e0 | Arctis Nova Pro Wireless | USB **4** | **0xFFC0/0x0001, 0xFF00/0x0001** | vendor-only → status reports |
| 046d:0afc | Yeti GX (USB microphone) | USB 2 | 0xFF43/0x0701, 0xFF43/0x0702, 0xFF43/0x0704 | vendor-only, HID++ — no battery |
| 046d:**b354** | G915 KEYBOARD, rel 0x0042 | **BLE**, iface −1 | 0x01/0x06 keyboard, 0x01/0x02 mouse, 0x01/0x01, 0x0C/0x01, **0xFF43/0x0202 (long HID++ only)** | **MIXED** — refused |

Findings:

- On USB, the vendor interface of the receiver and of the base station is a **separate
  `IOHIDDevice` with only vendor collections**; opening it cannot deliver keyboard or
  mouse reports. The privacy rule is enforceable per `IOHIDDevice`: refuse a path if
  *any* of its collections has a non-vendor usage page.
- On Bluetooth (BLE/HOGP) the keyboard is **one `IOHIDDevice` containing both the
  keyboard collections and the HID++ collection** (0xFF43/0x0202, long reports only).
  HID++ via hidapi on Bluetooth is therefore impossible without opening a device that
  produces keystrokes. The discovery tool refused it; it was not opened. See §4.3 for the
  alternative that works.
- HID++ is not only on usage page 0xFF00. Logitech also uses **0xFF43** with usages
  `0x__01` (short), `0x__02` (long), `0x__04` (very long). The Yeti GX is a wired HID++ 4.2
  device on 0xFF43 with no battery features: discovery must accept 0xFF43 and the
  provider must silently skip devices without battery features.
- `hidutil list` shows only the *primary* usage of each device; hidapi's per-collection
  view is needed to see the short/long pair.
- hidapi on macOS **opens devices exclusively by default** (`kIOHIDOptionsTypeSeizeDevice`).
  `HidApi::set_open_exclusive(false)` must be called before any open; this belongs in the
  real `HidTransport` and needs a test.

## 2. Logitech Lightspeed receiver (046d:c547)

Receiver itself (device index 0xFF) speaks **HID++ 1.0** (ping → error 0x01 invalid
sub-ID). Register reads (read-only):

| Register | Value | Meaning |
|---|---|---|
| short 0x00 notification flags | `00 09 00` | wireless + software-present notifications already enabled (by G HUB) |
| short 0x02 connection state | `00 02 00` | 2 — stays 2 when the keyboard moves to Bluetooth, so this is *paired*, not *online* |
| short 0xF1/01, 0xF1/02 | `04 02`, build `0009` | receiver firmware 04.02 b0009 |
| long 0xB5/0x20+n pairing info | slot 1: wpid **0x407C**, slot 2: wpid **0x4099** | Unifying-style pairing layout works on this Lightspeed receiver |
| long 0xB5/0x40+n codename | slot 1 `G915 KEYBOARD`, slot 2 `G502 X PLUS` | |
| slots 3–6 | HID++ 1.0 error **0x09** | empty pairing slot → *ResourceError* |

Ping results per device index:

| Situation | Ping reply |
|---|---|
| device online | HID++ 4.2 |
| empty slot (3–6) | HID++ 1.0 error **0x08** UnknownDevice |
| **keyboard paired in slot 1 but switched to Bluetooth** | HID++ 1.0 error **0x08** UnknownDevice (not 0x09) |

Consequences for the provider:

- "Paired but away" is **not** distinguishable from "empty" by ping. The provider must
  read the pairing registers (0xB5/0x20+n) to know which slots are paired, and map
  *paired + 0x08* → `disconnected`.
- 0x09 means different things per request (empty pairing slot vs. unreachable device
  on ping), so error mapping must be per request, never global.

### Device index 0x01 — keyboard (G915)

- HID++ **4.2**, 37 features + root. Battery: **only 0x1001 Battery Voltage v3**
  (no 0x1004, no 0x1000) → percent needs a discharge curve in the device database.
- Reading: **4200 mV, flags 0x81** = external power, charge status "full" (cable plugged).
- 0x0003 Device Information v4: `transports = 0x000E` (BLE | eQuad/Lightspeed | USB),
  model IDs **B354 / 407C / C33E** (BLE PID / Lightspeed wpid / wired USB PID),
  capabilities 0x00 → **no serial number**. Unit ID `<fp:feef5e>`.
- 0x0005 name `G915 WIRELESS RGB MECHANICAL GAMING KEYBOARD`, type 0 (keyboard).
- Has 0x1D4B Wireless Device Status, 0x1814/0x1815 (multi-host).

### Device index 0x02 — mouse (G502 X PLUS)

- HID++ **4.2**, 32 features + root. Battery: **0x1004 Unified Battery v5**.
- Capabilities: levels `0b1111`, flags `0x0F` (state of charge supported → `exact`).
- Reading: **11 %**, level critical, discharging. **G HUB: 11 %** ✔.
- 0x0003 v4: transports eQuad | USB, model IDs **4099 / C095**, serial supported.

## 3. SteelSeries base station (1038:12e0)

- Vendor interface (USB iface 4) opened non-exclusively while GG was running.
- Request `06 b0` (2-byte write; no padding needed) → one 64-byte reply:

  ```
  06 b0 00 00 01 00 08 04 08 00 02 0a 00 00 08 08 00 …
  ```

  | Byte | Value | Interpretation (HeadsetControl) |
  |---|---|---|
  | 6 | `08` | headset battery, scale 0–8 → **100 %** (coarse: 12.5 % steps) |
  | 15 | `08` | headset state: `0x01` offline, `0x02` cable charging, `0x08` online |
  | 7, 8, 10, 11, 14 | `04 08 02 0a 08` | **unknown** — candidates: spare battery in the base slot, ANC mode, volume, mic state |

- Byte 6 has only 9 levels → readings from the base are `precision = coarse`.
- Second capture ~35 min later, with the headset **also connected over Bluetooth**:

  ```
  14:0x  06 b0 00 00 04 01 08 06 08 00 02 0a 00 00 08 08 00 …
  13:2x  06 b0 00 00 01 00 08 04 08 00 02 0a 00 00 08 08 00 …
                    ^^ ^^    ^^
  ```

  | Byte | Before → after | Hypothesis |
  |---|---|---|
  | 4 | `01` → `04` | headset Bluetooth state (1 = off/idle, 4 = connected?) |
  | 5 | `00` → `01` | headset Bluetooth connected flag |
  | 7 | `04` → `06` | **spare battery charging in the base slot**, 0–8 scale (4/8 → 6/8 in 35 min) |

  These are hypotheses from two samples. The base display shows only a rough level, so
  byte 7 will be confirmed in Stage 1 by recording over a full charge of the spare
  battery (it should stop at 8). If confirmed, the device gets two components
  (`headset`, `spare battery`), and bytes 4/5 can tell the merger that the base-linked
  headset is the one on Bluetooth.

## 4. Bluetooth

### 4.1 What macOS reports

With the keyboard on Bluetooth (`bt-g915.txt`):

| Source | Keyboard battery |
|---|---|
| `system_profiler SPBluetoothDataType -json` | `device_batteryLevelMain: "100%"`, PID 0xB354, services `BLE`, fw `MPK09.04_0042` |
| IORegistry (`BatteryPercent*` anywhere) | **nothing** |
| IOBluetooth `pairedDevices()` | device listed, `isConnected = true`, but private `batteryPercentSingle/…` are **0** |
| CoreBluetooth, GATT Battery Service 0x180F / 0x2A19 | **100 %**, characteristic is read + notify |

So for BLE peripherals on macOS 26 the planned IOBluetooth/IORegistry path does **not**
carry the battery; the working public sources are **CoreBluetooth Battery Service**
(primary) and `system_profiler` (fallback). IOBluetooth `batteryPercent*` is still the
likely source for *classic* audio devices (HFP) — to be checked with the headset.

With nothing connected (`bt-none-connected.txt`) no source reports any battery value
→ `disconnected` without a percentage, as planned.

### 4.2 Bluetooth permission (TCC)

- A CLI calling IOBluetooth/CoreBluetooth is **killed by TCC** ("attempted to access
  privacy-sensitive data without a usage description"), even with an embedded
  Info.plist, because TCC attributes the access to the *responsible* process (the
  terminal host app).
- Packaged as a minimal ad-hoc-signed `.app` with `NSBluetoothAlwaysUsageDescription`
  and launched via `open`, it works (`authorization = allowedAlways`).
- Consequence: `battbar probe` (CLI) can do HID, but its Bluetooth part needs either
  the `.app` bundle (CLI inside the bundle, or a helper the CLI talks to) or a fallback
  to `system_profiler`. To decide in Stage 1.

### 4.3 HID++ over the vendor GATT service (no HID device opened)

Visible to an app via CoreBluetooth on the keyboard:

| Service | Characteristics |
|---|---|
| 0x180A Device Information | 2A29, 2A24, 2A25, 2A27, 2A26, 2A28, 2A50 (read) |
| 0x180F Battery | 2A19 (read, notify) |
| **`00010000-0000-1000-8000-011F2000046D`** (Logitech) | **`00010001-0000-1000-8000-011F2000046D`** (read, write w/o response, write, notify) |
| 0x1812 HID | *hidden from apps by macOS* |

HID++ 2.0 works over that characteristic (`bt-g915-gatt-hidpp.txt`):

- **Framing: 18 bytes, no report ID, no device index**: `[feature index, function<<4 | sw id, 16 parameter bytes]`.
  Write *with response*; the reply comes as a notification in the same layout.
  19- and 20-byte frames are rejected with ATT error 13 (invalid length).
- Ping → **HID++ 4.2**. 32 features + root (same user-facing features as over the
  receiver; a few engineering/DFU features differ).
- 0x0003 Device Information: **unit ID `<fp:feef5e>` — identical to the receiver
  transport**, same model IDs B354/407C/C33E. Cross-transport identity confirmed.
- 0x1001: **4158 mV, flags 0x00** (on battery, discharging). G HUB: 100 %.
- The keyboard `IOHIDDevice` is never opened on this path, so **keystrokes cannot be
  received**, and Input Monitoring is not involved for Bluetooth at all.

### 4.4 Precision of the Bluetooth battery value

Both BLE Battery Service and 0x1001 → curve report "about full" right now (100 % vs
4158 mV). Whether the Battery Service value is the keyboard's own voltage-based estimate
in 1 % steps or coarser has to be observed while it discharges; until then it is
`precision = exact` per the GATT spec, with a note to verify.

### 4.5 Headset over classic Bluetooth

With the headset paired to this Mac and connected over Bluetooth while still linked to
the base (`bt-headset.txt`, `bt-headset-iobluetooth.txt`):

| Source | Headset |
|---|---|
| `system_profiler` | connected, minor type Headphones, services `HFP AVRCP A2DP ACL`, **no battery field**, vendor **0x000A**, product **0xFFFF**, firmware `255.15.15` (no Device ID record) |
| IORegistry `BatteryPercent*` | nothing |
| IOBluetooth `batteryPercentSingle` | **85** (other `batteryPercent*` = 0) |
| CoreBluetooth | not visible (classic BR/EDR, no GATT) |

Findings:

- For classic Bluetooth audio, **IOBluetooth `batteryPercentSingle`** is the only
  source; `system_profiler` does not even carry it, so it cannot be the fallback for
  this class. `batteryPercentSingle` is a private property read through KVC; it must be
  guarded (`respondsToSelector`) and the provider must degrade to "battery not
  reported" if it disappears in a future macOS.
- The value 85 is not a multiple of 10, so it is not (only) `AT+IPHONEACCEV` in 10 %
  steps. The real step size will be measured in Stage 1 by recording the value over a
  discharge; until then `precision = coarse`.
- The headset reports no VID/PID over Bluetooth. Identity across base and Bluetooth can
  only come from the name (`Arctis Nova Pro Wireless`, same as the USB product string)
  plus the base's "headset on Bluetooth" bytes (§3), with manual merge/split as backup.

## Remaining measurements (Stage 1, not blocking)

1. Step size of `batteryPercentSingle` for the headset over a discharge.
2. Base byte 7 over a full charge of the spare battery.
3. 0x1001 voltage → percent curve for the keyboard, **off cable only**, against G HUB.
4. Precision of the keyboard's BLE Battery Service value while it discharges.
5. Input Monitoring: whether a bundled `.app` that opens only vendor-only USB
   interfaces is asked for it at all (vendor interfaces opened from the shell without
   a prompt). Checked in Stage 2.

## Decisions

Taken on 2026-11-19 after review of this document; reflected in [`SPEC.md`](SPEC.md).

1. **HID++ over Bluetooth via the Logitech GATT service** (CoreBluetooth), not `hidapi`.
   - Connect only via `retrieveConnectedPeripherals(withServices:)`; no
     `scanForPeripherals`, ever.
   - Implemented as another transport (frame without report ID / device index handled
     inside the transport); HID++ provider and feature logic are not duplicated.
   - Software-ID filtering is mandatory on GATT too (G HUB / Options+ may see the
     notifications).
   - Privacy tests extended: no `IOHIDDevice` is ever opened on Bluetooth.
2. **Bluetooth source order**: Logitech — HID++ via GATT → BLE Battery Service →
   `system_profiler`. Other BLE — Battery Service → `system_profiler`. Classic
   Bluetooth audio — open after the headset test (§4.5: IOBluetooth
   `batteryPercentSingle` is the only source found).
3. **CLI inside the `.app` bundle** (`Contents/MacOS`), symlinked into `PATH` by the cask.
4. **Receiver**: read pairing registers to tell an empty slot from "paired but not
   here". If the same unit ID is visible on another transport, merge; no separate UI
   state. Enable receiver notification flags.
5. **0x1001 curve calibration only from off-cable measurements**; readings under charge
   are marked `charging` and excluded from the curve.

Second round (Stage 0 review), also reflected in [`SPEC.md`](SPEC.md):

6. Classic Bluetooth audio: IOBluetooth `batteryPercentSingle` / `Left` / `Right` /
   `Case`, each guarded by `respondsToSelector:`; missing or 0 on a connected device →
   "battery not reported". No `system_profiler` for this class. `coarse` until the
   step is measured; 1 % step → `exact`.
7. Device on two transports: percent from the source with the best precision, flags
   combined.
8. Headset auto-merge only with base "headset on Bluetooth" flag **and** a
   `bt_name_patterns` match; never by name alone.
9. Spare battery (if byte 7 is confirmed): component `spare`; optional "spare fully
   charged" notification, off by default.
10. HID++ usage pages in the device database, not in code.
11. Input Monitoring: verify on the `.app` in Stage 2; if not needed, never request it.
