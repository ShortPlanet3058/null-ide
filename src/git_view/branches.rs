//! The Branches tab: the branches on this computer and the remotes', to switch to, merge
//! in or rebase onto; a new one started; the tags; the changes set aside, to bring back.

use super::{GitView, GitViewEvent};
use crate::git;
use crate::theme::Theme;
use crate::ui::{self, px};
use gpui::{ClickEvent, Context, Focusable as _, PromptLevel, Window, div, prelude::*};

gpui::actions!(git_view, [NameBranch, RenameBranch, CancelRename]);

pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([
        gpui::KeyBinding::new("enter", NameBranch, Some("GitBranchName")),
        gpui::KeyBinding::new("enter", RenameBranch, Some("GitBranchRename")),
        gpui::KeyBinding::new("escape", CancelRename, Some("GitBranchRename")),
    ]);
}

/// What the Branches tab shows, read off the main thread.
#[derive(Default)]
pub struct Branches {
    pub local: Vec<git::LocalBranch>,
    pub remote: Vec<String>,
    pub tags: Vec<String>,
    pub set_aside: Vec<git::SetAside>,
}

impl GitView {
    pub fn refresh_branches(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.branches_task = Some(cx.spawn(async move |this, cx| {
            let (read, branch) = cx
                .background_executor()
                .spawn(async move {
                    let read = Branches {
                        local: git::local_branches(&root),
                        remote: git::remote_branches(&root),
                        tags: git::tags(&root),
                        set_aside: git::set_aside_list(&root),
                    };
                    (read, git::current_branch(&root))
                })
                .await;
            this.update(cx, |this, cx| {
                this.repository = Some(branch.is_some());
                if let Some(branch) = &branch {
                    this.branch_name.update(cx, |input, _| input.set_placeholder(format!("New branch from {branch}…")));
                }
                this.branch = branch;
                this.branches = read;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Enter in the new branch's name: it's started (and switched to).
    pub(super) fn name_branch(&mut self, _: &NameBranch, _: &mut Window, cx: &mut Context<Self>) {
        let name = git::branch_name(self.branch_name.read(cx).text());
        if name.is_empty() {
            return;
        }
        self.branch_name.update(cx, |input, cx| input.set_text("", cx));
        cx.emit(GitViewEvent::NewBranch(name));
    }

    fn start_rename(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_input.update(cx, |input, cx| {
            input.set_text(&name, cx);
            input.select_all_text(cx);
        });
        window.focus(&self.rename_input.focus_handle(cx));
        self.renaming = Some(name);
        cx.notify();
    }

    pub(super) fn rename_branch(&mut self, _: &RenameBranch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(old) = self.renaming.take() else { return };
        let new = git::branch_name(self.rename_input.read(cx).text());
        // (The field goes: the keys stay in the tab.)
        window.focus(&self.focus_handle);
        cx.notify();
        if new.is_empty() || new == old {
            return;
        }
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn({
                let (old, new) = (old.clone(), new.clone());
                async move { git::rename_branch(&root, &old, &new) }
            });
            let result = result.await;
            let renamed = result.is_ok();
            let said = match result {
                Ok(()) => format!("Renamed {old} to {new}."),
                Err(error) => error,
            };
            this.update(cx, |this, cx| {
                if renamed {
                    this.picked = Some(new);
                }
                cx.emit(GitViewEvent::Said(said));
                cx.emit(GitViewEvent::Changed);
                this.refresh_branches(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn cancel_rename(&mut self, _: &CancelRename, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// Deletes a branch, asked first; one with commits merged nowhere else asked again.
    fn delete_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let answer = crate::modal::ask(
            window,
            PromptLevel::Warning,
            &format!("Delete the branch {name}?"),
            Some("Only on this computer: a remote's copy stays."),
            &["Delete", "Cancel"],
            cx,
        );
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let (r, n) = (root.clone(), name.clone());
            let mut result = cx.background_executor().spawn(async move { git::delete_branch(&r, &n, false) }).await;
            if result.as_ref().is_err_and(|e| e == git::NOT_MERGED) {
                let Ok(again) = this.update_in(cx, |_, window, cx| {
                    crate::modal::ask(
                        window,
                        PromptLevel::Warning,
                        &format!("{name} has commits merged nowhere else."),
                        Some("If you delete it, those commits are hard to get back."),
                        &["Delete Anyway", "Cancel"],
                        cx,
                    )
                }) else {
                    return;
                };
                if again.await != Ok(0) {
                    return;
                }
                let n = name.clone();
                result = cx.background_executor().spawn(async move { git::delete_branch(&root, &n, true) }).await;
            }
            this.update(cx, |this, cx| {
                let said = match result {
                    Ok(()) => format!("Deleted {name}."),
                    Err(error) => error,
                };
                this.picked = None;
                cx.emit(GitViewEvent::Said(said));
                this.refresh_branches(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Discards changes set aside, asked first.
    fn drop_set_aside(&mut self, set: git::SetAside, window: &mut Window, cx: &mut Context<Self>) {
        let answer = crate::modal::ask(
            window,
            PromptLevel::Warning,
            &format!("Discard “{}”?", set.message),
            Some("These changes can't be brought back afterwards."),
            &["Discard", "Cancel"],
            cx,
        );
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let result =
                cx.background_executor().spawn(async move { git::drop_set_aside(&root, set.index, &set.id) }).await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    cx.emit(GitViewEvent::Said(error));
                }
                this.refresh_branches(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_branches(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>().clone();
        if self.repository == Some(false) {
            return ui::empty_state("This folder isn't a git repository", &theme).into_any_element();
        }
        let heading = |text: &str| ui::sidebar_heading(text, &theme);
        // An action under the branch picked: a word to click.
        let link = |id: String, label: &'static str| ui::link(gpui::SharedString::from(id), label, &theme);
        let row = |id: String, picked: bool| {
            div()
                .id(gpui::SharedString::from(id))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .h(px(ui::ROW))
                .rounded(px(ui::R_ROW))
                .cursor_pointer()
                .when(picked, |r| r.bg(theme.hairline))
                .when(!picked, |r| r.hover(|s| s.bg(ui::hover(&theme))))
        };
        let actions_row = || {
            div()
                .flex()
                .flex_wrap()
                .gap_x(px(12.))
                .gap_y(px(2.))
                .pl(px(25.))
                .pr(px(10.))
                .pt(px(2.))
                .pb(px(6.))
                .text_size(px(ui::T_SM))
        };

        let mut local_rows = Vec::new();
        for branch in self.branches.local.clone() {
            let name = branch.name.clone();
            if self.renaming.as_deref() == Some(name.as_str()) {
                local_rows.push(
                    div()
                        .key_context("GitBranchRename")
                        .on_action(cx.listener(Self::rename_branch))
                        .on_action(cx.listener(Self::cancel_rename))
                        .mx(px(4.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(ui::R_ROW))
                        .border_1()
                        .border_color(theme.caret)
                        .code_font(cx)
                        .text_size(px(ui::T_SM))
                        .child(self.rename_input.clone())
                        .into_any_element(),
                );
                continue;
            }
            let picked = self.picked.as_deref() == Some(name.as_str());
            let counts = match (branch.ahead, branch.behind) {
                (0, 0) => String::new(),
                (a, 0) => format!("↑{a}"),
                (0, b) => format!("↓{b}"),
                (a, b) => format!("↑{a} ↓{b}"),
            };
            let pick = name.clone();
            local_rows.push(
                row(format!("branch-{name}"), picked)
                    .child(div().flex_none().size(px(7.)).rounded_full().when(branch.current, |d| d.bg(theme.caret)))
                    .child(
                        div().flex_1().min_w_0().truncate().code_font(cx).text_size(px(ui::T_SM)).child(name.clone()),
                    )
                    .child(div().flex_none().text_size(px(ui::T_XS)).text_color(theme.muted).child(counts))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.picked =
                            if this.picked.as_deref() == Some(pick.as_str()) { None } else { Some(pick.clone()) };
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            if !picked {
                continue;
            }
            let mut actions = actions_row();
            if !branch.current {
                let (a, b, c) = (name.clone(), name.clone(), name.clone());
                actions = actions
                    .child(link(format!("switch-{name}"), "Switch").on_click(cx.listener(
                        move |_, _: &ClickEvent, _, cx| {
                            cx.emit(GitViewEvent::SwitchTo(git::Branch {
                                name: a.clone(),
                                current: false,
                                remote: false,
                                when: String::new(),
                            }))
                        },
                    )))
                    .child(link(format!("merge-{name}"), "Merge In").on_click(cx.listener(
                        move |_, _: &ClickEvent, _, cx| {
                            cx.emit(GitViewEvent::Join { branch: b.clone(), rebase: false })
                        },
                    )))
                    .child(link(format!("rebase-{name}"), "Rebase Onto").on_click(cx.listener(
                        move |_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::Join { branch: c.clone(), rebase: true }),
                    )));
            }
            let (r, d) = (name.clone(), name.clone());
            actions = actions.child(link(format!("rename-{name}"), "Rename").on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| this.start_rename(r.clone(), window, cx)),
            ));
            if !branch.current {
                actions = actions.child(link(format!("delete-{name}"), "Delete").on_click(
                    cx.listener(move |this, _: &ClickEvent, window, cx| this.delete_branch(d.clone(), window, cx)),
                ));
            }
            local_rows.push(actions.into_any_element());
        }

        // The remotes' branches, under each remote's name.
        let mut remote_sections = Vec::new();
        let mut remotes: Vec<&str> =
            self.branches.remote.iter().filter_map(|r| r.split_once('/').map(|(r, _)| r)).collect();
        remotes.dedup();
        for remote in remotes {
            remote_sections.push(heading(remote).mx(px(-6.)).into_any_element());
            for full in self.branches.remote.iter().filter(|r| r.split_once('/').is_some_and(|(r, _)| r == remote)) {
                let short = full.split_once('/').map_or(full.as_str(), |(_, s)| s).to_string();
                let picked = self.picked.as_deref() == Some(full.as_str());
                let pick = full.clone();
                remote_sections.push(
                    row(format!("remote-{full}"), picked)
                        .pl(px(25.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .code_font(cx)
                                .text_size(px(ui::T_SM))
                                .text_color(theme.muted)
                                .child(short.clone()),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.picked =
                                if this.picked.as_deref() == Some(pick.as_str()) { None } else { Some(pick.clone()) };
                            cx.notify();
                        }))
                        .into_any_element(),
                );
                if picked {
                    let here = self.branches.local.iter().any(|b| b.name == short);
                    let (a, b, c) = (full.clone(), full.clone(), full.clone());
                    remote_sections.push(
                        actions_row()
                            .when(!here, |row| {
                                row.child(link(format!("switch-{full}"), "Switch").on_click(cx.listener(
                                    move |_, _: &ClickEvent, _, cx| {
                                        cx.emit(GitViewEvent::SwitchTo(git::Branch {
                                            name: a.clone(),
                                            current: false,
                                            remote: true,
                                            when: String::new(),
                                        }))
                                    },
                                )))
                            })
                            .child(link(format!("merge-{full}"), "Merge In").on_click(cx.listener(
                                move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(GitViewEvent::Join { branch: b.clone(), rebase: false })
                                },
                            )))
                            .child(link(format!("rebase-{full}"), "Rebase Onto").on_click(cx.listener(
                                move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(GitViewEvent::Join { branch: c.clone(), rebase: true })
                                },
                            )))
                            .into_any_element(),
                    );
                }
            }
        }

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let set_rows = self.branches.set_aside.clone().into_iter().map(|set| {
            let index = set.index;
            let message =
                if set.message == "Null: set aside" { "Changes set aside".to_string() } else { set.message.clone() };
            let (dropped, brought) = (set.clone(), set.clone());
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(25.))
                .pr(px(10.))
                .py(px(4.))
                .text_size(px(ui::T_SM))
                .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(message)).child(
                    div().truncate().text_size(px(ui::T_XS)).text_color(theme.muted).child(format!(
                        "on {} · {}",
                        set.branch,
                        git::ago(set.time, now)
                    )),
                ))
                .child(link(format!("bring-back-{index}"), "Bring Back").on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::BringBack(brought.clone()))),
                ))
                .child(ui::icon_button(("drop-set-aside", index), "icons/x.svg", "Discard", &theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.drop_set_aside(dropped.clone(), window, cx)
                    }),
                ))
        });
        div()
            .id("git-branches")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(12.))
            .child(
                ui::field(&theme)
                    .key_context("GitBranchName")
                    .on_action(cx.listener(Self::name_branch))
                    .mx(px(12.))
                    .mt(px(4.))
                    .child(self.branch_name.clone()),
            )
            .child(heading("This computer"))
            .child(div().px(px(6.)).flex().flex_col().children(local_rows))
            .children(remote_sections.into_iter().map(|e| div().px(px(6.)).child(e)))
            .when(!self.branches.tags.is_empty(), |d| {
                d.child(heading("Tags")).child(
                    div()
                        .pl(px(31.))
                        .pr(px(16.))
                        .code_font(cx)
                        .text_size(px(ui::T_SM))
                        .text_color(theme.muted)
                        .child(self.branches.tags.join(" · ")),
                )
            })
            .child(
                div().flex().items_center().pr(px(16.)).child(div().flex_1().child(heading("Set aside"))).child(
                    div().pt(px(10.)).text_size(px(ui::T_SM)).child(
                        link("set-aside-now".into(), "Set Changes Aside")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(GitViewEvent::SetAside))),
                    ),
                ),
            )
            .when(self.branches.set_aside.is_empty(), |d| {
                // In line with the rows' text.
                d.child(ui::empty_state("Nothing set aside", &theme).pt(px(0.)).pl(px(31.)))
            })
            .child(div().px(px(6.)).flex().flex_col().children(set_rows))
            .into_any_element()
    }
}

use crate::fonts::CodeFont as _;
