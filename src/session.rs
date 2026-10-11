//! What a project looked like when it was last closed, to open it the same way:
//! its tabs (with the caret and scroll of each), the folders open in the tree, the
//! terminal, and the window. One small file per project, in Null's own folder.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<TabState>,
    pub active: Option<usize>,
    /// The tab the right side showed, when the window was split.
    pub shown_right: Option<usize>,
    /// How much of the width the left side took, when split (of the height, stacked).
    pub split_ratio: Option<f32>,
    /// The terminal's height, when dragged to another.
    pub terminal_height: Option<f32>,
    /// Whether the two sides were one above the other.
    pub stacked: bool,
    /// Folders expanded in the file tree.
    pub expanded: Vec<PathBuf>,
    /// Files opened lately, most recent first, for ⌘P.
    pub recent_files: Vec<PathBuf>,
    pub terminal_open: bool,
    pub window: Option<WindowState>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    /// The first line shown, so the view comes back where it was.
    pub top_line: usize,
    /// 1 for the right side of a split window.
    pub side: usize,
    /// Folded regions, as (first line, last line).
    pub folds: Vec<(usize, usize)>,
    /// Lines with a breakpoint.
    pub breakpoints: Vec<usize>,
    /// Breakpoints that only stop when something holds: (line, condition).
    pub conditions: Vec<(usize, String)>,
    /// Lines with a bookmark.
    pub bookmarks: Vec<usize>,
    /// The language chosen for the file, when it isn't the one its name says.
    pub language: Option<String>,
    /// Pinned: kept first, and out of "Close Others".
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

fn sessions_dir() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("sessions"))
}

/// A project's session file: named after its folder, plus a hash of its full path so
/// two `src` folders don't share one.
fn file_for(root: &Path) -> Option<PathBuf> {
    Some(file_in(&sessions_dir()?, root))
}

fn file_in(dir: &Path, root: &Path) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hasher);
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "root".into());
    let safe: String =
        name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    dir.join(format!("{safe}-{:016x}.json", hasher.finish()))
}

impl Session {
    /// The project's last session, keeping only what still exists.
    pub fn load(root: &Path) -> Self {
        file_for(root).map(|f| Self::load_file(&f)).unwrap_or_default()
    }

    fn load_file(file: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(file) else { return Self::default() };
        let mut session: Self = serde_json::from_str(&text).unwrap_or_default();
        let before = session.tabs.len();
        let active_path = session.active.and_then(|i| session.tabs.get(i)).map(|t| t.path.clone());
        let right_path = session.shown_right.and_then(|i| session.tabs.get(i)).map(|t| t.path.clone());
        session.tabs.retain(|t| t.path.is_file());
        session.shown_right = right_path.and_then(|p| session.tabs.iter().position(|t| t.path == p));
        session.active = active_path
            .and_then(|p| session.tabs.iter().position(|t| t.path == p))
            .or_else(|| (before > 0 && !session.tabs.is_empty()).then_some(0));
        session.expanded.retain(|p| p.is_dir());
        session.recent_files.retain(|p| p.is_file());
        session
    }

    /// Saves it, quietly: losing a session is a small thing, an error dialog would be worse.
    pub fn save(&self, root: &Path) {
        let (Some(file), Ok(text)) = (file_for(root), serde_json::to_string_pretty(self)) else { return };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        // Write then rename, so a crash mid-write can't leave half a file.
        let temp = file.with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            std::fs::rename(&temp, &file).ok();
        }
        remember_last_project(root);
    }
}

/// Unsaved work, kept in Null's own folder until it's saved or let go, so a crash or a
/// forced quit doesn't lose it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    /// The file it's the unsaved text of; None for a new file with no name yet.
    pub path: Option<PathBuf>,
    pub text: String,
    /// The file on disk when the backup was made, to tell if it changed since.
    pub disk: Option<u64>,
}

/// A fingerprint of a file's contents (None when there's no such file).
pub fn disk_fingerprint(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

/// The project's backups of each Null running it (a build and the app at once, say):
/// `<project>.<pid>.json`, so one leaving doesn't remove the other's. (`<project>.json`
/// is how they were kept before: read as one left by a Null that's gone.)
fn backups_base(root: &Path) -> Option<PathBuf> {
    Some(file_in(&crate::tools::data_dir()?.join("backups"), root))
}

fn own_backups_file(root: &Path) -> Option<PathBuf> {
    Some(backups_base(root)?.with_extension(format!("{}.json", std::process::id())))
}

/// The project's backup files, and the process that keeps each (None: from before).
fn backup_files(root: &Path) -> Vec<(PathBuf, Option<u32>)> {
    let Some(base) = backups_base(root) else { return Vec::new() };
    let (Some(dir), Some(stem)) = (base.parent(), base.file_stem().map(|s| s.to_string_lossy().into_owned())) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| {
            let path = e.ok()?.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            let rest = name.strip_prefix(&stem)?.strip_suffix(".json")?;
            match rest {
                "" => Some((path, None)),
                _ => Some((path.clone(), Some(rest.strip_prefix('.')?.parse().ok()?))),
            }
        })
        .collect()
}

