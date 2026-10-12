//! The sidebar's Changes tab: git, always at hand. What changed since the last commit,
//! file by file and change by change, to commit some or all of it with a message; the
//! branch, with what's waiting to be pushed or pulled.

use crate::git::{self, FileStatus};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui::{self, px};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, Subscription, Task, Window,
    actions, div, prelude::*, svg,
};
use std::ops::Range;
use std::path::{Path, PathBuf};

mod branches;
mod github_tab;

pub use branches::Branches;
pub use github_tab::GitHub;

actions!(git_view, [CommitChanges]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-enter", CommitChanges, Some("GitMessage"))]);
    branches::bind_keys(cx);
    github_tab::bind_keys(cx);
}

/// A change within a file (as `git::diff` finds them), to commit or leave for later.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// Its lines in the file now (an empty range: lines taken out just above).
    pub lines: Range<usize>,
    /// What it says, for its row: its first line of text.
    pub label: String,
    pub kept: bool,
}

/// A file changed since the last commit.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub status: FileStatus,
    pub added: usize,
    pub removed: usize,
    /// In the next commit (some of it, when some of its parts aren't).
    pub kept: bool,
    pub parts: Vec<Part>,
    pub unfolded: bool,
}

impl ChangedFile {
    /// Only some of its changes go in.
    fn partly(&self) -> bool {
        self.kept && self.parts.iter().any(|p| !p.kept) && self.parts.iter().any(|p| p.kept)
    }
}

/// What the tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Changes,
    History,
    Branches,
    GitHub,
}

/// How a commit's row draws its part of the branches' graph: its lane (and colour), and
/// the lines through the row, each from a lane above (or the commit, mid-row) to one below
/// (or the commit).
#[derive(Clone, Debug, PartialEq)]
pub struct GraphRow {
    pub lane: usize,
    pub colour: usize,
    pub edges: Vec<Edge>,
    /// Lanes wide, at most, in this row.
    pub width: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// A lane going past the commit.
    Through { lane: usize, colour: usize },
    /// From a lane above into the commit (its children).
    In { from: usize, colour: usize },
    /// From the commit down to a lane below (its parents).
    Out { to: usize, colour: usize },
}

