# Development notes

## Building and testing

> **Do not run a development build and an installed release at the same time.** Both
> use the same HID++ software ID, so replies from receivers and devices can be consumed
> by the wrong process (wrong or missing values in both). Quit one before starting the
> other. Settings are separate since 0.2.0: development builds (bundle ID
> `io.github.battbar.dev`, or any debug build) use `~/.config/battbar-dev/`, releases
> `~/.config/battbar/`.

Hidden flags of the app binary (no UI): `battbar-app --login-status` prints the
SMAppService status, the bundle path and whether App Translocation is active;
`battbar-app --translocation-info [PATH]` prints the original location of a translocated
app (via `SecTranslocateCreateOriginalPathForURL`, resolved at run time);
`battbar-app --unregister-login-item` removes the login item.

```
cargo test --workspace            # all tests run on recorded traces, no hardware needed
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p battbar-cli -- probe --no-bluetooth
scripts/bundle.sh                 # builds target/battbar.app (debug app + CLI), signed
open target/battbar.app
open --env BATTBAR_LOG=1 --stderr /tmp/battbar.log target/battbar.app   # poll log
```

Start the app through `open` (LaunchServices), not by running
`Contents/MacOS/battbar-app` from a shell: then the shell is the responsible process for
TCC, has no Bluetooth usage description, and macOS kills the app
(`__TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION__`, SIGABRT). The CLI re-launches itself to
avoid this; the app does not.

Do not run the development app while the release app is running: both use the same
HID++ software ID, so over Bluetooth (GATT notifications reach both processes) each can
take the other's replies — seen as a garbled device name and a wrong "coarse" value.

`battbar debug-bt` (hidden) has two IOBluetooth diagnostics:
`BATTBAR_DEBUG_RUNLOOP=0` (run loop sources and battery values over the first 3 s of a
process) and `BATTBAR_DEBUG_WATCH=<seconds>` (battery values read every 5 s in this
process vs. through the helper process).

**Trying the update check** (development builds only): `BATTBAR_DEV_FAKE_VERSION`
makes the app compare the latest release with that version instead of its own, so
*Update available* appears against the real GitHub release. Turn on Settings → Check
for Updates in the dev app (it then checks right away; `BATTBAR_LOG=1` prints the
outcome). The code is behind `cfg(debug_assertions)` and is compiled out of release
builds; CI fails if a release binary contains the variable name
(`scripts/check-no-dev-hooks.sh`, with a self-test on a debug build).

```
open --env BATTBAR_DEV_FAKE_VERSION=0.1.0 target/battbar.app
open --env BATTBAR_LOG=1 --env BATTBAR_DEV_FAKE_VERSION=0.1.0 --stderr /tmp/battbar.log target/battbar.app
```

Release and universal (arm64 + x86_64) builds are made **only in CI**
(`.github/workflows/release.yml`); locally `scripts/bundle.sh` builds a debug bundle
for the host architecture.

## Threads, run loops and queues (rule)

**System objects bound to a run loop or a dispatch queue are created only on
long-lived threads/queues that service them, and provider threads are never torn
down while the app runs.**

Background: battbar 0.1.0 crashed (EXC_BREAKPOINT in `CFRunLoopAddSource`) when the
"headsetcontrol" switch rebuilt the providers. hidapi had created its `IOHIDManager` on
a provider thread and scheduled it on that thread's run loop; new threads enumerated
while the run loop of the exiting thread was being torn down. Reproduced on hardware by
`crates/battbar-hidpp/tests/hardware_rebuild.rs` (`--ignored`; SIGSEGV without the fix).

Audit (0.2.0):

| Object | Bound to | Lifetime | Status |
|---|---|---|---|
| hidapi `IOHIDManager` (enumerate, open) | run loop of the **HID service thread** (`battbar-hid`) | whole process; every hidapi call runs on it as a job; it services its run loop (`CFRunLoopRunInMode`, every ≤ 20 ms) so IOKit events do not pile up | fixed in 0.2.0 (tests: `thread_lifetime.rs`) |
| hidapi per-device read threads | their own run loops, created and stopped by hidapi (`hid_open_path` / `hid_close` unschedules before the thread ends) | per open device | hidapi-internal, correct |
| `CBCentralManager` + delegate | a private serial **dispatch queue** owned by the static `Central` | whole process (not tied to the calling thread) | safe |
| `GattSession` | std channels fed by the central's queue | per session | safe |
| IOBluetooth (`pairedDevices`, properties) | nothing registered: no notifications, delegates or async calls (checked: no run loop sources appear on the calling thread) | synchronous calls on the bt-system provider thread; battery values in a short-lived helper process (`battbar classic-batteries`), because a process gets them only during its first ~2 s | safe; if notifications are ever added, register them on a long-lived thread that runs its run loop |
| `NSStatusItem`, menus, `UNUserNotificationCenter`, `SMAppService` | main thread / main queue | whole process | safe |

Paths that could recreate threads:

- **Settings changes** — the app's engine is created once; settings act through shared
  switches (headsetcontrol: `SteelSeriesProvider::headsetcontrol_switch`) or the worker
  loop (interval). No provider thread exits while the app runs.
