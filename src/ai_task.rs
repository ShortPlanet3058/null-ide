//! An AI task's before and after: the project's text files are copied (in memory) when
//! a task starts, so afterwards every change can be found, reviewed and undone, with or
//! without git.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Files bigger than this aren't copied: they're almost never what a task edits.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 20_000;

pub struct Snapshot {
    files: HashMap<PathBuf, String>,
    /// Every file there was, copied or not (too big, not text, past the limit): one of
    /// these isn't new afterwards, so undoing never trashes it.
    present: std::collections::HashSet<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Changed,
    Added,
    Deleted,
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: ChangeKind,
    /// Lines added and removed.
    pub added: usize,
    pub removed: usize,
    /// The text before the task (empty for an added file).
    pub before: String,
    /// The text the task left (empty for a deleted file): undoing leaves the file alone
    /// if it reads otherwise by then (edited since).
    pub after: String,
    /// Saved by you while the task ran: its change isn't the task's alone.
    pub yours_too: bool,
}

/// The project's files, respecting .gitignore (hidden files included, .git not).
fn all_files(root: &Path) -> impl Iterator<Item = ignore::DirEntry> {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
}

/// The project's text files, by path.
fn text_files(root: &Path) -> HashMap<PathBuf, String> {
    all_files(root)
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES))
        .take(MAX_FILES)
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            (!text.contains('\0')).then(|| (e.into_path(), text))
        })
        .collect()
}

impl Snapshot {
    /// Copies the project's text files. Takes a moment on big projects: off the main thread.
    pub fn take(root: &Path) -> Self {
        Self { files: text_files(root), present: all_files(root).map(ignore::DirEntry::into_path).collect() }
    }

    /// What changed since: files changed, added and deleted, by path.
    pub fn changes(&self, root: &Path) -> Vec<FileChange> {
        let now = text_files(root);
        let mut changes: Vec<FileChange> = Vec::new();
        for (path, after) in &now {
            let (kind, before) = match self.files.get(path) {
                Some(before) if before == after => continue,
                Some(before) => (ChangeKind::Changed, before.as_str()),
                // There before, but not copied: there's nothing to compare or put back.
                None if self.present.contains(path) => continue,
                None => (ChangeKind::Added, ""),
            };
            let (added, removed) = line_counts(before, after);
            changes.push(FileChange {
                path: path.clone(),
                kind,
                added,
                removed,
                before: before.to_string(),
                after: after.clone(),
                yours_too: false,
            });
        }
        for (path, before) in &self.files {
            // Gone from the walk and from the disk (not merely ignored now).
            if !now.contains_key(path) && !path.exists() {
                let removed = before.lines().count();
                changes.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Deleted,
                    added: 0,
                    removed,
                    before: before.clone(),
                    after: String::new(),
                    yours_too: false,
                });
            }
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        changes
    }
}

/// Lines added and removed between two texts.
pub fn line_counts(before: &str, after: &str) -> (usize, usize) {
    let diff = similar::TextDiff::from_lines(before, after);
    let (mut added, mut removed) = (0, 0);
    for op in diff.ops() {
        match *op {
            similar::DiffOp::Equal { .. } => {}
            similar::DiffOp::Delete { old_len, .. } => removed += old_len,
            similar::DiffOp::Insert { new_len, .. } => added += new_len,
            similar::DiffOp::Replace { old_len, new_len, .. } => {
                removed += old_len;
                added += new_len;
            }
        }
    }
    (added, removed)
}

/// Puts a file back as it was before the task: its old text, or (for a file the task
/// added) into the Trash.
pub fn undo(change: &FileChange) -> Result<(), String> {
    match change.kind {
        ChangeKind::Added => crate::fs_ops::move_to_trash(&change.path),
        ChangeKind::Changed | ChangeKind::Deleted => {
            if let Some(dir) = change.path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            crate::fs_ops::write_file(&change.path, change.before.as_bytes()).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_changed_and_puts_it_back() {
        let root = crate::tools::test_dir("task");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("src/b.rs"), "keep\n").unwrap();
        std::fs::write(root.join("gone.txt"), "bye\n").unwrap();
        // Not text when the task starts, text after: it was there, so it isn't new.
        std::fs::write(root.join("data.bin"), "a\0b").unwrap();
        let snapshot = Snapshot::take(&root);
        std::fs::write(root.join("data.bin"), "ab\n").unwrap();
        std::fs::write(root.join("src/a.rs"), "one\n2\nthree\n").unwrap();
        std::fs::write(root.join("src/new.rs"), "fresh\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        let changes = snapshot.changes(&root);
        let summary: Vec<_> = changes
            .iter()
            .map(|c| (c.path.strip_prefix(&root).unwrap().to_string_lossy().into_owned(), c.kind, c.added, c.removed))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("gone.txt".into(), ChangeKind::Deleted, 0, 1),
                ("src/a.rs".into(), ChangeKind::Changed, 2, 1),
                ("src/new.rs".into(), ChangeKind::Added, 1, 0),
            ]
        );
        for change in changes.iter().filter(|c| c.kind != ChangeKind::Added) {
            undo(change).unwrap();
        }
        assert_eq!(std::fs::read_to_string(root.join("src/a.rs")).unwrap(), "one\ntwo\n");
        assert!(root.join("gone.txt").exists());
        std::fs::remove_dir_all(&root).ok();
    }
}
