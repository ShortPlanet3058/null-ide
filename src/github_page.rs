//! A pull request (or an issue), looked at in the place of the code: what it says, its
//! checks and files, each file's changes with the review's comments between the lines
//! (to answer), and the conversation (to join). A pull request can be merged, approved or
//! checked out here; anything else opens on GitHub.

use crate::editor::{Editor, EditorEvent, Thread};
use crate::github::{self, Checks, Comment, IssueDetail, MergeHow, PullDetail};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui::{self, px};
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, PromptLevel, Subscription,
    Task, Window, actions, div, prelude::*, svg,
};
use std::path::PathBuf;

actions!(github_page, [CloseGitHubPage, SendComment]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseGitHubPage, Some("GitHubPage")),
        KeyBinding::new("secondary-enter", SendComment, Some("GitHubComposer")),
    ]);
}

pub enum GitHubPageEvent {
    Close,
    Said(String),
    /// The repository changed here (a checkout, a merge pulled).
    Changed,
}

impl EventEmitter<GitHubPageEvent> for GitHubPage {}

pub enum Detail {
    Pull(PullDetail),
    Issue(IssueDetail),
}

/// What's shown beside the files: the conversation, or a file's changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Showing {
    Conversation,
    File(usize),
}

pub struct GitHubPage {
    root: PathBuf,
    pub number: u64,
    issue: bool,
    pub detail: Option<Detail>,
    error: Option<String>,
    pub showing: Showing,
    /// The file shown, as the pull request leaves it, its changes against where it began.
    pub shown: Option<Entity<Editor>>,
    /// Where the pull request's branch left its base (read once).
    merge_base: Option<String>,
    composer: Entity<TextInput>,
    /// The review thread being answered.
    pub replying: Option<u64>,
    sending: bool,
    /// Where the keyboard goes at the next drawing: the composer (a Reply… clicked: true),
    /// or the page (the reply sent, its composer gone: false).
    focus_next: Option<bool>,
    focus_handle: FocusHandle,
    loading: Option<Task<()>>,
    file_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for GitHubPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl GitHubPage {
    pub fn new(root: PathBuf, number: u64, issue: bool, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| TextInput::new("Comment…", cx));
        let subscriptions = vec![cx.subscribe(&composer, |_, _, _: &TextInputEvent, cx| cx.notify())];
        let mut page = Self {
            root,
            number,
            issue,
            detail: None,
            error: None,
            showing: Showing::Conversation,
            shown: None,
            merge_base: None,
            composer,
            replying: None,
            sending: false,
            focus_next: None,
            focus_handle: cx.focus_handle(),
            loading: None,
            file_task: None,
            _subscriptions: subscriptions,
        };
        page.load(cx);
        page
    }