/// The graph of `commits` (newest first): each one's lane, and the lines past it.
pub fn graph(commits: &[git::LogCommit]) -> Vec<GraphRow> {
    // Each lane: the commit it waits for, and its colour.
    let mut lanes: Vec<Option<(String, usize)>> = Vec::new();
    let mut colours = 0;
    let mut new_colour = || {
        colours += 1;
        colours - 1
    };
    let mut rows = Vec::new();
    for commit in commits {
        let before = lanes.clone();
        let lane = match before.iter().position(|l| l.as_ref().is_some_and(|(id, _)| *id == commit.id)) {
            Some(lane) => lane,
            None => match lanes.iter().position(Option::is_none) {
                Some(free) => free,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            },
        };
        let colour = before.get(lane).cloned().flatten().map_or_else(&mut new_colour, |(_, c)| c);
        let mut edges = Vec::new();
        for (i, slot) in before.iter().enumerate() {
            match slot {
                Some((id, c)) if *id == commit.id => {
                    edges.push(Edge::In { from: i, colour: *c });
                    if i != lane {
                        lanes[i] = None;
                    }
                }
                Some((_, c)) => edges.push(Edge::Through { lane: i, colour: *c }),
                None => {}
            }
        }
        lanes[lane] = commit.parents.first().map(|p| (p.clone(), colour));
        if !commit.parents.is_empty() {
            edges.push(Edge::Out { to: lane, colour });
        }
        for parent in commit.parents.iter().skip(1) {
            let to = match lanes.iter().position(|l| l.as_ref().is_some_and(|(id, _)| id == parent)) {
                Some(to) => to,
                None => {
                    let c = new_colour();
                    let free = match lanes.iter().position(Option::is_none) {
                        Some(free) => free,
                        None => {
                            lanes.push(None);
                            lanes.len() - 1
                        }
                    };
                    lanes[free] = Some((parent.clone(), c));
                    free
                }
            };
            let c = lanes[to].as_ref().map_or(colour, |(_, c)| *c);
            edges.push(Edge::Out { to, colour: c });
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        rows.push(GraphRow { lane, colour, width: before.len().max(lanes.len()).max(lane + 1), edges });
    }
    rows
}

pub enum GitViewEvent {
    /// A commit clicked in the history: shown.
    ShowCommit(git::LogCommit),
    /// A pull request (by number) clicked: shown.
    ShowPull(u64),
    ShowIssue(u64),
    /// A file's row clicked: shown with its changes.
    Open(PathBuf),
    /// Commit asked for: the open files are saved first (then `commit` runs).
    CommitAsked,
    /// Committed (its short id), or why not.
    Committed(Result<String, String>),
    /// A message written by AI asked for, from these files' changes.
    Draft(Vec<PathBuf>),
    Push,
    Pull,
    /// A branch to switch to (open files saved first).
    SwitchTo(git::Branch),
    /// A branch to start here, and switch to.
    NewBranch(String),
    /// A branch to merge in, or rebase onto.
    Join {
        branch: String,
        rebase: bool,
    },
    /// Changes set aside, to bring back.
    BringBack(git::SetAside),
    /// The changes since the last commit, to set aside.
    SetAside,
    /// Something done (or not), to say.
    Said(String),
    /// The repository changed.
    Changed,
}

impl EventEmitter<GitViewEvent> for GitView {}

pub struct GitView {
    root: PathBuf,
    focus_handle: FocusHandle,
    /// None until read: then whether this is a repository.
    pub repository: Option<bool>,
    pub files: Vec<ChangedFile>,
    pub branch: Option<String>,
    /// Commits waiting to go up, and to come down.
    pub ahead_behind: Option<(usize, usize)>,
    pub summary: Entity<TextInput>,
    pub body: Entity<TextInput>,
    pub committing: bool,
    /// A message being written by AI: Commit waits for it.
    pub drafting: bool,
    /// Moved on when a draft should stop (committed, or another asked for).
    pub draft_generation: u64,
    reading: Option<Task<()>>,
    pub tab: Tab,
    /// Whether a remote is on GitHub (its tab shows only then).
    on_github: bool,
    pub history: Vec<git::LogCommit>,
    graph: Vec<GraphRow>,
    /// Where the branches, tags and HEAD were when the history was read.
    history_refs: String,
    /// The commits the filter lets through (their places in `history`), and whether it's set.
    history_shown: Vec<usize>,
    history_filtered: bool,
    pub filter: Entity<TextInput>,
    /// The commit shown, in the history.
    pub shown: Option<String>,
    history_task: Option<Task<()>>,
    pub branches: Branches,
    /// A new branch's name, as typed.
    pub branch_name: Entity<TextInput>,
    rename_input: Entity<TextInput>,
    /// The branch being renamed.
    pub renaming: Option<String>,
    /// The branch clicked, its actions shown under it.
    pub picked: Option<String>,
    branches_task: Option<Task<()>>,
    pub github: GitHub,
    github_task: Option<Task<()>>,
    pr_title: Entity<TextInput>,
    pr_body: Entity<TextInput>,
    pr_base: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for GitView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What a changed file's parts say, read off the main thread.
struct Read {
    repository: bool,
    on_github: bool,
    branch: Option<String>,
    ahead_behind: Option<(usize, usize)>,
    files: Vec<ChangedFile>,
}

/// The parts of `current` against `base`, each with its first line as its label.
fn parts_of(base: &str, current: &str) -> Vec<Part> {
    let now = git::lines(current);
    git::diff(base, current)
        .into_iter()
        .map(|hunk| {
            let first = now
                .get(hunk.lines.start..hunk.lines.end)
                .and_then(|lines| lines.iter().map(|l| l.trim()).find(|l| !l.is_empty()).map(str::to_string));
            let label = match (hunk.change, first) {
                (git::Change::Deleted, _) => "lines taken out".to_string(),
                (_, Some(first)) => first.chars().take(60).collect(),
                (_, None) => "blank lines".to_string(),
            };
            Part { lines: hunk.lines, label, kept: true }
        })
        .collect()
}

fn read(root: &Path) -> Read {
    let branch = git::current_branch(root);
    if branch.is_none() {
        return Read { repository: false, on_github: false, branch: None, ahead_behind: None, files: Vec::new() };
    }
    let files = git::status(root)
        .into_iter()
        .map(|(path, status)| {
            let encoding = crate::encoding::read(&path).map(|(_, e)| e).unwrap_or_default();
            let base = git::committed_text(&path, encoding).unwrap_or_default();
            let current = crate::encoding::read(&path).map(|(t, _)| t).unwrap_or_default();
            let (added, removed) = git::counts(&base, &current);
            // A new, deleted or conflicted file goes in whole.
            let parts = if status == FileStatus::Modified { parts_of(&base, &current) } else { Vec::new() };
            ChangedFile { path, status, added, removed, kept: true, parts, unfolded: false }
        })
        .collect();
    Read {
        repository: true,
        on_github: git::remote_on_github(root),
        branch,
        ahead_behind: git::ahead_behind(root),
        files,
    }
}

impl GitView {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let summary = cx.new(|cx| TextInput::new("Commit message", cx));
        let body = cx.new(|cx| TextInput::new("Body: what and why (optional)", cx));
        let filter = cx.new(|cx| TextInput::new("Filter: message, author, id", cx));
        let branch_name = cx.new(|cx| TextInput::new("New branch…", cx));
        let rename_input = cx.new(|cx| TextInput::new("New name", cx));
        let pr_title = cx.new(|cx| TextInput::new("Title", cx));
        let pr_body = cx.new(|cx| TextInput::new("What and why (optional)", cx));
        let pr_base = cx.new(|cx| TextInput::new("Into", cx));
        let subscriptions = vec![
            cx.subscribe(&pr_title, |_, _, _: &TextInputEvent, cx| cx.notify()),
            cx.subscribe(&branch_name, |_, _, _: &TextInputEvent, cx| cx.notify()),
            cx.subscribe(&rename_input, |_, _, _: &TextInputEvent, cx| cx.notify()),
            cx.subscribe(&summary, |_, _, _: &TextInputEvent, cx| cx.notify()),
            cx.subscribe(&body, |_, _, _: &TextInputEvent, cx| cx.notify()),
            cx.subscribe(&filter, |_, _, _: &TextInputEvent, cx| cx.notify()),
        ];
        Self {
            root,
            focus_handle: cx.focus_handle(),
            repository: None,
            files: Vec::new(),
            branch: None,
            ahead_behind: None,
            summary,
            body,
            committing: false,
            drafting: false,
            draft_generation: 0,
            reading: None,
            tab: Tab::Changes,
            on_github: false,
            history: Vec::new(),
            graph: Vec::new(),
            history_refs: String::new(),
            history_shown: Vec::new(),
            history_filtered: false,
            filter,
            shown: None,
            history_task: None,
            branches: Branches::default(),
            branch_name,
            rename_input,
            renaming: None,
            picked: None,
            branches_task: None,
            github: GitHub::default(),
            github_task: None,
            pr_title,
            pr_body,
            pr_base,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = root;
        self.files.clear();
        self.repository = None;
        self.history.clear();
        self.graph.clear();
        self.history_refs.clear();
        self.shown = None;
        self.branches = Branches::default();
        self.picked = None;
        self.renaming = None;
        // Another repository's GitHub: read again (what was asked of the last one dropped).
        let (list, closed) = (self.github.list, self.github.closed);
        self.github = GitHub { list, closed, ..GitHub::default() };
        self.github_task = None;
        self.refresh(cx);
    }

    /// Reads the tab shown again: the changes, or the history.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Changes => self.refresh_changes(cx),
            Tab::History => self.refresh_history(cx),
            Tab::Branches => self.refresh_branches(cx),
            // (GitHub's asked only when the tab's shown or refreshed: not on every change here.)
            Tab::GitHub => self.refresh_github_if_stale(cx),
        }
    }

    /// The repository's history read again (in the background), with its graph.
    pub fn refresh_history(&mut self, cx: &mut Context<Self>) {
        const MOST: usize = 500;
        let (root, was) = (self.root.clone(), self.history_refs.clone());
        self.history_task = Some(cx.spawn(async move |this, cx| {
            // Read again only when a branch, tag or HEAD has moved (this runs as files change).
            let read = cx
                .background_executor()
                .spawn(async move {
                    let refs = git::refs_signature(&root);
                    if refs == was {
                        return None;
                    }
                    let history = git::log(&root, MOST);
                    let graph = graph(&history);
                    Some((refs, history, graph))
                })
                .await;
            let Some((refs, history, graph)) = read else { return };
            this.update(cx, |this, cx| {
                this.history_refs = refs;
                this.graph = graph;
                this.history = history;
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        if tab == Tab::GitHub {
            self.refresh_github(cx);
        } else {
            self.refresh(cx);
        }
        cx.notify();
    }

    /// Reads what changed again (in the background); what was chosen stays chosen.
    pub fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.reading = Some(cx.spawn(async move |this, cx| {
            let read = cx.background_executor().spawn(async move { read(&root) }).await;
            this.update(cx, |this, cx| {
                let before = std::mem::take(&mut this.files);
                this.files = read.files;
                for file in &mut this.files {
                    if let Some(was) = before.iter().find(|f| f.path == file.path) {
                        file.kept = was.kept;
                        file.unfolded = was.unfolded;
                        // Its parts as they were chosen: the same lines, else the same change
                        // moved (its first line the same, and only one like it).
                        let mut taken = vec![false; was.parts.len()];
                        for part in &mut file.parts {
                            let same = was.parts.iter().position(|p| p.lines == part.lines);
                            let moved = || {
                                let like: Vec<usize> = (0..was.parts.len())
                                    .filter(|&i| !taken[i] && was.parts[i].label == part.label)
                                    .collect();
                                (like.len() == 1).then(|| like[0])
                            };
                            if let Some(i) = same.filter(|&i| !taken[i]).or_else(moved) {
                                taken[i] = true;
                                part.kept = was.parts[i].kept;
                            }
                        }
                        // In as its parts now say (none ticked: left out; a new change: in).
                        if !file.parts.is_empty() {
                            file.kept = file.parts.iter().any(|p| p.kept);
                        }
                    }
                }
                this.repository = Some(read.repository);
                this.on_github = read.on_github;
                this.branch = read.branch;
                this.ahead_behind = read.ahead_behind;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The message: its first line, a blank line and its body, when there's one.
    pub fn message(&self, cx: &App) -> String {
        let summary = self.summary.read(cx).text().trim().to_string();
        let body = self.body.read(cx).text().trim().to_string();
        if body.is_empty() { summary } else { format!("{summary}\n\n{body}") }
    }

    /// Puts `message` in the fields: its first line, and the rest as the body.
    pub fn set_message(&mut self, message: &str, cx: &mut Context<Self>) {
        let (summary, body) = message.split_once('\n').unwrap_or((message, ""));
        self.summary.update(cx, |input, cx| input.set_text(summary.trim(), cx));
        self.body.update(cx, |input, cx| input.set_text(&body.split_whitespace().collect::<Vec<_>>().join(" "), cx));
    }

    fn kept_count(&self) -> usize {
        self.files.iter().filter(|f| f.kept).count()
    }

    fn can_commit(&self, cx: &App) -> bool {
        !self.committing && !self.drafting && self.kept_count() > 0 && !self.summary.read(cx).text().trim().is_empty()
    }

    fn ask_commit(&mut self, _: &CommitChanges, _: &mut Window, cx: &mut Context<Self>) {
        if self.can_commit(cx) {
            cx.emit(GitViewEvent::CommitAsked);
        }
    }

    /// Commits what's chosen (the open files saved already): the files kept, and of those
    /// kept partly only their parts kept.
    pub fn commit(&mut self, cx: &mut Context<Self>) {
        if !self.can_commit(cx) {
            return;
        }
        if let Some(file) = self.files.iter().find(|f| f.status == FileStatus::Conflicted) {
            let name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            cx.emit(GitViewEvent::Committed(Err(format!("{name} still has conflicts: resolve them first."))));
            return;
        }
        self.committing = true;
        // A message being written by AI stops here.
        self.draft_generation += 1;
        cx.notify();
        let message = self.message(cx);
        let root = self.root.clone();
        let kept: Vec<PathBuf> = self.files.iter().filter(|f| f.kept).map(|f| f.path.clone()).collect();
        let left_out: Vec<PathBuf> = self.files.iter().filter(|f| !f.kept).map(|f| f.path.clone()).collect();
        // Each part chosen with its lines, as listed: the file saved since must still have
        // those changes, or the choices would land on others.
        type Chosen = Vec<(Range<usize>, bool)>;
        let partly: Vec<(PathBuf, Chosen)> = self
            .files
            .iter()
            .filter(|f| f.partly())
            .map(|f| (f.path.clone(), f.parts.iter().map(|p| (p.lines.clone(), p.kept)).collect()))
            .collect();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut texts = Vec::new();
                    for (path, chosen) in partly {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let (current, encoding) =
                            crate::encoding::read(&path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
                        let base = git::committed_text(&path, encoding).unwrap_or_default();
                        let hunks: Vec<Range<usize>> =
                            git::diff(&base, &current).into_iter().map(|h| h.lines).collect();
                        if hunks.len() != chosen.len() || hunks.iter().zip(&chosen).any(|(h, (lines, _))| h != lines) {
                            return Err(format!(
                                "{name} changed since its changes were listed: look at them again, then commit."
                            ));
                        }
                        let keep: Vec<bool> = chosen.iter().map(|(_, kept)| *kept).collect();
                        let text = git::partly(&base, &current, &keep);
                        // In the file's own encoding (its BOM too), as `git add` would see it.
                        let bytes = crate::encoding::encode(&text, encoding)
                            .map_err(|c| format!("{name}: {c} can't be written in its encoding."))?;
                        texts.push((path, bytes));
                    }
                    git::commit_parts(&root, &message, &kept, &left_out, &texts)
                })
                .await;
            this.update(cx, |this, cx| {
                this.committing = false;
                if result.is_ok() {
                    this.summary.update(cx, |input, cx| input.set_text("", cx));
                    this.body.update(cx, |input, cx| input.set_text("", cx));
                    // What was left out stays left out for the next one.
                }
                cx.emit(GitViewEvent::Committed(result));
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    #[cfg(test)]
    pub fn toggle_file_for_test(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.toggle_file(ix, cx);
    }

    fn toggle_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(file) = self.files.get_mut(ix) {
            file.kept = !file.kept;
            // All of it in, or all of it out.
            let kept = file.kept;
            file.parts.iter_mut().for_each(|p| p.kept = kept);
            cx.notify();
        }
    }

    pub(crate) fn toggle_part(&mut self, file: usize, part: usize, cx: &mut Context<Self>) {
        let Some(f) = self.files.get_mut(file) else { return };
        if let Some(p) = f.parts.get_mut(part) {
            p.kept = !p.kept;
        }
        // None of it in: the file's out; some: it's in, partly.
        f.kept = f.parts.iter().any(|p| p.kept);
        cx.notify();
    }

    fn render_history(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        if self.repository == Some(false) {
            return ui::empty_state("This folder isn't a git repository.", &theme).into_any_element();
        }
        let query = self.filter.read(cx).text().trim().to_lowercase();
        self.history_shown = (0..self.history.len())
            .filter(|&i| {
                let c = &self.history[i];
                query.is_empty()
                    || c.subject.to_lowercase().contains(&query)
                    || c.author.to_lowercase().contains(&query)
                    || c.id.starts_with(&query)
            })
            .collect();
        self.history_filtered = !query.is_empty();
        let nothing = self.history_shown.is_empty();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(ui::field(&theme).mx(px(12.)).mt(px(4.)).mb(px(6.)).child(self.filter.clone()))
            .when(nothing && !self.history.is_empty(), |d| d.child(ui::empty_state("No commit says that.", &theme)))
            .child(
                // Only the rows in sight are made (all 500, each with its graph, were made
                // every frame).
                gpui::uniform_list(
                    "git-history",
                    self.history_shown.len(),
                    cx.processor(|this, range: Range<usize>, _, cx| {
                        // (Each row across the whole list: its highlight, and a click anywhere on it.)
                        range
                            .filter_map(|ix| {
                                let i = *this.history_shown.get(ix)?;
                                Some(div().w_full().child(this.history_row(i, cx)).into_any_element())
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .min_h_0()
                .text_size(px(ui::T_SM)),
            )
            .into_any_element()
    }

    /// Commit `i` of the history's row: its part of the graph (unless filtered), its
    /// message, who, when, and the branches and tags at it.
    fn history_row(&self, i: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let filtered = self.history_filtered;
        let palette: Vec<gpui::Hsla> =
            vec![theme.caret, theme.ansi[12], theme.ansi[10], theme.ansi[13], theme.ansi[14], theme.ansi[11]];
        // One width for the graph in every row, so the messages line up.
        const LANE_PX: f32 = 12.;
        const MOST_LANES: usize = 8;
        let lanes = self.graph.iter().map(|r| r.width).max().unwrap_or(1).min(MOST_LANES);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        {
            let commit = self.history[i].clone();
            let row = self.graph.get(i).cloned();
            let selected = self.shown.as_deref() == Some(commit.id.as_str());
            let refs: Vec<String> = commit
                .refs
                .iter()
                .map(|r| r.trim_start_matches("HEAD -> ").trim_start_matches("tag: ").to_string())
                .filter(|r| r != "HEAD")
                .collect();
            let width = lanes as f32 * LANE_PX + 6.;
            let colours = palette.clone();
            let graph = (!filtered).then(|| {
                let row = row.clone();
                gpui::canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let Some(row) = &row else { return };
                        let x = |lane: usize| bounds.left() + gpui::px(6. + lane.min(MOST_LANES - 1) as f32 * LANE_PX);
                        let (top, mid, bottom) = (bounds.top(), bounds.center().y, bounds.bottom());
                        let colour = |c: usize| colours[c % colours.len()];
                        let mut line = |from: gpui::Point<gpui::Pixels>, to: gpui::Point<gpui::Pixels>, c: usize| {
                            let mut path = gpui::PathBuilder::stroke(gpui::px(1.5));
                            path.move_to(from);
                            if from.x == to.x {
                                path.line_to(to);
                            } else {
                                let half = (to.y - from.y) / 2.;
                                path.cubic_bezier_to(
                                    to,
                                    gpui::point(from.x, from.y + half),
                                    gpui::point(to.x, to.y - half),
                                );
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, colour(c));
                            }
                        };
                        for edge in &row.edges {
                            match *edge {
                                Edge::Through { lane, colour } => {
                                    line(gpui::point(x(lane), top), gpui::point(x(lane), bottom), colour)
                                }
                                Edge::In { from, colour } => {
                                    line(gpui::point(x(from), top), gpui::point(x(row.lane), mid), colour)
                                }
                                Edge::Out { to, colour } => {
                                    line(gpui::point(x(row.lane), mid), gpui::point(x(to), bottom), colour)
                                }
                            }
                        }
                        let dot = gpui::Bounds::centered_at(
                            gpui::point(x(row.lane), mid),
                            gpui::size(gpui::px(8.), gpui::px(8.)),
                        );
                        window.paint_quad(gpui::quad(
                            dot,
                            gpui::px(4.),
                            colour(row.colour),
                            gpui::px(0.),
                            gpui::transparent_black(),
                            gpui::BorderStyle::default(),
                        ));
                    },
                )
                .w(gpui::px(width))
                .h_full()
                .flex_none()
            });
            let id = commit.id.clone();
            div()
                .id(gpui::SharedString::from(format!("commit-{}", commit.short)))
                .h(px(40.))
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(10.))
                .pr(px(12.))
                .cursor_pointer()
                .when(selected, |r| r.bg(theme.hairline))
                .when(!selected, |r| r.hover(|s| s.bg(ui::hover(&theme))))
                .children(graph)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().child(commit.subject.clone()))
                        .child(
                            div()
                                .flex()
                                .gap(px(4.))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(ui::T_XS))
                                .text_color(theme.faint)
                                .child(format!("{} · {}", commit.author, git::ago(commit.time, now)))
                                .when(!refs.is_empty(), |d| {
                                    d.child("·").child(div().text_color(theme.muted).truncate().child(refs.join(", ")))
                                }),
                        ),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.shown = Some(id.clone());
                    if let Some(commit) = this.history.iter().find(|c| c.id == id).cloned() {
                        cx.emit(GitViewEvent::ShowCommit(commit));
                    }
                    cx.notify();
                }))
        }
        .into_any_element()
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        let tab = |id: &'static str, label: &'static str, which: Tab, cx: &mut Context<Self>| {
            let on = self.tab == which;
            div()
                .id(id)
                .pb(px(4.))
                .cursor_pointer()
                .text_color(if on { theme.foreground } else { theme.muted })
                .when(on, |t| t.border_b_1().border_color(theme.line_strong))
                .when(!on, |t| t.hover(|s| s.text_color(theme.foreground)))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.show_tab(which, cx)))
        };
        div()
            .flex()
            .gap(px(14.))
            .px(px(16.))
            .pt(px(10.))
            .pb(px(6.))
            .text_size(px(ui::T_SM))
            .child(tab("git-tab-changes", "Changes", Tab::Changes, cx))
            .child(tab("git-tab-history", "History", Tab::History, cx))
            .child(tab("git-tab-branches", "Branches", Tab::Branches, cx))
            .when(self.on_github || self.tab == Tab::GitHub, |d| {
                d.child(tab("git-tab-github", "GitHub", Tab::GitHub, cx))
            })
            .into_any_element()
    }

    fn render_changes(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        if self.repository == Some(false) {
            return ui::empty_state("This folder isn't a git repository.", &theme).into_any_element();
        }
        let ai = cx.global::<crate::settings::Settings>().ai.ready();
        let kept = self.kept_count();
        let can_commit = self.can_commit(cx);
        let branch_row = self.branch.clone().map(|branch| {
            let (ahead, behind) = self.ahead_behind.unwrap_or((0, 0));
            let counts = match (ahead, behind) {
                (0, 0) => String::new(),
                (a, 0) => format!("↑{a}"),
                (0, b) => format!("↓{b}"),
                (a, b) => format!("↑{a} ↓{b}"),
            };
            let button = |id: &'static str, label: &'static str, event: fn() -> GitViewEvent| {
                ui::button(id, label, &theme).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(event())))
            };
            ui::card(&theme)
                .mx(px(12.))
                .mt(px(4.))
                .px(px(10.))
                .min_h(px(40.))
                .py(px(7.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(svg().path("icons/branch.svg").size(px(13.)).text_color(theme.muted))
                .child(div().flex_1().min_w_0().truncate().code_font(cx).text_size(px(ui::T_SM)).child(branch))
                .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(counts))
                .when(behind > 0, |d| d.child(button("git-pull", "Pull", || GitViewEvent::Pull)))
                .when(ahead > 0 && behind == 0, |d| d.child(button("git-push", "Push", || GitViewEvent::Push)))
        });
        let commit_label = if self.committing {
            "Committing…".to_string()
        } else if kept == 1 {
            format!("Commit 1 file {}", ui::keys(&CommitChanges, "⌘↵", cx))
        } else {
            format!("Commit {kept} files {}", ui::keys(&CommitChanges, "⌘↵", cx))
        };
        let message_box = ui::card(&theme)
            .key_context("GitMessage")
            .on_action(cx.listener(Self::ask_commit))
            .mx(px(12.))
            .mt(px(10.))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().text_size(px(ui::T_MD)).child(self.summary.clone()))
            .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(self.body.clone()))
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(ai, |d| {
                        d.child(ui::link("git-draft", "✦ Draft", &theme).text_size(px(ui::T_SM)).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                let files = this.files.iter().filter(|f| f.kept).map(|f| f.path.clone()).collect();
                                cx.emit(GitViewEvent::Draft(files));
                            },
                        )))
                    })
                    .child(div().flex_1())
                    .child(ui::primary_button("git-commit", commit_label, can_commit, &theme).when(can_commit, |b| {
                        b.on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::CommitAsked)))
                    })),
            );
        let root = self.root.clone();
        let file_row = |ix: usize, file: &ChangedFile, cx: &mut Context<Self>| -> gpui::AnyElement {
            let name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = file
                .path
                .parent()
                .and_then(|p| p.strip_prefix(&root).ok())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let (name_color, note) = match file.status {
                FileStatus::Added => (theme.git_added, " · new"),
                FileStatus::Deleted => (theme.git_deleted, " · deleted"),
                FileStatus::Conflicted => (theme.error, " · to resolve"),
                FileStatus::Modified => (theme.foreground, ""),
            };
            let path = file.path.clone();
            let many = file.parts.len() > 1;
            let unfolded = file.unfolded;
            let partly = file.partly();
            let row =
                div()
                    .id(("git-file", ix))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .h(px(ui::ROW))
                    .rounded(px(ui::R_ROW))
                    .hover(|s| s.bg(ui::hover(&theme)))
                    .child(
                        ui::checkbox(file.kept, partly, &theme)
                            .cursor_pointer()
                            .id(("git-keep", ix))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_file(ix, cx))),
                    )
                    .child(
                        div()
                            .id(("git-name", ix))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .text_color(if file.kept { name_color } else { theme.muted })
                            .child(div().flex_none().child(name))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.faint)
                                    .text_size(px(ui::T_SM))
                                    .child(format!("{folder}{note}")),
                            )
                            .on_click(
                                cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::Open(path.clone()))),
                            ),
                    )
                    .when(many, |r| {
                        r.child(
                            div()
                                .id(("git-unfold", ix))
                                .cursor_pointer()
                                .child(
                                    svg()
                                        .path("icons/chevron-right.svg")
                                        .size(px(10.))
                                        .text_color(theme.faint)
                                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                            if unfolded { std::f32::consts::FRAC_PI_2 } else { 0. },
                                        ))),
                                )
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.files.get_mut(ix) {
                                        f.unfolded = !f.unfolded;
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(counts(file.added, file.removed, &theme, cx));
            let parts = (many && unfolded).then(|| {
                div().flex().flex_col().pl(px(34.)).children(file.parts.iter().enumerate().map(|(p, part)| {
                    let lines = if part.lines.is_empty() {
                        format!("At line {}", part.lines.start + 1)
                    } else if part.lines.len() == 1 {
                        format!("Line {}", part.lines.start + 1)
                    } else {
                        format!("Lines {}–{}", part.lines.start + 1, part.lines.end)
                    };
                    div()
                        .id(gpui::SharedString::from(format!("git-part-{ix}-{p}")))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(ui::ROW_SM))
                        .text_size(px(ui::T_SM))
                        .text_color(theme.muted)
                        .cursor_pointer()
                        .child(ui::checkbox(part.kept, false, &theme))
                        .child(div().flex_none().child(lines))
                        .child(
                            div().min_w_0().truncate().text_color(theme.faint).code_font(cx).child(part.label.clone()),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_part(ix, p, cx)))
                }))
            });
            div().flex().flex_col().child(row).children(parts).into_any_element()
        };
        let (mut kept_rows, mut out_rows) = (Vec::new(), Vec::new());
        for ix in 0..self.files.len() {
            let file = self.files[ix].clone();
            let row = file_row(ix, &file, cx);
            if file.kept { kept_rows.push(row) } else { out_rows.push(row) }
        }
        let nothing = self.files.is_empty() && self.repository == Some(true);
        div()
            .id("git-changes")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(12.))
            .children(branch_row)
            // (Nothing to commit: no message to write either.)
            .when(!nothing, |d| d.child(message_box))
            .when(nothing, |d| d.child(ui::empty_state("Nothing changed since the last commit.", &theme)))
            .when(!kept_rows.is_empty(), |d| d.child(ui::sidebar_heading("In this commit", &theme)))
            .child(div().px(px(6.)).flex().flex_col().children(kept_rows))
            .when(!out_rows.is_empty(), |d| d.child(ui::sidebar_heading("Left out", &theme)))
            .child(div().px(px(6.)).flex().flex_col().children(out_rows))
            .into_any_element()
    }
}

