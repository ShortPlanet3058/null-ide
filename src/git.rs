//! Just enough git for the gutter: the committed version of a file, the
//! current branch, and a line diff. Uses the `git` command, so nothing
//! happens (quietly) where git isn't installed.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

/// git, run in `dir`. Paths given to it are names, never patterns: discarding `app/[id].tsx`
/// mustn't touch `app/d.tsx`.
fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).env("GIT_LITERAL_PATHSPECS", "1").env("GIT_TERMINAL_PROMPT", "0");
    // Asking (status) never writes git's index: that write was seen as a change, and set
    // every open file's committed version to be read again.
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = command(dir).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The file as of the last commit. None if it isn't tracked (or there's no repo).
/// Asked from the file's own folder, so a project opened through a link (or under
/// /tmp, which is /private/tmp on macOS) still finds it.
pub fn committed_text(path: &Path, encoding: crate::encoding::Encoding) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let output = command(path.parent()?).args(["show", &format!("HEAD:./{name}")]).output().ok()?;
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
/// ones not), and only under it: a project inside a bigger repository sees its own.
/// Empty outside a repository.
pub fn status(root: &Path) -> Vec<(std::path::PathBuf, FileStatus)> {
    let Some(top) = git(root, &["rev-parse", "--show-toplevel"]).map(|t| std::path::PathBuf::from(t.trim())) else {
        return Vec::new();
    };
    let Some(out) = git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", "."]) else {
        return Vec::new();
    };
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
            // Added, or only meant to be (`git add -N`): not in the last commit either way.
            ('?', '?') | ('A', _) | (_, 'A') => FileStatus::Added,
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
    run(root, &["add", "-A", "--", "."])?;
    if !left_out.is_empty() {
        let paths: Vec<String> = left_out.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let mut args = vec!["reset", "-q", "--"];
        args.extend(paths.iter().map(String::as_str));
        run(root, &args)?;
    }
    run(root, &["commit", "-q", "-m", message])?;
    Ok(git(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_default().trim().to_string())
}

/// Commits every change but those to `left_out` (kept as they are, uncommitted), and of
/// the files in `partly` only part: each as its text there (the committed text with only
/// some of its changes, see `partly`), the rest of its changes staying uncommitted.
pub fn commit_parts(
    root: &Path,
    message: &str,
    left_out: &[PathBuf],
    partly: &[(PathBuf, String)],
) -> Result<String, String> {
    run(root, &["add", "-A", "--", "."])?;
    if !left_out.is_empty() {
        let paths: Vec<String> = left_out.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let mut args = vec!["reset", "-q", "--"];
        args.extend(paths.iter().map(String::as_str));
        run(root, &args)?;
    }
    let top = PathBuf::from(run(root, &["rev-parse", "--show-toplevel"])?.trim());
    for (path, text) in partly {
        // The part committed, as a blob of its own; the file on disk stays as it is.
        let id = run_with_input(&top, &["hash-object", "-w", "--stdin"], text.as_bytes())?;
        let name = path.strip_prefix(&top).unwrap_or(path).to_string_lossy().into_owned();
        let mode = run(&top, &["ls-files", "-s", "--", &name])
            .ok()
            .and_then(|line| line.split_whitespace().next().map(str::to_string))
            .unwrap_or_else(|| "100644".into());
        run(&top, &["update-index", "--add", "--cacheinfo", &format!("{mode},{},{name}", id.trim())])?;
    }
    run(root, &["commit", "-q", "-m", message])?;
    Ok(git(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_default().trim().to_string())
}

/// git, given `input` to read.
fn run_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = command(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run git: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input).map_err(|e| format!("Couldn't run git: {e}"))?;
    }
    let output = child.wait_with_output().map_err(|e| format!("Couldn't run git: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

/// `base` with only some of the changes that make it `current`: each change (as `diff`
/// lists them, in order) whose `keep` is true taken, the others left as `base` has them.
pub fn partly(base: &str, current: &str, keep: &[bool]) -> String {
    use similar::DiffOp;
    let (old, new) = (lines(base), lines(current));
    let mut out = String::new();
    let mut change = 0;
    for op in line_ops(base, current) {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                old[old_index..old_index + len].iter().for_each(|l| out.push_str(l))
            }
            DiffOp::Insert { old_index, new_index, new_len }
            | DiffOp::Replace { old_index, new_index, new_len, .. } => {
                let old_len = match op {
                    DiffOp::Replace { old_len, .. } => old_len,
                    _ => 0,
                };
                if keep.get(change).copied().unwrap_or(true) {
                    new[new_index..new_index + new_len].iter().for_each(|l| out.push_str(l));
                } else {
                    old[old_index..old_index + old_len].iter().for_each(|l| out.push_str(l));
                }
                change += 1;
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                if !keep.get(change).copied().unwrap_or(true) {
                    old[old_index..old_index + old_len].iter().for_each(|l| out.push_str(l));
                }
                change += 1;
            }
        }
    }
    out
}

/// Lines added and removed from `base` to `current`.
pub fn counts(base: &str, current: &str) -> (usize, usize) {
    use similar::DiffOp;
    line_ops(base, current).iter().fold((0, 0), |(added, removed), op| match *op {
        DiffOp::Equal { .. } => (added, removed),
        DiffOp::Insert { new_len, .. } => (added + new_len, removed),
        DiffOp::Delete { old_len, .. } => (added, removed + old_len),
        DiffOp::Replace { old_len, new_len, .. } => (added + new_len, removed + old_len),
    })
}

