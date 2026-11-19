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