    /// Reads it (again) from GitHub; the file shown is shown again, its threads current.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let (root, number, issue) = (self.root.clone(), self.number, self.issue);
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if issue {
                        github::issue_detail(&root, number).map(Detail::Issue)
                    } else {
                        github::pull_detail(&root, number).map(Detail::Pull)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(detail) => {
                        this.detail = Some(detail);
                        this.error = None;
                        match this.showing {
                            Showing::File(ix) if this.shown.is_some() => this.refresh_threads(ix, cx),
                            Showing::File(ix) => this.show_file(ix, cx),
                            Showing::Conversation => {}
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn pull(&self) -> Option<&PullDetail> {
        match &self.detail {
            Some(Detail::Pull(pull)) => Some(pull),
            _ => None,
        }
    }

    fn threads_of(&self, path: &str) -> Vec<Thread> {
        let Some(pull) = self.pull() else { return Vec::new() };
        let now = now();
        let said = |c: &Comment| (format!("{} · {}", c.author, crate::git::ago(c.time, now)), c.body.clone());
        pull.review
            .iter()
            .filter(|c| c.reply_to.is_none() && c.path.as_deref() == Some(path))
            .map(|top| {
                let mut comments = vec![said(top)];
                comments.extend(pull.review.iter().filter(|c| c.reply_to == Some(top.id)).map(said));
                Thread { id: top.id, line: top.line.unwrap_or(1).saturating_sub(1), comments }
            })
            .collect()
    }

    fn refresh_threads(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(file) = self.pull().and_then(|p| p.files.get(ix)).cloned() else { return };
        let threads = self.threads_of(&file.path);
        if let Some(editor) = &self.shown {
            editor.update(cx, |e, cx| e.set_threads(threads, cx));
        }
    }

    /// File `ix`'s changes: as the pull request leaves it, against where its branch began.
    pub fn show_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(pull) = self.pull() else { return };
        let Some(file) = pull.files.get(ix).cloned() else { return };
        let (base_oid, head_oid) = (pull.base_oid.clone(), pull.head_oid.clone());
        self.showing = Showing::File(ix);
        self.replying = None;
        let (root, known_base) = (self.root.clone(), self.merge_base.clone());
        self.file_task = Some(cx.spawn(async move |this, cx| {
            let read = file.clone();
            let (base, before, after) = cx
                .background_executor()
                .spawn(async move {
                    let file = read;
                    let base = known_base.or_else(|| github::merge_base(&root, &base_oid, &head_oid)).unwrap_or(base_oid);
                    let before = if file.kind == 'A' { None } else { github::text_at(&root, &base, &file.path) };
                    let after = if file.kind == 'D' { None } else { github::text_at(&root, &head_oid, &file.path) };
                    (base, before.unwrap_or_default(), after.unwrap_or_default())
                })
                .await;
            this.update(cx, |this, cx| {
                this.merge_base = Some(base);
                let threads = this.threads_of(&file.path);
                let editor = cx.new(|cx| {
                    // No file behind it: nothing typed here is saved anywhere.
                    let mut editor = Editor::new(crate::buffer::Buffer::from_text(&after), None, cx);
                    editor.choose_language(crate::editor::language_by_path(std::path::Path::new(&file.path)), cx);
                    editor.read_only = true;
                    editor.start_review(before, cx);
                    editor.set_threads(threads, cx);
                    editor
                });
                this._subscriptions.push(cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
                    if let EditorEvent::ReplyAsked(id) = event {
                        this.reply_to(*id, cx);
                    }
                }));
                this.shown = Some(editor);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn show_conversation(&mut self, cx: &mut Context<Self>) {
        self.showing = Showing::Conversation;
        self.replying = None;
        self.composer.update(cx, |input, _| input.set_placeholder("Comment…"));
        cx.notify();
    }

    fn reply_to(&mut self, id: u64, cx: &mut Context<Self>) {
        let who = self
            .pull()
            .and_then(|p| p.review.iter().find(|c| c.id == id))
            .map(|c| c.author.clone())
            .unwrap_or_default();
        self.replying = Some(id);
        self.focus_next = Some(true);
        self.composer.update(cx, |input, _| input.set_placeholder(format!("Reply to {who}…")));
        cx.notify();
    }

    /// ⌘↵ (or the button): the comment (or reply) sent, then everything read again.
    fn send(&mut self, _: &SendComment, _: &mut Window, cx: &mut Context<Self>) {
        let body = self.composer.read(cx).text().trim().to_string();
        if body.is_empty() || self.sending {
            return;
        }
        let (root, number, issue, replying) = (self.root.clone(), self.number, self.issue, self.replying);
        self.sending = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match replying {
                        Some(id) => github::reply(&root, number, id, &body),
                        None => github::comment(&root, number, &body, issue),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.sending = false;
                match result {
                    Ok(()) => {
                        this.composer.update(cx, |input, cx| input.set_text("", cx));
                        if this.replying.take().is_some() {
                            this.composer.update(cx, |input, _| input.set_placeholder("Comment…"));
                            this.focus_next = Some(false);
                        }
                        this.load(cx);
                    }
                    Err(error) => cx.emit(GitHubPageEvent::Said(format!("Couldn't send it: {error}"))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Esc: a reply being written put away first, then the page closed.
    fn close(&mut self, _: &CloseGitHubPage, window: &mut Window, cx: &mut Context<Self>) {
        if self.replying.take().is_some() {
            self.composer.update(cx, |input, _| input.set_placeholder("Comment…"));
            window.focus(&self.focus_handle);
            cx.notify();
            return;
        }
        cx.emit(GitHubPageEvent::Close);
    }

    /// Runs a gh action on this pull request (off the main thread), then says how it went.
    fn act(&mut self, done: String, action: impl FnOnce(&std::path::Path, u64) -> Result<(), String> + Send + 'static, cx: &mut Context<Self>) {
        let (root, number) = (self.root.clone(), self.number);
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { action(&root, number) }).await;
            this.update(cx, |this, cx| {
                let said = match result {
                    Ok(()) => done,
                    Err(error) => error,
                };
                cx.emit(GitHubPageEvent::Said(said));
                cx.emit(GitHubPageEvent::Changed);
                this.load(cx);
            })
            .ok();
        })
        .detach();
    }

    fn merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pull) = self.pull() else { return };
        let (number, head, base) = (pull.pull.number, pull.pull.head.clone(), pull.pull.base.clone());
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Merge #{number} ({head} into {base})?"),
            Some("On GitHub: as a merge commit, its commits squashed into one, or its commits rebased onto the base."),
            &["Merge Commit", "Squash", "Rebase", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let how = match answer.await {
                Ok(0) => MergeHow::Merge,
                Ok(1) => MergeHow::Squash,
                Ok(2) => MergeHow::Rebase,
                _ => return,
            };
            this.update(cx, |this, cx| {
                this.act(format!("Merged #{number}."), move |root, n| github::merge(root, n, how), cx)
            })
            .ok();
        })
        .detach();
    }

}

#[cfg(all(test, unix))]
impl GitHubPage {
    pub fn composer_for_test(&self) -> Entity<TextInput> {
        self.composer.clone()
    }

    pub fn send_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send(&SendComment, window, cx);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Markdown, as plain paragraphs (code in the code font).
fn markdown(text: &str, theme: &Theme, cx: &App) -> Vec<gpui::AnyElement> {
    crate::markdown::blocks(text, None)
        .into_iter()
        .map(|b| {
            if b.code {
                div()
                    .my(px(4.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(ui::R_KEY))
                    .bg(theme.surface)
                    .code_font(cx)
                    .text_size(px(ui::T_SM))
                    .children(b.text.lines().map(|l| div().whitespace_nowrap().child(l.to_string())).collect::<Vec<_>>())
                    .into_any_element()
            } else {
                div()
                    .py(px(2.))
                    .children(b.text.lines().map(|l| div().min_h(px(8.)).child(l.to_string())).collect::<Vec<_>>())
                    .into_any_element()
            }
        })
        .collect()
}

impl Render for GitHubPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        match self.focus_next.take() {
            Some(true) => window.focus(&self.composer.focus_handle(cx)),
            Some(false) => window.focus(&self.focus_handle),
            None => {}
        }
        let page = div()
            .key_context("GitHubPage")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_size(px(ui::T_MD));
        let close = div()
            .id("close-github")
            .flex_none()
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(ui::R_KEY))
            .cursor_pointer()
            .hover(|s| s.bg(theme.hairline))
            .child(svg().path("icons/x.svg").size(px(12.)).text_color(theme.muted))
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(GitHubPageEvent::Close)));
        let Some(detail) = &self.detail else {
            let said = self.error.clone().unwrap_or_else(|| format!("Reading #{} from GitHub…", self.number));
            return page.child(
                div()
                    .flex()
                    .items_start()
                    .px(px(28.))
                    .pt(px(18.))
                    .child(div().flex_1().text_color(theme.muted).child(said))
                    .child(close),
            );
        };
        let (title, state, url) = match detail {
            Detail::Pull(p) => (p.pull.title.clone(), p.pull.state.clone(), p.pull.url.clone()),
            Detail::Issue(i) => (i.issue.title.clone(), i.issue.state.clone(), i.issue.url.clone()),
        };
        let mut meta: Vec<gpui::AnyElement> = Vec::new();
        let mut files: Vec<gpui::AnyElement> = Vec::new();
        let (body, conversation) = match detail {
            Detail::Pull(p) => {
                meta.push(div().code_font(cx).child(format!("{} → {}", p.pull.head, p.pull.base)).into_any_element());
                let (checks, colour) = match p.pull.checks {
                    Checks::Passed(n) => (format!("✓ {n} checks passed"), theme.git_added),
                    Checks::Running => ("checks running".to_string(), theme.caret),
                    Checks::Failed(n) => (format!("✕ {n} checks failed"), theme.git_deleted),
                    Checks::None => (String::new(), theme.muted),
                };
                if !checks.is_empty() {
                    meta.push(div().text_color(colour).child(checks).into_any_element());
                }
                let (added, removed) = p.files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
                let n = p.files.len();
                meta.push(
                    div()
                        .child(format!("{} · +{added} −{removed}", if n == 1 { "1 file".into() } else { format!("{n} files") }))
                        .into_any_element(),
                );
                let threads: Vec<usize> = p
                    .files
                    .iter()
                    .map(|f| p.review.iter().filter(|c| c.reply_to.is_none() && c.path.as_deref() == Some(f.path.as_str())).count())
                    .collect();
                for (ix, file) in p.files.iter().enumerate() {
                    let name = std::path::Path::new(&file.path)
                        .file_name()
                        .map_or_else(|| file.path.clone(), |n| n.to_string_lossy().into_owned());
                    let colour = match file.kind {
                        'A' => theme.git_added,
                        'D' => theme.git_deleted,
                        _ => theme.foreground,
                    };
                    let on = self.showing == Showing::File(ix);
                    files.push(
                        div()
                            .id(("pull-file", ix))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(8.))
                            .h(px(ui::ROW))
                            .rounded(px(ui::R_ROW))
                            .cursor_pointer()
                            .when(on, |r| r.bg(theme.hairline))
                            .when(!on, |r| r.hover(|s| s.bg(theme.hairline.opacity(0.6))))
                            .child(div().flex_1().min_w_0().truncate().text_color(colour).child(name))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.))
                                    .text_size(px(ui::T_XS))
                                    .when(threads[ix] > 0, |d| d.child(div().text_color(theme.muted).child(format!("💬 {}", threads[ix]))))
                                    .when(threads[ix] == 0, |d| {
                                        d.code_font(cx)
                                            .when(file.added > 0, |d| d.child(div().text_color(theme.git_added).child(format!("+{}", file.added))))
                                            .when(file.removed > 0, |d| {
                                                d.child(div().text_color(theme.git_deleted).child(format!("−{}", file.removed)))
                                            })
                                    }),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.show_file(ix, cx)))
                            .into_any_element(),
                    );
                }
                (p.body.clone(), p.conversation.clone())
            }
            Detail::Issue(i) => {
                meta.push(div().child(i.issue.author.clone()).into_any_element());
                if !i.issue.labels.is_empty() {
                    meta.push(div().child(i.issue.labels.join(", ")).into_any_element());
                }
                (i.body.clone(), i.conversation.clone())
            }
        };
        let state_word = match state.as_str() {
            "MERGED" => Some(("Merged", theme.ansi[13])),
            "CLOSED" => Some(("Closed", theme.faint)),
            _ => None,
        };
        let open = state == "OPEN";
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px(px(10.))
                .py(px(4.))
                .rounded(px(ui::R_KEY))
                .text_size(px(ui::T_SM))
                .cursor_pointer()
                .when(primary, |b| b.bg(theme.caret).text_color(theme.background).font_weight(gpui::FontWeight::SEMIBOLD))
                .when(!primary, |b| b.border_1().border_color(theme.hairline).hover(|s| s.bg(theme.hairline)))
                .active(|s| s.opacity(0.7))
                .child(label)
        };
        let is_pull = !self.issue;
        let header = div()
            .px(px(28.))
            .pt(px(18.))
            .pb(px(14.))
            .border_b_1()
            .border_color(theme.hairline)
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_wrap()
                            .items_baseline()
                            .gap_x(px(8.))
                            .text_size(px(ui::T_XL))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(title)
                            .child(div().text_color(theme.faint).font_weight(gpui::FontWeight::NORMAL).child(format!("#{}", self.number))),
                    )
                    .child(close),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(12.))
                    .text_size(px(ui::T_SM))
                    .text_color(theme.muted)
                    .when_some(state_word, |d, (word, colour)| d.child(div().text_color(colour).child(word)))
                    .children(meta),
            )
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(is_pull && open, |d| {
                        d.child(button("pull-merge", "Merge", true).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.merge(window, cx))))
                            .child(button("pull-approve", "Approve", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                let n = this.number;
                                this.act(format!("Approved #{n}."), github::approve, cx)
                            })))
                    })
                    .when(is_pull, |d| {
                        d.child(button("pull-checkout", "Check Out", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            let n = this.number;
                            this.act(format!("Checked out #{n}'s branch."), github::check_out, cx)
                        })))
                    })
                    .child(
                        div()
                            .id("github-open")
                            .px(px(6.))
                            .py(px(4.))
                            .text_size(px(ui::T_SM))
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.foreground))
                            .child("Open on GitHub")
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&url))),
                    ),
            );

        // The conversation: what it says, then its comments, then one to add.
        let composer = |this: &Self, cx: &mut Context<Self>| {
            let ready = !this.composer.read(cx).text().trim().is_empty() && !this.sending;
            div()
                .key_context("GitHubComposer")
                .on_action(cx.listener(Self::send))
                .w_full()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(ui::R_ROW_LG))
                .border_1()
                .border_color(if this.replying.is_some() { theme.caret.opacity(0.5) } else { theme.hairline })
                .text_size(px(ui::T_SM))
                .child(div().flex_1().min_w_0().child(this.composer.clone()))
                .child(
                    div()
                        .id("github-send-comment")
                        .flex_none()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(ui::R_KEY))
                        .when(ready, |b| b.bg(theme.caret).text_color(theme.background).cursor_pointer())
                        .when(!ready, |b| b.text_color(theme.faint))
                        .child(if this.sending { "Sending…" } else { "Send ⌘↵" })
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.send(&SendComment, window, cx))),
                )
        };
        let card = |who: String, text: &str, cx: &App| {
            div()
                .p(px(12.))
                .rounded(px(ui::R_CONTROL))
                .border_1()
                .border_color(theme.hairline)
                .bg(theme.surface)
                .child(div().pb(px(4.)).text_size(px(ui::T_XS)).text_color(theme.muted).child(who))
                .children(markdown(text, &theme, cx))
        };
        let now = now();
        let right: gpui::AnyElement = match self.showing {
            Showing::Conversation => {
                let body = if body.trim().is_empty() { "No description.".to_string() } else { body };
                div()
                    .id("github-conversation")
                    .size_full()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .py(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().max_w(px(720.)).text_color(theme.foreground).children(markdown(&body, &theme, cx)))
                    .children(
                        conversation
                            .iter()
                            .map(|c| card(format!("{} · {}", c.author, crate::git::ago(c.time, now)), &c.body, cx).max_w(px(720.))),
                    )
                    .child(div().w_full().max_w(px(720.)).child(composer(self, cx)))
                    .into_any_element()
            }
            Showing::File(_) => div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().children(self.shown.clone()))
                .when(self.replying.is_some(), |d| {
                    d.child(div().p(px(10.)).border_t_1().border_color(theme.hairline).child(composer(self, cx)))
                })
                .into_any_element(),
        };
        let conversation_row = div()
            .id("pull-conversation")
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .h(px(ui::ROW))
            .rounded(px(ui::R_ROW))
            .cursor_pointer()
            .when(self.showing == Showing::Conversation, |r| r.bg(theme.hairline))
            .when(self.showing != Showing::Conversation, |r| r.hover(|s| s.bg(theme.hairline.opacity(0.6))))
            .child(div().flex_1().child("Conversation"))
            .child(div().text_size(px(ui::T_XS)).text_color(theme.muted).child(match conversation.len() {
                0 => String::new(),
                n => format!("💬 {n}"),
            }))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_conversation(cx)));
        page.child(header).child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(
                    div()
                        .id("pull-files")
                        .w(px(230.))
                        .flex_none()
                        .p(px(8.))
                        .border_r_1()
                        .border_color(theme.hairline)
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(conversation_row)
                        .when(!files.is_empty(), |d| d.child(div().h(px(6.))))
                        .children(files),
                )
                .child(div().flex_1().min_w_0().child(right)),
        )
    }
}

use crate::fonts::CodeFont as _;
