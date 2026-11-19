# Contributing to battbar

Thanks for helping! battbar is maintained by volunteers who do not own every device, so
**data from your device is the most valuable contribution**:
[docs/ADDING_A_DEVICE.md](docs/ADDING_A_DEVICE.md) shows how to collect it.

## Ground rules

- Code, UI text, docs, commit messages and issues are in **English**.
- **No telemetry** — ever. The only network access is the opt-in update check in
  `crates/battbar-app/src/updates.rs`; a test fails if any other code uses the network.
- **Privacy rules are hard requirements** (see "Privacy and security" in
  [docs/SPEC.md](docs/SPEC.md)): open only vendor-specific HID interfaces through
  `battbar-hid`, never a keyboard/mouse interface, never an `IOHIDDevice` on Bluetooth,
  never scan for Bluetooth devices. Tests enforce this; do not weaken them.
- Device support belongs in `devices/*.toml` whenever the protocol is already
  supported. Code changes are for new protocols or protocol variants.
- No vendor trademarks or logos in names, icons or UI text beyond "works with …".

## Development

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). Before opening a pull request:

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask supported-devices      # if devices/*.toml changed
```

All tests run on recorded traces; no hardware is needed. A change to a provider should
come with a trace test (`traces/` + `crates/*/tests/traces.rs`).

## Porting from other projects

Protocol logic may be ported from [Solaar](https://github.com/pwr-Solaar/Solaar)
(GPL-2.0-or-later) and [HeadsetControl](https://github.com/Sapd/HeadsetControl)
(GPL-3.0). Say so in the file header and in [NOTICE](NOTICE).

## License

By contributing you agree that your contribution is licensed under
GPL-3.0-or-later.
