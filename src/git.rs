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

/// The file as of the last commit. None if it isn't tracked (or there's no repo).
/// Asked from the file's own folder, so a project opened through a link (or under
/// /tmp, which is /private/tmp on macOS) still finds it.
pub fn committed_text(path: &Path, encoding: crate::encoding::Encoding) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let output =
        Command::new("git").arg("-C").arg(path.parent()?).args(["show", &format!("HEAD:./{name}")]).output().ok()?;
    // In the file's own encoding, so a Windows-1252 file compares with its accents.
    output.status.success().then(|| crate::encoding::decode_as(output.stdout, encoding)).flatten()
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
        // A rename (or copy) lists its old path next. The new path isn't in the last commit:
        // it counts as new (taking it back sends it to the Trash, never deletes it), and a
        // renamed file's old path as deleted (taking that back restores it).
        if x == 'R' || x == 'C' {
            if let Some(old) = entries.next()
                && x == 'R'
            {
                found.push((top.join(old), FileStatus::Deleted));
            }
            found.push((top.join(path), FileStatus::Added));
            continue;
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
#[cfg(test)]
pub fn commit_all(root: &Path, message: &str) -> Result<String, String> {
    commit(root, message, &[])
}

/// Commits every change but those to `left_out`, which stay as they are, uncommitted.
pub fn commit(root: &Path, message: &str, left_out: &[PathBuf]) -> Result<String, String> {
    run(root, &["add", "-A"])?;
    if !left_out.is_empty() {
        let paths: Vec<String> = left_out.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let mut args = vec!["reset", "-q", "--"];
        args.extend(paths.iter().map(String::as_str));
        run(root, &args)?;
    }
    run(root, &["commit", "-q", "-m", message])?;
    Ok(git(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_default().trim().to_string())
}

/// Commits the branch has that its upstream doesn't, and the other way round, as far as
/// the last fetch knows: (to push, to pull). None without an upstream.
pub fn ahead_behind(root: &Path) -> Option<(usize, usize)> {
    let counts = git(root, &["rev-list", "--left-right", "--count", "HEAD...@{u}"])?;
    let mut numbers = counts.split_whitespace().map(|n| n.parse::<usize>().ok());
    Some((numbers.next()??, numbers.next()??))
}

/// A commit that changed a file, as its history lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCommit {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub subject: String,
    /// Where the file was then, from the repository's top (it may have been renamed since).
    pub path: String,
}

/// The commits that changed `path`, newest first, following it through renames.
pub fn file_history(path: &Path, max: usize) -> Vec<FileCommit> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let format = "--format=%x1e%H%x1f%h%x1f%an%x1f%at%x1f%s";
    let count = format!("-n{max}");
    let Some(log) = git(dir, &["log", "--follow", &count, format, "--name-only", "--", name]) else {
        return Vec::new();
    };
    log.split('\x1e')
        .filter_map(|record| {
            let mut lines = record.lines();
            let fields: Vec<&str> = lines.next()?.split('\x1f').collect();
            let path = lines.map(str::trim).find(|l| !l.is_empty())?.to_string();
            let [hash, short, author, time, subject] = fields.as_slice() else { return None };
            Some(FileCommit {
                hash: hash.to_string(),
                short: short.to_string(),
                author: author.to_string(),
                time: time.parse().unwrap_or(0),
                subject: subject.to_string(),
                path,
            })
        })
        .collect()
}

/// The file as it was in `commit`, in its own encoding.
pub fn file_at(path: &Path, commit: &FileCommit, encoding: crate::encoding::Encoding) -> Option<String> {
    let top = git(path.parent()?, &["rev-parse", "--show-toplevel"])?;
    let output = Command::new("git")
        .arg("-C")
        .arg(top.trim())
        .args(["show", &format!("{}:{}", commit.hash, commit.path)])
        .output()
        .ok()?;
    output.status.success().then(|| crate::encoding::decode_as(output.stdout, encoding)).flatten()
}

/// What a pull did.
#[derive(Debug, PartialEq, Eq)]
pub enum Pulled {
    UpToDate,
    Commits(usize),
    /// It merged, but these files conflict.
    Conflicts(Vec<PathBuf>),
}

