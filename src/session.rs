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
        session.tabs.retain(|t| t.path.is_file());
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

fn last_project_file() -> Option<PathBuf> {
    Some(sessions_dir()?.join("last-project"))
}

fn remember_last_project(root: &Path) {
    if let Some(file) = last_project_file() {
        std::fs::write(file, root.to_string_lossy().as_bytes()).ok();
    }
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