- **Sleep / wake, hotplug** — no threads are recreated: links fail and are reopened
  (HID devices are opened on the HID service thread; GATT sessions on the central's
  queue).
- **CLI** — one engine per process; `battbar debug-rebuild N` (hidden) rebuilds all
  providers N times with Bluetooth as a stress test (25 rounds pass on the reference
  hardware).

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

The certificate in use is pinned in `packaging/release-cert.sha1`
(`0d2f4d28ca5966872d894e69112d789e2774976d`). The release workflow fails if a bundle is
signed by any other certificate (`scripts/check-release-signature.sh`). Users can check
a download themselves:

```
codesign -d -r- /Applications/battbar.app
# designated => identifier "io.github.battbar" and certificate leaf = H"0d2f4d28ca5966872d894e69112d789e2774976d"
```

Replacing the certificate deliberately means updating that file in the same release
and accepting that every user is asked for permissions again.

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

## Secrets (repository StanislavTugarev/battbar → Settings → Secrets → Actions)

| Secret | Content | Format |
|---|---|---|
| `BATTBAR_RELEASE_P12_BASE64` | "battbar Release" certificate + private key (`battbar-release.p12`) | **base64** of the .p12 file, one line |
| `BATTBAR_RELEASE_P12_PASSWORD` | password of that .p12 | as is (no trailing newline needed) |
| `HOMEBREW_TAP_DEPLOY_KEY` | private half of an SSH deploy key with **write** access to `StanislavTugarev/homebrew-battbar` only | as is: the whole OpenSSH private key file, including the BEGIN/END lines |

The tap is updated over SSH with that deploy key (no personal access token); the
public half is registered as a deploy key on the tap repository only, so the key cannot
touch anything else.

```
# release certificate (once)
scripts/create-release-cert.sh ~/battbar-release-signing
base64 -i ~/battbar-release-signing/battbar-release.p12 | gh secret set BATTBAR_RELEASE_P12_BASE64 --repo StanislavTugarev/battbar
gh secret set BATTBAR_RELEASE_P12_PASSWORD --repo StanislavTugarev/battbar < ~/battbar-release-signing/battbar-release.p12.password

# tap deploy key (once)
ssh-keygen -t ed25519 -N "" -C "battbar release -> homebrew-battbar" -f ~/battbar-release-signing/tap-deploy-key
gh repo deploy-key add ~/battbar-release-signing/tap-deploy-key.pub --repo StanislavTugarev/homebrew-battbar --allow-write --title "battbar release workflow"
gh secret set HOMEBREW_TAP_DEPLOY_KEY --repo StanislavTugarev/battbar < ~/battbar-release-signing/tap-deploy-key
```

Back up `~/battbar-release-signing/` offline, then delete it from the machine.

## CI runners

CI checks and the release workflow run on **`macos-15-intel`**. In November 2026 the
arm64 macOS runner pool (`macos-15`) repeatedly failed to assign a runner for 15 minutes
("capacity constraints"), so jobs never started. The Intel image builds both slices of
the universal binary (aarch64 is cross-compiled, `lipo` joins them), and the tests do not
depend on the architecture. Move back to an arm64 label when its capacity is reliable;
GitHub plans to retire the Intel image in 2027.

**Known gap:** on the Intel runner the arm64 slice of the universal binary is compiled
and linked, but **never executed in CI** — tests run the x86_64 build only. This is
accepted until CI returns to an arm64 runner or runs a matrix (arm64 + Intel). Until
then, arm64 is exercised only on the maintainer's Apple Silicon Mac (development build,
and the first-launch check of every release).

## Disk image

`scripts/build-dmg.sh APP OUT.dmg` builds the drag-to-Applications disk image with
`dmgbuild` (`pip install dmgbuild==2.0.0`, MIT; it writes the Finder window layout
directly, so it also works on headless CI runners). Window layout:
`packaging/dmg/settings.py`; background: `design/dmg/background.svg` (rendered at @1x
and @2x into a multi-resolution TIFF). The release workflow signs the image with the
release certificate and lists it in `SHA256SUMS`.

## Making a release

Tags: `vX.Y.Z` for releases, `vX.Y.Z-rc.N` for pre-releases. `X.Y.Z` must equal
`version` in the workspace `Cargo.toml` (checked by `scripts/release-version.sh`).

- **Pre-release** (`v0.1.0-rc.1`): GitHub release marked *pre-release*, assets
  `battbar-0.1.0-rc.1.zip` + `SHA256SUMS`; notes from the `[0.1.0]` CHANGELOG section if
  it exists; the Homebrew tap is **not** updated.
- **Release** (`v0.1.0`): requires a `## [0.1.0] - date` section in `CHANGELOG.md`;
  the tap's `Casks/battbar.rb` is updated with the new version and checksum.

Steps:

1. Move the `[Unreleased]` entries in `CHANGELOG.md` to `## [X.Y.Z] - date` and set
   `version = "X.Y.Z"` in the workspace `Cargo.toml`.
2. Commit, then tag and push the tag (`git tag vX.Y.Z-rc.1 && git push origin vX.Y.Z-rc.1`).
   The workflow checks the version, runs the tests, builds universal binaries, signs
   them, zips, writes `SHA256SUMS` and publishes the release.
3. After testing the pre-release, tag `vX.Y.Z` the same way.
