//! Questions about your code, asked from the palette with `?`. The answer floats over
//! the editor and goes away with Esc; nothing stays docked.

use crate::ai::{self, AiEvent, Prompt};
use crate::fonts::Fonts;
use crate::markdown;
use crate::settings::Settings;
use crate::text_input::TextInput;
use crate::theme::Theme;
use futures::StreamExt;
use gpui::{
    App, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    ScrollHandle, Task, Window, actions, div, prelude::*, px,
};
use std::path::PathBuf;

actions!(ask, [FollowUp, CloseAsk]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("AskPanel");
    cx.bind_keys([KeyBinding::new("enter", FollowUp, ctx), KeyBinding::new("escape", CloseAsk, ctx)]);
}

/// The file the question is about, as it was when asked.
pub struct AskContext {
    pub path: Option<PathBuf>,
    pub language: &'static str,
    pub text: String,
    pub selection: String,
}

struct Turn {
    question: String,
    answer: String,
    failed: Option<String>,
}

pub enum AskEvent {
    Closed,
}

pub struct AskPanel {
    context: Option<AskContext>,
    turns: Vec<Turn>,
    follow_up: Entity<TextInput>,
    task: Option<Task<()>>,
    scroll: ScrollHandle,
}

impl EventEmitter<AskEvent> for AskPanel {}

impl AskPanel {
    pub fn new(context: Option<AskContext>, question: String, cx: &mut Context<Self>) -> Self {
        let follow_up = cx.new(|cx| TextInput::new("Ask a follow-up", cx));
        let mut panel = Self { context, turns: Vec::new(), follow_up, task: None, scroll: ScrollHandle::new() };
        panel.ask(question, cx);
        panel
    }

    fn ask(&mut self, question: String, cx: &mut Context<Self>) {
        let settings = cx.global::<Settings>().ai.clone();
        let mut user = String::new();
        if let Some(context) = &self.context {
            let path = context.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
            let text = crate::inline_assist::InlineAssist::trim_context(&context.text, &(0..0));
            user.push_str(&format!("The open file is {path} ({}):\n<file>\n{text}\n</file>\n\n", context.language));
            if !context.selection.trim().is_empty() {
                user.push_str(&format!("The selected code:\n<selection>\n{}\n</selection>\n\n", context.selection));
            }
        }
        for turn in &self.turns {
            user.push_str(&format!("Earlier question: {}\nYour answer: {}\n\n", turn.question, turn.answer));
        }
        user.push_str(&format!("Question: {question}"));
        let prompt = Prompt {
            system: "You are a concise, knowledgeable programming assistant inside the Null code editor, answering \
                     questions about the person's code. Be direct and specific to their code. Use markdown code \
                     fences for code. Don't pad the answer."
                .into(),
            user,
        };
        self.turns.push(Turn { question, answer: String::new(), failed: None });
        let mut events = ai::stream(settings, prompt);
        self.task = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let done = this
                    .update(cx, |this, cx| {
                        let Some(turn) = this.turns.last_mut() else { return true };
                        let done = match event {
                            AiEvent::Text(text) => {
                                turn.answer.push_str(&text);
                                false
                            }
                            AiEvent::Done => true,
                            AiEvent::Failed(message) => {
                                turn.failed = Some(message);
                                true
                            }
                        };
                        this.scroll.scroll_to_bottom();
                        cx.notify();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
            this.update(cx, |this, cx| {
                this.task = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn follow(&mut self, _: &FollowUp, _: &mut Window, cx: &mut Context<Self>) {
        let question = self.follow_up.read(cx).text().trim().to_string();
        if question.is_empty() || self.task.is_some() {
            return;
        }
        self.follow_up.update(cx, |input, cx| input.set_text("", cx));
        self.ask(question, cx);
    }

    fn close(&mut self, _: &CloseAsk, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(AskEvent::Closed);
    }
}

impl Focusable for AskPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.follow_up.focus_handle(cx)
    }
}

impl Render for AskPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let code_font = cx.global::<Fonts>().code.clone();
        let provider = cx.global::<Settings>().ai.provider.label();
        let writing = self.task.is_some();
        let turns = self.turns.iter().enumerate().map(|(t, turn)| {
            let blocks = markdown::blocks(&turn.answer, None).into_iter().enumerate().map(|(b, block)| {
                if block.code {
                    let text = block.text.clone();
                    div()
                        .relative()
                        .rounded(px(7.))
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.hairline)
                        .child(
                            div()
                                .id(("code", t * 1000 + b))
                                .overflow_x_scroll()
                                .px(px(12.))
                                .py(px(9.))
                                .font_family(code_font.clone())
                                .text_size(px(12.5))
                                .line_height(px(19.))
                                .whitespace_nowrap()
                                .children(block.text.lines().map(|l| div().child(l.to_string())).collect::<Vec<_>>()),
                        )
                        .child(
                            div()
                                .id(("copy", t * 1000 + b))
                                .absolute()
                                .top(px(6.))
                                .right(px(6.))
                                .px(px(7.))
                                .py(px(2.))
                                .rounded(px(5.))
                                .bg(theme.surface)
                                .text_size(px(11.))
                                .text_color(theme.muted)
                                .hover(|s| s.text_color(theme.foreground))
                                .child("Copy")
                                .on_click(move |_: &ClickEvent, _, cx: &mut App| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                                }),
                        )
                        .into_any_element()
                } else {
                    div().text_color(theme.foreground).child(block.text).into_any_element()
                }
            });
            let pending = turn.answer.is_empty() && turn.failed.is_none();
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .when(t > 0, |d| d.pt(px(14.)).border_t_1().border_color(theme.hairline))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.5)).child(turn.question.clone()))
                .children(blocks)
                .when(pending, |d| d.child(div().text_color(theme.faint).child(format!("Asking {provider}…"))))
                .children(turn.failed.clone().map(|m| div().text_color(theme.error).child(m)))
        });
        div()
            .key_context("AskPanel")
            .on_action(cx.listener(Self::follow))
            .on_action(cx.listener(Self::close))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(720.))
            .max_w_full()
            .max_h(px(560.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.raised)
            .shadow_lg()
            .text_size(px(13.5))
            .line_height(px(21.))
            .child(
                div()
                    .id("ask-scroll")
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .flex_1()
                    .min_h_0()
                    .px(px(22.))
                    .pt(px(18.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .children(turns),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(22.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(theme.hairline)
                    .child(div().flex_1().min_w_0().line_height(px(20.)).child(self.follow_up.clone()))
                    .child(div().flex_none().text_size(px(11.5)).text_color(theme.faint).child(if writing {
                        format!("{provider} is writing…")
                    } else {
                        format!("{provider} · Esc to close")
                    })),
            )
    }
}
