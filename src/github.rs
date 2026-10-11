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
    let mut command = Command::new(program);
    command
        .current_dir(dir)
        .env("PATH", crate::tools::search_path())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    Ok(command)
}

pub const MISSING: &str = "GitHub works through its gh command: install it (cli.github.com), then run gh auth login.";

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

/// Who's signed in to gh, if anyone (and whether gh is there at all).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Account {
    Missing,
    SignedOut,
    SignedIn(String),
}

pub fn account(dir: &Path) -> Account {
    if program().is_none() {
        return Account::Missing;
    }
    match run(dir, &["api", "user", "--jq", ".login"]) {
        Ok(login) if !login.trim().is_empty() => Account::SignedIn(login.trim().to_string()),
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
        } else if matches!(outcome.as_str(), "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE") {
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
    value
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map_or(0, |t| t.timestamp())
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

/// The pull request of the branch checked out, if it has one.
pub fn pull_for_branch(dir: &Path) -> Option<Pull> {
    run_json(dir, &["pr", "view", "--json", PULL_FIELDS]).ok().as_ref().and_then(pull)
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
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Review comments, one JSON object a line (as `--jq '.[]'` gives them, every page).
pub fn parse_review_comments(lines: &str) -> Vec<Comment> {
    lines
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|c| {
            let line = c.get("line").and_then(Value::as_u64).or_else(|| c.get("original_line").and_then(Value::as_u64));
            Some(Comment {
                id: c.get("id")?.as_u64()?,
                author: login(&c, "user"),
                body: text(&c, "body"),
                time: c.get("created_at").map_or(0, time),
                path: c.get("path").and_then(Value::as_str).map(str::to_string),
                line: line.map(|l| l as usize),
                reply_to: c.get("in_reply_to_id").and_then(Value::as_u64),
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct PullFile {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    /// A(dded), D(eleted), R(enamed) or M(odified).
    pub kind: char,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PullDetail {
    pub pull: Pull,
    pub body: String,
    pub files: Vec<PullFile>,
    pub base_oid: String,
    pub head_oid: String,
    /// MERGEABLE, CONFLICTING or UNKNOWN.
    pub mergeable: String,
    pub conversation: Vec<Comment>,
    pub review: Vec<Comment>,
}

pub fn parse_pull_detail(view: &Value, review: &str) -> Option<PullDetail> {
    let files = view
        .get("files")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .map(|f| PullFile {
                    path: text(f, "path"),
                    added: f.get("additions").and_then(Value::as_u64).unwrap_or(0) as usize,
                    removed: f.get("deletions").and_then(Value::as_u64).unwrap_or(0) as usize,
                    kind: match text(f, "changeType").as_str() {
                        "ADDED" => 'A',
                        "DELETED" => 'D',
                        "RENAMED" => 'R',
                        _ => 'M',
                    },
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PullDetail {
        pull: pull(view)?,
        body: text(view, "body"),
        files,
        base_oid: text(view, "baseRefOid"),
        head_oid: text(view, "headRefOid"),
        mergeable: text(view, "mergeable"),
        conversation: conversation(view),
        review: parse_review_comments(review),
    })
}

pub fn pull_detail(dir: &Path, number: u64) -> Result<PullDetail, String> {
    let n = number.to_string();
    let fields = format!("{PULL_FIELDS},body,files,baseRefOid,headRefOid,mergeable");
    let view = run_json(dir, &["pr", "view", &n, "--json", &fields])?;
    let review = run(dir, &["api", "--paginate", &format!("repos/{{owner}}/{{repo}}/pulls/{n}/comments"), "--jq", ".[]"])
        .unwrap_or_default();
    parse_pull_detail(&view, &review).ok_or_else(|| "gh said something unexpected.".to_string())
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
pub fn merge_base(dir: &Path, base: &str, head: &str) -> Option<String> {
    let local = Command::new("git").arg("-C").arg(dir).args(["merge-base", base, head]).output().ok();
    if let Some(out) = local.filter(|o| o.status.success()) {
        return Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let compare = format!("repos/{{owner}}/{{repo}}/compare/{base}...{head}");
    run(dir, &["api", &compare, "--jq", ".merge_base_commit.sha"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// A file (by its path from the repository's top) at commit `sha`: from here if the
/// commit's here, else from GitHub.
pub fn text_at(dir: &Path, sha: &str, path: &str) -> Option<String> {
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
    run(
        dir,
        &["api", "-H", "Accept: application/vnd.github.raw+json", &format!("repos/{{owner}}/{{repo}}/contents/{encoded}?ref={sha}")],
    )
    .ok()
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
pub fn reply(dir: &Path, number: u64, comment: u64, body: &str) -> Result<(), String> {
    let path = format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments/{comment}/replies");
    run(dir, &["api", "-X", "POST", &path, "-f", &format!("body={body}")]).map(|_| ())
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
    crate::git::push(dir)?;
    let mut args = vec!["pr", "create", "--title", title, "--body", body, "--base", base];
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
                "baseRefOid":"b1","headRefOid":"h1","mergeable":"MERGEABLE"}"#,
        )
        .unwrap();
        let review = concat!(
            r#"{"id":11,"user":{"login":"rev"},"body":"Which server?","created_at":"2026-10-11T00:00:00Z","path":"src/a.rs","line":4}"#,
            "\n",
            r#"{"id":12,"user":{"login":"a"},"body":"clangd","created_at":"2026-10-11T00:01:00Z","path":"src/a.rs","line":null,"original_line":4,"in_reply_to_id":11}"#,
            "\n"
        );
        let detail = parse_pull_detail(&view, review).unwrap();
        assert_eq!(detail.files.iter().map(|f| f.kind).collect::<String>(), "MA");
        assert_eq!((detail.files[0].added, detail.files[0].removed), (3, 1));
        assert_eq!(detail.conversation[0].body, "Looks good");
        assert_eq!(detail.review.len(), 2);
        assert_eq!(detail.review[1].line, Some(4), "an outdated comment keeps its first line");
        assert_eq!(detail.review[1].reply_to, Some(11));
        assert_eq!(detail.mergeable, "MERGEABLE");
    }
}
