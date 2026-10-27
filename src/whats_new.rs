//! What changed since the version Null last ran: told once after an update (a notice; a
//! click on it, or What's New in ⌘K, shows the changes), from the changelog it's built with.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// The changelog's sections for the versions after `since`, up to `current`, newest first;
/// `current`'s alone when `since` isn't known.
fn changes_in(changelog: &str, since: Option<&str>, current: &str) -> String {
    use crate::updates::newer;
    let mut out = Vec::new();
    for section in changelog.split("\n## ").skip(1) {
        let (version, body) = section.split_once('\n').unwrap_or((section, ""));
        let version = version.trim();
        // ("Unreleased", and versions after this one.)
        if !version.starts_with(|c: char| c.is_ascii_digit()) || newer(current, version) {
            continue;
        }
        let wanted = match since {
            Some(since) => newer(since, version),
            None => version == current,
        };
        if wanted {
            out.push((version, body.trim()));
        }
    }
    // (One version: under the title, which names it.)
    match out.as_slice() {
        [(_, body)] => format!("{body}\n"),
        _ => out.iter().map(|(version, body)| format!("## {version}\n\n{body}\n\n")).collect(),
    }
}

/// What's New: the changes since the version that ran before this one (this one's, when
/// it's not known), as Markdown.
pub fn text(since: Option<&str>) -> String {
    let current = crate::updates::current();
    let changes = changes_in(CHANGELOG, since, current);
    let changes = if changes.is_empty() { "Nothing written down for this version.\n".into() } else { changes };
    format!("# What's new in Null {current}\n\n{changes}")
}

fn last_run_file() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("last-version"))
}

/// Once a run (the first time it's asked): the version that ran before this one, read,
/// and this one kept as the last that ran in its place. None: none had run (or the same).
fn before() -> &'static Option<String> {
    static BEFORE: OnceLock<Option<String>> = OnceLock::new();
    BEFORE.get_or_init(|| {
        let file = last_run_file()?;
        let before = std::fs::read_to_string(&file).ok().map(|v| v.trim().to_string());
        let current = crate::updates::current();
        if before.as_deref() != Some(current) {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            std::fs::write(&file, current).ok();
        }
        before
    })
}

/// The version updated from, when this one's newer: kept from the one before (None for
/// one that didn't keep it: 0.2.0 and before, known to have run by its update check).
pub fn since() -> Option<Option<String>> {
    match before() {
        Some(before) => crate::updates::newer(before, crate::updates::current()).then(|| Some(before.clone())),
        None => crate::updates::checked_before().then_some(None),
    }
}

/// Once a run, in the first window: `since`, to tell about the update.
pub fn to_tell() -> Option<Option<String>> {
    static TOLD: AtomicBool = AtomicBool::new(false);
    let updated = since();
    if TOLD.swap(true, Ordering::Relaxed) { None } else { updated }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Changelog\n\n## Unreleased\n\n- soon\n\n## 0.3.0\n\n- three\n\n## 0.2.1\n\n- two one\n\n## 0.2.0\n\n- two\n\n## 0.1.1\n\n- one\n";

    #[test]
    fn the_changes_since_the_version_before_up_to_this_one() {
        assert_eq!(changes_in(LOG, Some("0.1.1"), "0.2.1"), "## 0.2.1\n\n- two one\n\n## 0.2.0\n\n- two\n\n");
        // Not known: this one's.
        assert_eq!(changes_in(LOG, None, "0.2.0"), "- two\n");
        assert_eq!(changes_in(LOG, Some("0.2.1"), "0.2.1"), "");
        // The real changelog has this version's section.
        assert!(!text(None).contains("Nothing written down"));
    }
}
