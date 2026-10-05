//! Update check: the latest release from the GitHub Releases API, asked
//! only when enabled in About (at most once a day, at start-up) or with its
//! "Check now" button. Nothing is downloaded: a newer version is offered
//! as a toolbar button that opens its release page.

use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// Cargo's version, without the commit `main::VERSION` adds: between
/// releases the next one with `-dev`.
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const API_HOST: &str = "api.github.com";
/// Automatic checks are at least this far apart, in seconds.
pub const INTERVAL: u64 = 24 * 60 * 60;
/// Resolving, connecting, sending and receiving each give up after this.
const TIMEOUT_MS: i32 = 10_000;
/// A release's JSON is a few kilobytes, its notes included.
const MAX_ANSWER: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Not checked in this run.
    Unknown,
    Checking,
    UpToDate,
    /// A newer release, by its tag (`v0.5.0`).
    Newer(String),
    /// Why the check failed (technical, in English).
    Failed(String),
}

pub struct Updates {
    /// Check at start-up, at most once per [`INTERVAL`] (a setting).
    pub enabled: bool,
    /// Unix seconds of the last check that got an answer (a setting).
    pub last_check: u64,
    pub status: Status,
    rx: Option<mpsc::Receiver<Result<String, String>>>,
}

impl Updates {
    pub fn new(enabled: bool, last_check: u64) -> Self {
        Self { enabled, last_check, status: Status::Unknown, rx: None }
    }

    /// Check now on a background thread, unless a check is running.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("update check".into()).spawn(move || {
            let _ = tx.send(latest_tag());
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => {
                self.rx = Some(rx);
                self.status = Status::Checking;
            }
            Err(e) => self.status = Status::Failed(e.to_string()),
        }
    }

    /// Check if enabled and the last check is older than [`INTERVAL`].
    pub fn start_if_due(&mut self, ctx: &egui::Context) {
        if self.enabled && due(now(), self.last_check) {
            self.start(ctx);
        }
    }

    /// Take the result of a finished check.
    pub fn poll(&mut self) {
        let Some(result) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.rx = None;
        self.status = match result {
            Ok(tag) => {
                self.last_check = now();
                match is_newer(&tag, CURRENT) {
                    Some(true) => Status::Newer(tag),
                    Some(false) => Status::UpToDate,
                    None => Status::Failed(format!("unexpected release tag {tag:?}")),
                }
            }
            Err(e) => {
                log::info!("update check failed: {e}");
                Status::Failed(e)
            }
        };
    }

    /// Tag of a newer release, if one was found.
    pub fn newer(&self) -> Option<&str> {
        match &self.status {
            Status::Newer(tag) => Some(tag),
            _ => None,
        }
    }
}

/// Page of the release with `tag`.
pub fn release_url(tag: &str) -> String {
    format!("{REPOSITORY}/releases/tag/{tag}")
}

