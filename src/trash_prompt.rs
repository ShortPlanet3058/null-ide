//! Moving files to the Trash, asked in the window (the system's dialog has no room for
//! "Don't ask again"): ↵ moves them, esc doesn't, and the box keeps it from being asked
//! again (Settings → Window brings the question back).

use crate::theme::Theme;
use crate::ui;
use crate::ui::px;
use gpui::{App, Context, EventEmitter, FocusHandle, Focusable, KeyBinding, Window, actions, div, prelude::*};

actions!(trash_prompt, [ConfirmTrash, CancelTrash, ToggleDontAsk]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("TrashPrompt");
    cx.bind_keys([
        KeyBinding::new("enter", ConfirmTrash, ctx),
        KeyBinding::new("escape", CancelTrash, ctx),
        KeyBinding::new("space", ToggleDontAsk, ctx),
    ]);
}

pub enum TrashPromptEvent {
    /// Whether to move them, and whether to ask again next time.
    Answered { trash: bool, dont_ask: bool },
}

pub struct TrashPrompt {
    question: String,
    detail: String,
    dont_ask: bool,
    focus: FocusHandle,
}

impl EventEmitter<TrashPromptEvent> for TrashPrompt {}

impl TrashPrompt {
    pub fn new(question: String, detail: String, cx: &mut Context<Self>) -> Self {
        Self { question, detail, dont_ask: false, focus: cx.focus_handle() }
    }

    fn confirm(&mut self, _: &ConfirmTrash, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TrashPromptEvent::Answered { trash: true, dont_ask: self.dont_ask });
    }

    fn cancel(&mut self, _: &CancelTrash, _: &mut Window, cx: &mut Context<Self>) {
        // (Not moved: nothing to stop asking about.)
        cx.emit(TrashPromptEvent::Answered { trash: false, dont_ask: false });
    }

    fn toggle(&mut self, _: &ToggleDontAsk, _: &mut Window, cx: &mut Context<Self>) {
        self.dont_ask = !self.dont_ask;
        cx.notify();
    }
}

impl Focusable for TrashPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TrashPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let button = |id: &'static str, label: &'static str, strong: bool| {
            div()
                .id(id)
                .px(px(12.))
                .h(px(ui::FIELD))
                .flex()
                .items_center()
                .rounded(px(ui::R_CONTROL))
                .cursor_pointer()
                .when(strong, |d| d.bg(theme.selection).text_color(theme.foreground))
                .when(!strong, |d| d.text_color(theme.muted).hover(|s| s.text_color(theme.foreground)))
                .active(|s| s.opacity(0.7))
                .child(label)
        };
        div()
            .key_context("TrashPrompt")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::toggle))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(440.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .rounded(px(ui::R_MODAL))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.raised)
            .shadow_lg()
            .text_size(px(ui::T_MD))
            .child(
                div()
                    .text_size(px(ui::T_LG))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(self.question.clone()),
            )
            .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(self.detail.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(px(4.))
                    .child(
                        div()
                            .id("dont-ask")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .text_color(theme.muted)
                            .hover(|d| d.text_color(theme.foreground))
                            .child(ui::checkbox(self.dont_ask, false, theme))
                            .child("Don't ask again")
                            .on_click(cx.listener(|this, _, window, cx| this.toggle(&ToggleDontAsk, window, cx))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                button("cancel", "Cancel", false)
                                    .on_click(cx.listener(|this, _, window, cx| this.cancel(&CancelTrash, window, cx))),
                            )
                            .child(
                                button("trash", crate::fs_ops::MOVE_TO_TRASH, true).on_click(
                                    cx.listener(|this, _, window, cx| this.confirm(&ConfirmTrash, window, cx)),
                                ),
                            ),
                    ),
            )
    }
}
