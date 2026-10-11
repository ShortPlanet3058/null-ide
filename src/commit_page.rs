//! A commit, looked at: its message, who and when, the files it changed, and each file's
//! changes (in the place of the code, until a file is opened): to check it out, take it
//! back (revert), make it again here (cherry-pick), or copy its id.

use crate::editor::Editor;
use crate::git::{self, CommitFile, LogCommit};
use crate::theme::Theme;
use crate::ui::{self, px};
use gpui::{
    App, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, PromptLevel,
    Task, Window, actions, div, prelude::*, svg,
};
use std::path::PathBuf;

actions!(commit_page, [CloseCommit]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseCommit, Some("CommitPage"))]);
}

pub enum CommitPageEvent {
    Close,
    /// Something done (or not), to say.
    Said(String),
    /// About to change the repository: the open files saved first, so git sees them.
    WillChange,
    /// The repository changed (a revert, a cherry-pick, a checkout).
    Changed,
    /// It changed, with conflicts in these files.
    Conflicts(Vec<PathBuf>),
}

impl EventEmitter<CommitPageEvent> for CommitPage {}

pub struct CommitPage {
    root: PathBuf,
    pub commit: LogCommit,
    message: String,
    pub files: Vec<CommitFile>,
    pub selected: Option<usize>,
    /// The file selected, as the commit left it, its changes shown against the one before.
    pub shown: Option<Entity<Editor>>,
    focus_handle: FocusHandle,
    loading: Option<Task<()>>,
}

