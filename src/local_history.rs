//! What a file held before Null wrote over it, kept for a while in the data folder: going
//! back needs no git, and works between commits. A burst of saves (auto-save while typing)
//! keeps one version a minute; a file keeps its last 50, for 30 days. Big files aren't kept.

use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 1024 * 1024;
const KEEP: usize = 50;
const DAYS: u64 = 30;
/// Versions closer together than this make one: the later replaces the earlier.
const BURST_MS: i64 = 60_000;

/// A version kept: when the file held it, when it was kept, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// Unix milliseconds: when the file was last written with this, before it was replaced.
    pub time: i64,
    /// Unix milliseconds: when it was replaced (and kept). Old versions go by this, so an
    /// old file's first version isn't gone the moment it's kept.
    kept: i64,
    file: PathBuf,
}

impl Saved {
    /// The text, in the file's own encoding.
    pub fn text(&self, encoding: crate::encoding::Encoding) -> Option<String> {
        crate::encoding::decode_as(std::fs::read(&self.file).ok()?, encoding)
    }
}

/// The folder a file's versions are kept in. Named by a hash that stays the same from one
/// version of Null to the next (so not std's hasher), with the file's name to read it by.
fn folder(path: &Path) -> Option<PathBuf> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_os_str().as_encoded_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    let name = path.file_name()?.to_string_lossy();
    let safe: String =
        name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    Some(crate::tools::data_dir()?.join("history").join(format!("{safe}-{hash:016x}")))
}

fn millis(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// About to write `new` over the file at `path` (its real path, links followed): what it
/// holds now is kept, unless it's big, the same, or what was kept last.
pub fn keep_before_writing(path: &Path, new: &[u8]) {
    keep_at(path, new, millis(std::time::SystemTime::now()));
}

/// `keep_before_writing`, `now` being the time (Unix milliseconds).
fn keep_at(path: &Path, new: &[u8], now: i64) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if !meta.is_file() || meta.len() > MAX_BYTES || meta.len() == 0 {
        return;
    }
    let Ok(old) = std::fs::read(path) else { return };
    if old == new {
        return;
    }
    let Some(dir) = folder(path) else { return };
    let time = meta.modified().map(millis).unwrap_or(0);
    let kept = versions(path);
    if let Some(last) = kept.first() {
        if std::fs::read(&last.file).is_ok_and(|bytes| bytes == old) {
            return;
        }
        // Saves in a burst: within a minute of the one before the last, this one stands for
        // the last. So about one a minute is kept, and how the file was before is still there.
        if kept.get(1).is_some_and(|before| (0..BURST_MS).contains(&(now - before.kept))) {
            std::fs::remove_file(&last.file).ok();
        }
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    std::fs::write(dir.join("path"), path.as_os_str().as_encoded_bytes()).ok();
    let mut kept_at = now;
    let mut file = dir.join(format!("{kept_at}-{time}"));
    while file.exists() {
        kept_at += 1;
        file = dir.join(format!("{kept_at}-{time}"));
    }
    std::fs::write(&file, &old).ok();
    prune(path, now);
}

/// The versions kept of the file at `path`, newest first.
pub fn versions(path: &Path) -> Vec<Saved> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Some(dir) = folder(&path) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut found: Vec<Saved> = entries.filter_map(Result::ok).filter_map(|e| saved(e.path())).collect();
    found.sort_by_key(|a| std::cmp::Reverse(a.kept));
    found
}

/// A version from its file's name: "kept-time" (or one time, as the first ones were named).
fn saved(file: PathBuf) -> Option<Saved> {
    let name = file.file_name()?.to_str()?.to_string();
    let (kept, time) = match name.split_once('-') {
        Some((kept, time)) => (kept.parse().ok()?, time.parse().ok()?),
        // Named before versions said when they were kept: kept when the copy was written.
        None => {
            let time = name.parse().ok()?;
            let kept = std::fs::metadata(&file).and_then(|m| m.modified()).map(millis).unwrap_or(time);
            (kept, time)
        }
    };
    Some(Saved { time, kept, file })
}

/// Before this, versions are too old to keep.
fn oldest(now: i64) -> i64 {
    now - (DAYS * 24 * 3600 * 1000) as i64
}

/// Only the last ones, and only from the last month.
fn prune(path: &Path, now: i64) {
    for (i, saved) in versions(path).into_iter().enumerate() {
        if i >= KEEP || saved.kept < oldest(now) {
            std::fs::remove_file(&saved.file).ok();
        }
    }
}

/// Versions past the month for every file (those deleted or renamed since too), and the
/// folders left empty. Run now and then, off the main thread.
pub fn sweep() {
    let Some(history) = crate::tools::data_dir().map(|d| d.join("history")) else { return };
    let Ok(folders) = std::fs::read_dir(&history) else { return };
    let oldest = oldest(millis(std::time::SystemTime::now()));
    for folder in folders.filter_map(Result::ok).map(|e| e.path()) {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut left = 0;
        for saved in entries.filter_map(Result::ok).filter_map(|e| saved(e.path())) {
            if saved.kept < oldest {
                std::fs::remove_file(&saved.file).ok();
            } else {
                left += 1;
            }
        }
        if left == 0 {
            std::fs::remove_file(folder.join("path")).ok();
            std::fs::remove_dir(&folder).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_time(path: &Path, ms: i64) {
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms as u64);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    /// Each write keeps what it replaced; a burst keeps about one a minute; the version
    /// from before it stays, even when the file was last changed months ago.
    #[test]
    fn keeps_what_each_write_replaced() {
        let dir = crate::tools::test_dir("local-history");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = std::fs::canonicalize(&dir).unwrap().join("notes.txt");
        let now = millis(std::time::SystemTime::now());
        let write = |text: &str, at: i64| {
            keep_at(&file, text.as_bytes(), at);
            std::fs::write(&file, text).unwrap();
            set_time(&file, at);
        };
        // Last changed two months ago.
        std::fs::write(&file, "long ago\n").unwrap();
        set_time(&file, now - 60 * 86_400_000);
        write("a\n", now - 50_000);
        write("ab\n", now - 40_000);
        write("abc\n", now - 30_000);
        write("abcd\n", now);
        let texts: Vec<String> =
            versions(&file).iter().map(|s| s.text(crate::encoding::Encoding::default()).unwrap()).collect();
        // All within a minute: the latest replaced stands for the burst. The first stays.
        assert_eq!(texts, ["abc\n", "long ago\n"]);
        // Shown by when the file held it.
        assert_eq!(versions(&file).last().unwrap().time, now - 60 * 86_400_000);
        // The same text again keeps nothing new.
        keep_at(&file, b"abcd\n", now + 1);
        assert_eq!(versions(&file).len(), 2);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(folder(&file).unwrap()).ok();
    }
}
