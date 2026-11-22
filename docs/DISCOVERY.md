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

## Stage 1 confirmations (2026-11-19)

Measured with the Stage 1 code and background loggers on the reference hardware.

| Hypothesis | Result |
|---|---|
| Base byte 5 = "headset also on Bluetooth" | **Confirmed.** Headset Bluetooth on: bytes 4/5 = `04 01`; turned off on the headset: `01 00`. |
| Base byte 7 = spare battery, 0–8 | **Supported**: rose 4 → 6 → 7 over ~1 h while the spare charged in the base slot. Not yet seen to stop at 8; implemented as component `spare` (coarse). |
| IOBluetooth `batteryPercentSingle` step | **1 %**: 85 → 84 → 83 over ~10 min of use → `precision = exact` (HFP 1.7 battery indicator, not `AT+IPHONEACCEV`). |
| Receiver extended pairing register holds the unit ID | **Confirmed**: 0xB5/0x30+n bytes 1–4 equal HID++ 0x0003 unit ID for both devices, so a keyboard away from the receiver still carries its identity. |
| Keyboard switched Lightspeed → BLE → Lightspeed stays one device | **Confirmed** live with `battbar probe`. |
| Headset via base + Bluetooth is one device | **Confirmed** live (base flag + `bt_name_patterns`). |

Bluetooth permission for a CLI: a plain binary started from a terminal is killed by
TCC; with an embedded `__info_plist` **and** re-launched as its own responsible process
(`posix_spawn` + `responsibility_spawnattrs_setdisclaim`) it gets its own permission
prompt and works. Each rebuilt ad-hoc-signed binary is a new identity for TCC and asks
again.

## Stage 2 confirmations (2026-11-19)

| Check | Result |
|---|---|
| Input Monitoring needed for vendor-only USB interfaces | **No.** Signed `.app`: receiver and base station read normally while `IOHIDCheckAccess(ListenEvent)` = unknown (never requested, never granted). |
| Spare battery byte 7 reaches 8/8 | **Yes**: 4 → 6 → 7 → 8 over ~1.5 h in the base slot, then stays at 8. |
| Learned headset ↔ Bluetooth link | Learned live: base flag 00 → 01 with exactly one matching Bluetooth device; with headset Bluetooth off afterwards it stays one device. |
| Stable self-signed signature keeps Bluetooth permission across rebuilds | Yes: designated requirement `identifier "io.github.battbar" and certificate leaf = H"…"`. |
| Keyboard hopping Lightspeed ↔ BLE | Event checks re-poll within seconds; one transient split (BLE Battery Service ready before the GATT HID++ link) fixed with a session identity memory. |

## Battery values while charging (2026-11-19)

Observation: plugging the mouse (G502 X PLUS) into its charger raised the reported
level from 8 % to 52 % within 15 s — identically in G HUB. Not a battbar bug: the
firmware estimates the state of charge from the cell voltage, and under charge the
terminal voltage is raised by the charge current (V = OCV + I·R); on the flat part of
the Li-ion curve that reads as tens of percent. After unplugging, the value relaxes back
over several minutes.

| Device | Battery feature | Reports charging? | Capture while charging |
|---|---|---|---|
| Mouse G502 X PLUS (Lightspeed) | **0x1004 Unified Battery v5**, state of charge in % | **Yes**: `charging_status = 1`, `external_power = 1` | `46 08 01 01` → 70 %, level "full", charging, external power |
| Keyboard G915 (Lightspeed, BLE) | **0x1001 Battery Voltage v3** only (mV) | **Yes**: flags bit 7 = external power, bits 0–2 = charge status (`0x81` = cable, full) | 4200 mV, flags `0x81` |

Both report charging, so the rule below applies to both; a device that reports only a
voltage without charge flags would fall under the same rule as soon as any merged source
reports charging.

Rule (SPEC "Battery values while charging"): while charging, the percentage is shown as
`approximate` whatever its source; for 10 minutes after charging stops as well
(settling); no low-battery alerts in either phase; curve calibration ignores both.

## Battery-powered device (2026-11-19)

