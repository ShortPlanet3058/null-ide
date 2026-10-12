//! GitHub, through its `gh` command, as the person signed in there: pull requests and
//! issues to look at, review and merge; a pull request made for the branch. Nothing here
//! signs in or keeps a token (gh does), and nothing goes to the network unasked: only when
//! the GitHub tab is shown, refreshed, or something on it is clicked.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A stand-in for gh, in tests: they never reach the real one (or GitHub).
#[cfg(test)]
pub static FAKE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

fn program() -> Option<PathBuf> {
    #[cfg(test)]
    return FAKE.lock().unwrap().clone();
    #[cfg(not(test))]
    crate::tools::find("gh")
}

/// gh, run in `dir`, never stopping to ask anything (there's no terminal to answer in).
fn gh(dir: &Path) -> Result<Command, String> {
    let program = program().ok_or_else(|| MISSING.to_string())?;
    let mut command = crate::tools::command(program);
    command
        .current_dir(dir)
        .env("PATH", crate::tools::search_path())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    Ok(command)
}

pub const MISSING: &str = "GitHub works through its gh command: install it (cli.github.com), then run “gh auth login”.";

fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = gh(dir)?.args(args).output().map_err(|e| format!("Couldn't run gh: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    let message: Vec<&str> = error.lines().filter(|l| !l.trim().is_empty()).take(2).collect();
    Err(message.join(" ").trim().to_string())
}

fn run_json(dir: &Path, args: &[&str]) -> Result<Value, String> {
    let out = run(dir, args)?;
    serde_json::from_str(&out).map_err(|e| format!("gh said something unexpected: {e}"))
}

/// The repository on GitHub, as gh resolves it for `gh pr` (its set default, else the
/// remotes): requests to its API name it, so they reach the same one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub host: String,
    /// "owner/name".
    pub name: String,
}

impl Repo {
    /// `gh api` arguments for `rest` under this repository ("pulls/5/files").
    fn api<'a>(&'a self, path: &'a str) -> [&'a str; 4] {
        ["api", "--hostname", &self.host, path]
    }

    fn path(&self, rest: &str) -> String {
        format!("repos/{}/{rest}", self.name)
    }
}

pub fn repo(dir: &Path) -> Result<Repo, String> {
    let view = run_json(dir, &["repo", "view", "--json", "nameWithOwner,url"])?;
    let name = text(&view, "nameWithOwner");
    let url = text(&view, "url");
    let host = url.split("://").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or("github.com").to_string();
    if name.is_empty() {
        return Err("gh found no GitHub repository here.".into());
    }
    Ok(Repo { host, name })
}

/// Who's signed in to gh, if anyone (and whether gh is there at all).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Account {
    Missing,
    SignedOut,
    /// Couldn't ask (offline, no GitHub remote, GitHub not answering): why.
    Failed(String),
    SignedIn(String),
}

fn signed_out(error: &str) -> bool {
    let error = error.to_lowercase();
    ["auth login", "not logged", "authentication", "http 401"].iter().any(|w| error.contains(w))
}

/// Who's signed in, on the host of the repository here (so GitHub Enterprise too).
pub fn account(dir: &Path) -> Account {
    if program().is_none() {
        return Account::Missing;
    }
    let repo = match repo(dir) {
        Ok(repo) => repo,
        Err(error) if signed_out(&error) => return Account::SignedOut,
        Err(error) => return Account::Failed(error),
    };
    match run(dir, &["api", "--hostname", &repo.host, "user", "--jq", ".login"]) {
        Ok(login) if !login.trim().is_empty() => Account::SignedIn(login.trim().to_string()),
        Err(error) if !signed_out(&error) => Account::Failed(error),
        _ => Account::SignedOut,
    }
}

/// How a pull request's checks went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    None,
    Running,
    Passed(usize),
    Failed(usize),
}

