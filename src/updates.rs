//! Whether a newer Null is out, when the person asked to be told (off until then). One
//! request to GitHub, at most once a day: the latest release's name, nothing sent about the
//! person or the computer beyond what any web request carries. Null never downloads or
//! installs an update itself: it says one is out, and where to get it.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

const LATEST: &str = "https://api.github.com/repos/ShortPlanet3058/null-ide/releases/latest";
/// Where to get it.
pub const DOWNLOAD: &str = "https://null-ide.com/#download";
/// Not asked again sooner than this.
const EVERY_SECS: u64 = 20 * 3600;

/// What the last check found, kept in Null's folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    /// When (seconds since 1970).
    checked: u64,
    /// The latest release's version ("0.2.0").
    latest: Option<String>,
}

/// A newer version found this session, to show.
static NEWER: Mutex<Option<String>> = Mutex::new(None);

/// This build's version.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Whether `latest` ("v0.2.0", "0.2.0") is newer than `current` ("0.1.1").
pub fn newer(current: &str, latest: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim().trim_start_matches('v').split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (parts(current), parts(latest));
    (0..3).map(|i| b.get(i).copied().unwrap_or(0).cmp(&a.get(i).copied().unwrap_or(0))).find(|o| o.is_ne())
        == Some(std::cmp::Ordering::Greater)
}

fn file() -> Option<std::path::PathBuf> {
    Some(crate::tools::data_dir()?.join("update.json"))
}

fn kept() -> Kept {
    file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether it's time to ask again.
pub fn due() -> bool {
    now().saturating_sub(kept().checked) >= EVERY_SECS
}

/// The newer version known, if there is one (see `remember`, `check`).
pub fn known() -> Option<String> {
    NEWER.lock().ok().and_then(|n| n.clone())
}

/// What the last check found, known again (a newer version found yesterday is still out).
pub fn remember() {
    let found = kept().latest.filter(|l| newer(current(), l));
    if let (Some(found), Ok(mut known)) = (found, NEWER.lock()) {
        known.get_or_insert(found);
    }
}

/// True the first time it's asked in this run: the check is made once, however many
/// windows there are.
pub fn first_this_run() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ASKED: AtomicBool = AtomicBool::new(false);
    !ASKED.swap(true, Ordering::Relaxed)
}

/// Asks GitHub for the latest release. Slow (a request): off the main thread. Ok(Some) when
/// it's newer than this one.
pub fn check() -> Result<Option<String>, String> {
    // (A QA run asks nothing of the network.)
    if crate::system_clipboard::kept_apart() {
        return Ok(None);
    }
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(15))).build().into();
    let response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", &format!("Null/{}", current()))
        .call()
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    let release: serde_json::Value =
        response.into_body().read_json().map_err(|e| format!("GitHub's answer couldn't be read: {e}"))?;
    let latest = release["tag_name"].as_str().ok_or("GitHub's answer has no version")?;
    let latest = latest.trim_start_matches('v').to_string();
    let kept = Kept { checked: now(), latest: Some(latest.clone()) };
    if let (Some(path), Ok(json)) = (file(), serde_json::to_string(&kept)) {
        // (Null's folder may not be there yet, on a first launch.)
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json);
    }
    let found = newer(current(), &latest).then_some(latest);
    if let Ok(mut newer) = NEWER.lock() {
        newer.clone_from(&found);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_newer_version() {
        assert!(newer("0.1.1", "v0.2.0"));
        assert!(newer("0.1.1", "0.1.2"));
        assert!(newer("0.9.9", "1.0.0"));
        assert!(!newer("0.2.0", "v0.2.0"));
        assert!(!newer("0.2.0", "0.1.9"));
        assert!(!newer("0.2.0", "0.2"));
        assert!(newer("0.2", "0.2.1"));
    }

    /// Asks GitHub for real: `cargo test asks_github -- --ignored`.
    #[test]
    #[ignore]
    fn asks_github_for_its_latest_release() {
        let found = check().unwrap();
        assert!(!due(), "the check is kept");
        assert_eq!(found.is_some(), known().is_some());
        println!("latest newer than {}: {found:?}", current());
    }
}
