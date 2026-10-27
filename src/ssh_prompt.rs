//! What ssh asks, asked in the window (see `askpass`): a password or a key's passphrase,
//! typed unseen; or whether to trust a server it hasn't met, with its fingerprint.

use crate::text_input::TextInput;
use crate::theme::Theme;
use crate::ui;
use crate::ui::px;
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, Window, actions, div, prelude::*};

actions!(ssh_prompt, [AnswerSsh, CancelSsh]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("SshPrompt");
    cx.bind_keys([KeyBinding::new("enter", AnswerSsh, ctx), KeyBinding::new("escape", CancelSsh, ctx)]);
}

pub enum SshPromptEvent {
    /// What to tell ssh (None: not answered, it gives up).
    Answered(Option<String>),
}

pub struct SshPrompt {
    question: String,
    /// Whether it asks to trust a server (answered yes or no, nothing typed).
    trust: bool,
    input: Entity<TextInput>,
}

impl EventEmitter<SshPromptEvent> for SshPrompt {}

impl SshPrompt {
    pub fn new(question: String, cx: &mut Context<Self>) -> Self {
        let trust = crate::askpass::is_trust_question(&question);
        let input = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.masked = true;
            input
        });
        Self { question, trust, input }
    }

    /// The same question, asked again meanwhile (by another of ssh's connections).
    pub fn asks(&self, question: &str) -> bool {
        self.question == question
    }

    fn answer(&mut self, _: &AnswerSsh, _: &mut Window, cx: &mut Context<Self>) {
        let answer = if self.trust { "yes".to_string() } else { self.input.read(cx).text().to_string() };
        cx.emit(SshPromptEvent::Answered(Some(answer)));
    }

    fn cancel(&mut self, _: &CancelSsh, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SshPromptEvent::Answered(None));
    }
}

impl Focusable for SshPrompt {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for SshPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        // ssh's own words, its last line the question ("me@box's password:").
        let lines: Vec<&str> = self.question.trim().lines().filter(|l| !l.trim().is_empty()).collect();
        let (title, said) = if self.trust {
            ("Trust this server?", lines.iter().take(lines.len().saturating_sub(1)).copied().collect::<Vec<_>>())
        } else {
            (lines.last().map_or("ssh asks", |l| l.trim().trim_end_matches(':')), Vec::new())
        };
        let hint = if self.trust {
            "↵ trust it and connect · esc don't".to_string()
        } else {
            "Asked by ssh; kept in memory until Null quits, never written down. Esc doesn't connect.".to_string()
        };
        div()
            .key_context("SshPrompt")
            .on_action(cx.listener(Self::answer))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(520.))
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
                    .child(title.to_string()),
            )
            .children(
                said.into_iter()
                    .map(|line| div().text_size(px(ui::T_SM)).text_color(theme.muted).child(line.trim().to_string())),
            )
            .when(!self.trust, |d| {
                d.child(
                    div()
                        .h(px(ui::FIELD))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(ui::R_CONTROL))
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.hairline)
                        .line_height(px(20.))
                        .child(self.input.clone()),
                )
            })
            // (Nothing typed for a trust question: the keys still need somewhere to go.)
            .when(self.trust, |d| d.child(div().size_0().overflow_hidden().child(self.input.clone())))
            .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(ui::key_words(&hint)))
    }
}
