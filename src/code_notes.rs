//! Notes on code: a line's note, written by the person (or left by the AI when asked to
//! review), kept privately in Null's own folder, one file a project, never in the project.
//! Each note remembers its line's text, so it finds its line again when the file changed
//! while Null wasn't looking.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeNote {
    pub id: u64,
    /// The file, from the project's folder, with `/` between folders.
    pub path: String,
    /// Its line (0-based), as last known.
    pub line: usize,
    /// That line's text, trimmed: how it's found again.
    pub anchor: String,
    pub text: String,
    /// Left by the AI (a review, an answer kept), not written by the person.
    #[serde(default)]
    pub by_ai: bool,
    /// From a review of changes before a commit: the next review of the file replaces it.
    #[serde(default)]
    pub review: bool,
    /// When it was written (seconds since 1970).
    #[serde(default)]
    pub created: u64,
}

/// Where a project's notes are kept: Null's folder, named after the project's.
fn file_for(root: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let hash: String =
        Sha256::digest(root.to_string_lossy().as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    let name = root.file_name().map_or("project".to_string(), |n| n.to_string_lossy().into_owned());
    Some(crate::tools::data_dir()?.join("notes").join(format!("{name}-{hash}.json")))
}

/// Every note of the project.
pub fn load(root: &Path) -> Vec<CodeNote> {
    file_for(root)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Keeps them (none left: the file goes).
pub fn save(root: &Path, notes: &[CodeNote]) {
    let Some(file) = file_for(root) else { return };
    if notes.is_empty() {
        let _ = std::fs::remove_file(file);
        return;
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(notes) {
        let _ = std::fs::write(file, json);
    }
}

/// `path` as notes name it, if it's in the project.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let inside = path.strip_prefix(root).ok()?;
    Some(inside.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
}

/// A new note's id: unique enough (the time, in microseconds).
pub fn new_id() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The line a note is on in `text` now: where it was if that line is still its line, else
/// the line with its text nearest to where it was, else where it was (within the file).
pub fn find_line(note: &CodeNote, lines: &[&str]) -> usize {
    let same = |l: &str| l.trim() == note.anchor;
    if lines.get(note.line).is_some_and(|l| same(l)) || note.anchor.is_empty() {
        return note.line.min(lines.len().saturating_sub(1));
    }
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| same(l))
        .min_by_key(|(i, _)| i.abs_diff(note.line))
        .map_or(note.line.min(lines.len().saturating_sub(1)), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(line: usize, anchor: &str) -> CodeNote {
        CodeNote {
            id: 1,
            path: "src/a.rs".into(),
            line,
            anchor: anchor.into(),
            text: "check this".into(),
            by_ai: false,
            review: false,
            created: 0,
        }
    }

    #[test]
    fn a_note_finds_its_line_again() {
        let lines = ["fn a() {", "    let x = 1;", "}", "", "fn b() {", "    let x = 1;", "}"];
        // Still there.
        assert_eq!(find_line(&note(4, "fn b() {"), &lines), 4);
        // Moved down by lines added above: the same text, nearest to where it was.
        assert_eq!(find_line(&note(2, "fn b() {"), &lines), 4);
        assert_eq!(find_line(&note(4, "let x = 1;"), &lines), 5);
        // Its line gone: where it was, within the file.
        assert_eq!(find_line(&note(3, "fn c() {"), &lines), 3);
        assert_eq!(find_line(&note(40, "fn c() {"), &lines), 6);
    }

    #[test]
    fn notes_are_kept_per_project_outside_it() {
        let root = crate::tools::test_dir("code-notes-project");
        let notes = vec![note(3, "fn b() {")];
        save(&root, &notes);
        assert_eq!(load(&root), notes);
        let file = file_for(&root).unwrap();
        assert!(!file.starts_with(&root) && file.starts_with(crate::tools::data_dir().unwrap()));
        save(&root, &[]);
        assert!(load(&root).is_empty() && !file.exists());
        assert_eq!(relative(&root, &root.join("src").join("a.rs")).as_deref(), Some("src/a.rs"));
        assert_eq!(relative(&root, Path::new("/elsewhere/a.rs")), None);
    }
}
