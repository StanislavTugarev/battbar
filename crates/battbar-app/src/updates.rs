// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in update check — the only network request battbar ever makes.
//!
//! Off by default. When the user turns it on (Settings → Check for Updates, or the
//! first-run question), battbar asks GitHub at most once every 24 hours for the latest
//! release of this repository:
//!
//! ```text
//! GET https://api.github.com/repos/StanislavTugarev/battbar/releases/latest
//! User-Agent: battbar/<version>
//! Accept: application/vnd.github+json
//! ```
//!
//! Nothing else is sent: no token, no cookies, no device data, no identifiers. GitHub
//! sees the Mac's IP address and the User-Agent above. The request is made with the
//! system `/usr/bin/curl` with `-q` (so `~/.curlrc` cannot add headers), HTTPS only, no
//! redirects. Pre-releases and drafts are ignored; the release page URL is built from
//! the version, never taken from the response.

use battbar_core::Settings;
use serde::Deserialize;
use std::cmp::Ordering;
use std::time::Duration;

pub const OWNER_REPO: &str = "StanislavTugarev/battbar";
pub const LATEST_API: &str = "https://api.github.com/repos/StanislavTugarev/battbar/releases/latest";
pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// After a failed attempt (offline, rate limit) try again sooner than a day.
pub const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60 * 60);
const CURL: &str = "/usr/bin/curl";
pub const UPGRADE_COMMAND: &str = "brew upgrade --cask battbar";

/// Semantic version `MAJOR.MINOR.PATCH[-PRE]` (build metadata ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().strip_prefix('v').unwrap_or(s.trim());
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, p.split('.').map(str::to_string).collect()),
            Some(_) => return None,
            None => (s, vec![]),
        };
        let mut it = core.split('.');
        let num = |x: Option<&str>| x.filter(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))?.parse().ok();
        let v = Version {
            major: num(it.next())?,
            minor: num(it.next())?,
            patch: num(it.next())?,
            pre,
        };
        it.next().is_none().then_some(v)
    }

    pub fn current() -> Version {
        Version::parse(battbar_core::version::FULL).expect("build version is semver")
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if self.is_prerelease() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(o.major, o.minor, o.patch))
            .then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater, // 1.0.0 > 1.0.0-rc.1
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&o.pre) {
                        let c = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if c != Ordering::Equal {
                            return c;
                        }
                    }
                    self.pre.len().cmp(&o.pre.len())
                }
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The version compared with the latest release: the crate version.
///
/// Development builds only: `BATTBAR_DEV_FAKE_VERSION=0.1.0` pretends to be that
/// version, to try the "Update available" flow against the real latest release. The
/// code (and the variable name) is compiled out of release builds; CI checks the
/// release binaries for it (`scripts/check-no-dev-hooks.sh`).
pub fn running_version() -> Version {
    #[cfg(debug_assertions)]
    {
        if let Some(v) = std::env::var("BATTBAR_DEV_FAKE_VERSION").ok().and_then(|s| Version::parse(&s)) {
            if battbar_macos::bundle::is_dev_build() {
                return v;
            }
        }
    }
    Version::current()
}

/// Release page for a version (built locally, not taken from the response).
pub fn release_page(v: &Version) -> String {
    format!("https://github.com/{OWNER_REPO}/releases/tag/v{v}")
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

/// Longest accepted tag; real ones are like `v0.3.0`.
const MAX_TAG_LEN: usize = 32;

/// A release tag in the strict form `^v\d+\.\d+\.\d+$` (ASCII digits only, nothing
/// before or after). Anything else is an error: the check fails and nothing is shown.
pub fn parse_release_tag(tag: &str) -> Result<Version, String> {
    let bad = || Err(format!("unexpected release tag ({} bytes)", tag.len()));
    if tag.len() > MAX_TAG_LEN {
        return bad();
    }
    let Some(rest) = tag.strip_prefix('v') else { return bad() };
    let parts: Vec<&str> = rest.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return bad();
    }
    let n = |p: &str| p.parse::<u64>().map_err(|_| "release tag number out of range".to_string());
    Ok(Version {
        major: n(parts[0])?,
        minor: n(parts[1])?,
        patch: n(parts[2])?,
        pre: vec![],
    })
}