Logitech Pebble Keys 2 K380s (2× AAA), over Bluetooth LE (GATT HID++):

| | |
|---|---|
| PID (BLE) | `B377`, HID++ 4.5, 30 features + root |
| Battery | **0x1004 Unified Battery v3** — capabilities `0f 02` (levels 0b1111, flags `0x02`), status `64 08 00 00` = 100 %, level full, discharging |
| Other sources | BLE Battery Service 100 %; `system_profiler` 100 % |
| battbar 0.1.0 | 100 %, exact (no false 0 %) |
| Features | 0000 0001 0003 0005 1D4B 0020 0007 0011 1004 1B04 1814 1815 40A3 4220 4521 4531 4540 00C3 1802 1807 1816 1805 1830 1891 18A1 1E00 1E02 1602 1EB0 1861 18B0 |

### 0x1004 getCapabilities: which byte was read (checked against Solaar)

battbar calls 0x1004 function 0 (getCapabilities) and reads the parameter bytes as
`[byte0, byte1, …]`:

| Device | byte0 | byte1 | getStatus (fn 1) |
|---|---|---|---|
| G502 X PLUS (rechargeable, 0x1004 v5) | `0x0f` | `0x0f` | `0b 01 00 00` (11 %, critical, discharging) |
| Pebble Keys 2 K380s (2× AAA, 0x1004 v3) | `0x0f` | `0x02` | `64 08 00 00` (100 %, full, discharging) |

- **byte0** is treated as the mask of supported levels (critical/low/good/full = bits
  0–3): `0x0f` on both devices = all four levels.
- **byte1** is treated as flags. battbar 0.1.0 uses only bit 1 (`0x02`, "state of
  charge in % supported") to decide `exact` vs level-based `coarse`.
- The idea "bit 0 of byte1 = rechargeable" comes only from these two samples (G502
  `0x0f` has it, Pebble `0x02` does not). It is **unconfirmed — a hypothesis from 2
  samples**. Since 0.2.0 battbar uses it only to word low-battery alerts ("charge it" /
  "replace the batteries"); without the byte the wording is neutral.
- **Solaar does not call getCapabilities at all** (`lib/logitech_receiver/hidpp20.py`,
  `get_battery_unified` / `decipher_battery_unified`): it reads only getStatus
  `[discharge %, level, status, —]`, treats `level` as a single bit (8 full, 4 good,
  2 low, 1 critical) and falls back to that level when the percentage is 0. So Solaar
  neither confirms nor contradicts the byte1 bit meanings.
- Before relying on byte1 bits (0.2.0), confirm them with the feature description or
  more samples, in particular a rechargeable device without state-of-charge reporting
  and another battery-powered one. The G502 having `0x0f` in byte1 too makes a single
  "byte = level mask" misread plausible in other code, so name both bytes explicitly. Voltage-only (0x1001) devices on AA cells are covered in 0.1.0 by a guard only
(synthetic trace `traces/synthetic-receiver-aa-battery-0x1001.jsonl`).

## Hotplug and wake-ups (0.5, 2026-11-21)

Reference Mac mini; Lightspeed receiver (046d:c547) with G502 X PLUS (slot 2) and
G915 (slot 1); Arctis Nova Pro base station (1038:12e0); Pebble Keys 2 K380s over
Bluetooth LE. Times from `scripts/measure-hotplug.py`: from pressing Enter at the
action to the poll that shows the new state (the log line is written right before the
menu bar is redrawn); development builds add `BATTBAR_DEV_LOG_HIDPP=1` timelines.

### Why 0.4 took a minute for a plugged-in receiver or base

- The 10 s re-scan in 0.4 **did find the receiver** within 10 s (log: the receiver's
  devices appear in the next poll as "Disconnected"). At that moment the devices had
  not reached the receiver by radio yet; when they did, nothing told battbar, so they
  appeared at the next minute poll (69.9 s). The base station likewise: found within
  10 s, the headset reached it a few seconds later, seen at the minute poll (64.1 s).
- The Pebble keyboard switched off stayed at 100 % (> 120 s in the first 0.5 build):
  the value came from the `system_profiler` fallback, whose output was cached for 5
  minutes.

### What the receiver announces (HID++ notification timelines)

- **Disconnect**: always a 0x41 with flag bit 0x40 ("link not established"), within
  ~2 s (seen for both devices in every run).
- **Connect**: not reliable. Sometimes a 0x41 without bit 0x40 (e.g. `10 01 41 11 b1 …`
  when the G915 was switched on), mostly only the device's own frames (keyboard
  `11 01 0a 10 01`, mouse `11 02 09 10 01` and other feature notifications) a few
  seconds after it connected, and once **nothing at all**: after plugging the receiver
  in, the mouse reached it silently (first poll 16:55:11 "Disconnected", no frame from
  slot 2 until 16:56:12, where a keyboard frame triggered a poll that found the mouse
  at 68 %).