/// Pulls the branch from its upstream, as the person's git is set up to (merge or
/// rebase). Never asks for a password in a terminal that isn't there.
pub fn pull(root: &Path) -> Result<Pulled, String> {
    let before = git(root, &["rev-parse", "HEAD"]).unwrap_or_default();
    if let Err(error) = run(root, &["pull", "-q", "--no-edit"]) {
        let conflicted: Vec<PathBuf> =
            status(root).into_iter().filter(|(_, s)| *s == FileStatus::Conflicted).map(|(p, _)| p).collect();
        return if conflicted.is_empty() { Err(error) } else { Ok(Pulled::Conflicts(conflicted)) };
    }
    // The commits that came, not counting a merge commit made to join them.
    let came = |to: &str| git(root, &["rev-list", "--count", &format!("{}..{to}", before.trim())]);
    let count = came("@{u}").or_else(|| came("HEAD")).and_then(|n| n.trim().parse().ok()).unwrap_or(0);
    Ok(if count == 0 { Pulled::UpToDate } else { Pulled::Commits(count) })
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

/// What `switch_branch` says when changes here would be overwritten there.
pub const WOULD_LOSE_CHANGES: &str = "Changes here would be lost on that branch: commit or revert them first.";

/// Switches to `branch` taking the changes not committed along: set aside, switched,
/// put back. The files they conflict in, if putting them back does (they're left marked).
pub fn switch_carrying_changes(root: &Path, branch: &Branch) -> Result<Vec<PathBuf>, String> {
    run(root, &["stash", "push", "-q", "--include-untracked", "-m", &format!("Null: carried to {}", branch.name)])
        .map_err(|e| format!("Couldn't set the changes aside: {e}"))?;
    if let Err(error) = switch_branch(root, branch) {
        // Back as they were, on the branch they were on.
        run(root, &["stash", "pop", "-q"]).ok();
        return Err(error);
    }
    match run(root, &["stash", "pop", "-q"]) {
        Ok(_) => Ok(Vec::new()),
        Err(error) => {
            let conflicted: Vec<PathBuf> =
                status(root).into_iter().filter(|(_, s)| *s == FileStatus::Conflicted).map(|(p, _)| p).collect();
            if conflicted.is_empty() { Err(error) } else { Ok(conflicted) }
        }
    }
}

fn explain_switch_error(error: &str) -> String {
    if error.contains("would be overwritten") {
        WOULD_LOSE_CHANGES.into()
    } else if error.contains("already exists") {
        "A branch with that name already exists.".into()
    } else if error.contains("not a valid branch name") {
        "That isn't a valid branch name.".into()
    } else {
        error.trim_start_matches("fatal: ").trim_start_matches("error: ").to_string()
    }
}

/// Who last changed a line, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBlame {
    /// "You" for the person committing here.
    pub author: String,
    /// Seconds since 1970.
    pub time: i64,
    pub summary: String,
}

/// Who last changed line `line` (from 0) of `path`, reading the file as `text` (so
/// unsaved edits don't shift the lines). None for a line not committed yet, or a file
/// git doesn't follow.
pub fn blame_line(path: &Path, text: &str, line: usize) -> Option<LineBlame> {
    use std::io::Write;
    let dir = path.parent()?;
    let name = path.file_name()?.to_str()?;
    let range = format!("{},{}", line + 1, line + 1);
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["blame", "--porcelain", "-L", &range, "--contents", "-", "--", name])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(text.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let out = String::from_utf8_lossy(&output.stdout);
    let mut lines = out.lines();
    let commit = lines.next()?.split(' ').next()?;
    if commit.chars().all(|c| c == '0') {
        return None;
    }
    let field = |key: &str| out.lines().find_map(|l| l.strip_prefix(key)).map(str::to_string);
    let author = field("author ")?;
    let me = git(dir, &["config", "user.name"]).map(|n| n.trim().to_string());
    Some(LineBlame {
        author: if me.as_deref() == Some(author.as_str()) { "You".into() } else { author },
        time: field("author-time ")?.parse().ok()?,
        summary: field("summary ").unwrap_or_default(),
    })
}