/// The released version in a `releases/latest` response. Drafts and pre-releases are
/// ignored (`None`); a tag that is not strictly `vX.Y.Z` is an error.
pub fn parse_latest(json: &str) -> Result<Option<Version>, String> {
    let r: ApiRelease = serde_json::from_str(json).map_err(|e| format!("unexpected response: {e}"))?;
    if r.prerelease || r.draft {
        return Ok(None);
    }
    parse_release_tag(&r.tag_name).map(Some)
}

/// Something that can fetch a URL. The app uses [`Curl`]; tests count calls.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<String, String>;
}

pub struct Curl;

impl Curl {
    /// Exact arguments, kept in one place so the documentation can quote them.
    pub fn args(url: &str) -> Vec<String> {
        let ua = format!("battbar/{}", battbar_core::version::FULL);
        [
            "-q", // must be first: ignore ~/.curlrc
            "--silent",
            "--show-error",
            "--fail",
            "--proto",
            "=https",
            "--max-redirs",
            "0",
            "--max-time",
            "20",
            "--max-filesize",
            "1000000",
            "--user-agent",
            &ua,
            "--header",
            "Accept: application/vnd.github+json",
            url,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

impl Fetch for Curl {
    fn get(&self, url: &str) -> Result<String, String> {
        let out = std::process::Command::new(CURL)
            .args(Self::args(url))
            .env_clear()
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        String::from_utf8(out.stdout).map_err(|e| e.to_string())
    }
}

/// Whether a check is due now (`now` in seconds since the Unix epoch).
pub fn due(s: &Settings, now: u64) -> bool {
    if !s.check_for_updates {
        return false;
    }
    match s.next_update_check {
        None => true,
        // A clock set back more than a day must not postpone checks for years.
        Some(next) => now >= next || next > now + INTERVAL.as_secs(),
    }
}

/// Start a check if it is due: schedules the next one and calls `start` (which does
/// the request, normally on another thread). Returns whether `start` was called.
/// With the option off this never calls `start` — no network at all.
pub fn tick(s: &mut Settings, now: u64, in_flight: bool, start: impl FnOnce()) -> bool {
    if in_flight || !due(s, now) {
        return false;
    }
    s.next_update_check = Some(now + INTERVAL.as_secs());
    start();
    true
}

/// What the app should do with a finished check.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A newer release exists; `notify` is true once per version.
    Available {
        version: Version,
        notify: bool,
    },
    UpToDate,
    /// The option was turned off while the request ran: the result is dropped.
    Ignored,
    Failed(String),
}

/// Record the result of a check in the settings.
pub fn finish(s: &mut Settings, now: u64, current: &Version, result: Result<Option<Version>, String>) -> Outcome {
    if !s.check_for_updates {
        return Outcome::Ignored;
    }
    match result {
        Err(e) => {
            s.next_update_check = Some(now + RETRY_AFTER_FAILURE.as_secs());
            Outcome::Failed(e)
        }
        Ok(Some(v)) if v > *current => {
            let tag = v.to_string();
            s.available_update = Some(tag.clone());
            let notify = s.update_notified.as_deref() != Some(tag.as_str());
            s.update_notified = Some(tag);
            Outcome::Available { version: v, notify }
        }
        Ok(_) => {
            s.available_update = None;
            Outcome::UpToDate
        }
    }
}

/// The newer version to show in the menu, if any (only while the option is on).
pub fn available(s: &Settings, current: &Version) -> Option<Version> {
    if !s.check_for_updates {
        return None;
    }
    s.available_update.as_deref().and_then(Version::parse).filter(|v| v > current)
}

/// Default Homebrew prefixes: Apple silicon, then Intel.
pub const DEFAULT_HOMEBREW_PREFIXES: [&str; 2] = ["/opt/homebrew", "/usr/local"];

/// Homebrew prefixes to look in, without running `brew`: `$HOMEBREW_PREFIX` first when
/// it is set to an absolute path (shells set up by `brew shellenv`; apps started from
/// the Dock usually do not have it), then the two defaults.
pub fn homebrew_prefixes(env_prefix: Option<std::ffi::OsString>) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = env_prefix
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .into_iter()
        .collect();
    for d in DEFAULT_HOMEBREW_PREFIXES {
        let d = std::path::PathBuf::from(d);
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

/// Whether the cask is installed under any of `prefixes` (`<prefix>/Caskroom/battbar`).
pub fn installed_with_homebrew_in(prefixes: &[std::path::PathBuf]) -> bool {
    prefixes.iter().any(|p| p.join("Caskroom/battbar").is_dir())
}

/// Installed with Homebrew: the cask has a Caskroom directory.
pub fn installed_with_homebrew() -> bool {
    installed_with_homebrew_in(&homebrew_prefixes(std::env::var_os("HOMEBREW_PREFIX")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Counting<'a>(&'a Cell<u32>);
    impl Fetch for Counting<'_> {
        fn get(&self, _url: &str) -> Result<String, String> {
            self.0.set(self.0.get() + 1);
            Ok(r#"{"tag_name":"v9.0.0","prerelease":false,"draft":false}"#.into())
        }
    }

    /// The app's flow: `tick` decides, the fetch runs only inside `start`.
    fn app_tick(s: &mut Settings, now: u64, f: &dyn Fetch) {
        let mut result = None;
        tick(s, now, false, || result = Some(f.get(LATEST_API)));
        if let Some(r) = result {
            let parsed = r.and_then(|b| parse_latest(&b));
            finish(s, now, &Version::parse("0.3.0").unwrap(), parsed);
        }
    }

    #[test]
    fn disabled_means_no_request_at_all() {
        let calls = Cell::new(0);
        let f = Counting(&calls);
        let mut s = Settings::default();
        assert!(!s.check_for_updates, "off by default");
        for now in [0, 1, 86_400, 10 * 86_400, u32::MAX as u64] {
            app_tick(&mut s, now, &f);
        }
        s.next_update_check = Some(0);
        app_tick(&mut s, 1_000_000, &f);
        assert_eq!(calls.get(), 0);
        assert_eq!(s.available_update, None);
    }

    #[test]
    fn enabled_checks_once_per_day() {
        let calls = Cell::new(0);
        let f = Counting(&calls);
        let mut s = Settings {
            check_for_updates: true,
            ..Settings::default()
        };
        let t0 = 1_800_000_000;
        app_tick(&mut s, t0, &f);
        assert_eq!(calls.get(), 1);
        for dt in [60, 3_600, 86_399] {
            app_tick(&mut s, t0 + dt, &f);
        }
        assert_eq!(calls.get(), 1);
        app_tick(&mut s, t0 + 86_400, &f);
        assert_eq!(calls.get(), 2);
        // Never two at once.
        s.next_update_check = None;
        assert!(!tick(&mut s, t0, true, || panic!("second request while one is running")));
    }

    #[test]
    fn clock_set_back_does_not_stop_checks() {
        let mut s = Settings {
            check_for_updates: true,
            next_update_check: Some(4_000_000_000),
            ..Settings::default()
        };
        assert!(due(&s, 1_800_000_000));
        s.next_update_check = Some(1_800_000_000 + 3_600);
        assert!(!due(&s, 1_800_000_000));
    }

    #[test]
    fn notification_once_per_version_and_menu_only_while_enabled() {
        let cur = Version::parse("0.2.0").unwrap();
        let mut s = Settings {
            check_for_updates: true,
            ..Settings::default()
        };
        let v3 = || Ok(Some(Version::parse("v0.3.0").unwrap()));
        assert!(matches!(finish(&mut s, 0, &cur, v3()), Outcome::Available { notify: true, .. }));
        assert!(matches!(finish(&mut s, 1, &cur, v3()), Outcome::Available { notify: false, .. }));
        assert_eq!(available(&s, &cur), Version::parse("0.3.0"));
        let v4 = Ok(Some(Version::parse("v0.4.0").unwrap()));
        assert!(matches!(finish(&mut s, 2, &cur, v4), Outcome::Available { notify: true, .. }));
        // After upgrading, the stored version is no longer newer.
        assert_eq!(available(&s, &Version::parse("0.4.0").unwrap()), None);
        s.check_for_updates = false;
        assert_eq!(available(&s, &cur), None);
        assert_eq!(finish(&mut s, 3, &cur, v3()), Outcome::Ignored);
    }

    #[test]
    fn failures_retry_within_the_hour() {
        let mut s = Settings {
            check_for_updates: true,
            ..Settings::default()
        };
        tick(&mut s, 100, false, || {});
        assert!(matches!(
            finish(&mut s, 100, &Version::current(), Err("offline".into())),
            Outcome::Failed(_)
        ));
        assert_eq!(s.next_update_check, Some(100 + 3_600));
    }

    /// A release candidate sees the final release (and later ones) as an update, and an
    /// older candidate a newer one; a release never offers a candidate (`/releases/latest`
    /// returns releases only, and their tags must be X.Y.Z).
    #[test]
    fn release_candidates_and_releases() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.4.0-rc.2") < v("0.4.0"));
        assert!(v("0.4.0-rc.1") < v("0.4.0-rc.2"));
        assert!(v("0.4.0-rc.9") < v("0.4.0-rc.10"));
        assert!(v("0.4.0") < v("0.4.1-rc.1"));
        assert!(v("0.4.0-rc.4") < v("0.5.0-rc.1"));
        let found = |latest: &str| Settings {
            check_for_updates: true,
            available_update: Some(latest.into()),
            ..Default::default()
        };
        // Running 0.4.0-rc.2, 0.4.0 is out: update.
        assert_eq!(available(&found("0.4.0"), &v("0.4.0-rc.2")), Some(v("0.4.0")));
        // Running the release itself, or a candidate of the next one: nothing.
        assert_eq!(available(&found("0.4.0"), &v("0.4.0")), None);
        assert_eq!(available(&found("0.4.0"), &v("0.4.1-rc.1")), None);
        // Running an old candidate, a later release is out.
        assert_eq!(available(&found("0.5.0"), &v("0.4.0-rc.4")), Some(v("0.5.0")));
        // The running version comes from the build (with -rc.N when built from such a tag).
        assert_eq!(Version::current().to_string(), battbar_core::version::FULL);
    }

    #[test]
    fn semver_ordering() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-rc.1") > v("1.0.0-beta.9"));
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        assert_eq!(v("v0.2.0"), v("0.2.0+build.5"));
        for bad in ["", "1", "1.2", "1.2.3.4", "1.x.3", "1.2.3-", "v", "01a.2.3"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn pre_releases_and_drafts_are_ignored() {
        let j = |tag: &str, pre: bool, draft: bool| {
            format!(r#"{{"tag_name":"{tag}","prerelease":{pre},"draft":{draft},"html_url":"https://evil.example/"}}"#)
        };
        assert_eq!(parse_latest(&j("v0.3.0", false, false)).unwrap(), Version::parse("0.3.0"));
        assert_eq!(parse_latest(&j("v0.3.0", true, false)).unwrap(), None);
        assert_eq!(parse_latest(&j("v0.3.0", false, true)).unwrap(), None);
        assert_eq!(parse_latest(&j("v0.3.0-rc.1", true, false)).unwrap(), None);
        // Not flagged as pre-release but not a plain vX.Y.Z tag: the check fails.
        assert!(parse_latest(&j("v0.3.0-rc.1", false, false)).is_err());
        assert!(parse_latest(&j("nightly", false, false)).is_err());
        assert!(parse_latest("<html>").is_err());
        assert!(parse_latest(r#"{"tag_name":5}"#).is_err());
        // The page is built from the version, never from html_url.
        assert_eq!(
            release_page(&Version::parse("0.3.0").unwrap()),
            "https://github.com/StanislavTugarev/battbar/releases/tag/v0.3.0"
        );
    }

    #[test]
    fn release_tag_must_be_strictly_v_x_y_z() {
        assert_eq!(parse_release_tag("v0.3.0"), Ok(Version::parse("0.3.0").unwrap()));
        assert_eq!(parse_release_tag("v10.20.30").unwrap().to_string(), "10.20.30");
        let long = format!("v{}.0.0", "9".repeat(10_000));
        let overflow = format!("v{}.0.0", "9".repeat(25)); // short enough, but > u64
        let junk = [
            "",
            "v",
            "0.3.0",
            "V0.3.0",
            "v0.3",
            "v0.3.0.1",
            "v0..3",
            "v.0.3.0",
            "v0.3.0-rc.1",
            "v0.3.0+build",
            " v0.3.0",
            "v0.3.0 ",
            "v0.3.0\n",
            "v0.3.0\t",
            "v 0.3.0",
            "../v0.3.0",
            "v0.3.0/../../evil",
            "v0.3.0/",
            "v0.3.0?x=1",
            "v0.3.0#a",
            "v-1.0.0",
            "v+1.0.0",
            "v0x1.0.0",
            "v\u{0660}.\u{0663}.\u{0660}", // Arabic-Indic digits
            "v\u{FF10}.\u{FF13}.\u{FF10}", // fullwidth digits
            "\u{FF56}0.3.0",               // fullwidth v
            "v0.3.0\u{200B}",              // zero-width space
            "v\u{0}0.3.0",
            "v0.3.0\u{0}",
            "в0.3.0", // Cyrillic
            long.as_str(),
            overflow.as_str(),
        ];
        for t in junk {
            assert!(parse_release_tag(t).is_err(), "{t:?} accepted");
            // Through the API response as well, and the error never echoes the tag.
            let body = serde_json::json!({ "tag_name": t, "prerelease": false, "draft": false }).to_string();
            let e = parse_latest(&body).unwrap_err();
            assert!(t.len() < 4 || !e.contains(t), "error echoes the tag: {e}");
        }
    }

    #[test]
    fn homebrew_prefixes_cover_both_defaults_and_the_environment() {
        use std::path::PathBuf;
        let defaults: Vec<PathBuf> = DEFAULT_HOMEBREW_PREFIXES.iter().map(PathBuf::from).collect();
        assert_eq!(homebrew_prefixes(None), defaults);
        assert_eq!(homebrew_prefixes(Some("relative/brew".into())), defaults);
        assert_eq!(
            homebrew_prefixes(Some("/usr/local".into())),
            vec![PathBuf::from("/usr/local"), PathBuf::from("/opt/homebrew")],
            "environment first, no duplicates"
        );
        let custom = homebrew_prefixes(Some("/Users/x/homebrew".into()));
        assert_eq!(custom[0], PathBuf::from("/Users/x/homebrew"));
        assert_eq!(&custom[1..], &defaults[..]);
    }

    #[test]
    fn homebrew_install_found_under_either_prefix() {
        let root = std::env::temp_dir().join(format!("battbar-brew-test-{}", std::process::id()));
        let arm = root.join("opt/homebrew");
        let intel = root.join("usr/local");
        let custom = root.join("custom");
        for p in [&arm, &intel, &custom] {
            std::fs::create_dir_all(p.join("Caskroom/other")).unwrap();
        }
        let all = vec![arm.clone(), intel.clone(), custom.clone()];
        assert!(!installed_with_homebrew_in(&all), "other casks do not count");
        for p in [&arm, &intel, &custom] {
            std::fs::create_dir_all(p.join("Caskroom/battbar/0.2.0")).unwrap();
            assert!(installed_with_homebrew_in(&all), "{}", p.display());
            assert!(installed_with_homebrew_in(std::slice::from_ref(p)));
            std::fs::remove_dir_all(p.join("Caskroom/battbar")).unwrap();
        }
        // A file named battbar is not an installation.
        std::fs::write(intel.join("Caskroom/battbar"), "").unwrap();
        assert!(!installed_with_homebrew_in(&all));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn malformed_tag_shows_nothing() {
        let mut s = Settings {
            check_for_updates: true,
            ..Settings::default()
        };
        let body = r#"{"tag_name":"v9.9.9-evil","prerelease":false,"draft":false}"#;
        let o = finish(&mut s, 0, &Version::parse("0.3.0").unwrap(), parse_latest(body));
        assert!(matches!(o, Outcome::Failed(_)));
        assert_eq!(s.available_update, None);
        assert_eq!(s.update_notified, None);
        assert_eq!(available(&s, &Version::parse("0.3.0").unwrap()), None);
    }

    #[test]
    fn curl_sends_only_user_agent_and_accept() {
        let a = Curl::args(LATEST_API);
        assert_eq!(a[0], "-q", "-q must come first to ignore ~/.curlrc");
        let headers: Vec<&String> = a
            .iter()
            .zip(a.iter().skip(1))
            .filter(|(k, _)| *k == "--header" || *k == "--user-agent")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(headers.len(), 2);
        assert!(headers[0].starts_with("battbar/"));
        assert_eq!(headers[1], "Accept: application/vnd.github+json");
        assert!(a.contains(&"=https".to_string()));
        for forbidden in [
            "--cookie",
            "-b",
            "--user",
            "-u",
            "Authorization",
            "--data",
            "-d",
            "--location",
            "-L",
        ] {
            assert!(!a.iter().any(|x| x == forbidden), "{forbidden}");
        }
        assert_eq!(a.last().unwrap(), LATEST_API);
    }
}