- **Waking up**: the mouse sends `11 02 09 10 01` when it wakes, `11 02 09 10 00`
  when it falls asleep; no 0x41.
- Register 0x02 ← 0x02 (Solaar `notify_devices`) makes the receiver send a 0x41 per
  device at once (tried in 74789ad: `10 01 41 11 31 …`, `10 02 41 11 32 …` right
  after the write), but only for the state at that moment: devices that connect a few
  seconds after the receiver is plugged in were still missed. Replaced (077c81b) by a
  ping of every slot that was disconnected at the last poll, in the existing 2 s check.
  The receiver answers "unknown device" for an absent device itself (~1 ms in the
  trace); in the runs below, the ping found the devices 2.3 s after the receiver link
  opened, before their own first frames (2–5 s later). Nothing was missed without the
  replay (receiver plugged in 3 times in a row: devices found by the first ping each
  time).

### `system_profiler` fallback: what it carried

`system_profiler SPBluetoothDataType` knows a battery value only for the Pebble
(`device_batteryLevelMain: 100%`); the headset, DualSense and G915 entries have none.
With the fallback disabled (development flag, removed with the path), the app showed
the same devices, sources (Pebble: `HID++ 0x1004` and `BLE Battery Service`),
values and kinds. The path, its cache and the live cross-check were removed (17bd056).

### Safety re-scan without hotplug notifications

`BATTBAR_DEV_NO_HOTPLUG=1 BATTBAR_DEV_RESCAN_SECS=20`: receiver unplugged, then
plugged in.

| Run | Enter (plug) | re-scan found it | shown with % |
|---|---|---|---|
| 1 (4182ca7) | 16:54:54 | 16:55:10.8 (16.8 s) | 78.4 s — the mouse connected silently later (see above) |
| 2 (077c81b) | 17:09:38 | 17:09:54.4 (16.4 s) | 16.5 s |

The re-scan runs at the first 2 s check at least 20 s after the last poll (here 21 s
after the unplug poll), so it finds a missed receiver within 20–22 s; in release builds
the interval is 5 minutes and the minute poll scans links anyway.

### Base station events

Listening to the base's vendor interface (`battbar debug-base-listen`, interface 4,
collections 0xFFC0/0xFF00; nothing sent, battbar not running) while the headset was
switched off and on, charged, and its Bluetooth switched on and off: the base sends a
report by itself for every change, and nothing in between (10 minutes).

| Clock | Report | Meaning (fields as in the status reply) |
|---|---|---|
| 17:37:33.62 | `07 b5 01 00 04` + `07 b7 00 08 01` | headset off (byte 15 = 01) |
| 17:37:45.95 | `07 b5 01 00 08` + `07 b7 07 08 08` | headset on, battery 7/8 |
| 17:37:50.96 | `07 b7 07 08 02` | charging (byte 15 = 02) |
| 17:38:17.96 | `07 b7 07 08 08` | charging stopped |
| 17:38:20.26 / 21.83 | `07 b5 04 02 08` / `07 b5 04 01 08` | Bluetooth connecting / connected (bytes 4–5) |
| 17:38:32.95 | `07 b5 01 00 08` | Bluetooth off |
| 17:37:08.02 | `07 25 19` | not headset state (probably the volume dial); ignored |

