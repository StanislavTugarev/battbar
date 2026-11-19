<!-- DRAFT — not published. Title: -->
# Help wanted: battery for older Logitech devices (HID++ 1.0)

battbar shows the battery of Logitech devices that speak **HID++ 2.0** (most devices
from about 2013 on). Older devices on a Unifying receiver speak **HID++ 1.0**, which
reports the battery through device registers instead of features. battbar currently
lists such devices as *"Battery not reported (older protocol, not supported yet)"*.

We do not own any HID++ 1.0 device, and we do not want to implement this blindly. If you
have one, a short recording from you lets us add support and test it without the
hardware.

## Is my device affected?

Run `battbar probe`. An affected device looks like this:

```
M705 Marathon  [Mouse]  n/r (HID++ 1.0)
    receiver       HID++ 1.0                  n/r (HID++ 1.0)
```

## What we need (about 5 minutes)

1. Install battbar 0.2.0 or later (README → Install) and connect the device through its **Unifying
   receiver**. Wake it up (move the mouse / press a key).
2. Write down the battery level that Logitech's software (Options+, Logitech Options or
   SetPoint) shows, or the LED state of the device.
3. In Terminal:

   ```
   battbar dump --out ~/Desktop/battbar-dump.json
   battbar record --out ~/Desktop/battbar-trace.jsonl --polls 3 --note "Options+ shows: good / 50 %"
   ```

4. Reply here with both files attached, and tell us the device model and, if you can,
   the battery type (AA alkaline, AA NiMH, AAA, built-in).

If you can, record twice: once with fresh batteries and once with used ones, each with
its reference value in `--note`.

## What the files contain

Only traffic with the receiver's vendor-specific HID++ interface and diagnostic
information (product IDs, protocol versions, feature lists). For HID++ 1.0 devices
both files include the answers of the two battery registers (0x07 battery status,
0x0D battery charge). No keystrokes, no serial numbers, no Bluetooth addresses; device
unit IDs are replaced with pseudonyms. Both files are plain text — please have a look
before uploading.

`battbar dump` and `battbar record` only *read*: register writes (SET_REGISTER) are
blocked in these commands, so nothing on your device or receiver is changed.

## Scope (for contributors)

Before any code, we will collect which HID++ 1.0 battery registers the recorded devices
answer (e.g. battery status / mileage registers), estimate the support needed and
discuss it here. See [docs/ADDING_A_DEVICE.md](../../docs/ADDING_A_DEVICE.md) and
`docs/SPEC.md` (backlog, item 7).

---
Labels: `help wanted`, `device support`, `hid++ 1.0`
