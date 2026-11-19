# Development notes

## Building and testing

```
cargo test --workspace            # all tests run on recorded traces, no hardware needed
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p battbar-cli -- probe --no-bluetooth
scripts/bundle.sh                 # builds target/battbar.app (app + CLI), signed
open target/battbar.app
```

## Code signing and macOS permissions (TCC)

macOS remembers privacy permissions (Bluetooth, notifications) per *code identity*.
An ad-hoc signature (the default for `cargo build` on Apple Silicon) changes with every
build, so macOS asks again after each rebuild. battbar is therefore signed with a
**stable self-signed code-signing certificate**: the identity stays the same across
rebuilds and updates.

Create the certificate once (it goes into your login keychain, valid 10 years):

```
scripts/create-dev-cert.sh            # creates the identity "battbar Development"
security find-identity -p codesigning # it is listed (as not trusted: that is fine)
```

`scripts/bundle.sh` signs with `BATTBAR_SIGN_IDENTITY` (default
`battbar Development`) and falls back to ad-hoc with a warning if it is missing.

In the release workflow the same kind of certificate is imported from CI secrets
(`BATTBAR_CERT_P12_BASE64`, `BATTBAR_CERT_PASSWORD`) into a temporary keychain.
Developer ID signing and notarization can be added later as a separate step.

## Installing binaries: never copy over an existing file

macOS caches a binary's code signature by inode. Overwriting a signed executable in
place (`cp new old`) keeps the inode but changes the contents; the next launch is
killed with **SIGKILL (exit code 137)**. Always:

```
rm -f dest && cp src dest          # or
cp src dest.tmp && mv -f dest.tmp dest
```

All scripts in `scripts/` follow this rule. `ditto`/`cp -R` of a whole `.app` into a
fresh directory is fine.

## Bluetooth from the CLI

A process started from a terminal is attributed to the terminal app by TCC, which has
no Bluetooth usage description, so the first Bluetooth call would kill it. The CLI
embeds its own Info.plist and re-launches itself as its own *responsible process*
(`posix_spawn` + `responsibility_spawnattrs_setdisclaim`) before touching Bluetooth.
macOS then asks for permission for `battbar` itself.

## Recording traces

`battbar record --out traces/<name>.jsonl --note "reference values"` records only
vendor interfaces (HID++ incl. GATT, base stations). Unit IDs are replaced with stable
pseudonyms unless `--keep-ids` is given. Bluetooth system values (IOBluetooth,
Battery Service) are not frame-based and are not part of traces.