/// `v0.5.0` shown as `0.5.0`.
pub fn version_of(tag: &str) -> &str {
    tag.trim_start_matches(['v', 'V'])
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether an automatic check is due; a clock set back counts as due.
fn due(now: u64, last: u64) -> bool {
    now < last || now - last >= INTERVAL
}

/// Tag of the latest release: drafts and pre-releases are left out by the
/// API itself.
fn latest_tag() -> Result<String, String> {
    let repo = REPOSITORY.strip_prefix("https://github.com/").ok_or("the repository is not on GitHub")?;
    let (status, body) = crate::win::https_get(
        API_HOST,
        &format!("/repos/{repo}/releases/latest"),
        &format!("qview/{CURRENT}"),
        "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28",
        TIMEOUT_MS,
        MAX_ANSWER,
    )?;
    match status {
        200 => {}
        404 => return Err("no release is published".into()),
        403 | 429 => return Err("GitHub refused the request (rate limit), try later".into()),
        s => return Err(format!("GitHub answered with HTTP {s}")),
    }
    let text = String::from_utf8_lossy(&body);
    tag_name(&text).map(String::from).ok_or_else(|| "no tag_name in the answer".into())
}

/// The `tag_name` string of a release's JSON. The key occurs once, at the
/// top level (assets and the author have `name`, not `tag_name`); a tag
/// with escapes is not a version anyway.
fn tag_name(json: &str) -> Option<&str> {
    const KEY: &str = "\"tag_name\"";
    let rest = &json[json.find(KEY)? + KEY.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let tag = &rest[..rest.find('"')?];
    (!tag.contains('\\')).then_some(tag)
}

/// `v1.2.3` or `1.2.3` as numbers, and whether it is a release: `1.2.3-dev`,
/// the version between releases, comes before `1.2.3`. Any other suffix is
/// not understood.
fn parse_version(s: &str) -> Option<(u32, u32, u32, bool)> {
    let s = version_of(s.trim());
    let (numbers, release) = match s.strip_suffix("-dev") {
        Some(n) => (n, false),
        None => (s, true),
    };
    let mut parts = numbers.split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??, release);
    parts.next().is_none().then_some(v)
}

/// Whether release `tag` is newer than `current`; `None` if either is not
/// understood. Releases are never `-dev`: only a build between them is.
fn is_newer(tag: &str, current: &str) -> Option<bool> {
    let release = parse_version(tag).filter(|v| v.3)?;
    Some(release > parse_version(current)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_tag() {
        // Shortened answer of /releases/latest, nested objects included.
        let json = r#"{
  "url": "https://api.github.com/repos/Fan4Metal/qview/releases/1",
  "html_url": "https://github.com/Fan4Metal/qview/releases/tag/v0.2.0",
  "author": { "login": "Fan4Metal", "html_url": "https://github.com/Fan4Metal" },
  "tag_name" : "v0.2.0",
  "name": "qview 0.2.0",
  "assets": [ { "name": "qview_0.2.0_Setup.exe" } ]
}"#;
        assert_eq!(tag_name(json), Some("v0.2.0"));
        assert_eq!(tag_name(r#"{"tag_name":"v1.0.0"}"#), Some("v1.0.0"));
        assert_eq!(tag_name(r#"{"name":"v1.0.0"}"#), None);
        assert_eq!(tag_name(r#"{"tag_name": 5}"#), None);
        assert_eq!(tag_name(r#"{"tag_name": "a\"b"}"#), None);
    }

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0, true)));
        assert_eq!(parse_version("0.3.0-dev"), Some((0, 3, 0, false)));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        assert_eq!(parse_version("v1.2.3-beta"), None);
        assert_eq!(is_newer("v0.4.1", "0.4.0"), Some(true));
        assert_eq!(is_newer("v0.10.0", "0.9.9"), Some(true));
        assert_eq!(is_newer("v0.4.0", "0.4.0"), Some(false));
        assert_eq!(is_newer("v0.3.9", "0.4.0"), Some(false));
        // Between releases: after the last one, before the next.
        assert_eq!(is_newer("v0.2.0", "0.3.0-dev"), Some(false));
        assert_eq!(is_newer("v0.3.0", "0.3.0-dev"), Some(true));
        assert_eq!(is_newer("nightly", "0.4.0"), None);
        assert_eq!(is_newer("v0.4.0-dev", "0.3.0"), None);
        assert_eq!(version_of("v0.5.0"), "0.5.0");
    }

    /// Asks GitHub for real: `cargo test update -- --ignored`.
    #[test]
    #[ignore = "needs the network"]
    fn asks_github() {
        let tag = latest_tag().unwrap();
        assert!(parse_version(&tag).is_some(), "{tag}");
    }

    #[test]
    fn checks_once_a_day() {
        assert!(due(INTERVAL, 0));
        assert!(!due(1000 + INTERVAL - 1, 1000));
        assert!(due(1000 + INTERVAL, 1000));
        assert!(due(500, 1000));
    }
}