fn counts(added: usize, removed: usize, theme: &Theme, cx: &App) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .gap(px(4.))
        .code_font(cx)
        .text_size(px(ui::T_XS))
        .when(added > 0, |d| d.child(div().text_color(theme.git_added).child(format!("+{added}"))))
        .when(removed > 0, |d| d.child(div().text_color(theme.git_deleted).child(format!("−{removed}"))))
}

use crate::fonts::CodeFont as _;

impl Render for GitView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.render_tabs(cx);
        let shown = match self.tab {
            Tab::Changes => self.render_changes(cx),
            Tab::Branches => self.render_branches(cx),
            Tab::GitHub => self.render_github(cx),
            Tab::History => self.render_history(cx),
        };
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(ui::T_MD))
            .child(tabs)
            .child(div().flex_1().min_h_0().child(shown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, parents: &[&str]) -> git::LogCommit {
        git::LogCommit {
            id: id.into(),
            short: id.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            time: 0,
            subject: id.into(),
            refs: Vec::new(),
        }
    }

    /// A branch merged back: the merge forks into two lanes, which join again where the
    /// branch began.
    #[test]
    fn the_branches_graph() {
        // m merges b2 (a branch: b1, b2) into a2; both came from a1.
        let commits = [
            commit("m", &["a2", "b2"]),
            commit("b2", &["b1"]),
            commit("a2", &["a1"]),
            commit("b1", &["a1"]),
            commit("a1", &[]),
        ];
        let rows = graph(&commits);
        assert_eq!(rows.iter().map(|r| r.lane).collect::<Vec<_>>(), [0, 1, 0, 1, 0]);
        assert_eq!(rows[0].edges, [Edge::Out { to: 0, colour: 0 }, Edge::Out { to: 1, colour: 1 }]);
        assert!(rows[1].edges.contains(&Edge::Through { lane: 0, colour: 0 }));
        assert_eq!(rows[1].colour, 1, "the branch keeps its colour");
        // Both lanes wait for a1: they join there.
        assert_eq!(rows[4].edges, [Edge::In { from: 0, colour: 0 }, Edge::In { from: 1, colour: 1 }], "{:?}", rows[4]);
        assert_eq!(rows[4].width, 2);
    }
}
