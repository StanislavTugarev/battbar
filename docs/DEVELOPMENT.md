# Development notes

## Building and testing

> **A development build and an installed release can run at the same time** (since
> 0.3.0): development builds use their own HID++ software ID (`0x0A`, release `0x0B`),
> so neither takes the other's replies — over Bluetooth, GATT notifications reach both
> processes, and with one shared ID that showed as a garbled device name and a wrong
> value. `battbar record` and `probe --replay` keep the release ID so traces replay in
> tests. Settings are separate since 0.2.0: development builds (bundle ID
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

`battbar debug-bt` (hidden) has two IOBluetooth diagnostics:
`BATTBAR_DEBUG_RUNLOOP=0` (run loop sources and battery values over the first 3 s of a
process) and `BATTBAR_DEBUG_WATCH=<seconds>` (battery values read every 5 s in this
process vs. through the helper process).

**Menu bar item** (0.4): `target/debug/battbar-app --render-bar-preview DIR` (debug
builds) draws the status item for sample devices in every layout (expanded, both
collapsed styles, a low device, no devices) as 2× PNGs with the real drawing code.
`BATTBAR_DEV_PAD_WIDTH=<points>` (debug builds) widens the expanded item, to try the
"not enough space" fallback on a large display:
`open --env BATTBAR_LOG=1 --env BATTBAR_DEV_PAD_WIDTH=1400 --stderr /tmp/battbar.log target/battbar.app`
(the log shows `status item visible=false` and then `collapsing`).
`BATTBAR_DEV_DUMP_MENU=1` (debug builds) prints the menu as an outline and the item's
window frame and occlusion state on every render. `battbar-app --probe-menu-bar
[--width N]` (all builds; no devices, Bluetooth or settings, so it can be started from
a shell) shows a test item of N points and logs the frontmost app, the item's window
frame and occlusion state, and the screen's menu bar areas around the notch whenever
they change.

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
| hidapi `IOHIDManager` (enumerate, open) | run loop of the **HID service thread** (`battbar-hid`) | whole process; every hidapi call runs on it as a job; between jobs it sleeps in its run loop until IOKit has an event or a job signals its own run loop source (0.2–0.3: woke every 20 ms) | fixed in 0.2.0, event-driven since 0.4.0 (tests: `thread_lifetime.rs`, `service_idle.rs`) |
| hidapi per-device read threads | their own run loops, created and stopped by hidapi (`hid_open_path` / `hid_close` unschedules before the thread ends) | per open device | hidapi-internal, correct |
| `CBCentralManager` + delegate | a private serial **dispatch queue** owned by the static `Central` | whole process (not tied to the calling thread) | safe |
| `GattSession` | std channels fed by the central's queue | per session | safe |
| IOBluetooth (`pairedDevices`, properties) | nothing registered (checked: no run loop sources appear on the calling thread) | synchronous calls on the bt-system provider thread; battery values in a short-lived helper process (`battbar classic-batteries`), because a process gets them only during its first ~2 s | safe |
| IOBluetooth connect/disconnect notifications (`battbar_macos::connections`) | run loop of the **main thread** | whole process; registered once Bluetooth permission is granted | safe (0.5) |
| HID hotplug IOHIDManager (`battbar_hid` hotplug, matches every HID device, opens none) | run loop of the **HID service thread** | whole process | safe (0.5; tests: `hotplug.rs`) |
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

## Timers and wake-ups (rule)

**The app does nothing periodically faster than every few seconds while idle.** A
menu bar app runs all day; on a notebook every wake-up costs energy and keeps macOS
from coalescing timers and from App Nap. Prefer notifications and one-shot timers;
the app has no repeating `NSTimer` (test `no_repeating_timers_in_the_app`).

| What | Period | Why |
|---|---|---|
| HID service thread | none — sleeps in its run loop until an IOKit event or a job | 0.3 polled every 20 ms (~45 context switches/s measured); fixed in 0.4 |
| HID hotplug (receivers, base stations, Bluetooth HID devices) | none — IOHIDManager matching/removal callbacks on the HID service thread (0.5) | — |
| Bluetooth connections (classic, e.g. headsets) | none — IOBluetooth connect/disconnect notifications on the main thread (0.5) | — |
| "Not enough space" check | none — occlusion and screen-change notifications, at most one one-shot timer (1 s, tolerance 0.1 s) after a change | 0.4 rc.3 polled every 0.5 s; replaced before release |
| Battery poll (worker) | the user's interval, 30 s – 5 min (default 60 s); GCD timer, leeway 15 % (≤ 15 s) | the batteries are read; nothing pushes battery levels for all devices |
| Device events (worker → providers) | **2 s**, GCD timer, leeway 0.3 s | kept, see below |
| Safety re-scans (HID links, base stations, Bluetooth connections) | 5 min, counted from the last poll | catches a missed hotplug notification (`Rescan`, test `a_missed_hotplug_is_caught_by_the_safety_rescan_only`) |
| Follow-up polls (hotplug debounce 0.3 s, new device settling, learned link) | one-shot, 0.3–4 s after the event | — |
| Update check | once a day, only if turned on | — |
| Child processes (`headsetcontrol`, the classic-battery helper) | wait in 10–50 ms steps, **only while one runs during a poll** (≤ 5 s) | `std::process::Child` has no wait-with-timeout; bounded, never while idle |
| HID++ / GATT request timeouts (1.5–2 s) | one wait per request, only during a poll | — |
| `--probe-menu-bar`, `battbar debug-events` | 0.5 s / run loop for N s | diagnostic tools, not the app |

The worker sleeps on its command channel without a timeout; one GCD timer source
(`WakeTimer`, re-armed one-shot for the next time the schedule needs) wakes it with a
leeway of 15 % of the period, so macOS can coalesce it with other wake-ups.

**The 2 s device-event check** (kept, with reason): it drains HID++ notifications
that hidapi has already buffered (device connected/disconnected on a receiver, battery
or charging events) — hidapi offers no callback for them, so they can only be seen by
asking — and, while a SteelSeries base station is open, asks the base for its status
(one request/reply on its vendor interface): the base sends nothing when its headset
connects, disconnects, moves to Bluetooth or starts charging. A poll follows only if
one of those changed. Making the HID++ part event-driven needs a blocking reader per
link (SPEC backlog).

**Idle wake-ups by thread** (reference Mac mini, 2026-11-21, Instruments *System
Trace*, 30 s between polls, Running intervals per thread of `battbar-app`):

| Thread | 0.4.0 | 0.5.0 | Source |
|---|---|---|---|
| `battbar-hid-service` | 1.73/s | 0 | 0.4: jobs of the 10 s HID re-scan; 0.5: only hotplug events and jobs of polls |
| `battbar-hidpp` | 1.57/s | 0.20/s | 2 s check (+ 10 s re-scan in 0.4) |
| `battbar-steelseries` | 1.37/s | 0.13/s | same |
| `battbar-bt-system` | 0.80/s | 0.13/s | same (+ 6 s Bluetooth list in 0.4) |
| main thread | 0.77/s | 0 | renders after polls only |
| `battbar-worker` | 0.40/s | 0.37/s | wakes for the 2 s check |
| unnamed (GCD workers: the wake-up timer, CoreBluetooth queue; hidapi read threads) | ~2.6/s | ~1.0/s | |
| **total** | **9.2/s** | **1.8/s** | |

Including the minute polls (`top`, 120 s, both builds side by side): 8.7 vs 6.6
context switches/s — the polls themselves (device I/O, the classic-battery helper
process) are now the larger part; the poll interval is the user's setting.

Measure with `top -l 2 -s 30 -stats pid,csw,time -pid <pid>` (the difference in
`CSW` between the two samples; `IDLEW` is cumulative and coalesced, not useful here).

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

Every build knows its **full version** with the suffix: the workflow sets
`BATTBAR_RELEASE_VERSION` (from the tag) for building and bundling; `battbar-core`'s
build.rs compiles it in (`battbar_core::version::FULL`; it must be `X.Y.Z` or
`X.Y.Z-rc.N` with the crate's `X.Y.Z`, otherwise the build fails) and `scripts/bundle.sh`
writes it to the `BattbarVersion` key of Info.plist. `CFBundleShortVersionString` and
`CFBundleVersion` stay `X.Y.Z`. The menu (line above Quit), `battbar --version`,
`battbar-app --version`, `battbar dump` and traces show it, and the update check
compares with it. The step *Version in the bundle matches the tag*
(`scripts/check-bundle-version.sh`) fails the release unless the plist key and both
binaries report the tag's version exactly. Local builds without the variable are
`X.Y.Z`; try a candidate build with `BATTBAR_RELEASE_VERSION=X.Y.Z-rc.99 scripts/bundle.sh`.

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