/// Takes the last commit back: its changes stay, not committed, and its message comes
/// back, to commit them again (with more, or reworded). Never a commit already pushed
/// (others may have it), a merge, or the first commit.
pub fn undo_last_commit(root: &Path) -> Result<String, String> {
    let commit = run(root, &["rev-list", "--parents", "-n", "1", "HEAD"]).map_err(|_| "There's no commit to undo.")?;
    if run(root, &["symbolic-ref", "-q", "HEAD"]).is_err() {
        return Err("No branch is checked out: switch to one first.".into());
    }
    let git_dir = run(root, &["rev-parse", "--absolute-git-dir"]).map(|d| PathBuf::from(d.trim()))?;
    if ["MERGE_HEAD", "rebase-merge", "rebase-apply", "CHERRY_PICK_HEAD", "REVERT_HEAD"]
        .iter()
        .any(|f| git_dir.join(f).exists())
    {
        return Err("A merge or rebase is under way: finish it first.".into());
    }
    match commit.split_whitespace().count() {
        2 => {}
        1 => return Err("That's the first commit: there's nothing before it to go back to.".into()),
        _ => return Err("The last commit is a merge: undo it from the terminal.".into()),
    }
    if !run(root, &["branch", "-r", "--contains", "HEAD"])?.trim().is_empty() {
        return Err("The last commit is pushed already: undoing it here would part from what others have.".into());
    }
    let message = run(root, &["log", "-1", "--format=%B"])?.trim_end().to_string();
    run(root, &["reset", "-q", "--soft", "HEAD~1"])?;
    Ok(message)
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

/// A commit, as the repository's history lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct LogCommit {
    pub id: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub subject: String,
    /// Branches and tags at it (`HEAD -> main`, `origin/main`, `tag: v1`).
    pub refs: Vec<String>,
}

/// The repository's commits, newest first, every branch's (`max` at most).
pub fn log(root: &Path, max: usize) -> Vec<LogCommit> {
    let count = format!("-n{max}");
    let format = "--format=%H%x1f%h%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D%x1e";
    let Some(out) = git(root, &["log", "--branches", "--tags", "--remotes", "HEAD", "--date-order", &count, format]) else { return Vec::new() };
    out.split('\x1e')
        .filter_map(|record| {
            let fields: Vec<&str> = record.trim_start_matches('\n').split('\x1f').collect();
            let [id, short, parents, author, time, subject, refs] = fields.as_slice() else { return None };
            Some(LogCommit {
                id: id.to_string(),
                short: short.to_string(),
                parents: parents.split_whitespace().map(str::to_string).collect(),
                author: author.to_string(),
                time: time.parse().unwrap_or(0),
                subject: subject.to_string(),
                refs: refs.split(", ").filter(|r| !r.is_empty()).map(str::to_string).collect(),
            })
        })
        .collect()
}

/// A commit's whole message.
pub fn message_of(root: &Path, id: &str) -> String {
    git(root, &["log", "-n1", "--format=%B", id]).unwrap_or_default().trim_end().to_string()
}

/// A file a commit changed: its path from the repository's top, its lines added and taken
/// out (none for a picture or other bytes), and how (`A`dded, `M`odified, `D`eleted…).
#[derive(Clone, Debug, PartialEq)]
pub struct CommitFile {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub kind: char,
}

/// The files commit `id` changed (against its first parent, for a merge).
pub fn commit_files(root: &Path, id: &str, first_parent: Option<&str>) -> Vec<CommitFile> {
    let range: Vec<&str> = match first_parent {
        Some(parent) => vec![parent, id],
        None => vec!["--root", id],
    };
    let mut numstat = vec!["diff-tree", "-r", "--no-commit-id", "--numstat", "-M"];
    numstat.extend(&range);
    let mut kinds = vec!["diff-tree", "-r", "--no-commit-id", "--name-status", "-M"];
    kinds.extend(&range);
    let kinds: Vec<(char, String)> = git(root, &kinds)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut parts = l.split('\t');
            let kind = parts.next()?.chars().next()?;
            Some((kind, parts.next_back()?.to_string()))
        })
        .collect();
    git(root, &numstat)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut parts = l.split('\t');
            let (added, removed) = (parts.next()?, parts.next()?);
            let path = parts.next_back()?.to_string();
            // A rename's path is written `old => new`: the new one.
            let path = path.rsplit(" => ").next().unwrap_or(&path).trim_end_matches('}').to_string();
            let kind = kinds.iter().find(|(_, p)| *p == path).map_or('M', |(k, _)| *k);
            Some(CommitFile { path, added: added.parse().unwrap_or(0), removed: removed.parse().unwrap_or(0), kind })
        })
        .collect()
}

