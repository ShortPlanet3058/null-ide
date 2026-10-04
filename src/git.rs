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

/// How a file stands against the last commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    /// New: not in the last commit (untracked, or added).
    Added,
    Deleted,
    /// Both sides of a merge changed it.
    Conflicted,
}

/// Every changed file under `root`, by absolute path (untracked ones included, ignored
/// ones not). Empty outside a repository.
pub fn status(root: &Path) -> Vec<(std::path::PathBuf, FileStatus)> {
    let Some(top) = git(root, &["rev-parse", "--show-toplevel"]).map(|t| std::path::PathBuf::from(t.trim())) else {
        return Vec::new();
    };
    let Some(out) = git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]) else { return Vec::new() };
    let mut entries = out.split('\0').filter(|e| !e.is_empty());
    let mut found = Vec::new();
    while let Some(entry) = entries.next() {
        let (code, path) = entry.split_at(entry.len().min(3));
        let (x, y) = (code.chars().next().unwrap_or(' '), code.chars().nth(1).unwrap_or(' '));
        // A rename lists its old path next: skip it.
        if x == 'R' || x == 'C' {
            entries.next();
        }
        let status = match (x, y) {
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => FileStatus::Conflicted,
            ('?', '?') | ('A', _) => FileStatus::Added,
            ('D', _) | (_, 'D') => FileStatus::Deleted,
            _ => FileStatus::Modified,
        };
        found.push((top.join(path), status));
    }
    found
}

/// Commits every change (new files included) with `message`; returns the short commit id.
pub fn commit_all(root: &Path, message: &str) -> Result<String, String> {
    run(root, &["add", "-A"])?;
    run(root, &["commit", "-q", "-m", message])?;
    Ok(git(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_default().trim().to_string())
}

/// Pushes the branch to where it's tracked (or sets that up on `origin`). Never asks
/// for a password in a terminal that isn't there: it fails with git's message instead.
pub fn push(root: &Path) -> Result<String, String> {
    let branch = current_branch(root).ok_or("Not on a branch.")?;
    let tracked = git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_some();
    if tracked {
        run(root, &["push", "-q"])?
    } else {
        run(root, &["push", "-q", "-u", "origin", &branch])?
    };
    Ok(branch)
}

/// A file back as it was in the last commit; a new file goes to the Trash.
pub fn revert(root: &Path, path: &Path, status: FileStatus) -> Result<(), String> {
    if status == FileStatus::Added {
        return crate::fs_ops::move_to_trash(path);
    }
    let path = path.to_string_lossy();
    run(root, &["restore", "--source=HEAD", "--staged", "--worktree", "--", &path]).map(|_| ())
}

/// A branch to switch to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// "main", or "origin/feature" for one only on a remote.
    pub name: String,
    pub current: bool,
    /// Only on a remote: switching to it makes a local branch that tracks it.
    pub remote: bool,
    /// When it last had a commit, as git says it ("3 days ago").
    pub when: String,
}

/// The branches, most recently worked on first; then the remote ones with no local branch.
pub fn branches(root: &Path) -> Vec<Branch> {
    let list = |refs: &str| {
        git(
            root,
            &[
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(HEAD)%09%(refname)%09%(committerdate:relative)",
                refs,
            ],
        )
        .unwrap_or_default()
    };
    let mut found: Vec<Branch> = list("refs/heads")
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (head, name, when) = (parts.next()?, parts.next()?, parts.next().unwrap_or(""));
            Some(Branch {
                name: name.strip_prefix("refs/heads/")?.to_string(),
                current: head == "*",
                remote: false,
                when: when.to_string(),
            })
        })
        .collect();
    let remote: Vec<Branch> = list("refs/remotes")
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (_, name, when) = (parts.next()?, parts.next()?, parts.next().unwrap_or(""));
            let name = name.strip_prefix("refs/remotes/")?;
            let (_, short) = name.split_once('/')?;
            let local = found.iter().any(|b| b.name == short);
            (short != "HEAD" && !local).then(|| Branch {
                name: name.to_string(),
                current: false,
                remote: true,
                when: when.to_string(),
            })
        })
        .collect();
    found.extend(remote);
    found
}