fn checks(rollup: &Value) -> Checks {
    let Some(all) = rollup.as_array() else { return Checks::None };
    let (mut failed, mut running) = (0, 0);
    for check in all {
        let word = |key: &str| check.get(key).and_then(Value::as_str).unwrap_or("").to_ascii_uppercase();
        // A check run (status, then conclusion), or an older status (state).
        let (status, outcome) = match check.get("__typename").and_then(Value::as_str) {
            Some("StatusContext") => ("COMPLETED".to_string(), word("state")),
            _ => (word("status"), word("conclusion")),
        };
        if status != "COMPLETED" || matches!(outcome.as_str(), "PENDING" | "EXPECTED" | "") {
            running += 1;
        } else if matches!(
            outcome.as_str(),
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
        ) {
            failed += 1;
        }
    }
    if failed > 0 {
        Checks::Failed(failed)
    } else if running > 0 {
        Checks::Running
    } else if all.is_empty() {
        Checks::None
    } else {
        Checks::Passed(all.len())
    }
}

/// When, from GitHub's "2026-10-11T18:54:49Z".
fn time(value: &Value) -> i64 {
    value.as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map_or(0, |t| t.timestamp())
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn login(value: &Value, key: &str) -> String {
    value.get(key).and_then(|a| a.get("login")).and_then(Value::as_str).unwrap_or("").to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pull {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    /// OPEN, CLOSED or MERGED.
    pub state: String,
    pub checks: Checks,
    pub comments: usize,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, or nothing.
    pub review: String,
    pub updated: i64,
    pub url: String,
}

const PULL_FIELDS: &str =
    "number,title,author,headRefName,baseRefName,isDraft,state,statusCheckRollup,comments,reviewDecision,updatedAt,url";

fn pull(value: &Value) -> Option<Pull> {
    Some(Pull {
        number: value.get("number")?.as_u64()?,
        title: text(value, "title"),
        author: login(value, "author"),
        head: text(value, "headRefName"),
        base: text(value, "baseRefName"),
        draft: value.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        state: text(value, "state"),
        checks: value.get("statusCheckRollup").map_or(Checks::None, checks),
        comments: value.get("comments").and_then(Value::as_array).map_or(0, Vec::len),
        review: text(value, "reviewDecision"),
        updated: value.get("updatedAt").map_or(0, time),
        url: text(value, "url"),
    })
}

pub fn parse_pulls(json: &Value) -> Vec<Pull> {
    json.as_array().map(|all| all.iter().filter_map(pull).collect()).unwrap_or_default()
}

/// Pull requests: `state` open, closed, merged or all; `search` as GitHub's search box.
pub fn pulls(dir: &Path, state: &str, search: Option<&str>) -> Result<Vec<Pull>, String> {
    let mut args = vec!["pr", "list", "--state", state, "--limit", "50", "--json", PULL_FIELDS];
    if let Some(search) = search {
        args.extend(["--search", search]);
    }
    run_json(dir, &args).map(|json| parse_pulls(&json))
}

/// The open pull request of the branch checked out, if it has one (a merged or closed
/// one leaves room for the next).
pub fn pull_for_branch(dir: &Path) -> Option<Pull> {
    run_json(dir, &["pr", "view", "--json", PULL_FIELDS]).ok().as_ref().and_then(pull).filter(|p| p.state == "OPEN")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub comments: usize,
    pub labels: Vec<String>,
    pub updated: i64,
    pub url: String,
}

const ISSUE_FIELDS: &str = "number,title,author,state,comments,labels,updatedAt,url";

fn issue(value: &Value) -> Option<Issue> {
    Some(Issue {
        number: value.get("number")?.as_u64()?,
        title: text(value, "title"),
        author: login(value, "author"),
        state: text(value, "state"),
        comments: value.get("comments").and_then(Value::as_array).map_or(0, Vec::len),
        labels: value
            .get("labels")
            .and_then(Value::as_array)
            .map(|all| all.iter().map(|l| text(l, "name")).collect())
            .unwrap_or_default(),
        updated: value.get("updatedAt").map_or(0, time),
        url: text(value, "url"),
    })
}

pub fn parse_issues(json: &Value) -> Vec<Issue> {
    json.as_array().map(|all| all.iter().filter_map(issue).collect()).unwrap_or_default()
}

pub fn issues(dir: &Path, state: &str, search: Option<&str>) -> Result<Vec<Issue>, String> {
    let mut args = vec!["issue", "list", "--state", state, "--limit", "50", "--json", ISSUE_FIELDS];
    if let Some(search) = search {
        args.extend(["--search", search]);
    }
    run_json(dir, &args).map(|json| parse_issues(&json))
}

/// A comment: in the conversation, or on a line of a file (a review's).
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub time: i64,
    /// A review comment's file (from the repository's top) and line there (1-based).
    pub path: Option<String>,
    pub line: Option<usize>,
    /// The comment this answers, in a review's thread.
    pub reply_to: Option<u64>,
    /// On a line taken out (its line counts the file before).
    pub left: bool,
    /// On a line the pull request has changed since: its line is where it was then.
    pub outdated: bool,
}

fn conversation(value: &Value) -> Vec<Comment> {
    value
        .get("comments")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .map(|c| Comment {
                    id: 0,
                    author: login(c, "author"),
                    body: text(c, "body"),
                    time: c.get("createdAt").map_or(0, time),
                    path: None,
                    line: None,
                    reply_to: None,
                    left: false,
                    outdated: false,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reviews with something to say (their text, or approving or asking for changes), as
/// comments in the conversation.
fn reviews(value: &Value) -> Vec<Comment> {
    let Some(all) = value.get("reviews").and_then(Value::as_array) else { return Vec::new() };
    all.iter()
        .filter_map(|r| {
            let body = text(r, "body");
            let word = match text(r, "state").as_str() {
                "APPROVED" => "approved",
                "CHANGES_REQUESTED" => "asked for changes",
                _ if body.trim().is_empty() => return None,
                _ => "reviewed",
            };
            Some(Comment {
                id: 0,
                author: format!("{} {word}", login(r, "author")),
                body,
                time: r.get("submittedAt").map_or(0, time),
                path: None,
                line: None,
                reply_to: None,
                left: false,
                outdated: false,
            })
        })
        .collect()
}

/// Review comments, one JSON object a line (as `--jq '.[]'` gives them, every page).
pub fn parse_review_comments(lines: &str) -> Vec<Comment> {
    lines
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|c| {
            let now = c.get("line").and_then(Value::as_u64);
            let line = now.or_else(|| c.get("original_line").and_then(Value::as_u64));
            Some(Comment {
                id: c.get("id")?.as_u64()?,
                author: login(&c, "user"),
                body: text(&c, "body"),
                time: c.get("created_at").map_or(0, time),
                path: c.get("path").and_then(Value::as_str).map(str::to_string),
                line: line.map(|l| l as usize),
                reply_to: c.get("in_reply_to_id").and_then(Value::as_u64),
                left: c.get("side").and_then(Value::as_str) == Some("LEFT"),
                outdated: now.is_none() && line.is_some(),
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct PullFile {
    pub path: String,
    /// Where a renamed (or copied) file was, before.
    pub old_path: Option<String>,
    pub added: usize,
    pub removed: usize,
    /// A(dded), D(eleted), R(enamed) or M(odified).
    pub kind: char,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PullDetail {
    pub repo: Repo,
    pub pull: Pull,
    /// Its size, all of it (`files` can be cut short on a huge one).
    pub changed_files: usize,
    pub added: usize,
    pub removed: usize,
    /// Something that couldn't be read (its review comments), to say.
    pub warning: Option<String>,
    pub body: String,
    pub files: Vec<PullFile>,
    pub base_oid: String,
    pub head_oid: String,
    /// MERGEABLE, CONFLICTING or UNKNOWN.
    pub mergeable: String,
    pub conversation: Vec<Comment>,
    pub review: Vec<Comment>,
}

/// The files of a pull request, one JSON object a line (the REST API's, every page).
pub fn parse_files(lines: &str) -> Vec<PullFile> {
    lines
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|f| PullFile {
            path: text(&f, "filename"),
            old_path: f.get("previous_filename").and_then(Value::as_str).map(str::to_string),
            added: f.get("additions").and_then(Value::as_u64).unwrap_or(0) as usize,
            removed: f.get("deletions").and_then(Value::as_u64).unwrap_or(0) as usize,
            kind: match text(&f, "status").as_str() {
                "added" => 'A',
                "removed" => 'D',
                "renamed" | "copied" => 'R',
                _ => 'M',
            },
        })
        .filter(|f| !f.path.is_empty())
        .collect()
}

/// gh's own list of files (at most its first 100, no old paths): when the API's can't be read.
fn view_files(view: &Value) -> Vec<PullFile> {
    view.get("files")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .map(|f| PullFile {
                    path: text(f, "path"),
                    old_path: None,
                    added: f.get("additions").and_then(Value::as_u64).unwrap_or(0) as usize,
                    removed: f.get("deletions").and_then(Value::as_u64).unwrap_or(0) as usize,
                    kind: match text(f, "changeType").as_str() {
                        "ADDED" => 'A',
                        "DELETED" => 'D',
                        "RENAMED" | "COPIED" => 'R',
                        _ => 'M',
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A pull request from `gh pr view`, its files (the API's, or None to use gh's) and its
/// review comments (or why they couldn't be read).
pub fn parse_pull_detail(
    repo: Repo,
    view: &Value,
    files: Option<Vec<PullFile>>,
    review: Result<String, String>,
) -> Option<PullDetail> {
    let count = |key: &str| view.get(key).and_then(Value::as_u64).map(|n| n as usize);
    let mut conversation = conversation(view);
    conversation.extend(reviews(view));
    conversation.sort_by_key(|c| c.time);
    let (review, warning) = match review {
        Ok(lines) => (parse_review_comments(&lines), None),
        Err(error) => (Vec::new(), Some(format!("Its review comments couldn't be read: {error}"))),
    };
    let files = files.unwrap_or_else(|| view_files(view));
    // (Its own totals, else its files' added up.)
    let (added, removed) = files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    Some(PullDetail {
        pull: pull(view)?,
        changed_files: count("changedFiles").unwrap_or(0).max(files.len()),
        added: count("additions").unwrap_or(added),
        removed: count("deletions").unwrap_or(removed),
        repo,
        warning,
        body: text(view, "body"),
        files,
        base_oid: text(view, "baseRefOid"),
        head_oid: text(view, "headRefOid"),
        mergeable: text(view, "mergeable"),
        conversation,
        review,
    })
}

pub fn pull_detail(dir: &Path, number: u64) -> Result<PullDetail, String> {
    let repo = repo(dir)?;
    let n = number.to_string();
    let fields =
        format!("{PULL_FIELDS},body,files,baseRefOid,headRefOid,mergeable,additions,deletions,changedFiles,reviews");
    let view = run_json(dir, &["pr", "view", &n, "--json", &fields])?;
    let every = |rest: &str| {
        let path = repo.path(rest);
        run(dir, &[&repo.api(&path)[..], &["--paginate", "--jq", ".[]"]].concat())
    };
    let files = every(&format!("pulls/{n}/files")).ok().map(|lines| parse_files(&lines)).filter(|f| !f.is_empty());
    let review = every(&format!("pulls/{n}/comments"));
    parse_pull_detail(repo, &view, files, review).ok_or_else(|| "gh said something unexpected.".to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub struct IssueDetail {
    pub issue: Issue,
    pub body: String,
    pub conversation: Vec<Comment>,
}

pub fn issue_detail(dir: &Path, number: u64) -> Result<IssueDetail, String> {
    let fields = format!("{ISSUE_FIELDS},body");
    let view = run_json(dir, &["issue", "view", &number.to_string(), "--json", &fields])?;
    let issue = issue(&view).ok_or("gh said something unexpected.")?;
    Ok(IssueDetail { issue, body: text(&view, "body"), conversation: conversation(&view) })
}

/// Where the pull request's branch left its base: what its changes are compared with.
pub fn merge_base(dir: &Path, repo: &Repo, base: &str, head: &str) -> Option<String> {
    let local = crate::tools::command("git").arg("-C").arg(dir).args(["merge-base", base, head]).output().ok();
    if let Some(out) = local.filter(|o| o.status.success()) {
        return Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let compare = repo.path(&format!("compare/{base}...{head}"));
    run(dir, &[&repo.api(&compare)[..], &["--jq", ".merge_base_commit.sha"]].concat())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A file (by its path from the repository's top) at commit `sha`: from here if the
/// commit's here, else from GitHub.
pub fn text_at(dir: &Path, repo: &Repo, sha: &str, path: &str) -> Option<String> {
    if let Some(text) = crate::git::text_at(dir, sha, path) {
        return Some(text);
    }
    let encoded: String = path
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let contents = repo.path(&format!("contents/{encoded}?ref={sha}"));
    run(dir, &[&repo.api(&contents)[..], &["-H", "Accept: application/vnd.github.raw+json"]].concat()).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeHow {
    Merge,
    Squash,
    Rebase,
}

pub fn merge(dir: &Path, number: u64, how: MergeHow) -> Result<(), String> {
    let how = match how {
        MergeHow::Merge => "--merge",
        MergeHow::Squash => "--squash",
        MergeHow::Rebase => "--rebase",
    };
    run(dir, &["pr", "merge", &number.to_string(), how]).map(|_| ())
}

pub fn approve(dir: &Path, number: u64) -> Result<(), String> {
    run(dir, &["pr", "review", &number.to_string(), "--approve"]).map(|_| ())
}

/// The pull request's branch, checked out here.
pub fn check_out(dir: &Path, number: u64) -> Result<(), String> {
    run(dir, &["pr", "checkout", &number.to_string()]).map(|_| ())
}

/// An answer in a review comment's thread.
pub fn reply(dir: &Path, repo: &Repo, number: u64, comment: u64, body: &str) -> Result<(), String> {
    let path = repo.path(&format!("pulls/{number}/comments/{comment}/replies"));
    let body = format!("body={body}");
    run(dir, &[&repo.api(&path)[..], &["-X", "POST", "-f", &body]].concat()).map(|_| ())
}

/// A comment in a pull request's (or an issue's) conversation.
pub fn comment(dir: &Path, number: u64, body: &str, issue: bool) -> Result<(), String> {
    let kind = if issue { "issue" } else { "pr" };
    run(dir, &[kind, "comment", &number.to_string(), "--body", body]).map(|_| ())
}

/// The repository's main branch on GitHub.
pub fn default_branch(dir: &Path) -> Option<String> {
    run(dir, &["repo", "view", "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A pull request for the branch checked out (pushed first), into `base`: its address.
pub fn create_pull(dir: &Path, title: &str, body: &str, base: &str, draft: bool) -> Result<String, String> {
    let branch = crate::git::push_as_itself(dir)?;
    let mut args = vec!["pr", "create", "--title", title, "--body", body, "--base", base, "--head", &branch];
    if draft {
        args.push("--draft");
    }
    run(dir, &args).map(|out| out.lines().last().unwrap_or("").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_requests_read_from_gh() {
        let json: Value = serde_json::from_str(
            r#"[{"number":370,"title":"Show Supertypes","author":{"login":"ada"},"headRefName":"feature/types",
                "baseRefName":"dev","isDraft":false,"state":"OPEN","comments":[{}],"reviewDecision":"APPROVED",
                "updatedAt":"2026-10-11T18:54:49Z","url":"https://github.com/o/r/pull/370",
                "statusCheckRollup":[
                  {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS"},
                  {"__typename":"StatusContext","context":"ci","state":"SUCCESS"}]},
               {"number":366,"title":"Windows paths","author":{"login":"bo"},"headRefName":"w","baseRefName":"dev",
                "isDraft":true,"state":"OPEN","comments":[],"reviewDecision":"","updatedAt":"2026-10-10T10:00:00Z",
                "url":"u","statusCheckRollup":[{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""}]},
               {"number":362,"title":"Bump gpui","author":{"login":"cy"},"headRefName":"g","baseRefName":"dev",
                "isDraft":false,"state":"OPEN","comments":[],"reviewDecision":"REVIEW_REQUIRED","updatedAt":"",
                "url":"u","statusCheckRollup":[
                  {"__typename":"CheckRun","status":"COMPLETED","conclusion":"FAILURE"},
                  {"__typename":"CheckRun","status":"COMPLETED","conclusion":"TIMED_OUT"},
                  {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SKIPPED"}]}]"#,
        )
        .unwrap();
        let pulls = parse_pulls(&json);
        assert_eq!(pulls.len(), 3);
        assert_eq!((pulls[0].number, pulls[0].author.as_str(), pulls[0].comments), (370, "ada", 1));
        assert_eq!(pulls[0].checks, Checks::Passed(2));
        assert_eq!(pulls[0].updated, 1791744889);
        assert!(pulls[1].draft);
        assert_eq!(pulls[1].checks, Checks::Running);
        assert_eq!(pulls[2].checks, Checks::Failed(2));
    }

    #[test]
    fn a_pull_request_s_files_and_comments() {
        let view: Value = serde_json::from_str(
            r#"{"number":5,"title":"t","author":{"login":"a"},"headRefName":"h","baseRefName":"b","isDraft":false,
                "state":"OPEN","comments":[{"author":{"login":"rev"},"body":"Looks good","createdAt":"2026-10-11T00:00:00Z"}],
                "reviewDecision":"","updatedAt":"","url":"u","statusCheckRollup":[],"body":"Why",
                "files":[{"path":"src/a.rs","additions":3,"deletions":1,"changeType":"MODIFIED"},
                         {"path":"b.md","additions":2,"deletions":0,"changeType":"ADDED"}],
                "baseRefOid":"b1","headRefOid":"h1","mergeable":"MERGEABLE","additions":5,"deletions":1,"changedFiles":2,
                "reviews":[{"author":{"login":"cy"},"body":"","state":"CHANGES_REQUESTED","submittedAt":"2026-10-11T00:02:00Z"},
                           {"author":{"login":"cy"},"body":"","state":"COMMENTED","submittedAt":"2026-10-11T00:03:00Z"}]}"#,
        )
        .unwrap();
        let review = concat!(
            r#"{"id":11,"user":{"login":"rev"},"body":"Which server?","created_at":"2026-10-11T00:00:00Z","path":"src/a.rs","line":4}"#,
            "\n",
            r#"{"id":12,"user":{"login":"a"},"body":"clangd","created_at":"2026-10-11T00:01:00Z","path":"src/a.rs","line":null,"original_line":4,"in_reply_to_id":11,"side":"LEFT"}"#,
            "\n"
        );
        let repo = Repo { host: "github.com".into(), name: "o/r".into() };
        let detail = parse_pull_detail(repo.clone(), &view, None, Ok(review.into())).unwrap();
        assert_eq!(detail.files.iter().map(|f| f.kind).collect::<String>(), "MA", "gh's own list, the API's unread");
        assert_eq!((detail.files[0].added, detail.files[0].removed), (3, 1));
        assert_eq!((detail.changed_files, detail.added, detail.removed), (2, 5, 1));
        assert_eq!(detail.conversation[0].body, "Looks good");
        // A review asking for changes says so; one only commenting, with nothing written, doesn't.
        assert_eq!(detail.conversation.len(), 2);
        assert_eq!(detail.conversation[1].author, "cy asked for changes");
        assert_eq!(detail.review.len(), 2);
        assert_eq!(detail.review[1].line, Some(4), "an outdated comment keeps its first line");
        assert!(detail.review[1].outdated && detail.review[1].left && !detail.review[0].outdated);
        assert_eq!(detail.review[1].reply_to, Some(11));
        assert_eq!(detail.mergeable, "MERGEABLE");
        assert!(detail.warning.is_none());
        // Its comments unread: said, not taken for none.
        let detail = parse_pull_detail(repo, &view, None, Err("HTTP 404".into())).unwrap();
        assert!(detail.warning.unwrap().contains("HTTP 404"));
    }

    /// The API's files: a renamed one with where it was.
    #[test]
    fn a_pull_request_s_files_from_the_api() {
        let lines = concat!(
            r#"{"filename":"src/b.rs","previous_filename":"src/a.rs","status":"renamed","additions":1,"deletions":1}"#,
            "\n",
            r#"{"filename":"gone.txt","status":"removed","additions":0,"deletions":3}"#,
            "\n"
        );
        let files = parse_files(lines);
        assert_eq!(files.len(), 2);
        assert_eq!((files[0].kind, files[0].old_path.as_deref()), ('R', Some("src/a.rs")));
        assert_eq!((files[1].kind, files[1].removed), ('D', 3));
    }
}