/// A file (by its path from the repository's top) as commit `rev` has it.
pub fn text_at(root: &Path, rev: &str, path: &str) -> Option<String> {
    let output = command(root).args(["show", &format!("{rev}:{path}")]).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Puts the work at commit `id` (no branch: looking at it, as it was).
pub fn check_out(root: &Path, id: &str) -> Result<(), String> {
    run(root, &["checkout", "-q", id]).map(|_| ())
}

/// A new commit taking commit `id`'s changes back.
pub fn revert_commit(root: &Path, id: &str) -> Result<(), String> {
    run(root, &with_mainline(root, &["revert", "--no-edit"], id)).map(|_| ())
}

/// Commit `id`'s changes, made again on this branch as a commit of its own.
pub fn cherry_pick(root: &Path, id: &str) -> Result<(), String> {
    run(root, &with_mainline(root, &["cherry-pick"], id)).map(|_| ())
}

/// `args` then `id`; a merge's changes are the ones it brought to its first parent (`-m 1`).
fn with_mainline<'a>(root: &Path, args: &[&'a str], id: &'a str) -> Vec<&'a str> {
    let parents = git(root, &["rev-list", "--parents", "-n", "1", id]).map_or(0, |l| l.split_whitespace().count() - 1);
    let mut all = args.to_vec();
    if parents > 1 {
        all.extend(["-m", "1"]);
    }
    all.push(id);
    all
}

/// The file as it was in `commit`, in its own encoding.
pub fn file_at(path: &Path, commit: &FileCommit, encoding: crate::encoding::Encoding) -> Option<String> {
    let top = git(path.parent()?, &["rev-parse", "--show-toplevel"])?;
    let output =
        command(Path::new(top.trim())).args(["show", &format!("{}:{}", commit.hash, commit.path)]).output().ok()?;
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

/// Asks the remotes what's new, so the counts to push and pull are current. Only when
/// asked: Null never goes to the network on its own.
pub fn fetch(root: &Path) -> Result<(), String> {
    run(root, &["fetch", "-q", "--prune"]).map(|_| ())
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

/// A file back as it was in the last commit; a file the last commit doesn't have goes to
/// the Trash (and out of what's staged), whatever `status` says: never deleted.
pub fn revert(root: &Path, path: &Path, status: FileStatus) -> Result<(), String> {
    let name = path.to_string_lossy();
    // A folder that isn't new (a submodule): taken back by git, never trashed whole.
    if status != FileStatus::Added && path.is_dir() {
        return run(root, &["restore", "--source=HEAD", "--staged", "--worktree", "--", &name]).map(|_| ());
    }
    if status == FileStatus::Added || (path.exists() && !in_last_commit(path)) {
        crate::fs_ops::move_to_trash(path)?;
        run(root, &["rm", "-q", "--cached", "--ignore-unmatch", "--", &name]).ok();
        return Ok(());
    }
    // What's thrown away is kept in the file's history first (git writes it, not Null).
    if path.is_file() {
        let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        crate::local_history::keep_before_writing(&real, &[]);
    }
    run(root, &["restore", "--source=HEAD", "--staged", "--worktree", "--", &name]).map(|_| ())
}

/// Whether the last commit has the file at `path` (its tree lists it: a submodule's commit
/// isn't in this repository, so asking for the object itself would say no).
fn in_last_commit(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else { return false };
    command(dir)
        .args(["ls-tree", "HEAD", "--", name])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
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

/// Sets every change since the last commit aside (new files too, only under `root`): the
/// files go back to the last commit until `bring_back` returns the changes.
pub fn set_aside(root: &Path) -> Result<(), String> {
    let latest_stash = || git(root, &["rev-parse", "-q", "--verify", "refs/stash"]);
    let before = latest_stash();
    run(root, &["stash", "push", "-q", "--include-untracked", "-m", "Null: set aside", "--", "."])
        .map_err(|e| format!("Couldn't set the changes aside: {e}"))?;
    if latest_stash() == before {
        Err("Nothing to set aside: nothing changed since the last commit.".into())
    } else {
        Ok(())
    }
}

/// Switches to `branch` taking the changes not committed along: set aside, switched,
/// put back. The files they conflict in, if putting them back does (they're left marked).
pub fn switch_carrying_changes(root: &Path, branch: &Branch) -> Result<Vec<PathBuf>, String> {
    let latest_stash = || git(root, &["rev-parse", "-q", "--verify", "refs/stash"]);
    let before = latest_stash();
    run(root, &["stash", "push", "-q", "--include-untracked", "-m", &format!("Null: carried to {}", branch.name)])
        .map_err(|e| format!("Couldn't set the changes aside: {e}"))?;
    // Nothing set aside (no changes after all): there's nothing to put back either, and
    // popping would bring back some older stash instead.
    let set_aside = latest_stash() != before;
    if let Err(error) = switch_branch(root, branch) {
        // Back as they were, on the branch they were on.
        if set_aside {
            run(root, &["stash", "pop", "-q"]).ok();
        }
        return Err(error);
    }
    if !set_aside {
        return Ok(Vec::new());
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

/// A branch on this computer, with how far it is from the one it tracks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalBranch {
    pub name: String,
    pub current: bool,
    /// The remote branch it tracks ("origin/dev").
    pub upstream: Option<String>,
    /// Its commits not there yet, and that one's not here yet.
    pub ahead: usize,
    pub behind: usize,
}

/// The branches on this computer, the one checked out first, then the most recently worked on.
pub fn local_branches(root: &Path) -> Vec<LocalBranch> {
    let list = git(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(HEAD)%09%(refname:short)%09%(upstream:short)%09%(upstream:track,nobracket)",
            "refs/heads",
        ],
    )
    .unwrap_or_default();
    let mut found: Vec<LocalBranch> = list
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (head, name) = (parts.next()?, parts.next()?);
            let upstream = parts.next().filter(|u| !u.is_empty()).map(str::to_string);
            let track = parts.next().unwrap_or("");
            // "ahead 2, behind 3", "ahead 2", "gone", or nothing.
            let count = |word: &str| {
                track
                    .split(", ")
                    .find_map(|part| part.strip_prefix(word))
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(0)
            };
            Some(LocalBranch {
                name: name.to_string(),
                current: head == "*",
                upstream,
                ahead: count("ahead "),
                behind: count("behind "),
            })
        })
        .collect();
    found.sort_by_key(|b| !b.current);
    found
}

/// The remotes' branches ("origin/dev"), by name.
pub fn remote_branches(root: &Path) -> Vec<String> {
    let list = git(root, &["for-each-ref", "--sort=refname", "--format=%(refname:short)", "refs/remotes"]);
    list.unwrap_or_default()
        .lines()
        // ("origin" alone: its HEAD, pointing at one of the others.)
        .filter(|name| name.contains('/') && !name.ends_with("/HEAD"))
        .map(str::to_string)
        .collect()
}

/// The tags, newest first.
pub fn tags(root: &Path) -> Vec<String> {
    let list = git(root, &["for-each-ref", "--sort=-creatordate", "--format=%(refname:short)", "refs/tags"]);
    list.unwrap_or_default().lines().map(str::to_string).collect()
}

/// Changes set aside (a stash).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetAside {
    /// Its place among them: 0 the latest.
    pub index: usize,
    /// What it says: the message given, or what the commit it came from said.
    pub message: String,
    /// The branch it was set aside on.
    pub branch: String,
    pub time: i64,
}