/// Switches to `branch`; one only on a remote gets a local branch tracking it.
/// Git refuses when changes here would be lost, and says so.
pub fn switch_branch(root: &Path, branch: &Branch) -> Result<(), String> {
    let result = if branch.remote {
        run(root, &["switch", "-q", "--track", &branch.name])
    } else {
        run(root, &["switch", "-q", &branch.name])
    };
    result.map(|_| ()).map_err(|e| explain_switch_error(&e))
}

/// Starts a branch here, with the changes not committed yet coming along.
pub fn create_branch(root: &Path, name: &str) -> Result<(), String> {
    run(root, &["switch", "-q", "-c", name]).map(|_| ()).map_err(|e| explain_switch_error(&e))
}

/// A branch name from what was typed: spaces become dashes.
pub fn branch_name(typed: &str) -> String {
    typed.split_whitespace().collect::<Vec<_>>().join("-")
}

fn explain_switch_error(error: &str) -> String {
    if error.contains("would be overwritten") {
        "Changes here would be lost on that branch: commit or revert them first.".into()
    } else if error.contains("already exists") {
        "A branch with that name already exists.".into()
    } else if error.contains("not a valid branch name") {
        "That isn't a valid branch name.".into()
    } else {
        error.trim_start_matches("fatal: ").trim_start_matches("error: ").to_string()
    }
}

/// Runs git, with its own words when it fails.
fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Couldn't run git: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    let message: Vec<&str> = error.lines().filter(|l| !l.trim().is_empty()).take(3).collect();
    Err(message.join(" ").trim().to_string())
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
    #[cfg(unix)]
    fn status_commit_and_revert() {
        let repo = std::env::temp_dir().join(format!("null-git-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("src/a.rs"), "one\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "x\n").unwrap();
        assert!(commit_all(&repo, "first").unwrap().len() >= 7);
        std::fs::write(repo.join("src/a.rs"), "two\n").unwrap();
        std::fs::write(repo.join("src/new.rs"), "new\n").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        let mut found: Vec<(String, FileStatus)> = status(&repo)
            .into_iter()
            .map(|(p, s)| (p.file_name().unwrap().to_string_lossy().into_owned(), s))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            found,
            vec![
                ("a.rs".into(), FileStatus::Modified),
                ("gone.txt".into(), FileStatus::Deleted),
                ("new.rs".into(), FileStatus::Added),
            ]
        );
        revert(&repo, &repo.join("src/a.rs"), FileStatus::Modified).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("src/a.rs")).unwrap(), "one\n");
        revert(&repo, &repo.join("gone.txt"), FileStatus::Deleted).unwrap();
        assert!(repo.join("gone.txt").exists());
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn lists_switches_and_creates_branches() {
        let repo = std::env::temp_dir().join(format!("null-git-branches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        if run(&repo, &["init", "-q", "-b", "main"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        commit_all(&repo, "first").unwrap();
        create_branch(&repo, &branch_name("new  parser")).unwrap();
        assert_eq!(current_branch(&repo).as_deref(), Some("new-parser"));
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        commit_all(&repo, "second").unwrap();
        let names: Vec<(String, bool)> = branches(&repo).into_iter().map(|b| (b.name, b.current)).collect();
        assert!(names.contains(&("new-parser".into(), true)) && names.contains(&("main".into(), false)));
        // A change that main's version would overwrite: git refuses, in plain words.
        std::fs::write(repo.join("a.txt"), "three\n").unwrap();
        let main = Branch { name: "main".into(), current: false, remote: false, when: String::new() };
        assert!(switch_branch(&repo, &main).unwrap_err().contains("commit or revert"));
        revert(&repo, &repo.join("a.txt"), FileStatus::Modified).unwrap();
        switch_branch(&repo, &main).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
        assert!(create_branch(&repo, "main").unwrap_err().contains("already exists"));
        std::fs::remove_dir_all(&repo).ok();
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