/// How long ago `time` was, in words: "just now", "5 minutes ago", "3 days ago"…
pub fn ago(time: i64, now: i64) -> String {
    let seconds = (now - time).max(0);
    let units = [
        (31_536_000, "year"),
        (2_592_000, "month"),
        (604_800, "week"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
    ];
    for (size, unit) in units {
        let n = seconds / size;
        if n >= 1 {
            return if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") };
        }
    }
    "just now".into()
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

/// A remote's address as the web page of its repository: `git@github.com:me/app.git`
/// and `https://github.com/me/app.git` both give `https://github.com/me/app`. An SSH
/// host alias (`github-work`) is taken for the service it names.
pub fn web_url(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let (host, path) = if let Some(rest) = remote.strip_prefix("https://").or_else(|| remote.strip_prefix("http://")) {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        rest.split_once('/')?
    } else {
        let rest = remote.strip_prefix("ssh://").unwrap_or(remote);
        let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
        // `host:owner/repo`, or `host/owner/repo` (and `host:22/owner/repo`) in the ssh:// form.
        match rest.split_once(':') {
            Some((host, path)) if !path.starts_with(|c: char| c.is_ascii_digit()) => (host, path),
            _ => {
                let (host, path) = rest.split_once('/')?;
                (
                    host.split(':').next()?,
                    path.split_once('/')
                        .filter(|(p, _)| p.chars().all(|c| c.is_ascii_digit()))
                        .map_or(path, |(_, p)| p),
                )
            }
        }
    };
    let host = ["github", "gitlab", "bitbucket"]
        .iter()
        .find(|service| host.contains(*service) && !host.contains('.'))
        .map(|service| if *service == "bitbucket" { "bitbucket.org".to_string() } else { format!("{service}.com") })
        .unwrap_or_else(|| host.to_string());
    Some(format!("https://{host}/{path}"))
}

/// A permanent link to lines of a file at the commit checked out: GitHub's, GitLab's or
/// Bitbucket's form. Lines are 1-based, inclusive.
pub fn line_link(path: &Path, first: usize, last: usize) -> Result<String, String> {
    let dir = path.parent().ok_or("No folder.")?;
    let remote = git(dir, &["remote", "get-url", "origin"]).ok_or("This repository has no remote called origin.")?;
    let web = web_url(&remote).ok_or_else(|| format!("Don't know the web address of {}", remote.trim()))?;
    let top = git(dir, &["rev-parse", "--show-toplevel"]).ok_or("Not in a git repository.")?;
    let commit = git(dir, &["rev-parse", "HEAD"]).ok_or("Nothing committed yet.")?;
    // Through any links: a project opened from /tmp is /private/tmp to git on macOS.
    let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (file_path, top) = (resolve(path), resolve(Path::new(top.trim())));
    let relative = file_path.strip_prefix(&top).map_err(|_| "The file isn't in the repository.")?;
    let file = relative.to_string_lossy().replace('\\', "/");
    let commit = commit.trim();
    let lines = |single: String, range: String| if first == last { single } else { range };
    Ok(if web.contains("gitlab") {
        format!("{web}/-/blob/{commit}/{file}#{}", lines(format!("L{first}"), format!("L{first}-{last}")))
    } else if web.contains("bitbucket") {
        format!("{web}/src/{commit}/{file}#lines-{}", lines(format!("{first}"), format!("{first}:{last}")))
    } else {
        format!("{web}/blob/{commit}/{file}#{}", lines(format!("L{first}"), format!("L{first}-L{last}")))
    })
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
        assert_eq!(committed_text(&dir.join("link/src/a.rs"), Default::default()).as_deref(), Some("committed\n"));
        assert_eq!(committed_text(&repo.join("src/new.rs"), Default::default()), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A renamed file: the new name is new (taking it back trashes it, never deletes it),
    /// the old one deleted (taking it back restores it).
    #[test]
    #[cfg(unix)]
    fn a_rename_is_a_new_file_and_a_deleted_one() {
        let repo = std::env::temp_dir().join(format!("null-git-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| git(&repo, args);
        if run(&["init", "-q"]).is_none() {
            return;
        }
        std::fs::write(repo.join("a.txt"), "text\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"]).unwrap();
        run(&["mv", "a.txt", "b.txt"]).unwrap();
        std::fs::write(repo.join("b.txt"), "text, edited\n").unwrap();
        let found = status(&repo);
        let top = std::path::PathBuf::from(git(&repo, &["rev-parse", "--show-toplevel"]).unwrap().trim());
        assert!(found.contains(&(top.join("b.txt"), FileStatus::Added)), "{found:?}");
        assert!(found.contains(&(top.join("a.txt"), FileStatus::Deleted)), "{found:?}");
        // Taking the old name back restores it; the new file is untouched.
        revert(&repo, &top.join("a.txt"), FileStatus::Deleted).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "text\n");
        assert_eq!(std::fs::read_to_string(repo.join("b.txt")).unwrap(), "text, edited\n");
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    #[cfg(unix)]
    fn changes_come_along_to_another_branch() {
        let repo = std::env::temp_dir().join(format!("null-git-carry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        if run(&repo, &["init", "-q", "-b", "main"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("f.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        commit_all(&repo, "first").unwrap();
        run(&repo, &["switch", "-q", "-c", "other"]).unwrap();
        std::fs::write(repo.join("f.txt"), "ONE\ntwo\nthree\nfour\n").unwrap();
        commit_all(&repo, "other's").unwrap();
        run(&repo, &["switch", "-q", "main"]).unwrap();
        let other = Branch { name: "other".into(), current: false, remote: false, when: String::new() };
        // A change to another line of the file: git won't switch, carried it merges.
        std::fs::write(repo.join("f.txt"), "one\ntwo\nthree\nFOUR\n").unwrap();
        assert_eq!(switch_branch(&repo, &other), Err(WOULD_LOSE_CHANGES.to_string()));
        assert_eq!(switch_carrying_changes(&repo, &other), Ok(Vec::new()));
        assert_eq!(current_branch(&repo).as_deref(), Some("other"));
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "ONE\ntwo\nthree\nFOUR\n");
        // Back on main with a change to the same line: carried, it conflicts.
        run(&repo, &["checkout", "-q", "--", "f.txt"]).unwrap();
        run(&repo, &["switch", "-q", "main"]).unwrap();
        std::fs::write(repo.join("f.txt"), "uno\ntwo\nthree\nfour\n").unwrap();
        let conflicts = switch_carrying_changes(&repo, &other).unwrap();
        assert_eq!(conflicts.iter().map(|p| p.file_name().unwrap().to_owned()).collect::<Vec<_>>(), ["f.txt"]);
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn remotes_become_web_addresses() {
        let web = |r: &str| web_url(r).unwrap();
        assert_eq!(web("git@github.com:me/app.git"), "https://github.com/me/app");
        assert_eq!(web("https://github.com/me/app.git"), "https://github.com/me/app");
        assert_eq!(web("https://token@github.com/me/app"), "https://github.com/me/app");
        assert_eq!(web("ssh://git@gitlab.com:22/group/sub/app.git"), "https://gitlab.com/group/sub/app");
        assert_eq!(web("git@bitbucket.org:team/app.git"), "https://bitbucket.org/team/app");
        // An SSH alias for a second account on GitHub.
        assert_eq!(
            web("git@github-shortplanet:ShortPlanet3058/null-ide.git"),
            "https://github.com/ShortPlanet3058/null-ide"
        );
        assert_eq!(web("git@git.example.org:me/app.git"), "https://git.example.org/me/app");
    }

    #[test]
    #[cfg(unix)]
    fn a_link_to_lines_names_the_commit() {
        let repo = std::env::temp_dir().join(format!("null-git-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        run(&repo, &["remote", "add", "origin", "git@github.com:me/app.git"]).unwrap();
        std::fs::write(repo.join("src/a.rs"), "fn a() {}\n").unwrap();
        commit_all(&repo, "first").unwrap();
        let head = git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let link = line_link(&repo.join("src/a.rs"), 3, 5).unwrap();
        assert_eq!(link, format!("https://github.com/me/app/blob/{}/src/a.rs#L3-L5", head.trim()));
        assert!(line_link(&repo.join("src/a.rs"), 3, 3).unwrap().ends_with("#L3"));
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    #[cfg(unix)]
    fn a_file_s_history_follows_renames() {
        let repo = std::env::temp_dir().join(format!("null-git-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "Ada"]).unwrap();
        run(&repo, &["config", "user.email", "a@a"]).unwrap();
        std::fs::write(repo.join("src/old.rs"), "fn one() {}\n").unwrap();
        commit_all(&repo, "Start").unwrap();
        run(&repo, &["mv", "src/old.rs", "src/new.rs"]).unwrap();
        commit_all(&repo, "Rename").unwrap();
        std::fs::write(repo.join("src/new.rs"), "fn one() {}\nfn two() {}\n").unwrap();
        commit_all(&repo, "Add two").unwrap();
        let history = file_history(&repo.join("src/new.rs"), 50);
        let subjects: Vec<&str> = history.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, ["Add two", "Rename", "Start"]);
        assert_eq!((history[2].author.as_str(), history[2].path.as_str()), ("Ada", "src/old.rs"));
        // The first version, from before the rename.
        let first = file_at(&repo.join("src/new.rs"), &history[2], Default::default());
        assert_eq!(first.as_deref(), Some("fn one() {}\n"));
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    #[cfg(unix)]
    fn pulls_and_counts_what_to_push_and_pull() {
        let dir = std::env::temp_dir().join(format!("null-git-pull-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if run(&dir, &["init", "-q", "--bare", "-b", "main", "origin.git"]).is_err() {
            return; // No git here.
        }
        let clone = |name: &str| {
            run(&dir, &["clone", "-q", "origin.git", name]).unwrap();
            let repo = dir.join(name);
            for (key, value) in [("user.name", "t"), ("user.email", "t@t"), ("pull.rebase", "false")] {
                run(&repo, &["config", key, value]).unwrap();
            }
            repo
        };
        let a = clone("a");
        std::fs::write(a.join("f.txt"), "one\n").unwrap();
        commit_all(&a, "first").unwrap();
        run(&a, &["push", "-q", "-u", "origin", "HEAD:main"]).unwrap();
        let b = clone("b");
        // A commits again: B, once it has fetched, has one to pull.
        std::fs::write(a.join("f.txt"), "two\n").unwrap();
        commit_all(&a, "second").unwrap();
        assert_eq!(ahead_behind(&a), Some((1, 0)));
        run(&a, &["push", "-q"]).unwrap();
        run(&b, &["fetch", "-q"]).unwrap();
        assert_eq!(ahead_behind(&b), Some((0, 1)));
        assert_eq!(pull(&b), Ok(Pulled::Commits(1)));
        assert_eq!(pull(&b), Ok(Pulled::UpToDate));
        // Both change the same line: the pull stops at a conflict.
        std::fs::write(a.join("f.txt"), "from a\n").unwrap();
        commit_all(&a, "a's").unwrap();
        run(&a, &["push", "-q"]).unwrap();
        std::fs::write(b.join("f.txt"), "from b\n").unwrap();
        commit_all(&b, "b's").unwrap();
        let Ok(Pulled::Conflicts(files)) = pull(&b) else { panic!("expected a conflict") };
        assert_eq!(files.iter().map(|f| f.file_name().unwrap().to_owned()).collect::<Vec<_>>(), ["f.txt"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[cfg(unix)]
    fn commits_all_but_the_files_left_out() {
        let repo = std::env::temp_dir().join(format!("null-git-partial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        commit_all(&repo, "first").unwrap();
        // Two changed, one new, one deleted: the change to b and the new file stay out.
        std::fs::write(repo.join("a.txt"), "a2\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b2\n").unwrap();
        std::fs::write(repo.join("new.txt"), "n\n").unwrap();
        commit(&repo, "second", &[repo.join("b.txt"), repo.join("new.txt")]).unwrap();
        let committed = git(&repo, &["show", "--name-only", "--format=", "HEAD"]).unwrap();
        assert_eq!(committed.trim(), "a.txt");
        let mut left: Vec<String> =
            status(&repo).into_iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["b.txt", "new.txt"]);
        // Still changed, as left.
        assert_eq!(std::fs::read_to_string(repo.join("b.txt")).unwrap(), "b2\n");
        std::fs::remove_dir_all(&repo).ok();
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
    fn says_who_changed_a_line_and_when() {
        let repo = std::env::temp_dir().join(format!("null-git-blame-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "Ada"]).unwrap();
        run(&repo, &["config", "user.email", "ada@example.com"]).unwrap();
        let file = repo.join("a.txt");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        commit_all(&repo, "First lines").unwrap();
        // A line typed above (not saved): the next line is still the committed "one".
        let blame = blame_line(&file, "new\none\ntwo\n", 1).unwrap();
        assert_eq!((blame.author.as_str(), blame.summary.as_str()), ("You", "First lines"));
        assert!(blame_line(&file, "new\none\ntwo\n", 0).is_none());
        assert!(blame_line(&repo.join("untracked.txt"), "x\n", 0).is_none());
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn says_how_long_ago() {
        assert_eq!(ago(1000, 1010), "just now");
        assert_eq!(ago(0, 60), "1 minute ago");
        assert_eq!(ago(0, 3 * 86_400 + 5), "3 days ago");
        assert_eq!(ago(0, 2 * 31_536_000), "2 years ago");
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
        assert!(committed_text(&readme, Default::default()).is_some_and(|t| t.contains("Null")));
        assert!(current_branch(readme.parent().unwrap()).is_some());
        assert!(committed_text(&readme.with_file_name("does-not-exist.txt"), Default::default()).is_none());
    }
}
