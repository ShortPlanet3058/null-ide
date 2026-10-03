//! Just enough git for the gutter: the committed version of a file, the
//! current branch, and a line diff. Uses the `git` command, so nothing
//! happens (quietly) where git isn't installed.

use std::ops::Range;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The file as of the last commit. None if it isn't tracked (or there's no repo).
/// Asked from the file's own folder, so a project opened through a link (or under
/// /tmp, which is /private/tmp on macOS) still finds it.
pub fn committed_text(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    git(path.parent()?, &["show", &format!("HEAD:./{name}")])
}

/// Whether a change inside `.git` can mean a new commit or branch: HEAD itself, the
/// branch files (a commit moves one without touching HEAD), the index.
pub fn is_state_change(path: &Path) -> bool {
    let mut inside = path.components().skip_while(|c| c.as_os_str() != ".git").skip(1);
    match inside.next().and_then(|c| c.as_os_str().to_str()) {
        Some("HEAD" | "index" | "ORIG_HEAD" | "packed-refs" | "refs") => true,
        Some("logs") => inside.next().is_some_and(|c| c.as_os_str() == "HEAD"),
        _ => false,
    }
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
    use std::path::PathBuf;

    #[test]
    #[cfg(unix)]
    fn finds_the_committed_text_through_a_link() {
        let dir = std::env::temp_dir().join(format!("null-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let run = |args: &[&str]| git(&repo, args);
        if run(&["init", "-q"]).is_none() {
            return; // No git here: nothing to test.
        }
        std::fs::write(repo.join("src/a.rs"), "committed\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"]).unwrap();
        std::fs::write(repo.join("src/a.rs"), "edited\n").unwrap();
        std::os::unix::fs::symlink(&repo, dir.join("link")).unwrap();
        assert_eq!(committed_text(&dir.join("link/src/a.rs")).as_deref(), Some("committed\n"));
        assert_eq!(committed_text(&repo.join("src/new.rs")), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn commits_and_branch_moves_count_as_git_changes() {
        let p = |s: &str| PathBuf::from(s);
        assert!(is_state_change(&p("/r/.git/HEAD")));
        assert!(is_state_change(&p("/r/.git/refs/heads/main")));
        assert!(is_state_change(&p("/r/.git/packed-refs")));
        assert!(is_state_change(&p("/r/.git/logs/HEAD")));
        assert!(!is_state_change(&p("/r/.git/objects/ab/cdef")));
        assert!(!is_state_change(&p("/r/src/HEAD")));
    }

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
        if git(readme.parent().unwrap(), &["rev-parse", "--git-dir"]).is_none() {
            return; // not running inside a git checkout
        }
        assert!(committed_text(&readme).is_some_and(|t| t.contains("Null")));
        assert!(current_branch(readme.parent().unwrap()).is_some());
        assert!(committed_text(&readme.with_file_name("does-not-exist.txt")).is_none());
    }
}