/// The lock a Null holds while it runs: `backups/running.<pid>.lock`.
fn lock_file(pid: u32) -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("backups").join(format!("running.{pid}.lock")))
}

/// Takes this Null's lock, held until it ends (the system lets go of it however it ends):
/// another Null tells it's still running by that, not by its number, which a process
/// started since may have been given.
fn hold_own_lock() {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        static HELD: std::sync::OnceLock<Option<std::fs::File>> = std::sync::OnceLock::new();
        HELD.get_or_init(|| {
            let path = lock_file(std::process::id())?;
            std::fs::create_dir_all(path.parent()?).ok()?;
            let file = std::fs::File::create(&path).ok()?;
            (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(file)
        });
    }
}

/// Whether process `pid` is a Null still running.
fn running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // Its lock: held, it runs; free, it's gone (and its number may be another's now).
        if let Some(path) = lock_file(pid)
            && let Ok(file) = std::fs::File::open(&path)
        {
            let free = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
            if free {
                std::fs::remove_file(&path).ok();
            }
            return !free;
        }
        // No lock (a Null from before them): whether any process has that number. Signal 0
        // only asks: there, or there but someone else's.
        unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Keeps `backups` as this Null's unsaved work in the project; none removes its file.
pub fn save_backups(root: &Path, backups: &[Backup]) {
    hold_own_lock();
    let Some(file) = own_backups_file(root) else { return };
    if backups.is_empty() {
        std::fs::remove_file(&file).ok();
        return;
    }
    let Ok(text) = serde_json::to_string(backups) else { return };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    // Write then rename: a crash mid-write mustn't lose the backup it replaces. A file of
    // its own for each write, so two at once (typing, and quitting) can't mix.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = file.with_extension(format!("json.{}-{n}.tmp", std::process::id()));
    if std::fs::write(&temp, text).is_ok() {
        std::fs::rename(&temp, &file).ok();
    }
}

/// The unsaved work left from last time (a crash, or a forced quit): this Null's, and
/// that of any that's gone, taken over (kept in this one's file first, then theirs
/// removed). A Null still running keeps its own.
pub fn load_backups(root: &Path) -> Vec<Backup> {
    hold_own_lock();
    let own = std::process::id();
    let mut files: Vec<PathBuf> = backup_files(root)
        .into_iter()
        .filter(|(_, pid)| pid.is_none_or(|pid| pid == own || !running(pid)))
        .map(|(file, _)| file)
        .collect();
    // Newest first: of two kept for one file, the later is the one.
    files.sort_by_key(|f| std::cmp::Reverse(std::fs::metadata(f).and_then(|m| m.modified()).ok()));
    let mut backups: Vec<Backup> = Vec::new();
    for file in &files {
        let read: Vec<Backup> =
            std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for backup in read {
            if backup.path.is_none() || !backups.iter().any(|b| b.path == backup.path) {
                backups.push(backup);
            }
        }
    }
    let own_file = own_backups_file(root);
    if files.iter().any(|f| Some(f) != own_file.as_ref()) {
        save_backups(root, &backups);
        for file in files.iter().filter(|f| Some(*f) != own_file.as_ref()) {
            std::fs::remove_file(file).ok();
        }
    }
    backups
}

fn last_project_file() -> Option<PathBuf> {
    Some(sessions_dir()?.join("last-project"))
}

fn remember_last_project(root: &Path) {
    if let Some(file) = last_project_file() {
        std::fs::write(file, root.to_string_lossy().as_bytes()).ok();
    }
    remember_recent_project(root);
}

/// How many projects Open Recent remembers.
const RECENT_PROJECTS: usize = 20;

fn recent_projects_file() -> Option<PathBuf> {
    Some(sessions_dir()?.join("recent-projects.json"))
}