impl Focusable for CommitPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CommitPage {
    pub fn new(root: PathBuf, commit: LogCommit, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            root,
            commit,
            message: String::new(),
            files: Vec::new(),
            selected: None,
            shown: None,
            focus_handle: cx.focus_handle(),
            loading: None,
        };
        page.load(cx);
        page
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let (root, id, parent) = (self.root.clone(), self.commit.id.clone(), self.commit.parents.first().cloned());
        self.loading = Some(cx.spawn(async move |this, cx| {
            let (message, files) = cx
                .background_executor()
                .spawn(async move { (git::message_of(&root, &id), git::commit_files(&root, &id, parent.as_deref())) })
                .await;
            this.update(cx, |this, cx| {
                this.message = message;
                this.files = files;
                if !this.files.is_empty() {
                    this.select(0, cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Shows file `ix`'s changes: the file as the commit left it, against the one before.
    pub fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(file) = self.files.get(ix).cloned() else { return };
        self.selected = Some(ix);
        let (root, id, parent) = (self.root.clone(), self.commit.id.clone(), self.commit.parents.first().cloned());
        self.loading = Some(cx.spawn(async move |this, cx| {
            let path = file.path.clone();
            // (A renamed file was elsewhere before.)
            let was = file.old_path.clone().unwrap_or_else(|| path.clone());
            let (after, before) = cx
                .background_executor()
                .spawn(async move {
                    let after = git::text_at(&root, &id, &path).unwrap_or_default();
                    let before = parent.and_then(|p| git::text_at(&root, &p, &was)).unwrap_or_default();
                    (after, before)
                })
                .await;
            this.update(cx, |this, cx| {
                // No file behind it: nothing typed here is ever saved anywhere.
                let editor = cx.new(|cx| {
                    let mut editor = Editor::new(crate::buffer::Buffer::from_text(&after), None, cx);
                    let kind = crate::editor::language_by_path(std::path::Path::new(&file.path));
                    editor.choose_language(kind, cx);
                    editor.view_only = true;
                    editor.start_review(before, cx);
                    editor
                });
                this.shown = Some(editor);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn close(&mut self, _: &CloseCommit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommitPageEvent::Close);
    }

    /// Runs a git action on this commit (off the main thread), then says how it went.
    fn act(
        &mut self,
        done: &'static str,
        action: fn(&std::path::Path, &str) -> Result<Vec<PathBuf>, String>,
        cx: &mut Context<Self>,
    ) {
        cx.emit(CommitPageEvent::WillChange);
        let (root, id, short) = (self.root.clone(), self.commit.id.clone(), self.commit.short.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { action(&root, &id) }).await;
            this.update(cx, |_, cx| match result {
                Ok(conflicts) if conflicts.is_empty() => {
                    cx.emit(CommitPageEvent::Said(format!("{done} {short}.")));
                    cx.emit(CommitPageEvent::Changed);
                }
                Ok(conflicts) => {
                    let n = conflicts.len();
                    let files = if n == 1 { "1 file".to_string() } else { format!("{n} files") };
                    cx.emit(CommitPageEvent::Said(format!("Conflicts in {files}: ⌘. resolves each.")));
                    cx.emit(CommitPageEvent::Changed);
                    cx.emit(CommitPageEvent::Conflicts(conflicts));
                }
                Err(error) => cx.emit(CommitPageEvent::Said(error)),
            })
            .ok();
        })
        .detach();
    }

    /// Asks first (its title, what it does, its button), then runs `action` on this
    /// commit, saying `done`.
    fn ask_then(
        &mut self,
        (title, detail, button): (String, &'static str, &'static str),
        done: &'static str,
        action: fn(&std::path::Path, &str) -> Result<Vec<PathBuf>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(PromptLevel::Info, &title, Some(detail), &[button, "Cancel"], cx);
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |this, cx| this.act(done, action, cx)).ok();
            }
        })
        .detach();
    }

    fn check_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let short = self.commit.short.clone();
        self.ask_then(
            (format!("Check out {short}?"), "The files go back to how they were then, on no branch (to look, or to start a branch from there). Switch to a branch to come back.", "Check Out"),
            "Checked out",
            |root, id| git::check_out(root, id).map(|()| Vec::new()),
            window,
            cx,
        );
    }

    fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let short = self.commit.short.clone();
        self.ask_then(
            (
                format!("Revert {short}?"),
                "A new commit takes its changes back (Undo Last Commit takes that back in turn).",
                "Revert",
            ),
            "Took back",
            git::revert_commit,
            window,
            cx,
        );
    }

    fn cherry_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let short = self.commit.short.clone();
        self.ask_then(
            (
                format!("Cherry-pick {short} onto this branch?"),
                "Its changes are made again here, as a new commit.",
                "Cherry-Pick",
            ),
            "Made again here:",
            git::cherry_pick,
            window,
            cx,
        );
    }
}

impl Render for CommitPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let commit = &self.commit;
        let when = chrono::DateTime::from_timestamp(commit.time, 0)
            .map(|t| t.with_timezone(&chrono::Local).format("%b %-d, %Y %H:%M").to_string())
            .unwrap_or_default();
        let (added, removed) = self.files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        let files = match self.files.len() {
            1 => "1 file".to_string(),
            n => format!("{n} files"),
        };
        let body: String = self.message.split_once('\n').map(|(_, b)| b.trim().to_string()).unwrap_or_default();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(10.))
                .py(px(4.))
                .rounded(px(ui::R_KEY))
                .border_1()
                .border_color(theme.hairline)
                .text_size(px(ui::T_SM))
                .cursor_pointer()
                .hover(|s| s.bg(theme.hairline))
                .active(|s| s.opacity(0.7))
                .child(label)
        };
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
                            .text_size(px(ui::T_XL))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(commit.subject.clone()),
                    )
                    .child(
                        div()
                            .id("close-commit")
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(ui::R_KEY))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.hairline))
                            .child(svg().path("icons/x.svg").size(px(12.)).text_color(theme.muted))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(CommitPageEvent::Close))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(14.))
                    .text_size(px(ui::T_SM))
                    .text_color(theme.muted)
                    .child(div().code_font(cx).child(commit.short.clone()))
                    .child(commit.author.clone())
                    .child(when)
                    .child(format!("{files} · +{added} −{removed}")),
            )
            .when(!body.is_empty(), |d| {
                d.child(div().max_w(px(680.)).text_size(px(ui::T_MD)).text_color(theme.muted).child(body))
            })
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        button("commit-checkout", "Check Out")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.check_out(window, cx))),
                    )
                    .child(
                        button("commit-revert", "Revert")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.revert(window, cx))),
                    )
                    .child(
                        button("commit-pick", "Cherry-Pick")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.cherry_pick(window, cx))),
                    )
                    .child(
                        div()
                            .id("commit-copy")
                            .px(px(6.))
                            .py(px(4.))
                            .text_size(px(ui::T_SM))
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.foreground))
                            .child("Copy Id")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                crate::system_clipboard::write(cx, ClipboardItem::new_string(this.commit.id.clone()));
                                cx.emit(CommitPageEvent::Said(format!("Copied {}", this.commit.id)));
                            })),
                    ),
            );
        let file_rows = self.files.iter().enumerate().map(|(ix, file)| {
            let name = std::path::Path::new(&file.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.path.clone());
            let color = match file.kind {
                'A' => theme.git_added,
                'D' => theme.git_deleted,
                _ => theme.foreground,
            };
            let selected = self.selected == Some(ix);
            div()
                .id(("commit-file", ix))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .h(px(ui::ROW))
                .rounded(px(ui::R_ROW))
                .cursor_pointer()
                .when(selected, |r| r.bg(theme.hairline))
                .when(!selected, |r| r.hover(|s| s.bg(theme.hairline.opacity(0.6))))
                .child(div().flex_1().min_w_0().truncate().text_color(color).child(name))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .code_font(cx)
                        .text_size(px(ui::T_XS))
                        .when(file.added > 0, |d| {
                            d.child(div().text_color(theme.git_added).child(format!("+{}", file.added)))
                        })
                        .when(file.removed > 0, |d| {
                            d.child(div().text_color(theme.git_deleted).child(format!("−{}", file.removed)))
                        }),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select(ix, cx)))
        });
        div()
            .key_context("CommitPage")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_size(px(ui::T_MD))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .id("commit-files")
                            .w(px(230.))
                            .flex_none()
                            .p(px(8.))
                            .border_r_1()
                            .border_color(theme.hairline)
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .children(file_rows),
                    )
                    .child(div().flex_1().min_w_0().children(self.shown.clone())),
            )
    }
}

use crate::fonts::CodeFont as _;