/// The changes set aside, latest first.
pub fn set_aside_list(root: &Path) -> Vec<SetAside> {
    let list = git(root, &["stash", "list", "--format=%gs%x1f%ct"]).unwrap_or_default();
    list.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let (subject, time) = line.split_once('\x1f')?;
            // "On main: the message", or "WIP on main: 1a2b3c the commit's subject".
            let (on, message) = subject.split_once(": ")?;
            let branch = on.trim_start_matches("WIP ").trim_start_matches("On ").trim_start_matches("on ").to_string();
            let message = if on.starts_with("WIP") {
                message.split_once(' ').map_or(message, |(_, rest)| rest).to_string()
            } else {
                message.to_string()
            };
            Some(SetAside { index, message, branch, time: time.trim().parse().unwrap_or(0) })
        })
        .collect()
}

/// Brings back the changes set aside at `index` (see `set_aside_list`; 0 the latest, made
/// by `set_aside` or elsewhere): the files they conflict in, if any (left marked, the
/// changes still kept aside).
pub fn bring_back_at(root: &Path, index: usize) -> Result<Vec<PathBuf>, String> {
    let at = format!("stash@{{{index}}}");
    if git(root, &["rev-parse", "-q", "--verify", &at]).is_none() {
        return Err("Nothing is set aside.".into());
    }
    match run(root, &["stash", "pop", "-q", &at]) {
        Ok(_) => Ok(Vec::new()),
        Err(error) => {
            let conflicted = conflicted(root);
            if conflicted.is_empty() { Err(format!("Couldn't bring them back: {error}")) } else { Ok(conflicted) }
        }
    }
}

/// Throws away the changes set aside at `index`.
pub fn drop_set_aside(root: &Path, index: usize) -> Result<(), String> {
    run(root, &["stash", "drop", "-q", &format!("stash@{{{index}}}")]).map(|_| ())
}

pub fn rename_branch(root: &Path, old: &str, new: &str) -> Result<(), String> {
    run(root, &["branch", "-m", old, new]).map(|_| ()).map_err(|e| explain_switch_error(&e))
}

/// What `delete_branch` says when the branch has commits no other branch has.
pub const NOT_MERGED: &str = "That branch has commits merged nowhere else.";

/// Deletes a branch on this computer; one with commits merged nowhere else only when
/// `anyway`.
pub fn delete_branch(root: &Path, name: &str, anyway: bool) -> Result<(), String> {
    run(root, &["branch", if anyway { "-D" } else { "-d" }, name]).map(|_| ()).map_err(|e| {
        if e.contains("not fully merged") { NOT_MERGED.into() } else { explain_switch_error(&e) }
    })
}

/// The files git has left conflicting (in a merge, a rebase, changes brought back).
pub fn conflicted(root: &Path) -> Vec<PathBuf> {
    status(root).into_iter().filter(|(_, s)| *s == FileStatus::Conflicted).map(|(p, _)| p).collect()
}

/// Merges `branch` into the one checked out: the files it conflicts in, if it does (git
/// then waits: see `joining`).
pub fn merge_in(root: &Path, branch: &str) -> Result<Vec<PathBuf>, String> {
    joined(root, run_without_editor(root, &["merge", "-q", "--no-edit", branch]))
}

/// Puts the branch checked out's own commits on top of `branch`, one by one: the files
/// the one it's at conflicts in, if any.
pub fn rebase_onto(root: &Path, branch: &str) -> Result<Vec<PathBuf>, String> {
    joined(root, run_without_editor(root, &["rebase", "-q", branch]))
}