So `0x07 0xb5` = `<byte 4> <byte 5: Bluetooth 0 none / 2 connecting / 1 connected>
<08 on / 04 off>` and `0x07 0xb7` = `<battery 0–8> <spare> <state: 01 off / 08 on / 02
charging>`. Since then battbar reads these reports on a second handle in a blocking
thread and no longer asks the base for its status every 2 s. In the running app (the
provider's own handle open at the same time) every report arrived on the listener's
handle and gave one poll (17:43–17:44: Bluetooth connecting/connected/off, headset
off/on, charging on/off; a pair of reports 0.1 s apart gave one poll); the poll
finished 0.6–1.2 s after the report.

Before (status request in the 2 s check), development log of the base's headset state: after plugging the base in (link open
17:11:14, headset "Disconnected"), the headset reached it at 17:11:18.86 and the poll
showing it finished at 17:11:20 (1.1 s; the poll itself takes ~1 s). Bluetooth on/off
on the headset: base state changed at 17:10:46.44 / 17:10:51.55, polls at 17:10:47 /
17:10:52.

### Results

Seconds from Enter to the new state. "now" = 077c81b (full run; receiver plug also
3 runs in a row: 2.9, 1.4, 2.4 s).

| Scenario | 0.4.0 | d9fcbbc | 74789ad | now | from the device being on line (log) |
|---|---|---|---|---|---|
| Mouse off | 0.0¹ | 0.3 | 1.0 | 1.2 | 0x41 at +1.2 s, poll at once |
| Mouse on | 0.0¹ | 0.3 | 0.2 | 0.8 | ping found it at +0.9 s |
| G915 off | 0.0¹ | 1.9 | 0.3 | 1.9 | 0x41 at +2.2 s, poll at once |
| G915 on | 3.1 | 6.2 | 1.1 | already² | ping found it before Enter |
| Pebble off | 2.6 | > 120 | 1.2 | 0.1 | |
| Pebble on | 6.4 | already | 3.3 | 4.9 | HID hotplug at ~+3.7 s (Bluetooth connection), shown 0.3 s later |
| Headset Bluetooth on | 1.7 | 0.0 | 1.1 | 7.0 | base state at +6.4 s, poll 0.6 s later |
| Headset Bluetooth off | 3.0 | 1.5 | 3.3 | 5.1 | base state at +4.5 s, poll 0.5 s later |
| Receiver unplugged | 0.8 | 0.0 | 0.0 | 4.3³ | HID removal, poll 0.3 s later |
| Receiver plugged in | 69.9 | 21.0 | 62.5 | 7.0³ (2.4 / 1.4 / 2.9) | link open +4 s, ping found the mouse 2.3 s later |
| Base unplugged | 0.6 | already | 0.0 | 5.2³ | HID removal, poll 0.3 s later |
| Base plugged in | 64.1 | 20.4 | 5.7 | 10.4³ | link open +4 s, headset on line +8.9 s, shown +10.1 s (2 s check; since then by the base's own report, see above) |

¹ 0.4 polled every 6–10 s, so a scenario could land right after a poll; not comparable.
² Shown before Enter was pressed. ³ The log shows the USB event itself 4–5 s after
Enter in this run (the receiver-only runs: 0.0–1.4 s), i.e. the time of reaching the
port; battbar reacted within 0.3 s of the event.

Remaining delays are the devices' own: the headset takes 4–9 s to connect by radio or
Bluetooth, the Pebble ~3–4 s to connect over Bluetooth. Every device was shown within
≤ 2.3 s after it was on line (the 2 s check), ≤ 0.3 s after a USB or Bluetooth HID
hotplug event.

Observations, not changed:
- When the Pebble connects, CoreBluetooth first reports it as "Pebble K380s" with
  the Battery Service not yet readable: for one poll (17:10:38) a separate row
  "Pebble K380s [Other] Battery not reported" existed, merged into the Pebble a second
  later (new devices are held back by the new-device check, so it is not expected in
  the menu).
- In this run no IOBluetooth connect/disconnect poll appeared for the headset's
  Bluetooth; the base's state caught both changes. Earlier runs showed it (0.3 s).

Idle wake-ups (Instruments System Trace, 30 s between polls): 9.2/s in 0.4.0, 1.8/s
in d9fcbbc; per thread in DEVELOPMENT.md "Timers and wake-ups". The ping of
disconnected slots rides on the existing 2 s check (no extra wake-up); not measured
again.

## Background load (0.5, 2026-11-22)

Reference Mac mini (M4), macOS 26.1. Release-optimized build (`BATTBAR_LOCAL_RELEASE=1
scripts/bundle.sh`, development bundle ID), worst case: base station plugged in, G915
switched off (its slot is pinged in every 2 s check).

### Cost of one 2 s check (`battbar debug-check-cost`, 30 checks each)

| State | CPU per check | Context switches per check | Wall time |
|---|---|---|---|
| HID++, every device on line | 17 µs | 0.1 | 0.01 ms |
| HID++, G915 off (ping of its slot) | 133 µs | 3.4 | 1.2 ms (the receiver answers itself) |
| Base station, status request (before rc: in every check) | 151 µs | 3.4 | 28 ms |
| Base station now (events read by a blocking listener; the check asks nothing) | 6 µs | 0 | 0 |

The ping of a disconnected slot costs ~1.7 context switches/s while the slot stays
disconnected; kept as is (decided 2026-11-22): narrowing it to the first seconds after a
plug-in would add state for an invisible gain. Further wake-up optimisation is stopped
at these numbers.

### 6-hour run (`scripts/soak.py`, sample every 5 min, 03:17–09:15)

| | Start | 1 h | 2 h | 3 h | 6 h |
|---|---|---|---|---|---|
| RSS | 46.9 MB | 47.5 MB | 47.5 MB | 52.0 MB¹ | 50.9 MB |
| CPU time (accumulated) | 0.3 s | 2.3 s | 4.7 s | 5.9 s | 11.6 s |
| Threads / open files | 16 / 29 | 16 / 29 | 16 / 29 | 16 / 30¹ | 16 / 30 |

- **CPU: ~1.9 s per hour** (first hour 2.3 s with the start-up).
- **Context switches: ~8.3/s** averaged over the run including the minute polls (the
  ping accounts for ~1.7/s of it; 6.6/s without it, measured for d9fcbbc).
- **Power assertions held by battbar: none** (all 72 samples). The Mac went to idle
  sleep twice during the run (05:19, 06:01) with the ping and the base listener active.
- **Settings file written once**, at start; no write in 6 hours. Since 0.5 a save with
  unchanged content writes nothing (test `polls_without_changes_write_nothing`). No log
  file, cache or temporary file in release builds (the poll log exists only with
  `BATTBAR_LOG`).
- **After waking**: 05:30:55 dark wake → two polls (`hid` +2 s, `device event` +6 s),
  then the minute rhythm; 06:18:15 dark wake → one poll (+30 s). No burst. The full
  wake at 05:30:58 ("due to HID Activity") followed a `bluetoothd` "Bluetooth LE HID
  Activity" assertion at 05:30:56 (the Pebble reconnecting), before battbar's first
  poll; the dark wake at 06:18, with a battbar poll at 06:18:45, did not become a full
  wake.

¹ One step at the first wake (05:30, the Pebble reappeared as a HID device): +4.4 MB
RSS, +1 open file; flat afterwards, and slightly down over the last 3 hours.

### Memory: polls do not leak

Accelerated run (poll every 10 s instead of 60 s, 35 min, 208 polls), heap summary
every 5 minutes: 30 233 malloc nodes (±6), 18.63 MB, 39 blocks of 352 KB, footprint
15.0–15.2 MB, 31 open files — unchanged from minute 5 to minute 35. The 6-hour
process, compared with a fresh one, had ~3 000 more nodes, 27 more 352 KB blocks and
+2.7 MB footprint: these came with events (wake, devices reconnecting, the menu being
opened), not with polls; see the reconnect test below.
