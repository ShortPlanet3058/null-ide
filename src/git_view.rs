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

actions!(git_view, [CommitChanges]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-enter", CommitChanges, Some("GitMessage"))]);
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

pub enum GitViewEvent {
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
    committing: bool,
    reading: Option<Task<()>>,
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
        return Read { repository: false, branch: None, ahead_behind: None, files: Vec::new() };
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
    Read { repository: true, branch, ahead_behind: git::ahead_behind(root), files }
}

impl GitView {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let summary = cx.new(|cx| TextInput::new("Commit message", cx));
        let body = cx.new(|cx| TextInput::new("Body: what and why (optional)", cx));
        let subscriptions = vec![
            cx.subscribe(&summary, |_, _, _: &TextInputEvent, cx| cx.notify()),
            cx.subscribe(&body, |_, _, _: &TextInputEvent, cx| cx.notify()),
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
            reading: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = root;
        self.files.clear();
        self.repository = None;
        self.refresh(cx);
    }

    /// Reads what changed again (in the background); what was chosen stays chosen.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
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
                        // Its parts as they were chosen, where they're the same lines.
                        for part in &mut file.parts {
                            if let Some(old) = was.parts.iter().find(|p| p.lines == part.lines) {
                                part.kept = old.kept;
                            }
                        }
                    }
                }
                this.repository = Some(read.repository);
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
        !self.committing && self.kept_count() > 0 && !self.summary.read(cx).text().trim().is_empty()
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
        self.committing = true;
        cx.notify();
        let message = self.message(cx);
        let root = self.root.clone();
        let left_out: Vec<PathBuf> = self.files.iter().filter(|f| !f.kept).map(|f| f.path.clone()).collect();
        let partly: Vec<(PathBuf, Vec<bool>)> = self
            .files
            .iter()
            .filter(|f| f.partly())
            .map(|f| (f.path.clone(), f.parts.iter().map(|p| p.kept).collect()))
            .collect();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let partly: Vec<(PathBuf, String)> = partly
                        .into_iter()
                        .map(|(path, keep)| {
                            let encoding = crate::encoding::read(&path).map(|(_, e)| e).unwrap_or_default();
                            let base = git::committed_text(&path, encoding).unwrap_or_default();
                            let current = crate::encoding::read(&path).map(|(t, _)| t).unwrap_or_default();
                            (path, git::partly(&base, &current, &keep))
                        })
                        .collect();
                    git::commit_parts(&root, &message, &left_out, &partly)
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

    fn toggle_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(file) = self.files.get_mut(ix) {
            file.kept = !file.kept;
            // Back in: all of it.
            if file.kept {
                file.parts.iter_mut().for_each(|p| p.kept = true);
            }
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

    fn render_changes(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        if self.repository == Some(false) {
            return quiet("This folder isn't a git repository.", &theme);
        }
        let ai = cx.global::<crate::settings::Settings>().ai.enabled;
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
                div()
                    .id(id)
                    .flex_none()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(ui::R_KEY))
                    .border_1()
                    .border_color(theme.hairline)
                    .text_size(px(ui::T_SM))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.hairline))
                    .active(|s| s.opacity(0.7))
                    .child(label)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(event())))
            };
            div()
                .mx(px(12.))
                .mt(px(4.))
                .px(px(10.))
                .py(px(7.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(ui::R_ROW_LG))
                .bg(theme.raised)
                .child(svg().path("icons/branch.svg").size(px(13.)).text_color(theme.muted))
                .child(div().flex_1().min_w_0().truncate().code_font(cx).text_size(px(ui::T_SM)).child(branch))
                .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(counts))
                .when(behind > 0, |d| d.child(button("git-pull", "Pull", || GitViewEvent::Pull)))
                .when(ahead > 0 && behind == 0, |d| d.child(button("git-push", "Push", || GitViewEvent::Push)))
        });
        let commit_label = if self.committing {
            "Committing…".to_string()
        } else if kept == 1 {
            "Commit 1 file ⌘↵".to_string()
        } else {
            format!("Commit {kept} files ⌘↵")
        };
        let message_box = div()
            .key_context("GitMessage")
            .on_action(cx.listener(Self::ask_commit))
            .mx(px(12.))
            .mt(px(10.))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .rounded(px(ui::R_ROW_LG))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.background)
            .child(div().text_size(px(ui::T_MD)).child(self.summary.clone()))
            .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(self.body.clone()))
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(ai, |d| {
                        d.child(
                            div()
                                .id("git-draft")
                                .text_size(px(ui::T_SM))
                                .text_color(theme.muted)
                                .cursor_pointer()
                                .hover(|s| s.text_color(theme.foreground))
                                .child("✦ Draft")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    let files = this.files.iter().filter(|f| f.kept).map(|f| f.path.clone()).collect();
                                    cx.emit(GitViewEvent::Draft(files));
                                })),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("git-commit")
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(px(ui::R_KEY))
                            .text_size(px(ui::T_SM))
                            .when(can_commit, |b| {
                                b.bg(theme.caret)
                                    .text_color(theme.on_accent)
                                    .cursor_pointer()
                                    .active(|s| s.opacity(0.8))
                                    .on_click(
                                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::CommitAsked)),
                                    )
                            })
                            .when(!can_commit, |b| b.bg(theme.hairline).text_color(theme.faint))
                            .child(commit_label),
                    ),
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
                FileStatus::Conflicted => (theme.error, " · to settle"),
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
                    .hover(|s| s.bg(theme.hairline.opacity(0.6)))
                    .child(
                        checkbox(file.kept, partly, &theme)
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
                        .id(("git-part", ix * 1000 + p))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(ui::ROW_SM))
                        .text_size(px(ui::T_SM))
                        .text_color(theme.muted)
                        .cursor_pointer()
                        .child(checkbox(part.kept, false, &theme))
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
        let heading = |text: &'static str| {
            div().px(px(16.)).pt(px(14.)).pb(px(4.)).text_size(px(ui::T_XS)).text_color(theme.faint).child(text)
        };
        let nothing = self.files.is_empty() && self.repository == Some(true);
        div()
            .id("git-changes")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(12.))
            .children(branch_row)
            .child(message_box)
            .when(nothing, |d| d.child(quiet("Nothing changed since the last commit.", &theme)))
            .when(!kept_rows.is_empty(), |d| d.child(heading("IN THIS COMMIT")))
            .child(div().px(px(6.)).flex().flex_col().children(kept_rows))
            .when(!out_rows.is_empty(), |d| d.child(heading("LEFT OUT")))
            .child(div().px(px(6.)).flex().flex_col().children(out_rows))
            .into_any_element()
    }
}

/// A box ticked, half-ticked (`partly`: some of it), or empty.
fn checkbox(on: bool, partly: bool, theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .text_size(px(10.))
        .when(on, |b| b.bg(theme.caret).text_color(theme.on_accent).child(if partly { "–" } else { "✓" }))
        .when(!on, |b| b.border_1().border_color(theme.line_strong))
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

fn quiet(text: &str, theme: &Theme) -> gpui::AnyElement {
    div()
        .px(px(18.))
        .pt(px(14.))
        .text_size(px(ui::T_MD))
        .text_color(theme.faint)
        .child(text.to_string())
        .into_any_element()
}

use crate::fonts::CodeFont as _;

impl Render for GitView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle).size_full().text_size(px(ui::T_MD)).child(self.render_changes(cx))
    }
}