fn joined(root: &Path, result: Result<String, String>) -> Result<Vec<PathBuf>, String> {
    match result {
        Ok(_) => Ok(Vec::new()),
        Err(error) => {
            let conflicted = conflicted(root);
            if conflicted.is_empty() { Err(error) } else { Ok(conflicted) }
        }
    }
}

/// git, with nothing to open an editor for a message: the one it has is taken.
fn run_without_editor(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = command(dir)
        .env("GIT_EDITOR", "true")
        .args(args)
        .output()
        .map_err(|e| format!("Couldn't run git: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let said = [output.stderr, output.stdout].concat();
    let error = String::from_utf8_lossy(&said);
    let message: Vec<&str> = error.lines().filter(|l| !l.trim().is_empty()).take(3).collect();
    Err(message.join(" ").trim().to_string())
}

/// What git is in the middle of, waiting for conflicts to be settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joining {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// A merge (or rebase, cherry-pick, revert) git is waiting in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoiningState {
    pub kind: Joining,
    /// What's coming in: the branch merged or rebased onto, or the commit.
    pub other: String,
    /// The branch it's coming into.
    pub into: String,
}

/// What git is in the middle of, if anything.
pub fn joining(root: &Path) -> Option<JoiningState> {
    let dir = PathBuf::from(git(root, &["rev-parse", "--absolute-git-dir"])?.trim());
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string());
    let name_of = |id: &str| {
        git(root, &["name-rev", "--name-only", "--no-undefined", "--exclude=tags/*", id])
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty() && !n.contains('~') && !n.contains('^'))
            .unwrap_or_else(|| id.chars().take(7).collect())
    };
    let into = || current_branch(root).unwrap_or_default();
    let (kind, other, into) = if let Some(head) = read("MERGE_HEAD") {
        // "Merge branch 'dev' into …": the name as merged.
        let message = read("MERGE_MSG").unwrap_or_default();
        let named = message.lines().next().and_then(|l| l.split('\'').nth(1)).map(str::to_string);
        (Joining::Merge, named.unwrap_or_else(|| name_of(head.lines().next().unwrap_or(""))), into())
    } else if let Some(onto) = read("rebase-merge/onto").or_else(|| read("rebase-apply/onto")) {
        let branch = read("rebase-merge/head-name").or_else(|| read("rebase-apply/head-name")).unwrap_or_default();
        (Joining::Rebase, name_of(&onto), branch.trim_start_matches("refs/heads/").to_string())
    } else if let Some(id) = read("CHERRY_PICK_HEAD") {
        (Joining::CherryPick, id.chars().take(7).collect(), into())
    } else {
        let id = read("REVERT_HEAD")?;
        (Joining::Revert, id.chars().take(7).collect(), into())
    };
    Some(JoiningState { kind, other, into })
}

/// Whether a file still has conflict markers in it.
pub fn still_marked(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.lines().any(|l| l.starts_with("<<<<<<< ")))
}

/// Stops a merge (rebase, cherry-pick, revert): everything back as it was before it.
pub fn stop_joining(root: &Path, kind: Joining) -> Result<(), String> {
    let what = match kind {
        Joining::Merge => "merge",
        Joining::Rebase => "rebase",
        Joining::CherryPick => "cherry-pick",
        Joining::Revert => "revert",
    };
    run(root, &[what, "--abort"]).map(|_| ())
}

/// Finishes a merge (rebase, cherry-pick, revert) once its conflicts are settled: the
/// files settled marked so, then its commit made. A rebase can stop at its next commit,
/// conflicting too: those files.
pub fn finish_joining(root: &Path, kind: Joining) -> Result<Vec<PathBuf>, String> {
    let conflicted = conflicted(root);
    let marked = conflicted.iter().filter(|p| still_marked(p)).count();
    match marked {
        0 => {}
        1 => return Err("A file still has conflicts: ⌘. resolves each.".into()),
        n => return Err(format!("{n} files still have conflicts: ⌘. resolves each.")),
    }
    if !conflicted.is_empty() {
        let names: Vec<String> = conflicted.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let mut args = vec!["add", "--"];
        args.extend(names.iter().map(String::as_str));
        run(root, &args)?;
    }
    let result = match kind {
        Joining::Merge => run_without_editor(root, &["commit", "-q", "--no-edit"]),
        Joining::Rebase => run_without_editor(root, &["rebase", "--continue"]),
        Joining::CherryPick => run_without_editor(root, &["cherry-pick", "--continue"]),
        Joining::Revert => run_without_editor(root, &["revert", "--continue"]),
    };
    joined(root, result)
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
    let mut child = command(dir)
        .args(["blame", "--porcelain", "-L", &range, "--contents", "-", "--", name])
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
    let output = command(dir).args(args).output().map_err(|e| format!("Couldn't run git: {e}"))?;
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

/// The changes from `before` to `after`, line by line (each line with its line break).
/// The lines are compared as they are: as `TextDiff::from_lines` finds them, many times
/// quicker on a long file (it's run as the text changes: a review, the gutter's marks).
/// A whole long file rewritten line by line takes long to compare well: after a moment,
/// it's compared roughly (bigger changes), not frozen on.
pub fn line_ops(before: &str, after: &str) -> Vec<similar::DiffOp> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
    similar::capture_diff_slices_deadline(similar::Algorithm::Myers, &lines(before), &lines(after), Some(deadline))
}

