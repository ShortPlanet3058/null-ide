//! Just enough git for the gutter: the committed version of a file, the
//! current branch, and a line diff. Uses the `git` command, so nothing
//! happens (quietly) where git isn't installed.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn repo_root(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_dir() { path } else { path.parent()? };
    git(dir, &["rev-parse", "--show-toplevel"]).map(|s| PathBuf::from(s.trim()))
}

/// The file as of the last commit. None if it isn't tracked (or there's no repo).
pub fn committed_text(path: &Path) -> Option<String> {
    let root = repo_root(path)?;
    let relative = path.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
    git(&root, &["show", &format!("HEAD:{relative}")])
}

pub fn current_branch(dir: &Path) -> Option<String> {
    let name = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    // A detached HEAD reports "HEAD": show the short commit instead.
    if name == "HEAD" { git(dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string()) } else { Some(name) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    /// Lines were removed just above `lines.start` (an empty range).
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub change: Change,
    /// Zero-based lines in the current text.
    pub lines: Range<usize>,
}

/// How the current text differs from `base`, line by line.
pub fn diff(base: &str, current: &str) -> Vec<Hunk> {
    use similar::{DiffOp, TextDiff};
    let diff = TextDiff::from_lines(base, current);
    diff.ops()
        .iter()
        .filter_map(|op| match *op {
            DiffOp::Equal { .. } => None,
            DiffOp::Insert { new_index, new_len, .. } => {
                Some(Hunk { change: Change::Added, lines: new_index..new_index + new_len })
            }
            DiffOp::Delete { new_index, .. } => Some(Hunk { change: Change::Deleted, lines: new_index..new_index }),
            DiffOp::Replace { new_index, new_len, .. } => {
                Some(Hunk { change: Change::Modified, lines: new_index..new_index + new_len })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_added_modified_and_deleted_lines() {
        let base = "a\nb\nc\nd\ne\n";
        let current = "a\nB\nc\nnew\nd\n";
        assert_eq!(
            diff(base, current),
            vec![
                Hunk { change: Change::Modified, lines: 1..2 },
                Hunk { change: Change::Added, lines: 3..4 },
                Hunk { change: Change::Deleted, lines: 5..5 },
            ]
        );
        assert!(diff(base, base).is_empty());
    }

    #[test]
    fn reads_committed_text_and_branch_from_this_repo() {
        let readme = Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
        if repo_root(&readme).is_none() {
            return; // not running inside a git checkout
        }
        assert!(committed_text(&readme).is_some_and(|t| t.contains("Null")));
        assert!(current_branch(readme.parent().unwrap()).is_some());
        assert!(committed_text(&readme.with_file_name("does-not-exist.txt")).is_none());
    }
}
