//! The GitHub tab: the branch's pull request (or one made for it), the repository's pull
//! requests and issues, and those for you (a review asked of you, an issue given you).
//! Read through gh when the tab's shown or refreshed: never on its own.

use super::{GitView, GitViewEvent};
use crate::github::{self, Account, Checks, Issue, Pull};
use crate::theme::Theme;
use crate::ui::{self, px};
use gpui::{ClickEvent, Context, Window, div, prelude::*};

gpui::actions!(git_view, [CreatePull, CancelPull]);

pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([
        gpui::KeyBinding::new("secondary-enter", CreatePull, Some("GitHubCreate")),
        gpui::KeyBinding::new("escape", CancelPull, Some("GitHubCreate")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum List {
    #[default]
    Pulls,
    Issues,
    ForYou,
}

/// What the GitHub tab shows, read off the main thread.
#[derive(Default)]
pub struct GitHub {
    /// None until read.
    pub account: Option<Account>,
    /// The branch's own pull request.
    pub mine: Option<Pull>,
    pub pulls: Vec<Pull>,
    pub issues: Vec<Issue>,
    /// Pull requests whose review is asked of you, and issues given you.
    pub review_asked: Vec<Pull>,
    pub assigned: Vec<Issue>,
    pub list: List,
    /// Closed ones shown instead of open ones.
    pub closed: bool,
    pub error: Option<String>,
    pub reading: bool,
    /// The form for a new pull request, open.
    pub creating: bool,
    pub draft: bool,
    pub sending: bool,
    /// The branch checked out (None: on no branch), when last read.
    pub on_branch: Option<String>,
}

struct Read {
    account: Account,
    on_branch: Option<String>,
    mine: Option<Pull>,
    pulls: Vec<Pull>,
    issues: Vec<Issue>,
    review_asked: Vec<Pull>,
    assigned: Vec<Issue>,
    /// The first of the lists that couldn't be read: why.
    error: Option<String>,
}

impl GitView {
    pub fn refresh_github(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let state = if self.github.closed { "closed" } else { "open" };
        self.github.reading = true;
        cx.notify();
        self.github_task = Some(cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    let account = github::account(&root);
                    let on_branch = crate::git::branch_checked_out(&root);
                    let nothing = |account, on_branch| Read {
                        account,
                        on_branch,
                        mine: None,
                        pulls: Vec::new(),
                        issues: Vec::new(),
                        review_asked: Vec::new(),
                        assigned: Vec::new(),
                        error: None,
                    };
                    if !matches!(account, Account::SignedIn(_)) {
                        return nothing(account, on_branch);
                    }
                    // Each a request to GitHub: side by side.
                    std::thread::scope(|s| {
                        let mine = s.spawn(|| github::pull_for_branch(&root));
                        let issues = s.spawn(|| github::issues(&root, state, None));
                        let asked = s.spawn(|| github::pulls(&root, "open", Some("review-requested:@me")));
                        let assigned = s.spawn(|| github::issues(&root, "open", Some("assignee:@me")));
                        let pulls = github::pulls(&root, state, None);
                        fn joined<T>(r: std::thread::Result<Result<T, String>>) -> Result<T, String> {
                            r.unwrap_or_else(|_| Err("gh stopped.".to_string()))
                        }
                        let (issues, asked, assigned) =
                            (joined(issues.join()), joined(asked.join()), joined(assigned.join()));
                        let error = [
                            pulls.as_ref().err(),
                            issues.as_ref().err(),
                            asked.as_ref().err(),
                            assigned.as_ref().err(),
                        ]
                        .into_iter()
                        .flatten()
                        .next()
                        .cloned();
                        Read {
                            account,
                            on_branch,
                            mine: mine.join().ok().flatten(),
                            pulls: pulls.unwrap_or_default(),
                            issues: issues.unwrap_or_default(),
                            review_asked: asked.unwrap_or_default(),
                            assigned: assigned.unwrap_or_default(),
                            error,
                        }
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                let github = &mut this.github;
                github.reading = false;
                github.account = Some(read.account);
                github.on_branch = read.on_branch;
                github.mine = read.mine;
                github.pulls = read.pulls;
                github.issues = read.issues;
                github.review_asked = read.review_asked;
                github.assigned = read.assigned;
                github.error = read.error;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Read again when what it shows has changed under it: first shown, or the branch
    /// switched (GitHub's asked; never on every change here).
    pub(super) fn refresh_github_if_stale(&mut self, cx: &mut Context<Self>) {
        if self.github.reading {
            return;
        }
        if self.github.account.is_none() {
            return self.refresh_github(cx);
        }
        let (root, was) = (self.root.clone(), self.github.on_branch.clone());
        self.github_task = Some(cx.spawn(async move |this, cx| {
            let now = cx.background_executor().spawn(async move { crate::git::branch_checked_out(&root) }).await;
            if now != was {
                this.update(cx, |this, cx| this.refresh_github(cx)).ok();
            }
        }));
    }

    /// The form for a pull request of this branch: its title the last commit's, into the
    /// repository's main branch (to change).
    fn open_create_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.github.creating = true;
        let root = self.root.clone();
        window.focus(&self.pr_title.focus_handle(cx));
        cx.spawn(async move |this, cx| {
            let (subject, base) = cx
                .background_executor()
                .spawn(async move {
                    let subject = crate::git::message_of(&root, "HEAD").lines().next().unwrap_or("").to_string();
                    (subject, github::default_branch(&root).unwrap_or_else(|| "main".into()))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.pr_title.read(cx).text().is_empty() {
                    this.pr_title.update(cx, |input, cx| input.set_text_caret_at_end(&subject, cx));
                }
                if this.pr_base.read(cx).text().is_empty() {
                    this.pr_base.update(cx, |input, cx| input.set_text_caret_at_end(&base, cx));
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn cancel_pull(&mut self, _: &CancelPull, _: &mut Window, cx: &mut Context<Self>) {
        self.github.creating = false;
        cx.notify();
    }

    /// ⌘↵ (or the button): the branch pushed, and its pull request made.
    pub(super) fn create_pull(&mut self, _: &CreatePull, _: &mut Window, cx: &mut Context<Self>) {
        let title = self.pr_title.read(cx).text().trim().to_string();
        let base = self.pr_base.read(cx).text().trim().to_string();
        if title.is_empty() || base.is_empty() || self.github.sending {
            return;
        }
        let body = self.pr_body.read(cx).text().to_string();
        let (root, draft) = (self.root.clone(), self.github.draft);
        self.github.sending = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { github::create_pull(&root, &title, &body, &base, draft) })
                .await;
            this.update(cx, |this, cx| {
                this.github.sending = false;
                let said = match result {
                    Ok(url) => {
                        this.github.creating = false;
                        this.github.draft = false;
                        for input in [&this.pr_title, &this.pr_body, &this.pr_base] {
                            input.update(cx, |input, cx| input.set_text("", cx));
                        }
                        format!("Opened {url}")
                    }
                    Err(error) => format!("Couldn't open the pull request: {error}"),
                };
                cx.emit(GitViewEvent::Said(said));
                cx.emit(GitViewEvent::Changed);
                this.refresh_github(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_github(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        if self.repository == Some(false) {
            return ui::empty_state("This folder isn't a git repository", &theme).into_any_element();
        }
        let said = match &self.github.account {
            None => Some("Asking GitHub…".to_string()),
            Some(Account::Missing) => Some(github::MISSING.to_string()),
            Some(Account::SignedOut) => {
                Some("gh isn't signed in to GitHub: run “gh auth login” in the terminal".into())
            }
            Some(Account::Failed(error)) => Some(format!("Couldn't ask GitHub: {error}")),
            Some(Account::SignedIn(_)) => None,
        };
        if let Some(said) = said {
            let retry = self.github.account.is_some();
            return div()
                .flex()
                .flex_col()
                // (Said as a list says it has nothing: no full stop.)
                .child(ui::empty_state(said.trim_end_matches('.').to_string(), &theme))
                .when(retry, |d| {
                    d.child(
                        div().flex().px(px(ui::INSET)).pt(px(8.)).text_size(px(ui::T_SM)).child(
                            ui::link("github-retry", if self.github.reading { "Asking…" } else { "Try Again" }, &theme)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_github(cx))),
                        ),
                    )
                })
                .into_any_element();
        }
        let login = match &self.github.account {
            Some(Account::SignedIn(login)) => login.clone(),
            _ => String::new(),
        };
        let link = |id: &'static str, label: String| ui::link(id, label, &theme);

        // The branch's pull request, or the way to make one.
        let branch = self.github.on_branch.clone();
        let mine: gpui::AnyElement = if branch.is_none() {
            ui::empty_state("Not on a branch: switch to one to see its pull request", &theme)
                .pt(px(4.))
                .pb(px(8.))
                .into_any_element()
        } else if let Some(pull) = self.github.mine.clone() {
            let number = pull.number;
            div()
                .id("github-mine")
                .mx(px(12.))
                .mt(px(4.))
                .mb(px(8.))
                .cursor_pointer()
                .child(ui::card(&theme).child(pull_row(&pull, false, &theme)))
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::ShowPull(number))))
                .into_any_element()
        } else if self.github.creating {
            self.render_create_form(&theme, cx)
        } else {
            ui::card(&theme)
                .mx(px(12.))
                .mt(px(4.))
                .mb(px(8.))
                .px(px(10.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(ui::T_SM))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.muted)
                        .child(format!("{} has no pull request", branch.unwrap_or_default())),
                )
                .child(
                    link("github-create", "Create…".into())
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_create_form(window, cx))),
                )
                .into_any_element()
        };

        let for_you = self.github.review_asked.len() + self.github.assigned.len();
        let chip = |id: &'static str, label: String, which: List, cx: &mut Context<Self>| {
            let on = self.github.list == which;
            ui::segment(on, &theme)
                .id(id)
                // (Three side by side: each as wide as its name, narrower when the sidebar is.)
                .px(px(6.))
                .flex_auto()
                .min_w_0()
                .flex()
                .justify_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .cursor_pointer()
                .when(!on, |c| c.hover(|s| s.text_color(theme.foreground)))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.github.list = which;
                    cx.notify();
                }))
        };
        let other_state = if self.github.closed { "Show Open" } else { "Show Closed" };
        let chips = ui::segmented(&theme)
            .mx(px(12.))
            .mb(px(8.))
            .child(chip("github-pulls", format!("Pull requests {}", self.github.pulls.len()), List::Pulls, cx))
            .child(chip("github-issues", format!("Issues {}", self.github.issues.len()), List::Issues, cx))
            .child(chip("github-for-you", format!("For you {for_you}"), List::ForYou, cx));
        let state_link = (self.github.list != List::ForYou).then(|| {
            ui::link("github-state", other_state, &theme).px(px(10.)).pt(px(8.)).text_size(px(ui::T_XS)).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.github.closed = !this.github.closed;
                    this.refresh_github(cx);
                }),
            )
        });

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let pull_rows = |pulls: &[Pull], asked: bool, rows: &mut Vec<gpui::AnyElement>, cx: &mut Context<Self>| {
            for pull in pulls {
                let number = pull.number;
                rows.push(
                    div()
                        .id(gpui::SharedString::from(format!("pull-{number}")))
                        .cursor_pointer()
                        .rounded(px(ui::R_ROW))
                        .hover(|s| s.bg(ui::hover(&theme)))
                        .child(pull_row(pull, asked, &theme))
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::ShowPull(number))))
                        .into_any_element(),
                );
            }
        };
        let issue_rows = |issues: &[Issue], rows: &mut Vec<gpui::AnyElement>, cx: &mut Context<Self>| {
            for issue in issues {
                let number = issue.number;
                rows.push(
                    div()
                        .id(gpui::SharedString::from(format!("issue-{number}")))
                        .cursor_pointer()
                        .rounded(px(ui::R_ROW))
                        .hover(|s| s.bg(ui::hover(&theme)))
                        .child(issue_row(issue, &theme))
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::ShowIssue(number))))
                        .into_any_element(),
                );
            }
        };
        match self.github.list {
            List::Pulls => pull_rows(&self.github.pulls.clone(), false, &mut rows, cx),
            List::Issues => issue_rows(&self.github.issues.clone(), &mut rows, cx),
            List::ForYou => {
                pull_rows(&self.github.review_asked.clone(), true, &mut rows, cx);
                issue_rows(&self.github.assigned.clone(), &mut rows, cx);
            }
        }
        let empty = match (self.github.list, self.github.closed) {
            _ if self.github.reading => "Asking GitHub…",
            (List::Pulls, false) => "No open pull requests",
            (List::Pulls, true) => "No closed pull requests",
            (List::Issues, false) => "No open issues",
            (List::Issues, true) => "No closed issues",
            (List::ForYou, _) => "Nothing asked of you",
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(mine)
            .child(chips)
            .child(
                div()
                    .id("github-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when_some(self.github.error.clone(), |d, error| {
                        d.child(div().px(px(10.)).text_size(px(ui::T_SM)).text_color(theme.error).child(error))
                    })
                    .when(rows.is_empty() && self.github.error.is_none(), |d| {
                        d.child(ui::empty_state(empty, &theme).px(px(10.)).pt(px(6.)))
                    })
                    .children(rows)
                    .children(state_link),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(10.))
                    .text_size(px(ui::T_XS))
                    .text_color(theme.muted)
                    .child(div().flex_1().truncate().child(format!("Signed in as {login} (gh)")))
                    .child(
                        ui::link("github-refresh", if self.github.reading { "Refreshing…" } else { "Refresh" }, &theme)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_github(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_create_form(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let field = |input: gpui::Entity<crate::text_input::TextInput>| ui::field(theme).child(input);
        let ready = !self.pr_title.read(cx).text().trim().is_empty() && !self.github.sending;
        let draft = self.github.draft;
        ui::card(theme)
            .key_context("GitHubCreate")
            .on_action(cx.listener(Self::create_pull))
            .on_action(cx.listener(Self::cancel_pull))
            .mx(px(12.))
            .mt(px(4.))
            .mb(px(8.))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_size(px(ui::T_SM))
            .child(field(self.pr_title.clone()))
            .child(field(self.pr_body.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.muted)
                            .child(format!("{} →", self.github.on_branch.clone().unwrap_or_default())),
                    )
                    .child(field(self.pr_base.clone()).flex_1().min_w_0().code_font(cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pt(px(4.))
                    .child(
                        div()
                            .id("github-draft")
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_color(theme.muted)
                            .child(ui::checkbox(draft, false, theme))
                            .child("Draft")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.github.draft = !this.github.draft;
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(ui::link("github-cancel", "Cancel", theme).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.cancel_pull(&CancelPull, window, cx)),
                    ))
                    .child(
                        ui::primary_button(
                            "github-send",
                            if self.github.sending {
                                "Pushing…".to_string()
                            } else {
                                format!("Create {}", ui::keys(&CreatePull, "⌘↵", cx))
                            },
                            ready,
                            theme,
                        )
                        .when(ready, |b| {
                            b.on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.create_pull(&CreatePull, window, cx)
                                }),
                            )
                        }),
                    ),
            )
            .into_any_element()
    }
}