/// `text`'s lines, each with its line break: a line ends at \n, \r\n or a lone \r, as the
/// editor's lines do (and `TextDiff::from_lines`'s).
pub fn lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' || (b == b'\r' && bytes.get(i + 1) != Some(&b'\n')) {
            lines.push(&text[start..=i]);
            start = i + 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// How the current text differs from `base`, line by line.
pub fn diff(base: &str, current: &str) -> Vec<Hunk> {
    use similar::DiffOp;
    line_ops(base, current)
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

    /// The quick comparison finds what the usual one does (lines changed, added, removed,
    /// and a last line without its line break).
    #[test]
    fn lines_are_compared_as_text_diff_does() {
        let cases = [
            ("a\nb\nc\n", "a\nB\nc\nd\n"),
            ("a\nb\nc", "a\nc"),
            ("", "x\n"),
            ("one\ntwo\n", "one\ntwo"),
            ("x\ny\nx\ny\n", "y\nx\ny\nx\n"),
            ("a\rb\rc", "a\rB\rc\r\nd"),
            ("one\r\ntwo\r\n", "one\r\n2\r\n"),
        ];
        for (before, after) in cases {
            let usual = similar::TextDiff::from_lines(before, after).ops().to_vec();
            assert_eq!(line_ops(before, after), usual, "{before:?} → {after:?}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn finds_the_committed_text_through_a_link() {
        let dir = crate::tools::test_dir("git");
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

    /// The history: each commit with its parents, refs and files; a file as a commit had it.
    #[test]
    fn the_history_and_a_commit_s_files() {
        let repo = crate::tools::test_dir("git-log");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| git(&repo, args);
        if run(&["init", "-q", "-b", "main"]).is_none() {
            return;
        }
        run(&["config", "user.name", "t"]).unwrap();
        run(&["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "second\n\nwhy it is"]).unwrap();
        run(&["tag", "v1"]).unwrap();
        // Changes set aside are commits too, but not the history's.
        std::fs::write(repo.join("a.txt"), "aside\n").unwrap();
        run(&["stash", "-q"]).unwrap();
        let log = log(&repo, 10);
        assert_eq!(log.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["second", "first"]);
        assert_eq!(log[0].parents, [log[1].id.clone()]);
        assert!(log[0].refs.iter().any(|r| r == "tag: v1"), "{:?}", log[0].refs);
        assert_eq!(message_of(&repo, &log[0].id), "second\n\nwhy it is");
        let files = commit_files(&repo, &log[0].id, Some(&log[1].id));
        assert_eq!(files.len(), 2);
        let a = files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((a.added, a.removed, a.kind), (1, 0, 'M'));
        assert_eq!(files.iter().find(|f| f.path == "b.txt").unwrap().kind, 'A');
        assert_eq!(commit_files(&repo, &log[1].id, None).len(), 1, "the first commit's own");
        assert_eq!(text_at(&repo, &log[1].id, "a.txt").as_deref(), Some("one\n"));
        // Taken back: a new commit; made again: another.
        revert_commit(&repo, &log[0].id).unwrap();
        assert!(!repo.join("b.txt").exists());
        cherry_pick(&repo, &log[0].id).unwrap();
        assert!(repo.join("b.txt").exists());
        // A merge, taken back: what it brought in goes.
        run(&["checkout", "-q", "-b", "side", &log[1].id]).unwrap();
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "side"]).unwrap();
        run(&["checkout", "-q", "main"]).unwrap();
        run(&["merge", "-q", "--no-ff", "side", "-m", "merge side"]).unwrap();
        assert!(repo.join("c.txt").exists());
        revert_commit(&repo, "HEAD").unwrap();
        assert!(!repo.join("c.txt").exists());
        std::fs::remove_dir_all(&repo).ok();
    }

    /// Branches made, renamed, merged in (conflicting, settled, finished), rebased, deleted;
    /// changes set aside and brought back by their place.
    #[test]
    fn branches_and_merging() {
        let repo = crate::tools::test_dir("git-branching");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let repo = std::fs::canonicalize(&repo).unwrap();
        let run = |args: &[&str]| git(&repo, args);
        if run(&["init", "-q", "-b", "main"]).is_none() {
            return;
        }
        run(&["config", "user.name", "t"]).unwrap();
        run(&["config", "user.email", "t@t"]).unwrap();
        let write = |name: &str, text: &str| std::fs::write(repo.join(name), text).unwrap();
        write("a.txt", "one\n");
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "first"]).unwrap();
        run(&["tag", "v1"]).unwrap();
        create_branch(&repo, "side").unwrap();
        write("a.txt", "side\n");
        run(&["commit", "-q", "-am", "side's"]).unwrap();
        run(&["switch", "-q", "main"]).unwrap();
        write("a.txt", "main\n");
        run(&["commit", "-q", "-am", "main's"]).unwrap();
        let local = local_branches(&repo);
        assert_eq!(local.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["main", "side"], "the current first");
        assert!(local[0].current && local[0].upstream.is_none());
        assert_eq!(tags(&repo), ["v1"]);
        rename_branch(&repo, "side", "other").unwrap();
        assert!(local_branches(&repo).iter().any(|b| b.name == "other"));
        assert!(joining(&repo).is_none());

        // Both changed the line: a conflict, git waiting.
        let conflicts = merge_in(&repo, "other").unwrap();
        assert_eq!(conflicts, [repo.join("a.txt")]);
        let state = joining(&repo).unwrap();
        assert_eq!((state.kind, state.other.as_str(), state.into.as_str()), (Joining::Merge, "other", "main"));
        assert!(finish_joining(&repo, Joining::Merge).unwrap_err().contains("still has conflicts"));
        write("a.txt", "both\n");
        assert_eq!(finish_joining(&repo, Joining::Merge).unwrap(), Vec::<PathBuf>::new());
        assert!(joining(&repo).is_none());
        assert_eq!(log(&repo, 1)[0].parents.len(), 2, "a merge commit");

        // Stopped: as before it.
        create_branch(&repo, "third").unwrap();
        write("a.txt", "third\n");
        run(&["commit", "-q", "-am", "third's"]).unwrap();
        run(&["switch", "-q", "main"]).unwrap();
        write("a.txt", "main again\n");
        run(&["commit", "-q", "-am", "main again"]).unwrap();
        assert_eq!(rebase_onto(&repo, "third").unwrap().len(), 1);
        assert_eq!(joining(&repo).unwrap().kind, Joining::Rebase);
        assert_eq!(joining(&repo).unwrap().into, "main");
        stop_joining(&repo, Joining::Rebase).unwrap();
        assert!(joining(&repo).is_none());
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "main again\n");

        // Merged nowhere: deleted only when asked again.
        assert_eq!(delete_branch(&repo, "third", false).unwrap_err(), NOT_MERGED);
        delete_branch(&repo, "third", true).unwrap();
        delete_branch(&repo, "other", false).unwrap();
        assert_eq!(local_branches(&repo).len(), 1);

        // Two sets aside; the older one brought back first.
        write("a.txt", "first set\n");
        set_aside(&repo).unwrap();
        write("b.txt", "second set\n");
        set_aside(&repo).unwrap();
        let sets = set_aside_list(&repo);
        assert_eq!(sets.len(), 2);
        assert_eq!((sets[0].branch.as_str(), sets[0].message.as_str()), ("main", "Null: set aside"));
        assert_eq!(bring_back_at(&repo, 1).unwrap(), Vec::<PathBuf>::new());
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "first set\n");
        assert!(!repo.join("b.txt").exists());
        drop_set_aside(&repo, 0).unwrap();
        assert!(set_aside_list(&repo).is_empty());
        std::fs::remove_dir_all(&repo).ok();
    }

    /// Some of a file's changes: the committed text with those only.
    #[test]
    fn part_of_a_file_s_changes() {
        let base = "a\nb\nc\nd\n";
        let current = "a\nB\nc\nd\ne\n";
        assert_eq!(diff(base, current).len(), 2);
        assert_eq!(partly(base, current, &[true, false]), "a\nB\nc\nd\n");
        assert_eq!(partly(base, current, &[false, true]), "a\nb\nc\nd\ne\n");
        assert_eq!(partly(base, current, &[true, true]), current);
        assert_eq!(partly(base, current, &[false, false]), base);
        // A line taken out, kept or not.
        assert_eq!(partly("x\ny\nz\n", "x\nz\n", &[false]), "x\ny\nz\n");
        assert_eq!(counts(base, current), (2, 1));
    }

    /// Committing a file partly: its part committed, the rest still a change; a file left
    /// out stays out.
    #[test]
    fn committing_part_of_a_file() {
        let repo = crate::tools::test_dir("git-parts");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| git(&repo, args);
        if run(&["init", "-q"]).is_none() {
            return;
        }
        run(&["config", "user.name", "t"]).unwrap();
        run(&["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("a.txt"), "a\nb\nc\n").unwrap();
        std::fs::write(repo.join("b.txt"), "one\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "x"]).unwrap();
        std::fs::write(repo.join("a.txt"), "A\nb\nc\nd\n").unwrap();
        std::fs::write(repo.join("b.txt"), "two\n").unwrap();
        let top = PathBuf::from(git(&repo, &["rev-parse", "--show-toplevel"]).unwrap().trim());
        let part = partly("a\nb\nc\n", "A\nb\nc\nd\n", &[true, false]);
        commit_parts(&repo, "part", &[top.join("b.txt")], &[(top.join("a.txt"), part)]).unwrap();
        assert_eq!(run(&["show", "HEAD:a.txt"]).unwrap(), "A\nb\nc\n");
        assert_eq!(run(&["show", "HEAD:b.txt"]).unwrap(), "one\n");
        // On disk as it was; what's left still a change.
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "A\nb\nc\nd\n");
        let left: Vec<_> = status(&repo).into_iter().map(|(p, _)| p).collect();
        assert!(left.contains(&top.join("a.txt")) && left.contains(&top.join("b.txt")), "{left:?}");
        std::fs::remove_dir_all(&repo).ok();
    }

    /// A renamed file: the new name is new (taking it back trashes it, never deletes it),
    /// the old one deleted (taking it back restores it).
    #[test]
    #[cfg(unix)]
    fn a_rename_is_a_new_file_and_a_deleted_one() {
        let repo = crate::tools::test_dir("git-rename");
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
        let repo = crate::tools::test_dir("git-carry");
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
        let repo = crate::tools::test_dir("git-link");
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
        let repo = crate::tools::test_dir("git-history");
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
        let dir = crate::tools::test_dir("git-pull");
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
        // Not known until fetched.
        assert_eq!(ahead_behind(&b), Some((0, 0)));
        fetch(&b).unwrap();
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
        let repo = crate::tools::test_dir("git-partial");
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

    /// Taking changes back never loses work it shouldn't: a name is a name, not a pattern;
    /// a file only meant to be added (`git add -N`) goes to the Trash, not away; and a
    /// project inside a bigger repository sees only its own changes.
    #[test]
    #[cfg(unix)]
    fn reverting_touches_only_what_it_names() {
        let repo = crate::tools::test_dir("git-literal");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("app/[id]")).unwrap();
        std::fs::create_dir_all(repo.join("app/d")).unwrap();
        std::fs::create_dir_all(repo.join("other")).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        for f in ["app/[id]/page.tsx", "app/d/page.tsx", "other/o.txt"] {
            std::fs::write(repo.join(f), "old\n").unwrap();
        }
        commit_all(&repo, "first").unwrap();
        for f in ["app/[id]/page.tsx", "app/d/page.tsx", "other/o.txt"] {
            std::fs::write(repo.join(f), "new\n").unwrap();
        }
        let top = PathBuf::from(git(&repo, &["rev-parse", "--show-toplevel"]).unwrap().trim());
        revert(&repo, &top.join("app/[id]/page.tsx"), FileStatus::Modified).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("app/[id]/page.tsx")).unwrap(), "old\n");
        assert_eq!(std::fs::read_to_string(repo.join("app/d/page.tsx")).unwrap(), "new\n", "a pattern took it back");
        // Only meant to be added: new, and taking it back keeps it (in the Trash).
        std::fs::write(repo.join("app/planned.ts"), "plan\n").unwrap();
        run(&repo, &["add", "-N", "app/planned.ts"]).unwrap();
        let app = status(&repo.join("app"));
        assert!(app.contains(&(top.join("app/planned.ts"), FileStatus::Added)), "{app:?}");
        // The project is app/: other/ isn't its business.
        assert!(app.iter().all(|(p, _)| p.starts_with(top.join("app"))), "{app:?}");
        revert(&repo.join("app"), &top.join("app/planned.ts"), FileStatus::Modified).unwrap();
        assert!(!repo.join("app/planned.ts").exists());
        assert!(
            crate::fs_ops::test_trash().join("planned.ts").exists() || {
                let trash = std::fs::read_dir(crate::fs_ops::test_trash()).unwrap();
                trash.filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().starts_with("planned.ts"))
            }
        );
        assert!(!status(&repo).iter().any(|(p, _)| p.ends_with("planned.ts")), "still staged");
        std::fs::remove_dir_all(&repo).ok();
    }

    /// Set Changes Aside, then Bring Back Changes: the files go back to the last commit,
    /// then their changes return, new files too.
    #[test]
    #[cfg(unix)]
    fn changes_set_aside_come_back() {
        let repo = crate::tools::test_dir("git-set-aside");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("f.txt"), "one\n").unwrap();
        commit_all(&repo, "first").unwrap();
        assert!(bring_back_at(&repo, 0).unwrap_err().contains("Nothing is set aside"));
        assert!(set_aside(&repo).unwrap_err().contains("Nothing to set aside"));
        std::fs::write(repo.join("f.txt"), "two\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new\n").unwrap();
        set_aside(&repo).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "one\n");
        assert!(!repo.join("new.txt").exists());
        assert!(status(&repo).is_empty());
        assert_eq!(bring_back_at(&repo, 0), Ok(Vec::new()));
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "two\n");
        assert_eq!(std::fs::read_to_string(repo.join("new.txt")).unwrap(), "new\n");
        std::fs::remove_dir_all(&repo).ok();
    }

    /// Undo Last Commit: the changes stay, the message comes back; a pushed one stays put.
    #[test]
    #[cfg(unix)]
    fn the_last_commit_comes_undone() {
        let repo = crate::tools::test_dir("git-undo-commit");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        if run(&repo, &["init", "-q"]).is_err() {
            return; // No git here.
        }
        run(&repo, &["config", "user.name", "t"]).unwrap();
        run(&repo, &["config", "user.email", "t@t"]).unwrap();
        std::fs::write(repo.join("f.txt"), "one\n").unwrap();
        commit_all(&repo, "first").unwrap();
        assert!(undo_last_commit(&repo).unwrap_err().contains("first commit"));
        std::fs::write(repo.join("f.txt"), "two\n").unwrap();
        commit_all(&repo, "Second\n\nWith a body.").unwrap();
        assert_eq!(undo_last_commit(&repo).unwrap(), "Second\n\nWith a body.");
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "two\n");
        assert_eq!(git(&repo, &["log", "-1", "--format=%s"]).unwrap().trim(), "first");
        assert_eq!(status(&repo).iter().map(|(_, s)| *s).collect::<Vec<_>>(), [FileStatus::Modified]);
        // Pushed (a remote branch has it): it stays.
        commit_all(&repo, "Second").unwrap();
        run(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]).unwrap();
        assert!(undo_last_commit(&repo).unwrap_err().contains("pushed"));
        assert_eq!(git(&repo, &["log", "-1", "--format=%s"]).unwrap().trim(), "Second");
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    #[cfg(unix)]
    fn status_commit_and_revert() {
        let repo = crate::tools::test_dir("git-status");
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
        let repo = crate::tools::test_dir("git-branches");
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
        let repo = crate::tools::test_dir("git-blame");
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
