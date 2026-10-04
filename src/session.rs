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
    /// How much of the width the left side took, when split.
    pub split_ratio: Option<f32>,
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

fn backups_file(root: &Path) -> Option<PathBuf> {
    Some(file_in(&crate::tools::data_dir()?.join("backups"), root))
}

/// Keeps `backups` as the project's unsaved work; none removes the file.
pub fn save_backups(root: &Path, backups: &[Backup]) {
    let Some(file) = backups_file(root) else { return };
    if backups.is_empty() {
        std::fs::remove_file(&file).ok();
        return;
    }
    let Ok(text) = serde_json::to_string(backups) else { return };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    // Write then rename: a crash mid-write mustn't lose the backup it replaces.
    let temp = file.with_extension("json.tmp");
    if std::fs::write(&temp, text).is_ok() {
        std::fs::rename(&temp, &file).ok();
    }
}

/// The unsaved work left from last time (a crash, or a forced quit).
pub fn load_backups(root: &Path) -> Vec<Backup> {
    let Some(file) = backups_file(root) else { return Vec::new() };
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
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

    #[test]
    fn recent_projects_go_first_once_each() {
        let dir = std::env::temp_dir().join(format!("null-recent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("recent.json");
        let _ = std::fs::remove_file(&file);
        for p in ["/a", "/b", "/a", "/c"] {
            remember_in(&file, Path::new(p));
        }
        assert_eq!(read_recent(&file), [PathBuf::from("/c"), PathBuf::from("/a"), PathBuf::from("/b")]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_files_are_dropped_and_the_active_tab_follows() {
        let dir = std::env::temp_dir().join(format!("null-session-{}", std::process::id()));
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
