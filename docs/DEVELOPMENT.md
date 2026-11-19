# Development notes

## Building and testing

```
cargo test --workspace            # all tests run on recorded traces, no hardware needed
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p battbar-cli -- probe --no-bluetooth
scripts/bundle.sh                 # builds target/battbar.app (debug app + CLI), signed
open target/battbar.app
BATTBAR_LOG=1 target/battbar.app/Contents/MacOS/battbar-app   # with a poll log on stderr
```

Release and universal (arm64 + x86_64) builds are made **only in CI**
(`.github/workflows/release.yml`); locally `scripts/bundle.sh` builds a debug bundle
for the host architecture.

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

## Release signing

Releases are signed with a **separate** self-signed certificate, **"battbar Release"**
(25 years), never with a development certificate. macOS keeps users' permissions
(Bluetooth, notifications) only while updates are signed by the same certificate: the
app's designated requirement is `identifier "io.github.battbar" and certificate leaf =
H"<sha1 of battbar Release>"`.

> **Losing the release key (or replacing it) means every user is asked for all
> permissions again after the next update**, and the Homebrew cask cannot tell them why.
> Keep two copies: the GitHub secrets and an offline backup.

Creating it (once, maintainer):

```
scripts/create-release-cert.sh ~/battbar-release-signing   # refuses to write inside the repo
gh secret set BATTBAR_RELEASE_P12_BASE64 < <(base64 -i ~/battbar-release-signing/battbar-release.p12)
gh secret set BATTBAR_RELEASE_P12_PASSWORD < ~/battbar-release-signing/battbar-release.p12.password
```

Then back up `~/battbar-release-signing/` offline (password manager or encrypted
storage) and delete it from the machine.

In the release workflow `scripts/ci-import-cert.sh` imports the certificate into a
temporary keychain, `scripts/bundle.sh` signs with `BATTBAR_REQUIRE_IDENTITY=1` (no
ad-hoc fallback), and the keychain is deleted at the end of the job. Developer ID
signing and notarization can later replace the "Import signing certificate" and
"Bundle and sign" steps without touching the rest of the workflow.

Signing material is never committed: `.gitignore` excludes `*.p12`, `*.cer`, `*.key`,
`*.pem` and keychains, and CI runs `scripts/check-no-secrets.sh`.

## Making a release

1. Move the `[Unreleased]` entries in `CHANGELOG.md` to a new `## [X.Y.Z] - date`
   section and set `version = "X.Y.Z"` in the workspace `Cargo.toml`.
2. Commit, tag `vX.Y.Z`, push the tag. The workflow checks tag = version = changelog
   section, runs the tests, builds universal binaries, signs, zips, writes
   `SHA256SUMS`, publishes the GitHub release and updates the Homebrew tap
   (`OWNER/homebrew-battbar`, needs the `HOMEBREW_TAP_TOKEN` secret).

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