fn read_recent(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn remember_recent_project(root: &Path) {
    if let Some(file) = recent_projects_file() {
        remember_in(&file, root);
    }
}

/// Puts `root` first in the list kept in `file`.
fn remember_in(file: &Path, root: &Path) {
    let mut recent = read_recent(file);
    if recent.first().is_some_and(|p| p == root) {
        return;
    }
    recent.retain(|p| p != root);
    recent.insert(0, root.to_path_buf());
    recent.truncate(RECENT_PROJECTS);
    if let Ok(text) = serde_json::to_string_pretty(&recent) {
        std::fs::write(file, text).ok();
    }
}

/// Takes `root` off the recent projects (it stays on disk).
pub fn forget_project(root: &Path) {
    if let Some(file) = recent_projects_file() {
        forget_in(&file, root);
    }
}

fn forget_in(file: &Path, root: &Path) {
    let mut recent = read_recent(file);
    let before = recent.len();
    recent.retain(|p| p != root);
    if recent.len() != before
        && let Ok(text) = serde_json::to_string_pretty(&recent)
    {
        std::fs::write(file, text).ok();
    }
}

/// Projects opened lately, most recent first, that are still there.
pub fn recent_projects() -> Vec<PathBuf> {
    let Some(file) = recent_projects_file() else { return Vec::new() };
    read_recent(&file).into_iter().filter(|p| p.is_dir()).collect()
}

/// The project open when Null last closed, if it's still there.
pub fn last_project() -> Option<PathBuf> {
    let path = PathBuf::from(std::fs::read_to_string(last_project_file()?).ok()?.trim());
    path.is_dir().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two Nulls on one project keep their own unsaved work: one's file isn't the other's,
    /// and only that of a Null that's gone is taken over (and kept before its file goes).
    #[test]
    fn each_null_keeps_its_own_backups() {
        let root = crate::tools::test_dir("backups-two").join("project");
        let base = backups_base(&root).unwrap();
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        let backup = |name: &str| Backup { path: Some(root.join(name)), text: name.into(), disk: None };
        let write =
            |file: &Path, backups: &[Backup]| std::fs::write(file, serde_json::to_string(backups).unwrap()).unwrap();
        // One kept the old way, one by a Null that's gone, one by a Null still running (launchd).
        write(&base, &[backup("old.txt")]);
        let gone = base.with_extension("999999.json");
        write(&gone, &[backup("gone.txt")]);
        let running = base.with_extension("1.json");
        write(&running, &[backup("running.txt")]);
        let mut taken: Vec<String> = load_backups(&root).into_iter().map(|b| b.text).collect();
        taken.sort();
        assert_eq!(taken, ["gone.txt", "old.txt"]);
        assert!(!base.exists() && !gone.exists(), "taken over");
        assert!(running.exists(), "the running one's left alone");
        assert_eq!(load_backups(&root).len(), 2, "kept in this one's own file");
        // This one has nothing unsaved: its file goes, not the other's.
        save_backups(&root, &[]);
        assert!(running.exists() && load_backups(&root).is_empty());
        std::fs::remove_file(&running).ok();
    }

    /// A Null runs while it holds its lock: a process given a gone Null's number since
    /// doesn't keep that Null's backups from coming back.
    #[test]
    fn a_running_null_is_told_by_its_lock() {
        hold_own_lock();
        assert!(running(std::process::id()));
        // A process that's there (this test's parent), with a lock left free: a Null gone.
        let parent = std::os::unix::process::parent_id();
        let lock = lock_file(parent).unwrap();
        std::fs::write(&lock, "").unwrap();
        assert!(!running(parent));
        assert!(!lock.exists(), "a gone Null's lock is cleared away");
    }

    #[test]
    fn recent_projects_go_first_once_each() {
        let dir = crate::tools::test_dir("recent");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("recent.json");
        let _ = std::fs::remove_file(&file);
        for p in ["/a", "/b", "/a", "/c"] {
            remember_in(&file, Path::new(p));
        }
        assert_eq!(read_recent(&file), [PathBuf::from("/c"), PathBuf::from("/a"), PathBuf::from("/b")]);
        // One taken off the list: the others stay, in order.
        forget_in(&file, Path::new("/a"));
        assert_eq!(read_recent(&file), [PathBuf::from("/c"), PathBuf::from("/b")]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_files_are_dropped_and_the_active_tab_follows() {
        let dir = crate::tools::test_dir("session");
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("kept.rs");
        std::fs::write(&kept, "fn main() {}").unwrap();
        let session = Session {
            tabs: vec![
                TabState { path: dir.join("gone.rs"), ..Default::default() },
                TabState { path: kept.clone(), line: 3, ..Default::default() },
            ],
            active: Some(1),
            ..Default::default()
        };
        let file = file_in(&dir, &dir);
        std::fs::write(&file, serde_json::to_string(&session).unwrap()).unwrap();
        let loaded = Session::load_file(&file);
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.tabs[0].path, kept);
        assert_eq!(loaded.active, Some(0));
        std::fs::remove_dir_all(dir).ok();
    }
}