/// A pull request's row: a dot for how its checks went, its title and number; then what's
/// to know (draft, checks, comments, review).
fn pull_row(pull: &Pull, asked: bool, theme: &Theme) -> gpui::Div {
    let dot = match (pull.state.as_str(), pull.checks) {
        ("MERGED", _) => theme.ansi[13],
        ("CLOSED", _) => theme.faint,
        (_, Checks::Failed(_)) => theme.git_deleted,
        (_, Checks::Running) => theme.caret,
        _ if pull.draft => theme.caret,
        (_, Checks::Passed(_)) => theme.git_added,
        (_, Checks::None) => theme.muted,
    };
    let mut said: Vec<String> = Vec::new();
    match pull.state.as_str() {
        "MERGED" => said.push("merged".into()),
        "CLOSED" => said.push("closed".into()),
        _ => {}
    }
    if pull.draft {
        said.push("draft".into());
    }
    match pull.checks {
        Checks::Passed(_) => said.push("checks passed".into()),
        Checks::Running => said.push("checks running".into()),
        Checks::Failed(1) => said.push("1 check failed".into()),
        Checks::Failed(n) => said.push(format!("{n} checks failed")),
        Checks::None => {}
    }
    match pull.comments {
        0 => {}
        1 => said.push("1 comment".into()),
        n => said.push(format!("{n} comments")),
    }
    match pull.review.as_str() {
        "APPROVED" => said.push("approved".into()),
        "CHANGES_REQUESTED" => said.push("changes requested".into()),
        _ if asked => said.push("your review requested".into()),
        _ => {}
    }
    if said.is_empty() {
        said.push(format!("{} → {}", pull.head, pull.base));
    }
    row(dot, pull.title.clone(), pull.number, said.join(" · "), theme)
}

fn issue_row(issue: &Issue, theme: &Theme) -> gpui::Div {
    let dot = if issue.state == "OPEN" { theme.git_added } else { theme.faint };
    let mut said = vec![issue.author.clone()];
    said.extend(issue.labels.iter().cloned());
    match issue.comments {
        0 => {}
        1 => said.push("1 comment".into()),
        n => said.push(format!("{n} comments")),
    }
    row(dot, issue.title.clone(), issue.number, said.join(" · "), theme)
}

fn row(dot: gpui::Hsla, title: String, number: u64, said: String, theme: &Theme) -> gpui::Div {
    div()
        .px(px(10.))
        .py(px(7.))
        .flex()
        .flex_col()
        .text_size(px(ui::T_SM))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_none().size(px(7.)).rounded_full().bg(dot))
                .child(div().flex_1().min_w_0().truncate().child(title))
                .child(div().flex_none().text_size(px(ui::T_XS)).text_color(theme.muted).child(format!("#{number}"))),
        )
        .child(div().pl(px(15.)).truncate().text_size(px(ui::T_XS)).text_color(theme.muted).child(said))
}

use crate::fonts::CodeFont as _;
use gpui::Focusable as _;
